pub use crate::core::messages::CoordinationPayload;
use crate::core::messages::{StationId, SystemId};
use crate::ship::control_source::{ControlSource, ControlSourceResolver, ControlTickPolicy};
use std::collections::HashMap;

// ── Coordination sender-label ids (issue #975) ────────────────────────────────
//
// The sender label rides the wire as one of these string ids, never as English
// prose. `localiseTree` resolves it on the client, so the host viewscreen
// chatter and the phone popup render the same origin name from the same CSV row
// (`assets/strings/strings.csv`). An emitter stamps the id for the system it
// speaks for; nothing downstream composes the word. Rule 11: no player-visible
// English in Rust.
pub const CHATTER_SENDER_AI: &str = "chatter.sender.ai";
pub const CHATTER_SENDER_SENSORS: &str = "chatter.sender.sensors";
pub const CHATTER_SENDER_WEAPONS: &str = "chatter.sender.weapons";
pub const CHATTER_SENDER_NAVIGATION: &str = "chatter.sender.navigation";
pub const CHATTER_SENDER_POWER: &str = "chatter.sender.power";
pub const CHATTER_SENDER_SHIELDS: &str = "chatter.sender.shields";

/// Sender/target label id for a channel-3 message a ship's Core owns — the
/// systems that belong to no station. A message from or to an ownerless system
/// is addressed to "Core" rather than to a bare system name.
pub const CHATTER_ADDRESSEE_CORE: &str = "chatter.addressee.core";

/// Client-resolvable display id naming the STATION that owns `target`, or
/// [`CHATTER_ADDRESSEE_CORE`] when the target is ownerless.
///
/// Channel-3 (the AI-coordination bus) addresses crew by STATION, not by the
/// fine system underneath it: John's playtest note is that "Navigation → Helm
/// Navigation" should read "Helm" on both sides — the station alone, never the
/// station+system pair, and never the raw system. Both the sender identity
/// (resolved at enqueue from the emitting system) and the target label (resolved
/// at delivery from the routed system) run through here, so the viewscreen
/// chatter bubble and the phone popup name the same station from the same
/// [`station_for_target`] resolution the router already trusts.
///
/// The returned value is a `station.<id>.name` string id (or the Core id),
/// resolved to words by `localiseTree` on the client exactly as the
/// `chatter.sender.*` ids were — nothing downstream composes the word.
pub fn station_addressee_label(
    config: &crate::ship::config::ShipConfig,
    target: &SystemId,
) -> String {
    match station_for_target(config, target) {
        Some(station) => format!("station.{}.name", station.0),
        None => CHATTER_ADDRESSEE_CORE.to_string(),
    }
}

/// What to do with a delivered coordination message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliverAction {
    /// Target is AI-controlled — the AI consumes it silently.
    Consume,
    /// Target is human-controlled, sender was AI — show a popup.
    Popup,
    /// Both sender and target are human — suppress (they can coordinate IRL).
    Suppress,
}

/// Route a coordination message based on sender origin and target control source.
///
/// Delivery-time matrix:
///   target AI         → Consume (AI drains silently)
///   target Offline    → Consume (no one to receive it)
///   target Human + sender AI     → Popup
///   target Human + sender Human  → Suppress
pub fn route_coordination(
    sender_origin: ControlSource,
    target_control: ControlSource,
) -> DeliverAction {
    match (target_control, sender_origin) {
        (ControlSource::Ai, _) | (ControlSource::Offline, _) => DeliverAction::Consume,
        (ControlSource::Human, ControlSource::Ai) => DeliverAction::Popup,
        (ControlSource::Human, ControlSource::Human)
        | (ControlSource::Human, ControlSource::Offline) => DeliverAction::Suppress,
    }
}

/// Resolves a channel-3 target to the station that should receive a popup.
///
/// Coordination uses both fine system ids and console-level keys. Unlike
/// command admission, the latter are valid delivery addresses: Helm and
/// Tactical are not `[[system]]` ids, while legacy aggregate targets can name
/// an authored station directly. An unresolved target remains ownerless so the
/// existing fallback delivery policy can decide what to do with it.
pub fn station_for_target(
    config: &crate::ship::config::ShipConfig,
    target: &SystemId,
) -> Option<StationId> {
    if let Some(system) = config.system(target) {
        return system.station.clone();
    }

    if *target == crate::ship::system_registry::helm_station_key() {
        return config
            .system(&crate::ship::system_registry::helm_steering_system_id())
            .and_then(|system| system.station.clone());
    }

    if *target == crate::ship::system_registry::tactical_station_key() {
        return config.weapons_station();
    }

    let station = StationId(target.0.clone());
    config.station(&station).map(|_| station)
}

