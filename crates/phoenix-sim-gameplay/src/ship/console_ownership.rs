//! Authored Console ownership. Admission and projections share these lookups.
use super::config::{ShipConfig, ShipConfigError, SystemInstanceConfig};
use crate::core::messages::{ConsoleId, StationId, SystemId};
use phoenix_model::console::ConsoleConfig;
use std::collections::HashSet;

/// Select a hosting/rating pair without consulting another Console's host.
/// A scenario floor can raise a visitor to a supported rung, never lower it.
pub fn resolve_assignment(
    config: &ShipConfig,
    console: &ConsoleConfig,
    available: impl Fn(&StationId) -> bool,
    active: &std::collections::HashMap<StationId, String>,
    detailed: &std::collections::BTreeSet<SystemId>,
) -> phoenix_model::console::ConsoleHostSnapshot {
    resolve_assignment_with_eligibility(
        config,
        console,
        |candidate, _| available(candidate),
        active,
        detailed,
    )
}

/// Eligibility is evaluated for this Console at the proposed rating, before
/// selecting its host. A rejected Console therefore stays on ordinary Backfill
/// without changing the rating tested on the next placement pass.
pub fn resolve_assignment_with_eligibility(
    config: &ShipConfig,
    console: &ConsoleConfig,
    available: impl Fn(&StationId, &str) -> bool,
    active: &std::collections::HashMap<StationId, String>,
    detailed: &std::collections::BTreeSet<SystemId>,
) -> phoenix_model::console::ConsoleHostSnapshot {
    let ratings = config.console_ratings(console);
    let rating_for = |candidate: &StationId| -> Option<String> {
        if console.home_station.as_ref() == Some(candidate) {
            Some(
                active
                    .get(candidate)
                    .cloned()
                    .or_else(|| ratings.first().map(|r| r.name.clone()))
                    .unwrap_or_default(),
            )
        } else if ratings.is_empty() {
            Some(String::new())
        } else {
            let baseline = console
                .visiting_rating
                .as_ref()
                .and_then(|name| ratings.iter().position(|r| &r.name == name))
                .unwrap_or(0);
            ratings[..=baseline]
                .iter()
                .rev()
                .find(|r| {
                    config
                        .systems
                        .iter()
                        .filter(|s| {
                            s.console.as_ref() == Some(&console.id) && detailed.contains(&s.id)
                        })
                        .all(|s| {
                            !s.ai_only
                                && !r.automated_systems.contains(&s.id)
                                && r.detailed_systems
                                    .as_ref()
                                    .is_none_or(|ids| ids.contains(&s.id))
                        })
                })
                .map(|r| r.name.clone())
        }
    };
    let selected = if console.screen.is_some() {
        console
            .home_station
            .iter()
            .chain(console.host_order.iter().filter(|_| console.human_seeking))
            .find_map(|candidate| {
                let rating = rating_for(candidate)?;
                available(candidate, &rating).then(|| (candidate.clone(), rating))
            })
    } else {
        None
    };
    let (host, rating) = selected.map_or_else(
        || (None, super::rating::BACKFILL_RATING.to_string()),
        |(host, rating)| (Some(host), rating),
    );
    phoenix_model::console::ConsoleHostSnapshot {
        console: console.id.clone(),
        host,
        rating,
    }
}

/// Borrowed access to the existing publications; hosting does not change this scope.
pub struct ConsoleKnowledgeView<'a> {
    config: &'a ShipConfig,
    console: Option<&'a ConsoleConfig>,
    publications:
        &'a std::collections::HashMap<SystemId, phoenix_model::messages::SystemBlackboard>,
}

impl<'a> ConsoleKnowledgeView<'a> {
    /// Metadata never bypasses the owned/typed-feed predicate used for its value.
    pub fn recorded_observations(
        &self,
        source: &SystemId,
    ) -> Vec<&'a phoenix_model::observation::ObservationMetadata> {
        self.get(source)
            .map_or_else(Vec::new, |board| board.recorded_observations())
    }

    pub fn get(&self, source: &SystemId) -> Option<&'a phoenix_model::messages::SystemBlackboard> {
        let console = self.console?;
        let value = self.publications.get(source)?;
        let derived_scan = if let phoenix_model::messages::SystemBlackboard::Scan(scan) = value {
            let owner = phoenix_model::sensing::scan_owner_from_blackboard_key_borrowed(source)?;
            if scan.owner.as_ref().map(|owner| owner.0.as_str()) != Some(owner) {
                return None;
            }
            Some(owner)
        } else {
            None
        };
        let owned = self
            .config
            .systems
            .iter()
            .find(|system| system.id.0 == derived_scan.unwrap_or(source.0.as_str()))
            .is_some_and(|system| system.console.as_ref() == Some(&console.id));
        (owned || console.feeds.iter().any(|feed| feed.accepts(source, value))).then_some(value)
    }

    pub fn permits_kind(&self, kind: phoenix_model::console::ConsoleObservationKind) -> bool {
        self.publications.iter().any(|(source, value)| {
            phoenix_model::console::ConsoleObservationKind::of(value) == kind
                && self.get(source).is_some()
        })
    }
}

