use crate::core::broadcast::sim::SimProducer;
use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::core::messages::{PowerGroupId, ServerMessage, SystemId};
use crate::modifiers::power_system::{
    PendingGroupPower, PowerSystem, ScriptedPowerLevel, HELM_POWER_GROUP, SHIELDS_POWER_GROUP,
    WEAPONS_POWER_GROUP,
};
use crate::ship_plugin::CoordinationEnqueue;

// ── Resources ──────────────────────────────────────────────────────────────────

// ── AI policy (issue #784) ──────────────────────────────────────────────────

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipPowerPlugin;

impl Plugin for ShipPowerPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::DeclareState;
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_power_messages` reads
        // the `power-reactor` system's admitted commands.
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::POWER_REACTOR_KIND,
            crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID,
        ).with_feedback(crate::command_admission::FeedbackAddress::MatcherSpelling, &[crate::core::messages::SystemControlPayloadDiscriminants::SetPowerGroupAllocation]));
        app.init_resource::<crate::core::messages::InterSystemQueue>()
            .add_message::<CoordinationEnqueue>();
        // The scripted power-order queue `drain_scripted_power_orders` drains
        // (issues #1223, #1398), registered and declared at this owning site —
        // it was the captain plugin's `EffectQueue<(String, bool)>` while
        // restraint was a hold. A transient inter-system queue — drained in
        // full every tick, empty at every fold/snapshot boundary — so
        // `ClearedAtFold`.
        app.init_resource::<crate::effect_queue::EffectQueue<PendingGroupPower>>()
            .declare_state::<crate::effect_queue::EffectQueue<PendingGroupPower>>(
                crate::authoritative::StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            );
        app.insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .add_systems(
                FixedUpdate,
                (
                    // In `SimSet::Physics`, not Input (issue #831, mirroring
                    // shields #826): `admit_system_commands` clears every
                    // ship's `AdmittedCommands` before Input each tick, and the
                    // AI decide system (`console_ai::server::ai_power_allocation`,
                    // Physics) refills it same-tick via `validate_and_admit` —
                    // so the applier must consume in Physics *after* the AI emit
                    // or AI power commands would be silently lost. `ConsoleAiPlugin`
                    // declares the explicit
                    // `ai_power_allocation.before(handle_power_messages)` edge.
                    //
                    // `tick_power_system` is ALSO in Physics and also takes
                    // `&mut ShipPowerSystem`, so set membership alone leaves
                    // their order unspecified. The explicit
                    // `.before(tick_power_system)` edge restores the guarantee
                    // the old Input placement gave for free: a same-tick
                    // reallocation is applied before this tick's battery
                    // integration reads `total()`. (Unlike shields, whose
                    // `tick_shields` lives in the later `Modifiers` set, so set
                    // ordering sufficed there.)
                    handle_power_messages
                        .in_set(crate::sim_sets::FixedStep::HandlePowerMessages)
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .before(tick_power_system),
                    tick_power_system
                        .in_set(crate::sim_sets::FixedStep::TickPowerSystem)
                        .in_set(crate::sim_sets::SimSet::Physics),
                    tick_power_brownout_advisory
                        .in_set(crate::sim_sets::FixedStep::TickPowerBrownoutAdvisory)
                        .in_set(crate::sim_sets::SimSet::Modifiers),
                    // The scenario's half of the restraint lever, and its
                    // mirror (issue #1398). Both in `Modifiers`, chained: a
                    // scripted order lands and is mirrored in the same tick, so
                    // an `on_flag_set` handler chaining off it fires on the next
                    // pipeline pass exactly as an Engineering officer's order
                    // does.
                    (
                        drain_scripted_power_orders
                            .in_set(crate::sim_sets::FixedStep::DrainScriptedPowerOrders),
                        mirror_weapons_cold_flags
                            .in_set(crate::sim_sets::FixedStep::MirrorWeaponsColdFlags),
                    )
                        .chain()
                        .in_set(crate::sim_sets::SimSet::Modifiers),
                    // Distinct registered keys; tests/publisher_ordering.rs checks
                    // the exact shared access and ordinary/opposed-order outputs.
                    publish_power_blackboard
                        .in_set(crate::sim_sets::FixedStep::PublishPowerBlackboard)
                        .in_set(crate::sim_sets::SimSet::Publish)
                        .ambiguous_with(crate::ship::shields::publish_shields_blackboard)
                        .ambiguous_with(crate::console::repair::server::publish_repair_blackboard),
                ),
            )
            .add_plugins(power_state_broadcaster());
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `PowerState` producer.
///
/// Broadcasts `PowerState` at 10 Hz to the `Power` console holder only.
/// This is the canonical registration; it is added by [`ShipPowerPlugin`]
/// and also by the test harness in `test_app()`.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipPowerSystem` component
/// on the LocalShip entity, falling back to the global `ShipPowerSystem`
/// resource for test harnesses that only insert the Resource form.
pub fn power_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::for_producer(SimProducer::Power).register(
        Audience::HoldingSystem(SystemId("power-reactor".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            // Prefer per-entity component on the LocalShip; fall back to the
            // global Resource for tests that only initialise the Resource.
            let mut q =
                world.query_filtered::<&ShipPowerSystem, With<crate::server_app::LocalShip>>();
            let power_snapshot = q
                .iter(world)
                .next()
                .cloned()
                .or_else(|| world.get_resource::<ShipPowerSystem>().cloned());
            let Some(power) = power_snapshot else {
                return vec![];
            };
            // Same component-then-Resource preference for the reactor config,
            // which `draining` needs to read this hull's own `rates`.
            let mut cq =
                world.query_filtered::<&PowerConfigResource, With<crate::server_app::LocalShip>>();
            let config = cq
                .iter(world)
                .next()
                .cloned()
                .or_else(|| world.get_resource::<PowerConfigResource>().cloned())
                .unwrap_or_default();
            vec![ServerMessage::PowerState {
                helm: power.0.level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
                weapons: power.0.level_for(&PowerGroupId(WEAPONS_POWER_GROUP.into())),
                shields: power.0.level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
                battery_charge: power.0.battery_charge,
                draining: power.0.is_draining(&config.0),
                locked: power.0.locked(),
            }]
        },
    )
}

