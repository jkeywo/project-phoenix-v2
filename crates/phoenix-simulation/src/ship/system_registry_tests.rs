use super::*;

// ── Stable id string values ───────────────────────────────────────────────
// These tests pin the naming convention so a rename of a constant breaks CI
// rather than silently drifting the wire format.

#[test]
fn coarse_system_ids_are_lowercase_kebab() {
    let ids = [
        RED_ALERT_SYSTEM_ID,
        POWER_SYSTEM_ID,
        SENSORS_SYSTEM_ID,
        NAVIGATION_SYSTEM_ID,
        SHIELDS_SYSTEM_ID,
        COMMS_SYSTEM_ID,
        CAPTAIN_SYSTEM_ID,
        VIEWSCREEN_SYSTEM_ID,
        REPAIR_SYSTEM_ID,
        COMMAND_SYSTEM_ID,
    ];
    for id in ids {
        assert_eq!(
            id,
            id.to_lowercase(),
            "SystemId constant {id:?} is not lowercase"
        );
        assert!(
            !id.contains('_'),
            "SystemId constant {id:?} contains underscore (use hyphen)"
        );
        assert!(!id.is_empty(), "SystemId constant must not be empty");
    }
}

#[test]
fn coarse_system_id_values_are_stable() {
    assert_eq!(RED_ALERT_SYSTEM_ID, "red-alert");
    assert_eq!(POWER_SYSTEM_ID, "power");
    assert_eq!(SENSORS_SYSTEM_ID, "sensors");
    assert_eq!(NAVIGATION_SYSTEM_ID, "navigation");
    assert_eq!(SHIELDS_SYSTEM_ID, "shields");
    assert_eq!(COMMS_SYSTEM_ID, "comms");
    assert_eq!(CAPTAIN_SYSTEM_ID, "captain");
    assert_eq!(VIEWSCREEN_SYSTEM_ID, "viewscreen");
    assert_eq!(REPAIR_SYSTEM_ID, "repair");
    assert_eq!(COMMAND_SYSTEM_ID, "command");
}

#[test]
fn system_id_helpers_return_expected_values() {
    assert_eq!(red_alert_system_id().0, RED_ALERT_SYSTEM_ID);
    // No `power_system_id()` helper — see note above the station-key helpers.
    // The coarse constant is still pinned by `coarse_system_id_values_are_stable`.
    assert_eq!(sensors_system_id().0, SENSORS_SYSTEM_ID);
    assert_eq!(navigation_system_id().0, NAVIGATION_SYSTEM_ID);
    assert_eq!(shields_system_id().0, SHIELDS_SYSTEM_ID);
    assert_eq!(comms_system_id().0, COMMS_SYSTEM_ID);
    assert_eq!(captain_system_id().0, CAPTAIN_SYSTEM_ID);
    assert_eq!(viewscreen_system_id().0, VIEWSCREEN_SYSTEM_ID);
    assert_eq!(repair_system_id().0, REPAIR_SYSTEM_ID);
}

// ── Registry API ─────────────────────────────────────────────────────────

#[test]
fn register_adds_kind() {
    let mut registry = SystemKindRegistry::new();

    registry
        .register("red_alert", ConsoleFamily::Captain)
        .unwrap();

    assert!(registry.contains("red_alert"));
}

