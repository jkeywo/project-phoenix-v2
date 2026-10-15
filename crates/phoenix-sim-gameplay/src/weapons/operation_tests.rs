use super::*;
fn feed() -> WeaponRoundFeed {
    WeaponRoundFeed {
        store: "local-rounds".into(),
        amount: 2,
    }
}

#[test]
fn rounds_are_reserved_before_power_and_exactly_consumed_or_released() {
    let mut stock = Stock::new(5, 8).unwrap();
    stock.reserve("other-owner", 1).unwrap();
    stock.reserve_incoming("delivery", 2).unwrap();
    let before = stock.clone();
    assert!(claim_rounds(&mut stock, &feed(), "barrel-0").unwrap());
    assert!(!finish_rounds(&mut stock, &feed(), "barrel-0", false).unwrap());
    assert_eq!(
        stock, before,
        "failed power releases its actual round claim"
    );
    assert!(claim_rounds(&mut stock, &feed(), "barrel-0").unwrap());
    assert!(finish_rounds(&mut stock, &feed(), "barrel-0", true).unwrap());
    assert_eq!(stock.level, 3);
    assert_eq!(stock.reserved, before.reserved);
    assert_eq!(stock.incoming, before.incoming);
    assert!(finish_rounds(&mut stock, &feed(), "barrel-0", true).is_err());
    stock.validate().unwrap();
}

#[test]
fn claim_collision_shortage_and_malformed_settlement_leave_stock_unchanged() {
    let mut stock = Stock::new(3, 4).unwrap();
    stock.reserve("existing", 2).unwrap();
    let before = stock.clone();
    assert!(claim_rounds(&mut stock, &feed(), "existing").is_err());
    assert!(!claim_rounds(&mut stock, &feed(), "new").unwrap());
    assert!(finish_rounds(&mut stock, &feed(), "wrong", true).is_err());
    assert!(claim_rounds(
        &mut stock,
        &WeaponRoundFeed {
            amount: 0,
            ..feed()
        },
        "new"
    )
    .is_err());
    assert_eq!(stock, before);
}

#[test]
fn fixed_beams_bound_work_while_sustained_beams_use_real_paid_time_without_infinity() {
    assert_eq!(
        beam_work_seconds(BeamOperation::Fixed, 0.25, 1.0).unwrap(),
        0.25
    );
    assert_eq!(
        beam_work_seconds(BeamOperation::Sustained, 0.0, 1.0).unwrap(),
        1.0
    );
    assert_eq!(
        beam_work_seconds(BeamOperation::Sustained, 0.0, 0.0).unwrap(),
        0.0
    );
    assert!(beam_work_seconds(BeamOperation::Sustained, f32::INFINITY, 1.0).is_err());
    let mode: BeamOperation =
        serde_json::from_str(&serde_json::to_string(&BeamOperation::Sustained).unwrap()).unwrap();
    assert_eq!(mode, BeamOperation::Sustained);
}

#[test]
fn actual_projectile_heading_yields_forward_and_broadside_recoil_without_mass_invention() {
    assert_eq!(recoil_delta(0.0, 6.0).unwrap(), [0.0, 6.0]);
    let side = recoil_delta(std::f32::consts::FRAC_PI_2, 6.0).unwrap();
    assert!((side[0] + 6.0).abs() < 1e-5 && side[1].abs() < 1e-5);
    let turned = recoil_delta(std::f32::consts::PI, 6.0).unwrap();
    assert!(turned[0].abs() < 1e-5 && (turned[1] + 6.0).abs() < 1e-5);
    assert!(recoil_delta(f32::NAN, 6.0).is_err());
    assert!(recoil_delta(0.0, -1.0).is_err());
}

#[test]
fn explicit_target_domains_do_not_guess_tags_or_replace_an_intentional_hold() {
    use phoenix_model::tags::EntityTag::{Missile, Ship};
    assert!(target_allowed(None, &[], true));
    assert!(!target_allowed(None, &[Missile], false));
    assert!(!target_allowed(Some(&[]), &[Ship], true));
    assert!(!target_allowed(Some(&[Missile]), &[], true));
    assert!(target_allowed(Some(&[Ship, Missile]), &[Missile], false));
    assert!(!target_allowed(Some(&[Missile]), &[Ship], true));
    validate_target_filter(Some(&[])).unwrap();
    assert!(validate_target_filter(Some(&[Missile, Missile])).is_err());
}
