use super::*;
use crate::core::messages::{AdmittedCommand, SystemControlPayload, SystemId};

fn system(id: &str, kind: &str) -> crate::ship::config::SystemInstanceConfig {
    crate::ship::config::SystemInstanceConfig {
        id: SystemId(id.into()),
        kind: kind.into(),
        station: None,
        ai_only: true,
        human_seeking: false,
        seek_order: Vec::new(),
        power_group: None,
        marker: None,
        config: None,
    }
}

fn ship_config(
    systems: Vec<crate::ship::config::SystemInstanceConfig>,
) -> crate::ship::config::ShipConfig {
    crate::ship::config::ShipConfig {
        stations: Vec::new(),
        systems,
        power_groups: std::collections::HashMap::new(),
        coordination_lag_secs: 0.0,
    }
}

fn admitted(targets: &[&str]) -> AdmittedCommands {
    AdmittedCommands(
        targets
            .iter()
            .map(|t| AdmittedCommand {
                target: SystemId((*t).into()),
                payload: SystemControlPayload::SetRedAlert { active: true },
                response_token: None,
                feedback_correlation: None,
            })
            .collect(),
    )
}

#[test]
fn undeclared_exact_matcher_routes_only_its_id() {
    let mut reg = AdmittedConsumerRegistry::default();
    reg.register(ConsumerMatcher::undeclared_exact("god-mode"));
    assert!(reg.is_routed("god-mode"));
    assert!(!reg.is_routed("god-mode-extra"));
    assert!(!reg.is_system_routed(&system("god-mode", "debug")));
}

#[test]
fn kind_matcher_routes_arbitrary_instance_ids() {
    let mut reg = AdmittedConsumerRegistry::default();
    reg.register(ConsumerMatcher::kind(
        crate::ship::system_registry::DOCK_KIND,
    ));

    assert!(reg.is_system_routed(&system(
        "berthing-clamps",
        crate::ship::system_registry::DOCK_KIND,
    )));
    assert!(
        !reg.is_routed("berthing-clamps"),
        "raw target matching must not infer a System kind without topology"
    );
}

#[test]
fn register_is_idempotent() {
    let mut reg = AdmittedConsumerRegistry::default();
    reg.register(ConsumerMatcher::exact("comms", "comms"));
    reg.register(ConsumerMatcher::exact("comms", "comms"));
    assert_eq!(reg.len(), 1);
}

#[test]
fn unrouted_targets_reports_only_unregistered_and_dedupes() {
    let mut reg = AdmittedConsumerRegistry::default();
    reg.register(ConsumerMatcher::undeclared_exact("sensors"));
    // Two "sensors" (registered) + two "ghost" (not) → only one "ghost".
    let cmds = admitted(&["sensors", "ghost", "sensors", "ghost"]);
    assert_eq!(unrouted_command_targets(&cmds, None, &reg), vec!["ghost"]);
}

#[test]
fn a_fully_registered_tick_has_no_unrouted_targets() {
    let mut reg = AdmittedConsumerRegistry::default();
    reg.register(ConsumerMatcher::undeclared_exact("sensors"));
    reg.register(ConsumerMatcher::undeclared_exact("power-reactor"));
    let cmds = admitted(&["sensors", "power-reactor"]);
    assert!(unrouted_command_targets(&cmds, None, &reg).is_empty());
}

/// Build through the same two registration entry points production uses:
/// canonical simulation composition, then the World plugin that owns the
/// Tractor/Dock/Umbilical family.
fn production_consumer_registry_app() -> App {
    let mut app = App::new();
    crate::server_app::add_simulation_plugins_with(
        &mut app,
        crate::server_app::SimPluginOptions {
            render: false,
            ..Default::default()
        },
    );
    app.add_plugins(crate::world::server::WorldPlugin);
    app
}

/// Every top-level `assets/entities/*.toml` ship, resolved through the
/// production include loader. Subdirectories are fixtures/fragments rather
/// than shipped hulls (the same fleet boundary as the other authored-content
/// censuses).
fn shipped_ship_configs() -> Vec<(String, crate::ship::config::ShipConfig)> {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .join("assets/entities");
    let mut entries: Vec<std::path::PathBuf> = crate::repo_fixtures::fs::read_dir(&dir)
        .expect("assets/entities must be readable")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    entries.sort();

    entries
        .into_iter()
        .filter_map(|path| {
            let stem = path
                .file_stem()
                .expect("TOML path has a stem")
                .to_string_lossy()
                .to_string();
            let key = path.to_string_lossy().replace('\\', "/");
            let config = crate::entities::include_resolve::load_entity_config(&key)
                .unwrap_or_else(|error| panic!("{stem} must load: {error}"));
            config.ship_config.map(|ship| (stem, ship))
        })
        .collect()
}

