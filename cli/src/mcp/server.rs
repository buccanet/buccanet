//! `bucca mcp --profile <name>`: the stdio loop, era selection and event delivery.

use std::sync::{Arc, Mutex};

use buccanet_sdk::{ChatMessage, ClientEvent};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::control::{self, Backend};
use super::events::EventHub;
use super::rpc::{INVALID_PARAMS, Out, PARSE_ERROR, RpcError};
use super::tools::ToolContext;
use super::{legacy, modern};
use crate::commands::CommandResult;
use crate::commands::output::kind_label;
use crate::host::{Events, open_client};
use crate::profile::{self, Profile, ProfileError};
use crate::wake::briefing::Briefer;
use crate::wake::claude_inbox::ClaudeInbox;
use crate::wake::{Wake, next_batch, prompt, socket, wakes};
use buccanet_agent::text;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Unknown,
    Legacy,
    Modern,
}

#[derive(Clone)]
struct Server {
    backend: Arc<Backend>,
    /// Set when this process runs inside Claude Code: wakes go to the session inbox.
    inbox: Option<tokio::sync::mpsc::UnboundedSender<Wake>>,
    hub: EventHub,
    out: Out,
    era: Arc<Mutex<Era>>,
}

fn server(backend: Backend, inbox: Option<tokio::sync::mpsc::UnboundedSender<Wake>>) -> Server {
    let out = Out::spawn();
    Server { backend: Arc::new(backend), inbox, hub: EventHub::new(out.clone()), out, era: Arc::new(Mutex::new(Era::Unknown)) }
}

/// The owner opens the device, follows its inbox, delivers wakes and answers other instances.
async fn owner(profile: Profile) -> Result<Server, Box<dyn std::error::Error>> {
    let (client, events) = open_client(&profile).await?;
    let client_id = client.device_id();
    let ctx = Arc::new(ToolContext { client: client.clone(), config: profile.config.clone(), profile_dir: profile.dir.clone() });
    control::serve(ctx.clone(), &profile.control_socket())?;
    let server = server(Backend::Local(ctx), ClaudeInbox::from_env().map(spawn_inbox));
    tokio::spawn(async move { client.run().await });
    let briefer = Briefer::new(profile.dir.clone());
    tokio::spawn(server.clone().deliver(events, profile.wake_socket(), briefer, client_id));
    Ok(server)
}

pub async fn run(profile_name: &str) -> CommandResult {
    let server = match Profile::open(profile_name) {
        Ok(profile) => owner(profile).await?,
        Err(ProfileError::Locked(..)) => server(Backend::Remote(profile::control_socket(profile_name)?), None),
        Err(error) => return Err(error.into()),
    };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let server = server.clone();
        tokio::spawn(async move { server.receive(&line).await });
    }
    tracing::info!("stdin closed, mcp server exiting");
    Ok(())
}

/// Merges wakes that arrive close together and writes each batch into the session inbox.
fn spawn_inbox(inbox: ClaudeInbox) -> tokio::sync::mpsc::UnboundedSender<Wake> {
    tracing::info!("delivering to the Claude Code session inbox");
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(batch) = next_batch(&mut receiver).await {
            if let Err(error) = inbox.deliver(&prompt(&batch)).await {
                tracing::warn!(%error, "session inbox refused the wake");
            }
        }
    });
    sender
}

impl Server {
    fn era(&self) -> Era {
        *self.era.lock().expect("era lock")
    }

    fn set_era(&self, era: Era) {
        *self.era.lock().expect("era lock") = era;
    }

    async fn receive(&self, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            self.out.error(&Value::Null, RpcError::new(PARSE_ERROR, "Parse error"));
            return;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else { return };
        let params = message.get("params").cloned().unwrap_or(json!({}));
        match message.get("id") {
            Some(id) => self.request(id, method, &params).await,
            None => self.notification(method, &params),
        }
    }

    fn notification(&self, method: &str, params: &Value) {
        if let ("notifications/cancelled", Some(request_id)) = (method, params.get("requestId")) {
            self.hub.cancel(request_id);
        }
    }

