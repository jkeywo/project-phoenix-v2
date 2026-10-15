//! Equipment-bound port work stages the two existing energy owners and the
//! existing PortOrder frontier. There is no provider progress or inventory copy.
use super::ports_server::{entity_for, CompiledPortService, PortTransfer};
use crate::core::messages::SystemId;
use crate::resources::ports::PortServicePolicy;
use crate::ship::power::ShipPowerSystem;
use crate::umbilical::TransferUmbilical;
use bevy::prelude::*;

const UNAVAILABLE: &str = "ports.blocked.service_unavailable";
const NO_STOCK: &str = "ports.blocked.no_stock";

fn factor(world: &World, entity: Entity, owner: &SystemId) -> Result<f32, &'static str> {
    let sources = world
        .get::<crate::ship_plugin::ShipSystemControlSources>(entity)
        .ok_or(UNAVAILABLE)?;
    if !sources.0.entries().any(|(id, _)| id == owner) || !sources.0.policy_for(owner).coordinate {
        return Err(UNAVAILABLE);
    }
    let damage = world
        .get::<crate::ship::compartment_damage::ShipDamage>(entity)
        .ok_or(UNAVAILABLE)?;
    let factor = damage.factor(owner, phoenix_model::damage::DamageBinding::Operation);
    if !factor.is_finite() || factor <= 0.0 || factor > 1.0 {
        return Err(UNAVAILABLE);
    }
    let power = world.get::<ShipPowerSystem>(entity).ok_or(UNAVAILABLE)?;
    if !power.0.config().consumers.iter().any(|row| {
        &row.system == owner
            && row.continuous_cost_per_sec.is_finite()
            && row.continuous_cost_per_sec > 0.0
    }) {
        return Err(UNAVAILABLE);
    }
    Ok(factor)
}

/// Only transaction-local clones are changed. Both endpoints buy a common
/// affordable interval; Operation changes output, never the energy clock.
fn pay_pair(
    visitor: &mut ShipPowerSystem,
    provider: &mut ShipPowerSystem,
    visitor_owner: &SystemId,
    provider_owner: &SystemId,
    seconds: f32,
    efficiency: f32,
) -> Result<f32, String> {
    let mut visitor_quote = visitor.clone();
    let mut provider_quote = provider.clone();
    let common = visitor_quote
        .0
        .work_continuous(visitor_owner, seconds, 1.0)?
        .fraction
        .min(
            provider_quote
                .0
                .work_continuous(provider_owner, seconds, 1.0)?
                .fraction,
        );
    let common_seconds = seconds * common;
    if !common_seconds.is_finite() || common_seconds <= 0.0 {
        return Err(UNAVAILABLE.into());
    }
    let left = visitor
        .0
        .work_continuous(visitor_owner, common_seconds, 1.0)?;
    let right = provider
        .0
        .work_continuous(provider_owner, common_seconds, 1.0)?;
    let funded =
        (common_seconds / seconds).min(1.0) * left.fraction.min(right.fraction) * efficiency;
    if !funded.is_finite() || funded <= 0.0 || funded > 1.0 {
        return Err(UNAVAILABLE.into());
    }
    Ok(funded)
}

