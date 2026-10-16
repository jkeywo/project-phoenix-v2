//! Offline Workshop services over explicit source bundles. Authoring validation
//! never touches a running session; Test owns a fresh disposable runtime with
//! captured source and a private, finite clock-control boundary.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::entities::config::EntityConfig;
use crate::entities::config_cache::ActivePack;
use crate::entities::include_resolve::{canonical_template_path, resolve_template};
use crate::entities::loader::TemplateLoader;
use crate::world::load::{load, LoadPolicy, LoadRequest, WorldReader};
use crate::world::manifest::{parse_content_identity, parse_manifest};
#[cfg(test)]
use crate::world::mod_pack::read_store_zip;
#[cfg(test)]
use crate::world::mod_pack::validate_mod_pack;
use crate::world::mod_pack::validate_mod_pack_with_assets;
use crate::world::script::load::ScriptResolver;
use crate::world::validate::{
    validate_composition_with_fragments, Severity, WorldFinding, WorldSource,
};

pub mod archive;
pub mod composition;
pub mod damage;
pub mod definitions;
pub mod document;
pub mod entity;
mod model_fields;
pub mod presets;
mod source_spans;
mod validation_support;

/// Immutable dependencies, oldest pack first. The editable candidate is passed
/// separately and always wins. No runtime cache or filesystem is consulted.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkshopDependencies {
    pub base_files: BTreeMap<String, String>,
    #[serde(default)]
    pub base_assets: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    pub packs: Vec<WorkshopDependencyPack>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkshopDependencyPack {
    pub id: String,
    pub manifest_toml: String,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkshopFinding {
    pub severity: String,
    pub category: String,
    pub message: String,
    pub file: String,
    pub line: Option<usize>,
}

impl From<WorldFinding> for WorkshopFinding {
    fn from(value: WorldFinding) -> Self {
        Self {
            severity: match value.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }
            .into(),
            category: value.category.into(),
            message: value.message,
            file: value.source.file,
            line: value.source.line,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct WorkshopValidation {
    pub accepted: bool,
    pub findings: Vec<WorkshopFinding>,
    /// The scenario catalogue the validated candidate would publish — what
    /// Test and the lobby list — read by ONE function in both `validate_pack`
    /// and `validate_project`, so a native save and a browser ZIP export of
    /// the same members cannot catalogue differently (issue #1475). A Test
    /// selection report, which validates one root, carries none.
    pub catalogue: Vec<composition::CatalogueEntry>,
}

/// Runtime-owned vocabulary used by Workshop's ship authoring controls.
/// Keeping this beside the validators means the browser and native surfaces
/// cannot drift into offering a System or Directive spelling the build rejects.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkshopShipSchema {
    pub damage: damage::WorkshopDamageSchema,
    pub console_stock_publication_kind: phoenix_model::console::ConsoleObservationKind,
    pub console_feed_kinds: Vec<String>,
    pub console_feed_sources: Vec<String>,
    pub console_publication_kinds:
        std::collections::BTreeMap<String, phoenix_model::console::ConsoleObservationKind>,
    pub console_instance_publication_kinds:
        std::collections::BTreeMap<String, phoenix_model::console::ConsoleObservationKind>,
    pub system_kinds: Vec<String>,
    pub component_fields: std::collections::BTreeMap<String, Vec<WorkshopComponentField>>,
    pub channel_kinds: Vec<String>,
    pub ai_state_tree: WorkshopAiTreeSchema,
    pub job_workflow: WorkshopJobWorkflowSchema,
    pub directive_kinds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkshopAiTreeSchema {
    pub system_kinds: Vec<String>,
    pub hosts: BTreeMap<String, crate::ai::state_tree::SchemaMetadata>,
    pub shield_recovery_bindings: WorkshopShieldRecoveryBindings,
    #[serde(flatten)]
    pub vocabulary: crate::ai::state_tree::SchemaMetadata,
}

/// Closed endpoint vocabulary. Authoring expands these prefixes only for the
/// selected arc's physical eligible targets; runtime binding admission remains Rust-owned.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkshopShieldRecoveryBindings {
    pub facts: Vec<String>,
    pub channels: Vec<String>,
    pub numeric_facts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkshopJobWorkflowSchema {
    #[serde(flatten)]
    pub vocabulary: crate::ai::state_tree::SchemaMetadata,
    pub default_tree: crate::ai::state_tree::AiTreeConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkshopComponentField {
    pub name: String,
    pub kind: String,
}

pub fn ship_schema() -> WorkshopShipSchema {
    let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("the built-in System registry is valid");
    let mut system_kinds = registry.kinds().map(str::to_owned).collect::<Vec<_>>();
    system_kinds.sort();
    let console_publication_kinds = crate::ship::system_registry::fixed_publication_kinds();
    let mut console_feed_sources: Vec<_> = console_publication_kinds.keys().cloned().collect();
    console_feed_sources.sort();
    let mut power_tree_vocabulary = crate::ai::state_tree::schema_metadata(
        &phoenix_sim_gameplay::resources::energy_ai::power_bindings(),
    );
    power_tree_vocabulary.sources.push("number_compare");
    power_tree_vocabulary.actions.push("write_number");
    let (shield_base, shield_numbers) = crate::ship::shield_ai::bindings(&[]);
    let (shield_target, shield_target_numbers) =
        crate::ship::shield_ai::bindings(&[crate::core::messages::SystemId(
            "workshop-target".into(),
        )]);
    let prefixes = |names: &std::collections::BTreeSet<String>| {
        names
            .iter()
            .filter_map(|name| name.strip_suffix("-workshop-target").map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let shield_recovery_bindings = WorkshopShieldRecoveryBindings {
        facts: prefixes(&shield_target.facts),
        channels: prefixes(&shield_target.channels),
        numeric_facts: prefixes(&shield_target_numbers.facts),
    };
    WorkshopShipSchema {
        damage: damage::schema(),
        console_feed_sources,
        console_publication_kinds,
        console_instance_publication_kinds:
            crate::ship::system_registry::instance_publication_kinds(),
        console_stock_publication_kind: phoenix_model::console::ConsoleObservationKind::Stock,
        console_feed_kinds: phoenix_model::console::ConsoleObservationKind::NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        component_fields: [
            ("power_input".into(), Vec::new()),
            (
                "craft_bay".into(),
                [
                    ("power_input", "power_input_component"),
                    ("config.compartment", "text"),
                    ("config.capacity_space", "integer"),
                    ("config.launch_work_secs", "number"),
                    ("config.recovery_work_secs", "number"),
                    ("config.recovery_range", "number"),
                    ("config.maximum_parent_speed", "number"),
                    ("config.release_offset.0", "number"),
                    ("config.release_offset.1", "number"),
                    ("config.release_offset.2", "number"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "jamming_emitter".into(),
                [
                    ("enabled_channel", "boolean_channel"),
                    ("target_channel", "entity_target_channel"),
                    ("power_input", "power_input_component"),
                    ("config.mount.facing_deg", "number"),
                    ("config.mount.fire_arc_deg", "number"),
                    ("config.mount.range", "number"),
                    ("config.effect", "text"),
                    ("config.falloff", "text"),
                    ("config.strength", "number"),
                    ("config.maximum_sample_age_secs", "number"),
                    ("config.reading_ticks", "integer"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "concealment_emitter".into(),
                [
                    ("enabled_channel", "boolean_channel"),
                    ("power_input", "power_input_component"),
                    ("config.max_suppression", "number"),
                    ("config.emission_scale", "number"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "identity_emitter".into(),
                [
                    ("enabled_channel", "boolean_channel"),
                    ("power_input", "power_input_component"),
                    ("config.callsign", "string"),
                    ("config.faction", "optional_string"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "signal_protection".into(),
                [
                    ("enabled_channel", "boolean_channel"),
                    ("priorities_channel", "system_priority_channel"),
                    ("power_input", "power_input_component"),
                    ("config.capacity", "integer"),
                    ("config.strength", "number"),
                    ("config.systems", "json"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "illumination_emitter".into(),
                [
                    ("enabled_channel", "boolean_channel"),
                    ("target_channel", "entity_target_channel"),
                    ("power_input", "power_input_component"),
                    ("config.mount.facing_deg", "number"),
                    ("config.mount.fire_arc_deg", "number"),
                    ("config.mount.range", "number"),
                    ("config.link_range", "number"),
                    ("config.minimum_transmission", "number"),
                    ("config.reading_ticks", "integer"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "turret".into(),
                [
                    ("power_input", "power_input_component"),
                    ("config.facing_deg", "number"),
                    ("config.traverse_arc_deg", "number"),
                    ("config.traverse_deg_per_sec", "number"),
                    ("config.aim_tolerance_deg", "number"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "access_boundary".into(),
                [
                    ("sealed_channel", "boolean_channel"),
                    ("transition_secs", "number"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
            (
                "dock_mating".into(),
                [
                    ("enabled_channel", "boolean_channel"),
                    ("target_channel", "entity_target_channel"),
                    ("power_input", "power_input_component"),
                    ("config.range", "number"),
                    ("config.engage_distance", "number"),
                    ("config.approach_speed", "number"),
                    ("config.mate_tolerance", "number"),
                    ("config.undock_clear_distance", "number"),
                    ("config.moves_body", "boolean"),
                    ("config.min_power_level", "integer"),
                ]
                .into_iter()
                .map(|(name, kind)| WorkshopComponentField {
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect(),
            ),
        ]
        .into_iter()
        .collect(),
        channel_kinds: vec![
            "boolean".into(),
            "entity_target".into(),
            "system_priority".into(),
        ],
        ai_state_tree: WorkshopAiTreeSchema {
            system_kinds: vec![
                crate::ship::system_registry::SIGNAL_PROTECTION_KIND.into(),
                crate::ship::system_registry::CRAFT_BAY_KIND.into(),
                crate::ship::system_registry::ILLUMINATION_KIND.into(),
                crate::ship::system_registry::JAMMING_KIND.into(),
                crate::ship::system_registry::IDENTITY_EMITTER_KIND.into(),
                crate::ship::system_registry::CONCEALMENT_KIND.into(),
                crate::ship::system_registry::DOCK_KIND.into(),
                crate::ship::system_registry::POWER_REACTOR_KIND.into(),
                crate::ship::system_registry::SHIELD_ARC_KIND.into(),
                crate::ship::system_registry::ACCESS_GATE_KIND.into(),
                crate::ship::system_registry::LOGISTICS_KIND.into(),
                crate::ship::system_registry::FABRICATION_KIND.into(),
                crate::ship::system_registry::RECOVERY_KIND.into(),
                crate::ship::system_registry::SENSORS_KIND.into(),
                crate::ship::system_registry::TACTICAL_RADAR_KIND.into(),
                crate::ship::system_registry::PASSIVE_RECEIVER_KIND.into(),
            ],
            shield_recovery_bindings,
            hosts: BTreeMap::from([
                (
                    crate::ship::system_registry::CONCEALMENT_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::electronic::concealment::bindings("enabled"),
                    ),
                ),
                (
                    crate::ship::system_registry::IDENTITY_EMITTER_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::electronic::identity::bindings("enabled"),
                    ),
                ),
                (
                    crate::ship::system_registry::SIGNAL_PROTECTION_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::electronic::protection::bindings("enabled"),
                    ),
                ),
                (
                    crate::ship::system_registry::CRAFT_BAY_KIND.into(),
                    crate::ai::state_tree::schema_metadata(&crate::ship::craft::bindings()),
                ),
                (
                    crate::ship::system_registry::JAMMING_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::ship::composition::enabled_target_bindings(
                            "enabled", "target",
                        ),
                    ),
                ),
                (
                    crate::ship::system_registry::ILLUMINATION_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::weapons::illumination::bindings("enabled", "target"),
                    ),
                ),
                (
                    crate::ship::system_registry::SENSORS_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::science::sensing_ai::focused_bindings(),
                    ),
                ),
                (
                    crate::ship::system_registry::TACTICAL_RADAR_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::science::sensing_ai::targeting_bindings(),
                    ),
                ),
                (
                    crate::ship::system_registry::PASSIVE_RECEIVER_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::science::sensing_ai::bindings(),
                    ),
                ),
                (
                    crate::ship::system_registry::RECOVERY_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::resources::recovery_ai::bindings(),
                    ),
                ),
                (
                    crate::ship::system_registry::FABRICATION_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::resources::production_ai::bindings(),
                    ),
                ),
                (
                    crate::ship::system_registry::LOGISTICS_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::resources::logistics_ai::bindings(),
                    ),
                ),
                (
                    crate::ship::system_registry::ACCESS_GATE_KIND.into(),
                    crate::ai::state_tree::schema_metadata(
                        &phoenix_sim_gameplay::ship::access::bindings("sealed"),
                    ),
                ),
                (
                    crate::ship::system_registry::DOCK_KIND.into(),
                    crate::ai::state_tree::schema_metadata(&crate::ai::state_tree::dock_bindings(
                        "enabled",
                    )),
                ),
                (
                    crate::ship::system_registry::POWER_REACTOR_KIND.into(),
                    power_tree_vocabulary,
                ),
                (
                    crate::ship::system_registry::SHIELD_ARC_KIND.into(),
                    crate::ai::state_tree::schema_metadata_with_numbers(
                        &shield_base,
                        &shield_numbers,
                    ),
                ),
            ]),
            vocabulary: crate::ai::state_tree::schema_metadata(
                &crate::ai::state_tree::dock_bindings("enabled"),
            ),
        },
        job_workflow: WorkshopJobWorkflowSchema {
            vocabulary: crate::ai::state_tree::schema_metadata(
                &crate::jobs::repair_workflow::bindings(),
            ),
            default_tree: crate::jobs::repair_workflow::default_config(),
        },
        system_kinds,
        directive_kinds: crate::objectives::directive::DirectiveKind::ALL
            .into_iter()
            .map(|kind| kind.name().to_owned())
            .collect(),
    }
}

impl WorkshopValidation {
    pub fn error(&mut self, category: &str, file: &str, message: String) {
        self.findings.push(WorkshopFinding {
            severity: "error".into(),
            category: category.into(),
            message,
            file: file.into(),
            line: None,
        });
    }

    pub fn extend(&mut self, findings: impl IntoIterator<Item = WorldFinding>) {
        self.extend_workshop(findings.into_iter().map(WorkshopFinding::from));
    }

    pub fn extend_workshop(&mut self, findings: impl IntoIterator<Item = WorkshopFinding>) {
        for finding in findings {
            if !self.findings.contains(&finding) {
                self.findings.push(finding);
            }
        }
    }
}

pub struct Sources(pub BTreeMap<String, String>);

impl WorldReader for Sources {
    fn read(&self, path: &str) -> Option<String> {
        self.0.get(&canonical_template_path(path)).cloned()
    }
}

impl ScriptResolver for Sources {
    fn read(&self, path: &str) -> Option<String> {
        WorldReader::read(self, path)
    }
}

impl TemplateLoader for Sources {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        resolve_template(path, &self.0).ok()?.parse().ok()
    }
    fn absence_is_final(&self) -> bool {
        true
    }
}

/// Runtime validation of the exact unsaved candidate archive. The ordinary
/// upload validator supplies the manifest/identity/path/include/script gate.
/// The ordinary world loader additionally compiles each selected root and its
/// static children, so script-spawn references use the exact compiled source
/// set, including read-only dependencies. We retain its ledger as unused data.
pub fn validate_pack(bytes: &[u8], dependencies: &WorkshopDependencies) -> WorkshopValidation {
    crate::world::script::init_hashing_seed();
    let mut report = WorkshopValidation::default();
    let Some(identity) = dependencies
        .base_files
        .get("assets/scenarios.toml")
        .and_then(|text| parse_content_identity(text))
    else {
        report.error(
            "missing-base-content",
            "assets/scenarios.toml",
            "The read-only base content manifest is unavailable or has no content identity.".into(),
        );
        return report;
    };
    let active: Vec<ActivePack> = dependencies
        .packs
        .iter()
        .map(|pack| ActivePack {
            id: pack.id.clone(),
            manifest_toml: pack.manifest_toml.clone(),
            files: pack.files.clone().into_iter().collect(),
            assets: pack
                .assets
                .iter()
                .map(|(path, bytes)| (path.clone(), std::sync::Arc::from(bytes.as_slice())))
                .collect(),
            ..Default::default()
        })
        .collect();
    let mut beneath = dependencies.base_files.clone();
    for pack in &dependencies.packs {
        beneath.extend(pack.files.clone());
    }
    let base_templates = Sources(beneath.clone());
    let validation = validate_mod_pack_with_assets(
        bytes,
        &identity,
        |path| {
            dependencies
                .base_files
                .get(&canonical_template_path(path))
                .cloned()
        },
        &base_templates,
        &active,
        &|path| {
            dependencies
                .base_assets
                .get(path)
                .map(|bytes| std::sync::Arc::from(bytes.as_slice()))
        },
        &dependencies.base_assets.keys().cloned().collect::<Vec<_>>(),
    );
    report.extend(validation.findings);
    let mut candidate = validation.files;
    candidate.insert("scenarios.toml".into(), validation.manifest_toml);
    // Definition references resolve against everything beneath the candidate,
    // so a pack may name a BASE faction as an enemy and a draft that deletes
    // a faction is told where it is still referenced (issue #1474).
    report.extend_workshop(definitions::findings(&candidate, &beneath));
    // Composition rules are findings here as well as edit-time refusals: a
    // hand-edited draft can still declare a missing or cyclic child, and
    // save, export and Test refuse it with a line (issue #1475).
    report.extend_workshop(composition::findings(&candidate, &beneath));
    // Include rules at the offending `includes` ENTRY line, for every entity
    // member the pack carries rather than only the templates a manifest root
    // spawns, so save and export refuse a broken closure with a location
    // (issue #1476).
    report.extend_workshop(entity::findings(&candidate, &beneath));
    // Role-preset rules at the offending value's own line, including the
    // entity names a widget's `ship` and a preset's `contacts` resolve against
    // — which one world's text cannot answer and the candidate can (issue
    // #1477).
    report.extend_workshop(presets::findings(&candidate, &beneath));
    report.catalogue = composition::scenario_catalogue(&candidate, &beneath);
    let mut sources = beneath;
    sources.extend(candidate.clone());
    let sources = Sources(sources);

    // Other complete source types parse through their runtime owner. Entity
    // members may be partial include fragments, so the existing pack gate
    // checks their syntax and validates COMPOSED reachable templates instead
    // of requiring every fragment to be a complete EntityConfig on its own.
    for (path, text) in &candidate {
        let result = if path.starts_with("assets/worlds/") && path.ends_with(".toml") {
            crate::world::config::parse_world(text).map(|_| ())
        } else if path.starts_with("assets/factions/") && path.ends_with(".toml") {
            crate::ai::faction::parse_faction_config(text)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else if path.starts_with("assets/models/") && path.ends_with(".toml") {
            crate::entities::model_rig::parse_model_rig(text)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            Ok(())
        };
        if let Err(error) = result {
            report.error("runtime-source-invalid", path, error);
        }
    }
    if let Some(manifest) = candidate
        .get("scenarios.toml")
        .and_then(|text| parse_manifest(text).ok())
    {
        for scenario in manifest.scenarios {
            validate_world(&scenario.world, &sources, &mut report);
        }
    }
    report.accepted = !report
        .findings
        .iter()
        .any(|finding| finding.severity == "error");
    report
}

pub fn validate_world(path: &str, sources: &Sources, report: &mut WorkshopValidation) {
    let read = |path: &str| {
        load(
            LoadRequest::new(path, sources, sources, LoadPolicy::Merge),
            sources,
            &sources.0,
        )
    };
    let root = match read(path) {
        Ok(root) => root,
        Err(error) => {
            report.error("runtime-world-invalid", path, error.to_string());
            return;
        }
    };
    let mut children = Vec::new();
    let mut walk = match crate::world::transit::SupportingWorldWalk::new(path, &root.config) {
        Ok(walk) => walk,
        Err(error) => {
            report.error("runtime-world-invalid", path, error);
            return;
        }
    };
    while let Some(child_path) = walk.next_path() {
        match read(&child_path) {
            Ok(child) => {
                if let Err(error) = walk.accept(&child_path, &child.config) {
                    report.error("runtime-world-invalid", &child_path, error);
                }
                children.push((child_path, child));
            }
            Err(error) => report.error("runtime-world-invalid", &child_path, error.to_string()),
        }
    }
    for world in std::iter::once(&root).chain(children.iter().map(|(_, child)| child)) {
        if let Some(compiled) = &world.scripts {
            report.extend(compiled.findings.clone());
        }
    }
    let mut recipient_catalog = crate::recipients::RecipientCatalog {
        ship_slots: root
            .config
            .effective_ship_slots()
            .into_iter()
            .map(|slot| slot.id)
            .collect(),
        factions: sources
            .0
            .iter()
            .filter(|(path, _)| path.starts_with("assets/factions/") && path.ends_with(".toml"))
            .filter_map(|(_, source)| crate::ai::faction::parse_faction_config(source).ok())
            .map(|faction| faction.name)
            .collect(),
        ..Default::default()
    };
    let mut recipient_references =
        crate::world::script::recipient_refs::RecipientScriptReferences::default();
    for world in std::iter::once(&root).chain(children.iter().map(|(_, child)| child)) {
        if let Some(compiled) = &world.scripts {
            recipient_references
                .selections
                .extend(compiled.recipient_references.selections.iter().cloned());
            recipient_references
                .malformed
                .extend(compiled.recipient_references.malformed.iter().cloned());
            recipient_references
                .instances
                .extend(compiled.recipient_references.instances.iter().cloned());
            recipient_references.computed_objective_identity |=
                compiled.recipient_references.computed_objective_identity;
            recipient_references.computed_objectives.extend(
                compiled
                    .recipient_references
                    .computed_objectives
                    .iter()
                    .cloned(),
            );
            recipient_catalog
                .objective_instances
                .extend(compiled.recipient_references.declarations.iter().cloned());
        }
    }
    report.extend(recipient_references.validate(&recipient_catalog));
    // Keep the source records separate from the borrowed validation views.
    let root_text = WorldReader::read(sources, path).unwrap_or_default();
    let mut root_source = WorldSource::new(path, &root_text, &root.config);
    if let Some(compiled) = &root.scripts {
        root_source = root_source.with_resolved_script_spawns(&compiled.spawned_templates);
    }
    let child_sources: Vec<_> = children
        .iter()
        .map(|(path, world)| {
            let text = sources
                .0
                .get(&canonical_template_path(path))
                .map(String::as_str)
                .unwrap_or("");
            let mut source = WorldSource::new(path, text, &world.config)
                .with_parent_path(walk.parent_path(path));
            if let Some(compiled) = &world.scripts {
                source = source.with_resolved_script_spawns(&compiled.spawned_templates);
            }
            source
        })
        .collect();
    report.extend(validate_composition_with_fragments(
        &root_source,
        &child_sources,
        sources,
        &sources.0,
    ));
}

/// A selected project's source set uses the ordinary project manifest and the
/// same offline world/include/script validators. No pack header is invented,
/// and no process-global template cache can fill a missing source member.
pub fn validate_project(files: &BTreeMap<String, Vec<u8>>) -> WorkshopValidation {
    crate::world::script::init_hashing_seed();
    let mut report = WorkshopValidation::default();
    let mut text_files = BTreeMap::new();
    let descriptor_sources = crate::world::pack_asset_validation::descriptor_sources(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    for (path, bytes) in files {
        if !path.ends_with(".toml")
            && !path.ends_with(".rhai")
            && path != "assets/strings/strings.csv"
        {
            if let Err(error) = crate::world::pack_asset_validation::validate_member(
                path,
                bytes,
                &|path| {
                    files
                        .get(path)
                        .map(|bytes| std::sync::Arc::from(bytes.as_slice()))
                },
                &descriptor_sources,
            ) {
                report.error("invalid-runtime-asset", path, error);
            }
            continue;
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                if path.ends_with(".toml") {
                    if let Err(error) = toml::from_str::<toml::Value>(text) {
                        report.error("runtime-source-invalid", path, error.to_string());
                    }
                }
                text_files.insert(path.clone(), text.to_string());
            }
            Err(error) => report.error("runtime-source-invalid", path, error.to_string()),
        }
    }
    let sources = Sources(text_files);
    if let Some(source) = sources.0.get(crate::sound_cues::PATH) {
        if let Err(error) =
            crate::world::pack_asset_validation::validate_sound_catalog(source, &|path| {
                files
                    .get(path)
                    .map(|bytes| std::sync::Arc::from(bytes.as_slice()))
            })
        {
            report.error("invalid-sound-cues", crate::sound_cues::PATH, error);
        }
    }
    report.extend(crate::world::mod_pack::validate_pack_scripts(&sources.0));
    // A project is its whole content set; nothing lies beneath it.
    report.extend_workshop(definitions::findings(&sources.0, &BTreeMap::new()));
    report.extend_workshop(composition::findings(&sources.0, &BTreeMap::new()));
    report.extend_workshop(entity::findings(&sources.0, &BTreeMap::new()));
    report.extend_workshop(presets::findings(&sources.0, &BTreeMap::new()));
    report.catalogue = composition::scenario_catalogue(&sources.0, &BTreeMap::new());
    for (path, text) in &sources.0 {
        let result = if path.starts_with("assets/worlds/") && path.ends_with(".toml") {
            crate::world::config::parse_world(text).map(|_| ())
        } else if path.starts_with("assets/factions/") && path.ends_with(".toml") {
            crate::ai::faction::parse_faction_config(text)
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else if path.starts_with("assets/models/") && path.ends_with(".toml") {
            crate::entities::model_rig::parse_model_rig(text)
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else {
            Ok(())
        };
        if let Err(error) = result {
            report.error("runtime-source-invalid", path, error);
        }
    }
    let path = "assets/scenarios.toml";
    match sources.0.get(path).map(|source| parse_manifest(source)) {
        Some(Ok(manifest)) => {
            report.extend(
                crate::world::manifest::validate_manifest(&manifest, &sources.0[path], |path| {
                    sources.0.get(path).cloned()
                })
                .into_iter()
                .map(|mut finding| {
                    if finding.source.file == "scenarios.toml" {
                        finding.source.file = path.into();
                    }
                    finding
                }),
            );
            for scenario in manifest.scenarios {
                validate_world(&scenario.world, &sources, &mut report);
            }
        }
        Some(Err(error)) => report.error("runtime-manifest-invalid", path, error.to_string()),
        None => report.error(
            "runtime-manifest-invalid",
            path,
            "Missing project manifest".into(),
        ),
    }
    report.accepted = !report
        .findings
        .iter()
        .any(|finding| finding.severity == "error");
    report
}

#[cfg(test)]
mod emission_profile_tests;
#[cfg(test)]
mod tests;

use crate::entities::include_resolve::ParseEntityTemplate as _;
