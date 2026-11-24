//! `bucca` with no arguments: the developer's own chat client in the terminal.

mod app;
mod commands;
mod editor;
mod external;
mod keys;
mod menu;
mod selection;
mod view;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, EventStream, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::supports_keyboard_enhancement;
use futures::StreamExt;

use crate::commands::{CommandResult, setup};
use crate::host::open_client;
use crate::profile::{self, Profile};
use app::App;

/// Takes the mouse (for the wheel and selection), pastes as one piece, and, where the terminal
/// can tell them apart, Shift+Enter from Enter.
fn capture_input() {
    let mut out = std::io::stdout();
    let _ = crossterm::execute!(out, EnableMouseCapture, EnableBracketedPaste);
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = crossterm::execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    }
}

/// Gives the terminal its own mouse, paste and keys back.
fn release_input() {
    let mut out = std::io::stdout();
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = crossterm::execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(out, DisableBracketedPaste, DisableMouseCapture);
}

pub async fn run(profile_name: &str, relay: &str) -> CommandResult {
    if !profile::exists(profile_name) {
        setup::onboard(profile_name, relay).await?;
    }
    let profile = Profile::open(profile_name)?;
    let (client, mut events) = open_client(&profile).await?;
    client.sync().await?;
    crate::mcp::control::serve_profile(&profile, &client)?;
    let follower = client.clone();
    tokio::spawn(async move { follower.run().await });
    let mut app = App::new(client.clone(), profile_name.to_string()).await;
    for room in app.rooms.clone() {
        if room.admin {
            app.pending.extend(client.pending_requests(&room.room_id).await.unwrap_or_default());
        }
    }
    let mut terminal = ratatui::try_init().map_err(|error| format!("the terminal UI needs an interactive terminal: {error}"))?;
    capture_input();
    let mut keys = EventStream::new();
    let mut update = Some(tokio::spawn(crate::update::notice()));
    let result = loop {
        match terminal.draw(|frame| view::draw(frame, &app)) {
            Ok(drawn) => app.screen = drawn.buffer.clone(),
            Err(error) => break Err(error.into()),
        }
        tokio::select! {
            Some(Ok(event)) = keys.next() => keys::on_terminal(&mut app, event).await,
            Some(event) = events.recv() => app.on_event(event).await,
            Ok(notice) = async { update.as_mut().expect("guarded").await }, if update.is_some() => {
                update = None;
                if let Some(notice) = notice {
                    app.status = notice;
                }
            }
        }
        if let Some(request) = app.external.take() {
            drop(keys);
            external::run(&mut terminal, &mut app, request).await;
            keys = EventStream::new();
        }
        if app.quit {
            break Ok(());
        }
    };
    release_input();
    ratatui::restore();
    result
}
