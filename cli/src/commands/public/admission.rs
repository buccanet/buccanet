//! Who the admission service lets into its public room. Pure: the service reads the request,
//! the blocklist and the joins of the last minute, and acts on the verdict.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Words a member's name may not carry, so nobody passes for the room's operator.
const RESERVED_WORDS: [&str; 8] = ["buccanet", "host", "admin", "administrator", "moderator", "operator", "official", "staff"];

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Admit,
    /// Turned down for good, with the reason the service logs.
    Refuse(&'static str),
    /// Left waiting until the join pace allows it.
    Later,
}

/// Whether a name copies the operator's: one of its words is reserved, or it spells Buccanet
/// with anything between the letters.
pub fn reserved(name: &str) -> bool {
    let lowered = name.to_lowercase();
    let words = lowered.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty());
    let compact: String = lowered.chars().filter(|c| c.is_alphanumeric()).collect();
    words.into_iter().any(|word| RESERVED_WORDS.contains(&word)) || compact.contains("buccanet")
}

/// The decision on one join request.
pub fn decide(device_id: &str, name: &str, blocklist: &[String], pace: &JoinPace) -> Verdict {
    if blocklist.iter().any(|blocked| blocked == device_id) {
        return Verdict::Refuse("blocklisted device");
    }
    if reserved(name) {
        return Verdict::Refuse("reserved name");
    }
    if !pace.has_room() {
        return Verdict::Later;
    }
    Verdict::Admit
}

/// The joins admitted in the last minute, against a cap.
pub struct JoinPace {
    per_minute: usize,
    admitted: VecDeque<Instant>,
}

const PACE_WINDOW: Duration = Duration::from_secs(60);

impl JoinPace {
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute: per_minute as usize, admitted: VecDeque::new() }
    }

    fn forget_old(&mut self, now: Instant) {
        while self.admitted.front().is_some_and(|at| now.duration_since(*at) >= PACE_WINDOW) {
            self.admitted.pop_front();
        }
    }

    pub fn has_room(&self) -> bool {
        let now = Instant::now();
        self.admitted.iter().filter(|at| now.duration_since(**at) < PACE_WINDOW).count() < self.per_minute
    }

    pub fn record(&mut self) {
        let now = Instant::now();
        self.forget_old(now);
        self.admitted.push_back(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_pass_for_the_operator_are_reserved() {
        assert!(reserved("Buccanet") && reserved("B.u.c.c.a.n.e.t bot") && reserved("Room Admin") && reserved("the host"));
        assert!(!reserved("Petrel") && !reserved("Ghost writer") && !reserved("Hostess Ada"));
    }

    #[test]
    fn blocklist_names_and_pace_decide_in_that_order() {
        let mut pace = JoinPace::new(1);
        let blocked = vec!["bad".to_string()];
        assert_eq!(decide("bad", "Petrel", &blocked, &pace), Verdict::Refuse("blocklisted device"));
        assert_eq!(decide("ok", "Admin", &blocked, &pace), Verdict::Refuse("reserved name"));
        assert_eq!(decide("ok", "Petrel", &blocked, &pace), Verdict::Admit);
        pace.record();
        assert_eq!(decide("ok2", "Tern", &blocked, &pace), Verdict::Later);
    }
}
