//! What the keyboard, the mouse and a paste do: the command menu first, then the input box,
//! which edits as a shell prompt does, then the rooms and the conversation.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::app::{App, Focus};
use super::commands;
use super::selection::Selection;
use super::view::PROMPT;

const WHEEL_LINES: isize = 3;
const PAGE_LINES: isize = 15;

pub async fn on_terminal(app: &mut App, event: Event) {
    match event {
        Event::Mouse(mouse) => on_mouse(app, mouse),
        Event::Paste(text) => {
            app.selection = None;
            app.focus = Focus::Input;
            app.input.insert(&text.replace("\r\n", "\n").replace('\r', "\n"));
            app.picked = 0;
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => on_key(app, key).await,
        _ => {}
    }
}

async fn on_key(app: &mut App, key: KeyEvent) {
    app.selection = None;
    if !app.menu().is_empty() && on_menu(app, key.code).await {
        return;
    }
    if edit(app, key) {
        app.focus = Focus::Input;
        app.picked = 0;
        return;
    }
    let width = app.input_width.get();
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.interrupt(),
        KeyCode::Esc => app.input.clear(),
        KeyCode::Tab => app.focus = if app.focus == Focus::Input { Focus::Rooms } else { Focus::Input },
        KeyCode::Up if !app.input.vertical(PROMPT, width, -1) => app.select(-1).await,
        KeyCode::Down if !app.input.vertical(PROMPT, width, 1) => app.select(1).await,
        KeyCode::PageUp => app.scroll(PAGE_LINES),
        KeyCode::PageDown => app.scroll(-PAGE_LINES),
        KeyCode::Enter => commands::submit(app).await,
        _ => {}
    }
}

/// Keys that change the input or move its cursor; false lets the key through.
fn edit(app: &mut App, key: KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let input = &mut app.input;
    match key.code {
        KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => input.insert("\n"),
        KeyCode::Char('j') if control => input.insert("\n"),
        KeyCode::Left if alt || control => input.word_left(),
        KeyCode::Right if alt || control => input.word_right(),
        KeyCode::Char('b') if alt => input.word_left(),
        KeyCode::Char('f') if alt => input.word_right(),
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.home(),
        KeyCode::End => input.end(),
        KeyCode::Char('a') if control => input.home(),
        KeyCode::Char('e') if control => input.end(),
        KeyCode::Backspace if alt || control => input.delete_word(),
        KeyCode::Char('w') if control => input.delete_word(),
        KeyCode::Char('u') if control => input.delete_to_line_start(),
        KeyCode::Char('k') if control => input.delete_to_line_end(),
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Char(c) if !control && !alt => input.insert(c.encode_utf8(&mut [0; 4])),
        _ => return false,
    }
    true
}

/// Keys the open command menu takes; false lets the key through to the input.
async fn on_menu(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Up => app.pick(-1),
        KeyCode::Down => app.pick(1),
        KeyCode::Tab => {
            app.complete();
        }
        KeyCode::Enter => commands::accept(app).await,
        _ => return false,
    }
    true
}

/// The wheel scrolls the conversation; a drag selects and copies; a click in the input moves
/// its cursor.
fn on_mouse(app: &mut App, mouse: MouseEvent) {
    let point = Position { x: mouse.column, y: mouse.row };
    match mouse.kind {
        MouseEventKind::ScrollUp => wheel(app, point, -1),
        MouseEventKind::ScrollDown => wheel(app, point, 1),
        MouseEventKind::Down(MouseButton::Left) => {
            let panes = app.panes.get();
            app.selection = panes.at(point).map(|pane| Selection::start(pane, point));
            if panes.input.contains(point) {
                let (column, row) = (point.x - panes.input.x, point.y - panes.input.y + app.input_offset.get());
                app.input.click(PROMPT, panes.input.width, column, row);
                app.focus = Focus::Input;
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(selection) = app.selection.as_mut() {
                selection.extend(point);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => copy_selection(app),
        _ => {}
    }
}

/// The wheel scrolls the pane under the mouse: the rooms, the members, or else the conversation.
/// `down` is 1 towards the end of a list, -1 towards its start.
fn wheel(app: &mut App, point: Position, down: isize) {
    let panes = app.panes.get();
    if panes.rooms.contains(point) {
        app.rooms_scroll = app.rooms_scroll.saturating_add_signed(down).min(app.rooms.len().saturating_sub(1));
    } else if panes.members.contains(point) {
        app.members_scroll = app.members_scroll.saturating_add_signed(down).min(app.members_limit.get());
    } else {
        app.scroll(-down * WHEEL_LINES);
    }
}

/// Copies what the drag covered; a click that did not move selects nothing.
fn copy_selection(app: &mut App) {
    let Some(selection) = app.selection.filter(|selection| !selection.is_empty()) else {
        app.selection = None;
        return;
    };
    let text = selection.text(&app.screen);
    app.status = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.clone())) {
        Ok(()) => format!("Copied {} characters.", text.chars().count()),
        Err(error) => format!("Could not copy: {error}"),
    };
}
