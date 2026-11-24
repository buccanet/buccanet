//! Terminal UI state: rooms, the selected conversation, the input line and what it means.

use std::cell::Cell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use buccanet_sdk::{ChatMessage, Client, ClientEvent, PendingJoin, RoomInfo};

use crate::profile;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Rooms,
    Input,
}

pub struct App {
    pub client: Client,
    pub profile: String,
    pub rooms: Vec<RoomInfo>,
    pub selected: usize,
    pub messages: Vec<ChatMessage>,
    pub unread: HashMap<String, u32>,
    pub pending: Vec<PendingJoin>,
    pub input: String,
    pub focus: Focus,
    pub status: String,
    pub live: bool,
    /// Device id of each local agent profile, so the UI can say which agents are yours.
    pub local_agents: HashMap<String, String>,
    pub quit: bool,
    /// Lines scrolled back from the newest message; 0 follows the conversation.
    pub scroll_back: usize,
    /// How far back the message view can go, measured by the last draw.
    pub scroll_limit: Cell<usize>,
    /// When Ctrl+C was pressed on an empty input; a second press soon after quits.
    quit_armed: Option<Instant>,
}

/// How long a first Ctrl+C waits for the second one.
const QUIT_WINDOW: Duration = Duration::from_secs(2);

fn local_agents() -> HashMap<String, String> {
    profile::list()
        .into_iter()
        .filter_map(|name| profile::read_config(&name).ok().map(|config| (config.device_id, name)))
        .collect()
}

impl App {
    pub async fn new(client: Client, profile: String) -> Self {
        let mut app = Self {
            client,
            profile,
            rooms: Vec::new(),
            selected: 0,
            messages: Vec::new(),
            unread: HashMap::new(),
            pending: Vec::new(),
            input: String::new(),
            focus: Focus::Input,
            status: "Type a message, or /help.".into(),
            live: false,
            local_agents: local_agents(),
            quit: false,
            scroll_back: 0,
            scroll_limit: Cell::new(0),
            quit_armed: None,
        };
        app.reload().await;
        app
    }

    pub fn room(&self) -> Option<&RoomInfo> {
        self.rooms.get(self.selected)
    }

    pub fn room_id(&self) -> Option<String> {
        self.room().map(|room| room.room_id.clone())
    }

    /// Re-reads rooms and the selected conversation from the SDK.
    pub async fn reload(&mut self) {
        let current = self.room_id();
        let mut rooms = self.client.rooms().await;
        rooms.sort_by(|a, b| b.last_seq.cmp(&a.last_seq).then(a.room_id.cmp(&b.room_id)));
        self.rooms = rooms;
        if let Some(current) = current {
            self.selected = self.rooms.iter().position(|room| room.room_id == current).unwrap_or(0);
        }
        self.selected = self.selected.min(self.rooms.len().saturating_sub(1));
        self.load_messages().await;
    }

    pub async fn load_messages(&mut self) {
        self.messages = match self.room_id() {
            Some(room) => {
                self.unread.remove(&room);
                self.client.messages(&room, 0).await.unwrap_or_default()
            }
            None => Vec::new(),
        };
    }

    /// Ctrl+C clears what is being typed; on an empty input it asks for a second press to quit.
    pub fn interrupt(&mut self) {
        if !self.input.is_empty() {
            self.input.clear();
            self.quit_armed = None;
            return;
        }
        if self.quit_armed.is_some_and(|at| at.elapsed() < QUIT_WINDOW) {
            self.quit = true;
            return;
        }
        self.quit_armed = Some(Instant::now());
        self.status = "Press Ctrl+C again to quit.".into();
    }

    /// Moves the message view: positive goes back in history, negative towards the newest.
    pub fn scroll(&mut self, lines: isize) {
        self.scroll_back = self.scroll_back.saturating_add_signed(lines).min(self.scroll_limit.get());
    }

    pub async fn select(&mut self, delta: isize) {
        if self.rooms.is_empty() {
            return;
        }
        let count = self.rooms.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(count)) as usize;
        self.scroll_back = 0;
        self.load_messages().await;
    }

    pub async fn on_event(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::Message { message } => {
                if Some(&message.room_id) != self.room_id().as_ref() {
                    *self.unread.entry(message.room_id.clone()).or_insert(0) += 1;
                }
                self.reload().await;
            }
            ClientEvent::JoinRequested { request } => {
                self.status = format!("{} asks to join. /approve to let them in, /reject to refuse.", request.name);
                self.pending.retain(|pending| pending.request_id != request.request_id);
                self.pending.push(request);
            }
            ClientEvent::JoinedRoom { .. } | ClientEvent::RoomUpdated { .. } | ClientEvent::LeftRoom { .. } => {
                self.local_agents = local_agents();
                self.reload().await;
            }
            ClientEvent::HistoryImported { count, shared_by, .. } => {
                self.status = format!("{shared_by} shared {count} earlier messages.");
                self.reload().await;
            }
            ClientEvent::JoinRejected { .. } => self.status = "Your join request was rejected.".into(),
            ClientEvent::Connection { live } => self.live = live,
            ClientEvent::Error { message } => self.status = format!("error: {message}"),
        }
    }
}
