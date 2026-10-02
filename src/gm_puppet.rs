//! Station-scoped Game Master puppeting (issue #1299).
//!
//! This resource records only the authoritative takeover membership.  It does
//! not replace `Player.station`, a Station rating, or a System command source:
//! those remain the existing tenure and admission vocabulary.  A takeover is
//! an overlay that suppresses the selected Station's AI emitters while one or
//! more equal GM operators are present.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command_admission::log::ShipKey;
use crate::core::messages::StationId;
use crate::entities::spawner::EntityUuid;
use crate::ship::components::{
    ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::control_source::ControlSource;

pub mod capability;

pub const MAX_CANONICAL_SYSTEM_COMMAND_BYTES: usize = 8192;
pub const MAX_STATION_PUPPET_ACTIVITY: usize = 128;
const GM_STATION_FEEDBACK_TOKEN_PREFIX: &str = "ai:gm-station-feedback:";

#[derive(bevy::ecs::schedule::SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct StationPuppetAdmissionSet;

/// Canonical JSON for one existing `SystemControlPayload`. JSON ownership stays
/// in `core::codec`; this bounded wrapper lets the typed GM journal retain an
/// Eq/serde shape without teaching the domain module about serde_json or
/// cloning the command enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalSystemCommandPayload(String);

impl CanonicalSystemCommandPayload {
    pub fn new(value: String) -> Result<Self, &'static str> {
        if value.is_empty() || value.len() > MAX_CANONICAL_SYSTEM_COMMAND_BYTES {
            return Err("canonical System command is empty or too large");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable address of one authored Station on one deterministic ship.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StationPuppetTarget {
    pub ship: ShipKey,
    pub station: StationId,
}

impl StationPuppetTarget {
    pub fn new(ship: ShipKey, station: StationId) -> Self {
        Self { ship, station }
    }
}

/// One occupied takeover row. Operators are sorted and unique so equal GMs
/// applying the same canonical action prefix always produce identical bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationPuppet {
    pub target: StationPuppetTarget,
    pub operators: Vec<String>,
}

/// The complete Station takeover set for the current run.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationPuppets {
    entries: Vec<StationPuppet>,
}

/// Previous authoritative target set used only to notice a release and
/// re-apply that Station's ordinary active rating. It is derived lifecycle
/// memory, never a second source of takeover truth.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviousStationPuppetTargets(Vec<StationPuppetTarget>);

impl PreviousStationPuppetTargets {
    pub(crate) fn from_puppets(puppets: &StationPuppets) -> Self {
        Self(puppets.targets())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingGmStationCommand {
    pub tick: u64,
    pub order: crate::gm_action::GmActionOrder,
    pub operator_id: String,
    pub correlation: crate::gm_action::GmActionId,
    pub ship: ShipKey,
    pub station: StationId,
    pub target: crate::core::messages::SystemId,
    /// Payload already accepted at the canonical GM action boundary. It is
    /// source-stripped here and waits only for the ordinary per-tick delivery
    /// buffer to be refilled.
    pub payload: crate::core::messages::SystemControlPayload,
}

/// Commands reduced in PreUpdate and awaiting this tick's ordinary System
/// Admission boundary. Empty again at every completed fold point.
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingGmStationCommands(Vec<PendingGmStationCommand>);

impl PendingGmStationCommands {
    pub fn push(&mut self, command: PendingGmStationCommand) {
        self.0.push(command);
    }

    pub fn entries(&self) -> &[PendingGmStationCommand] {
        &self.0
    }

    pub(crate) fn take_due(&mut self, tick: u64) -> Vec<PendingGmStationCommand> {
        let mut due = Vec::new();
        let mut future = Vec::new();
        for command in std::mem::take(&mut self.0) {
            if command.tick <= tick {
                due.push(command);
            } else {
                future.push(command);
            }
        }
        due.sort_by_key(|command| (command.tick, command.order));
        self.0 = future;
        due
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingGmStationFeedbackRoute {
    order: crate::gm_action::GmActionOrder,
    correlation: crate::core::messages::ActionCorrelationId,
}

/// Transient routing from an ordinary System consumer's feedback message back
/// to the durable GM action result it settles.  Entries contain no behavioral
/// authority and are deliberately absent from snapshots and digests: a
/// snapshotted accepted command is still in `PendingGmStationCommands`, so its
/// deterministic route is rebuilt when that command is delivered after
/// restore.
#[derive(Resource, Debug, Default)]
pub struct PendingGmStationFeedbackRoutes(
    std::collections::BTreeMap<String, PendingGmStationFeedbackRoute>,
);

impl PendingGmStationFeedbackRoutes {
    fn token(order: crate::gm_action::GmActionOrder) -> String {
        // `ai:` is already a host-reserved session-token prefix.  The rest is
        // the canonical action order, so equal peers and restore continuations
        // reconstruct the same opaque reply route without embedding GM identity.
        format!(
            "{GM_STATION_FEEDBACK_TOKEN_PREFIX}{}:{}",
            order.origin.0, order.sequence
        )
    }

    fn register(
        &mut self,
        order: crate::gm_action::GmActionOrder,
        correlation: crate::core::messages::ActionCorrelationId,
    ) -> Result<String, &'static str> {
        let token = Self::token(order);
        if !self.0.contains_key(&token)
            && self.0.len() >= crate::gm_action::MAX_STORED_GM_ACTIONS_PER_RUN
        {
            return Err("GM Station feedback routes exceed the GM journal bound");
        }
        match self.0.entry(token.clone()) {
            std::collections::btree_map::Entry::Occupied(existing)
                if existing.get().order == order && existing.get().correlation == correlation => {}
            std::collections::btree_map::Entry::Occupied(_) => {
                return Err("GM Station feedback route collided");
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(PendingGmStationFeedbackRoute { order, correlation });
            }
        }
        Ok(token)
    }

    fn take_matching(
        &mut self,
        token: &str,
        correlation: &crate::core::messages::ActionCorrelationId,
    ) -> Option<PendingGmStationFeedbackRoute> {
        let route = self.0.get(token)?;
        if route.correlation != *correlation {
            return None;
        }
        self.0.remove(token)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Crew-visible attribution recorded exactly where the command becomes a
/// source-stripped `AdmittedCommand`. Downstream systems never receive this
/// identity; projections and activity surfaces do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationPuppetActivityEntry {
    pub tick: u64,
    pub order: crate::gm_action::GmActionOrder,
    pub operator_id: String,
    pub ship: ShipKey,
    pub station: StationId,
    pub target: crate::core::messages::SystemId,
    pub action: String,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationPuppetActivity(Vec<StationPuppetActivityEntry>);

impl StationPuppetActivity {
    pub fn entries(&self) -> &[StationPuppetActivityEntry] {
        &self.0
    }

    pub(crate) fn push(&mut self, entry: StationPuppetActivityEntry) {
        self.0.push(entry);
        if self.0.len() > MAX_STATION_PUPPET_ACTIVITY {
            self.0.remove(0);
        }
    }

    /// Host loss removes presentation attribution belonging to the departed
    /// operator at the same agreed boundary as its authoritative memberships.
    pub(crate) fn remove_operator(&mut self, operator_id: &str) {
        self.0.retain(|entry| entry.operator_id != operator_id);
    }
}

pub fn validate_station_action(
    action: &crate::gm_action::GmAction,
    operator_id: &str,
    puppets: &StationPuppets,
    config: Option<&crate::ship::config::ShipConfig>,
    _ratings: Option<&ActiveStationRatings>,
) -> Result<(), crate::gm_action::GmActionRefusalReason> {
    use crate::gm_action::{GmAction, GmActionRefusalReason};
    let station = match action {
        // Neither family names a Station, so Station admission has nothing to
        // say about it and defers to its own reducer.
        GmAction::SetSessionPaused { .. }
        | GmAction::FireGmEvent { .. }
        | GmAction::ApplyDirectEffect { .. }
        | GmAction::SpawnPaletteEntity { .. }
        | GmAction::DespawnEntity { .. }
        | GmAction::SetNpcDoctrine { .. }
        | GmAction::SetNpcDoctrineChecked { .. }
        | GmAction::ObjectiveAction { .. }
        | GmAction::ObjectiveInstanceAction { .. }
        | GmAction::SetContactOverride { .. }
        | GmAction::SetContactClassification { .. }
        | GmAction::Presentation { .. }
        | GmAction::SetContactInformation { .. }
        | GmAction::SetSystemDisabled { .. }
        | GmAction::TransmitComms { .. }
        | GmAction::SetEventPaused { .. }
        | GmAction::SetFactionHostility { .. }
        | GmAction::UndoGmAction { .. }
        | GmAction::RequestLiveRestore { .. }
        | GmAction::ArmGmEventSkip { .. }
        | GmAction::BackfillShipSlot { .. } => return Ok(()),
        GmAction::SetStationPuppet { station, .. }
        | GmAction::IssueStationCommand { station, .. } => station,
    };
    let Some(config) = config else {
        return Err(GmActionRefusalReason::UnknownStation);
    };
    if config.station(station).is_none() {
        return Err(GmActionRefusalReason::UnknownStation);
    }
    let target = StationPuppetTarget::new(
        action
            .ship_key()
            .expect("Station action has a ship")
            .clone(),
        station.clone(),
    );
    match action {
        // Membership overlays AI operation, never human tenure. A holder's
        // live rating (including a reconnect before this boundary) does not
        // grant an exclusive lock against an equal GM operator.
        GmAction::SetStationPuppet { .. } => Ok(()),
        GmAction::IssueStationCommand {
            target: system,
            payload,
            ..
        } => {
            if !puppets.is_operated_by(&target, operator_id) {
                return Err(GmActionRefusalReason::StationNotPuppeted);
            }
            let payload = crate::core::codec::decode_canonical_system_command(payload.as_str())
                .ok_or(GmActionRefusalReason::InvalidAction)?;
            let effective =
                crate::command_admission::effective_target_for_command(config, system, &payload);
            crate::command_admission::station_for_system(config, None, &effective)
                .as_ref()
                .is_some_and(|owner| owner == station)
                .then_some(())
                .ok_or(GmActionRefusalReason::SystemOutsideStation)
        }
        GmAction::SetSessionPaused { .. }
        | GmAction::FireGmEvent { .. }
        | GmAction::ApplyDirectEffect { .. }
        | GmAction::SpawnPaletteEntity { .. }
        | GmAction::DespawnEntity { .. }
        | GmAction::SetNpcDoctrine { .. }
        | GmAction::SetNpcDoctrineChecked { .. }
        | GmAction::ObjectiveAction { .. }
        | GmAction::ObjectiveInstanceAction { .. }
        | GmAction::SetContactOverride { .. }
        | GmAction::SetContactClassification { .. }
        | GmAction::Presentation { .. }
        | GmAction::SetContactInformation { .. }
        | GmAction::SetSystemDisabled { .. }
        | GmAction::TransmitComms { .. }
        | GmAction::SetEventPaused { .. }
        | GmAction::SetFactionHostility { .. }
        | GmAction::UndoGmAction { .. }
        | GmAction::RequestLiveRestore { .. }
        | GmAction::ArmGmEventSkip { .. }
        | GmAction::BackfillShipSlot { .. } => unreachable!(),
    }
}

pub fn validate_station_action_in_world(
    world: &mut World,
    action: &crate::gm_action::GmAction,
    operator_id: &str,
) -> Result<(), crate::gm_action::GmActionRefusalReason> {
    // A ship key also routes contact policy and other directed actions. Only
    // actual Station actions require an authentic console capability.
    let (ship, station) = match action {
        crate::gm_action::GmAction::SetStationPuppet { ship, station, .. }
        | crate::gm_action::GmAction::IssueStationCommand { ship, station, .. } => (ship, station),
        _ => return Ok(()),
    };
    let puppets = world
        .get_resource::<StationPuppets>()
        .cloned()
        .unwrap_or_default();
    // An existing member can always release, including after target removal.
    // Takeover and commands use the same fail-closed verdict as the offer list.
    if !matches!(
        action,
        crate::gm_action::GmAction::SetStationPuppet { active: false, .. }
    ) {
        let mut query = world.query_filtered::<EntityRef, With<crate::server_app::Ship>>();
        let mut matches = query.iter(world).filter(|entity| {
            entity
                .get::<EntityUuid>()
                .is_some_and(|uuid| uuid.0 == ship.0)
        });
        let entity = matches
            .next()
            .ok_or(crate::gm_action::GmActionRefusalReason::UnknownStation)?;
        if matches.next().is_some() {
            return Err(crate::gm_action::GmActionRefusalReason::UnknownStation);
        }
        capability::station_capability(
            &entity,
            station,
            Some(
                &crate::ship::system_registry::SystemKindRegistry::with_core_systems()
                    .expect("built-in System descriptors"),
            ),
            world.get_resource::<crate::command_admission::router::AdmittedConsumerRegistry>(),
        )?;
    } else {
        return Ok(());
    }
    let found = {
        let mut query = world.query_filtered::<
            (&EntityUuid, &ShipConfigComponent, &ActiveStationRatings),
            With<crate::server_app::Ship>,
        >();
        query
            .iter(world)
            .find(|(uuid, ..)| uuid.0 == ship.0)
            .map(|(_, config, ratings)| (config.0.clone(), ratings.clone()))
    };
    validate_station_action(
        action,
        operator_id,
        &puppets,
        found.as_ref().map(|(config, _)| config),
        found.as_ref().map(|(_, ratings)| ratings),
    )
}

impl StationPuppets {
    pub fn operates_ship(&self, ship: &str) -> bool {
        self.entries.iter().any(|entry| entry.target.ship.0 == ship)
    }
    pub fn entries(&self) -> &[StationPuppet] {
        &self.entries
    }

    pub fn operators(&self, target: &StationPuppetTarget) -> &[String] {
        self.find(target)
            .map_or(&[], |entry| entry.operators.as_slice())
    }

    pub fn is_active(&self, target: &StationPuppetTarget) -> bool {
        self.find(target).is_some()
    }

    pub fn is_operated_by(&self, target: &StationPuppetTarget, operator_id: &str) -> bool {
        self.find(target).is_some_and(|entry| {
            entry
                .operators
                .binary_search_by(|id| id.as_str().cmp(operator_id))
                .is_ok()
        })
    }

    /// Apply one absolute operator membership assignment. Returns `true` only
    /// when authoritative state changed; an exact retry is a deterministic
    /// no-op. Releasing one GM never releases an equal peer.
    pub fn set_operator(
        &mut self,
        target: StationPuppetTarget,
        operator_id: String,
        active: bool,
    ) -> bool {
        match self.position(&target) {
            Ok(index) => {
                let operators = &mut self.entries[index].operators;
                match (operators.binary_search(&operator_id), active) {
                    (Ok(_), true) | (Err(_), false) => false,
                    (Err(position), true) => {
                        operators.insert(position, operator_id);
                        true
                    }
                    (Ok(position), false) => {
                        operators.remove(position);
                        if operators.is_empty() {
                            self.entries.remove(index);
                        }
                        true
                    }
                }
            }
            Err(_) if !active => false,
            Err(index) => {
                self.entries.insert(
                    index,
                    StationPuppet {
                        target,
                        operators: vec![operator_id],
                    },
                );
                true
            }
        }
    }

    /// Remove one departed GM from every takeover without disturbing equal
    /// peers. Returns the targets whose authoritative membership changed.
    pub fn remove_operator_everywhere(&mut self, operator_id: &str) -> Vec<StationPuppetTarget> {
        let targets: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .operators
                    .binary_search_by(|id| id.as_str().cmp(operator_id))
                    .is_ok()
            })
            .map(|entry| entry.target.clone())
            .collect();
        for target in &targets {
            self.set_operator(target.clone(), operator_id.to_string(), false);
        }
        targets
    }

    pub(crate) fn targets(&self) -> Vec<StationPuppetTarget> {
        self.entries
            .iter()
            .map(|entry| entry.target.clone())
            .collect()
    }

    fn find(&self, target: &StationPuppetTarget) -> Option<&StationPuppet> {
        self.position(target)
            .ok()
            .and_then(|index| self.entries.get(index))
    }

    fn position(&self, target: &StationPuppetTarget) -> Result<usize, usize> {
        self.entries.binary_search_by(|entry| {
            (
                entry.target.ship.0.as_str(),
                entry.target.station.0.as_str(),
            )
                .cmp(&(target.ship.0.as_str(), target.station.0.as_str()))
        })
    }
}

/// Membership itself is the fidelity hold. Promote before Admission, not in
/// Physics after the first consumer already ran. Snapshot continuation with a
/// queued command therefore reconstructs the same full consumer bundle before
/// delivery. A second equal GM never resets existing intent or policy state.
pub fn prepare_station_puppet_fidelity(
    time: Res<Time>,
    puppets: Res<StationPuppets>,
    pending: Res<PendingGmStationCommands>,
    ships: Query<
        (Entity, &EntityUuid),
        (
            With<crate::server_app::Ship>,
            Without<crate::lockstep::FleetSlotOf>,
            Without<crate::ai::server::AiHighFidelity>,
        ),
    >,
    mut commands: Commands,
) {
    for (entity, uuid) in &ships {
        if puppets.operates_ship(&uuid.0)
            || pending
                .entries()
                .iter()
                .any(|command| command.ship.0 == uuid.0)
        {
            commands.entity(entity).insert((
                crate::ai::server::ai_high_fidelity_components(),
                crate::ai::server::LodTransitionTimer {
                    last_state_change_secs: time.elapsed_secs() as f64,
                },
            ));
        }
    }
}

/// All destruction paths meet here after Damage, including beam, blaster,
/// script, layer unload and GM removal. History remains attributed; only live
/// memberships are pruned. Pending delivery still owns its honest refusal.
pub fn prune_removed_station_puppets(
    ships: Query<(&EntityUuid, &ShipConfigComponent), With<crate::server_app::Ship>>,
    mut puppets: ResMut<StationPuppets>,
) {
    puppets.entries.retain(|entry| {
        let mut matches = ships
            .iter()
            .filter(|(uuid, _)| uuid.0 == entry.target.ship.0);
        matches
            .next()
            .is_some_and(|(_, config)| config.0.station(&entry.target.station).is_some())
            && matches.next().is_none()
    });
}

/// A target can be destroyed after delivery but before its consumer runs.
/// First let actual consumer feedback settle (including an Applied answer
/// produced before destruction); only then refuse orphaned routes. Otherwise
/// the journal could retain an unprojected Pending fact forever.
pub fn settle_removed_station_feedback(
    ships: Query<(&EntityUuid, &ShipConfigComponent), With<crate::server_app::Ship>>,
    mut routes: ResMut<PendingGmStationFeedbackRoutes>,
    mut journal: Option<ResMut<crate::gm_action::GmActionJournal>>,
    mut log: Option<ResMut<crate::gm_action::GmActionLog>>,
) {
    let Some(journal) = journal.as_deref_mut() else {
        return;
    };
    let orphaned = routes
        .0
        .iter()
        .filter_map(|(token, route)| {
            let grant = journal
                .grants()
                .iter()
                .find(|grant| grant.order == route.order)?;
            let crate::gm_action::GmAction::IssueStationCommand { ship, station, .. } =
                &grant.action
            else {
                return None;
            };
            let mut matches = ships.iter().filter(|(uuid, _)| uuid.0 == ship.0);
            let live = matches
                .next()
                .is_some_and(|(_, config)| config.0.station(station).is_some())
                && matches.next().is_none();
            (!live).then_some((token.clone(), route.order))
        })
        .collect::<Vec<_>>();
    for (token, order) in orphaned {
        journal
            .refuse_pending_station_command(
                order,
                crate::gm_action::GmActionRefusalReason::SystemUnavailable,
            )
            .expect("an unfinished consumer route has a canonical pending result");
        routes.0.remove(&token);
        if let Some(log) = log.as_deref_mut() {
            *log = journal.applied_log();
        }
    }
}

/// Overlay active Station takeovers onto ordinary rating control. The overlay
/// deliberately uses `Human`: every existing command consumer then sees the
/// same source-stripped admitted command shape, while every AI emitter sees its
/// ordinary `operate_ai == false` policy. Damage-offline remains additive in
/// `ControlSourceResolver::policy_for` and is therefore not bypassed.
pub fn reconcile_station_puppet_control(
    puppets: Res<StationPuppets>,
    mut previous: ResMut<PreviousStationPuppetTargets>,
    mut ships: Query<
        (
            &EntityUuid,
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &ActiveStationRatings,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let current: Vec<_> = puppets
        .entries()
        .iter()
        .map(|entry| entry.target.clone())
        .collect();

    for (uuid, config, mut sources, ratings) in ships.iter_mut() {
        for released in previous
            .0
            .iter()
            .filter(|target| target.ship.0 == uuid.0 && !puppets.is_active(target))
        {
            if let Some(rating) = ratings.0.get(&released.station) {
                crate::ship::rating::apply_rating(
                    &config.0,
                    &released.station,
                    rating,
                    &mut sources.0,
                );
            }
        }

        for active in current.iter().filter(|target| target.ship.0 == uuid.0) {
            // A stale/forged station id controls nothing. Target admission may
            // refuse it earlier, but the authoritative reducer still fails
            // closed if malformed replay data reaches this boundary.
            if config.0.station(&active.station).is_none() {
                continue;
            }
            for system in config.0.systems_for_station(&active.station) {
                sources.0.set(system.id.clone(), ControlSource::Human);
            }
        }
    }

    previous.0 = current;
}

/// Re-assert the active overlay after any same-tick rating or human-seeking
/// resolver write. Admission has already seen the pre-Admission assertion;
/// this second pass is what keeps every later AI emitter suppressed.
pub fn enforce_station_puppet_control(
    puppets: Res<StationPuppets>,
    mut ships: Query<
        (
            &EntityUuid,
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (uuid, config, mut sources) in ships.iter_mut() {
        for entry in puppets
            .entries()
            .iter()
            .filter(|entry| entry.target.ship.0 == uuid.0)
        {
            if config.0.station(&entry.target.station).is_none() {
                continue;
            }
            for system in config.0.systems_for_station(&entry.target.station) {
                sources.0.set(system.id.clone(), ControlSource::Human);
            }
        }
    }
}

/// Append canonically ordered GM commands after ordinary network Admission has
/// cleared/refilled each ship buffer, but before any System consumer runs. The
/// accepted command loses operator identity here; attribution goes only to the
/// parallel activity record.
pub fn admit_station_puppet_commands(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut pending: ResMut<PendingGmStationCommands>,
    mut activity: ResMut<StationPuppetActivity>,
    mut feedback_routes: ResMut<PendingGmStationFeedbackRoutes>,
    mut journal: Option<ResMut<crate::gm_action::GmActionJournal>>,
    mut log: Option<ResMut<crate::gm_action::GmActionLog>>,
    mut ships: Query<
        (
            &EntityUuid,
            &ShipConfigComponent,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let now = tick.as_deref().map_or(0, |tick| tick.0);
    let mut refuse_delivery = |order, reason| {
        let Some(journal) = journal.as_deref_mut() else {
            return;
        };
        journal
            .refuse_pending_station_command(order, reason)
            .expect("an accepted GM Station command has a canonical result");
        if let Some(log) = log.as_deref_mut() {
            *log = journal.applied_log();
        }
    };
    for command in pending.take_due(now) {
        let Some((_, config, mut admitted)) =
            ships.iter_mut().find(|(uuid, ..)| uuid.0 == command.ship.0)
        else {
            refuse_delivery(
                command.order,
                crate::gm_action::GmActionRefusalReason::SystemUnavailable,
            );
            continue;
        };
        let action: &'static str =
            crate::core::messages::SystemControlPayloadDiscriminants::from(&command.payload).into();
        let target_kind = config
            .0
            .systems
            .iter()
            .find(|system| system.id == command.target)
            .map(|system| system.kind.as_str());
        let (response_token, feedback_correlation) =
            if crate::command_admission::supports_correlated_action_feedback_for_kind(
                &command.target,
                &command.payload,
                target_kind,
            ) {
                let correlation = crate::core::messages::ActionCorrelationId::new(
                    command.correlation.as_str().to_string(),
                )
                .expect("a bounded GM correlation is a bounded action correlation");
                let Ok(token) = feedback_routes.register(command.order, correlation.clone()) else {
                    refuse_delivery(
                        command.order,
                        crate::gm_action::GmActionRefusalReason::SystemUnavailable,
                    );
                    continue;
                };
                (Some(token), Some(correlation))
            } else {
                (None, None)
            };
        admitted.0.push(crate::core::messages::AdmittedCommand {
            target: command.target.clone(),
            payload: command.payload,
            response_token,
            feedback_correlation,
        });
        activity.push(StationPuppetActivityEntry {
            tick: command.tick,
            order: command.order,
            operator_id: command.operator_id,
            ship: command.ship,
            station: command.station,
            target: command.target,
            action: action.to_string(),
        });
    }
}

/// Fold ordinary correlated consumer feedback back into the canonical GM
/// result before FixedLast projects, snapshots or digests it.  The consumer
/// sees the exact same source-stripped `AdmittedCommand` shape as a player
/// command; this system alone understands the reserved reply route.
pub fn settle_station_puppet_feedback(
    mut outbound: MessageReader<crate::lobby::server::OutboundMessage>,
    mut feedback_routes: ResMut<PendingGmStationFeedbackRoutes>,
    mut journal: Option<ResMut<crate::gm_action::GmActionJournal>>,
    mut log: Option<ResMut<crate::gm_action::GmActionLog>>,
) {
    for message in outbound.read() {
        let crate::lobby::handler::Target::Token(token) = &message.target else {
            continue;
        };
        if !token.starts_with(GM_STATION_FEEDBACK_TOKEN_PREFIX) {
            continue;
        }
        let crate::core::messages::ServerMessage::ActionFeedback {
            correlation,
            outcome,
        } = &message.msg
        else {
            continue;
        };
        let Some(route) = feedback_routes.take_matching(token, correlation) else {
            continue;
        };
        let Some(journal) = journal.as_deref_mut() else {
            continue;
        };
        journal
            .settle_station_command_result(route.order, *outcome)
            .expect("a registered GM feedback route matches its canonical result");
        if let Some(log) = log.as_deref_mut() {
            *log = journal.applied_log();
        }
    }
}

#[cfg(test)]
#[path = "gm_puppet_tests.rs"]
mod tests;
