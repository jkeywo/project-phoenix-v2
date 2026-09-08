//! The Bevy adapter for the Security System (issue #1346, PRD #1337).
//!
//! Gathers the live world into the plain values the pure sibling
//! [`crate::security::teams`] takes — which teams are free, which targets are in
//! reach, what each of them offers, whether the System itself is still standing —
//! and applies what comes back: the per-ship [`ShipSecurityTeams`] component, the
//! per-target [`SecurityTargetActions`] component, the fixed-tick systems that
//! take the dispatch/recall commands, walk each team through deploy → work →
//! withdraw, raise the authored consequence when work completes, and publish the
//! console's readout. Nothing here decides eligibility or priority itself: rule
//! 10, the split the tractor keeps between `coupling` and `server`.
//!
//! # Two teams, two independent assignments
//!
//! Unlike the external repair dispatch (#1161), which claims ONE team against
//! whatever the ship has designated, a Security command NAMES its team, its
//! target and its action. It has to: two teams working two different compartments
//! is the whole point, and a single Tactical lock cannot say which of them a
//! recall means. That is also why each team gets its own task-lifecycle slot
//! (`security_team_0`, `security_team_1`), so two simultaneous assignments are
//! two simultaneous activations rather than one restarting the other.
//!
//! # There is no Duty Officer
//!
//! Security is a plain station-owned `[[system]]`, assigned by ship TOML — on the
//! Alliance Destroyer, to Tactical. It takes the ordinary station-tenure
//! admission path, so whoever holds that station may command it, and #1162's
//! backfill host may emit the byte-identical command when nobody does (AGENTS.md
//! rule 6). No second gate, no roster, no officer.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::core::messages::{
    AdmittedCommands, SecurityActionOption, SecurityBlackboard, SecurityTargetOption,
    SecurityTeamSlot, SystemAffinity, SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::core::task_lifecycle::{TaskLifecycleRequest, TaskSlot, TaskTerminalReason};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::security::teams::{
    dispatch_status, is_assigned, select_assignments, unassigned_candidates, SecurityAction,
    SecurityActionConfig, SecurityCandidate, SecurityConfig, SecurityRefusal, SecurityTargetConfig,
    SecurityTeam, SecurityTeamState,
};
use crate::ship::damage::DamageTier;
use crate::ship::system_registry::{security_system_id, SECURITY_KIND, SECURITY_SYSTEM_ID};
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

/// The task-lifecycle verb prefix a Security assignment is recorded under (issue
/// #1341's vocabulary, used by #1346).
///
/// One slot PER TEAM — `security_team_0`, `security_team_1` — rather than one per
/// action, because a [`TaskSlot`] holds at most one live activation and the two
/// teams are genuinely simultaneous: sharing a slot would make the second team's
/// dispatch silently [`TaskTerminalReason::Restarted`] the first team's work.
pub const TASK_VERB_SECURITY_TEAM: &str = "security_team";

/// The task-lifecycle slot for one team of one operator (issue #1346).
fn team_slot(operator: &str, team_idx: u8) -> TaskSlot {
    TaskSlot::new(
        operator,
        SECURITY_SYSTEM_ID,
        format!("{TASK_VERB_SECURITY_TEAM}_{team_idx}"),
    )
}

/// One ship's Security teams (issue #1346): the authored terms and every team's
/// live state.
///
/// Inserted at spawn only on a hull that authored a `[security]` table AND a
/// `kind = "security"` `[[system]]` — a hull with neither carries no component,
/// commands nothing and is byte-identical in every way to one built before this
/// existed (AGENTS.md rule 11).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShipSecurityTeams {
    /// The authored terms — team count, crossing times, reach.
    pub config: SecurityConfig,
    /// Every team, in authored index order. `teams.len() == config.team_count`.
    pub teams: Vec<SecurityTeam>,
    /// Why the last dispatch could not form, or why a live assignment was ended
    /// by the world rather than by the operator — the reason the console shows,
    /// retained until the operator dispatches or recalls again. A projection the
    /// next command clears; never folded, never saved.
    pub last_refusal: Option<SecurityRefusal>,
}

impl ShipSecurityTeams {
    /// A fresh muster: every authored team available, nothing refused.
    pub fn new(config: SecurityConfig) -> Self {
        let teams = vec![SecurityTeam::default(); config.team_count as usize];
        Self {
            config,
            teams,
            last_refusal: None,
        }
    }

    /// How many teams could take a fresh assignment right now.
    pub fn free_teams(&self) -> usize {
        self.teams.iter().filter(|t| t.is_available()).count()
    }

