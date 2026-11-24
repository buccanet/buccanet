//! The Buccanet SDK in the browser. State and events cross into JavaScript as plain objects;
//! the page persists the state blob (IndexedDB) and hands it back to `restore`.

use std::sync::Arc;

use buccanet_sdk::{Client, ClientEvent, ClientHost, DeviceKind, RoomPolicy};
use js_sys::{Function, Uint8Array};
use wasm_bindgen::prelude::*;

struct JsHost {
    on_event: Function,
    on_state: Function,
}

// The browser runs this module on one thread; the SDK's Send bounds are never exercised across threads.
unsafe impl Send for JsHost {}
unsafe impl Sync for JsHost {}

impl ClientHost for JsHost {
    fn on_event(&self, event: ClientEvent) {
        if let Ok(value) = serde_wasm_bindgen::to_value(&event) {
            let _ = self.on_event.call1(&JsValue::NULL, &value);
        }
    }

    fn save_state(&self, state: Vec<u8>) {
        let _ = self.on_state.call1(&JsValue::NULL, &Uint8Array::from(state.as_slice()));
    }
}

fn js_error(error: impl std::fmt::Display) -> JsError {
    JsError::new(&error.to_string())
}

fn to_js<T: serde::Serialize>(value: &T) -> Result<JsValue, JsError> {
    serde_wasm_bindgen::to_value(value).map_err(js_error)
}

fn kind(value: &str) -> Result<DeviceKind, JsError> {
    match value {
        "human" => Ok(DeviceKind::Human),
        "agent" => Ok(DeviceKind::Agent),
        other => Err(JsError::new(&format!("unknown device kind {other}"))),
    }
}

#[wasm_bindgen]
pub struct BuccanetClient {
    client: Client,
}

#[wasm_bindgen]
impl BuccanetClient {
    /// Creates a device on the relay; its keys stay in this page.
    pub async fn register(
        relay_url: String,
        name: String,
        device_kind: String,
        on_event: Function,
        on_state: Function,
    ) -> Result<BuccanetClient, JsError> {
        let host = Arc::new(JsHost { on_event, on_state });
        let client = Client::register(&relay_url, &name, kind(&device_kind)?, host).await.map_err(js_error)?;
        Ok(BuccanetClient { client })
    }

    pub fn restore(state: Uint8Array, on_event: Function, on_state: Function) -> Result<BuccanetClient, JsError> {
        let host = Arc::new(JsHost { on_event, on_state });
        let client = Client::restore(&state.to_vec(), host).map_err(js_error)?;
        Ok(BuccanetClient { client })
    }

    #[wasm_bindgen(js_name = deviceId)]
    pub fn device_id(&self) -> String {
        self.client.device_id()
    }

    pub fn name(&self) -> String {
        self.client.name()
    }

    /// Follows the inbox forever; call once and do not await.
    pub async fn run(&self) {
        self.client.run().await;
    }

    pub async fn sync(&self) -> Result<(), JsError> {
        self.client.sync().await.map_err(js_error)
    }

    pub async fn rooms(&self) -> Result<JsValue, JsError> {
        to_js(&self.client.rooms().await)
    }

    pub async fn messages(&self, room_id: String, after_seq: f64) -> Result<JsValue, JsError> {
        to_js(&self.client.messages(&room_id, after_seq as u64).await.map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = createRoom)]
    pub async fn create_room(&self, title: Option<String>, approval: bool, max_members: u32) -> Result<JsValue, JsError> {
        let policy = RoomPolicy { approval, max_members, ..RoomPolicy::default() };
        to_js(&self.client.create_room(title, policy).await.map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = createInvite)]
    pub async fn create_invite(&self, room_id: String) -> Result<String, JsError> {
        self.client.create_invite(&room_id).await.map_err(js_error)
    }

    #[wasm_bindgen(js_name = previewInvite)]
    pub async fn preview_invite(&self, link: String) -> Result<JsValue, JsError> {
        to_js(&self.client.preview_invite(&link).await.map_err(js_error)?)
    }

    pub async fn join(&self, link: String) -> Result<JsValue, JsError> {
        to_js(&self.client.join(&link).await.map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = pendingRequests)]
    pub async fn pending_requests(&self, room_id: String) -> Result<JsValue, JsError> {
        to_js(&self.client.pending_requests(&room_id).await.map_err(js_error)?)
    }

    pub async fn approve(&self, room_id: String, request_id: String) -> Result<(), JsError> {
        self.client.approve(&room_id, &request_id).await.map_err(js_error)
    }

    pub async fn reject(&self, room_id: String, request_id: String) -> Result<(), JsError> {
        self.client.reject(&room_id, &request_id).await.map_err(js_error)
    }

    #[wasm_bindgen(js_name = removeMember)]
    pub async fn remove_member(&self, room_id: String, device_id: String) -> Result<(), JsError> {
        self.client.remove_member(&room_id, &device_id).await.map_err(js_error)
    }

    #[wasm_bindgen(js_name = sendText)]
    pub async fn send_text(&self, room_id: String, text: String) -> Result<JsValue, JsError> {
        to_js(&self.client.send_text(&room_id, &text).await.map_err(js_error)?)
    }
}
