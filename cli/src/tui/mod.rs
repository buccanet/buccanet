//! `bucca` with no arguments: the developer's own chat client in the terminal.

mod app;
mod commands;
mod view;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use futures::StreamExt;

use crate::commands::{CommandResult, setup};
use crate::host::open_client;
use crate::profile::{self, Profile};
use app::{App, Focus};

pub async fn run(profile_name: &str, relay: &str) -> CommandResult {
    if !profile::exists(profile_name) {
        setup::onboard(profile_name, relay).await?;
    }
    let profile = Profile::open(profile_name)?;
    let (client, mut events) = open_client(&profile).await?;
    client.sync().await?;
    let follower = client.clone();
    tokio::spawn(async move { follower.run().await });
    let mut app = App::new(client.clone(), profile_name.to_string()).await;
    for room in app.rooms.clone() {
        if room.admin {
            app.pending.extend(client.pending_requests(&room.room_id).await.unwrap_or_default());
        }
    }
    let mut terminal = ratatui::try_init().map_err(|error| format!("the terminal UI needs an interactive terminal: {error}"))?;
    // The wheel scrolls the conversation instead of the terminal's own buffer.
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let mut keys = EventStream::new();
    let mut update = Some(tokio::spawn(crate::update::notice()));
    let result = loop {
        if let Err(error) = terminal.draw(|frame| view::draw(frame, &app)) {
            break Err(error.into());
        }
        tokio::select! {
            Some(Ok(event)) = keys.next() => on_terminal(&mut app, event).await,
            Some(event) = events.recv() => app.on_event(event).await,
            Ok(notice) = async { update.as_mut().expect("guarded").await }, if update.is_some() => {
                update = None;
                if let Some(notice) = notice {
                    app.status = notice;
                }
            }
        }
        if app.quit {
            break Ok(());
        }
    };
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

const WHEEL_LINES: isize = 3;
const PAGE_LINES: isize = 15;

async fn on_terminal(app: &mut App, event: Event) {
    if let Event::Mouse(mouse) = &event {
        match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll(WHEEL_LINES),
            MouseEventKind::ScrollDown => app.scroll(-WHEEL_LINES),
            _ => {}
        }
        return;
    }
    let Event::Key(KeyEvent { code, modifiers, kind: KeyEventKind::Press, .. }) = event else { return };
    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => app.interrupt(),
        KeyCode::Esc => app.input.clear(),
        KeyCode::Tab => app.focus = if app.focus == Focus::Input { Focus::Rooms } else { Focus::Input },
        KeyCode::Up => app.select(-1).await,
        KeyCode::Down => app.select(1).await,
        KeyCode::PageUp => app.scroll(PAGE_LINES),
        KeyCode::PageDown => app.scroll(-PAGE_LINES),
        KeyCode::Enter => commands::submit(app).await,
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char(c) => {
            app.focus = Focus::Input;
            app.input.push(c);
        }
        _ => {}
    }
}