    /// The indices of the teams that could take a fresh assignment, lowest first
    /// — the deterministic order a backfill host spends them in.
    pub fn free_team_indices(&self) -> Vec<usize> {
        self.teams
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_available())
            .map(|(index, _)| index)
            .collect()
    }

    /// The persistable half — every committed team's assignment — for the
    /// snapshot payload (issue #1346). The authored config rides the template and
    /// is re-derived on spawn, exactly as `UmbilicalSaveState` leaves the flow
    /// terms out; the last refusal is a projection the next tick re-derives.
    pub fn save_state(&self) -> SecuritySaveState {
        SecuritySaveState {
            teams: self
                .teams
                .iter()
                .enumerate()
                .filter(|(_, team)| team.is_committed())
                .map(|(index, team)| SecurityTeamSaveState {
                    team_idx: index as u8,
                    state: team.state,
                    target: team.target.clone(),
                    action: team.action,
                    risk: team.risk,
                    elapsed: team.elapsed,
                    phase_duration: team.phase_duration,
                })
                .collect(),
        }
    }

    /// Reseed the committed teams from a restored snapshot, onto a muster that
    /// already carries its authored config from the fresh spawn. A team the save
    /// does not name was home when the snapshot was taken and stays home; the
    /// last refusal is NOT restored, because it is a projection the next tick
    /// re-derives and a stale one would name a condition that no longer holds.
    pub fn restore(&mut self, save: &SecuritySaveState) {
        for team in self.teams.iter_mut() {
            *team = SecurityTeam::default();
        }
        for row in &save.teams {
            let Some(team) = self.teams.get_mut(row.team_idx as usize) else {
                continue;
            };
            team.state = row.state;
            team.target = row.target.clone();
            team.action = row.action;
            team.risk = row.risk;
            team.elapsed = row.elapsed;
            team.phase_duration = row.phase_duration;
        }
        self.last_refusal = None;
    }
}

/// One committed team's assignment, as the snapshot carries it (issue #1346).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecurityTeamSaveState {
    pub team_idx: u8,
    pub state: SecurityTeamState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<SecurityAction>,
    #[serde(default)]
    pub risk: f32,
    #[serde(default)]
    pub elapsed: f32,
    #[serde(default)]
    pub phase_duration: f32,
}

/// The snapshot-carried half of a [`ShipSecurityTeams`] (issue #1346): the
/// committed teams, and nothing else.
///
/// `Default` is the idle muster — every team home — which is what a hull that
/// authored Security and never used it captures, so a resume of such a ship
/// restores byte-identically and folds the same number.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecuritySaveState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<SecurityTeamSaveState>,
}

/// A world entity's authored `[security_target]` table (issue #1346) — what
/// Security teams may be sent here to do.
///
/// The mirror of [`ShipSecurityTeams`]: that says what a hull can do the work
/// with, this says what work a target offers. An entity that authors no
/// `[security_target]` carries no component and cannot be dispatched to, which is
/// why every shipped hull and every existing world is untouched by this slice.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct SecurityTargetActions(pub SecurityTargetConfig);

/// Registers the Security systems and its admitted-command consumer (issue
/// #1346). Added by `WorldPlugin` alongside `UmbilicalPlugin`.
pub struct SecurityPlugin;

