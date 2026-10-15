//! A disposable projection of actual own maintenance owners and reports.
use crate::resources::logistics_server::{CompiledShipLogistics, ShipLogistics};
use phoenix_sim_world::world::planning::opportunities::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn project(
    source: &CompiledMaintenanceSearch,
    now: u64,
    config: &crate::ship_plugin::ShipConfigComponent,
    damage: &crate::ship::compartment_damage::ShipDamage,
    teams: &crate::console::repair::server::ShipRepairTeams,
    reports: &crate::console::repair::server::RepairRequestQueue,
    profile: &CompiledShipLogistics,
    logistics: &ShipLogistics,
    inventory: &crate::infrastructure::InfrastructureCondition,
    torpedoes: Option<&crate::console::weapons::torpedo::TorpedoSystemResource>,
    services: &crate::resources::server::ResourceServices,
    access: Option<&crate::ship::access::InteriorAccess>,
    boards: &crate::server_app::ShipSystemBlackboards,
) -> Result<Vec<MaintenanceOpportunityInput>, String> {
    if damage.catastrophic() {
        return Ok(Vec::new());
    }
    let permitted = config
        .0
        .observations_for_system(&source.config.repair_owner, &boards.0);
    let has_repair_view = matches!(
        permitted.get(&source.config.repair_owner),
        Some(crate::core::messages::SystemBlackboard::Repair(_))
    );
    let stocks =
        crate::resources::logistics_server::endpoint_view(&profile.0, Some(inventory), torpedoes)?;
    profile.0.validate_stocks(&stocks)?;
    let main = profile.0.store(&source.config.source_store)?;
    let local = profile.0.store(&source.config.destination_store)?;
    let from = stocks
        .get(&main.endpoint)
        .ok_or("missing actual source stock")?;
    let to = stocks
        .get(&local.endpoint)
        .ok_or("missing actual local stock")?;
    let layout = teams
        .0
        .layout()
        .ok_or("maintenance requires actual team routes")?;
    let graph = phoenix_sim_gameplay::ship::interior::InteriorGraph::compile(
        layout,
        &config.0,
        teams.0.slots().len(),
    )?;
    let blocked = access
        .map(|access| access.blocked_connections(layout))
        .unwrap_or_default();
    let free = teams.0.free_team_indices(teams.0.external_team());
    let repair_idle =
        free.len() == teams.0.slots().len() && free.contains(&usize::from(source.config.team));
    let handling_idle = logistics.0.enabled
        && logistics.0.batches.is_empty()
        && graph
            .route(&main.compartment, &local.compartment, &blocked)
            .is_some();
    let outstanding_supply = to.stock.incoming_total() > 0
        || to.stock.reserved_total() > 0
        || !logistics.0.batches.is_empty();

    let mut inputs = Vec::new();
    for compartment in &source.config.compartments {
        let room = damage
            .definition
            .compartment(compartment)
            .ok_or("unknown actual maintenance room")?;
        let state = damage
            .state
            .compartments
            .get(compartment)
            .ok_or("missing actual compartment state")?;
        let remedy = damage
            .definition
            .config
            .remedies
            .get(&room.structural_remedy)
            .ok_or("missing actual structural remedy")?;
        if remedy.requirements.len() != 1 {
            return Err("unsupported structural part requirements".into());
        }
        let part = &remedy.requirements[0];
        if services.0.get(&part.service) != Some(&local.endpoint)
            || part.resource != source.config.commodity
            || local.compartment != *compartment
        {
            return Err("actual maintenance resource binding differs from its source".into());
        }
        let target = phoenix_model::messages::RepairTarget::Work(
            phoenix_model::damage::RepairWorkTarget::Compartment {
                compartment: compartment.clone(),
            },
        );
        let route_available = teams
            .0
            .navigation()
            .and_then(|rows| rows.get(usize::from(source.config.team)))
            .is_some_and(|navigation| match &navigation.location {
                phoenix_model::interior::TeamLocation::Compartment { compartment: from } => {
                    graph.route(from, compartment, &blocked).is_some()
                }
                _ => false,
            });
        let report = has_repair_view
            .then(|| {
                crate::jobs::activity_bindings::resolve_repair_visit(
                    &target,
                    &config.0,
                    &damage.definition,
                )
            })
            .flatten()
            .and_then(|visit| {
                let group = crate::modifiers::repair_teams::repair_group(&visit, &config.0);
                reports
                    .entries
                    .iter()
                    .filter(|entry| entry.group == group)
                    .flat_map(|entry| {
                        entry
                            .observations
                            .iter()
                            .chain(entry.readings.iter().map(|reading| &reading.source))
                    })
                    .filter(|report| {
                        let subject = match &report.subject {
                            phoenix_model::observation::ObservationSubject::System { id } => id,
                            _ => return false,
                        };
                        let Some(reporter) = &report.provenance.system else {
                            return false;
                        };
                        let permitted = match report.provenance.kind {
                            phoenix_model::observation::ObservationSource::SystemReport => {
                                reporter == subject
                            }
                            phoenix_model::observation::ObservationSource::RepairTeam => {
                                reporter == &source.config.repair_owner
                            }
                            _ => false,
                        };
                        permitted
                            && report.is_valid()
                            && report.observed_tick.zip(report.received_tick).is_some_and(
                                |(observed, received)| observed <= received && received <= now,
                            )
                            && damage.definition.interior.placements.get(subject)
                                == Some(compartment)
                    })
                    .max_by_key(|report| (report.received_tick, report.observed_tick))
                    .cloned()
            });
        let known = report.is_some();
        inputs.push(MaintenanceOpportunityInput {
            compartment: compartment.clone(),
            hp: state.hp,
            max_hp: f64::from(room.max_hp),
            remedy_hp: f64::from(
                remedy
                    .structural_hp
                    .ok_or("missing actual structural HP effect")?,
            ),
            required_parts: part.amount,
            local_available: to.stock.available(),
            local_headroom: profile.0.receiving_headroom(&stocks, &local.id)?,
            source_available_above_floor: from
                .stock
                .available()
                .saturating_sub(
                    *logistics
                        .0
                        .floors
                        .get(&main.id)
                        .ok_or("missing actual source floor")?,
                )
                .max(0),
            known,
            report,
            repair_idle,
            handling_idle,
            route_available,
            outstanding_supply,
        });
    }
    Ok(inputs)
}
