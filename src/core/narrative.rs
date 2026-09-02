//! Narrative telemetry — the authored mission timeline a headless run records.
//!
//! [`crate::core::balance`] answers "what did the simulation do to whom": every
//! hit, every shot, every knockout. That is the *combat* view, and it is
//! deliberately exhaustive — a ledger has to see every tick to total a rate.
//!
//! This module answers a different question: **what happened in the story**.
//! An Objective was posted and later failed; a deadline ran out; an authored
//! beat fired; the crew opened a channel and picked an answer; the shuttle the
//! scenario cared about was destroyed. Those are the facts an after-action
//! reading is built from, and none of them can be recovered from the damage
//! ledger.
//!
//! Three rules keep the two apart (PRD #1337, issue #1338):
//!
//! 1. **Separate from [`crate::core::balance::BalanceEvent`].** Narrative
//!    telemetry supplements the combat ledger; it never changes it. A story
//!    beat that also happens to be a kill produces one event in each stream,
//!    and the ledgers are computed from the balance stream alone.
//! 2. **Authored, not inferred.** Nothing here fires because an asteroid took a
//!    hit. An Objective/deadline/Comms transition is authored by construction;
//!    a beat is an explicit `ctx.effects.narrative_beat(..)` call; an entity
//!    outcome requires the scenario to have MARKED that entity
//!    ([`NarrativeMark`]). That is what keeps the timeline readable rather than
//!    drowned in routine simulation.
//! 3. **String Ids, never localized prose.** Every text-shaped field carries
//!    the `strings.csv` id verbatim (an Objective's `text`, a Comms body, a
//!    deadline's label), so downstream analysis can localize or narrate it
//!    consistently — and so the timeline is byte-stable across locales.
//!
//! # Determinism
//!
//! Nothing in `src/sim_digest.rs` or `src/snapshot.rs` reads a narrative event,
//! and no fold stage walks [`crate::core::telemetry::RunTelemetry`]'s narrative
//! vector. Recording is therefore inert to the authoritative digest by
//! construction: the events are produced from state the sim already decided,
//! stamped at collection, and read only by the run report.

use bevy::prelude::*;
use std::collections::BTreeMap;

