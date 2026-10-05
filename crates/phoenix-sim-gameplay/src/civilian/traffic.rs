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

// ── Authored order vocabulary ────────────────────────────────────────────────

/// Which of the three verbs an order is, for disposition lookup and for the
/// wire.
pub use phoenix_model::wire::OrderKind;

/// An order issued to one civilian.
///
/// Three verbs, matching the three things a Navigation officer actually needs to
/// say to traffic. `Divert` carries two mutually exclusive destinations rather
/// than splitting into two verbs, because "go somewhere else" is one instruction
/// whether the somewhere else is a whole lane or a single point;
/// [`CivilianOrder::validate`] refuses a divert that names both or neither.
pub use phoenix_model::wire::CivilianOrder;

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
    /// Override impulse travel for this civilian. Unset preserves the
    /// existing traffic default; a slow escort route can opt out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_impulse: Option<bool>,
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
#[path = "traffic_tests.rs"]
mod tests;

pub use phoenix_sim_contracts::routes::*;
