use super::*;

#[test]
fn dated_matching_is_one_distinct_retained_request_without_receipt_refresh() {
    let original = ThrustIntent::Assisted {
        demand: SpeedDemand::Match {
            contact: "5:radar:contact-2".into(),
            speed: 17.0,
            observed_tick: 42,
        },
    };
    assert!(!original.valid_at(41));
    assert!(original.valid_at(42));
    assert!(original.valid_at(1000));
    let restored: ThrustIntent =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(restored, original);
    for speed in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(!ThrustIntent::Assisted {
            demand: SpeedDemand::Match {
                contact: "5:radar:contact-2".into(),
                speed,
                observed_tick: 42,
            },
        }
        .valid_at(42));
    }
}

#[test]
fn force_speed_and_heading_requests_are_closed_and_preserve_their_units() {
    assert!(ThrustIntent::Direct { force: -1.0 }.valid_at(0));
    assert!(!ThrustIntent::Direct { force: 1.1 }.valid_at(0));
    assert!(ThrustIntent::Assisted {
        demand: SpeedDemand::Absolute { speed: -30.0 },
    }
    .valid_at(0));
    assert!(SteeringIntent::Assisted {
        demand: SteeringDemand::Heading { heading_rad: 1.5 },
    }
    .is_valid());
    for heading_rad in [-1.0, std::f32::consts::TAU, f32::NAN, f32::INFINITY] {
        assert!(!SteeringIntent::Assisted {
            demand: SteeringDemand::Heading { heading_rad },
        }
        .is_valid());
    }
    assert!(serde_json::from_str::<ThrustIntent>(
        r#"{"mode":"direct","force":0.5,"demand":{"kind":"absolute","speed":20}}"#
    )
    .is_err());
    assert!(
        serde_json::from_str::<SpeedDemand>(r#"{"kind":"live_target","uuid":"private"}"#).is_err()
    );
    assert!(serde_json::from_str::<HelmControlMode>(r#""autopilot""#).is_err());
}
