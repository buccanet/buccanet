//! OpenCode wakes through its server: we start `opencode serve`, attach the TUI to it, and
//! post each batch of wakes as a prompt into the most recent session. The server asks for a
//! password made for this launch (basic auth, user `opencode`), so other local users cannot
//! drive it, and it picks its own port, which it prints on stdout.

use std::process::ExitStatus;
use std::time::Duration;

use base64::Engine;
use rand::RngCore;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdout};

use super::{next_batch, prompt, socket};
use crate::commands::run::Launch;

type Error = Box<dyn std::error::Error + Send + Sync>;

const USER: &str = "opencode";
const STARTUP: Duration = Duration::from_secs(15);

/// Where the server listens and the password it asks for.
#[derive(Clone)]
struct Server {
    base: String,
    password: String,
}

impl Server {
    fn get(&self, http: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        http.get(format!("{}{path}", self.base)).basic_auth(USER, Some(&self.password))
    }

    fn post(&self, http: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        http.post(format!("{}{path}", self.base)).basic_auth(USER, Some(&self.password))
    }
}

fn new_password() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn serve(launch: &Launch, password: &str) -> std::io::Result<Child> {
    let config = json!({ "mcp": { "buccanet": { "type": "local", "command": [launch.exe, "mcp", "--profile", launch.profile], "environment": { "BUCCANET_HOME": launch.home }, "enabled": true } } });
    tokio::process::Command::new("opencode")
        .args(["serve", "--port", "0"])
        .env("OPENCODE_CONFIG_CONTENT", config.to_string())
        .env("OPENCODE_SERVER_USERNAME", USER)
        .env("OPENCODE_SERVER_PASSWORD", password)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// The address the server prints once it listens. Its later output is drained so it never
/// blocks on a full pipe.
async fn listening_on(stdout: ChildStdout) -> Result<String, Error> {
    let mut lines = BufReader::new(stdout).lines();
    let base = tokio::time::timeout(STARTUP, async {
        while let Some(line) = lines.next_line().await? {
            if let Some(start) = line.find("http://") {
                return Ok(line[start..].trim().trim_end_matches('/').to_string());
            }
        }
        Err::<String, Error>("opencode serve exited before listening".into())
    })
    .await
    .map_err(|_| "opencode serve did not start")??;
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok(base)
}

pub async fn launch(launch: &Launch, extra: &[String]) -> Result<ExitStatus, Error> {
    let password = new_password();
    let mut child = serve(launch, &password)?;
    let stdout = child.stdout.take().ok_or("opencode serve has no stdout")?;
    let server = Server { base: listening_on(stdout).await?, password };
    let http = reqwest::Client::new();
    wait_for(&http, &server).await?;
    let mut wakes = socket::listen(&launch.wake_socket())?;
    let target = server.clone();
    tokio::spawn(async move {
        while let Some(batch) = next_batch(&mut wakes).await {
            if let Err(error) = deliver(&http, &target, &prompt(&batch)).await {
                tracing::warn!(%error, "opencode wake not delivered");
            }
        }
    });
    let status = tokio::process::Command::new("opencode")
        .args(["attach", &server.base])
        .args(extra)
        .env("OPENCODE_SERVER_USERNAME", USER)
        .env("OPENCODE_SERVER_PASSWORD", &server.password)
        .status()
        .await?;
    let _ = child.kill().await;
    Ok(status)
}

/// Waits until the server answers an authenticated request, before anything is delivered.
async fn wait_for(http: &reqwest::Client, server: &Server) -> Result<(), Error> {
    for _ in 0..150 {
        if server.get(http, "/session").send().await.is_ok_and(|response| response.status().is_success()) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("opencode serve did not accept its password".into())
}

/// The session updated last: the one the TUI shows.
async fn latest_session(http: &reqwest::Client, server: &Server) -> Result<Option<String>, Error> {
    let sessions: Vec<Value> = server.get(http, "/session").send().await?.error_for_status()?.json().await?;
    let latest = sessions.iter().max_by_key(|session| session.pointer("/time/updated").and_then(Value::as_u64).unwrap_or(0));
    Ok(latest.and_then(|session| session.get("id")).and_then(Value::as_str).map(str::to_string))
}

async fn deliver(http: &reqwest::Client, server: &Server, text: &str) -> Result<(), Error> {
    let session = latest_session(http, server).await?.ok_or("no OpenCode session is open yet")?;
    let body = json!({ "parts": [{ "type": "text", "text": text }] });
    server.post(http, &format!("/session/{session}/prompt_async")).json(&body).send().await?.error_for_status()?;
    Ok(())
}
