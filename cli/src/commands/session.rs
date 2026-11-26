//! Commands an agent runs from inside its harness session, with no profile to name: `join`,
//! `send`, `read`, `rooms` and `admin`. In a Claude Code session the first one creates the
//! session's profile and starts the process that owns it (`wake::claude_session`); every
//! command then goes through that process. Any other agent that runs `join` gets the shared
//! agent profile, which it names on its next commands.

use std::io::Read;
use std::process::Stdio;
use std::time::Duration;

use buccanet_sdk::{SdkError, check_invite_relay};
use serde_json::{Value, json};

use super::run::{AGENT_PROFILE, OWNER_PROFILE};
use super::setup::{NewProfile, create_profile};
use super::{CommandResult, DEFAULT_RELAY, debug};
use crate::mcp::control::Backend;
use crate::profile::{self, Profile, ProfileError};
use crate::wake::claude_inbox::SessionReach;
use crate::wake::claude_session::ClaudeSession;

const STARTUP: Duration = Duration::from_secs(15);

/// Whom a command acts as: a named profile, or the harness session it runs in.
pub enum Target {
    Profile(String),
    Session(ClaudeSession),
}

pub fn target(profile: Option<String>) -> Result<Target, Box<dyn std::error::Error>> {
    match (profile, ClaudeSession::from_env()) {
        (Some(name), _) => Ok(Target::Profile(name)),
        (None, Some(session)) => Ok(Target::Session(session)),
        (None, None) => Err("pass --profile, or run this from inside a Claude Code session".into()),
    }
}

/// The session's agent profile, created the first time this session uses Buccanet.
async fn session_profile(session: &ClaudeSession, relay: &str) -> Result<String, Box<dyn std::error::Error>> {
    let name = session.profile();
    if !profile::exists(&name) {
        let owner = profile::exists(OWNER_PROFILE).then_some(OWNER_PROFILE);
        let display = ClaudeSession::display_name();
        create_profile(NewProfile {
            profile: &name,
            display: &display,
            kind: buccanet_sdk::DeviceKind::Agent,
            relay,
            owner_profile: owner,
        })
        .await?;
    }
    Ok(name)
}

/// The profile `bucca mcp` serves: the one named, or this session's.
pub async fn mcp_profile(profile: Option<String>, relay: &str) -> Result<String, Box<dyn std::error::Error>> {
    match target(profile)? {
        Target::Profile(name) => Ok(name),
        Target::Session(session) => session_profile(&session, relay).await,
    }
}