// ── Ship-wide broadcast (issue #879) ──────────────────────────────────────────

/// One crew seat on the source ship, as the ship-broadcast router sees it.
///
/// A *seat* is a station, not a system: a coordination popup lands on a
/// console, and a console belongs to whoever is holding the station. The
/// adapter reduces the station's fine systems to one [`ControlSource`] with
/// [`seat_control_source`] before building this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShipSeat {
    /// The station this seat is.
    pub station: StationId,
    /// Who is operating it, reduced from its fine systems.
    pub control: ControlSource,
    /// The session token of the connected holder, or `None` when nobody is
    /// browser-connected to it.
    pub holder: Option<String>,
}

/// Reduce a station's fine-system tick policies to the seat's control source.
///
/// A station owns several systems and a coordination popup is addressed to the
/// station, so the router needs one answer for the seat as a whole:
///
/// * any system still accepting human input → the seat is **Human**; someone is
///   sitting there and can read a popup.
/// * otherwise, any system operating AI → the seat is **Ai**; it is backfilled,
///   and there is nobody to show anything to.
/// * otherwise → **Offline**: damage-disabled, explicitly offline, or a station
///   with no systems at all.
///
/// The precedence is human-first for the same reason `process_coordination_lag`
/// treats a damage-disabled console as `Consume`: the question the router is
/// asking is "can a person read this", and one live console on the station is
/// enough for the answer to be yes.
pub fn seat_control_source(policies: &[ControlTickPolicy]) -> ControlSource {
    if policies.iter().any(|p| p.accept_human_input) {
        ControlSource::Human
    } else if policies.iter().any(|p| p.operate_ai) {
        ControlSource::Ai
    } else {
        ControlSource::Offline
    }
}

/// Fan one ship-wide advisory out to every seat on the SOURCE ship that the
/// existing delivery matrix resolves to a popup.
///
/// This is [`route_coordination`] applied per seat rather than to one addressed
/// target — the extension issue #879 needed and the reason it is not a second,
/// parallel rule. A backfilled seat's advisory therefore reaches every human
/// seat on the ship (target Human + sender Ai → `Popup`) and no AI or offline
/// seat (either → `Consume`), and an advisory whose sender is itself human is
/// suppressed at every seat, exactly as a human-to-human channel-3 message
/// already is: two people on the same bridge talk to each other.
///
/// Seats with no connected holder are dropped last, not first, so the
/// `Consume`/`Suppress` reasoning above is about the seat's control source and
/// not about whether anyone happens to be logged in.
///
/// Returns the recipients in the order `seats` was given, which the adapter
/// derives from the authored station list — a deterministic order, not a hash
/// order, so two lockstep peers emit the same popups in the same sequence.
pub fn broadcast_to_ship(sender_origin: ControlSource, seats: &[ShipSeat]) -> Vec<&ShipSeat> {
    seats
        .iter()
        .filter(|seat| route_coordination(sender_origin, seat.control) == DeliverAction::Popup)
        .filter(|seat| seat.holder.is_some())
        .collect()
}

// ── Human-seeking systems (issue #984) ────────────────────────────────────────

