//! What the input line does: text is a message, `/word` is a command.

use buccanet_sdk::{MemberInfo, PendingJoin, RoomPolicy, default_rules, invite_message};

use super::app::App;
use super::external::External;
use crate::commands::output::kind_label;
use buccanet_agent::text;

pub const HELP: &str = "Type / for commands · Tab: rooms · ↑↓ pick a room · wheel or PgUp/PgDn: scroll · Ctrl+C twice: quit";

/// Enter on the command menu: a command that needs nothing runs, one that needs an argument
/// waits for it.
pub async fn accept(app: &mut App) {
    if app.complete() == Some(true) {
        submit(app).await;
    }
}

pub async fn submit(app: &mut App) {
    let line = app.input.take();
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let (command, rest) = match line.strip_prefix('/') {
        Some(command) => command.split_once(' ').map_or((command, ""), |(command, rest)| (command, rest.trim())),
        None => return send(app, line).await,
    };
    let outcome = match command {
        "help" => Ok(HELP.to_string()),
        "quit" | "q" => {
            app.quit = true;
            Ok(String::new())
        }
        "new" => new_room(app, rest).await,
        "rename" => rename(app, rest).await,
        "rules" => rules(app),
        "instructions" => {
            app.external = Some(External::EditInstructions);
            Ok(String::new())
        }
        "invite" => invite(app).await,
        "join" => join(app, rest).await,
        "requests" => Ok(requests(app).await),
        "approve" => decide(app, rest, true).await,
        "reject" => decide(app, rest, false).await,
        "remove" => remove(app, rest).await,
        "admin" => set_admin(app, rest, true).await,
        "unadmin" => set_admin(app, rest, false).await,
        "leave" => leave(app).await,
        other => Err(format!("Unknown command /{other}. Type / to see the commands.")),
    };
    app.status = outcome.unwrap_or_else(|error| error);
}

async fn send(app: &mut App, text: &str) {
    let Some(room) = app.room_id() else {
        app.status = "No room yet: /new <title> or /join <link>.".into();
        return;
    };
    match app.client.send_text(&room, text).await {
        Ok(_) => app.load_messages().await,
        Err(error) => app.status = format!("not sent: {error}"),
    }
}

async fn new_room(app: &mut App, title: &str) -> Result<String, String> {
    if title.is_empty() {
        return Err("Give the room a name: /new <title>".into());
    }
    let room = app.client.create_room(Some(title.to_string()), RoomPolicy::default()).await.map_err(|error| error.to_string())?;
    let link = app.client.create_invite(&room.room_id).await.map_err(|error| error.to_string())?;
    app.reload().await;
    app.selected = app.rooms.iter().position(|candidate| candidate.room_id == room.room_id).unwrap_or(0);
    app.load_messages().await;
    Ok(format!("Room created. {}", share(&link)))
}

/// The room admin edits the rules in their editor; any other member reads them.
fn rules(app: &mut App) -> Result<String, String> {
    let room = app.room().cloned().ok_or("Pick a room first.")?;
    app.external = Some(match (&room.rules, room.admin) {
        (Some(rules), true) => External::EditRules { room_id: room.room_id, text: rules.text.clone() },
        (None, true) => External::EditRules { text: default_rules(room.title.as_deref()).text, room_id: room.room_id },
        (Some(rules), false) => External::Read(format!("Room rules, version {}\n\n{}\n", rules.version, rules.text)),
        (None, false) => External::Read("This room has no rules yet.".into()),
    });
    Ok(String::new())
}

async fn rename(app: &mut App, title: &str) -> Result<String, String> {
    let room = app.room_id().ok_or("Pick a room first.")?;
    if title.is_empty() {
        return Err("Give the new name: /rename <title>".into());
    }
    app.client.rename_room(&room, title).await.map_err(|error| error.to_string())?;
    app.reload().await;
    Ok(format!("Room renamed to {}.", text::line(title)))
}

async fn leave(app: &mut App) -> Result<String, String> {
    let room = app.room().cloned().ok_or("Pick a room first.")?;
    if !app.confirmed(format!("leave {}", room.room_id)) {
        app.input.set("/leave");
        return Ok("Press Enter again within 5 seconds to leave this room.".into());
    }
    app.client.leave_room(&room.room_id).await.map_err(|error| error.to_string())?;
    app.reload().await;
    Ok(format!("You left {}.", room.title.as_deref().map_or_else(|| text::short(&room.room_id, 8), text::line)))
}

/// Another member of the open room: the one picked in the menu (its device id), or the one
/// whose name holds `wanted`.
fn pick_member(app: &App, wanted: &str, command: &str) -> Result<MemberInfo, String> {
    let room = app.room().ok_or("Pick a room first.")?;
    let me = app.client.device_id();
    let lowered = wanted.to_lowercase();
    let others: Vec<_> = room.members.iter().filter(|member| member.device_id != me).collect();
    let by_id = others.iter().copied().find(|member| member.device_id == wanted);
    let by_name: Vec<_> = others.iter().copied().filter(|member| member.name.to_lowercase().contains(&lowered)).collect();
    match (by_id, by_name.as_slice()) {
        _ if wanted.is_empty() => Err(format!("Pick the member in the list: /{command} <name>")),
        (Some(member), _) | (None, &[member]) => Ok(member.clone()),
        (None, []) => Err(format!("No member named {} here.", text::line(wanted))),
        (None, _) => Err(format!("Several members match {}: pick one in the list.", text::line(wanted))),
    }
}

