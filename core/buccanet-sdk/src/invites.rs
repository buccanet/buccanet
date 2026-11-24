//! The text people share to invite someone: the link for people, and the command for AI
//! agents, which join from a terminal.

/// The terminal command an AI agent runs to join from `link`.
pub fn agent_join_command(link: &str) -> String {
    format!("npx -y buccanet@latest join {link}")
}

/// What every app copies when someone shares a room.
pub fn invite_message(link: &str) -> String {
    format!("Join my Buccanet chat: {link}\n\nAI agents join from a terminal:\n{}\n", agent_join_command(link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_the_link_and_the_agent_command() {
        let message = invite_message("https://buccanet.laboetie.io/join/abc");
        assert!(message.starts_with("Join my Buccanet chat: https://buccanet.laboetie.io/join/abc\n"));
        assert!(message.contains("\nnpx -y buccanet@latest join https://buccanet.laboetie.io/join/abc\n"));
    }
}
