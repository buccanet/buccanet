//! The Buccanet SDK in the browser. State and events cross into JavaScript as plain objects;
//! the page persists the state blob (IndexedDB) and hands it back to `restore`.

use std::sync::Arc;

use buccanet_agent::reply::{ReplyTurn, is_silent, reply_prompt};
use buccanet_agent::{secrets, wakes};
use buccanet_sdk::{ChatMessage, Client, ClientEvent, ClientHost, DeviceKind, RoomPolicy, SdkError};
use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

/// What a call throws: an `Error` whose `code` is the relay's refusal code, when there is one.
type JsResult<T> = Result<T, JsValue>;

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

fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}

fn sdk_error(error: SdkError) -> JsValue {
    let thrown = js_error(&error);
    if let Some(code) = error.relay_code() {
        let _ = Reflect::set(&thrown, &JsValue::from_str("code"), &JsValue::from_str(code));
    }
    thrown
}

fn to_js<T: serde::Serialize>(value: &T) -> JsResult<JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(js_error)
}

fn from_js<T: serde::de::DeserializeOwned>(value: JsValue) -> JsResult<T> {
    serde_wasm_bindgen::from_value(value).map_err(js_error)
}

fn kind(value: &str) -> JsResult<DeviceKind> {
    match value {
        "human" => Ok(DeviceKind::Human),
        "agent" => Ok(DeviceKind::Agent),
        other => Err(js_error(format!("unknown device kind {other}"))),
    }
}

/// The text to share for a room invite: the link, and the command AI agents run.
#[wasm_bindgen(js_name = inviteMessage)]
pub fn invite_message(link: String) -> String {
    buccanet_sdk::invite_message(&link)
}

/// The terminal command an AI agent runs to join from an invite link.
#[wasm_bindgen(js_name = agentJoinCommand)]
pub fn agent_join_command(link: String) -> String {
    buccanet_sdk::agent_join_command(&link)
}

/// Whether a message wakes the agent behind device `me`, by the rule every agent follows: a message
/// that names nobody with `@`, or names this agent.
#[wasm_bindgen(js_name = agentWakes)]
pub fn agent_wakes(message: JsValue, me: String) -> JsResult<bool> {
    let message: ChatMessage = from_js(message)?;
    Ok(wakes(&message, &me))
}

/// The system and user messages of one turn of an agent whose answer is its message.
#[wasm_bindgen(js_name = agentPrompt)]
pub fn agent_prompt(turn: JsValue) -> JsResult<JsValue> {
    let turn: ReplyTurn = from_js(turn)?;
    to_js(&reply_prompt(&turn))
}

/// Whether an agent's answer asks to stay quiet.
#[wasm_bindgen(js_name = agentSilent)]
pub fn agent_silent(answer: String) -> bool {
    is_silent(&answer)
}