impl Plugin for SecurityPlugin {
    fn build(&self, app: &mut App) {
        // The security `[[system]]` is an admitted-command consumer:
        // `handle_security_commands` reads `DispatchSecurityTeam` /
        // `RecallSecurityTeam` for it, so admission fans those commands into every
        // ship's `AdmittedCommands` each tick and the end-of-frame lint never
        // warns them unrouted.
        app.register_admitted_consumer(ConsumerMatcher::exact(SECURITY_KIND, SECURITY_SYSTEM_ID));
        // Gated AI decider; `register_ai_cadence` is idempotent.
        crate::ai::cadence::register_ai_cadence(app);
        // Authoritative-state exclusion declaration (issue #1221, Track 3 step C9).
        // `SecurityAiDispatched` is the DERIVED "these teams are mine" marker.
        // `operate_security_ai`'s adoption pass re-derives it every AI tick from
        // the folded team states plus the candidate pool the still-folded world
        // and operate directive produce: a committed team on work that is still in
        // that pool is work this host would have sent it to, so the host adopts
        // it. Never a second copy of either input, so a lost marker — including
        // the one a snapshot restore leaves behind, which brings committed teams
        // back with no marker at all — self-heals within one AI tick. Declared
        // here at its owning site; inert to the digest.
        {
            use crate::authoritative::{DeclareState, StateClass};
            app.declare_state::<SecurityAiDispatched>(StateClass::Derived, "security-team-state");
        }
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Security AI: on the shared AI cadence (rule 7),
                // emitting Dispatch / Recall BEFORE `handle_security_commands`
                // consumes the tick.
                operate_security_ai
                    .in_set(crate::sim_sets::FixedStep::OperateSecurityAi)
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_security_commands),
                handle_security_commands
                    .in_set(crate::sim_sets::FixedStep::HandleSecurityCommands)
                    .in_set(crate::sim_sets::SimSet::Input),
                // The team clock runs in `Modifiers`, after the operators have
                // moved, so a target that drifted out of reach this tick is seen
                // to have done so this tick.
                tick_security_teams
                    .in_set(crate::sim_sets::FixedStep::TickSecurityTeams)
                    .in_set(crate::sim_sets::SimSet::Modifiers),
                publish_security_blackboard
                    .in_set(crate::sim_sets::FixedStep::PublishSecurityBlackboard)
                    .in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── The world, read once per tick ────────────────────────────────────────────

/// One dispatchable target, read once per tick into plain values so the borrow of
/// the world is released before the verdicts and the write.
struct TargetRow {
    uuid: String,
    name: Option<String>,
    position: Vec3,
    config: SecurityTargetConfig,
}

/// Read every entity that authors Security work into plain rows, in UUID order so
/// two hosts walk them identically (the rule every fold and tick here keeps).
fn target_rows(
    query: &Query<(
        &EntityUuid,
        Option<&EntityName>,
        &Transform,
        &SecurityTargetActions,
    )>,
) -> Vec<TargetRow> {
    let mut rows: Vec<TargetRow> = query
        .iter()
        .map(|(uuid, name, transform, actions)| TargetRow {
            uuid: uuid.0.clone(),
            name: name.map(|n| n.0.clone()),
            position: transform.translation,
            config: actions.0.clone(),
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

/// Whether an operator's Security System is damaged out.
fn security_disabled(hull: Option<&EntitySystemHull>) -> bool {
    hull.map(|h| {
        matches!(
            h.0.tier_for(&security_system_id()),
            DamageTier::Disabled | DamageTier::Destroyed
        )
    })
    .unwrap_or(false)
}

// ── The dispatch / recall commands ───────────────────────────────────────────

/// What one admitted command asked for, gathered before the world is re-borrowed.
enum Request {
    Dispatch {
        team_idx: u8,
        target: String,
        action: Option<SecurityAction>,
    },
    Recall {
        team_idx: u8,
    },
}

/// Take this tick's `DispatchSecurityTeam` / `RecallSecurityTeam` commands and
/// apply them (issue #1346).
///
/// Runs in `SimSet::Input`, so a team sent this tick starts crossing on the same
/// tick's `tick_security_teams`. Every command is answered: a dispatch that
/// cannot form leaves the named team where it was and records the one reason the
/// console shows, and a recall of a team that is not out is a no-op rather than
/// an error — the same latest-wins, idempotent policy the tractor and the external
/// repair dispatch take, so a stale-UI double tap is harmless.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak (a Tactical tenure token at the network gate, or the backfill host
/// through the same `validate_and_admit` seam) and stripped the source, so nothing
/// here asks who sent the command (AGENTS.md rule 6).
#[allow(clippy::type_complexity)]
pub fn handle_security_commands(
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    targets: Query<(
        &EntityUuid,
        Option<&EntityName>,
        &Transform,
        &SecurityTargetActions,
    )>,
    mut operators: Query<(
        Entity,
        &EntityUuid,
        &AdmittedCommands,
        &Transform,
        Option<&EntitySystemHull>,
        &mut ShipSecurityTeams,
    )>,
) {
    // UUID order, not archetype order: two hosts must take the same ship's
    // commands in the same sequence.
    let mut ordered: Vec<(String, Entity, Vec<Request>, Vec3, bool)> = Vec::new();
    for (entity, uuid, admitted, transform, hull, _) in operators.iter() {
        let requests: Vec<Request> = admitted
            .for_target(SECURITY_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::DispatchSecurityTeam {
                    team_idx,
                    target,
                    action,
                } => Some(Request::Dispatch {
                    team_idx: *team_idx,
                    target: target.clone(),
                    action: SecurityAction::parse(action),
                }),
                SystemControlPayload::RecallSecurityTeam { team_idx } => Some(Request::Recall {
                    team_idx: *team_idx,
                }),
                _ => None,
            })
            .collect();
        if requests.is_empty() {
            continue;
        }
        ordered.push((
            uuid.0.clone(),
            entity,
            requests,
            transform.translation,
            security_disabled(hull),
        ));
    }
    if ordered.is_empty() {
        return;
    }
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));
    let rows = target_rows(&targets);

    for (operator_uuid, entity, requests, operator_pos, disabled) in ordered {
        let Ok((_, _, _, _, _, mut security)) = operators.get_mut(entity) else {
            continue;
        };
        for request in requests {
            match request {
                Request::Dispatch {
                    team_idx,
                    target,
                    action,
                } => {
                    let row = rows.iter().find(|r| r.uuid == target);
                    let separation = row.map(|r| operator_pos.distance(r.position));
                    // An action id nothing answers to and a target that does not
                    // offer the (known) action are the same answer to the crew:
                    // that cannot be done here.
                    let offered = action
                        .and_then(|action| row.and_then(|r| r.config.action(action)))
                        .cloned();
                    let verdict = dispatch_status(
                        security.teams.get(team_idx as usize),
                        row.is_some(),
                        offered.is_some(),
                        separation,
                        security.config.range,
                        disabled,
                    );
                    match (verdict, offered, action) {
                        (Ok(()), Some(offered), Some(action)) => {
                            let crossing = security.config.deploy_duration_secs;
                            if let Some(team) = security.teams.get_mut(team_idx as usize) {
                                team.deploy(target.clone(), action, offered.risk, crossing);
                            }
                            security.last_refusal = None;
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                TaskLifecycleRequest::Start {
                                    slot: team_slot(&operator_uuid, team_idx),
                                    target: Some(target.clone()),
                                },
                            );
                        }
                        (Err(refusal), ..) => {
                            security.last_refusal = Some(refusal);
                        }
                        // `dispatch_status` returned Ok only because both the
                        // target and the action resolved, so this arm is
                        // unreachable; recorded rather than panicking.
                        (Ok(()), ..) => {
                            security.last_refusal = Some(SecurityRefusal::ActionUnavailable);
                        }
                    }
                }
                Request::Recall { team_idx } => {
                    let withdraw = security.config.withdraw_duration_secs;
                    let Some(team) = security.teams.get_mut(team_idx as usize) else {
                        security.last_refusal = Some(SecurityRefusal::NoSuchTeam);
                        continue;
                    };
                    if !team.is_interruptible() {
                        // Already home, or already on its way. A deliberate recall
                        // of a team that is not out is idempotent, not an error.
                        continue;
                    }
                    team.withdraw(withdraw);
                    security.last_refusal = None;
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        TaskLifecycleRequest::End {
                            slot: team_slot(&operator_uuid, team_idx),
                            reason: TaskTerminalReason::Released,
                        },
                    );
                }
            }
        }
    }
}

