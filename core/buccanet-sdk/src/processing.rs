//! Applies one inbox event to the device state. Pure with respect to the network: what
//! needs the relay afterwards is returned as an [`Action`].

use buccanet_protocol::api::RoomPolicy;
use buccanet_protocol::envelope::{Envelope, RoomRules, SharedMessage};
use buccanet_protocol::events::{InboxBody, InboxEvent};

use crate::client::Inner;
use crate::events::{ChatMessage, ClientEvent, MemberInfo, PendingJoin};
use crate::mls::{Incoming, Joined, ReviewedCommit};
use crate::rules;
use crate::state::RoomState;
use crate::trust::{Proposed, check_commit, key_change, pin_new};

#[derive(Debug, PartialEq)]
pub enum Action {
    Approve {
        room_id: String,
        request_id: String,
    },
    /// Commits the removal of a member who left, as the room admin.
    Remove {
        room_id: String,
        device_id: String,
    },
    RefreshRoom {
        room_id: String,
    },
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

    fn error(message: String) -> Self {
        Self::event(ClientEvent::Error { message })
    }
}

pub fn apply(inner: &mut Inner, event: InboxEvent) -> Outcome {
    match event.body {
        InboxBody::Message { room_id, seq, mls_message, .. } => on_message(inner, &room_id, seq, &mls_message),
        InboxBody::Welcome { room_id, policy, start_seq, welcome } => on_welcome(inner, Arrival { room_id, policy, start_seq }, &welcome),
        InboxBody::JoinRequested { room_id, request_id, device_id, name, kind, .. } => {
            on_join_requested(inner, PendingJoin { room_id, request_id, device_id, name, kind })
        }
        InboxBody::JoinRejected { room_id, request_id } => Outcome::event(ClientEvent::JoinRejected { room_id, request_id }),
        InboxBody::MemberJoined { room_id, device_id, name, kind } => {
            on_member_joined(inner, &room_id, MemberInfo { device_id, name, kind, admin: false })
        }
        InboxBody::MemberRemoved { room_id, device_id } => on_member_removed(inner, &room_id, &device_id),
        InboxBody::MemberLeaving { room_id, device_id } => on_member_leaving(inner, room_id, device_id),
        InboxBody::Unknown => Outcome::default(),
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
        Err(error) => return Outcome::error(format!("room {room_id} seq {seq}: {error}")),
    };
    if let Some(room) = inner.state.rooms.get_mut(room_id) {
        room.last_seq = room.last_seq.max(seq);
    }
    match incoming {
        Incoming::Application { sender, plaintext } => on_envelope(inner, room_id, seq, sender, &plaintext),
        Incoming::Commit(commit) => on_commit(inner, room_id, commit),
        Incoming::Ignored => Outcome::default(),
    }
}

/// Applies a commit only when the room admin made it and the devices it adds check out.
fn on_commit(inner: &mut Inner, room_id: &str, commit: ReviewedCommit) -> Outcome {
    let Some(room) = inner.state.rooms.get(room_id) else { return Outcome::default() };
    let group_id = room.group_id.clone();
    let checked = inner.mls.members(&group_id).map_err(|error| error.to_string()).and_then(|current| {
        check_commit(Proposed { committer: &commit.committer, added: &commit.added }, &room.admin_id, &current, &inner.state.pins)
    });
    if let Err(reason) = checked {
        return Outcome::error(format!("room {room_id}: commit refused, {reason}"));
    }
    let (self_removed, added) = (commit.self_removed, commit.added.clone());
    if let Err(error) = inner.mls.merge_commit(&group_id, commit) {
        return Outcome::error(format!("room {room_id}: {error}"));
    }
    if self_removed {
        return left(inner, room_id);
    }
    pin_new(&mut inner.state.pins, &added);
    let mut outcome = reconcile(inner, room_id, &[]);
    outcome.actions.push(Action::RefreshRoom { room_id: room_id.into() });
    outcome
}

/// Rebuilds a room's members from its MLS tree, naming them from `directory` when given.
pub fn reconcile(inner: &mut Inner, room_id: &str, directory: &[MemberInfo]) -> Outcome {
    let Some(group_id) = inner.state.rooms.get(room_id).map(|room| room.group_id.clone()) else { return Outcome::default() };
    let leaves = match inner.mls.members(&group_id) {
        Ok(leaves) => leaves,
        Err(error) => return Outcome::error(format!("room {room_id}: {error}")),
    };
    let me = inner.state.identity.device_id.clone();
    let Some(room) = inner.state.rooms.get_mut(room_id) else { return Outcome::default() };
    room.reconcile_members(&leaves, directory);
    room.admin = room.admin_id == me;
    Outcome::event(ClientEvent::RoomUpdated { room: room.info() })
}

