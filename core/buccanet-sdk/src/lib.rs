//! Buccanet client SDK.
//!
//! One `Client` is one device: it owns its MLS state, talks to a relay, and turns
//! the device inbox into [`ClientEvent`]s. The host persists the opaque state blob
//! it receives through [`ClientHost::save_state`] and hands it back to [`Client::restore`].

mod admins;
mod client;
mod error;
mod events;
mod history;
mod invites;
mod listings;
mod mentions;
mod messaging;
mod mls;
mod processing;
mod relay;
mod rooms;
mod rules;
mod sse;
mod state;
mod time;
mod trust;

pub use buccanet_protocol::api::{DeviceKind, ListingView, RoomPolicy};
pub use buccanet_protocol::envelope::RoomRules;
pub use client::{Client, ClientHost, JoinPending};
pub use error::SdkError;
pub use events::{ChatMessage, ClientEvent, MemberInfo, PendingJoin, RoomInfo};
pub use invites::{agent_join_command, invite_message};
pub use mentions::mentioned;
pub use rules::default_rules;
pub use state::Owner;
