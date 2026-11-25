//! Secrets recognized by their shape, with the rules of gitleaks (MIT, `assets/gitleaks.toml`,
//! release v8.30.1, turned into JSON by `build.rs`): each rule is a regular expression, the
//! keywords that must appear for it to be tried, and the randomness the matched secret must
//! reach. Patterns read `\w`, `\d`, `\s` and case in ASCII, as keys are written.

use std::sync::OnceLock;

use regex::bytes::{Regex, RegexBuilder};
use serde::Deserialize;

const GITLEAKS: &str = include_str!(concat!(env!("OUT_DIR"), "/gitleaks-rules.json"));

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRule {
    id: String,
    regex: String,
    entropy: Option<f64>,
    #[serde(default)]
    keywords: Vec<String>,
    secret_group: Option<usize>,
}

pub struct Rule {
    pub id: String,
    regex: Regex,
    entropy: Option<f64>,
    keywords: Vec<String>,
    secret_group: Option<usize>,
}

/// The rules that compile with Rust's regex engine, built once.
pub fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let raw: Vec<RawRule> = serde_json::from_str(GITLEAKS).expect("the bundled gitleaks rules parse");
        raw.into_iter().filter_map(compile).collect()
    })
}

fn compile(raw: RawRule) -> Option<Rule> {
    let regex = RegexBuilder::new(&raw.regex).unicode(false).build().ok()?;
    let keywords = raw.keywords.into_iter().map(|keyword| keyword.to_lowercase()).collect();
    Some(Rule { id: raw.id, regex, entropy: raw.entropy, keywords, secret_group: raw.secret_group })
}

/// Shannon entropy in bits per character: how random a string looks.
fn entropy(text: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in text.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let length = text.chars().count() as f64;
    counts.values().map(|&count| count as f64 / length).map(|share| -share * share.log2()).sum()
}

/// The first rule whose pattern matches a secret in `text`.
pub fn find(text: &str) -> Option<&'static str> {
    let lowered = text.to_lowercase();
    rules().iter().find(|rule| rule.matches(text, &lowered)).map(|rule| rule.id.as_str())
}

impl Rule {
    fn matches(&self, text: &str, lowered: &str) -> bool {
        if !self.keywords.is_empty() && !self.keywords.iter().any(|keyword| lowered.contains(keyword.as_str())) {
            return false;
        }
        self.regex.captures_iter(text.as_bytes()).any(|captures| {
            let group = self.secret_group.unwrap_or(if captures.len() > 1 { 1 } else { 0 });
            let secret = captures.get(group).or_else(|| captures.get(0)).map_or(&b""[..], |found| found.as_bytes());
            self.entropy.is_none_or(|threshold| entropy(&String::from_utf8_lossy(secret)) >= threshold)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearly_every_gitleaks_rule_compiles() {
        assert!(rules().len() >= 200, "only {} rules compiled", rules().len());
    }

    #[test]
    fn recognizes_common_keys_and_leaves_prose_alone() {
        assert_eq!(find("my token is ghp_8RtZq2Lk3VbN9wXyP4mC7dJ1sF6hG0aTuE5o"), Some("github-pat"));
        assert!(
            find("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQ\n-----END OPENSSH PRIVATE KEY-----")
                .is_some()
        );
        assert_eq!(find("The OCR pass is done, page 12 needs a second look."), None);
        assert_eq!(find("Set api_key in your env file before running the script."), None);
    }
}
