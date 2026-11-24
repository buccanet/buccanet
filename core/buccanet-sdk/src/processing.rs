//! Applies one inbox event to the device state. Pure with respect to the network: what
//! needs the relay afterwards is returned as an [`Action`].

use buccanet_protocol::envelope::Envelope;
use buccanet_protocol::events::{InboxBody, InboxEvent};

use crate::client::Inner;
use crate::events::{ChatMessage, ClientEvent, MemberInfo, PendingJoin};
use crate::mls::Incoming;
use crate::state::RoomState;

#[derive(Debug, PartialEq)]
pub enum Action {
    Approve { room_id: String, request_id: String },
    RefreshRoom { room_id: String },
}

#[derive(Default)]
pub struct Outcome {
    pub events: Vec<ClientEvent>,
    pub actions: Vec<Action>,
}

impl Outcome {
    fn event(event: ClientEvent) -> Self {
        Self { events: vec![event], actions: Vec::new() }
    }
}

pub fn apply(inner: &mut Inner, event: InboxEvent) -> Outcome {
    match event.body {
        InboxBody::Message { room_id, seq, mls_message, .. } => on_message(inner, &room_id, seq, &mls_message),
        InboxBody::Welcome { room_id, policy, start_seq, welcome } => on_welcome(inner, room_id, policy, start_seq, &welcome),
        InboxBody::JoinRequested { room_id, request_id, device_id, name, kind, .. } => {
            on_join_requested(inner, PendingJoin { room_id, request_id, device_id, name, kind })
        }
        InboxBody::JoinRejected { room_id, request_id } => Outcome::event(ClientEvent::JoinRejected { room_id, request_id }),
        InboxBody::MemberJoined { room_id, device_id, name, kind } => {
            on_member_joined(inner, &room_id, MemberInfo { device_id, name, kind, admin: false })
        }
        InboxBody::MemberRemoved { room_id, device_id } => on_member_removed(inner, &room_id, &device_id),
    }
}

fn on_message(inner: &mut Inner, room_id: &str, seq: u64, bytes: &[u8]) -> Outcome {
    let Some(room) = inner.state.rooms.get(room_id) else { return Outcome::default() };
    if seq < room.start_seq {
        return Outcome::default();
    }
    let group_id = room.group_id.clone();
    let incoming = match inner.mls.process(&group_id, bytes) {
        Ok(incoming) => incoming,
        Err(error) => return Outcome::event(ClientEvent::Error { message: format!("room {room_id} seq {seq}: {error}") }),
    };
    if let Some(room) = inner.state.rooms.get_mut(room_id) {
        room.last_seq = room.last_seq.max(seq);
    }
    match incoming {
        Incoming::Application { sender, plaintext } => on_envelope(inner, room_id, seq, sender, &plaintext),
        Incoming::Commit { self_removed: true } => left(inner, room_id),
        Incoming::Commit { self_removed: false } => Outcome { events: Vec::new(), actions: vec![Action::RefreshRoom { room_id: room_id.into() }] },
        Incoming::Ignored => Outcome::default(),
    }
}

fn on_envelope(inner: &mut Inner, room_id: &str, seq: u64, sender: String, plaintext: &[u8]) -> Outcome {
    let Ok(envelope) = Envelope::decode(plaintext) else { return Outcome::default() };
    let me = inner.state.identity.device_id.clone();
    let Some(room) = inner.state.rooms.get_mut(room_id) else { return Outcome::default() };
    match envelope {
        Envelope::Text { author, text, at } => {
            let message = ChatMessage {
                room_id: room_id.into(),
                seq,
                sender_device_id: sender,
                author_name: author.name,
                author_kind: author.kind,
                text,
                at,
                own: false,
                shared: false,
            };
            room.store_message(message.clone());
            Outcome::event(ClientEvent::Message { message })
        }
        Envelope::RoomMeta { title } => {
            room.title = Some(title);
            Outcome::event(ClientEvent::RoomUpdated { room: room.info() })
        }
        Envelope::HistoryShare { shared_by, title, messages } if sender != me && !room.history_imported => {
            import_history(room, &shared_by.name, title, messages)
        }
        Envelope::HistoryShare { .. } | Envelope::Unknown { .. } => Outcome::default(),
    }
}

