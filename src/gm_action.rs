//! Typed, attributed Game Master actions (issue #1292).
//!
//! GM actions are authoritative simulation input, but they cannot use the
//! ordinary `FixedUpdate` command lane: `SetSessionPaused { active: true }`
//! deliberately starves that schedule, so its matching resume must remain
//! consumable from `PreUpdate`. The host mesh therefore carries a narrow,
//! authenticated grant whose absolute value is applied at a deterministic tick
//! boundary. Every peer stores the same bounded, canonically ordered journal;
//! arrival order is never an input to the result.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command_admission::HostSlot;

/// The ordinary hard protocol bound for one run. The journal never evicts an
/// entry: doing so would forget the idempotency key needed by a late duplicate.
///
/// One additional, bounded slot is reserved for an owner-sequenced Resume when
/// the ordinary lane filled while paused. Without that escape hatch a bounded
/// input queue could turn a deliberate Pause into a permanent deadlock. Once
/// used, the lane is exhausted in the safe (running) state.
pub const MAX_GM_ACTIONS_PER_RUN: usize = 4096;
pub const MAX_STORED_GM_ACTIONS_PER_RUN: usize = MAX_GM_ACTIONS_PER_RUN + 1;

/// Product pause state shared by raw local-host controls and typed GM actions.
/// The latter is the only replicated writer.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationPaused(pub bool);

/// Durable idempotency identity for one attributed GM mutation.
///
/// This deliberately is not `ActionCorrelationId`: that client-feedback type
/// is documented as transient and absent from logs, snapshots, mesh frames and
/// replay. A GM action needs the opposite contract while retaining the same
/// small opaque visible-ASCII wire shape for its UI correlation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GmActionId(String);

impl GmActionId {
    pub const MAX_BYTES: usize = 64;

    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty() {
            return Err("GM action id must not be empty");
        }
        if value.len() > Self::MAX_BYTES {
            return Err("GM action id is too long");
        }
        if !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err("GM action id must contain visible ASCII only");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for GmActionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// The first typed GM action family. Additive variants use this same grant,
/// ordering, result and replay path rather than raw ECS mutation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmAction {
    SetSessionPaused { active: bool },
}

/// Validated browser ingress before its technical slot and deterministic order
/// are attached by privileged admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GmActionRequest {
    pub operator_id: String,
    pub correlation: GmActionId,
    pub action: GmAction,
}

/// An authenticated GM request before the technical fleet owner has assigned
/// its canonical sequence and application boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmActionProposal {
    pub from: HostSlot,
    pub operator_id: String,
    pub correlation: GmActionId,
    pub action: GmAction,
}

impl GmActionProposal {
    pub fn validate(&self) -> Result<(), GmActionRefusalReason> {
        if self.operator_id.is_empty()
            || self.operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
        {
            return Err(GmActionRefusalReason::InvalidOperator);
        }
        Ok(())
    }
}

impl GmAction {
    pub fn requested_pause(&self) -> bool {
        match self {
            Self::SetSessionPaused { active } => *active,
        }
    }
}

/// Owner-assigned total order for actions, including multiple actions sharing
/// one paused application boundary. `sequence` is globally contiguous and
/// unique within the journal; `origin` preserves requester attribution and is
/// a defensive tie-break for malformed diagnostics, never a second sequencer.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct GmActionOrder {
    pub sequence: u64,
    pub origin: HostSlot,
}

impl GmActionOrder {
    pub const fn new(origin: HostSlot, sequence: u64) -> Self {
        Self { sequence, origin }
    }
}

/// One authenticated, replicated GM command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmActionGrant {
    /// The GM peer that requested the action. The frozen private roster binds
    /// it to `operator_id`; [`sequenced_by`](Self::sequenced_by) is the mesh
    /// sender authenticated on a committed decision.
    pub from: HostSlot,
    /// The technical fleet owner that assigned `order.sequence` and
    /// `apply_tick`. This is ordering machinery only, never product authority.
    pub sequenced_by: HostSlot,
    pub operator_id: String,
    pub correlation: GmActionId,
    /// Logical boundary at which the absolute value becomes authoritative.
    pub apply_tick: u64,
    pub order: GmActionOrder,
    pub action: GmAction,
}

impl GmActionGrant {
    pub fn key(&self) -> (u64, GmActionOrder) {
        (self.apply_tick, self.order)
    }

    pub fn validate(&self) -> Result<(), GmActionRefusalReason> {
        if self.from != self.order.origin {
            return Err(GmActionRefusalReason::OriginMismatch);
        }
        if self.operator_id.is_empty()
            || self.operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
        {
            return Err(GmActionRefusalReason::InvalidOperator);
        }
        Ok(())
    }
}

/// A canonical owner refusal. Refusals do not enter the simulation fold, but
/// they travel through the same authenticated owner decision lane so every GM
/// sees the same terminal answer and the requester cannot remain Pending.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmActionRefusal {
    pub sequenced_by: HostSlot,
    pub requester: HostSlot,
    pub operator_id: String,
    pub correlation: GmActionId,
    pub requested_active: bool,
    pub tick: u64,
    pub reason: GmActionRefusalReason,
}

impl GmActionRefusal {
    pub fn logged(&self) -> LoggedGmAction {
        LoggedGmAction::refused(
            self.operator_id.clone(),
            self.correlation.clone(),
            self.requested_active,
            self.tick,
            self.reason,
        )
    }
}

