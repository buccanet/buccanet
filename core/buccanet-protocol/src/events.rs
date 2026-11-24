//! Events of a device inbox, delivered over `GET /api/v1/stream` and `GET /api/v1/inbox/:after`.

use serde::{Deserialize, Serialize};

use crate::api::{DeviceKind, RoomPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Commit,
    Application,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxEvent {
    /// Position in this device's inbox; the SSE `id`.
    pub cursor: u64,
    #[serde(flatten)]
    pub body: InboxBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InboxBody {
    #[serde(rename = "message", rename_all = "camelCase")]
    Message {
        room_id: String,
        seq: u64,
        epoch: u64,
        kind: MessageKind,
        sender_device_id: String,
        sender_kind: DeviceKind,
        #[serde(with = "crate::b64")]
        mls_message: Vec<u8>,
    },
    #[serde(rename = "welcome", rename_all = "camelCase")]
    Welcome {
        room_id: String,
        policy: RoomPolicy,
        /// The first room seq this member processes; the Add commit is already applied.
        start_seq: u64,
        #[serde(with = "crate::b64")]
        welcome: Vec<u8>,
    },
    #[serde(rename = "join.requested", rename_all = "camelCase")]
    JoinRequested {
        room_id: String,
        request_id: String,
        device_id: String,
        name: String,
        kind: DeviceKind,
        #[serde(with = "crate::b64")]
        key_package: Vec<u8>,
    },
    #[serde(rename = "join.rejected", rename_all = "camelCase")]
    JoinRejected { room_id: String, request_id: String },
    #[serde(rename = "member.joined", rename_all = "camelCase")]
    MemberJoined { room_id: String, device_id: String, name: String, kind: DeviceKind },
    #[serde(rename = "member.removed", rename_all = "camelCase")]
    MemberRemoved { room_id: String, device_id: String },
    /// To the room admins: a member left, and its leaf waits for their Remove commit.
    #[serde(rename = "member.leaving", rename_all = "camelCase")]
    MemberLeaving { room_id: String, device_id: String },
    /// An event type this version does not know; clients skip it.
    #[serde(other)]
    Unknown,
}