fn import_history(
    room: &mut RoomState,
    shared_by: &str,
    title: Option<String>,
    messages: Vec<buccanet_protocol::envelope::SharedMessage>,
) -> Outcome {
    let before_join: Vec<_> = messages.into_iter().filter(|message| message.seq < room.start_seq).collect();
    let count = before_join.len() as u32;
    for shared in before_join {
        room.store_message(ChatMessage {
            room_id: room.room_id.clone(),
            seq: shared.seq,
            sender_device_id: shared.sender_device_id,
            author_name: shared.author.name,
            author_kind: shared.author.kind,
            text: shared.text,
            at: shared.at,
            own: false,
            shared: true,
        });
    }
    room.history_imported = true;
    if room.title.is_none() {
        room.title = title;
    }
    let events = vec![
        ClientEvent::HistoryImported { room_id: room.room_id.clone(), count, shared_by: shared_by.to_string() },
        ClientEvent::RoomUpdated { room: room.info() },
    ];
    Outcome { events, actions: Vec::new() }
}

fn on_welcome(inner: &mut Inner, room_id: String, policy: buccanet_protocol::api::RoomPolicy, start_seq: u64, welcome: &[u8]) -> Outcome {
    if inner.state.rooms.contains_key(&room_id) {
        return Outcome::default();
    }
    let group_id = match inner.mls.join(welcome) {
        Ok(group_id) => group_id,
        Err(error) => return Outcome::event(ClientEvent::Error { message: format!("joining room {room_id}: {error}") }),
    };
    let room = RoomState {
        room_id: room_id.clone(),
        group_id,
        title: None,
        policy,
        admin: false,
        start_seq,
        last_seq: start_seq.saturating_sub(1),
        members: Vec::new(),
        messages: Vec::new(),
        pending: Vec::new(),
        history_imported: false,
    };
    let info = room.info();
    inner.state.rooms.insert(room_id.clone(), room);
    Outcome { events: vec![ClientEvent::JoinedRoom { room: info }], actions: vec![Action::RefreshRoom { room_id }] }
}

fn on_join_requested(inner: &mut Inner, request: PendingJoin) -> Outcome {
    let owners = inner.state.auto_approve.owners.clone();
    let Some(room) = inner.state.rooms.get_mut(&request.room_id) else { return Outcome::default() };
    if !room.admin {
        return Outcome::default();
    }
    room.pending.retain(|pending| pending.request_id != request.request_id);
    room.pending.push(request.clone());
    let automatic = !room.policy.approval || owners.contains(&request.device_id);
    let actions = if automatic {
        vec![Action::Approve { room_id: request.room_id.clone(), request_id: request.request_id.clone() }]
    } else {
        Vec::new()
    };
    Outcome { events: vec![ClientEvent::JoinRequested { request }], actions }
}

fn on_member_joined(inner: &mut Inner, room_id: &str, member: MemberInfo) -> Outcome {
    let Some(room) = inner.state.rooms.get_mut(room_id) else { return Outcome::default() };
    room.pending.retain(|pending| pending.device_id != member.device_id);
    if !room.members.iter().any(|existing| existing.device_id == member.device_id) {
        room.members.push(member);
    }
    Outcome::event(ClientEvent::RoomUpdated { room: room.info() })
}

fn on_member_removed(inner: &mut Inner, room_id: &str, device_id: &str) -> Outcome {
    if device_id == inner.state.identity.device_id {
        return left(inner, room_id);
    }
    let Some(room) = inner.state.rooms.get_mut(room_id) else { return Outcome::default() };
    room.members.retain(|member| member.device_id != device_id);
    Outcome::event(ClientEvent::RoomUpdated { room: room.info() })
}

fn left(inner: &mut Inner, room_id: &str) -> Outcome {
    match inner.state.rooms.remove(room_id) {
        Some(_) => Outcome::event(ClientEvent::LeftRoom { room_id: room_id.into() }),
        None => Outcome::default(),
    }
}
