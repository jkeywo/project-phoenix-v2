use super::*;

fn bank() -> BlasterSystem {
    BlasterSystem::new(BlasterBankConfig {
        volley_count: 1,
        projectile_speed: 10.0,
        range: 100.0,
        collision_radius: 0.1,
        ..Default::default()
    })
}

#[test]
fn elevated_launch_has_one_paid_round_and_frozen_config_independent_flight() {
    let mut bank = bank();
    assert!(bank.request_fire());
    let mut paid = 0;
    let mut minted = 0;
    let events = bank.tick_with_origins_3d(
        0.1,
        0.1,
        &[Some([0.0, 5.0, 0.0])],
        0.0,
        [0.0, 15.0, -10.0],
        [0.0; 3],
        "shooter",
        &mut |_| {
            paid += 1;
            true
        },
        &mut || {
            minted += 1;
            "bolt".into()
        },
    );
    assert_eq!((paid, minted, events.len()), (1, 1, 1));
    assert_eq!(events[0].y, 5.0);
    assert_eq!(bank.in_flight[0].collision_interval_secs, None);
    assert!((events[0].pitch - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    let frozen = bank.in_flight[0].clone();
    bank.config = BlasterBankConfig::default();
    bank.advance_flight_only(1.0);
    let bolt = &bank.in_flight[0];
    assert_eq!(bolt.speed, frozen.speed);
    assert_eq!(bolt.damage, frozen.damage);
    assert!((bolt.y - (5.0 + 10.0 / 2.0_f32.sqrt())).abs() < 1e-5);
    assert!((bolt.z + 10.0 / 2.0_f32.sqrt()).abs() < 1e-5);
    assert_eq!(bolt.previous_position, Some([0.0, 5.0, 0.0]));
    assert_eq!(bolt.collision_interval_secs, Some(1.0));
    assert_eq!((paid, minted), (1, 1));
}

#[test]
fn impossible_lead_and_missing_barrel_do_not_pay_or_mint() {
    let mut bank = bank();
    assert!(bank.request_fire());
    let events = bank.tick_with_origins_3d(
        0.1,
        0.1,
        &[Some([0.0; 3])],
        0.0,
        [0.0, 15.0, 0.0],
        [0.0, 20.0, 0.0],
        "shooter",
        &mut |_| panic!("unreachable trajectory must not pay"),
        &mut || panic!("unreachable trajectory must not mint"),
    );
    assert!(events.is_empty());
    assert!(bank.in_flight.is_empty());
}

#[test]
fn swept_xyz_chooses_first_physical_body_and_consumes_the_interval_once() {
    let mut bank = bank();
    bank.request_fire();
    bank.tick_with_origins_3d(
        0.1,
        0.1,
        &[Some([0.0, 10.0, 0.0])],
        0.0,
        [0.0, 10.0, -10.0],
        [0.0; 3],
        "shooter",
        &mut |_| true,
        &mut || "bolt".into(),
    );
    bank.advance_flight_only(1.0);
    let bodies = vec![
        ("planar-ship".into(), [0.0, 0.0, -2.0], 1.0),
        ("far-missile".into(), [0.0, 10.0, -9.0], 0.1),
        ("near-friendly".into(), [0.0, 10.0, -4.0], 0.1),
    ];
    assert_eq!(
        bank.find_hits_3d(&bodies),
        vec![("bolt".into(), "near-friendly".into())]
    );
    bank.advance_flight_only(0.0);
    assert_eq!(bank.in_flight[0].collision_interval_secs, Some(1.0));
    bank.finish_collision_interval();
    assert_eq!(bank.in_flight[0].collision_interval_secs, None);
    assert!(
        bank.find_hits_3d(&bodies).is_empty(),
        "paused Damage cannot replay"
    );
    bank.advance_flight_only(0.0);
    assert!(bank.find_hits_3d(&bodies).is_empty());
    bank.in_flight[0].lifespan_remaining = 0.1;
    bank.advance_flight_only(100.0);
    assert_eq!(bank.in_flight[0].collision_interval_secs, Some(0.1));
    assert!(
        (bank.in_flight[0].z + 11.0).abs() < 1e-5,
        "last segment stops at actual range"
    );
    assert!(bank
        .find_hits_3d(&[("beyond".into(), [0.0, 10.0, -15.0], 0.1)])
        .is_empty());
    bank.finish_collision_interval();
    assert!(bank.in_flight.is_empty());
}
