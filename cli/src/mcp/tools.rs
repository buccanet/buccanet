//! The tools every harness gets, whatever its MCP era.

use buccanet_sdk::{ChatMessage, Client, RoomPolicy};
use serde_json::{Value, json};

use super::rpc::{INVALID_PARAMS, RpcError};
use crate::commands::output::kind_label;
use crate::profile::ProfileConfig;
use crate::text;

/// The room policy bounds the relay accepts.
const MAX_MEMBERS: std::ops::RangeInclusive<u64> = 2..=1000;
const AGENT_BURST: std::ops::RangeInclusive<u64> = 1..=10_000;

pub struct ToolContext {
    pub client: Client,
    pub config: ProfileConfig,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Value,
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(text: impl Into<String>, structured: Value) -> Self {
        Self { text: text.into(), structured, is_error: false }
    }

    fn failed(text: impl Into<String>) -> Self {
        let text = text.into();
        Self { structured: json!({ "error": text }), text, is_error: true }
    }

    pub fn content(&self) -> Value {
        json!([{ "type": "text", "text": self.text }])
    }
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false },
    })
}

pub fn definitions() -> Vec<Value> {
    let room = json!({ "type": "string", "description": "Room id, or a unique prefix of it" });
    vec![
        tool("list_rooms", "List the rooms you are in, with their members.", json!({}), &[]),
        tool(
            "create_room",
            "Create an end to end encrypted room and get an invite link to share.",
            json!({
                "title": { "type": "string" },
                "approval": { "type": "boolean", "description": "Joins wait for an admin decision (default true)" },
                "maxMembers": { "type": "integer", "minimum": 2, "maximum": 1000 },
                "agentBurst": { "type": "integer", "minimum": 1, "maximum": 10_000, "description": "Consecutive agent messages allowed before a person speaks" },
            }),
            &[],
        ),
        tool("create_invite", "Create a reusable invite link for a room.", json!({ "roomId": room }), &["roomId"]),
        tool(
            "join_room",
            "Ask to join a room from an invite link. You are told when an admin lets you in.",
            json!({ "invite": { "type": "string", "description": "Invite link such as https://…/join/<token>" } }),
            &["invite"],
        ),
        tool(
            "read_messages",
            "Read recent messages of a room, oldest first.",
            json!({
                "roomId": room,
                "afterSeq": { "type": "integer", "minimum": 0, "description": "Only messages after this seq" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
            }),
            &["roomId"],
        ),
        tool("send_message", "Send a message to a room.", json!({ "roomId": room, "text": { "type": "string" } }), &["roomId", "text"]),
        tool("list_requests", "List pending join requests of a room you administer.", json!({ "roomId": room }), &["roomId"]),
        tool(
            "approve_member",
            "Let a pending requester into a room you administer. Refused unless your owner enabled agent approvals.",
            json!({ "roomId": room, "requestId": { "type": "string" } }),
            &["roomId", "requestId"],
        ),
        tool(
            "reject_member",
            "Refuse a pending join request.",
            json!({ "roomId": room, "requestId": { "type": "string" } }),
            &["roomId", "requestId"],
        ),
        tool("list_contacts", "Everyone you share a room with.", json!({}), &[]),
    ]
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, RpcError> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("{key} is required")))
}

pub fn format_message(message: &ChatMessage) -> String {
    let shared = if message.shared { " (shared history)" } else { "" };
    format!(
        "#{} {} ({}){shared}: {}",
        message.seq,
        text::line(&message.author_name),
        kind_label(message.author_kind),
        text::indented(&message.text)
    )
}

async fn room_id(ctx: &ToolContext, args: &Value) -> Result<Result<String, String>, RpcError> {
    let wanted = arg(args, "roomId")?;
    let matches: Vec<String> = ctx.client.rooms().await.into_iter().map(|room| room.room_id).filter(|id| id.starts_with(wanted)).collect();
    Ok(match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("You are not in a room matching {wanted}.")),
        _ => Err(format!("{wanted} matches several rooms; use the full id.")),
    })
}

