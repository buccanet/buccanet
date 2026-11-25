//! Waking harnesses that have no push path of their own. The process that owns a profile
//! turns each new message into a wake: the MCP server announces it on the profile's
//! `wake.sock` for the launcher listening there, and a connector that owns the profile itself
//! (OpenClaw) follows its client directly. Wakes arriving close together become one prompt.

pub mod bridge;
pub mod briefing;
pub mod claude;
pub mod claude_inbox;
pub mod claude_session;
pub mod codex;
pub mod openclaw;
pub mod opencode;
pub mod socket;

use std::time::Duration;

use buccanet_agent::prompt::{Tools, render};
pub use buccanet_agent::{Wake, wakes};
use buccanet_sdk::{Client, ClientEvent};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::host::Events;
use crate::update;
use briefing::Briefer;

/// Wakes arriving within this window become one prompt.
const MERGE_WINDOW: Duration = Duration::from_millis(1500);

/// The wakes of a client this process owns: messages from others and rooms joined, each
/// briefed with the instructions and rules the agent follows.
pub fn follow(client: Client, mut events: Events, mut briefer: Briefer) -> UnboundedReceiver<Wake> {
    let (sender, receiver) = unbounded_channel();
    let me = client.device_id();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            let mut wake = match event {
                ClientEvent::Message { message } if wakes(&message, &me) => {
                    let title = client.room(&message.room_id).await.ok().and_then(|room| room.title);
                    Wake::message(&message, title, &me)
                }
                ClientEvent::JoinedRoom { room } => Wake::joined(&room.room_id, room.title.clone()),
                _ => continue,
            };
            briefer.brief(&client, &mut wake).await;
            if sender.send(wake).is_err() {
                return;
            }
        }
    });
    receiver
}

/// The prompt a harness with the buccanet MCP tools receives for a batch of wakes.
pub fn prompt(batch: &[Wake]) -> String {
    render(batch, Tools::Mcp)
}

/// The prompt for an agent that joined with the `bucca` command instead of MCP tools.
pub fn command_prompt(batch: &[Wake]) -> String {
    render(batch, Tools::Command(update::command()))
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
    use buccanet_agent::Author;
    use buccanet_sdk::DeviceKind;

    use super::*;

    fn wake(seq: u64, text: &str) -> Wake {
        let author = Author { device_id: "dev".into(), name: "Ada".into(), kind: DeviceKind::Human };
        Wake { text: Some(text.into()), from: Some(author), seq, ..Wake::joined("r", Some("Plan".into())) }
    }

    #[tokio::test]
    async fn wakes_close_together_merge_into_one_prompt() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(wake(1, "m1")).unwrap();
        sender.send(wake(2, "m2")).unwrap();
        let batch = next_batch(&mut receiver).await.unwrap();
        assert_eq!(batch.len(), 2);
        let text = prompt(&batch);
        assert!(text.contains("\"seq\":1") && text.contains("\"text\":\"m2\""));
    }
}
