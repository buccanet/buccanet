//! Codex wakes through its app-server: the TUI attaches to an app-server we start, and each
//! batch of wakes is queued on the TUI's thread with `codex queue --resume`.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;

use super::{next_batch, prompt, socket};
use crate::commands::run::Launch;

type Error = Box<dyn std::error::Error + Send + Sync>;

fn remote(sock: &Path) -> String {
    format!("unix://{}", sock.display())
}

pub async fn launch(launch: &Launch, extra: &[String]) -> Result<ExitStatus, Error> {
    let sock = launch.dir.join("codex.sock");
    let _ = std::fs::remove_file(&sock);
    let mut server = tokio::process::Command::new("codex")
        .args(["app-server", "--listen", &remote(&sock)])
        .args(["-c", &format!("mcp_servers.buccanet.command=\"{}\"", launch.exe)])
        .args(["-c", &format!("mcp_servers.buccanet.args=[\"mcp\", \"--profile\", \"{}\"]", launch.profile)])
        .args(["-c", &format!("mcp_servers.buccanet.env={{ BUCCANET_HOME = \"{}\" }}", launch.home)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    wait_for(&sock).await?;
    let mut wakes = socket::listen(&launch.wake_socket())?;
    let target = sock.clone();
    tokio::spawn(async move {
        while let Some(batch) = next_batch(&mut wakes).await {
            if let Err(error) = deliver(&target, &prompt(&batch)).await {
                tracing::warn!(%error, "codex wake not delivered");
            }
        }
    });
    let status = tokio::process::Command::new("codex").args(["--remote", &remote(&sock)]).args(extra).status().await?;
    let _ = server.kill().await;
    Ok(status)
}

async fn wait_for(sock: &PathBuf) -> Result<(), Error> {
    for _ in 0..100 {
        if UnixStream::connect(sock).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("codex app-server did not start".into())
}

/// Queues the prompt on the thread the TUI has loaded.
async fn deliver(sock: &Path, text: &str) -> Result<(), Error> {
    let thread = loaded_thread(sock).await?.ok_or("no Codex thread is open yet")?;
    let status = tokio::process::Command::new("codex")
        .args(["queue", "--thread", &thread, "--message", text, "--resume", "--remote", &remote(sock)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await?;
    if !status.success() {
        return Err(format!("codex queue exited with {status}").into());
    }
    Ok(())
}

/// Asks the app-server (WebSocket over its unix socket) which thread is loaded.
async fn loaded_thread(sock: &Path) -> Result<Option<String>, Error> {
    let stream = UnixStream::connect(sock).await?;
    let (mut ws, _) = tokio_tungstenite::client_async("ws://localhost/", stream).await?;
    let initialize = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "buccanet", "version": env!("CARGO_PKG_VERSION") } } });
    ws.send(Message::text(initialize.to_string())).await?;
    response(&mut ws, 1).await?;
    ws.send(Message::text(json!({ "jsonrpc": "2.0", "method": "initialized" }).to_string())).await?;
    ws.send(Message::text(json!({ "jsonrpc": "2.0", "id": 2, "method": "thread/loaded/list", "params": {} }).to_string())).await?;
    let result = response(&mut ws, 2).await?;
    let _ = ws.close(None).await;
    Ok(result.pointer("/data/0").and_then(Value::as_str).map(str::to_string))
}

async fn response<S>(ws: &mut S, id: i64) -> Result<Value, Error>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let wait = async {
        while let Some(message) = ws.next().await {
            let Message::Text(text) = message? else { continue };
            let value: Value = serde_json::from_str(&text)?;
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                return value.get("result").cloned().ok_or_else(|| Error::from(value.to_string()));
            }
        }
        Err("app-server closed the connection".into())
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await?
}
