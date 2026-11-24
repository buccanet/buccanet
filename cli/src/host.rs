//! The SDK host of a CLI process: state goes to the profile file, events to a channel.

use std::path::PathBuf;
use std::sync::Arc;

use buccanet_sdk::{Client, ClientEvent, ClientHost, DeviceKind, SdkError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::profile::{Profile, write_private};

pub struct FileHost {
    state_path: PathBuf,
    events: UnboundedSender<ClientEvent>,
}

impl ClientHost for FileHost {
    fn on_event(&self, event: ClientEvent) {
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

/// Restores the profile's device and applies its owner setting.
pub async fn open_client(profile: &Profile) -> Result<(Client, Events), SdkError> {
    let (host, events) = host_for(profile);
    let state = std::fs::read(profile.state_path()).map_err(|error| SdkError::State(error.to_string()))?;
    let client = Client::restore(&state, host)?;
    client.set_owners(profile.config.owner_device_id.iter().cloned().collect()).await;
    Ok((client, events))
}
