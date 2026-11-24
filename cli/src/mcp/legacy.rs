//! The `initialize` era (2025-xx): what Claude Code channels and most harnesses speak today.

use serde_json::{Value, json};

use super::rpc::{METHOD_NOT_FOUND, RpcError};
use super::control::Backend;
use super::tools;
use super::{INSTRUCTIONS, LEGACY_VERSIONS, SERVER_NAME};

pub fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = if LEGACY_VERSIONS.contains(&requested) { requested } else { LEGACY_VERSIONS[0] };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {}, "experimental": { "claude/channel": {} } },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

pub async fn handle(backend: &Backend, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let output = backend.call(name, params.get("arguments").unwrap_or(&json!({}))).await?;
            Ok(json!({ "content": output.content(), "structuredContent": output.structured, "isError": output.is_error }))
        }
        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    }
}
