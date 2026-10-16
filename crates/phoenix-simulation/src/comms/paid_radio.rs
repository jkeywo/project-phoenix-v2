//! Ephemeral receipt of the actual once-per-tick Radio listening work.
//! It is not machinery state, another payment, a route or an observation.
use bevy::prelude::*;
use phoenix_model::identity::SystemId;

#[derive(Component, Clone, Debug, Default)]
pub(crate) struct PaidRadioWork {
    owner: Option<SystemId>,
    tick: Option<u64>,
    fraction: f32,
}
impl PaidRadioWork {
    pub(super) fn record(&mut self, owner: &SystemId, tick: u64, fraction: f32) {
        self.owner = Some(owner.clone());
        self.tick = Some(tick);
        self.fraction = if fraction.is_finite() && (0.0..=1.0).contains(&fraction) {
            fraction
        } else {
            0.0
        };
    }
    /// Raw paid fraction, BEFORE Operation. A late consumer must independently
    /// prove current equipment/availability/Operation and use current poses.
    pub(crate) fn raw(&self, owner: &SystemId, tick: u64) -> f32 {
        if self.owner.as_ref() == Some(owner) && self.tick == Some(tick) {
            self.fraction
        } else {
            0.0
        }
    }
}

/// Late physical consumers reuse the actual listening payment but never the
/// pre-motion endpoint pose or its already Operation-scaled listening factor.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CurrentPaidEndpoints<'w, 's> {
    owners: Query<
        'w,
        's,
        (
            Entity,
            &'static crate::entities::spawner::EntityUuid,
            &'static super::radio::RadioProfile,
            &'static PaidRadioWork,
            Option<&'static crate::ship::state::ShipPhysics>,
            &'static Transform,
            &'static crate::ship_plugin::ShipConfigComponent,
            &'static crate::ship_plugin::ShipSystemControlSources,
            (
                Option<&'static crate::ship::compartment_damage::ShipDamage>,
                Option<&'static crate::entities::spawner::EntitySystemHull>,
                Option<&'static crate::modifiers::ShipModifiers>,
            ),
            (
                Option<&'static crate::lockstep::FleetSlotOf>,
                Option<&'static crate::entities::spawner::FactionComponent>,
                Option<&'static crate::entities::spawner::EntityName>,
            ),
            Option<&'static super::traffic::ShipCommsTraffic>,
        ),
    >,
}
impl CurrentPaidEndpoints<'_, '_> {
    pub(crate) fn collect(&self, tick: u64) -> Vec<super::radio::Endpoint> {
        let mut rows = Vec::new();
        for (
            entity,
            uuid,
            profile,
            receipt,
            physics,
            transform,
            config,
            controls,
            (damage, hull, modifiers),
            (slot, faction, name),
            traffic,
        ) in &self.owners
        {
            let owner = &profile.0.system;
            let available = controls.0.entries().any(|(id, _)| id == owner)
                && !controls.0.is_offline(owner)
                && controls.0.source_for(owner)
                    != crate::ship::control_source::ControlSource::Offline
                && super::server::information_endpoint_available(config, controls, modifiers)
                && !crate::ship::compartment_damage::operation_disabled(damage, hull, owner);
            let operation = damage.map_or(1.0, |damage| {
                damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
            });
            let listening = if available {
                receipt.raw(owner, tick) * operation.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let monitored = if profile.0.traffic.is_none() {
                super::traffic::channels(&profile.0)
            } else {
                traffic
                    .filter(|state| state.state.owner == *owner)
                    .and_then(|state| state.coverage.as_ref())
                    .filter(|(paid_tick, _)| *paid_tick == tick)
                    .map_or_else(Vec::new, |(_, channels)| channels.clone())
            };
            rows.push(super::radio::Endpoint {
                entity,
                uuid: uuid.0.clone(),
                name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
                point: physics.map_or(transform.translation, |pose| {
                    Vec3::new(pose.x, pose.y, pose.z)
                }),
                radio: profile.0.clone(),
                listening,
                monitored,
                fleet: slot.is_some(),
                faction: faction.map(|faction| faction.0.to_string()),
            });
        }
        rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        rows
    }
}

pub(crate) fn route_available(
    endpoints: &[super::radio::Endpoint],
    source: &super::radio::Endpoint,
    receiver: &super::radio::Endpoint,
    channel: &str,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
) -> bool {
    super::radio::route_available(endpoints, source, receiver, channel, paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_receipt_has_exact_owner_clock_and_no_restored_credit() {
        let owner = SystemId("comms".into());
        let mut receipt = PaidRadioWork::default();
        assert_eq!(receipt.raw(&owner, 7), 0.0);
        receipt.record(&owner, 7, 0.4);
        assert_eq!(receipt.raw(&owner, 7), 0.4);
        assert_eq!(receipt.raw(&owner, 8), 0.0);
        assert_eq!(receipt.raw(&SystemId("other".into()), 7), 0.0);
        receipt = PaidRadioWork::default();
        assert_eq!(receipt.raw(&owner, 7), 0.0);
    }
}