    async fn request(&self, id: &Value, method: &str, params: &Value) {
        let answer = if method == "initialize" {
            self.set_era(Era::Legacy);
            Ok(Some(legacy::initialize(params)))
        } else if modern::version_of(params).is_some() {
            self.set_era(Era::Modern);
            modern::handle(&self.backend, &self.hub, &self.out, modern::Request { id, method, params }).await
        } else if self.era() == Era::Legacy {
            legacy::handle(&self.backend, method, params).await.map(Some)
        } else {
            Err(RpcError::new(INVALID_PARAMS, "Send initialize first, or include _meta io.modelcontextprotocol/protocolVersion"))
        };
        match answer {
            Ok(Some(result)) => self.out.result(id, result),
            Ok(None) => {}
            Err(error) => self.out.error(id, error),
        }
    }

    /// The Claude channel, used only when no session inbox is available, so nothing arrives twice.
    fn channel(&self, content: String, meta: Value) {
        if self.inbox.is_none() && self.era() == Era::Legacy {
            self.out.notify("notifications/claude/channel", json!({ "content": content, "meta": meta }));
        }
    }

    async fn wake(&self, socket: &std::path::Path, wake: Wake) {
        match &self.inbox {
            Some(inbox) => {
                let _ = inbox.send(wake);
            }
            None => socket::notify(socket, &wake).await,
        }
    }

    /// Wakes the agent with the instructions and rules it follows, when this process owns the device.
    async fn briefed_wake(&self, socket: &std::path::Path, briefer: &mut Briefer, mut wake: Wake) {
        if let Ok(ctx) = self.backend.local() {
            briefer.brief(&ctx.client, &mut wake).await;
        }
        self.wake(socket, wake).await;
    }

    /// A message meant for the agent: announced on the channel and to event subscribers, then woken on.
    async fn deliver_message(&self, socket: &std::path::Path, briefer: &mut Briefer, message: &ChatMessage, me: &str) {
        let name = text::line(&message.author_name);
        let content = format!("{name} ({}): {}", kind_label(message.author_kind), text::indented(&message.text));
        let meta = json!({
            "room_id": message.room_id, "seq": message.seq.to_string(),
            "from": name, "from_kind": kind_label(message.author_kind),
        });
        self.channel(content, meta);
        self.hub.publish_message(message);
        let title = match self.backend.local() {
            Ok(ctx) => ctx.client.room(&message.room_id).await.ok().and_then(|room| room.title),
            Err(_) => None,
        };
        self.briefed_wake(socket, briefer, Wake::message(message, title, me)).await;
    }

    /// Publishes every message to event subscribers, and wakes the agent for the ones meant for it.
    async fn deliver(self, mut events: Events, socket: std::path::PathBuf, mut briefer: Briefer, me: String) {
        while let Some(event) = events.recv().await {
            match event {
                ClientEvent::Message { message } if !message.own && !message.shared && !wakes(&message, &me) => {
                    self.hub.publish_message(&message);
                }
                ClientEvent::Message { message } if !message.own && !message.shared => {
                    self.deliver_message(&socket, &mut briefer, &message, &me).await;
                }
                ClientEvent::JoinedRoom { room } => {
                    let detail = format!("You are now a member of room {}. Read it with read_messages.", room.room_id);
                    self.channel(detail.clone(), json!({ "room_id": room.room_id, "event": "joined" }));
                    self.hub.publish_room(&room.room_id, "joined", &detail);
                    self.briefed_wake(&socket, &mut briefer, Wake::joined(&room.room_id, room.title.clone())).await;
                }
                ClientEvent::JoinRequested { request } => {
                    let detail = format!(
                        "{} ({}) asks to join. Request id {}.",
                        text::line(&request.name),
                        kind_label(request.kind),
                        text::line(&request.request_id)
                    );
                    self.channel(detail.clone(), json!({ "room_id": request.room_id, "event": "join_requested" }));
                    self.hub.publish_room(&request.room_id, "join_requested", &detail);
                }
                ClientEvent::LeftRoom { room_id } => {
                    self.channel(format!("You were removed from room {room_id}."), json!({ "room_id": room_id, "event": "left" }));
                    self.hub.publish_room(&room_id, "left", "removed from the room");
                }
                ClientEvent::Error { message } => tracing::warn!(%message, "sdk"),
                _ => {}
            }
        }
    }
}
