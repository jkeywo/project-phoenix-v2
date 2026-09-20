//! Deterministic coverage inventory for the five Live Inspector domains.
//!
//! Domain adapters remain the owners of schema discovery and readings. This
//! module checks their published descriptor tables as one contract: a reading
//! key must have the same canonical grammar as its descriptor, every field has
//! a Live mutability classification, and a named action must name an existing
//! panel owner. Reading maps are deliberately not traversed, so an unknown
//! runtime extension remains visible only to the adapter that understands it
//! and cannot silently become a generic Live control.

use crate::gm_entity_inspector::EntityInspectorProjection;
use crate::gm_presentation_inspector::PresentationInspectorProjection;
use crate::gm_region_inspector::RegionInspectorProjection;
use crate::gm_ship_inspector::ShipInspectorProjection;
use crate::gm_world_inspector::WorldInspectorProjection;
use crate::inspector::{FieldDescriptor, LiveMutability};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiveInspectorDomain {
    Entity,
    World,
    Ship,
    Region,
    Presentation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveInspectorCoverageEntry {
    pub domain: LiveInspectorDomain,
    /// Canonical authored/derived schema path from the shared descriptor.
    pub descriptor: String,
    /// Canonical key grammar used by the domain's runtime reading map.
    pub runtime: String,
    pub mutability: LiveMutability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_panel: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveInspectorCoverageError {
    MissingDomain(LiveInspectorDomain),
    EmptyPath(LiveInspectorDomain),
    DescriptorRuntimeMismatch {
        domain: LiveInspectorDomain,
        descriptor: String,
        runtime: String,
    },
    ConflictingEntry {
        domain: LiveInspectorDomain,
        runtime: String,
    },
    MissingActionOwner {
        domain: LiveInspectorDomain,
        runtime: String,
    },
    UnexpectedActionOwner {
        domain: LiveInspectorDomain,
        runtime: String,
        action_panel: String,
    },
    InvalidActionOwner {
        domain: LiveInspectorDomain,
        runtime: String,
        action_panel: String,
    },
}

pub struct LiveInspectorCoverageSources<'a> {
    pub entity: &'a EntityInspectorProjection,
    pub world: &'a WorldInspectorProjection,
    pub ship: &'a ShipInspectorProjection,
    pub region: &'a RegionInspectorProjection,
    pub presentation: &'a PresentationInspectorProjection,
}

/// Replace instance keys (`station[helm]`, `event[first-contact]`) with their
/// schema grammar (`station[]`, `event[]`). Brackets are data, not structure;
/// malformed unmatched brackets are left intact and will fail the descriptor
/// comparison rather than being guessed into a valid path.
pub fn canonical_runtime_path(path: &str) -> String {
    let mut canonical = String::with_capacity(path.len());
    let mut chars = path.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch != '[' {
            canonical.push(ch);
            continue;
        }
        if let Some(end) = path[start + 1..].find(']') {
            canonical.push_str("[]");
            let end = start + 1 + end;
            while chars.peek().is_some_and(|(index, _)| *index <= end) {
                chars.next();
            }
        } else {
            canonical.push_str(&path[start..]);
            break;
        }
    }
    canonical
}

fn allowed_action_owners(domain: LiveInspectorDomain) -> &'static [&'static str] {
    match domain {
        LiveInspectorDomain::Entity => &["npc"],
        LiveInspectorDomain::World => &["mission", "objective", "session"],
        LiveInspectorDomain::Ship => &["effect", "station", "system"],
        LiveInspectorDomain::Region => &[],
        LiveInspectorDomain::Presentation => &["presentation"],
    }
}

