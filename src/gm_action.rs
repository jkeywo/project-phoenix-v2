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
    SetSessionPaused {
        active: bool,
    },
    SetStationPuppet {
        ship: crate::command_admission::log::ShipKey,
        station: crate::core::messages::StationId,
        active: bool,
    },
    IssueStationCommand {
        ship: crate::command_admission::log::ShipKey,
        station: crate::core::messages::StationId,
        target: crate::core::messages::SystemId,
        payload: crate::gm_puppet::CanonicalSystemCommandPayload,
    },
    /// Fire one authored, GM-operable event (issue #1301).
    ///
    /// `event` is the layer-qualified stable id
    /// ([`crate::gm_event::qualified_event_id`]), never an index and never a
    /// script path: the trigger table is rebuilt by replaying the same load, so
    /// an index is not an identity across a layer load/unload, while the
    /// qualified id is. Applying it ARMS the event; the ordinary trigger
    /// pipeline then runs the ordinary handler and consumes the ordinary
    /// lifecycle, so a GM Fire and an automatic firing are the same event.
    FireGmEvent {
        event: String,
    },
}

/// Presentation/result family for a typed GM action. The complete action stays
/// in the canonical journal; this small copy lets local result surfaces route a
/// Pause result without mistaking a Station takeover for session state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmActionKind {
    SessionPause,
    StationPuppet,
    StationCommand,
    /// The authored-event control family (issue #1301): Fire today, Pause
    /// (#1303) and Skip (#1304) next, all naming the same qualified event id.
    EventControl,
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
        self.action.validate()
    }
}

impl GmAction {
    pub fn ship_key(&self) -> Option<&crate::command_admission::log::ShipKey> {
        match self {
            Self::SetSessionPaused { .. } | Self::FireGmEvent { .. } => None,
            Self::SetStationPuppet { ship, .. } | Self::IssueStationCommand { ship, .. } => {
                Some(ship)
            }
        }
    }

    /// The stable target identity this action names, for the durable result.
    ///
    /// Only the event-control family has one today: a Station action's target
    /// is already two fields (`ship`, `station`) that the activity feed does
    /// not render, and inventing a joined spelling for them here would be a
    /// second identity for the same thing. `None` therefore means "this family
    /// carries no single stable target", not "unknown".
    pub fn target_id(&self) -> Option<&str> {
        match self {
            Self::FireGmEvent { event } => Some(event.as_str()),
            Self::SetSessionPaused { .. }
            | Self::SetStationPuppet { .. }
            | Self::IssueStationCommand { .. } => None,
        }
    }

    fn validate(&self) -> Result<(), GmActionRefusalReason> {
        let bounded = |value: &str| {
            !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
        };
        match self {
            Self::SetSessionPaused { .. } => Ok(()),
            // The qualified id shape is checked here rather than only against
            // the live table so a malformed one is refused as an invalid
            // ACTION, not mistaken for an unknown event.
            Self::FireGmEvent { event }
                if bounded(event)
                    && event.contains("::")
                    && !event.ends_with("::")
                    && !event.starts_with("::") =>
            {
                Ok(())
            }
            Self::SetStationPuppet { ship, station, .. }
                if bounded(&ship.0) && bounded(&station.0) =>
            {
                Ok(())
            }
            Self::IssueStationCommand {
                ship,
                station,
                target,
                payload,
            } if bounded(&ship.0)
                && bounded(&station.0)
                && bounded(&target.0)
                && crate::core::codec::decode_canonical_system_command(payload.as_str())
                    .is_some() =>
            {
                Ok(())
            }
            _ => Err(GmActionRefusalReason::InvalidAction),
        }
    }

    pub fn kind(&self) -> GmActionKind {
        match self {
            Self::SetSessionPaused { .. } => GmActionKind::SessionPause,
            Self::SetStationPuppet { .. } => GmActionKind::StationPuppet,
            Self::IssueStationCommand { .. } => GmActionKind::StationCommand,
            Self::FireGmEvent { .. } => GmActionKind::EventControl,
        }
    }

    pub fn requested_pause(&self) -> Option<bool> {
        match self {
            Self::SetSessionPaused { active } => Some(*active),
            Self::SetStationPuppet { .. }
            | Self::IssueStationCommand { .. }
            | Self::FireGmEvent { .. } => None,
        }
    }

