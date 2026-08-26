//! Pure, Bevy-free civilian **routes**, **orders** and **compliance**
//! (issue #1028, Falling Skyway foundation).
//!
//! Traffic control is only real work for Navigation if the traffic has somewhere
//! to be going and can be told otherwise. This module owns three vocabularies
//! and one state machine:
//!
//! * a **route** — an authored chain of world anchors with per-leg behaviour and
//!   an authored loop/terminate ending;
//! * an **order** — `hold`, `divert` or `dock`, the three things a crew can ask
//!   of a civilian;
//! * a **disposition** — how cooperative this particular hull is, per order
//!   verb, as authored data rather than a personality baked into code;
//!
//! and the **compliance state machine** that turns an order plus a disposition
//! into a sequence a console can watch: received → acknowledged → complying, or
//! refused, or — for a civilian that accepted and then could not carry it out —
//! non-compliant.
//!
//! # This module steers nothing
//!
//! It decides *what a civilian is trying to do*, never *how a hull gets there*.
//! [`CivilianState::travel`] answers the first question in terms of an existing
//! authored directive ([`CivilianTravel`]); the Bevy sibling
//! [`crate::civilian::server`] installs that answer as the entity's own doctrine
//! objective, and the ordinary NPC helm — `score_doctrine_pool` →
//! `plan_helm_travel` → `SetThrust` / `SetSteering` — flies it exactly as it
//! flies every other NPC's authored doctrine. There is no second steering
//! implementation here and none in the adapter; see the adapter's module docs
//! for the seam-by-seam accounting.
//!
//! # Ticks, not seconds
//!
//! Authoring is in whole seconds (`ack_secs`, `decide_secs`, a leg's
//! `hold_secs`), matching the integer-only script surface; the conversion to
//! absolute `SimTick`s happens once, at the moment the clock starts, through the
//! same [`seconds_to_ticks`] the callback queue already uses. Only ticks are
//! stored and only ticks are compared, so two peers running the same world at
//! the same `sim_tick_hz` acknowledge, refuse and comply on the same tick.

use serde::{Deserialize, Serialize};

use crate::world::script::schedule::seconds_to_ticks;

// ── Authored route vocabulary (`[[route]]` in a world TOML) ──────────────────

/// Default cruise fraction for a leg that does not author one.
///
/// A TOML-parse fallback, which is the only kind of hardcoded gameplay value
/// AGENTS.md #11 sanctions. Half throttle: ambient traffic that reads as
/// *going somewhere* without outrunning the crew's ability to talk to it.
fn default_leg_speed() -> f32 {
    0.5
}

/// What a civilian does when it runs out of legs.
///
/// `Loop` is the default because the vocabulary exists for *ambient traffic* —
/// a depot run, a shuttle circuit — and a mission that fills a sector with
/// haulers which all stop dead at their last anchor is the surprising outcome,
/// not the expected one. A convoy with somewhere final to be authors
/// `terminate`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteCompletion {
    /// Wrap back to the first leg and keep flying.
    #[default]
    Loop,
    /// Stop at the last leg and hold station there.
    Terminate,
}

impl RouteCompletion {
    /// The wire/script label, the same word the `[[route]]` vocabulary uses.
    /// Written by hand rather than derived, so the strings a console compares
    /// against are visible at the point they are promised.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::Terminate => "terminate",
        }
    }
}

/// One `[[route.leg]]` block: an anchor to make for, and how to fly it.
///
/// The anchor is a name in the world's `[anchors]` table — the same table every
/// `Patrol` / `Reach` doctrine directive resolves against. A leg naming an
/// anchor no world in the composition declares blocks activation
/// (`world::validate`), because a route whose leg reads as nothing is a civilian
/// that silently never goes anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteLeg {
    /// Anchor name this leg makes for.
    pub anchor: String,
    /// Cruise fraction `0.0..=1.0` while flying this leg. Per leg rather than
    /// per route: a hauler slows on the approach to a depot and opens up again
    /// on the outbound run, and that is the whole of what "traffic has a shape"
    /// means at this altitude.
    #[serde(default = "default_leg_speed")]
    pub speed: f32,
    /// Whole seconds to sit still after reaching this leg's anchor before
    /// pressing on. `0` (the default) flies straight through.
    #[serde(default)]
    pub hold_secs: i64,
}

/// One authored `[[route]]` block.
///
/// ```toml
/// [[route]]
/// id = "depot_run"
/// on_complete = "loop"
///
/// [[route.leg]]
/// anchor = "depot_north"
/// speed = 0.4
/// hold_secs = 20
/// ```
///
/// Routes are **world** data, not entity data: an anchor chain belongs to the
/// map it crosses, and two haulers running the same lane should be running the
/// same authored record rather than two copies that can drift. An entity names
/// one by id in its own `[civilian]` table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    /// Stable id, unique within a world. An order, a `[civilian]` table and a
    /// script all name the route by this.
    pub id: String,
    /// Legs in authored order. The field is named `legs` and the TOML key is
    /// `leg`, matching the `[[route.leg]]` array-of-tables spelling.
    #[serde(default, rename = "leg", skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<RouteLeg>,
    /// What happens after the last leg.
    #[serde(default)]
    pub on_complete: RouteCompletion,
}

