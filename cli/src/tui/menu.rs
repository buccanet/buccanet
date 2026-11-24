//! The menu that opens above the input, as in Claude Code: on `/`, the commands whose name
//! starts with what was typed; on `/remove `, the members of the open room whose name holds it.

use buccanet_sdk::{DeviceKind, MemberInfo};

use crate::text;

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

/// One row of the menu and what picking it does.
pub struct MenuItem {
    pub label: String,
    pub about: String,
    /// The input the row writes.
    pub fill: String,
    /// Whether Enter runs that input at once, or waits for the rest of it.
    pub runs: bool,
}

/// What the menu knows about the open room.
pub struct RoomContext<'a> {
    pub members: &'a [MemberInfo],
    pub me: &'a str,
}

/// The rows for what is typed; empty when the menu is closed.
pub fn matching(input: &str, room: &RoomContext) -> Vec<MenuItem> {
    let Some(typed) = input.strip_prefix('/') else { return Vec::new() };
    if let Some(wanted) = typed.strip_prefix("remove ") {
        return members(wanted, room);
    }
    if typed.contains(char::is_whitespace) {
        return Vec::new();
    }
    COMMANDS.iter().filter(|command| command.name.starts_with(typed)).map(command_item).collect()
}

fn command_item(command: &SlashCommand) -> MenuItem {
    let (label, fill) = match command.args {
        "" => (format!("/{}", command.name), format!("/{}", command.name)),
        args => (format!("/{} {args}", command.name), format!("/{} ", command.name)),
    };
    MenuItem { label, about: command.about.to_string(), fill, runs: command.args.is_empty() }
}

/// The other members whose name holds `wanted`, ignoring case, or whose device id starts with it.
fn members(wanted: &str, room: &RoomContext) -> Vec<MenuItem> {
    let wanted = wanted.trim().to_lowercase();
    room.members
        .iter()
        .filter(|member| member.device_id != room.me)
        .filter(|member| member.name.to_lowercase().contains(&wanted) || member.device_id.starts_with(&wanted))
        .map(|member| {
            let kind = if member.kind == DeviceKind::Agent { "agent" } else { "person" };
            MenuItem {
                label: text::line(&member.name),
                about: format!("{kind} · device {}", text::short(&member.device_id, 8)),
                fill: format!("/remove {}", member.device_id),
                runs: true,
            }
        })
        .collect()
}

/// The first row shown, so that `picked` stays among the visible ones.
pub fn first_visible(picked: usize) -> usize {
    picked.saturating_sub(VISIBLE - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, name: &str) -> MemberInfo {
        MemberInfo { device_id: id.into(), name: name.into(), kind: DeviceKind::Agent, admin: false }
    }

    #[test]
    fn matches_the_typed_start_of_a_command() {
        let room = RoomContext { members: &[], me: "me" };
        let names = |input: &str| matching(input, &room).into_iter().map(|item| item.label).collect::<Vec<_>>();
        assert_eq!(names("/").len(), COMMANDS.len());
        assert_eq!(names("/re"), vec!["/rename <title>", "/requests", "/reject", "/remove <name>"]);
        assert!(names("/new Plan").is_empty());
        assert!(names("hello").is_empty());
        assert_eq!((first_visible(2), first_visible(5)), (0, 2));
    }

    #[test]
    fn lists_other_members_by_any_part_of_their_name() {
        let members = [member("me", "Ilan"), member("a1", "Claude (tosafot-llm)"), member("b2", "Claude (Projects)")];
        let room = RoomContext { members: &members, me: "me" };
        let fills = |input: &str| matching(input, &room).into_iter().map(|item| item.fill).collect::<Vec<_>>();
        assert_eq!(fills("/remove "), vec!["/remove a1", "/remove b2"]);
        assert_eq!(fills("/remove proj"), vec!["/remove b2"]);
        assert_eq!(fills("/remove a1"), vec!["/remove a1"]);
        assert!(fills("/remove ilan").is_empty());
    }
}
