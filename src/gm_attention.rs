//! Peer-local Game Master attention queue (issues #1433, #1434 and #1435, PRD
//! #1419 M4).
//!
//! One advisory projection answers "what is waiting for a Game Master right
//! now". It reads facts the simulation already owns and publishes them onto the
//! page-local `gm_attention` Host Channel. Four producers today:
//!
//! * **Pending Comms** (#1433) — the ordinary Comms inbox and the authored
//!   `[[gm_comms_route]]` table.
//! * **Eligible beats** (#1434) — the ordinary trigger table with its GM event
//!   controls.
//! * **Idle NPCs** (#1435) — an NPC ship that has been given nothing to do for
//!   an authored grace of SIMULATION time. See the idle-advisory section
//!   further down.
//! * **Station health** (#1437) — the public technical health picture
//!   [`crate::gm_health`] takes from the lockstep barrier.
//!
//! # What this is NOT
//!
//! It is not a `ServerMessage`, a mesh frame, a digest fold, a replay record, or
//! a second GM event bus. Nothing here reaches
//! [`crate::gm_action::GmActionJournal`], and nothing here mutates the world:
//! an occurrence appears because a condition holds and disappears because it
//! stopped holding. Reading, filtering, holding and snoozing all happen in the
//! operator's own browser (`gui/gm-attention-panel.js`) and never cross back.
//!
//! Neither is the projection a snapshot field: occurrences are recomputed from
//! live facts on every publish. The one thing that travels is
//! [`GmIdleNpcWatch`], because elapsed idleness is history rather than a
//! repaint, and no restored world can be asked how long a hull has been
//! standing about. It is still folded into no digest.
//!
//! # Occurrence identity
//!
//! An occurrence's id is derived from the durable identity of the thing that is
//! waiting — for pending Comms, the `CommsMessage` id the world minted; for an
//! idle NPC, the hull plus the tick its current idle spell began. That is what
//! makes the two lifecycle rules fall out for free: while the condition holds
//! the row keeps one identity (so a browser can hold a snooze, a focus ring or a
//! reading position against it), and a *recurrence* — a second hail after the
//! first was answered, a second idle spell after an order was withdrawn — is a
//! fresh occurrence that no stale snooze can hide.
//!
//! An eligible beat (issue #1434) has no per-occurrence identity of its own to
//! borrow — a repeatable event is the SAME authored trigger every time it comes
//! back — so the same two rules are made true by hand: the id is the
//! layer-qualified event id plus an occurrence ordinal this peer increments
//! each time the beat re-enters eligibility. The row is stable for as long as
//! the beat stays ready, and a beat that fires and becomes ready again is a
//! genuinely new occurrence, not the old one returning with a stale snooze on
//! it. The ordinal is private presentation bookkeeping and reaches nothing
//! authoritative.
//!
//! # Eligibility is READ, never re-derived
//!
//! "Eligible" means exactly one thing: a Fire of this beat would land right
//! now. That question is answered by
//! [`crate::world::content::manual_fire_would_land`] — literally the predicate
//! [`crate::world::content::fire_manual_trigger`] itself applies, extracted so
//! that a reader and the writer cannot drift — plus the GM control state the
//! mission panel already publishes. No trigger condition is matched a second
//! time, no Rhai runs, no handler is dispatched and no latch, cooldown clock or
//! `seen_destroyed` set moves. Inspecting the queue must never be a way of
//! advancing the world.
//!
//! The lifecycle states the existing control contract already defines are what
//! withhold or resolve a row: a spent one-shot (completed), an event a GM has
//! PAUSED, an armed Fire (the GM already acted, the handler has not run yet),
//! an armed Skip (the GM has decided to spend the next occurrence quietly), a
//! false `when` predicate or an unelapsed cooldown. None of them is a second
//! rule invented here; every one is read from the state the ordinary evaluator
//! and the ordinary GM action reducer maintain.
//!
//! # Bands
//!
//! Three bands, `Urgent`/`Attention`/`Background`. Pending Comms and eligible
//! beats both default to `Attention`, idle NPCs to `Background`; a scenario
//! author may say otherwise on the route the sender speaks through
//! ([`GmCommsRoute::attention_band`]), on the beat's own control set
//! ([`GmEventControls::attention_band`](crate::world::config::GmEventControls::attention_band)),
//! or, for the idle advisory, in the `[gm_attention]` table
//! ([`GmAttentionSettings`]). The band is GM-facing triage only: it does not
//! touch `CommsPriority`, delivery, routing, when a beat fires, or anything a
//! crew console renders.
//!
//! Station health (issue #1437) is the one producer with no authored band at
//! all: a Station or fleet hull whose human has dropped is always `Urgent`,
//! decided by [`crate::gm_health`], and there is no `[[gm_*]]` key anywhere
//! that can lower it, rename it or turn it off. That asymmetry is PRD #1419's —
//! an author tunes their own advisory items, while the technical treatment is
//! system-defined — and it is why the guarantee that a Game Master cannot hide
//! a connection failure from themselves is held by the separate unfilterable
//! banner region beside the list (`#gm-attention-banners`), not by the queue
//! row itself; see the banner seam in `gui/gm-attention-panel.js`.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::comms::server::CommsInboxRes;
use crate::console_bridge::GmAttentionChanged;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::gm_health::{GmHealthAlert, GmHealthWatch};
use crate::gm_projection::GmEntityReference;
use crate::world::config::WorldConfig;

/// Presentation bound on one published queue. A GM cannot read past this many
/// rows in a live session, and an unbounded projection is an unbounded page.
pub const MAX_GM_ATTENTION_OCCURRENCES: usize = 128;

/// The complete authored band vocabulary, in queue order.
///
/// Deliberately closed: an author names one of these three or the world fails
/// to load. There is no numerical score to reverse-engineer and no fourth band
/// a mod pack can invent.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmAttentionBand {
    Urgent,
    Attention,
    Background,
}

