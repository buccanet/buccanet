//! What a device believes about other devices, decided from the MLS tree and never from the
//! relay: a room has one admin, and each device keeps the signing key it was first seen with.

use std::collections::BTreeMap;

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

/// What a received commit claims: who made it and which devices it adds.
pub struct Proposed<'a> {
    pub committer: &'a str,
    pub added: &'a [Leaf],
}

/// A commit counts only when the room admin made it and each device it adds is new to the
/// group and presents its pinned key.
pub fn check_commit(commit: Proposed, admin_id: &str, current: &[Leaf], pins: &Pins) -> Result<(), String> {
    if commit.committer != admin_id {
        return Err(format!("{} is not the room admin", commit.committer));
    }
    if let Some(duplicate) = commit.added.iter().find(|leaf| current.iter().any(|member| member.device_id == leaf.device_id)) {
        return Err(format!("{} is already in the group", duplicate.device_id));
    }
    match key_change(pins, commit.added) {
        Some(changed) => Err(format!("{} presents a signing key that differs from the one first seen", changed.device_id)),
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
    fn only_the_admin_adds_new_devices_with_their_pinned_keys() {
        let mut pins = Pins::new();
        pin_new(&mut pins, &[leaf("admin", 1), leaf("bob", 2)]);
        let current = [leaf("admin", 1)];
        let check = |committer: &str, added: &[Leaf], current: &[Leaf]| {
            check_commit(Proposed { committer, added }, "admin", current, &pins).is_ok()
        };
        assert!(check("admin", &[leaf("bob", 2)], &current));
        assert!(!check("bob", &[leaf("carol", 3)], &current));
        assert!(!check("admin", &[leaf("bob", 7)], &current));
        assert!(!check("admin", &[leaf("admin", 1)], &current));
    }
}
