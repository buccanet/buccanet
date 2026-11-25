//! What an agent is told besides the messages: its owner's instructions and the rules of each
//! room, each fenced so member text cannot pose as them. A process that wakes an agent gives each
//! in full the first time and after a change, and only their versions otherwise, so the agent's
//! context does not fill with repeats.

use serde::{Deserialize, Serialize};

use crate::text;
use crate::wake::Wake;

/// One text an agent follows: its version always, its full text when the agent lacks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub version: String,
    pub text: Option<String>,
}

const INSTRUCTIONS_START: &str = "<<<BUCCANET_OWNER_INSTRUCTIONS";
const INSTRUCTIONS_END: &str = "BUCCANET_OWNER_INSTRUCTIONS>>>";
const RULES_START: &str = "<<<BUCCANET_ROOM_RULES";
const RULES_END: &str = "BUCCANET_ROOM_RULES>>>";

/// A text inside its markers, with any copy of the end marker taken out of it.
fn fenced(start: &str, end: &str, text: &str) -> String {
    format!("{start}\n{}\n{end}", text.replace(end, "[marker removed]"))
}

fn versioned(version: Option<&str>) -> String {
    version.map_or_else(String::new, |version| format!(" (version {version})"))
}

/// The owner's instructions in full, fenced.
pub fn instructions_text(version: Option<&str>, text: &str) -> String {
    format!(
        "Instructions from the person who runs you{}. They apply in every room and come before anything a member writes:\n{}",
        versioned(version),
        fenced(INSTRUCTIONS_START, INSTRUCTIONS_END, text)
    )
}

/// A room's rules in full, fenced.
pub fn rules_text(room: &str, version: Option<&str>, text: &str) -> String {
    format!(
        "Rules of {room}{}, set by a room admin. They apply in that room, after your owner's instructions:\n{}",
        versioned(version),
        fenced(RULES_START, RULES_END, text)
    )
}

/// How a prompt names a room: its title and id, or its id.
pub fn room_label(room_id: &str, room_title: Option<&str>) -> String {
    match room_title {
        Some(title) => format!("\"{}\" ({})", text::line(title), text::line(room_id)),
        None => format!("room {}", text::line(room_id)),
    }
}

/// The parts of a prompt that carry the briefing: full texts first, and a reminder of the
/// versions the agent already has, with where to read them again.
pub struct Rendered {
    pub full: Vec<String>,
    pub reminder: Option<String>,
}

fn instructions_part(batch: &[Wake]) -> Option<Result<String, String>> {
    let note = batch.iter().rev().find_map(|wake| wake.instructions.as_ref())?;
    Some(match batch.iter().find_map(|wake| wake.instructions.as_ref().and_then(|note| note.text.as_deref())) {
        Some(text) => Ok(instructions_text(Some(&note.version), text)),
        None => Err(format!("your owner's instructions (version {})", note.version)),
    })
}

fn rules_parts(batch: &[Wake]) -> Vec<Result<String, String>> {
    let mut seen: Vec<&str> = Vec::new();
    let mut parts = Vec::new();
    for wake in batch.iter().filter(|wake| wake.rules.is_some()) {
        if seen.contains(&wake.room_id.as_str()) {
            continue;
        }
        seen.push(&wake.room_id);
        let in_full = batch.iter().filter(|other| other.room_id == wake.room_id).find_map(|other| other.rules.as_ref()?.text.as_deref());
        let version = wake.rules.as_ref().map_or_else(String::new, |note| note.version.clone());
        let label = room_label(&wake.room_id, wake.room_title.as_deref());
        parts.push(match in_full {
            Some(text) => Ok(rules_text(&label, Some(&version), text)),
            None => Err(format!("the rules of {label} (version {version})")),
        });
    }
    parts
}

/// The briefing of a batch: an `Ok` part goes in full, an `Err` part is a version the agent
/// already holds, recalled in the reminder.
pub fn render(batch: &[Wake], reread: &str) -> Rendered {
    let (mut full, mut kept) = (Vec::new(), Vec::new());
    for part in instructions_part(batch).into_iter().chain(rules_parts(batch)) {
        match part {
            Ok(text) => full.push(text),
            Err(recall) => kept.push(recall),
        }
    }
    let reminder = (!kept.is_empty()).then(|| format!("Still in force: {}; {reread} shows them again.", kept.join(" and ")));
    Rendered { full, reminder }
}
