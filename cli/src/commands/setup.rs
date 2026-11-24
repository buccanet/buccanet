//! Creating and inspecting profiles.

use buccanet_sdk::DeviceKind;

use super::CommandResult;
use super::debug::KindArg;
use super::output::kind_label;
use crate::host::register;
use crate::profile::{self, Profile, ProfileConfig};

pub struct NewProfile<'a> {
    pub profile: &'a str,
    pub display: &'a str,
    pub kind: DeviceKind,
    pub relay: &'a str,
    pub owner_profile: Option<&'a str>,
}

/// Registers a new device and writes its profile.
pub async fn create_profile(spec: NewProfile<'_>) -> Result<ProfileConfig, Box<dyn std::error::Error>> {
    let owner_device_id = match spec.owner_profile {
        Some(owner) => Some(profile::read_config(owner)?.device_id),
        None => None,
    };
    let draft = ProfileConfig {
        device_id: String::new(),
        name: spec.display.to_string(),
        kind: spec.kind,
        relay_url: spec.relay.to_string(),
        owner_device_id,
        allow_agent_approvals: false,
    };
    let mut created = Profile::create(spec.profile, draft)?;
    let (client, _) = register(&created, spec.relay, spec.display, spec.kind).await?;
    created.config.device_id = client.device_id();
    created.save_config()?;
    Ok(created.config.clone())
}

pub async fn init(name: &str, display: &str, kind: KindArg, relay: &str, owner: Option<String>) -> CommandResult {
    let spec = NewProfile { profile: name, display, kind: kind.into(), relay, owner_profile: owner.as_deref() };
    println!("{}", create_profile(spec).await?.device_id);
    Ok(())
}

/// First run of `bucca`: asks the person's name and creates their profile.
pub async fn onboard(name: &str, relay: &str) -> CommandResult {
    eprint!("Welcome to Buccanet. Your name: ");
    let mut display = String::new();
    std::io::stdin().read_line(&mut display)?;
    let display = display.trim();
    if display.is_empty() {
        return Err("a name is needed to start".into());
    }
    create_profile(NewProfile { profile: name, display, kind: DeviceKind::Human, relay, owner_profile: None }).await?;
    Ok(())
}

pub fn whoami(name: &str) -> CommandResult {
    let config = profile::read_config(name)?;
    println!("{} {} ({}) on {}", config.device_id, config.name, kind_label(config.kind), config.relay_url);
    Ok(())
}
