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

/// When a message wakes an agent.
#[derive(Debug, Clone, Copy)]
pub enum WakeRule<'a> {
    /// Every message for it: one that names nobody with `@`, or names this agent.
    Everyone,
    /// Only a message that names this agent, with `@` or by its name. Agents of a public room
    /// follow it, so a room of many agents does not answer every line.
    Addressed { name: &'a str },
}

/// A readable message from another member, not one shared from earlier history.
fn fresh(message: &ChatMessage) -> bool {
    !message.own && !message.shared && !message.unreadable
}

/// Whether `text` holds `name` as a whole phrase, whatever its case.
fn names(text: &str, name: &str) -> bool {
    let (text, name) = (text.to_lowercase(), name.trim().to_lowercase());
    if name.is_empty() {
        return false;
    }
    text.match_indices(&name).any(|(start, found)| {
        let before = text[..start].chars().next_back();
        let after = text[start + found.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Whether a message wakes the agent behind device `me`: a readable one from another member,
/// and when it names members with `@`, only when it names this one. Messages not for it stay
/// readable.
pub fn wakes(message: &ChatMessage, me: &str) -> bool {
    wakes_by(message, me, WakeRule::Everyone)
}

/// Whether a message wakes the agent behind device `me` under `rule`.
pub fn wakes_by(message: &ChatMessage, me: &str, rule: WakeRule) -> bool {
    let mentioned = message.mentions.iter().any(|id| id == me);
    let addressed = match rule {
        WakeRule::Everyone => message.mentions.is_empty() || mentioned,
        WakeRule::Addressed { name } => mentioned || names(&message.text, name),
    };
    fresh(message) && addressed
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

    #[test]
    fn addressed_rule_wakes_on_a_mention_or_the_name_only() {
        let rule = WakeRule::Addressed { name: "Petrel" };
        assert!(wakes_by(&message("hi all", &["me"], false), "me", rule));
        assert!(wakes_by(&message("petrel, what do you think?", &[], false), "me", rule));
        assert!(!wakes_by(&message("hi all", &[], false), "me", rule));
        assert!(!wakes_by(&message("the petrels fly", &[], false), "me", rule));
        assert!(!wakes_by(&message("Petrel?", &[], true), "me", rule));
    }
}
