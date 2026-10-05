use super::*;
use crate::entities::ai_declaration_manifest::source_scan::{
    read_non_test_source, spawn_site_source,
};
use std::collections::BTreeSet;

/// AC: the per-ship bus slots reach the PLAYER ship too.
///
/// `spawn_game_start_entities` is a hand-rolled second spawn path that
/// `entities::spawner::spawn_entity` does not feed, and four separate issues
/// (#785, #786, #882, #885) shipped a per-ship component attached on one and
/// not the other. The failure is always silent — the router writes into a
/// component that is not on the entity, so the advisory simply never lands.
///
/// Same technique as
/// `ai_declaration_manifest::tests::every_kind_is_attached_at_every_one_of_its_spawn_sites`,
/// which covers the AI *config* components. This covers the bus slots, which
/// that manifest's `FINE_SYSTEM_KINDS` walk cannot see at all.
#[test]
fn every_per_ship_bus_component_is_attached_at_every_spawn_site() {
    assert!(
        !PER_SHIP_BUS_COMPONENTS.is_empty() && !PER_SHIP_BUS_SPAWN_SITES.is_empty(),
        "the scan must have something to check"
    );
    for (file, func) in PER_SHIP_BUS_SPAWN_SITES {
        let body = spawn_site_source(file, func);
        for component in PER_SHIP_BUS_COMPONENTS {
            assert!(
                body.contains(component),
                "{file}::{func} never mentions `{component}`. Either the attachment \
                     moved (point PER_SHIP_BUS_SPAWN_SITES at where it went) or this path \
                     never got it — and for `spawn_game_start_entities` that means the \
                     PLAYER ship cannot RECEIVE that coordination advisory at all, \
                     silently."
            );
        }
    }
}

/// AC: the class cannot grow a member in silence.
///
/// The test above only checks the components someone remembered to name. A
/// new `Pending*` bus slot added to `ship::components` or `ship::shields`
/// without joining [`PER_SHIP_BUS_COMPONENTS`] would be back to having no
/// spawn-site guard — the same hole one layer up. So the roll call is
/// re-derived from the source, and anything deliberately outside the class
/// has to say so here.
#[test]
fn every_pending_ship_component_either_joins_the_bus_class_or_is_excused() {
    /// Not a channel-3 bus slot: a deferred whole-config apply, attached and
    /// consumed by the config-load path, not written by
    /// `process_coordination_lag`.
    const NOT_BUS_SLOTS: &[&str] = &["PendingShipConfig"];

    let mut found: BTreeSet<String> = BTreeSet::new();
    for file in [
        "crates/phoenix-simulation/src/ship/components.rs",
        "crates/phoenix-simulation/src/ship/shields.rs",
    ] {
        for line in read_non_test_source(file).lines() {
            let Some(rest) = line.trim_start().strip_prefix("pub struct Pending") else {
                continue;
            };
            let tail: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            found.insert(format!("Pending{tail}"));
        }
    }

    let accounted: BTreeSet<&str> = PER_SHIP_BUS_COMPONENTS
        .iter()
        .chain(NOT_BUS_SLOTS.iter())
        .copied()
        .collect();
    let unaccounted: Vec<&String> = found
        .iter()
        .filter(|name| !accounted.contains(name.as_str()))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "{unaccounted:?} is a per-ship `Pending*` component that is neither in \
             PER_SHIP_BUS_COMPONENTS nor excused in NOT_BUS_SLOTS. If it is a channel-3 \
             bus slot, add it to the class so the spawn-site guard covers it; if it is \
             not, excuse it here with the reason."
    );
    let stale: Vec<&&str> = PER_SHIP_BUS_COMPONENTS
        .iter()
        .chain(NOT_BUS_SLOTS.iter())
        .filter(|c| !found.contains(**c))
        .collect();
    assert!(
        stale.is_empty(),
        "{stale:?} is named here but no longer defined in the scanned files — a \
             rename or a move would leave the spawn-site guard checking a string nothing \
             uses, which passes for the wrong reason"
    );
}