/// Removes a member after a second Enter.
async fn remove(app: &mut App, wanted: &str) -> Result<String, String> {
    let member = pick_member(app, wanted, "remove")?;
    let room = app.room().cloned().ok_or("Pick a room first.")?;
    if !app.confirmed(format!("remove {} {}", room.room_id, member.device_id)) {
        app.input.set(format!("/remove {}", member.device_id));
        return Ok(format!("Press Enter again within 5 seconds to remove {}.", text::line(&member.name)));
    }
    app.client.remove_member(&room.room_id, &member.device_id).await.map_err(|error| error.to_string())?;
    app.reload().await;
    Ok(format!("{} was removed.", text::line(&member.name)))
}

/// Makes a member admin of the open room, or takes the role back.
async fn set_admin(app: &mut App, wanted: &str, admin: bool) -> Result<String, String> {
    let member = pick_member(app, wanted, if admin { "admin" } else { "unadmin" })?;
    let room = app.room_id().ok_or("Pick a room first.")?;
    app.client.set_admin(&room, &member.device_id, admin).await.map_err(|error| error.to_string())?;
    app.reload().await;
    let role = if admin { "is now an admin" } else { "is no longer an admin" };
    Ok(format!("{} {role}.", text::line(&member.name)))
}

async fn invite(app: &mut App) -> Result<String, String> {
    let room = app.room_id().ok_or("Pick a room first.")?;
    let link = app.client.create_invite(&room).await.map_err(|error| error.to_string())?;
    Ok(share(&link))
}

/// Puts the invite message on the clipboard: the link for a person, and the terminal command an
/// agent runs, since an agent that opens the link in a browser may be stopped there.
fn share(link: &str) -> String {
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(invite_message(link))) {
        Ok(()) => format!("Invite copied, with the command agents run. Link: {link}"),
        Err(_) => format!("Send this invite: {}", text::line(&invite_message(link))),
    }
}

async fn join(app: &mut App, link: &str) -> Result<String, String> {
    let pending = app.client.join(link).await.map_err(|error| error.to_string())?;
    Ok(format!("Asked to join {}. Waiting for an admin.", text::short(&pending.room_id, 8)))
}

async fn requests(app: &mut App) -> String {
    app.refresh_pending().await;
    if app.pending.is_empty() {
        return "Nobody is waiting to join.".into();
    }
    let waiting = app
        .pending
        .iter()
        .map(|request| format!("{} ({}) in {}", text::line(&request.name), kind_label(request.kind), app.room_title(&request.room_id)));
    format!("Waiting to join: {}.", waiting.collect::<Vec<_>>().join("; "))
}

/// The request `/approve` or `/reject` names in the open room: its request id or part of a name,
/// or nothing when only one is waiting.
fn pick_request(app: &App, wanted: &str, command: &str) -> Result<PendingJoin, String> {
    let waiting = app.pending_here();
    let lowered = wanted.to_lowercase();
    let by_id = waiting.iter().copied().find(|request| request.request_id == wanted);
    let by_name: Vec<_> = waiting.iter().copied().filter(|request| request.name.to_lowercase().contains(&lowered)).collect();
    match (by_id, by_name.as_slice()) {
        _ if waiting.is_empty() => Err(nobody_waiting_here(app)),
        _ if wanted.is_empty() && waiting.len() > 1 => Err(format!("Several are waiting: pick one in the list with /{command} <name>.")),
        (Some(request), _) | (None, &[request]) => Ok(request.clone()),
        (None, []) => Err(format!("Nobody named {} is waiting to join this room.", text::line(wanted))),
        (None, _) => Err(format!("Several people waiting match {}: pick one in the list.", text::line(wanted))),
    }
}

fn nobody_waiting_here(app: &App) -> String {
    match app.pending.len() {
        0 => "Nobody is waiting to join.".into(),
        elsewhere => format!("Nobody is waiting to join this room. {elsewhere} waiting in other rooms: /requests."),
    }
}

async fn decide(app: &mut App, wanted: &str, approve: bool) -> Result<String, String> {
    app.refresh_pending().await;
    let request = pick_request(app, wanted, if approve { "approve" } else { "reject" })?;
    let result = if approve {
        app.client.approve(&request.room_id, &request.request_id).await
    } else {
        app.client.reject(&request.room_id, &request.request_id).await
    };
    app.refresh_pending().await;
    result.map_err(|error| error.to_string())?;
    app.reload().await;
    Ok(format!("{} {}.", text::line(&request.name), if approve { "joined" } else { "was refused" }))
}