/// Find the human-held seat that should host a human-seeking system (pasm
/// decision `console-complexity-human-seeking-systems`).
///
/// Comms and navigation "always try to be under human control": the seek
/// walks the ship's authored station order — the system's own authored
/// `station` (`owner`) FIRST, then the remaining entries of `seats` in
/// AUTHORED order (the same deterministic ordering [`broadcast_to_ship`]
/// depends on, built by the adapter from `ShipConfig.stations`) — and returns
/// the first seat that is both [`ControlSource::Human`] and has a connected
/// holder. `None` means no human anywhere in the order; the caller falls back
/// to AI control exactly as today: "the seek order only ever chooses among
/// human-held stations: the mechanism prefers any human over the AI, it never
/// forces a human."
///
/// OWNER-FIRST IS LOAD-BEARING: comms is the LAST authored station on the
/// cruiser and battleship hulls. A naive first-human-in-authored-order scan
/// would steal the Comms console away from the Comms officer the instant
/// anyone else on the bridge is seated. Trying `owner` before the rest of
/// `seats` keeps the hull's own officer at their own console.
///
/// A hull may override that derived order with an authored one
/// (`SystemInstanceConfig::seek_order`); see [`seek_human_host_in`], which this
/// function is the no-authored-order case of. The derived order is still what
/// every hull that authors nothing gets, and the destroyer's authored order
/// starts at the same owner for the same reason.
///
/// `owner` is the seeking system's own authored station (`SystemInstanceConfig::station`,
/// resolved before calling — this function does no lookup of its own).
/// `seats` MUST be built with the seeking system's own fine-system policy
/// EXCLUDED from every seat's [`seat_control_source`] reduction: folding the
/// seek's own not-yet-decided control source back into its input is a
/// fixpoint over this function's own output, not an independent seat.
pub fn seek_human_host<'a>(
    owner: Option<&StationId>,
    seats: &'a [ShipSeat],
) -> Option<&'a ShipSeat> {
    seek_human_host_in(owner, &[], seats)
}

fn is_human_and_connected(seat: &ShipSeat) -> bool {
    seat.control == ControlSource::Human && seat.holder.is_some()
}

/// [`seek_human_host`] with an optional AUTHORED walk
/// (`SystemInstanceConfig::seek_order`) in place of the derived one.
///
/// An EMPTY `order` is the default and delegates to the derived walk above,
/// unchanged — not "equivalent to", the same code. That is deliberate: the
/// promise the field makes is that a hull authoring no `seek_order` picks the
/// same seat it picked before the field existed, and the cheapest way to keep a
/// promise like that is to leave one path rather than to maintain two that
/// agree.
///
/// A NON-EMPTY `order` is walked literally, first entry to last, and nothing
/// else is consulted — including `owner`, which
/// [`crate::ship::config::validate`] has already pinned to the head of the
/// list. The seek is still only ever a choice AMONG humans: an authored order
/// changes which human is preferred, never whether a human is required.
///
/// Names in `order` that no seat answers to are skipped rather than treated as
/// misses. Validation makes that unreachable from a parsed hull; the skip
/// exists so this pure function has no panic and no silent early stop if a
/// caller ever assembles the two halves by hand.
pub fn seek_human_host_in<'a>(
    owner: Option<&StationId>,
    order: &[StationId],
    seats: &'a [ShipSeat],
) -> Option<&'a ShipSeat> {
    if !order.is_empty() {
        return order
            .iter()
            .filter_map(|id| seats.iter().find(|seat| &seat.station == id))
            .find(|seat| is_human_and_connected(seat));
    }

    if let Some(owner_id) = owner {
        if let Some(owner_seat) = seats.iter().find(|seat| &seat.station == owner_id) {
            if is_human_and_connected(owner_seat) {
                return Some(owner_seat);
            }
        }
    }

    seats
        .iter()
        .filter(|seat| owner != Some(&seat.station))
        .find(|seat| is_human_and_connected(seat))
}

/// One complete human-seeking Station's resolved placement (issue #1097).
/// `host = None` is the ordinary Backfill/AI outcome, not a special controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitingStationAssignment {
    pub station: StationId,
    pub host: Option<StationId>,
    pub rating: String,
}

