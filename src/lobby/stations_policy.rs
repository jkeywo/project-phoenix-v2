use crate::stations_config::{ShipStations, StationAssignments, StationDef};
use std::collections::{HashMap, VecDeque};

// ── Reassignment helpers ─────────────────────────────────────────────────────

/// Given the current N-player assignment map and a new player's token, return
/// the N+1 assignment map.
///
/// Algorithm:
/// 1. Compute the target count = current occupied stations + 1.
/// 2. If target count > max_players, the new player becomes a spectator
///    (they are **not** inserted into the returned map).
/// 3. Otherwise:
///    - Each existing player follows their station's `next` chain (explicit or
///      implicit same-name) to the corresponding station at count N+1.
///    - The station at N+1 that has no `previous` (no station at N points to
///      it) is assigned to the new player.
pub fn reassign_on_join(
    stations: &ShipStations,
    current: &StationAssignments,
    new_player: &str,
) -> StationAssignments {
    let n = current.len() as u32;
    let n1 = n + 1;

    // At or above max_players: new player is spectator → return current unchanged
    // (caller must append to spectator queue)
    if n1 > stations.max_players {
        return current.clone();
    }

    let Some(next_defs) = stations.configs.get(&n1) else {
        // No definition for count n+1: leave everything as-is
        return current.clone();
    };

    let mut new_map: StationAssignments = HashMap::new();

    if n == 0 {
        // No existing players: new player takes the first (only) station at 1P.
        if let Some(def) = next_defs.first() {
            new_map.insert(new_player.to_string(), def.name.clone());
        }
        return new_map;
    }

    // Advance each existing player along their next chain.
    let Some(current_defs) = stations.configs.get(&n) else {
        return current.clone();
    };

    // Build a set of all station names at n+1 that will be claimed by existing
    // players, so we can find the one with no predecessor.
    let mut claimed_at_n1: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (token, station_name) in current.iter() {
        let resolved_next = resolve_next(current_defs, station_name, n1);
        if let Some(next_name) = resolved_next {
            new_map.insert(token.clone(), next_name.clone());
            claimed_at_n1.insert(next_name);
        }
    }

    // The unclaimed station at n+1 goes to the new player.
    if let Some(unclaimed) = next_defs.iter().find(|d| !claimed_at_n1.contains(&d.name)) {
        new_map.insert(new_player.to_string(), unclaimed.name.clone());
    }

    new_map
}

/// Lobby-safe variant of `reassign_on_join`.
///
/// Advances every existing assigned player along their `next` chain to the
/// N+1 station layout, but does NOT assign the new player.  The new player
/// remains unassigned and must select a station explicitly.
///
/// Returns the updated assignment map (same keys as `current`, different
/// values when a station name changes at the new count).
pub fn advance_on_join(
    stations: &ShipStations,
    current: &StationAssignments,
) -> StationAssignments {
    let n = current.len() as u32;
    let n1 = n + 1;

    // If there are no assigned players there is nothing to advance.
    if n == 0 {
        return current.clone();
    }

    // At or above max_players nothing changes (caller handles spectator queue).
    if n1 > stations.max_players {
        return current.clone();
    }

    let Some(current_defs) = stations.configs.get(&n) else {
        return current.clone();
    };

    // n+1 defs must exist for the advance to make sense.
    if !stations.configs.contains_key(&n1) {
        return current.clone();
    }

    let mut new_map: StationAssignments = HashMap::new();
    for (token, station_name) in current.iter() {
        let resolved_next = resolve_next(current_defs, station_name, n1);
        if let Some(next_name) = resolved_next {
            new_map.insert(token.clone(), next_name);
        } else {
            // Station has no next entry — keep where they are.
            new_map.insert(token.clone(), station_name.clone());
        }
    }
    new_map
}

