//! Accepted recipe checks shared by authoring and historical opportunity input.
use crate::entities::{config::EntityConfig, loader::TemplateLoader};
use phoenix_sim_world::world::planning::opportunities::*;

pub fn recipe_input(
    source: &CompiledMaintenanceSearch,
    recipe: &EntityConfig,
    compartment: &str,
) -> Result<MaintenanceOpportunityInput, String> {
    let room = recipe
        .interior
        .as_ref()
        .and_then(|layout| {
            layout
                .compartments
                .iter()
                .find(|room| room.id == compartment)
        })
        .ok_or("maintenance room is absent from the accepted layout")?;
    let remedy = recipe
        .damage
        .as_ref()
        .and_then(|damage| damage.remedies.get(&room.structural_remedy))
        .ok_or("maintenance room has no accepted structural remedy")?;
    if !matches!(
        remedy.outcome,
        phoenix_model::damage::RemedyOutcome::Structural
    ) || remedy.requirements.len() != 1
    {
        return Err(
            "first generated maintenance supports one actual structural-part requirement".into(),
        );
    }
    let part = &remedy.requirements[0];
    let profile =
        phoenix_sim_gameplay::resources::logistics_profile::compile_entity_logistics(recipe)?
            .ok_or("maintenance requires actual fitted logistics")?;
    let main = profile.store(&source.config.source_store)?;
    let local = profile.store(&source.config.destination_store)?;
    if profile.handling().system != source.config.handling_owner
        || part.resource != source.config.commodity
        || part.amount > profile.handling().max_batch
        || main.resource != part.resource
        || local.resource != part.resource
        || main.role != phoenix_model::logistics::StoreRole::Main
        || local.role != phoenix_model::logistics::StoreRole::Local
        || local.source.as_deref() != Some(main.id.as_str())
        || local.compartment != room.id
        || phoenix_sim_gameplay::resources::service::resolve_endpoint(
            &part.resource,
            &recipe.resource_services,
            &phoenix_sim_gameplay::resources::service::ResourceLink {
                service: Some(part.service.clone()),
                provider: None,
            },
        )? != local.endpoint
    {
        return Err(
            "maintenance stores, service, part or owner disagree with actual machinery".into(),
        );
    }
    let definition = phoenix_sim_gameplay::ship::physical_damage::ShipDamage::from_config(recipe)?
        .ok_or("missing accepted damage owner")?
        .definition;
    let target = phoenix_model::messages::RepairTarget::Work(
        phoenix_model::damage::RepairWorkTarget::Compartment {
            compartment: compartment.into(),
        },
    );
    let visit = phoenix_sim_gameplay::jobs::activity_bindings::resolve_repair_visit(
        &target,
        recipe.ship_config.as_ref().ok_or("missing accepted ship")?,
        &definition,
    )
    .ok_or("missing actual room visit")?;
    let input = MaintenanceOpportunityInput {
        compartment: compartment.into(),
        hp: 0.0,
        max_hp: f64::from(room.max_hp),
        remedy_hp: f64::from(
            remedy
                .structural_hp
                .ok_or("structural remedy lacks its actual HP effect")?,
        ),
        required_parts: part.amount,
        local_available: 0,
        local_headroom: part.amount,
        source_available_above_floor: part.amount,
        known: true,
        report: Some(
            phoenix_model::observation::ObservationMetadata::at(
                0,
                phoenix_model::observation::ObservationSubject::System { id: visit },
                phoenix_model::observation::ObservationProvenance {
                    kind: phoenix_model::observation::ObservationSource::RepairTeam,
                    system: Some(source.config.repair_owner.clone()),
                    label: String::new(),
                },
                phoenix_model::observation::ObservationUncertainty::Qualitative,
            )
            .received(0),
        ),
        repair_idle: true,
        handling_idle: true,
        route_available: true,
        outstanding_supply: false,
    };
    let reference = phoenix_model::comms_request::OpportunityRef {
        source: source.config.id.clone(),
        ordinal: 1,
        actor: "accepted-source-preview".into(),
    };
    let result = source
        .search(&input)?
        .ok_or("accepted maintenance operator cannot improve its actual room")?;
    let graph = source.derive(&reference, &input, &result)?;
    for config in graph.leaves.values() {
        if !super::activity_bindings::supports(&config.request, recipe)? {
            return Err("generated operator is outside an offered hull's actual capability".into());
        }
    }
    Ok(input)
}
pub fn validate_input(
    source: &CompiledMaintenanceSearch,
    recipe: &EntityConfig,
    input: &MaintenanceOpportunityInput,
) -> Result<(), String> {
    source.validate_input(input)?;
    let expected = recipe_input(source, recipe, &input.compartment)?;
    if let Some(report) = &input.report {
        let subject = match &report.subject {
            phoenix_model::observation::ObservationSubject::System { id } => id,
            _ => return Err("maintenance report lacks its exact own System subject".into()),
        };
        let reporter = report
            .provenance
            .system
            .as_ref()
            .ok_or("maintenance report lacks its original reporting System")?;
        let ship = recipe
            .ship_config
            .as_ref()
            .ok_or("maintenance report lacks accepted owning ship")?;
        let permitted = match report.provenance.kind {
            phoenix_model::observation::ObservationSource::SystemReport => reporter == subject,
            phoenix_model::observation::ObservationSource::RepairTeam => {
                reporter == &source.config.repair_owner
            }
            _ => false,
        };
        if !permitted
            || ship.system(reporter).is_none()
            || recipe
                .interior
                .as_ref()
                .and_then(|layout| layout.placements.get(subject))
                .map(String::as_str)
                != Some(input.compartment.as_str())
        {
            return Err(
                "maintenance report subject/source is outside its accepted own room".into(),
            );
        }
    }

    let profile =
        phoenix_sim_gameplay::resources::logistics_profile::compile_entity_logistics(recipe)?
            .ok_or("missing actual logistics")?;
    let main = profile.store(&source.config.source_store)?;
    let local = profile.store(&source.config.destination_store)?;
    if input.max_hp != expected.max_hp
        || input.remedy_hp != expected.remedy_hp
        || input.required_parts != expected.required_parts
        || input.local_available > profile.endpoints()[&local.endpoint].ceiling
        || input.local_headroom > profile.endpoints()[&local.endpoint].ceiling
        || input.source_available_above_floor > profile.endpoints()[&main.endpoint].ceiling
    {
        return Err("original maintenance input replaces accepted recipe limits or effect".into());
    }
    Ok(())
}
pub(super) fn findings(
    source: &super::validate::WorldSource,
    loader: &dyn TemplateLoader,
) -> Vec<super::validate::WorldFinding> {
    let Some(authored) = &source.config.opportunity_search else {
        return Vec::new();
    };
    let check = || -> Result<(), String> {
        if authored.actor.starts_with("slot:") {
            return Err(
                "first maintenance opportunity actor must be a declared base-world actor".into(),
            );
        }
        source.config.validate_situation_actor(&authored.actor)?;
        let compiled = source
            .config
            .compiled_opportunity_search
            .as_ref()
            .ok_or("maintenance operator imports must be resolved by the content reader")?;
        if &compiled.config != authored {
            return Err("compiled maintenance source differs from authored source".into());
        }
        if source
            .config
            .situations
            .iter()
            .any(|row| row.id.starts_with("htn-optional/"))
        {
            return Err("authored situations cannot occupy generated execution namespaces".into());
        }
        for recipe in super::activity_bindings::candidates(&authored.actor, source, loader)? {
            for room in &authored.compartments {
                recipe_input(compiled, &recipe, room)?;
            }
        }
        Ok(())
    };
    match check() {
        Ok(()) => Vec::new(),
        Err(error) if !loader.absence_is_final() && error.starts_with("cannot resolve") => {
            Vec::new()
        }
        Err(error) => vec![super::validate::WorldFinding::error(
            "unsupported-maintenance-opportunity",
            &source.path,
            source.toml,
            "opportunity_search",
            error,
        )],
    }
}
