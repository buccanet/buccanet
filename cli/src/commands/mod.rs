//! Command dispatch. Each command opens (and locks) the profile it acts as.

pub mod debug;
pub mod output;
pub mod run;
pub mod session;
pub mod setup;

use clap::Subcommand;

pub type CommandResult = Result<(), Box<dyn std::error::Error>>;

pub const DEFAULT_RELAY: &str = "http://127.0.0.1:8758";

#[derive(Subcommand)]
pub enum Command {
    /// Create a profile: a new device with its own keys, registered on the relay.
    Init {
        profile: String,
        #[arg(long)]
        name: String,
        #[arg(long, value_enum, default_value = "agent")]
        kind: debug::KindArg,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
        /// Profile of the human who runs this agent; their joins are approved automatically.
        #[arg(long)]
        owner: Option<String>,
    },
    /// Show the device behind a profile.
    Whoami {
        #[arg(long, short)]
        profile: String,
    },
    /// List the rooms of a profile, or of this Claude Code session.
    Rooms {
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
    },
    /// Print the messages of a room after a given seq.
    Read {
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
        room: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
    /// Create a room; prints its id.
    Create {
        #[arg(long, short)]
        profile: String,
        #[arg(long)]
        title: Option<String>,
        /// Let anyone with the link in without an admin decision.
        #[arg(long)]
        open: bool,
        #[arg(long, default_value_t = 32)]
        max_members: u32,
        #[arg(long, default_value_t = 40)]
        agent_burst: u32,
    },
    /// Create an invite link; prints it.
    Invite {
        #[arg(long, short)]
        profile: String,
        room: String,
    },
    /// Ask to join from an invite link. With a profile it waits until an admin lets it in; from
    /// inside a Claude Code session it returns at once and the room's messages reach the session.
    Join {
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
        link: String,
    },
    /// List pending join requests of a room.
    Requests {
        #[arg(long, short)]
        profile: String,
        room: String,
    },
    /// Let a pending request in: adds the device to the room's MLS group.
    Approve {
        #[arg(long, short)]
        profile: String,
        room: String,
        request: String,
    },
    /// Turn a pending request down.
    Reject {
        #[arg(long, short)]
        profile: String,
        room: String,
        request: String,
    },
    /// Send one message.
    Send {
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
        room: String,
        text: String,
    },
    /// Follow every room of a profile and print what arrives.
    Tail {
        #[arg(long, short)]
        profile: String,
    },
    /// Start an agent harness wired to Buccanet: it can join chats and wakes on new messages.
    Run {
        #[arg(value_enum)]
        harness: run::Harness,
        /// Agent profile to use; by default the first free `<harness>`, `<harness>-2`, …
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
        /// Arguments passed to the harness.
        #[arg(last = true)]
        extra: Vec<String>,
    },
    /// Own an agent profile and wake OpenClaw on new messages through its hooks endpoint.
    ServeOpenclaw {
        #[arg(long, short)]
        profile: String,
        #[arg(long, env = "OPENCLAW_HOOKS_URL", default_value = "http://127.0.0.1:18789/hooks")]
        hooks_url: String,
        #[arg(long, env = "OPENCLAW_HOOKS_TOKEN")]
        hooks_token: String,
    },
    /// Own a profile and expose the localhost bridge used by the Hermes platform plugin.
    ServeBridge {
        #[arg(long, short)]
        profile: String,
        #[arg(long, default_value_t = 8790)]
        port: u16,
        #[arg(long, env = "BUCCANET_BRIDGE_TOKEN")]
        token: String,
    },
    /// Serve MCP over stdio for one agent profile, or for the Claude Code session it runs in.
    Mcp {
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long, env = "BUCCANET_RELAY", default_value = DEFAULT_RELAY)]
        relay: String,
    },
    /// Internal: the process that owns a Claude Code session's profile.
    #[command(hide = true)]
    Attach {
        #[arg(long, short)]
        profile: String,
    },
    /// Chat in one room: lines typed on stdin are sent, arrivals are printed.
    Chat {
        #[arg(long, short)]
        profile: String,
        room: String,
    },
}

pub async fn run(command: Command) -> CommandResult {
    match command {
        Command::Init { profile, name, kind, relay, owner } => setup::init(&profile, &name, kind, &relay, owner).await,
        Command::Whoami { profile } => setup::whoami(&profile),
        Command::Rooms { profile, relay } => session::rooms_command(profile, &relay).await,
        Command::Read { profile, relay, room, after } => session::read_command(profile, &relay, &room, after).await,
        Command::Create { profile, title, open, max_members, agent_burst } => {
            debug::create(&profile, title, !open, max_members, agent_burst).await
        }
        Command::Invite { profile, room } => debug::invite(&profile, &room).await,
        Command::Join { profile, relay, link } => session::join_command(profile, &relay, &link).await,
        Command::Requests { profile, room } => debug::requests(&profile, &room).await,
        Command::Approve { profile, room, request } => debug::approve(&profile, &room, &request).await,
        Command::Reject { profile, room, request } => debug::reject(&profile, &room, &request).await,
        Command::Send { profile, relay, room, text } => session::send_command(profile, &relay, &room, &text).await,
        Command::Tail { profile } => debug::tail(&profile).await,
        Command::Chat { profile, room } => debug::chat(&profile, &room).await,
        Command::Mcp { profile, relay } => crate::mcp::server::run(&session::mcp_profile(profile, &relay).await?).await,
        Command::Attach { profile } => session::attach(&profile).await,
        Command::Run { harness, profile, relay, extra } => run::run(harness, profile, &relay, extra).await,
        Command::ServeOpenclaw { profile, hooks_url, hooks_token } => run::serve_openclaw(&profile, hooks_url, hooks_token).await,
        Command::ServeBridge { profile, port, token } => run::serve_bridge(&profile, port, token).await,
    }
}
