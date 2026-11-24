//! Keeping `bucca` current: the terminal UI says when npm has a newer version, at most once a
//! day, and `bucca update` installs it the way this copy was installed.

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::commands::CommandResult;
use crate::profile;

const LATEST_URL: &str = "https://registry.npmjs.org/buccanet/latest";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LastCheck {
    checked_at: u64,
    latest: String,
}

#[derive(Deserialize)]
struct Published {
    version: String,
}

/// How this copy of `bucca` was installed, read from where its binary lives.
#[derive(Debug, PartialEq, Eq)]
enum Install {
    /// `npm i -g buccanet`: the binary sits in a platform package under node_modules.
    NpmGlobal,
    /// `npx buccanet`: the binary sits in npm's npx cache.
    Npx,
    /// Built from source, or copied by hand.
    Source,
}

fn install_of(binary: &Path) -> Install {
    let path = binary.to_string_lossy();
    if path.contains("/_npx/") {
        Install::Npx
    } else if path.contains("node_modules/buccanet-") {
        Install::NpmGlobal
    } else {
        Install::Source
    }
}

/// The command that runs this copy of `bucca`, for instructions given to agents: an agent that
/// joined through npx has no `bucca` on its path.
pub fn command() -> &'static str {
    static COMMAND: OnceLock<&'static str> = OnceLock::new();
    COMMAND.get_or_init(|| match std::env::current_exe().map(|binary| install_of(&binary)) {
        Ok(Install::Npx) => "npx -y buccanet@latest",
        _ => "bucca",
    })
}

/// True when `latest` is a higher `major.minor.patch` than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |version: &str| version.split('.').map(|part| part.parse::<u64>().ok()).collect::<Option<Vec<u64>>>();
    matches!((parse(latest), parse(current)), (Some(latest), Some(current)) if latest > current)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

async fn fetch_latest() -> Result<String, reqwest::Error> {
    let client = reqwest::Client::builder().timeout(CHECK_TIMEOUT).build()?;
    Ok(client.get(LATEST_URL).send().await?.error_for_status()?.json::<Published>().await?.version)
}

/// The latest published version, asking npm at most once a day.
async fn latest_cached() -> Option<String> {
    let path = profile::home().join("update-check.json");
    let cached: Option<LastCheck> = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if let Some(cached) = cached.filter(|cached| now().saturating_sub(cached.checked_at) < CHECK_EVERY.as_secs()) {
        return Some(cached.latest);
    }
    let latest = fetch_latest().await.ok()?;
    let record = LastCheck { checked_at: now(), latest: latest.clone() };
    if let Ok(bytes) = serde_json::to_vec(&record) {
        let _ = profile::write_private(&path, &bytes);
    }
    Some(latest)
}

/// A line for the terminal UI when npm has a newer `bucca`.
pub async fn notice() -> Option<String> {
    let latest = latest_cached().await?;
    is_newer(&latest, CURRENT).then(|| format!("bucca {latest} is out (this is {CURRENT}): run bucca update"))
}

/// `bucca update`: installs the newest version the way this copy was installed.
pub async fn run() -> CommandResult {
    let latest = fetch_latest().await?;
    if !is_newer(&latest, CURRENT) {
        println!("bucca {CURRENT} is the latest version.");
        return Ok(());
    }
    match install_of(&std::env::current_exe()?) {
        Install::NpmGlobal => {
            println!("Updating bucca {CURRENT} to {latest}…");
            let status = std::process::Command::new("npm").args(["install", "--global", "buccanet@latest"]).status()?;
            if !status.success() {
                return Err("npm could not update buccanet; run: npm install --global buccanet@latest".into());
            }
            println!("bucca {latest} is installed.");
        }
        Install::Npx => println!("bucca {latest} is out. Run it with: npx -y buccanet@latest"),
        Install::Source => println!("bucca {latest} is out. This copy was built from source: install with npm i -g buccanet"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_by_number() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.0.0-stage", "0.1.0"));
    }

    #[test]
    fn tells_how_bucca_was_installed() {
        let npm = "/opt/homebrew/lib/node_modules/buccanet/node_modules/buccanet-darwin-arm64/bin/bucca";
        assert_eq!(install_of(Path::new(npm)), Install::NpmGlobal);
        let npx = "/Users/a/.npm/_npx/1a2b/node_modules/buccanet-linux-x64/bin/bucca";
        assert_eq!(install_of(Path::new(npx)), Install::Npx);
        assert_eq!(install_of(Path::new("/Users/a/.cargo/bin/bucca")), Install::Source);
    }
}
