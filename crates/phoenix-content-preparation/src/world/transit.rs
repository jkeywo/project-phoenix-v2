//! Accepted static placement recipes, shared by activation and physical commit.
//! These are private physical inputs, not an observation publication.
use crate::entities::config::EntityConfig;
use crate::world::config::WorldEntity;
use phoenix_sim_gameplay::ship::transit::TransitGravityBody;

pub fn body_for_row(
    row: &WorldEntity,
    resolved: &EntityConfig,
) -> Result<Option<TransitGravityBody>, String> {
    let Some(policy) = &resolved.arrival_gravity else {
        return Ok(None);
    };
    phoenix_sim_gameplay::ship::transit_profile::validate_fixed_body(resolved)?;
    let name = row
        .name
        .as_ref()
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
        })
        .ok_or("arrival_gravity requires an explicit named static World row")?;
    let transform = row
        .transform
        .as_ref()
        .ok_or("arrival_gravity requires explicit absolute transform.position")?;
    let position = transform
        .position
        .filter(|position| position.iter().all(|value| value.is_finite()))
        .ok_or("arrival_gravity requires a finite explicit absolute position")?;
    if transform.anchor.is_some()
        || transform.relative_to.is_some()
        || transform.offset.is_some()
        || row.when.is_some()
        || row.when_predicate.is_some()
    {
        return Err("arrival_gravity does not support relative, anchored, random or conditional placement in this first static-body slice".into());
    }
    Ok(Some(TransitGravityBody {
        body_id: name.clone(),
        position,
        policy: policy.clone(),
    }))
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;
