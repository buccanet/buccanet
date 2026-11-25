//! Turns the bundled gitleaks configuration into the JSON list of rules the crate embeds: the
//! id, pattern, keywords, entropy and secret group of each rule with a pattern. The TOML parser
//! stays in the build, so the WebAssembly build carries neither it nor the rules' prose.

use serde::{Deserialize, Serialize};

const SOURCE: &str = "assets/gitleaks.toml";

#[derive(Deserialize)]
struct Config {
    rules: Vec<SourceRule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRule {
    id: String,
    regex: Option<String>,
    entropy: Option<f64>,
    #[serde(default)]
    keywords: Vec<String>,
    secret_group: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Rule {
    id: String,
    regex: String,
    entropy: Option<f64>,
    keywords: Vec<String>,
    secret_group: Option<usize>,
}

fn main() {
    println!("cargo::rerun-if-changed={SOURCE}");
    let text = std::fs::read_to_string(SOURCE).expect("the gitleaks rules are bundled");
    let config: Config = toml::from_str(&text).expect("the bundled gitleaks rules parse");
    let rules: Vec<Rule> = config
        .rules
        .into_iter()
        .filter_map(|rule| {
            let regex = rule.regex?;
            Some(Rule { id: rule.id, regex, entropy: rule.entropy, keywords: rule.keywords, secret_group: rule.secret_group })
        })
        .collect();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("gitleaks-rules.json");
    std::fs::write(out, serde_json::to_string(&rules).expect("rules serialize")).expect("OUT_DIR is writable");
}