/// What kind of authored moment a [`NarrativeEvent`] records.
///
/// The vocabulary is the whole of PRD #1337's mission-timeline surface, not
/// just the part wired today: an owning slice that lands later (the ship's
/// computer, the structured post-mission report) finds its kind already
/// defined, already in the JSON schema, and already covered by the
/// stream-policy guard below. Kinds with no emitter yet are marked as such on
/// their own doc line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NarrativeKind {
    /// An Objective became `Active`. `id` is the objective id; `detail` carries
    /// its `text` String Id and whether it is mandatory.
    ObjectivePosted,
    /// An Objective transitioned to `Completed`.
    ObjectiveCompleted,
    /// An Objective transitioned to `Failed`.
    ObjectiveFailed,
    /// An Objective can no longer be completed even though it has not been
    /// failed. **No emitter yet**: `ObjectiveStatus` has no `Impossible`
    /// variant, so nothing can observe the transition. The kind is defined
    /// here so the slice that adds the status does not also have to reshape
    /// this schema.
    ObjectiveImpossible,
    /// A named `[[deadline]]` reached its due tick and dispatched its handler.
    /// `id` is the deadline's presentation id; `detail` carries its label
    /// String Id.
    DeadlineFired,
    /// An authored story beat fired — `ctx.effects.narrative_beat("id")`. The
    /// scenario's own punctuation, with no other meaning to the simulation.
    BeatFired,
    /// A Comms message reached the inbox. `id` is the thread id; `detail`
    /// carries the message id, the body String Id and the response count.
    CommsOpened,
    /// A Comms response was picked (by a human console or by the Comms AI —
    /// both traverse the same admitted command). `id` is the thread id, the
    /// same identity [`Self::CommsOpened`] carries, so an answer pairs with the
    /// message that asked; `detail` carries the message id, the response index
    /// and the `on_pick` fn name.
    CommsAnswered,
    /// A marked entity entered the world.
    MarkedEntitySpawned,
    /// A marked entity was disabled but not destroyed. Authored:
    /// `ctx.effects.narrative_outcome("name", "disabled")`.
    MarkedEntityDisabled,
    /// A marked entity was destroyed.
    ///
    /// Emitted automatically for any entity carrying a [`NarrativeMark`] on
    /// EITHER of the engine's two removal paths, so a marked hull can never
    /// vanish from the timeline unremarked:
    ///
    /// * a combat kill, off the `BalanceEvent::EntityDestroyed` chokepoint; and
    /// * a **scripted** removal — `ctx.effects.destroy_entity(name)` or the
    ///   deferred `ctx.schedule.in_seconds(n).destroy_entity(name)`.
    ///
    /// The scripted path is a *fallback*, not a second chokepoint. A scripted
    /// removal is an authorial act, and an author who removes a hull to say
    /// something else about it — rescued, escaped, abandoned — says so with
    /// `ctx.effects.narrative_outcome(..)` **before, or on the same tick as,**
    /// the destroy; that outcome then stands alone and no death is invented.
    /// The automatic death fires only when the run's timeline holds no authored
    /// outcome for that entity at all. Nothing is written to
    /// [`crate::core::balance`] either way: a rescue-by-despawn must never
    /// count as a destruction in the combat ledger.
    MarkedEntityDestroyed,
    /// A marked entity got away. Authored.
    MarkedEntityEscaped,
    /// A marked entity was saved. Authored.
    MarkedEntityRescued,
    /// A marked entity was left behind. Authored.
    MarkedEntityAbandoned,
    /// The ship's computer posted a message. **No emitter yet** — the
    /// computer-message system is a later slice of PRD #1337.
    ComputerMessagePosted,
    /// A posted computer message was acknowledged or retired. **No emitter
    /// yet**, same slice as [`Self::ComputerMessagePosted`].
    ComputerMessageCleared,
    /// One row of the structured post-mission report moved. **No emitter yet**
    /// — the scored report is a later slice of PRD #1337. Deliberately kept
    /// OUT of the ndjson timeline stream (see [`Self::in_timeline_stream`]).
    ReportRowUpdated,
    /// The post-mission report was finalized. **No emitter yet**, same slice as
    /// [`Self::ReportRowUpdated`].
    ReportFinalized,
}

impl NarrativeKind {
    /// How many kinds this enum has.
    ///
    /// Hand-maintained for the same reason
    /// [`crate::core::balance::BalanceEvent::VARIANT_COUNT`] is: its only job is
    /// to fail [`Self::ALL`]'s coverage test when a kind is added, forcing
    /// whoever adds one to say whether it belongs in the ndjson timeline or is
    /// fold-only. A derived count would track the enum silently and guard
    /// nothing.
    pub const KIND_COUNT: usize = 18;

    /// Every kind, in declaration order. The order is the enum's, which is also
    /// `Ord`'s, so a per-kind fold keyed on this is stable.
    pub const ALL: [NarrativeKind; Self::KIND_COUNT] = [
        NarrativeKind::ObjectivePosted,
        NarrativeKind::ObjectiveCompleted,
        NarrativeKind::ObjectiveFailed,
        NarrativeKind::ObjectiveImpossible,
        NarrativeKind::DeadlineFired,
        NarrativeKind::BeatFired,
        NarrativeKind::CommsOpened,
        NarrativeKind::CommsAnswered,
        NarrativeKind::MarkedEntitySpawned,
        NarrativeKind::MarkedEntityDisabled,
        NarrativeKind::MarkedEntityDestroyed,
        NarrativeKind::MarkedEntityEscaped,
        NarrativeKind::MarkedEntityRescued,
        NarrativeKind::MarkedEntityAbandoned,
        NarrativeKind::ComputerMessagePosted,
        NarrativeKind::ComputerMessageCleared,
        NarrativeKind::ReportRowUpdated,
        NarrativeKind::ReportFinalized,
    ];

