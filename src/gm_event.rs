//! GM-operable authored events: identity, registry and mission projection
//! (issues #1301 and #1302, PRD #930 milestone M2).
//!
//! # What an "event control" is
//!
//! An authored trigger becomes GM-operable only by declaring
//! [`GmEventControls`](crate::world::config::GmEventControls): a stable id, a
//! String Table label, and an explicit subset of the three levers Fire, Pause
//! and Skip. Missing controls are invisible and unavailable — the default for
//! every trigger every shipped world already authors.
//!
//! Two authoring surfaces declare one:
//!
//! * `gm_event(id, label, handler)` is the manual-only shorthand. It builds a
//!   [`TriggerCondition::Manual`](crate::world::config::TriggerCondition::Manual)
//!   trigger whose only possible cause is a GM Fire, and implies the Fire
//!   control without a redundant declaration (issue #1301).
//! * `<registration>.gm_controls(id, label)` attaches the SAME control set to an
//!   ORDINARY condition-bearing trigger, which keeps its automatic condition and
//!   also becomes GM-operable (issue #1302).
//!
//! * `.pauseable()` adds the persistent Pause lever to a control set either
//!   surface declared (issue #1303). Which events are paused RIGHT NOW is
//!   per-run authoritative state on
//!   [`WorldContentRuntime::paused_gm_events`](crate::world::server::WorldContentRuntime::paused_gm_events),
//!   not part of the control set: the set says which levers exist.
//! * `.skip()` adds the third lever, Skip-next, to a control set either
//!   surface declared (issue #1304). An armed Skip is consumed by the next
//!   AUTOMATIC occurrence that would actually have fired — the ordinary
//!   evaluator advances the ordinary lifecycle (a once-only event is spent, a
//!   repeating one enters its ordinary cooldown) and the fired record is
//!   dropped before handler dispatch, so nothing runs and the crew see
//!   nothing. It is deliberately orthogonal to the other two levers: a Fire
//!   consumes only
//!   [`crate::world::server::WorldContentRuntime::pending_gm_event_fires`], a
//!   Pause only toggles
//!   [`WorldContentRuntime::paused_gm_events`](crate::world::server::WorldContentRuntime::paused_gm_events),
//!   and neither touches
//!   [`WorldContentRuntime::pending_gm_event_skips`](crate::world::server::WorldContentRuntime::pending_gm_event_skips).
//!
//! Nothing that DECIDES anything about
//! a Fire reads `Trigger::condition` — not this module, not the `FireGmEvent`
//! admission, not the apply-tick revalidation, and not the eligibility gates of
//! the trigger pipeline's manual pass
//! ([`fire_manual_trigger`](crate::world::content::fire_manual_trigger) and
//! [`manual_fire_is_still_live`](crate::world::content::manual_fire_is_still_live),
//! which key off the once/repeat latch, the `when` predicate and the cooldown).
//! That is the point and not an accident: it is what makes "Fire bypasses the
//! condition and executes the ordinary handler and lifecycle" true by
//! construction rather than by a second code path that could drift from the
//! automatic one.
//!
//! The condition IS read at exactly one place on the manual path, and it has to
//! be: the `FiredTrigger` `fire_manual_trigger` returns names the condition's
//! subject through
//! [`entity_name_from_condition`](crate::world::content::entity_name_from_condition),
//! so a GM-fired `on_destroyed("courier", …)` is recorded against the courier
//! exactly as the automatic occurrence would have been (a
//! [`TriggerCondition::Manual`](crate::world::config::TriggerCondition::Manual)
//! event names nobody, because it is about nobody). "Bypasses the condition" is
//! a claim about what may CAUSE a firing, never about what the firing then says
//! happened — the record must not be able to tell a GM's Fire from the world's
//! own occurrence, and reading the subject off the condition is exactly what
//! keeps the two identical.
//!
//! # Identity is layer-qualified, and derived rather than stored
//!
//! Two layers may each author `breach_alarm` without colliding, so the id a GM
//! action names is `"<origin layer>::<authored id>"` — see
//! [`qualified_event_id`]. It is DERIVED from the trigger state's
//! `origin_layer` at read time rather than baked into the control struct at
//! compile time, because the same authored unit can be merged as a base world
//! in one run and as a layer in another, and only one of those two facts is
//! authored. Authored ids may not contain `::` (enforced by
//! [`GmEventControls::validate_authored`](crate::world::config::GmEventControls::validate_authored)),
//! so the join is unambiguous.
//!
//! # Determinism
//!
//! Everything here reads `WorldContentRuntime::triggers`, whose order is
//! a deterministic replay of the same load on every peer, and the pending-fire
//! set is a `BTreeSet`. No map iteration order, no wall clock and no
//! host-local gating reaches the projection or the pending set.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::world::config::GmEventControls;
use crate::world::content::TriggerState;

