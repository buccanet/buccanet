//! JSON-RPC framing: newline delimited messages, one writer task owning stdout.

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub const PARSE_ERROR: i64 = -32700;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const UNSUPPORTED_VERSION: i64 = -32022;

#[derive(Debug)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Everything written to stdout goes through here, so lines never interleave.
#[derive(Clone)]
pub struct Out {
    sender: UnboundedSender<Value>,
}

impl Out {
    pub fn spawn() -> Self {
        let (sender, mut receiver) = unbounded_channel::<Value>();
        tokio::spawn(async move {
            let mut stdout = tokio::io::stdout();
            while let Some(message) = receiver.recv().await {
                let mut line = serde_json::to_vec(&message).expect("JSON values always serialize");
                line.push(b'\n');
                if stdout.write_all(&line).await.is_err() || stdout.flush().await.is_err() {
                    break;
                }
            }
        });
        Self { sender }
    }

    pub fn result(&self, id: &Value, result: Value) {
        let _ = self.sender.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    pub fn error(&self, id: &Value, error: RpcError) {
        let mut body = json!({ "code": error.code, "message": error.message });
        if let Some(data) = error.data {
            body["data"] = data;
        }
        let _ = self.sender.send(json!({ "jsonrpc": "2.0", "id": id, "error": body }));
    }

    pub fn notify(&self, method: &str, params: Value) {
        let _ = self.sender.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }
}
