//! Tracks what a process already gave the agent it wakes: its owner's instructions, read from
//! the profile, and each room's rules. Each goes in full the first time and after a change, and
//! as its version otherwise (`buccanet_agent::briefing` renders them).

use std::collections::HashMap;
use std::path::PathBuf;

use buccanet_agent::Wake;
use buccanet_agent::briefing::Note;
use buccanet_sdk::Client;

use crate::instructions;

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
