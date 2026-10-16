//! Derived finite demand from the existing accepted plan and own Console feed.
//! No stored goal, resource claim, Job or controller is created here.
use super::logistics_server::{CompiledShipLogistics, ShipLogistics};
use crate::core::messages::{SystemBlackboard, SystemId};
use crate::resources::logistics::{
    CompiledLogistics, EndpointStock, LogisticsState, ResourceEndpoint,
};
use phoenix_model::activity::PlannedDeliveryRequest;
use phoenix_model::logistics_ui::{DeliveryRequest, LogisticsStoreView};
use std::collections::{BTreeMap, BTreeSet};

#[allow(clippy::too_many_arguments)]
pub(super) fn permitted_current(
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    base: Option<&crate::world::config::WorldConfig>,
    actor: &str,
    owner: &SystemId,
    config: &crate::ship_plugin::ShipConfigComponent,
    boards: &crate::server_app::ShipSystemBlackboards,
    tick: u64,
) -> Vec<PlannedDeliveryRequest> {
    let permitted = config.0.observations_for_system(owner, &boards.0);
    let Some(SystemBlackboard::Logistics(view)) = permitted.get(owner) else {
        return Vec::new();
    };
    let current = crate::world::planning::delivery_requests(runtime, base, actor, owner, tick);
    intersect(current, &view.planned_requests)
}

fn intersect(
    current: Vec<PlannedDeliveryRequest>,
    visible: &[PlannedDeliveryRequest],
) -> Vec<PlannedDeliveryRequest> {
    // Exact equality includes original scope, release clock, commodity and amount.
    // Current World projection removes accepted/terminal/unreleased requests.
    current
        .into_iter()
        .filter(|row| visible.contains(row))
        .collect()
}

fn supported<'a>(
    profile: &'a CompiledLogistics,
    state: &LogisticsState,
    row: &PlannedDeliveryRequest,
) -> Option<(
    &'a phoenix_model::logistics::ManagedStoreConfig,
    &'a phoenix_model::logistics::ManagedStoreConfig,
)> {
    let source = profile.store(&row.source).ok()?;
    let destination = profile.store(&row.destination).ok()?;
    (state.enabled
        && state.batches.len() < usize::from(profile.handling().max_active)
        && row.amount > 0
        && row.amount <= profile.handling().max_batch
        && source.role == phoenix_model::logistics::StoreRole::Main
        && destination.role == phoenix_model::logistics::StoreRole::Local
        && destination.source.as_deref() == Some(source.id.as_str())
        && source.resource == row.commodity
        && destination.resource == row.commodity
        && !state
            .batches
            .values()
            .any(|batch| batch.original_destination == row.destination))
    .then_some((source, destination))
}

/// Recommendation uses only the same permitted stock rows as routine refill.
pub(super) fn candidate(
    profile: &CompiledShipLogistics,
    state: &ShipLogistics,
    stores: &[LogisticsStoreView],
    rows: &[PlannedDeliveryRequest],
) -> Option<DeliveryRequest> {
    rows.iter().find_map(|row| {
        let (from, to) = supported(&profile.0, &state.0, row)?;
        let source = stores.iter().find(|store| store.id == from.id)?;
        let destination = stores.iter().find(|store| store.id == to.id)?;
        (source.resource == row.commodity
            && destination.resource == row.commodity
            && source
                .available
                .min(source.level.saturating_sub(source.reserve))
                >= row.amount
            && destination.incoming == 0
            && destination.held == 0
            && destination.headroom.min(destination.bay_free_space) >= row.amount)
            .then(|| DeliveryRequest {
                source: row.source.clone(),
                destination: row.destination.clone(),
                amount: row.amount,
            })
    })
}

/// Recheck actual physical owners before the existing dispatch transaction.
/// A whole exact batch is required; no smaller unbound Job can claim this scope.
pub(super) fn dispatch_amount(
    profile: &CompiledLogistics,
    state: &LogisticsState,
    stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
    blocked: &BTreeSet<String>,
    request: &DeliveryRequest,
    rows: &[PlannedDeliveryRequest],
) -> Option<i64> {
    rows.iter().find_map(|row| {
        if row.source != request.source
            || row.destination != request.destination
            || row.amount != request.amount
        {
            return None;
        }
        let (from, to) = supported(profile, state, row)?;
        let source = stocks.get(&from.endpoint)?;
        let destination = stocks.get(&to.endpoint)?;
        let floor = *state.floors.get(&from.id)?;
        let room = profile.receiving_headroom(stocks, &to.id).ok()?;
        (source
            .stock
            .available()
            .min(source.stock.level.saturating_sub(floor))
            >= row.amount
            && destination.stock.incoming_total() == 0
            && destination.held == 0
            && destination
                .stock
                .headroom()
                .saturating_sub(destination.held)
                .min(room)
                >= row.amount
            && profile
                .graph()
                .route(&from.compartment, &to.compartment, blocked)
                .is_some())
        .then_some(row.amount)
    })
}

#[cfg(test)]
#[path = "logistics_mission_tests.rs"]
mod tests;
