//! What a device believes about other devices, decided from the MLS tree and never from the
//! relay: only a room admin changes the group, and each device keeps the signing key it was
//! first seen with while it shares a room with this device.

use std::collections::{BTreeMap, BTreeSet};

use buccanet_protocol::b64;

use crate::mls::Leaf;

/// Device id to the base64 signing key it was first seen with.
pub type Pins = BTreeMap<String, String>;

pub fn pinned_key(pins: &Pins, device_id: &str) -> Option<Vec<u8>> {
    pins.get(device_id).and_then(|key| b64::decode(key).ok())
}

/// The first leaf whose key differs from the one pinned for its device.
pub fn key_change<'a>(pins: &Pins, leaves: &'a [Leaf]) -> Option<&'a Leaf> {
    leaves.iter().find(|leaf| pinned_key(pins, &leaf.device_id).is_some_and(|pinned| pinned != leaf.signature_key))
}

/// Pins the devices seen for the first time; keys already pinned stay.
pub fn pin_new(pins: &mut Pins, leaves: &[Leaf]) {
    for leaf in leaves {
        pins.entry(leaf.device_id.clone()).or_insert_with(|| b64::encode(&leaf.signature_key));
    }
}

/// Forgets the keys of devices that are in none of this device's rooms, so the pins grow with
/// the rooms and not with every device ever seen. `present` holds the devices still met.
pub fn prune(pins: &mut Pins, present: &BTreeSet<&str>) {
    pins.retain(|device_id, _| present.contains(device_id.as_str()));
}

/// What a received commit claims: who made it, which devices it adds and removes, and the
/// admin list it sets.
pub struct Proposed<'a> {
    pub committer: &'a str,
    pub added: &'a [Leaf],
    pub removed: &'a [String],
    pub admins: Option<&'a [String]>,
}

/// A commit counts only when a room admin made it, each device it adds is new to the group
/// and presents its pinned key, and the admin list it sets names members who stay.
pub fn check_commit(commit: Proposed, admins: &[String], current: &[Leaf], pins: &Pins) -> Result<(), String> {
    if !admins.iter().any(|admin| admin == commit.committer) {
        return Err(format!("{} is not a room admin", commit.committer));
    }
    if let Some(next) = commit.admins {
        check_admins(next, current, commit.removed)?;
    }
    if let Some(duplicate) = commit.added.iter().find(|leaf| current.iter().any(|member| member.device_id == leaf.device_id)) {
        return Err(format!("{} is already in the group", duplicate.device_id));
    }
    match key_change(pins, commit.added) {
        Some(changed) => Err(format!("{} presents a signing key that differs from the one first seen", changed.device_id)),
        None => Ok(()),
    }
}

fn check_admins(next: &[String], current: &[Leaf], removed: &[String]) -> Result<(), String> {
    if next.is_empty() {
        return Err("a room keeps at least one admin".into());
    }
    let stays = |device_id: &String| current.iter().any(|leaf| leaf.device_id == *device_id) && !removed.contains(device_id);
    match next.iter().find(|admin| !stays(admin)) {
        Some(outsider) => Err(format!("{outsider} cannot be an admin, it is not a member")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str, key: u8) -> Leaf {
        Leaf { device_id: id.into(), signature_key: vec![key; 32] }
    }

    #[test]
    fn first_key_wins() {
        let mut pins = Pins::new();
        pin_new(&mut pins, &[leaf("a", 1)]);
        pin_new(&mut pins, &[leaf("a", 2), leaf("b", 3)]);
        assert_eq!(pinned_key(&pins, "a"), Some(vec![1; 32]));
        assert_eq!(key_change(&pins, &[leaf("b", 3), leaf("a", 2)]).map(|leaf| leaf.device_id.as_str()), Some("a"));
        assert!(key_change(&pins, &[leaf("a", 1), leaf("c", 9)]).is_none());
    }

    #[test]
    fn devices_that_left_every_room_are_forgotten() {
        let mut pins = Pins::new();
        pin_new(&mut pins, &[leaf("me", 1), leaf("ada", 2), leaf("gone", 3)]);
        prune(&mut pins, &BTreeSet::from(["me", "ada"]));
        assert_eq!(pins.keys().map(String::as_str).collect::<Vec<_>>(), vec!["ada", "me"]);
    }

    #[test]
    fn only_an_admin_adds_new_devices_with_their_pinned_keys() {
        let mut pins = Pins::new();
        pin_new(&mut pins, &[leaf("admin", 1), leaf("bob", 2)]);
        let current = [leaf("admin", 1)];
        let admins = ["admin".to_string()];
        let check = |committer: &str, added: &[Leaf], current: &[Leaf]| {
            check_commit(Proposed { committer, added, removed: &[], admins: None }, &admins, current, &pins).is_ok()
        };
        assert!(check("admin", &[leaf("bob", 2)], &current));
        assert!(!check("bob", &[leaf("carol", 3)], &current));
        assert!(!check("admin", &[leaf("bob", 7)], &current));
        assert!(!check("admin", &[leaf("admin", 1)], &current));
    }

    #[test]
    fn an_admin_list_names_members_who_stay() {
        let pins = Pins::new();
        let current = [leaf("ada", 1), leaf("bob", 2), leaf("eve", 3)];
        let admins = ["ada".to_string(), "bob".to_string()];
        let check = |committer: &str, removed: &[String], next: &[&str]| {
            let next: Vec<String> = next.iter().map(|id| id.to_string()).collect();
            let proposed = Proposed { committer, added: &[], removed, admins: Some(&next) };
            check_commit(proposed, &admins, &current, &pins).is_ok()
        };
        assert!(check("bob", &[], &["ada", "bob", "eve"]));
        assert!(!check("eve", &[], &["eve"]));
        assert!(!check("ada", &[], &[]));
        assert!(!check("ada", &[], &["ada", "zed"]));
        assert!(!check("ada", &["bob".to_string()], &["ada", "bob"]));
        assert!(check("ada", &["bob".to_string()], &["ada"]));
    }
}
