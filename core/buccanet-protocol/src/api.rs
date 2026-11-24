//! Request and response bodies of the relay HTTP API v1.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDevice {
    /// Ed25519 public key that signs this device's MLS messages.
    #[serde(with = "crate::b64")]
    pub public_key: Vec<u8>,
    pub kind: DeviceKind,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistered {
    pub device_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomPolicy {
    /// Joins wait for an admin decision. When false, admin clients approve automatically.
    pub approval: bool,
    pub max_members: u32,
    /// Agent application messages accepted in a row before a human must speak.
    pub agent_burst: u32,
}

impl Default for RoomPolicy {
    fn default() -> Self {
        Self { approval: true, max_members: 32, agent_burst: 40 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoom {
    #[serde(with = "crate::b64")]
    pub group_id: Vec<u8>,
    pub policy: RoomPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    pub room_id: String,
    #[serde(with = "crate::b64")]
    pub group_id: Vec<u8>,
    pub policy: RoomPolicy,
    pub epoch: u64,
    pub last_seq: u64,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreated {
    pub invite_id: String,
    pub token: String,
    /// `<public origin>/join/<token>`: what people paste to an agent or open in a browser.
    pub url: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitePreview {
    pub room_id: String,
    pub member_count: u32,
    pub approval: bool,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    pub request_id: String,
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    #[serde(with = "crate::b64")]
    pub key_package: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestJoin {
    #[serde(with = "crate::b64")]
    pub key_package: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequested {
    pub request_id: String,
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approve {
    #[serde(with = "crate::b64")]
    pub commit: Vec<u8>,
    #[serde(with = "crate::b64")]
    pub welcome: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveMember {
    #[serde(with = "crate::b64")]
    pub commit: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostMessage {
    /// Chosen by the client so a retried POST never stores the message twice.
    pub client_message_id: String,
    #[serde(with = "crate::b64")]
    pub mls_message: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    pub seq: u64,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxPage {
    pub events: Vec<crate::events::InboxEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

/// Stable error codes a client can branch on.
pub mod codes {
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const FORBIDDEN: &str = "forbidden";
    pub const NOT_FOUND: &str = "not_found";
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const STALE_EPOCH: &str = "stale_epoch";
    pub const ROOM_FULL: &str = "room_full";
    pub const AGENT_BURST: &str = "agent_burst";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const INVITE_EXPIRED: &str = "invite_expired";
    pub const ALREADY_MEMBER: &str = "already_member";
    pub const INTERNAL: &str = "internal";
}
