//! Keeps secrets out of what an agent sends: every message an agent sends from this machine is
//! checked before it is encrypted, against the agent's own secret values and the key formats
//! gitleaks knows. A message that holds one is refused and never leaves the machine.

use buccanet_agent::secrets;
use buccanet_sdk::DeviceKind;

mod values;

/// Why a message was refused, without the secret itself.
#[derive(Debug, PartialEq, Eq)]
pub enum Leak {
    /// One of the agent's own secret values, from its environment or env files.
    KnownValue,
    /// A key format, by its gitleaks rule id.
    Pattern(&'static str),
}

impl std::error::Error for Leak {}

impl std::fmt::Display for Leak {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self {
            Leak::KnownValue => "one of your secret values (from your environment or an env file)".to_string(),
            Leak::Pattern(rule) => format!("what looks like a secret ({rule})"),
        };
        write!(formatter, "Not sent: the message contains {what}. Secrets never go into a room: remove it and send again.")
    }
}

/// Checks what a device sends when it is an agent; people send what they choose.
pub fn check_sender(kind: DeviceKind, text: &str) -> Result<(), Leak> {
    match kind {
        DeviceKind::Agent => check(text),
        DeviceKind::Human => Ok(()),
    }
}

/// Checks a message before an agent sends it.
pub fn check(text: &str) -> Result<(), Leak> {
    if secrets::contains_any(text, &values::known()) {
        return Err(Leak::KnownValue);
    }
    match secrets::key_shaped(text) {
        Some(rule) => Err(Leak::Pattern(rule)),
        None => Ok(()),
    }
}
