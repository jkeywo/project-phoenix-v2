use super::*;

#[test]
fn legacy_scalar_capacity_payload_reports_format_refusal_before_schema_decoding() {
    #[derive(serde::Serialize)]
    struct LegacyCapacity {
        id: String,
        level: i64,
        ceiling: i64,
    }
    #[derive(serde::Serialize)]
    struct LegacyInfrastructure {
        capacities: Vec<LegacyCapacity>,
    }
    #[derive(serde::Serialize)]
    struct LegacyEntity {
        uuid: String,
        infrastructure: Option<LegacyInfrastructure>,
    }
    #[derive(serde::Serialize)]
    struct LegacyState {
        entities: Vec<LegacyEntity>,
    }
    let previous = Run::<LoggedCommand, LegacyState> {
        versions: Versions::new(41, "0.13", 0),
        scenario: "legacy-capacity".into(),
        seed: 1,
        snapshot: Some(vellum_save::Snapshot {
            tick: 0,
            digest: 0,
            state: LegacyState {
                entities: vec![LegacyEntity {
                    uuid: "depot".into(),
                    infrastructure: Some(LegacyInfrastructure {
                        capacities: vec![LegacyCapacity {
                            id: "fuel".into(),
                            level: 2,
                            ceiling: 3,
                        }],
                    }),
                }],
            },
        }),
        commands: Vec::new(),
        ledger: Default::default(),
    };
    let text = ron::ser::to_string(&previous).unwrap();
    let store = TransferStore::holding(DEFAULT_SLOT, text);
    assert!(matches!(
        load_from(
            &store,
            DEFAULT_SLOT,
            &Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0)
        ),
        Err(LoadRefusal::Moved(vellum_save::Moved::Format { .. }))
    ));
}

#[test]
fn format_17_without_apply_outcomes_or_pending_station_commands_is_refused() {
    let previous = vellum_save::Versions::new(17, SIMULATION_RULES, 0);
    let current = vellum_save::Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0);
    assert!(matches!(
        previous
            .check(&current)
            .expect_err("format 17 cannot distinguish Applied from an undelivered payload"),
        vellum_save::Moved::Format { .. },
    ));
}

#[test]
fn format_21_without_the_armed_direct_effects_is_refused() {
    let previous = vellum_save::Versions::new(21, SIMULATION_RULES, 0);
    let current = vellum_save::Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0);
    assert!(matches!(
        previous
            .check(&current)
            .expect_err("format 21 records an Applied hit nothing will ever land"),
        vellum_save::Moved::Format { .. },
    ));
}
