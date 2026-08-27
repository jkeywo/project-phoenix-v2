//! Bevy adapter for infrastructure condition + capacity (issue #1025).
//!
//! One component, one fixed-tick system, and a mirror into the world flag
//! store. The arithmetic and every threshold edge live in the pure sibling
//! [`super::condition`]; nothing here decides anything a unit test cannot
//! already reach.
//!
//! # The one write site
//!
//! Condition moves for three reasons — authored decay, damage the entity took,
//! and a scripted repair or hit — and all three land in
//! [`tick_infrastructure_condition`]. That is deliberate: a threshold crossing
//! is only observable if the code that crosses it is also the code that mirrors
//! the resulting flag, so scripted adjustments are *queued* on the
//! `EffectQueue<ConditionAdjustment>` resource (issue #1223; formerly a
//! `pending_*` field on `WorldContentRuntime`) and drained here rather than
//! written where they are authored. `#1027`'s field-repair operation feeds the
//! same queue, one slice of progress per tick.
//!
//! # Where the flags become observable
//!
//! Each crossing is written into the base-world [`FlagStore`] and pushed onto
//! `WorldContentRuntime::pending_world_events` as a `FlagSet` / `FlagCleared`.
//! `collect_world_events` drains that queue at the top of the next tick's
//! `SimSet::Physics`, so an `on_flag_set` / `on_flag_cleared` hook fires one
//! tick after the crossing — the same one-tick bridge `WaypointReached` already
//! rides, and for the same reason: this system runs in `SimSet::Modifiers`,
//! after the collector has already run for the tick.
//!
//! [`FlagStore`]: crate::world::flags::FlagStore

use bevy::prelude::*;

use crate::authoritative::{DeclareState, StateClass};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntitySystemHull, EntityUuid};
use crate::infrastructure::condition::{
    CapacityAdjustment, ConditionAdjustment, FlagChange, InfrastructureState,
};
use crate::logging::LogFilterConfig;
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

/// Present when the entity's TOML declared an `[infrastructure]` table.
///
/// Authoritative per-entity simulation state: it decides whether a structure
/// can still transfer, dock or hold, and two hosts that disagreed about it
/// would disagree about whether a mission is winnable.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct InfrastructureCondition(pub InfrastructureState);

/// Registers the condition tick. Added by `WorldPlugin` — the flag store and
/// the world-event queue this writes to are its resources.
pub struct InfrastructurePlugin;

impl Plugin for InfrastructurePlugin {
    fn build(&self, app: &mut App) {
        // The two per-owner effect queues `tick_infrastructure_condition` drains
        // (issue #1223), registered and declared at this owning site. Each is a
        // transient inter-system queue — drained in full every tick, empty at
        // every fold/snapshot boundary — so `ClearedAtFold`. They key distinctly
        // in the census by full type path (`EffectQueue<ConditionAdjustment>` vs
        // `EffectQueue<CapacityAdjustment>`).
        app.init_resource::<EffectQueue<ConditionAdjustment>>()
            .init_resource::<EffectQueue<CapacityAdjustment>>()
            .declare_state::<EffectQueue<ConditionAdjustment>>(
                StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            )
            .declare_state::<EffectQueue<CapacityAdjustment>>(
                StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            );
        app.add_systems(
            FixedUpdate,
            tick_infrastructure_condition.in_set(crate::sim_sets::SimSet::Modifiers),
        );
    }
}