/// Resolve a complete human-seeking Station without Bevy state.
///
/// The station's own active direct holder always wins. Otherwise only its
/// finite authored compatibility list is walked, and a candidate is eligible
/// only when `is_directly_held` says a player actively holds that Station.
/// This admits a human-seeking Station in its direct state without ever using
/// a visiting Station as another visitor's host. `scenario_detailed_systems`
/// is the hull-resolved scenario-floor input: the visiting rating is raised to
/// the first authored rung at or above its baseline that leaves all those
/// owned systems human-operated.
pub fn resolve_visiting_station(
    config: &crate::ship::config::ShipConfig,
    station: &crate::ship::config::StationConfig,
    is_directly_held: impl Fn(&StationId) -> bool,
    scenario_detailed_systems: &std::collections::BTreeSet<SystemId>,
) -> VisitingStationAssignment {
    debug_assert!(station.human_seeking);
    let direct = is_directly_held(&station.id).then(|| station.id.clone());
    let host = direct.or_else(|| {
        station.host_order.iter().find_map(|host| {
            config.station(host)?;
            is_directly_held(host).then(|| host.clone())
        })
    });

    let (host, rating) = if host.is_none() {
        (None, crate::ship::rating::BACKFILL_RATING.to_string())
    } else if host.as_ref() == Some(&station.id) {
        // The adapter replaces this with the active direct rating. Keeping the
        // pure answer authored makes it useful without an ActiveStationRatings
        // resource and gives hand-built fixtures a deterministic result.
        let rating = station
            .ratings
            .first()
            .map(|rating| rating.name.clone())
            .unwrap_or_default();
        (host, rating)
    } else {
        match effective_visiting_rating(config, station, scenario_detailed_systems) {
            Some(rating) => (host, rating.to_string()),
            None => (None, crate::ship::rating::BACKFILL_RATING.to_string()),
        }
    };

    VisitingStationAssignment {
        station: station.id.clone(),
        host,
        rating,
    }
}

/// Resolve world-authored scenario detail-floor vocabulary onto one hull.
///
/// A scenario cannot name per-hull System ids because the crew chooses its hull
/// in the lobby. Each selector therefore names either a console family (the
/// authored Station id) or a System kind. Matching both namespaces and taking
/// their union preserves that hull independence; the result is the concrete,
/// deterministic System-id set consumed by the rating resolver.
pub fn resolve_scenario_detail_floor(
    config: &crate::ship::config::ShipConfig,
    selectors: &[String],
) -> std::collections::BTreeSet<SystemId> {
    config
        .systems
        .iter()
        .filter(|system| {
            selectors.iter().any(|selector| {
                system.kind == *selector
                    || system
                        .station
                        .as_ref()
                        .is_some_and(|station| station.0 == *selector)
            })
        })
        .map(|system| system.id.clone())
        .collect()
}

fn effective_visiting_rating<'a>(
    config: &crate::ship::config::ShipConfig,
    station: &'a crate::ship::config::StationConfig,
    scenario_detailed_systems: &std::collections::BTreeSet<SystemId>,
) -> Option<&'a str> {
    let baseline = station.visiting_rating.as_ref().and_then(|name| {
        station
            .ratings
            .iter()
            .position(|rating| &rating.name == name)
    })?;
    let owned_floor: std::collections::BTreeSet<&SystemId> = config
        .systems
        .iter()
        .filter(|system| system.station.as_ref() == Some(&station.id))
        .map(|system| &system.id)
        .filter(|id| scenario_detailed_systems.contains(*id))
        .collect();
    // Ratings are authored most-detailed first. Start at the visiting baseline
    // and walk TOWARD earlier, more-detailed rungs, choosing the smallest raise
    // that satisfies every scenario-required System.
    station.ratings[..=baseline]
        .iter()
        .rev()
        .find(|rating| {
            !rating
                .automated_systems
                .iter()
                .any(|system| owned_floor.contains(system))
        })
        .map(|rating| rating.name.as_str())
}

