//! Who the admission service lets into its public room and how long it keeps quiet members.
//! Pure: the service reads the request, the blocklist and the joins of the last minute, and acts
//! on the verdict.
//!
//! A name is read as readers see it (line breaks as spaces), in NFKC without its invisible
//! characters, word by word. Each word is compared on its confusable skeleton (Unicode UTS 39),
//! so `Ａdmin`, `Ad\u{200B}min` and `Adrnin` all read `admin`, and a word that mixes scripts,
//! such as `аdmin` with a Cyrillic `а`, is refused outright.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use buccanet_agent::text;
use buccanet_sdk::invisible::normalized;
use unicode_normalization::char::is_combining_mark;
use unicode_security::{MixedScript, skeleton};

/// Starts of words a member's name may not carry, so nobody passes for the room's operator or
/// its staff: `admins`, `moderators` and `officially` start with one.
const RESERVED_STEMS: [&str; 8] = ["buccanet", "admin", "moderat", "operator", "official", "staff", "support", "sysop"];
/// Words a member's name may not hold whole, alone or plural. As starts they would refuse
/// ordinary names such as Hostess or Model.
const RESERVED_WORDS: [&str; 2] = ["host", "mod"];

/// Above this share of its seats, in percent, the room is crowded.
const CROWDED_PERCENT: u64 = 90;
/// The longest a member may stay quiet in a crowded room.
pub const CROWDED_IDLE: Duration = Duration::from_secs(3600);

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Admit,
    /// Turned down for good, with the reason the service logs.
    Refuse(&'static str),
    /// Left waiting until the join pace allows it.
    Later,
}

/// The lowercase skeleton of a word without its combining marks: two words that look alike
/// share it.
fn skeleton_of(word: &str) -> String {
    skeleton(&word.to_lowercase()).filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase()
}

fn is_reserved(word: &str) -> bool {
    let plural = skeleton_of("s");
    let whole = |reserved: &&str| {
        let reserved = skeleton_of(reserved);
        word == reserved || word.strip_suffix(plural.as_str()) == Some(reserved.as_str())
    };
    RESERVED_STEMS.iter().any(|stem| word.starts_with(&skeleton_of(stem))) || RESERVED_WORDS.iter().any(whole)
}

/// What makes a name unfit for the room, if anything: a word that mixes scripts, a word that
/// passes for the operator's, or Buccanet spelled with anything between its letters.
pub fn name_problem(name: &str) -> Option<&'static str> {
    let folded = normalized(&text::line(name));
    let words: Vec<&str> = folded.split(|c: char| !c.is_alphanumeric() && !is_combining_mark(c)).filter(|word| !word.is_empty()).collect();
    if words.iter().any(|word| !word.is_single_script()) {
        return Some("mixed scripts");
    }
    let skeletons: Vec<String> = words.iter().map(|word| skeleton_of(word)).collect();
    let reserved = skeletons.iter().any(|word| is_reserved(word)) || skeletons.concat().contains(&skeleton_of("buccanet"));
    reserved.then_some("reserved name")
}

/// The decision on one join request.
pub fn decide(device_id: &str, name: &str, blocklist: &[String], pace: &JoinPace) -> Verdict {
    if blocklist.iter().any(|blocked| blocked == device_id) {
        return Verdict::Refuse("blocklisted device");
    }
    if let Some(problem) = name_problem(name) {
        return Verdict::Refuse(problem);
    }
    if !pace.has_room() {
        return Verdict::Later;
    }
    Verdict::Admit
}

/// How long a member may stay quiet: `idle`, or [`CROWDED_IDLE`] at most once the room holds
/// more than 90 percent of its seats, so quiet joins cannot keep it full for a day.
pub fn idle_limit(members: usize, max_members: u32, idle: Duration) -> Duration {
    let crowded = members as u64 * 100 > u64::from(max_members) * CROWDED_PERCENT;
    if crowded { idle.min(CROWDED_IDLE) } else { idle }
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

    fn reserved(name: &str) -> bool {
        name_problem(name).is_some()
    }

    #[test]
    fn names_that_pass_for_the_operator_are_reserved() {
        assert!(reserved("Buccanet") && reserved("B.u.c.c.a.n.e.t bot") && reserved("Room Admin") && reserved("the host"));
        for name in ["Admins", "Mods", "Moderators", "Staff", "Official bot", "Support", "Hosts", "Sysop"] {
            assert!(reserved(name), "{name} passes");
        }
        assert!(!reserved("Petrel") && !reserved("Ghost writer") && !reserved("Hostess Ada") && !reserved("Model T"));
        assert!(!reserved("Zoë") && !reserved("Влад") && !reserved("Ada 李明") && !reserved("Modi"));
    }

    #[test]
    fn disguised_names_are_reserved() {
        for name in ["Ad\u{200B}min", "Ad\u{200C}m\u{200D}in", "Ad\u{2060}min", "\u{FEFF}Admin", "Ad\u{00AD}min", "Ada\u{2028}Admin"] {
            assert_eq!(name_problem(name), Some("reserved name"), "{name:?} passes");
        }
        assert_eq!(name_problem("Ａｄｍｉｎ"), Some("reserved name"));
        assert_eq!(name_problem("а\u{0301}dmin"), Some("mixed scripts"));
        assert_eq!(name_problem("аdmin"), Some("mixed scripts"));
        assert_eq!(name_problem("Adrnin"), Some("reserved name"));
        assert_eq!(name_problem("A\u{0335}dmin"), Some("reserved name"));
    }

    #[test]
    fn a_crowded_room_keeps_quiet_members_one_hour() {
        let day = Duration::from_secs(86_400);
        assert_eq!(idle_limit(180, 200, day), day);
        assert_eq!(idle_limit(181, 200, day), CROWDED_IDLE);
        assert_eq!(idle_limit(181, 200, Duration::from_secs(60)), Duration::from_secs(60));
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
