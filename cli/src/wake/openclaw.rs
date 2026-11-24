//! OpenClaw wakes through its hooks endpoint: each batch becomes a turn of one persistent
//! session per profile. OpenClaw starts `bucca mcp` only inside a run, so this connector owns
//! the profile: it follows the messages, and those MCP servers forward their tool calls to it.

use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;

use super::{Wake, next_batch, prompt};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// While the gateway restarts or prepares its agents it refuses connections or answers 503.
const RETRY_EVERY: Duration = Duration::from_secs(3);

pub struct Hooks {
    /// For example `http://127.0.0.1:18789/hooks`.
    pub url: String,
    pub token: String,
}

/// Session keys must match `hooks.allowedSessionKeyPrefixes` in the OpenClaw config.
pub const SESSION_PREFIX: &str = "buccanet-";

pub async fn serve(profile: &str, mut wakes: UnboundedReceiver<Wake>, hooks: Hooks) -> Result<(), Error> {
    let http = reqwest::Client::new();
    let session_key = format!("{SESSION_PREFIX}{profile}");
    while let Some(mut batch) = next_batch(&mut wakes).await {
        loop {
            match deliver(&http, &hooks, &session_key, &batch).await {
                Ok(()) => break tracing::info!(count = batch.len(), "woke openclaw"),
                Err(Retry::Later(error)) => eprintln!("buccanet: openclaw not ready ({error}), retrying"),
                Err(Retry::Never(error)) => break eprintln!("buccanet: openclaw refused the wake: {error}"),
            }
            tokio::time::sleep(RETRY_EVERY).await;
            drain_into(&mut wakes, &mut batch);
        }
    }
    Ok(())
}

enum Retry {
    Later(reqwest::Error),
    Never(reqwest::Error),
}

async fn deliver(http: &reqwest::Client, hooks: &Hooks, session_key: &str, batch: &[Wake]) -> Result<(), Retry> {
    let body = serde_json::json!({
        "message": prompt(batch),
        "name": "Buccanet",
        "sessionKey": session_key,
        "sessionMode": "persistent",
        "wakeMode": "now",
    });
    let url = format!("{}/agent", hooks.url.trim_end_matches('/'));
    let response = http.post(url).bearer_auth(&hooks.token).json(&body).send().await.map_err(Retry::Later)?;
    let server_side = response.status().is_server_error();
    match response.error_for_status() {
        Ok(_) => Ok(()),
        Err(error) if server_side => Err(Retry::Later(error)),
        Err(error) => Err(Retry::Never(error)),
    }
}

/// Wakes that arrived while waiting join the pending batch, so OpenClaw gets one turn for all.
fn drain_into(wakes: &mut UnboundedReceiver<Wake>, batch: &mut Vec<Wake>) {
    while let Ok(wake) = wakes.try_recv() {
        batch.push(wake);
    }
}