/// Given the current N-player assignment map, the token of the departing
/// player, and the current spectator queue, return the N-1 assignment map plus
/// the (possibly shorter) spectator queue.
///
/// Algorithm:
/// 1. Compute the target count = current occupied stations - 1.
/// 2. If target count < min_players, nothing changes.
/// 3. Identify the "no-previous" station at count N (the one that was added
///    when the N-th player joined).
/// 4. If the leaver is NOT the no-previous station holder:
///    - The no-previous station holder first claims the leaver's slot via the
///      leaver's station's `previous` chain.
///    - Then all remaining players follow their own `previous` chain.
///      If the leaver IS the no-previous station holder:
///    - Everyone else just follows their `previous` chain.
/// 5. If a slot at N-1 remains empty after the cascade (possible when N was at
///    max_players), pop the front of the spectator queue into the bottom-of-chain
///    station.
pub fn reassign_on_leave(
    stations: &ShipStations,
    current: &StationAssignments,
    leaving_player: &str,
    spectators: &VecDeque<String>,
) -> (StationAssignments, VecDeque<String>) {
    let n = current.len() as u32;
    if n == 0 {
        return (current.clone(), spectators.clone());
    }
    let n1 = n.saturating_sub(1);
    if n1 < stations.min_players {
        return (current.clone(), spectators.clone());
    }

    let Some(current_defs) = stations.configs.get(&n) else {
        return (current.clone(), spectators.clone());
    };

    // Find the "no-previous" station at count N: the station that has no
    // `previous` AND no other station at N-1 resolves to it implicitly.
    let no_prev_station = find_no_previous_station(stations, current_defs, n);

    let mut new_map: StationAssignments = HashMap::new();

    if n1 == 0 {
        // No players remain.
        let new_spectators = spectators.clone();
        return (new_map, new_spectators);
    }

    let Some(prev_defs) = stations.configs.get(&n1) else {
        return (current.clone(), spectators.clone());
    };

    // Build assignment for everyone except the leaver.
    // If the leaver is the no-previous holder:
    //   Every remaining player just follows their own `previous`.
    // If the leaver is NOT the no-previous holder:
    //   The no-previous holder first fills the leaver's vacated slot (follows
    //   leaver's station's `previous`), then everyone else follows their own.

    let leaver_station = current.get(leaving_player).cloned();
    let no_prev_holder: Option<String> = no_prev_station.as_ref().and_then(|s| {
        current
            .iter()
            .find(|(_, v)| *v == s)
            .map(|(k, _)| k.clone())
    });

    let leaver_is_no_prev = match (&leaver_station, &no_prev_station) {
        (Some(ls), Some(nps)) => ls == nps,
        _ => false,
    };

    for (token, station_name) in current.iter() {
        if token == leaving_player {
            continue;
        }

        let target_station: Option<String> =
            if !leaver_is_no_prev && Some(token) == no_prev_holder.as_ref() {
                // No-prev holder fills the leaver's old slot at n-1.
                leaver_station
                    .as_deref()
                    .and_then(|ls| resolve_previous(current_defs, ls, n1))
            } else {
                resolve_previous(current_defs, station_name, n1)
            };

        if let Some(name) = target_station {
            new_map.insert(token.clone(), name);
        }
    }

    // Check if any slot at n-1 is unfilled (happens when N == max_players).
    let mut new_spectators = spectators.clone();
    if new_map.len() < prev_defs.len() {
        // Promote front of spectator queue to the empty bottom-of-chain slot.
        // The empty slot is the one with no `previous` at count n1.
        let no_prev_at_n1 = find_no_previous_station(stations, prev_defs, n1);
        if let (Some(slot_name), Some(promoted)) = (no_prev_at_n1, new_spectators.pop_front()) {
            new_map.insert(promoted, slot_name);
        }
    }

    (new_map, new_spectators)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Resolve the `next` target for a station at `n` toward count `n+1`.
/// Returns the station name at `n+1` the player should move to.
fn resolve_next(
    current_defs: &[StationDef],
    station_name: &str,
    _next_count: u32,
) -> Option<String> {
    let def = current_defs.iter().find(|d| d.name == station_name)?;
    // Explicit next takes precedence.
    if let Some(explicit) = &def.next {
        return Some(explicit.clone());
    }
    // Implicit: same name at next_count (validated at parse time, so it exists).
    Some(def.name.clone())
}

/// Resolve the `previous` target for a station at `n` toward count `n-1`.
fn resolve_previous(
    current_defs: &[StationDef],
    station_name: &str,
    _prev_count: u32,
) -> Option<String> {
    let def = current_defs.iter().find(|d| d.name == station_name)?;
    if let Some(explicit) = &def.previous {
        return Some(explicit.clone());
    }
    // Implicit: same name at prev_count.
    Some(def.name.clone())
}

/// Find the station name at count `n` that has no `previous` (i.e. the station
/// that is "new" when a player joins at count N — it had no predecessor at N-1).
///
/// A station has no previous when:
/// - Its `previous` field is `None`, AND
/// - No station at N-1 has an explicit or implicit `next` that resolves to it.
fn find_no_previous_station(
    stations: &ShipStations,
    defs: &[StationDef],
    n: u32,
) -> Option<String> {
    let prev_count = n.checked_sub(1)?;
    let prev_defs = stations.configs.get(&prev_count)?;

    // Build the set of station names at n that are reachable from n-1.
    let reachable: std::collections::HashSet<String> = prev_defs
        .iter()
        .filter_map(|d| resolve_next(prev_defs, &d.name, n))
        .collect();

    defs.iter()
        .find(|d| !reachable.contains(&d.name))
        .map(|d| d.name.clone())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::Console;
    use crate::stations_config::*;

    // ── Helper TOML fixtures ─────────────────────────────────────────────────

    fn minimal_toml() -> &'static str {
        r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
description = "Single-crew bridge"
consoles = ["CaptainChair", "Helm"]
"#
    }

    fn multi_count_toml() -> &'static str {
        r#"
[stations]
min_players = 1
max_players = 4

[[stations.1]]
name = "Bridge"
consoles = ["CaptainChair", "Helm", "Tactical"]

[[stations.2]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]

[[stations.2]]
name = "Weapons"
consoles = ["Tactical"]

[[stations.3]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]

[[stations.3]]
name = "Weapons"
consoles = ["Tactical"]

[[stations.3]]
name = "Ops"
consoles = ["Repair"]

[[stations.4]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]

[[stations.4]]
name = "Weapons"
consoles = ["Tactical"]

[[stations.4]]
name = "Ops"
consoles = ["Repair"]

[[stations.4]]
name = "Sensors"
consoles = ["Sensors"]
"#
    }

    // ── Std preset name (issue #303: TOML files declare "Std", not "Full") ──

    #[test]
    fn default_complexity_presets_uses_std_not_full() {
        let presets = default_complexity_presets();
        for (console, names) in &presets {
            assert!(
                !names.iter().any(|n| n == "Full"),
                "console {:?} still advertises 'Full' preset — should be 'Std'",
                console
            );
            assert!(
                names.iter().any(|n| n == "Std"),
                "console {:?} is missing 'Std' preset",
                console
            );
        }
    }

    #[test]
    fn default_complexity_presets_sensors_only_std() {
        let presets = default_complexity_presets();
        let got = presets
            .get(&Console::Sensors)
            .expect("Sensors should have presets");
        assert_eq!(got.len(), 1, "Sensors should have exactly one preset");
        assert_eq!(got[0], "Std", "Sensors preset should be 'Std'");
    }

    #[test]
    fn default_complexity_presets_shields_only_std() {
        let presets = default_complexity_presets();
        let got = presets
            .get(&Console::Shields)
            .expect("Shields should have presets");
        assert_eq!(got.len(), 1, "Shields should have exactly one preset");
        assert_eq!(got[0], "Std", "Shields preset should be 'Std'");
    }

    #[test]
    fn default_complexity_presets_navigation_only_std() {
        let presets = default_complexity_presets();
        let got = presets
            .get(&Console::Navigation)
            .expect("Navigation should have presets");
        assert_eq!(got.len(), 1, "Navigation should have exactly one preset");
        assert_eq!(got[0], "Std", "Navigation preset should be 'Std'");
    }

    // ── short_code field ─────────────────────────────────────────────────────

    #[test]
    fn station_def_short_code_parsed_from_toml() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = ["CaptainChair"]
