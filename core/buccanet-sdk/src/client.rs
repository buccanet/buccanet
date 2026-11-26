//! The device: state, persistence and the inbox loop. Room operations live in `rooms.rs`
//! and `messaging.rs`.

use std::collections::BTreeSet;
use std::sync::Arc;

use buccanet_protocol::api::{DeviceKind, RegisterDevice};
use buccanet_protocol::envelope::Author;
use buccanet_protocol::events::InboxEvent;
use futures::StreamExt;
use futures::lock::Mutex;

use crate::error::SdkError;
use crate::events::{ChatMessage, ClientEvent, RoomInfo};
use crate::mls::MlsEngine;
use crate::processing::{Action, apply};
use crate::relay::{RelayClient, check_relay_url};
use crate::sse::SseBuffer;
use crate::state::{Identity, Owner, Persisted, RoomState};
use crate::time::sleep_ms;
use crate::trust::{pin_new, prune};

const RECONNECT_MIN_MS: u64 = 1_000;
const RECONNECT_MAX_MS: u64 = 15_000;

/// Implemented by the app that embeds the SDK.
pub trait ClientHost: Send + Sync {
    fn on_event(&self, event: ClientEvent);
    /// Called after every change; store the blob where the platform keeps secrets.
    fn save_state(&self, state: Vec<u8>);
}

pub struct Inner {
    pub state: Persisted,
    pub mls: MlsEngine,
}

impl Inner {
    pub fn me(&self) -> Author {
        Author { name: self.state.identity.name.clone(), kind: self.state.identity.kind }
    }

    pub fn room(&self, room_id: &str) -> Result<&RoomState, SdkError> {
        self.state.rooms.get(room_id).ok_or_else(|| SdkError::UnknownRoom(room_id.into()))
    }

    pub fn room_mut(&mut self, room_id: &str) -> Result<&mut RoomState, SdkError> {
        self.state.rooms.get_mut(room_id).ok_or_else(|| SdkError::UnknownRoom(room_id.into()))
    }

    /// Reads each room's admins from its MLS group and pins every device in its tree. State
    /// saved before rooms tracked their admin takes the creator, the first leaf of the tree.
    fn adopt_trees(&mut self) -> Result<(), SdkError> {
        let me = self.state.identity.device_id.clone();
        for room in self.state.rooms.values_mut() {
            let leaves = self.mls.members(&room.group_id)?;
            if room.admin_id.is_empty() {
                room.admin_id = leaves.first().map(|leaf| leaf.device_id.clone()).unwrap_or_default();
            }
            room.adopt_admins(self.mls.admins(&room.group_id)?, &me);
            pin_new(&mut self.state.pins, &leaves);
        }
        self.forget_departed();
        Ok(())
    }

    /// Drops the pinned keys of devices that share no room with this device any more.
    pub fn forget_departed(&mut self) {
        let state = &mut self.state;
        let members = state.rooms.values().flat_map(|room| room.members.iter().map(|member| member.device_id.as_str()));
        let present: BTreeSet<&str> = members.chain([state.identity.device_id.as_str()]).collect();
        prune(&mut state.pins, &present);
    }
}

#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<Mutex<Inner>>,
    pub(crate) relay: RelayClient,
    pub(crate) host: Arc<dyn ClientHost>,
    identity: Arc<Identity>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinPending {
    pub room_id: String,
    pub request_id: String,
}

impl Client {
    /// Creates a new device on the relay. Its keys never leave this process.
    pub async fn register(relay_url: &str, name: &str, kind: DeviceKind, host: Arc<dyn ClientHost>) -> Result<Self, SdkError> {
        check_relay_url(relay_url)?;
        let signer = MlsEngine::new_signer()?;
        let anonymous = RelayClient::new(relay_url, None);
        let body = RegisterDevice { public_key: signer.public().to_vec(), kind, name: name.to_string() };
        let registered = anonymous.register(&body).await?;
        let mls = MlsEngine::new(&registered.device_id, signer)?;
        let identity = Identity {
            device_id: registered.device_id,
            token: registered.token,
            name: name.to_string(),
            kind,
            relay_url: relay_url.trim_end_matches('/').to_string(),
            public_key: mls.public_key(),
        };
        let mut state = Persisted::new(identity);
        pin_new(&mut state.pins, &[crate::mls::Leaf { device_id: state.identity.device_id.clone(), signature_key: mls.public_key() }]);
        let client = Self::assemble(Inner { state, mls }, host);
        client.persist(&*client.inner.lock().await);
        Ok(client)
    }

    /// Reopens a device from the blob last given to [`ClientHost::save_state`].
    pub fn restore(state: &[u8], host: Arc<dyn ClientHost>) -> Result<Self, SdkError> {
        let (persisted, snapshot) = Persisted::decode(state)?;
        check_relay_url(&persisted.identity.relay_url)?;
        let mls = MlsEngine::restore(&persisted.identity.device_id, &persisted.identity.public_key, snapshot)?;
        let mut inner = Inner { state: persisted, mls };
        inner.adopt_trees()?;
        Ok(Self::assemble(inner, host))
    }

    fn assemble(inner: Inner, host: Arc<dyn ClientHost>) -> Self {
        let identity = Arc::new(inner.state.identity.clone());
        let relay = RelayClient::new(&identity.relay_url, Some(identity.token.clone()));
        Self { inner: Arc::new(Mutex::new(inner)), relay, host, identity }
    }

