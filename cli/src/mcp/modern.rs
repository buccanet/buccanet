//! Revision 2026-07-28: no handshake, version and capabilities in every request's `_meta`.

use serde_json::{Value, json};

use super::control::Backend;
use super::events::{self, EventHub};
use super::rpc::{INVALID_PARAMS, METHOD_NOT_FOUND, Out, RpcError, UNSUPPORTED_VERSION};
use super::tools;
use super::{MODERN_VERSION, SERVER_NAME, instructions};

const VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const TOOLS_TTL_MS: u64 = 300_000;

pub fn version_of(params: &Value) -> Option<&str> {
    params.pointer("/_meta").and_then(|meta| meta.get(VERSION_META)).and_then(Value::as_str)
}

fn check_version(params: &Value) -> Result<(), RpcError> {
    match version_of(params) {
        Some(MODERN_VERSION) => Ok(()),
        Some(other) => Err(RpcError::new(UNSUPPORTED_VERSION, "Unsupported protocol version")
            .with_data(json!({ "supported": [MODERN_VERSION], "requested": other }))),
        None => Err(RpcError::new(INVALID_PARAMS, "_meta io.modelcontextprotocol/protocolVersion is required")),
    }
}

fn complete(mut result: Value) -> Value {
    result["resultType"] = json!("complete");
    result["_meta"] = json!({ "io.modelcontextprotocol/serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") } });
    result
}

fn discover() -> Value {
    json!({
        "supportedVersions": [MODERN_VERSION],
        "capabilities": { "tools": {}, "extensions": { "io.modelcontextprotocol/events": {} } },
        "instructions": instructions(),
        "ttlMs": TOOLS_TTL_MS,
        "cacheScope": "private",
    })
}

/// One JSON-RPC request as the client sent it.
pub struct Request<'a> {
    pub id: &'a Value,
    pub method: &'a str,
    pub params: &'a Value,
}

/// Answers one modern request. `events/stream` and `subscriptions/listen` stay open: they
/// answer nothing now and deliver notifications until the client cancels.
pub async fn handle(backend: &Backend, hub: &EventHub, out: &Out, request: Request<'_>) -> Result<Option<Value>, RpcError> {
    let Request { id, method, params } = request;
    check_version(params)?;
    let result = match method {
        "server/discover" => discover(),
        "tools/list" => json!({ "tools": tools::definitions(), "ttlMs": TOOLS_TTL_MS, "cacheScope": "private" }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let output = backend.call(name, params.get("arguments").unwrap_or(&json!({}))).await?;
            json!({ "content": output.content(), "structuredContent": output.structured, "isError": output.is_error })
        }
        "events/list" => events::list(),
        "events/poll" => events::poll(&backend.local()?.client, params).await?,
        "events/stream" => {
            hub.stream(&backend.local()?.client, id, params).await?;
            return Ok(None);
        }
        "subscriptions/listen" => {
            let filter = params.get("notifications").cloned().unwrap_or(json!({}));
            out.notify(
                "notifications/subscriptions/acknowledged",
                json!({ "_meta": { "io.modelcontextprotocol/subscriptionId": id }, "notifications": filter }),
            );
            return Ok(None);
        }
        other => return Err(RpcError::new(METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    };
    Ok(Some(complete(result)))
}
