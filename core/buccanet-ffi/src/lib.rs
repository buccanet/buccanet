//! The Buccanet SDK for Swift, Kotlin and C#. Same Rust core as the CLI and the web app.

mod types;

use std::sync::Arc;

use buccanet_sdk as sdk;
pub use types::*;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BuccanetError {
    #[error("{reason}")]
    Failed { reason: String },
}

impl From<sdk::SdkError> for BuccanetError {
    fn from(error: sdk::SdkError) -> Self {
        BuccanetError::Failed { reason: error.to_string() }
    }
}

/// Implemented by the app: receives events and persists the device state (Keychain, Keystore, DPAPI…).
#[uniffi::export(with_foreign)]
pub trait BuccanetHost: Send + Sync {
    fn on_event(&self, event: ClientEvent);
    fn save_state(&self, state: Vec<u8>);
}

struct HostBridge(Arc<dyn BuccanetHost>);

impl sdk::ClientHost for HostBridge {
    fn on_event(&self, event: sdk::ClientEvent) {
        self.0.on_event(event.into());
    }

    fn save_state(&self, state: Vec<u8>) {
        self.0.save_state(state);
    }
}

#[derive(uniffi::Object)]
pub struct BuccanetClient {
    client: sdk::Client,
}

#[uniffi::export(async_runtime = "tokio")]
impl BuccanetClient {
    /// Creates a device on the relay; its keys are generated here and never leave the app.
    #[uniffi::constructor]
    pub async fn register(relay_url: String, name: String, kind: DeviceKind, host: Arc<dyn BuccanetHost>) -> Result<Arc<Self>, BuccanetError> {
        let client = sdk::Client::register(&relay_url, &name, kind.into(), Arc::new(HostBridge(host))).await?;
        Ok(Arc::new(Self { client }))
    }

    /// Reopens the device from the last blob given to `save_state`.
    #[uniffi::constructor]
    pub fn restore(state: Vec<u8>, host: Arc<dyn BuccanetHost>) -> Result<Arc<Self>, BuccanetError> {
        let client = sdk::Client::restore(&state, Arc::new(HostBridge(host)))?;
        Ok(Arc::new(Self { client }))
    }

    pub fn device_id(&self) -> String {
        self.client.device_id()
    }

    pub fn name(&self) -> String {
        self.client.name()
    }

    /// Follows the inbox until the task is cancelled.
    pub async fn run(&self) {
        self.client.run().await;
    }

    pub async fn sync(&self) -> Result<(), BuccanetError> {
        Ok(self.client.sync().await?)
    }

    pub async fn rooms(&self) -> Vec<RoomInfo> {
        self.client.rooms().await.into_iter().map(Into::into).collect()
    }

    pub async fn messages(&self, room_id: String, after_seq: u64) -> Result<Vec<ChatMessage>, BuccanetError> {
        Ok(self.client.messages(&room_id, after_seq).await?.into_iter().map(Into::into).collect())
    }

    pub async fn create_room(&self, title: Option<String>, policy: RoomPolicy) -> Result<RoomInfo, BuccanetError> {
        Ok(self.client.create_room(title, policy.into()).await?.into())
    }

    pub async fn create_invite(&self, room_id: String) -> Result<String, BuccanetError> {
        Ok(self.client.create_invite(&room_id).await?)
    }

    pub async fn join(&self, link: String) -> Result<JoinPending, BuccanetError> {
        let pending = self.client.join(&link).await?;
        Ok(JoinPending { room_id: pending.room_id, request_id: pending.request_id })
    }

    pub async fn pending_requests(&self, room_id: String) -> Result<Vec<PendingJoin>, BuccanetError> {
        Ok(self.client.pending_requests(&room_id).await?.into_iter().map(Into::into).collect())
    }

    pub async fn approve(&self, room_id: String, request_id: String) -> Result<(), BuccanetError> {
        Ok(self.client.approve(&room_id, &request_id).await?)
    }

    pub async fn reject(&self, room_id: String, request_id: String) -> Result<(), BuccanetError> {
        Ok(self.client.reject(&room_id, &request_id).await?)
    }

    pub async fn remove_member(&self, room_id: String, device_id: String) -> Result<(), BuccanetError> {
        Ok(self.client.remove_member(&room_id, &device_id).await?)
    }

    pub async fn send_text(&self, room_id: String, text: String) -> Result<ChatMessage, BuccanetError> {
        Ok(self.client.send_text(&room_id, &text).await?.into())
    }
}

/// The default policy for new rooms.
#[uniffi::export]
pub fn default_room_policy() -> RoomPolicy {
    sdk::RoomPolicy::default().into()
}
