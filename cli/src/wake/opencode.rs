//! OpenCode wakes through its server: we start `opencode serve`, attach the TUI to it, and
//! post each batch of wakes as a prompt into the most recent session.

use std::process::ExitStatus;
use std::time::Duration;

use serde_json::{Value, json};

use super::{next_batch, prompt, socket};
use crate::commands::run::Launch;

type Error = Box<dyn std::error::Error + Send + Sync>;

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

pub async fn launch(launch: &Launch, extra: &[String]) -> Result<ExitStatus, Error> {
    let port = free_port()?;
    let base = format!("http://127.0.0.1:{port}");
    let config = json!({ "mcp": { "buccanet": { "type": "local", "command": [launch.exe, "mcp", "--profile", launch.profile], "environment": { "BUCCANET_HOME": launch.home }, "enabled": true } } });
    let mut server = tokio::process::Command::new("opencode")
        .args(["serve", "--port", &port.to_string()])
        .env("OPENCODE_CONFIG_CONTENT", config.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let http = reqwest::Client::new();
    wait_for(&http, &base).await?;
    let mut wakes = socket::listen(&launch.wake_socket())?;
    let target = base.clone();
    tokio::spawn(async move {
        while let Some(batch) = next_batch(&mut wakes).await {
            if let Err(error) = deliver(&http, &target, &prompt(&batch)).await {
                tracing::warn!(%error, "opencode wake not delivered");
            }
        }
    });
    let status = tokio::process::Command::new("opencode").args(["attach", &base]).args(extra).status().await?;
    let _ = server.kill().await;
    Ok(status)
}

async fn wait_for(http: &reqwest::Client, base: &str) -> Result<(), Error> {
    for _ in 0..150 {
        if http.get(format!("{base}/session")).send().await.is_ok_and(|response| response.status().is_success()) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("opencode serve did not start".into())
}

/// The session updated last: the one the TUI shows.
async fn latest_session(http: &reqwest::Client, base: &str) -> Result<Option<String>, Error> {
    let sessions: Vec<Value> = http.get(format!("{base}/session")).send().await?.error_for_status()?.json().await?;
    let latest = sessions.iter().max_by_key(|session| session.pointer("/time/updated").and_then(Value::as_u64).unwrap_or(0));
    Ok(latest.and_then(|session| session.get("id")).and_then(Value::as_str).map(str::to_string))
}

async fn deliver(http: &reqwest::Client, base: &str, text: &str) -> Result<(), Error> {
    let session = latest_session(http, base).await?.ok_or("no OpenCode session is open yet")?;
    let body = json!({ "parts": [{ "type": "text", "text": text }] });
    http.post(format!("{base}/session/{session}/prompt_async")).json(&body).send().await?.error_for_status()?;
    Ok(())
}