impl RouteConfig {
    /// Reject a `[[route]]` block that cannot mean anything.
    ///
    /// Called from `parse_world` so a typo is a load error naming the route,
    /// not a civilian that silently holds station forever. Anchor *resolution*
    /// is a separate, composition-wide pass (`world::validate`), because a
    /// route may legitimately cross anchors a sibling layer declares.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("[[route]] has an empty id; every route needs a stable id \
                        for an entity or an order to name it"
                .to_string());
        }
        if self.legs.is_empty() {
            return Err(format!(
                "route '{}' declares no [[route.leg]] blocks; a route with no legs \
                 is a civilian with nowhere to go",
                self.id
            ));
        }
        for (i, leg) in self.legs.iter().enumerate() {
            if leg.anchor.trim().is_empty() {
                return Err(format!(
                    "route '{}' leg #{i} has an empty anchor name",
                    self.id
                ));
            }
            if !leg.speed.is_finite() || leg.speed <= 0.0 || leg.speed > 1.0 {
                return Err(format!(
                    "route '{}' leg #{i} (anchor '{}') has speed {}; a leg's cruise \
                     fraction must be in (0.0, 1.0]",
                    self.id, leg.anchor, leg.speed
                ));
            }
            if leg.hold_secs < 0 {
                return Err(format!(
                    "route '{}' leg #{i} (anchor '{}') has hold_secs {}; a dwell \
                     cannot be negative",
                    self.id, leg.anchor, leg.hold_secs
                ));
            }
        }
        Ok(())
    }

    /// The leg anchors in authored order — exactly the `anchors` list an
    /// `AiDirective::Patrol` carries, which is how a route is flown.
    pub fn anchor_chain(&self) -> Vec<String> {
        self.legs.iter().map(|l| l.anchor.clone()).collect()
    }

    /// Whether the chain wraps, i.e. `AiDirective::Patrol { loop_path }`.
    pub fn loops(&self) -> bool {
        self.on_complete == RouteCompletion::Loop
    }

    /// The leg at `index`, wrapping for a looping route and saturating at the
    /// last leg for one that terminates.
    pub fn leg(&self, index: usize) -> Option<&RouteLeg> {
        if self.legs.is_empty() {
            return None;
        }
        if index < self.legs.len() {
            return self.legs.get(index);
        }
        if self.loops() {
            self.legs.get(index % self.legs.len())
        } else {
            self.legs.last()
        }
    }
}

// ── Authored order vocabulary ────────────────────────────────────────────────

/// Which of the three verbs an order is, for disposition lookup and for the
/// wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    /// `hold`.
    Hold,
    /// `divert`, either flavour.
    Divert,
    /// `dock`.
    Dock,
}

impl OrderKind {
    /// The wire/script label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Divert => "divert",
            Self::Dock => "dock",
        }
    }
}

/// An order issued to one civilian.
///
/// Three verbs, matching the three things a Navigation officer actually needs to
/// say to traffic. `Divert` carries two mutually exclusive destinations rather
/// than splitting into two verbs, because "go somewhere else" is one instruction
/// whether the somewhere else is a whole lane or a single point;
/// [`CivilianOrder::validate`] refuses a divert that names both or neither.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum CivilianOrder {
    /// Stop where you are and hold station.
    Hold,
    /// Take this alternate route, or make for this single anchor.
    Divert {
        /// A `[[route]]` id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<String>,
        /// An `[anchors]` name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
    },
    /// Proceed to and dock at the named structure.
    Dock {
        /// The structure's authored world entity name.
        structure: String,
    },
}

/// One authored control offered for a civilian on the Navigation console.
///
/// The option is data rather than client policy: a scenario chooses which
/// orders make sense for one craft, while the console only renders the label
/// and sends the already-supported [`CivilianOrder`] payload. `id` is stable
/// authoring identity for tests, automation and future save migrations; it is
/// never player-visible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CivilianOrderOption {
    /// Stable identifier, unique within this civilian's option list.
    pub id: String,
    /// Player-facing `strings.csv` id rendered on the order button.
    pub label: String,
    /// The authoritative order the button submits.
    pub order: CivilianOrder,
}

impl CivilianOrderOption {
    /// Reject an option that cannot identify, label or carry out its order.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("[civilian.order_options] id must not be empty".to_string());
        }
        if self.label.trim().is_empty() {
            return Err(format!(
                "[civilian.order_options] '{}' label must be a strings.csv id",
                self.id
            ));
        }
        self.order.validate().map_err(|err| {
            format!(
                "[civilian.order_options] '{}' contains an invalid order: {err}",
                self.id
            )
        })
    }
}

impl CivilianOrder {
    /// Divert onto a named route.
    pub fn divert_to_route(route: impl Into<String>) -> Self {
        Self::Divert {
            route: Some(route.into()),
            anchor: None,
        }
    }

    /// Divert to a single named anchor.
    pub fn divert_to_anchor(anchor: impl Into<String>) -> Self {
        Self::Divert {
            route: None,
            anchor: Some(anchor.into()),
        }
    }

    /// Dock at a named structure.
    pub fn dock_at(structure: impl Into<String>) -> Self {
        Self::Dock {
            structure: structure.into(),
        }
    }

    /// Which verb this is.
    pub fn kind(&self) -> OrderKind {
        match self {
            Self::Hold => OrderKind::Hold,
            Self::Divert { .. } => OrderKind::Divert,
            Self::Dock { .. } => OrderKind::Dock,
        }
    }

    /// Authored route destination, when this is a route divert. World
    /// validation uses this to reject a button that can only submit a dangling
    /// lane reference before the scenario activates.
    pub fn route_destination(&self) -> Option<&str> {
        match self {
            Self::Divert {
                route: Some(route), ..
            } => Some(route),
            _ => None,
        }
    }

    /// Reject an order that cannot be carried out by construction.
    ///
    /// Checked at the admission and script boundaries both, so a malformed
    /// order is refused *at the console* with a reason rather than becoming a
    /// civilian stuck in `non_compliant` for a reason nobody can act on.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hold => Ok(()),
            Self::Divert { route, anchor } => match (route, anchor) {
                (Some(r), None) if !r.trim().is_empty() => Ok(()),
                (None, Some(a)) if !a.trim().is_empty() => Ok(()),
                (Some(_), Some(_)) => Err("a divert order names both a route and an \
                                           anchor; it takes exactly one"
                    .to_string()),
                _ => Err("a divert order names neither a route nor an anchor; it \
                          takes exactly one"
                    .to_string()),
            },
            Self::Dock { structure } if structure.trim().is_empty() => {
                Err("a dock order names no structure".to_string())
            }
            Self::Dock { .. } => Ok(()),
        }
    }
}

// ── Authored compliance disposition ──────────────────────────────────────────

/// Whether this hull does as it is told, per verb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderResponse {
    /// Acknowledge and comply.
    #[default]
    Comply,
    /// Acknowledge and decline. The civilian carries on with its own route.
    Refuse,
}