impl GmAttentionBand {
    /// The authored spelling, which is also the wire spelling.
    pub fn as_authored(self) -> &'static str {
        match self {
            Self::Urgent => "urgent",
            Self::Attention => "attention",
            Self::Background => "background",
        }
    }

    /// Parse one authored override. Exact, lower-case, no aliases: a world that
    /// says `Urgent` or `critical` is a world whose author believed something
    /// this build does not do, and guessing on their behalf is how a scenario
    /// ships with a priority nobody chose.
    pub fn from_authored(value: &str) -> Option<Self> {
        match value {
            "urgent" => Some(Self::Urgent),
            "attention" => Some(Self::Attention),
            "background" => Some(Self::Background),
            _ => None,
        }
    }

    /// Every band an author may write, for an error message that tells them
    /// what to write instead.
    pub fn authored_vocabulary() -> String {
        [Self::Urgent, Self::Attention, Self::Background]
            .iter()
            .map(|band| format!("'{}'", band.as_authored()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Why a row is in the queue.
///
/// `StationHealth` (issue #1437) is the one category whose band is system-fixed
/// at [`GmAttentionBand::Urgent`] and whose rows no authored table can invent,
/// re-band or suppress. It is deliberately still a category, and therefore
/// still filterable *in the list*: the guarantee that a Game Master cannot hide
/// a connection failure from themselves is held by the unfilterable banner
/// region beside the list (`#gm-attention-banners`), not by making one row type
/// un-narrowable in a triage tool.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmAttentionCategory {
    PendingComms,
    /// An authored beat a Game Master could release right now (issue #1434).
    EligibleBeat,
    /// An NPC ship that has been given nothing to do (issue #1435).
    IdleNpc,
    /// A Station or fleet hull whose human is gone (issue #1437). Always
    /// Urgent, always system-defined.
    StationHealth,
}

/// The short human reason, as a String Table id plus its runtime parameters.
///
/// Never prose: the projection has no locale, and the values are themselves
/// authored ids (entity names) the page resolves at its own presentation
/// boundary.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionReason {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

/// Where activating a row takes the operator. Every field names something that
/// already exists on the GM desk — an authored Comms route, a ship, a sender —
/// so "open" is a navigation, never a new action surface.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionTarget {
    /// The authored `[[gm_comms_route]]` id that already speaks as this sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// The ship whose console is waiting. `None` for legacy fleet-wide traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship: Option<GmEntityReference>,
    /// The fictional speaker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<GmEntityReference>,
    /// The conversation thread, so a later surface can open the exact exchange.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
    /// The authored beat this row is about, and which of its levers the mission
    /// panel is already offering (issue #1434).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<GmAttentionEventTarget>,
}

/// The GM-operable event one eligible-beat row points at (issue #1434).
///
/// Every field is a copy of something [`crate::gm_event::GmMissionEvent`]
/// already publishes on `gm_mission`. It is repeated here so a row can say
/// which controls exist without the panel having to join two projections —
/// never so that the attention queue can offer a control of its own. Opening a
/// row is navigation to the mission panel's existing row; the Fire, Pause and
/// Skip a GM then presses are that panel's buttons, taking that panel's
/// admission check and the apply-tick revalidation
/// ([`crate::gm_event::fireable_index`] and its twins) with them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionEventTarget {
    /// The layer-qualified id a `FireGmEvent` action names.
    pub id: String,
    /// String Table id for the authored label.
    pub label: String,
    /// The levers this beat DECLARES, exactly as the mission panel lists them.
    pub fire: bool,
    pub pause: bool,
    pub skip: bool,
}

/// One thing waiting for a Game Master.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionOccurrence {
    /// Stable while the condition holds; a recurrence mints a different one.
    pub id: String,
    pub category: GmAttentionCategory,
    pub band: GmAttentionBand,
    /// The simulation tick this peer first observed the condition.
    pub first_seen_tick: u64,
    /// Real milliseconds elapsed since that first observation, sampled as this
    /// payload was built. Real time, not simulation time: it keeps running
    /// while the world is paused, which is exactly the basis a GM ages a queue
    /// on and the basis the personal snooze expires on.
    pub age_ms: u64,
    pub reason: GmAttentionReason,
    #[serde(default)]
    pub target: GmAttentionTarget,
}

/// The absolute queue, ordered band-agnostically by age then id. The page
/// groups it into bands; publishing one ordered list keeps the tie-break in one
/// place rather than three.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionProjection {
    pub occurrences: Vec<GmAttentionOccurrence>,
}

/// When this peer first saw one occurrence, in both bases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FirstSeen {
    tick: u64,
    real_ms: u64,
}

/// This peer's own attention bookkeeping. `Presentation`: derived entirely from
/// the Comms inbox, the authored route table, the ordinary trigger table and
/// the health projection, all already classified, and read by nothing
/// authoritative.
#[derive(Resource, Default)]
pub struct GmAttentionState {
    first_seen: BTreeMap<String, FirstSeen>,
    /// Which occurrence of each authored beat is on the desk (issue #1434).
    beats: BTreeMap<String, BeatOccurrence>,
    last: Option<GmAttentionProjection>,
}

/// The authored beat's own lifecycle stamp, read straight off the trigger state
/// the ordinary evaluator maintains (issue #1434).
///
/// It is what makes "this is a DIFFERENT occurrence" a fact about the world
/// rather than about how often this peer happened to look: the stamp moves
/// exactly when the beat fires (and moves back when a scenario's
/// `reset_trigger` re-arms it), whether or not any frame in between caught the
/// beat mid-flight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BeatStamp {
    fired: bool,
    last_fired_elapsed: Option<f32>,
}

/// One beat's occurrence bookkeeping on this peer.
#[derive(Clone, Copy, Debug, Default)]
struct BeatOccurrence {
    /// Incremented once per occurrence, so a snooze, a focus ring or a reading
    /// position taken against a beat that has since fired cannot be inherited
    /// by the next time it comes round.
    ordinal: u64,
    /// The stamp the CURRENT occurrence began on.
    stamp: BeatStamp,
    /// Was the beat eligible at the last publish? A beat that goes ineligible
    /// and comes back — an authored gate closing and reopening — ended one
    /// occurrence and began another just as surely as a firing does.
    eligible: bool,
}

impl GmAttentionState {
    /// The projection this peer last published, for tests and for a late
    /// mount that needs the current queue without waiting for a change.
    pub fn last(&self) -> Option<&GmAttentionProjection> {
        self.last.as_ref()
    }
}

/// Is this inbox message actually waiting on a human decision?
///
/// Three conditions, and each one is a lifecycle exit:
/// - `is_orphaned` — the sender left; the conversation was withdrawn.
/// - `selected_response` — somebody answered; it resolved.
/// - no `responses` — nothing to answer yet. An empty response list is the
///   `…` follow-up placeholder while the other side is still speaking, not a
///   demand on the crew, and a literal GM transmission never grows one.
///
/// A message leaving the inbox entirely (a Comms officer's `ClearComms`, a
/// world-layer unload) removes it from the walk below and so also removes the
/// occurrence, without needing a fourth rule here.
fn pending(message: &crate::core::messages::CommsMessage) -> bool {
    !message.is_orphaned && message.selected_response.is_none() && !message.responses.is_empty()
}

/// The authored route this sender already speaks through, if any.
///
/// First authored match wins, so a world that lists one speaker on several
/// routes gets a stable answer rather than an order-of-iteration one. The
/// world's own author order is the tie-break, which is the one a scenario
/// writer can see and change.
fn route_for<'a>(
    world: Option<&'a WorldConfig>,
    sender_name: Option<&str>,
) -> Option<&'a crate::gm_comms::GmCommsRoute> {
    let name = sender_name?;
    world?
        .gm_comms_routes
        .iter()
        .find(|route| route.senders.iter().any(|s| s == name))
}

/// Band for one pending Comms occurrence: the author's choice on the route that
/// speaks as this sender, else the system default.
fn band_for(route: Option<&crate::gm_comms::GmCommsRoute>) -> GmAttentionBand {
    route
        .and_then(|route| route.attention_band.as_deref())
        .and_then(GmAttentionBand::from_authored)
        .unwrap_or(GmAttentionBand::Attention)
}

/// The String Table id a pending Comms row addressed at a single ship explains
/// itself with. Takes `{sender}` and `{ship}`.
pub const PENDING_COMMS_REASON: &str = "server.gm.attention.reason.pending_comms";

/// The String Table id a pending Comms row with no single recipient ship
/// explains itself with. Takes `{sender}` alone.
///
/// Most real traffic lands here: only a GM's own `TransmitComms` addresses a
/// dialogue at named hulls, while a world-authored `open_comms` and the crew's
/// own hails carry no `recipient_ship` at all. Reusing the addressed sentence
/// for those would interpolate an empty `{ship}` and put "Cordon Control is
/// waiting on ." on the GM desk, so the fleet-wide case gets its own sentence
/// and its own parameter set rather than a blank.
pub const PENDING_COMMS_FLEET_REASON: &str = "server.gm.attention.reason.pending_comms_fleet";

