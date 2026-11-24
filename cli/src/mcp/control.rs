//! One process owns a profile; harnesses may still start several MCP servers for it (OpenClaw
//! starts one per worker). The owner answers tool calls on the profile's `control.sock`, and
//! every other instance forwards its calls there instead of failing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use buccanet_sdk::Client;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::rpc::{INVALID_PARAMS, RpcError};
use super::tools::{self, ToolContext, ToolOutput};
use crate::profile::Profile;

const OWNER_GONE: i64 = -32603;

pub enum Backend {
    /// This process owns the profile.
    Local(Arc<ToolContext>),
    /// Another process owns it; calls go to its control socket.
    Remote(PathBuf),
}

impl Backend {
    pub async fn call(&self, name: &str, args: &Value) -> Result<ToolOutput, RpcError> {
        match self {
            Backend::Local(ctx) => tools::call(ctx, name, args).await,
            Backend::Remote(socket) => forward(socket, name, args).await,
        }
    }

    /// The SDK client, for what only the owner can do (event streams).
    pub fn local(&self) -> Result<&ToolContext, RpcError> {
        match self {
            Backend::Local(ctx) => Ok(ctx),
            Backend::Remote(_) => Err(RpcError::new(OWNER_GONE, "Events are served by the MCP process that owns this profile.")),
        }
    }
}

/// Every long running process that holds a profile (`tail`, `chat`, the terminal UI) answers the
/// other commands of that profile on its control socket.
pub fn serve_profile(profile: &Profile, client: &Client) -> std::io::Result<()> {
    let ctx = Arc::new(ToolContext { client: client.clone(), config: profile.config.clone(), profile_dir: profile.dir.clone() });
    serve(ctx, &profile.control_socket())
}

/// Owner side: answers one `{name, arguments}` line per connection.
pub fn serve(ctx: Arc<ToolContext>, socket: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let ctx = ctx.clone();
            tokio::spawn(async move { answer(ctx, stream).await });
        }
    });
    Ok(())
}

async fn answer(ctx: Arc<ToolContext>, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let Ok(Some(line)) = BufReader::new(read).lines().next_line().await else { return };
    let request: Value = serde_json::from_str(&line).unwrap_or_default();
    let name = request.get("name").and_then(Value::as_str).unwrap_or_default();
    let reply = match tools::call(&ctx, name, request.get("arguments").unwrap_or(&json!({}))).await {
        Ok(output) => json!({ "ok": output }),
        Err(error) => json!({ "error": { "code": error.code, "message": error.message } }),
    };
    let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
}

async fn forward(socket: &Path, name: &str, args: &Value) -> Result<ToolOutput, RpcError> {
    let unavailable = |error: std::io::Error| RpcError::new(OWNER_GONE, format!("The profile owner is not reachable: {error}"));
    let stream = UnixStream::connect(socket).await.map_err(unavailable)?;
    let (read, mut write) = stream.into_split();
    let request = json!({ "name": name, "arguments": args });
    write.write_all(format!("{request}\n").as_bytes()).await.map_err(unavailable)?;
    let line = BufReader::new(read).lines().next_line().await.map_err(unavailable)?.unwrap_or_default();
    let reply: Value = serde_json::from_str(&line).map_err(|_| RpcError::new(OWNER_GONE, "The profile owner closed the call."))?;
    if let Some(output) = reply.get("ok") {
        return serde_json::from_value(output.clone()).map_err(|_| RpcError::new(OWNER_GONE, "Unreadable tool result."));
    }
    let code = reply.pointer("/error/code").and_then(Value::as_i64).unwrap_or(INVALID_PARAMS);
    let message = reply.pointer("/error/message").and_then(Value::as_str).unwrap_or("Tool call failed.");
    Err(RpcError::new(code, message))
}
