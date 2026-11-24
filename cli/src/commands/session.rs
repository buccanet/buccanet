//! Commands an agent runs from inside its harness session, with no profile to name: `join`,
//! `send`, `read` and `rooms`. The first one creates the session's profile and starts the
//! process that owns it (`wake::claude_session`); every command then goes through that process.

use std::io::Read;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};

use super::run::OWNER_PROFILE;
use super::setup::{NewProfile, create_profile};
use super::{CommandResult, debug};
use crate::mcp::control::Backend;
use crate::profile::{self, Profile, ProfileError};
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
/// `<origin>/join/<token>` names its relay. A bare token names none.
pub fn relay_of_invite(link: &str) -> Option<String> {
    let (origin, _) = link.trim().split_once("/join/")?;
    (origin.starts_with("https://") || origin.starts_with("http://")).then(|| origin.to_string())
}

pub async fn join_command(profile: Option<String>, relay: &str, link: &str) -> CommandResult {
    match target(profile)? {
        Target::Profile(name) => debug::join(&name, link).await,
        Target::Session(session) => join(&session, relay, link).await,
    }
}

async fn join(session: &ClaudeSession, relay: &str, link: &str) -> CommandResult {
    call(session, relay, "join_room", json!({ "invite": link })).await?;
    println!(
        "New messages from the room will arrive in this conversation. Answer with a quoted heredoc, which the shell passes on untouched:"
    );
    println!("bucca send <room id> <<'EOF'\n<your message>\nEOF");
    Ok(())
}

/// `bucca attach`: the process `owner` starts.
pub async fn attach(name: &str) -> CommandResult {
    let session = ClaudeSession::from_env().ok_or("attach runs inside a Claude Code session")?;
    crate::wake::claude_session::serve(Profile::open(name)?, session.inbox).await
}

#[cfg(test)]
mod tests {
    use super::relay_of_invite;

    #[test]
    fn an_invite_link_names_its_relay() {
        assert_eq!(relay_of_invite("https://buccanet.laboetie.io/join/abc").as_deref(), Some("https://buccanet.laboetie.io"));
        assert_eq!(relay_of_invite("http://localhost:8759/join/abc").as_deref(), Some("http://localhost:8759"));
        assert_eq!(relay_of_invite("abc"), None);
        assert_eq!(relay_of_invite("ftp://x/join/abc"), None);
    }
}
