//! The menu that opens above the input, as in Claude Code: on `/`, the commands whose name
//! starts with what was typed; after a command that takes a member, such as `/remove `, the
//! members of the open room whose name holds what follows; after `/approve ` or `/reject `, the
//! people waiting to join it.

use buccanet_sdk::{DeviceKind, MemberInfo, PendingJoin};

use buccanet_agent::text;

pub struct SlashCommand {
    pub name: &'static str,
    /// What follows the name, empty when the command takes nothing.
    pub args: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: [SlashCommand; 15] = [
    SlashCommand { name: "new", args: "<title>", about: "Create a room and copy its invite" },
    SlashCommand { name: "invite", args: "", about: "Copy an invite to the open room" },
    SlashCommand { name: "rename", args: "<title>", about: "Rename the open room" },
    SlashCommand { name: "rules", args: "", about: "Read or edit the rules of the open room" },
    SlashCommand { name: "instructions", args: "", about: "Edit what your agents follow in every room" },
    SlashCommand { name: "join", args: "<link>", about: "Ask to join a room from an invite link" },
    SlashCommand { name: "requests", args: "", about: "List who is waiting to join" },
    SlashCommand { name: "approve", args: "<name>", about: "Let someone waiting into the open room" },
    SlashCommand { name: "reject", args: "<name>", about: "Turn down someone waiting to join the open room" },
    SlashCommand { name: "remove", args: "<name>", about: "Remove a member from the open room" },
    SlashCommand { name: "admin", args: "<name>", about: "Make a member admin of the open room" },
    SlashCommand { name: "unadmin", args: "<name>", about: "Take the admin role back from a member" },
    SlashCommand { name: "leave", args: "", about: "Leave the open room" },
    SlashCommand { name: "help", args: "", about: "Show the keys" },
    SlashCommand { name: "quit", args: "", about: "Leave bucca" },
];

/// The commands whose argument is a member of the open room.
const MEMBER_COMMANDS: [&str; 3] = ["remove", "admin", "unadmin"];

/// The commands whose argument is someone waiting to join the open room.
const WAITING_COMMANDS: [&str; 2] = ["approve", "reject"];

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
    pub waiting: &'a [PendingJoin],
    pub me: &'a str,
}

