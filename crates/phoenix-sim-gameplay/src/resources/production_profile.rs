//! Resolve finite fabrication against fitted machinery and actual managed stores.
use super::{logistics_profile::compile_entity_logistics, production::CompiledProduction};
use crate::entities::config::EntityConfig;
pub fn compile_entity_production(
    config: &EntityConfig,
) -> Result<Option<CompiledProduction>, String> {
    let Some(authored) = &config.production else {
        if config.ship_config.as_ref().is_some_and(|ship| {
            ship.systems
                .iter()
                .any(|s| s.kind == super::production::FABRICATION_KIND)
        }) {
            return Err(
                "A fitted fabrication System requires a complete [production] profile".into(),
            );
        }
        return Ok(None);
    };
    let ship = config
        .ship_config
        .as_ref()
        .ok_or("fabrication needs installed Systems")?;
    if ship
        .systems
        .iter()
        .filter(|system| system.kind == super::production::FABRICATION_KIND)
        .count()
        != 1
    {
        return Err("This build supports exactly one fabrication System per [production] profile; remove additional unowned fabricators".into());
    }
    let logistics = compile_entity_logistics(config)?
        .ok_or("fabrication needs real managed resource stores")?;
    let profile = CompiledProduction::compile(authored, &logistics, ship)?;
    if !config
        .energy
        .as_ref()
        .is_some_and(|energy| energy.consumers.iter().any(|c| c.system == authored.system))
    {
        return Err("fabrication requires an actual local-energy consumer".into());
    }
    let owner = logistics.handling().system.clone();
    let boards = std::collections::HashMap::from([(
        owner.clone(),
        crate::core::messages::SystemBlackboard::Logistics(Default::default()),
    )]);
    if ship
        .observations_for_system(&authored.system, &boards)
        .get(&owner)
        .is_none()
    {
        return Err("Fabrication needs a permitted typed Logistics feed; co-hosting does not share stock knowledge".into());
    }
    Ok(Some(profile))
}