/// Runs a tool. Unknown tools and missing arguments are protocol errors; failures are tool results.
pub async fn call(ctx: &ToolContext, name: &str, args: &Value) -> Result<ToolOutput, RpcError> {
    match name {
        "list_rooms" => Ok(list_rooms(ctx).await),
        "create_room" => Ok(create_room(ctx, args).await),
        "create_invite" => with_room(ctx, args, |ctx, room| Box::pin(create_invite(ctx, room))).await,
        "join_room" => Ok(join_room(ctx, arg(args, "invite")?).await),
        "read_messages" => {
            let after = args.get("afterSeq").and_then(Value::as_u64).unwrap_or(0);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(30) as usize;
            with_room(ctx, args, move |ctx, room| Box::pin(read_messages(ctx, room, after, limit))).await
        }
        "send_message" => {
            let text = arg(args, "text")?.to_string();
            with_room(ctx, args, move |ctx, room| Box::pin(send_message(ctx, room, text))).await
        }
        "list_requests" => with_room(ctx, args, |ctx, room| Box::pin(list_requests(ctx, room))).await,
        "approve_member" | "reject_member" => {
            let request = arg(args, "requestId")?.to_string();
            let approve = name == "approve_member";
            with_room(ctx, args, move |ctx, room| Box::pin(decide(ctx, room, request, approve))).await
        }
        "list_contacts" => Ok(list_contacts(ctx).await),
        other => Err(RpcError::new(INVALID_PARAMS, format!("Unknown tool: {other}"))),
    }
}

type ToolFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ToolOutput> + Send + 'a>>;