#[test]
fn every_core_descriptor_owns_its_console_family() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();
    let expected = [
        (RED_ALERT_KIND, ConsoleFamily::Captain),
        (POWER_KIND, ConsoleFamily::Power),
        (SENSORS_KIND, ConsoleFamily::Sensors),
        (NAVIGATION_KIND, ConsoleFamily::Navigation),
        (SHIELDS_KIND, ConsoleFamily::Shields),
        (COMMS_KIND, ConsoleFamily::Comms),
        (CAPTAIN_KIND, ConsoleFamily::Captain),
        (VIEWSCREEN_KIND, ConsoleFamily::Captain),
        (REPAIR_KIND, ConsoleFamily::Repair),
        (COMMAND_KIND, ConsoleFamily::Command),
        (TRACTOR_KIND, ConsoleFamily::Tractor),
        (DOCK_KIND, ConsoleFamily::Helm),
        (UMBILICAL_KIND, ConsoleFamily::Umbilical),
        (SECURITY_KIND, ConsoleFamily::Security),
        (TRANSPORTER_KIND, ConsoleFamily::Transporter),
        (HELM_JOYSTICK_KIND, ConsoleFamily::Helm),
        (HELM_ENGINE_KIND, ConsoleFamily::Helm),
        (HELM_RADAR_KIND, ConsoleFamily::Helm),
        (HELM_IMPULSE_KIND, ConsoleFamily::Helm),
        (LATERAL_THRUST_KIND, ConsoleFamily::Helm),
        (VERTICAL_THRUST_KIND, ConsoleFamily::Helm),
        (HELM_THRUST_KIND, ConsoleFamily::Helm),
        (HELM_STEERING_KIND, ConsoleFamily::Helm),
        (HELM_BOOST_KIND, ConsoleFamily::Helm),
        (PHASER_BANK_KIND, ConsoleFamily::Tactical),
        (TORPEDO_TUBE_KIND, ConsoleFamily::Tactical),
        (TORPEDO_MAGAZINE_KIND, ConsoleFamily::Tactical),
        (BLASTER_BANK_KIND, ConsoleFamily::Tactical),
        (PHASER_CONTROL_KIND, ConsoleFamily::Tactical),
        (TACTICAL_RADAR_KIND, ConsoleFamily::Tactical),
        (SENSOR_RADAR_KIND, ConsoleFamily::Sensors),
        (POWER_REACTOR_KIND, ConsoleFamily::Power),
        (POWER_BATTERY_KIND, ConsoleFamily::Power),
        (SHIELD_ARC_KIND, ConsoleFamily::Shields),
    ];

    assert_eq!(registry.kinds().count(), expected.len());
    for (kind, family) in expected {
        assert_eq!(
            registry
                .descriptor(kind)
                .map(SystemKindDescriptor::console_family),
            Some(family),
            "{kind:?} has the wrong presentation family"
        );
    }
}

#[test]
fn descriptors_own_admitted_command_capability() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        registry
            .descriptor(COMMAND_KIND)
            .is_some_and(SystemKindDescriptor::accepts_admitted_commands),
        "Command is an admitted-command capability"
    );
    assert!(
        registry
            .descriptor(SHIELD_ARC_KIND)
            .is_some_and(SystemKindDescriptor::accepts_admitted_commands),
        "every authored shield arc is an admitted-command capability"
    );
    assert!(
        !registry
            .descriptor(POWER_BATTERY_KIND)
            .is_some_and(SystemKindDescriptor::accepts_admitted_commands),
        "the battery is an observed/inter-system capability, not a ControlSystem target"
    );
}

#[test]
fn console_family_projection_resolves_instance_ids_by_kind() {
    use crate::ship::config::SystemInstanceConfig;

    let systems = vec![
        SystemInstanceConfig {
            id: SystemId("bridge-orders".into()),
            kind: COMMAND_KIND.into(),
            station: None,
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        },
        SystemInstanceConfig {
            id: SystemId("berthing-clamps".into()),
            kind: DOCK_KIND.into(),
            station: None,
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        },
        SystemInstanceConfig {
            id: SystemId("main-drive".into()),
            kind: HELM_THRUST_KIND.into(),
            station: None,
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        },
    ];

    let projected = SystemKindRegistry::with_core_systems()
        .unwrap()
        .project_console_families(&systems);
    assert_eq!(
        projected.get("bridge-orders"),
        Some(&ConsoleFamily::Command)
    );
    assert_eq!(projected.get("berthing-clamps"), Some(&ConsoleFamily::Helm));
    assert_eq!(projected.get("main-drive"), Some(&ConsoleFamily::Helm));
}

