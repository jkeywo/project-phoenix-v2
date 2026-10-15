//! Finite recipes over the same actual stores and receiving-room claims.
use super::logistics::{CompiledLogistics, EndpointStock, ResourceEndpoint};
use crate::ship::config::ShipConfig;
pub use phoenix_model::production::*;
use std::collections::{BTreeMap, BTreeSet};

#[path = "production_state.rs"]
mod execution;
pub use execution::*;
#[path = "production_stock.rs"]
mod transactions;
pub use transactions::*;
#[path = "production_observation.rs"]
mod observation;
pub use observation::*;

pub const FABRICATION_KIND: &str = "fabrication";
pub type EndpointMap = BTreeMap<ResourceEndpoint, EndpointStock>;
pub type IncomingClaims = BTreeMap<ResourceEndpoint, BTreeMap<String, i64>>;

#[derive(Clone, Debug)]
pub struct CompiledProduction {
    config: ProductionConfig,
    logistics: CompiledLogistics,
}
impl CompiledProduction {
    pub fn compile(
        config: &ProductionConfig,
        logistics: &CompiledLogistics,
        ship: &ShipConfig,
    ) -> Result<Self, String> {
        if ship
            .systems
            .iter()
            .filter(|s| s.kind == FABRICATION_KIND)
            .count()
            != 1
            || ship
                .system(&config.system)
                .is_none_or(|s| s.kind != FABRICATION_KIND)
            || logistics
                .graph()
                .compartment_for_system(&config.system)
                .is_none()
            || config.recipes.is_empty()
        {
            return Err(
                "production requires a physically fitted fabrication System and recipes".into(),
            );
        }
        let mut ids = BTreeSet::new();
        for recipe in &config.recipes {
            if recipe.id.is_empty()
                || !recipe
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || !ids.insert(&recipe.id)
                || !recipe.work_secs.is_finite()
                || recipe.work_secs <= 0.0
                || recipe.inputs.is_empty()
                || recipe.outputs.is_empty()
            {
                return Err("production needs unique recipe identities, inputs/outputs and positive finite work".into());
            }
            for amounts in [&recipe.inputs, &recipe.outputs] {
                let mut stores = BTreeSet::new();
                for item in amounts {
                    let store = logistics.store(&item.store)?;
                    if Some(&store.compartment)
                        != logistics.graph().compartment_for_system(&config.system)
                    {
                        return Err("recipe stores must be in the fabrication compartment; deliver distant inputs first".into());
                    }
                    if item.amount <= 0
                        || !stores.insert(&item.store)
                        || item.amount > logistics.endpoints()[&store.endpoint].ceiling
                    {
                        return Err(
                            "recipe rows require unique stores and positive bounded quantities"
                                .into(),
                        );
                    }
                }
            }
        }
        let profile = Self {
            config: config.clone(),
            logistics: logistics.clone(),
        };
        profile.validate_target(config.initial_recipe.as_deref(), config.initial_target)?;
        Ok(profile)
    }
    pub fn config(&self) -> &ProductionConfig {
        &self.config
    }
    pub fn logistics(&self) -> &CompiledLogistics {
        &self.logistics
    }
    pub fn recipe(&self, id: &str) -> Result<&ProductionRecipeConfig, String> {
        self.config
            .recipes
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("unknown production recipe {id:?}"))
    }
    pub fn validate_target(&self, recipe: Option<&str>, target: i64) -> Result<(), String> {
        if target < 0 {
            return Err("production target cannot be negative".into());
        }
        let Some(id) = recipe else {
            return if target == 0 {
                Ok(())
            } else {
                Err("production without a selected recipe must have target zero".into())
            };
        };
        let primary = self.logistics.store(&self.recipe(id)?.outputs[0].store)?;
        if target > self.logistics.endpoints()[&primary.endpoint].ceiling {
            return Err("production target exceeds the actual primary stock ceiling".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "production_tests.rs"]
mod tests;