fn add_entry(
    entries: &mut BTreeMap<(LiveInspectorDomain, String), LiveInspectorCoverageEntry>,
    domain: LiveInspectorDomain,
    runtime: &str,
    descriptor: &FieldDescriptor,
    action_panel: Option<&str>,
) -> Result<(), LiveInspectorCoverageError> {
    if runtime.is_empty() || descriptor.origin.schema_path.is_empty() {
        return Err(LiveInspectorCoverageError::EmptyPath(domain));
    }
    let runtime = canonical_runtime_path(runtime);
    let descriptor_path = canonical_runtime_path(&descriptor.origin.schema_path);
    if runtime != descriptor_path {
        return Err(LiveInspectorCoverageError::DescriptorRuntimeMismatch {
            domain,
            descriptor: descriptor_path,
            runtime,
        });
    }
    match (descriptor.live_mutability, action_panel) {
        (LiveMutability::NamedAction, None) => {
            return Err(LiveInspectorCoverageError::MissingActionOwner { domain, runtime });
        }
        (LiveMutability::NamedAction, Some(owner))
            if !allowed_action_owners(domain).contains(&owner) =>
        {
            return Err(LiveInspectorCoverageError::InvalidActionOwner {
                domain,
                runtime,
                action_panel: owner.into(),
            });
        }
        (LiveMutability::NamedAction, Some(_)) => {}
        (_, Some(owner)) => {
            return Err(LiveInspectorCoverageError::UnexpectedActionOwner {
                domain,
                runtime,
                action_panel: owner.into(),
            });
        }
        (_, None) => {}
    }
    let entry = LiveInspectorCoverageEntry {
        domain,
        descriptor: descriptor_path,
        runtime: runtime.clone(),
        mutability: descriptor.live_mutability,
        action_panel: action_panel.map(str::to_owned),
    };
    match entries.entry((domain, runtime.clone())) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(entry);
        }
        std::collections::btree_map::Entry::Occupied(slot) if slot.get() == &entry => {}
        std::collections::btree_map::Entry::Occupied(_) => {
            return Err(LiveInspectorCoverageError::ConflictingEntry { domain, runtime });
        }
    }
    Ok(())
}

