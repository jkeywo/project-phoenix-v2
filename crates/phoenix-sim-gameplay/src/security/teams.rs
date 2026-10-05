//! The pure, Bevy-free heart of the Security System (issue #1346, PRD #1337).
//!
//! Security is people, not a beam: a hull authors a `[security]` table saying how
//! many teams it musters and how long they take to cross over and come back, and
//! a world entity authors a `[security_target]` table saying what those teams may
//! DO to it — the action, how long it takes, how dangerous it is, and the
//! consequence it leaves behind. Everything in this module is one of those two
//! halves, plus the two decisions that read them:
//!
//! * the **dispatch verdict** ([`dispatch_status`]) — may this team be sent to
//!   this target for this action right now, and if not, the one refusal the
//!   console shows;
//! * the **backfill selection** ([`select_assignments`]) — with a limited number
//!   of teams and a pool of things worth doing, which of them does an AI-operated
//!   Security seat actually commit to, in the priority order issue #1346 names,
//!   holding a team back when spending the lot would strand a higher-priority job.
//!
//! # Why this is a module of its own, Bevy-free (rule 10)
//!
//! Both decisions are made here, in isolation, and unit-tested here; the sibling
//! [`crate::security::server`] adapter gathers the real components — the teams,
//! the targets' authored actions, the separations — calls in, and applies what
//! comes back, deciding nothing itself. The split the tractor keeps between
//! `coupling` and `server`, and the umbilical between `flow` and `server`.
//!
//! # The engine never learns a scenario's names
//!
//! The action vocabulary ([`SecurityAction`]) is fixed and generic — secure /
//! contain, assist evacuation, board, place charges — and every number attached
//! to one (duration, risk, priority class, the flag its success raises) is
//! authored on the TARGET. Nothing here, and nothing in the adapter, branches on
//! a world entity's name: a compartment on the Falling Skyway and a boarding
//! target in some later mission are the same code path with different TOML.

use serde::{Deserialize, Serialize};

/// The generic vocabulary of things a Security team can be sent to do (issue
/// #1346).
///
/// A closed enum rather than a free string, because the engine must be able to
/// reject an authored action nobody implements at load rather than at the moment
/// a team is dispatched into a verb that does nothing. What each action MEANS at
/// a given target — how long, how risky, what it leaves behind — is the target's
/// authored business, which is what keeps this list scenario-agnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    /// Secure or contain the target — a fire, a breach, a hostile boarding party
    /// already aboard.
    SecureContain,
    /// Assist an evacuation already under way at the target.
    AssistEvacuation,
    /// Board the target.
    Board,
    /// Place demolition charges on the target.
    PlaceCharges,
}

impl SecurityAction {
    /// Every action, in declaration order (which is also `Ord`'s).
    pub const ALL: [SecurityAction; 4] = [
        SecurityAction::SecureContain,
        SecurityAction::AssistEvacuation,
        SecurityAction::Board,
        SecurityAction::PlaceCharges,
    ];

    /// The stable snake_case id written on the wire and authored in TOML. Matches
    /// the `serde(rename_all = "snake_case")` spelling above, hand-written so the
    /// wire vocabulary is visible where it is promised.
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityAction::SecureContain => "secure_contain",
            SecurityAction::AssistEvacuation => "assist_evacuation",
            SecurityAction::Board => "board",
            SecurityAction::PlaceCharges => "place_charges",
        }
    }

    /// Parse a wire/authored id back to an action, or `None` when nothing answers
    /// to it. Used by the command handler, which receives the id as a string off
    /// the wire and must refuse an unknown one rather than guess.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == id)
    }
}

/// How urgent a piece of Security work is, as issue #1346 orders it (issue
/// #1346).
///
/// `Ord` follows the declaration order, so the whole backfill policy — "immediate
/// life safety, an evacuation already underway, an urgent Objective, active
/// threat containment, then optional work" — is a `sort`, not a chain of `if`s.
/// A LOWER value is MORE urgent.
///
/// [`SecurityPriority::UrgentObjective`] is the one class an authored target
/// rarely declares for itself: it is what a live mission Objective naming that
/// target PROMOTES an authored class to, so "this compartment matters right now
/// because the mission says so" is a fact about the run rather than about the
/// TOML. Authoring it directly is still allowed — a scenario that knows a target
/// is objective-critical from the start may say so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityPriority {
    /// An immediate threat to life.
    LifeSafety,
    /// An evacuation that is already under way and would fail if abandoned.
    EvacuationUnderway,
    /// Work a live, urgent mission Objective names.
    UrgentObjective,
    /// Containing an active threat that is not yet killing anyone.
    ThreatContainment,
    /// Worth doing when there is nothing better.
    Optional,
}

