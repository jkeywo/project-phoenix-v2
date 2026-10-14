//! Located finite stock movement; existing endpoints remain authoritative.
use crate::ship::{config::ShipConfig, interior::InteriorGraph};
pub use phoenix_model::logistics::*;
use std::collections::{BTreeMap, BTreeSet};

#[path = "logistics_stock.rs"]
mod transactions;
pub use transactions::*;
#[path = "logistics_state.rs"]
mod execution;
pub use execution::*;

pub const LOGISTICS_KIND: &str = "logistics";
#[derive(Clone, Debug)]
pub struct EndpointDefinition {
    pub ceiling: i64,
}

#[derive(Clone, Debug)]
pub struct CompiledLogistics {
    config: LogisticsConfig,
    graph: InteriorGraph,
    endpoints: BTreeMap<ResourceEndpoint, EndpointDefinition>,
}
fn named(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
impl CompiledLogistics {
    pub fn compile(
        config: &LogisticsConfig,
        ship: &ShipConfig,
        graph: &InteriorGraph,
        endpoints: &BTreeMap<ResourceEndpoint, EndpointDefinition>,
    ) -> Result<Self, String> {
        if config.handling.len() != 1 {
            return Err("logistics requires exactly one handling System".into());
        }
        let handling = &config.handling[0];
        if handling.max_active == 0
            || handling.max_batch <= 0
            || !handling.units_per_sec.is_finite()
            || handling.units_per_sec <= 0.0
            || !(handling.max_batch as f32 / handling.units_per_sec).is_finite()
            || ship
                .system(&handling.system)
                .is_none_or(|s| s.kind != LOGISTICS_KIND)
        {
            return Err(
                "logistics handling requires a fitted logistics System and positive finite limits"
                    .into(),
            );
        }
        let rooms: BTreeSet<_> = graph.config.compartments.iter().map(|r| &r.id).collect();
        let mut bays = BTreeSet::new();
        for bay in &config.storage {
            if !rooms.contains(&bay.compartment)
                || !bays.insert(&bay.compartment)
                || bay.space <= 0
                || bay.unit_space.is_empty()
                || bay
                    .unit_space
                    .iter()
                    .any(|(r, q)| r.trim().is_empty() || *q <= 0)
            {
                return Err("logistics storage requires unique real compartments and positive compatible space costs".into());
            }
        }
        let mut ids = BTreeSet::new();
        let mut located = BTreeSet::new();
        for store in &config.stores {
            store.endpoint.validate_resource(&store.resource)?;
            let endpoint = endpoints
                .get(&store.endpoint)
                .ok_or("logistics store references an absent actual endpoint")?;
            if !named(&store.id)
                || !ids.insert(&store.id)
                || !located.insert(&store.endpoint)
                || endpoint.ceiling < 0
                || store.target < 0
                || store.reserve < 0
                || store.target > endpoint.ceiling
                || store.reserve > endpoint.ceiling
            {
                return Err("logistics stores require unique identities/locations and bounded initial goals".into());
            }
            if config
                .storage
                .iter()
                .find(|b| b.compartment == store.compartment)
                .is_none_or(|b| !b.unit_space.contains_key(&store.resource))
            {
                return Err("managed store lacks compatible compartment storage".into());
            }
            match store.role {
                StoreRole::Main if store.system.is_some() || store.source.is_some() => {
                    return Err(
                        "main stores belong to Inventory; no local owner/source binding is allowed"
                            .into(),
                    )
                }
                StoreRole::Local => {
                    let owner = store
                        .system
                        .as_ref()
                        .ok_or("local store requires its actual owning System")?;
                    if ship.system(owner).is_none()
                        || graph.compartment_for_system(owner) != Some(&store.compartment)
                        || store.source.is_none()
                    {
                        return Err("local stock owner must occupy its store compartment and name a main source".into());
                    }
                    if matches!(store.endpoint, ResourceEndpoint::TorpedoMagazine {})
                        && ship.system(owner).is_none_or(|s| {
                            s.kind != crate::ship::system_registry::TORPEDO_MAGAZINE_KIND
                        })
                    {
                        return Err(
                            "magazine stock must belong to its actual magazine System".into()
                        );
                    }
                }
                _ => {}
            }
        }
        if config.stores.is_empty() {
            return Err("logistics requires managed stores".into());
        }
        for local in config.stores.iter().filter(|s| s.role == StoreRole::Local) {
            if config
                .stores
                .iter()
                .find(|s| Some(&s.id) == local.source.as_ref())
                .is_none_or(|s| s.role != StoreRole::Main || s.resource != local.resource)
            {
                return Err("local source must be a same-resource main store".into());
            }
        }
        Ok(Self {
            config: config.clone(),
            graph: graph.clone(),
            endpoints: endpoints.clone(),
        })
    }
    pub fn handling(&self) -> &HandlingConfig {
        &self.config.handling[0]
    }
    pub fn config(&self) -> &LogisticsConfig {
        &self.config
    }
    pub fn graph(&self) -> &InteriorGraph {
        &self.graph
    }
    pub fn endpoints(&self) -> &BTreeMap<ResourceEndpoint, EndpointDefinition> {
        &self.endpoints
    }
    pub fn store(&self, id: &str) -> Result<&ManagedStoreConfig, String> {
        self.config
            .stores
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("unknown managed store {id:?}"))
    }
}

#[cfg(test)]
#[path = "logistics_tests.rs"]
mod tests;