pub(super) fn settle(world: &mut World, request: &PortTransfer) -> Result<i64, &'static str> {
    let visitor = entity_for(world, &request.operator).ok_or(UNAVAILABLE)?;
    let provider = entity_for(world, &request.provider).ok_or(UNAVAILABLE)?;
    if visitor == provider {
        return Err(UNAVAILABLE);
    }
    let profile = world
        .get::<CompiledPortService>(provider)
        .ok_or(UNAVAILABLE)?
        .0
        .clone();
    let owner = profile.system.as_ref().ok_or(UNAVAILABLE)?;
    let visitor_owner = crate::ship::system_registry::umbilical_system_id();
    let efficiency = factor(world, visitor, &visitor_owner)?.min(factor(world, provider, owner)?);
    let mut tool = world
        .get::<TransferUmbilical>(visitor)
        .ok_or(UNAVAILABLE)?
        .clone();
    if !(tool.running || tool.port_maintenance)
        || request.ordered != (profile.policy == PortServicePolicy::ManualOffer)
        || request.maintain_goal != (!request.ordered && tool.port_maintenance)
        || request.tick
            != world
                .get_resource::<crate::sim_tick::SimTick>()
                .ok_or(UNAVAILABLE)?
                .0
    {
        return Err(UNAVAILABLE);
    }
    let limit = super::server::preview_port_exchange(world, request).map_err(|_| NO_STOCK)?;
    if limit <= 0 {
        return Err(NO_STOCK);
    }
    let selected_store = request
        .store
        .clone()
        .unwrap_or_else(|| "named-service".into());
    let mut plan = if request.ordered {
        tool.port_order.clone()
    } else {
        tool.port_flow
            .clone()
            .filter(|plan| {
                plan.partner.as_ref() == Some(&request.provider)
                    && plan.visit == Some(request.visit)
                    && plan.active
                    && plan.current_line().is_some_and(|(_, line)| {
                        line.offer == request.offer && line.store == selected_store
                    })
            })
            .unwrap_or_default()
    };
    if !request.ordered && plan.partner.is_none() {
        let offer = profile.offer(&request.offer).ok_or(UNAVAILABLE)?;
        plan.approve(
            &profile,
            &request.provider,
            request.visit,
            vec![phoenix_model::ports::PortOrderLine {
                offer: request.offer.clone(),
                store: selected_store.clone(),
                lots: limit.min(offer.quota),
            }],
        )
        .map_err(|_| UNAVAILABLE)?;
    }
    if !plan.active
        || plan.partner.as_ref() != Some(&request.provider)
        || plan.visit != Some(request.visit)
        || !plan
            .current_line()
            .is_some_and(|(_, line)| line.offer == request.offer && line.store == selected_store)
    {
        return Err(UNAVAILABLE);
    }
    if request.tick <= plan.last_tick {
        return Ok(0);
    }
    // All stock/quota/shared-space checks occur before any actual energy write.
    let mut visitor_power = world
        .get::<ShipPowerSystem>(visitor)
        .ok_or(UNAVAILABLE)?
        .clone();
    let mut provider_power = world
        .get::<ShipPowerSystem>(provider)
        .ok_or(UNAVAILABLE)?
        .clone();
    let dt = world
        .get_resource::<Time>()
        .ok_or(UNAVAILABLE)?
        .delta_secs();
    let lots = plan
        .advance(
            &profile,
            &visitor_owner,
            request.tick,
            dt,
            tool.config.rate,
            limit,
            |system, seconds| {
                pay_pair(
                    &mut visitor_power,
                    &mut provider_power,
                    system,
                    owner,
                    seconds,
                    efficiency,
                )
            },
        )
        .map_err(|_| UNAVAILABLE)?;
    plan.record_receipt(request.tick, lots)
        .map_err(|_| UNAVAILABLE)?;
    if lots > 0 {
        let mut actual = request.clone();
        actual.lots = lots;
        if super::server::preview_port_exchange(world, &actual).map_err(|_| NO_STOCK)? != lots {
            return Err(NO_STOCK);
        }
        // Exclusive World access: no consumer can change the validated owners
        // between this identical canonical preview and its stock commit.
        if super::server::settle_port_exchange(world, &actual).map_err(|_| NO_STOCK)? != lots {
            return Err(NO_STOCK);
        }
    }
    if request.ordered {
        tool.port_order = plan;
    } else {
        tool.port_flow = Some(plan);
    }
    *world
        .get_mut::<ShipPowerSystem>(visitor)
        .ok_or(UNAVAILABLE)? = visitor_power;
    *world
        .get_mut::<ShipPowerSystem>(provider)
        .ok_or(UNAVAILABLE)? = provider_power;
    *world
        .get_mut::<TransferUmbilical>(visitor)
        .ok_or(UNAVAILABLE)? = tool;
    Ok(lots)
}

pub(super) fn update_view(world: &mut World, request: &PortTransfer, blocked: Option<&str>) {
    let Some(entity) = entity_for(world, &request.operator) else {
        return;
    };
    let Some(mut tool) = world.get_mut::<TransferUmbilical>(entity) else {
        return;
    };
    let plan = if request.ordered {
        Some(&tool.port_order)
    } else {
        tool.port_flow.as_ref()
    };
    let order = phoenix_model::ports_ui::PortOrderView {
        active: plan.is_some_and(|plan| plan.active),
        lines: plan
            .map(|plan| {
                plan.lines
                    .iter()
                    .map(|line| phoenix_model::ports_ui::PortOrderLineView {
                        offer: line.offer.clone(),
                        store: line.store.clone(),
                        requested_lots: line.requested_lots,
                        delivered_lots: line.delivered_lots,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        blocked: blocked.map(str::to_owned),
    };
    if let Some(view) = &mut tool.port_view {
        view.order = Some(order);
    }
}
