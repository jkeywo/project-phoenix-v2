//! Peer-local Game Master **Station workload** advisory (issue #1438, PRD
//! #1419 M4 stories 11/12/14, presentation contract PRD #1418).
//!
//! One advisory projection answers a question the attention queue deliberately
//! does not: *how much is being asked of the people at each Station right now*.
//! It is a count of **distinct outstanding demands that genuinely require a
//! human**, never a measure of how busy the console looks, how fast anyone is
//! typing, or how many notifications went past.
//!
//! # What a demand is
//!
//! A demand is a piece of live owner state that (a) some Station's human
//! operator has to act on, and (b) stops existing when they do. Every counted
//! demand comes from a **producer** in the documented inventory below, and
//! every producer declares five things:
//!
//! | field | meaning |
//! |---|---|
//! | source | which existing subsystem's state is read |
//! | stable key | the canonical identity used to deduplicate and to keep the row steady |
//! | owner | the ship, System and therefore Station it belongs to |
//! | human-action-needed | while what is true does a person have to act |
//! | terminal | what makes it stop counting |
//!
//! The complete inventory, with its prose, is
//! `docs/gm-station-workload-inventory.md`; the design contract is the
//! `gm-t3-station-workload` slice of `pasm/spec/design/gm-console-t3.yaml`.
//! The three shipped producers are:
//!
//! * **Pending Comms** ([`COMMS_SOURCE`]) — the same predicate the attention
//!   queue's pending-Comms producer uses, [`crate::gm_attention::pending`],
//!   applied to the same live inbox. Keyed by the `CommsMessage` id the world
//!   minted, owned by the ship's `comms` System and therefore by whichever
//!   Station currently holds it.
//! * **Typed Coordination decisions** ([`NAVIGATION_SOURCE`],
//!   [`REPAIR_SOURCE`]) — the two already-supported explicit human decisions
//!   the Coordination bus carries: a Navigation clearance a person has actually
//!   been asked to take up and has not yet flown, and a Repair dispatch request
//!   sitting in the ship's own repair queue. Both ask the ROUTER what happened
//!   to the delivery rather than assuming: a clearance between two people is
//!   suppressed, and a suppressed message asks nobody for anything.
//! * **Task activations** ([`TASK_SOURCE`]) — every live
//!   [`crate::core::task_lifecycle::TaskActivation`], read through
//!   [`TASK_DEMAND_INVENTORY`]. Every verb this build ships is documented
//!   there as **never counted**, because its work progresses and terminates on
//!   its own: a scan that a human started is not a demand on that human, it is
//!   a scan. The inventory is a table rather than an `if` so that the day a
//!   verb gains genuine "waiting on the operator" owner state, that fact is
//!   declared in one place instead of a fourth producer being invented.
//!
//! Nothing here parses text, counts notifications, or reads a popup. A
//! Coordination popup and the repair queue entry behind it are the same demand;
//! only the owner state is read, so the duplicate cannot arise. What
//! deduplication there is happens by canonical key, per Station, so two
//! independent demands still count twice and one demand reported twice counts
//! once.
//!
//! # Demands end, and two of them end by being DONE
//!
//! Every producer names a terminal, and for the two Coordination decisions that
//! terminal had to be built rather than merely described (issue #1438):
//!
//! * a Navigation clearance auto-completes when the hull ARRIVES, inside the
//!   waypoint's existing authored tolerance. A human Helm that has flown the
//!   course has answered it; there is deliberately no acknowledge button, which
//!   would be work invented in order to measure work. The COURSE is untouched —
//!   Navigation still owns setting and clearing waypoints.
//! * a Repair dispatch request ends when the damage it names is gone. That is
//!   now true on a human seat as well as a backfilled one, because the queue's
//!   prune moved out of `operate_repair_ai` into the seat-independent
//!   [`crate::console::repair::server::prune_repair_request_queue`].
//!
//! A demand with no terminal is not an advisory, it is a growing number.
//!
//! # What the levels mean
//!
//! Zero demands is **Underused**, one or two is **Engaged**, and three or more
//! *continuously for thirty simulation seconds* is **Overloaded**. While the
//! duration is still running the Station reads Engaged — the count alone is
//! never the answer, because a facilitator should not be told a seat is
//! drowning because three things landed in the same second. Falling below the
//! threshold ends the overload and resets its timer, so a Station has to earn
//! the label again.
//!
//! Both numbers are authored overrides on the existing `[gm_attention]` table
//! ([`crate::gm_attention::GmAttentionSettings`]), positive and validated at
//! world load, with a separate disable that silences this advisory and nothing
//! else. There is no zero sentinel: zero is not "off", it is a threshold that
//! fires before anything has happened, and guessing which one an author meant
//! is how a scenario ships with a rule nobody chose.
//!
//! # Backfill, and mixed Stations
//!
//! A Station nobody is sitting at is not underused, it is **Backfill**: it
//! reports that word instead of a count, because "0 demands" about an
//! AI-operated seat would be an observation about nobody. A Station whose fine
//! Systems are all damage-disabled or explicitly offline reports **Offline**
//! for the same reason — there is no one to ask and nothing to ask them with.
//!
//! A *mixed* Station — some Systems human, some backfilled — is a human seat,
//! and counts only the subset of demands whose own System is human-operated.
//! The AI half's work is the AI's.
//!
//! Which Systems a Station *owns* for all of that is the **authored**
//! membership — the `[[system]] station = …` blocks, the same set the lobby
//! roster pill and the Channel-3 router already reduce. A Station every one of
//! whose authored Systems is currently hosted at ANOTHER Station (human-seeking
//! migration) is **omitted from the summary entirely**: it is nobody's seat
//! this tick, and its demands are already counted at the host. See
//! [`seat_standing`] for why that is an omission rather than a fifth word.
//!
//! # What this is NOT
//!
//! Not a peer frame, not a digest fold, not a player performance rating, and
//! not a new authority. It publishes on the page-local `gm_workload` Host
//! Channel exactly as [`crate::gm_attention`] publishes `gm_attention`, and the
//! evidence a GM expands is the list of source demands that produced the count
//! — never a score, a grade or an opinion about the person.
//!
//! The one thing that travels in a snapshot is [`GmWorkloadWatch`], for
//! [`crate::gm_attention::GmIdleNpcWatch`]'s reason: elapsed overload is
//! history, and a restored world cannot be asked how long a seat has been
//! underwater. It is folded into no digest.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::comms::server::CommsInboxRes;
use crate::console_bridge::GmWorkloadChanged;
use crate::core::messages::{StationId, SystemId};
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::gm_attention::GmAttentionReason;
use crate::gm_projection::GmEntityReference;
use crate::ship::control_source::ControlSource;
use crate::world::config::WorldConfig;

