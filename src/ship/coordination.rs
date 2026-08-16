pub use crate::messages::CoordinationPayload;
use crate::messages::{StationId, SystemId};
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

    if *target == crate::system_registry::helm_station_key() {
        return config
            .system(&crate::system_registry::helm_steering_system_id())
            .and_then(|system| system.station.clone());
    }

    if *target == crate::system_registry::tactical_station_key() {
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
            let control = if policies.is_empty() {
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
mod tests {
    use super::*;

    // ── Routing matrix ────────────────────────────────────────────────────

    #[test]
    fn target_ai_always_consumes_regardless_of_sender() {
        assert_eq!(
            route_coordination(ControlSource::Ai, ControlSource::Ai),
            DeliverAction::Consume
        );
        assert_eq!(
            route_coordination(ControlSource::Human, ControlSource::Ai),
            DeliverAction::Consume
        );
    }

    #[test]
    fn target_human_sender_human_suppresses() {
        assert_eq!(
            route_coordination(ControlSource::Human, ControlSource::Human),
            DeliverAction::Suppress
        );
    }

    #[test]
    fn station_for_target_resolves_console_keys_and_ownerless_targets() {
        let config = crate::ship::components::ShipConfigComponent::default().0;

        assert_eq!(
            station_for_target(&config, &crate::system_registry::helm_station_key()),
            config
                .system(&crate::system_registry::helm_steering_system_id())
                .and_then(|system| system.station.clone())
        );
        assert_eq!(
            station_for_target(&config, &crate::system_registry::tactical_station_key()),
            config.weapons_station()
        );
        assert_eq!(
            station_for_target(&config, &SystemId("not-a-real-target".into())),
            None
        );
    }

    #[test]
    fn target_human_sender_ai_shows_popup() {
        assert_eq!(
            route_coordination(ControlSource::Ai, ControlSource::Human),
            DeliverAction::Popup
        );
    }

    /// #673: a Channel-3 message targeting a damage-disabled human console
    /// must resolve via `policy_for` (which honours `offline_systems`), not
    /// the raw `source_for`. A damage-disabled console can neither operate
    /// AI nor accept human input, so delivery must be treated as Consume,
    /// never Popup — mirroring the resolution logic in
    /// `process_coordination_lag` (src/ship_plugin.rs).
    #[test]
    fn target_damage_disabled_human_console_is_not_a_popup_candidate() {
        let mut resolver = ControlSourceResolver::new();
        let helm = SystemId("helm".into());

        // Station rating says Human, but damage has taken the console offline.
        resolver.set(helm.clone(), ControlSource::Human);
        resolver.set_offline(helm.clone(), true);

        let target_policy = resolver.policy_for(&helm);
        assert!(!target_policy.operate_ai && !target_policy.accept_human_input);

        // The caller must treat this as Consume rather than routing through
        // route_coordination with the raw (stale) ControlSource::Human, which
        // would incorrectly produce a Popup.
        let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
            DeliverAction::Consume
        } else {
            route_coordination(ControlSource::Ai, resolver.source_for(&helm))
        };
        assert_eq!(action, DeliverAction::Consume);
    }

    // ── Ship-wide broadcast (issue #879) ──────────────────────────────────

    fn seat(station: &str, control: ControlSource, holder: Option<&str>) -> ShipSeat {
        ShipSeat {
            station: StationId(station.into()),
            control,
            holder: holder.map(|h| h.to_string()),
        }
    }

    fn human_policy() -> ControlTickPolicy {
        crate::ship::control_source::control_tick_policy(ControlSource::Human)
    }

    fn ai_policy() -> ControlTickPolicy {
        crate::ship::control_source::control_tick_policy(ControlSource::Ai)
    }

    fn offline_policy() -> ControlTickPolicy {
        crate::ship::control_source::control_tick_policy(ControlSource::Offline)
    }

    #[test]
    fn a_station_with_no_systems_is_an_offline_seat() {
        assert_eq!(seat_control_source(&[]), ControlSource::Offline);
    }

    #[test]
    fn a_fully_backfilled_station_is_an_ai_seat() {
        assert_eq!(
            seat_control_source(&[ai_policy(), ai_policy(), ai_policy()]),
            ControlSource::Ai
        );
    }

    /// One live human console on the station is enough: the question the router
    /// asks is "can a person read this".
    #[test]
    fn one_human_system_makes_the_whole_seat_human() {
        assert_eq!(
            seat_control_source(&[ai_policy(), human_policy(), offline_policy()]),
            ControlSource::Human
        );
    }

    #[test]
    fn a_damage_disabled_station_is_an_offline_seat() {
        assert_eq!(
            seat_control_source(&[offline_policy(), offline_policy()]),
            ControlSource::Offline
        );
    }

    /// AC: a backfilled seat's advisory reaches EVERY human seat on the source
    /// ship, and no AI or offline seat.
    #[test]
    fn a_backfilled_senders_advisory_reaches_every_human_seat_and_no_other() {
        let seats = vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("helm", ControlSource::Human, Some("hikaru")),
            seat("tactical", ControlSource::Ai, None),
            seat("shields", ControlSource::Ai, Some("stale-token")),
            seat("power", ControlSource::Offline, Some("scotty")),
        ];

        let recipients = broadcast_to_ship(ControlSource::Ai, &seats);

        let stations: Vec<&str> = recipients.iter().map(|s| s.station.0.as_str()).collect();
        assert_eq!(
            stations,
            vec!["captain", "helm"],
            "every human seat, in authored order — and neither the backfilled \
             Tactical/Shields nor the offline Power, whatever token they carry"
        );
        let tokens: Vec<&str> = recipients
            .iter()
            .map(|s| s.holder.as_deref().unwrap())
            .collect();
        assert_eq!(tokens, vec!["alice", "hikaru"]);
    }

    /// The seat semantics are about the CONTROL SOURCE, not about who happens
    /// to be logged in: a human-held station whose holder has dropped is still
    /// a human seat, it simply has nobody to deliver to.
    #[test]
    fn a_human_seat_with_no_connected_holder_receives_nothing() {
        let seats = vec![seat("helm", ControlSource::Human, None)];
        assert!(broadcast_to_ship(ControlSource::Ai, &seats).is_empty());
    }

    /// The broadcast is `route_coordination` applied per seat, so it inherits
    /// the human→human `Suppress`: two officers on the same bridge do not need
    /// popups about each other.
    #[test]
    fn a_human_senders_broadcast_is_suppressed_at_every_seat() {
        let seats = vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("helm", ControlSource::Human, Some("hikaru")),
        ];
        assert!(
            broadcast_to_ship(ControlSource::Human, &seats).is_empty(),
            "human sender + human seats is Suppress, as it has always been"
        );
    }

    #[test]
    fn a_ship_with_no_human_seats_broadcasts_to_nobody() {
        let seats = vec![
            seat("captain", ControlSource::Ai, None),
            seat("helm", ControlSource::Ai, None),
        ];
        assert!(broadcast_to_ship(ControlSource::Ai, &seats).is_empty());
    }

    // ── Human-seeking systems (issue #984) ─────────────────────────────────

    /// OWNER-FIRST IS LOAD-BEARING: an earlier-authored human seat must not
    /// steal the console from the system's own (later-authored) human owner.
    #[test]
    fn owner_human_wins_over_earlier_authored_human() {
        let seats = vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("comms", ControlSource::Human, Some("bob")),
        ];
        let owner = StationId("comms".into());

        let found = seek_human_host(Some(&owner), &seats).unwrap();

        assert_eq!(found.station, StationId("comms".into()));
        assert_eq!(found.holder.as_deref(), Some("bob"));
    }

    /// No owner (an ownerless/NPC-style seek, or a caller that has not
    /// resolved one) falls back to the first human in authored order.
    #[test]
    fn no_owner_falls_back_to_first_authored_human() {
        let seats = vec![
            seat("captain", ControlSource::Ai, None),
            seat("helm", ControlSource::Human, Some("hikaru")),
            seat("comms", ControlSource::Human, Some("bob")),
        ];

        let found = seek_human_host(None, &seats).unwrap();

        assert_eq!(found.station, StationId("helm".into()));
    }

    #[test]
    fn no_humans_anywhere_yields_none() {
        let seats = vec![
            seat("captain", ControlSource::Ai, None),
            seat("helm", ControlSource::Ai, None),
        ];
        let owner = StationId("comms".into());

        assert!(seek_human_host(Some(&owner), &seats).is_none());
        assert!(seek_human_host(None, &seats).is_none());
    }

    /// A human-held station with nobody browser-connected is not a seek
    /// candidate — same "control source vs. connected holder" distinction
    /// `broadcast_to_ship` already draws.
    #[test]
    fn disconnected_human_seat_is_skipped() {
        let seats = vec![
            seat("captain", ControlSource::Human, None),
            seat("helm", ControlSource::Human, Some("hikaru")),
        ];

        let found = seek_human_host(None, &seats).unwrap();

        assert_eq!(found.station, StationId("helm".into()));
    }

    /// The owner seat is backfilled (AI) — not human-and-connected — so the
    /// seek continues past it to the next human in authored order.
    #[test]
    fn owner_is_ai_falls_through_to_later_human() {
        let seats = vec![
            seat("comms", ControlSource::Ai, Some("stale-token")),
            seat("captain", ControlSource::Human, Some("alice")),
        ];
        let owner = StationId("comms".into());

        let found = seek_human_host(Some(&owner), &seats).unwrap();

        assert_eq!(found.station, StationId("captain".into()));
        assert_eq!(found.holder.as_deref(), Some("alice"));
    }

    // ── Authored seek order (issue #984) ──────────────────────────────────

    /// The destroyer's shape: a bridge where the system's owner (Tactical) is
    /// empty and three other seats are crewed. The DERIVED order would hand the
    /// system to the earliest authored seat; the AUTHORED order hands it to
    /// Engineering.
    fn destroyer_bridge() -> Vec<ShipSeat> {
        vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("helm", ControlSource::Human, Some("hikaru")),
            seat("tactical", ControlSource::Ai, None),
            seat("engineering", ControlSource::Human, Some("scotty")),
        ]
    }

    fn order(ids: &[&str]) -> Vec<StationId> {
        ids.iter().map(|s| StationId((*s).into())).collect()
    }

    #[test]
    fn an_authored_order_is_walked_instead_of_the_derived_one() {
        let seats = destroyer_bridge();
        let owner = StationId("tactical".into());

        let derived = seek_human_host(Some(&owner), &seats).unwrap();
        assert_eq!(
            derived.station,
            StationId("captain".into()),
            "the derived walk takes the earliest authored human"
        );

        let authored = seek_human_host_in(
            Some(&owner),
            &order(&["tactical", "engineering", "captain", "helm"]),
            &seats,
        )
        .unwrap();
        assert_eq!(
            authored.station,
            StationId("engineering".into()),
            "John's ruling: Engineering is promoted ahead of the Captain"
        );
    }

    /// The authored order starts at the owner, so a crewed owner still wins —
    /// the promotion only decides who gets it when Tactical is empty.
    #[test]
    fn an_authored_order_still_prefers_a_crewed_owner() {
        let mut seats = destroyer_bridge();
        seats[2] = seat("tactical", ControlSource::Human, Some("chang"));

        let found = seek_human_host_in(
            Some(&StationId("tactical".into())),
            &order(&["tactical", "engineering", "captain", "helm"]),
            &seats,
        )
        .unwrap();

        assert_eq!(found.station, StationId("tactical".into()));
    }

    /// The authored walk skips non-human seats exactly as the derived one does:
    /// an order chooses among humans, it never conjures one.
    #[test]
    fn an_authored_order_with_no_humans_yields_none() {
        let seats = vec![
            seat("captain", ControlSource::Ai, Some("alice")),
            seat("tactical", ControlSource::Offline, None),
            seat("engineering", ControlSource::Human, None),
        ];

        assert!(
            seek_human_host_in(
                Some(&StationId("tactical".into())),
                &order(&["tactical", "engineering", "captain"]),
                &seats,
            )
            .is_none(),
            "backfilled, offline and disconnected seats are all skipped"
        );
    }

    /// THE DEFAULT IS THE OLD PATH, not a re-implementation of it. Every seat
    /// arrangement a four-station hull can be in, against every owner it can
    /// name: an empty `seek_order` must choose the identical seat.
    #[test]
    fn an_empty_seek_order_chooses_exactly_what_the_derived_walk_chooses() {
        let stations = ["captain", "helm", "tactical", "engineering"];
        let controls = [
            (ControlSource::Human, Some("crew")),
            (ControlSource::Human, None),
            (ControlSource::Ai, Some("crew")),
            (ControlSource::Offline, None),
        ];

        // Base 4 over 4 seats: 256 bridges, each tried against all four owners
        // and against no owner at all.
        for combo in 0..256u32 {
            let seats: Vec<ShipSeat> = stations
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let (control, holder) = controls[((combo >> (2 * i)) & 0b11) as usize];
                    seat(id, control, holder)
                })
                .collect();

            let owners: Vec<Option<StationId>> = std::iter::once(None)
                .chain(stations.iter().map(|s| Some(StationId((*s).into()))))
                .collect();
            for owner in &owners {
                assert_eq!(
                    seek_human_host_in(owner.as_ref(), &[], &seats),
                    seek_human_host(owner.as_ref(), &seats),
                    "bridge {combo:#010b}, owner {owner:?}"
                );
            }
        }
    }

    /// A name no seat answers to is stepped over, not treated as the end of the
    /// walk. `ShipConfig::validate` makes this unreachable from parsed TOML;
    /// the property is here so the pure function has no cliff for a caller that
    /// assembles the two halves by hand.
    #[test]
    fn an_authored_order_steps_over_a_station_that_is_not_seated() {
        let seats = vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("tactical", ControlSource::Ai, None),
        ];

        let found = seek_human_host_in(
            Some(&StationId("tactical".into())),
            &order(&["tactical", "science", "captain"]),
            &seats,
        )
        .unwrap();

        assert_eq!(found.station, StationId("captain".into()));
    }

    // ── seeking_seats (issue #984) ────────────────────────────────────────

    /// A hull shaped like the destroyer's Tactical seat: one station owning a
    /// mixed roster, plus a bare station whose ONLY system is human-seeking
    /// (the battleship's `comms`/`navigation` shape).
    fn seeking_config() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."