/// Default seconds between an order arriving and the civilian answering it.
///
/// A TOML-parse fallback (AGENTS.md #11). Non-zero on purpose: an order that is
/// obeyed on the tick it is sent is a remote control, and the whole point of
/// this vocabulary is that it is a *negotiation with an actor*. Two seconds is
/// long enough for a console to render `received` before `acknowledged` replaces
/// it at any authored `ai_snapshot_hz`.
fn default_ack_secs() -> i64 {
    2
}

/// Default seconds between acknowledging an order and acting on it.
fn default_decide_secs() -> i64 {
    3
}

/// The string id reported when a civilian accepts an order and then finds it
/// cannot be carried out — the dock target is gone, the diverted-to route
/// resolves nowhere. Distinct from an authored refusal reason because nobody
/// authored this: it is the world changing under an accepted order.
pub const REASON_UNABLE: &str = "civilian.compliance.reason.unable";

/// How cooperative one civilian is, as authored data.
///
/// Authored on an entity's `[civilian.compliance]` table, or on its faction, or
/// neither — a hull that authors nothing is a cooperative one that answers in
/// the default times. Nothing here is a threshold the code invented (AGENTS.md
/// #11): the verbs a hull refuses, how long it takes to answer, and what it says
/// when it declines are all the scenario's to tune.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplianceDisposition {
    /// Whole seconds from an order arriving to the civilian answering it.
    #[serde(default = "default_ack_secs")]
    pub ack_secs: i64,
    /// Whole seconds from answering to acting.
    #[serde(default = "default_decide_secs")]
    pub decide_secs: i64,
    /// Response to a `hold` order.
    #[serde(default)]
    pub hold: OrderResponse,
    /// Response to a `divert` order.
    #[serde(default)]
    pub divert: OrderResponse,
    /// Response to a `dock` order.
    #[serde(default)]
    pub dock: OrderResponse,
    /// `strings.csv` id for what this hull says when it refuses. Display text,
    /// so an id and not English (AGENTS.md #11's sanctioned exception).
    #[serde(default = "default_refusal_reason")]
    pub refusal_reason: String,
}

/// Default `strings.csv` id for a refusal that authors no reason of its own.
fn default_refusal_reason() -> String {
    "civilian.compliance.reason.declined".to_string()
}

impl Default for ComplianceDisposition {
    /// Hand-written so it calls the same `default_*` fns serde does — two copies
    /// of these numbers could only ever drift apart.
    fn default() -> Self {
        Self {
            ack_secs: default_ack_secs(),
            decide_secs: default_decide_secs(),
            hold: OrderResponse::default(),
            divert: OrderResponse::default(),
            dock: OrderResponse::default(),
            refusal_reason: default_refusal_reason(),
        }
    }
}

impl ComplianceDisposition {
    /// This hull's authored response to `kind`.
    pub fn response(&self, kind: OrderKind) -> OrderResponse {
        match kind {
            OrderKind::Hold => self.hold,
            OrderKind::Divert => self.divert,
            OrderKind::Dock => self.dock,
        }
    }

    /// Reject a disposition that cannot mean anything.
    pub fn validate(&self) -> Result<(), String> {
        if self.ack_secs < 0 || self.decide_secs < 0 {
            return Err(format!(
                "[civilian.compliance] ack_secs and decide_secs must not be negative, \
                 got {} and {}",
                self.ack_secs, self.decide_secs
            ));
        }
        if self.refusal_reason.trim().is_empty() {
            return Err(
                "[civilian.compliance] refusal_reason must be a strings.csv id".to_string(),
            );
        }
        Ok(())
    }
}

/// The `[civilian]` table on an entity TOML.
///
/// An entity that omits it is not civilian traffic and carries none of this
/// state — which is every entity shipped before this existed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CivilianConfig {
    /// The `[[route]]` id this hull flies. `None` is legal: a civilian with no
    /// standing route holds station until it is told to go somewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Utility priority of the route objective this hull's doctrine pool
    /// receives. Above a courier's own `reach-destination` (30.0) by default, so
    /// a hull that authors both flies the route it was assigned.
    #[serde(default = "default_route_priority")]
    pub route_priority: f32,
    /// Per-hull compliance. Absent falls back to the faction's, then to the
    /// cooperative default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compliance: Option<ComplianceDisposition>,
    /// Scenario-authored controls exposed for this craft on Navigation.
    /// Empty keeps older entities and worlds read-only on that panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_options: Vec<CivilianOrderOption>,
}

/// Default utility priority of a civilian's route objective.
fn default_route_priority() -> f32 {
    60.0
}

impl CivilianConfig {
    /// Reject a `[civilian]` table that cannot mean anything.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(route) = self.route.as_ref() {
            if route.trim().is_empty() {
                return Err("[civilian] route is an empty string; omit the field to \
                            author a civilian with no standing route"
                    .to_string());
            }
        }
        if !self.route_priority.is_finite() || self.route_priority < 0.0 {
            return Err(format!(
                "[civilian] route_priority must be a non-negative finite number, got {}",
                self.route_priority
            ));
        }
        if let Some(compliance) = self.compliance.as_ref() {
            compliance.validate()?;
        }
        let mut option_ids = std::collections::HashSet::new();
        for option in &self.order_options {
            option.validate()?;
            if !option_ids.insert(option.id.as_str()) {
                return Err(format!(
                    "[civilian.order_options] duplicate id '{}'",
                    option.id
                ));
            }
        }
        Ok(())
    }
}

// ── Compliance state ─────────────────────────────────────────────────────────

/// Where a civilian stands with respect to its current order.
///
/// The five states the issue names, plus the resting one an unordered civilian
/// sits in. `Refused` and `NonCompliant` are deliberately different things and a
/// console must be able to tell them apart: a refusal is a *decision* (it said
/// no and carried on with its own route), while non-compliance is a *failure*
/// (it agreed, set off, and the world moved — the dock is gone, the lane
/// resolves nowhere). The second is the one that needs a crew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceState {
    /// No standing order; flying its own route.
    #[default]
    Unordered,
    /// An order has arrived and has not been answered yet.
    Received,
    /// Answered, not yet acted on.
    Acknowledged,
    /// Doing as asked.
    Complying,
    /// Agreed, and now cannot carry it out.
    NonCompliant,
    /// Declined, per its authored disposition.
    Refused,
}

