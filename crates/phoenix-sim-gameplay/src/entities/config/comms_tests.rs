use super::*;

#[test]
fn all_shipped_seat_counts_bind_real_paid_radio_equipment() {
    for hull in ["courier", "destroyer", "cruiser", "battleship"] {
        let source = crate::test_fixtures::resolve_from_disk(&format!(
            "assets/entities/alliance_{hull}.toml"
        ))
        .unwrap();
        let config = EntityConfig::from_toml(&source.toml).unwrap();
        validate_radio(&config).unwrap();
        assert_eq!(
            config
                .comms
                .as_ref()
                .unwrap()
                .radio
                .as_ref()
                .unwrap()
                .channels
                .len(),
            2
        );
        for continuous in [false, true] {
            let mut unpaid = config.clone();
            let cost = unpaid
                .energy
                .as_mut()
                .unwrap()
                .consumers
                .iter_mut()
                .find(|consumer| consumer.system.0 == "comms")
                .unwrap();
            if continuous {
                cost.continuous_cost_per_sec = 0.0;
            } else {
                cost.discrete_cost = 0.0;
            }
            assert!(
                validate_radio(&unpaid).is_err(),
                "{hull} must not turn authored paid work into a free grant"
            );
        }
        let mut wrong = config.clone();
        wrong
            .comms
            .as_mut()
            .unwrap()
            .radio
            .as_mut()
            .unwrap()
            .system
            .0 = "helm-thrust".into();
        assert!(validate_radio(&wrong).is_err());
    }
}
