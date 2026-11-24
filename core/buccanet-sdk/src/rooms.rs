//! Creating rooms, invites, joining, and the admin side of membership.

use buccanet_protocol::api::{Approve, CreateRoom, InvitePreview, PendingRequest, RemoveMember, RoomPolicy, RoomView};
use buccanet_protocol::envelope::Envelope;
use rand::RngCore;

use crate::client::{Client, JoinPending};
use crate::error::SdkError;
use crate::events::{ClientEvent, MemberInfo, PendingJoin, RoomInfo};
use crate::history::build_share;
use crate::mls::ExpectedJoiner;
use crate::processing::reconcile;
use crate::relay::invite_token;
use crate::state::RoomState;
use crate::trust::{pin_new, pinned_key};

const COMMIT_ATTEMPTS: usize = 3;

fn members_of(view: &RoomView) -> Vec<MemberInfo> {
    view.members
        .iter()
        .map(|member| MemberInfo { device_id: member.device_id.clone(), name: member.name.clone(), kind: member.kind, admin: member.admin })
        .collect()
}

impl Client {
    pub async fn create_room(&self, title: Option<String>, policy: RoomPolicy) -> Result<RoomInfo, SdkError> {
        let mut group_id = vec![0u8; 16];
        rand::thread_rng().fill_bytes(&mut group_id);
        let info = {
            let mut inner = self.inner.lock().await;
            inner.mls.create_group(&group_id)?;
            let view = self.relay.create_room(&CreateRoom { group_id: group_id.clone(), policy }).await?;
            let room = RoomState {
                room_id: view.room_id.clone(),
                group_id,
                title: title.clone(),
                policy,
                admin: true,
                start_seq: 1,
                last_seq: 0,
                members: members_of(&view),
                messages: Vec::new(),
                pending: Vec::new(),
                history_imported: true,
                admin_id: inner.state.identity.device_id.clone(),
            };
            let info = room.info();
            inner.state.rooms.insert(view.room_id, room);
            self.persist(&inner);
            info
        };
        if let Some(title) = title {
            self.send_envelope(&info.room_id, &Envelope::RoomMeta { title }).await?;
        }
        Ok(info)
    }

    /// A reusable invite link, valid seven days, every join still subject to approval.
    pub async fn create_invite(&self, room_id: &str) -> Result<String, SdkError> {
        Ok(self.relay.create_invite(room_id).await?.url)
    }

    pub async fn preview_invite(&self, link: &str) -> Result<InvitePreview, SdkError> {
        self.relay.preview(&invite_token(link)?).await
    }

    /// Asks to join; the Welcome arrives in the inbox once an admin approves. The state stays
    /// locked until the request is recorded, so an immediate Welcome finds it.
    pub async fn join(&self, link: &str) -> Result<JoinPending, SdkError> {
        let token = invite_token(link)?;
        let mut inner = self.inner.lock().await;
        let key_package = inner.mls.key_package()?;
        self.persist(&inner);
        let requested = self.relay.request_join(&token, key_package).await?;
        if !inner.state.pending_joins.contains(&requested.room_id) {
            inner.state.pending_joins.push(requested.room_id.clone());
        }
        self.persist(&inner);
        Ok(JoinPending { room_id: requested.room_id, request_id: requested.request_id })
    }

    /// Join requests waiting in a room this device administers.
    pub async fn pending_requests(&self, room_id: &str) -> Result<Vec<PendingJoin>, SdkError> {
        let pending = self.relay.pending(room_id).await?;
        Ok(pending
            .into_iter()
            .map(|request| PendingJoin {
                room_id: room_id.to_string(),
                request_id: request.request_id,
                device_id: request.device_id,
                name: request.name,
                kind: request.kind,
            })
            .collect())
    }

    /// Adds the requester with an MLS commit, then shares recent history with them.
    pub async fn approve(&self, room_id: &str, request_id: &str) -> Result<(), SdkError> {
        let request = self
            .relay
            .pending(room_id)
            .await?
            .into_iter()
            .find(|request| request.request_id == request_id)
            .ok_or_else(|| SdkError::Invalid(format!("request {request_id} is not pending")))?;
        let mut attempt = 1;
        loop {
            match self.commit_add(room_id, &request).await {
                Err(error) if error.is_stale_epoch() && attempt < COMMIT_ATTEMPTS => {
                    attempt += 1;
                    self.sync().await?;
                }
                other => break other?,
            }
        }
        self.share_history(room_id).await
    }

