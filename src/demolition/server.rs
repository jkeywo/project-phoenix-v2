//! The Bevy adapter for the controlled-demolition operation (issue #1350, PRD
//! #1337).
//!
//! Gathers the live world into the plain values the pure sibling
//! [`crate::demolition::ops`] takes — which obstruction was named, whether its
//! charges are placed, whether the team has withdrawn, whether the tractor is
//! holding it, whether it has already gone off, whether the System that fires it
//! is still standing — and applies what comes back: raise the authored world flag
//! for the outcome the scenario hangs its four consequences off, mark the
//! operation spent, and publish the console's readout. Nothing here decides an
//! outcome or a refusal itself: rule 10, the split Security and the tractor keep
//! between their pure and adapter halves.
//!
//! # It owns an operation, not a system
//!
//! Detonation READS Security (which team is where) and the tractor (what the beam
//! holds) but is owned by neither — an obstruction is what gets detonated. So
//! `DetonateCharges` targets the `security` system (the station that fires the
//! charges is the one that dispatched the team — Tactical, on the Alliance
//! Destroyer), and this module simply reads that already-routed command out of
//! `AdmittedCommands`. It registers no consumer of its own and declares no new
//! `[[system]]`: the Security consumer already claims that target, and the
//! demolition state is published on its OWN reserved channel rather than a real
//! system id, because an operation is not a thing aboard the ship that can be
//! damaged or commanded.
//!
//! # The one interface to the scenario is a world flag
//!
//! A completed detonation raises exactly one of three authored flags — safe,
//! unsupported, premature — plus a generic "detonated" marker, on the same
//! `WorldContentRuntime` store Security raises its `outcome_flag` on. The
//! scenario hangs every debris fragment, casualty and infrastructure consequence
//! off those flags in TOML. The fourth outcome the issue names — ordinary weapons
//! fire — needs nothing here at all: it is a target killed by gunnery with no
//! charges ever placed, which is the scenario's own `on_destroyed` handler.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::core::messages::{
    AdmittedCommands, DemolitionBlackboard, DemolitionSlot, SystemBlackboard, SystemControlPayload,
    SystemId,
};
use crate::demolition::ops::{
    detonation_status, resolve_outcome, DemolitionConfig, DemolitionRefusal,
};
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::security::ShipSecurityTeams;
use crate::ship::damage::DamageTier;
use crate::ship::system_registry::{security_system_id, SECURITY_SYSTEM_ID};
use crate::tractor::TractorBeam;
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

/// The reserved channel key the demolition operation publishes its blackboard
/// under (issue #1350). Not a real system id — see the module docs.
pub const DEMOLITION_BLACKBOARD_KEY: &str = "demolition";

/// The published blackboard channel key as a [`SystemId`], matching
/// `science::scan_blackboard_key`'s shape.
pub fn demolition_blackboard_key() -> SystemId {
    SystemId(DEMOLITION_BLACKBOARD_KEY.to_string())
}

/// What a Security team may be sent to DEMOLISH (issue #1350): the authored
/// `[demolition_target]` table on a world entity that can be cleared by
/// controlled demolition.
///
/// Inserted at spawn only on an entity that authored a `[demolition_target]`
/// table — everything else carries no component and cannot be detonated, which is
/// why every shipped hull and every existing world is untouched by this slice.
/// The authored terms never change once loaded, so this is `DeferredFold` for the
/// digest: authoritative, but `content_digest` already answers for it.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct DemolitionTarget(pub DemolitionConfig);

/// One ship's demolition control state (issue #1350): the refusal projection the
/// console shows.
///
/// Inserted on any hull that musters Security teams — the station that fires the
/// charges. It holds no authoritative operation state itself: charged, detonated
/// and each outcome are WORLD FLAGS (already folded, snapshotted and
/// deterministic), and team-clear and stabilised are read live from Security and
/// the tractor. All this carries is the last refusal, a projection the next
/// command re-derives, which is why it is inert to the digest.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct DemolitionControl {
    /// Why the last detonation was refused — the reason the console shows,
    /// retained until the operator detonates again. Never folded, never saved.
    pub last_refusal: Option<DemolitionRefusal>,
}

/// Registers the demolition systems (issue #1350). Added by `WorldPlugin`
/// alongside `SecurityPlugin`, whose command target it borrows.
pub struct DemolitionPlugin;

