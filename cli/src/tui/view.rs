//! Green on black, monospace, keyboard first.

use buccanet_sdk::{ChatMessage, DeviceKind, RoomInfo};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Focus};
use super::menu::{self, MenuItem};
use crate::text;

const GREEN: Color = Color::Rgb(0, 255, 102);
const DIM: Color = Color::Rgb(0, 130, 60);
const CYAN: Color = Color::Rgb(0, 215, 255);
const AMBER: Color = Color::Rgb(255, 190, 0);
const BG: Color = Color::Black;

fn block(title: &str, focused: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(if focused { GREEN } else { DIM }))
        .title(Span::styled(format!(" {title} "), Style::default().fg(GREEN).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(BG).fg(GREEN))
}

/// Bright on black and apart from the green of the text, so every author stands out.
const MEMBER_COLORS: [Color; 12] = [
    Color::Rgb(255, 121, 198),
    Color::Rgb(139, 233, 253),
    Color::Rgb(255, 184, 108),
    Color::Rgb(189, 147, 249),
    Color::Rgb(241, 250, 140),
    Color::Rgb(255, 85, 85),
    Color::Rgb(110, 140, 255),
    Color::Rgb(0, 215, 175),
    Color::Rgb(255, 215, 0),
    Color::Rgb(255, 135, 95),
    Color::Rgb(215, 135, 215),
    Color::Rgb(95, 215, 255),
];

fn kind_tag(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Agent => "ai",
        DeviceKind::Human => "hu",
    }
}

/// Members of the open room get distinct colors in join order; someone who left keeps a color
/// derived from their device id.
fn member_color(app: &App, device_id: &str) -> Color {
    let position = app.room().and_then(|room| room.members.iter().position(|member| member.device_id == device_id));
    let index = position.unwrap_or_else(|| device_id.bytes().fold(0usize, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte as usize)));
    MEMBER_COLORS[index % MEMBER_COLORS.len()]
}

pub fn draw(frame: &mut Frame, app: &App) {
    let menu = app.menu();
    let menu_rows = menu.len().min(menu::VISIBLE) as u16;
    let [top, main, menu_area, status, input] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(menu_rows),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());
    let [rooms, messages, members] = Layout::horizontal([Constraint::Length(30), Constraint::Min(30), Constraint::Length(32)]).areas(main);
    draw_top(frame, app, top);
    draw_rooms(frame, app, rooms);
    draw_messages(frame, app, messages);
    draw_members(frame, app, members);
    draw_menu(frame, &menu, app.picked, menu_area);
    let line = Line::from(Span::styled(format!(" {}", text::line(&app.status)), Style::default().fg(AMBER)));
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(BG)), status);
    let target = app.room().map_or_else(|| "no room yet".to_string(), |room| format!("to {}", room_name(room)));
    let input_title = format!("{target} · / for commands");
    let prompt = Paragraph::new(format!("› {}▌", app.input)).block(block(&input_title, app.focus == Focus::Input));
    frame.render_widget(prompt, input);
}

/// Width of the command column, so the descriptions line up.
const COMMAND_WIDTH: usize = 26;