/// The layer name used to qualify an event authored by the base world.
///
/// The same spelling `tick_trigger_pipeline` already uses when it attributes a
/// base-world fire, so one vocabulary answers "where did this come from" in the
/// activity feed and in a GM action alike.
pub const BASE_WORLD_LAYER: &str = "base-world";

/// The one join between an origin layer and an authored id.
pub fn qualified_event_id(origin_layer: Option<&str>, authored_id: &str) -> String {
    format!(
        "{}::{}",
        origin_layer.unwrap_or(BASE_WORLD_LAYER),
        authored_id
    )
}

/// The qualified id of one trigger state, or `None` when it declares no GM
/// controls and is therefore not addressable by a GM action at all.
pub fn state_event_id(state: &TriggerState) -> Option<String> {
    state
        .trigger
        .gm_controls
        .as_ref()
        .map(|controls| qualified_event_id(state.origin_layer.as_deref(), &controls.id))
}

/// Which lever of the event-control family one durable result reports.
///
/// An absent `LoggedGmAction::lever` means this is not Skip: Fire and Pause
/// identify themselves through `LoggedGmAction::verb`. Other action families
/// carry neither field. An event-control fact carrying neither is malformed,
/// never an implicit Fire. Keeping Skip in its own optional field preserves
/// the pre-#1304 Fire/Pause wire shape and keeps every control in one
/// [`crate::gm_action::GmActionKind::EventControl`] result feed.
///
/// Variants are APPEND-ONLY: the durable result is postcard-encoded into the
/// deterministic digest, which writes an enum by VARIANT INDEX.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmEventLever {
    /// Arm the next matching automatic occurrence to advance the ordinary
    /// lifecycle WITHOUT running the handler (issue #1304).
    SkipNext,
}

/// One controllable event as the GM mission panel sees it.
///
/// Absolute: the panel never accumulates, so a reconnecting or restored GM sees
/// the same rows the live one does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmMissionEvent {
    /// Layer-qualified stable id — what a `FireGmEvent` action names.
    pub id: String,
    /// String Table id for the operator-facing label.
    pub label: String,
    /// Whether the Fire lever is declared.
    pub fire: bool,
    /// Whether the Pause lever is DECLARED (issue #1303) — authored config,
    /// fixed for the life of the trigger.
    pub pause: bool,
    /// Whether the event is paused RIGHT NOW (issue #1303) — per-run state a
    /// GM toggles. Always false for an event that declares no Pause lever,
    /// because nothing can put it in the set.
    pub paused: bool,
    /// Whether the Skip lever is declared (issue #1304).
    pub skip: bool,
    /// Whether the event is repeatable. A one-shot event is spent after one
    /// fire; a repeatable one is a reusable quick tool.
    pub repeatable: bool,
    /// The ordinary single-shot latch: a spent one-shot event cannot fire again.
    pub spent: bool,
    /// A Fire has been granted and applied but its handler has not run yet —
    /// normally the same tick, but a `when` predicate or a cooldown can hold it.
    pub armed: bool,
    /// A Skip has been granted and applied and the occurrence it will consume
    /// has not happened yet (issue #1304).
    ///
    /// Separate from [`Self::armed`] because the two levers are orthogonal and
    /// live on different clocks: a Fire arm is normally consumed the same tick,
    /// while a Skip arm waits for the world to produce the occurrence it
    /// stands in front of — which may be many minutes, or never.
    pub skip_armed: bool,
}

/// Absolute GM mission-panel projection: the controllable events, plus the
/// bounded attributed results of the event-control action family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GmMissionProjection {
    pub events: Vec<GmMissionEvent>,
    pub results: Vec<crate::gm_action::LoggedGmAction>,
    #[serde(default)]
    pub objective_palette: Vec<crate::gm_objective::ObjectiveRow>,
    #[serde(default)]
    pub objectives: Vec<crate::gm_objective::ObjectiveRow>,
    #[serde(default)]
    pub objective_results: Vec<crate::gm_action::LoggedGmAction>,
}

