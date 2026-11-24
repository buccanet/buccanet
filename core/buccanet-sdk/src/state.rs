//! Everything a device keeps between runs, serialized as one opaque blob.

use std::collections::BTreeMap;

use buccanet_protocol::api::{DeviceKind, RoomPolicy};
use serde::{Deserialize, Serialize};

use crate::error::SdkError;
use crate::events::{ChatMessage, MemberInfo, PendingJoin, RoomInfo};
use crate::mls::{Leaf, StorageSnapshot};
use crate::trust::Pins;

const STATE_VERSION: u32 = 1;
/// Messages kept per room on the device for display and history sharing.
const MESSAGES_KEPT: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub device_id: String,
    pub token: String,
    pub name: String,
    pub kind: DeviceKind,
    pub relay_url: String,
    #[serde(with = "buccanet_protocol::b64")]
    pub public_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub device_id: String,
    /// The owner's signing key: a request with another key is not approved automatically.
    #[serde(with = "buccanet_protocol::b64")]
    pub public_key: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoApprove {
    /// Devices this device always lets in, such as the human who runs an agent.
    #[serde(default)]
    pub owners: Vec<Owner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomState {
    pub room_id: String,
    #[serde(with = "buccanet_protocol::b64")]
    pub group_id: Vec<u8>,
    pub title: Option<String>,
    pub policy: RoomPolicy,
    pub admin: bool,
    pub start_seq: u64,
    pub last_seq: u64,
    pub members: Vec<MemberInfo>,
    pub messages: Vec<ChatMessage>,
    pub pending: Vec<PendingJoin>,
    pub history_imported: bool,
    /// The room admin as the MLS tree shows it: this device for a room it created, else the
    /// member whose Welcome let this device in. Only its commits and history share count.
    #[serde(default)]
    pub admin_id: String,
}

impl RoomState {
    pub fn info(&self) -> RoomInfo {
        RoomInfo {
            room_id: self.room_id.clone(),
            title: self.title.clone(),
            policy: self.policy,
            members: self.members.clone(),
            admin: self.admin,
            last_seq: self.last_seq,
        }
    }

    pub fn member(&self, device_id: &str) -> Option<&MemberInfo> {
        self.members.iter().find(|member| member.device_id == device_id)
    }

    /// Rebuilds the member list from the MLS tree. Names and kinds come from `directory` (what
    /// the relay reports) or the previous list; a leaf nobody reported shows as unverified.
    pub fn reconcile_members(&mut self, leaves: &[Leaf], directory: &[MemberInfo]) {
        let members: Vec<MemberInfo> = leaves
            .iter()
            .map(|leaf| {
                let known = directory.iter().chain(&self.members).find(|member| member.device_id == leaf.device_id);
                let mut member = known.cloned().unwrap_or_else(|| MemberInfo::unverified(&leaf.device_id));
                member.admin = leaf.device_id == self.admin_id;
                member
            })
            .collect();
        self.members = members;
    }

    /// Inserts in seq order, ignoring a seq already present.
    pub fn store_message(&mut self, message: ChatMessage) {
        if self.messages.iter().any(|existing| existing.seq == message.seq) {
            return;
        }
        let at = self.messages.partition_point(|existing| existing.seq < message.seq);
        self.messages.insert(at, message);
        if self.messages.len() > MESSAGES_KEPT {
            self.messages.remove(0);
        }
        self.last_seq = self.last_seq.max(self.messages.last().map_or(0, |last| last.seq));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persisted {
    pub version: u32,
    pub identity: Identity,
    pub cursor: u64,
    pub rooms: BTreeMap<String, RoomState>,
    pub auto_approve: AutoApprove,
    /// Rooms this device asked to join: a Welcome for any other room is refused.
    #[serde(default)]
    pub pending_joins: Vec<String>,
    #[serde(default)]
    pub pins: Pins,
    pub mls: Vec<(String, String)>,
}

impl Persisted {
    pub fn new(identity: Identity) -> Self {
        Self {
            version: STATE_VERSION,
            identity,
            cursor: 0,
            rooms: BTreeMap::new(),
            auto_approve: AutoApprove::default(),
            pending_joins: Vec::new(),
            pins: Pins::new(),
            mls: Vec::new(),
        }
    }

    pub fn encode(&self, snapshot: StorageSnapshot) -> Result<Vec<u8>, SdkError> {
        let mut copy = self.clone();
        copy.mls =
            snapshot.iter().map(|(key, value)| (buccanet_protocol::b64::encode(key), buccanet_protocol::b64::encode(value))).collect();
        serde_json::to_vec(&copy).map_err(|error| SdkError::State(error.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<(Persisted, StorageSnapshot), SdkError> {
        let mut persisted: Persisted = serde_json::from_slice(bytes).map_err(|error| SdkError::State(error.to_string()))?;
        if persisted.version != STATE_VERSION {
            return Err(SdkError::State(format!("state version {} is not supported", persisted.version)));
        }
        let snapshot = std::mem::take(&mut persisted.mls)
            .into_iter()
            .map(|(key, value)| Ok((decode_b64(&key)?, decode_b64(&value)?)))
            .collect::<Result<StorageSnapshot, SdkError>>()?;
        Ok((persisted, snapshot))
    }
}

fn decode_b64(text: &str) -> Result<Vec<u8>, SdkError> {
    buccanet_protocol::b64::decode(text).map_err(|error| SdkError::State(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(seq: u64) -> ChatMessage {
        ChatMessage {
            room_id: "r".into(),
            seq,
            sender_device_id: "d".into(),
            author_name: "Ada".into(),
            author_kind: DeviceKind::Agent,
            text: format!("m{seq}"),
            at: "2026-11-23T00:00:00Z".into(),
            own: false,
            shared: false,
        }
    }

    #[test]
    fn messages_stay_ordered_and_unique() {
        let mut room = RoomState {
            room_id: "r".into(),
            group_id: vec![1],
            title: None,
            policy: RoomPolicy::default(),
            admin: false,
            start_seq: 1,
            last_seq: 0,
            members: vec![],
            messages: vec![],
            pending: vec![],
            history_imported: false,
            admin_id: String::new(),
        };
        for seq in [3, 1, 2, 3] {
            room.store_message(message(seq));
        }
        assert_eq!(room.messages.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(room.last_seq, 3);
    }
}
