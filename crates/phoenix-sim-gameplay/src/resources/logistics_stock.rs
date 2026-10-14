use super::*;
use crate::resources::Stock;

/// A transaction-local reading of an existing owner, never saved by Logistics.
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointStock {
    pub stock: Stock,
    pub held: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogisticsStockAction {
    Dispatch {
        source: String,
        destination: String,
        requested: i64,
        floor: i64,
        claim: String,
    },
    Receive {
        destination: String,
        amount: i64,
        claim: String,
    },
    Retarget {
        previous: String,
        destination: String,
        amount: i64,
        claim: String,
    },
}

impl CompiledLogistics {
    pub fn validate_stocks(
        &self,
        stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
    ) -> Result<(), String> {
        for store in &self.config.stores {
            let value = stocks
                .get(&store.endpoint)
                .ok_or("managed endpoint has no actual stock")?;
            value.stock.validate()?;
            if value.held < 0
                || value.stock.ceiling != self.endpoints[&store.endpoint].ceiling
                || value
                    .stock
                    .level
                    .checked_add(value.stock.incoming_total())
                    .and_then(|v| v.checked_add(value.held))
                    .is_none_or(|v| v > value.stock.ceiling)
            {
                return Err(
                    "managed stock, incoming and held occupancy disagree with authored endpoint"
                        .into(),
                );
            }
        }
        for bay in &self.config.storage {
            if self.occupied_space(stocks, &bay.compartment)? > bay.space {
                return Err("managed stocks overfill shared compartment storage".into());
            }
        }
        Ok(())
    }
    pub fn occupied_space(
        &self,
        stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
        room: &str,
    ) -> Result<i64, String> {
        let bay = self
            .config
            .storage
            .iter()
            .find(|b| b.compartment == room)
            .ok_or("unknown storage compartment")?;
        let mut used = 0i64;
        for store in self.config.stores.iter().filter(|s| s.compartment == room) {
            let value = stocks
                .get(&store.endpoint)
                .ok_or("missing managed endpoint stock")?;
            let quantity = value
                .stock
                .level
                .checked_add(value.stock.incoming_total())
                .and_then(|v| v.checked_add(value.held))
                .ok_or("storage occupancy overflows")?;
            used = used
                .checked_add(
                    quantity
                        .checked_mul(bay.unit_space[&store.resource])
                        .ok_or("storage space overflows")?,
                )
                .ok_or("storage space overflows")?;
        }
        Ok(used)
    }
    /// Additional ordinary loading room; callers also honour endpoint-specific
    /// restrictions such as tube-held rounds. No quantity is claimed here.
    pub fn receiving_headroom(
        &self,
        stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
        store: &str,
    ) -> Result<i64, String> {
        let store = self.store(store)?;
        let value = stocks
            .get(&store.endpoint)
            .ok_or("missing managed endpoint stock")?;
        let bay = self
            .config
            .storage
            .iter()
            .find(|b| b.compartment == store.compartment)
            .expect("compiled bay");
        let space = (bay.space - self.occupied_space(stocks, &bay.compartment)?).max(0)
            / bay.unit_space[&store.resource];
        Ok(value
            .stock
            .headroom()
            .saturating_sub(value.held)
            .max(0)
            .min(space))
    }
}

/// Apply one atomic mutation to a temporary view of the caller's actual owners.
/// A caller commits all changed Inventory/magazine owners together only on Ok.
pub fn apply_stock_action(
    profile: &CompiledLogistics,
    action: &LogisticsStockAction,
    stocks: &mut BTreeMap<ResourceEndpoint, EndpointStock>,
) -> Result<i64, String> {
    profile.validate_stocks(stocks)?;
    let mut staged = stocks.clone();
    let moved = match action {
        LogisticsStockAction::Dispatch {
            source,
            destination,
            requested,
            floor,
            claim,
        } => {
            let source = profile.store(source)?;
            let destination = profile.store(destination)?;
            if source.endpoint == destination.endpoint
                || source.resource != destination.resource
                || *requested <= 0
                || *floor < 0
                || claim.is_empty()
            {
                return Err(
                    "delivery requires distinct same-resource stores and positive quantity".into(),
                );
            }
            let from = &staged[&source.endpoint];
            let to = &staged[&destination.endpoint];
            if from.stock.incoming.contains_key(claim) || to.stock.incoming.contains_key(claim) {
                return Err("delivery claim already exists".into());
            }
            let available = from
                .stock
                .available()
                .min(from.stock.level.saturating_sub(*floor).max(0));
            let space = if source.compartment == destination.compartment {
                i64::MAX
            } else {
                profile.receiving_headroom(&staged, &destination.id)?
            };
            let q = (*requested)
                .min(available)
                .min(to.stock.headroom().saturating_sub(to.held).max(0))
                .min(space);
            if q == 0 {
                return Ok(0);
            }
            let from = &mut staged
                .get_mut(&source.endpoint)
                .expect("resolved source")
                .stock;
            let key = format!("{claim}/departure");
            from.reserve(&key, q)?;
            from.consume(&key).expect("just secured source");
            staged
                .get_mut(&destination.endpoint)
                .expect("resolved destination")
                .stock
                .reserve_incoming(claim, q)?;
            q
        }
        LogisticsStockAction::Receive {
            destination,
            amount,
            claim,
        } => {
            let destination = profile.store(destination)?;
            staged
                .get_mut(&destination.endpoint)
                .ok_or("missing destination")?
                .stock
                .receive(claim, *amount)?;
            *amount
        }
        LogisticsStockAction::Retarget {
            previous,
            destination,
            amount,
            claim,
        } => {
            let previous = profile.store(previous)?;
            let destination = profile.store(destination)?;
            if *amount <= 0 || previous.resource != destination.resource {
                return Err("return requires compatible actual quantity".into());
            }
            if staged[&previous.endpoint].stock.incoming.get(claim) != Some(amount) {
                return Err("return has no matching previous incoming claim".into());
            }
            if previous.endpoint == destination.endpoint {
                return Ok(*amount);
            }
            staged
                .get_mut(&previous.endpoint)
                .expect("previous endpoint")
                .stock
                .release_incoming(claim);
            if profile.receiving_headroom(&staged, &destination.id)? < *amount {
                return Ok(0);
            }
            staged
                .get_mut(&destination.endpoint)
                .expect("return endpoint")
                .stock
                .reserve_incoming(claim, *amount)?;
            *amount
        }
    };
    profile.validate_stocks(&staged)?;
    *stocks = staged;
    Ok(moved)
}