/// Presentation bound on one published summary. A fleet cannot grow an
/// unbounded desk panel; a scenario with more Stations than this has a bigger
/// problem than the advisory.
pub const MAX_GM_WORKLOAD_STATIONS: usize = 64;

/// Presentation bound on the evidence one Station row expands to. The count is
/// still the true count — this bounds only how many source demands are named.
pub const MAX_GM_WORKLOAD_EVIDENCE: usize = 16;

/// Distinct outstanding demands at which a Station becomes a candidate for
/// Overloaded, before the duration is considered.
pub const DEFAULT_OVERLOAD_COUNT: u32 = 3;

/// How long the count must stay at or above the threshold, in SIMULATION
/// seconds, before Overloaded is the answer.
pub const DEFAULT_OVERLOAD_SECS: f32 = 30.0;

// ── The producer inventory ────────────────────────────────────────────────────

/// Source id of the pending-Comms producer.
pub const COMMS_SOURCE: &str = "pending_comms";
/// Source id of the Navigation-clearance producer.
pub const NAVIGATION_SOURCE: &str = "navigation_clearance";
/// Source id of the Repair-dispatch producer.
pub const REPAIR_SOURCE: &str = "repair_dispatch";
/// Source id of the task-activation producer.
pub const TASK_SOURCE: &str = "task_activation";

/// String Table id explaining one pending-Comms demand. Takes `{sender}`.
pub const COMMS_REASON: &str = "server.gm.workload.reason.pending_comms";
/// String Table id explaining one outstanding Navigation clearance. Takes
/// `{x}` and `{z}`.
pub const NAVIGATION_REASON: &str = "server.gm.workload.reason.navigation_clearance";
/// String Table id explaining one outstanding Repair dispatch request. Takes
/// `{station}` (the authored label of the damaged Station) and `{tier}` (itself
/// a String Table id — see [`tier_string_id`]).
pub const REPAIR_REASON: &str = "server.gm.workload.reason.repair_dispatch";

/// The String Table id naming one [`crate::ship::damage::DamageTier`] inside a
/// repair-demand sentence.
///
/// A tier is a domain value, not a word, so it travels as an id and the page
/// resolves it — the panel already resolves any reason parameter the String
/// Table knows. `format!("{tier:?}")` used to put `Disabled` — the Rust
/// identifier — straight into a sentence a facilitator reads, which is exactly
/// the untranslated raw-Debug leak the String Table exists to prevent.
///
/// Its own `server.gm.workload.tier.*` rows rather than the
/// `component.repair_teams.tier.*` chips: those are shouted abbreviations for a
/// damage-control readout (`DMG`, `DISABLED`) and read as noise inside a
/// sentence, and they have no `Operational` row at all.
pub fn tier_string_id(tier: crate::ship::damage::DamageTier) -> &'static str {
    use crate::ship::damage::DamageTier;
    match tier {
        DamageTier::Operational => "server.gm.workload.tier.operational",
        DamageTier::Damaged => "server.gm.workload.tier.damaged",
        DamageTier::Disabled => "server.gm.workload.tier.disabled",
        DamageTier::Destroyed => "server.gm.workload.tier.destroyed",
    }
}

/// String Table id explaining one counted task activation. Takes `{verb}`.
///
/// Unreached by this build, because [`TASK_DEMAND_INVENTORY`] counts no shipped
/// verb. It exists as ONE stable sentence rather than a per-verb id built by
/// interpolation, so the day a verb does expose a waiting-on-the-operator
/// state, the row explains itself with copy that is actually in the String
/// Table instead of rendering a key nobody wrote.
pub const TASK_REASON: &str = "server.gm.workload.reason.task_activation";

/// Whether one live task activation's verb is EVER a human demand, and why.
///
/// The table is exhaustive over the verbs this build can produce — a lib test
/// asserts that, so a new `TASK_VERB_*` cannot be added without a decision
/// being recorded here rather than defaulting silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskDemandRule {
    /// The lifecycle verb, or its prefix when the verb is per-instance (the
    /// Security teams are `security_team_0`, `security_team_1`, …).
    pub verb: &'static str,
    /// Whether the verb's owner state can require a human decision while the
    /// activation is live.
    pub counts: bool,
    /// Why. Kept beside the answer because "no" is the interesting half of this
    /// table and an unexplained "no" is indistinguishable from an oversight.
    pub why: &'static str,
}

/// Every task-lifecycle verb this build can open an activation on, and whether
/// it is a human demand.
///
/// All of them are `counts: false`, and that is the finding rather than a
/// placeholder: PRD #1419 says in terms that "running tasks are not demands
/// merely because a human started them", and not one of these activations has
/// owner state that stops it progressing until a person decides something. A
/// scan returns a reading, a tractor coupling holds, a dispatched team crosses,
/// works and comes home. What a person genuinely has to act on when that work
/// goes wrong is a *different* piece of state — a repair request, a
/// conversation — and those are counted by their own producers, once.
pub const TASK_DEMAND_INVENTORY: &[TaskDemandRule] = &[
    TaskDemandRule {
        verb: crate::core::task_lifecycle::TASK_VERB_SCAN,
        counts: false,
        why: "a science scan runs to its own reading or refusal; the operator is not asked \
              anything while it runs",
    },
    TaskDemandRule {
        verb: crate::core::task_lifecycle::TASK_VERB_TRACTOR_HOLD,
        counts: false,
        why: "a tractor coupling holds by itself and ends on release, range or power; holding is \
              not a pending decision",
    },
    TaskDemandRule {
        verb: crate::core::task_lifecycle::TASK_VERB_DOCK_HOLD,
        counts: false,
        why: "a formed dock mate persists on its own; the approach that formed it is already over",
    },
    TaskDemandRule {
        verb: crate::core::task_lifecycle::TASK_VERB_UMBILICAL_FLOW,
        counts: false,
        why: "a running transfer moves capacity automatically and closes on capacity, range or \
              cancellation",
    },
    TaskDemandRule {
        verb: crate::core::task_lifecycle::TASK_VERB_EXTERNAL_REPAIR,
        counts: false,
        why: "a dispatched repair team crosses, repairs and returns unattended; the request that \
              asked for it is the demand, and it is counted by the repair producer",
    },
    TaskDemandRule {
        verb: crate::core::task_lifecycle::TASK_VERB_TRANSPORT,
        counts: false,
        why: "a running rescue transport recovers automatically and closes on range, target loss \
              or completion",
    },
    TaskDemandRule {
        verb: crate::security::server::TASK_VERB_SECURITY_TEAM,
        counts: false,
        why: "a committed Security team deploys, works and withdraws on the authored clock; a \
              refused dispatch never opens an activation at all",
    },
];