impl ComplianceState {
    /// The wire/script label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unordered => "unordered",
            Self::Received => "received",
            Self::Acknowledged => "acknowledged",
            Self::Complying => "complying",
            Self::NonCompliant => "non_compliant",
            Self::Refused => "refused",
        }
    }

    /// Whether this state is one an order is still moving through, i.e. whether
    /// the civilian owes the crew an answer or an outcome.
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Received | Self::Acknowledged)
    }
}

/// One compliance transition, for logging and for the console's event feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComplianceTransition {
    /// The state left behind.
    pub from: ComplianceState,
    /// The state entered.
    pub to: ComplianceState,
    /// `strings.csv` id explaining a refusal or a failure; `None` otherwise.
    pub reason: Option<String>,
}

/// What a civilian is currently trying to do, in terms the adapter can install
/// as an ordinary authored directive.
///
/// This is the whole of the "no second steering implementation" contract: every
/// variant here names something the NPC helm already flies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CivilianTravel {
    /// Fly the named route — an `AiDirective::Patrol` over its anchor chain,
    /// with the existing `PatrolCursor` as the leg pointer.
    Route {
        /// The `[[route]]` id.
        id: String,
    },
    /// Make for a single named anchor — an `AiDirective::Reach`.
    Anchor {
        /// The `[anchors]` name.
        name: String,
    },
    /// Close on a named structure — the ship's own `NavigationWaypoint`,
    /// anchored to that entity, plus the existing docking close manoeuvre.
    Dock {
        /// The structure's authored world entity name.
        structure: String,
    },
    /// Hold station: no helm-relevant directive at all, which is how every
    /// objective-less NPC already comes to a stop.
    Hold,
}

/// One civilian's live traffic state.
///
/// Authoritative per-entity simulation state: it decides where a hull is going
/// and whether the crew's order is being honoured, and two hosts that disagreed
/// about it would disagree about whether a mission is going well.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CivilianState {
    /// The route currently assigned — the authored one until a complied divert
    /// replaces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    /// Index of the leg being flown, mirrored from the entity's `PatrolCursor`.
    #[serde(default)]
    leg: usize,
    /// Where the civilian stands with its order.
    #[serde(default)]
    compliance: ComplianceState,
    /// Absolute `SimTick` the current compliance stage completes on. Only read
    /// while [`ComplianceState::is_pending`].
    #[serde(default)]
    due_tick: u64,
    /// Absolute `SimTick` an authored per-leg dwell ends on. `0` = not dwelling.
    #[serde(default)]
    dwell_until_tick: u64,
    /// `strings.csv` id explaining a refusal or a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// The standing order, if any.
    ///
    /// Declared last because it is the one field that serialises as a *table*
    /// (an internally-tagged enum), and TOML refuses a scalar emitted after a
    /// table. Field order here is therefore load-bearing for the save path, not
    /// cosmetic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<CivilianOrder>,
}

impl CivilianState {
    /// The state an entity spawns with, from its `[civilian]` table.
    pub fn from_config(config: &CivilianConfig) -> Self {
        Self {
            route: config.route.clone(),
            ..Self::default()
        }
    }

    /// Restore a state from a save (issue #863/#864's adapters call this).
    #[allow(clippy::too_many_arguments)]
    pub fn restored(
        route: Option<String>,
        leg: usize,
        order: Option<CivilianOrder>,
        compliance: ComplianceState,
        due_tick: u64,
        dwell_until_tick: u64,
        reason: Option<String>,
    ) -> Self {
        Self {
            route,
            leg,
            compliance,
            due_tick,
            dwell_until_tick,
            reason,
            order,
        }
    }

    /// The route currently assigned.
    pub fn route(&self) -> Option<&str> {
        self.route.as_deref()
    }

    /// Index of the leg being flown.
    pub fn leg(&self) -> usize {
        self.leg
    }

    /// The standing order.
    pub fn order(&self) -> Option<&CivilianOrder> {
        self.order.as_ref()
    }

    /// Where the civilian stands with its order.
    pub fn compliance(&self) -> ComplianceState {
        self.compliance
    }

    /// Absolute tick the current compliance stage completes on.
    pub fn due_tick(&self) -> u64 {
        self.due_tick
    }

    /// Absolute tick an authored dwell ends on; `0` when not dwelling.
    pub fn dwell_until_tick(&self) -> u64 {
        self.dwell_until_tick
    }

    /// `strings.csv` id explaining a refusal or a failure.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Take an order.
    ///
    /// Always starts the clock, whatever the disposition says: a hull that
    /// refuses still *receives* first, so the console sees the same shape for a
    /// cooperative and an uncooperative civilian and the difference is in what
    /// the answer turns out to be. Replaces any order still in flight — the
    /// latest instruction is the operative one.
    pub fn receive_order(
        &mut self,
        order: CivilianOrder,
        disposition: &ComplianceDisposition,
        now: u64,
        tick_hz: f32,
    ) -> Option<ComplianceTransition> {
        let from = self.compliance;
        self.order = Some(order);
        self.compliance = ComplianceState::Received;
        self.due_tick = now.saturating_add(seconds_to_ticks(disposition.ack_secs, tick_hz));
        self.reason = None;
        Some(ComplianceTransition {
            from,
            to: ComplianceState::Received,
            reason: None,
        })
    }

    /// Cancel any standing order and return to the civilian's own route.
    pub fn clear_order(&mut self) -> Option<ComplianceTransition> {
        if self.order.is_none() && self.compliance == ComplianceState::Unordered {
            return None;
        }
        let from = self.compliance;
        self.order = None;
        self.compliance = ComplianceState::Unordered;
        self.due_tick = 0;
        self.reason = None;
        Some(ComplianceTransition {
            from,
            to: ComplianceState::Unordered,
            reason: None,
        })
    }

