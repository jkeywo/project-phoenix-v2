//! One ordinary state-tree controller per shield arc, using its Console view.
use crate::core::messages::{SystemBlackboard, SystemControlPayload as Control, SystemId};
use bevy::prelude::*;
use std::collections::BTreeMap;

pub(super) fn ai_shields(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut ships: Query<(
        Option<&crate::entities::spawner::EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        Option<&crate::ship::shields::ShieldsAiConfigResource>,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut crate::core::messages::AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree_with_numbers, TreeHostOutcome};
    let defaults = crate::ship::shields::ShieldsAiConfigResource::default();
    for (uuid, sources, config, boards, programs, tuning, mut runtimes, mut admitted) in &mut ships
    {
        for system in config
            .0
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
        {
            if !crate::ai::host::ai_operates(&sources.0, &system.id) {
                continue;
            }
            let permitted = config.0.observations_for_system(&system.id, &boards.0);
            let mut view = BTreeMap::new();
            let feeds = config
                .0
                .console_for_system(&system.id)
                .into_iter()
                .flat_map(|console| console.feeds.iter().map(|feed| &feed.source));
            for id in config
                .0
                .systems
                .iter()
                .map(|system| &system.id)
                .chain(feeds)
            {
                if let Some(board) = permitted.get(id) {
                    view.insert(id.clone(), board);
                }
            }
            let Some(SystemBlackboard::ShieldArc(own)) = view.get(&system.id).copied() else {
                continue;
            };
            let Some(operation) = &own.operation else {
                continue;
            };
            let Some(reserve) = &operation.reserve else {
                continue;
            };
            // Focus ranks only capable equipment. Protection/recovery retains
            // the whole permitted picture, including arcs that cannot focus.
            let preferred = preferred_arc(&view, tuning.unwrap_or(&defaults), false);
            let preferred_focus = preferred_arc(&view, tuning.unwrap_or(&defaults), true);
            let mut facts = BTreeMap::from([
                (
                    "preferred_focus".into(),
                    preferred_focus.as_ref() == Some(&system.id),
                ),
                (
                    "preferred_protection".into(),
                    preferred.as_ref() == Some(&system.id),
                ),
                ("raised".into(), operation.raised),
                ("thermal_stopped".into(), reserve.thermal_stopped),
            ]);
            let mut numbers = BTreeMap::from([
                (
                    "strength_fraction".into(),
                    fraction(own.hp as f32, own.max_hp as f32),
                ),
                (
                    "reserve_fraction".into(),
                    fraction(reserve.energy, reserve.capacity),
                ),
                (
                    "heat_fraction".into(),
                    fraction(reserve.heat, reserve.trip_heat),
                ),
                ("recent_damage".into(), own.recent_damage as f64),
            ]);
            let mut channels = BTreeMap::from([
                ("raised".into(), operation.raised),
                ("focus".into(), own.is_focused),
            ]);
            for target in &operation.recovery_targets {
                let observed = view.get(target).and_then(|board| match board {
                    SystemBlackboard::ShieldArc(reading) => Some(reading),
                    _ => None,
                });
                let target_operation = observed.and_then(|reading| reading.operation.as_ref());
                let target_reserve =
                    target_operation.and_then(|operation| operation.reserve.as_ref());
                facts.insert(
                    format!("target-visible-{}", target.0),
                    target_reserve.is_some(),
                );
                facts.insert(
                    format!("target-preferred-{}", target.0),
                    preferred.as_ref() == Some(target),
                );
                facts.insert(
                    format!("target-ready-{}", target.0),
                    target_operation.is_some_and(|operation| operation.can_recharge),
                );
                let recovering = operation.recovery_target.as_ref() == Some(target);
                facts.insert(format!("recovering-{}", target.0), recovering);
                channels.insert(format!("recover-{}", target.0), recovering);
                numbers.insert(
                    format!("target-strength-{}", target.0),
                    observed.map_or(0.0, |reading| {
                        fraction(reading.hp as f32, reading.max_hp as f32)
                    }),
                );
                numbers.insert(
                    format!("target-reserve-{}", target.0),
                    target_reserve
                        .map_or(0.0, |reserve| fraction(reserve.energy, reserve.capacity)),
                );
            }
            let Some(runtime) = runtimes.0.get_mut(&system.id) else {
                continue;
            };
            let result = match decide_tree_with_numbers(
                &sources.0,
                &system.id,
                programs.0.get(&system.id),
                &facts,
                &numbers,
                &channels,
                runtime,
                tick.0,
            ) {
                Ok(TreeHostOutcome::Evaluated(result)) => result,
                Ok(_) => continue,
                Err(error) => {
                    crate::pwarn!(
                        log,
                        crate::logging::LogCat::Shields,
                        "shield.tree_refused: {error}"
                    );
                    continue;
                }
            };
            let mut commands = Vec::new();
            if let Some(raised) = result
                .intents
                .get("raised")
                .filter(|value| **value != operation.raised)
            {
                commands.push(Control::SetShieldRaised { raised: *raised });
            }
            if let Some(focused) = result.intents.get("focus").filter(|value| {
                **value != own.is_focused && (!**value || operation.focus_available)
            }) {
                commands.push(Control::SetShieldArcFocus { focused: *focused });
            }
            // The supported result is one physical destination. Resolve multiple
            // true outputs in authored endpoint order, deterministically last.
            let mut target = operation.recovery_target.clone();
            for candidate in &operation.recovery_targets {
                if let Some(enabled) = result.intents.get(&format!("recover-{}", candidate.0)) {
                    if *enabled {
                        target = Some(candidate.clone());
                    } else if target.as_ref() == Some(candidate) {
                        target = None;
                    }
                }
            }
            if target != operation.recovery_target {
                commands.push(Control::SetShieldRecovery { target });
            }
            for command in commands {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    system.id.clone(),
                    command,
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}

fn fraction(value: f32, capacity: f32) -> f64 {
    if capacity > 0.0 {
        (value as f64 / capacity as f64).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn preferred_arc(
    view: &BTreeMap<SystemId, &SystemBlackboard>,
    tuning: &crate::ship::shields::ShieldsAiConfigResource,
    focus_only: bool,
) -> Option<SystemId> {
    let bearing = view.values().find_map(|board| match board {
        SystemBlackboard::Shields(reading) => {
            reading.threat_bearing.or(reading.combat_lock_bearing)
        }
        _ => None,
    });
    let mut arcs: Vec<_> = view
        .iter()
        .filter_map(|(id, board)| match board {
            SystemBlackboard::ShieldArc(reading)
                if !focus_only
                    || reading
                        .operation
                        .as_ref()
                        .is_some_and(|operation| operation.focus_available) =>
            {
                Some((id, reading))
            }
            _ => None,
        })
        .collect();
    if bearing.is_none() {
        let total_damage: f64 = arcs
            .iter()
            .map(|(_, reading)| reading.recent_damage as f64)
            .sum();
        let most = arcs.iter().max_by(|(a_id, a), (b_id, b)| {
            a.recent_damage
                .total_cmp(&b.recent_damage)
                .then_with(|| b_id.cmp(a_id))
        });
        if let Some((id, reading)) = most {
            if total_damage > 0.0
                && reading.recent_damage as f64
                    >= total_damage * tuning.damage_pct_threshold as f64 / 100.0
            {
                return Some((*id).clone());
            }
        }
        arcs.sort_by(|(a_id, a), (b_id, b)| {
            fraction(a.hp as f32, a.max_hp as f32)
                .total_cmp(&fraction(b.hp as f32, b.max_hp as f32))
                .then_with(|| a_id.cmp(b_id))
        });
        if arcs.len() > 1
            && fraction(arcs[0].1.hp as f32, arcs[0].1.max_hp as f32)
                < fraction(arcs[1].1.hp as f32, arcs[1].1.max_hp as f32)
                    * tuning.health_ratio_threshold as f64
                    / 100.0
        {
            return Some(arcs[0].0.clone());
        }
        return arcs
            .iter()
            .find(|(_, reading)| reading.is_focused)
            .map(|(id, _)| (*id).clone());
    }
    arcs.into_iter()
        .min_by(|(a_id, a), (b_id, b)| {
            let score = |reading: &crate::core::messages::ShieldArcBlackboard| {
                bearing.map_or_else(
                    || fraction(reading.hp as f32, reading.max_hp as f32),
                    |bearing| {
                        phoenix_sim_gameplay::ship::shields::angular_distance_deg(
                            reading.center_deg,
                            bearing,
                        ) as f64
                    },
                )
            };
            score(a)
                .total_cmp(&score(b))
                .then_with(|| b.is_focused.cmp(&a.is_focused))
                .then_with(|| a_id.cmp(b_id))
        })
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
#[path = "shields_tree_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shields_tree_migration_tests.rs"]
mod migration_tests;