impl SecurityPriority {
    /// Every class, most urgent first (declaration order, which is `Ord`'s).
    pub const ALL: [SecurityPriority; 5] = [
        SecurityPriority::LifeSafety,
        SecurityPriority::EvacuationUnderway,
        SecurityPriority::UrgentObjective,
        SecurityPriority::ThreatContainment,
        SecurityPriority::Optional,
    ];

    /// The stable snake_case id written on the wire and authored in TOML.
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityPriority::LifeSafety => "life_safety",
            SecurityPriority::EvacuationUnderway => "evacuation_underway",
            SecurityPriority::UrgentObjective => "urgent_objective",
            SecurityPriority::ThreatContainment => "threat_containment",
            SecurityPriority::Optional => "optional",
        }
    }

    /// The class a live Objective naming this target promotes an authored class
    /// to (issue #1346): never DOWN. An authored life-safety job that an
    /// Objective also names stays life safety — the mission cannot make a fire
    /// less urgent by mentioning it.
    pub fn promoted_by_objective(self) -> Self {
        self.min(SecurityPriority::UrgentObjective)
    }
}

/// The authored `[security]` terms for a hull that musters Security teams (issue
/// #1346).
///
/// Every field is a designer's number, read from TOML: AGENTS.md rule 11, no
/// hardcoded gameplay values. A hull that authors no `[security]` table carries
/// no [`crate::security::server::ShipSecurityTeams`] component, has no Security
/// System to command, and is unchanged in every way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityConfig {
    /// How many teams this hull musters. Each is dispatched and recalled
    /// independently and holds at most one assignment.
    pub team_count: u8,
    /// Seconds a team spends crossing to a target before its work begins.
    pub deploy_duration_secs: f32,
    /// Seconds a team spends coming home after its work ends (or is stopped).
    pub withdraw_duration_secs: f32,
    /// The furthest a target may sit from the operator and still receive a team,
    /// in world units. Dispatching past it is refused
    /// ([`SecurityRefusal::OutOfRange`]); drifting past it once a team is over
    /// there brings the team home.
    pub range: f32,
}

impl SecurityConfig {
    /// Reject an authored `[security]` table that describes a capability that
    /// could never do anything (issue #1346) — a hull with no teams, a negative
    /// crossing time, or a reach that puts every target out of range. Each is an
    /// author mistake whose only other symptom would be a console control the
    /// crew can press that quietly never helps anyone.
    pub fn validate(&self) -> Result<(), String> {
        if self.team_count == 0 {
            return Err("[security] team_count must be at least one team".to_string());
        }
        if !self.deploy_duration_secs.is_finite() || self.deploy_duration_secs < 0.0 {
            return Err(format!(
                "[security] deploy_duration_secs must be a non-negative number of seconds, got {}",
                self.deploy_duration_secs
            ));
        }
        if !self.withdraw_duration_secs.is_finite() || self.withdraw_duration_secs < 0.0 {
            return Err(format!(
                "[security] withdraw_duration_secs must be a non-negative number of seconds, got {}",
                self.withdraw_duration_secs
            ));
        }
        if !self.range.is_finite() || self.range <= 0.0 {
            return Err(format!(
                "[security] range must be a positive distance, got {}",
                self.range
            ));
        }
        Ok(())
    }
}

