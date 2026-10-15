//! Ephemeral routing from this tool Console's published contact picture.
//! Actual associations are execution inputs; only measured fields leave here.
use crate::core::messages::{SystemBlackboard, SystemId};
use crate::science::broad::{BroadTracks, SensingControls};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone)]
pub(super) struct Contact {
    pub source: String,
    pub handle: String,
    pub name: Option<String>,
    pub position: Option<[f32; 2]>,
}

pub(super) struct Picture {
    pub fitted: bool,
    pub contacts: BTreeMap<String, Contact>,
}

impl Picture {
    pub fn capture(
        config: &crate::ship::config::ShipConfig,
        owner: &SystemId,
        boards: &HashMap<SystemId, SystemBlackboard>,
        tracks: Option<&BroadTracks>,
        controls: Option<&SensingControls>,
        observer: &str,
        runtime: Option<&crate::world::server::WorldContentRuntime>,
    ) -> Self {
        let mut result = Self {
            fitted: tracks.is_some() || controls.is_some(),
            contacts: BTreeMap::new(),
        };
        let concealed = |source: &str| {
            runtime.is_some_and(|runtime| {
                crate::gm::operations::contact::mode(&runtime.contact_overrides, observer, source)
                    == crate::gm::operations::contact::ContactMode::Conceal
            })
        };
        if !result.fitted {
            for source in super::known_contacts(config, owner, boards, None)
                .into_iter()
                .filter(|source| !concealed(source))
            {
                result.contacts.insert(
                    source.clone(),
                    Contact {
                        source: source.clone(),
                        handle: source,
                        name: None,
                        position: None,
                    },
                );
            }
            return result;
        }
        let Some(tracks) = tracks else { return result };
        let permitted = config.observations_for_system(owner, boards);
        for (instrument, state) in &tracks.0 {
            let source = crate::science::contact_routing::publication_source(config, instrument);
            let Some(board) = permitted.get(&source) else {
                continue;
            };
            // Default Science handoff still requires its selected contact.
            // Only an explicitly supplied Tactical picture enumerates returns.
            let selected = match board {
                SystemBlackboard::SensorRadar(board) => Some(board.selected_target.as_deref()),
                SystemBlackboard::TacticalRadar(_) => None,
                _ => continue,
            };
            let Some(records) = crate::science::contact_routing::observations(board) else {
                continue;
            };
            for record in records
                .iter()
                .filter(|record| record.selectable && !record.predicted)
            {
                let Some(actual) = state.resolve_current(&record.contact_id) else {
                    continue;
                };
                if concealed(actual) {
                    continue;
                }
                if selected.is_some_and(|selected| {
                    selected != Some(record.contact_id.as_str()) && selected != Some(actual)
                }) {
                    continue;
                }
                if record.position.iter().any(|value| !value.is_finite()) {
                    continue;
                }
                result.contacts.insert(
                    record.contact_id.clone(),
                    Contact {
                        source: actual.into(),
                        handle: record.contact_id.clone(),
                        name: record
                            .identity
                            .as_ref()
                            .map(|identity| identity.name.clone()),
                        position: Some(record.position),
                    },
                );
            }
        }
        result
    }

    pub fn resolve(&self, handle: &str) -> Option<&Contact> {
        self.contacts.get(handle)
    }
    pub fn handle_for_source(&self, source: &str) -> Option<String> {
        let mut matches = self
            .contacts
            .values()
            .filter(|contact| contact.source == source);
        let found = matches.next()?;
        matches.next().is_none().then(|| found.handle.clone())
    }
    pub fn known_for_work(&self, retained: Option<&str>) -> BTreeSet<String> {
        self.contacts
            .values()
            .map(|contact| contact.source.clone())
            .chain(retained.map(str::to_owned))
            .collect()
    }
}
