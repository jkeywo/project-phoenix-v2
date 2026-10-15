//! Read-only aliases for outbound copies. Internal actuator references stay real.
use crate::core::messages::{ConsoleId, SystemBlackboard, SystemId};
use crate::science::broad::{BroadProfile, BroadTracks};
use crate::ship::components::{ConsoleHosts, ShipConfigComponent};
use crate::ship::system_registry::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone)]
pub(super) struct KnownContact {
    pub id: String,
    pub name: Option<String>,
    source: String,
    kind: String,
    owner: SystemId,
}

#[derive(Default)]
pub(super) struct ReferenceProjection {
    measured: bool,
    contexts: BTreeMap<ConsoleId, Vec<KnownContact>>,
    publishers: BTreeMap<SystemId, ConsoleId>,
    receipt: BTreeMap<String, BTreeSet<SystemId>>,
    tactical: Option<SystemId>,
    bound_combat: Option<SystemId>,
    concealed: BTreeSet<String>,
}

impl ReferenceProjection {
    pub fn capture(
        config: Option<&ShipConfigComponent>,
        boards: Option<&crate::server_app::ShipSystemBlackboards>,
        hosts: Option<&ConsoleHosts>,
        profiles: Option<&BroadProfile>,
        tracks: Option<&BroadTracks>,
        controls: Option<&crate::science::broad::SensingControls>,
        sessions: Option<&crate::lobby::Sessions>,
        concealed: &BTreeSet<String>,
    ) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        let config = &config.0;
        let empty = HashMap::new();
        let boards = boards.map_or(&empty, |boards| &boards.0);
        let mut result = Self::default();
        result.measured = tracks.is_some() || controls.is_some();
        result.concealed = concealed.clone();
        if let (Some(sessions), Some(hosts)) = (sessions, hosts) {
            for player in sessions
                .0
                .players()
                .iter()
                .filter(|player| player.connected)
            {
                result.receipt.insert(
                    player.token.clone(),
                    crate::ship::console_publication::permitted_publications(
                        config,
                        hosts,
                        player.station.as_ref(),
                        boards,
                    )
                    .into_keys()
                    .collect(),
                );
            }
        }
        for system in &config.systems {
            let Some(console) = config.console_for_system(&system.id) else {
                continue;
            };
            result
                .publishers
                .insert(system.id.clone(), console.id.clone());
            if result.contexts.contains_key(&console.id) {
                continue;
            }
            let view = config.observations_for_system(&system.id, boards);
            let mut known = Vec::new();
            if let (Some(profiles), Some(tracks)) = (profiles, tracks) {
                for (owner, instrument) in &profiles.0 {
                    let Some(owner_config) = config.system(owner) else {
                        continue;
                    };
                    // Every fitted instrument publishes under its actual
                    // owner. The legacy Helm aggregate contains motion only.
                    let source = owner.clone();
                    let public = match (owner_config.kind.as_str(), view.get(&source)) {
                        (HELM_RADAR_KIND, Some(SystemBlackboard::Helm(board))) => {
                            board.tracks.as_deref()
                        }
                        (TACTICAL_RADAR_KIND, Some(SystemBlackboard::TacticalRadar(board))) => {
                            board.tracks.as_deref()
                        }
                        (SENSOR_RADAR_KIND, Some(SystemBlackboard::SensorRadar(board))) => {
                            board.tracks.as_deref()
                        }
                        _ => None,
                    };
                    let Some(public) = public.filter(|_| instrument.profile.system == *owner)
                    else {
                        continue;
                    };
                    let Some(state) = tracks.0.get(owner) else {
                        continue;
                    };
                    for (source, id) in &state.current {
                        if concealed.contains(source) {
                            continue;
                        }
                        let Some(record) = public
                            .iter()
                            .find(|record| &record.contact_id == id && !record.predicted)
                        else {
                            continue;
                        };
                        known.push(KnownContact {
                            id: id.clone(),
                            source: source.clone(),
                            kind: owner_config.kind.clone(),
                            owner: owner.clone(),
                            name: record
                                .identity
                                .as_ref()
                                .map(|identity| identity.name.clone()),
                        });
                    }
                }
            }
            result.contexts.insert(console.id.clone(), known);
        }
        // These are existing aggregate publication aliases, never new Systems.
        result.bound_combat = controls
            .and_then(|controls| crate::science::broad::combat_owner(Some(controls), true))
            .filter(|owner| {
                config
                    .system(owner)
                    .is_some_and(|system| system.kind == TACTICAL_RADAR_KIND)
            });
        for (publication, kind) in [
            (helm_station_key(), HELM_RADAR_KIND),
            (tactical_station_key(), TACTICAL_RADAR_KIND),
        ] {
            let mut owners = config.systems.iter().filter(|system| system.kind == kind);
            let unique = owners.next().filter(|_| owners.next().is_none());
            let owner = if kind == TACTICAL_RADAR_KIND && controls.is_some() {
                result
                    .bound_combat
                    .as_ref()
                    .and_then(|owner| config.system(owner))
            } else {
                unique
            };
            if let Some(system) = owner {
                if kind == TACTICAL_RADAR_KIND {
                    result.tactical = Some(system.id.clone());
                }
                if let Some(console) = config.console_for_system(&system.id) {
                    result.publishers.insert(publication, console.id.clone());
                }
            } else {
                result.publishers.remove(&publication);
            }
        }
        result
    }

    pub fn permits(&self, token: &str, publication: &SystemId) -> bool {
        self.receipt
            .get(token)
            .is_some_and(|receipt| receipt.contains(publication))
    }

    fn resolve(
        &self,
        publication: &SystemId,
        reference: Option<&str>,
        kind: Option<&str>,
    ) -> Option<KnownContact> {
        let reference = reference?;
        let console = self.publishers.get(publication)?;
        let matches: Vec<_> = self
            .contexts
            .get(console)?
            .iter()
            .filter(|contact| {
                (contact.source == reference || contact.id == reference)
                    && kind.is_none_or(|kind| contact.kind == kind)
            })
            .collect();
        // A retained explicit contact handle keeps its own measured provenance.
        // For private machinery references, prefer that actuator's own record;
        // never choose an unrelated feed merely because its key sorts first.
        let preferred = if self
            .contexts
            .get(console)?
            .iter()
            .any(|contact| &contact.owner == publication)
        {
            Some(publication)
        } else if kind == Some(TACTICAL_RADAR_KIND) || publication == &tactical_station_key() {
            self.tactical.as_ref()
        } else if kind.is_none() {
            self.bound_combat.as_ref()
        } else {
            None
        };
        if let Some(found) = matches.iter().find(|contact| contact.id == reference) {
            return Some((**found).clone());
        }
        if let Some(found) =
            preferred.and_then(|owner| matches.iter().find(|contact| &contact.owner == owner))
        {
            return Some((**found).clone());
        }
        // Several independent observations are not interchangeable command
        // identities. Without one unambiguous producing owner, disclose none.
        (matches.len() == 1).then(|| (*matches[0]).clone())
    }

    pub fn tactical(&self, token: &str, reference: Option<&str>) -> Option<KnownContact> {
        if !self.permits(token, &tactical_station_key())
            && !self
                .tactical
                .as_ref()
                .is_some_and(|owner| self.permits(token, owner))
        {
            return None;
        }
        self.resolve(
            &tactical_station_key(),
            reference,
            Some(TACTICAL_RADAR_KIND),
        )
    }

    pub fn board(&self, source: &SystemId, board: &mut SystemBlackboard) {
        let alias = |reference: Option<&str>, kind: Option<&str>| {
            self.resolve(source, reference, kind)
                .map(|contact| contact.id)
        };
        match board {
            SystemBlackboard::Viewscreen(board) => {
                board.combat_lock = alias(board.combat_lock.as_deref(), Some(TACTICAL_RADAR_KIND));
                board.science_target =
                    alias(board.science_target.as_deref(), Some(SENSOR_RADAR_KIND));
                board.last_attacker_uuid = alias(board.last_attacker_uuid.as_deref(), None);
            }
            SystemBlackboard::Weapons(board) => {
                let target = self.resolve(
                    source,
                    board.target_uuid.as_deref(),
                    Some(TACTICAL_RADAR_KIND),
                );
                board.target_uuid = target.as_ref().map(|contact| contact.id.clone());
                board.target_name = target.and_then(|contact| contact.name);
                board.locked_target =
                    alias(board.locked_target.as_deref(), Some(TACTICAL_RADAR_KIND));
            }
            SystemBlackboard::Sensors(board) => {
                board.science_target_uuid = alias(
                    board.science_target_uuid.as_deref(),
                    Some(SENSOR_RADAR_KIND),
                );
                // Fitted instruments already carry these reports through the
                // ordinary track DTO. A second decoy-specific list would reveal
                // that the evidence was synthetic. The host retains originals.
                if self.measured {
                    board.contact_ghosts.clear();
                }
                // Legacy unconfigured hosts retain explicit reported points.
                board
                    .contact_ghosts
                    .retain(|ghost| !self.concealed.contains(&ghost.uuid));
                let ghosts: BTreeSet<_> = board
                    .contact_ghosts
                    .iter()
                    .map(|ghost| ghost.uuid.clone())
                    .collect();
                let key = |reference: &str| {
                    alias(Some(reference), Some(SENSOR_RADAR_KIND))
                        .or_else(|| ghosts.contains(reference).then(|| reference.to_owned()))
                };
                board.contact_overrides = std::mem::take(&mut board.contact_overrides)
                    .into_iter()
                    .filter_map(|(source, value)| key(&source).map(|id| (id, value)))
                    .collect();
                board.contact_classifications = std::mem::take(&mut board.contact_classifications)
                    .into_iter()
                    .filter_map(|(source, value)| key(&source).map(|id| (id, value)))
                    .collect();
                board.contact_reports = std::mem::take(&mut board.contact_reports)
                    .into_iter()
                    .filter_map(|(source, mut value)| {
                        key(&source).map(|id| {
                            if let Some(report) = &mut value {
                                if let Some(observation) = &mut report.observation {
                                    observation.subject =
                                        phoenix_model::observation::ObservationSubject::Entity {
                                            uuid: id.clone(),
                                        };
                                }
                            }
                            (id, value)
                        })
                    })
                    .collect();
            }
            SystemBlackboard::SensorRadar(board) => {
                board.selected_target =
                    alias(board.selected_target.as_deref(), Some(SENSOR_RADAR_KIND));
                if board.selected_target.is_none() {
                    board.selected_target_alert = None;
                    board.selected_target_relative_velocity = None;
                    board.selected_target_weapons_cold = None;
                }
            }
            SystemBlackboard::TacticalRadar(board) => {
                board.selected_target =
                    alias(board.selected_target.as_deref(), Some(TACTICAL_RADAR_KIND));
                // Actual broad records have their own explicitly measured DTO.
                board.blips.clear();
                board.regions.clear();
            }
            SystemBlackboard::Helm(board) => board.hostile_weapon_arcs.clear(),
            _ => {}
        }
    }
}