/// One action a target authors as available to a Security team (issue #1346).
///
/// This is the whole of what the engine knows about "what can be done here": the
/// verb, how long it takes, how dangerous the crew are told it is, how urgent it
/// is, and the world flag its success raises. A scenario adds a containment path
/// by authoring one of these; it never adds Rust.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityActionConfig {
    /// Which of the generic verbs this is.
    pub action: SecurityAction,
    /// Seconds a team works here once it has arrived.
    pub duration_secs: f32,
    /// How dangerous this work is, 0.0–1.0, shown to the crew as the assignment's
    /// risk. A displayed authored figure, deliberately NOT a dice roll: the
    /// simulation is seeded and bit-identical across hosts, and a hidden roll here
    /// would be one more thing two hosts could disagree about for no gain in
    /// meaning.
    pub risk: f32,
    /// How urgent this work is, which is what the backfill selection ranks on.
    pub priority: SecurityPriority,
    /// The world flag this action's SUCCESS raises, so a scenario can hang a
    /// consequence off it. A machine id, never English. `None` for an action whose
    /// only consequence is the doing of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_flag: Option<String>,
    /// The counter value [`Self::outcome_flag`] is set to on success. Defaults to
    /// 1 — the plain boolean flag — so a scenario that only wants "this happened"
    /// authors nothing.
    #[serde(default = "default_outcome_value")]
    pub outcome_value: i64,
    /// A `strings.csv` id for the warning the console shows beside this action —
    /// what the crew are being asked to walk into. Never English (AGENTS.md rule
    /// 11). `None` for work that needs no warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn default_outcome_value() -> i64 {
    1
}

impl SecurityActionConfig {
    /// Reject an authored action that could never resolve (issue #1346): a
    /// non-positive duration (a team that arrives and is instantly finished), a
    /// risk outside the 0–1 band the console renders, or a blank outcome flag.
    pub fn validate(&self) -> Result<(), String> {
        if !self.duration_secs.is_finite() || self.duration_secs <= 0.0 {
            return Err(format!(
                "[[security_target.action]] '{}' duration_secs must be a positive number of \
                 seconds, got {}",
                self.action.as_str(),
                self.duration_secs
            ));
        }
        if !self.risk.is_finite() || !(0.0..=1.0).contains(&self.risk) {
            return Err(format!(
                "[[security_target.action]] '{}' risk must be between 0.0 and 1.0, got {}",
                self.action.as_str(),
                self.risk
            ));
        }
        if self
            .outcome_flag
            .as_deref()
            .is_some_and(|flag| flag.trim().is_empty())
        {
            return Err(format!(
                "[[security_target.action]] '{}' outcome_flag must name a flag, or be omitted",
                self.action.as_str()
            ));
        }
        if self
            .warning
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(format!(
                "[[security_target.action]] '{}' warning must be a strings.csv id, or be omitted",
                self.action.as_str()
            ));
        }
        Ok(())
    }
}

/// The authored `[security_target]` table on a world entity Security teams can be
/// sent to (issue #1346) — the mirror image of [`SecurityConfig`]: that says what
/// a hull can DO the work with, this says what work a target OFFERS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityTargetConfig {
    /// The actions available here, authored as `[[security_target.action]]`
    /// blocks. Order is the author's; the engine sorts by priority wherever
    /// ordering matters, so a reshuffle of the TOML never moves the simulation.
    #[serde(rename = "action")]
    pub actions: Vec<SecurityActionConfig>,
}

impl SecurityTargetConfig {
    /// Reject a target that offers nothing, or offers the same verb twice — the
    /// second would be unreachable, because a dispatch names an action id and the
    /// first match answers.
    pub fn validate(&self) -> Result<(), String> {
        if self.actions.is_empty() {
            return Err(
                "[security_target] needs at least one [[security_target.action]] block — a target \
                 that offers nothing cannot be dispatched to"
                    .to_string(),
            );
        }
        for action in &self.actions {
            action.validate()?;
        }
        for (index, action) in self.actions.iter().enumerate() {
            if self.actions[..index]
                .iter()
                .any(|a| a.action == action.action)
            {
                return Err(format!(
                    "[security_target] authors the action '{}' twice; a dispatch names one action \
                     id, so the second block could never be reached",
                    action.action.as_str()
                ));
            }
        }
        Ok(())
    }

    /// The authored terms for one action here, or `None` when this target does
    /// not offer it.
    pub fn action(&self, action: SecurityAction) -> Option<&SecurityActionConfig> {
        self.actions.iter().find(|a| a.action == action)
    }
}

