//! Mentions: `@Name` in a text names a member of the room. The sender's device lists the
//! members it names, so connectors can wake only the agents a message is for.

use crate::events::MemberInfo;

/// The device ids of the members `text` names with `@` and their full name, ignoring case.
/// A longer name wins over a shorter one it starts with, and the sender never names itself.
pub fn mentioned(text: &str, members: &[MemberInfo], sender: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut by_length: Vec<&MemberInfo> = members.iter().filter(|member| !member.name.is_empty()).collect();
    by_length.sort_by_key(|member| std::cmp::Reverse(member.name.chars().count()));
    let mut taken: Vec<std::ops::Range<usize>> = Vec::new();
    let mut found = Vec::new();
    for member in by_length {
        let needle = format!("@{}", member.name.to_lowercase());
        for (start, _) in lowered.match_indices(&needle) {
            let end = start + needle.len();
            let whole = lowered[end..].chars().next().is_none_or(|next| !next.is_alphanumeric());
            if whole && !taken.iter().any(|span| span.start < end && start < span.end) {
                taken.push(start..end);
                if member.device_id != sender && !found.contains(&member.device_id) {
                    found.push(member.device_id.clone());
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use buccanet_protocol::api::DeviceKind;

    use super::*;

    fn member(id: &str, name: &str) -> MemberInfo {
        MemberInfo { device_id: id.into(), name: name.into(), kind: DeviceKind::Agent, admin: false }
    }

    #[test]
    fn finds_whole_names_and_prefers_the_longest() {
        let members = [member("c1", "Claude"), member("c2", "Claude (commons)"), member("x", "Codex"), member("me", "Ilan")];
        assert_eq!(mentioned("@codex tu nous reçois ?", &members, "me"), vec!["x"]);
        assert_eq!(mentioned("@Claude (commons) et @Claude", &members, "me"), vec!["c2", "c1"]);
        assert!(mentioned("@Codexy et mail@claudes.fr", &members, "me").is_empty());
        assert!(mentioned("moi @Ilan", &members, "me").is_empty());
    }
}
