//! Terminal UI state: rooms, the selected conversation, the input line and what it means.

use std::cell::Cell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use buccanet_sdk::{ChatMessage, Client, ClientEvent, PendingJoin, RoomInfo};
use ratatui::buffer::Buffer;

use super::editor::Editor;
use super::external::External;
use super::menu::{self, MenuItem, RoomContext};
use super::selection::{Panes, Selection};
use crate::profile;
use buccanet_agent::text;

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
    /// Who waits to join the rooms this device administers, as the relay last said.
    pub pending: Vec<PendingJoin>,
    pub input: Editor,
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
    /// Rows the rooms and members lists are scrolled down by the wheel, and how far the members
    /// can go, measured by the last draw.
    pub rooms_scroll: usize,
    pub members_scroll: usize,
    pub members_limit: Cell<usize>,
    /// When Ctrl+C was pressed on an empty input; a second press soon after quits.
    quit_armed: Option<Instant>,
    /// The highlighted row of the command menu.
    pub picked: usize,
    /// A command that undoes something, asked once, and when: running it again soon confirms it.
    pending_confirm: Option<(String, Instant)>,
    /// Another program to hand the terminal to before the next draw, such as the editor.
    pub external: Option<External>,
    /// Where the last draw put each pane, the width of the input inside its borders, and how
    /// many of its rows it scrolled past to keep the cursor in view.
    pub panes: Cell<Panes>,
    pub input_width: Cell<u16>,
    pub input_offset: Cell<u16>,
    /// What the last draw showed, to copy a selection from.
    pub screen: Buffer,
    pub selection: Option<Selection>,
}

/// How long a first Ctrl+C waits for the second one.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
/// How long `/leave` or `/remove` waits to be run a second time.
const CONFIRM_WINDOW: Duration = Duration::from_secs(5);

fn local_agents() -> HashMap<String, String> {
    profile::list().into_iter().filter_map(|name| profile::read_config(&name).ok().map(|config| (config.device_id, name))).collect()
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
            input: Editor::default(),
            focus: Focus::Input,
            status: "Type a message, or / for commands.".into(),
            live: false,
            local_agents: local_agents(),
            quit: false,
            scroll_back: 0,
            scroll_limit: Cell::new(0),
            rooms_scroll: 0,
            members_scroll: 0,
            members_limit: Cell::new(0),
            quit_armed: None,
            picked: 0,
            pending_confirm: None,
            external: None,
            panes: Cell::new(Panes::default()),
            input_width: Cell::new(0),
            input_offset: Cell::new(0),
            screen: Buffer::default(),
            selection: None,
        };
        app.reload().await;
        app.refresh_pending().await;
        app
    }

    pub fn room(&self) -> Option<&RoomInfo> {
        self.rooms.get(self.selected)
    }

    /// Re-reads who waits to join from the relay. Only the relay knows that a request was decided
    /// elsewhere: by another admin, by the automatic approval of an open room, or by `bucca approve`.
    pub async fn refresh_pending(&mut self) {
        let mut pending = Vec::new();
        for room in self.rooms.iter().filter(|room| room.admin) {
            match self.client.pending_requests(&room.room_id).await {
                Ok(requests) => pending.extend(requests),
                Err(error) => {
                    self.status = format!("Could not read who waits to join: {}", text::line(&error.to_string()));
                    return;
                }
            }
        }
        self.pending = pending;
    }

    /// The requests waiting to join the open room.
    pub fn pending_here(&self) -> Vec<&PendingJoin> {
        let room = self.room_id();
        self.pending.iter().filter(|request| Some(&request.room_id) == room.as_ref()).collect()
    }

    /// A room's title, or the start of its id when it has none.
    pub fn room_title(&self, room_id: &str) -> String {
        let title = self.rooms.iter().find(|room| room.room_id == room_id).and_then(|room| room.title.clone());
        title.map_or_else(|| text::short(room_id, 8), |title| text::line(&title))
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

    /// True when `action` was asked within the confirmation window; otherwise remembers it.
    pub fn confirmed(&mut self, action: String) -> bool {
        let again = self.pending_confirm.take().is_some_and(|(asked, at)| asked == action && at.elapsed() < CONFIRM_WINDOW);
        if !again {
            self.pending_confirm = Some((action, Instant::now()));
        }
        again
    }

    /// The menu for what is typed; empty when the menu is closed.
    pub fn menu(&self) -> Vec<MenuItem> {
        let me = self.client.device_id();
        let members = self.room().map(|room| room.members.as_slice()).unwrap_or_default();
        let waiting: Vec<PendingJoin> = self.pending_here().into_iter().cloned().collect();
        menu::matching(self.input.text(), &RoomContext { members, waiting: &waiting, me: &me })
    }

    /// Moves the menu's highlight, wrapping at both ends.
    pub fn pick(&mut self, delta: isize) {
        let count = self.menu().len() as isize;
        if count > 0 {
            self.picked = (self.picked as isize + delta).rem_euclid(count) as usize;
        }
    }

    /// Writes the highlighted row into the input; true when that input is ready to run.
    pub fn complete(&mut self) -> Option<bool> {
        let item = self.menu().into_iter().nth(self.picked)?;
        self.input.set(item.fill);
        self.picked = 0;
        Some(item.runs)
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
                self.refresh_pending().await;
                if self.pending.iter().any(|pending| pending.request_id == request.request_id) {
                    let room = self.room_title(&request.room_id);
                    self.status = format!("{} asks to join {room}. /approve or /reject to decide.", text::line(&request.name));
                }
            }
            ClientEvent::JoinedRoom { .. } | ClientEvent::RoomUpdated { .. } | ClientEvent::LeftRoom { .. } => {
                self.local_agents = local_agents();
                self.reload().await;
                self.refresh_pending().await;
            }
            ClientEvent::HistoryImported { count, shared_by, .. } => {
                self.status = format!("{} shared {count} earlier messages.", text::line(&shared_by));
                self.reload().await;
            }
            ClientEvent::JoinRejected { .. } => self.status = "Your join request was rejected.".into(),
            ClientEvent::Connection { live } => self.live = live,
            ClientEvent::Error { message } => self.status = format!("error: {}", text::line(&message)),
        }
    }
}