    /// Adds the requester's key package, which must carry the requester's device id and, for
    /// a device seen before or an owner, the key known for it.
    async fn commit_add(&self, room_id: &str, request: &PendingRequest) -> Result<(), SdkError> {
        let request_id = request.request_id.as_str();
        let mut inner = self.inner.lock().await;
        let group_id = inner.room(room_id)?.group_id.clone();
        let owner_key =
            inner.state.auto_approve.owners.iter().find(|owner| owner.device_id == request.device_id).map(|owner| owner.public_key.clone());
        let pinned = owner_key.or_else(|| pinned_key(&inner.state.pins, &request.device_id));
        let expected = ExpectedJoiner { device_id: &request.device_id, pinned_key: pinned.as_deref() };
        let (commit, welcome) = inner.mls.add_member(&group_id, &request.key_package, expected)?;
        match self.relay.approve(room_id, request_id, &Approve { commit, welcome }).await {
            Ok(accepted) => {
                inner.mls.merge_pending(&group_id)?;
                let leaves = inner.mls.members(&group_id)?;
                pin_new(&mut inner.state.pins, &leaves);
                let room = inner.room_mut(room_id)?;
                room.last_seq = room.last_seq.max(accepted.seq);
                room.pending.retain(|pending| pending.request_id != request_id);
                self.persist(&inner);
                Ok(())
            }
            Err(error) => {
                inner.mls.clear_pending(&group_id)?;
                Err(error)
            }
        }
    }

    async fn share_history(&self, room_id: &str) -> Result<(), SdkError> {
        let share = {
            let inner = self.inner.lock().await;
            build_share(inner.room(room_id)?, inner.me())
        };
        match share {
            Some(envelope) => self.send_envelope(room_id, &envelope).await.map(|_| ()),
            None => Ok(()),
        }
    }

    pub async fn reject(&self, room_id: &str, request_id: &str) -> Result<(), SdkError> {
        self.relay.reject(room_id, request_id).await?;
        let mut inner = self.inner.lock().await;
        inner.room_mut(room_id)?.pending.retain(|pending| pending.request_id != request_id);
        self.persist(&inner);
        Ok(())
    }

    pub async fn remove_member(&self, room_id: &str, device_id: &str) -> Result<(), SdkError> {
        let mut attempt = 1;
        loop {
            match self.commit_remove(room_id, device_id).await {
                Err(error) if error.is_stale_epoch() && attempt < COMMIT_ATTEMPTS => {
                    attempt += 1;
                    self.sync().await?;
                }
                other => return other,
            }
        }
    }

    async fn commit_remove(&self, room_id: &str, device_id: &str) -> Result<(), SdkError> {
        let mut inner = self.inner.lock().await;
        let group_id = inner.room(room_id)?.group_id.clone();
        let commit = inner.mls.remove_member(&group_id, device_id)?;
        match self.relay.remove(room_id, device_id, &RemoveMember { commit }).await {
            Ok(accepted) => {
                inner.mls.merge_pending(&group_id)?;
                let room = inner.room_mut(room_id)?;
                room.last_seq = room.last_seq.max(accepted.seq);
                reconcile(&mut inner, room_id, &[]);
                self.persist(&inner);
                Ok(())
            }
            Err(error) => {
                inner.mls.clear_pending(&group_id)?;
                Err(error)
            }
        }
    }

    /// Reloads member names from the relay. Who is in the room, who administers it and the
    /// policy of a room this device administers stay what the MLS tree and this device say.
    pub async fn refresh_room(&self, room_id: &str) -> Result<RoomInfo, SdkError> {
        let view = self.relay.room(room_id).await?;
        let info = {
            let mut inner = self.inner.lock().await;
            let room = inner.room_mut(room_id)?;
            if room.group_id != view.group_id {
                return Err(SdkError::Invalid(format!("the relay maps room {room_id} to another MLS group")));
            }
            if !room.admin {
                room.policy = view.policy;
            }
            reconcile(&mut inner, room_id, &members_of(&view));
            let info = inner.room(room_id)?.info();
            self.persist(&inner);
            info
        };
        self.host.on_event(ClientEvent::RoomUpdated { room: info.clone() });
        Ok(info)
    }
}