/// Push one lifecycle report, when the queue exists. `Option` so a reduced
/// fixture that runs these systems without the narrative plugin behaves exactly
/// as it did before this existed.
fn push_lifecycle(
    queue: Option<&mut EffectQueue<TaskLifecycleRequest>>,
    request: TaskLifecycleRequest,
) {
    if let Some(queue) = queue {
        queue.0.push(request);
    }
}

// ── The team clock ───────────────────────────────────────────────────────────

/// One team's decided transition, applied in the write phase.
struct Transition {
    operator: String,
    team_idx: u8,
    /// The lifecycle terminal this transition reports, when it ends an assignment.
    terminal: Option<TaskTerminalReason>,
    /// The world flag this transition raises, when a piece of work completed.
    outcome: Option<(String, i64)>,
    /// What the team becomes.
    next: TeamTransition,
    /// The refusal to show the crew, when the world ended the assignment rather
    /// than the operator.
    refusal: Option<SecurityRefusal>,
}

enum TeamTransition {
    BeginWork(f32),
    Withdraw,
    Home,
    Unavailable,
    Restore,
}

/// Advance every Security team through its assignment (issue #1346).
///
/// Runs in `SimSet::Modifiers`, after the operators have moved. Each committed
/// team's phase clock advances by the tick's delta and the four things that can
/// happen are decided in one place:
///
/// * **Arrival.** A deploying team that has crossed begins its authored work.
/// * **Success.** A working team that has finished raises the action's authored
///   `outcome_flag` on the world store — the ONLY consequence path, so a scenario
///   hangs a trigger off the flag and the engine never learns the scenario's names
///   — reports [`TaskTerminalReason::Completed`], and starts home.
/// * **Interruption.** A deploying or working team whose target has left the
///   world, or drifted past the authored reach, reports the matching terminal and
///   starts home with nothing banked. A team whose own System is knocked out is
///   pulled off the job entirely and marked `Unavailable` until it comes back.
/// * **Return.** A withdrawing team that has arrived is available again.
#[allow(clippy::type_complexity)]
pub fn tick_security_teams(
    time: Option<Res<Time>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    targets: Query<(
        &EntityUuid,
        Option<&EntityName>,
        &Transform,
        &SecurityTargetActions,
    )>,
    mut operators: Query<(
        &EntityUuid,
        &Transform,
        Option<&EntitySystemHull>,
        &mut ShipSecurityTeams,
    )>,
) {
    if operators.is_empty() {
        return;
    }
    let dt = time.map(|t| t.delta_secs()).unwrap_or(0.0);
    let rows = target_rows(&targets);

    // Decide from read-only reads first, in UUID order, so the queued flags and
    // lifecycle reports are identical on two hosts.
    let mut decided: Vec<Transition> = Vec::new();
    {
        let mut operator_rows: Vec<(String, Vec3, bool, ShipSecurityTeams)> = operators
            .iter()
            .map(|(uuid, transform, hull, security)| {
                (
                    uuid.0.clone(),
                    transform.translation,
                    security_disabled(hull),
                    security.clone(),
                )
            })
            .collect();
        operator_rows.sort_by(|a, b| a.0.cmp(&b.0));

        for (operator_uuid, operator_pos, disabled, security) in &operator_rows {
            for (index, team) in security.teams.iter().enumerate() {
                let team_idx = index as u8;
                // A knocked-out System takes every team off the board; one that
                // comes back releases the ones it held.
                if *disabled {
                    if team.state == SecurityTeamState::Unavailable {
                        continue;
                    }
                    decided.push(Transition {
                        operator: operator_uuid.clone(),
                        team_idx,
                        terminal: team
                            .is_interruptible()
                            .then_some(TaskTerminalReason::Disabled),
                        outcome: None,
                        next: TeamTransition::Unavailable,
                        refusal: Some(SecurityRefusal::Disabled),
                    });
                    continue;
                }
                if team.state == SecurityTeamState::Unavailable {
                    decided.push(Transition {
                        operator: operator_uuid.clone(),
                        team_idx,
                        terminal: None,
                        outcome: None,
                        next: TeamTransition::Restore,
                        refusal: None,
                    });
                    continue;
                }
                if !team.is_committed() {
                    continue;
                }

                // An assignment's target can vanish or drift; either ends the work
                // where it stands, and the team walks home with nothing banked.
                if team.is_interruptible() {
                    let target = team.target.as_deref().unwrap_or_default();
                    let row = rows.iter().find(|r| r.uuid == target);
                    let interruption = match row {
                        // `TargetLost` is what the emitter upgrades to
                        // `TargetDestroyed` when the subject really has left the
                        // world (issue #1341), so a scripted removal and a hull
                        // blown apart read correctly without this site guessing.
                        None => Some((
                            TaskTerminalReason::TargetLost,
                            SecurityRefusal::NoSuchTarget,
                        )),
                        Some(row)
                            if operator_pos.distance(row.position) > security.config.range =>
                        {
                            Some((TaskTerminalReason::OutOfRange, SecurityRefusal::OutOfRange))
                        }
                        Some(_) => None,
                    };
                    if let Some((terminal, refusal)) = interruption {
                        decided.push(Transition {
                            operator: operator_uuid.clone(),
                            team_idx,
                            terminal: Some(terminal),
                            outcome: None,
                            next: TeamTransition::Withdraw,
                            refusal: Some(refusal),
                        });
                        continue;
                    }
                }

                if team.elapsed + dt < team.phase_duration {
                    continue;
                }
                match team.state {
                    SecurityTeamState::Deploying => {
                        let duration = team
                            .action
                            .and_then(|action| {
                                rows.iter()
                                    .find(|r| Some(r.uuid.as_str()) == team.target.as_deref())
                                    .and_then(|r| r.config.action(action))
                            })
                            .map(|a| a.duration_secs)
                            .unwrap_or(0.0);
                        decided.push(Transition {
                            operator: operator_uuid.clone(),
                            team_idx,
                            terminal: None,
                            outcome: None,
                            next: TeamTransition::BeginWork(duration),
                            refusal: None,
                        });
                    }
                    SecurityTeamState::Working => {
                        let offered: Option<SecurityActionConfig> =
                            team.action.and_then(|action| {
                                rows.iter()
                                    .find(|r| Some(r.uuid.as_str()) == team.target.as_deref())
                                    .and_then(|r| r.config.action(action))
                                    .cloned()
                            });
                        decided.push(Transition {
                            operator: operator_uuid.clone(),
                            team_idx,
                            terminal: Some(TaskTerminalReason::Completed),
                            outcome: offered
                                .and_then(|a| a.outcome_flag.map(|flag| (flag, a.outcome_value))),
                            next: TeamTransition::Withdraw,
                            refusal: None,
                        });
                    }
                    SecurityTeamState::Withdrawing => decided.push(Transition {
                        operator: operator_uuid.clone(),
                        team_idx,
                        terminal: None,
                        outcome: None,
                        next: TeamTransition::Home,
                        refusal: None,
                    }),
                    SecurityTeamState::Available | SecurityTeamState::Unavailable => {}
                }
            }
        }
    }

    // Advance every committed clock and apply the decided transitions.
    for (uuid, _, _, mut security) in operators.iter_mut() {
        let withdraw = security.config.withdraw_duration_secs;
        for team in security.teams.iter_mut() {
            if team.is_committed() {
                team.elapsed += dt;
            }
        }
        for transition in decided.iter().filter(|t| t.operator == uuid.0) {
            let Some(team) = security.teams.get_mut(transition.team_idx as usize) else {
                continue;
            };
            match transition.next {
                TeamTransition::BeginWork(duration) => team.begin_work(duration),
                TeamTransition::Withdraw => team.withdraw(withdraw),
                TeamTransition::Home => team.arrive_home(),
                TeamTransition::Unavailable => {
                    team.arrive_home();
                    team.state = SecurityTeamState::Unavailable;
                }
                TeamTransition::Restore => team.arrive_home(),
            }
            if let Some(refusal) = transition.refusal {
                security.last_refusal = Some(refusal);
            }
        }
    }

    // Report the terminals and raise the consequences, in the order they were
    // decided (which is operator-uuid then team index).
    for transition in &decided {
        if let Some(reason) = transition.terminal {
            push_lifecycle(
                lifecycle.as_deref_mut(),
                TaskLifecycleRequest::End {
                    slot: team_slot(&transition.operator, transition.team_idx),
                    reason,
                },
            );
        }
    }
    if let Some(runtime) = runtime.as_deref_mut() {
        for transition in &decided {
            let Some((flag, value)) = &transition.outcome else {
                continue;
            };
            let (before, after) = runtime.flags.set_flag_value(flag, *value);
            if (before != 0) == (after != 0) {
                continue;
            }
            // The same mirror the infrastructure thresholds keep: a scenario hangs
            // its consequence off `on_flag_set`, so the edge has to reach the
            // world event stream, not just the store.
            runtime.pending_world_events.push(if after != 0 {
                WorldEvent::FlagSet {
                    name: flag.clone(),
                    origin_layer: None,
                }
            } else {
                WorldEvent::FlagCleared {
                    name: flag.clone(),
                    origin_layer: None,
                }
            });
        }
    }
}

