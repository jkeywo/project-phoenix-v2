use super::*;

fn power() -> crate::modifiers::power_system::PowerSystem {
    let base = crate::modifiers::power_system::PowerSystem::default();
    let mut hardware = base.config().clone();
    let mut ship = base.ship_config().clone();
    for (index, (id, kind)) in [
        (
            crate::ship::system_registry::helm_thrust_system_id(),
            crate::ship::system_registry::HELM_THRUST_KIND,
        ),
        (
            crate::ship::system_registry::helm_steering_system_id(),
            crate::ship::system_registry::HELM_STEERING_KIND,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let previous = hardware.consumers[index].system.clone();
        hardware.consumers[index].system = id.clone();
        let system = ship
            .systems
            .iter_mut()
            .find(|system| system.id == previous)
            .unwrap();
        system.id = id;
        system.kind = kind.into();
    }
    crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship).unwrap()
}

#[test]
fn distant_missing_or_empty_reserves_allow_coasting_but_no_new_thrust_or_turn() {
    let before = ShipPhysics {
        forward_speed: 10.0,
        ..Default::default()
    };
    for mut power in [None, Some(power())] {
        if let Some(power) = power.as_mut() {
            for reserve in power.network_mut().consumers.values_mut() {
                reserve.energy = 0.0;
            }
        }
        let mut proposed = ShipPhysics {
            forward_speed: 12.0,
            yaw: 1.0,
            ..before
        };
        fund_low_lod_motion(
            &before,
            &mut proposed,
            power.as_mut(),
            None,
            None,
            1.0,
            20.0,
            0.5,
            true,
        );
        assert_eq!(proposed.forward_speed, 10.0);
        assert_eq!(proposed.yaw, before.yaw);
    }
}

#[test]
fn distant_partial_grants_are_independent_and_zero_intake_keeps_local_reserve_useful() {
    let mut power = power();
    for group in power.config().groups.clone() {
        power.set_flow(&group.id, 0.0).unwrap();
    }
    power
        .network_mut()
        .consumers
        .get_mut(&crate::ship::system_registry::helm_thrust_system_id())
        .unwrap()
        .energy = 0.25;
    power
        .network_mut()
        .consumers
        .get_mut(&crate::ship::system_registry::helm_steering_system_id())
        .unwrap()
        .energy = 0.5;
    let before = ShipPhysics {
        forward_speed: 2.0,
        ..Default::default()
    };
    let mut proposed = ShipPhysics {
        forward_speed: 4.0,
        yaw: 1.0,
        ..before
    };
    fund_low_lod_motion(
        &before,
        &mut proposed,
        Some(&mut power),
        None,
        None,
        1.0,
        4.0,
        0.5,
        true,
    );
    assert_eq!(proposed.forward_speed, 2.5);
    assert_eq!(proposed.yaw, 0.25);
}

#[test]
fn distant_unchanged_drift_and_passive_deceleration_do_not_spend() {
    let mut power = power();
    let before_power = power.capture_continuation();
    let before = ShipPhysics {
        forward_speed: 10.0,
        ..Default::default()
    };
    let mut coast = ShipPhysics {
        forward_speed: 9.0,
        ..before
    };
    fund_low_lod_motion(
        &before,
        &mut coast,
        Some(&mut power),
        None,
        None,
        1.0,
        20.0,
        0.5,
        false,
    );
    assert_eq!(coast.forward_speed, 9.0);
    assert_eq!(power.capture_continuation(), before_power);
}

#[test]
fn distant_sign_crossing_pays_for_reverse_motion_after_passive_stop() {
    for (previous, desired) in [(-1.0, 1.0), (10.0, -5.0)] {
        for energy in [0.0, 0.5] {
            let mut power = power();
            power
                .network_mut()
                .consumers
                .get_mut(&crate::ship::system_registry::helm_thrust_system_id())
                .unwrap()
                .energy = energy;
            let before = ShipPhysics {
                forward_speed: previous,
                ..Default::default()
            };
            let mut proposed = ShipPhysics {
                forward_speed: desired,
                ..before
            };
            fund_low_lod_motion(
                &before,
                &mut proposed,
                Some(&mut power),
                None,
                None,
                1.0,
                desired.abs(),
                0.5,
                true,
            );
            assert_eq!(proposed.forward_speed, desired * energy);
        }
    }
}