// ── Systems ────────────────────────────────────────────────────────────────────

// ── Power brownout advisory (issue #678) ─────────────────────────────────────
//
// The old fused `operate_power_ai` (absolute-set, non-timer, non-
// AiHighFidelity-gated) was removed in issue #693. It is replaced by
// `console_ai::server::ai_power_allocation`, which since issue #831 emits an
// admitted `SetPowerGroupAllocation` applied by `handle_power_messages` above
// — the single applier for the human and AI paths alike (the intermediate
// `PowerReactorIntents` + `integrate_power_state` adapter has been retired).

// ── Scripted restraint (issue #1398) ─────────────────────────────────────────

/// Drain the scenario's queued power orders onto their ships (issue #1398).
///
/// The scripted half of the reactor, and it writes the SAME state an
/// Engineering officer's console command does: `hold_fire(name)` in a world
/// script and `SetPowerGroupAllocation { group: "weapons", level: 0 }` from a
/// Power console both land in
/// [`PowerSystem::set_group_allocation`](crate::modifiers::power_system::PowerSystem::set_group_allocation),
/// so the fire gate has one thing to read and [`mirror_weapons_cold_flags`]
/// below has one thing to publish.
///
/// It writes the reactor directly rather than manufacturing an admitted
/// command, which is the shape every other scripted world effect already has
/// (`destroy_entity`, `damage_infrastructure`, `set_workforce_disposition`).
/// Admission is the boundary between an OPERATOR and the ship — human or AI, the
/// same table either way — and the world is not an operator. What it is not is a
/// bypass: the order goes through the very setter a console order does, so the
/// group's authored `min_level` clamps it exactly the same way. A hull whose
/// `[power_groups.weapons]` says `min_level = 1` cannot be silenced by a script
/// any more than by its own crew, and that refusal is a designer's decision in
/// the entity TOML rather than this system's.
///
/// [`ScriptedPowerLevel::AuthoredDefault`] is resolved HERE and nowhere else,
/// because here is the first place that holds both the ship and its
/// [`crate::ship_plugin::ShipConfigComponent`]: the Rhai closure that pushed the
/// order has no world access at all, and a script that wrote `2` would be
/// authoring one hull's number into every scenario that used the verb.
pub fn drain_scripted_power_orders(
    // The scripted power-order queue, owned and declared by [`ShipPowerPlugin`].
    // `Option` so a reduced test app that runs this system without the
    // registering plugin is a no-op rather than a panic.
    power_orders: Option<ResMut<crate::effect_queue::EffectQueue<PendingGroupPower>>>,
    mut ships: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &mut ShipPowerSystem,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let Some(mut power_orders) = power_orders else {
        return;
    };
    // A `Deref` read, so a world that queues nothing never marks the queue
    // changed — the `tick_operations` precedent.
    if power_orders.0.is_empty() {
        return;
    }
    let queued = std::mem::take(&mut power_orders.0);
    for order in queued {
        let mut found = false;
        for (ship_uuid, mut power, ship_config) in ships.iter_mut() {
            if ship_uuid.0 != order.uuid {
                continue;
            }
            found = true;
            let level = match order.level {
                ScriptedPowerLevel::Exact(level) => level,
                ScriptedPowerLevel::AuthoredDefault => ship_config
                    .and_then(|config| config.0.power_groups.get(&order.group))
                    .map(|group| group.default_level)
                    .unwrap_or_else(crate::ship::config::default_power_level),
            };
            if let Err(err) = power.0.set_group_allocation(&order.group, level) {
                // A machine token, not prose: `crates/phoenix-simulation/src/ship/power.rs` is on
                // `check-strings.mjs`'s wire-visible allowlist, where every
                // prose literal in the production region is an error — spaces
                // and capitals included, so the uuid rides a colon.
                warn!("power.scripted_order_ignored:{}:{err:?}", order.uuid);
            }
        }
        if !found {
            warn!("power.scripted_order_unknown_ship:{}", order.uuid);
        }
    }
}