/// Advance every infrastructure track by one logical tick.
///
/// Per structure, in UUID order: publish its starting flags before any mutation
/// on the first tick it exists, apply this tick's queued capacity moves, publish
/// its capacities, fold in any hull it lost, apply authored decay, then apply
/// queued condition adjustments. Every
/// operational flag that changed along the way is mirrored into the base-world
/// flag store, and every capacity that moved is re-published onto its counter.
///
/// Capacity moves (issue #1027) queue here rather than being written onto the
/// component by whoever decided them, for exactly the reason condition moves
/// do: this is the one system that mirrors a structure's published numbers into
/// the scenario's flag store. A `transfer` writing the component directly would
/// move the goods and leave every script predicate reading the old count.
///
/// UUID order, not query order: Bevy's archetype iteration order is not part of
/// the simulation's contract, and two structures sharing a flag name would
/// otherwise resolve differently on two hosts. Same rule
/// [`crate::sim_digest`] applies to its own walks.
pub fn tick_infrastructure_condition(
    runtime: Option<ResMut<WorldContentRuntime>>,
    // The two script/console effect queues, extracted off `WorldContentRuntime`
    // (issue #1223). This is their owning drain.
    mut condition_queue: ResMut<EffectQueue<ConditionAdjustment>>,
    mut capacity_queue: ResMut<EffectQueue<CapacityAdjustment>>,
    time: Option<Res<Time>>,
    mut structures: Query<(
        Entity,
        &EntityUuid,
        Option<&EntitySystemHull>,
        &mut InfrastructureCondition,
    )>,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    // A read (`Deref`, not `DerefMut`), so a world with no infrastructure and no
    // queued work marks neither the runtime nor either queue changed.
    if structures.is_empty() && condition_queue.0.is_empty() && capacity_queue.0.is_empty() {
        return;
    }
    let queued = std::mem::take(&mut condition_queue.0);
    // Taken through `DerefMut` only when there is something in it, so a world
    // whose structures never trade does not mark the queue changed each tick.
    let queued_capacity = if capacity_queue.0.is_empty() {
        Vec::new()
    } else {
        std::mem::take(&mut capacity_queue.0)
    };
    let delta_secs = time.map(|t| t.delta_secs()).unwrap_or(0.0);

    let mut rows: Vec<(String, Entity)> = structures
        .iter()
        .map(|(entity, uuid, _, _)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (uuid, entity) in rows {
        let Ok((_, _, hull, mut condition)) = structures.get_mut(entity) else {
            continue;
        };
        let first_tick = condition.is_added();
        let hull_total = hull.map(|h| h.0.total_current());
        // Immutable reads through `Mut`'s `Deref`, so deciding there is nothing
        // to do costs no change-detection mark.
        let hull_moved = hull_total.is_some() && condition.0.last_observed_hull() != hull_total;
        let decay = condition.0.decay_per_sec() * delta_secs;
        let adjustments: Vec<f32> = queued
            .iter()
            .filter(|a| a.uuid == uuid)
            .map(|a| a.delta)
            .collect();
        let capacity_moves: Vec<(&str, i64)> = queued_capacity
            .iter()
            .filter(|a| a.uuid == uuid)
            .map(|a| (a.capacity.as_str(), a.delta))
            .collect();
        if !first_tick
            && !hull_moved
            && decay <= 0.0
            && adjustments.is_empty()
            && capacity_moves.is_empty()
        {
            continue;
        }

        let mut changes: Vec<FlagChange> = Vec::new();
        let capacities: Vec<(String, i64)>;
        {
            let state = &mut condition.0;
            // Initial truth must be mirrored before a first-tick mutation. A
            // full capacity drained on its spawn tick is a real trueâ†’false
            // edge, not a structure that merely appeared false.
            if first_tick {
                changes.extend(state.initial_flags());
            }
            for (id, delta) in capacity_moves {
                match state.adjust_capacity(id, delta) {
                    Some((_, capacity_changes)) => changes.extend(capacity_changes),
                    None => {
                        crate::pwarn!(
                            log,
                            crate::logging::LogCat::World,
                            entity = entity,
                            "capacity move for '{id}' on {uuid}: this structure declares no such \
                             capacity — ignoring"
                        );
                    }
                }
            }
            // Re-published whenever anything about this structure moved, not
            // only on its first tick: since #1027 the level is live, and a
            // counter that only ever carried the authored number would tell a
            // scenario predicate the depot was still full after it had been
            // emptied.
            capacities = state
                .capacities()
                .iter()
                .map(|c| (c.id.clone(), c.level))
                .collect();
            if let Some(total) = hull_total {
                changes.extend(state.observe_hull(total));
            }
            if decay > 0.0 {
                changes.extend(state.degrade(decay));
            }
            for delta in adjustments {
                changes.extend(state.apply_delta(delta));
            }
        }

        // A capacity is a published quantity, not an operational flag by
        // default: its counter is readable from a script predicate, but a plain
        // move deliberately fires no `on_flag_set`. A threshold explicitly
        // naming that capacity is the opt-in operational edge, mirrored with
        // the condition-backed changes collected above.
        for (id, amount) in capacities {
            runtime.flags.set_flag_value(&id, amount);
        }
        mirror_flags(&mut runtime, &changes, &uuid, &log);
    }
}

/// Write each changed operational flag into the base-world store and queue the
/// matching world event.
///
/// The transition is decided from the store's own `(before, after)` rather than
/// from `FlagChange::raised`, so a flag two structures share does not emit a
/// second `FlagSet` for a value that was already up.
fn mirror_flags(
    runtime: &mut WorldContentRuntime,
    changes: &[FlagChange],
    uuid: &str,
    log: &Option<Res<LogFilterConfig>>,
) {
    for change in changes {
        let (before, after) = if change.raised {
            runtime.flags.set_flag(&change.flag)
        } else {
            runtime.flags.clear_flag(&change.flag)
        };
        if (before != 0) == (after != 0) {
            continue;
        }
        crate::pdebug!(
            log,
            crate::logging::LogCat::World,
            "infrastructure {uuid}: {} -> {}",
            change.flag,
            change.raised
        );
        runtime.pending_world_events.push(if after != 0 {
            WorldEvent::FlagSet {
                name: change.flag.clone(),
                origin_layer: None,
            }
        } else {
            WorldEvent::FlagCleared {
                name: change.flag.clone(),
                origin_layer: None,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::condition::{
        CapacityConfig, ConditionAdjustment, InfrastructureConfig, ThresholdConfig,
    };

    const FLAG: &str = "depot_transfer_capable";

    fn depot_config(decay_per_sec: f32) -> InfrastructureConfig {
        InfrastructureConfig {
            condition_max: 100.0,
            decay_per_sec,
            capacities: vec![CapacityConfig {
                label: None,
                id: "depot_transfer_throughput".to_string(),
                amount: 40,
                ceiling: None,
            }],
            thresholds: vec![ThresholdConfig {
                label: None,
                flag: FLAG.to_string(),
                capacity: None,
                fails_below: 0.4,
                restores_above: None,
            }],
            ..Default::default()
        }
    }

    /// A bare app with the one system under test, ticked by hand. No
    /// `TimePlugin`: a decay-free fixture must not depend on a clock, and the
    /// decay tests insert `Time` themselves.
    fn app_with(config: &InfrastructureConfig) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.init_resource::<EffectQueue<ConditionAdjustment>>();
        app.init_resource::<EffectQueue<CapacityAdjustment>>();
        app.add_systems(Update, tick_infrastructure_condition);
        let entity = app
            .world_mut()
            .spawn((
                EntityUuid("depot-1".to_string()),
                InfrastructureCondition(InfrastructureState::from_config(config)),
            ))
            .id();
        (app, entity)
    }

    fn flag(app: &App, name: &str) -> bool {
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag(name)
    }

    fn counter(app: &App, name: &str) -> i64 {
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter(name)
    }

    fn drain_events(app: &mut App) -> Vec<WorldEvent> {
        std::mem::take(
            &mut app
                .world_mut()
                .resource_mut::<WorldContentRuntime>()
                .pending_world_events,
        )
    }

    // ── AC3: the flag surface ──

    #[test]
    fn a_structure_publishes_its_flags_and_capacities_on_its_first_tick() {
        let (mut app, _) = app_with(&depot_config(0.0));
        app.update();
        assert!(
            flag(&app, FLAG),
            "an intact depot's operational flag is up in the world store, where a script \
             predicate can read it"
        );
        assert_eq!(
            counter(&app, "depot_transfer_throughput"),
            40,
            "…and its authored capacity is a readable counter, so a scenario asks the depot \
             how much it moves instead of restating the number"
        );
        let events = drain_events(&mut app);
        assert_eq!(
            events,
            vec![WorldEvent::FlagSet {
                name: FLAG.to_string(),
                origin_layer: None,
            }],
            "exactly one world event: the flag going up. The capacity counter deliberately \
             fires none — a published quantity is not an operational event."
        );
    }

    #[test]
    fn a_crossing_writes_the_store_and_queues_the_event_a_hook_reacts_to() {
        let (mut app, entity) = app_with(&depot_config(0.0));
        app.update();
        drain_events(&mut app);

        app.world_mut()
            .resource_mut::<EffectQueue<ConditionAdjustment>>()
            .0
            .push(ConditionAdjustment {
                uuid: "depot-1".to_string(),
                delta: -65.0,
            });
        app.update();

        assert!(
            !flag(&app, FLAG),
            "crossing the authored threshold clears the flag in the world store"
        );
        assert_eq!(
            drain_events(&mut app),
            vec![WorldEvent::FlagCleared {
                name: FLAG.to_string(),
                origin_layer: None,
            }],
            "…and queues the FlagCleared a scenario's on_flag_cleared hook fires from"
        );

        app.world_mut()
            .resource_mut::<EffectQueue<ConditionAdjustment>>()
            .0
            .push(ConditionAdjustment {
                uuid: "depot-1".to_string(),
                delta: 20.0,
            });
        app.update();
        assert!(flag(&app, FLAG), "and a repair puts it back");
        assert_eq!(
            drain_events(&mut app),
            vec![WorldEvent::FlagSet {
                name: FLAG.to_string(),
                origin_layer: None,
            }],
            "…with the matching FlagSet — the flag flips in BOTH directions"
        );
        let condition = app
            .world()
            .get::<InfrastructureCondition>(entity)
            .expect("the component is still attached");
        assert_eq!(condition.0.condition(), 55.0);
    }

    #[test]
    fn a_capacity_threshold_crossing_queues_the_same_authoritative_flag_event() {
        let config = InfrastructureConfig {
            capacities: vec![CapacityConfig {
                id: "reserve_fuel".to_string(),
                amount: 0,
                ceiling: Some(100),
                label: None,
            }],
            thresholds: vec![ThresholdConfig {
                flag: "transfer_primed".to_string(),
                capacity: Some("reserve_fuel".to_string()),
                fails_below: 0.5,
                restores_above: Some(0.5),
                label: None,
            }],
            ..Default::default()
        };
        let (mut app, _) = app_with(&config);
        app.update();
        assert!(!flag(&app, "transfer_primed"));
        assert!(drain_events(&mut app).is_empty());

        app.world_mut()
            .resource_mut::<EffectQueue<CapacityAdjustment>>()
            .0
            .push(CapacityAdjustment {
                uuid: "depot-1".to_string(),
                capacity: "reserve_fuel".to_string(),
                delta: 50,
            });
        app.update();

        assert_eq!(counter(&app, "reserve_fuel"), 50);
        assert!(flag(&app, "transfer_primed"));
        assert_eq!(
            drain_events(&mut app),
            vec![WorldEvent::FlagSet {
                name: "transfer_primed".to_string(),
                origin_layer: None,
            }],
            "the receiving entity's own threshold supplies the on_flag_set seam"
        );
    }

    #[test]
    fn a_first_tick_capacity_drain_publishes_both_sides_of_the_edge() {
        let config = InfrastructureConfig {
            capacities: vec![CapacityConfig {
                id: "reserve_fuel".to_string(),
                amount: 100,
                ceiling: Some(100),
                label: None,
            }],
            thresholds: vec![ThresholdConfig {
                flag: "transfer_primed".to_string(),
                capacity: Some("reserve_fuel".to_string()),
                fails_below: 0.5,
                restores_above: Some(0.5),
                label: None,
            }],
            ..Default::default()
        };
        let (mut app, _) = app_with(&config);
        app.world_mut()
            .resource_mut::<EffectQueue<CapacityAdjustment>>()
            .0
            .push(CapacityAdjustment {
                uuid: "depot-1".to_string(),
                capacity: "reserve_fuel".to_string(),
                delta: -100,
            });

        app.update();

        assert_eq!(counter(&app, "reserve_fuel"), 0);
        assert!(!flag(&app, "transfer_primed"));
        assert_eq!(
            drain_events(&mut app),
            vec![
                WorldEvent::FlagSet {
                    name: "transfer_primed".to_string(),
                    origin_layer: None,
                },
                WorldEvent::FlagCleared {
                    name: "transfer_primed".to_string(),
                    origin_layer: None,
                },
            ],
            "initial truth is published before the same tick's drain, preserving the exact edge"
        );
    }

    #[test]
    fn a_queued_adjustment_for_an_unknown_entity_is_simply_not_applied() {
        let (mut app, entity) = app_with(&depot_config(0.0));
        app.update();
        app.world_mut()
            .resource_mut::<EffectQueue<ConditionAdjustment>>()
            .0
            .push(ConditionAdjustment {
                uuid: "no-such-depot".to_string(),
                delta: -90.0,
            });
        app.update();
        let condition = app.world().get::<InfrastructureCondition>(entity).unwrap();
        assert_eq!(
            condition.0.condition(),
            100.0,
            "an adjustment naming an entity that is not there must not land on whichever \
             structure happens to be first"
        );
        assert!(
            app.world()
                .resource::<EffectQueue<ConditionAdjustment>>()
                .0
                .is_empty(),
            "…and the queue is drained regardless, so a stale name cannot accumulate"
        );
    }

    // ── Decay ──

    #[test]
    fn authored_decay_walks_a_structure_down_through_its_threshold() {
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.init_resource::<EffectQueue<ConditionAdjustment>>();
        app.init_resource::<EffectQueue<CapacityAdjustment>>();
        app.insert_resource(Time::<()>::default());
        app.add_systems(Update, tick_infrastructure_condition);
        app.world_mut().spawn((
            EntityUuid("depot-1".to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&depot_config(10.0))),
        ));
        // One second per update, so ten condition points a tick.
        for _ in 0..7 {
            app.world_mut()
                .resource_mut::<Time<()>>()
                .advance_by(std::time::Duration::from_secs(1));
            app.update();
        }
        assert!(
            !flag(&app, FLAG),
            "seventy points of authored decay takes a depot below its 40 % threshold with no \
             damage and no script involved"
        );
    }

    #[test]
    fn a_structure_with_nothing_to_do_leaves_the_runtime_unmarked() {
        let (mut app, _) = app_with(&depot_config(0.0));
        app.update();
        app.update();
        let changed = app
            .world()
            .resource_ref::<WorldContentRuntime>()
            .is_changed();
        assert!(
            !changed,
            "a static structure on a quiet tick must not mark WorldContentRuntime changed — \
             every world in the repo carries that resource, and a needless mark is a needless \
             wake-up for everything that watches it"
        );
    }

    // ── Determinism ──

    #[test]
    fn structures_are_walked_in_uuid_order_whatever_order_they_spawned_in() {
        let mut config = depot_config(0.0);
        config.thresholds[0].flag = "shared_flag".to_string();
        config.capacities.clear();
        let mut forward = App::new();
        forward.init_resource::<WorldContentRuntime>();
        forward.init_resource::<EffectQueue<ConditionAdjustment>>();
        forward.init_resource::<EffectQueue<CapacityAdjustment>>();
        forward.add_systems(Update, tick_infrastructure_condition);
        for uuid in ["depot-a", "depot-b", "depot-c"] {
            forward.world_mut().spawn((
                EntityUuid(uuid.to_string()),
                InfrastructureCondition(InfrastructureState::from_config(&config)),
            ));
        }
        forward.update();

        let mut reverse = App::new();
        reverse.init_resource::<WorldContentRuntime>();
        reverse.init_resource::<EffectQueue<ConditionAdjustment>>();
        reverse.init_resource::<EffectQueue<CapacityAdjustment>>();
        reverse.add_systems(Update, tick_infrastructure_condition);
        for uuid in ["depot-c", "depot-b", "depot-a"] {
            reverse.world_mut().spawn((
                EntityUuid(uuid.to_string()),
                InfrastructureCondition(InfrastructureState::from_config(&config)),
            ));
        }
        reverse.update();

        assert_eq!(
            drain_events(&mut forward),
            drain_events(&mut reverse),
            "the emitted event sequence is a function of the UUIDs, not of the order the \
             entities happen to sit in the archetype — two hosts that spawned the same \
             structures in different orders must agree"
        );
    }
}