    pub fn device_id(&self) -> String {
        self.identity.device_id.clone()
    }

    pub fn name(&self) -> String {
        self.identity.name.clone()
    }

    pub fn kind(&self) -> DeviceKind {
        self.identity.kind
    }

    pub fn relay_url(&self) -> String {
        self.identity.relay_url.clone()
    }

    pub(crate) fn persist(&self, inner: &Inner) {
        match inner.mls.snapshot().and_then(|snapshot| inner.state.encode(snapshot)) {
            Ok(bytes) => self.host.save_state(bytes),
            Err(error) => self.host.on_event(ClientEvent::Error { message: format!("state not saved: {error}") }),
        }
    }

    pub async fn export_state(&self) -> Result<Vec<u8>, SdkError> {
        let inner = self.inner.lock().await;
        inner.state.encode(inner.mls.snapshot()?)
    }

    /// The device a saved state blob belongs to, as an owner entry for another device.
    pub fn owner_from_state(state: &[u8]) -> Result<Owner, SdkError> {
        let (persisted, _) = Persisted::decode(state)?;
        Ok(Owner { device_id: persisted.identity.device_id, public_key: persisted.identity.public_key })
    }

    /// Devices whose join requests this device approves without asking, such as its owner,
    /// as long as the request carries the owner's key.
    pub async fn set_owners(&self, owners: Vec<Owner>) {
        let mut inner = self.inner.lock().await;
        inner.state.auto_approve.owners = owners;
        self.persist(&inner);
    }

    pub async fn rooms(&self) -> Vec<RoomInfo> {
        self.inner.lock().await.state.rooms.values().map(RoomState::info).collect()
    }

    pub async fn room(&self, room_id: &str) -> Result<RoomInfo, SdkError> {
        Ok(self.inner.lock().await.room(room_id)?.info())
    }

    /// Messages of a room with a seq above `after_seq`, oldest first.
    pub async fn messages(&self, room_id: &str, after_seq: u64) -> Result<Vec<ChatMessage>, SdkError> {
        let inner = self.inner.lock().await;
        Ok(inner.room(room_id)?.messages.iter().filter(|message| message.seq > after_seq).cloned().collect())
    }

    /// Applies inbox events in cursor order; replays are skipped.
    pub(crate) async fn ingest(&self, events: Vec<InboxEvent>) {
        let mut emitted = Vec::new();
        let mut actions = Vec::new();
        {
            let mut inner = self.inner.lock().await;
            for event in events {
                if event.cursor <= inner.state.cursor {
                    continue;
                }
                let cursor = event.cursor;
                let outcome = apply(&mut inner, event);
                inner.state.cursor = cursor;
                emitted.extend(outcome.events);
                actions.extend(outcome.actions);
            }
            self.persist(&inner);
        }
        for event in emitted {
            self.host.on_event(event);
        }
        for action in actions {
            Box::pin(self.run_action(action)).await;
        }
    }

    async fn run_action(&self, action: Action) {
        let result = match &action {
            Action::Approve { room_id, request_id } => self.approve(room_id, request_id).await,
            Action::Remove { room_id, device_id } => self.remove_member(room_id, device_id).await,
            Action::RefreshRoom { room_id } => self.refresh_room(room_id).await.map(|_| ()),
        };
        if let Err(error) = result {
            self.host.on_event(ClientEvent::Error { message: format!("{action:?} failed: {error}") });
        }
    }

    /// Pulls every inbox event after the stored cursor.
    pub async fn sync(&self) -> Result<(), SdkError> {
        loop {
            let after = self.inner.lock().await.state.cursor;
            let page = self.relay.inbox_after(after).await?;
            if page.events.is_empty() {
                return Ok(());
            }
            self.ingest(page.events).await;
        }
    }

    /// Follows the live inbox forever, reconnecting with backoff. Spawn it once per device. A
    /// dropped or unreachable connection shows as `Connection { live: false }` only: the next
    /// connection catches up from the stored cursor, so nothing is lost.
    pub async fn run(&self) {
        let mut delay = RECONNECT_MIN_MS;
        loop {
            match self.follow().await {
                Ok(()) => delay = RECONNECT_MIN_MS,
                Err(SdkError::Network(_)) => {}
                Err(error) => self.host.on_event(ClientEvent::Error { message: format!("stream: {error}") }),
            }
            self.host.on_event(ClientEvent::Connection { live: false });
            sleep_ms(delay).await;
            delay = (delay * 2).min(RECONNECT_MAX_MS);
        }
    }

    async fn follow(&self) -> Result<(), SdkError> {
        self.sync().await?;
        self.adopt_default_rules().await;
        let after = self.inner.lock().await.state.cursor;
        let response = self.relay.stream(after).await?;
        self.host.on_event(ClientEvent::Connection { live: true });
        let mut body = response.bytes_stream();
        let mut buffer = SseBuffer::default();
        while let Some(chunk) = body.next().await {
            let records = buffer.push(&chunk?).map_err(|_| SdkError::State("the relay sent an oversized stream record".into()))?;
            for record in records {
                let event: InboxEvent = serde_json::from_str(&record.data).map_err(|error| SdkError::State(error.to_string()))?;
                self.ingest(vec![event]).await;
            }
        }
        Ok(())
    }
}