    /// The stable snake_case label written into JSON and ndjson. Hand-written,
    /// not derived, so the wire vocabulary is visible at the point it is
    /// promised.
    pub fn as_str(self) -> &'static str {
        match self {
            NarrativeKind::ObjectivePosted => "objective_posted",
            NarrativeKind::ObjectiveCompleted => "objective_completed",
            NarrativeKind::ObjectiveFailed => "objective_failed",
            NarrativeKind::ObjectiveImpossible => "objective_impossible",
            NarrativeKind::DeadlineFired => "deadline_fired",
            NarrativeKind::BeatFired => "beat_fired",
            NarrativeKind::CommsOpened => "comms_opened",
            NarrativeKind::CommsAnswered => "comms_answered",
            NarrativeKind::MarkedEntitySpawned => "marked_entity_spawned",
            NarrativeKind::MarkedEntityDisabled => "marked_entity_disabled",
            NarrativeKind::MarkedEntityDestroyed => "marked_entity_destroyed",
            NarrativeKind::MarkedEntityEscaped => "marked_entity_escaped",
            NarrativeKind::MarkedEntityRescued => "marked_entity_rescued",
            NarrativeKind::MarkedEntityAbandoned => "marked_entity_abandoned",
            NarrativeKind::ComputerMessagePosted => "computer_message_posted",
            NarrativeKind::ComputerMessageCleared => "computer_message_cleared",
            NarrativeKind::ReportRowUpdated => "report_row_updated",
            NarrativeKind::ReportFinalized => "report_finalized",
        }
    }

    /// Whether this kind belongs in the ndjson *timeline stream*.
    ///
    /// The narrative stream is already authored-only, so unlike
    /// [`crate::core::balance::BalanceEvent::in_timeline_stream`] this is not
    /// holding back a per-tick flood today. It exists from day one anyway, and
    /// with one real member, because the flood this surface WILL see is
    /// [`Self::ReportRowUpdated`]: a scored report row is re-scored as the
    /// mission moves, which is a *rate*, not a beat. Its final value is what an
    /// after-action reading wants, and that arrives in
    /// [`NarrativeKind::ReportFinalized`] and in the folded timeline; a reader
    /// scrolling the stream does not want the intermediate arithmetic.
    ///
    /// Filtered at the *stream*, never at emission — the same split
    /// `BalanceEvent` makes, and for the same reason: the fold has to see
    /// everything to be exact.
    pub fn in_timeline_stream(self) -> bool {
        !matches!(self, NarrativeKind::ReportRowUpdated)
    }

    /// Parse the author-facing outcome word on
    /// `ctx.effects.narrative_outcome(entity, outcome)`.
    ///
    /// Only the marked-entity outcomes are authorable: Objective, deadline and
    /// Comms transitions are observed from the state that already changed, and
    /// letting a scenario *declare* one would let it lie about what happened.
    /// `Err` on anything else, so a typo raises at the script boundary rather
    /// than silently recording the wrong beat.
    pub fn parse_outcome(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "spawned" => Ok(NarrativeKind::MarkedEntitySpawned),
            "disabled" => Ok(NarrativeKind::MarkedEntityDisabled),
            "destroyed" => Ok(NarrativeKind::MarkedEntityDestroyed),
            "escaped" => Ok(NarrativeKind::MarkedEntityEscaped),
            "rescued" => Ok(NarrativeKind::MarkedEntityRescued),
            "abandoned" => Ok(NarrativeKind::MarkedEntityAbandoned),
            other => Err(format!(
                "unknown narrative outcome '{other}' (expected one of: spawned, \
                 disabled, destroyed, escaped, rescued, abandoned)"
            )),
        }
    }

    /// Whether this kind is a marked entity's *outcome* — the fate the story
    /// records for it, as opposed to its arrival.
    ///
    /// The one consumer is the no-silent-vanish fallback in
    /// `crate::narrative::emit_authored_and_marked_entity_narrative`: an entity
    /// that already has one of these recorded has had its fate stated, so a
    /// later scripted removal adds nothing and stays silent. Spawning is
    /// deliberately excluded — arriving is not a fate.
    pub fn is_marked_entity_outcome(self) -> bool {
        matches!(
            self,
            NarrativeKind::MarkedEntityDisabled
                | NarrativeKind::MarkedEntityDestroyed
                | NarrativeKind::MarkedEntityEscaped
                | NarrativeKind::MarkedEntityRescued
                | NarrativeKind::MarkedEntityAbandoned
        )
    }
}

