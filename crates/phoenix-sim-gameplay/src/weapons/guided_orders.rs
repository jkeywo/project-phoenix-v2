//! Operations on own sent/acknowledged knowledge. No physical projectile queries.
use phoenix_model::guided_orders::{
    GuidanceAcknowledgement, GuidedOrderView, GuidedSourceKnowledge, IssuedGuidanceOrder,
    KnownGuidedLaunch, KnownGuidedLaunchCommand,
};
use phoenix_model::identity::SystemId;

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
fn public_contact(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
fn valid_order(order: &IssuedGuidanceOrder, launched: u64, now: u64) -> bool {
    order.issued_tick >= launched
        && order.issued_tick <= now
        && order.target.as_deref().is_none_or(public_contact)
        && order
            .sampled_tick
            .is_none_or(|sample| sample <= order.issued_tick)
        && (order.target.is_some() || order.sampled_tick.is_none())
        && (order.revision == 0 || order.target.is_some() == order.sampled_tick.is_some())
}

pub fn validate(state: &GuidedSourceKnowledge, now: u64) -> Result<(), String> {
    for (first_emitted_id, command) in &state.launch_commands {
        if !identity(first_emitted_id)
            || !identity(&command.tube.0)
            || command.issued_tick > now
            || command
                .target
                .as_deref()
                .is_some_and(|target| !public_contact(target))
            || command
                .sampled_tick
                .is_some_and(|sample| sample > command.issued_tick)
            || (command.target.is_none() && command.sampled_tick.is_some())
        {
            return Err("invalid source-owned burst launch command".into());
        }
    }
    for (key, launch) in &state.launches {
        if key != &launch.round_uuid
            || !identity(key)
            || !identity(&launch.tube.0)
            || launch.launched_tick > now
            || !launch.lifespan_secs.is_finite()
            || launch.lifespan_secs <= 0.0
            || launch
                .receiver
                .as_ref()
                .is_some_and(|profile| profile.validate().is_err())
            || !valid_order(&launch.issued, launch.launched_tick, now)
            || (launch.issued.revision == 0 && launch.issued.issued_tick != launch.launched_tick)
            || (launch.receiver.is_none() && launch.issued.revision > 0)
            || launch.acknowledged.as_ref().is_some_and(|receipt| {
                launch.receiver.is_none()
                    || receipt.order.revision == 0
                    || !valid_order(&receipt.order, launch.launched_tick, now)
                    || receipt.received_tick < receipt.order.issued_tick
                    || receipt.received_tick > now
                    || receipt.order.revision > launch.issued.revision
                    || receipt.order.issued_tick > launch.issued.issued_tick
                    || (receipt.order.revision == launch.issued.revision
                        && receipt.order != launch.issued)
            })
        {
            return Err("invalid source-owned guided launch/order knowledge".into());
        }
    }
    Ok(())
}

pub fn note_launch(state: &mut GuidedSourceKnowledge, launch: KnownGuidedLaunch, now: u64) -> bool {
    if state.launches.contains_key(&launch.round_uuid)
        || launch.launched_tick != now
        || launch.issued.revision != 0
        || launch.issued.issued_tick != now
        || launch.acknowledged.is_some()
    {
        return false;
    }
    let mut candidate = GuidedSourceKnowledge::default();
    candidate
        .launches
        .insert(launch.round_uuid.clone(), launch.clone());
    if validate(&candidate, now).is_err() {
        return false;
    }
    state.launches.insert(launch.round_uuid.clone(), launch);
    true
}

/// Recording an issued order does not assert that a round still exists or heard it.
pub fn issue(
    state: &mut GuidedSourceKnowledge,
    tube: &SystemId,
    round: &str,
    target: Option<String>,
    sampled_tick: Option<u64>,
    now: u64,
) -> Option<IssuedGuidanceOrder> {
    let launch = state.launches.get_mut(round)?;
    if &launch.tube != tube || launch.receiver.is_none() || now < launch.issued.issued_tick {
        return None;
    }
    let order = IssuedGuidanceOrder {
        revision: launch.issued.revision.checked_add(1)?,
        issued_tick: now,
        target,
        sampled_tick,
    };
    if !valid_order(&order, launch.launched_tick, now) {
        return None;
    }
    launch.issued = order.clone();
    Some(order)
}

/// Only the physical link owner may call this after an actual reverse delivery.
pub fn acknowledge(
    state: &mut GuidedSourceKnowledge,
    tube: &SystemId,
    round: &str,
    order: &IssuedGuidanceOrder,
    received: u64,
) -> bool {
    let Some(launch) = state.launches.get_mut(round) else {
        return false;
    };
    if &launch.tube != tube
        || launch.receiver.is_none()
        || order.revision == 0
        || order != &launch.issued
        || received < order.issued_tick
        || launch
            .acknowledged
            .as_ref()
            .is_some_and(|old| old.order.revision > order.revision || old.received_tick > received)
    {
        return false;
    }
    launch.acknowledged = Some(GuidanceAcknowledgement {
        order: order.clone(),
        received_tick: received,
    });
    true
}

/// Retirement depends on locally known launch endurance, never remote death.
pub fn retire(state: &mut GuidedSourceKnowledge, now: u64, fixed_secs: f64) -> bool {
    if !fixed_secs.is_finite() || fixed_secs <= 0.0 {
        return false;
    }
    state.launches.retain(|_, launch| {
        now < launch.launched_tick
            || (now - launch.launched_tick) as f64 * fixed_secs < f64::from(launch.lifespan_secs)
    });
    true
}

pub fn project(
    state: &GuidedSourceKnowledge,
    tube: &SystemId,
    source_operable: bool,
) -> Vec<GuidedOrderView> {
    state
        .launches
        .values()
        .filter(|launch| &launch.tube == tube)
        .map(|launch| GuidedOrderView {
            round_uuid: launch.round_uuid.clone(),
            tube: launch.tube.clone(),
            target: launch.issued.target.clone(),
            issued_tick: launch.issued.issued_tick,
            sampled_tick: launch.issued.sampled_tick,
            acknowledged_tick: launch
                .acknowledged
                .as_ref()
                .map(|receipt| receipt.received_tick),
            unresolved: launch
                .acknowledged
                .as_ref()
                .is_none_or(|receipt| receipt.order != launch.issued),
            can_retarget: source_operable && launch.receiver.is_some(),
        })
        .collect()
}

#[cfg(test)]
#[path = "guided_orders_tests.rs"]
mod tests;

/// Tied to the first real emitted ID; a later launch on this tube is distinct.
pub fn note_launch_command(
    state: &mut GuidedSourceKnowledge,
    first_emitted_id: String,
    command: KnownGuidedLaunchCommand,
    now: u64,
) -> bool {
    if state.launch_commands.contains_key(&first_emitted_id) || command.issued_tick != now {
        return false;
    }
    let mut candidate = GuidedSourceKnowledge::default();
    candidate
        .launch_commands
        .insert(first_emitted_id.clone(), command.clone());
    if validate(&candidate, now).is_err() {
        return false;
    }
    state.launch_commands.insert(first_emitted_id, command);
    true
}