    /// Advance the compliance clock by one logical tick.
    ///
    /// `destination_resolves` is the adapter's answer to "can this order still
    /// be carried out?" — the dock target exists, the diverted-to route is
    /// declared. It is an input rather than something decided here because
    /// answering it needs the live world, and this module has none.
    ///
    /// At most one transition per tick, so a console rendering at any authored
    /// `ai_snapshot_hz` sees the sequence rather than the endpoint.
    pub fn advance(
        &mut self,
        now: u64,
        destination_resolves: bool,
        disposition: &ComplianceDisposition,
        tick_hz: f32,
    ) -> Option<ComplianceTransition> {
        match self.compliance {
            ComplianceState::Received if now >= self.due_tick => {
                let kind = self.order.as_ref()?.kind();
                if disposition.response(kind) == OrderResponse::Refuse {
                    return Some(self.enter(
                        ComplianceState::Refused,
                        Some(disposition.refusal_reason.clone()),
                    ));
                }
                let t = self.enter(ComplianceState::Acknowledged, None);
                self.due_tick =
                    now.saturating_add(seconds_to_ticks(disposition.decide_secs, tick_hz));
                Some(t)
            }
            ComplianceState::Acknowledged if now >= self.due_tick => {
                if !destination_resolves {
                    return Some(self.enter(
                        ComplianceState::NonCompliant,
                        Some(REASON_UNABLE.to_string()),
                    ));
                }
                // A complied divert onto a route *becomes* this civilian's
                // route: from here on it is flying its own traffic pattern
                // again, just a different one, and the leg pointer restarts.
                if let Some(CivilianOrder::Divert {
                    route: Some(route), ..
                }) = self.order.as_ref()
                {
                    self.route = Some(route.clone());
                    self.leg = 0;
                }
                Some(self.enter(ComplianceState::Complying, None))
            }
            ComplianceState::Complying if !destination_resolves => Some(self.enter(
                ComplianceState::NonCompliant,
                Some(REASON_UNABLE.to_string()),
            )),
            // A civilian that got stuck and then found its destination again
            // resumes rather than needing a fresh order — the crew already told
            // it what to do and it never stopped agreeing.
            ComplianceState::NonCompliant if destination_resolves => {
                Some(self.enter(ComplianceState::Complying, None))
            }
            _ => None,
        }
    }

    /// Mirror the entity's live `PatrolCursor` index onto the state, starting an
    /// authored dwell when the leg it just left asked for one.
    ///
    /// The cursor is the leg pointer — this module keeps no second one. Index
    /// `i` means "steering towards leg `i`", so the leg that was *reached* when
    /// the index moves off `i` is `i` itself.
    pub fn observe_leg(
        &mut self,
        index: usize,
        route: Option<&RouteConfig>,
        now: u64,
        tick_hz: f32,
    ) {
        if index == self.leg {
            return;
        }
        let reached = self.leg;
        self.leg = index;
        let Some(route) = route else {
            return;
        };
        let Some(leg) = route.leg(reached) else {
            return;
        };
        if leg.hold_secs > 0 {
            self.dwell_until_tick = now.saturating_add(seconds_to_ticks(leg.hold_secs, tick_hz));
        }
    }

    /// Whether an authored per-leg dwell is still running.
    pub fn is_dwelling(&self, now: u64) -> bool {
        now < self.dwell_until_tick
    }

    /// The cruise fraction to fly right now: the current leg's authored speed,
    /// or zero while sitting out an authored dwell.
    pub fn cruise_speed(&self, route: Option<&RouteConfig>, now: u64) -> f32 {
        if self.is_dwelling(now) {
            return 0.0;
        }
        route
            .and_then(|r| r.leg(self.leg))
            .map(|l| l.speed)
            .unwrap_or_else(default_leg_speed)
    }

    /// What this civilian is trying to do, for the adapter to install.
    ///
    /// A civilian only flies its *order* once it is [`ComplianceState::Complying`]
    /// — while it is still answering, it carries on doing what it was doing,
    /// which is what makes the acknowledgement delay observable rather than
    /// cosmetic. A refusal leaves it on its own route; a failure stops it where
    /// it is, so "stuck" and "declined" do not look the same out of the window
    /// either.
    pub fn travel(&self) -> CivilianTravel {
        match (self.compliance, self.order.as_ref()) {
            (ComplianceState::Complying, Some(CivilianOrder::Hold)) => CivilianTravel::Hold,
            (ComplianceState::Complying, Some(CivilianOrder::Dock { structure })) => {
                CivilianTravel::Dock {
                    structure: structure.clone(),
                }
            }
            (
                ComplianceState::Complying,
                Some(CivilianOrder::Divert {
                    anchor: Some(anchor),
                    ..
                }),
            ) => CivilianTravel::Anchor {
                name: anchor.clone(),
            },
            (ComplianceState::NonCompliant, _) => CivilianTravel::Hold,
            // Everything else — unordered, mid-answer, refused, or complying
            // with a divert that has already become this civilian's route.
            _ => match self.route.as_ref() {
                Some(id) => CivilianTravel::Route { id: id.clone() },
                None => CivilianTravel::Hold,
            },
        }
    }