/// The complete controllable-event registry for one live world, in
/// `triggers` order.
///
/// A duplicate qualified id cannot reach here: the load-time validation pass
/// refuses a world that authors one within a layer, and the layer prefix
/// separates the rest. If one somehow did, [`find_event`] would resolve the
/// FIRST — deterministic on every peer rather than arbitrary.
pub fn controllable_events<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
    pending_fires: &std::collections::BTreeSet<String>,
    paused_events: &std::collections::BTreeSet<String>,
    pending_skips: &std::collections::BTreeSet<String>,
) -> Vec<GmMissionEvent> {
    states
        .into_iter()
        .filter_map(|state| {
            let controls = state.trigger.gm_controls.as_ref()?;
            let id = qualified_event_id(state.origin_layer.as_deref(), &controls.id);
            let armed = pending_fires.contains(&id);
            let paused = paused_events.contains(&id);
            let skip_armed = pending_skips.contains(&id);
            Some(GmMissionEvent {
                id,
                label: controls.label.clone(),
                fire: controls.fire,
                pause: controls.pause,
                skip: controls.skip,
                repeatable: state.trigger.repeat,
                spent: state.fired && !state.trigger.repeat,
                armed,
                paused,
                skip_armed,
            })
        })
        .collect()
}

/// Every qualified id the LIVE trigger table can still answer to.
///
/// The one producer of the "an arm that names no live trigger cannot ever be
/// honoured" set, shared by the Fire and Skip cleanups in
/// [`crate::world::server::tick_trigger_pipeline`] so the two levers can never
/// disagree about which ids a layer unload took with it.
pub fn live_event_ids<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
) -> std::collections::BTreeSet<String> {
    states.into_iter().filter_map(state_event_id).collect()
}

/// Resolve one qualified id to its `triggers` index.
pub fn find_event<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
    qualified: &str,
) -> Option<usize> {
    states
        .into_iter()
        .position(|state| state_event_id(state).as_deref() == Some(qualified))
}

/// Whether the named event exists AND declares the Fire lever.
///
/// This is the revalidation `apply_due_actions` performs at the agreed apply
/// tick rather than at request time: a layer carrying the event can be unloaded
/// between the two, and the answer must be the same on every peer.
pub fn fireable_index<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
    qualified: &str,
) -> Option<usize> {
    let (index, state) = states
        .into_iter()
        .enumerate()
        .find(|(_, state)| state_event_id(state).as_deref() == Some(qualified))?;
    state
        .trigger
        .gm_controls
        .as_ref()
        .is_some_and(GmEventControls::declares_fire)
        .then_some(index)
}

/// Whether the named event exists AND declares the Pause lever (issue #1303).
///
/// [`fireable_index`]'s twin, and revalidated at the same moment and for the
/// same reason: an event whose layer unloaded between the request and the apply
/// tick is not pausable, and every peer must say so at exactly the same tick.
/// Note what it does NOT consult — the once/repeat latch. A spent one-shot can
/// still be paused and unpaused, because Pause is a statement about whether the
/// condition is evaluated at all and a spent trigger is simply one whose
/// evaluation would decline anyway; making it a refusal would give the GM a
/// toggle that silently stops answering.
pub fn pausable_index<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
    qualified: &str,
) -> Option<usize> {
    let (index, state) = states
        .into_iter()
        .enumerate()
        .find(|(_, state)| state_event_id(state).as_deref() == Some(qualified))?;
    state
        .trigger
        .gm_controls
        .as_ref()
        .is_some_and(GmEventControls::declares_pause)
        .then_some(index)
}

/// Whether the named event exists AND declares the Skip lever (issue #1304).
///
/// [`fireable_index`]'s twin, and revalidated at the same agreed apply tick for
/// the same reason: the answer must be identical on every peer, and a layer
/// carrying the event can be unloaded between request and application. The two
/// levers are read separately rather than through one "is operable" predicate
/// because an author may declare either without the other, and a GM who presses
/// a button a scenario never authored must get a refusal, not the other lever.
pub fn skippable_index<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
    qualified: &str,
) -> Option<usize> {
    let (index, state) = states
        .into_iter()
        .enumerate()
        .find(|(_, state)| state_event_id(state).as_deref() == Some(qualified))?;
    state
        .trigger
        .gm_controls
        .as_ref()
        .is_some_and(GmEventControls::declares_skip)
        .then_some(index)
}

/// Page-local last-published mission projection. Presentation only: the
/// authoritative facts are the trigger table and the GM action journal.
#[derive(Resource, Clone, Debug, Default)]
pub struct LastGmMissionProjection(Option<GmMissionProjection>);

