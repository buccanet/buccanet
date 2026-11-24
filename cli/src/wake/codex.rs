//! Codex wakes through its app-server: the TUI attaches to an app-server we start, and each
//! batch of wakes starts a turn on the TUI's thread (or joins the running one) through the
//! app-server's own `turn/start`, so the prompt never shows in a process list.

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

/// A TOML string: JSON string escapes are valid TOML basic string escapes.
fn toml_string(value: &str) -> String {
    Value::from(value).to_string()
}

/// The `-c` overrides that give the app-server the buccanet MCP server.
fn mcp_overrides(launch: &Launch) -> [String; 3] {
    let args = format!("[{}, {}, {}]", toml_string("mcp"), toml_string("--profile"), toml_string(&launch.profile));
    [
        format!("mcp_servers.buccanet.command={}", toml_string(&launch.exe)),
        format!("mcp_servers.buccanet.args={args}"),
        format!("mcp_servers.buccanet.env={{ BUCCANET_HOME = {} }}", toml_string(&launch.home)),
    ]
}

pub async fn launch(launch: &Launch, extra: &[String]) -> Result<ExitStatus, Error> {
    let sock = launch.dir.join("codex.sock");
    let _ = std::fs::remove_file(&sock);
    let mut server = tokio::process::Command::new("codex")
        .args(["app-server", "--listen", &remote(&sock)])
        .args(mcp_overrides(launch).iter().flat_map(|value| ["-c", value.as_str()]))
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

type Socket = tokio_tungstenite::WebSocketStream<UnixStream>;

/// Sends one request and waits for its answer.
async fn request(ws: &mut Socket, id: i64, method: &str, params: Value) -> Result<Value, Error> {
    ws.send(Message::text(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string())).await?;
    response(ws, id).await
}

/// A WebSocket session on the app-server's unix socket, past the initialize handshake.
async fn connect(sock: &Path) -> Result<Socket, Error> {
    let stream = UnixStream::connect(sock).await?;
    let (mut ws, _) = tokio_tungstenite::client_async("ws://localhost/", stream).await?;
    request(&mut ws, 1, "initialize", json!({ "clientInfo": { "name": "buccanet", "version": env!("CARGO_PKG_VERSION") } })).await?;
    ws.send(Message::text(json!({ "jsonrpc": "2.0", "method": "initialized" }).to_string())).await?;
    Ok(ws)
}

/// Starts a turn with the prompt on the thread the TUI has loaded; while a turn runs, the
/// app-server adds the prompt to it.
async fn deliver(sock: &Path, text: &str) -> Result<(), Error> {
    let mut ws = connect(sock).await?;
    let loaded = request(&mut ws, 2, "thread/loaded/list", json!({})).await?;
    let thread = loaded.pointer("/data/0").and_then(Value::as_str).ok_or("no Codex thread is open yet")?.to_string();
    let input = json!([{ "type": "text", "text": text, "text_elements": [] }]);
    request(&mut ws, 3, "turn/start", json!({ "threadId": thread, "input": input })).await?;
    let _ = ws.close(None).await;
    Ok(())
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
