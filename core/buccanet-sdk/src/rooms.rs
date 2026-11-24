//! Creating rooms, invites, joining, and the admin side of membership.

use buccanet_protocol::api::{Approve, CreateRoom, InvitePreview, RemoveMember, RoomPolicy, RoomView};
use buccanet_protocol::envelope::Envelope;
use rand::RngCore;

use crate::client::{Client, JoinPending};
use crate::error::SdkError;
use crate::events::{ClientEvent, MemberInfo, PendingJoin, RoomInfo};
use crate::history::build_share;
use crate::relay::invite_token;
use crate::state::RoomState;

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

    /// Asks to join; the Welcome arrives in the inbox once an admin approves.
    pub async fn join(&self, link: &str) -> Result<JoinPending, SdkError> {
        let token = invite_token(link)?;
        let key_package = {
            let inner = self.inner.lock().await;
            let key_package = inner.mls.key_package()?;
            self.persist(&inner);
            key_package
        };
        let requested = self.relay.request_join(&token, key_package).await?;
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
            match self.commit_add(room_id, request_id, &request.key_package).await {
                Err(error) if error.is_stale_epoch() && attempt < COMMIT_ATTEMPTS => {
                    attempt += 1;
                    self.sync().await?;
                }
                other => break other?,
            }
        }
        self.share_history(room_id).await
    }

    async fn commit_add(&self, room_id: &str, request_id: &str, key_package: &[u8]) -> Result<(), SdkError> {
        let mut inner = self.inner.lock().await;
        let group_id = inner.room(room_id)?.group_id.clone();
        let (commit, welcome) = inner.mls.add_member(&group_id, key_package)?;
        match self.relay.approve(room_id, request_id, &Approve { commit, welcome }).await {
            Ok(accepted) => {
                inner.mls.merge_pending(&group_id)?;
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
                room.members.retain(|member| member.device_id != device_id);
                self.persist(&inner);
                Ok(())
            }
            Err(error) => {
                inner.mls.clear_pending(&group_id)?;
                Err(error)
            }
        }
    }

    /// Reloads member names and roles from the relay.
    pub async fn refresh_room(&self, room_id: &str) -> Result<RoomInfo, SdkError> {
        let view = self.relay.room(room_id).await?;
        let info = {
            let mut inner = self.inner.lock().await;
            let me = inner.state.identity.device_id.clone();
            let room = inner.room_mut(room_id)?;
            room.members = members_of(&view);
            room.admin = view.members.iter().any(|member| member.device_id == me && member.admin);
            room.policy = view.policy;
            let info = room.info();
            self.persist(&inner);
            info
        };
        self.host.on_event(ClientEvent::RoomUpdated { room: info.clone() });
        Ok(info)
    }
}