/// Whether a live activation of `verb` counts as a human demand.
///
/// An unknown verb counts as **no**, deliberately: an activation whose meaning
/// this inventory has never been told is unattributed source state, and PRD
/// #1419 excludes unattributed state rather than guessing at it.
pub fn task_verb_counts(verb: &str) -> bool {
    TASK_DEMAND_INVENTORY
        .iter()
        .find(|rule| verb == rule.verb || verb.starts_with(rule.verb))
        .is_some_and(|rule| rule.counts)
}

// ── The projection ────────────────────────────────────────────────────────────

/// What one Station's workload reads as.
///
/// Four words, not a number and not a colour: PRD #1419 asks for "broad
/// workload meanings", and a percentage would be an opaque precision the
/// evidence cannot justify.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmWorkloadLevel {
    /// Every System this Station AUTHORS is AI-operated. Not "underused" —
    /// there is nobody there to be under-used.
    Backfill,
    /// Every System this Station AUTHORS is damage-disabled or explicitly
    /// offline. Distinct from Backfill because an AI is operating one and
    /// nothing is operating the other, and distinct from a Station whose
    /// Systems have merely MIGRATED to a human-seeking host — that one works
    /// fine and is omitted from the summary rather than levelled
    /// ([`seat_standing`]).
    Offline,
    /// A human seat with no outstanding demand.
    Underused,
    /// One or two outstanding demands, or the count is at the Overloaded
    /// threshold but has not held there for the authored duration yet.
    Engaged,
    /// The count has been at or above the threshold continuously for the
    /// authored duration.
    Overloaded,
}

impl GmWorkloadLevel {
    /// The stable snake_case id the page renders a word for.
    pub fn as_str(self) -> &'static str {
        match self {
            GmWorkloadLevel::Backfill => "backfill",
            GmWorkloadLevel::Offline => "offline",
            GmWorkloadLevel::Underused => "underused",
            GmWorkloadLevel::Engaged => "engaged",
            GmWorkloadLevel::Overloaded => "overloaded",
        }
    }

    /// Whether this level is a statement about a person's workload at all.
    pub fn counts_people(self) -> bool {
        matches!(
            self,
            GmWorkloadLevel::Underused | GmWorkloadLevel::Engaged | GmWorkloadLevel::Overloaded
        )
    }
}

/// One counted demand, as the evidence list names it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmWorkloadDemand {
    /// The canonical source key. Stable while the demand holds, unique across
    /// producers, and the identity deduplication is done on.
    pub key: String,
    /// Which producer in the inventory this came from.
    pub source: String,
    /// One short sentence, as a String Table id plus parameters — the same
    /// shape the attention queue explains a row with, and for the same reason.
    pub reason: GmAttentionReason,
}

/// One Station's advisory workload.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmStationWorkload {
    /// The hull the Station belongs to.
    pub ship: GmEntityReference,
    /// The authored Station id.
    pub station_id: String,
    /// The authored Station name (a String Table id or authored label; the page
    /// resolves it, exactly as the roster does).
    pub station_name: String,
    pub level: GmWorkloadLevel,
    /// Distinct outstanding demands. Always the true count, even when the
    /// evidence list below is truncated.
    pub count: u32,
    /// Which source demands produced the count. Bounded by
    /// [`MAX_GM_WORKLOAD_EVIDENCE`]; this is evidence, not a rating.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub demands: Vec<GmWorkloadDemand>,
    /// How long the count has been at or above [`Self::overload_count`], in
    /// whole simulation seconds. Zero whenever it is below.
    pub sustained_secs: u32,
    /// The authored (or default) count threshold in force.
    pub overload_count: u32,
    /// The authored (or default) duration in force, in simulation seconds.
    pub overload_secs: u32,
}

/// The complete advisory, one row per Station, in a stable order that never
/// moves under a reading operator: ship id, then Station id.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmWorkloadProjection {
    pub stations: Vec<GmStationWorkload>,
}

/// How long each Station has been at or above its overload count, in fixed
/// simulation steps.
///
/// `Presentation`: no fixed-tick system reads it, nothing branches on it, and
/// it is folded into no digest. It IS captured in the snapshot, because elapsed
/// overload is history a restored world cannot recompute — the same argument
/// [`crate::gm_attention::GmIdleNpcWatch`] carries, and the same consequence if
/// it were dropped: a seat thirty seconds deep at save time would be forgiven
/// on resume and the advisory would become a function of when somebody saved.
#[derive(Resource, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmWorkloadWatch {
    /// `ship uuid + '/' + station id` → fixed steps observed at or above the
    /// threshold. A Station below it is simply absent, which is what makes
    /// "falling below resets the timer" true by construction rather than by a
    /// clearing pass.
    spells: BTreeMap<String, u64>,
    /// `ship uuid` → the Navigation-clearance generation this hull has already
    /// flown to (issue #1438). The Navigation demand auto-completes on arrival,
    /// and completion has to be REMEMBERED: a hull that reaches its waypoint and
    /// then flies on past the tolerance has still answered that clearance, and
    /// without the latch the demand would come back at the Helm the moment the
    /// hull drifted out again. A new generation replaces the entry and a cleared
    /// waypoint removes it, so it can only ever silence the course it names.
    ///
    /// Defaulted rather than required, so a snapshot written before #1438
    /// restores into a peer that simply has not seen anybody arrive yet.
    #[serde(default)]
    nav_arrivals: BTreeMap<String, u64>,
}

impl GmWorkloadWatch {
    /// The key one Station's spell is stored under.
    pub fn key(ship: &str, station: &str) -> String {
        format!("{ship}/{station}")
    }

    /// Fixed steps this Station has spent at or above its threshold.
    pub fn ticks(&self, ship: &str, station: &str) -> u64 {
        self.spells
            .get(&Self::key(ship, station))
            .copied()
            .unwrap_or(0)
    }

    /// The Navigation-clearance generation this hull has already flown to, if it
    /// has flown one.
    pub fn nav_arrival(&self, ship: &str) -> Option<u64> {
        self.nav_arrivals.get(ship).copied()
    }

    /// How many Stations are currently mid-spell.
    pub fn len(&self) -> usize {
        self.spells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }
}

/// This peer's last published summary, so an unchanged one is not republished.
///
/// `Presentation` for the ordinary reason: it is a repaint cache derived
/// entirely from live facts, read by nothing authoritative.
#[derive(Resource, Default)]
pub struct GmWorkloadState {
    last: Option<GmWorkloadProjection>,
    /// The summary the last fixed step built, waiting to be published.
    ///
    /// The two halves run in different schedules on purpose: the count and its
    /// stopwatch are decided in the fixed loop (so a paused world contributes
    /// no elapsed overload), while publication is a frame-rate concern like
    /// every other Host Channel flush.
    pending: Option<GmWorkloadProjection>,
}

