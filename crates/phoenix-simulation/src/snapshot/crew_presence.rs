//! Full-candidate validation for the existing mesh crew authority projection.
//! Presence cannot be inferred from a Human control source or a local Session.
use super::*;

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world
            .try_query::<(Entity, &EntityUuid)>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
            });
        let result: Result<(), String> = (|| {
            let config = if let Some(origin) = &row.spawn {
                let mut warnings = Vec::new();
                let resolved = origin
                    .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
                    .ok_or("cannot resolve candidate crew equipment")?;
                if !warnings.is_empty() {
                    return Err("candidate crew equipment did not resolve exactly".into());
                }
                resolved.ship_config
            } else {
                live.and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
                    .map(|config| config.0.clone())
            };
            let Some(crew) = &row.mesh_crew else {
                if config.is_some()
                    || live.is_some_and(|entity| {
                        world
                            .get::<crate::lobby::crew_replication::ShipCrewPresence>(entity)
                            .is_some()
                    })
                {
                    return Err("saved ship lost its mesh crew participation owner".into());
                }
                return Ok(());
            };
            if crew.occupied.len() > phoenix_sim_session::lockstep::crew::MAX_FLEET_CREW_SEATS
                || crew.occupied.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err("occupied Stations must be bounded, unique and sorted".into());
            }
            for station in &crew.occupied {
                if station.is_empty()
                    || station.len()
                        > phoenix_sim_session::lockstep::crew::MAX_FLEET_CREW_FIELD_BYTES
                    || station.chars().any(char::is_control)
                    || config.as_ref().is_none_or(|config| {
                        config
                            .station(&crate::core::messages::StationId(station.clone()))
                            .is_none()
                    })
                    || crew.ratings.iter().filter(|(id, _)| id == station).count() != 1
                {
                    return Err(
                        "occupied Station lacks its actual authored identity and saved rating"
                            .into(),
                    );
                }
            }
            Ok(())
        })();
        result.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{
        AdmittedCommand, AdmittedCommands, StationId, SystemControlPayload, SystemId,
    };
    use crate::lobby::crew_replication::ShipCrewPresence;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn actual_spawn_crew_json_and_forged_candidate_preserve_one_replicated_presence_owner() {
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/alliance_cruiser.toml",
        )
        .unwrap();
        let mut world = World::new();
        world.register_component::<EntitySystemHull>();
        world.register_component::<EntityShipArcHull>();
        world.register_component::<crate::console::repair::server::RepairRequestQueue>();
        world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
        world.register_component::<crate::ai::server::ObjectiveCursors>();
        world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
        world.insert_resource(SimTick(20));
        world.insert_resource(Time::<Fixed>::default());
        let actor = crate::entities::spawner::spawn_entity(
            &mut world.commands(),
            &config,
            Vec3::ZERO,
            "actual-crew-ship".into(),
            None,
        );
        world.flush();
        assert!(
            world.get::<ShipCrewPresence>(actor).unwrap().0.is_empty(),
            "NPC/default control does not create people"
        );
        world
            .entity_mut(actor)
            .insert(AdmittedCommands(vec![AdmittedCommand {
                target: SystemId(
                    crate::ship::system_registry::ASSIGN_STATION_RATING_SYSTEM_ID.into(),
                ),
                payload: SystemControlPayload::AssignStationRating {
                    station: StationId("engineering".into()),
                    rating: "Std".into(),
                    occupied: Some(true),
                },
                response_token: None,
                feedback_correlation: None,
            }]));
        world
            .run_system_once(crate::lobby::crew_replication::apply_assigned_station_rating)
            .unwrap();
        let saved = capture(&world);
        let decoded: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        let row = decoded
            .entities
            .iter()
            .position(|row| row.uuid == "actual-crew-ship")
            .unwrap();
        assert_eq!(
            decoded.entities[row].mesh_crew.as_ref().unwrap().occupied,
            vec!["engineering"]
        );
        validate(&world, &decoded).unwrap();
        let before = capture(&world);
        let digest = crate::sim_digest::world_digest(&world);
        for fault in 0..4 {
            let mut candidate = decoded.clone();
            match fault {
                0 => candidate.entities[row]
                    .mesh_crew
                    .as_mut()
                    .unwrap()
                    .occupied
                    .push("engineering".into()),
                1 => {
                    candidate.entities[row].mesh_crew.as_mut().unwrap().occupied =
                        vec!["not-a-station".into()]
                }
                2 => candidate.entities[row]
                    .mesh_crew
                    .as_mut()
                    .unwrap()
                    .ratings
                    .retain(|(station, _)| station != "engineering"),
                _ => candidate.entities[row].mesh_crew = None,
            }
            assert!(
                !restore(&mut world, &candidate).is_complete(),
                "forgery{fault}"
            );
            assert_eq!(capture(&world), before);
            assert_eq!(crate::sim_digest::world_digest(&world), digest);
        }
        world.get_mut::<ShipCrewPresence>(actor).unwrap().0.clear();
        application::restore_mesh_crew(&mut world, &decoded);
        assert_eq!(
            capture::capture_mesh_crew(world.entity(actor)),
            decoded.entities[row].mesh_crew
        );
        for _ in 0..360 {
            validate(&world, &decoded).unwrap();
            assert_eq!(
                capture::capture_mesh_crew(world.entity(actor)),
                decoded.entities[row].mesh_crew
            );
        }
    }
}