/// One field of a narrative event's structured outcome data.
///
/// A tiny closed set rather than a free-form string map: a response index is a
/// number, `mandatory` is a boolean, and flattening either into text would make
/// every consumer re-parse it. Hand-encoded, because `serde_json` is confined
/// to `codec.rs`.
#[derive(Clone, Debug, PartialEq)]
pub enum NarrativeValue {
    /// A String Id, uuid, or other opaque identifier — never localized prose.
    Text(String),
    Int(i64),
    Real(f64),
    Flag(bool),
}

impl NarrativeValue {
    /// Encode as a JSON value. Reals are fixed to 4 decimals so two seeded runs
    /// cannot differ in a trailing digit's formatting.
    pub fn to_json(&self) -> String {
        match self {
            NarrativeValue::Text(s) => format!("{s:?}"),
            NarrativeValue::Int(i) => i.to_string(),
            NarrativeValue::Real(f) => format!("{f:.4}"),
            NarrativeValue::Flag(b) => b.to_string(),
        }
    }
}

/// Where a narrative event came from, when it has a locatable source.
///
/// All three fields are optional and independently meaningful: an Objective is
/// posted by no one in particular (all `None`), a Comms message comes from an
/// entity, and a console-originated beat can name the Station and System that
/// produced it. `Default` is "no source at all", which encodes as `null`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NarrativeActor {
    /// UUID of the entity the event came from.
    pub entity: Option<String>,
    /// Station id (`"tactical"`, `"helm"`, …).
    pub station: Option<String>,
    /// Ship-system id (`"comms"`, `"sensors"`, …).
    pub system: Option<String>,
}

impl NarrativeActor {
    /// An actor that is just an entity uuid — the common case.
    pub fn entity(uuid: impl Into<String>) -> Self {
        Self {
            entity: Some(uuid.into()),
            ..Default::default()
        }
    }

    /// Whether nothing at all is named, in which case the event encodes
    /// `"source": null` rather than an object of three nulls.
    pub fn is_empty(&self) -> bool {
        self.entity.is_none() && self.station.is_none() && self.system.is_none()
    }

    /// Encode as a JSON object, or `null` when [`Self::is_empty`].
    pub fn to_json(&self) -> String {
        if self.is_empty() {
            return "null".to_string();
        }
        format!(
            "{{\"entity\":{},\"station\":{},\"system\":{}}}",
            opt_string(&self.entity),
            opt_string(&self.station),
            opt_string(&self.system),
        )
    }
}

/// One authored moment in the mission timeline.
///
/// The shape is PRD #1337's, field for field: an event kind, an authored
/// semantic identifier or String Id, an optional source Station/System/entity,
/// an optional target, and structured outcome data. The monotonic sequence and
/// the simulation tick/time are NOT here — they are stamped at collection into
/// a [`StampedNarrativeEvent`], so a chokepoint only has to know what it knows.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct NarrativeEvent {
    /// What kind of moment this is.
    pub kind: NarrativeKind,
    /// The authored semantic identifier this event is about: an objective id, a
    /// deadline id, a beat id, a comms thread id, a marked entity's authored
    /// name. Never localized text.
    pub id: String,
    /// Who produced it, when that is knowable.
    pub source: NarrativeActor,
    /// What it was done to — an entity uuid or an authored name, depending on
    /// the kind. `None` for events with no second party.
    pub target: Option<String>,
    /// Structured outcome data, keyed for stable ordering.
    pub detail: BTreeMap<String, NarrativeValue>,
}