#[test]
fn every_shipped_system_instance_projects_its_descriptor_family() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .join("assets/entities");
    let mut entries: Vec<std::path::PathBuf> = crate::repo_fixtures::fs::read_dir(&dir)
        .expect("assets/entities must be readable")
        .map(|entry| entry.expect("readable entity entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    entries.sort();

    let registry = SystemKindRegistry::with_core_systems().unwrap();
    let mut hulls = 0usize;
    let mut systems = 0usize;
    let mut command_instances = 0usize;
    let mut command_targets = 0usize;
    let mut dock_instances = 0usize;

    for path in entries {
        let source = path.to_string_lossy().replace('\\', "/");
        let entity = crate::entities::include_resolve::load_entity_config(&source)
            .unwrap_or_else(|error| panic!("parse {source}: {error}"));
        let Some(ship) = entity.ship_config.as_ref() else {
            continue;
        };
        hulls += 1;
        command_targets += ship
            .stations
            .iter()
            .filter(|station| station.command_target.is_some())
            .count();
        let projected = registry.project_console_families(&ship.systems);
        for system in &ship.systems {
            systems += 1;
            command_instances += usize::from(system.kind == COMMAND_KIND);
            dock_instances += usize::from(system.kind == DOCK_KIND);
            let expected = registry
                .descriptor(&system.kind)
                .unwrap_or_else(|| panic!("{source}: missing descriptor for {:?}", system.kind))
                .console_family();
            assert_eq!(
                projected.get(&system.id.0),
                Some(&expected),
                "{source}: {:?} must reach the public topology projection by kind",
                system.id
            );
        }
    }

    assert!(hulls >= 10, "expected the complete shipped hull catalogue");
    assert!(
        systems >= 50,
        "expected every shipped hull's System topology"
    );
    // Command is authored per hull, not once for the fleet: the Destroyer
    // (issue #1107) and the Cruiser (issue #1387) each own one. Counted
    // against the hulls that actually DIRECT a Station rather than pinned
    // to a literal, so a third hull adopting Command extends the invariant
    // instead of failing it — while a `command` System with nothing to
    // direct, or a directing Station with no System to admit its orders,
    // still fails.
    assert!(
        command_instances >= 2,
        "the shipped Command topology disappeared, got {command_instances}"
    );
    assert_eq!(
        command_instances, command_targets,
        "every hull that authors a `command_target` owns exactly one \
             `command` System, and no hull owns one without directing anything"
    );
    assert!(
        dock_instances >= 4,
        "expected every shipped Dock tracer topology, got {dock_instances}"
    );
}

#[test]
fn rejects_empty_kind() {
    let mut registry = SystemKindRegistry::new();

    assert_eq!(
        registry.register("  ", ConsoleFamily::Captain),
        Err(SystemRegistryError::EmptyKind)
    );
}

#[test]
fn rejects_duplicate_kind() {
    let mut registry = SystemKindRegistry::new();
    registry
        .register("red_alert", ConsoleFamily::Captain)
        .unwrap();

    assert_eq!(
        registry.register("red_alert", ConsoleFamily::Captain),
        Err(SystemRegistryError::DuplicateKind {
            kind: "red_alert".into()
        })
    );
}

#[test]
fn reserved_blackboard_keys_are_complete_and_not_system_kinds() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();
    let expected = HashMap::from([
        (HELM_STATION_ID.to_string(), ConsoleFamily::Helm),
        (TACTICAL_STATION_ID.to_string(), ConsoleFamily::Tactical),
        (POWER_SYSTEM_ID.to_string(), ConsoleFamily::Power),
        (SHIELDS_SYSTEM_ID.to_string(), ConsoleFamily::Shields),
        (
            crate::dossier::DOSSIER_BLACKBOARD_KEY.to_string(),
            ConsoleFamily::Comms,
        ),
        (
            crate::science::SCAN_BLACKBOARD_KEY.to_string(),
            ConsoleFamily::Sensors,
        ),
    ]);

    assert_eq!(registry.project_blackboard_console_families(), expected);
    for key in [
        HELM_STATION_ID,
        TACTICAL_STATION_ID,
        crate::dossier::DOSSIER_BLACKBOARD_KEY,
        crate::science::SCAN_BLACKBOARD_KEY,
    ] {
        assert!(
            !registry.contains(key),
            "reserved blackboard key {key:?} must not masquerade as a System kind"
        );
    }
    // `power` and `shields` legitimately exist in both namespaces: their
    // coarse System kinds remain valid while aggregate blackboards use the
    // same wire strings. Separate descriptor tables preserve the meanings.
    for key in [POWER_SYSTEM_ID, SHIELDS_SYSTEM_ID] {
        assert!(registry.contains(key));
        assert!(registry.blackboard_descriptor(key).is_some());
    }
}

#[test]
fn rejects_empty_and_duplicate_blackboard_keys() {
    let mut registry = SystemKindRegistry::new();
    assert_eq!(
        registry.register_blackboard_key("  ", ConsoleFamily::Captain),
        Err(SystemRegistryError::EmptyBlackboardKey)
    );
    registry
        .register_blackboard_key("aggregate", ConsoleFamily::Captain)
        .unwrap();
    assert_eq!(
        registry.register_blackboard_key("aggregate", ConsoleFamily::Helm),
        Err(SystemRegistryError::DuplicateBlackboardKey {
            key: "aggregate".into()
        })
    );
}

