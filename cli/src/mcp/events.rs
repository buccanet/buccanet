//! The Events extension (SEP-3415): `buccanet.message` (push and poll) and `buccanet.room` (push).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buccanet_sdk::{ChatMessage, Client};
use serde_json::{Value, json};

use super::cursor::{self, Positions};
use super::rpc::{INVALID_PARAMS, Out, RpcError};

pub const MESSAGE_EVENT: &str = "buccanet.message";
pub const ROOM_EVENT: &str = "buccanet.room";
const SUBSCRIPTION_META: &str = "io.modelcontextprotocol/subscriptionId";
const NOT_FOUND: i64 = -32023;
const DEFAULT_HEARTBEAT_MS: u64 = 30_000;
const MIN_HEARTBEAT_MS: u64 = 5_000;
const POLL_INTERVAL_MS: u64 = 5_000;

struct Subscription {
    id: Value,
    name: String,
    room: Option<String>,
    positions: Positions,
}

#[derive(Clone)]
pub struct EventHub {
    subscriptions: Arc<Mutex<HashMap<String, Subscription>>>,
    out: Out,
}

pub fn list() -> Value {
    let room_filter = json!({ "type": "object", "properties": { "roomId": { "type": "string" } }, "additionalProperties": false });
    let author = json!({ "type": "object", "properties": { "deviceId": { "type": "string" }, "name": { "type": "string" }, "kind": { "type": "string" } } });
    json!({ "events": [
        {
            "name": MESSAGE_EVENT,
            "description": "A member of one of your rooms posted a message.",
            "delivery": ["push", "poll"],
            "inputSchema": room_filter,
            "payloadSchema": { "type": "object", "properties": {
                "roomId": { "type": "string" }, "seq": { "type": "integer" }, "from": author, "text": { "type": "string" }, "at": { "type": "string" }
            } },
        },
        {
            "name": ROOM_EVENT,
            "description": "You joined or left a room, or someone asks to join a room you administer.",
            "delivery": ["push"],
            "inputSchema": room_filter,
            "payloadSchema": { "type": "object", "properties": {
                "roomId": { "type": "string" }, "change": { "type": "string", "enum": ["joined", "left", "join_requested"] }, "detail": { "type": "string" }
            } },
        },
    ] })
}

fn message_payload(message: &ChatMessage) -> Value {
    json!({
        "roomId": message.room_id,
        "seq": message.seq,
        "from": { "deviceId": message.sender_device_id, "name": message.author_name, "kind": message.author_kind },
        "text": message.text,
        "at": message.at,
    })
}

fn occurrence(name: &str, event_id: String, data: Value, cursor: Option<String>) -> Value {
    let mut value = json!({ "eventId": event_id, "name": name, "timestamp": buccanet_sdk_now(), "data": data });
    if let Some(cursor) = cursor {
        value["cursor"] = json!(cursor);
    }
    value
}

fn buccanet_sdk_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn subscription_name(params: &Value) -> Result<String, RpcError> {
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| RpcError::new(INVALID_PARAMS, "name is required"))?;
    if name != MESSAGE_EVENT && name != ROOM_EVENT {
        return Err(RpcError::new(NOT_FOUND, "NotFound").with_data(json!({ "reason": format!("unknown event {name}") })));
    }
    Ok(name.to_string())
}

fn room_filter(params: &Value) -> Option<String> {
    params.pointer("/arguments/roomId").and_then(Value::as_str).map(str::to_string)
}

/// Last seq of every room the client knows: the "start from now" position.
async fn current_positions(client: &Client) -> Positions {
    client.rooms().await.into_iter().map(|room| (room.room_id, room.last_seq)).collect()
}

/// Messages from others after the given positions, oldest first.
async fn missed(client: &Client, positions: &Positions, room: Option<&str>) -> Vec<ChatMessage> {
    let mut found = Vec::new();
    for info in client.rooms().await {
        if room.is_some_and(|wanted| wanted != info.room_id) {
            continue;
        }
        let after = positions.get(&info.room_id).copied().unwrap_or(0);
        if let Ok(messages) = client.messages(&info.room_id, after).await {
            found.extend(messages.into_iter().filter(|message| !message.own && !message.shared));
        }
    }
    found.sort_by(|a, b| a.at.cmp(&b.at));
    found
}

impl EventHub {
    pub fn new(out: Out) -> Self {
        Self { subscriptions: Arc::new(Mutex::new(HashMap::new())), out }
    }

    fn send(&self, method: &str, id: &Value, mut params: Value) {
        params["_meta"] = json!({ SUBSCRIPTION_META: id });
        self.out.notify(method, params);
    }