impl NarrativeEvent {
    /// A bare event of `kind` about `id`, with no source, target, or detail.
    pub fn new(kind: NarrativeKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            source: NarrativeActor::default(),
            target: None,
            detail: BTreeMap::new(),
        }
    }

    /// Name the source.
    pub fn from_actor(mut self, source: NarrativeActor) -> Self {
        self.source = source;
        self
    }

    /// Name the target.
    pub fn to_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Add one structured outcome field.
    pub fn detail(mut self, key: &str, value: NarrativeValue) -> Self {
        self.detail.insert(key.to_string(), value);
        self
    }

    /// Add one String-Id-shaped outcome field. A convenience over
    /// [`Self::detail`] with [`NarrativeValue::Text`], because most detail is
    /// exactly that.
    pub fn text(self, key: &str, value: impl Into<String>) -> Self {
        self.detail(key, NarrativeValue::Text(value.into()))
    }

    /// Whether this event belongs in the ndjson timeline stream — see
    /// [`NarrativeKind::in_timeline_stream`].
    pub fn in_timeline_stream(&self) -> bool {
        self.kind.in_timeline_stream()
    }

    /// The event's own JSON fields, without the surrounding braces, so a
    /// stamped wrapper can prefix its sequence and timestamps without nesting a
    /// second object.
    fn body_json(&self) -> String {
        let detail = self
            .detail
            .iter()
            .map(|(k, v)| format!("{:?}:{}", k, v.to_json()))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "\"kind\":{:?},\"id\":{:?},\"source\":{},\"target\":{},\"detail\":{{{}}}",
            self.kind.as_str(),
            self.id,
            self.source.to_json(),
            opt_string(&self.target),
            detail,
        )
    }

    /// Encode as a standalone JSON object.
    pub fn to_json(&self) -> String {
        format!("{{{}}}", self.body_json())
    }
}

/// A [`NarrativeEvent`] with its position in the run's timeline.
///
/// `seq` is the monotonic sequence PRD #1337 asks for, assigned once at
/// collection: two events on the same tick still have a total order, and that
/// order is the order the simulation produced them in. `tick` and `sim_t` are
/// the fixed simulation tick and its derived time — never a wall clock.
#[derive(Debug, Clone, PartialEq)]
pub struct StampedNarrativeEvent {
    /// Monotonic run-scoped sequence, starting at 0.
    pub seq: u64,
    /// The `SimTick` this was collected on.
    pub tick: u64,
    /// Simulation seconds elapsed at collection.
    pub sim_t: f64,
    pub event: NarrativeEvent,
}

impl StampedNarrativeEvent {
    /// Encode as one flat JSON object: sequence, timestamps, then the event's
    /// own fields. Flat rather than `{"seq":..,"event":{..}}` so a reader
    /// filtering a timeline never has to reach through a wrapper.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"seq\":{},\"tick\":{},\"sim_t\":{:.4},{}}}",
            self.seq,
            self.tick,
            self.sim_t,
            self.event.body_json()
        )
    }

    /// Encode as one ndjson stream line, in the `{"tick":..,"sim_t":..,"<surface>":{..}}`
    /// envelope every other headless stream record uses.
    pub fn to_stream_json(&self) -> String {
        format!(
            "{{\"tick\":{},\"sim_t\":{:.4},\"narrative\":{{\"seq\":{},{}}}}}",
            self.tick,
            self.sim_t,
            self.seq,
            self.event.body_json()
        )
    }
}