    pub fn requested_active(&self) -> bool {
        match self {
            Self::SetSessionPaused { active } | Self::SetStationPuppet { active, .. } => *active,
            // A Fire is always a request to make something happen. There is no
            // "un-fire", so the flag is a constant rather than a policy.
            Self::IssueStationCommand { .. } | Self::FireGmEvent { .. } => true,
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
    /// Canonical recovery generation of `from` when the owner sequenced this
    /// grant. A slot recovery advances the generation without changing the
    /// frozen GM binding, so work queued by the departed incarnation can never
    /// become live again merely because `LockstepSession::rejoin` cleared its
    /// transient departed flag.
    #[serde(default)]
    pub recovery_generation: u64,
    /// Logical boundary at which the absolute value becomes authoritative.
    pub apply_tick: u64,
    pub order: GmActionOrder,
    pub action: GmAction,
}

/// One canonical slot-recovery generation boundary retained by the GM journal.
///
/// Slot recovery is mesh state, not browser connection state. Keeping its
/// generation beside the grants makes the stale-incarnation decision survive
/// snapshot transfer and replay. Boundaries are effective after all GM actions
/// already ordered for that same logical tick; a genuinely post-recovery grant
/// carries the new generation and may also apply at the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmSlotRecoveryGeneration {
    pub slot: HostSlot,
    pub boundary_tick: u64,
    pub generation: u64,
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
        self.action.validate()
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
    pub action_kind: GmActionKind,
    pub requested_active: bool,
    pub tick: u64,
    pub reason: GmActionRefusalReason,
    /// The stable target identity the refused action named, when its family has
    /// one ([`GmAction::target_id`]) — the layer-qualified event id for the
    /// event-control family (issue #1301), `None` for every older family.
    ///
    /// A refusal replaces the grant that never existed, so without this the
    /// terminal answer to "fire base-world::breach_alarm" would name no event
    /// at all on any GM's activity feed or mission panel. `default` keeps an
    /// older peer's refusal frame readable, and `skip_serializing_if` keeps a
    /// pause-only refusal byte-identical to its pre-#1301 shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl GmActionRefusal {
    pub fn logged(&self) -> LoggedGmAction {
        LoggedGmAction::refused(
            self.operator_id.clone(),
            self.correlation.clone(),
            self.action_kind,
            self.requested_active,
            self.tick,
            self.reason,
        )
        .with_target(self.target.clone())
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
            // A refusal is the terminal answer to a NAMED action, so its target
            // must match what its family carries: an event-control refusal with
            // no event could only be published as a fire of the empty id, and a
            // pause refusal with one would invent a second identity for a family
            // that has none. Both are malformed frames, not facts to project.
            if (refusal.action_kind == GmActionKind::EventControl) != refusal.target.is_some() {
                return Err(GmActionRefusalReason::InvalidAction);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmActionOutcome {
    /// Canonical admission succeeded, but the authentic System consumer has
    /// not yet supplied its terminal result.
    Pending,
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
    InvalidAction,
    UnknownStation,
    StationNotBackfill,
    StationNotPuppeted,
    SystemOutsideStation,
    SystemUnavailable,
    /// The command passed Station/System admission, but its authentic System
    /// consumer refused the requested state transition.
    SystemRefused,
    OriginMismatch,
    ConflictingGrant,
    NonContiguousSequence,
    JournalFull,
    WrongPhase,
    UnreadableRequest,
    /// No live authored event carries this layer-qualified id, or the one that
    /// does declares no Fire control (issue #1301). Both are the same answer to
    /// a GM: nothing here is operable under that name at this apply tick.
    ///
    /// Appended rather than grouped with its Station-shaped neighbours on
    /// purpose: the durable journal is folded into the deterministic digest
    /// through postcard, which encodes an enum by VARIANT INDEX, so inserting a
    /// reason in the middle would silently move the digest of every past run
    /// that recorded one of the reasons after it.
    UnknownGmEvent,
}

/// One terminal fact in the GM command log and local activity projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedGmAction {
    pub operator_id: String,
    pub correlation: GmActionId,
    pub action_kind: GmActionKind,
    pub requested_active: bool,
    pub outcome: GmActionOutcome,
    pub tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<GmActionRefusalReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<GmActionOrder>,
    /// The stable target identity of the action that produced this fact, when
    /// its family has one ([`GmAction::target_id`]) — the layer-qualified event
    /// id for the event-control family (issue #1301), `None` for every family
    /// that existed before it.
    ///
    /// It rides the durable result rather than being looked up from the grant
    /// because the activity feed and the mission panel both read the RESULT
    /// surface, which is bounded and outlives the journal window a supplemental
    /// local refusal can fall outside of. `skip_serializing_if` keeps a
    /// pause-only journal byte-identical to its pre-#1301 shape, so no existing
    /// world's digest moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl LoggedGmAction {
    pub fn refused(
        operator_id: String,
        correlation: GmActionId,
        action_kind: GmActionKind,
        requested_active: bool,
        tick: u64,
        reason: GmActionRefusalReason,
    ) -> Self {
        Self {
            operator_id,
            correlation,
            action_kind,
            requested_active,
            outcome: GmActionOutcome::Refused,
            tick,
            reason: Some(reason),
            order: None,
            target: None,
        }
    }

    /// Attach the action's stable target identity to a refusal built above.
    pub fn with_target(mut self, target: Option<String>) -> Self {
        self.target = target;
        self
    }

    /// The durable fact for a request [`submit_local`] refused at ingress,
    /// before any grant existed.
    ///
    /// It is derived from the REQUEST rather than assembled field by field at
    /// the call site so an ingress refusal cannot quietly lose the identity the
    /// operator named: `action_kind`, `requested_active` and `target` all come
    /// from the one action, and a new action family gets all three for free.
    pub fn refused_request(
        request: &GmActionRequest,
        tick: u64,
        reason: GmActionRefusalReason,
    ) -> Self {
        Self::refused(
            request.operator_id.clone(),
            request.correlation.clone(),
            request.action.kind(),
            request.action.requested_active(),
            tick,
            reason,
        )
        .with_target(request.action.target_id().map(str::to_string))
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
    /// Actual terminal outcomes committed at the canonical apply boundary,
    /// aligned one-for-one with `grants[..applied_grants]`. Sequencing-time
    /// validation cannot substitute for this: live Backfill membership and
    /// System availability may change before a delayed grant becomes due.
    applied_results: Vec<LoggedGmAction>,
    /// Canonical recovery incarnations, sorted by `(boundary_tick, slot)`.
    /// This is intentionally retained even after recovery completes: clearing
    /// it would make pre-loss queued grants valid again after a later rejoin.
    #[serde(default)]
    recovery_generations: Vec<GmSlotRecoveryGeneration>,
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
            #[serde(default)]
            applied_results: Vec<LoggedGmAction>,
            #[serde(default)]
            recovery_generations: Vec<GmSlotRecoveryGeneration>,
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
            applied_results: Vec::new(),
            recovery_generations: Vec::new(),
            grants: Vec::new(),
        };
        let stored_recovery_generations = stored.recovery_generations;
        for event in &stored_recovery_generations {
            let generation = journal
                .record_slot_recovery(event.slot, event.boundary_tick)
                .map_err(serde::de::Error::custom)?;
            if generation != event.generation {
                return Err(serde::de::Error::custom(
                    "GM slot recovery generation is not contiguous",
                ));
            }
        }
        if journal.recovery_generations != stored_recovery_generations {
            return Err(serde::de::Error::custom(
                "GM slot recovery generations are not in canonical order",
            ));
        }
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
        if stored.applied_results.is_empty() {
            // Compatibility for development fixtures written before outcomes
            // were persisted. Current snapshot format always stores them.
            journal
                .restore_applied_frontier(stored.applied_grants)
                .map_err(serde::de::Error::custom)?;
        } else {
            if stored.applied_results.len() != stored.applied_grants {
                return Err(serde::de::Error::custom(
                    "GM applied result count does not match its frontier",
                ));
            }
            for result in stored.applied_results {
                journal
                    .record_applied_result(result)
                    .map_err(serde::de::Error::custom)?;
            }
        }
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

    pub fn applied_results(&self) -> &[LoggedGmAction] {
        &self.applied_results
    }

    /// Every retained canonical slot-recovery generation boundary.
    pub fn recovery_generations(&self) -> &[GmSlotRecoveryGeneration] {
        &self.recovery_generations
    }

    /// Record one canonical slot recovery and return its durable generation.
    /// Exact repeats are inert; a slot cannot recover at an earlier or equal
    /// distinct boundary after a later recovery has already been retained.
    pub fn record_slot_recovery(
        &mut self,
        slot: HostSlot,
        boundary_tick: u64,
    ) -> Result<u64, &'static str> {
        if let Some(existing) = self
            .recovery_generations
            .iter()
            .find(|event| event.slot == slot && event.boundary_tick == boundary_tick)
        {
            return Ok(existing.generation);
        }
        let latest = self
            .recovery_generations
            .iter()
            .filter(|event| event.slot == slot)
            .max_by_key(|event| event.generation);
        if latest.is_some_and(|event| event.boundary_tick >= boundary_tick) {
            return Err("GM slot recovery boundary moved backwards");
        }
        let generation = latest
            .map_or(0, |event| event.generation)
            .checked_add(1)
            .ok_or("GM slot recovery generation overflowed")?;
        self.recovery_generations.push(GmSlotRecoveryGeneration {
            slot,
            boundary_tick,
            generation,
        });
        self.recovery_generations
            .sort_by_key(|event| (event.boundary_tick, event.slot, event.generation));
        Ok(generation)
    }

    /// The latest generation the owner must stamp on genuinely new work.
    pub fn current_recovery_generation(&self, slot: HostSlot) -> u64 {
        self.recovery_generations
            .iter()
            .filter(|event| event.slot == slot)
            .map(|event| event.generation)
            .max()
            .unwrap_or(0)
    }

    /// Earliest application boundary for work stamped with `generation`.
    fn recovery_generation_boundary(&self, slot: HostSlot, generation: u64) -> Option<u64> {
        self.recovery_generations
            .iter()
            .find(|event| event.slot == slot && event.generation == generation)
            .map(|event| event.boundary_tick)
    }

    /// Whether a grant's slot incarnation is valid at this apply boundary.
    /// Both adjacent generations are valid exactly ON a recovery boundary:
    /// old work already ordered there precedes recovery, while work stamped
    /// after the canonical recovery event may follow it. On the next tick only
    /// the recovered incarnation remains valid.
    pub fn grant_generation_is_valid(&self, grant: &GmActionGrant, tick: u64) -> bool {
        let generation_before = self
            .recovery_generations
            .iter()
            .filter(|event| event.slot == grant.from && event.boundary_tick < tick)
            .map(|event| event.generation)
            .max()
            .unwrap_or(0);
        let generation_through = self
            .recovery_generations
            .iter()
            .filter(|event| event.slot == grant.from && event.boundary_tick <= tick)
            .map(|event| event.generation)
            .max()
            .unwrap_or(0);
        (generation_before..=generation_through).contains(&grant.recovery_generation)
    }

    /// Recovery events that have reached canonical simulation state at `tick`.
    pub fn recovery_generations_through(&self, tick: u64) -> &[GmSlotRecoveryGeneration] {
        let end = self
            .recovery_generations
            .partition_point(|event| event.boundary_tick <= tick);
        &self.recovery_generations[..end]
    }

    /// Restore/validate a captured application frontier without applying any
    /// new grant. The custom deserializer and replay validator both use this.
    pub fn restore_applied_frontier(&mut self, applied: usize) -> Result<(), &'static str> {
        if applied > self.grants.len() {
            return Err("GM applied frontier exceeds the journal");
        }
        let derived = self.derived_log_prefix(applied);
        self.applied_grants = applied;
        self.applied_results = derived.entries;
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
                let escape = grant.action.requested_pause() == Some(false)
                    && self.log_through(grant.apply_tick).paused();
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
        if end > self.applied_grants {
            // Compatibility helper for pure journal fixtures. Production uses
            // `record_applied_result` so its live refusals are retained.
            self.restore_applied_frontier(end)
                .expect("partition point is within the journal");
        }
        self.applied_log()
    }

    /// Commit one live outcome for the next canonical grant. The metadata is
    /// checked against that grant so snapshot/replay can never detach a result
    /// from the order and idempotency key whose application produced it.
    pub(crate) fn record_applied_result(
        &mut self,
        result: LoggedGmAction,
    ) -> Result<(), &'static str> {
        let Some(grant) = self.grants.get(self.applied_grants) else {
            return Err("GM applied result has no matching grant");
        };
        if result.operator_id != grant.operator_id
            || result.correlation != grant.correlation
            || result.action_kind != grant.action.kind()
            || result.requested_active != grant.action.requested_active()
            || result.tick != grant.apply_tick
            || result.order != Some(grant.order)
            || result.target.as_deref() != grant.action.target_id()
        {
            return Err("GM applied result does not match its canonical grant");
        }
        match result.outcome {
            GmActionOutcome::Pending
                if !matches!(grant.action, GmAction::IssueStationCommand { .. }) =>
            {
                return Err("pending GM result is not a Station command");
            }
            GmActionOutcome::Refused if result.reason.is_none() => {
                return Err("refused GM result has no reason");
            }
            GmActionOutcome::Pending | GmActionOutcome::Applied | GmActionOutcome::NoOp
                if result.reason.is_some() =>
            {
                return Err("successful GM result has a refusal reason");
            }
            _ => {}
        }
        self.applied_results.push(result);
        self.applied_grants += 1;
        Ok(())
    }

