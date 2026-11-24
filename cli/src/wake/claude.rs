//! Claude Code wakes through its session inbox: `bucca mcp` runs inside the session and
//! writes each new message there (see `claude_inbox`). Nothing to relay from the launcher.

use std::process::ExitStatus;

use serde_json::json;

use crate::commands::run::Launch;
use crate::profile::write_private;

pub async fn launch(launch: &Launch, extra: &[String]) -> std::io::Result<ExitStatus> {
    let config = json!({ "mcpServers": { "buccanet": { "command": launch.exe, "args": ["mcp", "--profile", launch.profile], "env": { "BUCCANET_HOME": launch.home } } } });
    let path = launch.dir.join("claude-mcp.json");
    write_private(&path, &serde_json::to_vec_pretty(&config).expect("JSON always serializes"))?;
    let command = claude_command();
    tokio::process::Command::new(&command[0])
        .args(&command[1..])
        .arg("--mcp-config")
        .arg(&path)
        .args(extra)
        .status()
        .await
}

/// The command that starts Claude Code on this machine, such as a wrapper that loads the
/// account (`BUCCANET_CLAUDE_COMMAND="claude-profile claude"`). Defaults to `claude`.
fn claude_command() -> Vec<String> {
    let configured = std::env::var("BUCCANET_CLAUDE_COMMAND").unwrap_or_else(|_| "claude".to_string());
    let parts: Vec<String> = configured.split_whitespace().map(str::to_string).collect();
    if parts.is_empty() { vec!["claude".to_string()] } else { parts }
}
