//! Profiles under `~/.buccanet/profiles/<name>/`: one device each, owned by one process at a time.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use buccanet_sdk::DeviceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile {0} does not exist; create it with `bucca init {0}`")]
    Missing(String),
    #[error("profile {0} already exists")]
    Exists(String),
    #[error("profile {0} is open in another process (pid {1}); one process owns a profile at a time")]
    Locked(String, String),
    #[error("profile names use letters, digits, dash and underscore")]
    BadName,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("profile file is unreadable: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileConfig {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub relay_url: String,
    /// The human device that runs this agent; its join requests are approved automatically.
    pub owner_device_id: Option<String>,
    /// When false, the `approve_member` MCP tool refuses: a requester's name could steer the agent.
    pub allow_agent_approvals: bool,
}

pub struct Profile {
    pub dir: PathBuf,
    pub config: ProfileConfig,
    _lock: File,
}

pub fn home() -> PathBuf {
    match std::env::var_os("BUCCANET_HOME") {
        Some(home) => PathBuf::from(home),
        None => dirs::home_dir().expect("a home directory").join(".buccanet"),
    }
}

pub fn profiles_dir() -> PathBuf {
    home().join("profiles")
}

fn dir_of(name: &str) -> Result<PathBuf, ProfileError> {
    let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| profiles_dir().join(name)).ok_or(ProfileError::BadName)
}

pub fn list() -> Vec<String> {
    let Ok(entries) = fs::read_dir(profiles_dir()) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("profile.json").exists())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Where the process that owns a profile answers tool calls for other MCP processes of it.
const CONTROL_SOCKET: &str = "control.sock";

pub fn control_socket(name: &str) -> Result<PathBuf, ProfileError> {
    Ok(dir_of(name)?.join(CONTROL_SOCKET))
}

pub fn read_config(name: &str) -> Result<ProfileConfig, ProfileError> {
    let path = dir_of(name)?.join("profile.json");
    if !path.exists() {
        return Err(ProfileError::Missing(name.to_string()));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

/// Writes a file readable by its owner only, atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)
}

pub fn exists(name: &str) -> bool {
    dir_of(name).map(|dir| dir.join("profile.json").exists()).unwrap_or(false)
}

impl Profile {
    pub fn create(name: &str, config: ProfileConfig) -> Result<Profile, ProfileError> {
        let dir = dir_of(name)?;
        if dir.join("profile.json").exists() {
            return Err(ProfileError::Exists(name.to_string()));
        }
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        write_private(&dir.join("profile.json"), &serde_json::to_vec_pretty(&config)?)?;
        Profile::open(name)
    }

    /// Opens and locks a profile for this process.
    pub fn open(name: &str) -> Result<Profile, ProfileError> {
        let config = read_config(name)?;
        let dir = dir_of(name)?;
        let lock_path = dir.join("profile.lock");
        let mut lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).mode(0o600).open(&lock_path)?;
        if lock.try_lock().is_err() {
            let pid = fs::read_to_string(&lock_path).unwrap_or_default();
            return Err(ProfileError::Locked(name.to_string(), pid.trim().to_string()));
        }
        lock.set_len(0)?;
        write!(lock, "{}", std::process::id())?;
        Ok(Profile { dir, config, _lock: lock })
    }

    pub fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir.join("buccanet.log")
    }

    pub fn wake_socket(&self) -> PathBuf {
        self.dir.join("wake.sock")
    }

    pub fn control_socket(&self) -> PathBuf {
        self.dir.join(CONTROL_SOCKET)
    }

    pub fn save_config(&self) -> Result<(), ProfileError> {
        write_private(&self.dir.join("profile.json"), &serde_json::to_vec_pretty(&self.config)?)?;
        Ok(())
    }
}
