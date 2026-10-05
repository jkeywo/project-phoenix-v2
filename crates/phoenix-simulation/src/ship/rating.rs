use crate::core::messages::{StationId, SystemId};
use crate::ship::config::{ShipConfig, StationConfig};
use crate::ship::control_source::ControlSource;
use crate::ship::control_source::ControlSourceResolver;
use std::collections::HashMap;

/// Rating name that automates every system owned by the station.
pub const BACKFILL_RATING: &str = "Backfill";

/// Authored control depth, ordered so a scenario can only raise it.
pub use phoenix_model::wire::SystemDepth;

pub trait SystemDepthRuntime {
    fn control_source(self) -> ControlSource;
}
impl SystemDepthRuntime for SystemDepth {
    fn control_source(self) -> ControlSource {
        match self {
            Self::Ai => ControlSource::Ai,
            Self::Simplified => ControlSource::Simplified,
            Self::Detailed => ControlSource::Human,
        }
    }
}

/// Resolve one chosen rung without changing its identity. The floor may leave
/// a station between authored rungs; it never chooses another rung for the crew.
/// Scenario selectors are resolved to hull IDs by the caller (#1067).
pub fn resolve_system_depths(
    config: &ShipConfig,
    station_id: &StationId,
    rating_name: &str,
    floor: &HashMap<SystemId, SystemDepth>,
) -> Option<std::collections::BTreeMap<SystemId, SystemDepth>> {
    let station = config.station(station_id)?;
    let rating = if rating_name == BACKFILL_RATING {
        None
    } else {
        Some(station.ratings.iter().find(|r| r.name == rating_name)?)
    };
    Some(
        config
            .systems_for_station(station_id)
            .map(|system| {
                let chosen = match rating {
                    None => SystemDepth::Ai,
                    Some(r) if r.automated_systems.contains(&system.id) => SystemDepth::Ai,
                    Some(r)
                        if r.detailed_systems
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&system.id)) =>
                    {
                        SystemDepth::Detailed
                    }
                    Some(_) => SystemDepth::Simplified,
                };
                let effective =
                    chosen.max(floor.get(&system.id).copied().unwrap_or(SystemDepth::Ai));
                (system.id.clone(), effective)
            })
            .collect(),
    )
}

/// Resolve the set of systems that are automated for a station/rating combination.
///
/// Returns `None` when the station or rating is not found. When the rating is
/// `BACKFILL_RATING`, returns all systems owned by the station. Otherwise
/// returns the explicitly declared automated system set from the station's
/// rating table.
pub fn resolve_automated_systems(
    config: &ShipConfig,
    station_id: &StationId,
    rating_name: &str,
) -> Option<Vec<SystemId>> {
    let station = config.station(station_id)?;

    if rating_name == BACKFILL_RATING {
        return Some(
            config
                .systems_for_station(station_id)
                .map(|s| s.id.clone())
                .collect(),
        );
    }

    let rating = station.ratings.iter().find(|r| r.name == rating_name)?;

    Some(rating.automated_systems.clone())
}

/// Apply a station's rating to a `ControlSourceResolver`.
///
/// Systems declared as automated by the rating are set to `ControlSource::Ai`;
/// all other systems owned by the station are set back to `Human`. When the
/// station or rating is missing the resolver is left unchanged.
pub fn apply_rating(
    config: &ShipConfig,
    station_id: &StationId,
    rating_name: &str,
    resolver: &mut ControlSourceResolver,
) {
    let Some(depths) = resolve_system_depths(config, station_id, rating_name, &HashMap::new())
    else {
        return;
    };
    for (id, depth) in depths {
        resolver.set(id, depth.control_source());
    }
}

/// Return every rating name defined for a station, plus the implicit
/// `BACKFILL_RATING` (always available).
pub fn available_ratings_for_station<'a>(
    config: &'a ShipConfig,
    station_id: &StationId,
) -> Vec<&'a str> {
    let station = match config.station(station_id) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut names: Vec<&str> = station.ratings.iter().map(|r| r.name.as_str()).collect();
    names.push(BACKFILL_RATING);
    names
}

/// Seed a freshly spawned ship's control sources and active-rating map — the
/// ONE boot path for every hull, player or NPC (issue #871).
///
/// `rating_for` names the rating each station boots on. The player game-start
/// path passes the lobby-chosen rating for a manned station and
/// [`BACKFILL_RATING`] for an unmanned one; the generic entity spawner passes
/// [`BACKFILL_RATING`] for every station, because "NPC" is just "a stationed
/// ship with nobody connected yet".
///
/// Two passes, and the second is the one that is easy to forget:
///
/// 1. Every station applies its rating, which sets its owned systems to `Ai`
///    (Backfill automates all of them) or `Human`.
/// 2. Every `ai_only` system is set to `Ai`. Those are ownerless by
///    construction — [`crate::ship::config::validate`] rejects an ownerless
///    system that is not `ai_only` — so no station rating can ever reach them.
///    They are the auto-generated ones: the per-arc `shield_arc` systems
///    synthesised for a hull with no shields station, and the `red_alert`
///    capability provisioned for a `[behaviour]` hull that authors none.
///    Without this pass they would fall to `ControlSourceResolver`'s
///    `Human` default and silently stop being AI-operated.
pub fn seed_boot_ratings(
    config: &ShipConfig,
    rating_for: impl Fn(&StationConfig) -> String,
) -> (ControlSourceResolver, HashMap<StationId, String>) {
    let mut resolver = ControlSourceResolver::new();
    let mut active_ratings: HashMap<StationId, String> = HashMap::new();
    for station in &config.stations {
        let rating_name = rating_for(station);
        apply_rating(config, &station.id, &rating_name, &mut resolver);
        active_ratings.insert(station.id.clone(), rating_name);
    }
    for system_id in ai_only_systems(config) {
        resolver.set(system_id, ControlSource::Ai);
    }
    (resolver, active_ratings)
}

/// All system ids that are fully automated (no rating needed): `ai_only`
/// systems whose source is never human.
pub fn ai_only_systems(config: &ShipConfig) -> Vec<SystemId> {
    config
        .systems
        .iter()
        .filter(|s| s.ai_only)
        .map(|s| s.id.clone())
        .collect()
}

#[cfg(test)]
#[path = "rating_tests.rs"]
mod tests;
