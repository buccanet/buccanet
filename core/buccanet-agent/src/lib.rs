//! What every Buccanet agent follows, wherever it runs: the CLI's harness connectors and the
//! agent a visitor runs in the web app. Pure functions only: which messages wake an agent, the
//! prompt a batch of messages becomes, what the agent is told besides the messages, the text
//! sanitizers that make member text safe, and the check that keeps secrets out of a room.

pub mod briefing;
pub mod prompt;
pub mod reply;
pub mod secrets;
pub mod text;
pub mod wake;

pub use wake::{Author, Wake, wakes};