impl Plugin for DemolitionPlugin {
    fn build(&self, app: &mut App) {
        // No `register_admitted_consumer`: `DetonateCharges` targets the
        // `security` system, whose consumer `SecurityPlugin` already registered,
        // so admission fans it into every ship's `AdmittedCommands` and the
        // unrouted lint stays quiet.
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Tactical: emit `DetonateCharges` on the shared AI
                // cadence (rule 7), BEFORE the handler consumes the tick, and only
                // when the shot would be the safe one.
                operate_demolition_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_demolition_commands),
                handle_demolition_commands.in_set(crate::sim_sets::SimSet::Input),
                publish_demolition_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

/// One obstruction, read once per tick into plain values so the borrow of the
/// world is released before the verdicts and the write.
struct TargetRow {
    uuid: String,
    name: Option<String>,
    config: DemolitionConfig,
}

/// Read every entity that authors a demolition target into plain rows, in uuid
/// order so two hosts walk them identically.
fn target_rows(
    query: &Query<(&EntityUuid, Option<&EntityName>, &DemolitionTarget)>,
) -> Vec<TargetRow> {
    let mut rows: Vec<TargetRow> = query
        .iter()
        .map(|(uuid, name, target)| TargetRow {
            uuid: uuid.0.clone(),
            name: name.map(|n| n.0.clone()),
            config: target.0.clone(),
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

/// Whether a ship's Security System is damaged out — the System that fires the
/// charges. The same read `security::server::security_disabled` makes.
fn security_disabled(hull: Option<&EntitySystemHull>) -> bool {
    hull.map(|h| {
        matches!(
            h.0.tier_for(&security_system_id()),
            DamageTier::Disabled | DamageTier::Destroyed
        )
    })
    .unwrap_or(false)
}

/// Whether no Security team on `security` is still committed to `target` — the
/// team-safety state a completed placement must NOT be mistaken for. A team that
/// is deploying, working or withdrawing to this obstruction has not cleared it;
/// only when none is does a detonation catch nobody.
fn team_clear(security: Option<&ShipSecurityTeams>, target: &str) -> bool {
    match security {
        None => true,
        Some(security) => !security
            .teams
            .iter()
            .any(|team| team.is_committed() && team.target.as_deref() == Some(target)),
    }
}

/// Whether this ship's tractor beam is currently holding `target`.
fn stabilized(beam: Option<&TractorBeam>, target: &str) -> bool {
    beam.and_then(|b| b.coupled_target.as_deref()) == Some(target)
}

/// Take this tick's `DetonateCharges` commands and apply them (issue #1350).
///
/// Runs in `SimSet::Input`. Every command is answered: a detonation that cannot
/// form leaves the operation as it was and records the one reason the console
/// shows; a detonation that forms raises the outcome flag the pure decision
/// picks, plus the generic "detonated" marker, and pushes the rising edge onto
/// the world-event stream so the scenario's `on_flag_set` handlers fire the same
/// tick — the same mirror Security keeps for its `outcome_flag`.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak and stripped the source (AGENTS.md rule 6).
#[allow(clippy::type_complexity)]
pub fn handle_demolition_commands(
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    targets: Query<(&EntityUuid, Option<&EntityName>, &DemolitionTarget)>,
    mut operators: Query<(
        Entity,
        &EntityUuid,
        &AdmittedCommands,
        Option<&EntitySystemHull>,
        Option<&ShipSecurityTeams>,
        Option<&TractorBeam>,
        &mut DemolitionControl,
    )>,
) {
    // UUID order, not archetype order: two hosts must take the same ship's
    // commands in the same sequence.
    let mut ordered: Vec<(String, Entity, Vec<String>)> = Vec::new();
    for (entity, uuid, admitted, ..) in operators.iter() {
        let requests: Vec<String> = admitted
            .for_target(SECURITY_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::DetonateCharges { target } => Some(target.clone()),
                _ => None,
            })
            .collect();
        if requests.is_empty() {
            continue;
        }
        ordered.push((uuid.0.clone(), entity, requests));
    }
    if ordered.is_empty() {
        return;
    }
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));
    let rows = target_rows(&targets);

    // The rising edges to publish, gathered before the world store is borrowed
    // mutably, in the order they were decided (operator-uuid, then command order).
    let mut raised: Vec<String> = Vec::new();

    for (_, entity, requests) in ordered {
        let Ok((_, _, _, hull, security, beam, mut control)) = operators.get_mut(entity) else {
            continue;
        };
        let disabled = security_disabled(hull);
        for target in requests {
            let row = rows.iter().find(|r| r.uuid == target);
            let (charged, detonated) = match (row, runtime.as_deref()) {
                (Some(row), Some(rt)) => (
                    rt.flags.flag(&row.config.charges_flag),
                    rt.flags.flag(&row.config.detonated_flag),
                ),
                // With no world store there are no flags to read: nothing is
                // charged, so the verdict below reports it honestly.
                _ => (false, false),
            };
            match detonation_status(row.is_some(), charged, detonated, disabled) {
                Err(refusal) => {
                    control.last_refusal = Some(refusal);
                }
                Ok(()) => {
                    // Safe unwrap: `detonation_status` returned Ok only because the
                    // row and the store both resolved.
                    let row = row.expect("Ok verdict implies a known target");
                    let outcome = resolve_outcome(
                        team_clear(security, &target),
                        row.config.stabilization_required,
                        stabilized(beam, &target),
                    );
                    control.last_refusal = None;
                    // Both flags rise: the specific outcome the scenario branches
                    // on, and the generic marker that arms the refusal gate and
                    // carries the "the charges went off" beat.
                    raised.push(row.config.outcome_flag(outcome).to_string());
                    raised.push(row.config.detonated_flag.clone());
                }
            }
        }
    }

    if let Some(runtime) = runtime.as_deref_mut() {
        for flag in raised {
            let (before, after) = runtime.flags.set_flag_value(&flag, 1);
            if (before != 0) == (after != 0) {
                continue;
            }
            // The same mirror Security keeps: a scenario hangs its consequence off
            // `on_flag_set`, so the edge has to reach the world-event stream, not
            // just the store.
            runtime.pending_world_events.push(WorldEvent::FlagSet {
                name: flag,
                origin_layer: None,
            });
        }
    }
}

/// Backfill Tactical (issue #1350).
///
/// Fires the charges only when doing so is the SAFEST valid path: the operation
/// is charged, not yet detonated, the team has withdrawn, and the mass is held if
/// it needed holding. The host never chooses the premature or unsupported
/// outcome — those are a human's call to make when the window forces it, and a
/// Backfilled seat that reached for them would be taking casualties or scattering
/// debris on its own initiative. The concrete command is exactly the
/// `DetonateCharges` a human at the Tactical console emits, through the SAME
/// `emit_ai_command` seam, so the handler never learns who spoke (rule 6). Decides
/// ONLY on the shared AI cadence (rule 7).
#[allow(clippy::type_complexity)]
pub fn operate_demolition_ai(
    runtime: Option<Res<WorldContentRuntime>>,
    targets: Query<(&EntityUuid, Option<&EntityName>, &DemolitionTarget)>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Entity,
        &EntityUuid,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&EntitySystemHull>,
        Option<&ShipSecurityTeams>,
        Option<&TractorBeam>,
        &mut AdmittedCommands,
        &DemolitionControl,
    )>,
) {
    let Some(runtime) = runtime.as_deref() else {
        return;
    };
    let rows = target_rows(&targets);
    if rows.is_empty() {
        return;
    }
    let system_id = security_system_id();

    let mut order: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (_, entity) in order {
        let Ok((_, uuid, sources, config, hull, security, beam, mut admitted, _control)) =
            ships.get_mut(entity)
        else {
            continue;
        };
        if !sources.0.policy_for(&system_id).operate_ai {
            continue;
        }
        if security_disabled(hull) {
            continue;
        }
        // The first target, in uuid order, whose shot would be safe. One per tick
        // is enough — the cadence brings the host back for the next.
        for row in &rows {
            if !runtime.flags.flag(&row.config.charges_flag) {
                continue;
            }
            if runtime.flags.flag(&row.config.detonated_flag) {
                continue;
            }
            if !team_clear(security, &row.uuid) {
                continue;
            }
            if row.config.stabilization_required && !stabilized(beam, &row.uuid) {
                continue;
            }
            emit_ai_command(
                Some(uuid),
                system_id.clone(),
                SystemControlPayload::DetonateCharges {
                    target: row.uuid.clone(),
                },
                sources,
                &sessions,
                config,
                &mut admitted,
            );
            break;
        }
    }
}

