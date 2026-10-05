use super::*;

#[test]
fn exact_charge_pays_once_then_insufficient_attack_shuts_every_weapon_off() {
    let weapon = StrikeWeaponConfig {
        cost: 5.0,
        damage_multiplier: 2.0,
    };
    let mut boost = StrikeBoost::default();
    let mut charge = 5.0;
    boost.set(true);
    assert_eq!(boost.fire(&mut charge, Some(&weapon)), 2.0);
    assert_eq!(charge, 0.0);
    assert_eq!(boost.fire(&mut charge, Some(&weapon)), 1.0);
    assert!(boost.depleted);
    charge = 10.0;
    assert_eq!(boost.fire(&mut charge, Some(&weapon)), 1.0);
    assert_eq!(charge, 10.0, "recharging does not re-enable boost");
    boost.set(true);
    assert_eq!(boost.fire(&mut charge, Some(&weapon)), 2.0);
    assert_eq!(charge, 5.0);
}

#[test]
fn restored_boost_spends_identically_across_weapon_costs() {
    let cheap = StrikeWeaponConfig {
        cost: 3.0,
        damage_multiplier: 1.5,
    };
    let expensive = StrikeWeaponConfig {
        cost: 8.0,
        damage_multiplier: 3.0,
    };
    let mut live = StrikeBoost::default();
    live.set(true);
    let mut restored: StrikeBoost =
        serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
    let mut live_charge = 10.0;
    let mut restored_charge = live_charge;
    for weapon in [&cheap, &expensive, &cheap] {
        assert_eq!(
            live.fire(&mut live_charge, Some(weapon)),
            restored.fire(&mut restored_charge, Some(weapon))
        );
        assert_eq!(live_charge, restored_charge);
        assert_eq!(live, restored);
    }
    assert_eq!(
        live_charge, 7.0,
        "failed expensive shot disables the later cheap shot"
    );
}
