//! What the input line does: text is a message, `/word` is a command.

use buccanet_sdk::RoomPolicy;

use super::app::App;
use crate::text;

pub const HELP: &str = "/new <title>  /invite  /join <link>  /requests  /approve  /reject  /quit  · Tab: rooms · ↑↓ pick a room · wheel or PgUp/PgDn: scroll · Ctrl+C twice: quit";

pub async fn submit(app: &mut App) {
    let line = std::mem::take(&mut app.input);
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
        "invite" => invite(app).await,
        "join" => join(app, rest).await,
        "requests" => Ok(requests(app)),
        "approve" => decide(app, true).await,
        "reject" => decide(app, false).await,
        other => Err(format!("unknown command /{other}; {HELP}")),
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
    let title = (!title.is_empty()).then(|| title.to_string());
    let room = app.client.create_room(title, RoomPolicy::default()).await.map_err(|error| error.to_string())?;
    let link = app.client.create_invite(&room.room_id).await.map_err(|error| error.to_string())?;
    app.reload().await;
    app.selected = app.rooms.iter().position(|candidate| candidate.room_id == room.room_id).unwrap_or(0);
    app.load_messages().await;
    Ok(format!("Room created. {}", share(&link)))
}

async fn invite(app: &mut App) -> Result<String, String> {
    let room = app.room_id().ok_or("Pick a room first.")?;
    let link = app.client.create_invite(&room).await.map_err(|error| error.to_string())?;
    Ok(share(&link))
}

/// Puts the invite on the clipboard as the message a person or an agent acts on: an agent told
/// `join this chat: <link>` reads the page and joins.
fn share(link: &str) -> String {
    let message = format!("join this chat: {link}");
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(message.clone())) {
        Ok(()) => format!("Copied, paste it to a person or an agent: {message}"),
        Err(_) => format!("Send this to a person or an agent: {message}"),
    }
}

async fn join(app: &mut App, link: &str) -> Result<String, String> {
    let pending = app.client.join(link).await.map_err(|error| error.to_string())?;
    Ok(format!("Asked to join {}. Waiting for an admin.", text::short(&pending.room_id, 8)))
}

fn requests(app: &App) -> String {
    if app.pending.is_empty() {
        return "No pending request.".into();
    }
    app.pending.iter().map(|request| format!("{} ({:?})", text::line(&request.name), request.kind)).collect::<Vec<_>>().join(", ")
}

async fn decide(app: &mut App, approve: bool) -> Result<String, String> {
    let request = app.pending.first().cloned().ok_or("No pending request.")?;
    let result = if approve {
        app.client.approve(&request.room_id, &request.request_id).await
    } else {
        app.client.reject(&request.room_id, &request.request_id).await
    };
    result.map_err(|error| error.to_string())?;
    app.pending.remove(0);
    app.reload().await;
    Ok(format!("{} {}.", text::line(&request.name), if approve { "joined" } else { "was refused" }))
}