impl ShipConfig {
    pub fn observations_for_system<'a>(
        &'a self,
        system: &SystemId,
        publications: &'a std::collections::HashMap<
            SystemId,
            phoenix_model::messages::SystemBlackboard,
        >,
    ) -> ConsoleKnowledgeView<'a> {
        ConsoleKnowledgeView {
            config: self,
            console: self.console_for_system(system),
            publications,
        }
    }

    pub fn console_ratings<'a>(
        &'a self,
        console: &'a ConsoleConfig,
    ) -> &'a [super::config::StationRatingConfig] {
        if !console.ratings.is_empty() {
            &console.ratings
        } else {
            console
                .home_station
                .as_ref()
                .and_then(|id| self.station(id))
                .map(|s| s.ratings.as_slice())
                .unwrap_or(&[])
        }
    }

    pub fn console_system_depths(
        &self,
        console: &ConsoleConfig,
        name: &str,
    ) -> std::collections::BTreeMap<SystemId, super::rating::SystemDepth> {
        use super::rating::{SystemDepth, BACKFILL_RATING};
        let rating = self
            .console_ratings(console)
            .iter()
            .find(|r| r.name == name);
        self.systems
            .iter()
            .filter(|s| s.console.as_ref() == Some(&console.id))
            .map(|system| {
                let depth = match rating {
                    _ if system.ai_only || name == BACKFILL_RATING => SystemDepth::Ai,
                    Some(r) if r.automated_systems.contains(&system.id) => SystemDepth::Ai,
                    Some(r)
                        if r.detailed_systems
                            .as_ref()
                            .is_some_and(|ids| !ids.contains(&system.id)) =>
                    {
                        SystemDepth::Simplified
                    }
                    _ => SystemDepth::Detailed,
                };
                (system.id.clone(), depth)
            })
            .collect()
    }

    pub fn console(&self, id: &ConsoleId) -> Option<&ConsoleConfig> {
        self.consoles.iter().find(|console| &console.id == id)
    }

    pub fn console_for_system(&self, id: &SystemId) -> Option<&ConsoleConfig> {
        self.console(self.system(id)?.console.as_ref()?)
    }

    /// Authored home only. Live human-seeking admission must consult placement.
    pub fn system_home_station<'a>(
        &'a self,
        system: &'a SystemInstanceConfig,
    ) -> Option<&'a StationId> {
        self.console(system.console.as_ref()?)?
            .home_station
            .as_ref()
    }
}

