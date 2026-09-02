//! Deterministic replication of a ship's per-station control-source (rating)
//! changes across every fleet peer (issue #1119).
//!
//! In a fleet, a ship host resolves its OWN ship's station ratings from live
//! Sessions, but every remote peer — a stationless GM included — seeds that ship
//! from the FROZEN roster and never sees the host's live crew connect, claim,
//! release, reconnect or rating change. The two then disagree about which
//! stations run on Backfill AI, so a remote peer's AI re-asserts state a
//! connected human just set. That is the divergence #1300's exit tracer catches:
//! a reconnected Captain's Red Alert applies on the ship host but the GM peer,
//! still holding the frozen Backfill, has its Captain AI overwrite it, and the
//! folded `ShipRedAlert` splits true/false.
//!
//! This closes the gap the way `ToggleGodMode` crosses to peers (issue #900):
//! when the local ship's `ActiveStationRatings` changes during active lockstep,
//! the host mints a server-authored [`SystemControlPayload::AssignStationRating`]
//! under `LOCAL_CONSOLE_TOKEN` at an ownerless synthetic system id. It enters the
//! ordinary inbound-admission boundary, is stamped for a future tick and staged
//! to the fleet mesh, and every peer applies the same Backfill<->Human transition
//! to that ship on the same agreed tick.
//!
//! The one imperfection this slice keeps: the LOCAL ship applies the change
//! immediately (through the lobby handler that produced it) while peers apply it
//! `command_delay` ticks later, so a bounded transient window can differ. It is a
//! strict improvement over the previous PERMANENT divergence and never reaches a
//! join capture — those pause well after a crew change has settled — but a fully
//! tick-synchronised apply (deferring the local write onto the same command) is a
//! later refinement this deliberately does not attempt.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use crate::lobby::server::InboundMessage;
use crate::ship::components::ActiveStationRatings;
use crate::ship::system_registry::ASSIGN_STATION_RATING_SYSTEM_ID;

/// The last per-station ratings the local ship replicated to the fleet.
///
/// `None` until the first lockstep tick primes it from the boot ratings — which
/// every peer already seeded identically from the frozen roster, so priming
/// emits nothing. Cleared back to `None` whenever no fleet session is present,
/// so a fresh fleet re-primes rather than replaying a stale delta.
#[derive(Resource, Default)]
pub struct LastReplicatedRatings(pub Option<HashMap<StationId, String>>);

/// Emit an [`SystemControlPayload::AssignStationRating`] for every station whose
/// rating changed on the local ship since the last fleet replication (#1119).
///
/// Runs only in a fleet. Reads the local ship's authoritative
/// `ActiveStationRatings` — which every lobby crew handler writes through
/// `apply_result`, so a select/release/rating/reconnect/disconnect all land here
/// — diffs it against the last replicated snapshot, and injects one
/// server-authored command per change under `LOCAL_CONSOLE_TOKEN`. Admission
/// stamps and stages it to the mesh; [`apply_assigned_station_rating`] lands it
/// on every peer's copy of this ship.
pub fn replicate_local_crew_ratings(
    session: Option<Res<crate::lockstep::FleetLockstep>>,
    local_ratings: Query<&ActiveStationRatings, With<crate::server_app::LocalShip>>,
    mut last: ResMut<LastReplicatedRatings>,
    mut inbound: MessageWriter<InboundMessage>,
) {
    if session.is_none() {
        // Single-player never replicates; drop any primed snapshot so a later
        // fleet starts clean rather than replaying a delta against stale keys.
        if last.0.is_some() {
            last.0 = None;
        }
        return;
    }
    // A stationless GM host owns no local ship and has nothing to replicate; a
    // ship host owns exactly one. Either way `single()` erroring means "not us".
    let Ok(current) = local_ratings.single() else {
        return;
    };
    match last.0.as_mut() {
        None => {
            // Prime from the boot ratings without emitting: every peer seeded
            // these identically from the frozen roster, so replaying them would
            // be a redundant Backfill burst on the first tick.
            last.0 = Some(current.0.clone());
        }
        Some(prev) => {
            // `ActiveStationRatings` is a HashMap, whose iteration order varies
            // run-to-run. When more than one station changes on the same tick, the
            // emitted commands admit and fold in emission order, so that order MUST
            // be deterministic or two same-seed runs (and two peers) diverge.
            // Sort the changed stations by id before writing.
            let mut changed: Vec<(&StationId, &String)> = current
                .0
                .iter()
                .filter(|(station, rating)| {
                    prev.get(*station).map(String::as_str) != Some(rating.as_str())
                })
                .collect();
            changed.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
            for (station, rating) in changed {
                inbound.write(InboundMessage {
                    token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
                    msg: ClientMessage::ControlSystem {
                        target: SystemId(ASSIGN_STATION_RATING_SYSTEM_ID.to_string()),
                        payload: SystemControlPayload::AssignStationRating {
                            station: station.clone(),
                            rating: rating.clone(),
                        },
                    },
                });
            }
            // A station is only ever reassigned (Backfill included), never
            // removed from the map, so an absent-in-current key needs no
            // revocation; snapshot what we just replicated.
            prev.clone_from(&current.0);
        }
    }
}

