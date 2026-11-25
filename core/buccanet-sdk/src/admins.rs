//! Room admins: an admin names another member admin, or takes the role back, through an MLS
//! commit that replaces the group's admin list.

use buccanet_protocol::api::SetAdmins;

use crate::client::Client;
use crate::error::SdkError;
use crate::events::{ClientEvent, RoomInfo};
use crate::processing::reconcile;
use crate::state::RoomState;

impl Client {
    /// Makes a member an admin of a room this device administers, or takes the role back. A
    /// room keeps at least one admin, and each member must run a Buccanet that knows admin lists.
    pub async fn set_admin(&self, room_id: &str, device_id: &str, admin: bool) -> Result<RoomInfo, SdkError> {
        self.retrying(|| self.commit_admins(room_id, device_id, admin)).await?;
        let info = self.room(room_id).await?;
        self.host.on_event(ClientEvent::RoomUpdated { room: info.clone() });
        Ok(info)
    }

    async fn commit_admins(&self, room_id: &str, device_id: &str, admin: bool) -> Result<(), SdkError> {
        let mut inner = self.inner.lock().await;
        let room = inner.room(room_id)?;
        let group_id = room.group_id.clone();
        let admins = next_admins(room, device_id, admin)?;
        let outdated = inner.mls.without_admin_support(&group_id)?;
        if !outdated.is_empty() {
            return Err(outdated_error(room, &outdated, inner.state.identity.device_id.as_str()));
        }
        let commit = inner.mls.set_admins(&group_id, &admins)?;
        match self.relay.set_admins(room_id, &SetAdmins { commit, admins }).await {
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
}

/// The admin list once `device_id` gains or loses the role.
fn next_admins(room: &RoomState, device_id: &str, admin: bool) -> Result<Vec<String>, SdkError> {
    if !room.admin {
        return Err(SdkError::Invalid("only a room admin can change who administers it".into()));
    }
    let Some(member) = room.member(device_id) else {
        return Err(SdkError::Invalid(format!("{device_id} is not a member of this room")));
    };
    if room.is_admin(device_id) == admin {
        let state = if admin { "already" } else { "not" };
        return Err(SdkError::Invalid(format!("{} is {state} an admin", member.name)));
    }
    let mut admins = room.admins.clone();
    if admin {
        admins.push(device_id.to_string());
    } else {
        admins.retain(|existing| existing != device_id);
    }
    if admins.is_empty() {
        return Err(SdkError::Invalid("a room keeps at least one admin".into()));
    }
    Ok(admins)
}

/// Why a room whose members include devices older than admin lists cannot get a second admin.
fn outdated_error(room: &RoomState, outdated: &[String], me: &str) -> SdkError {
    if outdated.iter().any(|device_id| device_id == me) {
        return SdkError::Invalid(
            "this room was created before Buccanet had several admins per room: create a new room to name other admins".into(),
        );
    }
    let names: Vec<String> =
        outdated.iter().map(|device_id| room.member(device_id).map_or_else(|| device_id.clone(), |member| member.name.clone())).collect();
    SdkError::Invalid(format!("{} must update Buccanet and join again before this room can have several admins", names.join(", ")))
}

#[cfg(test)]
mod tests {
    use buccanet_protocol::api::{DeviceKind, RoomPolicy};

    use super::*;
    use crate::events::MemberInfo;

    fn room(admins: &[&str]) -> RoomState {
        let member = |id: &str| MemberInfo { device_id: id.into(), name: id.to_uppercase(), kind: DeviceKind::Human, admin: false };
        RoomState {
            room_id: "r".into(),
            group_id: vec![1],
            title: None,
            policy: RoomPolicy::default(),
            admin: admins.contains(&"ada"),
            start_seq: 1,
            last_seq: 0,
            members: vec![member("ada"), member("bob")],
            messages: vec![],
            pending: vec![],
            history_imported: true,
            listed: false,
            rules: None,
            admin_id: "ada".into(),
            admins: admins.iter().map(|id| id.to_string()).collect(),
            sealed: 0,
        }
    }

    #[test]
    fn an_admin_names_and_drops_admins_but_keeps_one() {
        assert_eq!(next_admins(&room(&["ada"]), "bob", true).unwrap(), vec!["ada", "bob"]);
        assert_eq!(next_admins(&room(&["ada", "bob"]), "ada", false).unwrap(), vec!["bob"]);
        assert!(next_admins(&room(&["ada"]), "ada", false).is_err());
        assert!(next_admins(&room(&["ada", "bob"]), "bob", true).is_err());
        assert!(next_admins(&room(&["ada"]), "zed", true).is_err());
        assert!(next_admins(&room(&["bob"]), "ada", true).is_err());
    }
}
