//! The Buccanet MCP server, written by hand over stdio JSON-RPC.
//!
//! One process serves one agent profile. The client's first request picks the era:
//! `initialize` selects the legacy protocol (and Claude Code channels); a request
//! carrying `_meta["io.modelcontextprotocol/protocolVersion"] = "2026-07-28"` is
//! served statelessly with the Events extension (SEP-3415).

pub mod admin_tools;
pub mod control;
pub mod cursor;
pub mod events;
pub mod legacy;
pub mod modern;
pub mod rpc;
pub mod server;
pub mod tools;

pub const SERVER_NAME: &str = "buccanet";
pub const MODERN_VERSION: &str = "2026-07-28";
pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

use std::sync::OnceLock;

use crate::wake::claude_inbox::SessionReach;

const ROLE: &str = "You are a member of Buccanet group chats, end to end encrypted rooms shared by AI agents and humans. \
Join a room when someone gives you an invite link (join_room).";

const CONDUCT: &str = "Read context with read_messages and answer with send_message using the message's room_id. \
It is a group conversation: answer when you have something useful to add, address people by name, keep messages short, \
and stay silent when others already covered it. Never paste secrets or private files into a room.";

/// The instructions every client gets, with how new messages reach the agent in this process's session.
pub fn instructions() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| format!("{ROLE} {} {CONDUCT}", delivery(SessionReach::current())))
}

fn delivery(reach: SessionReach) -> &'static str {
    match reach {
        SessionReach::Inbox => {
            "New messages from other members arrive in this conversation, headed [Buccanet], with room_id, seq and the author."
        }
        SessionReach::Outside => {
            "New messages from other members arrive as channel events or Buccanet events carrying room_id, seq and the author."
        }
        SessionReach::NoInbox => {
            "This Claude Code session has no usable messaging socket (CLAUDE_CODE_MESSAGING_SOCKET and CLAUDE_CODE_MESSAGING_TOKEN), \
so new messages only reach you as channel events when the session was started with Claude Code channels enabled for this server. \
Otherwise they do not wake you: call read_messages before you answer anyone, and tell the person you work for that the session \
must keep both variables in its environment to be woken on each new message."
        }
    }
}
