//! The SDK host of a CLI process: state goes to the profile file, events to a channel.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use buccanet_sdk::{Client, ClientEvent, ClientHost, DeviceKind, Owner, SdkError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::profile::{self, Profile, write_private};

pub struct FileHost {
    state_path: PathBuf,
    events: UnboundedSender<ClientEvent>,
}

impl ClientHost for FileHost {
    /// Every SDK error also goes, whole, to the profile's log.
    fn on_event(&self, event: ClientEvent) {
        if let ClientEvent::Error { message } = &event {
            tracing::warn!(%message, "sdk");
        }
        let _ = self.events.send(event);
    }

    fn save_state(&self, state: Vec<u8>) {
        if let Err(error) = write_private(&self.state_path, &state) {
            tracing::error!(?error, "state not saved");
        }
    }
}

pub type Events = UnboundedReceiver<ClientEvent>;

fn host_for(profile: &Profile) -> (Arc<FileHost>, Events) {
    let (sender, receiver) = unbounded_channel();
    (Arc::new(FileHost { state_path: profile.state_path(), events: sender }), receiver)
}

pub async fn register(profile_dir_state: &Profile, relay_url: &str, name: &str, kind: DeviceKind) -> Result<(Client, Events), SdkError> {
    let (host, events) = host_for(profile_dir_state);
    let client = Client::register(relay_url, name, kind, host).await?;
    Ok((client, events))
}

/// Sends this process's log lines to the profile's `buccanet.log`; the first call wins.
fn log_to_profile(profile: &Profile) {
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(profile.log_path()) else { return };
    let _ = tracing_subscriber::fmt().with_ansi(false).with_writer(Mutex::new(file)).try_init();
}

/// Restores the profile's device and applies its owner setting.
pub async fn open_client(profile: &Profile) -> Result<(Client, Events), SdkError> {
    log_to_profile(profile);
    let (host, events) = host_for(profile);
    let state = std::fs::read(profile.state_path()).map_err(|error| SdkError::State(error.to_string()))?;
    let client = Client::restore(&state, host)?;
    client.set_owners(profile.config.owner_device_id.as_deref().and_then(local_owner).into_iter().collect()).await;
    Ok((client, events))
}

/// The owner device with its signing key, read from the local profile that holds it. An owner
/// that is not on this machine anymore gets no automatic approval.
fn local_owner(device_id: &str) -> Option<Owner> {
    profile::list().into_iter().find_map(|name| {
        let config = profile::read_config(&name).ok()?;
        (config.device_id == device_id).then_some(())?;
        Client::owner_from_state(&profile::read_state(&name).ok()?).ok()
    })
}