[[station]]
id = "tactical"
name = "Tactical"
description = "Guns."
rank = "Ltn."

[[station]]
id = "comms"
name = "Comms"
description = "Chatter."
rank = "Ens."

[[system]]
id = "captain"
kind = "captain"
station = "captain"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"

[[system]]
id = "navigation"
kind = "navigation"
station = "tactical"
human_seeking = true

[[system]]
id = "comms"
kind = "comms"
station = "comms"
human_seeking = true
"#,
            &["captain", "tactical_radar", "navigation", "comms"],
        )
        .unwrap()
    }

    fn ratings(pairs: &[(&str, &str)]) -> HashMap<StationId, String> {
        pairs
            .iter()
            .map(|(s, r)| (StationId((*s).into()), (*r).to_string()))
            .collect()
    }

    /// The fixpoint guard: a seeking system's own `ControlSource` — which this
    /// very resolution WROTE last tick — must not be readable as evidence that
    /// its host station is crewed. Here `navigation` lives on Tactical and is
    /// `Human` because the seek put it there; Tactical's real roster is
    /// backfilled, so the seat must still read `Ai`.
    #[test]
    fn a_seeking_systems_own_source_is_not_evidence_about_its_host_seat() {
        let config = seeking_config();
        let mut resolver = ControlSourceResolver::new();
        resolver.set(SystemId("tactical-radar".into()), ControlSource::Ai);
        resolver.set(SystemId("navigation".into()), ControlSource::Human);
        resolver.set(SystemId("comms".into()), ControlSource::Human);

        let seats = seeking_seats(&config, &resolver, &ratings(&[]), |_| {
            Some("someone".to_string())
        });

        let tactical = seats
            .iter()
            .find(|s| s.station == StationId("tactical".into()))
            .unwrap();
        assert_eq!(
            tactical.control,
            ControlSource::Ai,
            "only `tactical-radar` counts toward the Tactical seat; folding the \
             seek's own write back in would latch the console to itself"
        );
    }

    /// A station whose entire roster is human-seeking has no rating evidence
    /// left to reduce, so the seat is decided by its ACTIVE RATING. This is the
    /// battleship's `comms` and `navigation` stations exactly.
    #[test]
    fn a_station_owning_only_seeking_systems_is_decided_by_its_rating() {
        let config = seeking_config();
        let resolver = ControlSourceResolver::new();
        let comms = StationId("comms".into());

        let manned = seeking_seats(
            &config,
            &resolver,
            &ratings(&[("comms", "Standard")]),
            |_| Some("bob".to_string()),
        );
        assert_eq!(
            manned.iter().find(|s| s.station == comms).unwrap().control,
            ControlSource::Human,
            "a rated, connected Comms officer must be able to host their own console"
        );

        let backfilled = seeking_seats(
            &config,
            &resolver,
            &ratings(&[("comms", crate::ship::rating::BACKFILL_RATING)]),
            |_| Some("bob".to_string()),
        );
        assert_eq!(
            backfilled
                .iter()
                .find(|s| s.station == comms)
                .unwrap()
                .control,
            ControlSource::Ai,
            "an officer who asked the AI to run their station does not get the \
             console pushed back at them"
        );

        let unrated = seeking_seats(&config, &resolver, &ratings(&[]), |_| {
            Some("bob".to_string())
        });
        assert_eq!(
            unrated.iter().find(|s| s.station == comms).unwrap().control,
            ControlSource::Human,
            "an explicit Backfill is the ONLY thing that reads as automation — a \
             station with no rating recorded and a person sitting at it is a human seat"
        );

        let empty_seat = seeking_seats(&config, &resolver, &ratings(&[]), |_| None);
        assert_eq!(
            empty_seat
                .iter()
                .find(|s| s.station == comms)
                .unwrap()
                .control,
            ControlSource::Offline,
            "…and with nobody connected it is nobody's seat at all"
        );
    }

    /// Seats come back in AUTHORED station order — the property `seek_human_host`
    /// and two lockstep peers both depend on.
    #[test]
    fn seeking_seats_are_returned_in_authored_station_order() {
        let config = seeking_config();
        let resolver = ControlSourceResolver::new();
        let seats = seeking_seats(&config, &resolver, &ratings(&[]), |_| None);
        assert_eq!(
            seats
                .iter()
                .map(|s| s.station.0.as_str())
                .collect::<Vec<_>>(),
            vec!["captain", "tactical", "comms"],
        );
    }

    // ── Lag queue ─────────────────────────────────────────────────────────

    #[test]
    fn queue_starts_empty() {
        let queue = CoordinationLagQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn enqueue_adds_message() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
    }

    #[test]
    fn due_messages_returns_nothing_before_deadline() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        let due = queue.due_messages(5.0);
        assert!(due.is_empty());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn due_messages_returns_ready_at_deadline() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        let due = queue.due_messages(10.0);
        assert_eq!(due.len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn due_messages_returns_ready_after_deadline() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        let due = queue.due_messages(15.0);
        assert_eq!(due.len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn due_messages_respects_per_message_deadlines() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "first".into(),
            },
            sender_label: String::new(),
            due_time: 1.0,
        });
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "early".into(),
            },
            sender_label: String::new(),
            due_time: 5.0,
        });
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "late".into(),
            },
            sender_label: String::new(),
            due_time: 15.0,
        });

        let due = queue.due_messages(10.0);
        assert_eq!(due.len(), 2);
        assert_eq!(
            due[0].payload,
            CoordinationPayload::Advisory {
                message: "first".into()
            }
        );
        assert_eq!(
            due[1].payload,
            CoordinationPayload::Advisory {
                message: "early".into()
            }
        );
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.0[0].payload,
            CoordinationPayload::Advisory {
                message: "late".into()
            }
        );
    }

    #[test]
    fn sender_origin_is_captured_at_enqueue_time() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Alert {
                title: "AI Alert".into(),
                body: "Incoming threat detected.".into(),
            },
            sender_label: "AI Tactical".into(),
            due_time: 5.0,
        });
        let due = queue.due_messages(10.0);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].sender_origin, ControlSource::Ai);
        assert_eq!(due[0].sender_label, "AI Tactical");
    }

    #[test]
    fn target_is_resolved_live_at_delivery_time() {
        let mut queue = CoordinationLagQueue::new();
        let target = SystemId("red-alert".into());
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: target.clone(),
            payload: CoordinationPayload::Alert {
                title: "test".into(),
                body: "body".into(),
            },
            sender_label: String::new(),
            due_time: 1.0,
        });
        let due = queue.due_messages(2.0);
        assert_eq!(due[0].target, target);
    }
}
