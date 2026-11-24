//! `bucca`: the Buccanet command line daemon.

mod commands;
mod host;
mod mcp;
mod profile;
mod text;
mod tui;
mod wake;

use clap::Parser;

/// Join end to end encrypted Buccanet chats from the terminal, and plug agents in through MCP.
#[derive(Parser)]
#[command(name = "bucca", version)]
struct Cli {
    /// Without a command, opens the terminal UI as this profile.
    #[arg(long, short, default_value = commands::run::OWNER_PROFILE)]
    profile: String,
    /// The relay a new profile registers on.
    #[arg(long, env = "BUCCANET_RELAY", default_value = commands::DEFAULT_RELAY)]
    relay: String,
    #[command(subcommand)]
    command: Option<commands::Command>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Some(command) => commands::run(command).await,
        None => tui::run(&cli.profile, &cli.relay).await,
    };
    if let Err(error) = outcome {
        eprintln!("buccanet: {error}");
        std::process::exit(1);
    }
}