#[test]
fn red_alert_registry_contains_kind() {
    let registry = SystemKindRegistry::with_red_alert().unwrap();

    assert!(registry.contains(RED_ALERT_KIND));
}

/// The coarse `helm` / `tactical` kinds were deleted by #801: `"helm"`
/// and `"tactical"` are station ids only. A ship TOML declaring either
/// as a `[[system]]` kind must fail validation.
#[test]
fn coarse_helm_and_tactical_kinds_are_not_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        !registry.contains("helm"),
        "coarse `helm` must not be a registered system kind"
    );
    assert!(
        !registry.contains("tactical"),
        "coarse `tactical` must not be a registered system kind"
    );
}

/// Station-id keys are stable wire/blackboard strings — the client reads
/// `blackboards['helm']` / `blackboards['tactical']`, so these must never
/// drift.
#[test]
fn station_key_values_are_stable() {
    assert_eq!(HELM_STATION_ID, "helm");
    assert_eq!(TACTICAL_STATION_ID, "tactical");
    assert_eq!(helm_station_key().0, HELM_STATION_ID);
    assert_eq!(tactical_station_key().0, TACTICAL_STATION_ID);
}

#[test]
fn core_registry_contains_all_coarse_kinds() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    for kind in [
        RED_ALERT_KIND,
        POWER_KIND,
        SENSORS_KIND,
        NAVIGATION_KIND,
        SHIELDS_KIND,
        COMMS_KIND,
        CAPTAIN_KIND,
        VIEWSCREEN_KIND,
        REPAIR_KIND,
    ] {
        assert!(
            registry.contains(kind),
            "coarse kind {kind:?} not registered"
        );
    }
}

// ── Fine Helm system tests (issue #511) ───────────────────────────────────

#[test]
fn fine_helm_kinds_are_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        registry.contains(HELM_JOYSTICK_KIND),
        "helm_joystick not registered"
    );
    assert!(
        registry.contains(HELM_ENGINE_KIND),
        "helm_engine not registered"
    );
    assert!(
        registry.contains(HELM_RADAR_KIND),
        "helm_radar not registered"
    );
    assert!(
        registry.contains(HELM_IMPULSE_KIND),
        "helm_impulse not registered"
    );
    assert!(
        registry.contains(LATERAL_THRUST_KIND),
        "lateral_thrust not registered"
    );
}

/// The per-axis Helm kinds (issue #701) must be registered like every
/// other fine kind, or a ship TOML naming them fails `ShipConfig` parse.
#[test]
fn per_axis_helm_kinds_are_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        registry.contains(HELM_THRUST_KIND),
        "helm_thrust not registered"
    );
    assert!(
        registry.contains(HELM_STEERING_KIND),
        "helm_steering not registered"
    );
    assert!(
        registry.contains(HELM_BOOST_KIND),
        "helm_boost not registered"
    );
}

#[test]
fn fine_helm_system_ids_are_lowercase_kebab() {
    let ids = [
        HELM_JOYSTICK_SYSTEM_ID,
        HELM_ENGINE_PORT_SYSTEM_ID,
        HELM_ENGINE_STARBOARD_SYSTEM_ID,
        HELM_RADAR_SYSTEM_ID,
        HELM_IMPULSE_SYSTEM_ID,
        LATERAL_THRUST_SYSTEM_ID,
        HELM_THRUST_SYSTEM_ID,
        HELM_STEERING_SYSTEM_ID,
        HELM_BOOST_SYSTEM_ID,
    ];
    for id in ids {
        assert_eq!(
            id,
            id.to_lowercase(),
            "Fine helm SystemId {id:?} is not lowercase"
        );
        assert!(
            !id.contains('_'),
            "Fine helm SystemId {id:?} contains underscore (use hyphen)"
        );
        assert!(!id.is_empty(), "Fine helm SystemId must not be empty");
    }
    assert_eq!(HELM_JOYSTICK_SYSTEM_ID, "helm-joystick");
    assert_eq!(HELM_ENGINE_PORT_SYSTEM_ID, "helm-engine-port");
    assert_eq!(HELM_ENGINE_STARBOARD_SYSTEM_ID, "helm-engine-starboard");
    assert_eq!(HELM_RADAR_SYSTEM_ID, "helm-radar");
    assert_eq!(HELM_IMPULSE_SYSTEM_ID, "helm-impulse");
    assert_eq!(LATERAL_THRUST_SYSTEM_ID, "helm-lateral-thrust");
    assert_eq!(HELM_THRUST_SYSTEM_ID, "helm-thrust");
    assert_eq!(HELM_STEERING_SYSTEM_ID, "helm-steering");
    assert_eq!(HELM_BOOST_SYSTEM_ID, "helm-boost");
}