/// Publish each demolishing ship's operation readout (issue #1350).
///
/// Only ships that carry [`DemolitionControl`] — those that can fire charges —
/// publish one, and only when the world holds at least one demolition target, so
/// a world that authors none puts exactly the payload on the wire it did before
/// this existed. No English crosses.
#[allow(clippy::type_complexity)]
pub fn publish_demolition_blackboard(
    runtime: Option<Res<WorldContentRuntime>>,
    targets: Query<(&EntityUuid, Option<&EntityName>, &DemolitionTarget)>,
    mut ships: Query<(
        Option<&ShipSecurityTeams>,
        Option<&TractorBeam>,
        &DemolitionControl,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    if ships.is_empty() {
        return;
    }
    let rows = target_rows(&targets);
    if rows.is_empty() {
        return;
    }
    let key = demolition_blackboard_key();
    for (security, beam, control, mut blackboards) in ships.iter_mut() {
        let slots = rows
            .iter()
            .map(|row| {
                let (charged, detonated) = match runtime.as_deref() {
                    Some(rt) => (
                        rt.flags.flag(&row.config.charges_flag),
                        rt.flags.flag(&row.config.detonated_flag),
                    ),
                    None => (false, false),
                };
                DemolitionSlot {
                    target: row.uuid.clone(),
                    target_name: row.name.clone(),
                    charged,
                    team_clear: team_clear(security, &row.uuid),
                    stabilization_required: row.config.stabilization_required,
                    stabilized: stabilized(beam, &row.uuid),
                    detonated,
                    warning: row.config.warning.clone(),
                }
            })
            .collect();
        let blackboard = SystemBlackboard::Demolition(DemolitionBlackboard {
            targets: slots,
            refusal: control.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