/// Mirror every ship's `weapons` power group into the world flag store (issue
/// #1398), so scenario script can read the posture and react to it.
///
/// Replaces `console::captain::server::mirror_weapons_hold_flags` (issue #1041)
/// key for key: `weapons_cold.own_ship` for the hull the crew fly, and
/// `weapons_cold.<name>` for any ship carrying an authored reference name. The
/// role key exists because a world's player hull is not required to declare a
/// name (`falling_skyway.toml` gives its `player-ship` an `id` and no `name`),
/// and "have the crew gone cold?" is exactly the question a scenario wants to
/// ask.
///
/// The transition is decided from the store's own `(before, after)` and the
/// event pushed onto `pending_world_events`, exactly as
/// `infrastructure::server::mirror_flags` does — so an
/// `on_flag_set("weapons_cold.own_ship", …)` handler chains off an order on the
/// next pipeline pass, through machinery that was already there.
///
/// # Why every ship every tick, and why that is still cheap
///
/// The retired hold mirror could filter on `Changed<ShipWeaponsHold>` because
/// that component only moved when somebody pulled the lever. [`ShipPowerSystem`]
/// is not like that: `tick_power_system` takes it by `&mut` to integrate the
/// battery, so change detection fires for every ship on every tick and the
/// filter would buy nothing but a false sense of one. So the walk is
/// unconditional — and the store is READ before it is written, which is the part
/// that actually matters. Reading first is not an optimisation: it is what keeps
/// a world nobody pulls the lever in byte-identical, because a `DerefMut` on
/// `WorldContentRuntime` marks the resource changed and change detection on it
/// is read elsewhere (`probe_radiation.toml` is where that showed up under
/// #1041, with the lever untouched and its committed digest moved).
///
/// Rows are sorted by flag name before anything is written, so the order of the
/// emitted events is a function of the content and never of archetype iteration
/// order.
pub fn mirror_weapons_cold_flags(
    runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    ships: Query<
        (
            &ShipPowerSystem,
            Option<&crate::entities::spawner::EntityName>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    if ships.is_empty() {
        return;
    }
    let weapons = PowerGroupId(WEAPONS_POWER_GROUP.to_string());
    let mut writes: Vec<(String, bool)> = Vec::new();
    for (power, name, is_local) in ships.iter() {
        let cold = power.0.is_group_cold(&weapons);
        if is_local {
            writes.push((OWN_SHIP_WEAPONS_COLD_FLAG.to_string(), cold));
        }
        if let Some(name) = name {
            writes.push((weapons_cold_flag(&name.0), cold));
        }
    }
    writes.sort();
    let pending: Vec<(String, bool)> = writes
        .into_iter()
        .filter(|(flag, cold)| runtime.flags.flag(flag) != *cold)
        .collect();
    if pending.is_empty() {
        return;
    }
    for (flag, cold) in pending {
        if cold {
            runtime.flags.set_flag(&flag);
        } else {
            runtime.flags.clear_flag(&flag);
        }
        runtime.pending_world_events.push(if cold {
            crate::world::content::WorldEvent::FlagSet {
                name: flag,
                origin_layer: None,
            }
        } else {
            crate::world::content::WorldEvent::FlagCleared {
                name: flag,
                origin_layer: None,
            }
        });
    }
}

// ── Blackboard publish (issue #561) ──────────────────────────────────────────

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "power_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::ship::power::*;