impl GmWorkloadState {
    /// The most recently published summary, if one has been published.
    pub fn last(&self) -> Option<&GmWorkloadProjection> {
        self.last.as_ref()
    }

    /// The summary the last fixed step built, published or not. Tests read this
    /// to assert on one step's answer without waiting for a change to publish.
    pub fn pending(&self) -> Option<&GmWorkloadProjection> {
        self.pending.as_ref()
    }
}

// ── Collection ────────────────────────────────────────────────────────────────

/// One ship's demands, already attributed, before levels are decided.
///
/// A `BTreeMap` keyed by the canonical source key is the deduplication: a
/// demand reported twice by two representations of the same owner state
/// occupies one entry, while two genuinely independent demands occupy two.
type StationDemands = BTreeMap<String, BTreeMap<String, GmWorkloadDemand>>;

/// The per-ship state one workload pass reads. Grouped into a struct because
/// Bevy's parameter limit is real and every field here is one component.
struct ShipReading<'a> {
    uuid: &'a str,
    config: &'a crate::ship::config::ShipConfig,
    hosts: Option<&'a crate::ship_plugin::HumanSeekingHosts>,
    control_sources: &'a crate::ship_plugin::ShipSystemControlSources,
    waypoint: Option<&'a crate::console::navigation::NavigationWaypoint>,
    issue_state: Option<&'a crate::console::navigation::server::NavClearanceIssueState>,
    clearance: Option<&'a crate::ship_plugin::HelmWaypointClearance>,
    repairs: Option<&'a crate::console::repair::server::RepairRequestQueue>,
    /// Where the hull actually is, for the Navigation clearance's arrival
    /// terminal. The same `ShipPhysics` the AI helm reads.
    physics: Option<&'a crate::ship::state::ShipPhysics>,
    /// The hull's authored `[behaviour]`, for its `waypoint_arrival_radius`.
    behaviour: Option<&'a crate::entities::spawner::BehaviourSection>,
}

impl ShipReading<'_> {
    /// The live control-source resolver, as every predicate here reads it.
    fn sources(&self) -> &crate::ship::control_source::ControlSourceResolver {
        &self.control_sources.0
    }
}

/// The Station that currently owns `system`, or `None` when the hull has no
/// such System at all.
///
/// This is [`crate::command_admission::station_for_system`] and nothing else:
/// the same resolution admission uses, so a human-seeking System's demand is
/// attributed to the seat that is actually presenting it rather than to the
/// seat that authored it.
fn owning_station(reading: &ShipReading<'_>, system: &SystemId) -> Option<StationId> {
    crate::command_admission::station_for_system(reading.config, reading.hosts, system)
}

/// Does a person have to be able to act on `system` for a demand addressed to
/// it to count?
///
/// The live control policy, not the authored rating: a System that is
/// backfilled, damage-disabled or GM-disabled is not asking a human for
/// anything, and this is the predicate that makes a MIXED Station count only
/// its human half.
fn needs_human(reading: &ShipReading<'_>, system: &SystemId) -> bool {
    reading.sources().policy_for(system).accept_human_input
}

/// Add one demand to its Station's bucket, deduplicating by canonical key.
fn record(
    demands: &mut StationDemands,
    station: &StationId,
    key: String,
    source: &str,
    reason: GmAttentionReason,
) {
    demands
        .entry(station.0.clone())
        .or_default()
        .entry(key.clone())
        .or_insert(GmWorkloadDemand {
            key,
            source: source.to_string(),
            reason,
        });
}

/// The pending-Comms producer.
///
/// Source: the live [`CommsInboxRes`]. Stable key: `comms:<message id>` — the
/// id the world minted, which is the same identity the attention queue holds a
/// snooze against. Owner: the ship named by the message, at whichever Station
/// currently holds its `comms` System. Human-action-needed:
/// [`crate::gm_attention::pending`] (unanswered, unwithdrawn, actually offering
/// a response) AND that System accepting human input. Terminal: answered,
/// orphaned, cleared from the inbox, or the requirement for a human ceasing —
/// all of which are the predicate no longer holding, so nothing has to retire
/// a row.
///
/// An unread informational message with no responses is not a demand, and a
/// follow-up placeholder while the other side is still speaking is not a
/// demand: both are the empty-response case `pending` already excludes.
fn collect_comms(
    reading: &ShipReading<'_>,
    inbox: &crate::console::comms::inbox::CommsInbox,
    names: &BTreeMap<String, String>,
    demands: &mut StationDemands,
) {
    let comms = crate::ship::system_registry::comms_system_id();
    let Some(station) = owning_station(reading, &comms) else {
        return;
    };
    if !needs_human(reading, &comms) {
        return;
    }
    for message in inbox.iter() {
        if !crate::gm_attention::pending(message) {
            continue;
        }
        // Attribution, not a guess: a message addressed at a hull belongs to
        // that hull's Comms Station. One with no recipient ship is fleet-wide
        // traffic the crew of this inbox is being asked to answer, so it
        // belongs to the ship whose inbox it is.
        if let Some(recipient) = message.recipient_ship.as_ref() {
            if recipient.0 != reading.uuid {
                continue;
            }
        }
        let sender = names
            .get(&message.sender_uuid)
            .cloned()
            .unwrap_or_else(|| message.sender_uuid.clone());
        record(
            demands,
            &station,
            format!("comms:{}", message.id),
            COMMS_SOURCE,
            GmAttentionReason {
                id: COMMS_REASON.to_string(),
                params: BTreeMap::from([("sender".to_string(), sender)]),
            },
        );
    }
}

