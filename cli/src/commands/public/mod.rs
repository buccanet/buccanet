//! `bucca serve-public`: the admission service of a public room. It creates the room and its
//! listing on first run, admits each join request the listing brings (unless the device is on
//! the blocklist, its name passes for the operator's or mixes scripts, or the join pace is
//! reached), removes members who stay idle (one hour at most once the room is more than 90
//! percent full), and can move the listing to a fresh room on a schedule, leaving the old one.
//! After a failed open or move it waits 10 seconds, then twice as long after each failure, up to
//! 10 minutes. It sends no message.

mod admission;
mod ledger;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use buccanet_agent::text;
use buccanet_protocol::api::codes;
use buccanet_sdk::{Client, ClientEvent, DeviceKind, PendingJoin, RoomInfo, RoomPolicy, SdkError};

use super::CommandResult;
use super::setup::{NewProfile, create_profile};
use crate::host::open_client;
use crate::mcp::control;
use crate::profile::{self, Profile};
use admission::{JoinPace, Verdict, decide, idle_limit};
use ledger::{Ledger, blocklist, now_ms};

/// How often waiting requests, the blocklist, idle members and the reset are checked.
const TICK: Duration = Duration::from_secs(10);
/// The longest wait before trying again a room open or a reset that failed.
const BACKOFF_MAX: Duration = Duration::from_secs(600);

#[derive(clap::Args)]
pub struct PublicOptions {
    #[arg(long, short)]
    pub profile: String,
    /// The name of the profile when the service creates it on its first start.
    #[arg(long, default_value = "Buccanet door")]
    pub name: String,
    #[arg(long, env = "BUCCANET_RELAY", default_value = super::DEFAULT_RELAY)]
    pub relay: String,
    /// The listing's slug, the path segment people find the room by.
    #[arg(long)]
    pub slug: String,
    #[arg(long, default_value = "Buccanet public room")]
    pub title: String,
    #[arg(long, default_value = "People and their AI agents, in one end to end encrypted room anyone can join.")]
    pub description: String,
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u32).range(2..=1000))]
    pub max_members: u32,
    /// Joins admitted in one minute at most; the others wait for the next minute.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
    pub joins_per_minute: u32,
    /// Remove a member who joined this long ago and wrote nothing for as long: 30s, 90m, 24h, 7d.
    #[arg(long, default_value = "24h", value_parser = parse_duration)]
    pub idle: Duration,
    /// Move the listing to a fresh room this often (7d, for one); off when not given.
    #[arg(long, value_parser = parse_duration)]
    pub reset_every: Option<Duration>,
}

/// A duration written as a number and a unit: `30s`, `90m`, `24h`, `7d`.
pub fn parse_duration(value: &str) -> Result<Duration, String> {
    let split = value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: u64 = number.parse().map_err(|_| format!("{value} is not a duration such as 30s, 90m, 24h or 7d"))?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(format!("{value} needs a unit: s, m, h or d")),
    };
    Ok(Duration::from_secs(number * seconds))
}

fn rules(idle: Duration) -> String {
    let hours = idle.as_secs().div_ceil(3600);
    format!(
        "This room is public. Anyone can join it from https://app.buccanet.com/try and read what is written here. Messages are end to end encrypted to the room's members. Anyone can become a member.\n\n\
        Most members are AI agents, each run by a visitor who can also write as it.\n\n\
        1. Write nothing private: no key, password or personal detail.\n\
        2. Agents answer when a message names them with @ or by their name, in a few sentences.\n\
        3. Members who spam or abuse others are removed and blocked.\n\
        4. A member who joined more than {hours} hours ago and wrote nothing for {hours} hours is removed, after one hour once the room is more than 90 percent full. Joining again takes one click."
    )
}

/// The wait after a failed room open or reset: one tick, doubled after each failure up to
/// [`BACKOFF_MAX`], back to one tick after a success.
struct Backoff {
    delay: Duration,
    until: Option<Instant>,
}

impl Backoff {
    fn new() -> Self {
        Self { delay: TICK, until: None }
    }

    fn ready(&self) -> bool {
        self.until.is_none_or(|until| Instant::now() >= until)
    }

    /// Records a failure and returns how long to wait before the next attempt.
    fn failed(&mut self) -> Duration {
        let delay = self.delay;
        self.until = Some(Instant::now() + delay);
        self.delay = (delay * 2).min(BACKOFF_MAX);
        delay
    }

    fn succeeded(&mut self) {
        *self = Self::new();
    }
}

struct Door {
    client: Client,
    dir: PathBuf,
    options: PublicOptions,
    ledger: Ledger,
    pace: JoinPace,
    backoff: Backoff,
    room_id: String,
}

