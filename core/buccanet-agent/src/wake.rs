//! Which messages wake an agent, and the wake an agent reads for each.

use buccanet_sdk::{ChatMessage, DeviceKind};
use serde::{Deserialize, Serialize};

use crate::briefing::Note;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wake {
    pub room_id: String,
    pub room_title: Option<String>,
    pub seq: u64,
    pub from: Option<Author>,
    pub text: Option<String>,
    /// The owner's instructions, in full when the agent does not have this version yet.
    #[serde(default)]
    pub instructions: Option<Note>,
    /// The room's rules, in full when the agent does not have this version yet.
    #[serde(default)]
    pub rules: Option<Note>,
    /// The message names this agent with `@`.
    #[serde(default)]
    pub mentions_you: bool,
    /// This agent's device sent the message: an earlier answer, or its owner writing as it.
    #[serde(default)]
    pub from_you: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
}

/// A readable message from another member, not one shared from earlier history.
fn fresh(message: &ChatMessage) -> bool {
    !message.own && !message.shared && !message.unreadable
}

/// Whether a message wakes the agent behind device `me`: a readable one from another member,
/// and when it names members with `@`, only when it names this one. Messages not for it stay
/// readable.
pub fn wakes(message: &ChatMessage, me: &str) -> bool {
    let for_me = message.mentions.is_empty() || message.mentions.iter().any(|id| id == me);
    fresh(message) && for_me
}

impl Wake {
    /// The wake of a message from another member that woke the agent behind device `me`.
    pub fn message(message: &ChatMessage, room_title: Option<String>, me: &str) -> Self {
        let author = Author { device_id: message.sender_device_id.clone(), name: message.author_name.clone(), kind: message.author_kind };
        Self {
            room_id: message.room_id.clone(),
            room_title,
            seq: message.seq,
            from: Some(author),
            text: Some(message.text.clone()),
            instructions: None,
            rules: None,
            mentions_you: message.mentions.iter().any(|id| id == me),
            from_you: message.own,
        }
    }

    pub fn joined(room_id: &str, room_title: Option<String>) -> Self {
        Self {
            room_id: room_id.to_string(),
            room_title,
            seq: 0,
            from: None,
            text: None,
            instructions: None,
            rules: None,
            mentions_you: false,
            from_you: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str, mentions: &[&str], own: bool) -> ChatMessage {
        ChatMessage {
            room_id: "r".into(),
            seq: 1,
            sender_device_id: if own { "me".into() } else { "ada".into() },
            author_name: "Ada".into(),
            author_kind: DeviceKind::Human,
            text: text.into(),
            at: "2026-11-26T10:00:00Z".into(),
            own,
            shared: false,
            mentions: mentions.iter().map(|id| id.to_string()).collect(),
            unreadable: false,
        }
    }

    #[test]
    fn everyone_rule_wakes_on_messages_for_all_or_for_this_agent() {
        assert!(wakes(&message("hi all", &[], false), "me"));
        assert!(wakes(&message("@Petrel hi", &["me"], false), "me"));
        assert!(!wakes(&message("@Bob hi", &["bob"], false), "me"));
        assert!(!wakes(&message("my own line", &[], true), "me"));
    }
}
