//! A Claude Code session that joins from inside the chat. The agent runs `bucca join <link>`;
//! that starts this process, tied to the session: it owns the session's agent profile, answers
//! the agent's `bucca send` and `bucca read` on the control socket, and writes new room
//! messages into the session inbox so Claude wakes on its own.

use std::sync::Arc;

use super::claude_inbox::ClaudeInbox;
use super::{command_prompt, follow, next_batch};
use crate::commands::CommandResult;
use crate::host::open_client;
use crate::mcp::control;
use crate::mcp::tools::ToolContext;
use crate::profile::Profile;

pub struct ClaudeSession {
    pub id: String,
    pub inbox: ClaudeInbox,
}

impl ClaudeSession {
    /// The Claude Code session this process runs in, if any.
    pub fn from_env() -> Option<Self> {
        let id = std::env::var("CLAUDE_CODE_SESSION_ID").ok().filter(|id| !id.is_empty())?;
        Some(Self { id, inbox: ClaudeInbox::from_env()? })
    }

    /// One agent profile per session, so two sessions are two members.
    pub fn profile(&self) -> String {
        format!("claude-{}", self.id.chars().take(8).collect::<String>())
    }

    /// "Claude (folder)", so people tell the sessions apart.
    pub fn display_name() -> String {
        let folder = std::env::current_dir().ok().and_then(|dir| dir.file_name().map(|name| name.to_string_lossy().into_owned()));
        match folder {
            Some(folder) => format!("Claude ({folder})"),
            None => "Claude".to_string(),
        }
    }
}

/// Runs until the session is gone: a refused delivery means nobody is left to wake.
pub async fn serve(profile: Profile, inbox: ClaudeInbox) -> CommandResult {
    let (client, events) = open_client(&profile).await?;
    client.sync().await?;
    let ctx = Arc::new(ToolContext { client: client.clone(), config: profile.config.clone() });
    control::serve(ctx, &profile.control_socket())?;
    let mut wakes = follow(client.clone(), events);
    tokio::spawn(async move { client.run().await });
    while let Some(batch) = next_batch(&mut wakes).await {
        if let Err(error) = inbox.deliver(&command_prompt(&batch)).await {
            tracing::info!(%error, "the Claude Code session is gone");
            break;
        }
    }
    Ok(())
}