/// The descriptor-derived completeness invariant. There is no expected-id
/// fixture here: every shipped System instance whose authoritative kind
/// descriptor says it accepts admitted commands must be claimed by the
/// registry assembled through production composition.
#[test]
fn every_commandable_shipped_system_has_a_production_consumer() {
    use crate::ship::system_registry as sr;

    let app = production_consumer_registry_app();
    let consumers = app.world().resource::<AdmittedConsumerRegistry>();
    let descriptors = sr::SystemKindRegistry::with_core_systems().unwrap();
    let mut covered_kinds = std::collections::BTreeSet::new();
    let mut missing = Vec::new();

    // Descriptor completeness does not depend on a kind already having a
    // shipped instance. This checks only that production names an address
    // domain for every commandable descriptor; the shipped-hull census
    // below separately proves each real instance falls inside that domain.
    let missing_descriptor_kinds: Vec<_> = descriptors
        .kinds()
        .filter(|kind| {
            descriptors
                .descriptor(kind)
                .is_some_and(|descriptor| descriptor.accepts_admitted_commands())
                && !consumers.claims_kind(kind)
        })
        .collect();
    assert!(
        missing_descriptor_kinds.is_empty(),
        "commandable descriptors without a production consumer: {}",
        missing_descriptor_kinds.join(", ")
    );

    for (hull, config) in shipped_ship_configs() {
        for system in &config.systems {
            let descriptor = descriptors
                .descriptor(&system.kind)
                .unwrap_or_else(|| panic!("{hull}: unknown System kind `{}`", system.kind));
            if descriptor.accepts_admitted_commands() {
                covered_kinds.insert(system.kind.clone());
            }
        }
        for system in unrouted_commandable_systems(&config.systems, &descriptors, consumers) {
            missing.push(format!("{hull}: {} (kind {})", system.id.0, system.kind));
        }
    }

    assert!(
        missing.is_empty(),
        "commandable Systems without a production consumer:\n{}",
        missing.join("\n")
    );

    // Acceptance coverage guard: these are kinds, not representative ids.
    // A content edit that removed the last real instance would otherwise
    // make this issue's named dynamic/auxiliary cases vacuous.
    for required in [
        sr::COMMAND_KIND,
        sr::TRACTOR_KIND,
        sr::DOCK_KIND,
        sr::UMBILICAL_KIND,
        sr::PHASER_BANK_KIND,
        sr::BLASTER_BANK_KIND,
        sr::TORPEDO_TUBE_KIND,
        sr::SHIELD_ARC_KIND,
    ] {
        assert!(
            covered_kinds.contains(required),
            "the shipped-hull census did not exercise commandable kind `{required}`"
        );
    }
}

#[test]
fn production_helm_consumers_route_canonical_and_authored_instance_ids_by_kind() {
    use crate::ship::system_registry as sr;

    let app = production_consumer_registry_app();
    let consumers = app.world().resource::<AdmittedConsumerRegistry>();
    let descriptors = sr::SystemKindRegistry::with_core_systems().expect("core descriptors");
    let cases = [
        (sr::HELM_THRUST_KIND, sr::HELM_THRUST_SYSTEM_ID),
        (sr::HELM_STEERING_KIND, sr::HELM_STEERING_SYSTEM_ID),
        (sr::HELM_IMPULSE_KIND, sr::HELM_IMPULSE_SYSTEM_ID),
        (sr::LATERAL_THRUST_KIND, sr::LATERAL_THRUST_SYSTEM_ID),
        (sr::VERTICAL_THRUST_KIND, sr::VERTICAL_THRUST_SYSTEM_ID),
        (sr::HELM_BOOST_KIND, sr::HELM_BOOST_SYSTEM_ID),
    ];

    let mut systems = Vec::new();
    for (kind, canonical_id) in cases {
        systems.push(system(canonical_id, kind));
        systems.push(system(&format!("alternate-{canonical_id}"), kind));
    }
    let config = ship_config(systems);
    let targets: Vec<_> = config
        .systems
        .iter()
        .map(|system| system.id.0.as_str())
        .collect();
    let commands = admitted(&targets);

    assert!(
        unrouted_command_targets(&commands, Some(&config), consumers).is_empty(),
        "canonical and ship-authored Helm ids must resolve through their System kind"
    );
    assert!(
        unrouted_commandable_systems(&config.systems, &descriptors, consumers).is_empty(),
        "every canonical and ship-authored Helm instance must be claimed by production"
    );
}