/// The String Table id an eligible MANUAL beat explains itself with. Takes
/// `{beat}`, the authored label id.
///
/// Its own sentence because a `gm_event` beat has no automatic condition at
/// all: nothing but a Game Master can ever cause it, so "ready" means "nobody
/// else is going to do this".
pub const ELIGIBLE_BEAT_MANUAL_REASON: &str = "server.gm.attention.reason.eligible_beat_manual";

/// The String Table id an eligible beat that ALSO has an automatic condition
/// explains itself with. Takes `{beat}`.
///
/// The distinction is the one a facilitator actually acts on: this moment will
/// arrive on its own if left alone, and Fire brings it forward. Saying "yours
/// to start" about a wave that is already on a timer would be false.
pub const ELIGIBLE_BEAT_REASON: &str = "server.gm.attention.reason.eligible_beat";

/// The `event:` prefix every eligible-beat occurrence id carries, so a page can
/// tell one row's family from another's without parsing the category twice.
pub const ELIGIBLE_BEAT_ID_PREFIX: &str = "event:";

fn reference(uuid: &str, names: &BTreeMap<String, String>) -> GmEntityReference {
    GmEntityReference {
        entity_id: uuid.to_string(),
        name: names.get(uuid).cloned().unwrap_or_else(|| uuid.to_string()),
    }
}

/// One occurrence before this peer's own first-seen bookkeeping is attached.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingRow {
    id: String,
    category: GmAttentionCategory,
    band: GmAttentionBand,
    reason: GmAttentionReason,
    target: GmAttentionTarget,
}

/// Build the queue from the live inbox. Pure over its inputs so the ordering
/// and lifecycle rules are testable without a `World`.
fn collect(
    world: Option<&WorldConfig>,
    inbox: &crate::console::comms::inbox::CommsInbox,
    names: &BTreeMap<String, String>,
) -> Vec<PendingRow> {
    let mut rows = Vec::new();
    for message in inbox.iter() {
        if !pending(message) {
            continue;
        }
        let sender_name = names.get(&message.sender_uuid).map(String::as_str);
        let route = route_for(world, sender_name);
        let sender = reference(&message.sender_uuid, names);
        let ship = message
            .recipient_ship
            .as_ref()
            .map(|ship| reference(&ship.0, names));
        let mut params = BTreeMap::new();
        params.insert("sender".to_string(), sender.name.clone());
        // Two sentences, not one sentence with an optional blank: a hail with
        // no recipient ship is waiting on the fleet, and saying so is not the
        // same as saying it is waiting on nobody.
        let reason_id = match ship.as_ref() {
            Some(ship) => {
                params.insert("ship".to_string(), ship.name.clone());
                PENDING_COMMS_REASON
            }
            None => PENDING_COMMS_FLEET_REASON,
        };
        rows.push(PendingRow {
            id: format!("comms:{}", message.id),
            category: GmAttentionCategory::PendingComms,
            band: band_for(route),
            reason: GmAttentionReason {
                id: reason_id.to_string(),
                params,
            },
            target: GmAttentionTarget {
                route: route.map(|route| route.id.clone()),
                ship,
                sender: Some(sender),
                conversation: (!message.thread_id.is_empty()).then(|| message.thread_id.clone()),
                event: None,
            },
        });
    }
    rows
}

/// Build the eligible-beat rows from the live trigger table (issue #1434).
///
/// The whole of the read is here, and every gate is somebody else's rule:
///
/// * no `gm_controls` — the trigger is not GM-addressable at all, which is the
///   default for every trigger every shipped world already authors;
/// * no Fire lever — a beat a GM cannot cause is not a beat waiting on one.
///   Pause and Skip are levers ON an occurrence the world will produce, not
///   ways to produce it, so they do not by themselves put a row on the desk;
/// * PAUSED — the GM has already said "not now" about this exact event. (Fire
///   still works while paused, deliberately; the queue withholds the row
///   anyway, because a standing decision is not something to keep asking
///   about.);
/// * an armed Fire — the GM acted and the handler has not run yet;
/// * an armed Skip — the GM decided to spend the next occurrence quietly;
/// * anything [`manual_fire_would_land`](crate::world::content::manual_fire_would_land)
///   declines: a spent one-shot (completed), a `when` predicate reading false,
///   a cooldown that has not elapsed.
///
/// The id it returns is the beat's stable BASE key, paired with the beat's
/// lifecycle stamp. The occurrence ordinal is attached by the publisher, which
/// is the only place that knows what the previous occurrence was.
fn collect_beats(
    runtime: &crate::world::server::WorldContentRuntime,
    layer_map: Option<&crate::world::server::WorldLayerMap>,
    current_elapsed: f32,
) -> Vec<(PendingRow, BeatStamp)> {
    let mut rows = Vec::new();
    for state in runtime.triggers.iter() {
        let Some(controls) = state.trigger.gm_controls.as_ref() else {
            continue;
        };
        if !controls.declares_fire() {
            continue;
        }
        let id = crate::gm_event::qualified_event_id(state.origin_layer.as_deref(), &controls.id);
        if runtime.paused_gm_events.contains(&id)
            || runtime.pending_gm_event_fires.contains(&id)
            || runtime.pending_gm_event_skips.contains(&id)
        {
            continue;
        }
        let chain = crate::world::server::layered_flag_chain(
            state.origin_layer.as_deref(),
            &runtime.flags,
            layer_map,
        );
        if !crate::world::content::manual_fire_would_land(state, &chain, current_elapsed) {
            continue;
        }
        // A manual beat is the Game Master's alone; anything else will also
        // arrive on its own, and Fire only brings it forward.
        let manual = matches!(
            state.trigger.condition,
            crate::world::config::TriggerCondition::Manual
        );
        rows.push((
            PendingRow {
                id: format!("{ELIGIBLE_BEAT_ID_PREFIX}{id}"),
                category: GmAttentionCategory::EligibleBeat,
                band: controls
                    .attention_band
                    .as_deref()
                    .and_then(GmAttentionBand::from_authored)
                    .unwrap_or(GmAttentionBand::Attention),
                reason: GmAttentionReason {
                    id: if manual {
                        ELIGIBLE_BEAT_MANUAL_REASON.to_string()
                    } else {
                        ELIGIBLE_BEAT_REASON.to_string()
                    },
                    // The authored label id, not English: the page resolves it
                    // through the same String Table the mission panel renders
                    // the beat's own row with, so both name it identically.
                    params: BTreeMap::from([("beat".to_string(), controls.label.clone())]),
                },
                target: GmAttentionTarget {
                    event: Some(GmAttentionEventTarget {
                        id,
                        label: controls.label.clone(),
                        fire: controls.fire,
                        pause: controls.pause,
                        skip: controls.skip,
                    }),
                    ..Default::default()
                },
            },
            BeatStamp {
                fired: state.fired,
                last_fired_elapsed: state.last_fired_elapsed,
            },
        ));
    }
    rows
}

// ── Idle NPC advisory (issue #1435) ───────────────────────────────────────────

/// The grace an NPC ship must spend with nothing to do before the queue
/// mentions it, in SIMULATION seconds.
pub const DEFAULT_IDLE_NPC_GRACE_SECS: f32 = 30.0;

