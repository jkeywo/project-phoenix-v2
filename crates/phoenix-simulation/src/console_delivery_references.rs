//! Read-only aliases for outbound copies. Internal actuator references stay real.
use crate::core::messages::{ConsoleId, SystemBlackboard, SystemId};
use crate::science::broad::{BroadProfile, BroadTracks};
use crate::ship::components::{ConsoleHosts, ShipConfigComponent};
use crate::ship::system_registry::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

// The same privacy transforms serve owned fixtures and immutable outgoing
// values. Mutation is requested only after the projected fields differ.
pub(super) trait ProjectionBoard {
    fn value(&self) -> &SystemBlackboard;
    fn edit(&mut self) -> &mut SystemBlackboard;
}
impl ProjectionBoard for SystemBlackboard {
    fn value(&self) -> &SystemBlackboard {
        self
    }
    fn edit(&mut self) -> &mut SystemBlackboard {
        self
    }
}
impl ProjectionBoard for std::sync::Arc<SystemBlackboard> {
    fn value(&self) -> &SystemBlackboard {
        self.as_ref()
    }
    fn edit(&mut self) -> &mut SystemBlackboard {
        std::sync::Arc::make_mut(self)
    }
}

macro_rules! replace_projected_fields {
    ($target:ident, $variant:ident, $( $field:ident = $value:expr ),+ $(,)?) => {{
        let ($( $field, )+) = ($( $value, )+);
        let SystemBlackboard::$variant(current) = $target.value() else { unreachable!() };
        if $( current.$field != $field )||+ {
            let SystemBlackboard::$variant(current) = $target.edit() else { unreachable!() };
            $( current.$field = $field; )+
        }
    }};
}

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
    weapon_members: BTreeMap<ConsoleId, BTreeSet<SystemId>>,
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
            result.weapon_members.insert(
                console.id.clone(),
                config
                    .systems
                    .iter()
                    .filter(|member| {
                        matches!(
                            member.kind.as_str(),
                            PHASER_BANK_KIND | BLASTER_BANK_KIND | TORPEDO_TUBE_KIND
                        ) && (member.console.as_ref() == Some(&console.id)
                            || boards.get(&member.id).is_some_and(|board| {
                                console
                                    .feeds
                                    .iter()
                                    .any(|feed| feed.accepts(&member.id, board))
                            }))
                    })
                    .map(|member| member.id.clone())
                    .collect(),
            );
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
                    for (source, id) in state.current_routes() {
                        if concealed.contains(source) {
                            continue;
                        }
                        let Some(record) = public
                            .iter()
                            .find(|record| record.contact_id == id && !record.predicted)
                        else {
                            continue;
                        };
                        known.push(KnownContact {
                            id: id.to_owned(),
                            source: source.to_owned(),
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

    pub fn board(&self, source: &SystemId, board: &mut impl ProjectionBoard) {
        let alias = |reference: Option<&str>, kind: Option<&str>| {
            self.resolve(source, reference, kind)
                .map(|contact| contact.id)
        };
        match board.value() {
            SystemBlackboard::Viewscreen(current) => {
                replace_projected_fields!(
                    board,
                    Viewscreen,
                    combat_lock = alias(current.combat_lock.as_deref(), Some(TACTICAL_RADAR_KIND)),
                    science_target =
                        alias(current.science_target.as_deref(), Some(SENSOR_RADAR_KIND)),
                    last_attacker_uuid = alias(current.last_attacker_uuid.as_deref(), None),
                );
            }
            SystemBlackboard::Weapons(current) => {
                use phoenix_model::weapon_targets_view::WeaponTargetReference;
                let members = self
                    .publishers
                    .get(source)
                    .and_then(|console| self.weapon_members.get(console));
                let mut assignments = current.assignments.clone();
                assignments
                    .retain(|owner, _| members.is_some_and(|members| members.contains(owner)));
                for assignment in assignments.values_mut() {
                    let project = |target: &mut WeaponTargetReference| {
                        if let WeaponTargetReference::Target { contact } = target {
                            // Never retarget a private/stale handle implicitly.
                            *target = self
                                .resolve(source, Some(contact), None)
                                .filter(|known| known.id == *contact)
                                .map_or(WeaponTargetReference::Unavailable, |known| {
                                    WeaponTargetReference::Target { contact: known.id }
                                });
                        }
                    };
                    if let Some(group) = &mut assignment.group {
                        project(&mut group.target);
                    }
                    if let Some(automatic) = &mut assignment.automatic {
                        project(automatic);
                    }
                    if let Some(individual) = &mut assignment.individual {
                        project(individual);
                    }
                }
                let target = self.resolve(
                    source,
                    current.target_uuid.as_deref(),
                    Some(TACTICAL_RADAR_KIND),
                );
                replace_projected_fields!(
                    board,
                    Weapons,
                    assignments = assignments,
                    target_uuid = target.as_ref().map(|contact| contact.id.clone()),
                    target_name = target.and_then(|contact| contact.name),
                    locked_target =
                        alias(current.locked_target.as_deref(), Some(TACTICAL_RADAR_KIND)),
                );
            }
            SystemBlackboard::Dock(current) => {
                let available = self.resolve(source, current.available_target.as_deref(), None);
                let coupled = self.resolve(source, current.docked_to.as_deref(), None);
                replace_projected_fields!(
                    board,
                    Dock,
                    available = current.available && available.is_some(),
                    available_target = available.as_ref().map(|contact| contact.id.clone()),
                    available_target_name = available.and_then(|contact| contact.name),
                    docked_to = coupled.as_ref().map(|contact| contact.id.clone()),
                    docked_to_name = coupled.and_then(|contact| contact.name),
                );
            }
            SystemBlackboard::Tractor(current) => {
                let coupled = self.resolve(source, current.coupled_target.as_deref(), None);
                replace_projected_fields!(
                    board,
                    Tractor,
                    coupled_target = coupled.as_ref().map(|contact| contact.id.clone()),
                    coupled_target_name = coupled.and_then(|contact| contact.name),
                );
            }
            SystemBlackboard::Transporter(current) => {
                let target = self.resolve(source, current.selected_contact.as_deref(), None);
                // A General structure reading never grants live personnel counts.
                replace_projected_fields!(
                    board,
                    Transporter,
                    discovered = false,
                    civilians_remaining = 0,
                    selected_contact = target.as_ref().map(|contact| contact.id.clone()),
                    selected_contact_name = target.and_then(|contact| contact.name),
                );
            }
            SystemBlackboard::Sensors(current) => {
                // Fitted instruments already carry reports in ordinary tracks.
                let contact_ghosts: Vec<_> = current
                    .contact_ghosts
                    .iter()
                    .filter(|ghost| !self.measured && !self.concealed.contains(&ghost.uuid))
                    .cloned()
                    .collect();
                let ghosts: BTreeSet<_> = contact_ghosts
                    .iter()
                    .map(|ghost| ghost.uuid.clone())
                    .collect();
                let key = |reference: &str| {
                    alias(Some(reference), Some(SENSOR_RADAR_KIND))
                        .or_else(|| ghosts.contains(reference).then(|| reference.to_owned()))
                };
                let contact_overrides = current
                    .contact_overrides
                    .iter()
                    .filter_map(|(source, value)| key(source).map(|id| (id, *value)))
                    .collect();
                let contact_classifications = current
                    .contact_classifications
                    .iter()
                    .filter_map(|(source, value)| key(source).map(|id| (id, value.clone())))
                    .collect();
                let contact_reports = current
                    .contact_reports
                    .iter()
                    .filter_map(|(source, value)| {
                        key(source).map(|id| {
                            let mut value = value.clone();
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
                replace_projected_fields!(
                    board,
                    Sensors,
                    science_target_uuid = alias(
                        current.science_target_uuid.as_deref(),
                        Some(SENSOR_RADAR_KIND)
                    ),
                    contact_ghosts = contact_ghosts,
                    contact_overrides = contact_overrides,
                    contact_classifications = contact_classifications,
                    contact_reports = contact_reports,
                );
            }
            SystemBlackboard::SensorRadar(current) => {
                let selected_target =
                    alias(current.selected_target.as_deref(), Some(SENSOR_RADAR_KIND));
                let known = selected_target.is_some();
                replace_projected_fields!(
                    board,
                    SensorRadar,
                    selected_target = selected_target,
                    selected_target_alert = if known {
                        current.selected_target_alert
                    } else {
                        None
                    },
                    selected_target_relative_velocity = if known {
                        current.selected_target_relative_velocity
                    } else {
                        None
                    },
                    selected_target_weapons_cold = if known {
                        current.selected_target_weapons_cold
                    } else {
                        None
                    },
                );
            }
            SystemBlackboard::TacticalRadar(current) => {
                replace_projected_fields!(
                    board,
                    TacticalRadar,
                    selected_target = alias(
                        current.selected_target.as_deref(),
                        Some(TACTICAL_RADAR_KIND)
                    ),
                    blips = Vec::new(),
                    regions = Vec::new(),
                );
            }
            SystemBlackboard::Helm(_) => {
                replace_projected_fields!(board, Helm, hostile_weapon_arcs = Vec::new());
            }
            _ => {}
        }
    }

    /// Navigation can retain an explicitly charted public landmark, but a
    /// private moving anchor must use this publisher's own measured handle.
    pub fn navigation(
        &self,
        source: &SystemId,
        board: &mut impl ProjectionBoard,
        charted: impl Fn(&str) -> bool,
    ) {
        if !self.measured {
            return;
        }
        let SystemBlackboard::Navigation(current) = board.value() else {
            return;
        };
        let reference = |id: &str| {
            self.resolve(source, Some(id), None)
                .or_else(|| {
                    let console = self.publishers.get(source)?;
                    let mut known = self
                        .contexts
                        .get(console)?
                        .iter()
                        .filter(|contact| contact.name.as_deref() == Some(id));
                    let first = known.next()?;
                    known.next().is_none().then(|| first.clone())
                })
                .map(|contact| contact.id)
                .or_else(|| charted(id).then(|| id.to_owned()))
        };
        let mut navigation_waypoint = current.navigation_waypoint.clone();
        if let Some(waypoint) = &mut navigation_waypoint {
            waypoint.source_uuid = waypoint.source_uuid.as_deref().and_then(reference);
        }
        let mut civilians = current.civilians.clone();
        civilians.retain_mut(|row| {
            let Some(contact) = self.resolve(source, Some(&row.uuid), None) else {
                return false;
            };
            let Some(name) = contact.name else {
                return false;
            };
            row.uuid = contact.id;
            row.name = name;
            if row.order == "dock" {
                row.order_destination = reference(&row.order_destination).unwrap_or_default();
            }
            row.order_options.retain_mut(|option| {
                if let crate::civilian::CivilianOrder::Dock { structure } = &mut option.order {
                    let Some(id) = reference(structure) else {
                        return false;
                    };
                    *structure = id;
                }
                true
            });
            true
        });
        replace_projected_fields!(
            board,
            Navigation,
            navigation_waypoint = navigation_waypoint,
            civilians = civilians,
        );
    }
}