#[test]
fn missing_commandable_consumer_is_detected_but_passive_kind_is_ignored() {
    let mut descriptors = crate::ship::system_registry::SystemKindRegistry::new();
    descriptors
        .register_commandable(
            "test_commandable",
            crate::core::messages::ConsoleFamily::Command,
        )
        .unwrap();
    descriptors
        .register(
            "test_passive",
            crate::core::messages::ConsoleFamily::Command,
        )
        .unwrap();
    let systems = vec![
        system("orders", "test_commandable"),
        system("telemetry", "test_passive"),
    ];
    let mut consumers = AdmittedConsumerRegistry::default();

    let missing = unrouted_commandable_systems(&systems, &descriptors, &consumers);
    assert_eq!(
        missing
            .iter()
            .map(|system| system.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["orders"],
        "a passive capability must not become a false coverage failure"
    );

    consumers.register(ConsumerMatcher::kind("test_commandable"));
    assert!(unrouted_commandable_systems(&systems, &descriptors, &consumers).is_empty());
}

#[test]
fn fixed_target_domain_rejects_noncanonical_instance_but_dock_accepts_arbitrary_id() {
    let app = production_consumer_registry_app();
    let consumers = app.world().resource::<AdmittedConsumerRegistry>();
    let descriptors = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("core descriptors");
    let config = ship_config(vec![
        system("tow-a", crate::ship::system_registry::TRACTOR_KIND),
        system("berthing-clamps", crate::ship::system_registry::DOCK_KIND),
    ]);

    let commands = admitted(&["tow-a", "berthing-clamps"]);
    assert_eq!(
        unrouted_command_targets(&commands, Some(&config), consumers),
        vec!["tow-a"],
        "Tractor reads its canonical id, while Dock reads its authored component id"
    );
    assert_eq!(
        unrouted_commandable_systems(&config.systems, &descriptors, consumers)
            .into_iter()
            .map(|system| system.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["tow-a"]
    );
}

/// The unrouted-lint decision keys on the registry: an admitted command for
/// a known-but-unregistered system id is flagged (would `pwarn!`), while a
/// registered one is not — and the lint mutates nothing.
///
/// (Note: `cargo test` installs no `tracing` subscriber, so the `pwarn!`
/// text itself cannot be captured — see `logging::macros` tests. The
/// decision that drives the warn is pinned here directly via the pure
/// `unrouted_command_targets`, and the full Bevy system is exercised for
/// no-panic / no-mutation below.)
#[test]
fn unrouted_lint_flags_unregistered_but_not_registered_ids() {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin);
    // Register Dock, whose consumer really reads the arbitrary authored id
    // stored in its runtime component. "ghost-system" is absent from both.
    app.register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::DOCK_KIND,
    ));

    let config = ship_config(vec![system(
        "berthing-clamps",
        crate::ship::system_registry::DOCK_KIND,
    )]);
    let before = admitted(&["berthing-clamps", "ghost-system"]);
    let ship = app
        .world_mut()
        .spawn((
            AdmittedCommands(before.0.clone()),
            crate::ship_plugin::ShipConfigComponent(config.clone()),
        ))
        .id();

    // Decision the lint acts on.
    let reg = app.world().resource::<AdmittedConsumerRegistry>();
    let stored = app.world().entity(ship).get::<AdmittedCommands>().unwrap();
    assert_eq!(
        unrouted_command_targets(stored, Some(&config), reg),
        vec!["ghost-system"]
    );

    // Run the real lint system: it must not panic and must not mutate the
    // admitted set (warning-only).
    app.world_mut()
        .run_system_cached(warn_unrouted_admitted_commands)
        .unwrap();
    let after = app.world().entity(ship).get::<AdmittedCommands>().unwrap();
    assert_eq!(after.0.len(), 2, "the lint must not drop admitted commands");
}

#[test]
fn feedback_registrations_are_idempotent_and_detect_ambiguous_ownership() {
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    let mut registry = AdmittedConsumerRegistry::default();
    let owner = ConsumerMatcher::kind("dock").with_feedback(
        FeedbackAddress::DeclaredKindOrCanonical("dock"),
        &[Payload::Dock],
    );
    registry.register(owner.clone());
    registry.register(owner);
    assert_eq!(registry.len(), 1);
    let config = ship_config(vec![system("berthing-clamps", "dock")]);
    assert_eq!(
        registry.feedback_support(
            &SystemId("berthing-clamps".into()),
            &SystemControlPayload::Dock,
            &config
        ),
        FeedbackSupport::Supported
    );
    registry.register(
        ConsumerMatcher::exact("dock", "berthing-clamps")
            .with_feedback(FeedbackAddress::MatcherSpelling, &[Payload::Dock]),
    );
    assert_eq!(
        registry.feedback_support(
            &SystemId("berthing-clamps".into()),
            &SystemControlPayload::Dock,
            &config
        ),
        FeedbackSupport::Ambiguous
    );
    assert_eq!(
        registry.feedback_support(
            &SystemId("ghost".into()),
            &SystemControlPayload::Dock,
            &config
        ),
        FeedbackSupport::Unsupported
    );
}

#[test]
#[should_panic(expected = "contradictory feedback metadata")]
fn conflicting_consumer_feedback_is_a_build_error() {
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    let mut registry = AdmittedConsumerRegistry::default();
    registry.register(ConsumerMatcher::exact("dock", "dock"));
    registry.register(
        ConsumerMatcher::exact("dock", "dock")
            .with_feedback(FeedbackAddress::MatcherSpelling, &[Payload::Dock]),
    );
}