/// The String Table id an idle-NPC row explains itself with. Takes `{ship}` and
/// `{idle}` — the hull, and how long it has been without work — so the sentence
/// names both the observed condition and its age.
pub const IDLE_NPC_REASON: &str = "server.gm.attention.reason.idle_npc";

/// The authored `[gm_attention]` table.
///
/// Every knob here is scoped to the GM's own advisory queue: none of it changes
/// what a ship flies, what a crew console renders, or what the digest folds. It
/// is a separate table from `[[gm_comms_route]].attention_band` because that
/// override belongs to one route, while these belong to the world.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GmAttentionSettings {
    /// How long an NPC ship must be idle, in simulation seconds, before it
    /// reaches the queue. Positive and finite; validated at world load.
    #[serde(default = "default_idle_npc_grace_secs")]
    pub idle_npc_grace_secs: f32,
    /// The band an idle-NPC row lands in, from the same closed three-word
    /// vocabulary [`GmAttentionBand`] parses. `None` keeps the system default,
    /// [`GmAttentionBand::Background`] — an idle hull is a thing to notice, not
    /// a thing to drop a conversation for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_npc_band: Option<String>,
    /// Silence THIS advisory and nothing else. A scenario whose NPCs are meant
    /// to be parked — a dockyard, a field of derelicts — does not want thirty
    /// rows saying so, and it must not have to give up the pending-Comms queue
    /// to say that.
    #[serde(default)]
    pub idle_npc_disabled: bool,
}

fn default_idle_npc_grace_secs() -> f32 {
    DEFAULT_IDLE_NPC_GRACE_SECS
}

impl Default for GmAttentionSettings {
    fn default() -> Self {
        Self {
            idle_npc_grace_secs: DEFAULT_IDLE_NPC_GRACE_SECS,
            idle_npc_band: None,
            idle_npc_disabled: false,
        }
    }
}

impl GmAttentionSettings {
    /// Refuse an unusable authored value at world load, naming the section and
    /// the key so the author is told what to write instead.
    ///
    /// A non-positive or non-finite grace is not a slow advisory, it is an
    /// advisory with no boundary at all: zero fires on the first idle step,
    /// negative fires before the ship has done anything, and `nan` never fires
    /// while looking exactly like a setting that should. Guessing on the
    /// author's behalf is how a scenario ships with a threshold nobody chose —
    /// the same argument the band vocabulary is closed for.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.idle_npc_grace_secs.is_finite() && self.idle_npc_grace_secs > 0.0) {
            return Err(format!(
                "[gm_attention] idle_npc_grace_secs = {} must be a positive, finite number of \
                 simulation seconds",
                self.idle_npc_grace_secs
            ));
        }
        if let Some(band) = &self.idle_npc_band {
            if GmAttentionBand::from_authored(band).is_none() {
                return Err(format!(
                    "[gm_attention] declares idle_npc_band '{band}'; the GM attention bands are {}",
                    GmAttentionBand::authored_vocabulary()
                ));
            }
        }
        Ok(())
    }

    /// The band an idle-NPC row lands in.
    pub fn idle_npc_band(&self) -> GmAttentionBand {
        self.idle_npc_band
            .as_deref()
            .and_then(GmAttentionBand::from_authored)
            .unwrap_or(GmAttentionBand::Background)
    }

    /// The authored grace as an exact whole number of simulation ticks at `hz`.
    ///
    /// Rounded to the nearest tick and floored at one: a grace shorter than a
    /// tick is one the fixed loop cannot express, and answering "zero ticks"
    /// would put every NPC in the queue on the step it stopped working. The
    /// rounding is IEEE-deterministic, so every peer turns the same authored
    /// seconds into the same tick count.
    pub fn idle_grace_ticks(&self, hz: f32) -> u64 {
        let hz = f64::from(hz);
        let secs = f64::from(self.idle_npc_grace_secs);
        if !(hz.is_finite() && hz > 0.0 && secs.is_finite() && secs > 0.0) {
            return u64::MAX;
        }
        let ticks = (hz * secs).round();
        if !ticks.is_finite() || ticks >= u64::MAX as f64 {
            return u64::MAX;
        }
        (ticks as u64).max(1)
    }
}

/// One NPC ship's current idle spell, in the only clock that can answer the
/// question honestly: fixed simulation steps.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmIdleSpell {
    /// The simulation tick the spell began. Identity only — the row's id is
    /// built from it, so a LATER idle spell on the same hull is a different
    /// occurrence exactly as a second hail is a different pending-Comms
    /// occurrence, and no snooze taken against the first can hide it.
    pub started_tick: u64,
    /// Fixed steps observed idle since `started_tick`.
    ///
    /// COUNTED, never differenced against the live [`crate::sim_tick::SimTick`].
    /// A difference would be wrong twice over: sampled outside the fixed loop it
    /// would advance across a pause, and it would take its answer from whichever
    /// frame happened to read the counter. Counting one per observed step makes
    /// a paused world contribute exactly nothing — a stall withholds the tick,
    /// so this system does not run — and makes the boundary exact rather than
    /// frame-paced.
    pub ticks: u64,
}

/// How long each live NPC ship has had nothing to do.
///
/// `Presentation`: no fixed-tick system reads it, nothing here changes what a
/// ship flies, and it is not folded into the authoritative digest. It IS
/// captured in the snapshot, and that is the one place it differs from an
/// ordinary repaint cache: elapsed history cannot be recomputed from a restored
/// world. A resume that dropped it would silently forgive a hull that had been
/// idle for twenty-nine seconds when the save was taken, and the advisory would
/// become a function of when somebody happened to save rather than of what the
/// NPC was doing.
#[derive(Resource, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmIdleNpcWatch {
    /// Entity UUID → its current spell. A ship with an order, with an Objective
    /// of its own, or with no existence at all is simply absent.
    spells: BTreeMap<String, GmIdleSpell>,
}

impl GmIdleNpcWatch {
    /// This ship's current idle spell, if it is having one.
    pub fn spell(&self, ship: &str) -> Option<GmIdleSpell> {
        self.spells.get(ship).copied()
    }

    /// Every ship currently mid-spell, in stable identity order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &GmIdleSpell)> {
        self.spells.iter()
    }

    /// How many ships are mid-spell — NOT how many are in the queue: a spell
    /// under the authored grace is real and not yet worth saying.
    pub fn len(&self) -> usize {
        self.spells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }
}

/// Is this ship flying nothing?
///
/// "No valid order" is read from the same pool the helm and the weapons act on
/// — the ship's own `ViewscreenBlackboard::scored_objectives`, through the same
/// `score > 0` and "not `AiDirective::None`" filter
/// [`crate::ai::decision_trace::top_directive`] applies. So a standing Patrol, a
/// Reach that holds station, an Escort or any other authored directive that is
/// currently scoring IS an order and this returns `false`; a doctrine that has
/// gated itself to zero — or a hull with no `[behaviour]` block at all — has
/// nothing to do and returns `true`.
///
/// Movement and gunnery are deliberately not consulted. A ship coasting on last
/// tick's velocity is not busy, and a ship shooting because something shot at it
/// has still been given no orders; both are conditions this advisory exists to
/// surface rather than to hide.
fn without_orders(blackboards: &crate::server_app::ShipSystemBlackboards) -> bool {
    let scored = match blackboards
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => {
            bb.scored_objectives.as_slice()
        }
        // No Viewscreen entry at all: `aggregate_doctrine_blackboards` writes
        // one for every `BehaviourSection` hull, so a ship without one authored
        // no doctrine and is idle by construction.
        _ => &[],
    };
    crate::ai::decision_trace::top_directive(scored).is_none()
}

