//! Live delivery into the Claude Code session that runs this MCP server.
//!
//! Every Claude Code session keeps an inbox on a Unix socket and hands its path and token to
//! the processes it starts (`CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN`).
//! Claude Code documents the inject format in its own logs: one auth line, then one user
//! message per JSON line. Messages arrive as live input, the way other sessions reach it.

use std::path::{Path, PathBuf};

use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

pub struct ClaudeInbox {
    socket: PathBuf,
    token: String,
}

/// How far up the process tree the session's Claude Code process is looked for.
const MAX_ANCESTORS: usize = 64;

impl ClaudeInbox {
    /// The inbox of the parent session, when this process runs inside Claude Code.
    pub fn from_env() -> Option<Self> {
        let socket = std::env::var_os("CLAUDE_CODE_MESSAGING_SOCKET").map(PathBuf::from)?;
        let token = std::env::var("CLAUDE_CODE_MESSAGING_TOKEN").ok()?;
        (socket.exists() && inside_session(&socket)).then_some(Self { socket, token })
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

/// Whether this process descends from the Claude Code process that owns the inbox. Claude Code
/// names each inbox socket after its own process id (`<pid>.sock`). Processes started elsewhere,
/// such as another harness under a tmux server launched from a session, inherit the session's
/// variables too and must not deliver into it.
fn inside_session(socket: &Path) -> bool {
    let Some(session) = socket.file_stem().and_then(|stem| stem.to_str()).and_then(|stem| stem.parse::<u32>().ok()) else { return false };
    let mut pid = std::process::id();
    for _ in 0..MAX_ANCESTORS {
        match parent_of(pid) {
            Some(parent) if parent == session => return true,
            Some(parent) if parent > 1 => pid = parent,
            _ => return false,
        }
    }
    false
}

fn parent_of(pid: u32) -> Option<u32> {
    let output = std::process::Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}