/// The one reason a Security dispatch was refused (or a team already out was
/// brought home) this tick (issue #1346), as the console shows it — a
/// `strings.csv` id, never English. Mirrors
/// [`crate::console::repair::ExternalRepairRefusal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityRefusal {
    /// The Security System is damaged out, so nobody is going anywhere.
    Disabled,
    /// The command named a team index this hull does not muster.
    NoSuchTeam,
    /// That team already holds an assignment. One team, one job.
    TeamBusy,
    /// Nothing in the world answers to the named target, or what does offers no
    /// Security work at all.
    NoSuchTarget,
    /// The target is real, but it does not offer the action that was asked for.
    ActionUnavailable,
    /// The target sits further than the authored `range`.
    OutOfRange,
}

impl SecurityRefusal {
    /// Every refusal, in declaration order — the coverage guard for the
    /// `strings.csv` rows below.
    pub const ALL: [SecurityRefusal; 6] = [
        SecurityRefusal::Disabled,
        SecurityRefusal::NoSuchTeam,
        SecurityRefusal::TeamBusy,
        SecurityRefusal::NoSuchTarget,
        SecurityRefusal::ActionUnavailable,
        SecurityRefusal::OutOfRange,
    ];

    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed `format!("security.dispatch.refused.{...}")`, so
    /// `check-strings.mjs` can see every id a new variant needs a row for.
    pub fn string_id(self) -> &'static str {
        match self {
            SecurityRefusal::Disabled => "security.dispatch.refused.disabled",
            SecurityRefusal::NoSuchTeam => "security.dispatch.refused.no_such_team",
            SecurityRefusal::TeamBusy => "security.dispatch.refused.team_busy",
            SecurityRefusal::NoSuchTarget => "security.dispatch.refused.no_such_target",
            SecurityRefusal::ActionUnavailable => "security.dispatch.refused.action_unavailable",
            SecurityRefusal::OutOfRange => "security.dispatch.refused.out_of_range",
        }
    }
}

/// Where one Security team is in its assignment (issue #1346) — the five states
/// the issue names.
///
/// A team's whole life is `Available → Deploying → Working → Withdrawing →
/// Available`, with `Unavailable` standing outside it for a team that cannot be
/// used at all because the Security System itself is damaged out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityTeamState {
    /// Mustered aboard, holding no assignment, ready to be sent.
    #[default]
    Available,
    /// Crossing to its assigned target; the work has not started.
    Deploying,
    /// At the target, doing the assigned action.
    Working,
    /// Coming home, whether the work finished, was recalled, or was interrupted.
    Withdrawing,
    /// Cannot be used: the Security System is disabled or destroyed.
    Unavailable,
}

impl SecurityTeamState {
    /// The stable snake_case id written on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityTeamState::Available => "available",
            SecurityTeamState::Deploying => "deploying",
            SecurityTeamState::Working => "working",
            SecurityTeamState::Withdrawing => "withdrawing",
            SecurityTeamState::Unavailable => "unavailable",
        }
    }
}

/// One Security team's live state (issue #1346).
///
/// `phase_duration` is the length of whatever the team is doing NOW — the
/// crossing, the work, or the return — so `progress` is one division whichever
/// state it is in, and the console does not have to know which authored number to
/// divide by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SecurityTeam {
    /// Which of the five states this team is in.
    pub state: SecurityTeamState,
    /// The uuid of the target it is assigned to, while it holds an assignment.
    pub target: Option<String>,
    /// The action it was sent to perform, while it holds an assignment.
    pub action: Option<SecurityAction>,
    /// The authored risk of that action, carried so the console can show it
    /// without re-reading the target.
    pub risk: f32,
    /// Seconds spent in the current state.
    pub elapsed: f32,
    /// Seconds the current state lasts. Zero when the team holds no assignment.
    pub phase_duration: f32,
}

impl SecurityTeam {
    /// Whether this team can take a fresh assignment right now.
    pub fn is_available(&self) -> bool {
        self.state == SecurityTeamState::Available
    }

    /// Whether this team is out on a job — deploying, working, or coming home.
    /// A withdrawing team counts as committed: it is not back yet.
    pub fn is_committed(&self) -> bool {
        matches!(
            self.state,
            SecurityTeamState::Deploying
                | SecurityTeamState::Working
                | SecurityTeamState::Withdrawing
        )
    }