/// Creates the service's profile on its first start, as a human device, since it reads and
/// admits and sends no message.
async fn ensure_profile(options: &PublicOptions) -> CommandResult {
    if profile::exists(&options.profile) {
        return Ok(());
    }
    let spec = NewProfile {
        profile: &options.profile,
        display: &options.name,
        kind: DeviceKind::Human,
        relay: &options.relay,
        owner_profile: None,
    };
    let config = create_profile(spec).await?;
    println!("created the profile {} as device {}", options.profile, config.device_id);
    Ok(())
}

pub async fn serve(options: PublicOptions) -> CommandResult {
    ensure_profile(&options).await?;
    let profile = Profile::open(&options.profile)?;
    let (client, mut events) = open_client(&profile).await?;
    client.sync().await?;
    control::serve_profile(&profile, &client)?;
    let ledger = Ledger::load(&profile.dir)?;
    let pace = JoinPace::new(options.joins_per_minute);
    let mut door = Door { client, dir: profile.dir.clone(), options, ledger, pace, backoff: Backoff::new(), room_id: String::new() };
    door.open_until_served().await;
    let follower = door.client.clone();
    tokio::spawn(async move { follower.run().await });
    let mut tick = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(event) => door.on_event(event).await,
                None => return Ok(()),
            },
            _ = tick.tick() => door.on_tick().await,
        }
    }
}

impl Door {
    /// Opens the room, waiting longer after each failure, so a relay that refuses does not get a
    /// new attempt every few seconds.
    async fn open_until_served(&mut self) {
        while let Err(error) = self.open_room().await {
            let delay = self.backoff.failed();
            eprintln!("the room did not open: {error}; next attempt in {}s", delay.as_secs());
            tokio::time::sleep(delay).await;
        }
        self.backoff.succeeded();
    }

    /// Serves the room of the ledger while this device administers it, or creates one, gives it
    /// the door's rules and points the listing at it.
    async fn open_room(&mut self) -> Result<(), SdkError> {
        let room = match self.ledger_room().await {
            Some(room) => room,
            None => {
                let room_id = self.create_room().await?;
                self.client.room(&room_id).await?
            }
        };
        let rules = rules(self.options.idle);
        if room.rules.as_ref().map(|current| current.text.as_str()) != Some(rules.as_str()) {
            self.client.set_room_rules(&room.room_id, &rules).await?;
        }
        let listed = self.client.set_listing(&room.room_id, &self.options.slug, &self.options.title, &self.options.description).await?;
        self.room_id = room.room_id;
        self.adopt_members(&listed.members.iter().map(|member| member.device_id.clone()).collect::<Vec<_>>());
        println!("serving room {} under the listing {}", self.room_id, self.options.slug);
        Ok(())
    }

    /// The room of the ledger, while this device administers it.
    async fn ledger_room(&self) -> Option<RoomInfo> {
        let room_id = self.ledger.room_id.as_deref()?;
        self.client.room(room_id).await.ok().filter(|room| room.admin)
    }

    /// Creates a room and records it in the ledger at once, so a failure afterwards leaves a room
    /// the next attempt serves instead of one more room.
    async fn create_room(&mut self) -> Result<String, SdkError> {
        let policy = RoomPolicy { approval: true, max_members: self.options.max_members, ..RoomPolicy::default() };
        let room = self.client.create_room(Some(self.options.title.clone()), policy).await?;
        self.ledger.start_room(&room.room_id, now_ms());
        self.save();
        println!("created room {}", room.room_id);
        Ok(room.room_id)
    }

    fn adopt_members(&mut self, member_ids: &[String]) {
        self.ledger.adopt(member_ids, now_ms());
        self.save();
    }

    fn save(&self) {
        if let Err(error) = self.ledger.save(&self.dir) {
            eprintln!("the ledger was not saved: {error}");
        }
    }

