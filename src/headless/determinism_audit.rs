//! Read-only schedule debt census for #1400. No executor or simulation changes.
use bevy::{ecs::schedule::LogLevel, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Complete ordinary plugin construction before inspecting its schedules.
/// Finish hooks may register systems; inspecting only Plugin::build omits them.
/// Never wait for asynchronous readiness or run Startup/a frame to obtain it.
pub(super) fn prepare_inspection(app: &mut App) -> Result<(), String> {
    use bevy::app::PluginsState;
    match app.plugins_state() {
        PluginsState::Adding => {
            return Err(
                "inspection requires ready plugins; no frame or readiness wait is allowed".into(),
            );
        }
        PluginsState::Ready => {
            app.finish();
            app.cleanup();
        }
        PluginsState::Finished => app.cleanup(),
        PluginsState::Cleaned => {}
    }
    Ok(())
}

/// Full type names are stable across registration order; numeric ECS IDs are not.
/// Repeated identical rows are intentional: two instances of a system must not
/// collapse into one permission in the debt ledger.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ambiguity {
    pub systems: [String; 2],
    pub access: Vec<String>,
}

/// Finish/clean ready plugins, then initialize but never run FixedUpdate and read Bevy's
/// unordered conflicting pairs. Existing explicit ambiguous_with declarations
/// are honored by Bevy; this function adds no exemption of its own.
pub fn fixed_update_census(app: &mut App) -> Result<Vec<Ambiguity>, String> {
    prepare_inspection(app)?;
    app.world_mut()
        .try_schedule_scope(FixedUpdate, |world, schedule| {
            schedule
                .initialize(world)
                .map_err(|error| format!("{error:?}"))?;
            let names: HashMap<_, _> = schedule
                .systems()
                .map_err(|error| format!("{error:?}"))?
                .map(|(key, system)| (key, system.name().as_string()))
                .collect();
            let mut rows = Vec::new();
            for (a, b, components) in schedule.graph().conflicting_systems().iter() {
                let mut systems = [
                    names.get(a).ok_or("missing first system name")?.clone(),
                    names.get(b).ok_or("missing second system name")?.clone(),
                ];
                systems.sort();
                let mut access = components
                    .iter()
                    .map(|id| {
                        world
                            .components()
                            .get_name(*id)
                            .map(|name| name.as_string())
                            .ok_or_else(|| format!("missing component name for {id:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if access.is_empty() {
                    access.push("<exclusive World access>".into());
                }
                access.sort();
                rows.push(Ambiguity { systems, access });
            }
            rows.sort();
            Ok(rows)
        })
        .map_err(|error| format!("{error:?}"))?
}

/// Return debt not covered by a previous census, including extra multiplicity.
/// Call once for live versus allowlist, and once for allowlist versus trusted base.
/// Access may shrink, but each allowed instance covers at most one current row.
pub fn uncovered(current: &[Ambiguity], allowed: &[Ambiguity]) -> Vec<Ambiguity> {
    let mut by_pair = BTreeMap::<_, Vec<usize>>::new();
    for (index, row) in allowed.iter().enumerate() {
        by_pair.entry(&row.systems).or_default().push(index);
    }
    let candidates: Vec<Vec<usize>> = current
        .iter()
        .map(|row| {
            by_pair
                .get(&row.systems)
                .into_iter()
                .flatten()
                .copied()
                .filter(|index| access_is_subset(&row.access, &allowed[*index].access))
                .collect()
        })
        .collect();
    // A greedy choice can strand B after assigning A to allowance [A,B],
    // despite a second allowance [A]. Augmenting paths reassign that first
    // match, retaining a one-to-one maximum matching for duplicate instances.
    fn assign(
        row: usize,
        candidates: &[Vec<usize>],
        owners: &mut [Option<usize>],
        seen: &mut [bool],
    ) -> bool {
        for &slot in &candidates[row] {
            if seen[slot] {
                continue;
            }
            seen[slot] = true;
            if owners[slot].is_none_or(|previous| assign(previous, candidates, owners, seen)) {
                owners[slot] = Some(row);
                return true;
            }
        }
        false
    }
    let mut owners = vec![None; allowed.len()];
    current
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            !assign(
                *index,
                &candidates,
                &mut owners,
                &mut vec![false; allowed.len()],
            )
        })
        .map(|(_, row)| row.clone())
        .collect()
}

fn access_is_subset(current: &[String], allowed: &[String]) -> bool {
    const EXCLUSIVE: &str = "<exclusive World access>";
    if allowed == [EXCLUSIVE] {
        return true;
    }
    if current.iter().any(|name| name == EXCLUSIVE) {
        return false;
    }
    let mut counts = BTreeMap::new();
    for name in allowed {
        *counts.entry(name).or_insert(0usize) += 1;
    }
    current.iter().all(|name| {
        let remaining = counts.entry(name).or_default();
        if *remaining == 0 {
            false
        } else {
            *remaining -= 1;
            true
        }
    })
}

pub fn parse_census(raw: &str) -> Result<Vec<Ambiguity>, String> {
    let rows: Vec<Ambiguity> = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if rows.windows(2).any(|p| p[0] > p[1])
        || rows.iter().any(|row| {
            row.systems[0].is_empty()
                || row.systems[0] > row.systems[1]
                || row.access.is_empty()
                || row.access.iter().any(String::is_empty)
                || row.access.windows(2).any(|p| p[0] > p[1])
        })
    {
        return Err("ambiguity census is not canonical".into());
    }
    Ok(rows)
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
struct StrictAuditRebuild;

/// Once debt reaches zero, exercise Bevy's own strict build check as well.
/// Only this test helper changes the inspected app's build setting.
pub fn require_unambiguous_fixed_update(app: &mut App) -> Result<(), String> {
    prepare_inspection(app)?;
    app.world_mut()
        .try_schedule_scope(FixedUpdate, |world, schedule| {
            let mut settings = schedule.get_build_settings();
            settings.ambiguity_detection = LogLevel::Error;
            schedule.set_build_settings(settings);
            // Bevy 0.18 set_build_settings does not dirty an already built graph.
            // An empty test-only set requests a rebuild without adding access/order.
            schedule.configure_sets(StrictAuditRebuild);
            schedule.initialize(world).map_err(|e| format!("{e:?}"))
        })
        .map_err(|e| format!("{e:?}"))?
}

/// Instance-level diagnostic export; separate from the name-based debt ledger.
pub mod graph;