async fn with_room<'a, F>(ctx: &'a ToolContext, args: &Value, run: F) -> Result<ToolOutput, RpcError>
where
    F: FnOnce(&'a ToolContext, String) -> ToolFuture<'a>,
{
    Ok(match room_id(ctx, args).await? {
        Ok(room) => run(ctx, room).await,
        Err(message) => ToolOutput::failed(message),
    })
}

async fn list_rooms(ctx: &ToolContext) -> ToolOutput {
    let rooms = ctx.client.rooms().await;
    if rooms.is_empty() {
        return ToolOutput::ok("You are in no room yet.", json!({ "rooms": [] }));
    }
    let lines: Vec<String> = rooms
        .iter()
        .map(|room| {
            let members: Vec<String> =
                room.members.iter().map(|member| format!("{} ({})", text::line(&member.name), kind_label(member.kind))).collect();
            let title = room.title.as_deref().map_or_else(|| "untitled".to_string(), text::line);
            format!("{} {} admin={} members: {}", room.room_id, Value::from(title), room.admin, members.join(", "))
        })
        .collect();
    ToolOutput::ok(lines.join("\n"), json!({ "rooms": rooms }))
}

/// An optional integer argument inside the bounds the relay accepts.
fn bounded(args: &Value, key: &str, range: std::ops::RangeInclusive<u64>, default: u32) -> Result<u32, String> {
    let Some(value) = args.get(key) else { return Ok(default) };
    let out_of_range = || format!("{key} must be an integer from {} to {}.", range.start(), range.end());
    let number = value.as_u64().filter(|number| range.contains(number)).ok_or_else(out_of_range)?;
    u32::try_from(number).map_err(|_| out_of_range())
}

fn room_policy(args: &Value) -> Result<RoomPolicy, String> {
    let defaults = RoomPolicy::default();
    Ok(RoomPolicy {
        approval: args.get("approval").and_then(Value::as_bool).unwrap_or(defaults.approval),
        max_members: bounded(args, "maxMembers", MAX_MEMBERS, defaults.max_members)?,
        agent_burst: bounded(args, "agentBurst", AGENT_BURST, defaults.agent_burst)?,
    })
}

async fn create_room(ctx: &ToolContext, args: &Value) -> ToolOutput {
    let title = args.get("title").and_then(Value::as_str).map(str::to_string);
    let policy = match room_policy(args) {
        Ok(policy) => policy,
        Err(message) => return ToolOutput::failed(message),
    };
    let room = match ctx.client.create_room(title, policy).await {
        Ok(room) => room,
        Err(error) => return ToolOutput::failed(error.to_string()),
    };
    match ctx.client.create_invite(&room.room_id).await {
        Ok(link) => {
            ToolOutput::ok(format!("Room {} created. Invite link: {link}", room.room_id), json!({ "roomId": room.room_id, "invite": link }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn create_invite(ctx: &ToolContext, room: String) -> ToolOutput {
    match ctx.client.create_invite(&room).await {
        Ok(link) => ToolOutput::ok(format!("Invite link: {link}"), json!({ "invite": link })),
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn join_room(ctx: &ToolContext, invite: &str) -> ToolOutput {
    match ctx.client.join(invite).await {
        Ok(pending) => ToolOutput::ok(
            format!("Join requested for room {}. You will be notified once an admin lets you in.", pending.room_id),
            json!({ "roomId": pending.room_id, "requestId": pending.request_id }),
        ),
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn read_messages(ctx: &ToolContext, room: String, after: u64, limit: usize) -> ToolOutput {
    match ctx.client.messages(&room, after).await {
        Ok(messages) => {
            let recent: Vec<ChatMessage> = messages.into_iter().rev().take(limit).rev().collect();
            let text = if recent.is_empty() {
                "No messages.".to_string()
            } else {
                recent.iter().map(format_message).collect::<Vec<_>>().join("\n")
            };
            ToolOutput::ok(text, json!({ "roomId": room, "messages": recent }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn send_message(ctx: &ToolContext, room: String, text: String) -> ToolOutput {
    match ctx.client.send_text(&room, &text).await {
        Ok(message) => ToolOutput::ok(format!("Sent as #{}.", message.seq), json!({ "roomId": room, "seq": message.seq })),
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn list_requests(ctx: &ToolContext, room: String) -> ToolOutput {
    match ctx.client.pending_requests(&room).await {
        Ok(requests) if requests.is_empty() => ToolOutput::ok("No pending request.", json!({ "requests": [] })),
        Ok(requests) => {
            let lines: Vec<String> = requests
                .iter()
                .map(|request| format!("{} {} ({})", text::line(&request.request_id), text::line(&request.name), kind_label(request.kind)))
                .collect();
            ToolOutput::ok(lines.join("\n"), json!({ "requests": requests }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn decide(ctx: &ToolContext, room: String, request: String, approve: bool) -> ToolOutput {
    if approve && !ctx.config.allow_agent_approvals {
        return ToolOutput::failed(
            "Your owner has not enabled agent approvals. Ask them to approve from the Buccanet app or `bucca approve`.",
        );
    }
    let result = if approve { ctx.client.approve(&room, &request).await } else { ctx.client.reject(&room, &request).await };
    match result {
        Ok(()) => ToolOutput::ok(if approve { "Approved." } else { "Rejected." }, json!({ "roomId": room, "requestId": request })),
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn list_contacts(ctx: &ToolContext) -> ToolOutput {
    let me = ctx.client.device_id();
    let mut contacts: Vec<buccanet_sdk::MemberInfo> = Vec::new();
    for room in ctx.client.rooms().await {
        for member in room.members {
            if member.device_id != me && !contacts.iter().any(|known| known.device_id == member.device_id) {
                contacts.push(member);
            }
        }
    }
    let lines: Vec<String> = contacts
        .iter()
        .map(|contact| format!("{} ({}) {}", text::line(&contact.name), kind_label(contact.kind), contact.device_id))
        .collect();
    ToolOutput::ok(if lines.is_empty() { "No contacts yet.".to_string() } else { lines.join("\n") }, json!({ "contacts": contacts }))
}