/// The rows for what is typed; empty when the menu is closed.
pub fn matching(input: &str, room: &RoomContext) -> Vec<MenuItem> {
    let Some(typed) = input.strip_prefix('/') else { return mentions(input, room) };
    let argument = |command: &str| typed.strip_prefix(command).and_then(|rest| rest.strip_prefix(' '));
    if let Some((command, wanted)) = MEMBER_COMMANDS.iter().find_map(|command| Some((*command, argument(command)?))) {
        return members(command, wanted, room);
    }
    if let Some((command, wanted)) = WAITING_COMMANDS.iter().find_map(|command| Some((*command, argument(command)?))) {
        return waiting(command, wanted, room);
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
fn members(command: &str, wanted: &str, room: &RoomContext) -> Vec<MenuItem> {
    let wanted = wanted.trim().to_lowercase();
    room.members
        .iter()
        .filter(|member| member.device_id != room.me)
        .filter(|member| member.name.to_lowercase().contains(&wanted) || member.device_id.starts_with(&wanted))
        .map(|member| {
            let kind = if member.kind == DeviceKind::Agent { "agent" } else { "person" };
            MenuItem {
                label: text::line(&member.name),
                about: format!("{kind}{} · device {}", if member.admin { " · admin" } else { "" }, text::short(&member.device_id, 8)),
                fill: format!("/{command} {}", member.device_id),
                runs: true,
            }
        })
        .collect()
}

/// The people waiting to join whose name holds `wanted`, ignoring case, or whose request id starts with it.
fn waiting(command: &str, wanted: &str, room: &RoomContext) -> Vec<MenuItem> {
    let wanted = wanted.trim().to_lowercase();
    room.waiting
        .iter()
        .filter(|request| request.name.to_lowercase().contains(&wanted) || request.request_id.starts_with(&wanted))
        .map(|request| {
            let kind = if request.kind == DeviceKind::Agent { "agent" } else { "person" };
            MenuItem {
                label: text::line(&request.name),
                about: format!("{kind} · asks to join · device {}", text::short(&request.device_id, 8)),
                fill: format!("/{command} {}", request.request_id),
                runs: true,
            }
        })
        .collect()
}

/// While the last word typed starts with `@`, the other members whose name starts with what
/// follows it; picking one writes `@Name ` in its place.
fn mentions(input: &str, room: &RoomContext) -> Vec<MenuItem> {
    let start = input.rfind(char::is_whitespace).map_or(0, |index| index + 1);
    let Some(wanted) = input[start..].strip_prefix('@') else { return Vec::new() };
    let wanted = wanted.to_lowercase();
    room.members
        .iter()
        .filter(|member| member.device_id != room.me && member.name.to_lowercase().starts_with(&wanted))
        .map(|member| {
            let kind = if member.kind == DeviceKind::Agent { "agent" } else { "person" };
            MenuItem {
                label: format!("@{}", text::line(&member.name)),
                about: format!("{kind} · wakes only the members you name"),
                fill: format!("{}@{} ", &input[..start], member.name),
                runs: false,
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
        let room = RoomContext { members: &[], waiting: &[], me: "me" };
        let names = |input: &str| matching(input, &room).into_iter().map(|item| item.label).collect::<Vec<_>>();
        assert_eq!(names("/").len(), COMMANDS.len());
        assert_eq!(names("/re"), vec!["/rename <title>", "/requests", "/reject <name>", "/remove <name>"]);
        assert!(names("/new Plan").is_empty());
        assert!(names("hello").is_empty());
        assert_eq!((first_visible(2), first_visible(5)), (0, 2));
    }

    #[test]
    fn lists_other_members_by_any_part_of_their_name() {
        let members = [member("me", "Ilan"), member("a1", "Claude (tosafot-llm)"), member("b2", "Claude (Projects)")];
        let room = RoomContext { members: &members, waiting: &[], me: "me" };
        let fills = |input: &str| matching(input, &room).into_iter().map(|item| item.fill).collect::<Vec<_>>();
        assert_eq!(fills("/remove "), vec!["/remove a1", "/remove b2"]);
        assert_eq!(fills("/remove proj"), vec!["/remove b2"]);
        assert_eq!(fills("/remove a1"), vec!["/remove a1"]);
        assert!(fills("/remove ilan").is_empty());
        assert_eq!(fills("/admin tosa"), vec!["/admin a1"]);
        assert_eq!(fills("/unadmin "), vec!["/unadmin a1", "/unadmin b2"]);
    }

    #[test]
    fn lists_who_waits_to_join_by_name() {
        let request = |id: &str, name: &str| PendingJoin {
            room_id: "r".into(),
            request_id: id.into(),
            device_id: format!("device-{id}"),
            name: name.into(),
            kind: DeviceKind::Agent,
        };
        let waiting = [request("q1", "Claude (scratch)"), request("q2", "david-cluster")];
        let room = RoomContext { members: &[], waiting: &waiting, me: "me" };
        let fills = |input: &str| matching(input, &room).into_iter().map(|item| item.fill).collect::<Vec<_>>();
        assert_eq!(fills("/approve "), vec!["/approve q1", "/approve q2"]);
        assert_eq!(fills("/reject david"), vec!["/reject q2"]);
        assert_eq!(fills("/approve q1"), vec!["/approve q1"]);
        assert!(fills("/approve codex").is_empty());
    }

    #[test]
    fn completes_a_mention_in_place() {
        let members = [member("me", "Ilan"), member("x", "Codex"), member("c", "Claude (commons)")];
        let room = RoomContext { members: &members, waiting: &[], me: "me" };
        let fills = |input: &str| matching(input, &room).into_iter().map(|item| item.fill).collect::<Vec<_>>();
        assert_eq!(fills("salut @co"), vec!["salut @Codex "]);
        assert_eq!(fills("@c"), vec!["@Codex ", "@Claude (commons) "]);
        assert!(fills("salut @Codex ").is_empty());
        assert!(fills("@il").is_empty());
    }
}
