//! How events read in a terminal. Names, titles and message text pass through `text` first.

use buccanet_sdk::{ClientEvent, DeviceKind};

use buccanet_agent::text;

pub fn kind_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Human => "human",
        DeviceKind::Agent => "agent",
    }
}

pub fn short(id: &str) -> String {
    text::short(id, 8)
}

pub fn describe(event: &ClientEvent) -> Option<String> {
    Some(match event {
        ClientEvent::Message { message } => format!(
            "[{}] #{} {} ({}): {}",
            short(&message.room_id),
            message.seq,
            text::line(&message.author_name),
            kind_label(message.author_kind),
            text::indented(&message.text)
        ),
        ClientEvent::JoinedRoom { room } => format!("[{}] joined", short(&room.room_id)),
        ClientEvent::JoinRequested { request } => format!(
            "[{}] join request {} from {} ({})",
            short(&request.room_id),
            text::line(&request.request_id),
            text::line(&request.name),
            kind_label(request.kind)
        ),
        ClientEvent::JoinRejected { room_id, .. } => format!("[{}] join request rejected", short(room_id)),
        ClientEvent::LeftRoom { room_id } => format!("[{}] removed from the room", short(room_id)),
        ClientEvent::HistoryImported { room_id, count, shared_by } => {
            format!("[{}] {count} earlier messages shared by {}", short(room_id), text::line(shared_by))
        }
        ClientEvent::Error { message } => format!("error: {}", text::line(message)),
        ClientEvent::RoomUpdated { .. } | ClientEvent::Connection { .. } => return None,
    })
}