#[test]
fn fine_helm_system_id_helpers_return_expected_values() {
    assert_eq!(helm_joystick_system_id().0, HELM_JOYSTICK_SYSTEM_ID);
    assert_eq!(helm_engine_port_system_id().0, HELM_ENGINE_PORT_SYSTEM_ID);
    assert_eq!(
        helm_engine_starboard_system_id().0,
        HELM_ENGINE_STARBOARD_SYSTEM_ID
    );
    assert_eq!(helm_radar_system_id().0, HELM_RADAR_SYSTEM_ID);
    assert_eq!(helm_impulse_system_id().0, HELM_IMPULSE_SYSTEM_ID);
    assert_eq!(lateral_thrust_system_id().0, LATERAL_THRUST_SYSTEM_ID);
    assert_eq!(helm_thrust_system_id().0, HELM_THRUST_SYSTEM_ID);
    assert_eq!(helm_steering_system_id().0, HELM_STEERING_SYSTEM_ID);
    assert_eq!(helm_boost_system_id().0, HELM_BOOST_SYSTEM_ID);
}

// ── Fine Tactical system tests (issue #512) ───────────────────────────────

#[test]
fn fine_tactical_kinds_are_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        registry.contains(PHASER_BANK_KIND),
        "phaser_bank not registered"
    );
    assert!(
        registry.contains(TORPEDO_TUBE_KIND),
        "torpedo_tube not registered"
    );
    assert!(
        registry.contains(TORPEDO_MAGAZINE_KIND),
        "torpedo_magazine not registered"
    );
    assert!(
        registry.contains(PHASER_CONTROL_KIND),
        "phaser_control not registered"
    );
}

#[test]
fn fine_tactical_system_ids_are_lowercase_kebab() {
    let ids = [
        PHASER_FORE_SYSTEM_ID,
        PHASER_AFT_SYSTEM_ID,
        TORPEDO_TUBE_FORE_PORT_SYSTEM_ID,
        TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
        TORPEDO_TUBE_AFT_SYSTEM_ID,
        TORPEDO_MAGAZINE_SYSTEM_ID,
        PHASER_CONTROL_SYSTEM_ID,
    ];
    for id in ids {
        assert_eq!(
            id,
            id.to_lowercase(),
            "Fine tactical SystemId {id:?} is not lowercase"
        );
        assert!(
            !id.contains('_'),
            "Fine tactical SystemId {id:?} contains underscore (use hyphen)"
        );
        assert!(!id.is_empty(), "Fine tactical SystemId must not be empty");
    }
    assert_eq!(PHASER_FORE_SYSTEM_ID, "phaser-fore");
    assert_eq!(PHASER_AFT_SYSTEM_ID, "phaser-aft");
    assert_eq!(TORPEDO_TUBE_FORE_PORT_SYSTEM_ID, "torpedo-tube-fore-port");
    assert_eq!(
        TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
        "torpedo-tube-fore-starboard"
    );
    assert_eq!(TORPEDO_TUBE_AFT_SYSTEM_ID, "torpedo-tube-aft");
    assert_eq!(TORPEDO_MAGAZINE_SYSTEM_ID, "torpedo-magazine");
    assert_eq!(PHASER_CONTROL_SYSTEM_ID, "phaser-control");
}

#[test]
fn fine_tactical_system_id_helpers_return_expected_values() {
    assert_eq!(phaser_fore_system_id().0, PHASER_FORE_SYSTEM_ID);
    assert_eq!(phaser_aft_system_id().0, PHASER_AFT_SYSTEM_ID);
    assert_eq!(
        torpedo_tube_fore_port_system_id().0,
        TORPEDO_TUBE_FORE_PORT_SYSTEM_ID
    );
    assert_eq!(
        torpedo_tube_fore_starboard_system_id().0,
        TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID
    );
    assert_eq!(torpedo_tube_aft_system_id().0, TORPEDO_TUBE_AFT_SYSTEM_ID);
    assert_eq!(torpedo_magazine_system_id().0, TORPEDO_MAGAZINE_SYSTEM_ID);
    assert_eq!(phaser_control_system_id().0, PHASER_CONTROL_SYSTEM_ID);
}

