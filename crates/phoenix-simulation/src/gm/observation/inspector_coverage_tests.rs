use super::*;
use crate::gm::observation::entity_inspector::{
    EntityFieldGroup, EntityInspection, EntityInspectorField,
};
use crate::gm::observation::presentation_inspector::PresentationInspectorField;
use crate::gm::observation::region_inspector::{RegionFieldGroup, RegionInspectorField};
use crate::gm::observation::ship_inspector::{ShipFieldGroup, ShipInspectorField};
use crate::gm::observation::world_inspector::WorldInspectorField;
use crate::inspector::FieldOrigin;

fn descriptor(path: &str, mutability: LiveMutability) -> FieldDescriptor {
    FieldDescriptor {
        kind: "string".into(),
        default_source: None,
        live_mutability: mutability,
        origin: FieldOrigin {
            schema_path: path.into(),
            document: None,
            line: None,
            layer: None,
        },
        validation: Vec::new(),
    }
}

fn fixture_sources() -> (
    EntityInspectorProjection,
    WorldInspectorProjection,
    ShipInspectorProjection,
    RegionInspectorProjection,
    PresentationInspectorProjection,
) {
    let entity = EntityInspectorProjection {
        fields: vec![EntityInspectorField {
            id: "behaviour.doctrine".into(),
            label: "entity".into(),
            group: EntityFieldGroup::Behaviour,
            action_panel: Some("npc".into()),
            descriptor: descriptor("behaviour.doctrine", LiveMutability::NamedAction),
        }],
        readings: BTreeMap::from([(
            "entity".into(),
            EntityInspection {
                values: BTreeMap::from([("extension.private".into(), "ignored".into())]),
                ..Default::default()
            },
        )]),
    };
    let world = WorldInspectorProjection {
        fields: vec![WorldInspectorField {
            id: "event[first-contact].fire".into(),
            label: "world".into(),
            group: "events".into(),
            descriptor: descriptor("event[].fire", LiveMutability::NamedAction),
            action_panel: Some("mission".into()),
        }],
        readings: BTreeMap::new(),
    };
    let ship = ShipInspectorProjection {
        fields: vec![ShipInspectorField {
            id: "runtime.system[engine].health.current_hp".into(),
            label: "ship".into(),
            group: ShipFieldGroup::Runtime,
            action_panel: None,
            descriptor: descriptor(
                "runtime.system[engine].health.current_hp",
                LiveMutability::Derived,
            ),
        }],
        readings: BTreeMap::new(),
    };
    let region = RegionInspectorProjection {
        fields: vec![RegionInspectorField {
            id: "shape.sphere.radius".into(),
            label: "region".into(),
            group: RegionFieldGroup::Shape,
            descriptor: descriptor("shape.sphere.radius", LiveMutability::RecreateRequired),
        }],
        readings: BTreeMap::new(),
    };
    let presentation = PresentationInspectorProjection {
        fields: vec![PresentationInspectorField {
            id: "views.mode[].id".into(),
            label: "presentation".into(),
            group: "views".into(),
            descriptor: descriptor("views.mode[].id", LiveMutability::NamedAction),
            action_panel: Some("presentation".into()),
        }],
        readings: BTreeMap::new(),
    };
    (entity, world, ship, region, presentation)
}

fn build(
    sources: &(
        EntityInspectorProjection,
        WorldInspectorProjection,
        ShipInspectorProjection,
        RegionInspectorProjection,
        PresentationInspectorProjection,
    ),
) -> Result<Vec<LiveInspectorCoverageEntry>, LiveInspectorCoverageError> {
    inventory(LiveInspectorCoverageSources {
        entity: &sources.0,
        world: &sources.1,
        ship: &sources.2,
        region: &sources.3,
        presentation: &sources.4,
    })
}

