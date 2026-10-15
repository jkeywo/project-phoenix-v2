//! Closed physical carriers have no authored ship recipe or machinery.
use super::*;
use crate::console::weapons::flight_carrier::WeaponFlightCarrier;

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
            });
        let carrier = live.is_some_and(|entity| world.get::<WeaponFlightCarrier>(entity).is_some());
        if live.is_some() && carrier != row.weapon_flight_carrier {
            return Err((
                row.uuid.clone(),
                "weapon-flight carrier cannot replace an actual ship or regain machinery".into(),
            ));
        }
        if !row.weapon_flight_carrier {
            if !row.carrier_bolt_banks.is_empty() {
                return Err((
                    row.uuid.clone(),
                    "ordinary ships cannot carry a flight-only bank recipe".into(),
                ));
            }
            continue;
        }
        if row.uuid.trim().is_empty()
            || row.uuid.len() > 128
            || row.uuid.chars().any(char::is_control)
        {
            return Err((
                row.uuid.clone(),
                "weapon-flight carrier requires a nonempty bounded physical identity".into(),
            ));
        }
        if snapshot
            .entities
            .iter()
            .filter(|candidate| candidate.uuid == row.uuid)
            .count()
            != 1
            || snapshot
                .asteroids
                .iter()
                .any(|candidate| candidate.uuid == row.uuid)
        {
            return Err((
                row.uuid.clone(),
                "weapon-flight carrier identity must be unique across the candidate roster".into(),
            ));
        }
        let Some(weapons) = &row.weapons else {
            return Err((
                row.uuid.clone(),
                "weapon-flight carrier is missing its actual flights".into(),
            ));
        };
        let expected_weapons = WeaponState {
            magazine: Some(crate::resources::Stock::new(0, 0).unwrap()),
            torpedoes_in_flight: weapons.torpedoes_in_flight.clone(),
            blasters: weapons
                .blasters
                .iter()
                .map(|bank| BlasterRuntime {
                    in_flight: bank.in_flight.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let expected = EntityState {
            uuid: row.uuid.clone(),
            weapon_flight_carrier: true,
            carrier_bolt_banks: row.carrier_bolt_banks.clone(),
            weapons: Some(expected_weapons),
            ..Default::default()
        };
        let mut ids = std::collections::BTreeSet::new();
        let valid_banks = row.carrier_bolt_banks.len() == weapons.blasters.len()
            && row.carrier_bolt_banks.iter().all(|id| {
                crate::ship::system_registry::blaster_bank_system_id(id).is_some() && ids.insert(id)
            })
            && weapons
                .blasters
                .iter()
                .all(|bank| !bank.in_flight.is_empty());
        if row != &expected
            || !valid_banks
            || (weapons.torpedoes_in_flight.is_empty() && weapons.blasters.is_empty())
        {
            return Err((
                row.uuid.clone(),
                "weapon-flight carrier must contain only its nonempty actual flight vector".into(),
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
    let saved_ids: std::collections::BTreeSet<_> = saved
        .torpedoes_in_flight
        .iter()
        .map(|round| round.uuid.as_str())
        .chain(
            saved
                .blasters
                .iter()
                .flat_map(|bank| bank.in_flight.iter().map(|round| round.id.as_str())),
        )
        .collect();
    let guided = world
        .try_query::<(Entity, &crate::console::weapons::TorpedoSystemResource)>()
        .is_none_or(|mut query| {
            query
                .iter(world)
                .filter(|(owner, _)| *owner != entity)
                .all(|(_, owner)| {
                    !owner
                        .0
                        .in_flight
                        .iter()
                        .any(|round| saved_ids.contains(round.uuid.as_str()))
                })
        });
    let bolts = world
        .try_query::<(
            Entity,
            &crate::console::weapons::blaster::BlasterSystemResource,
        )>()
        .is_none_or(|mut query| {
            query
                .iter(world)
                .filter(|(owner, _)| *owner != entity)
                .all(|(_, owner)| {
                    !owner.0.iter().any(|bank| {
                        bank.in_flight
                            .iter()
                            .any(|round| saved_ids.contains(round.id.as_str()))
                    })
                })
        });
    guided && bolts
}
