use super::*;
use crate::core::messages::StationId;
use crate::ship::config::{ShipConfig, SystemInstanceConfig};
use crate::ship::damage::SystemHull;

fn system(id: &str, station: Option<&str>) -> SystemInstanceConfig {
    SystemInstanceConfig {
        id: SystemId(id.into()),
        kind: "generic".into(),
        station: station.map(|s| StationId(s.into())),
        ai_only: false,
        human_seeking: false,
        seek_order: Vec::new(),
        power_group: None,
        marker: None,
        config: None,
    }
}

fn config(systems: Vec<SystemInstanceConfig>) -> crate::ship_plugin::ShipConfigComponent {
    crate::ship_plugin::ShipConfigComponent(ShipConfig {
        stations: vec![],
        systems,
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    })
}

/// The `alliance_cruiser` collision: `science` is BOTH a station and an
/// OWNERLESS hull row (a `[[hull.system_hull]]` with no `[[system]]` behind
/// it, so it lives in the `core` sweep bucket), while the three systems the
/// `science` station owns carry no hull rows at all.
///
/// A dispatch to that station therefore finds no repairable owned system and
/// falls through to the station-name fallback — where emitting
/// `SystemId("science")` would hand `repair_teams::sweep_from` a real hull
/// row, pass its gate, and let the team sweep the OWNERLESS bucket instead of
/// bouncing. `None` is the honest answer: nothing the station owns is
/// damaged, so there is no work to send a team to.
#[test]
fn station_name_colliding_with_a_hull_row_resolves_to_no_dispatch() {
    let cfg = config(vec![
        system("sensor-array", Some("science")),
        system("sensor-probe", Some("science")),
        system("sensor-lab", Some("science")),
    ]);
    // The hull tracks the ownerless `science` row and the ship-wide `core`
    // row — and NONE of the science station's own systems.
    let hull = SystemHull::from_config(&[
        (SystemId("core".into()), 100.0_f32),
        (SystemId("science".into()), 58.0),
    ]);

    assert_eq!(
        resolve_repair_target(
            &RepairTarget::Station(StationId("science".into())),
            &cfg,
            Some(&hull),
        ),
        None,
        "a station whose own name is a hull row must not fall back to it"
    );
}

/// The complementary half: a station name the hull does NOT track still
/// falls back, so the pre-#1013 bounce behaviour for coarse hull layouts is
/// untouched. `repair_teams::sweep_from` rejects exactly these arrivals.
#[test]
fn station_name_that_is_not_a_hull_row_still_falls_back() {
    let cfg = config(vec![system("helm-engine-port", Some("helm"))]);
    // `helm-engine-port` is at full HP, so no owned system is repairable.
    let hull = SystemHull::from_config(&[(SystemId("helm-engine-port".into()), 100.0_f32)]);

    assert_eq!(
        resolve_repair_target(
            &RepairTarget::Station(StationId("helm".into())),
            &cfg,
            Some(&hull),
        ),
        Some(SystemId("helm".into())),
        "an untracked station name is still the coarse-layout fallback"
    );
}

/// A damaged owned system is picked as before — the collision guard only
/// governs the fallback arm.
#[test]
fn a_damaged_owned_system_still_wins_over_the_fallback() {
    let cfg = config(vec![system("science-scope", Some("science"))]);
    let mut hull = SystemHull::from_config(&[
        (SystemId("science".into()), 58.0_f32),
        (SystemId("science-scope".into()), 40.0),
    ]);
    hull.set_hp(&SystemId("science-scope".into()), 10.0);

    assert_eq!(
        resolve_repair_target(
            &RepairTarget::Station(StationId("science".into())),
            &cfg,
            Some(&hull),
        ),
        Some(SystemId("science-scope".into())),
    );
}
