//! Plain commands to drive a profile by hand or from scripts.

use std::sync::Arc;

use clap::ValueEnum;
use buccanet_sdk::{Client, ClientEvent, DeviceKind, RoomPolicy};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::CommandResult;
use super::output::{describe, kind_label};
use crate::host::{Events, open_client};
use crate::mcp::control::{self, Backend};
use crate::mcp::tools::ToolContext;
use crate::profile::{self, Profile, ProfileError};

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

/// Long running commands answer the other commands of their profile on its control socket.
fn serve_control(profile: &Profile, client: &Client) -> std::io::Result<()> {
    let ctx = Arc::new(ToolContext { client: client.clone(), config: profile.config.clone() });
    control::serve(ctx, &profile.control_socket())
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

pub async fn rooms(name: &str) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    for room in client.rooms().await {
        let members: Vec<String> = room.members.iter().map(|member| format!("{} ({})", member.name, kind_label(member.kind))).collect();
        let title = room.title.unwrap_or_else(|| "untitled".into());
        println!("{} {title}: {}", room.room_id, members.join(", "));
    }
    Ok(())
}

pub async fn create(name: &str, title: Option<String>, approval: bool, max_members: u32, agent_burst: u32) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    let room = client.create_room(title, RoomPolicy { approval, max_members, agent_burst }).await?;
    println!("{}", room.room_id);
    Ok(())
}

pub async fn invite(name: &str, room: &str) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    let room = resolve_room(&client, room).await?;
    println!("{}", client.create_invite(&room).await?);
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

pub async fn requests(name: &str, room: &str) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    let room = resolve_room(&client, room).await?;
    for request in client.pending_requests(&room).await? {
        println!("{} {} ({}) device {}", request.request_id, request.name, kind_label(request.kind), request.device_id);
    }
    Ok(())
}

pub async fn approve(name: &str, room: &str, request: &str) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    let room = resolve_room(&client, room).await?;
    client.approve(&room, request).await?;
    Ok(())
}

pub async fn reject(name: &str, room: &str, request: &str) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    let room = resolve_room(&client, room).await?;
    client.reject(&room, request).await?;
    Ok(())
}

pub async fn send(name: &str, room: &str, text: &str) -> CommandResult {
    let profile = match Profile::open(name) {
        Ok(profile) => profile,
        Err(ProfileError::Locked(..)) => return send_through_owner(name, room, text).await,
        Err(error) => return Err(error.into()),
    };
    let (_profile, client, _) = connect(profile).await?;
    let room = resolve_room(&client, room).await?;
    let message = client.send_text(&room, text).await?;
    println!("{}", message.seq);
    Ok(())
}

/// The profile is open in a long running process (`tail`, `chat`, an MCP server): it sends.
async fn send_through_owner(name: &str, room: &str, text: &str) -> CommandResult {
    let owner = Backend::Remote(profile::control_socket(name)?);
    let output = owner.call("send_message", &json!({ "roomId": room, "text": text })).await.map_err(|error| error.message)?;
    if output.is_error {
        return Err(output.text.into());
    }
    println!("{}", output.structured["seq"]);
    Ok(())
}

pub async fn read(name: &str, room: &str, after: u64) -> CommandResult {
    let (_profile, client, _) = open(name).await?;
    let room = resolve_room(&client, room).await?;
    for message in client.messages(&room, after).await? {
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
    serve_control(&profile, &client)?;
    let follower = client.clone();
    tokio::spawn(async move { follower.run().await });
    while let Some(event) = events.recv().await {
        print_event(&event);
    }
    Ok(())
}

pub async fn chat(name: &str, room: &str) -> CommandResult {
    let (profile, client, mut events) = open(name).await?;
    serve_control(&profile, &client)?;
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
