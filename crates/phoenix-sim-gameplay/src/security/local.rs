//! Local Security uses the existing actual interior graph, not another movement model.
use super::teams::{SecurityAction, SecurityActionConfig, SecurityTargetConfig};
use crate::ship::interior::{InteriorGraph, RouteCursor, RouteLocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSecurityConfig {
    pub team_homes: Vec<String>,
    #[serde(rename = "target")]
    pub targets: Vec<LocalSecurityTarget>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSecurityTarget {
    pub compartment: String,
    #[serde(rename = "action")]
    pub actions: Vec<SecurityActionConfig>,
}

#[derive(Clone, Debug)]
pub struct CompiledLocalSecurity {
    pub graph: InteriorGraph,
    pub config: LocalSecurityConfig,
}

pub struct LocalWorkResult {
    pub team_idx: usize,
    pub completed: bool,
    pub outcome: Option<(String, i64)>,
}

/// Actual movement and paid work use one canonical Job/cursor. The adapter
/// supplies only already-funded work time for teams actually at their target.
pub fn tick(
    profile: &CompiledLocalSecurity,
    state: &mut crate::jobs::security::SecurityJobs,
    dt: f32,
    funded_work: &[f32],
    blocked: &BTreeSet<String>,
    disabled: bool,
) -> Vec<LocalWorkResult> {
    use super::teams::SecurityTeamState as Phase;
    use phoenix_model::job::JobOutcome;
    let mut events = Vec::new();
    if !dt.is_finite() || dt < 0.0 || profile.validate_navigation(state).is_err() {
        return events;
    }
    state.set_unavailable(disabled);
    for index in 0..state.len() {
        let team = state[index].clone();
        if !team.is_committed() {
            continue;
        }
        if disabled && team.is_interruptible() {
            state.withdraw(index, 0.0, JobOutcome::Failed);
            events.push(LocalWorkResult {
                team_idx: index,
                completed: false,
                outcome: None,
            });
            continue;
        }
        let Some(target) = team
            .target
            .as_deref()
            .and_then(|target| profile.target(target))
        else {
            continue;
        };
        match team.state {
            Phase::Deploying => {
                let cursor = &mut state.navigation.as_mut().expect("validated local cursor")[index];
                if profile
                    .graph
                    .advance(cursor, &target.compartment, dt, blocked)
                {
                    if let Some(action) = team.action.and_then(|action| {
                        target
                            .actions
                            .iter()
                            .find(|candidate| candidate.action == action)
                    }) {
                        state
                            .get_mut(index)
                            .expect("committed team")
                            .begin_work(action.duration_secs);
                    }
                }
            }
            Phase::Working => {
                let offered = funded_work.get(index).copied().unwrap_or(0.0);
                if !offered.is_finite() || offered < 0.0 {
                    continue;
                }
                let team = state.get_mut(index).expect("committed team");
                team.elapsed = (team.elapsed + offered.min(dt)).min(team.phase_duration);
                if team.elapsed >= team.phase_duration {
                    let outcome = target
                        .actions
                        .iter()
                        .find(|action| Some(action.action) == team.action)
                        .and_then(|action| {
                            action
                                .outcome_flag
                                .as_ref()
                                .map(|flag| (flag.clone(), action.outcome_value))
                        });
                    state.withdraw(index, 0.0, JobOutcome::Completed);
                    events.push(LocalWorkResult {
                        team_idx: index,
                        completed: true,
                        outcome,
                    });
                }
            }
            Phase::Withdrawing => {
                let cursor = &mut state.navigation.as_mut().expect("validated local cursor")[index];
                if profile
                    .graph
                    .advance(cursor, &profile.config.team_homes[index], dt, blocked)
                {
                    state.home(index);
                }
            }
            _ => {}
        }
    }
    events
}

impl PartialEq for CompiledLocalSecurity {
    fn eq(&self, other: &Self) -> bool {
        self.graph.config == other.graph.config && self.config == other.config
    }
}

/// Local addresses are scoped by the command's actual actor/System ownership.
/// They are never World entity identities or remote transfer destinations.
pub fn address(compartment: &str) -> String {
    format!("compartment:{compartment}")
}

impl CompiledLocalSecurity {
    pub fn target(&self, target: &str) -> Option<&LocalSecurityTarget> {
        self.config
            .targets
            .iter()
            .find(|row| address(&row.compartment) == target)
    }
    pub fn initial_navigation(&self) -> Vec<RouteCursor> {
        self.config
            .team_homes
            .iter()
            .map(|home| RouteCursor {
                location: RouteLocation::Compartment {
                    compartment: home.clone(),
                },
                destination: None,
                route: Vec::new(),
                blocked: false,
            })
            .collect()
    }
    pub fn validate_navigation(
        &self,
        state: &crate::jobs::security::SecurityJobs,
    ) -> Result<(), String> {
        let navigation = state
            .navigation
            .as_ref()
            .ok_or("local Security requires its one actual team cursor")?;
        if navigation.len() != self.config.team_homes.len() {
            return Err("local Security team cursor count mismatch".into());
        }
        for (index, cursor) in navigation.iter().enumerate() {
            self.graph.validate_cursor(cursor)?;
            if matches!(cursor.location, RouteLocation::External { .. }) {
                return Err("local-only Security cannot restore external custody".into());
            }
            let team = state.get(index).ok_or("missing actual Security team")?;
            use super::teams::SecurityTeamState as Phase;
            if team.is_committed() {
                let target = team
                    .target
                    .as_deref()
                    .and_then(|id| self.target(id))
                    .ok_or("local Security work is outside accepted compartments")?;
                let action = team
                    .action
                    .and_then(|action| {
                        target
                            .actions
                            .iter()
                            .find(|offered| offered.action == action)
                    })
                    .ok_or("local Security has no actual accepted action")?;
                if team.risk != action.risk
                    || (team.state == Phase::Working && team.phase_duration != action.duration_secs)
                    || (matches!(team.state, Phase::Deploying | Phase::Withdrawing)
                        && (team.phase_duration != 0.0 || team.elapsed != 0.0))
                {
                    return Err("local Security terms differ from actual accepted work or physical route mode".into());
                }
            }
            let at = |room: &str| matches!(&cursor.location, RouteLocation::Compartment { compartment } if compartment == room);
            match team.state {
                Phase::Available | Phase::Unavailable if !at(&self.config.team_homes[index]) => {
                    return Err("released local Security team is not actually home".into());
                }
                Phase::Working => {
                    let target = team
                        .target
                        .as_deref()
                        .and_then(|target| self.target(target))
                        .ok_or("local Security names an unsupported compartment target")?;
                    if !at(&target.compartment) {
                        return Err("local Security work is not at its actual destination".into());
                    }
                }
                Phase::Deploying | Phase::Withdrawing => {
                    if !team
                        .target
                        .as_deref()
                        .is_some_and(|target| self.target(target).is_some())
                    {
                        return Err(
                            "local Security assignment is outside its accepted compartments".into(),
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

pub fn compile_entity(
    entity: &crate::entities::config::EntityConfig,
) -> Result<Option<CompiledLocalSecurity>, String> {
    let Some(security) = &entity.security else {
        return Ok(None);
    };
    let Some(local) = &security.local else {
        return Ok(None);
    };
    let interior = entity
        .interior
        .as_ref()
        .ok_or("local Security requires actual compartment topology")?;
    let ship = entity
        .ship_config
        .as_ref()
        .ok_or("local Security requires actual owned Systems")?;
    let repair_count = entity
        .repair
        .as_ref()
        .map_or(0, |repair| repair.repair_team_count as usize);
    let graph = InteriorGraph::compile(interior, ship, repair_count)?;
    let owner = crate::ship::system_registry::security_system_id();
    if ship
        .system(&owner)
        .is_none_or(|system| system.kind != crate::ship::system_registry::SECURITY_KIND)
        || !entity.energy.as_ref().is_some_and(|energy| {
            energy
                .consumers
                .iter()
                .any(|consumer| consumer.system == owner && consumer.continuous_cost_per_sec > 0.0)
        })
    {
        return Err(
            "local Security requires its actual canonical owner and positive work energy profile"
                .into(),
        );
    }
    let rooms: BTreeSet<_> = interior
        .compartments
        .iter()
        .map(|room| room.id.as_str())
        .collect();
    if local.team_homes.len() != security.team_count as usize
        || local
            .team_homes
            .iter()
            .any(|room| !rooms.contains(room.as_str()))
        || local.targets.is_empty()
    {
        return Err(
            "local Security requires actual homes for its finite team count and compartment work"
                .into(),
        );
    }
    let mut seen = BTreeSet::new();
    for target in &local.targets {
        if !rooms.contains(target.compartment.as_str())
            || !seen.insert(&target.compartment)
            || target.actions.iter().any(|action| {
                !matches!(
                    action.action,
                    SecurityAction::SecureContain | SecurityAction::AssistEvacuation
                )
            })
        {
            return Err("local Security requires distinct actual compartments with supported containment/evacuation work".into());
        }
        SecurityTargetConfig {
            actions: target.actions.clone(),
        }
        .validate()?;
    }
    Ok(Some(CompiledLocalSecurity {
        graph,
        config: local.clone(),
    }))
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