/// The folded mission timeline a run report carries.
///
/// Built by [`fold_narrative`], a pure function of the stamped log — no ECS
/// access at all, so the projection is unit-testable without booting an app,
/// exactly as [`crate::core::balance::aggregate_ledgers`] is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NarrativeTimeline {
    /// Every recorded event, in sequence order.
    pub events: Vec<StampedNarrativeEvent>,
    /// How many of each kind the run produced, keyed by the kind's stable
    /// label. `BTreeMap` so the report is byte-identical across runs.
    pub counts_by_kind: BTreeMap<&'static str, u64>,
}

impl NarrativeTimeline {
    /// Encode as the report's `narrative` object.
    pub fn to_json(&self) -> String {
        let counts = self
            .counts_by_kind
            .iter()
            .map(|(k, v)| format!("{k:?}: {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        let events = self
            .events
            .iter()
            .map(|e| e.to_json())
            .collect::<Vec<_>>()
            .join(",\n      ");
        if self.events.is_empty() {
            return format!("{{\"count\": 0, \"counts_by_kind\": {{{counts}}}, \"events\": []}}");
        }
        format!(
            "{{\n    \"count\": {},\n    \"counts_by_kind\": {{{}}},\n    \"events\": [\n      {}\n    ]\n  }}",
            self.events.len(),
            counts,
            events,
        )
    }
}

/// Fold the stamped narrative log into the report's timeline projection.
///
/// Pure: no world access, no clock read. The events are copied through in the
/// order they were collected (which is already sequence order) and counted by
/// kind, so a reader gets both the story and its shape without re-walking the
/// array.
pub fn fold_narrative(events: &[StampedNarrativeEvent]) -> NarrativeTimeline {
    let mut counts_by_kind: BTreeMap<&'static str, u64> = BTreeMap::new();
    for stamped in events {
        *counts_by_kind
            .entry(stamped.event.kind.as_str())
            .or_insert(0) += 1;
    }
    NarrativeTimeline {
        events: events.to_vec(),
        counts_by_kind,
    }
}

/// Marks an entity as narratively significant (issue #1338).
///
/// Authored, never inferred: a world `[[entity]]` opts in with `narrative =
/// true`, and the payload is that entity's authored `name` — the world's own
/// unique reference id, which is also what triggers, comms and objectives
/// address it by. Without this component an entity produces no narrative event
/// at all, however violently it dies, which is the whole point: the timeline is
/// the story the scenario chose to tell, not a second copy of the damage log.
///
/// Presentation-class state for the #894 digest boundary: the fixed tick never
/// reads it, nothing branches on it, and `sim_digest`/`snapshot` do not walk
/// it. It decides only what the after-action surface SHOWS.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct NarrativeMark(pub String);

/// A narrative moment on its way from the script boundary to the event stream
/// (issue #1338).
///
/// Buffered onto an [`crate::effect_queue::EffectQueue`] by the shared dispatch
/// applier rather than written straight to `Messages<NarrativeEvent>`, for the
/// reason every other #1223 effect is: the applier is lent plain `&mut Vec<_>`
/// sinks and holds no message writers, and the systems that call it are at
/// Bevy's parameter limit.
/// `crate::narrative::emit_authored_and_marked_entity_narrative` turns each
/// request into its event, in queue order.
///
/// Two variants because the queue carries two different KINDS of claim, and
/// only one of them is the author speaking. [`Self::Authored`] is a statement —
/// it becomes its event unconditionally. [`Self::ScriptedRemoval`] is a
/// *report of a mechanical act* that may or may not deserve a death beat, and
/// the emitter decides.
#[derive(Clone, Debug, PartialEq)]
pub enum NarrativeRequest {
    /// A beat or marked-entity outcome the scenario declared outright —
    /// `ctx.effects.narrative_beat(..)` / `ctx.effects.narrative_outcome(..)`.
    Authored {
        /// The kind this request becomes.
        kind: NarrativeKind,
        /// The authored id: a beat id, or a marked entity's authored name.
        id: String,
        /// The entity uuid, when the applier resolved one.
        entity_uuid: Option<String>,
    },
    /// A script removed an entity from the world —
    /// `ctx.effects.destroy_entity(name)`, or its deferred schedule form.
    ///
    /// Queued for EVERY scripted removal, marked or not, because the applier
    /// cannot see a [`NarrativeMark`]: it is lent plain `&mut Vec<_>` sinks and
    /// no component query, so the mark gate lives in the emitter, which already
    /// remembers every marked uuid it has seen. An unmarked removal is dropped
    /// there and produces nothing.
    ///
    /// It exists so a marked hull cannot leave the world silently — see
    /// [`NarrativeKind::MarkedEntityDestroyed`] for the whole contract, and
    /// note that it is deliberately NOT a
    /// [`crate::core::balance::BalanceEvent`]: an authorial removal must not
    /// enter the combat ledger.
    ScriptedRemoval {
        /// The uuid the `DestroyEntity` command named.
        entity_uuid: String,
    },
}

/// `Some("x")` → `"x"`, `None` → `null`. The same escaping trick the run report
/// and [`crate::core::balance`] use for their string fields.
fn opt_string(v: &Option<String>) -> String {
    match v {
        Some(s) => format!("{s:?}"),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The coverage guard: every kind is deliberately classified as timeline or
    /// fold-only, and `KIND_COUNT` matches the enum. Adding a kind without
    /// touching `ALL` fails to compile the array; adding one to `ALL` without
    /// bumping `KIND_COUNT` fails to compile too — this test then forces the
    /// remaining decision, which is what the stream is for.
    #[test]
    fn every_kind_declares_whether_it_reaches_the_timeline() {
        assert_eq!(NarrativeKind::ALL.len(), NarrativeKind::KIND_COUNT);
        let fold_only: Vec<&str> = NarrativeKind::ALL
            .iter()
            .filter(|k| !k.in_timeline_stream())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            fold_only,
            vec!["report_row_updated"],
            "a kind changed its timeline policy — say why in `in_timeline_stream`'s doc"
        );
    }

    /// Labels are the wire vocabulary, so they must be unique and stable.
    #[test]
    fn kind_labels_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for kind in NarrativeKind::ALL {
            assert!(
                seen.insert(kind.as_str()),
                "duplicate narrative kind label {:?}",
                kind.as_str()
            );
        }
        assert_eq!(seen.len(), NarrativeKind::KIND_COUNT);
    }

    /// Only marked-entity outcomes are authorable, and a typo is an error
    /// rather than a silently different beat.
    #[test]
    fn parse_outcome_accepts_only_the_entity_outcomes() {
        assert_eq!(
            NarrativeKind::parse_outcome("Rescued"),
            Ok(NarrativeKind::MarkedEntityRescued)
        );
        assert_eq!(
            NarrativeKind::parse_outcome(" abandoned "),
            Ok(NarrativeKind::MarkedEntityAbandoned)
        );
        // An Objective transition is observed, never declared.
        assert!(NarrativeKind::parse_outcome("completed").is_err());
        assert!(NarrativeKind::parse_outcome("wibble").is_err());
    }

    /// The fates a marked entity can be given, spelled out: these are the kinds
    /// whose presence tells the scripted-removal fallback that the story has
    /// already said what became of this hull. Spawning is not a fate, and
    /// nothing outside the marked-entity family is one — a new kind that should
    /// count has to be added here deliberately.
    #[test]
    fn only_the_marked_entity_fates_count_as_an_outcome() {
        let fates: Vec<&str> = NarrativeKind::ALL
            .iter()
            .filter(|k| k.is_marked_entity_outcome())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            fates,
            vec![
                "marked_entity_disabled",
                "marked_entity_destroyed",
                "marked_entity_escaped",
                "marked_entity_rescued",
                "marked_entity_abandoned",
            ]
        );
        assert!(!NarrativeKind::MarkedEntitySpawned.is_marked_entity_outcome());
        assert!(!NarrativeKind::BeatFired.is_marked_entity_outcome());
    }