/// The gitleaks rule a text matches when it holds something shaped like a key, else null.
#[wasm_bindgen(js_name = checkSecret)]
pub fn check_secret(text: String) -> Option<String> {
    secrets::key_shaped(&text).map(str::to_string)
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
    ) -> JsResult<BuccanetClient> {
        let host = Arc::new(JsHost { on_event, on_state });
        let client = Client::register(&relay_url, &name, kind(&device_kind)?, host).await.map_err(sdk_error)?;
        Ok(BuccanetClient { client })
    }

    pub fn restore(state: Uint8Array, on_event: Function, on_state: Function) -> JsResult<BuccanetClient> {
        let host = Arc::new(JsHost { on_event, on_state });
        let client = Client::restore(&state.to_vec(), host).map_err(sdk_error)?;
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

    pub async fn sync(&self) -> JsResult<()> {
        self.client.sync().await.map_err(sdk_error)
    }

    pub async fn rooms(&self) -> JsResult<JsValue> {
        to_js(&self.client.rooms().await)
    }

    pub async fn messages(&self, room_id: String, after_seq: f64) -> JsResult<JsValue> {
        to_js(&self.client.messages(&room_id, after_seq as u64).await.map_err(sdk_error)?)
    }

    #[wasm_bindgen(js_name = createRoom)]
    pub async fn create_room(&self, title: Option<String>, approval: bool, max_members: u32) -> JsResult<JsValue> {
        let policy = RoomPolicy { approval, max_members, ..RoomPolicy::default() };
        to_js(&self.client.create_room(title, policy).await.map_err(sdk_error)?)
    }

    #[wasm_bindgen(js_name = createInvite)]
    pub async fn create_invite(&self, room_id: String) -> JsResult<String> {
        self.client.create_invite(&room_id).await.map_err(sdk_error)
    }

    #[wasm_bindgen(js_name = previewInvite)]
    pub async fn preview_invite(&self, link: String) -> JsResult<JsValue> {
        to_js(&self.client.preview_invite(&link).await.map_err(sdk_error)?)
    }

    /// What the relay shows of the room listed under `slug`.
    pub async fn listing(&self, slug: String) -> JsResult<JsValue> {
        to_js(&self.client.listing(&slug).await.map_err(sdk_error)?)
    }

    /// Asks to join the room listed under `slug`; the room arrives once an admin lets the device in.
    #[wasm_bindgen(js_name = joinListing)]
    pub async fn join_listing(&self, slug: String) -> JsResult<JsValue> {
        to_js(&self.client.join_listing(&slug).await.map_err(sdk_error)?)
    }

    pub async fn join(&self, link: String) -> JsResult<JsValue> {
        to_js(&self.client.join(&link).await.map_err(sdk_error)?)
    }

    #[wasm_bindgen(js_name = pendingRequests)]
    pub async fn pending_requests(&self, room_id: String) -> JsResult<JsValue> {
        to_js(&self.client.pending_requests(&room_id).await.map_err(sdk_error)?)
    }

    pub async fn approve(&self, room_id: String, request_id: String) -> JsResult<()> {
        self.client.approve(&room_id, &request_id).await.map_err(sdk_error)
    }

    pub async fn reject(&self, room_id: String, request_id: String) -> JsResult<()> {
        self.client.reject(&room_id, &request_id).await.map_err(sdk_error)
    }

    #[wasm_bindgen(js_name = renameRoom)]
    pub async fn rename_room(&self, room_id: String, title: String) -> JsResult<JsValue> {
        let room = self.client.rename_room(&room_id, &title).await.map_err(sdk_error)?;
        to_js(&room)
    }

    #[wasm_bindgen(js_name = setRoomRules)]
    pub async fn set_room_rules(&self, room_id: String, text: String) -> JsResult<JsValue> {
        let room = self.client.set_room_rules(&room_id, &text).await.map_err(sdk_error)?;
        to_js(&room)
    }

    #[wasm_bindgen(js_name = leaveRoom)]
    pub async fn leave_room(&self, room_id: String) -> JsResult<()> {
        self.client.leave_room(&room_id).await.map_err(sdk_error)
    }

    #[wasm_bindgen(js_name = setAdmin)]
    pub async fn set_admin(&self, room_id: String, device_id: String, admin: bool) -> JsResult<JsValue> {
        let room = self.client.set_admin(&room_id, &device_id, admin).await.map_err(sdk_error)?;
        to_js(&room)
    }

    #[wasm_bindgen(js_name = removeMember)]
    pub async fn remove_member(&self, room_id: String, device_id: String) -> JsResult<()> {
        self.client.remove_member(&room_id, &device_id).await.map_err(sdk_error)
    }

    #[wasm_bindgen(js_name = sendText)]
    pub async fn send_text(&self, room_id: String, text: String) -> JsResult<JsValue> {
        to_js(&self.client.send_text(&room_id, &text).await.map_err(sdk_error)?)
    }
}
