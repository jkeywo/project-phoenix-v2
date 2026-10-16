//! Checked physical marker continuation at both existing restore doors.
use super::*;
use bevy::ecs::query::Allow;
use phoenix_model::marking::{FrozenMarkingPayload, MarkingLocation};
use phoenix_sim_gameplay::weapons::marking::ShipMarkingAttachment;

fn live(world: &World, id: &str) -> Option<Entity> {
    let mut query = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>(
        )?;
    query
        .iter(world)
        .find_map(|(entity, uuid)| (uuid.0 == id).then_some(entity))
}
fn accepted_payload(
    payload: &FrozenMarkingPayload,
) -> Result<crate::entities::config::EntityConfig, String> {
    payload.validate()?;
    let config = crate::entities::config_cache::get_cached_entity_config(&payload.recipe.template)
        .ok_or("marker payload is outside accepted frozen content closure")?;
    phoenix_sim_gameplay::weapons::marking::validate_payload_recipe(&payload.recipe, &config)?;
    Ok(config)
}
fn minted(
    id: &str,
    namespace: crate::world_id::IdNamespace,
) -> Result<crate::world_id::WorldId, String> {
    crate::world_id::WorldId::parse(id)
        .filter(|value| value.namespace == namespace && value.render() == id)
        .ok_or_else(|| "marker causal identity has the wrong actual mint namespace".into())
}
fn before_frontier(
    id: crate::world_id::WorldId,
    mint: &crate::world_id::WorldIdMintState,
    tick: u64,
) -> Result<(), String> {
    if id.tick > mint.tick
        || mint.tick > tick
        || mint
            .next_seq
            .iter()
            .any(|next| *next > crate::world_id::SEQ_LIMIT)
        || id.tick == mint.tick && id.seq >= mint.next_seq[id.namespace.code() as usize]
    {
        return Err("marker causal identity is beyond the actual mint frontier".into());
    }
    Ok(())
}
fn causal(
    row: &EntityState,
    mint: Option<&crate::world_id::WorldIdMintState>,
    tick: u64,
) -> Result<(), String> {
    let Some(state) = &row.marking_attachment else {
        return Ok(());
    };
    use crate::world_id::IdNamespace::{Entity, Projectile};
    let body = minted(&row.uuid, Entity)?;
    let round = minted(&state.round, Projectile)?;
    let source = minted(&state.source, Entity)?;
    if body.tick != state.contact_tick
        || round.tick > state.contact_tick
        || source.tick > round.tick
    {
        return Err("marker causal mint clocks disagree with actual contact/launch".into());
    }
    if let MarkingLocation::Attached { subject, .. } = &state.location {
        let parent = minted(subject, Entity)?;
        if parent.tick > state.contact_tick || parent.tick == body.tick && parent.seq >= body.seq {
            return Err("marker subject was not present before actual contact".into());
        }
        if let Some(mint) = mint {
            before_frontier(parent, mint, tick)?;
        }
    }
    if let Some(mint) = mint {
        for id in [body, round, source] {
            before_frontier(id, mint, tick)?;
        }
    }
    Ok(())
}
fn profiles(row: &EntityState) -> Result<(), String> {
    let Some(weapons) = &row.weapons else {
        return Ok(());
    };
    for (profile, source) in weapons
        .torpedoes_in_flight
        .iter()
        .map(|round| (&round.profile, &round.source_uuid))
        .chain(
            weapons
                .bursts
                .iter()
                .map(|burst| (&burst.profile, &burst.source_uuid)),
        )
    {
        if let Some(payload) = &profile.marking {
            profile.validate()?;
            accepted_payload(payload)?;
            if source.as_ref().is_none_or(|source| source.is_empty()) {
                return Err("frozen marker payload requires its actual original launcher".into());
            }
        }
    }
    Ok(())
}
fn pose(value: [f32; 8]) -> crate::ship::state::ShipPhysics {
    crate::ship::state::ShipPhysics {
        x: value[0],
        y: value[1],
        z: value[2],
        yaw: value[3],
        forward_speed: value[4],
        roll: value[5],
        lateral_speed: value[6],
        vertical_speed: value[7],
    }
}
fn values(pose: crate::ship::state::ShipPhysics) -> [f32; 8] {
    [
        pose.x,
        pose.y,
        pose.z,
        pose.yaw,
        pose.forward_speed,
        pose.roll,
        pose.lateral_speed,
        pose.vertical_speed,
    ]
}
fn parent(
    world: &World,
    id: &str,
    candidates: Option<&[EntityState]>,
) -> Result<
    (
        crate::ship::state::ShipPhysics,
        crate::entities::config::ColliderConfig,
    ),
    String,
