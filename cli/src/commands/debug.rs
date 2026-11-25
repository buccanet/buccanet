//! Plain commands to drive a profile by hand or from scripts.

use std::sync::Arc;

use buccanet_sdk::{ChatMessage, Client, ClientEvent, DeviceKind, RoomInfo, RoomPolicy};
use clap::ValueEnum;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::CommandResult;
use super::output::{describe, kind_label};
use crate::host::{Events, open_client};
use crate::mcp::control::{self, Backend};
use crate::mcp::tools::ToolContext;
use crate::profile::{self, Profile, ProfileError};
use crate::{guard, text};

#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    Human,
    Agent,
}

impl From<KindArg> for DeviceKind {
    fn from(kind: KindArg) -> Self {
        match kind {
            KindArg::Human => DeviceKind::Human,
            KindArg::Agent => DeviceKind::Agent,
        }
    }
}

async fn open(name: &str) -> Result<(Profile, Client, Events), Box<dyn std::error::Error>> {
    connect(Profile::open(name)?).await
}

async fn connect(profile: Profile) -> Result<(Profile, Client, Events), Box<dyn std::error::Error>> {
    let (client, events) = open_client(&profile).await?;
    client.sync().await?;
    Ok((profile, client, events))
}

/// A room id, or a unique prefix of one.
pub async fn resolve_room(client: &Client, room: &str) -> Result<String, Box<dyn std::error::Error>> {
    let matches: Vec<String> = client.rooms().await.into_iter().map(|info| info.room_id).filter(|id| id.starts_with(room)).collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("no room matches {room}").into()),
        _ => Err(format!("{room} matches several rooms").into()),
    }
}

/// How a command reaches its profile: it opens the profile, or the long running process that
/// holds it (`tail`, `chat`, an MCP server, a connector) acts for it on the control socket.
enum Access {
    Direct(Client),
    Owner(Backend),
}

async fn access(name: &str) -> Result<Access, Box<dyn std::error::Error>> {
    match Profile::open(name) {
        Ok(profile) => Ok(Access::Direct(connect(profile).await?.1)),
        Err(ProfileError::Locked(..)) => Ok(Access::Owner(Backend::Remote(profile::control_socket(name)?))),
        Err(error) => Err(error.into()),
    }
}

/// Runs a tool in the owning process and returns its structured result.
async fn through_owner(owner: &Backend, tool: &str, args: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let output = owner.call(tool, &args).await.map_err(|error| error.message)?;
    if output.is_error {
        return Err(output.text.into());
    }
    Ok(output.structured)
}

pub async fn rooms(name: &str) -> CommandResult {
    let rooms: Vec<RoomInfo> = match access(name).await? {
        Access::Direct(client) => client.rooms().await,
        Access::Owner(owner) => serde_json::from_value(through_owner(&owner, "list_rooms", json!({})).await?["rooms"].take())?,
    };
    for room in rooms {
        let members: Vec<String> =
            room.members.iter().map(|member| format!("{} ({})", text::line(&member.name), kind_label(member.kind))).collect();
        let title = room.title.as_deref().map_or_else(|| "untitled".into(), text::line);
        println!("{} {title}: {}", text::line(&room.room_id), members.join(", "));
    }
    Ok(())
}

pub async fn create(name: &str, title: Option<String>, policy: RoomPolicy) -> CommandResult {
    let room_id = match access(name).await? {
        Access::Direct(client) => client.create_room(title, policy).await?.room_id,
        Access::Owner(owner) => {
            let args =
                json!({ "title": title, "approval": policy.approval, "maxMembers": policy.max_members, "agentBurst": policy.agent_burst });
            through_owner(&owner, "create_room", args).await?["roomId"].as_str().unwrap_or_default().to_string()
        }
    };
    println!("{room_id}");
    Ok(())
}

pub async fn invite(name: &str, room: &str) -> CommandResult {
    let link = match access(name).await? {
        Access::Direct(client) => client.create_invite(&resolve_room(&client, room).await?).await?,
        Access::Owner(owner) => {
            through_owner(&owner, "create_invite", json!({ "roomId": room })).await?["invite"].as_str().unwrap_or_default().to_string()
        }
    };
    print!("{}", buccanet_sdk::invite_message(&link));
    Ok(())
}

pub async fn join(name: &str, link: &str) -> CommandResult {
    let (_profile, client, mut events) = open(name).await?;
    let pending = client.join(link).await?;
    eprintln!("waiting for an admin of room {} to approve request {}", pending.room_id, pending.request_id);
    let follower = client.clone();
    let task = tokio::spawn(async move { follower.run().await });
    while let Some(event) = events.recv().await {
        match event {
            ClientEvent::JoinedRoom { room } if room.room_id == pending.room_id => {
                println!("{}", room.room_id);
                break;
            }
            ClientEvent::JoinRejected { request_id, .. } if request_id == pending.request_id => {
                task.abort();
                return Err("the join request was rejected".into());
            }
            other => print_event(&other),
        }
    }
    client.sync().await?;
    task.abort();
    Ok(())
}

