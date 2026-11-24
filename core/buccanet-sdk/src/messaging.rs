//! Sending: encrypt, post, and on a stale epoch catch up and encrypt again.

use buccanet_protocol::api::{Accepted, PostMessage};
use buccanet_protocol::envelope::Envelope;

use crate::client::Client;
use crate::error::SdkError;
use crate::events::ChatMessage;
use crate::time::now_iso;

const SEND_ATTEMPTS: usize = 3;

impl Client {
    pub async fn send_text(&self, room_id: &str, text: &str) -> Result<ChatMessage, SdkError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(SdkError::Invalid("a message needs text".into()));
        }
        let me = self.inner.lock().await.me();
        let at = now_iso();
        let envelope = Envelope::Text { author: me.clone(), text: text.to_string(), at: at.clone() };
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
            let mls_message = {
                let inner = self.inner.lock().await;
                let group_id = inner.room(room_id)?.group_id.clone();
                let sealed = inner.mls.encrypt(&group_id, &plaintext)?;
                self.persist(&inner);
                sealed
            };
            let body = PostMessage { client_message_id: client_message_id.clone(), mls_message };
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
