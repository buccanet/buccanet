//! The plaintext inside every MLS application message: `{v: 1, type, body}`.
//! Clients ignore types they do not know, so integrators can add their own.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::DeviceKind;

pub const ENVELOPE_VERSION: u32 = 1;
/// Upper bound of a serialized history share, so it always fits a relay request.
pub const HISTORY_SHARE_MAX_BYTES: usize = 64 * 1024;
pub const HISTORY_SHARE_MAX_MESSAGES: usize = 50;
/// Longest text a message carries, in characters.
pub const TEXT_MAX_CHARS: usize = 16_000;
/// Longest room title, in characters.
pub const TITLE_MAX_CHARS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEnvelope {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedMessage {
    pub seq: u64,
    pub sender_device_id: String,
    pub author: Author,
    pub text: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Envelope {
    Text { author: Author, text: String, at: String },
    RoomMeta { title: String },
    HistoryShare { shared_by: Author, title: Option<String>, messages: Vec<SharedMessage> },
    Unknown { kind: String },
}

#[derive(Serialize, Deserialize)]
struct TextBody {
    author: Author,
    text: String,
    at: String,
}

#[derive(Serialize, Deserialize)]
struct RoomMetaBody {
    title: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryBody {
    shared_by: Author,
    title: Option<String>,
    messages: Vec<SharedMessage>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("envelope is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("envelope version {0} is not supported")]
    Version(u32),
    #[error("cannot encode an unknown envelope")]
    UnknownType,
    #[error("{0} is too long")]
    TooLong(&'static str),
}

fn within(text: &str, max_chars: usize, what: &'static str) -> Result<(), EnvelopeError> {
    if text.chars().count() > max_chars {
        return Err(EnvelopeError::TooLong(what));
    }
    Ok(())
}

impl Envelope {
    /// Bounds every field a member writes, on both sides of the wire.
    fn validate(&self) -> Result<(), EnvelopeError> {
        match self {
            Envelope::Text { text, .. } => within(text, TEXT_MAX_CHARS, "text"),
            Envelope::RoomMeta { title } => within(title, TITLE_MAX_CHARS, "title"),
            Envelope::HistoryShare { title, messages, .. } => {
                within(title.as_deref().unwrap_or_default(), TITLE_MAX_CHARS, "title")?;
                if messages.len() > HISTORY_SHARE_MAX_MESSAGES {
                    return Err(EnvelopeError::TooLong("history share"));
                }
                messages.iter().try_for_each(|message| within(&message.text, TEXT_MAX_CHARS, "text"))
            }
            Envelope::Unknown { .. } => Ok(()),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, EnvelopeError> {
        self.validate()?;
        let (kind, body) = match self {
            Envelope::Text { author, text, at } => {
                ("text", serde_json::to_value(TextBody { author: author.clone(), text: text.clone(), at: at.clone() })?)
            }
            Envelope::RoomMeta { title } => ("room_meta", serde_json::to_value(RoomMetaBody { title: title.clone() })?),
            Envelope::HistoryShare { shared_by, title, messages } => (
                "history_share",
                serde_json::to_value(HistoryBody { shared_by: shared_by.clone(), title: title.clone(), messages: messages.clone() })?,
            ),
            Envelope::Unknown { .. } => return Err(EnvelopeError::UnknownType),
        };
        let raw = RawEnvelope { v: ENVELOPE_VERSION, kind: kind.to_string(), body };
        Ok(serde_json::to_vec(&raw)?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
        let raw: RawEnvelope = serde_json::from_slice(bytes)?;
        if raw.v != ENVELOPE_VERSION {
            return Err(EnvelopeError::Version(raw.v));
        }
        let envelope = Self::from_raw(raw)?;
        envelope.validate()?;
        Ok(envelope)
    }

    fn from_raw(raw: RawEnvelope) -> Result<Envelope, EnvelopeError> {
        Ok(match raw.kind.as_str() {
            "text" => {
                let body: TextBody = serde_json::from_value(raw.body)?;
                Envelope::Text { author: body.author, text: body.text, at: body.at }
            }
            "room_meta" => Envelope::RoomMeta { title: serde_json::from_value::<RoomMetaBody>(raw.body)?.title },
            "history_share" => {
                let body: HistoryBody = serde_json::from_value(raw.body)?;
                Envelope::HistoryShare { shared_by: body.shared_by, title: body.title, messages: body.messages }
            }
            _ => Envelope::Unknown { kind: raw.kind },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_unknown_types_are_tolerated() {
        let author = Author { name: "Ada".into(), kind: DeviceKind::Agent };
        let text = Envelope::Text { author, text: "hi".into(), at: "2026-11-23T00:00:00Z".into() };
        assert_eq!(Envelope::decode(&text.encode().unwrap()).unwrap(), text);
        let future = br#"{"v":1,"type":"reaction","body":{"emoji":"+1"}}"#;
        assert_eq!(Envelope::decode(future).unwrap(), Envelope::Unknown { kind: "reaction".into() });
        assert!(Envelope::decode(br#"{"v":2,"type":"text","body":{}}"#).is_err());
    }

    #[test]
    fn oversized_fields_are_refused() {
        let author = Author { name: "Ada".into(), kind: DeviceKind::Agent };
        let long = Envelope::Text { author, text: "x".repeat(TEXT_MAX_CHARS + 1), at: String::new() };
        assert!(long.encode().is_err());
        let raw = format!(r#"{{"v":1,"type":"room_meta","body":{{"title":"{}"}}}}"#, "t".repeat(TITLE_MAX_CHARS + 1));
        assert!(Envelope::decode(raw.as_bytes()).is_err());
    }
}