short_code = "BRG"
"#;
        let s = parse_and_validate(toml).unwrap();
        let def = &s.configs[&1][0];
        assert_eq!(def.short_code, "BRG");
    }

    #[test]
    fn station_def_short_code_defaults_to_empty_when_omitted() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        let def = &s.configs[&1][0];
        assert_eq!(def.short_code, "");
    }

    // ── Tracer bullet: happy-path parse ──────────────────────────────────────

    #[test]
    fn parse_minimal_toml_succeeds() {
        let result = parse_and_validate(minimal_toml());
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn parsed_stations_has_correct_bounds() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        assert_eq!(s.min_players, 1);
        assert_eq!(s.max_players, 1);
    }

    #[test]
    fn parsed_station_def_fields_are_correct() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        let def = get_station(&s, 1, "Bridge").expect("Bridge at 1 player not found");
        assert_eq!(def.name, "Bridge");
        assert_eq!(def.description, "Single-crew bridge");
        assert_eq!(def.consoles, vec![Console::CaptainChair, Console::Helm]);
    }

    // ── Multi-count happy path ───────────────────────────────────────────────

    #[test]
    fn parse_multi_count_toml_succeeds() {
        let result = parse_and_validate(multi_count_toml());
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn multi_count_has_expected_station_counts() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        assert_eq!(s.configs[&1].len(), 1);
        assert_eq!(s.configs[&2].len(), 2);
        assert_eq!(s.configs[&3].len(), 3);
        assert_eq!(s.configs[&4].len(), 4);
    }

    // ── get_station lookup ───────────────────────────────────────────────────

    #[test]
    fn get_station_hit() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        let def = get_station(&s, 3, "Ops");
        assert!(def.is_some());
        assert_eq!(def.unwrap().consoles, vec![Console::Repair]);
    }

    #[test]
    fn get_station_miss_unknown_name() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        assert!(get_station(&s, 2, "Nonexistent").is_none());
    }

    #[test]
    fn get_station_miss_unknown_count() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        assert!(get_station(&s, 99, "Bridge").is_none());
    }

    // ── all_stations_filled ───────────────────────────────────────────────────

    #[test]
    fn all_stations_filled_when_all_occupied() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        let current = vec![Console::CaptainChair, Console::Tactical];
        assert!(all_stations_filled(&s, 2, &current));
    }

    #[test]
    fn all_stations_not_filled_when_one_missing() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        // Only Bridge occupied; Weapons (Tactical) is not
        let current = vec![Console::CaptainChair];
        assert!(!all_stations_filled(&s, 2, &current));
    }

    #[test]
    fn all_stations_filled_returns_false_for_unknown_count() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        let current = vec![Console::CaptainChair];
        assert!(!all_stations_filled(&s, 99, &current));
    }

    // ── StationConfigError variants ──────────────────────────────────────────

    #[test]
    fn error_duplicate_name_same_count() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]