fn on_envelope(inner: &mut Inner, room_id: &str, seq: u64, sender: String, plaintext: &[u8]) -> Outcome {
    let Ok(envelope) = Envelope::decode(plaintext) else { return Outcome::default() };
    let me = inner.state.identity.device_id.clone();
    let Some(room) = inner.state.rooms.get_mut(room_id) else { return Outcome::default() };
    let author = room.member(&sender).cloned().unwrap_or_else(|| MemberInfo::unverified(&sender));
    match envelope {
        Envelope::Text { text, at, .. } => {
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
        Envelope::RoomMeta { title } if sender == room.admin_id => {
            room.title = Some(title);
            Outcome::event(ClientEvent::RoomUpdated { room: room.info() })
        }
        Envelope::RoomRules(rules) if sender == room.admin_id && rules::replaces(room.rules.as_ref(), &rules) => {
            room.rules = Some(rules);
            Outcome::event(ClientEvent::RoomUpdated { room: room.info() })
        }
        Envelope::HistoryShare { title, rules, messages, .. } if sender != me && sender == room.admin_id && !room.history_imported => {
            import_history(room, &author.name, SharedHistory { title, rules, messages })
        }
        Envelope::RoomMeta { .. } | Envelope::RoomRules(_) | Envelope::HistoryShare { .. } | Envelope::Unknown { .. } => Outcome::default(),
    }
}

/// What the admin who let this device in shares with it.
struct SharedHistory {
    title: Option<String>,
    rules: Option<RoomRules>,
    messages: Vec<SharedMessage>,
}

fn import_history(room: &mut RoomState, shared_by: &str, share: SharedHistory) -> Outcome {
    if let Some(rules) = share.rules.filter(|rules| rules::replaces(room.rules.as_ref(), rules)) {
        room.rules = Some(rules);
    }
    let title = share.title;
    let before_join: Vec<_> = share.messages.into_iter().filter(|message| message.seq < room.start_seq).collect();
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

/// Where a Welcome lets this device in, as the relay announced it.
struct Arrival {
    room_id: String,
    policy: RoomPolicy,
    start_seq: u64,
}

fn on_welcome(inner: &mut Inner, arrival: Arrival, welcome: &[u8]) -> Outcome {
    let room_id = arrival.room_id.clone();
    if inner.state.rooms.contains_key(&room_id) {
        return Outcome::default();
    }
    if !inner.state.pending_joins.contains(&room_id) {
        return Outcome::error(format!("refused a Welcome to room {room_id}, which this device never asked to join"));
    }
    let joined = match inner.mls.join(welcome) {
        Ok(joined) => joined,
        Err(error) => return Outcome::error(format!("joining room {room_id}: {error}")),
    };
    inner.state.pending_joins.retain(|pending| *pending != room_id);
    if let Some(changed) = key_change(&inner.state.pins, &joined.members) {
        let message = format!("refused room {room_id}: {} presents a signing key that differs from the one first seen", changed.device_id);
        let _ = inner.mls.delete_group(&joined.group_id);
        return Outcome::error(message);
    }
    pin_new(&mut inner.state.pins, &joined.members);
    let room = joined_room(arrival, joined);
    let info = room.info();
    inner.state.rooms.insert(room_id.clone(), room);
    reconcile(inner, &room_id, &[]);
    Outcome { events: vec![ClientEvent::JoinedRoom { room: info }], actions: vec![Action::RefreshRoom { room_id }] }
}

fn joined_room(arrival: Arrival, joined: Joined) -> RoomState {
    RoomState {
        room_id: arrival.room_id,
        group_id: joined.group_id,
        title: None,
        policy: arrival.policy,
        admin: false,
        start_seq: arrival.start_seq,
        last_seq: arrival.start_seq.saturating_sub(1),
        members: Vec::new(),
        messages: Vec::new(),
        pending: Vec::new(),
        history_imported: false,
        rules: None,
        admin_id: joined.welcomer,
    }
}

fn on_join_requested(inner: &mut Inner, request: PendingJoin) -> Outcome {
    let owners: Vec<String> = inner.state.auto_approve.owners.iter().map(|owner| owner.device_id.clone()).collect();
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

/// Names a member the commit already added; the MLS tree decides who is in the room.
fn on_member_joined(inner: &mut Inner, room_id: &str, member: MemberInfo) -> Outcome {
    let Some(room) = inner.state.rooms.get_mut(room_id) else { return Outcome::default() };
    room.pending.retain(|pending| pending.device_id != member.device_id);
    reconcile(inner, room_id, &[member])
}

/// A member left: the room admin takes its leaf out of the group.
fn on_member_leaving(inner: &mut Inner, room_id: String, device_id: String) -> Outcome {
    let is_admin = inner.state.rooms.get(&room_id).is_some_and(|room| room.admin && room.member(&device_id).is_some());
    let actions = if is_admin { vec![Action::Remove { room_id, device_id }] } else { Vec::new() };
    Outcome { events: Vec::new(), actions }
}

fn on_member_removed(inner: &mut Inner, room_id: &str, device_id: &str) -> Outcome {
    if device_id == inner.state.identity.device_id {
        return left(inner, room_id);
    }
    reconcile(inner, room_id, &[])
}

fn left(inner: &mut Inner, room_id: &str) -> Outcome {
    let Some(room) = inner.state.rooms.remove(room_id) else { return Outcome::default() };
    let mut outcome = Outcome::event(ClientEvent::LeftRoom { room_id: room_id.into() });
    if let Err(error) = inner.mls.delete_group(&room.group_id) {
        outcome.events.push(ClientEvent::Error { message: format!("room {room_id}: secrets not deleted, {error}") });
    }
    outcome
}