/// Validate the supported topology before any runtime authority is derived.
pub fn validate_consoles(config: &ShipConfig) -> Result<(), ShipConfigError> {
    let refuse = |message: String| ShipConfigError::ParseError(message);
    let mut publications = super::system_registry::fixed_publication_kinds();
    let instance_kinds = super::system_registry::instance_publication_kinds();
    for system in &config.systems {
        if system.id.0.starts_with("stock/") || system.id.0.starts_with("scan/") {
            return Err(refuse(
                "stock/ and scan/ are reserved for typed publications, not System identities"
                    .into(),
            ));
        }
        publications.insert(
            phoenix_model::logistics_ui::stock_publication(&system.id).0,
            phoenix_model::console::ConsoleObservationKind::Stock,
        );
        if system.kind == super::system_registry::SENSORS_KIND {
            publications.insert(
                phoenix_model::sensing::scan_blackboard_key_for(&system.id).0,
                phoenix_model::console::ConsoleObservationKind::Scan,
            );
        }
        if let Some(kind) = instance_kinds.get(&system.kind) {
            if publications
                .get(&system.id.0)
                .is_some_and(|existing| existing != kind)
            {
                return Err(refuse(format!(
                    "System {:?} publication kind {:?} conflicts with the reserved publication key",
                    system.id, kind
                )));
            }
            publications.entry(system.id.0.clone()).or_insert(*kind);
        }
    }
    let mut ids = HashSet::new();
    for console in &config.consoles {
        if console.id.0.trim().is_empty() || !ids.insert(&console.id) {
            return Err(refuse(format!(
                "Console id must be nonempty and unique: {:?}",
                console.id
            )));
        }
        if console
            .screen
            .as_ref()
            .is_some_and(|screen| screen.trim().is_empty())
        {
            return Err(refuse(format!(
                "Console {:?} has an empty screen URL",
                console.id
            )));
        }
        if console
            .home_station
            .iter()
            .chain(&console.host_order)
            .any(|id| config.station(id).is_none())
        {
            return Err(refuse(format!(
                "Console {:?} references an unknown Station",
                console.id
            )));
        }
        if !console.human_seeking
            && (!console.host_order.is_empty() || console.visiting_rating.is_some())
        {
            return Err(refuse(format!(
                "Fixed Console {:?} cannot author visiting settings",
                console.id
            )));
        }
        if let Some(name) = &console.visiting_rating {
            if !config
                .console_ratings(console)
                .iter()
                .any(|rating| &rating.name == name)
            {
                return Err(refuse(format!(
                    "Console {:?} has an unknown visiting rating",
                    console.id
                )));
            }
        }
        if let Some(target) = &console.command_target {
            let target_station = config.station(target).ok_or_else(|| {
                ShipConfigError::ConsoleCommandTargetUnknownStation {
                    console: console.id.clone(),
                    target: target.clone(),
                }
            })?;
            if target_station.stances.is_empty() {
                return Err(ShipConfigError::ConsoleCommandTargetHasNoStances {
                    console: console.id.clone(),
                    target: target.clone(),
                });
            }
        }
        let mut names = HashSet::new();
        for rating in &console.ratings {
            if rating.name.trim().is_empty() || !names.insert(&rating.name) {
                return Err(refuse(format!(
                    "Console {:?} has an empty or duplicate rating",
                    console.id
                )));
            }
            if rating
                .automated_systems
                .iter()
                .chain(rating.detailed_systems.iter().flatten())
                .any(|id| {
                    config
                        .system(id)
                        .is_none_or(|system| system.console.as_ref() != Some(&console.id))
                })
            {
                return Err(refuse(format!(
                    "Console {:?} rating {:?} references equipment it does not own",
                    console.id, rating.name
                )));
            }
        }
        let mut hosts = HashSet::new();
        if console.host_order.iter().any(|host| !hosts.insert(host)) {
            return Err(refuse(format!("Console {:?} repeats a host", console.id)));
        }
        let mut feeds = HashSet::new();
        if console
            .feeds
            .iter()
            .any(|feed| feed.source.0.trim().is_empty() || !feeds.insert(&feed.source))
        {
            return Err(refuse(format!(
                "Console {:?} has an empty or duplicate feed",
                console.id
            )));
        }
        for feed in &console.feeds {
            let Some(expected) = publications.get(&feed.source.0) else {
                return Err(refuse(format!("Console {:?} references unknown publication {:?}; choose a supported publication key, not a Console or command-only System id", console.id, feed.source)));
            };
            if expected != &feed.kind {
                return Err(refuse(format!(
                    "Console {:?} publication {:?} has kind {:?}, not {:?}",
                    console.id, feed.source, expected, feed.kind
                )));
            }
        }
    }
    for system in &config.systems {
        let Some(id) = &system.console else {
            return Err(refuse(format!("System {:?} has no Console. Convert this pack to the Console ownership format: add [[console]] and replace each system.station with system.console; otherwise use its original game version.", system.id)));
        };
        let console = config.console(id).ok_or_else(|| {
            refuse(format!(
                "System {:?} references unknown Console {:?}",
                system.id, id
            ))
        })?;
        if !system.ai_only && console.screen.is_none() {
            return Err(refuse(format!("Operable System {:?} needs a Console screen; screenless knowledge scopes are for AI-only equipment", system.id)));
        }
        if !system.ai_only
            && console.home_station.is_none()
            && (!console.human_seeking || console.host_order.is_empty())
        {
            return Err(refuse(format!(
                "Operable System {:?} needs a Console with a human hosting route",
                system.id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "console_ownership_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "console_observation_tests.rs"]
mod observation_tests;

#[cfg(test)]
#[path = "console_borrowed_scan_tests.rs"]
mod borrowed_scan_tests;