/// The Navigation-clearance producer.
///
/// Source: the ship's own [`crate::console::navigation::NavigationWaypoint`],
/// its exactly-once issuer frontier
/// ([`crate::console::navigation::server::NavClearanceIssueState`]), the Helm's
/// latch ([`crate::ship_plugin::HelmWaypointClearance`]), the routing decision
/// [`crate::ship::coordination::route_coordination`] makes about that delivery,
/// and the hull's own position. Stable key: `nav:<ship>#<generation>` — the
/// request generation, so a REPLACEMENT waypoint is a different demand rather
/// than the old one persisting. Owner: the Station holding `helm-steering`, the
/// System the clearance is addressed to.
///
/// # Human-action-needed
///
/// A waypoint is set, its clearance has been issued, the Helm has not latched
/// that generation, the delivery of that clearance actually ROUTED to a person
/// (below), and the hull has not yet arrived.
///
/// # Why routing rather than the issuer frontier alone
///
/// `NavClearanceIssueState::issued_generation` is latched once per generation
/// *whatever the helm's control state* — that is the exactly-once policy the
/// issuer documents, and it is deliberately blind to what happens at the far
/// end. Three different things happen there: an AI Helm CONSUMES the clearance
/// and latches it, an AI Navigation to a human Helm raises a POPUP, and a human
/// Navigation to a human Helm is SUPPRESSED outright, because two people at the
/// same table coordinate out loud. Only the middle case ever asks a person for
/// anything, so the producer asks the router's own question —
/// `route_coordination(navigation's source, the Helm seat's delivery control)`
/// — instead of treating the frontier latch as the demand. Reading it live is
/// the point: a Navigation seat a human takes over stops asking the Helm for
/// something the two of them can settle by talking.
///
/// # Terminal
///
/// Modelled as a task-style demand that AUTO-COMPLETES ON ARRIVAL, plus the two
/// ordinary endings:
///
/// * **arrival** — the hull inside the waypoint's existing arrival tolerance:
///   the hull's authored `[behaviour] waypoint_arrival_radius`, falling back to
///   [`crate::ai::WAYPOINT_ARRIVAL_RADIUS`], which is the same radius
///   `ai::server`'s patrol cursor and `helm_ai`'s Reach completion use. No new
///   constant, and no new acknowledge action on the Helm: a human Helm that has
///   flown the course has answered the clearance by arriving, and asking them to
///   also press something would be inventing work to measure it. Latched per
///   generation, so a hull that arrives and then flies onward does not have the
///   demand come back at it.
/// * **replacement** — a new generation, hence a new key; the old demand simply
///   stops being produced.
/// * **withdrawal** — the waypoint cleared, or the routing ceasing to be a
///   popup (an AI Helm taking the seat, the Navigation seat becoming human).
///
/// The ACTUAL navigation waypoint is never touched here. Arrival completes the
/// DEMAND, not the course: Navigation keeps sole ownership of setting and
/// clearing waypoints, and a standing order to hold at a point is still a
/// standing order after the hull gets there.
///
/// # Why the completion is a producer-side check, not a `TaskKey` activation
///
/// [`crate::core::task_lifecycle`] activations are opened and closed by the
/// consoles that own the work, from real admitted commands, and they are
/// authoritative simulation state that every peer folds identically. This
/// advisory is peer-local presentation on a GM desk; it must not write task
/// state, and there is no console verb behind a nav clearance to open an
/// activation from — the clearance is a Channel-3 message, not a task. So the
/// completion is computed where it is read, from owner state, in the fixed loop:
/// deterministic simulation-time input (position, waypoint, control sources),
/// no authority touched. The per-generation latch it needs rides in
/// [`GmWorkloadWatch`], beside the overload stopwatch and for the same reason —
/// an arrival is history a restored world cannot recompute.
fn collect_navigation(
    reading: &ShipReading<'_>,
    arrived: &mut BTreeMap<String, u64>,
    demands: &mut StationDemands,
) {
    let steering = crate::ship::system_registry::helm_steering_system_id();
    let (Some(waypoint), Some(issue)) = (reading.waypoint, reading.issue_state) else {
        arrived.remove(reading.uuid);
        return;
    };
    // No waypoint at all: a clear bumps the generation but asks for nothing —
    // and takes any arrival latch with it, so the next course starts clean.
    let Some(snapshot) = waypoint.snapshot() else {
        arrived.remove(reading.uuid);
        return;
    };
    let generation = waypoint.generation();
    // A latch from an earlier generation is about a course that no longer
    // exists. Drop it rather than let it silence the replacement.
    if arrived
        .get(reading.uuid)
        .is_some_and(|latched| *latched != generation)
    {
        arrived.remove(reading.uuid);
    }
    // Not yet issued means it is still Navigation's to send, not Helm's to fly.
    if issue.issued_generation() != Some(generation) {
        return;
    }
    if reading.clearance.map(|latch| latch.0) == Some(Some(generation)) {
        return;
    }
    let Some(station) = owning_station(reading, &steering) else {
        return;
    };
    if !delivers_to_a_person(reading, &station) {
        return;
    }
    // Already flown. The demand is complete for this generation and stays
    // complete; the course itself is untouched.
    if arrived.get(reading.uuid) == Some(&generation) {
        return;
    }
    if hull_has_arrived(reading, snapshot.x, snapshot.z) {
        arrived.insert(reading.uuid.to_string(), generation);
        return;
    }
    record(
        demands,
        &station,
        format!("nav:{}#{generation}", reading.uuid),
        NAVIGATION_SOURCE,
        GmAttentionReason {
            id: NAVIGATION_REASON.to_string(),
            params: BTreeMap::from([
                ("x".to_string(), format!("{}", snapshot.x.round() as i64)),
                ("z".to_string(), format!("{}", snapshot.z.round() as i64)),
            ]),
        },
    );
}

/// Would a Navigation clearance delivered to `station` right now actually reach
/// a person?
///
/// The router's own two calls, in the router's own order: the Station's delivery
/// control ([`crate::ship::coordination_systems::station_delivery_policy`],
/// which for a Helm seat is the AXES rather than one representative System) and
/// then [`crate::ship::coordination::route_coordination`] against Navigation's
/// live control source. `Popup` — an AI sender to a human seat — is the only
/// outcome that asks anybody for anything: `Consume` is the AI Helm's own work
/// and `Suppress` is two people who can talk to each other.
fn delivers_to_a_person(reading: &ShipReading<'_>, station: &StationId) -> bool {
    let (_, target) = crate::ship::coordination_systems::station_delivery_policy(
        reading.config,
        reading.control_sources,
        station,
    );
    let sender = reading
        .sources()
        .source_for(&crate::ship::system_registry::navigation_system_id());
    crate::ship::coordination::route_coordination(sender, target)
        == crate::ship::coordination::DeliverAction::Popup
}

/// Is the hull inside the waypoint's arrival tolerance?
///
/// The tolerance is the hull's authored `[behaviour] waypoint_arrival_radius`,
/// falling back to [`crate::ai::WAYPOINT_ARRIVAL_RADIUS`] — read, not invented,
/// so a designer who widens a hull's arrival radius widens what counts as
/// "flown" here too. Strictly inside, matching both existing readers. A hull
/// with no position at all has demonstrably not arrived.
fn hull_has_arrived(reading: &ShipReading<'_>, x: f32, z: f32) -> bool {
    let Some(physics) = reading.physics else {
        return false;
    };
    let radius = reading
        .behaviour
        .map_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS, |behaviour| {
            behaviour.0.waypoint_arrival_radius
        });
    let dx = x - physics.x;
    let dz = z - physics.z;
    (dx * dx + dz * dz).sqrt() < radius
}