/// Apply every admitted [`SystemControlPayload::AssignStationRating`] to the ship
/// it names (issue #1119).
///
/// `With<Ship>`, not `With<LocalShip>`: on a remote peer the command names another
/// host's ship, and the whole point is that this peer makes the same transition
/// to that ship. Idempotent — `apply_rating` re-derives the station's control
/// sources from the rating each time, so a duplicate, or the re-apply that lands
/// on the origin host beside its own immediate lobby write, is a no-op.
pub fn apply_assigned_station_rating(
    mut ships: Query<
        (
            &crate::core::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &mut crate::ship_plugin::ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, config, mut sources, mut ratings) in ships.iter_mut() {
        for cmd in admitted.for_target(ASSIGN_STATION_RATING_SYSTEM_ID) {
            let SystemControlPayload::AssignStationRating { station, rating } = &cmd.payload else {
                continue;
            };
            crate::ship::rating::apply_rating(&config.0, station, rating, &mut sources.0);
            ratings.0.insert(station.clone(), rating.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{AdmittedCommand, AdmittedCommands};
    use crate::ship::config::parse_and_validate;
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

    const KINDS: &[&str] = &["red_alert", "viewscreen"];

    /// A minimal hull whose Captain owns one automatable system, with a rating
    /// that automates it and one that does not — enough to prove the applier
    /// drives the control source in both directions.
    const CONFIG_TOML: &str = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Manual"
automated_systems = []

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"
"#;

    fn assign(station: &str, rating: &str) -> AdmittedCommand {
        AdmittedCommand {
            target: SystemId(ASSIGN_STATION_RATING_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::AssignStationRating {
                station: StationId(station.into()),
                rating: rating.into(),
            },
            response_token: None,
            feedback_correlation: None,
        }
    }

    fn source_of(app: &App, ship: Entity, system: &str) -> ControlSource {
        app.world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap()
            .0
            .source_for(&SystemId(system.into()))
    }

    fn spawn_ship(app: &mut App, cmd: AdmittedCommand, seed: ControlSource) -> Entity {
        let config = parse_and_validate(CONFIG_TOML, KINDS).expect("hull parses");
        let mut sources = ControlSourceResolver::default();
        sources.set(SystemId("red-alert".into()), seed);
        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                AdmittedCommands(vec![cmd]),
                ShipConfigComponent(config),
                ShipSystemControlSources(sources),
                ActiveStationRatings::default(),
            ))
            .id()
    }

    #[test]
    fn an_admitted_assign_backfills_the_captains_own_system_and_records_the_rating() {
        let mut app = App::new();
        // Seed the human-held state a live crew member would have set.
        let ship = spawn_ship(
            &mut app,
            assign("captain", "Assisted"),
            ControlSource::Human,
        );
        app.world_mut()
            .run_system_cached(apply_assigned_station_rating)
            .unwrap();
        assert_eq!(
            source_of(&app, ship, "red-alert"),
            ControlSource::Ai,
            "the Assisted rating automates the Captain's red-alert on this peer"
        );
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<ActiveStationRatings>()
                .unwrap()
                .0
                .get(&StationId("captain".into()))
                .map(String::as_str),
            Some("Assisted"),
            "the replicated rating is recorded for the GM projection to read"
        );
    }

    #[test]
    fn an_admitted_assign_restores_human_control_when_the_rating_automates_nothing() {
        let mut app = App::new();
        // A Station the AI was holding; a returning human's Manual rating claims it.
        let ship = spawn_ship(&mut app, assign("captain", "Manual"), ControlSource::Ai);
        app.world_mut()
            .run_system_cached(apply_assigned_station_rating)
            .unwrap();
        assert_eq!(
            source_of(&app, ship, "red-alert"),
            ControlSource::Human,
            "a rating that automates nothing hands the Station back to the human"
        );
    }

    #[test]
    fn a_command_for_another_target_is_ignored() {
        let mut app = App::new();
        let mut other = assign("captain", "Assisted");
        other.target = SystemId("red-alert".into());
        let ship = spawn_ship(&mut app, other, ControlSource::Human);
        app.world_mut()
            .run_system_cached(apply_assigned_station_rating)
            .unwrap();
        assert_eq!(
            source_of(&app, ship, "red-alert"),
            ControlSource::Human,
            "the applier reads only its own ownerless system id"
        );
    }
}