    /// Replace the provisional acceptance result for one Station command with
    /// the authentic System consumer's terminal answer.
    ///
    /// The applied frontier must advance when the canonical action boundary is
    /// crossed, before the command enters `AdmittedCommands`; therefore its
    /// initial result is `Pending`.  Correlated consumers run later in the same
    /// fixed tick and call this seam with the actual outcome.  Production
    /// projection, snapshot and digest systems all run after that consumer
    /// feedback has been folded, and terminal projections omit the provisional
    /// value even when a render frame advances no fixed tick.
    pub(crate) fn settle_station_command_result(
        &mut self,
        order: GmActionOrder,
        outcome: crate::core::messages::ActionFeedbackOutcome,
    ) -> Result<bool, &'static str> {
        let (terminal_outcome, terminal_reason) = match outcome {
            crate::core::messages::ActionFeedbackOutcome::Applied => {
                (GmActionOutcome::Applied, None)
            }
            crate::core::messages::ActionFeedbackOutcome::Refused => (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::SystemRefused),
            ),
        };
        self.settle_pending_station_command(order, terminal_outcome, terminal_reason, false)
    }

    /// Refuse an accepted command that cannot reach its already-validated ship
    /// or cannot reserve its bounded consumer-reply route.
    pub(crate) fn refuse_pending_station_command(
        &mut self,
        order: GmActionOrder,
        reason: GmActionRefusalReason,
    ) -> Result<bool, &'static str> {
        self.settle_pending_station_command(order, GmActionOutcome::Refused, Some(reason), true)
    }

    fn settle_pending_station_command(
        &mut self,
        order: GmActionOrder,
        terminal_outcome: GmActionOutcome,
        terminal_reason: Option<GmActionRefusalReason>,
        allow_immediate_applied: bool,
    ) -> Result<bool, &'static str> {
        let Some(index) = self
            .grants
            .iter()
            .take(self.applied_grants)
            .position(|grant| grant.order == order)
        else {
            return Err("GM Station feedback has no applied canonical grant");
        };
        if !matches!(
            self.grants[index].action,
            GmAction::IssueStationCommand { .. }
        ) {
            return Err("GM Station feedback matched a non-command grant");
        }
        let Some(result) = self.applied_results.get_mut(index) else {
            return Err("GM Station feedback has no durable result");
        };
        if terminal_outcome == GmActionOutcome::Pending
            || terminal_outcome == GmActionOutcome::NoOp
            || (terminal_outcome == GmActionOutcome::Refused) != terminal_reason.is_some()
        {
            return Err("GM Station terminal result has an invalid outcome/reason pair");
        }
        if result.outcome == terminal_outcome && result.reason == terminal_reason {
            return Ok(false);
        }
        if (result.outcome != GmActionOutcome::Pending
            && !(allow_immediate_applied && result.outcome == GmActionOutcome::Applied))
            || result.reason.is_some()
        {
            return Err("GM Station result was already settled differently");
        }
        result.outcome = terminal_outcome;
        result.reason = terminal_reason;
        Ok(true)
    }

    /// Grants newly due at `tick`, excluding the prefix already applied. The
    /// caller clones this narrow slice before advancing the durable frontier so
    /// non-pause reducers can apply each canonical mutation exactly once.
    pub fn pending_through(&self, tick: u64) -> &[GmActionGrant] {
        let end = self
            .grants
            .partition_point(|grant| grant.apply_tick <= tick);
        &self.grants[self.applied_grants..end]
    }

    pub fn applied_log(&self) -> GmActionLog {
        self.log_prefix(self.applied_grants)
    }

    fn log_prefix(&self, end: usize) -> GmActionLog {
        if self.applied_results.is_empty() {
            return self.derived_log_prefix(end);
        }
        let mut paused = self.initial_paused;
        let mut puppets = std::collections::BTreeSet::new();
        // Which one-shot events this canonical prefix has already spent, so a
        // second Fire reduces to the same No-op on every peer even when the
        // live trigger table is not available to this reducer.
        let mut fired_events = std::collections::BTreeSet::new();
        let mut entries = Vec::new();
        for (index, grant) in self.grants.iter().take(end).enumerate() {
            let requested_active = grant.action.requested_active();
            if let Some(result) = self.applied_results.get(index) {
                if result.outcome == GmActionOutcome::Applied {
                    match &grant.action {
                        GmAction::SetSessionPaused { active } => paused = *active,
                        GmAction::SetStationPuppet {
                            ship,
                            station,
                            active,
                        } => {
                            let key =
                                (ship.0.clone(), station.0.clone(), grant.operator_id.clone());
                            if *active {
                                puppets.insert(key);
                            } else {
                                puppets.remove(&key);
                            }
                        }
                        GmAction::FireGmEvent { event } => {
                            fired_events.insert(event.clone());
                        }
                        GmAction::IssueStationCommand { .. } => {}
                    }
                }
                entries.push(result.clone());
                continue;
            }
            let outcome = match &grant.action {
                GmAction::FireGmEvent { event } if fired_events.contains(event) => {
                    GmActionOutcome::NoOp
                }
                GmAction::FireGmEvent { event } => {
                    fired_events.insert(event.clone());
                    GmActionOutcome::Applied
                }
                GmAction::SetSessionPaused { active } if paused == *active => GmActionOutcome::NoOp,
                GmAction::SetSessionPaused { active } => {
                    paused = *active;
                    GmActionOutcome::Applied
                }
                GmAction::SetStationPuppet {
                    ship,
                    station,
                    active,
                } => {
                    let key = (ship.0.clone(), station.0.clone(), grant.operator_id.clone());
                    let changed = if *active {
                        puppets.insert(key)
                    } else {
                        puppets.remove(&key)
                    };
                    if changed {
                        GmActionOutcome::Applied
                    } else {
                        GmActionOutcome::NoOp
                    }
                }
                GmAction::IssueStationCommand { .. } => GmActionOutcome::Applied,
            };
            entries.push(LoggedGmAction {
                operator_id: grant.operator_id.clone(),
                correlation: grant.correlation.clone(),
                action_kind: grant.action.kind(),
                requested_active,
                outcome,
                tick: grant.apply_tick,
                reason: None,
                order: Some(grant.order),
                target: grant.action.target_id().map(str::to_string),
            });
        }
        GmActionLog { entries, paused }
    }

    /// Legacy/pure-fixture reducer used only when an applied frontier is
    /// reconstructed without live world state. Current production snapshots
    /// persist the actual results and therefore never guess here.
    fn derived_log_prefix(&self, end: usize) -> GmActionLog {
        let mut paused = self.initial_paused;
        let mut puppets = std::collections::BTreeSet::new();
        let mut fired_events = std::collections::BTreeSet::new();
        let mut entries = Vec::new();
        for grant in self.grants.iter().take(end) {
            let requested_active = grant.action.requested_active();
            let outcome = match &grant.action {
                GmAction::FireGmEvent { event } if fired_events.contains(event) => {
                    GmActionOutcome::NoOp
                }
                GmAction::FireGmEvent { event } => {
                    fired_events.insert(event.clone());
                    GmActionOutcome::Applied
                }
                GmAction::SetSessionPaused { active } if paused == *active => GmActionOutcome::NoOp,
                GmAction::SetSessionPaused { active } => {
                    paused = *active;
                    GmActionOutcome::Applied
                }
                GmAction::SetStationPuppet {
                    ship,
                    station,
                    active,
                } => {
                    let key = (ship.0.clone(), station.0.clone(), grant.operator_id.clone());
                    let changed = if *active {
                        puppets.insert(key)
                    } else {
                        puppets.remove(&key)
                    };
                    if changed {
                        GmActionOutcome::Applied
                    } else {
                        GmActionOutcome::NoOp
                    }
                }
                GmAction::IssueStationCommand { .. } => GmActionOutcome::Applied,
            };
            entries.push(LoggedGmAction {
                operator_id: grant.operator_id.clone(),
                correlation: grant.correlation.clone(),
                action_kind: grant.action.kind(),
                requested_active,
                outcome,
                tick: grant.apply_tick,
                reason: None,
                order: Some(grant.order),
                target: grant.action.target_id().map(str::to_string),
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
        self.log_through(tick).entries.into_iter().find(|entry| {
            entry.outcome != GmActionOutcome::Pending
                && entry.operator_id == operator_id
                && entry.correlation == *correlation
        })
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
            .find(|entry| {
                entry.outcome != GmActionOutcome::Pending
                    && entry.operator_id == operator_id
                    && entry.correlation == *correlation
            })
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

/// Presentation-bounded terminal facts for one typed GM action family.
///
/// The Session and authentic Station surfaces share this exact selection and
/// ordering seam. Supplemental local refusals (including a replayed terminal
/// fact that has fallen outside the canonical window) retain the same
/// precedence as the Pause UI, while the durable journal remains complete.
pub fn projected_results(
    action_kind: GmActionKind,
    log: &GmActionLog,
    refusals: &LocalGmActionRefusals,
) -> Vec<LoggedGmAction> {
    const RESULT_LIMIT: usize = 128;
    let supplemental: Vec<_> = refusals
        .entries
        .iter()
        .filter(|entry| {
            entry.action_kind == action_kind && entry.outcome != GmActionOutcome::Pending
        })
        .cloned()
        .collect();
    let mut results: Vec<_> = log
        .entries
        .iter()
        .filter(|entry| {
            entry.action_kind == action_kind && entry.outcome != GmActionOutcome::Pending
        })
        .cloned()
        .collect();
    // Supplemental facts are deliberately protected from the ordinary oldest-
    // first presentation bound. This is what makes an exact retry of a cached
    // action re-project that terminal fact even after 128 later results exist.
    results.retain(|entry| {
        !supplemental.iter().any(|supplemental| {
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
    let canonical_limit = RESULT_LIMIT.saturating_sub(supplemental.len());
    if results.len() > canonical_limit {
        results.drain(0..results.len() - canonical_limit);
    }
    results.extend(supplemental);
    results
}

pub fn projection(
    paused: bool,
    log: &GmActionLog,
    refusals: &LocalGmActionRefusals,
) -> GmSessionProjection {
    GmSessionProjection {
        paused,
        results: projected_results(GmActionKind::SessionPause, log, refusals),
    }
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

/// Whether a Station grant has crossed its operator's canonical agreed loss
/// boundary.
///
/// `FleetLockstep::has_departed` is replicated mesh state, not a browser-local
/// connection observation. [`crate::lockstep::PendingHostLoss`] supplies the cross-schedule
/// boundary: an unapplied grant stamped AT that tick still precedes the
/// FixedUpdate loss transition and may run, while a later grant may not add the
/// departed operator again. Once the loss has applied every leftover grant is
/// stale. Slot recovery's canonical `rejoin` clears `has_departed`, making new
/// grants valid again without erasing the historical loss log.
fn station_grant_outlives_operator(
    grant: &GmActionGrant,
    journal: &GmActionJournal,
    now: u64,
    session: Option<&crate::lockstep::FleetLockstep>,
    losses: Option<&crate::lockstep::PendingHostLoss>,
) -> bool {
    // This is the durable half of the check. `rejoin` intentionally clears the
    // barrier's departed bit; it must not thereby bless work queued by the old
    // incarnation of the same frozen slot/operator binding.
    if !journal.grant_generation_is_valid(grant, now) {
        return true;
    }
    let Some(session) = session else {
        return false;
    };
    if !session.has_departed(grant.from) {
        return false;
    }
    let Some(losses) = losses else {
        // A fleet session that canonically says the slot departed but carries no
        // boundary record cannot safely let that identity mutate a Station.
        return true;
    };
    if losses.is_applied(grant.from) {
        return true;
    }
    losses
        .agreed_tick(grant.from)
        .is_none_or(|loss_tick| grant.apply_tick > loss_tick)
}

/// Recompute and apply every due action. It runs before the mesh gate, which
/// may add its own hold after a GM resume; resume therefore removes only the GM
/// pause and never overrides recovery/model-readiness holds.
pub fn apply_due_actions(
    session: Option<Res<crate::lockstep::FleetLockstep>>,
    losses: Option<Res<crate::lockstep::PendingHostLoss>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut journal: ResMut<GmActionJournal>,
    mut log: ResMut<GmActionLog>,
    mut paused: ResMut<SimulationPaused>,
    mut puppets: Option<ResMut<crate::gm_puppet::StationPuppets>>,
    mut station_commands: Option<ResMut<crate::gm_puppet::PendingGmStationCommands>>,
    // The live authored-trigger table (issue #1301). `Option` for the same
    // reason every other product resource here is: the pure journal fixtures
    // and the replay harness run this exact production reducer without a world.
    mut content: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    ships: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &crate::ship::components::ShipConfigComponent,
            &crate::ship::components::ActiveStationRatings,
            &crate::ship::components::ShipSystemControlSources,
            Option<&crate::ship_plugin::HumanSeekingHosts>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut virtual_time: Option<ResMut<Time<Virtual>>>,
    mut fixed_time: Option<ResMut<Time<Fixed>>>,
    mut join_hold: Option<ResMut<crate::gm_join::GmJoinPauseHold>>,
) {
    // Outside a fleet, an empty typed lane must not overwrite the ordinary
    // local host pause surface. Replay and restored saves deliberately carry a
    // non-empty journal and still use this exact production reducer without a
    // synthetic fleet.
    if session.is_none() && journal.is_empty() {
        return;
    }
    let now = tick.as_deref().map_or(0, |tick| tick.0);
    let due = journal.pending_through(now).to_vec();
    for grant in due {
        let requested_active = grant.action.requested_active();
        let station_grant_after_loss = station_grant_outlives_operator(
            &grant,
            &journal,
            now,
            session.as_deref(),
            losses.as_deref(),
        );
        let (outcome, reason) = match &grant.action {
            // Fire ARMS the event; it never calls a handler here. The Rhai
            // runtime lives behind `tick_trigger_pipeline`'s FixedUpdate
            // parameter set, which this PreUpdate system deliberately does not
            // hold — and building a second route into a scripted handler is the
            // one shape that would make a GM fire differ from an automatic one.
            // Everything that decides the RESULT is revalidated here, at the
            // agreed apply tick, so every peer commits the same answer.
            GmAction::FireGmEvent { event } => {
                let states_and_pending = content
                    .as_deref_mut()
                    .map(|content| (&content.trigger_states, &mut content.pending_gm_event_fires));
                match states_and_pending {
                    // No world at all: nothing is operable, which is the same
                    // answer a GM gets for an id that names no live event.
                    None => (
                        GmActionOutcome::Refused,
                        Some(GmActionRefusalReason::UnknownGmEvent),
                    ),
                    Some((states, pending)) => {
                        match crate::gm_event::fireable_index(states, event) {
                            None => (
                                GmActionOutcome::Refused,
                                Some(GmActionRefusalReason::UnknownGmEvent),
                            ),
                            // A spent once-only event, or one whose Fire is
                            // already armed and not yet run, is a deterministic
                            // No-op: the handler runs exactly once per authored
                            // lifecycle no matter how many GMs press the button.
                            Some(index)
                                if !crate::world::content::manual_fire_is_still_live(
                                    &states[index],
                                ) || pending.contains(event) =>
                            {
                                (GmActionOutcome::NoOp, None)
                            }
                            Some(_) => {
                                pending.insert(event.clone());
                                (GmActionOutcome::Applied, None)
                            }
                        }
                    }
                }
            }
            GmAction::SetSessionPaused { active } if paused.0 == *active => {
                (GmActionOutcome::NoOp, None)
            }
            GmAction::SetSessionPaused { active } => {
                paused.0 = *active;
                (GmActionOutcome::Applied, None)
            }
            GmAction::SetStationPuppet { active: true, .. } if station_grant_after_loss => (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::NotGameMaster),
            ),
            GmAction::SetStationPuppet {
                ship,
                station,
                active,
            } => {
                let target =
                    crate::gm_puppet::StationPuppetTarget::new(ship.clone(), station.clone());
                let Some(puppets) = puppets.as_deref_mut() else {
                    let reason = if *active {
                        GmActionRefusalReason::UnknownStation
                    } else {
                        GmActionRefusalReason::StationNotPuppeted
                    };
                    let result = LoggedGmAction {
                        operator_id: grant.operator_id.clone(),
                        correlation: grant.correlation.clone(),
                        action_kind: grant.action.kind(),
                        requested_active,
                        outcome: GmActionOutcome::Refused,
                        tick: grant.apply_tick,
                        reason: Some(reason),
                        order: Some(grant.order),
                        target: grant.action.target_id().map(str::to_string),
                    };
                    journal
                        .record_applied_result(result)
                        .expect("live GM result matches its canonical grant");
                    continue;
                };

                if puppets.is_operated_by(&target, &grant.operator_id) == *active {
                    (GmActionOutcome::NoOp, None)
                } else if !*active {
                    puppets.set_operator(target, grant.operator_id.clone(), false);
                    (GmActionOutcome::Applied, None)
                } else {
                    let found = ships
                        .iter()
                        .find(|(uuid, ..)| uuid.0 == ship.0)
                        .map(|(_, config, ratings, ..)| (config, ratings));
                    match crate::gm_puppet::validate_station_action(
                        &grant.action,
                        &grant.operator_id,
                        puppets,
                        found.map(|(config, _)| &config.0),
                        found.map(|(_, ratings)| ratings),
                    ) {
                        Ok(()) => {
                            puppets.set_operator(target, grant.operator_id.clone(), true);
                            (GmActionOutcome::Applied, None)
                        }
                        Err(reason) => (GmActionOutcome::Refused, Some(reason)),
                    }
                }
            }
            GmAction::IssueStationCommand { .. } if station_grant_after_loss => (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::NotGameMaster),
            ),
            GmAction::IssueStationCommand {
                ship,
                station,
                target,
                payload,
            } => {
                let puppet_target =
                    crate::gm_puppet::StationPuppetTarget::new(ship.clone(), station.clone());
                if !puppets.as_deref().is_some_and(|puppets| {
                    puppets.is_operated_by(&puppet_target, &grant.operator_id)
                }) {
                    (
                        GmActionOutcome::Refused,
                        Some(GmActionRefusalReason::StationNotPuppeted),
                    )
                } else if station_commands.is_none() {
                    (
                        GmActionOutcome::Refused,
                        Some(GmActionRefusalReason::SystemUnavailable),
                    )
                } else {
                    let Some((_, config, _, sources, hosts)) =
                        ships.iter().find(|(uuid, ..)| uuid.0 == ship.0)
                    else {
                        let result = LoggedGmAction {
                            operator_id: grant.operator_id.clone(),
                            correlation: grant.correlation.clone(),
                            action_kind: grant.action.kind(),
                            requested_active,
                            outcome: GmActionOutcome::Refused,
                            tick: grant.apply_tick,
                            reason: Some(GmActionRefusalReason::UnknownStation),
                            order: Some(grant.order),
                            target: grant.action.target_id().map(str::to_string),
                        };
                        journal
                            .record_applied_result(result)
                            .expect("live GM result matches its canonical grant");
                        continue;
                    };
                    if config.0.station(station).is_none() {
                        (
                            GmActionOutcome::Refused,
                            Some(GmActionRefusalReason::UnknownStation),
                        )
                    } else {
                        match crate::core::codec::decode_canonical_system_command(payload.as_str())
                        {
                            None => (
                                GmActionOutcome::Refused,
                                Some(GmActionRefusalReason::InvalidAction),
                            ),
                            Some(payload) => match crate::command_admission::validate_station_command(
                                station,
                                target.clone(),
                                payload,
                                sources,
                                &config.0,
                                hosts,
                            ) {
                                Ok(command) => {
                                    let target_kind = config
                                        .0
                                        .systems
                                        .iter()
                                        .find(|system| system.id == command.target)
                                        .map(|system| system.kind.as_str());
                                    let awaits_terminal_consumer = crate::command_admission::
                                        supports_correlated_action_feedback_for_kind(
                                            &command.target,
                                            &command.payload,
                                            target_kind,
                                        );
                                    station_commands.as_deref_mut().expect("checked above").push(
                                        crate::gm_puppet::PendingGmStationCommand {
                                            tick: grant.apply_tick,
                                            order: grant.order,
                                            operator_id: grant.operator_id.clone(),
                                            correlation: grant.correlation.clone(),
                                            ship: ship.clone(),
                                            station: station.clone(),
                                            target: command.target,
                                            payload: command.payload,
                                        },
                                    );
                                    (
                                        if awaits_terminal_consumer {
                                            GmActionOutcome::Pending
                                        } else {
                                            GmActionOutcome::Applied
                                        },
                                        None,
                                    )
                                }
                                Err(
                                    crate::command_admission::StationCommandPolicyFailure::SystemOutsideStation,
                                ) => (
                                    GmActionOutcome::Refused,
                                    Some(GmActionRefusalReason::SystemOutsideStation),
                                ),
                                Err(
                                    crate::command_admission::StationCommandPolicyFailure::SystemUnavailable,
                                ) => (
                                    GmActionOutcome::Refused,
                                    Some(GmActionRefusalReason::SystemUnavailable),
                                ),
                            },
                        }
                    }
                }
            }
        };
        journal
            .record_applied_result(LoggedGmAction {
                operator_id: grant.operator_id,
                correlation: grant.correlation,
                action_kind: grant.action.kind(),
                requested_active,
                outcome,
                tick: grant.apply_tick,
                reason,
                order: Some(grant.order),
                target: grant.action.target_id().map(str::to_string),
            })
            .expect("live GM result matches its canonical grant");
    }
    *log = journal.applied_log();
    // A first-time join's technical hold is independent of the product Pause
    // reducer. It remains until an explicit canonical Resume reaches this
    // journal, without masking the station mutations applied above.
    let join_paused = join_hold
        .as_deref_mut()
        .is_some_and(|hold| hold.retain_until_explicit_resume(&journal));
    paused.0 |= join_paused;
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
}

/// Reset the replicated GM lane at a new fleet/run boundary.
pub fn reset(world: &mut World) {
    world.insert_resource(GmActionJournal::default());
    world.insert_resource(GmActionLog::default());
    world.insert_resource(LocalGmActionRefusals::default());
    world.insert_resource(LastGmSessionProjection::default());
    world.insert_resource(SimulationPaused(false));
    world.insert_resource(crate::gm_puppet::StationPuppets::default());
    world.insert_resource(crate::gm_puppet::PendingGmStationCommands::default());
    world.insert_resource(crate::gm_puppet::PendingGmStationFeedbackRoutes::default());
    world.insert_resource(crate::gm_puppet::StationPuppetActivity::default());
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

/// The one owner-lane refusal constructor. Every field — the identity, the
/// kind, the requested value and the action's stable target — is read from the
/// proposal so no refusing branch can assemble a partial answer.
pub(crate) fn refusal_for(
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
        action_kind: proposal.action.kind(),
        requested_active: proposal.action.requested_active(),
        tick,
        reason,
        target: proposal.action.target_id().map(str::to_string),
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
    technical_join_hold: bool,
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
    let recovery_generation = journal.current_recovery_generation(proposal.from);
    let recovery_boundary = journal
        .recovery_generation_boundary(proposal.from, recovery_generation)
        .unwrap_or(0);
    let projected_paused = journal.projected_pause();
    let apply_tick = if projected_paused || (current_paused && technical_join_hold) {
        // A technical join hold can make the live session paused even when the
        // durable GM prefix last projected Running. Resume must land at this
        // stopped logical boundary; scheduling it at `ready + 1` would require
        // the very tick the hold forbids and deadlock forever.
        journal.last_apply_tick().unwrap_or(now).max(now)
    } else {
        ready_through.saturating_add(1).max(now).max(
            journal
                .last_apply_tick()
                .map_or(0, |closed| closed.saturating_add(1)),
        )
    }
    .max(recovery_boundary);
    let grant = GmActionGrant {
        from: proposal.from,
        sequenced_by: owner,
        operator_id: proposal.operator_id.clone(),
        correlation: proposal.correlation.clone(),
        recovery_generation,
        apply_tick,
        order: GmActionOrder::new(proposal.from, journal.next_sequence()),
        action: proposal.action.clone(),
    };
    journal.insert(grant.clone())?;
    Ok(grant)
}

/// Insert an owner-sequenced grant on a receiving peer with the same initial
/// pause baseline the owner used when it sequenced the first grant.
///
/// This is intentionally the replicated admission boundary rather than
/// snapshot capture: a technical join hold is current pause state, not a
/// historical GM action, so the transferred journal must remain byte-exact
/// until a real typed action exists.
pub(crate) fn insert_replicated_grant(
    journal: &mut GmActionJournal,
    current_paused: bool,
    grant: GmActionGrant,
) -> Result<(), GmActionRefusalReason> {
    if journal.is_empty() {
        journal.adopt_initial_pause(current_paused);
    }
    journal.insert(grant).map(|_| ())
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
    request.action.validate()?;
    crate::gm_puppet::validate_station_action_in_world(world, &request.action, &bound)?;
    let paused = world
        .get_resource::<SimulationPaused>()
        .is_some_and(|paused| paused.0);
    let technical_join_hold = world
        .get_resource::<crate::gm_join::GmJoinPauseHold>()
        .is_some_and(crate::gm_join::GmJoinPauseHold::active);
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
        sequence_owner_proposal(
            &mut journal,
            &proposal,
            owner,
            now,
            ready_through,
            paused,
            technical_join_hold,
        )
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
            recovery_generation: 0,
            apply_tick,
            order: GmActionOrder::new(origin, sequence),
            action: GmAction::SetSessionPaused { active },
        }
    }

    fn station_grant(
        sequence: u64,
        apply_tick: u64,
        correlation: &str,
        action: GmAction,
    ) -> GmActionGrant {
        GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new(correlation).unwrap(),
            recovery_generation: 0,
            apply_tick,
            order: GmActionOrder::new(HostSlot(1), sequence),
            action,
        }
    }

    fn station_apply_app(
        tick: u64,
        ratings: crate::ship::components::ActiveStationRatings,
        puppets: crate::gm_puppet::StationPuppets,
        grants: impl IntoIterator<Item = GmActionGrant>,
    ) -> App {
        let config = crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
console = "helm.html"

[[station.rating]]
name = "Manual"
automated_systems = []

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"
"#,
            &["helm_thrust"],
        )
        .unwrap();
        let mut sources = crate::ship::components::ShipSystemControlSources::default();
        sources.0.set(
            crate::core::messages::SystemId("helm-thrust".into()),
            crate::ship::control_source::ControlSource::Ai,
        );
        let mut journal = GmActionJournal::default();
        for grant in grants {
            journal.insert(grant).unwrap();
        }
        let mut app = App::new();
        app.insert_resource(crate::sim_tick::SimTick(tick))
            .insert_resource(SimulationPaused(false))
            .insert_resource(journal)
            .init_resource::<GmActionLog>()
            .insert_resource(puppets)
            .init_resource::<crate::gm_puppet::PendingGmStationCommands>()
            .add_systems(Update, apply_due_actions);
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("player-1".into()),
            crate::ship::components::ShipConfigComponent(config),
            ratings,
            sources,
        ));
        app
    }

    // -- Firing an authored GM event (issue #1301) ---------------------------

    fn manual_event_state(
        id: &str,
        layer: Option<&str>,
        repeat: bool,
        fire: bool,
    ) -> crate::world::content::TriggerState {
        let mut trigger =
            crate::world::config::scripted_trigger(crate::world::config::TriggerCondition::Manual);
        trigger.id = Some(id.to_string());
        trigger.repeat = repeat;
        let mut controls = crate::world::config::GmEventControls::fire_only(
            id.to_string(),
            format!("world.gm.event.{id}"),
        );
        controls.fire = fire;
        trigger.gm_controls = Some(controls);
        crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: layer.map(str::to_string),
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }
    }

    fn fire_grant(sequence: u64, apply_tick: u64, correlation: &str, event: &str) -> GmActionGrant {
        GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new(correlation).unwrap(),
            recovery_generation: 0,
            apply_tick,
            order: GmActionOrder::new(HostSlot(1), sequence),
            action: GmAction::FireGmEvent {
                event: event.into(),
            },
        }
    }

    fn fire_app(
        tick: u64,
        states: Vec<crate::world::content::TriggerState>,
        grants: impl IntoIterator<Item = GmActionGrant>,
    ) -> App {
        let mut journal = GmActionJournal::default();
        for grant in grants {
            journal.insert(grant).unwrap();
        }
        let runtime = crate::world::server::WorldContentRuntime {
            trigger_states: states,
            ..Default::default()
        };
        let mut app = App::new();
        app.insert_resource(crate::sim_tick::SimTick(tick))
            .insert_resource(SimulationPaused(false))
            .insert_resource(journal)
            .init_resource::<GmActionLog>()
            .insert_resource(runtime)
            .add_systems(Update, apply_due_actions);
        app
    }

    fn outcomes(app: &App) -> Vec<(GmActionOutcome, Option<GmActionRefusalReason>)> {
        app.world()
            .resource::<GmActionJournal>()
            .applied_results()
            .iter()
            .map(|result| (result.outcome, result.reason))
            .collect()
    }

    fn armed(app: &App) -> Vec<String> {
        app.world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_gm_event_fires
            .iter()
            .cloned()
            .collect()
    }

    /// The whole idempotency contract in one run: the first Fire arms the
    /// event, and a SECOND Fire -- a different correlation, so not a retry --
    /// is a deterministic No-op rather than a second arm.
    #[test]
    fn a_second_fire_of_an_armed_event_is_a_deterministic_no_op() {
        let mut app = fire_app(
            5,
            vec![manual_event_state("breach", None, false, true)],
            [
                fire_grant(1, 5, "fire-a", "base-world::breach"),
                fire_grant(2, 5, "fire-b", "base-world::breach"),
            ],
        );
        app.update();

        assert_eq!(
            outcomes(&app),
            vec![
                (GmActionOutcome::Applied, None),
                (GmActionOutcome::NoOp, None)
            ]
        );
        assert_eq!(
            armed(&app),
            vec!["base-world::breach".to_string()],
            "one arm, no matter how many GMs pressed the button"
        );
    }

    /// A spent once-only event revalidates as a No-op at the apply boundary,
    /// which is what stops a delayed grant re-running a lifecycle the trigger
    /// pipeline has already consumed.
    #[test]
    fn firing_a_spent_one_shot_event_is_a_no_op_and_a_repeatable_one_re_arms() {
        let mut spent = manual_event_state("breach", None, false, true);
        spent.fired = true;
        let mut reusable = manual_event_state("scan", None, true, true);
        reusable.fired = true;
        let mut app = fire_app(
            9,
            vec![spent, reusable],
            [
                fire_grant(1, 9, "fire-a", "base-world::breach"),
                fire_grant(2, 9, "fire-b", "base-world::scan"),
            ],
        );
        app.update();

        assert_eq!(
            outcomes(&app),
            vec![
                (GmActionOutcome::NoOp, None),
                (GmActionOutcome::Applied, None)
            ]
        );
        assert_eq!(armed(&app), vec!["base-world::scan".to_string()]);
    }

    /// Unknown, wrong-layer and control-less ids are all the same answer: a
    /// canonical refusal, decided against the LIVE table at the apply tick.
    #[test]
    fn a_fire_that_names_no_operable_event_is_refused_at_the_apply_boundary() {
        let mut app = fire_app(
            2,
            vec![
                manual_event_state("breach", Some("assets/worlds/layer.toml"), false, true),
                manual_event_state("locked", None, false, false),
            ],
            [
                fire_grant(1, 2, "fire-a", "base-world::missing"),
                // Right authored id, wrong layer.
                fire_grant(2, 2, "fire-b", "base-world::breach"),
                // Listed, but declares no Fire control.
                fire_grant(3, 2, "fire-c", "base-world::locked"),
            ],
        );
        app.update();

        assert_eq!(
            outcomes(&app),
            vec![
                (
                    GmActionOutcome::Refused,
                    Some(GmActionRefusalReason::UnknownGmEvent)
                ),
                (
                    GmActionOutcome::Refused,
                    Some(GmActionRefusalReason::UnknownGmEvent)
                ),
                (
                    GmActionOutcome::Refused,
                    Some(GmActionRefusalReason::UnknownGmEvent)
                ),
            ]
        );
        assert!(armed(&app).is_empty());
    }

    /// Every durable result names the event it fired, so the activity feed and
    /// the mission panel can attribute it without re-reading the journal.
    #[test]
    fn a_fire_result_carries_its_stable_event_identity() {
        let mut app = fire_app(
            1,
            vec![manual_event_state("breach", None, false, true)],
            [fire_grant(1, 1, "fire-a", "base-world::breach")],
        );
        app.update();

        let journal = app.world().resource::<GmActionJournal>();
        let result = &journal.applied_results()[0];
        assert_eq!(result.action_kind, GmActionKind::EventControl);
        assert_eq!(result.target.as_deref(), Some("base-world::breach"));
        assert!(result.requested_active, "a Fire is always a request to act");
        // And the projection seam the mission panel reads selects exactly it.
        let projected = projected_results(
            GmActionKind::EventControl,
            &journal.applied_log(),
            &LocalGmActionRefusals::default(),
        );
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].target.as_deref(), Some("base-world::breach"));
        assert!(
            projected_results(
                GmActionKind::SessionPause,
                &journal.applied_log(),
                &LocalGmActionRefusals::default()
            )
            .is_empty(),
            "an event result must not leak into the Session feed"
        );
    }

    /// A Fire refused BEFORE it could become a grant still names the event it
    /// tried to fire, on both lanes that can refuse one: the local browser
    /// ingress and the owner's canonical decision. Without the identity the
    /// operator gets "someone fired ''" in the feed and the mission panel.
    #[test]
    fn a_refused_fire_still_names_the_event_on_both_refusal_lanes() {
        let fire = GmAction::FireGmEvent {
            event: "base-world::breach_alarm".into(),
        };

        // Ingress: the operator claim does not match the frozen slot binding,
        // exactly as `drain_gm_action_input` sees it in the browser.
        let mut world = admitted_world();
        let request = GmActionRequest {
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("fire-a").unwrap(),
            action: fire.clone(),
        };
        assert_eq!(
            submit_local(&mut world, request.clone()),
            Err(GmActionRefusalReason::OperatorMismatch)
        );
        let ingress =
            LoggedGmAction::refused_request(&request, 10, GmActionRefusalReason::OperatorMismatch);
        assert_eq!(ingress.action_kind, GmActionKind::EventControl);
        assert_eq!(ingress.target.as_deref(), Some("base-world::breach_alarm"));

        // Owner lane: the same identity survives the replicated refusal frame
        // and the durable fact derived from it.
        let proposal = GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("fire-b").unwrap(),
            action: fire,
        };
        let refusal = refusal_for(
            HostSlot(1),
            &proposal,
            11,
            GmActionRefusalReason::JournalFull,
        );
        assert_eq!(refusal.target.as_deref(), Some("base-world::breach_alarm"));
        assert_eq!(
            refusal.logged().target.as_deref(),
            Some("base-world::breach_alarm")
        );

        // And both reach the mission panel's feed as event results, not as
        // targetless rows the Session feed would have to explain.
        let mut refusals = LocalGmActionRefusals::default();
        refusals.push(ingress);
        refusals.push(refusal.logged());
        let projected = projected_results(
            GmActionKind::EventControl,
            &GmActionLog::default(),
            &refusals,
        );
        assert_eq!(projected.len(), 2);
        assert!(projected
            .iter()
            .all(|entry| entry.target.as_deref() == Some("base-world::breach_alarm")));
    }

    /// A world-less peer (the pure fixtures and the replay harness) refuses
    /// rather than panicking or silently succeeding.
    #[test]
    fn a_fire_without_a_loaded_world_is_refused_rather_than_lost() {
        let mut journal = GmActionJournal::default();
        journal
            .insert(fire_grant(1, 3, "fire-a", "base-world::breach"))
            .unwrap();
        let mut app = App::new();
        app.insert_resource(crate::sim_tick::SimTick(3))
            .insert_resource(SimulationPaused(false))
            .insert_resource(journal)
            .init_resource::<GmActionLog>()
            .add_systems(Update, apply_due_actions);
        app.update();

        assert_eq!(
            outcomes(&app),
            vec![(
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            )]
        );
    }

    #[test]
    fn delayed_takeover_revalidates_backfill_at_the_apply_boundary() {
        let helm = crate::core::messages::StationId("helm".into());
        let ship = crate::command_admission::ShipKey("player-1".into());
        let action = GmAction::SetStationPuppet {
            ship: ship.clone(),
            station: helm.clone(),
            active: true,
        };
        let mut sequenced_ratings = crate::ship::components::ActiveStationRatings::default();
        sequenced_ratings
            .0
            .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
        // The sequencing snapshot was valid while the holder was disconnected.
        // A reconnect changes the live rating before the delayed grant is due.
        let mut current_ratings = sequenced_ratings;
        current_ratings.0.insert(helm.clone(), "Manual".into());
        let mut app = station_apply_app(
            12,
            current_ratings,
            crate::gm_puppet::StationPuppets::default(),
            [station_grant(1, 12, "delayed-takeover", action)],
        );
        app.update();

        let target = crate::gm_puppet::StationPuppetTarget::new(ship, helm);
        assert!(!app
            .world()
            .resource::<crate::gm_puppet::StationPuppets>()
            .is_active(&target));
        let entry = &app.world().resource::<GmActionLog>().entries()[0];
        assert_eq!(entry.outcome, GmActionOutcome::Refused);
        assert_eq!(
            entry.reason,
            Some(GmActionRefusalReason::StationNotBackfill)
        );
        assert_eq!(
            app.world().resource::<GmActionJournal>().applied_results(),
            app.world().resource::<GmActionLog>().entries(),
            "the apply-time refusal is durable journal state",
        );
    }

    #[test]
    fn recovery_generation_keeps_old_work_stale_after_rejoin_and_accepts_new_work() {
        let helm = crate::core::messages::StationId("helm".into());
        let ship = crate::command_admission::ShipKey("player-1".into());
        let mut ratings = crate::ship::components::ActiveStationRatings::default();
        ratings
            .0
            .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
        let mut new_takeover = station_grant(
            2,
            13,
            "new-incarnation-takeover",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: true,
            },
        );
        new_takeover.recovery_generation = 1;
        let mut app = station_apply_app(
            12,
            ratings,
            crate::gm_puppet::StationPuppets::default(),
            [
                station_grant(
                    1,
                    12,
                    "old-incarnation-takeover",
                    GmAction::SetStationPuppet {
                        ship: ship.clone(),
                        station: helm.clone(),
                        active: true,
                    },
                ),
                new_takeover,
            ],
        );
        app.world_mut()
            .resource_mut::<GmActionJournal>()
            .record_slot_recovery(HostSlot(1), 10)
            .unwrap();
        let mut session = crate::lockstep::LockstepSession::new(HostSlot(2), [HostSlot(1)], 0);
        session.depart(HostSlot(1));
        session.rejoin(HostSlot(1), 10);
        assert!(!session.has_departed(HostSlot(1)), "fixture crossed rejoin");
        app.world_mut()
            .insert_resource(crate::lockstep::FleetLockstep(session));

        app.update();
        let first = &app.world().resource::<GmActionLog>().entries()[0];
        assert_eq!(first.outcome, GmActionOutcome::Refused);
        assert_eq!(first.reason, Some(GmActionRefusalReason::NotGameMaster));

        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 13;
        app.update();
        assert_eq!(
            app.world()
                .resource::<GmActionLog>()
                .entries()
                .iter()
                .map(|entry| (entry.outcome, entry.reason))
                .collect::<Vec<_>>(),
            vec![
                (
                    GmActionOutcome::Refused,
                    Some(GmActionRefusalReason::NotGameMaster),
                ),
                (GmActionOutcome::Applied, None),
            ],
        );
        assert!(app
            .world()
            .resource::<crate::gm_puppet::StationPuppets>()
            .is_active(&crate::gm_puppet::StationPuppetTarget::new(ship, helm)));
    }

    #[test]
    fn recovery_boundary_preserves_same_tick_order_and_round_trips() {
        let helm = crate::core::messages::StationId("helm".into());
        let ship = crate::command_admission::ShipKey("player-1".into());
        let mut ratings = crate::ship::components::ActiveStationRatings::default();
        ratings
            .0
            .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
        let mut recovered_release = station_grant(
            2,
            10,
            "recovered-boundary-release",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: false,
            },
        );
        recovered_release.recovery_generation = 1;
        let mut app = station_apply_app(
            10,
            ratings,
            crate::gm_puppet::StationPuppets::default(),
            [
                station_grant(
                    1,
                    10,
                    "old-boundary-takeover",
                    GmAction::SetStationPuppet {
                        ship: ship.clone(),
                        station: helm.clone(),
                        active: true,
                    },
                ),
                recovered_release,
            ],
        );
        app.world_mut()
            .resource_mut::<GmActionJournal>()
            .record_slot_recovery(HostSlot(1), 10)
            .unwrap();
        app.update();
        assert_eq!(
            app.world()
                .resource::<GmActionLog>()
                .entries()
                .iter()
                .map(|entry| entry.outcome)
                .collect::<Vec<_>>(),
            [GmActionOutcome::Applied, GmActionOutcome::Applied],
        );
        assert!(!app
            .world()
            .resource::<crate::gm_puppet::StationPuppets>()
            .is_active(&crate::gm_puppet::StationPuppetTarget::new(ship, helm)));

        let journal = app.world().resource::<GmActionJournal>();
        let text = ron::to_string(journal).unwrap();
        let restored: GmActionJournal = ron::from_str(&text).unwrap();
        assert_eq!(&restored, journal);
    }

    #[test]
    fn owner_stamps_new_work_with_the_recovery_generation_and_boundary() {
        let mut journal = GmActionJournal::default();
        assert_eq!(journal.record_slot_recovery(HostSlot(2), 30), Ok(1));
        let proposal = GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("after-recovery").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        let grant =
            sequence_owner_proposal(&mut journal, &proposal, HostSlot(1), 20, 20, false, false)
                .unwrap();
        assert_eq!(grant.recovery_generation, 1);
        assert_eq!(grant.apply_tick, 30);
    }

    #[test]
    fn same_tick_station_command_and_release_keep_canonical_admission_order() {
        use crate::core::messages::{StationId, SystemControlPayload, SystemId};

        let ship = crate::command_admission::ShipKey("player-1".into());
        let helm = StationId("helm".into());
        let puppet_target = crate::gm_puppet::StationPuppetTarget::new(ship.clone(), helm.clone());
        let command = || GmAction::IssueStationCommand {
            ship: ship.clone(),
            station: helm.clone(),
            target: SystemId("helm-thrust".into()),
            payload: crate::core::codec::canonical_system_command(
                &SystemControlPayload::SetThrust { value: 0.75 },
            )
            .unwrap(),
        };
        let release = || GmAction::SetStationPuppet {
            ship: ship.clone(),
            station: helm.clone(),
            active: false,
        };
        let mut ratings = crate::ship::components::ActiveStationRatings::default();
        ratings
            .0
            .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());

        let mut puppets = crate::gm_puppet::StationPuppets::default();
        puppets.set_operator(puppet_target.clone(), "gm-1".into(), true);
        let mut command_first = station_apply_app(
            20,
            ratings.clone(),
            puppets.clone(),
            [
                station_grant(1, 20, "command-first", command()),
                station_grant(2, 20, "release-second", release()),
            ],
        );
        command_first.update();
        assert_eq!(
            command_first
                .world()
                .resource::<crate::gm_puppet::PendingGmStationCommands>()
                .entries()
                .len(),
            1,
            "a later same-tick release cannot retroactively drop an admitted command",
        );
        assert!(!command_first
            .world()
            .resource::<crate::gm_puppet::StationPuppets>()
            .is_active(&puppet_target));
        assert_eq!(
            command_first
                .world()
                .resource::<GmActionLog>()
                .entries()
                .iter()
                .map(|entry| (entry.outcome, entry.reason))
                .collect::<Vec<_>>(),
            vec![
                (GmActionOutcome::Applied, None),
                (GmActionOutcome::Applied, None),
            ],
        );

        let mut release_first = station_apply_app(
            20,
            ratings,
            puppets,
            [
                station_grant(1, 20, "release-first", release()),
                station_grant(2, 20, "command-second", command()),
            ],
        );
        release_first.update();
        assert!(release_first
            .world()
            .resource::<crate::gm_puppet::PendingGmStationCommands>()
            .entries()
            .is_empty());
        assert_eq!(
            release_first
                .world()
                .resource::<GmActionLog>()
                .entries()
                .iter()
                .map(|entry| (entry.outcome, entry.reason))
                .collect::<Vec<_>>(),
            vec![
                (GmActionOutcome::Applied, None),
                (
                    GmActionOutcome::Refused,
                    Some(GmActionRefusalReason::StationNotPuppeted),
                ),
            ],
        );
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
        let resume = sequence_owner_proposal(&mut canonical, &resume, owner, 20, 20, true, false)
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
        let late_pause =
            sequence_owner_proposal(&mut canonical, &late_pause, owner, 20, 20, true, false)
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
    fn a_replicated_first_resume_adopts_the_same_paused_baseline_as_the_owner() {
        let owner = HostSlot(1);
        let proposal = GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("resume-technical-hold").unwrap(),
            action: GmAction::SetSessionPaused { active: false },
        };
        let mut canonical = GmActionJournal::default();
        let resume = sequence_owner_proposal(&mut canonical, &proposal, owner, 20, 20, true, true)
            .expect("the owner sequences the typed Resume at the stopped boundary");

        let mut receiver = GmActionJournal::default();
        insert_replicated_grant(&mut receiver, true, resume)
            .expect("the authenticated owner grant is admitted");

        assert_eq!(receiver, canonical);
        assert_eq!(
            receiver.log_through(20).entries()[0].outcome,
            GmActionOutcome::Applied
        );
        assert!(!receiver.log_through(20).paused());
    }

    #[test]
    fn technical_join_hold_sequences_resume_at_the_stopped_tick() {
        let owner = HostSlot(1);
        let mut canonical = GmActionJournal::default();
        canonical
            .insert(grant(2, 1, 10, "old-pause", true))
            .unwrap();
        canonical
            .insert(grant(2, 2, 10, "old-resume", false))
            .unwrap();
        let resume = GmActionProposal {
            from: HostSlot(3),
            operator_id: "gm-3".into(),
            correlation: GmActionId::new("join-hold-resume").unwrap(),
            action: GmAction::SetSessionPaused { active: false },
        };

        let grant = sequence_owner_proposal(&mut canonical, &resume, owner, 42, 99, true, true)
            .expect("the technical hold accepts an explicit Resume");
        assert_eq!(
            grant.apply_tick, 42,
            "the action cannot wait for a future tick the join hold forbids"
        );
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
            recovery_generation: 0,
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
            action_kind: GmActionKind::SessionPause,
            requested_active: true,
            tick: 7,
            reason: GmActionRefusalReason::JournalFull,
            target: None,
        };
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(refused.clone()), &roster),
            Ok(())
        );
        // A replicated refusal must carry exactly the target its family has:
        // an event-control refusal names its event, and no other family does.
        let mut targetless_fire = refused.clone();
        targetless_fire.action_kind = GmActionKind::EventControl;
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(targetless_fire.clone()), &roster),
            Err(GmActionRefusalReason::InvalidAction)
        );
        targetless_fire.target = Some("base-world::breach_alarm".into());
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(targetless_fire), &roster),
            Ok(())
        );
        let mut targeted_pause = refused.clone();
        targeted_pause.target = Some("base-world::breach_alarm".into());
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(targeted_pause), &roster),
            Err(GmActionRefusalReason::InvalidAction)
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
    fn station_command_projection_preserves_exact_terminal_correlations_and_outcomes() {
        let station_applied = LoggedGmAction {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("iframe-applied").unwrap(),
            action_kind: GmActionKind::StationCommand,
            requested_active: true,
            outcome: GmActionOutcome::Applied,
            tick: 41,
            reason: None,
            order: Some(GmActionOrder::new(HostSlot(1), 1)),
            target: None,
        };
        let pause = LoggedGmAction {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("pause-other-surface").unwrap(),
            action_kind: GmActionKind::SessionPause,
            requested_active: true,
            outcome: GmActionOutcome::Applied,
            tick: 40,
            reason: None,
            order: Some(GmActionOrder::new(HostSlot(1), 0)),
            target: None,
        };
        let station_refused = LoggedGmAction::refused(
            "gm-1".into(),
            GmActionId::new("iframe-refused").unwrap(),
            GmActionKind::StationCommand,
            true,
            42,
            GmActionRefusalReason::StationNotPuppeted,
        );
        let station_pending = LoggedGmAction {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("iframe-still-pending").unwrap(),
            action_kind: GmActionKind::StationCommand,
            requested_active: true,
            outcome: GmActionOutcome::Pending,
            tick: 43,
            reason: None,
            order: Some(GmActionOrder::new(HostSlot(1), 2)),
            target: None,
        };
        let log = GmActionLog {
            entries: vec![pause, station_applied.clone(), station_pending],
            paused: true,
        };
        let mut supplemental = LocalGmActionRefusals::default();
        supplemental.push(station_refused.clone());

        assert_eq!(
            projected_results(GmActionKind::StationCommand, &log, &supplemental),
            [station_applied, station_refused]
        );
    }

    #[test]
    fn pending_is_valid_only_for_a_station_command_waiting_on_its_consumer() {
        let mut journal = GmActionJournal::default();
        let pause = grant(1, 1, 41, "pause-cannot-pend", true);
        journal.insert(pause.clone()).unwrap();
        assert_eq!(
            journal.record_applied_result(LoggedGmAction {
                operator_id: pause.operator_id,
                correlation: pause.correlation,
                action_kind: GmActionKind::SessionPause,
                requested_active: true,
                outcome: GmActionOutcome::Pending,
                tick: pause.apply_tick,
                reason: None,
                order: Some(pause.order),
                target: None,
            }),
            Err("pending GM result is not a Station command"),
        );
        assert_eq!(journal.applied_grants(), 0);
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