/// The one Rust-owned `gm-action` wire body. JavaScript ferries it opaquely.
/// Requests may originate at any equal GM peer; only an owner decision can
/// mutate the durable journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmActionFrame {
    Proposal(GmActionProposal),
    Granted(GmActionGrant),
    Refused(GmActionRefusal),
}

impl GmActionFrame {
    /// The slot whose transport identity authenticates this frame.
    pub fn wire_from(&self) -> HostSlot {
        match self {
            Self::Proposal(proposal) => proposal.from,
            Self::Granted(grant) => grant.sequenced_by,
            Self::Refused(refusal) => refusal.sequenced_by,
        }
    }

    pub fn tick(&self) -> u64 {
        match self {
            Self::Proposal(_) => 0,
            Self::Granted(grant) => grant.apply_tick,
            Self::Refused(refusal) => refusal.tick,
        }
    }
}

/// Validate the two distinct authorities on a GM mesh body: proposals belong
/// to their authenticated GM binding, while terminal decisions belong to the
/// technical owner and retain the requester's binding separately.
pub fn validate_fleet_frame(
    frame: &GmActionFrame,
    roster: &crate::lockstep::FleetRoster,
) -> Result<(), GmActionRefusalReason> {
    match frame {
        GmActionFrame::Proposal(proposal) => {
            proposal.validate()?;
            if roster.gm_operator(proposal.from) != Some(proposal.operator_id.as_str()) {
                return Err(GmActionRefusalReason::OperatorMismatch);
            }
        }
        GmActionFrame::Granted(grant) => {
            grant.validate()?;
            if grant.sequenced_by != roster.owner() {
                return Err(GmActionRefusalReason::OriginMismatch);
            }
            if roster.gm_operator(grant.from) != Some(grant.operator_id.as_str()) {
                return Err(GmActionRefusalReason::OperatorMismatch);
            }
        }
        GmActionFrame::Refused(refusal) => {
            if refusal.sequenced_by != roster.owner() {
                return Err(GmActionRefusalReason::OriginMismatch);
            }
            if roster.gm_operator(refusal.requester) != Some(refusal.operator_id.as_str()) {
                return Err(GmActionRefusalReason::OperatorMismatch);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmActionOutcome {
    Applied,
    NoOp,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmActionRefusalReason {
    NotInFleet,
    NotGameMaster,
    OperatorMismatch,
    InvalidOperator,
    OriginMismatch,
    ConflictingGrant,
    NonContiguousSequence,
    JournalFull,
    WrongPhase,
    UnreadableRequest,
}

/// One terminal fact in the GM command log and local activity projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedGmAction {
    pub operator_id: String,
    pub correlation: GmActionId,
    pub requested_active: bool,
    pub outcome: GmActionOutcome,
    pub tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<GmActionRefusalReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<GmActionOrder>,
}

impl LoggedGmAction {
    pub fn refused(
        operator_id: String,
        correlation: GmActionId,
        requested_active: bool,
        tick: u64,
        reason: GmActionRefusalReason,
    ) -> Self {
        Self {
            operator_id,
            correlation,
            requested_active,
            outcome: GmActionOutcome::Refused,
            tick,
            reason: Some(reason),
            order: None,
        }
    }
}

/// Durable, authoritative command/idempotency journal. It is captured and
/// folded. Terminal outcomes are re-derived from this canonical sequence.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GmActionJournal {
    /// State immediately before the first retained grant. Normally false; a
    /// standalone restore of an older raw host pause adopts true so its first
    /// typed Resume is an Applied transition rather than an impossible No-op.
    initial_paused: bool,
    /// Canonical prefix that has actually passed through `apply_due_actions`.
    /// This is deliberately not inferred from `SimTick`: outside FixedLast the
    /// tick names the next step, so a grant at exactly that value is still
    /// pending until the next PreUpdate.
    applied_grants: usize,
    grants: Vec<GmActionGrant>,
}

impl<'de> Deserialize<'de> for GmActionJournal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StoredJournal {
            initial_paused: bool,
            applied_grants: usize,
            grants: Vec<GmActionGrant>,
        }

        let stored = StoredJournal::deserialize(deserializer)?;
        if stored.grants.len() > MAX_STORED_GM_ACTIONS_PER_RUN {
            return Err(serde::de::Error::custom(
                "GM action journal exceeds its run bound",
            ));
        }
        let mut journal = Self {
            initial_paused: stored.initial_paused,
            applied_grants: 0,
            grants: Vec::new(),
        };
        for grant in stored.grants {
            match journal
                .insert(grant)
                .map_err(|reason| serde::de::Error::custom(format!("{reason:?}")))?
            {
                GmActionInsert::Inserted => {}
                GmActionInsert::Duplicate => {
                    return Err(serde::de::Error::custom(
                        "GM action journal contains a duplicate grant",
                    ));
                }
            }
        }
        journal
            .restore_applied_frontier(stored.applied_grants)
            .map_err(serde::de::Error::custom)?;
        Ok(journal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GmActionInsert {
    Inserted,
    Duplicate,
}

impl GmActionJournal {
    pub fn grants(&self) -> &[GmActionGrant] {
        &self.grants
    }

    /// The authoritative prefix whose application boundary has been reached.
    /// Future commits stay transport/replay input, not current state digest.
    pub fn grants_through(&self, tick: u64) -> &[GmActionGrant] {
        let end = self
            .grants
            .partition_point(|grant| grant.apply_tick <= tick);
        &self.grants[..end]
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn applied_grants(&self) -> usize {
        self.applied_grants
    }

    /// The prefix that has actually reached the product reducer. Future grants
    /// remain captured input but are not current state merely because the wire
    /// delivered them early.
    pub fn applied_prefix(&self) -> &[GmActionGrant] {
        &self.grants[..self.applied_grants]
    }

    /// Restore/validate a captured application frontier without applying any
    /// new grant. The custom deserializer and replay validator both use this.
    pub fn restore_applied_frontier(&mut self, applied: usize) -> Result<(), &'static str> {
        if applied > self.grants.len() {
            return Err("GM applied frontier exceeds the journal");
        }
        self.applied_grants = applied;
        Ok(())
    }

    pub fn initial_paused(&self) -> bool {
        self.initial_paused
    }

    /// Adopt the pre-journal state of a standalone restored session. This is
    /// intentionally legal only before the first typed action exists.
    pub fn adopt_initial_pause(&mut self, paused: bool) {
        if self.grants.is_empty() {
            self.initial_paused = paused;
        }
    }

    /// The canonical grant already associated with one operator's idempotency
    /// key. Correlations are scoped to their authenticated operator so two GMs
    /// may independently mint the same opaque text without aliasing.
    pub fn grant_for(&self, operator_id: &str, correlation: &GmActionId) -> Option<&GmActionGrant> {
        self.grants
            .iter()
            .find(|grant| grant.operator_id == operator_id && grant.correlation == *correlation)
    }

    /// Sequence for this host's next action after every grant it has observed.
    pub fn next_sequence(&self) -> u64 {
        self.grants
            .iter()
            .map(|grant| grant.order.sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Insert on the owner's reliable ordered decision lane. New grants must be
    /// contiguous and monotone; an exact retransmission is inert, while reuse
    /// of the same canonical key for different bytes is refused.
    pub fn insert(
        &mut self,
        grant: GmActionGrant,
    ) -> Result<GmActionInsert, GmActionRefusalReason> {
        grant.validate()?;
        if let Some(existing) = self.grant_for(&grant.operator_id, &grant.correlation) {
            return if existing == &grant {
                Ok(GmActionInsert::Duplicate)
            } else {
                Err(GmActionRefusalReason::ConflictingGrant)
            };
        }
        // Every canonical decision originates at one technical owner and leaves
        // it on one reliable ordered stream. Enforce that contract at the
        // journal too: if an impossible transport/recovery reordering occurs,
        // fail closed instead of letting peers consume different capacity slots.
        if grant.order.sequence != self.next_sequence() {
            return Err(GmActionRefusalReason::NonContiguousSequence);
        }
        if self
            .grants
            .last()
            .is_some_and(|last| grant.key() <= last.key())
        {
            return Err(GmActionRefusalReason::NonContiguousSequence);
        }
        match self
            .grants
            .binary_search_by_key(&grant.key(), GmActionGrant::key)
        {
            Ok(index) if self.grants[index] == grant => Ok(GmActionInsert::Duplicate),
            Ok(_) => Err(GmActionRefusalReason::ConflictingGrant),
            Err(_) if self.grants.len() >= MAX_STORED_GM_ACTIONS_PER_RUN => {
                Err(GmActionRefusalReason::JournalFull)
            }
            Err(_) if self.grants.len() == MAX_GM_ACTIONS_PER_RUN => {
                // The sole overflow slot is a paused-state escape, never one
                // more ordinary mutation. Owner sequencing makes this verdict
                // canonical; deserialisation rebuilds through the same branch.
                let escape =
                    !grant.action.requested_pause() && self.log_through(grant.apply_tick).paused();
                if !escape {
                    return Err(GmActionRefusalReason::JournalFull);
                }
                let index = self
                    .grants
                    .binary_search_by_key(&grant.key(), GmActionGrant::key)
                    .expect_err("new grant was not present in the branch above");
                self.grants.insert(index, grant);
                Ok(GmActionInsert::Inserted)
            }
            Err(index) => {
                self.grants.insert(index, grant);
                Ok(GmActionInsert::Inserted)
            }
        }
    }

    /// Canonical terminal facts through `tick`, independent of insertion order.
    pub fn log_through(&self, tick: u64) -> GmActionLog {
        let end = self
            .grants
            .partition_point(|grant| grant.apply_tick <= tick);
        self.log_prefix(end)
    }

    /// Apply the now-due prefix and durably advance the exact reducer frontier.
    pub fn apply_through(&mut self, tick: u64) -> GmActionLog {
        let end = self
            .grants
            .partition_point(|grant| grant.apply_tick <= tick);
        self.applied_grants = self.applied_grants.max(end);
        self.applied_log()
    }

    pub fn applied_log(&self) -> GmActionLog {
        self.log_prefix(self.applied_grants)
    }

    fn log_prefix(&self, end: usize) -> GmActionLog {
        let mut paused = self.initial_paused;
        let mut entries = Vec::new();
        for grant in self.grants.iter().take(end) {
            let requested_active = grant.action.requested_pause();
            let outcome = if paused == requested_active {
                GmActionOutcome::NoOp
            } else {
                paused = requested_active;
                GmActionOutcome::Applied
            };
            entries.push(LoggedGmAction {
                operator_id: grant.operator_id.clone(),
                correlation: grant.correlation.clone(),
                requested_active,
                outcome,
                tick: grant.apply_tick,
                reason: None,
                order: Some(grant.order),
            });
        }
        GmActionLog { entries, paused }
    }

    pub fn terminal_fact_for(
        &self,
        operator_id: &str,
        correlation: &GmActionId,
        tick: u64,
    ) -> Option<LoggedGmAction> {
        self.log_through(tick)
            .entries
            .into_iter()
            .find(|entry| entry.operator_id == operator_id && entry.correlation == *correlation)
    }

    fn projected_pause(&self) -> bool {
        self.log_through(u64::MAX).paused()
    }

    fn last_apply_tick(&self) -> Option<u64> {
        self.grants.last().map(|grant| grant.apply_tick)
    }
}

/// Derived result log used by replay diagnostics and the local GM projection.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmActionLog {
    entries: Vec<LoggedGmAction>,
    paused: bool,
}

impl GmActionLog {
    pub fn entries(&self) -> &[LoggedGmAction] {
        &self.entries
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn terminal_fact_for(
        &self,
        operator_id: &str,
        correlation: &GmActionId,
    ) -> Option<LoggedGmAction> {
        self.entries
            .iter()
            .find(|entry| entry.operator_id == operator_id && entry.correlation == *correlation)
            .cloned()
    }
}

/// Supplemental terminal results stay outside snapshots/digests. Canonical
/// owner refusals settle every GM UI, while an exact retry can temporarily pin
/// an old successful fact that has fallen outside the presentation window.
#[derive(Resource, Clone, Debug, Default)]
pub struct LocalGmActionRefusals {
    entries: Vec<LoggedGmAction>,
}

impl LocalGmActionRefusals {
    pub fn push(&mut self, entry: LoggedGmAction) {
        const LIMIT: usize = 64;
        self.entries.retain(|existing| {
            existing.operator_id != entry.operator_id || existing.correlation != entry.correlation
        });
        if self.entries.len() == LIMIT {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[LoggedGmAction] {
        &self.entries
    }
}

/// Absolute local Host Channel projection. `results` is presentation-bounded;
/// the durable journal remains complete up to its protocol cap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmSessionProjection {
    pub paused: bool,
    pub results: Vec<LoggedGmAction>,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct LastGmSessionProjection(Option<GmSessionProjection>);

pub fn projection(
    paused: bool,
    log: &GmActionLog,
    refusals: &LocalGmActionRefusals,
) -> GmSessionProjection {
    const RESULT_LIMIT: usize = 128;
    let mut results = log.entries.clone();
    // Supplemental facts are deliberately protected from the ordinary oldest-
    // first presentation bound. This is what makes an exact retry of a cached
    // action re-project that terminal fact even after 128 later results exist.
    results.retain(|entry| {
        !refusals.entries.iter().any(|supplemental| {
            supplemental.operator_id == entry.operator_id
                && supplemental.correlation == entry.correlation
        })
    });
    results.sort_by(|left, right| {
        (
            left.tick,
            left.order,
            left.operator_id.as_str(),
            left.correlation.as_str(),
        )
            .cmp(&(
                right.tick,
                right.order,
                right.operator_id.as_str(),
                right.correlation.as_str(),
            ))
    });
    let canonical_limit = RESULT_LIMIT.saturating_sub(refusals.entries.len());
    if results.len() > canonical_limit {
        results.drain(0..results.len() - canonical_limit);
    }
    results.extend(refusals.entries.iter().cloned());
    GmSessionProjection { paused, results }
}

/// Push an absolute page-local projection whenever pause or the bounded result
/// feed changes. Frame-driven so a paused session can still report Resume.
pub fn publish_session_projection(
    paused: Res<SimulationPaused>,
    log: Res<GmActionLog>,
    refusals: Res<LocalGmActionRefusals>,
    mut last: ResMut<LastGmSessionProjection>,
    mut writer: MessageWriter<crate::console_bridge::GmSessionChanged>,
) {
    let next = projection(paused.0, &log, &refusals);
    if last.0.as_ref() == Some(&next) {
        return;
    }
    last.0 = Some(next.clone());
    writer.write(crate::console_bridge::GmSessionChanged { payload: next });
}

/// Recompute and apply every due action. It runs before the mesh gate, which
/// may add its own hold after a GM resume; resume therefore removes only the GM
/// pause and never overrides recovery/model-readiness holds.
pub fn apply_due_actions(
    session: Option<Res<crate::lockstep::FleetLockstep>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut journal: ResMut<GmActionJournal>,
    mut log: ResMut<GmActionLog>,
    mut paused: ResMut<SimulationPaused>,
    mut virtual_time: Option<ResMut<Time<Virtual>>>,
    mut fixed_time: Option<ResMut<Time<Fixed>>>,
) {
    // Outside a fleet, an empty typed lane must not overwrite the ordinary
    // local host pause surface. Replay and restored saves deliberately carry a
    // non-empty journal and still use this exact production reducer without a
    // synthetic fleet.
    if session.is_none() && journal.is_empty() {
        return;
    }
    let now = tick.as_deref().map_or(0, |tick| tick.0);
    let next = journal.apply_through(now);
    paused.0 = next.paused;
    if let Some(virtual_time) = virtual_time.as_deref_mut() {
        if paused.0 {
            virtual_time.pause();
            virtual_time.advance_by(std::time::Duration::ZERO);
        } else {
            virtual_time.unpause();
        }
    }
    if paused.0 {
        // Time<Virtual>::pause prevents new accumulation, but a rendered frame
        // can already carry whole fixed steps in its accumulator. Drop only
        // those unbegun whole steps so an apply-at-now Pause cannot leak one
        // simulation tick before the next frame observes the stopped clock.
        if let Some(fixed) = fixed_time.as_deref_mut() {
            let remaining = fixed.overstep();
            let timestep = fixed.timestep();
            let remainder_nanos = remaining.as_nanos() % timestep.as_nanos();
            let remainder = std::time::Duration::new(
                u64::try_from(remainder_nanos / 1_000_000_000).unwrap_or(u64::MAX),
                (remainder_nanos % 1_000_000_000) as u32,
            );
            fixed.discard_overstep(remaining - remainder);
        }
    }
    *log = next;
}

/// Reset the replicated GM lane at a new fleet/run boundary.
pub fn reset(world: &mut World) {
    world.insert_resource(GmActionJournal::default());
    world.insert_resource(GmActionLog::default());
    world.insert_resource(LocalGmActionRefusals::default());
    world.insert_resource(LastGmSessionProjection::default());
    world.insert_resource(SimulationPaused(false));
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GmActionSubmission {
    /// A non-owner peer sent an authenticated proposal to the technical owner.
    Pending,
    /// The owner assigned and replicated the canonical grant.
    Granted(GmActionGrant),
    /// An exact retry reused an existing canonical grant.
    Replayed(GmActionGrant),
    /// The owner replicated a canonical terminal refusal.
    Refused(GmActionRefusal),
}

fn refusal_for(
    owner: HostSlot,
    proposal: &GmActionProposal,
    tick: u64,
    reason: GmActionRefusalReason,
) -> GmActionRefusal {
    GmActionRefusal {
        sequenced_by: owner,
        requester: proposal.from,
        operator_id: proposal.operator_id.clone(),
        correlation: proposal.correlation.clone(),
        requested_active: proposal.action.requested_pause(),
        tick,
        reason,
    }
}

/// Assign the one owner-controlled order and boundary for a proposal.
///
/// A boundary remains open only while its canonical final value is Paused. As
/// soon as a commit leaves it running, the next proposal is forced strictly
/// later. Therefore a peer that receives the releasing commit early can never
/// later learn a canonically-later mutation for a tick it was allowed to run.
pub fn sequence_owner_proposal(
    journal: &mut GmActionJournal,
    proposal: &GmActionProposal,
    owner: HostSlot,
    now: u64,
    ready_through: u64,
    current_paused: bool,
) -> Result<GmActionGrant, GmActionRefusalReason> {
    proposal.validate()?;
    if let Some(existing) = journal.grant_for(&proposal.operator_id, &proposal.correlation) {
        return if existing.from == proposal.from && existing.action == proposal.action {
            Ok(existing.clone())
        } else {
            Err(GmActionRefusalReason::ConflictingGrant)
        };
    }

    if journal.is_empty() {
        journal.adopt_initial_pause(current_paused);
    }
    let projected_paused = journal.projected_pause();
    let apply_tick = if projected_paused || (journal.is_empty() && current_paused) {
        journal.last_apply_tick().unwrap_or(now)
    } else {
        ready_through.saturating_add(1).max(now).max(
            journal
                .last_apply_tick()
                .map_or(0, |closed| closed.saturating_add(1)),
        )
    };
    let grant = GmActionGrant {
        from: proposal.from,
        sequenced_by: owner,
        operator_id: proposal.operator_id.clone(),
        correlation: proposal.correlation.clone(),
        apply_tick,
        order: GmActionOrder::new(proposal.from, journal.next_sequence()),
        action: proposal.action.clone(),
    };
    journal.insert(grant.clone())?;
    Ok(grant)
}

/// Privileged local admission. Identity comes from the frozen private slot
/// binding; the request's operator id is only a claim checked against it.
pub fn submit_local(
    world: &mut World,
    request: GmActionRequest,
) -> Result<GmActionSubmission, GmActionRefusalReason> {
    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    // Session pause is run state, not lobby/countdown state. Production apps
    // always carry GamePhase; the absent-state allowance keeps the pure and
    // replay fixtures intentionally phase-agnostic.
    if world
        .get_resource::<State<crate::core::messages::GamePhase>>()
        .is_some_and(|phase| phase.get() != &crate::core::messages::GamePhase::InProgress)
        || world
            .get_resource::<NextState<crate::core::messages::GamePhase>>()
            .is_some_and(|next| {
                matches!(
                    next,
                    NextState::Pending(phase)
                        if phase != &crate::core::messages::GamePhase::InProgress
                )
            })
    {
        return Err(GmActionRefusalReason::WrongPhase);
    }
    let (local, bound) = world
        .get_resource::<crate::lockstep::FleetRoster>()
        .map(|roster| {
            (
                roster.local(),
                roster.gm_operator(roster.local()).map(str::to_string),
            )
        })
        .ok_or(GmActionRefusalReason::NotInFleet)?;
    let bound = bound.ok_or(GmActionRefusalReason::NotGameMaster)?;
    if bound != request.operator_id {
        return Err(GmActionRefusalReason::OperatorMismatch);
    }
    if world
        .get_resource::<crate::gm_roster::GmRoster>()
        .is_none_or(|roster| !roster.is_connected(&bound))
    {
        return Err(GmActionRefusalReason::NotGameMaster);
    }
    if let Some(existing) = world
        .resource::<GmActionJournal>()
        .grant_for(&bound, &request.correlation)
    {
        if existing.action != request.action {
            return Err(GmActionRefusalReason::ConflictingGrant);
        }
        let existing = existing.clone();
        // A retry after the terminal projection was bounded out receives the
        // same cached fact again without another authoritative activity row.
        if let Some(fact) = world
            .resource::<GmActionLog>()
            .terminal_fact_for(&bound, &request.correlation)
        {
            world.resource_mut::<LocalGmActionRefusals>().push(fact);
        }
        world.resource_mut::<LastGmSessionProjection>().0 = None;
        return Ok(GmActionSubmission::Replayed(existing));
    }
    let paused = world
        .get_resource::<SimulationPaused>()
        .is_some_and(|paused| paused.0);
    let owner = world.resource::<crate::lockstep::FleetRoster>().owner();
    let proposal = GmActionProposal {
        from: local,
        operator_id: bound,
        correlation: request.correlation,
        action: request.action,
    };

    if local != owner {
        world.resource_mut::<crate::lockstep::MeshOutbox>().push(
            crate::lockstep::MeshFrame::GmAction(GmActionFrame::Proposal(proposal)),
        );
        return Ok(GmActionSubmission::Pending);
    }

    // A restored standalone roster deliberately has no FleetLockstep. Its local
    // preserved GM binding still sequences safely because there is no peer to
    // wait for; the current logical boundary is its whole frontier.
    let ready_through =
        if let Some(session) = world.get_resource::<crate::lockstep::FleetLockstep>() {
            session.ready_through(now)
        } else {
            now.saturating_sub(1)
        };
    let sequenced = {
        let mut journal = world.resource_mut::<GmActionJournal>();
        sequence_owner_proposal(&mut journal, &proposal, owner, now, ready_through, paused)
    };
    let grant = match sequenced {
        Ok(grant) => grant,
        Err(reason) => {
            let refusal = refusal_for(owner, &proposal, now, reason);
            world
                .resource_mut::<LocalGmActionRefusals>()
                .push(refusal.logged());
            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                crate::lockstep::MeshFrame::GmAction(GmActionFrame::Refused(refusal.clone())),
            );
            return Ok(GmActionSubmission::Refused(refusal));
        }
    };
    world
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .push(crate::lockstep::MeshFrame::GmAction(
            GmActionFrame::Granted(grant.clone()),
        ));
    Ok(GmActionSubmission::Granted(grant))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(
        slot: u32,
        sequence: u64,
        apply_tick: u64,
        correlation: &str,
        active: bool,
    ) -> GmActionGrant {
        let origin = HostSlot(slot);
        GmActionGrant {
            from: origin,
            sequenced_by: HostSlot(1),
            operator_id: format!("gm-{slot}"),
            correlation: GmActionId::new(correlation).unwrap(),
            apply_tick,
            order: GmActionOrder::new(origin, sequence),
            action: GmAction::SetSessionPaused { active },
        }
    }

    #[test]
    fn applied_no_op_and_resume_are_explicit_terminal_facts() {
        let mut journal = GmActionJournal::default();
        journal.insert(grant(1, 1, 10, "pause", true)).unwrap();
        journal.insert(grant(1, 2, 10, "duplicate", true)).unwrap();
        journal.insert(grant(1, 3, 10, "resume", false)).unwrap();

        let log = journal.log_through(10);
        assert!(!log.paused());
        assert_eq!(
            log.entries()
                .iter()
                .map(|entry| entry.outcome)
                .collect::<Vec<_>>(),
            vec![
                GmActionOutcome::Applied,
                GmActionOutcome::NoOp,
                GmActionOutcome::Applied,
            ]
        );
    }

    #[test]
    fn owner_closes_a_released_boundary_before_a_late_concurrent_proposal() {
        let owner = HostSlot(1);
        let mut canonical = GmActionJournal::default();
        canonical.adopt_initial_pause(true);
        let resume = GmActionProposal {
            from: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("resume").unwrap(),
            action: GmAction::SetSessionPaused { active: false },
        };
        let resume = sequence_owner_proposal(&mut canonical, &resume, owner, 20, 20, true)
            .expect("owner sequences resume");
        assert_eq!(resume.apply_tick, 20);

        // This proposal was concurrent in product time but reached the owner
        // after the releasing commit. It cannot mutate boundary 20 retroactively.
        let late_pause = GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("late-pause").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        let late_pause = sequence_owner_proposal(&mut canonical, &late_pause, owner, 20, 20, true)
            .expect("owner sequences late proposal");
        assert_eq!(late_pause.apply_tick, 21);

        let mut early_delivery = GmActionJournal::default();
        early_delivery.adopt_initial_pause(true);
        early_delivery.insert(resume.clone()).unwrap();
        assert!(!early_delivery.log_through(20).paused());
        early_delivery.insert(late_pause.clone()).unwrap();

        let mut batched_delivery = GmActionJournal::default();
        batched_delivery.adopt_initial_pause(true);
        batched_delivery.insert(resume).unwrap();
        batched_delivery.insert(late_pause).unwrap();
        assert_eq!(early_delivery, batched_delivery);
        assert!(!early_delivery.log_through(20).paused());
        assert!(early_delivery.log_through(21).paused());
    }

    #[test]
    fn exact_wire_retransmission_is_inert_but_key_reuse_is_refused() {
        let mut journal = GmActionJournal::default();
        let original = grant(1, 1, 10, "same", true);
        assert_eq!(
            journal.insert(original.clone()),
            Ok(GmActionInsert::Inserted)
        );
        assert_eq!(
            journal.insert(original.clone()),
            Ok(GmActionInsert::Duplicate)
        );
        let mut conflicting = original;
        conflicting.action = GmAction::SetSessionPaused { active: false };
        assert_eq!(
            journal.insert(conflicting),
            Err(GmActionRefusalReason::ConflictingGrant)
        );
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn contiguous_owner_sequence_cannot_move_its_apply_boundary_backwards() {
        let mut journal = GmActionJournal::default();
        journal
            .insert(grant(1, 1, 10, "first-boundary", true))
            .unwrap();
        assert_eq!(
            journal.insert(grant(1, 2, 9, "backwards-boundary", false)),
            Err(GmActionRefusalReason::NonContiguousSequence)
        );
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn correlation_is_an_operator_scoped_idempotency_key() {
        let mut journal = GmActionJournal::default();
        let original = grant(1, 1, 10, "same", true);
        journal.insert(original).unwrap();

        let same_operator_new_order = grant(1, 2, 11, "same", true);
        assert_eq!(
            journal.insert(same_operator_new_order),
            Err(GmActionRefusalReason::ConflictingGrant)
        );

        let same_text_other_operator = grant(2, 2, 11, "same", false);
        assert_eq!(
            journal.insert(same_text_other_operator),
            Ok(GmActionInsert::Inserted)
        );
        assert_eq!(journal.len(), 2);
    }

    #[test]
    fn future_grant_remains_pending_until_its_exact_tick() {
        let mut journal = GmActionJournal::default();
        journal.insert(grant(1, 1, 33, "future", true)).unwrap();
        assert!(journal.log_through(32).entries().is_empty());
        assert!(!journal.log_through(32).paused());
        assert!(journal.log_through(33).paused());
    }

    #[test]
    fn proposal_grant_and_refusal_have_distinct_frozen_authorities() {
        let roster = crate::lockstep::FleetRoster::with_participants_and_gms(
            Vec::new(),
            vec![HostSlot(1), HostSlot(2)],
            vec![
                crate::lockstep::FleetGm {
                    host: HostSlot(1),
                    operator_id: "gm-1".into(),
                },
                crate::lockstep::FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-2".into(),
                },
            ],
            HostSlot(1),
            HostSlot(1),
        )
        .unwrap();
        let proposal = GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("auth-proposal").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Proposal(proposal.clone()), &roster),
            Ok(())
        );
        let mut forged_proposal = proposal.clone();
        forged_proposal.operator_id = "gm-1".into();
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Proposal(forged_proposal), &roster),
            Err(GmActionRefusalReason::OperatorMismatch)
        );

        let granted = GmActionGrant {
            from: proposal.from,
            sequenced_by: HostSlot(1),
            operator_id: proposal.operator_id.clone(),
            correlation: proposal.correlation.clone(),
            apply_tick: 7,
            order: GmActionOrder::new(proposal.from, 1),
            action: proposal.action.clone(),
        };
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Granted(granted.clone()), &roster),
            Ok(())
        );
        let mut forged_grant = granted.clone();
        forged_grant.sequenced_by = HostSlot(2);
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Granted(forged_grant), &roster),
            Err(GmActionRefusalReason::OriginMismatch)
        );

        let refused = GmActionRefusal {
            sequenced_by: HostSlot(1),
            requester: HostSlot(2),
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("auth-refusal").unwrap(),
            requested_active: true,
            tick: 7,
            reason: GmActionRefusalReason::JournalFull,
        };
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(refused.clone()), &roster),
            Ok(())
        );
        let mut forged_refusal = refused;
        forged_refusal.sequenced_by = HostSlot(2);
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(forged_refusal), &roster),
            Err(GmActionRefusalReason::OriginMismatch)
        );
    }

    #[test]
    fn full_lane_has_one_ordered_resume_escape_and_never_accepts_it_early() {
        let mut journal = GmActionJournal::default();
        for sequence in 1..=MAX_GM_ACTIONS_PER_RUN as u64 {
            journal
                .insert(grant(1, sequence, 10, &format!("fill-{sequence}"), true))
                .unwrap();
        }
        assert!(journal.log_through(10).paused());
        let escape = grant(
            1,
            MAX_STORED_GM_ACTIONS_PER_RUN as u64,
            10,
            "escape-resume",
            false,
        );
        journal.insert(escape).expect("bounded resume escape");
        assert_eq!(journal.len(), MAX_STORED_GM_ACTIONS_PER_RUN);
        assert!(!journal.log_through(10).paused());
        assert_eq!(
            journal.insert(grant(
                1,
                MAX_STORED_GM_ACTIONS_PER_RUN as u64 + 1,
                11,
                "past-bound",
                true,
            )),
            Err(GmActionRefusalReason::JournalFull)
        );

        let mut skewed = GmActionJournal::default();
        for sequence in 1..MAX_GM_ACTIONS_PER_RUN as u64 {
            skewed
                .insert(grant(1, sequence, 10, &format!("skew-{sequence}"), true))
                .unwrap();
        }
        let early_escape = grant(
            1,
            MAX_STORED_GM_ACTIONS_PER_RUN as u64,
            10,
            "early-escape",
            false,
        );
        assert_eq!(
            skewed.insert(early_escape),
            Err(GmActionRefusalReason::NonContiguousSequence),
            "an impossible transport reorder fails closed before it can consume capacity"
        );
    }

    #[test]
    fn standalone_restored_pause_has_a_working_typed_resume() {
        let mut world = admitted_world();
        world.remove_resource::<crate::lockstep::FleetLockstep>();
        world.resource_mut::<SimulationPaused>().0 = true;
        let result = submit_local(
            &mut world,
            GmActionRequest {
                operator_id: "gm-1".into(),
                correlation: GmActionId::new("restored-resume").unwrap(),
                action: GmAction::SetSessionPaused { active: false },
            },
        )
        .expect("the preserved local GM binding remains authoritative");
        let GmActionSubmission::Granted(grant) = result else {
            panic!("standalone owner should sequence immediately");
        };
        assert_eq!(grant.apply_tick, 10);
        let log = world.resource::<GmActionJournal>().log_through(10);
        assert!(!log.paused());
        assert_eq!(log.entries()[0].outcome, GmActionOutcome::Applied);
    }

    #[test]
    fn retry_reprojects_an_exact_terminal_fact_after_the_feed_bounds_it_out() {
        let mut world = admitted_world();
        world.insert_resource(crate::sim_tick::SimTick(500));
        let mut journal = GmActionJournal::default();
        for sequence in 1..=140 {
            journal
                .insert(grant(
                    1,
                    sequence,
                    sequence,
                    &format!("result-{sequence}"),
                    sequence % 2 == 1,
                ))
                .unwrap();
        }
        let log = journal.log_through(500);
        let bounded = projection(false, &log, &LocalGmActionRefusals::default());
        assert!(bounded
            .results
            .iter()
            .all(|entry| entry.correlation.as_str() != "result-1"));
        world.insert_resource(journal);
        world.insert_resource(log);

        assert!(matches!(
            submit_local(
                &mut world,
                GmActionRequest {
                    operator_id: "gm-1".into(),
                    correlation: GmActionId::new("result-1").unwrap(),
                    action: GmAction::SetSessionPaused { active: true },
                }
            ),
            Ok(GmActionSubmission::Replayed(_))
        ));
        let retried = projection(
            false,
            world.resource::<GmActionLog>(),
            world.resource::<LocalGmActionRefusals>(),
        );
        let exact = retried
            .results
            .iter()
            .find(|entry| entry.correlation.as_str() == "result-1")
            .expect("retried old fact is pinned into the bounded projection");
        assert_eq!(exact.tick, 1);
        assert_eq!(exact.outcome, GmActionOutcome::Applied);
    }

    fn admitted_world() -> World {
        let mut world = World::new();
        let slot = HostSlot(1);
        world.insert_resource(
            crate::lockstep::FleetRoster::with_participants_and_gms(
                Vec::new(),
                vec![slot],
                vec![crate::lockstep::FleetGm {
                    host: slot,
                    operator_id: "gm-1".into(),
                }],
                slot,
                slot,
            )
            .unwrap(),
        );
        world.insert_resource(crate::lockstep::FleetLockstep(
            crate::lockstep::LockstepSession::new(slot, [slot], 6),
        ));
        world.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
                "gm-1".into(),
                "Morgan".into(),
                true,
            )])
            .unwrap(),
        );
        world.insert_resource(crate::sim_tick::SimTick(10));
        world.insert_resource(SimulationPaused(false));
        world.insert_resource(GmActionJournal::default());
        world.insert_resource(GmActionLog::default());
        world.insert_resource(LocalGmActionRefusals::default());
        world.insert_resource(LastGmSessionProjection::default());
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world
    }

    #[test]
    fn local_admission_binds_identity_and_reuses_the_cached_grant() {
        let mut world = admitted_world();
        let request = GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("pause-once").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        let first = submit_local(&mut world, request.clone()).unwrap();
        let retried = submit_local(&mut world, request).unwrap();
        let GmActionSubmission::Granted(first) = first else {
            panic!("owner must grant its local proposal");
        };
        assert_eq!(retried, GmActionSubmission::Replayed(first));
        assert_eq!(world.resource::<GmActionJournal>().len(), 1);
        assert_eq!(
            world
                .resource::<crate::lockstep::MeshOutbox>()
                .pending_frames()
                .len(),
            1,
            "a local retry reuses the cached fact instead of duplicating the wire action"
        );

        let conflicting = GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("pause-once").unwrap(),
            action: GmAction::SetSessionPaused { active: false },
        };
        assert_eq!(
            submit_local(&mut world, conflicting),
            Err(GmActionRefusalReason::ConflictingGrant)
        );
    }

    #[test]
    fn local_admission_refuses_spoofed_or_disconnected_operators() {
        let mut world = admitted_world();
        let spoofed = GmActionRequest {
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("spoofed").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        assert_eq!(
            submit_local(&mut world, spoofed),
            Err(GmActionRefusalReason::OperatorMismatch)
        );

        world.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
                "gm-1".into(),
                "Morgan".into(),
                false,
            )])
            .unwrap(),
        );
        let disconnected = GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("disconnected").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        assert_eq!(
            submit_local(&mut world, disconnected),
            Err(GmActionRefusalReason::NotGameMaster)
        );
    }

    #[test]
    fn local_session_pause_is_admitted_only_during_an_active_run() {
        let mut world = admitted_world();
        world.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
        let request = GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("lobby-pause").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        assert_eq!(
            submit_local(&mut world, request.clone()),
            Err(GmActionRefusalReason::WrongPhase)
        );
        assert!(world.resource::<GmActionJournal>().is_empty());
        assert!(world
            .resource::<crate::lockstep::MeshOutbox>()
            .pending_frames()
            .is_empty());

        world.insert_resource(State::new(crate::core::messages::GamePhase::InProgress));
        world.insert_resource(NextState::Pending(crate::core::messages::GamePhase::Lobby));
        assert_eq!(
            submit_local(&mut world, request.clone()),
            Err(GmActionRefusalReason::WrongPhase),
            "a same-frame accepted ReturnToLobby closes ingress before State changes"
        );
        world.insert_resource(NextState::<crate::core::messages::GamePhase>::Unchanged);
        assert!(matches!(
            submit_local(&mut world, request),
            Ok(GmActionSubmission::Granted(_))
        ));
    }
}
