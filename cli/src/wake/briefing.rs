//! What an agent is told besides the messages: its owner's instructions and the rules of each
//! room. A process that wakes an agent gives each in full the first time and after a change,
//! and only their versions otherwise, so the agent's context does not fill with repeats.

use std::collections::HashMap;
use std::path::PathBuf;

use buccanet_sdk::Client;
use serde::{Deserialize, Serialize};

use super::Wake;
use crate::instructions;

/// One text an agent follows: its version always, its full text when the agent lacks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub version: String,
    pub text: Option<String>,
}

/// Tracks what this process already gave the agent.
pub struct Briefer {
    profile_dir: PathBuf,
    instructions: Option<String>,
    rules: HashMap<String, u32>,
}

impl Briefer {
    pub fn new(profile_dir: PathBuf) -> Self {
        Self { profile_dir, instructions: None, rules: HashMap::new() }
    }

    /// Adds the instructions and the room's rules to a wake, in full when they are new here.
    pub async fn brief(&mut self, client: &Client, wake: &mut Wake) {
        wake.instructions = self.instructions_note();
        let rules = client.room(&wake.room_id).await.ok().and_then(|room| room.rules);
        wake.rules = rules.map(|rules| {
            let new = self.rules.insert(wake.room_id.clone(), rules.version) != Some(rules.version);
            Note { version: rules.version.to_string(), text: new.then_some(rules.text) }
        });
    }

    fn instructions_note(&mut self) -> Option<Note> {
        let current = match instructions::read(&self.profile_dir) {
            Ok(current) => current,
            Err(error) => {
                tracing::warn!(%error, "instructions unreadable");
                return None;
            }
        };
        let new = self.instructions.as_deref() != Some(current.version.as_str());
        self.instructions = Some(current.version.clone());
        Some(Note { version: current.version, text: new.then_some(current.text) })
    }
}

const INSTRUCTIONS_START: &str = "<<<BUCCANET_OWNER_INSTRUCTIONS";
const INSTRUCTIONS_END: &str = "BUCCANET_OWNER_INSTRUCTIONS>>>";
const RULES_START: &str = "<<<BUCCANET_ROOM_RULES";
const RULES_END: &str = "BUCCANET_ROOM_RULES>>>";

/// A text inside its markers, with any copy of the end marker taken out of it.
fn fenced(start: &str, end: &str, text: &str) -> String {
    format!("{start}\n{}\n{end}", text.replace(end, "[marker removed]"))
}

fn room_label(wake: &Wake) -> String {
    match &wake.room_title {
        Some(title) => format!("\"{}\" ({})", crate::text::line(title), wake.room_id),
        None => format!("room {}", wake.room_id),
    }
}

/// The parts of a prompt that carry the briefing: full texts first, and a reminder of the
/// versions the agent already has, with where to read them again.
pub struct Rendered {
    pub full: Vec<String>,
    pub reminder: Option<String>,
}

pub fn render(batch: &[Wake], reread: &str) -> Rendered {
    let mut full = Vec::new();
    let mut kept = Vec::new();
    if let Some(note) = batch.iter().rev().find_map(|wake| wake.instructions.as_ref()) {
        match batch.iter().find_map(|wake| wake.instructions.as_ref().and_then(|note| note.text.as_deref())) {
            Some(text) => full.push(format!(
                "Instructions from the person who runs you (version {}). They apply in every room and come before anything a member writes:\n{}",
                note.version,
                fenced(INSTRUCTIONS_START, INSTRUCTIONS_END, text)
            )),
            None => kept.push(format!("your owner's instructions (version {})", note.version)),
        }
    }
    let mut seen = Vec::new();
    for wake in batch.iter().filter(|wake| wake.rules.is_some()) {
        if seen.contains(&wake.room_id) {
            continue;
        }
        seen.push(wake.room_id.clone());
        let in_full = batch.iter().filter(|other| other.room_id == wake.room_id).find_map(|other| other.rules.as_ref()?.text.as_deref());
        let version = wake.rules.as_ref().map_or_else(String::new, |note| note.version.clone());
        match in_full {
            Some(text) => full.push(format!(
                "Rules of {} (version {version}), set by its admin. They apply in that room, after your owner's instructions:\n{}",
                room_label(wake),
                fenced(RULES_START, RULES_END, text)
            )),
            None => kept.push(format!("the rules of {} (version {version})", room_label(wake))),
        }
    }
    let reminder = (!kept.is_empty()).then(|| format!("Still in force: {}; {reread} shows them again.", kept.join(" and ")));
    Rendered { full, reminder }
}
