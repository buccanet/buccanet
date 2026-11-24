//! The history share a member sends after letting someone in (WhatsApp's model).

use buccanet_protocol::envelope::{Author, Envelope, HISTORY_SHARE_MAX_BYTES, HISTORY_SHARE_MAX_MESSAGES, SharedMessage};

use crate::state::RoomState;

/// The latest messages that fit the size cap, oldest first. Shared messages are not re-shared.
pub fn build_share(room: &RoomState, shared_by: Author) -> Option<Envelope> {
    let mut messages: Vec<SharedMessage> = room
        .messages
        .iter()
        .filter(|message| !message.shared)
        .rev()
        .take(HISTORY_SHARE_MAX_MESSAGES)
        .map(|message| SharedMessage {
            seq: message.seq,
            sender_device_id: message.sender_device_id.clone(),
            author: Author { name: message.author_name.clone(), kind: message.author_kind },
            text: message.text.clone(),
            at: message.at.clone(),
        })
        .collect();
    messages.reverse();
    loop {
        let envelope = Envelope::HistoryShare { shared_by: shared_by.clone(), title: room.title.clone(), messages: messages.clone() };
        let size = envelope.encode().map(|bytes| bytes.len()).unwrap_or(usize::MAX);
        if size <= HISTORY_SHARE_MAX_BYTES {
            return (!messages.is_empty() || room.title.is_some()).then_some(envelope);
        }
        if messages.is_empty() {
            return None;
        }
        messages.remove(0);
    }
}