    /// Enter `to`, recording the reason and clearing the stage clock.
    fn enter(&mut self, to: ComplianceState, reason: Option<String>) -> ComplianceTransition {
        let from = self.compliance;
        self.compliance = to;
        self.reason = reason.clone();
        self.due_tick = 0;
        ComplianceTransition { from, to, reason }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 60 Hz, the default `[global] sim_tick_hz`.
    const HZ: f32 = 60.0;

    fn depot_run() -> RouteConfig {
        toml::from_str(
            r#"
id = "depot_run"
on_complete = "loop"

[[leg]]
anchor = "depot_north"
speed = 0.4
hold_secs = 10

[[leg]]
anchor = "depot_south"
"#,
        )
        .expect("the fixture parses")
    }

    fn cooperative() -> ComplianceDisposition {
        ComplianceDisposition::default()
    }

    fn stubborn() -> ComplianceDisposition {
        ComplianceDisposition {
            divert: OrderResponse::Refuse,
            refusal_reason: "world.hauler.refuses".to_string(),
            ..ComplianceDisposition::default()
        }
    }

    fn ordered(
        state: &mut CivilianState,
        order: CivilianOrder,
        disposition: &ComplianceDisposition,
    ) {
        state.receive_order(order, disposition, 0, HZ);
    }

    /// Run `advance` from tick 0 up to and including `until`, collecting every
    /// transition — the deterministic shape the headless probe asserts on.
    fn run(
        state: &mut CivilianState,
        until: u64,
        resolves: bool,
        disposition: &ComplianceDisposition,
    ) -> Vec<ComplianceState> {
        let mut seen = Vec::new();
        for tick in 0..=until {
            if let Some(t) = state.advance(tick, resolves, disposition, HZ) {
                seen.push(t.to);
            }
        }
        seen
    }

    // ── AC1: the route vocabulary parses, and a broken one is refused ──

    #[test]
    fn an_authored_route_parses_into_an_anchor_chain_and_a_loop_flag() {
        let route = depot_run();
        assert_eq!(
            route.anchor_chain(),
            vec!["depot_north".to_string(), "depot_south".to_string()],
            "the anchor chain is exactly what an AiDirective::Patrol carries"
        );
        assert!(route.loops(), "on_complete = \"loop\" wraps the chain");
        assert_eq!(
            route.leg(0).map(|l| l.speed),
            Some(0.4),
            "an authored per-leg speed survives"
        );
        assert_eq!(
            route.leg(1).map(|l| l.speed),
            Some(default_leg_speed()),
            "…and a leg that authors none takes the parse fallback"
        );
        assert_eq!(
            route.leg(2).map(|l| l.anchor.as_str()),
            Some("depot_north"),
            "a looping route wraps its leg lookup"
        );
    }

    #[test]
    fn a_terminating_route_saturates_at_its_last_leg_instead_of_wrapping() {
        let route = RouteConfig {
            id: "one_way".into(),
            legs: vec![
                RouteLeg {
                    anchor: "a".into(),
                    speed: 0.5,
                    hold_secs: 0,
                },
                RouteLeg {
                    anchor: "b".into(),
                    speed: 0.5,
                    hold_secs: 0,
                },
            ],
            on_complete: RouteCompletion::Terminate,
        };
        assert!(!route.loops());
        assert_eq!(
            route.leg(9).map(|l| l.anchor.as_str()),
            Some("b"),
            "past the end, a terminating route is parked on its final leg"
        );
    }

    #[test]
    fn a_route_that_cannot_mean_anything_is_refused_by_name() {
        let cases: Vec<(RouteConfig, &str)> = vec![
            (
                RouteConfig {
                    id: "  ".into(),
                    legs: vec![RouteLeg {
                        anchor: "a".into(),
                        speed: 0.5,
                        hold_secs: 0,
                    }],
                    on_complete: RouteCompletion::Loop,
                },
                "empty id",
            ),
            (
                RouteConfig {
                    id: "empty".into(),
                    legs: vec![],
                    on_complete: RouteCompletion::Loop,
                },
                "no [[route.leg]]",
            ),
            (
                RouteConfig {
                    id: "blank_anchor".into(),
                    legs: vec![RouteLeg {
                        anchor: "".into(),
                        speed: 0.5,
                        hold_secs: 0,
                    }],
                    on_complete: RouteCompletion::Loop,
                },
                "empty anchor",
            ),
            (
                RouteConfig {
                    id: "too_fast".into(),
                    legs: vec![RouteLeg {
                        anchor: "a".into(),
                        speed: 1.5,
                        hold_secs: 0,
                    }],
                    on_complete: RouteCompletion::Loop,
                },
                "speed above 1.0",
            ),
        ];
        for (route, what) in cases {
            assert!(
                route.validate().is_err(),
                "a route with {what} must be a load error, not a civilian that \
                 silently never goes anywhere"
            );
        }
        assert!(depot_run().validate().is_ok(), "the exemplar is legal");
    }

    // ── AC2: orders share one evaluation path, and a malformed one is refused ──

    #[test]
    fn a_divert_naming_both_or_neither_destination_is_refused() {
        assert!(CivilianOrder::divert_to_route("depot_run")
            .validate()
            .is_ok());
        assert!(CivilianOrder::divert_to_anchor("holding_point")
            .validate()
            .is_ok());
        assert!(CivilianOrder::Divert {
            route: Some("a".into()),
            anchor: Some("b".into()),
        }
        .validate()
        .is_err());
        assert!(CivilianOrder::Divert {
            route: None,
            anchor: None,
        }
        .validate()
        .is_err());
        assert!(CivilianOrder::dock_at("").validate().is_err());
        assert!(CivilianOrder::Hold.validate().is_ok());
    }

    #[test]
    fn every_verb_reports_its_own_kind_for_disposition_lookup() {
        assert_eq!(CivilianOrder::Hold.kind(), OrderKind::Hold);
        assert_eq!(
            CivilianOrder::divert_to_route("r").kind(),
            OrderKind::Divert
        );
        assert_eq!(
            CivilianOrder::divert_to_anchor("a").kind(),
            OrderKind::Divert
        );
        assert_eq!(CivilianOrder::dock_at("s").kind(), OrderKind::Dock);
    }

    // ── AC3/AC4: the compliance machine, one transition at a time ──

    #[test]
    fn a_cooperative_civilian_walks_received_acknowledged_complying_on_authored_ticks() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = cooperative();
        let t = state
            .receive_order(CivilianOrder::Hold, &d, 0, HZ)
            .expect("taking an order is always a transition");
        assert_eq!(t.from, ComplianceState::Unordered);
        assert_eq!(t.to, ComplianceState::Received);
        assert_eq!(
            state.due_tick(),
            120,
            "two authored seconds at 60 Hz is 120 ticks"
        );

        assert!(
            state.advance(119, true, &d, HZ).is_none(),
            "nothing moves before the authored acknowledgement time"
        );
        assert_eq!(
            state.advance(120, true, &d, HZ).map(|t| t.to),
            Some(ComplianceState::Acknowledged)
        );
        assert_eq!(
            state.due_tick(),
            300,
            "…and the decide clock is three more authored seconds"
        );
        assert!(state.advance(299, true, &d, HZ).is_none());
        assert_eq!(
            state.advance(300, true, &d, HZ).map(|t| t.to),
            Some(ComplianceState::Complying)
        );
        assert_eq!(state.travel(), CivilianTravel::Hold, "and it holds station");
        assert!(
            state.advance(301, true, &d, HZ).is_none(),
            "complying is a resting state, not a loop that keeps transitioning"
        );
    }