/// NPC ships: a hull nobody's crew is aboard.
///
/// `FleetSlotOf` rides every fleet ship on EVERY host — the marker deliberately
/// chosen so "a peer's player ship" is told apart by a component rather than by
/// absence — so this classification is identical on every peer, unlike anything
/// gated on `LocalShip`. A `StaticPointDefence` turret is excluded because it is
/// a structure, not a ship with orders to be given. It is the same NPC/player
/// split `gm_projection`'s `GmEntityKind` draws.
type IdleNpcQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        &'static crate::server_app::ShipSystemBlackboards,
    ),
    (
        With<crate::server_app::Ship>,
        Without<crate::lockstep::FleetSlotOf>,
        Without<crate::entities::spawner::StaticPointDefence>,
    ),
>;

/// Advance every idle NPC's stopwatch by exactly one simulation step.
///
/// Registered in `FixedLast`, `.before(advance_sim_tick)`, so `started_tick` is
/// the index of the step that observed the spell begin, and so it runs once per
/// fixed step rather than once per rendered frame. Pause needs no special case:
/// a paused world withholds the tick entirely (`SimulationPaused` and the
/// lockstep stall both starve `Time<Virtual>`, which starves the fixed
/// accumulator), and a step that never starts cannot count.
pub fn observe_idle_npcs(
    tick: Res<crate::sim_tick::SimTick>,
    objectives: Option<Res<crate::world::server::ObjectiveManagerRes>>,
    ships: IdleNpcQuery,
    mut watch: ResMut<GmIdleNpcWatch>,
) {
    let mut live: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (uuid, blackboards) in &ships {
        live.insert(uuid.0.clone());
        // Two ways to have been given something to do, and either is enough:
        // an Objective addressed to this hull, or a doctrine directive that is
        // currently scoring.
        let ordered = objectives
            .as_deref()
            .is_some_and(|manager| manager.0.has_active_for_ship(&uuid.0))
            || !without_orders(blackboards);
        if ordered {
            // An order or an Objective ends the spell outright. The next spell
            // starts from zero at a new `started_tick`, so it is a new
            // occurrence rather than a continuation of the resolved row.
            watch.spells.remove(&uuid.0);
            continue;
        }
        let spell = watch.spells.entry(uuid.0.clone()).or_insert(GmIdleSpell {
            started_tick: tick.0,
            ticks: 0,
        });
        spell.ticks = spell.ticks.saturating_add(1);
    }
    // A ship that left the world — destroyed, despawned, warped out, unloaded
    // with its layer — takes its spell with it, so a hull recreated under the
    // same name is watched from scratch rather than inheriting a dead one's wait.
    watch.spells.retain(|id, _| live.contains(id));
}

