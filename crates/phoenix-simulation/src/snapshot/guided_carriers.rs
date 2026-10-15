//! Closed physical carriers have no authored ship recipe or machinery.
use super::*;
use crate::console::weapons::guided_flight::GuidedFlightCarrier;

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world
            .try_query::<(Entity, &EntityUuid)>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
            });
        let carrier = live.is_some_and(|entity| world.get::<GuidedFlightCarrier>(entity).is_some());
        if live.is_some() && carrier != row.guided_flight_carrier {
            return Err((
                row.uuid.clone(),
                "guided carrier cannot replace an actual ship or regain machinery".into(),
            ));
        }
        if !row.guided_flight_carrier {
            continue;
        }
        let Some(weapons) = &row.weapons else {
            return Err((
                row.uuid.clone(),
                "guided carrier is missing its actual flights".into(),
            ));
        };
        let expected_weapons = WeaponState {
            magazine: Some(crate::resources::Stock::new(0, 0).unwrap()),
            torpedoes_in_flight: weapons.torpedoes_in_flight.clone(),
            ..Default::default()
        };
        let expected = EntityState {
            uuid: row.uuid.clone(),
            guided_flight_carrier: true,
            weapons: Some(expected_weapons),
            ..Default::default()
        };
        if row != &expected || weapons.torpedoes_in_flight.is_empty() {
            return Err((
                row.uuid.clone(),
                "guided carrier must contain only its nonempty actual flight vector".into(),
            ));
        }
    }
    Ok(())
}

/// Single-actor GM inverses cannot rewind a physical round now owned elsewhere.
/// Refuse rather than silently duplicate it; bulk restore validates its full roster.
pub(super) fn unique_single_row(world: &World, entity: Entity, row: &EntityState) -> bool {
    let Some(saved) = &row.weapons else {
        return true;
    };
    world
        .try_query::<(Entity, &crate::console::weapons::TorpedoSystemResource)>()
        .is_none_or(|mut query| {
            query
                .iter(world)
                .filter(|(owner, _)| *owner != entity)
                .all(|(_, owner)| {
                    owner.0.in_flight.iter().all(|round| {
                        !saved
                            .torpedoes_in_flight
                            .iter()
                            .any(|candidate| candidate.uuid == round.uuid)
                    })
                })
        })
}
