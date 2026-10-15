//! One emitted barrel pays its real local stock and energy together.
use crate::core::messages::SystemId;
use crate::infrastructure::InfrastructureCondition;
use crate::ship::power::ShipPowerSystem;
use phoenix_model::weapon_operation::WeaponRoundFeed;

pub(super) fn pay_barrel(
    owner: &SystemId,
    feed: Option<&WeaponRoundFeed>,
    barrel: usize,
    power: Option<&mut ShipPowerSystem>,
    inventory: Option<&mut InfrastructureCondition>,
    logistics: Option<&crate::resources::logistics_server::CompiledShipLogistics>,
    pending_debits: &std::collections::BTreeMap<String, i64>,
) -> Option<Vec<crate::infrastructure::condition::FlagChange>> {
    let Some(power) = power.filter(|p| !p.0.weapons_restrained()) else {
        return None;
    };
    let Some(feed) = feed else {
        return matches!(power.0.try_work_discrete_count(owner, 1), Ok(true)).then(Vec::new);
    };
    let inventory = inventory?;
    let store = logistics?.0.store(&feed.store).ok()?;
    if store.role != phoenix_model::logistics::StoreRole::Local
        || store.system.as_ref() != Some(owner)
    {
        return None;
    }
    let phoenix_model::logistics::ResourceEndpoint::Capacity { id: capacity } = &store.endpoint
    else {
        return None;
    };
    let Some(mut stock) = inventory.0.capacity_stock(capacity).cloned() else {
        return None;
    };
    // Earlier physical owners can have committed withdrawals awaiting the
    // ordinary capacity drain. Do not spend that quantity a second time, or
    // count not-yet-applied deposits as ammunition.
    if stock
        .available()
        .saturating_sub(pending_debits.get(capacity).copied().unwrap_or(0))
        < feed.amount
    {
        return None;
    }
    let claim = format!("weapon:{}:barrel:{barrel}", owner.0);
    if !matches!(
        crate::weapons::operation::claim_rounds(&mut stock, feed, &claim),
        Ok(true)
    ) {
        return None;
    }
    let mut staged_power = power.0.clone();
    let paid = matches!(staged_power.try_work_discrete_count(owner, 1), Ok(true));
    if !matches!(
        crate::weapons::operation::finish_rounds(&mut stock, feed, &claim, paid),
        Ok(true)
    ) {
        return None;
    }
    // Stock validation can refuse without spending Power or changing inventory.
    let flags = inventory.0.replace_capacity_stock(capacity, stock).ok()?;
    power.0 = staged_power;
    Some(flags)
}

#[cfg(test)]
#[path = "round_work_tests.rs"]
mod tests;