> {
    let actual = live(world, id);
    if let Some(rows) = candidates {
        let row = rows
            .iter()
            .find(|row| row.uuid == id)
            .ok_or("attached marker has no actual candidate subject")?;
        if row.marking_attachment.is_some() || row.probe_custody.is_some() {
            return Err("marker cannot attach to another marker or carry-capable subject".into());
        }
        let section = if let Some(origin) = &row.spawn {
            let config = origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or("candidate marker subject recipe cannot resolve")?;
            if config.marking_beacon.is_some() || config.probe.is_some() {
                return Err("candidate subject is not a supported independent hull".into());
            }
            config
                .collider
                .ok_or("candidate subject has no accepted collider")?
        } else {
            if actual.is_some_and(|entity| {
                world
                    .get::<crate::entities::craft::PhysicalProbe>(entity)
                    .is_some()
                    || world
                        .get::<crate::entities::craft::ProbeCustody>(entity)
                        .is_some()
            }) {
                return Err("candidate subject has actual carry-capable equipment".into());
            }
            actual
                .and_then(|entity| {
                    world
                        .get::<crate::entities::spawner::ColliderSection>(entity)
                        .map(|section| section.0.clone())
                })
                .ok_or("marker subject has no accepted physical hull")?
        };
        if row.physical_damage.is_none() {
            return Err("marker subject has no actual compartment damage owner".into());
        }
        let p = row
            .physics
            .ok_or("marker subject has no actual physical pose")?;
        return Ok((pose(p), section));
    }
    let entity = actual.ok_or("attached marker subject is absent")?;
    if world.get::<ShipMarkingAttachment>(entity).is_some()
        || world
            .get::<crate::entities::craft::PhysicalProbe>(entity)
            .is_some()
        || world
            .get::<crate::entities::craft::ProbeCustody>(entity)
            .is_some()
        || world
            .get::<crate::ship::compartment_damage::ShipDamage>(entity)
            .is_none()
    {
        return Err("marker subject is not an eligible actual hull".into());
    }
    Ok((
        *world
            .get::<crate::ship::state::ShipPhysics>(entity)
            .ok_or("marker subject has no actual pose")?,
        world
            .get::<crate::entities::spawner::ColliderSection>(entity)
            .ok_or("marker subject has no actual collider")?
            .0
            .clone(),
    ))
}
/// A successful emission retains its original own sensor token, not a copied
/// receipt. Tracks may have aged out; the existing contact allocation frontier
/// still bounds that historical token without creating a report ledger.
fn validate_report_frontier(
    row: &EntityState,
    equipment: &phoenix_model::marking::MarkingBeaconConfig,
) -> Result<(), String> {
    let Some(report) = row
        .marking_attachment
        .as_ref()
        .and_then(|state| state.report.as_ref())
    else {
        return Ok(());
    };
    let refuse =
        || "marker report is not an original emitted sample of its actual sensor".to_string();
    let state = row
        .broad
        .as_ref()
        .and_then(|owners| owners.get(&equipment.sensor))
        .ok_or_else(refuse)?;
    if state.owner != equipment.sensor
        || report.emitted_tick != report.observed_tick
        || state
            .last_sample_tick
            .is_none_or(|last| last < report.observed_tick)
        || state
            .last_tick
            .is_none_or(|last| last < report.emitted_tick)
    {
        return Err(refuse());
    }
    let source = phoenix_sim_gameplay::science::sharing::origin_handle(&row.uuid);
    let prefix = format!(
        "{}:{}{}:{}",
        source.len(),
        source,
        equipment.sensor.0.len(),
        equipment.sensor.0
    );
    let tail = report.sample.strip_prefix(&prefix).ok_or_else(refuse)?;
    let (length, rest) = tail.split_once(':').ok_or_else(refuse)?;
    let length = length
        .parse::<usize>()
        .ok()
        .filter(|length| *length <= 512)
        .ok_or_else(refuse)?;
    let contact = rest.get(..length).ok_or_else(refuse)?;
    if rest.get(length..) != Some(format!(":{}", report.observed_tick).as_str()) {
        return Err(refuse());
    }
    let contact_prefix = format!(
        "{}:{}:contact-",
        equipment.sensor.0.len(),
        equipment.sensor.0
    );
    let sequence = contact
        .strip_prefix(&contact_prefix)
        .and_then(|n| n.parse::<u64>().ok())
        .filter(|n| *n > 0 && *n < state.next_contact)
        .ok_or_else(refuse)?;
    let origin = phoenix_model::sensing_sharing::RadarSampleOrigin {
        source,
        system: equipment.sensor.clone(),
        contact: format!("{contact_prefix}{sequence}"),
    };
    if report.sample
        != phoenix_sim_gameplay::science::sharing::sample_id(&origin, report.observed_tick)
    {
        return Err(refuse());
    }
    Ok(())
}
fn validate(
    world: &World,
    row: &EntityState,
    tick: u64,
    candidates: Option<&[EntityState]>,
    check_parent: bool,
) -> Result<(), String> {
    profiles(row)?;
    causal(row, None, tick)?;
    let actual = live(world, &row.uuid);
    let birth = row.spawn.as_ref().and_then(|origin| {
        crate::entities::config_cache::get_cached_entity_config(&origin.template_path)
    });
    let equipment = birth
        .as_ref()
        .and_then(|config| config.marking_beacon.as_ref())
        .or_else(|| {
            actual.and_then(|entity| {
                world
                    .get::<crate::console::weapons::marking::MarkingBeaconProfile>(entity)
                    .map(|profile| &profile.0)
            })
        });
    let required = actual
        .is_some_and(|entity| world.get::<ShipMarkingAttachment>(entity).is_some())
        || row.spawn.is_some()
            && birth
                .as_ref()
                .is_some_and(|config| config.marking_beacon.is_some());
    if required && row.marking_attachment.is_none() {
        return Err("actual runtime marker is missing its causal attachment owner".into());
    }
    let Some(state) = &row.marking_attachment else {
        return Ok(());
    };
    let equipment = equipment.ok_or("foreign marker state has no actual accepted equipment")?;
    state.validate(&row.uuid, tick)?;
    validate_report_frontier(row, equipment)?;
    let config = accepted_payload(&state.payload)?;
    if config.marking_beacon.as_ref() != Some(equipment) {
        return Err("marker state disagrees with actual accepted equipment".into());
    }
    let origin = row
        .spawn
        .as_ref()
        .ok_or("runtime marker has no immutable ordinary spawn origin")?;
    let mut exact_overrides = toml::map::Map::new();
    exact_overrides.insert(
        "faction".into(),
        toml::Value::String(state.payload.faction.to_string()),
    );
    if !origin.name.is_empty()
        || origin.rotation.is_some()
        || origin.scale.is_some()
        || origin.layer_path.is_some()
        || origin.position.iter().any(|v| !v.is_finite())
        || origin.overrides.as_ref() != Some(&toml::Value::Table(exact_overrides))
    {
        return Err("marker has a foreign runtime birth/override".into());
    }
    let resolved = origin
        .resolve(
            &crate::entities::loader::WasmTemplateLoader,
            &mut Vec::new(),
        )
        .ok_or("marker birth cannot resolve")?;
    if origin.template_path != state.payload.recipe.template
        || resolved.marking_beacon != config.marking_beacon
        || resolved.faction != Some(state.payload.faction)
    {
        return Err("marker birth differs from frozen consumed payload/faction".into());
    }
    let physical = row.physics.ok_or("marker has no actual physical pose")?;
    if physical.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite marker pose".into());
    }
    match &state.location {
        MarkingLocation::Attached { subject, offset } if check_parent => {
            let (subject_pose, collider) = parent(world, subject, candidates)?;
            let shape = crate::entities::spawner::physical_collider(&collider);
            let point = Vec3::from_array(*offset);
            let surface = shape
                .project_point(Vec3::ZERO, Quat::IDENTITY, point, false)
                .point;
            if point.distance(surface) > 0.0001 * (1.0 + point.length()) {
                return Err("marker offset is outside actual subject surface contact".into());
            }
            let expected =
                crate::ship::physics_systems::marking_attached_pose(&subject_pose, *offset);
            if physical != values(expected) {
                return Err("attached saved pose is not the checked rigid projection".into());
            }
        }
        MarkingLocation::Detached { .. }
            if physical[4] != 0.0 || physical[6] != 0.0 || physical[7] != 0.0 =>
        {
            return Err("detached inert marker has competing propulsion".into());
        }
        _ => {}
    }
    Ok(())
}
/// Disposable materialization sees accepted row shape, not the complete parent
/// roster. Full-candidate/actual single-row joins run before either write door.
pub(super) fn validate_row(world: &World, row: &EntityState) -> Result<(), String> {
    validate(world, row, u64::MAX, None, false)
}
pub(super) fn validate_single(
    world: &World,
    _: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    validate(world, row, tick, None, true)?;
    if row.marking_attachment.is_some() {
        let mint = world
            .get_resource::<crate::world_id::WorldIdMint>()
            .ok_or("actual marker restore has no live mint frontier")?
            .state();
        causal(row, Some(&mint), tick)?;
    }
    if let Some(state) = &row.marking_attachment {
        let duplicate = world.try_query_filtered::<(&EntityUuid, &ShipMarkingAttachment), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .is_some_and(|mut query| query.iter(world).any(|(id, other)| id.0 != row.uuid && other.0.round == state.round));
        if duplicate {
            return Err("consumed marking round already belongs to another body".into());
        }
        let flying = world.try_query_filtered::<&crate::console::weapons::TorpedoSystemResource, Allow<bevy::ecs::entity_disabling::Disabled>>()
            .is_some_and(|mut query| query.iter(world).any(|owner| owner.0.in_flight.iter().any(|round| round.uuid == state.round)));
        if flying {
            return Err("consumed marking round is still owned by an actual flight vector".into());
        }
    }
    Ok(())
}
pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut rounds = std::collections::BTreeSet::new();
    for row in &snapshot.entities {
        let checked = (|| {
            validate(world, row, snapshot.tick, Some(&snapshot.entities), true)?;
            if row.marking_attachment.is_some() {
                causal(
                    row,
                    Some(
                        snapshot
                            .mint
                            .as_ref()
                            .ok_or("runtime marker requires its captured mint frontier")?,
                    ),
                    snapshot.tick,
                )?;
            }
            if let Some(state) = &row.marking_attachment {
                if !rounds.insert(&state.round)
                    || snapshot
                        .entities
                        .iter()
                        .filter_map(|row| row.weapons.as_ref())
                        .any(|weapons| {
                            weapons
                                .torpedoes_in_flight
                                .iter()
                                .any(|round| round.uuid == state.round)
                        })
                {
                    return Err("consumed marking round has duplicate body/flight ownership".into());
                }
            }
            Ok(())
        })();
        checked.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "marking_tests.rs"]
mod tests;
