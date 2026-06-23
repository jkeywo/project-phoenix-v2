use crate::messages::{StationId, SystemId};
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSource;
use crate::ship::control_source::ControlSourceResolver;
use std::collections::HashSet;

/// Rating name that automates every system owned by the station.
pub const BACKFILL_RATING: &str = "Backfill";

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
    let Some(automated) = resolve_automated_systems(config, station_id, rating_name) else {
        return;
    };

    let automated_set: HashSet<&SystemId> = automated.iter().collect();

    // Set automated systems to Ai
    for system_id in &automated {
        resolver.set(system_id.clone(), ControlSource::Ai);
    }

    // Set all other station-owned systems back to Human
    for system in config.systems_for_station(station_id) {
        if !automated_set.contains(&system.id) {
            resolver.set(system.id.clone(), ControlSource::Human);
        }
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
mod tests {
    use super::*;
    use crate::ship::config::parse_and_validate;

    const KINDS: &[&str] = &[
        "red_alert",
        "helm",
        "phaser_bank",
        "torpedo_magazine",
        "torpedo_tube",
        "viewscreen",
    ];

    fn valid_toml() -> &'static str {
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"
console = "captain"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Manual"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons and threat response."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Assisted"
automated_systems = ["torpedo-magazine", "torpedo-tube-fore-port"]

[[station.rating]]
name = "FullAuto"
automated_systems = ["phaser-fore", "torpedo-magazine", "torpedo-tube-fore-port"]

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[power_groups.weapons]
label = "Weapons"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"

[system.config]
facing_deg = 0
fire_arc_deg = 270

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"
power_group = "weapons"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"
power_group = "weapons"

[[system]]
id = "viewscreen"
kind = "viewscreen"
ai_only = true
power_group = "ops"
"#
    }

    fn parse() -> ShipConfig {
        parse_and_validate(valid_toml(), KINDS).expect("ship config should parse")
    }

    // ── resolve_automated_systems ───────────────────────────────────────

    #[test]
    fn resolves_explicit_rating_systems() {
        let config = parse();
        let result = resolve_automated_systems(&config, &StationId("tactical".into()), "Assisted");
        assert_eq!(
            result,
            Some(vec![
                SystemId("torpedo-magazine".into()),
                SystemId("torpedo-tube-fore-port".into()),
            ])
        );
    }

    #[test]
    fn resolves_empty_rating_systems() {
        let config = parse();
        let result = resolve_automated_systems(&config, &StationId("captain".into()), "Manual");
        assert_eq!(result, Some(vec![]));
    }

    #[test]
    fn backfill_returns_all_station_systems() {
        let config = parse();
        let result =
            resolve_automated_systems(&config, &StationId("tactical".into()), BACKFILL_RATING);
        let expected: Vec<SystemId> = vec![
            SystemId("phaser-fore".into()),
            SystemId("torpedo-magazine".into()),
            SystemId("torpedo-tube-fore-port".into()),
        ];
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn unknown_rating_returns_none() {
        let config = parse();
        let result = resolve_automated_systems(&config, &StationId("captain".into()), "Bogus");
        assert_eq!(result, None);
    }

    #[test]
    fn unknown_station_returns_none() {
        let config = parse();
        let result = resolve_automated_systems(&config, &StationId("ghost".into()), "Assisted");
        assert_eq!(result, None);
    }

    #[test]
    fn varying_rating_counts_per_station() {
        let config = parse();
        // captain has 2 ratings
        let captain = config.station(&StationId("captain".into())).unwrap();
        assert_eq!(captain.ratings.len(), 2);
        // tactical has 2 ratings
        let tactical = config.station(&StationId("tactical".into())).unwrap();
        assert_eq!(tactical.ratings.len(), 2);
    }

    // ── available_ratings_for_station ───────────────────────────────────

    #[test]
    fn available_ratings_includes_backfill() {
        let config = parse();
        let ratings = available_ratings_for_station(&config, &StationId("captain".into()));
        assert!(ratings.contains(&"Assisted"));
        assert!(ratings.contains(&"Manual"));
        assert!(ratings.contains(&BACKFILL_RATING));
    }

    // ── apply_rating ────────────────────────────────────────────────────

    #[test]
    fn apply_rating_sets_ai_for_automated_systems() {
        let config = parse();
        let mut resolver = ControlSourceResolver::new();
        let station = StationId("tactical".into());

        // Before: all human
        assert_eq!(
            resolver.source_for(&SystemId("phaser-fore".into())),
            ControlSource::Human
        );

        apply_rating(&config, &station, "FullAuto", &mut resolver);

        // Automated systems → Ai
        assert_eq!(
            resolver.source_for(&SystemId("phaser-fore".into())),
            ControlSource::Ai
        );
        assert_eq!(
            resolver.source_for(&SystemId("torpedo-magazine".into())),
            ControlSource::Ai
        );
    }

    #[test]
    fn apply_rating_restores_human_for_unlisted_systems() {
        let config = parse();
        let mut resolver = ControlSourceResolver::new();
        let station = StationId("tactical".into());

        // Pre-set everything to Ai
        for system in config.systems_for_station(&station) {
            resolver.set(system.id.clone(), ControlSource::Ai);
        }

        // Apply Assisted rating — only torpedo-magazine and torpedo-tube-fore-port stay Ai
        apply_rating(&config, &station, "Assisted", &mut resolver);

        assert_eq!(
            resolver.source_for(&SystemId("torpedo-magazine".into())),
            ControlSource::Ai
        );
        assert_eq!(
            resolver.source_for(&SystemId("phaser-fore".into())),
            ControlSource::Human
        );
    }

    #[test]
    fn apply_rating_does_not_affect_other_station_systems() {
        let config = parse();
        let mut resolver = ControlSourceResolver::new();

        // Pre-set tactical systems to Ai
        for system in config.systems_for_station(&StationId("tactical".into())) {
            resolver.set(system.id.clone(), ControlSource::Ai);
        }

        // Apply Manual rating on captain (empty automated list)
        apply_rating(
            &config,
            &StationId("captain".into()),
            "Manual",
            &mut resolver,
        );

        // Tactical systems should be unaffected
        assert_eq!(
            resolver.source_for(&SystemId("phaser-fore".into())),
            ControlSource::Ai
        );
    }

    #[test]
    fn apply_rating_unknown_station_is_noop() {
        let config = parse();
        let mut resolver = ControlSourceResolver::new();
        let before = resolver.clone();

        apply_rating(
            &config,
            &StationId("ghost".into()),
            "Assisted",
            &mut resolver,
        );
        assert_eq!(resolver, before);
    }

    #[test]
    fn backfill_rating_sets_all_station_systems_to_ai() {
        let config = parse();
        let mut resolver = ControlSourceResolver::new();
        let station = StationId("tactical".into());

        apply_rating(&config, &station, BACKFILL_RATING, &mut resolver);

        for system in config.systems_for_station(&station) {
            assert_eq!(
                resolver.source_for(&system.id),
                ControlSource::Ai,
                "system {} should be Ai under backfill",
                system.id.0
            );
        }
    }

    // ── ai_only_systems ─────────────────────────────────────────────────

    #[test]
    fn returns_ai_only_systems() {
        let config = parse();
        let systems = ai_only_systems(&config);
        assert_eq!(systems, vec![SystemId("viewscreen".into())]);
    }
}
