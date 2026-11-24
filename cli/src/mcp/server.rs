//! `bucca mcp --profile <name>`: the stdio loop, era selection and event delivery.

use std::sync::{Arc, Mutex};

use buccanet_sdk::ClientEvent;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::events::EventHub;
use super::rpc::{INVALID_PARAMS, Out, PARSE_ERROR, RpcError};
use super::control::{self, Backend};
use super::tools::ToolContext;
use super::{legacy, modern};
use crate::wake::claude_inbox::ClaudeInbox;
use crate::wake::{Wake, next_batch, prompt, socket};
use crate::commands::CommandResult;
use crate::commands::output::kind_label;
use crate::host::{Events, open_client};
use crate::profile::{self, Profile, ProfileError};

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

fn init_logging(profile: &Profile) {
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(profile.log_path()) else { return };
    let _ = tracing_subscriber::fmt().with_ansi(false).with_writer(Mutex::new(file)).try_init();
}

fn server(backend: Backend, inbox: Option<tokio::sync::mpsc::UnboundedSender<Wake>>) -> Server {
    let out = Out::spawn();
    Server { backend: Arc::new(backend), inbox, hub: EventHub::new(out.clone()), out, era: Arc::new(Mutex::new(Era::Unknown)) }
}

/// The owner opens the device, follows its inbox, delivers wakes and answers other instances.
async fn owner(profile: Profile) -> Result<Server, Box<dyn std::error::Error>> {
    init_logging(&profile);
    let (client, events) = open_client(&profile).await?;
    let ctx = Arc::new(ToolContext { client: client.clone(), config: profile.config.clone() });
    control::serve(ctx.clone(), &profile.control_socket())?;
    let server = server(Backend::Local(ctx), ClaudeInbox::from_env().map(spawn_inbox));
    tokio::spawn(async move { client.run().await });
    tokio::spawn(server.clone().deliver(events, profile.wake_socket()));
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
            modern::handle(&self.backend, &self.hub, &self.out, id, method, params).await
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

    async fn deliver(self, mut events: Events, socket: std::path::PathBuf) {
        while let Some(event) = events.recv().await {
            match event {
                ClientEvent::Message { message } if !message.own && !message.shared => {
                    let content = format!("{} ({}): {}", message.author_name, kind_label(message.author_kind), message.text);
                    let meta = json!({
                        "room_id": message.room_id, "seq": message.seq.to_string(),
                        "from": message.author_name, "from_kind": kind_label(message.author_kind),
                    });
                    self.channel(content, meta);
                    self.hub.publish_message(&message);
                    let title = match self.backend.local() {
                        Ok(ctx) => ctx.client.room(&message.room_id).await.ok().and_then(|room| room.title),
                        Err(_) => None,
                    };
                    self.wake(&socket, Wake::message(&message, title)).await;
                }
                ClientEvent::JoinedRoom { room } => {
                    let detail = format!("You are now a member of room {}. Read it with read_messages.", room.room_id);
                    self.channel(detail.clone(), json!({ "room_id": room.room_id, "event": "joined" }));
                    self.hub.publish_room(&room.room_id, "joined", &detail);
                    self.wake(&socket, Wake::joined(&room.room_id, room.title.clone())).await;
                }
                ClientEvent::JoinRequested { request } => {
                    let detail = format!("{} ({}) asks to join. Request id {}.", request.name, kind_label(request.kind), request.request_id);
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
