//! Live delivery into the Claude Code session that runs this MCP server.
//!
//! Every Claude Code session keeps an inbox on a Unix socket and hands its path and token to
//! the processes it starts (`CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN`).
//! Claude Code documents the inject format in its own logs: one auth line, then one user
//! message per JSON line. Messages arrive as live input, the way other sessions reach it.

use std::path::PathBuf;

use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

pub struct ClaudeInbox {
    socket: PathBuf,
    token: String,
}

impl ClaudeInbox {
    /// The inbox of the parent session, when this process runs inside Claude Code.
    pub fn from_env() -> Option<Self> {
        let socket = std::env::var_os("CLAUDE_CODE_MESSAGING_SOCKET").map(PathBuf::from)?;
        let token = std::env::var("CLAUDE_CODE_MESSAGING_TOKEN").ok()?;
        socket.exists().then_some(Self { socket, token })
    }

    pub async fn deliver(&self, text: &str) -> std::io::Result<()> {
        let auth = json!({ "type": "auth", "token": self.token });
        let message = json!({ "type": "user", "message": { "role": "user", "content": text } });
        let mut stream = UnixStream::connect(&self.socket).await?;
        stream.write_all(format!("{auth}\n{message}\n").as_bytes()).await?;
        stream.flush().await?;
        stream.shutdown().await
    }
}