#[test]
fn phaser_bank_system_id_resolves_known_ids() {
    assert_eq!(phaser_bank_system_id("fore"), Some(phaser_fore_system_id()));
    assert_eq!(phaser_bank_system_id("aft"), Some(phaser_aft_system_id()));
    // Derives arbitrary bank ids via the `phaser-<id>` naming convention;
    // NPC ships that declare e.g. "port"/"starboard" get their own fine
    // SystemIds without needing a match-arm update.
    assert_eq!(
        phaser_bank_system_id("port"),
        Some(SystemId("phaser-port".into()))
    );
    assert_eq!(
        phaser_bank_system_id("starboard"),
        Some(SystemId("phaser-starboard".into()))
    );
    assert_eq!(phaser_bank_system_id(""), None);
}

#[test]
fn torpedo_tube_system_id_resolves_known_ids() {
    assert_eq!(
        torpedo_tube_system_id("fore_port"),
        Some(torpedo_tube_fore_port_system_id())
    );
    assert_eq!(
        torpedo_tube_system_id("fore_starboard"),
        Some(torpedo_tube_fore_starboard_system_id())
    );
    assert_eq!(
        torpedo_tube_system_id("aft"),
        Some(torpedo_tube_aft_system_id())
    );
    // Underscore-to-hyphen conversion is the convention.
    assert_eq!(
        torpedo_tube_system_id("dorsal_upper"),
        Some(SystemId("torpedo-tube-dorsal-upper".into()))
    );
    assert_eq!(torpedo_tube_system_id(""), None);
}

// ── Fine Blaster system tests (issue #631) ────────────────────────────────

#[test]
fn blaster_bank_kind_is_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();
    assert!(
        registry.contains(BLASTER_BANK_KIND),
        "blaster_bank not registered"
    );
}

#[test]
fn blaster_bank_kind_constant_is_correct() {
    assert_eq!(BLASTER_BANK_KIND, "blaster_bank");
}

#[test]
fn blaster_bank_system_id_resolves_known_ids() {
    assert_eq!(
        blaster_bank_system_id("fore"),
        Some(SystemId("blaster-fore".into()))
    );
    assert_eq!(
        blaster_bank_system_id("aft"),
        Some(SystemId("blaster-aft".into()))
    );
    assert_eq!(
        blaster_bank_system_id("fore_port"),
        Some(SystemId("blaster-fore-port".into()))
    );
    assert_eq!(blaster_bank_system_id(""), None);
}

#[test]
fn blaster_bank_system_ids_are_lowercase_kebab() {
    for bank_id in &["fore", "aft", "port", "starboard", "fore_port"] {
        let sid = blaster_bank_system_id(bank_id).expect("non-empty id should produce a SystemId");
        assert_eq!(
            sid.0,
            sid.0.to_lowercase(),
            "SystemId {sid:?} for blaster bank {bank_id:?} is not lowercase"
        );
        assert!(
            !sid.0.contains('_'),
            "SystemId {sid:?} for blaster bank {bank_id:?} contains underscore"
        );
        assert!(
            sid.0.starts_with("blaster-"),
            "SystemId {sid:?} must start with blaster-"
        );
    }
}

// ── Fine Tactical/Sensor Radar system tests ───────────────────────────────

#[test]
fn radar_kinds_are_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        registry.contains(TACTICAL_RADAR_KIND),
        "tactical_radar not registered"
    );
    assert!(
        registry.contains(SENSOR_RADAR_KIND),
        "sensor_radar not registered"
    );
}

#[test]
fn radar_system_ids_are_lowercase_kebab() {
    let ids = [TACTICAL_RADAR_SYSTEM_ID, SENSOR_RADAR_SYSTEM_ID];
    for id in ids {
        assert_eq!(
            id,
            id.to_lowercase(),
            "Radar SystemId {id:?} is not lowercase"
        );
        assert!(
            !id.contains('_'),
            "Radar SystemId {id:?} contains underscore (use hyphen)"
        );
        assert!(!id.is_empty(), "Radar SystemId must not be empty");
    }
    assert_eq!(TACTICAL_RADAR_SYSTEM_ID, "tactical-radar");
    assert_eq!(SENSOR_RADAR_SYSTEM_ID, "sensor-radar");
}