    #[test]
    fn a_refusing_civilian_answers_with_its_authored_reason_and_keeps_flying_its_route() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = stubborn();
        ordered(&mut state, CivilianOrder::divert_to_anchor("far_side"), &d);
        let seen = run(&mut state, 400, true, &d);
        assert_eq!(
            seen,
            vec![ComplianceState::Refused],
            "a refusal is one transition out of received — it never acknowledges \
             its way to complying"
        );
        assert_eq!(
            state.reason(),
            Some("world.hauler.refuses"),
            "the reason is the authored string id, not one the code invented"
        );
        assert_eq!(
            state.travel(),
            CivilianTravel::Route {
                id: "depot_run".into()
            },
            "a refusal is a decision, so the civilian carries on with its own route"
        );
    }

    #[test]
    fn the_same_hull_complies_with_a_verb_it_does_not_refuse() {
        let mut state = CivilianState::default();
        let d = stubborn();
        ordered(&mut state, CivilianOrder::Hold, &d);
        let seen = run(&mut state, 400, true, &d);
        assert_eq!(
            seen,
            vec![ComplianceState::Acknowledged, ComplianceState::Complying],
            "disposition is per verb: this hull refuses diverts, not holds"
        );
    }

    #[test]
    fn a_complied_divert_onto_a_route_becomes_the_civilians_own_route_from_leg_zero() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = cooperative();
        state.observe_leg(1, Some(&depot_run()), 0, HZ);
        ordered(
            &mut state,
            CivilianOrder::divert_to_route("storm_detour"),
            &d,
        );
        run(&mut state, 400, true, &d);
        assert_eq!(state.route(), Some("storm_detour"));
        assert_eq!(state.leg(), 0, "a new lane is flown from its first leg");
        assert_eq!(
            state.travel(),
            CivilianTravel::Route {
                id: "storm_detour".into()
            }
        );
    }

    #[test]
    fn a_divert_to_a_bare_anchor_reaches_it_without_becoming_a_route() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = cooperative();
        ordered(
            &mut state,
            CivilianOrder::divert_to_anchor("holding_point"),
            &d,
        );
        run(&mut state, 400, true, &d);
        assert_eq!(
            state.travel(),
            CivilianTravel::Anchor {
                name: "holding_point".into()
            }
        );
        assert_eq!(
            state.route(),
            Some("depot_run"),
            "its own lane is still its lane — a holding point is not a route"
        );
    }

    #[test]
    fn a_dock_order_names_the_structure_for_the_adapter_to_close_on() {
        let mut state = CivilianState::default();
        let d = cooperative();
        ordered(&mut state, CivilianOrder::dock_at("skyhook_depot"), &d);
        run(&mut state, 400, true, &d);
        assert_eq!(
            state.travel(),
            CivilianTravel::Dock {
                structure: "skyhook_depot".into()
            }
        );
    }

    // ── AC6: unable to comply is its own state, not a silent stall ──

    #[test]
    fn an_order_whose_destination_never_resolves_lands_in_non_compliant_with_a_reason() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = cooperative();
        ordered(
            &mut state,
            CivilianOrder::dock_at("a_depot_that_is_gone"),
            &d,
        );
        let seen = run(&mut state, 400, false, &d);
        assert_eq!(
            seen,
            vec![ComplianceState::Acknowledged, ComplianceState::NonCompliant],
            "it agreed and then could not — which is not the same as refusing"
        );
        assert_eq!(state.reason(), Some(REASON_UNABLE));
        assert_eq!(
            state.travel(),
            CivilianTravel::Hold,
            "a stuck civilian stops where it is rather than wandering back onto \
             its lane as if nothing happened"
        );
        assert_ne!(
            state.compliance(),
            ComplianceState::Refused,
            "the whole point of the state is that a console can tell them apart"
        );
    }

    #[test]
    fn a_civilian_mid_order_that_loses_its_destination_falls_out_of_complying() {
        let mut state = CivilianState::default();
        let d = cooperative();
        ordered(&mut state, CivilianOrder::dock_at("skyhook_depot"), &d);
        run(&mut state, 400, true, &d);
        assert_eq!(state.compliance(), ComplianceState::Complying);
        assert_eq!(
            state.advance(401, false, &d, HZ).map(|t| t.to),
            Some(ComplianceState::NonCompliant),
            "the depot going away mid-approach is exactly the case the state exists for"
        );
        assert_eq!(
            state.advance(402, true, &d, HZ).map(|t| t.to),
            Some(ComplianceState::Complying),
            "…and it resumes on its own if the world puts it back, because the \
             crew never withdrew the order"
        );
    }

    // ── AC1 (per-leg behaviour): the cursor is the leg pointer, dwell is authored ──

    #[test]
    fn the_cruise_speed_tracks_the_leg_the_cursor_is_on() {
        let route = depot_run();
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        assert_eq!(
            state.cruise_speed(Some(&route), 0),
            0.4,
            "leg 0's authored speed"
        );
        state.observe_leg(1, Some(&route), 0, HZ);
        assert_eq!(
            state.leg(),
            1,
            "the cursor index is mirrored, not recomputed"
        );
        assert!(
            state.is_dwelling(0),
            "leg 0 authored a 10 s dwell, and leaving it starts the clock"
        );
        assert_eq!(
            state.cruise_speed(Some(&route), 0),
            0.0,
            "a dwelling civilian sits still — at zero throttle, on the same \
             directive, so its cursor keeps its place"
        );
        assert!(
            !state.is_dwelling(600),
            "…for exactly the authored ten seconds"
        );
        assert_eq!(
            state.cruise_speed(Some(&route), 600),
            default_leg_speed(),
            "and then flies leg 1 at leg 1's speed"
        );
    }

    #[test]
    fn a_leg_with_no_authored_dwell_flies_straight_through() {
        let route = depot_run();
        let mut state = CivilianState::default();
        state.observe_leg(1, Some(&route), 0, HZ);
        state.observe_leg(0, Some(&route), 700, HZ);
        assert!(
            !state.is_dwelling(700),
            "leg 1 authors no hold_secs, so wrapping past it starts no dwell"
        );
    }

    // ── AC5: the disposition is authored, including its absence ──

    #[test]
    fn an_unauthored_disposition_is_a_cooperative_one_and_authoring_is_per_verb() {
        let parsed: ComplianceDisposition = toml::from_str("").expect("an empty table is legal");
        assert_eq!(parsed, ComplianceDisposition::default());
        assert_eq!(parsed.response(OrderKind::Dock), OrderResponse::Comply);

        let authored: ComplianceDisposition = toml::from_str(
            r#"
ack_secs = 1
decide_secs = 1
dock = "refuse"
refusal_reason = "world.convoy.will_not_dock"
"#,
        )
        .expect("the vocabulary parses");
        assert_eq!(authored.response(OrderKind::Dock), OrderResponse::Refuse);
        assert_eq!(authored.response(OrderKind::Hold), OrderResponse::Comply);
        assert_eq!(authored.response(OrderKind::Divert), OrderResponse::Comply);
    }

    #[test]
    fn a_civilian_table_that_cannot_mean_anything_is_refused() {
        assert!(CivilianConfig::default().validate().is_ok());
        assert!(CivilianConfig {
            route: Some("   ".into()),
            ..CivilianConfig::default()
        }
        .validate()
        .is_err());
        assert!(CivilianConfig {
            route_priority: -1.0,
            ..CivilianConfig::default()
        }
        .validate()
        .is_err());
        assert!(CivilianConfig {
            compliance: Some(ComplianceDisposition {
                ack_secs: -1,
                ..ComplianceDisposition::default()
            }),
            ..CivilianConfig::default()
        }
        .validate()
        .is_err());
        assert!(CivilianConfig {
            order_options: vec![
                CivilianOrderOption {
                    id: "clear_lane".into(),
                    label: "world.test.clear_lane".into(),
                    order: CivilianOrder::Hold,
                },
                CivilianOrderOption {
                    id: "clear_lane".into(),
                    label: "world.test.clear_lane_again".into(),
                    order: CivilianOrder::divert_to_route("lee"),
                },
            ],
            ..CivilianConfig::default()
        }
        .validate()
        .is_err());
        assert!(CivilianConfig {
            order_options: vec![CivilianOrderOption {
                id: "bad".into(),
                label: "world.test.bad".into(),
                order: CivilianOrder::Divert {
                    route: None,
                    anchor: None,
                },
            }],
            ..CivilianConfig::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn a_civilian_table_parses_from_its_authored_shape() {
        let parsed: CivilianConfig = toml::from_str(
            r#"
route = "depot_run"
route_priority = 80.0
order_options = [
  { id = "storm_shelter", label = "world.test.storm_shelter", order = { verb = "divert", route = "storm_shelter_run" } },
]

[compliance]
divert = "refuse"
"#,
        )
        .expect("the vocabulary parses");
        assert_eq!(parsed.route.as_deref(), Some("depot_run"));
        assert_eq!(parsed.route_priority, 80.0);
        assert_eq!(parsed.order_options.len(), 1);
        assert_eq!(parsed.order_options[0].id, "storm_shelter");
        assert_eq!(
            parsed.order_options[0].order,
            CivilianOrder::divert_to_route("storm_shelter_run")
        );
        assert_eq!(
            parsed.compliance.expect("authored").divert,
            OrderResponse::Refuse
        );
        assert!(
            toml::from_str::<CivilianConfig>("rout = \"depot_run\"").is_err(),
            "a misspelled key is a load error, not a civilian with no route"
        );
    }

    // ── Housekeeping: the state survives a round trip and a cancellation ──

    #[test]
    fn the_live_state_round_trips_through_serde_for_the_snapshot_path() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = cooperative();
        ordered(&mut state, CivilianOrder::dock_at("skyhook_depot"), &d);
        run(&mut state, 400, true, &d);
        state.observe_leg(1, Some(&depot_run()), 400, HZ);

        let bytes = toml::to_string(&state).expect("serialises…");
        let back: CivilianState = toml::from_str(&bytes).expect("…and comes back");
        assert_eq!(back, state, "every field a resume needs must survive");
    }

    #[test]
    fn clearing_an_order_returns_the_civilian_to_its_own_route() {
        let mut state = CivilianState::from_config(&CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        });
        let d = cooperative();
        ordered(&mut state, CivilianOrder::Hold, &d);
        run(&mut state, 400, true, &d);
        assert_eq!(state.travel(), CivilianTravel::Hold);
        let t = state.clear_order().expect("there was an order to clear");
        assert_eq!(t.to, ComplianceState::Unordered);
        assert_eq!(
            state.travel(),
            CivilianTravel::Route {
                id: "depot_run".into()
            }
        );
        assert!(
            state.clear_order().is_none(),
            "clearing nothing is not a transition"
        );
    }

    #[test]
    fn a_new_order_replaces_one_still_in_flight() {
        let mut state = CivilianState::default();
        let d = cooperative();
        ordered(&mut state, CivilianOrder::Hold, &d);
        state.advance(120, true, &d, HZ);
        assert_eq!(state.compliance(), ComplianceState::Acknowledged);
        state.receive_order(CivilianOrder::dock_at("skyhook"), &d, 130, HZ);
        assert_eq!(
            state.compliance(),
            ComplianceState::Received,
            "the latest instruction is the operative one"
        );
        assert_eq!(state.order(), Some(&CivilianOrder::dock_at("skyhook")));
    }

    #[test]
    fn a_civilian_with_no_route_and_no_order_holds_station() {
        assert_eq!(CivilianState::default().travel(), CivilianTravel::Hold);
    }
}
