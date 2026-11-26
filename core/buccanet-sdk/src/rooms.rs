//! Creating rooms, invites, joining, and the admin side of membership.

use buccanet_protocol::api::{Approve, CreateRoom, InvitePreview, JoinRequested, PendingRequest, RemoveMember, RoomPolicy, RoomView};
use buccanet_protocol::envelope::Envelope;
use rand::RngCore;
use std::future::Future;

use crate::client::{Client, JoinPending};
use crate::error::SdkError;
use crate::events::{ClientEvent, MemberInfo, PendingJoin, RoomInfo};
use crate::history::build_share;
use crate::mls::ExpectedJoiner;
use crate::processing::reconcile;
use crate::relay::invite_token;
use crate::rules;
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
                listed: false,
                rules: None,
                admin_id: inner.state.identity.device_id.clone(),
                admins: vec![inner.state.identity.device_id.clone()],
                sealed: 0,
            };
            let info = room.info();
            inner.state.rooms.insert(view.room_id, room);
            self.persist(&inner);
            info
        };
        if let Some(title) = &title {
            self.send_envelope(&info.room_id, &Envelope::RoomMeta { title: title.clone() }).await?;
        }
        self.set_room_rules(&info.room_id, &rules::default_rules(title.as_deref()).text).await
    }

    /// Gives the default rules to the rooms this device administers that have none, such as
    /// rooms created before rooms had rules.
    pub(crate) async fn adopt_default_rules(&self) {
        let missing: Vec<(String, Option<String>)> = {
            let inner = self.inner.lock().await;
            inner
                .state
                .rooms
                .values()
                .filter(|room| room.admin && room.rules.is_none())
                .map(|room| (room.room_id.clone(), room.title.clone()))
                .collect()
        };
        for (room_id, title) in missing {
            if let Err(error) = self.set_room_rules(&room_id, &rules::default_rules(title.as_deref()).text).await {
                self.host.on_event(ClientEvent::Error { message: format!("default rules for room {room_id}: {error}") });
            }
        }
    }

    /// Replaces the rules of a room this device administers; every member receives them.
    pub async fn set_room_rules(&self, room_id: &str, text: &str) -> Result<RoomInfo, SdkError> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(SdkError::Invalid("room rules need some text".into()));
        }
        let rules = {
            let inner = self.inner.lock().await;
            let room = inner.room(room_id)?;
            if !room.admin {
                return Err(SdkError::Invalid("only a room admin can change its rules".into()));
            }
            rules::next(room.rules.as_ref(), text)
        };
        self.send_envelope(room_id, &Envelope::RoomRules(rules.clone())).await?;
        let info = {
            let mut inner = self.inner.lock().await;
            inner.room_mut(room_id)?.rules = Some(rules);
            let info = inner.room(room_id)?.info();
            self.persist(&inner);
            info
        };
        self.host.on_event(ClientEvent::RoomUpdated { room: info.clone() });
        Ok(info)
    }

    /// Leaves a room: the relay stops counting this device in, a room admin removes it from the
    /// group, and this device forgets the room and its keys at once. The last admin cannot leave.
    pub async fn leave_room(&self, room_id: &str) -> Result<(), SdkError> {
        let last_admin = {
            let inner = self.inner.lock().await;
            let room = inner.room(room_id)?;
            room.admin && room.admins.len() == 1
        };
        if last_admin {
            return Err(SdkError::Invalid("you are the only admin of this room: make another member admin before leaving".into()));
        }
        self.relay.leave(room_id).await?;
        self.forget_room(room_id).await
    }

    /// Drops a room from this device alone: its keys, messages and members go, and later events
    /// of the room are ignored, while the relay still counts the device in. It serves a device
    /// that stops tending a room it is the only admin of, and so cannot leave, such as the
    /// admission service of a public room after it moves its listing.
    pub async fn forget_room(&self, room_id: &str) -> Result<(), SdkError> {
        let mut inner = self.inner.lock().await;
        let room = inner.state.rooms.remove(room_id).ok_or_else(|| SdkError::UnknownRoom(room_id.into()))?;
        inner.mls.delete_group(&room.group_id)?;
        inner.forget_departed();
        self.persist(&inner);
        drop(inner);
        self.host.on_event(ClientEvent::LeftRoom { room_id: room_id.into() });
        Ok(())
    }

    /// Renames a room this device administers. Members take a title from a room admin only.
    pub async fn rename_room(&self, room_id: &str, title: &str) -> Result<RoomInfo, SdkError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(SdkError::Invalid("a room needs a title".into()));
        }
        if !self.inner.lock().await.room(room_id)?.admin {
            return Err(SdkError::Invalid("only a room admin can rename it".into()));
        }
        self.send_envelope(room_id, &Envelope::RoomMeta { title: title.clone() }).await?;
        let info = {
            let mut inner = self.inner.lock().await;
            inner.room_mut(room_id)?.title = Some(title);
            let info = inner.room(room_id)?.info();
            self.persist(&inner);
            info
        };
        self.host.on_event(ClientEvent::RoomUpdated { room: info.clone() });
        Ok(info)
    }

    /// A reusable invite link, valid seven days, every join still subject to approval.
    pub async fn create_invite(&self, room_id: &str) -> Result<String, SdkError> {
        Ok(self.relay.create_invite(room_id).await?.url)
    }

    pub async fn preview_invite(&self, link: &str) -> Result<InvitePreview, SdkError> {
        self.relay.preview(&invite_token(link)?).await
    }

    /// Asks to join from an invite link; the Welcome arrives in the inbox once an admin approves.
    pub async fn join(&self, link: &str) -> Result<JoinPending, SdkError> {
        let token = invite_token(link)?;
        self.request_join(|key_package| self.relay.request_join(&token, key_package)).await
    }

    /// Sends a fresh key package with `request` and records the room as one this device asked to
    /// join. The state stays locked until the request is recorded, so an immediate Welcome finds it.
    pub(crate) async fn request_join<F, Fut>(&self, request: F) -> Result<JoinPending, SdkError>
    where
        F: FnOnce(Vec<u8>) -> Fut,
        Fut: Future<Output = Result<JoinRequested, SdkError>>,
    {
        let mut inner = self.inner.lock().await;
        let key_package = inner.mls.key_package()?;
        self.persist(&inner);
        let requested = request(key_package).await?;
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
        self.retrying(|| self.commit_add(room_id, &request)).await?;
        self.share_history(room_id).await
    }

    /// Runs a commit again, after a sync, while another admin moved the room to a newer epoch.
    pub(crate) async fn retrying<F, Fut>(&self, mut commit: F) -> Result<(), SdkError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<(), SdkError>>,
    {
        let mut attempt = 1;
        loop {
            match commit().await {
                Err(error) if error.is_stale_epoch() && attempt < COMMIT_ATTEMPTS => {
                    attempt += 1;
                    self.sync().await?;
                }
                other => return other,
            }
        }
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
        self.retrying(|| self.commit_remove(room_id, device_id)).await
    }

    /// Removes a member; an admin leaves the admin list in the same commit.
    async fn commit_remove(&self, room_id: &str, device_id: &str) -> Result<(), SdkError> {
        let mut inner = self.inner.lock().await;
        let room = inner.room(room_id)?;
        let group_id = room.group_id.clone();
        let admins: Option<Vec<String>> =
            room.is_admin(device_id).then(|| room.admins.iter().filter(|admin| *admin != device_id).cloned().collect());
        let commit = inner.mls.remove_member(&group_id, device_id, admins.as_deref())?;
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
            room.listed = view.listed;
            reconcile(&mut inner, room_id, &members_of(&view));
            let info = inner.room(room_id)?.info();
            self.persist(&inner);
            info
        };
        self.host.on_event(ClientEvent::RoomUpdated { room: info.clone() });
        Ok(info)
    }
}