    async fn on_event(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::JoinRequested { request } if request.room_id == self.room_id => self.decide(&request).await,
            ClientEvent::Message { message } if message.room_id == self.room_id && !message.own => {
                self.ledger.wrote(&message.sender_device_id, now_ms());
                self.save();
            }
            ClientEvent::RoomUpdated { room } if room.room_id == self.room_id => {
                self.adopt_members(&room.members.iter().map(|member| member.device_id.clone()).collect::<Vec<_>>());
            }
            ClientEvent::Error { message } => eprintln!("error: {}", text::line(&message)),
            _ => {}
        }
    }

    async fn decide(&mut self, request: &PendingJoin) {
        let name = text::line(&request.name);
        match decide(&request.device_id, &request.name, &blocklist(&self.dir), &self.pace) {
            Verdict::Admit => self.admit(request, &name).await,
            Verdict::Refuse(reason) => {
                let outcome = self.client.reject(&request.room_id, &request.request_id).await;
                println!("refused {name} ({reason}){}", failure(outcome));
            }
            Verdict::Later => {}
        }
    }

    async fn admit(&mut self, request: &PendingJoin, name: &str) {
        match self.client.approve(&request.room_id, &request.request_id).await {
            Ok(()) => {
                self.pace.record();
                println!("admitted {name}");
            }
            Err(error) if error.relay_code() == Some(codes::ROOM_FULL) => {
                let outcome = self.client.reject(&request.room_id, &request.request_id).await;
                println!("refused {name} (room full){}", failure(outcome));
            }
            Err(error) => eprintln!("could not admit {name}: {error}"),
        }
    }

    async fn on_tick(&mut self) {
        let unfinished_move = self.ledger.room_id.as_deref() != Some(self.room_id.as_str());
        if self.backoff.ready() && (self.reset_due() || unfinished_move) {
            self.reset().await;
        }
        match self.client.pending_requests(&self.room_id).await {
            Ok(waiting) => {
                for request in waiting {
                    self.decide(&request).await;
                }
            }
            Err(error) => eprintln!("could not read waiting requests: {error}"),
        }
        self.remove_unwanted().await;
    }

    fn reset_due(&self) -> bool {
        self.options.reset_every.is_some_and(|every| now_ms() - self.ledger.created_ms > every.as_millis() as i64)
    }

    /// Removes the members on the blocklist and those idle too long.
    async fn remove_unwanted(&mut self) {
        let me = self.client.device_id();
        let blocked = blocklist(&self.dir);
        let limit = idle_limit(self.ledger.members.len(), self.options.max_members, self.options.idle);
        let idle = self.ledger.idle(now_ms(), limit, std::slice::from_ref(&me));
        let blocked_members: Vec<String> = self.ledger.members.keys().filter(|id| blocked.contains(id) && **id != me).cloned().collect();
        for (device_id, reason) in blocked_members.iter().map(|id| (id, "blocklisted")).chain(idle.iter().map(|id| (id, "idle"))) {
            let name = self.member_name(device_id).await;
            match self.client.remove_member(&self.room_id, device_id).await {
                Ok(()) => println!("removed {name} ({reason}, device {device_id})"),
                Err(error) => eprintln!("could not remove {name} (device {device_id}): {error}"),
            }
        }
    }

    async fn member_name(&self, device_id: &str) -> String {
        let room = self.client.room(&self.room_id).await.ok();
        let member = room.and_then(|room| room.members.into_iter().find(|member| member.device_id == device_id));
        member.map_or_else(|| "a member".to_string(), |member| text::line(&member.name))
    }

    /// Moves the listing to a fresh room once the reset is due, or finishes a move that failed
    /// after its room was created, then leaves the old room, which keeps its members and gets no
    /// new ones.
    async fn reset(&mut self) {
        let old = self.room_id.clone();
        let moved = if self.reset_due() { self.create_room().await.map(|_| ()) } else { Ok(()) };
        match moved {
            Ok(()) => self.finish_move(&old).await,
            Err(error) => self.move_failed(error),
        }
    }

    async fn finish_move(&mut self, old: &str) {
        if let Err(error) = self.open_room().await {
            return self.move_failed(error);
        }
        self.backoff.succeeded();
        println!("moved the listing {} to room {}", self.options.slug, self.room_id);
        if old == self.room_id {
            return;
        }
        match self.client.forget_room(old).await {
            Ok(()) => println!("left room {old}"),
            Err(error) => eprintln!("could not leave room {old}: {error}"),
        }
    }

    fn move_failed(&mut self, error: SdkError) {
        let delay = self.backoff.failed();
        eprintln!("the reset failed: {error}; next attempt in {}s", delay.as_secs());
    }
}

/// What a log line adds when the relay refused the decision.
fn failure(outcome: Result<(), SdkError>) -> String {
    outcome.err().map_or_else(String::new, |error| format!(", but the relay refused: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_double_the_wait_up_to_ten_minutes() {
        let mut backoff = Backoff::new();
        let waits: Vec<u64> = (0..9).map(|_| backoff.failed().as_secs()).collect();
        assert_eq!(waits, vec![10, 20, 40, 80, 160, 320, 600, 600, 600]);
        assert!(!backoff.ready());
        backoff.succeeded();
        assert!(backoff.ready() && backoff.failed() == TICK);
    }

    #[test]
    fn durations_take_a_unit() {
        assert_eq!(parse_duration("90m"), Ok(Duration::from_secs(5400)));
        assert_eq!(parse_duration("7d"), Ok(Duration::from_secs(604_800)));
        assert!(parse_duration("12").is_err() && parse_duration("h").is_err() && parse_duration("3w").is_err());
    }
}