pub async fn send(name: &str, room: &str, text: &str) -> CommandResult {
    let seq = match access(name).await? {
        Access::Direct(client) => {
            guard::check_sender(client.kind(), text)?;
            client.send_text(&resolve_room(&client, room).await?, text).await?.seq
        }
        Access::Owner(owner) => {
            through_owner(&owner, "send_message", json!({ "roomId": room, "text": text })).await?["seq"].as_u64().unwrap_or_default()
        }
    };
    println!("{seq}");
    Ok(())
}

/// Lists a room's join requests; a profile held by another process answers through it.
pub async fn requests(name: &str, room: &str) -> CommandResult {
    tool(name, "list_requests", json!({ "roomId": room })).await
}

/// Lets a join request in, or turns it down; a profile held by another process acts through it.
pub async fn decide(name: &str, room: &str, request: &str, approve: bool) -> CommandResult {
    let action = if approve { "approve_member" } else { "reject_member" };
    tool(name, action, json!({ "roomId": room, "requestId": request })).await
}

/// Runs one MCP tool for a profile, on its device or through the process that owns it, and
/// prints its text.
pub async fn tool(name: &str, tool: &str, args: Value) -> CommandResult {
    let owner = match access(name).await? {
        Access::Direct(client) => {
            let context = ToolContext { client, config: profile::read_config(name)?, profile_dir: profile::dir(name)? };
            Backend::Local(Arc::new(context))
        }
        Access::Owner(owner) => owner,
    };
    let output = owner.call(tool, &args).await.map_err(|error| error.message)?;
    if output.is_error {
        return Err(output.text.into());
    }
    println!("{}", output.text);
    Ok(())
}

pub async fn leave(name: &str, room: &str) -> CommandResult {
    match access(name).await? {
        Access::Direct(client) => client.leave_room(&resolve_room(&client, room).await?).await?,
        Access::Owner(owner) => {
            through_owner(&owner, "leave_room", json!({ "roomId": room })).await?;
        }
    }
    println!("You left the room.");
    Ok(())
}

/// Every message a profile keeps for a room.
const READ_LIMIT: u64 = 1000;

pub async fn read(name: &str, room: &str, after: u64) -> CommandResult {
    let messages: Vec<ChatMessage> = match access(name).await? {
        Access::Direct(client) => client.messages(&resolve_room(&client, room).await?, after).await?,
        Access::Owner(owner) => {
            let args = json!({ "roomId": room, "afterSeq": after, "limit": READ_LIMIT });
            serde_json::from_value(through_owner(&owner, "read_messages", args).await?["messages"].take())?
        }
    };
    for message in messages {
        print_event(&ClientEvent::Message { message });
    }
    Ok(())
}

fn print_event(event: &ClientEvent) {
    if let Some(line) = describe(event) {
        println!("{line}");
    }
}

pub async fn tail(name: &str) -> CommandResult {
    let (profile, client, mut events) = open(name).await?;
    control::serve_profile(&profile, &client)?;
    let follower = client.clone();
    tokio::spawn(async move { follower.run().await });
    while let Some(event) = events.recv().await {
        print_event(&event);
    }
    Ok(())
}

/// Prints the messages that reached the profile since it last ran, or else waits up to
/// `timeout` for the next ones, then exits: an agent without a harness loops on it.
pub async fn wait(name: &str, timeout: std::time::Duration) -> CommandResult {
    let (profile, client, mut events) = open(name).await?;
    control::serve_profile(&profile, &client)?;
    let me = client.device_id();
    let mut arrived = false;
    while let Ok(event) = events.try_recv() {
        arrived |= print_arrival(&event, &me);
    }
    if arrived {
        return Ok(());
    }
    let follower = client.clone();
    let task = tokio::spawn(async move { follower.run().await });
    let next = async {
        while let Some(event) = events.recv().await {
            if print_arrival(&event, &me) {
                break;
            }
        }
        while let Ok(event) = events.try_recv() {
            print_arrival(&event, &me);
        }
    };
    if tokio::time::timeout(timeout, next).await.is_err() {
        println!("No new message for {} seconds. Run wait again to keep listening.", timeout.as_secs());
    }
    task.abort();
    Ok(())
}

/// Prints a message meant for this agent, as the wake rules decide; true when it was one.
fn print_arrival(event: &ClientEvent, me: &str) -> bool {
    let arrival = matches!(event, ClientEvent::Message { message } if crate::wake::wakes(message, me));
    if arrival {
        print_event(event);
    }
    arrival
}

pub async fn chat(name: &str, room: &str) -> CommandResult {
    let (profile, client, mut events) = open(name).await?;
    control::serve_profile(&profile, &client)?;
    let room = resolve_room(&client, room).await?;
    while events.try_recv().is_ok() {}
    for message in client.messages(&room, 0).await? {
        print_event(&ClientEvent::Message { message });
    }
    let follower = client.clone();
    tokio::spawn(async move { follower.run().await });
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            print_event(&event);
        }
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Err(error) = client.send_text(&room, &line).await {
            eprintln!("not sent: {error}");
        }
    }
    Ok(())
}