/// `m:ss` in SIMULATION time, the same shape the page reads a real-time wait in.
///
/// Formatted here rather than on the page because this is a projection
/// parameter, not a rendered age: the panel's own clock is real milliseconds
/// (that is what a facilitator's patience runs on), while what an idle row has
/// to report is how much of the WORLD's time the hull spent doing nothing.
fn format_sim_clock(ticks: u64, hz: f32) -> String {
    let hz = f64::from(hz);
    let seconds = if hz.is_finite() && hz > 0.0 {
        (ticks as f64 / hz).floor().max(0.0) as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The idle-NPC rows of the queue, as a pure function of the authored settings
/// and this peer's stopwatch. Pure over its inputs for [`collect`]'s reason.
fn idle_rows(
    settings: &GmAttentionSettings,
    hz: f32,
    watch: &GmIdleNpcWatch,
    names: &BTreeMap<String, String>,
) -> Vec<PendingRow> {
    if settings.idle_npc_disabled {
        return Vec::new();
    }
    let grace = settings.idle_grace_ticks(hz);
    let band = settings.idle_npc_band();
    watch
        .spells
        .iter()
        .filter(|(_, spell)| spell.ticks >= grace)
        .map(|(uuid, spell)| {
            let ship = reference(uuid, names);
            let mut params = BTreeMap::new();
            params.insert("ship".to_string(), ship.name.clone());
            params.insert("idle".to_string(), format_sim_clock(spell.ticks, hz));
            PendingRow {
                // The spell's own start tick is part of the identity, so a hull
                // given an order and later falling idle again produces a fresh
                // occurrence rather than reviving the resolved one.
                id: format!("idle:{}:{}", uuid, spell.started_tick),
                category: GmAttentionCategory::IdleNpc,
                band,
                reason: GmAttentionReason {
                    id: IDLE_NPC_REASON.to_string(),
                    params,
                },
                // Only the ship. Activating the row selects that hull on the map
                // the desk already draws, which is what opens its inspector and
                // the actions that hull actually allows — no order is chosen,
                // offered or issued on the operator's behalf.
                target: GmAttentionTarget {
                    route: None,
                    ship: Some(ship),
                    sender: None,
                    conversation: None,
                    event: None,
                },
            }
        })
        .collect()
}

// ── Station health advisory (issue #1437) ─────────────────────────────────────

/// Turn the Station-scoped half of the technical health projection (issue
/// #1437) into queue rows.
///
/// The band is not a parameter and not authored: a Station whose human has
/// dropped is Urgent, decided here, and there is no `[[gm_*]]` key anywhere
/// that can lower it, rename it or turn it off. That is the difference between
/// the advisory items an author tunes and the technical treatment PRD #1419
/// calls system-defined.
fn health_rows(alerts: &[GmHealthAlert]) -> Vec<PendingRow> {
    alerts
        .iter()
        .filter(|alert| alert.kind.is_station_attention())
        .map(|alert| PendingRow {
            id: format!("health:{}", alert.id),
            category: GmAttentionCategory::StationHealth,
            band: GmAttentionBand::Urgent,
            reason: GmAttentionReason {
                id: alert.reason.id.clone(),
                params: alert.reason.params.clone(),
            },
            target: GmAttentionTarget {
                route: None,
                ship: alert.ship.clone(),
                sender: None,
                conversation: None,
                event: None,
            },
        })
        .collect()
}

/// Publish the absolute attention queue onto the page-local Host Channel when
/// it changes. Never emits an unchanged payload: a GM desk that repainted a
/// held list sixty times a second would defeat the reading stability this whole
/// feature exists for.
#[allow(clippy::too_many_arguments)]
pub fn publish_attention_projection(
    world: Option<Res<WorldConfig>>,
    inbox: Option<Res<CommsInboxRes>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    entities: Query<(&EntityUuid, Option<&EntityName>)>,
    tick: Res<crate::sim_tick::SimTick>,
    real: Option<Res<Time<Real>>>,
    sim_time: Option<Res<Time>>,
    idle: Option<Res<GmIdleNpcWatch>>,
    health: Option<Res<GmHealthWatch>>,
    mut state: ResMut<GmAttentionState>,
    mut writer: MessageWriter<GmAttentionChanged>,
) {
    let names: BTreeMap<String, String> = entities
        .iter()
        .map(|(uuid, name)| {
            (
                uuid.0.clone(),
                name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
            )
        })
        .collect();
    let now_ms = real.map_or(0, |real| {
        real.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    });
    let mut rows = collect(
        world.as_deref(),
        inbox.as_deref().map_or(&EMPTY_INBOX, |inbox| &inbox.0),
        &names,
    );
    if let Some(runtime) = runtime.as_deref() {
        // The same world-elapsed clock `collect_world_events` stamps its
        // `TimerElapsed` with, so the cooldown gate reads eligibility off the
        // clock the evaluator itself will use, not a second one. No anchor
        // means the mission clock has not started, and a cooldown measured
        // from zero is the same answer the pipeline would give.
        let elapsed = sim_time
            .as_deref()
            .zip(runtime.mission_clock_anchor_secs)
            .map_or(0.0, |(time, anchor)| {
                (time.elapsed_secs() - anchor).max(0.0)
            });
        let beats = collect_beats(runtime, layers.as_deref(), elapsed);
        // A new occurrence begins when the previous one ENDED — the beat fired
        // (its stamp moved), or it stopped being eligible and came back. A beat
        // that is simply still ready keeps the id, and therefore the snooze,
        // the focus ring and the reading position, it already had.
        let mut eligible_now = std::collections::BTreeSet::new();
        for (mut row, stamp) in beats {
            let base = std::mem::take(&mut row.id);
            let entry = state.beats.entry(base.clone()).or_default();
            if !entry.eligible || entry.stamp != stamp {
                entry.ordinal += 1;
                entry.stamp = stamp;
            }
            row.id = format!("{base}#{}", entry.ordinal);
            eligible_now.insert(base);
            rows.push(row);
        }
        // Bookkeeping is kept for every beat the LIVE table can still answer
        // to, not only the eligible ones: a beat that fires and comes back is
        // the case the ordinal exists for, and it is ineligible in between. A
        // layer unload takes its beats' bookkeeping with it, exactly as it
        // takes their arms.
        let addressable: std::collections::BTreeSet<String> =
            crate::gm_event::live_event_ids(&runtime.triggers)
                .into_iter()
                .map(|id| format!("{ELIGIBLE_BEAT_ID_PREFIX}{id}"))
                .collect();
        state.beats.retain(|base, _| addressable.contains(base));
        for (base, entry) in state.beats.iter_mut() {
            entry.eligible = eligible_now.contains(base);
        }
    }
    // The idle-NPC producer (issue #1435). It reads its own authored settings
    // and its own stopwatch, and it joins the same one ordered queue rather than
    // a parallel list, so a GM reads one screen and one age order.
    if let Some(watch) = idle.as_deref() {
        let settings = world
            .as_deref()
            .map(|world| world.gm_attention.clone())
            .unwrap_or_default();
        let hz = world.as_deref().map_or_else(
            || crate::entities::config::GlobalConfig::default().sim_tick_hz,
            |world| world.global.sim_tick_hz,
        );
        rows.extend(idle_rows(&settings, hz, watch, &names));
    }
    // The technical half (issue #1437). It rides this same queue rather than a
    // second list because a facilitator triages ONE ordered set of things that
    // are waiting; the banner region beside it is the part that cannot be
    // filtered, snoozed or held.
    if let Some(health) = health.as_deref().and_then(GmHealthWatch::last) {
        rows.extend(health_rows(&health.alerts));
    }

    // Retire the bookkeeping for anything that stopped holding, so a recurrence
    // that somehow reused an identity still gets a fresh age rather than
    // inheriting the resolved row's.
    let live: std::collections::BTreeSet<&String> = rows.iter().map(|row| &row.id).collect();
    state.first_seen.retain(|id, _| live.contains(id));

    let mut occurrences: Vec<GmAttentionOccurrence> = rows
        .into_iter()
        .map(|row| {
            let seen = *state.first_seen.entry(row.id.clone()).or_insert(FirstSeen {
                tick: tick.0,
                real_ms: now_ms,
            });
            GmAttentionOccurrence {
                id: row.id,
                category: row.category,
                band: row.band,
                first_seen_tick: seen.tick,
                age_ms: now_ms.saturating_sub(seen.real_ms),
                reason: row.reason,
                target: row.target,
            }
        })
        .collect();
    // Oldest first, stable-id tie-break. Bands are a grouping the page applies
    // on top; the age order inside each one is decided exactly once, here.
    occurrences.sort_by(|a, b| {
        a.first_seen_tick
            .cmp(&b.first_seen_tick)
            .then_with(|| a.id.cmp(&b.id))
    });
    occurrences.truncate(MAX_GM_ATTENTION_OCCURRENCES);

    let next = GmAttentionProjection { occurrences };
    // Age alone is not a change: it advances every frame by construction, and
    // republishing on it would make "held" meaningless. Compare the queue's
    // membership, order, band, reason and target instead — the things a GM
    // reads — and let the page age its own rows from the last honest sample.
    let changed = state.last.as_ref().is_none_or(|last| {
        last.occurrences.len() != next.occurrences.len()
            || last
                .occurrences
                .iter()
                .zip(&next.occurrences)
                .any(|(a, b)| {
                    a.id != b.id
                        || a.category != b.category
                        || a.band != b.band
                        || a.first_seen_tick != b.first_seen_tick
                        || a.reason != b.reason
                        || a.target != b.target
                })
    });
    if changed {
        state.last = Some(next.clone());
        writer.write(GmAttentionChanged { payload: next });
    }
}

/// A world with no Comms runtime at all still publishes an empty queue rather
/// than nothing, so a GM desk shows "nothing is waiting" instead of a stale
/// list from the previous scenario.
static EMPTY_INBOX: std::sync::LazyLock<crate::console::comms::inbox::CommsInbox> =
    std::sync::LazyLock::new(crate::console::comms::inbox::CommsInbox::new);

/// Registers the attention projection on a GM-presenting peer, exactly as
/// [`crate::gm_activity::GmActivityPlugin`] registers the activity feed.
pub struct GmAttentionPlugin;

impl Plugin for GmAttentionPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<GmAttentionState>()
            .declare_state::<GmAttentionState>(StateClass::Presentation, "gm-t3-attention-queue")
            .init_resource::<GmIdleNpcWatch>()
            // Presentation for the ordinary reason — no fixed-tick system reads
            // it and it is not folded — but unlike the queue state it IS carried
            // in the snapshot, because elapsed idleness is history a restored
            // world cannot recompute. See the type's own doc comment.
            .declare_state::<GmIdleNpcWatch>(StateClass::Presentation, "gm-t3-idle-npc-advisory")
            .add_message::<GmAttentionChanged>()
            .add_systems(
                FixedLast,
                observe_idle_npcs
                    .before(crate::sim_tick::advance_sim_tick)
                    .run_if(crate::gm_projection::gm_presentation_active),
            )
            .add_systems(
                PostUpdate,
                publish_attention_projection.run_if(crate::gm_projection::gm_presentation_active),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{CommsMessage, CommsPriority, CommsResponseView};
    use crate::gm_comms::{GmCommsRoute, GmCommsVisibility};

    fn message(id: &str, sender: &str) -> CommsMessage {
        CommsMessage::injected(
            id.into(),
            sender.into(),
            "sender".into(),
            "body".into(),
            Default::default(),
            vec![CommsResponseView {
                text: "reply".into(),
                important: false,
                available: true,
            }],
            format!("thread-{id}"),
            true,
            CommsPriority::Routine,
        )
    }

    fn route(id: &str, band: Option<&str>) -> GmCommsRoute {
        GmCommsRoute {
            id: id.into(),
            label: "label".into(),
            visibility: GmCommsVisibility::SelectedShips,
            senders: vec!["speaker".into()],
            hails: Vec::new(),
            attention_band: band.map(str::to_string),
        }
    }

    #[test]
    fn authored_band_overrides_the_default_and_an_unknown_word_is_ignored_by_the_reader() {
        assert_eq!(band_for(None), GmAttentionBand::Attention);
        assert_eq!(
            band_for(Some(&route("a", None))),
            GmAttentionBand::Attention
        );
        assert_eq!(
            band_for(Some(&route("a", Some("urgent")))),
            GmAttentionBand::Urgent
        );
        assert_eq!(
            band_for(Some(&route("a", Some("background")))),
            GmAttentionBand::Background
        );
        // The loader refuses this spelling outright (see `gm_comms::validate_routes`);
        // the reader still refuses to invent a band from it.
        assert_eq!(
            band_for(Some(&route("a", Some("Urgent")))),
            GmAttentionBand::Attention
        );
        assert_eq!(GmAttentionBand::from_authored("critical"), None);
    }

    #[test]
    fn only_a_live_unanswered_message_with_options_is_pending() {
        let live = message("m1", "speaker");
        assert!(pending(&live));
        let mut answered = live.clone();
        answered.selected_response = Some(0);
        assert!(!pending(&answered));
        let mut orphaned = live.clone();
        orphaned.is_orphaned = true;
        assert!(!pending(&orphaned));
        let mut placeholder = live;
        placeholder.responses.clear();
        assert!(!pending(&placeholder));
    }

    #[test]
    fn occurrence_identity_follows_the_message_and_carries_the_authored_route() {
        let world = WorldConfig {
            gm_comms_routes: vec![route("private", Some("background"))],
            ..Default::default()
        };
        let mut inbox = crate::console::comms::inbox::CommsInbox::new();
        let mut addressed = message("m1", "speaker-uuid");
        addressed.recipient_ship = Some(crate::command_admission::log::ShipKey(
            "ship-uuid".to_string(),
        ));
        inbox.inject(addressed);
        let names = BTreeMap::from([
            ("speaker-uuid".to_string(), "speaker".to_string()),
            ("ship-uuid".to_string(), "Valiant".to_string()),
        ]);
        let rows = collect(Some(&world), &inbox, &names);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "comms:m1");
        assert_eq!(rows[0].band, GmAttentionBand::Background);
        assert_eq!(rows[0].reason.id, PENDING_COMMS_REASON);
        assert_eq!(rows[0].reason.params.get("sender").unwrap(), "speaker");
        assert_eq!(rows[0].reason.params.get("ship").unwrap(), "Valiant");
        assert_eq!(rows[0].target.route.as_deref(), Some("private"));
        assert_eq!(rows[0].target.conversation.as_deref(), Some("thread-m1"));
    }

    /// A hail nobody addressed at one hull says so, rather than rendering the
    /// addressed sentence around an empty `{ship}`.
    #[test]
    fn a_conversation_with_no_recipient_ship_reads_as_fleet_wide() {
        let mut inbox = crate::console::comms::inbox::CommsInbox::new();
        let fleet_wide = message("m1", "speaker-uuid");
        assert!(fleet_wide.recipient_ship.is_none());
        inbox.inject(fleet_wide);
        let names = BTreeMap::from([("speaker-uuid".to_string(), "speaker".to_string())]);
        let rows = collect(None, &inbox, &names);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reason.id, PENDING_COMMS_FLEET_REASON);
        assert_eq!(rows[0].reason.params.get("sender").unwrap(), "speaker");
        // No blank parameter at all: the sentence that reads this id does not
        // mention a ship, so carrying an empty one would be a lie in waiting.
        assert!(rows[0].reason.params.get("ship").is_none());
        assert!(rows[0].target.ship.is_none());
    }

    // ── Idle NPC advisory (issue #1435) ──────────────────────────────────────
    //
    // The world-level behaviour lives in `tests/gm_idle_npc.rs`, over real
    // authored hulls and real GM actions. What is here is the half that cannot
    // be reached from an integration test: an Objective SCOPED to a hull, whose
    // only writer (`ObjectiveManager::set_recipients`) is crate-private because
    // the trusted activation seam is its only caller.

    /// Build the smallest world the stopwatch needs: a tick, an objective
    /// manager, the watch, and ships carrying exactly the components the
    /// production query filters on.
    fn idle_world() -> App {
        let mut app = App::new();
        app.init_resource::<crate::sim_tick::SimTick>()
            .init_resource::<crate::world::server::ObjectiveManagerRes>()
            .init_resource::<GmIdleNpcWatch>();
        app
    }

    /// One hull with the given scored pool on its Viewscreen blackboard.
    fn hull(app: &mut App, uuid: &str, pool: Vec<crate::core::messages::ScoredObjective>) {
        let mut blackboards = crate::server_app::ShipSystemBlackboards(Default::default());
        blackboards.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            crate::core::messages::SystemBlackboard::Viewscreen(
                crate::core::messages::ViewscreenBlackboard {
                    red_alert: false,
                    hull_integrity_pct: 100.0,
                    last_damage_taken_secs: None,
                    last_weapon_fired_secs: None,
                    last_attacker_uuid: None,
                    scored_objectives: pool,
                    combat_lock: None,
                    science_target: None,
                },
            ),
        );
        app.world_mut().spawn((
            EntityUuid(uuid.to_string()),
            blackboards,
            crate::server_app::Ship,
        ));
    }

    fn patrol_pool(score: f32) -> Vec<crate::core::messages::ScoredObjective> {
        vec![crate::core::messages::ScoredObjective {
            id: "patrol".into(),
            score,
            directive: crate::core::messages::AiDirective::Patrol {
                anchors: vec!["a".into(), "b".into()],
                loop_path: true,
            },
            source: crate::core::messages::ObjectiveSource::Doctrine,
            relevance: Vec::new(),
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "patrol".into(),
                text: "patrol".into(),
                text_params: Default::default(),
                mandatory: false,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: Vec::new(),
                source: crate::core::messages::ObjectiveSource::Doctrine,
            },
        }]
    }

    fn observe(app: &mut App) {
        use bevy::ecs::system::RunSystemOnce;
        app.world_mut().run_system_once(observe_idle_npcs).unwrap();
    }

    /// A standing order is an order; a doctrine entry that has gated itself down
    /// to zero is not, which is exactly the difference between "the ship is
    /// holding station" and "the ship has nothing left to do".
    #[test]
    fn a_positively_scored_standing_directive_is_an_order_and_a_gated_out_one_is_not() {
        let mut app = idle_world();
        hull(&mut app, "patrolling", patrol_pool(45.0));
        hull(&mut app, "gated-out", patrol_pool(0.0));
        hull(&mut app, "no-doctrine", Vec::new());
        observe(&mut app);
        let watch = app.world().resource::<GmIdleNpcWatch>();
        assert_eq!(watch.spell("patrolling"), None);
        assert_eq!(watch.spell("gated-out").map(|s| s.ticks), Some(1));
        assert_eq!(watch.spell("no-doctrine").map(|s| s.ticks), Some(1));
    }

    /// An Objective addressed to this hull is a job, and resolving it puts the
    /// hull back on the clock from zero. An Objective addressed to NOBODY in
    /// particular is the mission's, and must not silence the advisory for every
    /// NPC in the world.
    #[test]
    fn an_objective_scoped_to_the_hull_ends_the_spell_and_an_unscoped_one_does_not() {
        let mut app = idle_world();
        hull(&mut app, "idle-one", Vec::new());
        hull(&mut app, "idle-two", Vec::new());
        observe(&mut app);
        assert_eq!(
            app.world()
                .resource::<GmIdleNpcWatch>()
                .spell("idle-one")
                .map(|s| s.ticks),
            Some(1)
        );

        // A mission line nobody addressed changes nothing for either hull.
        {
            let mut manager = app
                .world_mut()
                .resource_mut::<crate::world::server::ObjectiveManagerRes>();
            manager.0.add("fleet-wide", "text", false, Vec::new());
        }
        observe(&mut app);
        let watch = app.world().resource::<GmIdleNpcWatch>();
        assert_eq!(watch.spell("idle-one").map(|s| s.ticks), Some(2));
        assert_eq!(watch.spell("idle-two").map(|s| s.ticks), Some(2));

        // One addressed at `idle-one` ends only that hull's spell.
        {
            let mut manager = app
                .world_mut()
                .resource_mut::<crate::world::server::ObjectiveManagerRes>();
            manager.0.add("escort", "text", false, Vec::new());
            assert!(manager.0.set_recipients("escort", vec!["idle-one".into()]));
        }
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 40;
        observe(&mut app);
        let watch = app.world().resource::<GmIdleNpcWatch>();
        assert_eq!(watch.spell("idle-one"), None);
        assert_eq!(watch.spell("idle-two").map(|s| s.ticks), Some(3));

        // Completing it puts the hull back on the clock — from zero, at a new
        // start tick, so the row it eventually raises is a new occurrence.
        {
            let mut manager = app
                .world_mut()
                .resource_mut::<crate::world::server::ObjectiveManagerRes>();
            assert!(manager.0.complete("escort"));
        }
        observe(&mut app);
        assert_eq!(
            app.world().resource::<GmIdleNpcWatch>().spell("idle-one"),
            Some(GmIdleSpell {
                started_tick: 40,
                ticks: 1
            })
        );
    }

    /// A hull that leaves the world takes its wait with it, so a hull recreated
    /// under the same identity is watched from scratch.
    #[test]
    fn a_ship_that_leaves_the_world_is_forgotten_rather_than_frozen() {
        let mut app = idle_world();
        hull(&mut app, "gone-soon", Vec::new());
        observe(&mut app);
        observe(&mut app);
        assert_eq!(
            app.world()
                .resource::<GmIdleNpcWatch>()
                .spell("gone-soon")
                .map(|s| s.ticks),
            Some(2)
        );
        let entity = app
            .world_mut()
            .query_filtered::<Entity, With<EntityUuid>>()
            .iter(app.world())
            .next()
            .unwrap();
        app.world_mut().despawn(entity);
        observe(&mut app);
        assert!(app.world().resource::<GmIdleNpcWatch>().is_empty());

        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 99;
        hull(&mut app, "gone-soon", Vec::new());
        observe(&mut app);
        assert_eq!(
            app.world().resource::<GmIdleNpcWatch>().spell("gone-soon"),
            Some(GmIdleSpell {
                started_tick: 99,
                ticks: 1
            })
        );
    }

    /// The authored knobs, as pure arithmetic over the settings.
    #[test]
    fn the_authored_grace_converts_to_exact_ticks_and_refuses_the_unusable() {
        let mut settings = GmAttentionSettings::default();
        assert_eq!(settings.idle_npc_grace_secs, DEFAULT_IDLE_NPC_GRACE_SECS);
        assert_eq!(settings.idle_npc_band(), GmAttentionBand::Background);
        assert_eq!(settings.idle_grace_ticks(60.0), 1800);
        assert_eq!(settings.idle_grace_ticks(30.0), 900);
        assert!(settings.validate().is_ok());

        // A grace shorter than a tick still costs a whole tick — the fixed loop
        // has no smaller unit to spend.
        settings.idle_npc_grace_secs = 0.001;
        assert_eq!(settings.idle_grace_ticks(30.0), 1);

        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            settings.idle_npc_grace_secs = bad;
            let error = settings.validate().unwrap_err();
            assert!(
                error.contains("[gm_attention]") && error.contains("idle_npc_grace_secs"),
                "{error}"
            );
        }

        settings.idle_npc_grace_secs = 5.0;
        settings.idle_npc_band = Some("urgent".into());
        assert!(settings.validate().is_ok());
        assert_eq!(settings.idle_npc_band(), GmAttentionBand::Urgent);
        settings.idle_npc_band = Some("Urgent".into());
        let error = settings.validate().unwrap_err();
        assert!(
            error.contains("idle_npc_band") && error.contains("'urgent'"),
            "{error}"
        );
    }

    /// The rows themselves: below the grace nothing is said, at it exactly one
    /// row is, and the off switch says nothing at any age.
    #[test]
    fn idle_rows_appear_at_the_grace_and_the_off_switch_suppresses_them_at_any_age() {
        let mut watch = GmIdleNpcWatch::default();
        watch.spells.insert(
            "ship-a".into(),
            GmIdleSpell {
                started_tick: 7,
                ticks: 59,
            },
        );
        let names = BTreeMap::from([("ship-a".to_string(), "Drifter".to_string())]);
        let settings = GmAttentionSettings {
            idle_npc_grace_secs: 2.0,
            idle_npc_band: Some("urgent".into()),
            idle_npc_disabled: false,
        };
        assert!(idle_rows(&settings, 30.0, &watch, &names).is_empty());

        watch.spells.get_mut("ship-a").unwrap().ticks = 60;
        let rows = idle_rows(&settings, 30.0, &watch, &names);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "idle:ship-a:7");
        assert_eq!(rows[0].band, GmAttentionBand::Urgent);
        assert_eq!(rows[0].category, GmAttentionCategory::IdleNpc);
        assert_eq!(rows[0].reason.id, IDLE_NPC_REASON);
        assert_eq!(rows[0].reason.params.get("ship").unwrap(), "Drifter");
        assert_eq!(rows[0].reason.params.get("idle").unwrap(), "0:02");
        assert_eq!(
            rows[0]
                .target
                .ship
                .as_ref()
                .map(|ship| ship.entity_id.as_str()),
            Some("ship-a")
        );
        assert!(rows[0].target.route.is_none());
        assert!(rows[0].target.event.is_none());

        watch.spells.get_mut("ship-a").unwrap().ticks = 30 * 3600;
        let disabled = GmAttentionSettings {
            idle_npc_disabled: true,
            ..settings
        };
        assert!(idle_rows(&disabled, 30.0, &watch, &names).is_empty());
    }

    /// Simulation minutes and seconds, floored — a wait is reported as the time
    /// actually served, never rounded up into one the hull has not spent.
    #[test]
    fn the_reported_idle_age_is_floored_simulation_time() {
        assert_eq!(format_sim_clock(0, 30.0), "0:00");
        assert_eq!(format_sim_clock(29, 30.0), "0:00");
        assert_eq!(format_sim_clock(30, 30.0), "0:01");
        assert_eq!(format_sim_clock(30 * 90, 30.0), "1:30");
        assert_eq!(format_sim_clock(60 * 125, 60.0), "2:05");
        // A world with no usable rate cannot claim an age it cannot measure.
        assert_eq!(format_sim_clock(600, 0.0), "0:00");
    }
}