// ── The backfill host ────────────────────────────────────────────────────────

/// Marks the teams the backfill host is driving (issue #1346), one bit per team
/// index. Claimed on dispatch, released on recall and the moment a team is home;
/// the host recalls only a team whose bit is set, so it never recalls one sent to
/// work the host itself would not have chosen.
///
/// Neither folded nor snapshotted, because it is genuinely derived:
/// `operate_security_ai` re-derives it each AI tick by ADOPTING every committed
/// team whose current target-and-action is still in this tick's candidate pool —
/// the pool built from the folded world and the folded operate directive. A
/// restore, which brings committed teams back with no marker at all, therefore
/// heals within one AI tick and the host goes on to recall those teams when their
/// job leaves the pool.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecurityAiDispatched(pub u32);

impl SecurityAiDispatched {
    /// Whether the host is driving the team at `index`.
    pub fn holds(self, index: usize) -> bool {
        index < 32 && self.0 & (1 << index) != 0
    }

    /// Claim the team at `index`.
    pub fn claim(&mut self, index: usize) {
        if index < 32 {
            self.0 |= 1 << index;
        }
    }

    /// Release the team at `index`.
    pub fn release(&mut self, index: usize) {
        if index < 32 {
            self.0 &= !(1 << index);
        }
    }
}

