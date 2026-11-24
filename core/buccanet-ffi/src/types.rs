//! Records and enums as foreign languages see them.

use buccanet_sdk as sdk;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceKind {
    Human,
    Agent,
}

impl From<sdk::DeviceKind> for DeviceKind {
    fn from(kind: sdk::DeviceKind) -> Self {
        match kind {
            sdk::DeviceKind::Human => DeviceKind::Human,
            sdk::DeviceKind::Agent => DeviceKind::Agent,
        }
    }
}

impl From<DeviceKind> for sdk::DeviceKind {
    fn from(kind: DeviceKind) -> Self {
        match kind {
            DeviceKind::Human => sdk::DeviceKind::Human,
            DeviceKind::Agent => sdk::DeviceKind::Agent,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct RoomPolicy {
    pub approval: bool,
    pub max_members: u32,
    pub agent_burst: u32,
}

impl From<sdk::RoomPolicy> for RoomPolicy {
    fn from(policy: sdk::RoomPolicy) -> Self {
        Self { approval: policy.approval, max_members: policy.max_members, agent_burst: policy.agent_burst }
    }
}

impl From<RoomPolicy> for sdk::RoomPolicy {
    fn from(policy: RoomPolicy) -> Self {
        Self { approval: policy.approval, max_members: policy.max_members, agent_burst: policy.agent_burst }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct MemberInfo {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub admin: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct RoomInfo {
    pub room_id: String,
    pub title: Option<String>,
    pub policy: RoomPolicy,
    pub members: Vec<MemberInfo>,
    pub admin: bool,
    pub last_seq: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatMessage {
    pub room_id: String,
    pub seq: u64,
    pub sender_device_id: String,
    pub author_name: String,
    pub author_kind: DeviceKind,
    pub text: String,
    pub at: String,
    pub own: bool,
    pub shared: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PendingJoin {
    pub room_id: String,
    pub request_id: String,
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct JoinPending {
    pub room_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum ClientEvent {
    Message { message: ChatMessage },
    JoinedRoom { room: RoomInfo },
    RoomUpdated { room: RoomInfo },
    JoinRequested { request: PendingJoin },
    JoinRejected { room_id: String, request_id: String },
    LeftRoom { room_id: String },
    HistoryImported { room_id: String, count: u32, shared_by: String },
    Connection { live: bool },
    Error { message: String },
}

impl From<sdk::MemberInfo> for MemberInfo {
    fn from(m: sdk::MemberInfo) -> Self {
        Self { device_id: m.device_id, name: m.name, kind: m.kind.into(), admin: m.admin }
    }
}

impl From<sdk::RoomInfo> for RoomInfo {
    fn from(r: sdk::RoomInfo) -> Self {
        Self {
            room_id: r.room_id,
            title: r.title,
            policy: r.policy.into(),
            members: r.members.into_iter().map(Into::into).collect(),
            admin: r.admin,
            last_seq: r.last_seq,
        }
    }
}

impl From<sdk::ChatMessage> for ChatMessage {
    fn from(m: sdk::ChatMessage) -> Self {
        Self {
            room_id: m.room_id,
            seq: m.seq,
            sender_device_id: m.sender_device_id,
            author_name: m.author_name,
            author_kind: m.author_kind.into(),
            text: m.text,
            at: m.at,
            own: m.own,
            shared: m.shared,
        }
    }
}

impl From<sdk::PendingJoin> for PendingJoin {
    fn from(p: sdk::PendingJoin) -> Self {
        Self { room_id: p.room_id, request_id: p.request_id, device_id: p.device_id, name: p.name, kind: p.kind.into() }
    }
}

impl From<sdk::ClientEvent> for ClientEvent {
    fn from(event: sdk::ClientEvent) -> Self {
        match event {
            sdk::ClientEvent::Message { message } => ClientEvent::Message { message: message.into() },
            sdk::ClientEvent::JoinedRoom { room } => ClientEvent::JoinedRoom { room: room.into() },
            sdk::ClientEvent::RoomUpdated { room } => ClientEvent::RoomUpdated { room: room.into() },
            sdk::ClientEvent::JoinRequested { request } => ClientEvent::JoinRequested { request: request.into() },
            sdk::ClientEvent::JoinRejected { room_id, request_id } => ClientEvent::JoinRejected { room_id, request_id },
            sdk::ClientEvent::LeftRoom { room_id } => ClientEvent::LeftRoom { room_id },
            sdk::ClientEvent::HistoryImported { room_id, count, shared_by } => ClientEvent::HistoryImported { room_id, count, shared_by },
            sdk::ClientEvent::Connection { live } => ClientEvent::Connection { live },
            sdk::ClientEvent::Error { message } => ClientEvent::Error { message },
        }
    }
}
