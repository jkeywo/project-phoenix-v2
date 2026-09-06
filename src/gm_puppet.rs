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
    ratings: Option<&ActiveStationRatings>,
) -> Result<(), crate::gm_action::GmActionRefusalReason> {
    use crate::gm_action::{GmAction, GmActionRefusalReason};
    let station = match action {
        // Neither family names a Station, so Station admission has nothing to
        // say about it and defers to its own reducer.
        GmAction::SetSessionPaused { .. }
        | GmAction::FireGmEvent { .. }
        | GmAction::ApplyDirectEffect { .. }
        | GmAction::SpawnPaletteEntity { .. } => return Ok(()),
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
        GmAction::SetStationPuppet { active: false, .. } => Ok(()),
        GmAction::SetStationPuppet { active: true, .. } => {
            let backfill = ratings
                .and_then(|ratings| ratings.0.get(station))
                .is_some_and(|rating| rating == crate::ship::rating::BACKFILL_RATING);
            backfill
                .then_some(())
                .ok_or(GmActionRefusalReason::StationNotBackfill)
        }
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
        | GmAction::SpawnPaletteEntity { .. } => unreachable!(),
    }
}

pub fn validate_station_action_in_world(
    world: &mut World,
    action: &crate::gm_action::GmAction,
    operator_id: &str,
) -> Result<(), crate::gm_action::GmActionRefusalReason> {
    let puppets = world
        .get_resource::<StationPuppets>()
        .cloned()
        .unwrap_or_default();
    let Some(ship) = action.ship_key() else {
        return Ok(());
    };
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
mod tests {
    use super::*;
    use crate::ship::control_source::ControlSource;

    fn target(ship: &str, station: &str) -> StationPuppetTarget {
        StationPuppetTarget::new(ShipKey(ship.into()), StationId(station.into()))
    }

    #[test]
    fn takeover_is_station_scoped_and_an_exact_retry_is_a_no_op() {
        let mut puppets = StationPuppets::default();
        let helm = target("player-1", "helm");
        let tactical = target("player-1", "tactical");

        assert!(puppets.set_operator(helm.clone(), "gm-2".into(), true));
        assert!(!puppets.set_operator(helm.clone(), "gm-2".into(), true));
        assert!(puppets.is_operated_by(&helm, "gm-2"));
        assert!(!puppets.is_active(&tactical));
    }

    #[test]
    fn equal_gms_share_a_station_and_release_only_their_own_membership() {
        let mut puppets = StationPuppets::default();
        let helm = target("player-1", "helm");

        assert!(puppets.set_operator(helm.clone(), "gm-2".into(), true));
        assert!(puppets.set_operator(helm.clone(), "gm-1".into(), true));
        assert_eq!(puppets.operators(&helm), &["gm-1", "gm-2"]);

        assert!(puppets.set_operator(helm.clone(), "gm-1".into(), false));
        assert!(puppets.is_active(&helm));
        assert_eq!(puppets.operators(&helm), &["gm-2"]);
        assert!(!puppets.set_operator(helm.clone(), "gm-1".into(), false));

        assert!(puppets.set_operator(helm.clone(), "gm-2".into(), false));
        assert!(!puppets.is_active(&helm));
        assert!(puppets.entries().is_empty());
    }

    #[test]
    fn rows_and_operator_membership_are_canonical_not_arrival_ordered() {
        let actions = [
            (target("player-2", "tactical"), "gm-3"),
            (target("player-1", "helm"), "gm-2"),
            (target("player-1", "helm"), "gm-1"),
        ];
        let mut forward = StationPuppets::default();
        let mut reverse = StationPuppets::default();
        for (target, operator) in actions.iter() {
            forward.set_operator(target.clone(), (*operator).into(), true);
        }
        for (target, operator) in actions.iter().rev() {
            reverse.set_operator(target.clone(), (*operator).into(), true);
        }
        assert_eq!(forward, reverse);
    }

    fn control_test_config() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
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

[[station]]
id = "tactical"
name = "Tactical"
description = ""
rank = ""
console = "weapons.html"

[[station.rating]]
name = "Manual"
automated_systems = []

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"

[[system]]
id = "impulse-drive"
kind = "helm_impulse"
station = "helm"

[[system]]
id = "phaser"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "main-view"
kind = "viewscreen"
station = "tactical"

[[system]]
id = "helm-radar"
kind = "helm_radar"
station = "helm"

[[system]]
id = "science-sensors"
kind = "sensors"
station = "tactical"
"#,
            &[
                "helm_thrust",
                "helm_impulse",
                "phaser_bank",
                "viewscreen",
                "helm_radar",
                "sensors",
            ],
        )
        .unwrap()
    }

    #[test]
    fn takeover_suppresses_only_the_selected_station_and_release_restores_backfill() {
        let config = control_test_config();
        let helm = StationId("helm".into());
        let tactical = StationId("tactical".into());
        let mut ratings = ActiveStationRatings::default();
        ratings
            .0
            .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
        ratings.0.insert(
            tactical.clone(),
            crate::ship::rating::BACKFILL_RATING.into(),
        );
        let mut sources = ShipSystemControlSources::default();
        crate::ship::rating::apply_rating(
            &config,
            &helm,
            crate::ship::rating::BACKFILL_RATING,
            &mut sources.0,
        );
        crate::ship::rating::apply_rating(
            &config,
            &tactical,
            crate::ship::rating::BACKFILL_RATING,
            &mut sources.0,
        );

        let ship = ShipKey("player-1".into());
        let mut app = App::new();
        app.init_resource::<StationPuppets>()
            .init_resource::<PreviousStationPuppetTargets>()
            .add_systems(Update, reconcile_station_puppet_control);
        app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid(ship.0.clone()),
            ShipConfigComponent(config),
            sources,
            ratings,
        ));

        let target = StationPuppetTarget::new(ship, helm.clone());
        app.world_mut()
            .resource_mut::<StationPuppets>()
            .set_operator(target.clone(), "gm-1".into(), true);
        app.update();

        let sources = app
            .world_mut()
            .query::<&ShipSystemControlSources>()
            .single(app.world())
            .unwrap();
        assert_eq!(
            sources
                .0
                .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
            ControlSource::Human,
        );
        assert_eq!(
            sources
                .0
                .source_for(&crate::core::messages::SystemId("phaser".into())),
            ControlSource::Ai,
            "a different Backfill Station keeps operating AI",
        );

        app.world_mut()
            .resource_mut::<StationPuppets>()
            .set_operator(target, "gm-1".into(), false);
        app.update();
        let sources = app
            .world_mut()
            .query::<&ShipSystemControlSources>()
            .single(app.world())
            .unwrap();
        assert_eq!(
            sources
                .0
                .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
            ControlSource::Ai,
            "release restores the Station's ordinary Backfill rating",
        );
    }

    #[test]
    fn gm_commands_follow_human_admission_in_canonical_order_and_keep_attribution_sidecar_only() {
        use crate::core::messages::{
            AdmittedCommand, AdmittedCommands, SystemControlPayload, SystemId,
        };

        let config = control_test_config();
        let helm = StationId("helm".into());
        let helm_target = SystemId("helm-thrust".into());
        let mut sources = ShipSystemControlSources::default();
        sources.0.set(helm_target.clone(), ControlSource::Human);
        let mut app = App::new();
        app.insert_resource(crate::sim_tick::SimTick(12))
            .init_resource::<PendingGmStationCommands>()
            .init_resource::<PendingGmStationFeedbackRoutes>()
            .init_resource::<StationPuppetActivity>()
            .add_systems(Update, admit_station_puppet_commands);
        app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid("player-1".into()),
            ShipConfigComponent(config),
            sources,
            AdmittedCommands(vec![AdmittedCommand {
                target: helm_target.clone(),
                payload: SystemControlPayload::SetThrust { value: 0.1 },
                response_token: Some("crew-route-only".into()),
                feedback_correlation: None,
            }]),
        ));

        for (sequence, operator, value) in [(3, "gm-2", 0.3), (2, "gm-1", 0.2)] {
            app.world_mut()
                .resource_mut::<PendingGmStationCommands>()
                .push(PendingGmStationCommand {
                    tick: 12,
                    order: crate::gm_action::GmActionOrder::new(
                        crate::command_admission::HostSlot(sequence as u32),
                        sequence,
                    ),
                    operator_id: operator.into(),
                    correlation: crate::gm_action::GmActionId::new(format!("cmd-{sequence}"))
                        .unwrap(),
                    ship: ShipKey("player-1".into()),
                    station: helm.clone(),
                    target: helm_target.clone(),
                    payload: SystemControlPayload::SetThrust { value },
                });
        }
        app.update();

        let admitted = app
            .world_mut()
            .query::<&AdmittedCommands>()
            .single(app.world())
            .unwrap();
        let values: Vec<_> = admitted
            .0
            .iter()
            .map(|command| match &command.payload {
                SystemControlPayload::SetThrust { value } => *value,
                _ => panic!("unexpected payload"),
            })
            .collect();
        assert_eq!(values, vec![0.1, 0.2, 0.3]);
        assert_eq!(
            admitted.0[0].response_token.as_deref(),
            Some("crew-route-only")
        );
        assert!(admitted.0[1..]
            .iter()
            .all(|command| command.response_token.is_none()));
        assert_eq!(
            app.world()
                .resource::<StationPuppetActivity>()
                .entries()
                .iter()
                .map(|entry| entry.operator_id.as_str())
                .collect::<Vec<_>>(),
            vec!["gm-1", "gm-2"],
        );
    }

    #[test]
    fn authentic_helm_consumer_settles_gm_correlation_applied_or_refused_exactly_once() {
        use crate::core::messages::{AdmittedCommands, SystemControlPayload, SystemId};
        use crate::gm_action::{
            GmAction, GmActionGrant, GmActionJournal, GmActionKind, GmActionLog, GmActionOutcome,
            GmActionRefusalReason, LoggedGmAction,
        };

        for (with_impulse_owner, expected_outcome, expected_reason) in [
            (true, GmActionOutcome::Applied, None),
            (
                false,
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::SystemRefused),
            ),
        ] {
            let order =
                crate::gm_action::GmActionOrder::new(crate::command_admission::HostSlot(2), 1);
            let correlation = crate::gm_action::GmActionId::new(if with_impulse_owner {
                "iframe-impulse-applied"
            } else {
                "iframe-impulse-refused"
            })
            .unwrap();
            let payload = SystemControlPayload::StartImpulseCharge;
            let grant = GmActionGrant {
                from: order.origin,
                sequenced_by: crate::command_admission::HostSlot(1),
                operator_id: "gm-2".into(),
                correlation: correlation.clone(),
                recovery_generation: 0,
                apply_tick: 12,
                order,
                action: GmAction::IssueStationCommand {
                    ship: ShipKey("player-1".into()),
                    station: StationId("helm".into()),
                    target: SystemId("impulse-drive".into()),
                    payload: crate::core::codec::canonical_system_command(&payload).unwrap(),
                },
            };
            let mut journal = GmActionJournal::default();
            journal.insert(grant).unwrap();
            journal
                .record_applied_result(LoggedGmAction {
                    operator_id: "gm-2".into(),
                    correlation: correlation.clone(),
                    action_kind: GmActionKind::StationCommand,
                    requested_active: true,
                    outcome: GmActionOutcome::Pending,
                    tick: 12,
                    reason: None,
                    order: Some(order),
                    target: None,
                    effect: None,
                })
                .unwrap();
            let provisional_log = journal.applied_log();

            let mut app = App::new();
            app.add_message::<crate::lobby::server::OutboundMessage>()
                .insert_resource(crate::sim_tick::SimTick(12))
                .insert_resource(journal)
                .insert_resource(provisional_log)
                .init_resource::<PendingGmStationCommands>()
                .init_resource::<PendingGmStationFeedbackRoutes>()
                .init_resource::<StationPuppetActivity>()
                .add_systems(
                    Update,
                    (
                        admit_station_puppet_commands,
                        crate::ship::helm_admission::process_helm_inputs,
                        settle_station_puppet_feedback,
                    )
                        .chain(),
                );
            let entity = app
                .world_mut()
                .spawn((
                    crate::server_app::Ship,
                    EntityUuid("player-1".into()),
                    ShipConfigComponent(control_test_config()),
                    AdmittedCommands::default(),
                ))
                .id();
            if with_impulse_owner {
                app.world_mut()
                    .entity_mut(entity)
                    .insert(crate::ship::helm::ImpulseCommand::default());
            }
            app.world_mut()
                .resource_mut::<PendingGmStationCommands>()
                .push(PendingGmStationCommand {
                    tick: 12,
                    order,
                    operator_id: "gm-2".into(),
                    correlation: correlation.clone(),
                    ship: ShipKey("player-1".into()),
                    station: StationId("helm".into()),
                    target: SystemId("impulse-drive".into()),
                    payload,
                });

            app.update();

            let result = &app.world().resource::<GmActionJournal>().applied_results()[0];
            assert_eq!(result.outcome, expected_outcome);
            assert_eq!(result.reason, expected_reason);
            assert_eq!(
                app.world().resource::<GmActionLog>().entries()[0],
                result.clone(),
            );
            assert_eq!(
                app.world()
                    .resource::<PendingGmStationFeedbackRoutes>()
                    .len(),
                0,
                "the authentic consumer's first terminal answer closes its route",
            );
            let admitted = app.world().get::<AdmittedCommands>(entity).unwrap();
            assert_eq!(
                admitted.0[0]
                    .feedback_correlation
                    .as_ref()
                    .map(|value| value.as_str()),
                Some(correlation.as_str()),
            );
            assert!(admitted.0[0]
                .response_token
                .as_deref()
                .is_some_and(|token| token.starts_with(GM_STATION_FEEDBACK_TOKEN_PREFIX)));

            // The admitted buffer intentionally remains populated in this
            // narrow fixture, so its authentic consumer emits the same reply a
            // second time.  With no route left, that duplicate is inert.
            let terminal = result.clone();
            app.update();
            assert_eq!(
                app.world().resource::<GmActionJournal>().applied_results()[0],
                terminal,
            );
            assert_eq!(
                app.world()
                    .resource::<PendingGmStationFeedbackRoutes>()
                    .len(),
                0,
            );
        }
    }

    #[test]
    fn helm_station_admission_uses_the_set_view_effective_target_and_refuses_a_forged_station() {
        use crate::command_admission::{validate_station_command, StationCommandPolicyFailure};
        use crate::core::messages::{SystemControlPayload, SystemId, ViewMode};

        let config = control_test_config();
        let mut sources = ShipSystemControlSources::default();
        // Authentic takeover begins while Helm remains Backfill. The Station
        // authority substitution accepts AI-rated Systems but never Offline.
        sources
            .0
            .set(SystemId("helm-radar".into()), ControlSource::Ai);
        sources
            .0
            .set(SystemId("science-sensors".into()), ControlSource::Ai);
        let viewscreen = SystemId("main-view".into());
        let helm = StationId("helm".into());

        let admitted = validate_station_command(
            &helm,
            viewscreen.clone(),
            SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
            &sources,
            &config,
            None,
        )
        .expect("Radar is authored from Helm's effective target");
        assert_eq!(admitted.target, viewscreen);
        assert!(admitted.response_token.is_none());

        assert_eq!(
            validate_station_command(
                &helm,
                SystemId("main-view".into()),
                SystemControlPayload::SetView {
                    mode: ViewMode::ScienceRadar,
                },
                &sources,
                &config,
                None,
            ),
            Err(StationCommandPolicyFailure::SystemOutsideStation),
            "a Helm puppet cannot forge a view whose effective System belongs to Tactical",
        );

        sources
            .0
            .set(SystemId("helm-radar".into()), ControlSource::Offline);
        assert_eq!(
            validate_station_command(
                &helm,
                SystemId("main-view".into()),
                SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
                &sources,
                &config,
                None,
            ),
            Err(StationCommandPolicyFailure::SystemUnavailable),
        );
    }
}
