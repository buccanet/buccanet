//! The agent's own secrets, by value: environment variables whose name says secret, the
//! `.env` files of the folder it works in, and the files its owner lists in
//! `~/.buccanet/secret-files`, one path per line.

use std::path::{Path, PathBuf};

use crate::profile;

/// Values shorter than this are too common to tell apart from ordinary text.
const MIN_LENGTH: usize = 8;

const SECRET_NAME_PARTS: [&str; 12] =
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "PWD", "CREDENTIAL", "AUTH", "PRIVATE", "DSN", "COOKIE", "DATABASE_URL"];

fn looks_secret(name: &str) -> bool {
    let name = name.to_uppercase();
    SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

/// The values of `KEY=VALUE` lines, quotes removed, comments skipped.
fn dotenv_values(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(_, value)| value.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .collect()
}

fn listed_files() -> Vec<PathBuf> {
    let Ok(list) = std::fs::read_to_string(profile::home().join("secret-files")) else { return Vec::new() };
    list.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| match line.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
            None => PathBuf::from(line),
        })
        .collect()
}

fn env_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name == ".env" || name.starts_with(".env.")))
        .collect()
}

/// Every secret value this process can see, longest first.
pub fn known() -> Vec<String> {
    let mut values: Vec<String> = std::env::vars().filter(|(name, _)| looks_secret(name)).map(|(_, value)| value).collect();
    let folder = std::env::current_dir().unwrap_or_default();
    for file in env_files(&folder).into_iter().chain(listed_files()) {
        if let Ok(text) = std::fs::read_to_string(&file) {
            values.extend(dotenv_values(&text));
        }
    }
    values.retain(|value| value.len() >= MIN_LENGTH);
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_env_files_and_secret_names() {
        assert_eq!(dotenv_values("# note\nA=\"one two three\"\nB='x'\n"), vec!["one two three", "x"]);
        assert!(looks_secret("openai_api_key") && !looks_secret("HOME"));
    }
}