    /// The stamped JSON carries the sequence, the fixed tick, the derived time
    /// and the event's own fields — the whole of PRD #1337's per-event shape.
    #[test]
    fn stamped_json_carries_sequence_tick_and_time() {
        let stamped = StampedNarrativeEvent {
            seq: 7,
            tick: 420,
            sim_t: 14.0,
            event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "reach_axiom")
                .text("text", "world.probe.objective.reach_axiom")
                .detail("mandatory", NarrativeValue::Flag(true)),
        };
        let json = stamped.to_json();
        assert!(
            json.starts_with("{\"seq\":7,\"tick\":420,\"sim_t\":14.0000,"),
            "{json}"
        );
        assert!(json.contains("\"kind\":\"objective_posted\""), "{json}");
        assert!(json.contains("\"id\":\"reach_axiom\""), "{json}");
        // The String Id passes through verbatim — never resolved to English.
        assert!(
            json.contains("\"text\":\"world.probe.objective.reach_axiom\""),
            "{json}"
        );
        assert!(json.contains("\"mandatory\":true"), "{json}");
        // No source and no target named.
        assert!(json.contains("\"source\":null"), "{json}");
        assert!(json.contains("\"target\":null"), "{json}");
    }

    /// The ndjson envelope matches every other headless stream record, so a
    /// consumer splitting lines on `tick` sees narrative beats in tick order
    /// beside the balance events.
    #[test]
    fn stream_json_uses_the_shared_envelope() {
        let stamped = StampedNarrativeEvent {
            seq: 0,
            tick: 12,
            sim_t: 0.4,
            event: NarrativeEvent::new(NarrativeKind::BeatFired, "storm_hits"),
        };
        let line = stamped.to_stream_json();
        assert!(
            line.starts_with("{\"tick\":12,\"sim_t\":0.4000,\"narrative\":{\"seq\":0,"),
            "{line}"
        );
    }

    /// A source with an entity/station/system encodes all three, so an event
    /// that came off a console can be attributed to it.
    #[test]
    fn actor_encodes_the_three_source_axes() {
        let actor = NarrativeActor {
            entity: Some("uuid-1".into()),
            station: Some("tactical".into()),
            system: Some("comms".into()),
        };
        assert_eq!(
            actor.to_json(),
            "{\"entity\":\"uuid-1\",\"station\":\"tactical\",\"system\":\"comms\"}"
        );
        assert_eq!(NarrativeActor::default().to_json(), "null");
    }

    /// The fold is pure and order-preserving, and counts every kind it saw.
    #[test]
    fn fold_preserves_order_and_counts_kinds() {
        let events = vec![
            StampedNarrativeEvent {
                seq: 0,
                tick: 1,
                sim_t: 0.0,
                event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "a"),
            },
            StampedNarrativeEvent {
                seq: 1,
                tick: 2,
                sim_t: 0.1,
                event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "b"),
            },
            StampedNarrativeEvent {
                seq: 2,
                tick: 3,
                sim_t: 0.2,
                event: NarrativeEvent::new(NarrativeKind::ObjectiveCompleted, "a"),
            },
        ];
        let timeline = fold_narrative(&events);
        assert_eq!(
            timeline.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(timeline.counts_by_kind.get("objective_posted"), Some(&2));
        assert_eq!(timeline.counts_by_kind.get("objective_completed"), Some(&1));
        assert_eq!(timeline.counts_by_kind.len(), 2);
    }

    /// An empty run still produces valid JSON with an explicit zero, so a
    /// consumer never has to distinguish "absent" from "nothing happened".
    #[test]
    fn empty_timeline_encodes_as_an_explicit_zero() {
        let json = fold_narrative(&[]).to_json();
        assert_eq!(
            json,
            "{\"count\": 0, \"counts_by_kind\": {}, \"events\": []}"
        );
    }
}
