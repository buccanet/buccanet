//! Waking harnesses that have no push path of their own. The process that owns a profile
//! turns each new message into a wake: the MCP server announces it on the profile's
//! `wake.sock` for the launcher listening there, and a connector that owns the profile itself
//! (OpenClaw) follows its client directly. Wakes arriving close together become one prompt.

pub mod bridge;
pub mod claude;
pub mod claude_inbox;
pub mod claude_session;
pub mod codex;
pub mod openclaw;
pub mod opencode;
pub mod socket;

use std::time::Duration;

use buccanet_sdk::{ChatMessage, Client, ClientEvent};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::host::Events;

/// Wakes arriving within this window become one prompt.
const MERGE_WINDOW: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wake {
    pub room_id: String,
    pub room_title: Option<String>,
    pub seq: u64,
    pub from: Option<String>,
    pub text: Option<String>,
}

impl Wake {
    pub fn message(message: &ChatMessage, room_title: Option<String>) -> Self {
        Self {
            room_id: message.room_id.clone(),
            room_title,
            seq: message.seq,
            from: Some(message.author_name.clone()),
            text: Some(message.text.clone()),
        }
    }

    pub fn joined(room_id: &str, room_title: Option<String>) -> Self {
        Self { room_id: room_id.to_string(), room_title, seq: 0, from: None, text: None }
    }

    fn line(&self) -> String {
        let room = match &self.room_title {
            Some(title) => format!("\"{title}\" ({})", self.room_id),
            None => self.room_id.clone(),
        };
        match (&self.from, &self.text) {
            (Some(from), Some(text)) => format!("- room {room}, #{} from {from}: {text}", self.seq),
            _ => format!("- you joined room {room}"),
        }
    }
}

/// The wakes of a client this process owns: messages from others and rooms joined.
pub fn follow(client: Client, mut events: Events) -> UnboundedReceiver<Wake> {
    let (sender, receiver) = unbounded_channel();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            let wake = match event {
                ClientEvent::Message { message } if !message.own && !message.shared => {
                    let title = client.room(&message.room_id).await.ok().and_then(|room| room.title);
                    Wake::message(&message, title)
                }
                ClientEvent::JoinedRoom { room } => Wake::joined(&room.room_id, room.title.clone()),
                ClientEvent::Error { message } => {
                    tracing::warn!(%message, "sdk");
                    continue;
                }
                _ => continue,
            };
            if sender.send(wake).is_err() {
                return;
            }
        }
    });
    receiver
}

fn activity(batch: &[Wake]) -> String {
    let lines: Vec<String> = batch.iter().map(Wake::line).collect();
    format!("[Buccanet] New activity in your chats:\n{}", lines.join("\n"))
}

/// The prompt a harness with the buccanet MCP tools receives for a batch of wakes.
pub fn prompt(batch: &[Wake]) -> String {
    format!(
        "{}\nRead the room with the buccanet read_messages tool and answer with send_message if you have something useful to add. Stay silent otherwise.",
        activity(batch)
    )
}

/// The prompt for an agent that joined with the `bucca` command instead of MCP tools.
pub fn command_prompt(batch: &[Wake]) -> String {
    format!(
        "{}\nIf you have something useful to add, answer with `bucca send <room id> \"<text>\"`; `bucca read <room id>` shows the conversation. Stay silent otherwise.",
        activity(batch)
    )
}

/// Yields batches: the first wake opens a window, everything arriving inside it joins the batch.
pub async fn next_batch(wakes: &mut UnboundedReceiver<Wake>) -> Option<Vec<Wake>> {
    let first = wakes.recv().await?;
    let mut batch = vec![first];
    let deadline = tokio::time::Instant::now() + MERGE_WINDOW;
    while let Ok(Some(wake)) = tokio::time::timeout_at(deadline, wakes.recv()).await {
        batch.push(wake);
    }
    Some(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wakes_close_together_merge_into_one_prompt() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let wake = |seq| Wake { room_id: "r".into(), room_title: Some("Plan".into()), seq, from: Some("Ada".into()), text: Some(format!("m{seq}")) };
        sender.send(wake(1)).unwrap();
        sender.send(wake(2)).unwrap();
        let batch = next_batch(&mut receiver).await.unwrap();
        assert_eq!(batch.len(), 2);
        let text = prompt(&batch);
        assert!(text.contains("#1 from Ada: m1") && text.contains("#2 from Ada: m2"));
    }
}
