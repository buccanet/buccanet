//! The command menu that opens above the input on `/`, as in Claude Code: the commands whose
//! name starts with what was typed, the picked one highlighted.

pub struct SlashCommand {
    pub name: &'static str,
    /// What follows the name, empty when the command takes nothing.
    pub args: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: [SlashCommand; 11] = [
    SlashCommand { name: "new", args: "<title>", about: "Create a room and copy its invite" },
    SlashCommand { name: "invite", args: "", about: "Copy an invite to the open room" },
    SlashCommand { name: "rename", args: "<title>", about: "Rename the open room" },
    SlashCommand { name: "join", args: "<link>", about: "Ask to join a room from an invite link" },
    SlashCommand { name: "requests", args: "", about: "List who is waiting to join" },
    SlashCommand { name: "approve", args: "", about: "Let the first one waiting in" },
    SlashCommand { name: "reject", args: "", about: "Turn the first one waiting down" },
    SlashCommand { name: "remove", args: "<name>", about: "Remove a member from the open room" },
    SlashCommand { name: "leave", args: "", about: "Leave the open room" },
    SlashCommand { name: "help", args: "", about: "Show the keys" },
    SlashCommand { name: "quit", args: "", about: "Leave bucca" },
];

/// Rows the menu shows at once; it scrolls to keep the picked one in view.
pub const VISIBLE: usize = 4;

/// The commands matching an input that is a `/` followed by part of a name, nothing else.
pub fn matching(input: &str) -> Vec<&'static SlashCommand> {
    let Some(typed) = input.strip_prefix('/') else { return Vec::new() };
    if typed.contains(char::is_whitespace) {
        return Vec::new();
    }
    COMMANDS.iter().filter(|command| command.name.starts_with(typed)).collect()
}

/// The first row shown, so that `picked` stays among the visible ones.
pub fn first_visible(picked: usize) -> usize {
    picked.saturating_sub(VISIBLE - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_typed_start_of_a_name() {
        assert_eq!(matching("/").len(), COMMANDS.len());
        assert_eq!(matching("/re").iter().map(|command| command.name).collect::<Vec<_>>(), vec!["rename", "requests", "reject", "remove"]);
        assert!(matching("/new Plan").is_empty());
        assert!(matching("hello").is_empty());
        assert_eq!((first_visible(2), first_visible(5)), (0, 2));
    }
}