[[stations.1]]
name = "Bridge"
consoles = ["Tactical"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::DuplicateName { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_unknown_console() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = ["Torpedoes"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::UnknownConsole { .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_empty_consoles() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = []
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::EmptyConsoles { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_count_out_of_range() {
        let toml = r#"
[stations]
min_players = 2
max_players = 4

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(
                err,
                StationConfigError::CountOutOfRange {
                    count: 1,
                    min: 2,
                    max: 4
                }
            ),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_dangling_next() {
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]
next = "NoSuchStation"

[[stations.2]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::DanglingNext { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_dangling_previous() {
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]

[[stations.2]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]
previous = "NoSuchStation"
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::DanglingPrevious { count: 2, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_missing_next_when_count_plus_one_has_no_same_named_station() {
        // At count=1 "Alpha" has no explicit next and count=2 has "Beta" only.
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Alpha"
consoles = ["Helm"]

[[stations.2]]
name = "Beta"
consoles = ["CaptainChair"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::MissingNext { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_parse_error_on_invalid_toml() {
        let err = parse_and_validate("this is not toml ][").unwrap_err();
        assert!(
            matches!(err, StationConfigError::ParseError(_)),
            "unexpected error: {:?}",
            err
        );
    }

    // ── Implicit next/previous resolution ───────────────────────────────────

    #[test]
    fn implicit_next_resolved_when_same_name_exists_at_next_count() {
        // multi_count_toml has "Bridge" at all counts — should validate fine
        let result = parse_and_validate(multi_count_toml());
        assert!(
            result.is_ok(),
            "implicit next resolution failed: {:?}",
            result
        );
    }

    #[test]
    fn explicit_next_overrides_implicit() {
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Solo"
consoles = ["Helm"]
next = "Duo"

[[stations.2]]
name = "Duo"
consoles = ["CaptainChair", "Helm"]
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_ok(), "explicit next failed: {:?}", result);
    }

}
