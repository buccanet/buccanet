//! How events read in a terminal.

use buccanet_sdk::{ClientEvent, DeviceKind};

pub fn kind_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Human => "human",
        DeviceKind::Agent => "agent",
    }
}

pub fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

pub fn describe(event: &ClientEvent) -> Option<String> {
    Some(match event {
        ClientEvent::Message { message } => {
            format!("[{}] #{} {} ({}): {}", short(&message.room_id), message.seq, message.author_name, kind_label(message.author_kind), message.text)
        }
        ClientEvent::JoinedRoom { room } => format!("[{}] joined", short(&room.room_id)),
        ClientEvent::JoinRequested { request } => format!(
            "[{}] join request {} from {} ({})",
            short(&request.room_id),
            request.request_id,
            request.name,
            kind_label(request.kind)
        ),
        ClientEvent::JoinRejected { room_id, .. } => format!("[{}] join request rejected", short(room_id)),
        ClientEvent::LeftRoom { room_id } => format!("[{}] removed from the room", short(room_id)),
        ClientEvent::HistoryImported { room_id, count, shared_by } => {
            format!("[{}] {count} earlier messages shared by {shared_by}", short(room_id))
        }
        ClientEvent::Error { message } => format!("error: {message}"),
        ClientEvent::RoomUpdated { .. } | ClientEvent::Connection { .. } => return None,
    })
}