/// Build the seat list every [`seek_human_host`] call on one ship runs over
/// (issue #984).
///
/// Same shape as the channel-3 broadcast adapter's `ship_seats` — one entry per
/// authored station, in AUTHORED order, each station's fine systems reduced by
/// [`seat_control_source`] and paired with its connected holder — with two
/// deliberate differences, both load-bearing.
///
/// **1. Every `human_seeking` system is dropped from every reduction**, not
/// just the one being resolved. `seek_human_host` requires at least the seeking
/// system's own exclusion: it writes that system's `ControlSource`, so folding
/// last tick's answer back into this tick's input is a fixpoint over the seek's
/// own output. Concretely, the destroyer homes `comms` on `tactical`; a seek
/// that landed `comms` on the Captain would write `comms = Human`, and next
/// tick the *Tactical* seat would read that write as evidence of a human and
/// take the console back — a latch driven by nothing but the seek itself.
/// Excluding the whole class rather than the single system costs nothing extra
/// and buys order-independence: one seat list serves every seeking system on
/// the hull, so the answer cannot depend on which one is resolved first (the
/// cruiser homes BOTH `comms` and `navigation` on its `comms` station, so the
/// two would otherwise read each other's writes).
///
/// **2. A station left with no systems at all by that filter is decided by its
/// active rating and its holder**, not by the empty list's `Offline`. There is
/// no rating evidence left to reduce, so the seat takes the source
/// [`crate::ship::rating::apply_rating`] would have written for a hypothetical
/// non-seeking system on that station: an explicit
/// [`crate::ship::rating::BACKFILL_RATING`] is `Ai`; anything else — including
/// no recorded rating — is `Human` while the station has a connected holder and
/// `Offline` while it does not. This is not a corner case: the battleship's
/// `comms` and `navigation` stations own EXACTLY ONE system each and it is the
/// seeking one, so without this rule both would reduce to `Offline` and the
/// hull's own Comms and Navigation officers could never host their own consoles
/// — precisely the outcome `seek_human_host`'s owner-first order exists to
/// prevent. Only an explicit `Backfill` reads as automation, because that is the
/// only positive statement the ratings map ever makes about a seat; an officer
/// who asks the AI to run their station does not get the console pushed back at
/// them, and an officer whose station simply has no rating recorded is still a
/// person sitting at a console.
pub fn seeking_seats(
    config: &crate::ship::config::ShipConfig,
    resolver: &ControlSourceResolver,
    ratings: &HashMap<StationId, String>,
    holder_for: impl Fn(&StationId) -> Option<String>,
) -> Vec<ShipSeat> {
    config
        .stations
        .iter()
        .map(|station| {
            let policies: Vec<ControlTickPolicy> = config
                .systems
                .iter()
                .filter(|s| s.station.as_ref() == Some(&station.id))
                .filter(|s| !s.human_seeking)
                .map(|s| resolver.policy_for(&s.id))
                .collect();
            let holder = holder_for(&station.id);
            let control = if station.human_seeking {
                // A complete visiting Station may host a legacy seeking System
                // only while a connected player holds it DIRECTLY. `holder_for`
                // is keyed by Session::station, so a Station merely presented
                // as somebody else's visiting tab has no holder here and cannot
                // create a visitor-on-visitor chain. Backfill is likewise not a
                // human destination even if the direct owner remains connected.
                let automated = ratings
                    .get(&station.id)
                    .is_some_and(|name| name == crate::ship::rating::BACKFILL_RATING);
                match (automated, holder.is_some()) {
                    (false, true) => ControlSource::Human,
                    _ => ControlSource::Offline,
                }
            } else if policies.is_empty() {
                let automated = ratings
                    .get(&station.id)
                    .is_some_and(|name| name == crate::ship::rating::BACKFILL_RATING);
                match (automated, holder.is_some()) {
                    (true, _) => ControlSource::Ai,
                    (false, true) => ControlSource::Human,
                    (false, false) => ControlSource::Offline,
                }
            } else {
                seat_control_source(&policies)
            };
            ShipSeat {
                station: station.id.clone(),
                control,
                holder,
            }
        })
        .collect()
}

/// A coordination message queued for lagged delivery.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedCoordination {
    /// Sender control origin captured at enqueue time.
    pub sender_origin: ControlSource,
    /// Target system instance — resolved to live control source at delivery time.
    pub target: SystemId,
    /// Typed coordination payload.
    pub payload: CoordinationPayload,
    /// Human-readable label for the sender (e.g. "AI Tactical", "Captain").
    pub sender_label: String,
    /// Simulation time (seconds) at which this message is due for delivery.
    pub due_time: f32,
}

/// Lag scheduler for channel-3 coordination messages.
///
/// Every coordination message is queued with `due_time = sent_at + lag_secs`.
/// At delivery time the target's live control source is resolved by the caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinationLagQueue(Vec<QueuedCoordination>);

impl CoordinationLagQueue {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Enqueue a coordination message with its due time already computed.
    pub fn enqueue(&mut self, msg: QueuedCoordination) {
        self.0.push(msg);
    }

    /// Drain all messages whose due_time has passed, returning them in
    /// enqueue order. `now` is the current simulation time in seconds.
    pub fn due_messages(&mut self, now: f32) -> Vec<QueuedCoordination> {
        let mut due = Vec::new();
        self.0.retain(|msg| {
            if msg.due_time <= now {
                due.push(msg.clone());
                false
            } else {
                true
            }
        });
        due
    }
}

#[cfg(test)]
#[path = "coordination_tests.rs"]
mod tests;