    /// Whether this team is doing something that a lost target or a lost range
    /// would interrupt. A withdrawing team is already on its way home, so nothing
    /// about the target can interrupt it any more.
    pub fn is_interruptible(&self) -> bool {
        matches!(
            self.state,
            SecurityTeamState::Deploying | SecurityTeamState::Working
        )
    }

    /// How far through the current state this team is, 0.0–1.0. A zero-length
    /// state reads as complete rather than dividing by zero.
    pub fn progress(&self) -> f32 {
        if self.phase_duration <= 0.0 {
            return 1.0;
        }
        (self.elapsed / self.phase_duration).clamp(0.0, 1.0)
    }

    /// Put the team on a fresh assignment, crossing over.
    pub fn deploy(&mut self, target: String, action: SecurityAction, risk: f32, crossing: f32) {
        self.state = SecurityTeamState::Deploying;
        self.target = Some(target);
        self.action = Some(action);
        self.risk = risk;
        self.elapsed = 0.0;
        self.phase_duration = crossing;
    }

    /// Start the work, having arrived.
    pub fn begin_work(&mut self, duration: f32) {
        self.state = SecurityTeamState::Working;
        self.elapsed = 0.0;
        self.phase_duration = duration;
    }

    /// Send the team home, whatever it was doing. The assignment is retained
    /// until it arrives, so the console can still say where it has been.
    pub fn withdraw(&mut self, duration: f32) {
        self.state = SecurityTeamState::Withdrawing;
        self.elapsed = 0.0;
        self.phase_duration = duration;
    }

    /// The team is home: clear the assignment.
    pub fn arrive_home(&mut self) {
        self.state = SecurityTeamState::Available;
        self.target = None;
        self.action = None;
        self.risk = 0.0;
        self.elapsed = 0.0;
        self.phase_duration = 0.0;
    }
}

/// **The dispatch verdict.** `Ok(())` when `team` may be sent to the named target
/// for the named action this instant, else the one refusal the console shows
/// (issue #1346).
///
/// Pure: the adapter reads the live world into these scalars and applies the
/// answer. Re-run every tick a team is committed with `team` reported available
/// and the captured target present, so the only things that can drop a live
/// assignment are the target vanishing, drifting past the range, or the System
/// being knocked out.
///
/// # Check order is the console's "most actionable first"
///
/// A knocked-out Security System sends nobody anywhere, so it is reported before
/// anything about a particular team; a team that cannot go is reported before
/// anything about the target it was going to, the same tool-state-before-
/// acquisition order [`crate::console::repair::external::dispatch_status`] takes.
/// Among the acquisition checks there is no action to look up on a target that
/// does not exist and no range to a target that was never found, so the order is
/// target, then action, then range.
///
/// `separation` is the distance from the operator to the target, or `None` when
/// the target cannot be located — which reads as out of range, because nothing
/// that cannot be found is within reach.
pub fn dispatch_status(
    team: Option<&SecurityTeam>,
    target_known: bool,
    action_available: bool,
    separation: Option<f32>,
    range: f32,
    disabled: bool,
) -> Result<(), SecurityRefusal> {
    if disabled {
        return Err(SecurityRefusal::Disabled);
    }
    let Some(team) = team else {
        return Err(SecurityRefusal::NoSuchTeam);
    };
    if !team.is_available() {
        return Err(SecurityRefusal::TeamBusy);
    }
    if !target_known {
        return Err(SecurityRefusal::NoSuchTarget);
    }
    if !action_available {
        return Err(SecurityRefusal::ActionUnavailable);
    }
    match separation {
        Some(sep) if sep <= range => Ok(()),
        _ => Err(SecurityRefusal::OutOfRange),
    }
}

/// One piece of Security work the backfill host can see (issue #1346).
///
/// Built by the adapter from every reachable target's authored actions, with
/// `priority` already promoted by any live Objective naming that target, and
/// `eligible` already answering "could a team actually be sent to this right
/// now" — which is the pure [`dispatch_status`] verdict with a free team
/// substituted in, so the host never proposes something the applier would refuse.
#[derive(Clone, Debug, PartialEq)]
pub struct SecurityCandidate {
    /// The target's uuid.
    pub target: String,
    /// The action to perform there.
    pub action: SecurityAction,
    /// How urgent it is, after any Objective promotion.
    pub priority: SecurityPriority,
    /// Whether a team could be sent to it this instant.
    pub eligible: bool,
}