/// The Repair-dispatch producer.
///
/// Source: the ship's own
/// [`crate::console::repair::server::RepairRequestQueue`] — the queue the
/// Channel-3 `RepairRequest` writes into, which is the owner state behind the
/// popup rather than the popup itself. Stable key:
/// `repair:<ship>/<damaged station id>`, which is exactly the identity the
/// queue merges on, so a second worsening request for the same Station is the
/// same demand and counts once. Owner: the Station holding the `repair`
/// System, the destination the request is addressed to. Human-action-needed:
/// the entry is in the queue and that System accepts human input. Terminal:
/// the queue pruning the entry when the damage it named is gone, or the seat
/// ceasing to be human.
fn collect_repairs(reading: &ShipReading<'_>, demands: &mut StationDemands) {
    let Some(queue) = reading.repairs else {
        return;
    };
    let repair = crate::ship::system_registry::repair_system_id();
    let Some(station) = owning_station(reading, &repair) else {
        return;
    };
    if !needs_human(reading, &repair) {
        return;
    }
    for entry in &queue.entries {
        record(
            demands,
            &station,
            format!("repair:{}/{}", reading.uuid, entry.station_id),
            REPAIR_SOURCE,
            GmAttentionReason {
                id: REPAIR_REASON.to_string(),
                params: BTreeMap::from([
                    (
                        "station".to_string(),
                        if entry.station_label.is_empty() {
                            entry.station_id.clone()
                        } else {
                            entry.station_label.clone()
                        },
                    ),
                    ("tier".to_string(), tier_string_id(entry.tier).to_string()),
                ]),
            },
        );
    }
}

/// The task-activation producer.
///
/// Source: the live [`crate::core::task_lifecycle::TaskLifecycles`] registry.
/// Stable key: the activation's own [`TaskKey`](crate::core::task_lifecycle::TaskKey)
/// wire form, which already separates a restart from the activation it
/// replaced. Owner: the activation's operator hull and the Station holding its
/// `slot.system`. Human-action-needed: [`task_verb_counts`], which is `false`
/// for every verb this build ships — see [`TASK_DEMAND_INVENTORY`]. Terminal:
/// the activation's own single terminal event.
///
/// The walk is written out in full rather than short-circuited on the constant,
/// because the exclusions it performs are the contract: an activation on
/// another hull, on a System this hull does not own, on an AI-operated System,
/// or with a verb the inventory has never heard of, is excluded for a stated
/// reason rather than by never being looked at.
fn collect_tasks(
    reading: &ShipReading<'_>,
    lifecycles: &crate::core::task_lifecycle::TaskLifecycles,
    demands: &mut StationDemands,
) {
    for activation in lifecycles.active() {
        if activation.key.slot.operator != reading.uuid {
            continue;
        }
        if !task_verb_counts(&activation.key.slot.verb) {
            continue;
        }
        let system = SystemId(activation.key.slot.system.clone());
        let Some(station) = owning_station(reading, &system) else {
            continue;
        };
        if !needs_human(reading, &system) {
            continue;
        }
        record(
            demands,
            &station,
            format!("task:{}", activation.key.as_str()),
            TASK_SOURCE,
            GmAttentionReason {
                id: TASK_REASON.to_string(),
                params: BTreeMap::from([("verb".to_string(), activation.key.slot.verb.clone())]),
            },
        );
    }
}

/// What one Station is, before a single demand is counted.
enum SeatStanding {
    /// Every System this Station authors is currently hosted at ANOTHER
    /// Station, because human-seeking migration moved it there. Nobody is
    /// sitting HERE, and this is not a seat the advisory has anything to say
    /// about — see [`seat_standing`].
    Migrated,
    /// The Station is a seat, reduced to one control source.
    Here(ControlSource),
}

/// Decide whether a Station is a seat at all, and if so who is operating it.
///
/// Membership is **authored** — [`crate::ship::config::ShipConfig::systems_for_station`],
/// the same `[[system]] station = ...` blocks the lobby roster pill projects
/// into `station_systems` (`crate::lobby::server`) and the same set the
/// Channel-3 router's own generic branch reduces
/// (`crate::ship::coordination_systems::station_delivery_policy`). It is
/// deliberately NOT the live [`owning_station`] resolution, which is a
/// different question: *where is this System being operated from right now*.
/// Filtering the fleet's Systems by live ownership made a Station whose
/// Systems had all migrated to a human-seeking host collect the EMPTY set, and
/// [`crate::ship::coordination::seat_control_source`] answers `Offline` for an
/// empty set — so the stock Alliance Cruiser's `navigation`, `command` and
/// `comms` rows each read "No System here can be operated" the moment somebody
/// crewed the seat that was hosting them, contradicting the roster pill beside
/// them and [`GmWorkloadLevel::Offline`]'s own meaning.
///
/// So the two facts are now read from the two sources that mean them:
///
/// * Backfill / Offline / the mixed-Station human subset come from the LIVE
///   control sources of the AUTHORED Systems. `Offline` therefore means what
///   it says — every authored System damage-disabled or explicitly offline.
/// * A Station every one of whose authored Systems is hosted elsewhere is
///   [`SeatStanding::Migrated`] and is OMITTED from the summary entirely. It is
///   not Offline (its Systems work), not Backfill (no AI holds it) and not
///   Underused (nobody is there to be under-used): it is not a seat anybody
///   holds this tick. Its demands are already attributed to, and counted at,
///   the host Station by [`owning_station`], so omitting the row loses nothing
///   and naming it twice would double the fleet's apparent workload.
///
/// A Station that HOSTS migrated Systems keeps counting their human-required
/// demands — that is [`owning_station`] doing its job, and it is unaffected by
/// this: only the Station's own identity is decided here.
///
/// A Station that authors no Systems at all is `Here(Offline)`, exactly as
/// [`crate::ship::coordination::seat_control_source`] documents.
fn seat_standing(reading: &ShipReading<'_>, station: &StationId) -> SeatStanding {
    let authored: Vec<&crate::ship::config::SystemInstanceConfig> =
        reading.config.systems_for_station(station).collect();
    if !authored.is_empty()
        && authored
            .iter()
            .all(|system| owning_station(reading, &system.id).as_ref() != Some(station))
    {
        return SeatStanding::Migrated;
    }
    let policies: Vec<_> = authored
        .iter()
        .map(|system| reading.sources().policy_for(&system.id))
        .collect();
    SeatStanding::Here(crate::ship::coordination::seat_control_source(&policies))
}

// ── Authored settings ─────────────────────────────────────────────────────────

/// The overload thresholds in force for one world, resolved once.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Thresholds {
    count: u32,
    ticks: u64,
    secs: u32,
}

fn thresholds(settings: &crate::gm_attention::GmAttentionSettings, hz: f32) -> Thresholds {
    Thresholds {
        count: settings.workload_overload_count(),
        ticks: settings.workload_overload_ticks(hz),
        secs: settings.workload_overload_secs.max(0.0).round() as u32,
    }
}

// ── Systems ───────────────────────────────────────────────────────────────────

type WorkloadShipQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        Option<&'static EntityName>,
        &'static crate::ship_plugin::ShipConfigComponent,
        &'static crate::ship_plugin::ShipSystemControlSources,
        Option<&'static crate::ship_plugin::HumanSeekingHosts>,
        Option<&'static crate::console::navigation::NavigationWaypoint>,
        Option<&'static crate::console::navigation::server::NavClearanceIssueState>,
        Option<&'static crate::ship_plugin::HelmWaypointClearance>,
        Option<&'static crate::console::repair::server::RepairRequestQueue>,
        Option<&'static crate::ship::state::ShipPhysics>,
        Option<&'static crate::entities::spawner::BehaviourSection>,
    ),
    (
        With<crate::server_app::Ship>,
        With<crate::lockstep::FleetSlotOf>,
    ),
>;

/// Build the complete summary from live state, and advance each Station's
/// overload stopwatch by exactly one fixed step.
///
/// One system, in `FixedLast` before `advance_sim_tick`, for the reason
/// [`crate::gm_attention::observe_idle_npcs`] is: the duration is counted in
/// SIMULATION steps, so a paused world contributes nothing (the step never
/// starts) and the boundary is exact rather than frame-paced. Counting and
/// levelling in one pass is what keeps the count that moved the stopwatch and
/// the count the level was decided from the same number.
#[allow(clippy::too_many_arguments)]
pub fn observe_station_workload(
    world_config: Option<Res<WorldConfig>>,
    inbox: Option<Res<CommsInboxRes>>,
    lifecycles: Option<Res<crate::core::task_lifecycle::TaskLifecycles>>,
    ships: WorkloadShipQuery,
    names_query: Query<(&EntityUuid, Option<&EntityName>)>,
    mut watch: ResMut<GmWorkloadWatch>,
    mut state: ResMut<GmWorkloadState>,
) {
    let settings = world_config
        .as_deref()
        .map(|world| world.gm_attention.clone())
        .unwrap_or_default();
    if settings.workload_disabled {
        // A silenced advisory publishes an EMPTY summary rather than nothing,
        // so a desk shows no workload rather than the last one it happened to
        // see before the world silenced it. The stopwatch is dropped with it:
        // resuming the advisory should not inherit a wait nobody was watching.
        watch.spells.clear();
        watch.nav_arrivals.clear();
        state.pending = Some(GmWorkloadProjection::default());
        return;
    }
    let hz = world_config.as_deref().map_or_else(
        || crate::entities::config::GlobalConfig::default().sim_tick_hz,
        |world| world.global.sim_tick_hz,
    );
    let limits = thresholds(&settings, hz);
    let names: BTreeMap<String, String> = names_query
        .iter()
        .map(|(uuid, name)| {
            (
                uuid.0.clone(),
                name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
            )
        })
        .collect();
    let empty_inbox = crate::console::comms::inbox::CommsInbox::new();
    let empty_lifecycles = crate::core::task_lifecycle::TaskLifecycles::default();
    let inbox = inbox.as_deref().map_or(&empty_inbox, |inbox| &inbox.0);
    let lifecycles = lifecycles.as_deref().unwrap_or(&empty_lifecycles);

    let mut rows = Vec::new();
    let mut live: BTreeSet<String> = BTreeSet::new();
    let mut live_ships: BTreeSet<String> = BTreeSet::new();
    for (
        uuid,
        name,
        config,
        sources,
        hosts,
        waypoint,
        issue_state,
        clearance,
        repairs,
        physics,
        behaviour,
    ) in &ships
    {
        live_ships.insert(uuid.0.clone());
        let reading = ShipReading {
            uuid: &uuid.0,
            config: &config.0,
            hosts,
            control_sources: sources,
            waypoint,
            issue_state,
            clearance,
            repairs,
            physics,
            behaviour,
        };
        let mut demands: StationDemands = BTreeMap::new();
        collect_comms(&reading, inbox, &names, &mut demands);
        collect_navigation(&reading, &mut watch.nav_arrivals, &mut demands);
        collect_repairs(&reading, &mut demands);
        collect_tasks(&reading, lifecycles, &mut demands);

        let ship = GmEntityReference {
            entity_id: uuid.0.clone(),
            name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
        };
        for station in &config.0.stations {
            let counted = demands.get(&station.id.0);
            let count = counted.map_or(0, BTreeMap::len) as u32;
            // A Station whose Systems have all migrated to a human-seeking
            // host is nobody's seat this tick: it is omitted rather than
            // levelled, and it takes its stopwatch with it (the key is never
            // added to `live`).
            let seat = match seat_standing(&reading, &station.id) {
                SeatStanding::Migrated => continue,
                SeatStanding::Here(source) => source,
            };
            let key = GmWorkloadWatch::key(&uuid.0, &station.id.0);
            // Only a seat with a person at it runs a stopwatch. A Backfill or
            // Offline seat that later gains a human starts from zero, which is
            // the honest reading: nobody was overloaded while nobody was there.
            let sustained_ticks = if seat == ControlSource::Human && count >= limits.count {
                live.insert(key.clone());
                let entry = watch.spells.entry(key).or_insert(0);
                *entry = entry.saturating_add(1);
                *entry
            } else {
                0
            };
            let level = match seat {
                ControlSource::Ai => GmWorkloadLevel::Backfill,
                ControlSource::Offline => GmWorkloadLevel::Offline,
                ControlSource::Human if count == 0 => GmWorkloadLevel::Underused,
                ControlSource::Human
                    if sustained_ticks >= limits.ticks && count >= limits.count =>
                {
                    GmWorkloadLevel::Overloaded
                }
                ControlSource::Human => GmWorkloadLevel::Engaged,
            };
            let demands = if level.counts_people() {
                counted.map_or_else(Vec::new, |bucket| {
                    bucket
                        .values()
                        .take(MAX_GM_WORKLOAD_EVIDENCE)
                        .cloned()
                        .collect()
                })
            } else {
                Vec::new()
            };
            rows.push(GmStationWorkload {
                ship: ship.clone(),
                station_id: station.id.0.clone(),
                station_name: station.name.clone(),
                level,
                // A seat with nobody at it reports its word, not a count of
                // work the AI is quietly doing.
                count: if level.counts_people() { count } else { 0 },
                demands,
                sustained_secs: sim_seconds(sustained_ticks, hz),
                overload_count: limits.count,
                overload_secs: limits.secs,
            });
        }
    }
    // A Station that fell below the threshold — or stopped existing — takes its
    // spell with it, which is what makes "below the threshold resets the timer"
    // a consequence of the data rather than a second rule.
    watch.spells.retain(|key, _| live.contains(key));
    // A hull that has left the projection takes its arrival latch with it, so a
    // recycled uuid cannot inherit somebody else's flown course.
    watch
        .nav_arrivals
        .retain(|ship, _| live_ships.contains(ship));
    rows.sort_by(|left, right| {
        left.ship
            .entity_id
            .cmp(&right.ship.entity_id)
            .then_with(|| left.station_id.cmp(&right.station_id))
    });
    rows.truncate(MAX_GM_WORKLOAD_STATIONS);
    state.pending = Some(GmWorkloadProjection { stations: rows });
}

