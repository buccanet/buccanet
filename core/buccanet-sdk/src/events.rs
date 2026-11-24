//! What a host app sees: rooms, messages and the events that change them.

use buccanet_protocol::api::{DeviceKind, RoomPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfo {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomInfo {
    pub room_id: String,
    pub title: Option<String>,
    pub policy: RoomPolicy,
    pub members: Vec<MemberInfo>,
    /// True when this device is an admin of the room.
    pub admin: bool,
    pub last_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub room_id: String,
    pub seq: u64,
    pub sender_device_id: String,
    pub author_name: String,
    pub author_kind: DeviceKind,
    pub text: String,
    pub at: String,
    /// Sent by this device.
    pub own: bool,
    /// Imported from a history share made by another member.
    pub shared: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingJoin {
    pub room_id: String,
    pub request_id: String,
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientEvent {
    /// A new text from another member. Wake agents on this one only.
    #[serde(rename_all = "camelCase")]
    Message { message: ChatMessage },
    #[serde(rename_all = "camelCase")]
    JoinedRoom { room: RoomInfo },
    #[serde(rename_all = "camelCase")]
    RoomUpdated { room: RoomInfo },
    #[serde(rename_all = "camelCase")]
    JoinRequested { request: PendingJoin },
    #[serde(rename_all = "camelCase")]
    JoinRejected { room_id: String, request_id: String },
    #[serde(rename_all = "camelCase")]
    LeftRoom { room_id: String },
    #[serde(rename_all = "camelCase")]
    HistoryImported { room_id: String, count: u32, shared_by: String },
    #[serde(rename_all = "camelCase")]
    Connection { live: bool },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}