/// Build the backfill host's candidate pool from the world (issue #1346).
///
/// Every action every target offers becomes one candidate UNLESS its consequence
/// has already landed ([`already_done`]) — finished work leaves the pool, so the
/// host neither re-dispatches to it forever nor keeps a team standing on it. Each
/// candidate's authored priority is PROMOTED when a live `Secure` directive names
/// that target, which is how "urgent Objective" enters a ranking that is otherwise
/// entirely authored on the targets. `eligible` is the pure dispatch verdict with
/// a free team substituted in, so the host can never propose something the applier
/// would refuse.
///
/// This is the pool the RECALL arm reads too, which is why completion is filtered
/// here rather than at selection: a team still standing on work whose flag has
/// come up finds its job gone from the pool and is called home.
///
/// Pure but for its inputs, and deterministic: rows arrive in UUID order and each
/// target's actions in authored order, and [`select_assignments`] re-sorts by
/// priority with uuid/action tiebreaks anyway.
fn build_candidates(
    rows: &[TargetRow],
    operator_pos: Vec3,
    range: f32,
    ordered_targets: &[String],
    flags: Option<&crate::world::flags::FlagStore>,
) -> Vec<SecurityCandidate> {
    let mut candidates = Vec::new();
    for row in rows {
        let in_range = operator_pos.distance(row.position) <= range;
        let named = ordered_targets.contains(&row.uuid);
        for action in &row.config.actions {
            if already_done(action, flags) {
                continue;
            }
            let priority = if named {
                action.priority.promoted_by_objective()
            } else {
                action.priority
            };
            candidates.push(SecurityCandidate {
                target: row.uuid.clone(),
                action: action.action,
                priority,
                eligible: in_range,
            });
        }
    }
    candidates
}

/// Whether an authored action's consequence has ALREADY landed on the world flag
/// store (issue #1346) — the pool's completion condition.
///
/// The authored `outcome_flag` is the whole consequence path of a Security action
/// (`tick_security_teams` raises it on success and mirrors the edge onto the world
/// event stream), so a raised flag is the one durable, authored record that this
/// work is done. The host reads it back and stops proposing the job.
///
/// An action that authors NO flag has no completion condition anything can
/// observe — the field's own contract is "an action whose only consequence is the
/// doing of it" — so it stays in the pool. A scenario that wants a Security job to
/// be finishable authors the flag; that is the same lever its triggers already use.
fn already_done(
    action: &SecurityActionConfig,
    flags: Option<&crate::world::flags::FlagStore>,
) -> bool {
    match (action.outcome_flag.as_deref(), flags) {
        (Some(flag), Some(store)) => store.flag(flag),
        _ => false,
    }
}

