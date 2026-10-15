//! Deterministic private placement using only accepted base static body recipes.
use super::*;

pub(super) fn accepted_bodies(
    world: &crate::world::config::WorldConfig,
) -> Result<Vec<TransitGravityBody>, String> {
    let mut bodies = Vec::new();
    for row in &world.entities {
        let resolved = crate::entities::loader::resolve_entity_via(
            row,
            &crate::entities::config_cache::get_config_cache(),
            &crate::entities::loader::WasmTemplateLoader,
        )?;
        if let Some(body) =
            phoenix_content_preparation::world::transit::body_for_row(row, &resolved)?
        {
            bodies.push(body);
        }
    }
    bodies.sort_by(|a, b| a.body_id.cmp(&b.body_id));
    if bodies.len() > MAX_TRANSIT_GRAVITY_BODIES {
        return Err("base World exceeds the supported static arrival body bound".into());
    }
    if bodies
        .windows(2)
        .any(|pair| pair[0].body_id == pair[1].body_id)
    {
        return Err("arrival source identity is ambiguous".into());
    }
    Ok(bodies)
}

fn draw(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *seed;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^= value >> 31;
    f64::from((value >> 40) as u32) / 16_777_216.0
}

pub(super) fn solution(
    order: &TransitOrder,
    uuid: &str,
    owner: &SystemId,
    bodies: Vec<TransitGravityBody>,
) -> Option<TransitArrivalSolution> {
    // A stateless named draw consumes no World ID, global RNG or receipt clock.
    // Cancelling/retrying an identical same-tick order cannot reroll placement.
    let bytes = serde_json::to_vec(&(uuid, owner, order)).ok()?;
    let mut seed = 0xcbf29ce484222325_u64;
    for byte in bytes {
        seed = (seed ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    let mut direction: [f64; 3] = std::array::from_fn(|_| draw(&mut seed) * 2.0 - 1.0);
    let mut length = libm::sqrt(direction.iter().map(|value| value * value).sum());
    if length == 0.0 {
        direction = [1.0, 0.0, 0.0];
        length = 1.0;
    }
    let radius = f64::from(order.profile.config.scatter_radius) * draw(&mut seed);
    let offset = direction.map(|value| (value / length * radius) as f32);
    TransitArrivalSolution::resolve(order, offset, direction.map(|value| value as f32), bodies).ok()
}

pub(crate) fn validate_saved(
    state: &TransitState,
    uuid: &str,
    owner: &SystemId,
    world: &crate::world::config::WorldConfig,
    scopes: Option<&StaticTransitScopes>,
) -> Result<(), String> {
    let (order, arrival) = match &state.phase {
        TransitPhase::Committed { order, arrival, .. }
        | TransitPhase::Arrived { order, arrival, .. } => (order, arrival),
        _ => return Ok(()),
    };
    let bodies = if let Some(scope) = scopes.and_then(|scopes| scopes.for_order(order)) {
        scope.bodies.iter().map(|(_, body)| body.clone()).collect()
    } else if order.original_world == TransitWorldBinding::Base
        && matches!(order.destination, TransitDestination::Local { .. })
        && world.transit_destinations.is_empty()
    {
        accepted_bodies(world)?
    } else {
        return Err("saved Transit arrival lacks its accepted current/selected World scope".into());
    };
    let expected = solution(order, uuid, owner, bodies)
        .ok_or("saved accepted arrival has no supported deterministic solution")?;
    if &expected != arrival {
        return Err(
            "saved arrival disagrees with accepted static body policy or original keyed placement"
                .into(),
        );
    }
    Ok(())
}
