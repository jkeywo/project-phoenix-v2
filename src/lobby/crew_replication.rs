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
//! Crew changes enter the ordinary admission boundary before they change any
//! authoritative rating or control source. Every peer, including the originating
//! host, applies the transition on the same future tick. Applying locally early
//! is not a bounded transient: those extra AI ticks permanently change physics.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use crate::lobby::server::InboundMessage;
use crate::ship::components::ActiveStationRatings;
use crate::ship::system_registry::ASSIGN_STATION_RATING_SYSTEM_ID;

/// Host-local crew requests not yet submitted to ordinary command admission.
/// This is ingress bookkeeping, not a second queue of accepted commands.
/// Preserve arrival order, including A -> B -> A before either applies.
#[derive(Resource, Default)]
pub struct PendingCrewRatingChanges(Vec<(StationId, String)>);

impl PendingCrewRatingChanges {
    pub fn push(&mut self, station: StationId, rating: String) {
        self.0.push((station, rating));
    }

    /// Project the crew's latest choice for reconnect/AFK bookkeeping without
    /// changing simulation state. Call only before admission or after application.
    pub fn overlay(
        &self,
        ratings: &mut HashMap<StationId, String>,
        pending: Option<&crate::command_admission::PendingCommands>,
        entity: Entity,
        uuid: Option<&crate::entities::spawner::EntityUuid>,
    ) {
        if let Some(pending) = pending {
            for command in pending.iter().filter(|command| {
                uuid.map_or(command.route == entity, |uuid| command.ship.0 == uuid.0)
                    && command.command.target.0 == ASSIGN_STATION_RATING_SYSTEM_ID
            }) {
                if let SystemControlPayload::AssignStationRating { station, rating } =
                    &command.command.payload
                {
                    ratings.insert(station.clone(), rating.clone());
                }
            }
        }
        for (station, rating) in &self.0 {
            ratings.insert(station.clone(), rating.clone());
        }
    }
}

pub fn clear_pending_crew_ratings(mut pending: ResMut<PendingCrewRatingChanges>) {
    pending.0.clear();
}

/// Submit local intents before admission. Lobby handlers have already run;
/// mid-game rating requests collected in Input enter on the following tick.
/// Once drained, accepted intents live only in PendingCommands. Refused ones
/// leave no shadow, and applied ones are read from ActiveStationRatings.
pub fn replicate_local_crew_ratings(
    session: Option<Res<crate::lockstep::FleetLockstep>>,
    local_ship: Query<(), With<crate::server_app::LocalShip>>,
    mut pending: ResMut<PendingCrewRatingChanges>,
    mut inbound: MessageWriter<InboundMessage>,
) {
    if session.is_none() || local_ship.single().is_err() {
        pending.0.clear();
        return;
    }
    for (station, rating) in pending.0.drain(..) {
        inbound.write(InboundMessage {
            token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(ASSIGN_STATION_RATING_SYSTEM_ID.to_string()),
                payload: SystemControlPayload::AssignStationRating { station, rating },
            },
        });
    }
}

/// Apply every admitted [`SystemControlPayload::AssignStationRating`] to the ship
/// it names (issue #1119).
///
/// `With<Ship>`, not `With<LocalShip>`: on a remote peer the command names another
/// host's ship, and the whole point is that this peer makes the same transition
/// to that ship. Idempotent — `apply_rating` re-derives the station's control
/// sources from the rating each time, so a duplicate is a no-op.
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