/// Build the absolute projection from live authoritative state.
pub fn projection<'a>(
    states: impl IntoIterator<Item = &'a TriggerState>,
    pending_fires: &std::collections::BTreeSet<String>,
    paused_events: &std::collections::BTreeSet<String>,
    pending_skips: &std::collections::BTreeSet<String>,
    log: &crate::gm_action::GmActionLog,
    refusals: &crate::gm_action::LocalGmActionRefusals,
) -> GmMissionProjection {
    GmMissionProjection {
        events: controllable_events(states, pending_fires, paused_events, pending_skips),
        objective_palette: Vec::new(),
        objectives: Vec::new(),
        objective_results: Vec::new(),
        results: crate::gm_action::projected_results(
            crate::gm_action::GmActionKind::EventControl,
            log,
            refusals,
        ),
    }
}

/// Push an absolute page-local projection whenever the controllable-event set,
/// its state, or the bounded result feed changes.
///
/// Frame-driven for [`crate::gm_action::publish_session_projection`]'s reason: a
/// paused session still has to report the result of a Fire that was refused at
/// admission.
pub fn publish_mission_projection(
    objectives: Option<Res<crate::world::server::ObjectiveManagerRes>>,
    ships: Query<&crate::entities::spawner::EntityUuid, With<crate::lockstep::FleetSlotOf>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    log: Res<crate::gm_action::GmActionLog>,
    refusals: Res<crate::gm_action::LocalGmActionRefusals>,
    mut last: ResMut<LastGmMissionProjection>,
    mut writer: MessageWriter<crate::console_bridge::GmMissionChanged>,
) {
    let empty_states = crate::world::trigger_registry::WorldTriggerRegistry::default();
    let empty_ids = std::collections::BTreeSet::new();
    let (states, fires, paused, skips) = match runtime.as_deref() {
        Some(runtime) => (
            &runtime.triggers,
            &runtime.pending_gm_event_fires,
            &runtime.paused_gm_events,
            &runtime.pending_gm_event_skips,
        ),
        None => (&empty_states, &empty_ids, &empty_ids, &empty_ids),
    };
    let mut next = projection(states, fires, paused, skips, &log, &refusals);
    let live: Vec<String> = ships.iter().map(|uuid| uuid.0.clone()).collect();
    (next.objective_palette, next.objectives) = crate::gm_objective::rows(
        runtime.as_deref(),
        objectives.as_deref().map(|o| &o.0),
        &live,
    );
    next.objective_results = crate::gm_action::projected_results(
        crate::gm_action::GmActionKind::ObjectiveControl,
        &log,
        &refusals,
    );
    if last.0.as_ref() == Some(&next) {
        return;
    }
    last.0 = Some(next.clone());
    writer.write(crate::console_bridge::GmMissionChanged { payload: next });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::{scripted_trigger, TriggerCondition};

    fn state(id: &str, layer: Option<&str>, repeat: bool, fired: bool) -> TriggerState {
        let mut trigger = scripted_trigger(TriggerCondition::Manual);
        trigger.id = Some(id.to_string());
        trigger.repeat = repeat;
        trigger.gm_controls = Some(GmEventControls::fire_only(
            id.to_string(),
            format!("world.gm.event.{id}"),
        ));
        TriggerState {
            trigger,
            fired,
            origin_layer: layer.map(str::to_string),
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }
    }

    #[test]
    fn the_same_authored_id_in_two_layers_is_two_qualified_events() {
        let states = vec![
            state("breach", None, false, false),
            state("breach", Some("assets/worlds/layer.toml"), false, false),
        ];
        let events = controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(
            events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["base-world::breach", "assets/worlds/layer.toml::breach"],
        );
        assert_eq!(find_event(&states, "base-world::breach"), Some(0));
        assert_eq!(
            find_event(&states, "assets/worlds/layer.toml::breach"),
            Some(1)
        );
        assert_eq!(find_event(&states, "base-world::missing"), None);
    }

    #[test]
    fn a_trigger_without_controls_is_invisible_and_unaddressable() {
        let mut plain = state("breach", None, false, false);
        plain.trigger.gm_controls = None;
        let states = vec![plain];
        assert!(controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .is_empty());
        assert_eq!(find_event(&states, "base-world::breach"), None);
        assert_eq!(fireable_index(&states, "base-world::breach"), None);
    }

    #[test]
    fn a_control_set_without_fire_is_not_fireable_but_is_still_listed() {
        let mut listed = state("breach", None, false, false);
        listed.trigger.gm_controls.as_mut().expect("controls").fire = false;
        let states = vec![listed];
        assert_eq!(
            controllable_events(
                &states,
                &Default::default(),
                &Default::default(),
                &Default::default(),
            )
            .len(),
            1
        );
        assert_eq!(find_event(&states, "base-world::breach"), Some(0));
        assert_eq!(fireable_index(&states, "base-world::breach"), None);
    }

    #[test]
    fn spent_reports_the_one_shot_latch_and_never_a_repeatable_one() {
        let states = vec![
            state("once", None, false, true),
            state("again", None, true, true),
        ];
        let events = controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        assert!(events[0].spent && !events[0].repeatable);
        assert!(!events[1].spent && events[1].repeatable);
    }

    /// Issue #1302: nothing in this module reads the condition, and this is the
    /// assertion that says so rather than leaving it to inspection. An ORDINARY
    /// condition-bearing trigger carrying the same control set is listed,
    /// resolved and fireable on exactly the same terms as a manual one — and a
    /// declaration WITHOUT Fire is still listed and still not fireable, whatever
    /// its condition.
    #[test]
    fn an_automatic_event_is_listed_and_fireable_on_the_same_terms_as_a_manual_one() {
        let automatic = |id: &str, fire: bool| {
            let mut st = state(id, None, false, false);
            st.trigger.condition = TriggerCondition::OnDestroyed {
                entity_name: "courier".to_string(),
            };
            st.trigger.gm_controls.as_mut().expect("controls").fire = fire;
            st
        };
        let states = vec![automatic("evac", true), automatic("silent", false)];

        let events = controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(
            events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["base-world::evac", "base-world::silent"],
        );
        assert_eq!(
            state_event_id(&states[0]).as_deref(),
            Some("base-world::evac"),
        );
        assert_eq!(fireable_index(&states, "base-world::evac"), Some(0));
        assert_eq!(
            fireable_index(&states, "base-world::silent"),
            None,
            "a listed automatic event without Fire is not fireable"
        );

        // And an automatic trigger that declares nothing is invisible, which is
        // every trigger every shipped world already authors.
        let mut plain = automatic("evac", true);
        plain.trigger.gm_controls = None;
        assert!(controllable_events(
            &[plain],
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .is_empty());
    }

    #[test]
    fn an_armed_fire_is_projected_until_the_pipeline_runs_it() {
        let states = vec![state("breach", None, false, false)];
        let pending = std::collections::BTreeSet::from(["base-world::breach".to_string()]);
        assert!(
            controllable_events(&states, &pending, &Default::default(), &Default::default(),)[0]
                .armed
        );
        assert!(
            !controllable_events(
                &states,
                &Default::default(),
                &Default::default(),
                &Default::default(),
            )[0]
            .armed
        );
    }

    /// Issue #1303: the Pause LEVER and the paused STATE are two facts, and
    /// the projection reports both. A control set that does not declare Pause
    /// is not pausable however the run has gone, which is the absent-control
    /// refusal expressed at its source.
    #[test]
    fn only_a_declared_pause_lever_is_pausable_and_paused_is_a_separate_fact() {
        let mut declared = state("breach", None, false, false);
        declared
            .trigger
            .gm_controls
            .as_mut()
            .expect("controls")
            .pause = true;
        // Fire and Pause are independent levers, so an event may declare Pause
        // alone: `pausable_index` must not reach for `declares_fire`.
        let mut pause_only = state("quiet", None, false, false);
        {
            let controls = pause_only.trigger.gm_controls.as_mut().expect("controls");
            controls.fire = false;
            controls.pause = true;
        }
        let states = vec![declared, pause_only, state("sweep", None, true, false)];

        assert_eq!(pausable_index(&states, "base-world::breach"), Some(0));
        assert_eq!(pausable_index(&states, "base-world::quiet"), Some(1));
        assert_eq!(
            pausable_index(&states, "base-world::sweep"),
            None,
            "an event that declares no Pause lever exposes no toggle"
        );
        assert_eq!(pausable_index(&states, "base-world::missing"), None);
        assert_eq!(
            fireable_index(&states, "base-world::quiet"),
            None,
            "and declaring Pause does not quietly add Fire"
        );

        let paused = std::collections::BTreeSet::from(["base-world::breach".to_string()]);
        let events =
            controllable_events(&states, &Default::default(), &paused, &Default::default());
        assert_eq!(
            events
                .iter()
                .map(|e| (e.pause, e.paused))
                .collect::<Vec<_>>(),
            vec![(true, true), (true, false), (false, false)],
            "the lever is authored; the engaged state belongs to the run"
        );
    }

    /// A spent one-shot is still pausable (issue #1303). Pause decides whether
    /// the condition is EVALUATED, and a GM offered a toggle that silently
    /// stopped answering once the event had fired would have no way to tell
    /// that apart from a broken control.
    #[test]
    fn a_spent_one_shot_event_is_still_pausable() {
        let mut spent = state("breach", None, false, true);
        spent.trigger.gm_controls.as_mut().expect("controls").pause = true;
        let states = vec![spent];
        assert!(
            controllable_events(
                &states,
                &Default::default(),
                &Default::default(),
                &Default::default(),
            )[0]
            .spent
        );
        assert_eq!(pausable_index(&states, "base-world::breach"), Some(0));
    }

    /// Issue #1304: the Skip lever is declared, resolved and projected
    /// independently of Fire. A control set may carry either, both or neither,
    /// and the panel must be able to tell which — an event that declares only
    /// Skip is listed and skippable but NOT fireable, and the reverse.
    #[test]
    fn the_skip_lever_is_declared_and_resolved_independently_of_fire() {
        let lever = |id: &str, fire: bool, skip: bool| {
            let mut st = state(id, None, false, false);
            st.trigger.condition = TriggerCondition::OnDestroyed {
                entity_name: "courier".to_string(),
            };
            let controls = st.trigger.gm_controls.as_mut().expect("controls");
            controls.fire = fire;
            controls.skip = skip;
            st
        };
        let states = vec![
            lever("both", true, true),
            lever("fire_only", true, false),
            lever("skip_only", false, true),
        ];

        let events = controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(
            events
                .iter()
                .map(|event| (event.fire, event.skip))
                .collect::<Vec<_>>(),
            vec![(true, true), (true, false), (false, true)],
            "the panel is told exactly which levers each event declares"
        );

        assert_eq!(skippable_index(&states, "base-world::both"), Some(0));
        assert_eq!(
            skippable_index(&states, "base-world::fire_only"),
            None,
            "a listed event without Skip is not skippable"
        );
        assert_eq!(skippable_index(&states, "base-world::skip_only"), Some(2));
        assert_eq!(
            fireable_index(&states, "base-world::skip_only"),
            None,
            "and declaring Skip does not imply Fire"
        );
        assert_eq!(skippable_index(&states, "base-world::missing"), None);

        // A trigger that declares nothing is unaddressable by either lever.
        let mut plain = lever("both", true, true);
        plain.trigger.gm_controls = None;
        assert_eq!(skippable_index(&[plain], "base-world::both"), None);
    }

    /// The two arms are projected from two sets, so an event can be armed on
    /// one lever, the other, both or neither — which is what makes "Fire does
    /// not consume an armed Skip" visible on the panel rather than merely true
    /// in the simulation.
    #[test]
    fn an_armed_skip_is_projected_beside_an_armed_fire_and_never_instead_of_it() {
        let states = vec![state("breach", None, false, false)];
        let armed = std::collections::BTreeSet::from(["base-world::breach".to_string()]);
        let none = std::collections::BTreeSet::new();

        let neither = &controllable_events(&states, &none, &none, &none)[0];
        assert!(!neither.armed && !neither.skip_armed);

        let skip_only = &controllable_events(&states, &none, &none, &armed)[0];
        assert!(!skip_only.armed && skip_only.skip_armed);

        let fire_only = &controllable_events(&states, &armed, &none, &none)[0];
        assert!(fire_only.armed && !fire_only.skip_armed);

        let both = &controllable_events(&states, &armed, &none, &armed)[0];
        assert!(both.armed && both.skip_armed);
    }

    /// The one live-id set both cleanups read. An event with no control set has
    /// no qualified id at all, so it can never appear here — which is what
    /// keeps a Skip arm from being retained against a trigger no GM can name.
    #[test]
    fn live_event_ids_names_every_addressable_event_and_nothing_else() {
        let mut plain = state("silent", None, false, false);
        plain.trigger.gm_controls = None;
        let states = vec![
            state("breach", None, false, false),
            state("breach", Some("assets/worlds/layer.toml"), false, false),
            plain,
        ];
        assert_eq!(
            live_event_ids(&states).into_iter().collect::<Vec<_>>(),
            vec![
                "assets/worlds/layer.toml::breach".to_string(),
                "base-world::breach".to_string(),
            ],
        );
    }
}