/// The rows matching the input, the picked one highlighted, as Claude Code shows its own.
fn draw_menu(frame: &mut Frame, menu: &[MenuItem], picked: usize, area: Rect) {
    let first = menu::first_visible(picked);
    let lines: Vec<Line> = menu
        .iter()
        .enumerate()
        .skip(first)
        .take(menu::VISIBLE)
        .map(|(index, item)| {
            let (marker, style) = if index == picked {
                ("❯ ", Style::default().fg(GREEN).add_modifier(Modifier::BOLD))
            } else {
                ("  ", Style::default().fg(DIM))
            };
            Line::from(vec![
                Span::styled(format!(" {marker}{:<COMMAND_WIDTH$}", item.label), style),
                Span::styled(item.about.clone(), Style::default().fg(if index == picked { CYAN } else { DIM })),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(BG)), area);
}

/// A room's title, or its short id while it has none.
fn room_name(room: &RoomInfo) -> String {
    room.title.as_deref().map_or_else(|| text::short(&room.room_id, 8), text::line)
}

fn draw_top(frame: &mut Frame, app: &App, area: Rect) {
    let (dot, color) = if app.live { ("● live", GREEN) } else { ("○ connecting", AMBER) };
    let line = Line::from(vec![
        Span::styled(" BUCCANET ", Style::default().fg(BG).bg(GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  end to end encrypted · profile {} · ", app.profile), Style::default().fg(DIM)),
        Span::styled(dot, Style::default().fg(color)),
    ]);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(BG)), area);
}

fn draw_rooms(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .rooms
        .iter()
        .map(|room| {
            let mine =
                room.members.iter().any(|member| app.local_agents.contains_key(&member.device_id) && member.kind == DeviceKind::Agent);
            let unread = app.unread.get(&room.room_id).copied().unwrap_or(0);
            let mut spans = vec![Span::raw(room_name(room))];
            if mine {
                spans.push(Span::styled(" ◆", Style::default().fg(CYAN)));
            }
            if unread > 0 {
                spans.push(Span::styled(format!(" +{unread}"), Style::default().fg(AMBER)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(items)
        .block(block("rooms", app.focus == Focus::Rooms))
        .highlight_style(Style::default().fg(BG).bg(GREEN))
        .highlight_symbol("› ");
    let mut state = ListState::default().with_selected((!app.rooms.is_empty()).then_some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

/// One line per line of the message: the first carries time and author, the rest are indented.
fn message_lines(app: &App, message: &ChatMessage) -> Vec<Line<'static>> {
    let tag = kind_tag(message.author_kind);
    let color = member_color(app, &message.sender_device_id);
    let time = chrono::DateTime::parse_from_rfc3339(&message.at)
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|_| "--:--".into());
    let name_style = Style::default().fg(color).add_modifier(if message.own { Modifier::BOLD } else { Modifier::empty() });
    let text_color = if message.shared { DIM } else { GREEN };
    let text_style = Style::default().fg(text_color);
    let body = text::body(&message.text);
    let mut rows = body.lines().map(str::to_string);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{time} "), Style::default().fg(DIM)),
        Span::styled(text::line(&message.author_name), name_style),
        Span::styled(format!(" [{tag}] › "), Style::default().fg(DIM)),
        Span::styled(rows.next().unwrap_or_default(), text_style),
    ])];
    lines.extend(rows.map(|row| Line::from(vec![Span::raw("      "), Span::styled(row, text_style)])));
    lines
}

fn draw_messages(frame: &mut Frame, app: &App, area: Rect) {
    let mut title = match app.room() {
        Some(room) if room.title.is_none() => format!("{} · /rename <title> to name it", room_name(room)),
        Some(room) => room_name(room),
        None => "no room · /new <title> or /join <link>".into(),
    };
    if app.scroll_back > 0 {
        title.push_str(" · scrolled back, PgDn for newer");
    }
    let lines: Vec<Line> = app.messages.iter().flat_map(|message| message_lines(app, message)).collect();
    let frame_block = block(&title, false);
    let inner = frame_block.inner(area);
    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    // Measured inside the borders, wrapping by words as the paragraph does, so the newest line
    // shows unless the person scrolled back; scrolling back stops at the first line.
    let bottom = body.line_count(inner.width).saturating_sub(inner.height as usize);
    app.scroll_limit.set(bottom);
    let scroll = bottom.saturating_sub(app.scroll_back);
    frame.render_widget(body.block(frame_block).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)), area);
}

fn draw_members(frame: &mut Frame, app: &App, area: Rect) {
    let me = app.client.device_id();
    let mut lines: Vec<Line> = app
        .room()
        .map(|room| room.members.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|member| {
            let color = member_color(app, &member.device_id);
            let note = if member.device_id == me {
                " (you)".to_string()
            } else {
                app.local_agents.get(&member.device_id).map(|profile| format!(" ◆ {profile}")).unwrap_or_default()
            };
            Line::from(vec![
                Span::styled(format!("[{}] ", kind_tag(member.kind)), Style::default().fg(DIM)),
                Span::styled(text::line(&member.name), Style::default().fg(color)),
                Span::styled(note, Style::default().fg(DIM)),
            ])
        })
        .collect();
    if !app.pending.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(format!("{} waiting · /approve", app.pending.len()), Style::default().fg(AMBER))));
    }
    frame.render_widget(Paragraph::new(lines).block(block("members", false)), area);
}