/// Backfill Security AI (issue #1346).
///
/// Ranks every piece of Security work the ship can see by the priority order the
/// issue names — immediate life safety, an evacuation already underway, an urgent
/// Objective, active threat containment, then optional work — and commits its free
/// teams to the top of that list, reserving one rather than stranding a known
/// higher-priority job ([`select_assignments`]). The concrete command is exactly
/// the `DispatchSecurityTeam` a human at the Tactical console emits, sent through
/// the SAME `emit_ai_command` seam, so `handle_security_commands` never learns who
/// spoke (AGENTS.md rule 6).
///
/// The one thing a mission contributes is which targets are URGENT: a live
/// `Secure` directive naming a target promotes that target's authored classes to
/// [`SecurityPriority::UrgentObjective`] (never below what they already were).
/// With no directive at all the host still works the list — a fire is a fire
/// whether or not the mission mentioned it — which is what "prioritises immediate
/// threats to life" means when the Objective pool is silent.
///
/// A team the host committed is recalled when its job leaves the candidate pool:
/// the target left the world, or the work there is FINISHED — its authored
/// `outcome_flag` has come up, which is the one durable record of a Security
/// action's consequence ([`already_done`]). Which teams are the host's is
/// RE-DERIVED each tick rather than remembered: it adopts a committed team whose
/// work is still in the pool — work it would have sent that team to itself,
/// whoever actually did — and lets go of a team the moment that team is home. A
/// team on work outside the pool is nobody's but the console's and is left alone.
/// Decides ONLY on the shared AI cadence (rule 7).
#[allow(clippy::type_complexity)]
pub fn operate_security_ai(
    mut commands: Commands,
    sessions: Res<crate::lobby::Sessions>,
    runtime: Option<Res<WorldContentRuntime>>,
    targets: Query<(
        &EntityUuid,
        Option<&EntityName>,
        &Transform,
        &SecurityTargetActions,
    )>,
    mut ships: Query<(
        Entity,
        &EntityUuid,
        &Transform,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&EntitySystemHull>,
        &ShipSecurityTeams,
        &crate::server_app::ShipSystemBlackboards,
        Option<&SecurityAiDispatched>,
        &mut AdmittedCommands,
    )>,
) {
    let rows = target_rows(&targets);
    let system_id = security_system_id();

    let mut order: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (_, entity) in order {
        let Ok((
            entity,
            uuid,
            transform,
            sources,
            config,
            hull,
            security,
            blackboards,
            host_dispatched,
            mut admitted,
        )) = ships.get_mut(entity)
        else {
            continue;
        };
        if !sources.0.policy_for(&system_id).operate_ai {
            continue;
        }
        if security_disabled(hull) {
            continue;
        }

        // Every target a live `Secure` directive names, resolved to the uuid the
        // world carries — the same name→uuid resolution the external repair host
        // uses for a `FieldRepair` order.
        let ordered_targets: Vec<String> = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(vbb)) => vbb
                .scored_objectives
                .iter()
                .filter(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Security))
                .filter_map(|o| crate::objectives::secure_directive_target(&o.directive))
                .map(|name| {
                    runtime
                        .as_deref()
                        .and_then(|rt| rt.name_to_uuid.get(name).cloned())
                        .unwrap_or_else(|| name.to_string())
                })
                .collect(),
            _ => Vec::new(),
        };

        let candidates = build_candidates(
            &rows,
            transform.translation,
            security.config.range,
            &ordered_targets,
            runtime.as_deref().map(|rt| &rt.flags),
        );
        // What is actually left to assign. `select_assignments` returns at most
        // one pick per FREE team, so a pool still carrying the job a busy team is
        // already on would spend a free team's slot on a duplicate — and that team
        // would sit at home while the next job down went unworked. Each free team
        // is ranked against the best REMAINING job.
        let unassigned = unassigned_candidates(&candidates, &security.teams);
        let free = security.free_team_indices();
        let picks = select_assignments(&unassigned, free.len());

        let mut claimed = host_dispatched.copied().unwrap_or_default();
        let mut emitted = false;

        // A team that is home is nobody's: let go of it before anything else, so
        // the marker only ever means "the host is driving this COMMITTED team" and
        // a console dispatching that team next tick is never recalled by us.
        for (index, team) in security.teams.iter().enumerate() {
            if !team.is_committed() {
                claimed.release(index);
            }
        }

        // Then RE-DERIVE the claim, which is what makes the marker a derived
        // state rather than a second copy of one. A committed team whose current
        // (target, action) is still in this tick's candidate pool is a team this
        // host would have sent there itself, so it adopts it — and after a
        // snapshot restore, which brings back committed teams and no marker at
        // all, that is how the host remembers within one AI tick which teams are
        // its own and goes on to recall them when their job leaves the pool.
        //
        // The cost is deliberate and small: a team a console sent to work the
        // host also wanted becomes the host's, and comes home when that work is
        // done. A team on work the host would NOT have chosen — a job outside the
        // pool — is never adopted and never recalled by us.
        //
        // Teams in index order, candidates in the deterministic order the adapter
        // built: no map iteration feeds this.
        for (index, team) in security.teams.iter().enumerate() {
            if claimed.holds(index) || !team.is_committed() {
                continue;
            }
            let host_would_have_sent_it = team.target.as_deref().is_some_and(|target| {
                candidates
                    .iter()
                    .any(|c| c.target == target && Some(c.action) == team.action)
            });
            if host_would_have_sent_it {
                claimed.claim(index);
            }
        }

        // Recall next: a host-driven team whose job has left the pool — the target
        // left the world, or the work there is finished — has nothing left to do.
        // (Drifting out of reach does NOT leave the pool: the candidate is still
        // there, merely ineligible, and `tick_security_teams` ends that assignment
        // itself with its own refusal. One interruption, one owner.)
        for (index, team) in security.teams.iter().enumerate() {
            if !team.is_interruptible() || !claimed.holds(index) {
                continue;
            }
            let still_worth_it = team.target.as_deref().is_some_and(|target| {
                candidates
                    .iter()
                    .any(|c| c.target == target && Some(c.action) == team.action)
            });
            if still_worth_it {
                continue;
            }
            claimed.release(index);
            emit_ai_command(
                Some(uuid),
                system_id.clone(),
                SystemControlPayload::RecallSecurityTeam {
                    team_idx: index as u8,
                },
                sources,
                &sessions,
                config,
                &mut admitted,
            );
            emitted = true;
        }

        for (slot, pick) in picks.iter().enumerate() {
            let Some(team_idx) = free.get(slot) else {
                break;
            };
            let candidate = &unassigned[*pick];
            // An unreachable safety net, not a decision: `unassigned_candidates`
            // already removed every job a committed team holds, so a duplicate
            // here would mean the pool and the muster disagreed.
            debug_assert!(
                !is_assigned(candidate, &security.teams),
                "the selection pool must never offer work a team already holds"
            );
            if is_assigned(candidate, &security.teams) {
                continue;
            }
            claimed.claim(*team_idx);
            emit_ai_command(
                Some(uuid),
                system_id.clone(),
                SystemControlPayload::DispatchSecurityTeam {
                    team_idx: *team_idx as u8,
                    target: candidate.target.clone(),
                    action: candidate.action.as_str().to_string(),
                },
                sources,
                &sessions,
                config,
                &mut admitted,
            );
            emitted = true;
        }

        // Write the claim back whenever it says anything — an emitted command, a
        // marker already on the entity to keep up to date, or an adoption that
        // put a bit on where there was no marker at all. That last one is what
        // carries a re-derived claim across to the tick where the job leaves the
        // pool and the recall arm needs it.
        if emitted || host_dispatched.is_some() || claimed != SecurityAiDispatched::default() {
            commands.entity(entity).insert(claimed);
        }
    }
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each Security-fitted ship's blackboard under its system id (issue
/// #1346) — the team list, each team's state, assignment, progress and risk, and
/// the eligible targets with the actions each of them offers.
///
/// This is the whole data half of the repair-team-style interface AC2 asks for:
/// the console renders it and sends `DispatchSecurityTeam`/`RecallSecurityTeam`
/// back. Only ships that carry [`ShipSecurityTeams`] publish one, so a world whose
/// hulls author no `[security]` puts exactly the payload on the wire it did before
/// this existed. No English crosses: names are world entity name ids, actions and
/// priorities are machine ids, and the refusal and each action's warning are
/// `strings.csv` ids.
#[allow(clippy::type_complexity)]
pub fn publish_security_blackboard(
    targets: Query<(
        &EntityUuid,
        Option<&EntityName>,
        &Transform,
        &SecurityTargetActions,
    )>,
    mut ships: Query<(
        &Transform,
        &ShipSecurityTeams,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    if ships.is_empty() {
        return;
    }
    let key = security_system_id();
    let rows = target_rows(&targets);
    for (transform, security, mut blackboards) in ships.iter_mut() {
        let teams = security
            .teams
            .iter()
            .map(|team| SecurityTeamSlot {
                state: team.state.as_str().to_string(),
                target: team.target.clone(),
                target_name: team
                    .target
                    .as_deref()
                    .and_then(|t| rows.iter().find(|r| r.uuid == t))
                    .and_then(|r| r.name.clone()),
                action: team.action.map(|a| a.as_str().to_string()),
                progress: team.progress(),
                risk: team.risk,
            })
            .collect();
        let target_options = rows
            .iter()
            .map(|row| {
                let separation = transform.translation.distance(row.position);
                SecurityTargetOption {
                    uuid: row.uuid.clone(),
                    name: row.name.clone(),
                    separation,
                    in_range: separation <= security.config.range,
                    actions: row
                        .config
                        .actions
                        .iter()
                        .map(|action| SecurityActionOption {
                            action: action.action.as_str().to_string(),
                            duration_secs: action.duration_secs,
                            risk: action.risk,
                            priority: action.priority.as_str().to_string(),
                            warning: action.warning.clone(),
                        })
                        .collect(),
                }
            })
            .collect();
        let blackboard = SystemBlackboard::Security(SecurityBlackboard {
            range: security.config.range,
            teams,
            targets: target_options,
            refusal: security.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

/// The Security System's published blackboard channel key — its system id (issue
/// #1346). A convenience mirror of [`security_system_id`], matching
/// `umbilical_blackboard_key`.
pub fn security_blackboard_key() -> SystemId {
    security_system_id()
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