/// The process owning the session's profile, started the first time.
async fn owner(session: &ClaudeSession, relay: &str) -> Result<Backend, Box<dyn std::error::Error>> {
    let name = session_profile(session, relay).await?;
    let socket = profile::control_socket(&name)?;
    match Profile::open(&name) {
        Err(ProfileError::Locked(..)) => return Ok(Backend::Remote(socket)),
        Err(error) => return Err(error.into()),
        Ok(probe) => drop(probe),
    }
    start_owner(&name)?;
    let deadline = tokio::time::Instant::now() + STARTUP;
    while tokio::net::UnixStream::connect(&socket).await.is_err() {
        if tokio::time::Instant::now() > deadline {
            return Err(format!(
                "the Buccanet session process did not start; see {}",
                profile::profiles_dir().join(&name).join("buccanet.log").display()
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(Backend::Remote(socket))
}

/// Detached from the command's process group, so it outlives the shell the agent ran.
fn start_owner(name: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(profile::profiles_dir().join(name).join("buccanet.log"))?;
    std::process::Command::new(std::env::current_exe()?)
        .args(["attach", "--profile", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    Ok(())
}

async fn call(session: &ClaudeSession, relay: &str, tool: &str, args: Value) -> CommandResult {
    let output = owner(session, relay).await?.call(tool, &args).await.map_err(|error| error.message)?;
    println!("{}", output.text);
    if output.is_error {
        return Err("the command failed".into());
    }
    Ok(())
}

pub async fn rooms_command(profile: Option<String>, relay: &str) -> CommandResult {
    match target(profile)? {
        Target::Profile(name) => debug::rooms(&name).await,
        Target::Session(session) => call(&session, relay, "list_rooms", json!({})).await,
    }
}

pub async fn rules_command(profile: Option<String>, relay: &str, room: &str, new_rules: Option<String>) -> CommandResult {
    let (tool, args) = match new_rules {
        Some(text) => ("set_room_rules", json!({ "roomId": room, "text": text })),
        None => ("get_room_rules", json!({ "roomId": room })),
    };
    match target(profile)? {
        Target::Profile(name) => debug::tool(&name, tool, args).await,
        Target::Session(session) => call(&session, relay, tool, args).await,
    }
}

pub async fn leave_command(profile: Option<String>, relay: &str, room: &str) -> CommandResult {
    match target(profile)? {
        Target::Profile(name) => debug::leave(&name, room).await,
        Target::Session(session) => call(&session, relay, "leave_room", json!({ "roomId": room })).await,
    }
}

pub async fn read_command(profile: Option<String>, relay: &str, room: &str, after: u64) -> CommandResult {
    match target(profile)? {
        Target::Profile(name) => debug::read(&name, room, after).await,
        Target::Session(session) => call(&session, relay, "read_messages", json!({ "roomId": room, "afterSeq": after })).await,
    }
}

/// The text of `bucca send`: the argument, or stdin up to its end without the final line break.
pub fn message_text(text: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    let text = match text {
        Some(text) => text,
        None => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            input.strip_suffix('\n').map(str::to_string).unwrap_or(input)
        }
    };
    if text.trim().is_empty() {
        return Err("nothing to send: pass the text, or write it on stdin with bucca send <room id> <<'EOF'".into());
    }
    Ok(text)
}

pub async fn send_command(profile: Option<String>, relay: &str, room: &str, text: &str) -> CommandResult {
    match target(profile)? {
        Target::Profile(name) => debug::send(&name, room, text).await,
        Target::Session(session) => call(&session, relay, "send_message", json!({ "roomId": room, "text": text })).await,
    }
}

/// The relay behind an invite link: the web app and its relay share one origin, so a link
/// `<origin>/join/<token>` names its relay. A bare token names none. Anyone can write a link, so
/// its relay takes HTTPS, or plain HTTP on this machine only; `--relay` names any other one.
pub fn relay_of_invite(link: &str) -> Result<Option<String>, SdkError> {
    let Some((origin, _)) = link.trim().split_once("/join/") else { return Ok(None) };
    check_invite_relay(origin)?;
    Ok(Some(origin.to_string()))
}

/// Joins from an invite link. A new profile registers on `relay` when one is named, else on the
/// relay of the link, else on the default relay.
pub async fn join_command(profile: Option<String>, relay: Option<String>, link: &str) -> CommandResult {
    let relay = match relay {
        Some(relay) => relay,
        None => relay_of_invite(link)?.unwrap_or_else(|| DEFAULT_RELAY.to_string()),
    };
    match (profile, ClaudeSession::from_env()) {
        (Some(name), _) => join_as_profile(&name, link).await,
        (None, Some(session)) => join(&session, &relay, link).await,
        (None, None) => join_as_agent(&relay, link).await,
    }
}

/// A named profile joins on its own. When a Claude Code session runs the command, nothing
/// follows that profile once it returns, so the session learns why it will not be woken.
async fn join_as_profile(name: &str, link: &str) -> CommandResult {
    debug::join(name, debug::JoinTarget::Invite(link)).await?;
    let bucca = crate::update::command();
    if let Some(notice) = wake_notice(SessionReach::current(), Some(name), bucca) {
        print!("{notice}{}", listening_guide(bucca, name));
    }
    Ok(())
}

/// Why a Claude Code session that joined this way stays asleep, and how to get woken.
fn wake_notice(reach: SessionReach, profile: Option<&str>, bucca: &str) -> Option<String> {
    match (reach, profile) {
        (SessionReach::NoInbox, _) => Some(format!(
            "This Claude Code session has no usable messaging socket: CLAUDE_CODE_MESSAGING_SOCKET or \
             CLAUDE_CODE_MESSAGING_TOKEN is missing, or the socket belongs to a process this command does not descend from. \
             New messages cannot wake it. To be woken on each new message, keep both variables in the session's environment \
             and join with `{bucca} join <invite link>`, without -p.\n"
        )),
        (SessionReach::Inbox, Some(name)) => Some(format!(
            "This Claude Code session joined as the profile `{name}`, and messages to a named profile do not reach the session. \
             To be woken on each new message, run `{bucca} leave -p {name} <room id>`, then `{bucca} join <invite link>`, without -p.\n"
        )),
        (SessionReach::Inbox, None) | (SessionReach::Outside, _) => None,
    }
}

/// An agent outside Claude Code, such as Codex pasted an invite: it joins as the shared agent
/// profile, waits for an admin, then learns the commands that read, answer and wait.
async fn join_as_agent(relay: &str, link: &str) -> CommandResult {
    if !profile::exists(AGENT_PROFILE) {
        let owner = profile::exists(OWNER_PROFILE).then_some(OWNER_PROFILE);
        let display = agent_display_name();
        create_profile(NewProfile {
            profile: AGENT_PROFILE,
            display: &display,
            kind: buccanet_sdk::DeviceKind::Agent,
            relay,
            owner_profile: owner,
        })
        .await?;
    }
    debug::join(AGENT_PROFILE, debug::JoinTarget::Invite(link)).await?;
    let bucca = crate::update::command();
    let notice = wake_notice(SessionReach::current(), None, bucca).unwrap_or_default();
    print!("{notice}{}", listening_guide(bucca, AGENT_PROFILE));
    Ok(())
}

/// How an agent with no harness to wake it stays in the conversation: nothing reaches it
/// between its own commands, so it keeps a `wait` running.
fn listening_guide(bucca: &str, p: &str) -> String {
    format!(
        "You are in the room. Nothing wakes you when someone writes, so stay in the conversation by looping:\n\
         1. Run `{bucca} wait -p {p}`. It returns with the new messages, or after 5 minutes without any. \
         Give your terminal tool a timeout of at least 6 minutes.\n\
         2. When a message calls for an answer, send it with a quoted heredoc:\n\
         {bucca} send -p {p} <room id> <<'EOF'\n<your message>\nEOF\n\
         3. Run wait again, until the person you work for tells you to stop.\n\
         Read earlier messages with `{bucca} read -p {p} <room id>`.\n"
    )
}

/// The name people see for an agent that joined on its own: its harness when it shows one.
fn agent_display_name() -> String {
    let codex = std::env::vars().any(|(key, _)| key.starts_with("CODEX_"));
    if codex { "Codex".to_string() } else { "Agent".to_string() }
}

pub async fn admin_command(profile: Option<String>, relay: &str, room: &str, member: &str, admin: bool) -> CommandResult {
    admin_tool(profile, relay, "set_admin", json!({ "roomId": room, "member": member, "admin": admin })).await
}

pub async fn remove_command(profile: Option<String>, relay: &str, room: &str, member: &str) -> CommandResult {
    admin_tool(profile, relay, "remove_member", json!({ "roomId": room, "member": member })).await
}

/// Runs a room admin tool as the profile named or as this session; a profile held by another
/// process acts through it.
async fn admin_tool(profile: Option<String>, relay: &str, tool: &str, args: Value) -> CommandResult {
    match target(profile)? {
        Target::Profile(name) => debug::tool(&name, tool, args).await,
        Target::Session(session) => call(&session, relay, tool, args).await,
    }
}

async fn join(session: &ClaudeSession, relay: &str, link: &str) -> CommandResult {
    call(session, relay, "join_room", json!({ "invite": link })).await?;
    println!(
        "New messages from the room will arrive in this conversation. Answer with a quoted heredoc, which the shell passes on untouched:"
    );
    let bucca = crate::update::command();
    println!("{bucca} send <room id> <<'EOF'\n<your message>\nEOF");
    println!("To stop receiving a room's messages: {bucca} leave <room id>");
    Ok(())
}

/// `bucca attach`: the process `owner` starts.
pub async fn attach(name: &str) -> CommandResult {
    let session = ClaudeSession::from_env().ok_or("attach runs inside a Claude Code session")?;
    crate::wake::claude_session::serve(Profile::open(name)?, session.inbox).await
}

#[cfg(test)]
mod tests {
    use super::{relay_of_invite, wake_notice};
    use crate::wake::claude_inbox::SessionReach;

    #[test]
    fn a_session_that_cannot_be_woken_is_told_why() {
        let removed = wake_notice(SessionReach::NoInbox, Some("ocr"), "bucca").expect("a notice");
        assert!(removed.contains("CLAUDE_CODE_MESSAGING_SOCKET") && removed.contains("bucca join <invite link>"));
        let named = wake_notice(SessionReach::Inbox, Some("ocr"), "bucca").expect("a notice");
        assert!(named.contains("bucca leave -p ocr <room id>"));
        assert_eq!(wake_notice(SessionReach::Inbox, None, "bucca"), None);
        assert_eq!(wake_notice(SessionReach::Outside, Some("ocr"), "bucca"), None);
    }

    #[test]
    fn an_invite_link_names_an_https_relay_or_one_on_this_machine() {
        let relay = |link: &str| relay_of_invite(link).ok().flatten();
        assert_eq!(relay("https://app.buccanet.com/join/abc").as_deref(), Some("https://app.buccanet.com"));
        assert_eq!(relay("http://localhost:8759/join/abc").as_deref(), Some("http://localhost:8759"));
        assert_eq!(relay("abc"), None);
        for refused in ["ftp://x/join/abc", "http://app:8758/join/abc", "http://192.168.1.4/join/abc", "http://relay.example.com/join/abc"]
        {
            assert!(relay_of_invite(refused).is_err(), "{refused} is accepted");
        }
    }
}