fn active_domain_sources() -> (
    EntityInspectorProjection,
    WorldInspectorProjection,
    ShipInspectorProjection,
    RegionInspectorProjection,
    PresentationInspectorProjection,
) {
    use crate::core::messages::{PowerGroupId, StationId, SystemId};
    use crate::entities::config::EntityConfig;
    use crate::ship::components::ActiveStationRatings;
    use crate::ship::config::{PowerGroupConfig, ShipConfig, StationConfig, SystemInstanceConfig};
    use crate::ship::control_source::ControlSourceResolver;
    use crate::ship::damage::SystemHull;
    use std::collections::HashMap;

    let world_config = crate::world::config::parse_world(
        "[global]\ntitle='Coverage probe'\n[[deadline]]\nid='end'\ndue_secs=10\n",
    )
    .unwrap();
    let world = crate::gm::observation::world_inspector::projection(
        Some(&world_config),
        None,
        None,
        None,
        Some(false),
    );
    let station = StationConfig {
        id: StationId("helm".into()),
        name: "Helm".into(),
        description: "Fly".into(),
        rank: "Officer".into(),
        short_code: "H".into(),
        ratings: Vec::new(),
        manual_overview: None,
        tutorials: Vec::new(),
        stances: Vec::new(),
    };
    let console = phoenix_model::console::ConsoleConfig {
        id: phoenix_model::identity::ConsoleId("helm-screen".into()),
        home_station: Some(station.id.clone()),
        screen: Some("consoles/helm.html".into()),
        ..Default::default()
    };
    let system = SystemInstanceConfig {
        damage_components: Vec::new(),
        components: Vec::new(),
        channels: Vec::new(),
        console: Some(console.id.clone()),
        id: SystemId("engine".into()),
        kind: "helm-engine".into(),
        ai_only: false,
        power_group: Some(PowerGroupId("engines".into())),
        marker: None,
        config: None,
    };
    let ship_config = ShipConfig {
        stations: vec![station],
        systems: vec![system],
        power_groups: HashMap::from([(
            PowerGroupId("engines".into()),
            PowerGroupConfig {
                label: "Engines".into(),
                default_level: 2,
                min_level: 1,
                max_level: 4,
            },
        )]),
        consoles: vec![console],
        coordination_lag_secs: 2.0,
    };
    let authored = EntityConfig::default();
    let ratings = ActiveStationRatings::default();
    let controls = ControlSourceResolver::default();
    let hull = SystemHull::from_config(&[(SystemId("engine".into()), 100.0)]);
    let probe = crate::gm::observation::ship_inspector::projection([
        crate::gm::observation::ship_inspector::ShipInspectorInputs {
            id: "ship",
            label: "Coverage probe",
            config: &ship_config,
            authored: Some(&authored),
            authored_document: Some("assets/entities/coverage.toml"),
            ratings: &ratings,
            controls: &controls,
            hull: Some(&hull),
            physical: None,
            blackboards: None,
        },
    ]);
    let mut ship_fields: BTreeMap<String, ShipInspectorField> = probe
        .fields
        .into_iter()
        .map(|field| (canonical_runtime_path(&field.id), field))
        .collect();
    // Every shipped hull independently probes the optional
    // EntityConfig/ShipConfig sections and registered System kinds that can
    // be active. The union cannot silently collapse to one sparse hull.
    let entity_dir =
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("assets/entities");
    for entry in crate::repo_fixtures::fs::read_dir(entity_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let relative = path
            .strip_prefix(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let authored = crate::entities::include_resolve::resolve_from_disk(&relative)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .parse()
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let Some(config) = authored.ship_config.as_ref() else {
            continue;
        };
        let health = config
            .systems
            .iter()
            .map(|system| (system.id.clone(), 100.0))
            .collect::<Vec<_>>();
        let hull = SystemHull::from_config(&health);
        let physical = crate::ship::compartment_damage::ShipDamage::from_config(&authored).unwrap();
        let projection = crate::gm::observation::ship_inspector::projection([
            crate::gm::observation::ship_inspector::ShipInspectorInputs {
                id: "ship",
                label: "Coverage hull",
                config,
                authored: Some(&authored),
                authored_document: path.to_str(),
                ratings: &ratings,
                controls: &controls,
                hull: Some(&hull),
                physical: physical.as_ref(),
                blackboards: None,
            },
        ]);
        for field in projection.fields {
            ship_fields
                .entry(canonical_runtime_path(&field.id))
                .or_insert(field);
        }
    }
    let ship = ShipInspectorProjection {
        fields: ship_fields.into_values().collect(),
        readings: BTreeMap::new(),
    };
    (
        EntityInspectorProjection {
            fields: crate::gm::observation::entity_inspector::fields(),
            readings: BTreeMap::new(),
        },
        world,
        ship,
        RegionInspectorProjection {
            fields: crate::gm::observation::region_inspector::fields(),
            readings: BTreeMap::new(),
        },
        PresentationInspectorProjection {
            fields: crate::gm::observation::presentation_inspector::fields(),
            readings: BTreeMap::new(),
        },
    )
}

#[test]
fn inventory_is_sorted_canonical_and_ignores_unknown_runtime_extensions() {
    let sources = fixture_sources();
    let inventory = build(&sources).unwrap();
    assert_eq!(inventory.len(), 5);
    assert_eq!(inventory[0].domain, LiveInspectorDomain::Entity);
    assert_eq!(inventory[1].runtime, "event[].fire");
    assert_eq!(inventory[2].runtime, "runtime.system[].health.current_hp");
    assert!(!inventory
        .iter()
        .any(|row| row.runtime.contains("extension")));
}

#[test]
fn inventory_refuses_unnamed_or_unowned_action_routes() {
    let mut sources = fixture_sources();
    sources.0.fields[0].action_panel = None;
    assert!(matches!(
        build(&sources),
        Err(LiveInspectorCoverageError::MissingActionOwner { .. })
    ));
    sources.0.fields[0].action_panel = Some("generic-setter".into());
    assert!(matches!(
        build(&sources),
        Err(LiveInspectorCoverageError::InvalidActionOwner { .. })
    ));
}

#[test]
fn inventory_refuses_descriptor_runtime_drift_and_conflicts() {
    let mut sources = fixture_sources();
    sources.3.fields[0].descriptor.origin.schema_path = "shape.box.yaw".into();
    assert!(matches!(
        build(&sources),
        Err(LiveInspectorCoverageError::DescriptorRuntimeMismatch { .. })
    ));
    let mut sources = fixture_sources();
    let mut conflict = sources.2.fields[0].clone();
    conflict.id = "runtime.system[impulse].health.current_hp".into();
    conflict.descriptor.origin.schema_path = conflict.id.clone();
    conflict.descriptor.live_mutability = LiveMutability::RecreateRequired;
    sources.2.fields.push(conflict);
    assert!(matches!(
        build(&sources),
        Err(LiveInspectorCoverageError::ConflictingEntry { .. })
    ));
}

#[test]
fn complete_active_schema_inventory_is_ratcheted_across_all_five_domains() {
    // Exhaustive destructuring is the compile-time half of the ratchet.
    // A new top-level authored field cannot hide behind serde's absent
    // Option/default behaviour: it must be assigned to a Live domain (or
    // deliberately excluded) here before this test compiles again.
    let crate::entities::config::EntityConfig {
        name: _,
        display_name: _,
        tags: _,
        hull: _,
        collider: _,
        appearance: _,
        helm_console: _,
        helm_control: _,
        arrival_gravity: _,
        probe: _,
        marking_beacon: _,
        helm_capability: _,
        weapons_console: _,
        engineering_console: _,
        captain_console: _,
        comms_console: _,
        power: _,
        energy: _,
        interior: _,
        logistics: _,
        production: _,
        recovery: _,
        recoverable: _,
        port_service: _,
        damage: _,
        sensors_console: _,
        navigation_console: _,
        shields_console: _,
        torpedoes: _,
        repair: _,
        audio: _,
        comms: _,
        asteroid_field: _,
        shape: _,
        effects: _,
        infrastructure: _,
        scan: _,
        debris: _,
        tractor: _,
        held_response: _,
        dock: _,
        umbilical: _,
        security: _,
        security_target: _,
        transporter: _,
        civilian_rescue: _,
        demolition_target: _,
        reference_grid: _,
        civilian: _,
        faction: _,
        behaviour: _,
        ai_profile: _,
        lod_bubble: _,
        radar_appearance: _,
        target: _,
        mesh: _,
        star: _,
        planet: _,
        class: _,
        hull_id: _,
        power_rating: _,
        css: _,
        mass: _,
        light: _,
        ship_config: _,
        cinematic_camera: _,
        shield_arcs: _,
        resource_services: _,
        fitting: _,
        refit_facility: _,
        personnel: _,
        // Authored spatial/instrument recipes are recreate-required in Live;
        // their bounded supported editors belong to Workshop.
        field: _,
        signal_material: _,
        sensing: _,
        enquiry_observation: _,
    } = crate::entities::config::EntityConfig::default();
    // These patterns are intentionally separate from the maximal active
    // hull union above. The union proves that shipped combinations map to
    // descriptors; exhaustive nested patterns make an added optional or
    // skipped leaf fail compilation even before any hull authors it.
    macro_rules! exhaustive_struct {
            ($name:ident, $type:path, $($field:ident),+ $(,)?) => {
                fn $name(value: $type) {
                    let $type { $($field: _,)+ } = value;
                }
                let _ = $name as fn($type);
            };
        }
    exhaustive_struct!(
        logistics_schema,
        phoenix_model::logistics::LogisticsConfig,
        stores,
        storage,
        handling
    );
    exhaustive_struct!(
        managed_store_schema,
        phoenix_model::logistics::ManagedStoreConfig,
        id,
        endpoint,
        resource,
        compartment,
        role,
        system,
        source,
        target,
        reserve,
        priority
    );
    exhaustive_struct!(
        storage_bay_schema,
        phoenix_model::logistics::StorageBayConfig,
        compartment,
        space,
        unit_space
    );
    exhaustive_struct!(
        handling_schema,
        phoenix_model::logistics::HandlingConfig,
        system,
        max_active,
        max_batch,
        units_per_sec,
        reorder_fraction
    );
    exhaustive_struct!(
        production_schema,
        phoenix_model::production::ProductionConfig,
        system,
        recipes,
        initial_recipe,
        initial_target
    );
    exhaustive_struct!(
        recovery_schema,
        phoenix_model::recovery::RecoveryToolConfig,
        system,
        range,
        units_per_sec,
        resources
    );
    exhaustive_struct!(
        recoverable_schema,
        phoenix_model::recovery::RecoverableSourceConfig,
        availability,
        resources
    );
    exhaustive_struct!(
        port_service_schema,
        phoenix_model::ports::PortServiceConfig,
        system,
        policy,
        eligible_factions,
        offers
    );
    exhaustive_struct!(
        port_offer_schema,
        phoenix_model::ports::PortOfferConfig,
        id,
        resource,
        quota,
        unit_amount,
        costs
    );
    exhaustive_struct!(
        port_cost_schema,
        phoenix_model::ports::PortCost,
        resource,
        amount
    );
    exhaustive_struct!(
        production_recipe_schema,
        phoenix_model::production::ProductionRecipeConfig,
        id,
        inputs,
        outputs,
        work_secs
    );
    exhaustive_struct!(
        production_amount_schema,
        phoenix_model::production::ProductionAmount,
        store,
        amount
    );
    // Physical definitions are authored structure, never generic Live setters.
    // The descriptor walk classifies their leaves RecreateRequired. Explicit
    // nested patterns also catch optional future leaves absent from today's hulls.
    exhaustive_struct!(
        damage_schema,
        phoenix_model::damage::DamageConfig,
        emp,
        materials,
        remedies
    );
    exhaustive_struct!(
        material_schema,
        phoenix_model::damage::MaterialConfig,
        resistance,
        residual_fraction
    );
    exhaustive_struct!(
        remedy_schema,
        phoenix_model::damage::RemedyConfig,
        work_secs,
        manual_work_fraction,
        structural_hp,
        outcome,
        requirements
    );
    exhaustive_struct!(
        part_requirement_schema,
        phoenix_model::damage::PartRequirement,
        resource,
        service,
        amount
    );
    exhaustive_struct!(
        damage_component_schema,
        phoenix_model::damage::DamageComponentConfig,
        id,
        binding,
        susceptibility,
        fault_dose_threshold,
        degraded_factor,
        full_remedy,
        patch_remedy,
        field_replaceable
    );
    exhaustive_struct!(
        impact_schema,
        phoenix_model::damage::DamageProfile,
        emp,
        damage_type,
        penetration
    );
    exhaustive_struct!(
        interior_schema,
        phoenix_model::interior::InteriorConfig,
        decks,
        compartments,
        connections,
        placements,
        team_homes
    );
    exhaustive_struct!(
        deck_schema,
        phoenix_model::interior::DeckConfig,
        id,
        label,
        damage_weight
    );
    exhaustive_struct!(
        compartment_schema,
        phoenix_model::interior::CompartmentConfig,
        id,
        deck,
        label,
        x,
        z,
        width,
        depth,
        material,
        max_hp,
        recovery_fraction,
        structural_remedy
    );
    exhaustive_struct!(
        connection_schema,
        phoenix_model::interior::ConnectionConfig,
        id,
        from,
        to,
        travel_secs,
        boundary
    );
    exhaustive_struct!(
        physical_system_status_schema,
        phoenix_model::damage::PhysicalSystemStatus,
        compartment,
        structural_hp,
        structural_max_hp,
        structurally_disabled,
        components
    );
    exhaustive_struct!(
        physical_component_status_schema,
        phoenix_model::damage::PhysicalComponentStatus,
        id,
        binding,
        condition
    );
    // Private dose/source are intentionally absent from permitted status and
    // never become Live controls. Their persistence remains the damage owner.
    exhaustive_struct!(
        private_component_state_schema,
        phoenix_model::damage::ComponentState,
        emp,
        condition,
        dose,
        last_source
    );
    fn closed_damage_type(value: phoenix_model::damage::DamageType) {
        match value {
            phoenix_model::damage::DamageType::Physical
            | phoenix_model::damage::DamageType::Laser => {}
        }
    }
    fn closed_damage_binding(value: phoenix_model::damage::DamageBinding) {
        match value {
            phoenix_model::damage::DamageBinding::Operation
            | phoenix_model::damage::DamageBinding::Intake
            | phoenix_model::damage::DamageBinding::Cooling => {}
        }
    }
    fn closed_remedy_outcome(value: phoenix_model::damage::RemedyOutcome) {
        match value {
            phoenix_model::damage::RemedyOutcome::Structural
            | phoenix_model::damage::RemedyOutcome::Full => {}
            phoenix_model::damage::RemedyOutcome::Patch { factor: _ } => {}
        }
    }
    fn closed_component_condition(value: phoenix_model::damage::ComponentCondition) {
        match value {
            phoenix_model::damage::ComponentCondition::Healthy
            | phoenix_model::damage::ComponentCondition::Damaged
            | phoenix_model::damage::ComponentCondition::Destroyed => {}
            phoenix_model::damage::ComponentCondition::Patched { factor: _ } => {}
        }
    }
    let _ = closed_damage_type as fn(phoenix_model::damage::DamageType);
    let _ = closed_damage_binding as fn(phoenix_model::damage::DamageBinding);
    let _ = closed_remedy_outcome as fn(phoenix_model::damage::RemedyOutcome);
    let _ = closed_component_condition as fn(phoenix_model::damage::ComponentCondition);
    exhaustive_struct!(
        energy_schema,
        crate::entities::config::EnergyConfigSection,
        reactor,
        consumers,
        groups
    );
    exhaustive_struct!(
        reactor_energy_schema,
        crate::entities::config::EnergyReactorConfig,
        sustainable_output_per_sec,
        max_output_per_sec,
        ramp_per_sec,
        initial_output_per_sec,
        initial_energy,
        initial_heat,
        max_cooling_per_sec,
        default_cooling_per_sec,
        reserve,
        coolant
    );
    exhaustive_struct!(
        consumer_energy_schema,
        crate::entities::config::EnergyConsumerConfig,
        system,
        initial_energy,
        initial_heat,
        continuous_cost_per_sec,
        discrete_cost,
        reserve,
        coolant
    );
    exhaustive_struct!(
        energy_group_schema,
        crate::entities::config::EnergyGroupConfig,
        id,
        parent,
        max_flow_per_sec,
        default_flow_per_sec,
        max_cooling_per_sec,
        default_cooling_per_sec,
        priority,
        restore_delay_secs
    );
    exhaustive_struct!(
        energy_capacity_schema,
        crate::resources::energy::EnergyConfig,
        capacity,
        max_intake_per_sec,
        passive_loss_at_full_per_sec,
        charge_heat_per_unit,
        discharge_heat_per_unit,
        passive_cooling_per_sec,
        coolant_heat_per_unit,
        trip_heat,
        recover_heat
    );
    exhaustive_struct!(
        ship_config_schema,
        crate::ship::config::ShipConfig,
        consoles,
        stations,
        systems,
        power_groups,
        coordination_lag_secs,
    );
    exhaustive_struct!(
        station_config_schema,
        crate::ship::config::StationConfig,
        id,
        name,
        description,
        rank,
        short_code,
        ratings,
        manual_overview,
        tutorials,
        stances,
    );
    exhaustive_struct!(
        station_rating_schema,
        crate::ship::config::StationRatingConfig,
        name,
        automated_systems,
        detailed_systems,
        ai_tuning,
    );
    exhaustive_struct!(
        station_stance_schema,
        crate::ship::config::StationStanceConfig,
        id,
        label,
        kind,
        high_alert,
        persist_behind_human,
        ai_engaged,
    );
    exhaustive_struct!(
        system_instance_schema,
        crate::ship::config::SystemInstanceConfig,
        damage_components,
        components,
        channels,
        console,
        id,
        kind,
        ai_only,
        power_group,
        marker,
        config,
    );
    exhaustive_struct!(
        power_group_schema,
        crate::ship::config::PowerGroupConfig,
        label,
        default_level,
        min_level,
        max_level,
    );
    exhaustive_struct!(
        hull_schema,
        crate::entities::config::HullConfig,
        hull_integrity,
        system_hull,
    );
    exhaustive_struct!(
        system_hull_schema,
        crate::entities::config::SystemHullEntry,
        system_id,
        display_name,
        max_hp,
        damaged_threshold_pct,
        disabled_threshold_pct,
        debuff_magnitude,
    );
    exhaustive_struct!(
        helm_console_schema,
        crate::entities::config::HelmConsoleConfig,
        projection,
        max_speed,
        max_reverse_speed,
        acceleration,
        deceleration,
        max_yaw_rate,
        low_speed_turn_boost,
        radar,
        hostile_arc_color,
        power_multipliers,
        impulse_charge_duration,
        impulse_speed_multiplier,
        impulse_acceleration_multiplier,
        impulse_engage_distance,
        impulse_cancel_distance,
        max_bank_deg,
        bank_lerp_rate,
        boost,
        engine_pfx,
        lateral_thrust,
        engines_ai,
        steering_ai,
        lateral_ai,
        vertical_ai,
        impulse_ai,
        boost_ai,
    );
    exhaustive_struct!(
        helm_capability_schema,
        crate::entities::config::HelmCapabilityConfig,
        vertical_movement_mode,
        max_vertical_offset,
        vertical_return_rate,
        impulse,
    );
    exhaustive_struct!(
        impulse_capability_schema,
        crate::entities::config::ImpulseCapabilityConfig,
        steering_multiplier,
    );
    exhaustive_struct!(
        boost_schema,
        crate::entities::config::BoostConfig,
        multiplier,
        steering_multiplier,
        active_duration,
        recharge_duration,
    );
    exhaustive_struct!(
        engine_pfx_schema,
        crate::entities::config::EnginePfxConfig,
        color,
        markers,
        roll_degrees,
        scale,
        trail_lifetime_secs,
        trail_spawn_interval_secs,
    );
    exhaustive_struct!(
        lateral_thrust_schema,
        crate::entities::config::LateralThrustConfig,
        max_lateral_speed,
        lateral_acceleration,
    );
    exhaustive_struct!(
        weapons_console_schema,
        crate::entities::config::WeaponsConsoleConfig,
        groups,
        torpedo_arc_color,
        power_multipliers,
        phaser_banks,
        blaster_banks,
        radar,
        selector,
        selector_idle,
        ai,
    );
    exhaustive_struct!(
        phaser_bank_schema,
        crate::entities::config::PhaserBankConfig,
        targets,
        operation,
        impact,
        id,
        facing_deg,
        fire_arc_deg,
        auto_arc_deg,
        beam_range,
        beam_damage_per_sec,
        beam_duration_secs,
        cooldown_secs,
        cycle_jitter,
        beam_color,
        shield_pierce,
        marker,
        ai,
    );
    exhaustive_struct!(
        blaster_bank_schema,
        crate::entities::config::BlasterBankConfig,
        targets,
        rounds,
        impact,
        id,
        facing_deg,
        fire_arc_deg,
        volley_count,
        volley_interval_secs,
        cooldown_secs,
        charge_time_secs,
        projectile_speed,
        collision_radius,
        visual_scale,
        damage,
        shield_pierce,
        recoil_impulse,
        screenshake_magnitude,
        marker,
        barrels,
        pattern,
        range,
        ai,
    );
    exhaustive_struct!(
        torpedoes_schema,
        crate::entities::config::TorpedoesConfig,
        durability,
        seeker,
        receiver,
        count,
        initial_count,
        damage_hull,
        damage_shields,
        speed,
        turn_rate_deg_per_sec,
        lifespan,
        load_time,
        detonation_radius,
        shield_pierce,
        tubes,
        loader_groups,
        burst_interval_secs,
        ai_volley_target,
        ai,
    );
    exhaustive_struct!(
        torpedo_tube_schema,
        crate::entities::config::TorpedoTubeConfig,
        marking,
        targets,
        impact,
        id,
        facing_deg,
        fire_arc_deg,
        load_time,
        load_priority,
        marker,
        barrels,
        pattern,
        volley_max,
        ai_target_count,
        ai,
    );
    exhaustive_struct!(
        captain_console_schema,
        crate::entities::config::CaptainConsoleConfig,
        ai,
    );
    exhaustive_struct!(
        comms_console_schema,
        crate::entities::config::CommsConsoleConfig,
        selector,
        ai,
    );
    exhaustive_struct!(
        power_schema,
        crate::entities::config::PowerConfigSection,
        strike_reserve,
        capacity,
        rates,
        sustainable_total,
        max_commanded_total,
        emergency_threshold,
        ai_policy,
    );
    exhaustive_struct!(
        navigation_console_schema,
        crate::entities::config::NavigationConsoleConfig,
        system_chart,
        selector,
        sharing,
    );
    exhaustive_struct!(
        sensors_console_schema,
        crate::entities::config::SensorsConsoleConfig,
        long_range_radar,
        ai,
        selector,
        projection,
    );
    exhaustive_struct!(
        sensors_ai_schema,
        crate::entities::config::SensorsAiConfigToml,
        frequency_hint_delay_secs,
    );
    exhaustive_struct!(
        sensors_projection_schema,
        crate::entities::config::SensorsProjectionConfig,
        horizon_secs,
        marker_interval_secs,
    );
    exhaustive_struct!(
        shields_console_schema,
        crate::entities::config::ShieldsConsoleConfig,
        frequency_response,
        frequency_cycle,
        power_multipliers,
        focus_bonus_max_hp,
        focus_bonus_regen,
        focus_penalty_max_hp,
        focus_penalty_regen,
        focus_decay_rate,
        focus_focused_damage_multiplier,
        focus_unfocused_damage_multiplier,
        base,
        frequency,
        ai,
        ai_policy,
    );
    exhaustive_struct!(
        shield_frequency_response_schema,
        phoenix_model::shield_enquiry::ShieldFrequencyResponse,
        mismatch_loss
    );
    exhaustive_struct!(
        shield_frequency_cycle_schema,
        phoenix_model::shield_enquiry::ShieldFrequencyCycle,
        frequencies,
        period_secs
    );
    exhaustive_struct!(
        shields_base_schema,
        crate::entities::config::ShieldsBaseConfig,
        num_facings,
        max_hp,
        regen_per_sec,
        offline_duration,
    );
    exhaustive_struct!(
        shields_ai_schema,
        crate::entities::config::ShieldsAiConfigToml,
        damage_window_secs,
        min_damage_window_secs,
        damage_pct_threshold,
        health_ratio_threshold,
    );
    exhaustive_struct!(
        shield_arc_schema,
        crate::entities::config::ShieldArcConfig,
        damage_components,
        energy,
        ai,
        id,
        label,
        center_deg,
        width_deg,
        max_hp,
        regen_per_sec,
        offline_duration,
        priority,
        frequency,
    );
    exhaustive_struct!(
        repair_schema,
        crate::entities::config::RepairConfig,
        repair_team_count,
        travel_duration_secs,
        repair_rate_hp_per_sec,
        workflow,
        selector,
        external_dispatch,
    );
    exhaustive_struct!(
        external_repair_schema,
        crate::console::repair::external::ExternalRepairConfig,
        range,
        repair_rate,
    );
    exhaustive_struct!(
        radar_schema,
        crate::radar_config::RadarConfig,
        range,
        shows,
        selects,
    );
    exhaustive_struct!(
        fine_ai_selector_schema,
        crate::entities::config::FineSystemAiSelectorToml,
        param,
        sources,
        horizon,
        switch_margin,
        eligibility,
        score,
    );
    exhaustive_struct!(
        fine_ai_config_schema,
        crate::entities::config::FineSystemAiConfigToml,
        evaluate_every_ticks,
        idle,
        param,
        rule,
        initial_state,
        state,
        memory,
    );
    exhaustive_struct!(
        fine_ai_score_schema,
        crate::entities::config::ScoreTermToml,
        when,
        weight,
    );
    exhaustive_struct!(
        fine_ai_rule_schema,
        crate::entities::config::FineSystemAiRuleToml,
        priority,
        channel,
        when,
        verb,
        value,
        level,
        response_index,
    );
    exhaustive_struct!(
        fine_ai_state_schema,
        crate::entities::config::FineSystemAiStateToml,
        id,
        rule,
        transition,
        yields_to_arc_requests,
    );
    exhaustive_struct!(
        fine_ai_transition_schema,
        crate::entities::config::FineSystemAiTransitionToml,
        priority,
        to,
        when,
    );
    exhaustive_struct!(
        tutorial_schema,
        crate::core::messages::TutorialOverlayWire,
        id,
        trigger,
        title,
        text,
        anchor,
        priority,
    );
    exhaustive_struct!(
        tutorial_trigger_schema,
        crate::core::messages::TutorialTriggerWire,
        kind,
        control,
        path,
        op,
        value,
    );
    exhaustive_struct!(
        scan_schema,
        crate::science::ScanConfig,
        power_group,
        min_power_level,
        bands,
        degraded_by,
        interference_bands,
        mass_classes,
    );
    exhaustive_struct!(
        scan_band_schema,
        crate::science::ScanBandConfig,
        id,
        label,
        max_range,
        condition_step,
        report_thresholds,
        report_capacities,
    );
    exhaustive_struct!(
        scan_mass_class_schema,
        crate::science::scan::ScanMassClassConfig,
        id,
        label,
        max_mass,
    );
    exhaustive_struct!(
        tractor_schema,
        crate::tractor::TractorConfig,
        range,
        coupling_offset,
        min_power_level,
        tow_load,
    );
    exhaustive_struct!(
        tow_load_schema,
        crate::tractor::TowLoadCurve,
        half_penalty_mass,
        max_penalty,
    );
    exhaustive_struct!(
        dock_schema,
        crate::dock::DockConfig,
        moves_body,
        range,
        engage_distance,
        approach_speed,
        mate_tolerance,
        undock_clear_distance,
        min_power_level,
    );
    exhaustive_struct!(
        umbilical_schema,
        crate::umbilical::UmbilicalConfig,
        allowed_resources,
        link,
        capacity,
        rate,
        direction,
        min_power_level,
    );
    exhaustive_struct!(
        security_schema,
        crate::security::SecurityConfig,
        team_count,
        deploy_duration_secs,
        withdraw_duration_secs,
        range,
        local,
    );
    exhaustive_struct!(
        transporter_schema,
        crate::transporter::TransporterConfig,
        range,
        seconds_per_civilian,
        min_power_level,
    );
    fn engineering_console_schema(value: crate::entities::config::EngineeringConsoleConfig) {
        let crate::entities::config::EngineeringConsoleConfig {} = value;
    }
    let _ = engineering_console_schema as fn(crate::entities::config::EngineeringConsoleConfig);
    let crate::world::config::WorldConfig {
        transit_destinations: _,
        situation_definitions: _,
        situations: _,
        planning: _,
        compiled_planning: _,
        opportunity_search: _,
        compiled_opportunity_search: _,
        global: _,
        scenario_detail_floor: _,
        anchors: _,
        entities: _,
        name_to_uuid: _,
        extra_worlds: _,
        delayed_unload_policy: _,
        ambient_light: _,
        render: _,
        audio: _,
        dust: _,
        available_ships: _,
        ship_slots: _,
        player_spawn: _,
        deadlines: _,
        routes: _,
        workforces: _,
        gm_role_presets: _,
        gm_palette: _,
        gm_objective_palette: _,
        gm_comms_routes: _,
        gm_npc_doctrine_palette: _,
        gm_attention: _,
        script_sources: _,
    } = crate::world::config::WorldConfig::default();
    fn global_schema(value: crate::entities::config::GlobalConfig) {
        let crate::entities::config::GlobalConfig {
            seed: _,
            title: _,
            description: _,
            sim_tick_hz: _,
            autosave_interval_secs: _,
            ai_tick_hz: _,
            ai_snapshot_hz: _,
            intent_break_off_hull_fraction: _,
            attacked_memory_secs: _,
            station_activity_bucket_secs: _,
            trigger_fire_history_depth: _,
            gm_activity_history_depth: _,
            command_delay_ticks: _,
        } = value;
    }
    fn trigger_schema(value: crate::world::config::Trigger) {
        let crate::world::config::Trigger {
            condition: _,
            when: _,
            id: _,
            repeat: _,
            cooldown_secs: _,
            gm_controls: _,
        } = value;
    }
    fn gm_controls_schema(value: crate::world::config::GmEventControls) {
        let crate::world::config::GmEventControls {
            id: _,
            label: _,
            fire: _,
            pause: _,
            skip: _,
            attention_band: _,
        } = value;
    }
    fn scenario_payload_schema(value: crate::debug::payload::ScenarioStatePayload) {
        let crate::debug::payload::ScenarioStatePayload {
            schema_version: _,
            flags: _,
            objectives: _,
            triggers: _,
            delayed_actions: _,
            deadlines: _,
            commitments: _,
            dossier: _,
            situations: _,
            planning: _,
            opportunities: _,
        } = value;
    }
    fn scenario_flag_schema(value: crate::debug::payload::ScenarioFlag) {
        let crate::debug::payload::ScenarioFlag { name: _, value: _ } = value;
    }
    fn scenario_objective_schema(value: crate::debug::payload::ScenarioObjective) {
        let crate::debug::payload::ScenarioObjective {
            id: _,
            status: _,
            mandatory: _,
            base_priority: _,
            directive: _,
        } = value;
    }
    fn scenario_trigger_schema(value: crate::debug::payload::ScenarioTrigger) {
        let crate::debug::payload::ScenarioTrigger {
            id: _,
            condition: _,
            when: _,
            repeat: _,
            fired: _,
            pending: _,
            when_holds: _,
            last_fired_secs: _,
            fire_history: _,
        } = value;
    }
    fn trigger_fire_schema(value: crate::debug::payload::TriggerFire) {
        let crate::debug::payload::TriggerFire {
            fired_secs: _,
            predicate_values: _,
        } = value;
    }
    fn predicate_value_schema(value: crate::debug::payload::PredicateValue) {
        let crate::debug::payload::PredicateValue { atom: _, value: _ } = value;
    }
    fn delayed_action_schema(value: crate::debug::payload::ScenarioDelayedAction) {
        let crate::debug::payload::ScenarioDelayedAction {
            action: _,
            entity: _,
            fire_at_secs: _,
        } = value;
    }
    fn scenario_deadline_schema(value: crate::debug::payload::ScenarioDeadline) {
        let crate::debug::payload::ScenarioDeadline {
            id: _,
            label: _,
            visible: _,
            due_tick: _,
            state: _,
        } = value;
    }
    fn scenario_commitment_schema(value: crate::debug::payload::ScenarioCommitment) {
        let crate::debug::payload::ScenarioCommitment {
            id: _,
            made_to: _,
            terms: _,
            resolves_when: _,
            state: _,
            made_at_tick: _,
            resolved_at_tick: _,
        } = value;
    }
    fn scenario_dossier_schema(value: crate::debug::payload::ScenarioDossierEntry) {
        let crate::debug::payload::ScenarioDossierEntry {
            subject_uuid: _,
            text: _,
            provenance: _,
            gathered_at_tick: _,
        } = value;
    }
    fn mission_event_schema(value: crate::gm::operations::event::GmMissionEvent) {
        let crate::gm::operations::event::GmMissionEvent {
            id: _,
            label: _,
            fire: _,
            pause: _,
            paused: _,
            skip: _,
            repeatable: _,
            spent: _,
            armed: _,
            skip_armed: _,
        } = value;
    }
    fn objective_schema(value: crate::objectives::ObjectiveDebugView<'_>) {
        let crate::objectives::ObjectiveDebugView {
            id: _,
            status: _,
            mandatory: _,
            base_priority: _,
            directive: _,
        } = value;
    }
    fn deadline_schema(value: crate::world::deadlines::Deadline) {
        let crate::world::deadlines::Deadline {
            id: _,
            label: _,
            due_secs: _,
            visible: _,
        } = value;
    }
    fn deadline_runtime_schema(value: crate::world::deadlines::DeadlineRecord) {
        let crate::world::deadlines::DeadlineRecord {
            id: _,
            origin_layer: _,
            label: _,
            visible: _,
            due_tick: _,
            state: _,
            armed: _,
        } = value;
    }
    let _ = mission_event_schema as fn(crate::gm::operations::event::GmMissionEvent);
    let _ = global_schema as fn(crate::entities::config::GlobalConfig);
    let _ = trigger_schema as fn(crate::world::config::Trigger);
    let _ = gm_controls_schema as fn(crate::world::config::GmEventControls);
    let _ = scenario_payload_schema as fn(crate::debug::payload::ScenarioStatePayload);
    let _ = scenario_flag_schema as fn(crate::debug::payload::ScenarioFlag);
    let _ = scenario_objective_schema as fn(crate::debug::payload::ScenarioObjective);
    let _ = scenario_trigger_schema as fn(crate::debug::payload::ScenarioTrigger);
    let _ = trigger_fire_schema as fn(crate::debug::payload::TriggerFire);
    let _ = predicate_value_schema as fn(crate::debug::payload::PredicateValue);
    let _ = delayed_action_schema as fn(crate::debug::payload::ScenarioDelayedAction);
    let _ = scenario_deadline_schema as fn(crate::debug::payload::ScenarioDeadline);
    let _ = scenario_commitment_schema as fn(crate::debug::payload::ScenarioCommitment);
    let _ = scenario_dossier_schema as fn(crate::debug::payload::ScenarioDossierEntry);
    let _ = objective_schema as fn(crate::objectives::ObjectiveDebugView<'_>);
    let _ = deadline_schema as fn(crate::world::deadlines::Deadline);
    let _ = deadline_runtime_schema as fn(crate::world::deadlines::DeadlineRecord);
    let sources = active_domain_sources();
    let inventory = build(&sources).unwrap();
    let counts = [
        LiveInspectorDomain::Entity,
        LiveInspectorDomain::World,
        LiveInspectorDomain::Ship,
        LiveInspectorDomain::Region,
        LiveInspectorDomain::Presentation,
    ]
    .map(|domain| inventory.iter().filter(|row| row.domain == domain).count());
    // This is intentionally exact. Domain-specific tests compare their
    // descriptors with the authored schema; this cross-domain ratchet then
    // requires every accepted schema change to update the reviewed total
    // and fingerprint. The fingerprint includes every recorded column, so
    // a classification or action-owner change cannot hide behind the same
    // number of rows.
    // Explicit Console identity, hosting, feeds and ownership replace the
    // retired Station screen/hosting and direct System ownership paths.
    // Dock composition replaces six legacy tuning leaves with thirteen typed
    // component/channel leaves; the other four domains remain unchanged.
    // Dock state-tree definitions add thirty authored ship schema leaves.
    // Repair workflows add sixteen authored Job-context leaves.
    // Resource services add one authored endpoint-kind leaf, recreate-required.
    // Physical damage adds explicit materials/remedies/component/impact/layout
    // leaves and checked physical System actions. Sensing, focused enquiry,
    // explicit radar sharing, Navigation sharing and Helm projection extend
    // the shipped-hull union to 1011 recreate-required/derived ship rows.
    let mut fingerprint = 0xcbf2_9ce4_8422_2325_u64;
    for row in &inventory {
        let line = format!(
            "{:?}\t{}\t{}\t{:?}\t{}\n",
            row.domain,
            row.descriptor,
            row.runtime,
            row.mutability,
            row.action_panel.as_deref().unwrap_or("")
        );
        for byte in line.bytes() {
            fingerprint ^= u64::from(byte);
            fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    println!("reviewed schema counts={counts:?}; fingerprint={fingerprint}");
    assert_eq!(
        counts,
        [55, 71, 1011, 51, 56],
        "update the reviewed inventory ratchet"
    );
    assert_eq!(
        fingerprint, 1_058_332_460_400_903_743,
        "update the reviewed inventory fingerprint"
    );
}

#[test]
fn region_and_presentation_variant_schemas_are_exhaustively_mapped() {
    use crate::gm::operations::presentation::PresentationCue;
    use crate::regions::{effects::RegionEffectKind, shape::RegionShape};

    fn shape_paths(shape: RegionShape) -> &'static [&'static str] {
        match shape {
            RegionShape::Sphere { radius: _ } => &["shape.sphere.radius"],
            RegionShape::Box {
                half_extents: _,
                yaw: _,
            } => &[
                "shape.box.half_extents.x",
                "shape.box.half_extents.y",
                "shape.box.half_extents.z",
                "shape.box.yaw",
            ],
            RegionShape::Torus {
                inner_radius: _,
                outer_radius: _,
            } => &["shape.torus.inner_radius", "shape.torus.outer_radius"],
        }
    }
    fn effect_paths(effect: RegionEffectKind) -> &'static [&'static str] {
        match effect {
            RegionEffectKind::DamageZone {
                dps: _,
                shield_pierce: _,
            } => &[
                "effects.damage_zone.damage_per_second",
                "effects.damage_zone.shield_pierce",
            ],
            RegionEffectKind::SlowZone {
                thrust_modifier: _,
                yaw_rate_modifier: _,
            } => &[
                "effects.slow_zone.thrust_modifier",
                "effects.slow_zone.yaw_rate_modifier",
            ],
            RegionEffectKind::BlocksImpulse => &["effects.blocks_impulse.present"],
            RegionEffectKind::RadarDampening { multiplier: _ } => {
                &["effects.radar_dampening.range_modifier"]
            }
            RegionEffectKind::CommsJam => &["effects.comms_jammed.present"],
            RegionEffectKind::SensorBlind => &["effects.sensor_blind.present"],
            RegionEffectKind::NebulaFog {
                color: _,
                density: _,
            } => &[
                "effects.nebula_fog.color.r",
                "effects.nebula_fog.color.g",
                "effects.nebula_fog.color.b",
                "effects.nebula_fog.density",
            ],
        }
    }
    fn cue_paths(cue: PresentationCue) -> &'static [&'static str] {
        match cue {
            PresentationCue::ForceView {
                view: _,
                duration_ticks: _,
            } => &["cue.force_view.view", "cue.force_view.duration_ticks"],
            PresentationCue::ReleaseView => &["cue.release_view"],
            PresentationCue::TitleCard {
                title: _,
                subtitle: _,
                duration_ticks: _,
            } => &[
                "cue.title_card.title",
                "cue.title_card.subtitle",
                "cue.title_card.duration_ticks",
            ],
            PresentationCue::IncomingComms {
                message: _,
                duration_ticks: _,
            } => &[
                "cue.incoming_comms.message",
                "cue.incoming_comms.duration_ticks",
            ],
            PresentationCue::ClearCard => &["cue.clear_card"],
            PresentationCue::Sound { id: _, source: _ } => &["cue.sound.id", "cue.sound.source"],
        }
    }

    let region = crate::gm::observation::region_inspector::fields()
        .into_iter()
        .map(|field| field.id)
        .collect::<BTreeSet<_>>();
    for shape in [
        RegionShape::Sphere { radius: 1.0 },
        RegionShape::Box {
            half_extents: [1.0; 3],
            yaw: 0.0,
        },
        RegionShape::Torus {
            inner_radius: 1.0,
            outer_radius: 2.0,
        },
    ] {
        for path in shape_paths(shape) {
            assert!(region.contains(*path), "missing Region descriptor {path}");
        }
    }
    for effect in [
        RegionEffectKind::DamageZone {
            dps: 1.0,
            shield_pierce: 0.0,
        },
        RegionEffectKind::SlowZone {
            thrust_modifier: None,
            yaw_rate_modifier: None,
        },
        RegionEffectKind::BlocksImpulse,
        RegionEffectKind::RadarDampening { multiplier: -1.0 },
        RegionEffectKind::CommsJam,
        RegionEffectKind::SensorBlind,
        RegionEffectKind::NebulaFog {
            color: [0.0; 3],
            density: 0.0,
        },
    ] {
        for path in effect_paths(effect) {
            assert!(region.contains(*path), "missing Region descriptor {path}");
        }
    }
    let presentation = crate::gm::observation::presentation_inspector::fields()
        .into_iter()
        .map(|field| field.id)
        .collect::<BTreeSet<_>>();
    for cue in [
        PresentationCue::ForceView {
            view: crate::gm::operations::presentation::PresentationView::Radar,
            duration_ticks: 1,
        },
        PresentationCue::ReleaseView,
        PresentationCue::TitleCard {
            title: String::new(),
            subtitle: String::new(),
            duration_ticks: 1,
        },
        PresentationCue::IncomingComms {
            message: String::new(),
            duration_ticks: 1,
        },
        PresentationCue::ClearCard,
        PresentationCue::Sound {
            id: String::new(),
            source: None,
        },
    ] {
        for path in cue_paths(cue) {
            assert!(
                presentation.contains(*path),
                "missing Presentation descriptor {path}"
            );
        }
    }

    let crate::sound_cues::Catalog {
        version: _,
        assets,
        cues,
    } = crate::sound_cues::bundled();
    for crate::sound_cues::Asset {
        file: _,
        category: _,
        informative: _,
    } in assets
    {
        for path in ["asset.file", "asset.category", "asset.informative"] {
            assert!(
                presentation.contains(path),
                "missing Asset descriptor {path}"
            );
        }
    }
    for crate::sound_cues::SoundDefinition {
        id: _,
        label: _,
        file: _,
        category: _,
        audience: _,
        volume: _,
        equivalent,
    } in cues
    {
        for path in [
            "sound.id",
            "sound.label",
            "sound.file",
            "sound.category",
            "sound.audience",
            "sound.volume",
            "sound.equivalent.present",
        ] {
            assert!(
                presentation.contains(path),
                "missing Sound descriptor {path}"
            );
        }
        if let Some(crate::sound_cues::Equivalent {
            meaning: _,
            source: _,
            urgency: _,
            bearing: _,
            elevation: _,
        }) = equivalent
        {
            for path in [
                "sound.equivalent.meaning",
                "sound.equivalent.source",
                "sound.equivalent.urgency",
                "sound.equivalent.bearing",
                "sound.equivalent.elevation",
            ] {
                assert!(
                    presentation.contains(path),
                    "missing Equivalent descriptor {path}"
                );
            }
        }
    }
}

use crate::entities::include_resolve::ParseEntityTemplate as _;

#[test]
fn physical_authored_domains_and_every_weapon_profile_require_recreation() {
    let sources = active_domain_sources();
    let inventory = build(&sources).unwrap();
    let physical: Vec<_> = inventory
        .iter()
        .filter(|row| {
            row.domain == LiveInspectorDomain::Ship
                && !row.runtime.starts_with("runtime.")
                && (row.runtime.starts_with("damage.")
                    || row.runtime.starts_with("interior.")
                    || row.runtime.contains(".damage_component[")
                    || row.runtime.contains(".impact."))
        })
        .collect();
    assert!(!physical.is_empty());
    for row in &physical {
        assert_eq!(
            row.mutability,
            LiveMutability::RecreateRequired,
            "{}",
            row.runtime
        );
        assert!(row.action_panel.is_none(), "{}", row.runtime);
    }
    for required in [
        "damage.materials.",
        "damage.remedies.",
        "interior.compartment[].max_hp",
        "interior.compartment[].recovery_fraction",
        "interior.compartment[].structural_remedy",
        "system[].damage_component[].fault_dose_threshold",
        "shield_arc[].damage_component[].binding",
        "weapons_console.phaser_banks[].impact.penetration",
        "weapons_console.blaster_banks[].impact.damage_type",
        "torpedoes.tubes[].impact.penetration",
    ] {
        assert!(
            physical.iter().any(|row| row.runtime.starts_with(required)),
            "unmapped physical schema {required}"
        );
    }
}