/// Build the sorted, de-duplicated active-schema inventory.
///
/// A domain with no descriptors is an error. This prevents a projection
/// wiring regression from producing a plausible four-domain manifest.
pub fn inventory(
    sources: LiveInspectorCoverageSources<'_>,
) -> Result<Vec<LiveInspectorCoverageEntry>, LiveInspectorCoverageError> {
    let mut entries = BTreeMap::new();
    let mut domains = BTreeSet::new();
    for field in &sources.entity.fields {
        domains.insert(LiveInspectorDomain::Entity);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Entity,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for field in &sources.world.fields {
        domains.insert(LiveInspectorDomain::World);
        add_entry(
            &mut entries,
            LiveInspectorDomain::World,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for field in &sources.ship.fields {
        domains.insert(LiveInspectorDomain::Ship);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Ship,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for field in &sources.region.fields {
        domains.insert(LiveInspectorDomain::Region);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Region,
            &field.id,
            &field.descriptor,
            None,
        )?;
    }
    for field in &sources.presentation.fields {
        domains.insert(LiveInspectorDomain::Presentation);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Presentation,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for domain in [
        LiveInspectorDomain::Entity,
        LiveInspectorDomain::World,
        LiveInspectorDomain::Ship,
        LiveInspectorDomain::Region,
        LiveInspectorDomain::Presentation,
    ] {
        if !domains.contains(&domain) {
            return Err(LiveInspectorCoverageError::MissingDomain(domain));
        }
    }
    Ok(entries.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gm_entity_inspector::{EntityFieldGroup, EntityInspection, EntityInspectorField};
    use crate::gm_presentation_inspector::PresentationInspectorField;
    use crate::gm_region_inspector::{RegionFieldGroup, RegionInspectorField};
    use crate::gm_ship_inspector::{ShipFieldGroup, ShipInspectorField};
    use crate::gm_world_inspector::WorldInspectorField;
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
        use crate::ship::config::{
            PowerGroupConfig, ShipConfig, StationConfig, SystemInstanceConfig,
        };
        use crate::ship::control_source::ControlSourceResolver;
        use crate::ship::damage::SystemHull;
        use std::collections::HashMap;

        let world_config = crate::world::config::parse_world(
            "[global]\ntitle='Coverage probe'\n[[deadline]]\nid='end'\ndue_secs=10\n",
        )
        .unwrap();
        let world = crate::gm_world_inspector::projection(
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
            console: None,
            manual_overview: None,
            tutorials: Vec::new(),
            human_seeking: false,
            host_order: Vec::new(),
            visiting_rating: None,
            auxiliary: false,
            command_target: None,
            stances: Vec::new(),
        };
        let system = SystemInstanceConfig {
            id: SystemId("engine".into()),
            kind: "helm-engine".into(),
            station: Some(station.id.clone()),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
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
            coordination_lag_secs: 2.0,
        };
        let authored = EntityConfig::default();
        let ratings = ActiveStationRatings::default();
        let controls = ControlSourceResolver::default();
        let hull = SystemHull::from_config(&[(SystemId("engine".into()), 100.0)]);
        let probe =
            crate::gm_ship_inspector::projection([crate::gm_ship_inspector::ShipInspectorInputs {
                id: "ship",
                label: "Coverage probe",
                config: &ship_config,
                authored: Some(&authored),
                authored_document: Some("assets/entities/coverage.toml"),
                ratings: &ratings,
                controls: &controls,
                hull: &hull,
                blackboards: None,
            }]);
        let mut ship_fields: BTreeMap<String, ShipInspectorField> = probe
            .fields
            .into_iter()
            .map(|field| (canonical_runtime_path(&field.id), field))
            .collect();
        // Every shipped hull independently probes the optional
        // EntityConfig/ShipConfig sections and registered System kinds that can
        // be active. The union cannot silently collapse to one sparse hull.
        let entity_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
        for entry in std::fs::read_dir(entity_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }
            let relative = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
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
            let projection = crate::gm_ship_inspector::projection([
                crate::gm_ship_inspector::ShipInspectorInputs {
                    id: "ship",
                    label: "Coverage hull",
                    config,
                    authored: Some(&authored),
                    authored_document: path.to_str(),
                    ratings: &ratings,
                    controls: &controls,
                    hull: &hull,
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
                fields: crate::gm_entity_inspector::fields(),
                readings: BTreeMap::new(),
            },
            world,
            ship,
            RegionInspectorProjection {
                fields: crate::gm_region_inspector::fields(),
                readings: BTreeMap::new(),
            },
            PresentationInspectorProjection {
                fields: crate::gm_presentation_inspector::fields(),
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
            helm_capability: _,
            weapons_console: _,
            engineering_console: _,
            captain_console: _,
            comms_console: _,
            power: _,
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
            ship_config_schema,
            crate::ship::config::ShipConfig,
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
            console,
            manual_overview,
            tutorials,
            human_seeking,
            host_order,
            visiting_rating,
            auxiliary,
            command_target,
            stances,
        );
        exhaustive_struct!(
            station_rating_schema,
            crate::ship::config::StationRatingConfig,
            name,
            automated_systems,
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
            id,
            kind,
            station,
            ai_only,
            human_seeking,
            seek_order,
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
            count,
            damage_hull,
            damage_shields,
            speed,
            turn_rate_deg_per_sec,
            lifespan,
            load_time,
            detonation_radius,
            shield_pierce,
            tubes,
            burst_interval_secs,
            ai_volley_target,
            ai,
        );
        exhaustive_struct!(
            torpedo_tube_schema,
            crate::entities::config::TorpedoTubeConfig,
            id,
            facing_deg,
            fire_arc_deg,
            load_time,
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
            id,
            label,
            center_deg,
            width_deg,
            max_hp,
            regen_per_sec,
            offline_duration,
            hull_max_hp,
            hull_damaged_threshold_pct,
            hull_disabled_threshold_pct,
            hull_debuff_magnitude,
            priority,
            frequency,
        );
        exhaustive_struct!(
            repair_schema,
            crate::entities::config::RepairConfig,
            repair_team_count,
            travel_duration_secs,
            repair_rate_hp_per_sec,
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
        fn mission_event_schema(value: crate::gm_event::GmMissionEvent) {
            let crate::gm_event::GmMissionEvent {
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
        let _ = mission_event_schema as fn(crate::gm_event::GmMissionEvent);
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
        assert_eq!(
            counts,
            [55, 71, 639, 51, 56],
            "update the reviewed inventory ratchet"
        );
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
        assert_eq!(
            fingerprint, 17_948_140_228_494_028_562,
            "update the reviewed inventory fingerprint"
        );
    }

    #[test]
    fn region_and_presentation_variant_schemas_are_exhaustively_mapped() {
        use crate::gm_presentation::PresentationCue;
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
                PresentationCue::Sound { id: _, source: _ } => {
                    &["cue.sound.id", "cue.sound.source"]
                }
            }
        }

        let region = crate::gm_region_inspector::fields()
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
        let presentation = crate::gm_presentation_inspector::fields()
            .into_iter()
            .map(|field| field.id)
            .collect::<BTreeSet<_>>();
        for cue in [
            PresentationCue::ForceView {
                view: crate::gm_presentation::PresentationView::Radar,
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
}
