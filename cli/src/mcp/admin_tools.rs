//! The tools of a room admin: rules, join requests and who else administers the room. An
//! agent uses the ones that change a room only when its owner enabled agent approvals.

use buccanet_sdk::{DeviceKind, MemberInfo};
use serde_json::{Value, json};

use super::rpc::{INVALID_PARAMS, RpcError};
use super::tools::{ToolContext, ToolOutput, arg, tool, with_room};
use crate::commands::output::kind_label;
use crate::guard;
use buccanet_agent::text;

const NOT_ALLOWED: &str = "Your owner has not enabled agent approvals. Ask them to do this from the Buccanet app or `bucca`.";

pub fn definitions(room: &Value) -> Vec<Value> {
    let rules = tool(
        "set_room_rules",
        "Replace the rules of a room you administer; every member receives them. Refused unless your owner enabled agent approvals.",
        json!({ "roomId": room, "text": { "type": "string", "description": "The new rules, in Markdown" } }),
        &["roomId", "text"],
    );
    std::iter::once(rules).chain(request_tools(room)).chain(member_tools(room)).collect()
}

/// The tools that read and decide join requests.
fn request_tools(room: &Value) -> Vec<Value> {
    vec![
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
    ]
}

/// The tools that remove members and change who administers the room.
fn member_tools(room: &Value) -> Vec<Value> {
    vec![
        tool(
            "remove_member",
            "Remove a member from a room you administer. Refused unless your owner enabled agent approvals.",
            json!({
                "roomId": room,
                "member": { "type": "string", "description": "The member's name, device id or a unique prefix of its device id" },
            }),
            &["roomId", "member"],
        ),
        tool(
            "set_admin",
            "Make a member an admin of a room you administer, or take the role back with admin false. \
             Refused unless your owner enabled agent approvals.",
            json!({
                "roomId": room,
                "member": { "type": "string", "description": "The member's name, device id or a unique prefix of its device id" },
                "admin": { "type": "boolean", "description": "True to make the member admin (default), false to take the role back" },
            }),
            &["roomId", "member"],
        ),
    ]
}

pub async fn call(ctx: &ToolContext, name: &str, args: &Value) -> Result<ToolOutput, RpcError> {
    match name {
        "set_room_rules" => {
            let text = arg(args, "text")?.to_string();
            with_room(ctx, args, move |ctx, room| Box::pin(set_room_rules(ctx, room, text))).await
        }
        "list_requests" => with_room(ctx, args, |ctx, room| Box::pin(list_requests(ctx, room))).await,
        "approve_member" | "reject_member" => {
            let request = arg(args, "requestId")?.to_string();
            let approve = name == "approve_member";
            with_room(ctx, args, move |ctx, room| Box::pin(decide(ctx, room, request, approve))).await
        }
        "remove_member" => {
            let member = arg(args, "member")?.to_string();
            with_room(ctx, args, move |ctx, room| Box::pin(remove_member(ctx, room, member))).await
        }
        "set_admin" => {
            let member = arg(args, "member")?.to_string();
            let admin = args.get("admin").and_then(Value::as_bool).unwrap_or(true);
            with_room(ctx, args, move |ctx, room| Box::pin(set_admin(ctx, room, member, admin))).await
        }
        other => Err(RpcError::new(INVALID_PARAMS, format!("Unknown tool: {other}"))),
    }
}

/// Whether the owner lets this profile change a room: always for a person, on approval for an agent.
fn may_change_rooms(ctx: &ToolContext) -> bool {
    ctx.config.kind == DeviceKind::Human || ctx.config.allow_agent_approvals
}

async fn set_room_rules(ctx: &ToolContext, room: String, text: String) -> ToolOutput {
    if !may_change_rooms(ctx) {
        return ToolOutput::failed(NOT_ALLOWED);
    }
    if let Err(leak) = guard::check_sender(ctx.config.kind, &text) {
        return ToolOutput::failed(leak.to_string());
    }
    match ctx.client.set_room_rules(&room, &text).await {
        Ok(info) => {
            let version = info.rules.map_or(0, |rules| rules.version);
            ToolOutput::ok(format!("The rules of room {room} are now version {version}."), json!({ "roomId": room, "version": version }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn list_requests(ctx: &ToolContext, room: String) -> ToolOutput {
    match ctx.client.pending_requests(&room).await {
        Ok(requests) if requests.is_empty() => ToolOutput::ok("No pending request.", json!({ "requests": [] })),
        Ok(requests) => {
            let lines: Vec<String> = requests
                .iter()
                .map(|request| {
                    let (id, name, device) = (text::line(&request.request_id), text::line(&request.name), text::line(&request.device_id));
                    format!("{id} {name} ({}) device {device}", kind_label(request.kind))
                })
                .collect();
            ToolOutput::ok(lines.join("\n"), json!({ "requests": requests }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn decide(ctx: &ToolContext, room: String, request: String, approve: bool) -> ToolOutput {
    if approve && !may_change_rooms(ctx) {
        return ToolOutput::failed(NOT_ALLOWED);
    }
    let result = if approve { ctx.client.approve(&room, &request).await } else { ctx.client.reject(&room, &request).await };
    match result {
        Ok(()) => ToolOutput::ok(if approve { "Approved." } else { "Rejected." }, json!({ "roomId": room, "requestId": request })),
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

/// The device of the one member `wanted` names: its name, ignoring case, or its device id or a
/// prefix of it.
fn member_device(members: &[MemberInfo], wanted: &str) -> Result<String, String> {
    let matches: Vec<&MemberInfo> =
        members.iter().filter(|member| member.name.eq_ignore_ascii_case(wanted) || member.device_id.starts_with(wanted)).collect();
    match matches.as_slice() {
        [one] => Ok(one.device_id.clone()),
        [] => Err(format!("No member of this room matches {wanted}.")),
        _ => Err(format!("{wanted} matches several members; use a device id.")),
    }
}

/// The device of the member `wanted` names in `room`.
async fn room_member(ctx: &ToolContext, room: &str, wanted: &str) -> Result<String, String> {
    let info = ctx.client.room(room).await.map_err(|error| error.to_string())?;
    member_device(&info.members, wanted)
}

async fn remove_member(ctx: &ToolContext, room: String, member: String) -> ToolOutput {
    if !may_change_rooms(ctx) {
        return ToolOutput::failed(NOT_ALLOWED);
    }
    let device = match room_member(ctx, &room, &member).await {
        Ok(device) => device,
        Err(message) => return ToolOutput::failed(message),
    };
    match ctx.client.remove_member(&room, &device).await {
        Ok(()) => {
            ToolOutput::ok(format!("{} was removed from room {room}.", text::line(&member)), json!({ "roomId": room, "deviceId": device }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}

async fn set_admin(ctx: &ToolContext, room: String, member: String, admin: bool) -> ToolOutput {
    if !may_change_rooms(ctx) {
        return ToolOutput::failed(NOT_ALLOWED);
    }
    let device = match room_member(ctx, &room, &member).await {
        Ok(device) => device,
        Err(message) => return ToolOutput::failed(message),
    };
    match ctx.client.set_admin(&room, &device, admin).await {
        Ok(info) => {
            let admins: Vec<String> = info.members.iter().filter(|member| member.admin).map(|member| text::line(&member.name)).collect();
            ToolOutput::ok(format!("Admins of room {room}: {}.", admins.join(", ")), json!({ "roomId": room, "members": info.members }))
        }
        Err(error) => ToolOutput::failed(error.to_string()),
    }
}
