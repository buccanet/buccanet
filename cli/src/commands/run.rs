//! `bucca run <harness>` and `bucca serve`: plug an agent harness into Buccanet.

use std::path::PathBuf;
use std::sync::Arc;

use buccanet_sdk::DeviceKind;
use clap::ValueEnum;

use super::CommandResult;
use super::setup::{NewProfile, create_profile};
use crate::host::open_client;
use crate::mcp::control;
use crate::mcp::tools::ToolContext;
use crate::profile::{self, Profile, ProfileError};
use crate::wake::{self, bridge, claude, codex, openclaw, opencode};

/// The human profile that owns the agents started on this machine.
pub const OWNER_PROFILE: &str = "me";

#[derive(Clone, Copy, ValueEnum)]
pub enum Harness {
    Claude,
    Codex,
    Opencode,
}

impl Harness {
    fn slug(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
        }
    }

    fn display(self) -> &'static str {
        match self {
            Harness::Claude => "Claude",
            Harness::Codex => "Codex",
            Harness::Opencode => "OpenCode",
        }
    }
}

/// What a connector needs to start a harness on a profile.
pub struct Launch {
    pub profile: String,
    pub dir: PathBuf,
    pub exe: String,
    /// Passed to the MCP child explicitly: harnesses start MCP servers with a cleaned environment.
    pub home: String,
}

impl Launch {
    pub fn wake_socket(&self) -> PathBuf {
        self.dir.join("wake.sock")
    }
}

fn is_free(name: &str) -> bool {
    !matches!(Profile::open(name), Err(ProfileError::Locked(..)))
}

/// The given profile, or the first `<harness>`, `<harness>-2`, … not used by a running process.
async fn agent_profile(harness: Harness, wanted: Option<String>, relay: &str) -> Result<String, Box<dyn std::error::Error>> {
    let candidates: Vec<(String, String)> = match wanted {
        Some(name) => vec![(name.clone(), name)],
        None => (1..=50)
            .map(|n| match n {
                1 => (harness.slug().to_string(), harness.display().to_string()),
                n => (format!("{}-{n}", harness.slug()), format!("{} {n}", harness.display())),
            })
            .collect(),
    };
    for (name, display) in candidates {
        if profile::exists(&name) {
            if is_free(&name) {
                return Ok(name);
            }
            continue;
        }
        let owner = profile::exists(OWNER_PROFILE).then_some(OWNER_PROFILE);
        create_profile(NewProfile { profile: &name, display: &display, kind: DeviceKind::Agent, relay, owner_profile: owner }).await?;
        eprintln!("buccanet: created agent profile {name}");
        return Ok(name);
    }
    Err("every agent profile is in use".into())
}

fn launch_for(name: String) -> Result<Launch, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let home = profile::home().to_string_lossy().into_owned();
    Ok(Launch { dir: profile::profiles_dir().join(&name), profile: name, exe, home })
}

pub async fn run(harness: Harness, wanted: Option<String>, relay: &str, extra: Vec<String>) -> CommandResult {
    let name = agent_profile(harness, wanted, relay).await?;
    let launch = launch_for(name)?;
    eprintln!("buccanet: {} runs as profile {}; tell it \"join this chat: <invite link>\"", harness.display(), launch.profile);
    let status = match harness {
        Harness::Claude => claude::launch(&launch, &extra).await?,
        Harness::Codex => codex::launch(&launch, &extra).await.map_err(|error| error.to_string())?,
        Harness::Opencode => opencode::launch(&launch, &extra).await.map_err(|error| error.to_string())?,
    };
    if !status.success() {
        return Err(format!("{} exited with {status}", harness.display()).into());
    }
    Ok(())
}

/// A secret read from a file, or else from an environment variable. Never a flag value: every
/// local user sees process arguments.
pub fn secret(variable: &str, file: Option<PathBuf>) -> Result<String, Box<dyn std::error::Error>> {
    let value = match file {
        Some(path) => std::fs::read_to_string(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
        None => std::env::var(variable).map_err(|_| format!("set {variable}, or pass the path of a file holding it"))?,
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("the {variable} secret is empty").into());
    }
    Ok(value.to_string())
}

/// Owns the profile and wakes OpenClaw through its hooks; the `bucca mcp` servers OpenClaw
/// starts forward their tool calls here through the control socket.
pub async fn serve_openclaw(name: &str, hooks_url: String, hooks_token: String) -> CommandResult {
    let profile = Profile::open(name)?;
    let (client, events) = open_client(&profile).await?;
    let ctx = Arc::new(ToolContext { client: client.clone(), config: profile.config.clone(), profile_dir: profile.dir.clone() });
    control::serve(ctx, &profile.control_socket())?;
    let wakes = wake::follow(client.clone(), events, wake::briefing::Briefer::new(profile.dir.clone()));
    tokio::spawn(async move { client.run().await });
    eprintln!("buccanet: waking OpenClaw for profile {name} through {hooks_url}");
    let hooks = openclaw::Hooks { url: hooks_url, token: hooks_token };
    openclaw::serve(name, wakes, hooks).await.map_err(|error| error.to_string())?;
    Ok(())
}

/// Owns the profile and exposes the localhost bridge Hermes' Buccanet platform plugin talks to.
pub async fn serve_bridge(name: &str, port: u16, token: String) -> CommandResult {
    let profile = Profile::open(name)?;
    let (client, events) = open_client(&profile).await?;
    client.sync().await?;
    bridge::serve(client, events, port, token).await?;
    Ok(())
}