/// **The backfill selection.** Which candidates an AI-operated Security seat
/// commits its free teams to, in issue #1346's priority order, holding a team
/// back rather than stranding a known higher-priority job (issue #1346).
///
/// Returns indices into `candidates`, in the order the teams should be spent.
///
/// # The order
///
/// Candidates are ranked by [`SecurityPriority`] — life safety, an evacuation
/// already underway, an urgent Objective, active threat containment, then
/// optional work — and ties are broken by target uuid and then action id, so two
/// hosts walking the same world in different archetype order select the same
/// work. No clock, no RNG, no `HashMap` walk.
///
/// # The reservation
///
/// "Reserve capacity when spending both teams would make a known higher-priority
/// task impossible" is one rule, applied at exactly one moment: before the LAST
/// free team is committed. If any candidate of STRICTLY higher priority exists
/// that is not currently eligible — the burning compartment that is out of reach
/// this second but will not be in ten — the last team stays home rather than
/// being spent on the lesser job. The reserve is the direct analogue of the
/// external-repair host's "one team per outstanding critical local repair"
/// (`crate::console::repair::external_server`): both fold a policy into the one
/// availability answer rather than second-guessing the applier, and both can only
/// ever make the host MORE conservative than the applier's own gate — so a
/// selection this returns is always one `dispatch_status` admits.
///
/// A candidate that is already the most urgent thing known is never held back:
/// there is nothing above it for the reserve to protect.
pub fn select_assignments(candidates: &[SecurityCandidate], free_teams: usize) -> Vec<usize> {
    if free_teams == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|a, b| {
        let (a, b) = (&candidates[*a], &candidates[*b]);
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.action.as_str().cmp(b.action.as_str()))
    });

    let mut chosen: Vec<usize> = Vec::new();
    for index in order {
        if chosen.len() == free_teams {
            break;
        }
        let candidate = &candidates[index];
        if !candidate.eligible {
            continue;
        }
        let last_team = chosen.len() + 1 == free_teams;
        if last_team && higher_priority_pending(candidates, candidate.priority) {
            // Spending this team would leave nothing for the more urgent job we
            // already know about. Hold it.
            continue;
        }
        chosen.push(index);
    }
    chosen
}

/// Whether any candidate strictly more urgent than `priority` is known but not
/// currently dispatchable — the condition the reservation above protects.
fn higher_priority_pending(candidates: &[SecurityCandidate], priority: SecurityPriority) -> bool {
    candidates
        .iter()
        .any(|c| !c.eligible && c.priority < priority)
}

/// **The pool the selection actually ranks:** every candidate no team is already
/// committed to (issue #1346).
///
/// [`select_assignments`] is asked for at most one pick per FREE team, so a pool
/// that still carried the job a busy team is already working would spend a free
/// team's slot on a duplicate the caller can only drop — and the free team would
/// sit at home while genuinely unassigned work went unworked. Filtering FIRST is
/// what makes every pick a real assignment: each free team is matched against the
/// best REMAINING job rather than against the best job overall.
///
/// It keeps the reservation honest too. Work already under way needs no team held
/// back for it, so removing it from the pool also removes it from
/// [`higher_priority_pending`]'s reckoning.
///
/// Order-preserving, so the deterministic pool order the adapter built survives.
pub fn unassigned_candidates(
    candidates: &[SecurityCandidate],
    teams: &[SecurityTeam],
) -> Vec<SecurityCandidate> {
    candidates
        .iter()
        .filter(|candidate| !is_assigned(candidate, teams))
        .cloned()
        .collect()
}

/// Whether some committed team already holds exactly this target-and-action.
pub fn is_assigned(candidate: &SecurityCandidate, teams: &[SecurityTeam]) -> bool {
    teams.iter().any(|team| {
        team.is_committed()
            && team.target.as_deref() == Some(candidate.target.as_str())
            && team.action == Some(candidate.action)
    })
}

#[cfg(test)]
#[path = "teams_tests.rs"]
mod tests;
