//! What the admission service keeps beside its profile: the room it serves, when it created
//! it, and when each member joined and last wrote, in `public.json`. The operator's blocklist
//! is the `blocklist` file of the profile folder, one device id per line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::profile::write_private;

const LEDGER_FILE: &str = "public.json";
const BLOCKLIST_FILE: &str = "blocklist";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub joined_ms: i64,
    pub active_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub room_id: Option<String>,
    pub created_ms: i64,
    pub members: BTreeMap<String, Presence>,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ledger_path(dir: &Path) -> PathBuf {
    dir.join(LEDGER_FILE)
}

impl Ledger {
    pub fn load(dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        match std::fs::read(ledger_path(dir)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        write_private(&ledger_path(dir), &serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)
    }

    /// A new room to serve, with nobody known in it yet.
    pub fn start_room(&mut self, room_id: &str, now: i64) {
        *self = Self { room_id: Some(room_id.to_string()), created_ms: now, members: BTreeMap::new() };
    }

    /// Keeps a presence for each member of the room, new ones counted from `now`, and forgets
    /// the others.
    pub fn adopt(&mut self, member_ids: &[String], now: i64) {
        self.members.retain(|device_id, _| member_ids.contains(device_id));
        for device_id in member_ids {
            self.members.entry(device_id.clone()).or_insert(Presence { joined_ms: now, active_ms: now });
        }
    }

    pub fn wrote(&mut self, device_id: &str, now: i64) {
        if let Some(presence) = self.members.get_mut(device_id) {
            presence.active_ms = now;
        }
    }

    /// Members who joined more than `idle` ago and wrote nothing for as long, `keep` excepted.
    pub fn idle(&self, now: i64, idle: Duration, keep: &[String]) -> Vec<String> {
        let limit = now - idle.as_millis() as i64;
        self.members
            .iter()
            .filter(|(device_id, presence)| !keep.contains(device_id) && presence.joined_ms < limit && presence.active_ms < limit)
            .map(|(device_id, _)| device_id.clone())
            .collect()
    }
}

/// The device ids the operator blocked, comments and blank lines skipped.
pub fn blocklist(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(BLOCKLIST_FILE)) else { return Vec::new() };
    text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_quiet_since_they_joined_long_ago_are_idle() {
        let mut ledger = Ledger::default();
        ledger.start_room("r", 0);
        ledger.adopt(&["door".into(), "quiet".into(), "talker".into()], 0);
        ledger.wrote("talker", 50_000);
        ledger.adopt(&["door".into(), "quiet".into(), "talker".into(), "late".into()], 55_000);
        let idle = ledger.idle(70_000, Duration::from_secs(30), &["door".into()]);
        assert_eq!(idle, vec!["quiet".to_string()]);
        ledger.adopt(&["door".into(), "talker".into()], 70_000);
        assert!(!ledger.members.contains_key("quiet"));
    }
}