#[test]
fn radar_system_id_helpers_return_expected_values() {
    assert_eq!(tactical_radar_system_id().0, TACTICAL_RADAR_SYSTEM_ID);
    assert_eq!(sensor_radar_system_id().0, SENSOR_RADAR_SYSTEM_ID);
}

// ── Fine Power system tests (issue #513) ──────────────────────────────────

#[test]
fn fine_power_kinds_are_registered() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();

    assert!(
        registry.contains(POWER_REACTOR_KIND),
        "power_reactor not registered"
    );
    assert!(
        registry.contains(POWER_BATTERY_KIND),
        "power_battery not registered"
    );
}

#[test]
fn fine_power_system_ids_are_lowercase_kebab() {
    let ids = [POWER_REACTOR_SYSTEM_ID, POWER_BATTERY_SYSTEM_ID];
    for id in ids {
        assert_eq!(
            id,
            id.to_lowercase(),
            "Fine power SystemId {id:?} is not lowercase"
        );
        assert!(
            !id.contains('_'),
            "Fine power SystemId {id:?} contains underscore (use hyphen)"
        );
        assert!(!id.is_empty(), "Fine power SystemId must not be empty");
    }
    assert_eq!(POWER_REACTOR_SYSTEM_ID, "power-reactor");
    assert_eq!(POWER_BATTERY_SYSTEM_ID, "power-battery");
}

#[test]
fn fine_power_system_id_helpers_return_expected_values() {
    assert_eq!(power_reactor_system_id().0, POWER_REACTOR_SYSTEM_ID);
    assert_eq!(power_battery_system_id().0, POWER_BATTERY_SYSTEM_ID);
}

// ── Fine Shields system tests (issue #514) ────────────────────────────────

#[test]
fn shield_arcs_registered_in_system_kind_registry() {
    let registry = SystemKindRegistry::with_core_systems().unwrap();
    assert!(
        registry.contains(SHIELD_ARC_KIND),
        "shield_arc not registered"
    );
}

#[test]
fn shield_arc_kind_string_is_lowercase_snake() {
    // Kind key uses snake_case per registry convention (matches
    // `phaser_bank`, `power_reactor` etc.).
    assert_eq!(SHIELD_ARC_KIND, "shield_arc");
}

#[test]
fn shield_arc_system_id_helper_returns_expected_shape() {
    assert_eq!(
        shield_arc_system_id("fore"),
        Some(SystemId("shield-arc-fore".into()))
    );
    assert_eq!(
        shield_arc_system_id("port"),
        Some(SystemId("shield-arc-port".into()))
    );
    assert_eq!(
        shield_arc_system_id("aft"),
        Some(SystemId("shield-arc-aft".into()))
    );
    assert_eq!(
        shield_arc_system_id("starboard"),
        Some(SystemId("shield-arc-starboard".into()))
    );
    // Single-omni NPC arc id
    assert_eq!(
        shield_arc_system_id("all"),
        Some(SystemId("shield-arc-all".into()))
    );
    // Underscore-to-hyphen conversion is the convention.
    assert_eq!(
        shield_arc_system_id("dorsal_upper"),
        Some(SystemId("shield-arc-dorsal-upper".into()))
    );
    assert_eq!(shield_arc_system_id(""), None);
}

#[test]
fn shield_arc_kebab_case_conformance() {
    // Every SystemId synthesised for shield arcs must be lowercase kebab.
    for arc_id in &["fore", "port", "aft", "starboard", "all", "dorsal_upper"] {
        let sid = shield_arc_system_id(arc_id).expect("non-empty id should synthesise");
        assert_eq!(
            sid.0,
            sid.0.to_lowercase(),
            "SystemId {sid:?} for arc {arc_id:?} is not lowercase"
        );
        assert!(
            !sid.0.contains('_'),
            "SystemId {sid:?} for arc {arc_id:?} contains underscore (use hyphen)"
        );
        assert!(
            sid.0.starts_with("shield-arc-"),
            "SystemId {sid:?} must start with shield-arc-"
        );
    }
}