/// Whole simulation seconds from a fixed-step count, floored.
fn sim_seconds(ticks: u64, hz: f32) -> u32 {
    let hz = f64::from(hz);
    if !(hz.is_finite() && hz > 0.0) {
        return 0;
    }
    ((ticks as f64 / hz).floor().max(0.0) as u64).min(u64::from(u32::MAX)) as u32
}

/// Publish the summary onto the page-local Host Channel when it changes.
///
/// Never emits an unchanged payload, for the attention queue's reason: a desk
/// panel that repainted sixty times a second would move controls under a
/// reading operator, which is the failure PRD #1418 story 23 names. Every
/// published field is a discrete fact — level, count, whole seconds — so an
/// equality comparison is a real "did anything change", not a float that
/// drifts every frame.
pub fn publish_workload_projection(
    mut state: ResMut<GmWorkloadState>,
    mut writer: MessageWriter<GmWorkloadChanged>,
) {
    let Some(next) = state.pending.take() else {
        return;
    };
    if state.last.as_ref() == Some(&next) {
        return;
    }
    state.last = Some(next.clone());
    writer.write(GmWorkloadChanged { payload: next });
}

/// Registers the workload advisory on a GM-presenting peer, exactly as
/// [`crate::gm_attention::GmAttentionPlugin`] registers the attention queue.
pub struct GmWorkloadPlugin;

impl Plugin for GmWorkloadPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<GmWorkloadState>()
            .declare_state::<GmWorkloadState>(StateClass::Presentation, "gm-t3-station-workload")
            .init_resource::<GmWorkloadWatch>()
            // Presentation for the ordinary reason, but carried in the snapshot
            // because elapsed overload is history — see the type's own doc.
            .declare_state::<GmWorkloadWatch>(StateClass::Presentation, "gm-t3-station-workload")
            .add_message::<GmWorkloadChanged>()
            .add_systems(
                FixedLast,
                observe_station_workload
                    .before(crate::sim_tick::advance_sim_tick)
                    .run_if(crate::gm_projection::gm_presentation_active),
            )
            .add_systems(
                PostUpdate,
                publish_workload_projection.run_if(crate::gm_projection::gm_presentation_active),
            );
    }
}

/// Restore the sustained-overload stopwatch, only into a peer that keeps one.
pub fn restore_watch(world: &mut World, watch: &GmWorkloadWatch) {
    if world.contains_resource::<GmWorkloadWatch>() {
        world.insert_resource(watch.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_task_verb_has_a_recorded_decision() {
        // The ratchet: a new TASK_VERB_* constant must be given an entry (and
        // therefore a reason) here rather than silently defaulting to "not a
        // demand" through the unknown-verb rule.
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/core/task_lifecycle.rs"
        ))
        .expect("task_lifecycle.rs is readable");
        let declared: Vec<String> = source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("pub const TASK_VERB_"))
            .filter_map(|rest| rest.split('"').nth(1).map(str::to_string))
            .collect();
        assert!(
            declared.len() >= 6,
            "expected the shipped verb vocabulary, found {declared:?}"
        );
        for verb in declared {
            assert!(
                TASK_DEMAND_INVENTORY.iter().any(|rule| rule.verb == verb),
                "task verb '{verb}' has no entry in TASK_DEMAND_INVENTORY: decide whether it can \
                 require a human and say why"
            );
        }
    }

    #[test]
    fn no_shipped_task_verb_is_a_demand_and_an_unknown_verb_is_excluded() {
        for rule in TASK_DEMAND_INVENTORY {
            assert!(!rule.counts, "{} unexpectedly counts", rule.verb);
            assert!(!rule.why.is_empty(), "{} has no reason", rule.verb);
            assert!(!task_verb_counts(rule.verb));
        }
        // Per-team Security slots are `security_team_0`, `security_team_1`, …
        assert!(!task_verb_counts("security_team_1"));
        // And an activation this build has never heard of is unattributed
        // source state, which PRD #1419 excludes rather than guesses at.
        assert!(!task_verb_counts("teleport_the_admiral"));
    }

    #[test]
    fn levels_say_whether_they_are_about_a_person() {
        assert!(!GmWorkloadLevel::Backfill.counts_people());
        assert!(!GmWorkloadLevel::Offline.counts_people());
        assert!(GmWorkloadLevel::Underused.counts_people());
        assert!(GmWorkloadLevel::Engaged.counts_people());
        assert!(GmWorkloadLevel::Overloaded.counts_people());
        assert_eq!(GmWorkloadLevel::Overloaded.as_str(), "overloaded");
    }

    #[test]
    fn one_demand_reported_twice_occupies_one_slot() {
        let mut demands: StationDemands = BTreeMap::new();
        let station = StationId("engineering".into());
        let reason = GmAttentionReason {
            id: REPAIR_REASON.into(),
            params: BTreeMap::new(),
        };
        record(
            &mut demands,
            &station,
            "repair:ship/helm".into(),
            REPAIR_SOURCE,
            reason.clone(),
        );
        record(
            &mut demands,
            &station,
            "repair:ship/helm".into(),
            REPAIR_SOURCE,
            reason.clone(),
        );
        record(
            &mut demands,
            &station,
            "repair:ship/weapons".into(),
            REPAIR_SOURCE,
            reason,
        );
        assert_eq!(demands[&station.0].len(), 2);
    }

    #[test]
    fn the_stopwatch_keys_and_reads_one_station_at_a_time() {
        let watch = GmWorkloadWatch {
            spells: BTreeMap::from([(GmWorkloadWatch::key("ship-a", "helm"), 12)]),
            nav_arrivals: BTreeMap::from([("ship-a".to_string(), 7)]),
        };
        assert_eq!(watch.ticks("ship-a", "helm"), 12);
        assert_eq!(watch.ticks("ship-a", "comms"), 0);
        assert_eq!(watch.len(), 1);
        assert!(!watch.is_empty());
        // The arrival latch is per HULL, not per Station: one course, one hull,
        // one answer to "has it been flown".
        assert_eq!(watch.nav_arrival("ship-a"), Some(7));
        assert_eq!(watch.nav_arrival("ship-b"), None);
    }

    #[test]
    fn whole_simulation_seconds_are_floored_and_survive_a_broken_rate() {
        assert_eq!(sim_seconds(90, 30.0), 3);
        assert_eq!(sim_seconds(89, 30.0), 2);
        assert_eq!(sim_seconds(90, 0.0), 0);
    }
}
