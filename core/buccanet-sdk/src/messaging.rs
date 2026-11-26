//! Sending: encrypt, post, and on a stale epoch catch up and encrypt again.

use buccanet_protocol::api::{Accepted, DeviceKind, PostMessage};
use buccanet_protocol::envelope::Envelope;

use crate::client::Client;
use crate::error::SdkError;
use crate::events::ChatMessage;
use crate::mentions::mentioned;
use crate::secrets::key_shaped;
use crate::time::now_iso;

const SEND_ATTEMPTS: usize = 3;

/// What an agent may not send: a text shaped like a key. People send what they choose.
fn check_sendable(kind: DeviceKind, text: &str) -> Result<(), SdkError> {
    if text.is_empty() {
        return Err(SdkError::Invalid("a message needs text".into()));
    }
    match (kind, key_shaped(text)) {
        (DeviceKind::Agent, Some(rule)) => Err(SdkError::SecretShaped(rule.to_string())),
        _ => Ok(()),
    }
}

impl Client {
    /// Encrypts and posts a text. An agent device refuses a text shaped like a key here, before
    /// encryption, whatever app calls it.
    pub async fn send_text(&self, room_id: &str, text: &str) -> Result<ChatMessage, SdkError> {
        let text = text.trim();
        check_sendable(self.kind(), text)?;
        let (me, mentions) = {
            let inner = self.inner.lock().await;
            (inner.me(), mentioned(text, &inner.room(room_id)?.members, &self.device_id()))
        };
        let at = now_iso();
        let envelope = Envelope::Text { author: me.clone(), text: text.to_string(), at: at.clone(), mentions: mentions.clone() };
        let accepted = self.send_envelope(room_id, &envelope).await?;
        let message = ChatMessage {
            room_id: room_id.to_string(),
            seq: accepted.seq,
            sender_device_id: self.device_id(),
            author_name: me.name,
            author_kind: me.kind,
            text: text.to_string(),
            at,
            own: true,
            shared: false,
            mentions,
            unreadable: false,
        };
        let mut inner = self.inner.lock().await;
        inner.room_mut(room_id)?.store_message(message.clone());
        self.persist(&inner);
        Ok(message)
    }

    pub(crate) async fn send_envelope(&self, room_id: &str, envelope: &Envelope) -> Result<Accepted, SdkError> {
        let plaintext = envelope.encode().map_err(|error| SdkError::Invalid(error.to_string()))?;
        let client_message_id = uuid::Uuid::new_v4().to_string();
        let mut attempt = 1;
        loop {
            let (mls_message, device_seq) = {
                let mut inner = self.inner.lock().await;
                let room = inner.room_mut(room_id)?;
                room.sealed += 1;
                let (group_id, device_seq) = (room.group_id.clone(), room.sealed);
                let sealed = inner.mls.encrypt(&group_id, &plaintext)?;
                self.persist(&inner);
                (sealed, device_seq)
            };
            let body = PostMessage { client_message_id: client_message_id.clone(), mls_message, device_seq: Some(device_seq) };
            match self.relay.post_message(room_id, &body).await {
                Err(error) if error.is_stale_epoch() && attempt < SEND_ATTEMPTS => {
                    attempt += 1;
                    self.sync().await?;
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_cannot_send_a_key_even_split_by_a_zero_width_space() {
        let split = "here ghp_8RtZq2Lk3Vb\u{200B}N9wXyP4mC7dJ1sF6hG0aTuE5o";
        assert!(matches!(check_sendable(DeviceKind::Agent, split), Err(SdkError::SecretShaped(rule)) if rule == "github-pat"));
        assert!(check_sendable(DeviceKind::Human, split).is_ok());
        assert!(check_sendable(DeviceKind::Agent, "The OCR pass is done.").is_ok());
        assert!(check_sendable(DeviceKind::Agent, "").is_err());
    }
}
