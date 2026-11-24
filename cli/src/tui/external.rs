//! Things the terminal UI hands to another program, as `git commit` does: the person's editor
//! for room rules and agent instructions, a pager to read rules they cannot change.

use std::path::{Path, PathBuf};
use std::process::Command;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::DefaultTerminal;

use super::app::App;
use crate::{instructions, profile};

pub enum External {
    /// The room admin edits the rules of a room.
    EditRules { room_id: String, text: String },
    /// The owner edits the instructions all their agents follow.
    EditInstructions,
    /// Text to read only, such as rules the person cannot change.
    Read(String),
}

/// Whether a program of that name is in one of the `PATH` folders, without running it.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|folder| folder.join(program).is_file()))
}

/// The editor the person chose, else nano, which most people can use, else vi.
fn editor() -> Vec<String> {
    let chosen = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).ok().filter(|editor| !editor.trim().is_empty());
    let editor = chosen.unwrap_or_else(|| if on_path("nano") { "nano".into() } else { "vi".into() });
    editor.split_whitespace().map(str::to_string).collect()
}

fn open_with(program: Vec<String>, file: &Path) -> std::io::Result<()> {
    let (command, args) = program.split_first().ok_or_else(|| std::io::Error::other("no editor"))?;
    Command::new(command).args(args).arg(file).status().map(|_| ())
}

/// Leaves the terminal to the other program, then takes it back. The caller stops reading
/// keys first, so the program gets every keystroke.
pub async fn run(terminal: &mut DefaultTerminal, app: &mut App, external: External) {
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    app.status = match handle(app, external).await {
        Ok(status) => status,
        Err(error) => format!("error: {error}"),
    };
    let _ = enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture);
    let _ = terminal.clear();
}

async fn handle(app: &mut App, external: External) -> Result<String, Box<dyn std::error::Error>> {
    match external {
        External::EditRules { room_id, text } => {
            let edited = edit_text(&scratch(app, "ROOM_RULES.md")?, &text)?;
            if edited.trim() == text.trim() || edited.trim().is_empty() {
                return Ok("The room rules are unchanged.".into());
            }
            let info = app.client.set_room_rules(&room_id, &edited).await?;
            app.reload().await;
            Ok(format!("Room rules saved as version {}; every member receives them.", info.rules.map_or(0, |rules| rules.version)))
        }
        External::EditInstructions => {
            open_with(editor(), &instructions::ensure_shared()?)?;
            Ok("Your agents receive these instructions with their next message.".into())
        }
        External::Read(text) => {
            let file = scratch(app, "ROOM_RULES.txt")?;
            profile::write_private(&file, text.as_bytes())?;
            let pager = std::env::var("PAGER").unwrap_or_else(|_| "less".into());
            let shown = open_with(pager.split_whitespace().map(str::to_string).collect(), &file);
            let _ = std::fs::remove_file(&file);
            shown?;
            Ok(String::new())
        }
    }
}

/// A private file in the profile's folder for the text being edited.
fn scratch(app: &App, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(profile::dir(&app.profile)?.join(name))
}

/// Opens `text` in the editor and returns what was saved; the file is removed afterwards.
fn edit_text(file: &Path, text: &str) -> Result<String, Box<dyn std::error::Error>> {
    profile::write_private(file, text.as_bytes())?;
    let edited = open_with(editor(), file).and_then(|()| std::fs::read_to_string(file));
    let _ = std::fs::remove_file(file);
    Ok(edited?)
}
