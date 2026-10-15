use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductionStockAction {
    Begin {
        recipe: String,
        claims: BTreeMap<String, String>,
        floors: BTreeMap<String, i64>,
    },
    Complete {
        recipe: String,
        claims: BTreeMap<String, String>,
    },
    Discard {
        recipe: String,
        claims: BTreeMap<String, String>,
    },
}

/// All mutations apply to a transaction-local copy of real endpoint owners.
/// Callers commit these owners together with the staged energy and job state.
pub fn apply_production_stock_action(
    profile: &CompiledProduction,
    action: &ProductionStockAction,
    stocks: &mut EndpointMap,
) -> Result<bool, String> {
    profile.logistics.validate_stocks(stocks)?;
    let (recipe, claims) = match action {
        ProductionStockAction::Begin { recipe, claims, .. }
        | ProductionStockAction::Complete { recipe, claims }
        | ProductionStockAction::Discard { recipe, claims } => (profile.recipe(recipe)?, claims),
    };
    if claims.len() != recipe.outputs.len()
        || recipe.outputs.iter().any(|output| {
            claims
                .get(&output.store)
                .is_none_or(|id| id.trim().is_empty())
        })
    {
        return Err("production output claims do not match the recipe".into());
    }
    let mut staged = stocks.clone();
    match action {
        ProductionStockAction::Begin { floors, .. } => {
            for (id, floor) in floors {
                let store = profile.logistics.store(id)?;
                if *floor < 0 || *floor > profile.logistics.endpoints()[&store.endpoint].ceiling {
                    return Err("production input reserve floor is outside its actual store".into());
                }
            }
            for input in &recipe.inputs {
                let store = profile.logistics.store(&input.store)?;
                let stock = &mut staged
                    .get_mut(&store.endpoint)
                    .ok_or("missing actual input stock")?
                    .stock;
                let floor = *floors
                    .get(&input.store)
                    .ok_or("production requires each input's current Inventory floor")?;
                if stock.available() < input.amount
                    || stock.level.saturating_sub(floor) < input.amount
                {
                    return Ok(false);
                }
                // No persistent ingredient reservation: this exact stage consumes once.
                let claim = format!("{}/input/{}", claims[&recipe.outputs[0].store], input.store);
                stock.reserve(&claim, input.amount)?;
                stock.consume(&claim).expect("just reserved ingredient");
            }
            for output in &recipe.outputs {
                let store = profile.logistics.store(&output.store)?;
                if profile
                    .logistics
                    .receiving_headroom(&staged, &output.store)?
                    < output.amount
                {
                    return Ok(false);
                }
                let stock = &mut staged
                    .get_mut(&store.endpoint)
                    .ok_or("missing output stock")?
                    .stock;
                if stock.incoming.contains_key(&claims[&output.store]) {
                    return Err("production output claim already exists".into());
                }
                stock.reserve_incoming(&claims[&output.store], output.amount)?;
            }
        }
        ProductionStockAction::Complete { .. } | ProductionStockAction::Discard { .. } => {
            for output in &recipe.outputs {
                let store = profile.logistics.store(&output.store)?;
                let stock = &mut staged
                    .get_mut(&store.endpoint)
                    .ok_or("missing output stock")?
                    .stock;
                let claim = &claims[&output.store];
                if stock.incoming.get(claim) != Some(&output.amount) {
                    return Err("production output is missing its exact receiving claim".into());
                }
                if matches!(action, ProductionStockAction::Complete { .. }) {
                    stock.receive(claim, output.amount)?;
                } else {
                    stock.release_incoming(claim);
                }
            }
        }
    }
    profile.logistics.validate_stocks(&staged)?;
    *stocks = staged;
    Ok(true)
}