    /// `events/stream`: acknowledges, replays from the cursor, then pushes until cancelled.
    pub async fn stream(&self, client: &Client, id: &Value, params: &Value) -> Result<(), RpcError> {
        let name = subscription_name(params)?;
        let room = room_filter(params);
        let requested = params.get("cursor").and_then(Value::as_str).and_then(cursor::decode);
        let heartbeat = params.get("heartbeatMs").and_then(Value::as_u64).unwrap_or(DEFAULT_HEARTBEAT_MS).max(MIN_HEARTBEAT_MS);
        let (positions, replay) = match requested {
            Some(from) if name == MESSAGE_EVENT => {
                let replay = missed(client, &from, room.as_deref()).await;
                (from, replay)
            }
            _ => (current_positions(client).await, Vec::new()),
        };
        self.send("notifications/events/active", id, json!({ "cursor": cursor::encode(&positions), "truncated": false, "heartbeatMs": heartbeat }));
        let key = id.to_string();
        self.subscriptions.lock().expect("subscriptions lock").insert(key.clone(), Subscription { id: id.clone(), name, room, positions });
        for message in &replay {
            self.publish_message(message);
        }
        self.spawn_heartbeat(key, heartbeat);
        Ok(())
    }

    fn spawn_heartbeat(&self, key: String, every_ms: u64) {
        let hub = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(every_ms)).await;
                let beat = {
                    let subscriptions = hub.subscriptions.lock().expect("subscriptions lock");
                    subscriptions.get(&key).map(|sub| (sub.id.clone(), cursor::encode(&sub.positions)))
                };
                let Some((id, cursor)) = beat else { break };
                hub.send("notifications/events/heartbeat", &id, json!({ "cursor": cursor }));
            }
        });
    }

    /// `notifications/cancelled` for a stream: stop delivering.
    pub fn cancel(&self, request_id: &Value) {
        self.subscriptions.lock().expect("subscriptions lock").remove(&request_id.to_string());
    }

    pub fn publish_message(&self, message: &ChatMessage) {
        let mut subscriptions = self.subscriptions.lock().expect("subscriptions lock");
        for sub in subscriptions.values_mut() {
            if sub.name != MESSAGE_EVENT || sub.room.as_ref().is_some_and(|room| room != &message.room_id) {
                continue;
            }
            let position = sub.positions.entry(message.room_id.clone()).or_insert(0);
            *position = (*position).max(message.seq);
            let event = occurrence(MESSAGE_EVENT, format!("{}:{}", message.room_id, message.seq), message_payload(message), Some(cursor::encode(&sub.positions)));
            let mut params = event;
            params["_meta"] = json!({ SUBSCRIPTION_META: sub.id });
            self.out.notify("notifications/events/event", params);
        }
    }

    pub fn publish_room(&self, room_id: &str, change: &str, detail: &str) {
        let subscriptions = self.subscriptions.lock().expect("subscriptions lock");
        for sub in subscriptions.values() {
            if sub.name != ROOM_EVENT || sub.room.as_ref().is_some_and(|room| room != room_id) {
                continue;
            }
            let data = json!({ "roomId": room_id, "change": change, "detail": detail });
            let mut params = occurrence(ROOM_EVENT, format!("{room_id}:{change}:{}", uuid::Uuid::new_v4()), data, None);
            params["_meta"] = json!({ SUBSCRIPTION_META: sub.id });
            self.out.notify("notifications/events/event", params);
        }
    }
}

/// `events/poll` for `buccanet.message`.
pub async fn poll(client: &Client, params: &Value) -> Result<Value, RpcError> {
    let name = subscription_name(params)?;
    if name != MESSAGE_EVENT {
        return Err(RpcError::new(-32026, "Unsupported").with_data(json!({ "reason": "buccanet.room is push only" })));
    }
    let room = room_filter(params);
    let max = params.get("maxEvents").and_then(Value::as_u64).unwrap_or(50).clamp(1, 500) as usize;
    let Some(mut positions) = params.get("cursor").and_then(Value::as_str).and_then(cursor::decode) else {
        let now = current_positions(client).await;
        return Ok(json!({ "events": [], "cursor": cursor::encode(&now), "truncated": false, "hasMore": false, "nextPollMs": POLL_INTERVAL_MS }));
    };
    let missed = missed(client, &positions, room.as_deref()).await;
    let has_more = missed.len() > max;
    let events: Vec<Value> = missed
        .into_iter()
        .take(max)
        .map(|message| {
            let position = positions.entry(message.room_id.clone()).or_insert(0);
            *position = (*position).max(message.seq);
            occurrence(MESSAGE_EVENT, format!("{}:{}", message.room_id, message.seq), message_payload(&message), None)
        })
        .collect();
    Ok(json!({ "events": events, "cursor": cursor::encode(&positions), "truncated": false, "hasMore": has_more, "nextPollMs": POLL_INTERVAL_MS }))
}
