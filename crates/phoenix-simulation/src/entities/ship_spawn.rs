//! One Bevy installation path for authored ship capabilities and common ship state.
//! Crew selection and local/fleet identity belong to the GameStart adapter.

use super::config::EntityConfig;
use super::spawner::{HelmConsoleSection, WeaponsConsoleSection};
use bevy::prelude::*;

/// Resolved ship inputs. No equipment or player/NPC branch belongs in this seed.
pub(crate) struct ShipSpawnSeed {
    pub(crate) ship_config: crate::ship_plugin::ShipConfigComponent,
    pub(crate) control_sources: crate::ship_plugin::ShipSystemControlSources,
    pub(crate) active_ratings: crate::ship_plugin::ActiveStationRatings,
    pub(crate) crew_presence: crate::lobby::crew_replication::ShipCrewPresence,
    pub(crate) initial_yaw: f32,
}

impl ShipSpawnSeed {
    fn backfill(config: &EntityConfig) -> Self {
        let ship_config =
            crate::ship_plugin::ShipConfigComponent(config.ship_config.clone().unwrap_or_else(
                || crate::ship::config::ShipConfig {
                    stations: Vec::new(),
                    systems: Vec::new(),
                    power_groups: Default::default(),
                    consoles: Vec::new(),
                    coordination_lag_secs: 0.0,
                },
            ));
        let (resolver, ratings) = crate::ship::rating::seed_boot_ratings(&ship_config.0, |_| {
            crate::ship::rating::BACKFILL_RATING.to_string()
        });
        Self {
            ship_config,
            control_sources: crate::ship_plugin::ShipSystemControlSources(resolver),
            active_ratings: crate::ship_plugin::ActiveStationRatings(ratings),
            crew_presence: Default::default(),
            initial_yaw: 0.0,
        }
    }
}

pub(super) fn install(
    config: &EntityConfig,
    position: Vec3,
    uuid: &str,
    seed: Option<ShipSpawnSeed>,
    cmds: &mut EntityCommands,
) {
    let is_ship = seed.is_some() || config.behaviour.is_some() || config.is_static_point_defence();
    if is_ship {
        let seed = seed.unwrap_or_else(|| ShipSpawnSeed::backfill(config));
        let groups = insert_ship_config_and_core_bundle(config, position, seed, cmds);
        insert_ship_scratch_state(cmds);
        insert_power_state(config, &groups, cmds);
        insert_target_selectors(config, cmds);
        insert_ai_policies(config, cmds);
        insert_power_multipliers_and_modifiers(config, uuid, build_power_multipliers(config), cmds);
        insert_ship_markers_and_trackers(cmds);
    } else if config.energy.is_some()
        || config.ship_config.as_ref().is_some_and(|ship| {
            ship.systems
                .iter()
                .any(|system| system.kind == crate::ship::system_registry::ACCESS_GATE_KIND)
        })
    {
        // Physical equipment keeps the same System/Console ownership and energy
        // owner on a static entity. This adds neither crew seats nor ship AI.
        let seed = ShipSpawnSeed::backfill(config);
        cmds.insert((
            seed.ship_config,
            seed.control_sources,
            seed.active_ratings,
            seed.crew_presence,
            crate::server_app::ShipSystemBlackboards::default(),
            crate::core::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::ship::shields::ShieldsDamageHistory::default(),
        ));
        insert_power_state(config, &[], cmds);
        if config.ship_config.as_ref().is_some_and(|ship| {
            ship.systems.iter().any(|system| {
                crate::ship::composition::dock_composition(&system.components)
                    .is_some_and(|dock| dock.config.moves_body)
            })
        }) {
            // Controlled mechanical equipment shares the existing captured pose;
            // this adds no Ship, crew, AI or ordinary propulsion integrator.
            cmds.insert(crate::ship::state::ShipPhysics {
                x: position.x,
                y: position.y,
                z: position.z,
                ..Default::default()
            });
        }
    }
    // These capabilities also belong to independently authored nonship entities.
    if let Some(layout) = &config.interior {
        cmds.insert(crate::ship::access::InteriorLayout(layout.clone()));
    }
    if let Some(ship) = &config.ship_config {
        let access = crate::ship::access::InteriorAccess::from_ship(ship);
        if !access.controls.is_empty() {
            cmds.insert(access);
        }
    }
    super::craft::install(config, position, cmds);
    crate::resources::logistics_server::install(config, cmds);
    crate::resources::production_server::install(config, cmds);
    crate::resources::recovery_server::install(config, cmds);
    crate::resources::fitting_server::install(config, uuid, cmds);
    crate::resources::equipment_recovery_server::install(config, cmds);
    crate::science::passive::install(config, cmds);
    crate::science::identity::install(config, cmds);
    crate::science::protection::install(config, cmds);
    crate::science::broad::install(config, cmds);
    crate::comms::radio::install(config, cmds);
    crate::comms::jamming::install(config, cmds);
    crate::weapons::mounts::install(config, cmds);
    crate::console::weapons::illumination::install(config, cmds);
    crate::console::helm::transit::install(config, cmds);
    crate::console::weapons::assignments::install(config, cmds);
    crate::console::navigation::sharing::install(config, cmds);
    crate::console::navigation::knowledge::install(config, cmds);
    insert_shield_tuning(config, cmds);
    install_weapons(config, cmds);
    install_torpedoes(config, cmds);
    install_blasters(config, cmds);
    install_helm(config, is_ship, cmds);
    install_shields(config, cmds);
}

fn insert_ship_config_and_core_bundle(
    config: &EntityConfig,
    position: Vec3,
    seed: ShipSpawnSeed,
    cmds: &mut EntityCommands,
) -> Vec<crate::modifiers::power_system::AuthoredPowerGroup> {
    let ShipSpawnSeed {
        ship_config,
        control_sources,
        active_ratings,
        crew_presence,
        initial_yaw,
    } = seed;
    cmds.insert(crew_presence);
    cmds.insert(crate::server_app::ShipSystemBlackboards::default());
    // Seed the reactor from the ship's authored power groups (issue #762)
    // BEFORE `ship_config` is moved into the entity, so authored groups
    // beyond the canonical three (e.g. `ops`) are allocatable rather than
    // returning `UnknownGroup`. Empty for ships with no `[power_groups.*]`.
    let power_group_seed =
        crate::ship::power::authored_power_group_seed(&ship_config.0.power_groups);
    // Seed ShipPhysics from the spawn position so the per-entity helm loop
    // starts with the correct initial state rather than (0, 0).
    let ship_physics = crate::ship::state::ShipPhysics {
        x: position.x,
        z: position.z,
        yaw: initial_yaw,
        ..Default::default()
    };
    cmds.insert((
        ship_config,
        crate::core::messages::AdmittedCommands::default(),
        control_sources,
        active_ratings,
        crate::ship_plugin::CoordinationQueue::default(),
        ship_physics,
        crate::ship_plugin::HelmWaypointClearance::default(),
        crate::console::weapons::TacticalRadarSelection::default(),
        crate::console::weapons::ActiveBeam::default(),
        crate::console::weapons::PhaserCooldown::default(),
        crate::ship::sensors::SensorRadarSelection::default(),
        // The alert. Its former bundle-mate `ShipWeaponsHold` (issue #1041)
        // was retired in #1398: restraint is a POWER order now, so the state
        // the fire gate reads is the ship's own reactor and there is no second
        // per-ship boolean to spawn.
        crate::ship::state::ShipRedAlert::default(),
        crate::ship::state::ShipViewMode::default(),
        crate::ship::state::ShipPhaserFrequency::default(),
        crate::console::navigation::NavigationWaypoint::default(),
    ));
    cmds.insert(crate::gm::operations::puppet::capability::NpcStationConfig(
        crate::lobby::server::project_ship_client_config(config),
    ));
    power_group_seed
}

/// Per-ship scratch/coordination state every ship carries: helm intent,
/// objective cursors, the channel-3 debounce cells, and the repair queue. All
/// default-constructed, so this depends on neither the config nor the position.
fn insert_ship_scratch_state(cmds: &mut EntityCommands) {
    // Per-entity helm intent (audit follow-up). Every ship carries
    // its own `LastHelmInput` so systems that iterate `With<Ship>`
    // and read `Option<&LastHelmInput>` see a real value on NPCs
    // instead of the `unwrap_or_default()` fallback. Notably
    // `console_ai::server::ai_power_allocation` reads `thrust` to drive
    // its hysteresis-based movement rule (`tick_power_movement_rule`),
    // engaging/disengaging helm power by ±1 on sustained high/low
    // thrust rather than pinning to an absolute level. Inserted
    // separately because Bevy's tuple Bundle max is 15 elements.
    // `ShipStationStances` rides the same command as `LastHelmInput` (a
    // tuple is one Bundle, one archetype move) so a hull nobody commands
    // pays no extra insert and stays byte-identical. Empty by default: only
    // a human Command operator's explicit stance pick ever fills it.
    cmds.insert((
        crate::ship_plugin::LastHelmInput::default(),
        crate::console::command::server::ShipStationStances::default(),
        // Edge-detection scratch for the persist-behind-human trigger
        // (issue #1108). Transient and NOT folded into the sim digest —
        // see the type's own docs; a fresh/reloaded hull records its first
        // observation and fires no edge.
        crate::console::command::server::LastDirectedControl::default(),
    ));
    // Per-objective route cursors: where this ship is on each objective's
    // route. Read by the low-LOD `simulate_low_lod_ships` path, the high-LOD
    // `helm_patrol`, and `operate_navigation_ai`; written only by
    // `advance_objective_cursors`. A ship without one cannot patrol.
    // Inserted separately to stay under Bevy's tuple-Bundle element cap.
    cmds.insert(crate::ai::server::ObjectiveCursors::default());
    // Per-ship coordination bus state (audit follow-up). Every ship
    // tracks its own shields down/restore notification cycle and its
    // own sensors→tactical frequency-hint dedupe state so the two
    // coordination emitters (`emit_shields_coordination`,
    // `tick_sensors_frequency_hint`) can iterate `With<Ship>` and
    // route into each ship's own `CoordinationQueue` via
    // `CoordinationEnqueue.source_entity`.
    cmds.insert(crate::ship::shields::ShieldsCoordinationState::default());
    cmds.insert(crate::ship::sensors::SensorsFrequencyState::default());
    cmds.insert(crate::ship::sensors::SensorsThreatState::default());
    // Power brownout advisory debounce state (issue #678): per-ship
    // so each ship tracks its own brownout notification cycle.
    cmds.insert(crate::ship::power::PowerBrownoutState::default());
    // Weapons->Helm arc-bearing request state (issue #677): per-ship
    // debounce for the channel-3 request, and the pending bearing Helm
    // AI folds into its steering once the request is consumed.
    cmds.insert(crate::console::weapons::WeaponsArcRequestState::default());
    cmds.insert(crate::ship_plugin::PendingArcBearingRequest::default());
    // Distinct docking intent (issue #742): the sanctioned home for
    // controlled reverse / lateral close manoeuvres, kept separate from the
    // facing-only arc-bearing request above.
    cmds.insert(crate::ship_plugin::DockingMotionIntent::default());
    cmds.insert(crate::ship::shields::PendingShieldsThreatBearing::default());
    // Sensors→Tactical frequency advisory a backfilled Tactical consumes
    // off the channel-3 bus (issue #873).
    cmds.insert(crate::ship_plugin::PendingTacticalFrequencyHint::default());
    // Per-ship intent-narration memory (issue #879): the previous decision
    // snapshot of each narrating seat plus this ship's advisory counter.
    // Belongs to the ship rather than to its LOD tier — a demoted hull
    // still has seats to narrate for if a human ever takes one.
    cmds.insert(crate::ship_plugin::ShipIntentNarration::default());
    cmds.insert(crate::ship_plugin::LastSystemTiers::default());
    cmds.insert(crate::ship_plugin::RepairHumanAlerted::default());
    cmds.insert(crate::console::repair::server::RepairRequestQueue::default());
}

/// The reactor and its allocation AI: the [`ShipPowerSystem`] (seeded from the
/// authored power groups), the per-entity [`PowerConfigResource`], and the
/// closed authored System trees with their own private continuation.
fn insert_power_state(
    config: &EntityConfig,
    _power_group_seed: &[crate::modifiers::power_system::AuthoredPowerGroup],
    cmds: &mut EntityCommands,
) {
    if let Some(ship) = &config.ship_config {
        let trees = crate::ai::state_tree_runtime::compile_ship_trees_with_energy(
            ship,
            config.energy.as_ref(),
        )
        .expect("validated System AI trees");
        cmds.insert(trees);
    }
    let Some(hardware) = config.energy.as_ref() else {
        // Reduced test entities may omit machinery; accepted playable content
        // is required to declare its physical energy profile at validation.
        return;
    };
    let ship = config
        .ship_config
        .as_ref()
        .expect("energy hardware requires a ship definition");
    let mut power =
        crate::modifiers::power_system::PowerSystem::from_energy(hardware.clone(), ship.clone())
            .expect("validated energy hardware");
    let mut legacy = crate::ship::power::PowerConfigResource::default();
    legacy.0.capacity = hardware.reactor.reserve.capacity;
    if let Some(config) = &config.power {
        legacy.0.strike_reserve = config.strike_reserve.clone();
    }
    power = power
        .with_strike_reserve(legacy.0.strike_reserve.clone())
        .expect("validated strike overdrive");
    cmds.insert((crate::ship::power::ShipPowerSystem(power), legacy));
}

/// Per-entity power multipliers, defaulted then overridden by any per-console
/// `power_multipliers` blocks. Pure — no inserts; the caller inserts the result
/// via [`insert_power_multipliers_and_modifiers`] at the original insert site.
fn build_power_multipliers(
    config: &EntityConfig,
) -> std::collections::HashMap<crate::core::messages::PowerGroupId, [f32; 4]> {
    // Per-entity power multipliers. Seeded from any per-console TOML
    // `power_multipliers` blocks (helm_console/weapons_console/shields_console)
    // and otherwise defaulted so NPC ships still get MaxSpeed / PhaserDamage
    // / ShieldRegen bonuses translated by `translate_power_modifiers`.
    //
    // After issue #617 the map is keyed by `PowerGroupId`.
    let defaults = [-0.5f32, 0.0, 0.25, 0.5];
    let mut multipliers: std::collections::HashMap<crate::core::messages::PowerGroupId, [f32; 4]> =
        std::collections::HashMap::from([
            (
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                defaults,
            ),
            (
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                defaults,
            ),
            (
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                defaults,
            ),
        ]);
    if let Some(hc) = &config.helm_console {
        if let Some(pm) = hc.power_multipliers {
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    if let Some(wc) = &config.weapons_console {
        if let Some(pm) = wc.power_multipliers {
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    if let Some(sc) = &config.shields_console {
        if let Some(pm) = sc.power_multipliers {
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    multipliers
}

/// The AI target/selector policies read straight off the authored `selector`
/// blocks: sensors AI config, then the sensors / tactical / navigation / repair
/// target selectors.
fn insert_target_selectors(config: &EntityConfig, cmds: &mut EntityCommands) {
    // Every ship carries its own sensors tuning, with schema defaults when absent.
    cmds.insert(
        config
            .sensors_console
            .as_ref()
            .and_then(|sc| sc.ai.as_ref())
            .map(|ai| crate::ship::sensors::SensorsAiConfigResource {
                frequency_hint_delay_secs: ai.frequency_hint_delay_secs,
            })
            .unwrap_or_default(),
    );
    // Sensors target selector (issue #776) — the per-system ranking policy
    // `operate_sensors_ai` runs to pick the science target, from the
    // authored `[sensors_console.selector]` block. Since #885b stage 5d
    // there is no Rust-side synthesiser behind it: strict AI-declaration
    // mode rejects an AI-capable hull that omits the block at load, so an
    // unauthored selector means no component and therefore no ranking rather
    // than an invented one. `to_selector` cannot fail here: the block was
    // validated in `EntityConfig::from_toml`. Power rating is exposed to the
    // selector as `self_fact(power_rating)`.
    if let Some(s) = config
        .sensors_console
        .as_ref()
        .and_then(|sc| sc.selector.as_ref())
    {
        cmds.insert(crate::ship::sensors::SensorsTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    }
    // Tactical target selector (issue #777) — the per-system ranking policy
    // `ai_target_selection` runs to pick the authoritative weapons target,
    // from the authored `[weapons_console.selector]` block.
    if let Some(s) = config
        .weapons_console
        .as_ref()
        .and_then(|wc| wc.selector.as_ref())
    {
        cmds.insert(crate::console::weapons::TacticalTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
            // AC6 (issue #781): explicit radar idle from `[weapons_console]
            // selector_idle`, else the baseline (radar runs its selector).
            idle: config
                .weapons_console
                .as_ref()
                .map(|wc| wc.selector_idle)
                .unwrap_or(false),
        });
    }
    // Navigation target selector (issue #778) — the per-system ranking
    // policy `operate_navigation_ai` runs to rank objective destinations and
    // eligible chart contacts into the shared Waypoint, from the authored
    // `[navigation_console.selector]` block.
    if let Some(s) = config
        .navigation_console
        .as_ref()
        .and_then(|nc| nc.selector.as_ref())
    {
        cmds.insert(crate::console::navigation::NavigationTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    }
    // Repair target selector (issue #785) — the per-system ranking policy
    // `operate_repair_ai` runs once per free repair team to rank the ship's
    // damaged stations into ordinary admitted `DispatchRepairTeam` inputs,
    // from the authored `[repair.selector]` block. Attached whenever the
    // selector is authored, not only to ships that declare repair TEAMS —
    // the teams component is what gates dispatch, and a ship that gains
    // teams later still has its ranking.
    if let Some(s) = config.repair.as_ref().and_then(|rc| rc.selector.as_ref()) {
        cmds.insert(crate::console::repair::server::RepairTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    }
}

/// The remaining per-console AI policies: comms hail selector + response policy,
/// shields AI config + focus policy, the captain red-alert policy, and the helm
/// fine-system policy map.
fn insert_ai_policies(config: &EntityConfig, cmds: &mut EntityCommands) {
    // Reuse the console's authored-policy conversion; missing declarations add nothing.
    let (comms_selector, comms_response_policy, comms_response_cadence) =
        crate::console::comms::server::comms_console_ai_components(config);
    if let Some(sel) = comms_selector {
        cmds.insert(sel);
    }
    if let Some(policy) = comms_response_policy {
        cmds.insert(policy);
    }
    if let Some(cadence) = comms_response_cadence {
        cmds.insert(cadence);
    }
    // Captain AI policy (issue #775) — the inline stateless Red Alert
    // policy from the authored `[captain_console.ai]` block, so
    // `operate_captain_ai` reads a data-authored policy rather than a
    // hardcoded controller.
    if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
        cmds.insert(crate::console::captain::server::CaptainAiPolicy(
            ai.to_policy().unwrap_or_default(),
        ));
    }
    // Helm fine-system AI policies (issues #779/#780, collapsed by #1209):
    // the inline `[helm_console.*_ai]` policies — engines (longitudinal),
    // steering (yaw), lateral, vertical, impulse, boost — resolved into ONE
    // keyed `FineSystemAiPolicies` map so each host reads a data-authored mode
    // verb by its `system_id()` rather than actuating unconditionally. One
    // entry per authored block; an unauthored axis contributes none (strict
    // AI-declaration mode rejects an unauthored AI-capable axis at load).
    // Built the same shape the weapon banks use — mirror of
    // `PhaserBankAiPolicies`. `to_policy` cannot fail: each block was
    // validated in `EntityConfig::from_toml`.
    if let Some(hc) = config.helm_console.as_ref() {
        use crate::ship::system_registry as sr;
        let mut fine_policies: std::collections::BTreeMap<
            crate::core::messages::SystemId,
            crate::ai::policy::AiPolicy,
        > = std::collections::BTreeMap::new();
        let boost_owner = if let Some(config) = &config.ship_config {
            let mut owners = config
                .systems
                .iter()
                .filter(|system| system.kind == sr::HELM_BOOST_KIND);
            let first = owners.next().map(|system| system.id.clone());
            if owners.next().is_none() {
                first
            } else {
                None
            }
        } else {
            Some(sr::helm_boost_system_id())
        };
        for (block, system_id) in [
            (
                hc.engines_ai.as_ref(),
                config
                    .helm_control
                    .as_ref()
                    .map_or_else(sr::helm_thrust_system_id, |profile| profile.thrust.clone()),
            ),
            (
                hc.steering_ai.as_ref(),
                config
                    .helm_control
                    .as_ref()
                    .map_or_else(sr::helm_steering_system_id, |profile| {
                        profile.steering.clone()
                    }),
            ),
            (
                hc.lateral_ai.as_ref(),
                config
                    .helm_control
                    .as_ref()
                    .and_then(|profile| profile.lateral.clone())
                    .unwrap_or_else(sr::lateral_thrust_system_id),
            ),
            (hc.vertical_ai.as_ref(), sr::vertical_thrust_system_id()),
            (hc.impulse_ai.as_ref(), sr::helm_impulse_system_id()),
            (
                hc.boost_ai.as_ref().filter(|_| boost_owner.is_some()),
                boost_owner.unwrap_or_else(sr::helm_boost_system_id),
            ),
        ] {
            if let Some(ai) = block {
                fine_policies.insert(system_id, ai.to_policy().unwrap_or_default());
            }
        }
        cmds.insert(crate::ship::helm_ai::FineSystemAiPolicies(fine_policies));
    }
}

/// Damage observation and ranking tuning belongs to the equipped entity,
/// independently of whether it has mobile ship behaviour.
fn insert_shield_tuning(config: &EntityConfig, cmds: &mut EntityCommands) {
    if config.shield_arcs.is_empty() {
        return;
    }
    cmds.insert(
        config
            .shields_console
            .as_ref()
            .and_then(|console| console.ai.as_ref())
            .map(|ai| crate::ship::shields::ShieldsAiConfigResource {
                damage_window_secs: ai.damage_window_secs,
                min_damage_window_secs: ai.min_damage_window_secs,
                damage_pct_threshold: ai.damage_pct_threshold,
                health_ratio_threshold: ai.health_ratio_threshold,
                ..Default::default()
            })
            .unwrap_or_default(),
    );
}

/// Inserts the [`PowerMultiplierResource`] built by [`build_power_multipliers`],
/// the empty per-entity [`ShipModifiers`] cache, and — only when the config
/// declares repair TEAMS — the [`ShipRepairTeams`].
fn insert_power_multipliers_and_modifiers(
    config: &EntityConfig,
    uuid: &str,
    multipliers: std::collections::HashMap<crate::core::messages::PowerGroupId, [f32; 4]>,
    cmds: &mut EntityCommands,
) {
    cmds.insert(crate::ship::power::PowerMultiplierResource { multipliers });
    // ShipModifiers as per-entity component (PR 6/9 — PRD #597). Every ship
    // gets an empty modifier cache. Region-entry observers and
    // translate_power_modifiers and translate_impulse_modifiers write to the
    // subject entity's cache, independent of which ship is locally projected.
    cmds.insert(crate::modifiers::ShipModifiers::new());
    // Per-entity ShipRepairTeams — only insert when the entity TOML declares
    // repair TEAMS, i.e. a `[repair] repair_team_count` above zero. No
    // count means the ship has no repair teams (the default behaviour for
    // NPCs today).
    //
    // The gate is the COUNT and not the presence of `[repair]`, because
    // since #885b every hull authors `[repair.selector]` and TOML cannot
    // write that sub-table without bringing `[repair]` into existence — a
    // presence gate would hand two teams to six NPC hulls that never had
    // any. See `RepairConfig::declares_teams`.
    if let Some(repair_cfg) = &config.repair {
        if repair_cfg.declares_teams() {
            let timings = repair_cfg.to_runtime();
            match crate::modifiers::repair_teams::RepairTeams::new_with_workflow(
                repair_cfg.repair_team_count as usize,
                timings,
                crate::ship::system_registry::repair_system_id(),
                repair_cfg.workflow.as_ref(),
            ) {
                Ok(mut teams) => {
                    if let (Some(interior), Some(ship)) = (&config.interior, &config.ship_config) {
                        match phoenix_sim_gameplay::ship::interior::InteriorGraph::compile(
                            interior,
                            ship,
                            repair_cfg.repair_team_count as usize,
                        ) {
                            Ok(layout) => teams.install_layout(layout),
                            Err(error) => {
                                error!("refusing repair teams with invalid interior: {error}");
                                return;
                            }
                        }
                    }
                    if let (Some(interior), Some(damage), Some(ship)) =
                        (&config.interior, &config.damage, &config.ship_config)
                    {
                        match phoenix_sim_gameplay::ship::compartment_damage::DamageDefinition::compile(interior,damage,ship) {
                            Ok(definition) => teams.install_damage_definition(definition),
                            Err(error) => { error!("refusing repair teams with invalid damage definition: {error}"); return; }
                        }
                    }
                    match phoenix_sim_gameplay::personnel::compile_entity(config) {
                        Ok(Some(profile)) => {
                            let actor = uuid.to_owned();
                            let entity = cmds.id();
                            cmds.insert(profile.clone());
                            cmds.commands().queue(move |world:&mut World| {
                                let source=world.get::<super::spawner::EntitySpawnOrigin>(entity).and_then(|origin|crate::console::repair::personnel::source(&origin.0,&actor).ok());
                                let tick=source.as_ref().map_or(0,|source|source.birth_tick);
                                world.init_resource::<phoenix_sim_gameplay::personnel::PersonnelRoster>();
                                if let Err(reason)=world.resource_mut::<phoenix_sim_gameplay::personnel::PersonnelRoster>().accept(&actor,profile,source,tick) {
                                    error!("refusing duplicate personnel origin: {reason}");
                                }
                            });
                        }
                        Ok(None) => {}
                        Err(error) => {
                            error!("refusing invalid personnel roster: {error}");
                            return;
                        }
                    }
                    cmds.insert(crate::console::repair::server::ShipRepairTeams(teams));
                }
                Err(error) => {
                    error!("refusing repair teams with invalid workflow: {error}");
                }
            }
        }
    }
}

/// The `Ship` marker and the per-ship trackers every hull (player + NPC) carries:
/// collision cooldown, combat-activity trackers, and the idle impulse/boost drives.
fn insert_ship_markers_and_trackers(cmds: &mut EntityCommands) {
    // All ship entities carry the Ship marker — player and NPC alike.
    // The LocalShip marker (not set here) is the viewscreen selector only.
    cmds.insert(crate::server_app::Ship);
    // Each ship owns its collision cooldown.
    cmds.insert(crate::server_app::CollisionCooldown::default());
    // Per-entity combat activity trackers (PRD #597 PR-10). Every ship
    // (player + NPC) records its own recent damage/hostile-fire/weapon
    // fire and last attacker.
    cmds.insert(crate::ship::combat_activity::RecentCombatActivity::default());
    cmds.insert(crate::server_app::WeaponFiredThisTick::default());
    cmds.insert(crate::server_app::ShipAttackedThisTick::default());
    cmds.insert(crate::console::weapons::LastShipAttacker::default());
    // Per-ship impulse drive state (audit follow-up). NPCs carry an
    // idle `ShipImpulse` so `handle_blocks_impulse_region_enter` can
    // route per-subject and future NPC helm AI can toggle impulse
    // through the same per-ship pathway the player uses.
    cmds.insert(crate::server_app::ShipImpulse::default());
    // Per-ship boost drive battery (audit follow-up). NPCs carry an
    // empty `ShipBoost` so future NPC helm AI can engage boost through
    // the same per-ship pathway the player uses.
    cmds.insert(crate::server_app::ShipBoost::default());
}

fn install_weapons(config: &EntityConfig, cmds: &mut EntityCommands) {
    // WeaponsConsole — attach a WeaponsConsoleSection so the AI can read weapons config from ECS.
    // Also insert PhaserCombatConfigResource and PhaserRenderConfig as per-entity Components
    // (PR 5/gap-review — PRD #597) so NPC ships share the same per-bank arc/range/damage
    // model as the player ship. tick_beams reads these components uniformly.
    if let Some(wc) = &config.weapons_console {
        cmds.insert(WeaponsConsoleSection(wc.clone()));
        // PhaserCombatConfig is built directly from the [[weapons_console.phaser_banks]] list.
        let combat_config = crate::entities::config::PhaserCombatConfig::from_weapons_console(wc);
        cmds.insert(crate::console::weapons::PhaserCombatConfigResource(
            combat_config,
        ));
        // Per-bank phaser open-fire AI policies (issue #781): each bank's inline
        // authored `ai` block. A bank that authors none contributes no entry —
        // since #885b stage 5d there is no synthesised fallback, and strict
        // AI-declaration mode rejects an unauthored bank at load. `to_policy`
        // cannot fail here — every authored bank block was validated in
        // `EntityConfig::from_toml`.
        let phaser_bank_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> =
            wc.phaser_banks
                .iter()
                .filter_map(|b| {
                    let ai = b.ai.as_ref()?;
                    Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                })
                .collect();
        cmds.insert(crate::console::weapons::PhaserBankAiPolicies(
            phaser_bank_policies,
        ));
        // The ship-level WEAPONS DOCTRINE (issue #956): which family this hull
        // turns to bring to bear. Validated in `EntityConfig::from_toml`, so
        // `to_policy` cannot fail here; a hull that authors none attaches no
        // component and asks Helm to turn for nothing, which strict
        // AI-declaration mode makes unreachable for an AI-bearing hull.
        if let Some(ai) = wc.ai.as_ref() {
            cmds.insert(crate::console::weapons::WeaponsDoctrineAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        // PhaserRenderConfig: take the first bank's beam_color if any, else default.
        let render_config = if let Some(first_bank) = wc.phaser_banks.first() {
            crate::console::weapons::PhaserRenderConfig {
                beam_color: crate::weapons::beam_render::resolve_beam_color(&first_bank.beam_color),
                beam_range: if first_bank.beam_range > 0.0 {
                    first_bank.beam_range
                } else {
                    40.0
                },
            }
        } else {
            crate::console::weapons::PhaserRenderConfig::default()
        };
        cmds.insert(render_config);
    }
}

fn install_torpedoes(config: &EntityConfig, cmds: &mut EntityCommands) {
    // An authored torpedo block owns both runtime state and its policies.
    if let Some(tc) = &config.torpedoes {
        let runtime_config = tc.to_runtime();
        let torpedo_system = if !tc.tubes.is_empty() {
            crate::weapons::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
        } else {
            crate::weapons::torpedo::TorpedoSystem::new(runtime_config)
        };
        cmds.insert(crate::console::weapons::TorpedoSystemResource(
            torpedo_system,
        ));
        cmds.insert(crate::console::weapons::guidance_link::ShipGuidanceKnowledge::default());

        // Per-tube torpedo load + launch AI policies (issue #782): each tube's
        // inline authored `ai` block. A tube that authors none contributes no
        // entry — strict AI-declaration mode rejects that at load.
        // Validated at load, so `to_policy` cannot fail here.
        let tube_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> = tc
            .tubes
            .iter()
            .filter_map(|t| {
                let ai = t.ai.as_ref()?;
                Some((t.id.clone(), ai.to_policy().unwrap_or_default()))
            })
            .collect();
        cmds.insert(crate::console::weapons::TorpedoTubeAiPolicies(
            tube_policies,
        ));

        // The shared magazine's grant AI policy (issue #782, AC1): the authored
        // `[torpedoes].ai` block.
        if let Some(ai) = tc.ai.as_ref() {
            cmds.insert(crate::console::weapons::TorpedoMagazineAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
    }
}

fn install_blasters(config: &EntityConfig, cmds: &mut EntityCommands) {
    // Blasters — attach a `BlasterSystemResource` component when the entity
    // TOML has a non-empty `[[weapons_console.blaster_banks]]` list. Mirrors
    // the torpedo insertion above so NPCs and the player ship both participate
    // in the per-entity component model (issue #631 Finding 1).
    if let Some(wc) = &config.weapons_console {
        if !wc.blaster_banks.is_empty() {
            let blaster_systems: Vec<crate::weapons::blaster::BlasterSystem> = wc
                .blaster_banks
                .iter()
                .map(|bc| crate::weapons::blaster::BlasterSystem::new(bc.to_runtime()))
                .collect();
            cmds.insert(crate::console::weapons::BlasterSystemResource(
                blaster_systems,
            ));
            // Per-bank blaster open-fire AI policies (issue #781): each bank's
            // inline authored `ai` block. A bank that authors none contributes no
            // entry. Validated at load, so `to_policy` cannot fail.
            let blaster_bank_policies: std::collections::HashMap<
                String,
                crate::ai::policy::AiPolicy,
            > = wc
                .blaster_banks
                .iter()
                .filter_map(|b| {
                    let ai = b.ai.as_ref()?;
                    Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                })
                .collect();
            cmds.insert(crate::console::weapons::BlasterBankAiPolicies(
                blaster_bank_policies,
            ));
        }
    }
}

fn install_helm(config: &EntityConfig, is_ship: bool, cmds: &mut EntityCommands) {
    if let Some(profile) = &config.helm_control {
        cmds.insert((
            phoenix_sim_gameplay::ship::helm_control::CompiledHelmControl(profile.clone()),
            phoenix_sim_gameplay::ship::helm_control::HelmControlState::new(profile),
        ));
    }
    // HelmConsole - attach a HelmConsoleSection so the AI tick can read movement params.
    // Also insert the four drive-config Components (PR 4 — PRD #597) so NPC ships
    // participate in the per-entity config model alongside the player ship.
    if let Some(hc) = &config.helm_console {
        cmds.insert(HelmConsoleSection(hc.clone()));

        // Physics config
        cmds.insert(crate::ship_plugin::ShipPhysicsConfigResource(
            crate::ship::physics::ShipPhysicsConfig {
                max_speed: hc.max_speed,
                max_reverse_speed: hc.max_reverse_speed,
                acceleration: hc.acceleration,
                deceleration: hc.deceleration,
                max_yaw_rate: hc.max_yaw_rate,
                low_speed_turn_boost: hc.low_speed_turn_boost,
                max_lateral_speed: hc
                    .lateral_thrust
                    .as_ref()
                    .map(|lt| lt.max_lateral_speed)
                    .unwrap_or(15.0),
                lateral_acceleration: hc
                    .lateral_thrust
                    .as_ref()
                    .map(|lt| lt.lateral_acceleration)
                    .unwrap_or(15.0),
                // Vertical axis (issue #744): no dedicated helm_console TOML yet,
                // so take the ShipPhysicsConfig defaults.
                ..crate::ship::physics::ShipPhysicsConfig::new()
            },
        ));
        // Impulse config — steering_multiplier from [helm_capability] when present,
        // falling back to the const default (0.1) when absent.
        let impulse_steering = config
            .helm_capability
            .as_ref()
            .map(|cap| cap.impulse.steering_multiplier)
            .unwrap_or(crate::ship::impulse::IMPULSE_STEERING_MULTIPLIER_DEFAULT);
        cmds.insert(crate::ship_plugin::ImpulseConfigResource {
            charge_duration: hc.impulse_charge_duration,
            speed_multiplier: hc.impulse_speed_multiplier,
            acceleration_multiplier: hc.impulse_acceleration_multiplier,
            engage_distance: hc.impulse_engage_distance,
            cancel_distance: hc.impulse_cancel_distance,
            steering_multiplier: impulse_steering,
        });
        // Boost config (disabled when [helm_console.boost] is absent)
        let boost_cfg = hc
            .boost
            .as_ref()
            .map(|b| crate::ship_plugin::BoostConfigResource {
                enabled: true,
                multiplier: b.multiplier,
                steering_multiplier: b.steering_multiplier,
                active_duration: b.active_duration,
                recharge_duration: b.recharge_duration,
            })
            .unwrap_or_default();
        cmds.insert(boost_cfg);
        // Bank config
        cmds.insert(crate::ship_plugin::BankConfigResource {
            max_bank_deg: hc.max_bank_deg,
            bank_lerp_rate: hc.bank_lerp_rate,
        });
    } else if is_ship {
        cmds.insert((
            crate::ship_plugin::ShipPhysicsConfigResource(
                crate::ship::physics::ShipPhysicsConfig::default(),
            ),
            crate::ship_plugin::ImpulseConfigResource::default(),
            crate::ship_plugin::BoostConfigResource::default(),
            crate::ship_plugin::BankConfigResource::default(),
        ));
    }
}

fn install_shields(config: &EntityConfig, cmds: &mut EntityCommands) {
    // A policy-only console does not grant equipment. Base values or arcs do.
    let shields_content = config
        .shields_console
        .as_ref()
        .filter(|sc| sc.base.is_some() || !config.shield_arcs.is_empty());
    if let Some(sc) = shields_content {
        use crate::weapons::shield::{ShieldFocusConfig, ShieldSystem};
        let ship_wide = sc.base.as_ref().map(|b| b.to_runtime()).unwrap_or_default();
        let shield_system = if !config.shield_arcs.is_empty() {
            let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
            ShieldSystem::from_arcs(&arcs, &ship_wide)
        } else {
            ShieldSystem::new(&ship_wide)
        };
        let freq = config
            .shield_arcs
            .first()
            .map(|a| a.frequency)
            .unwrap_or(sc.frequency);
        let mut shields = crate::ship::shields::ShipShields(shield_system, freq);
        for facing in &mut shields.0.facings {
            facing.protection_fraction = 0.0;
        }
        shields.0.focus_config = ShieldFocusConfig {
            bonus_max_hp: sc.focus_bonus_max_hp,
            bonus_regen: sc.focus_bonus_regen,
            penalty_max_hp: sc.focus_penalty_max_hp,
            penalty_regen: sc.focus_penalty_regen,
            decay_rate: sc.focus_decay_rate,
            focused_damage_multiplier: sc.focus_focused_damage_multiplier,
            unfocused_damage_multiplier: sc.focus_unfocused_damage_multiplier,
        };
        cmds.insert(shields);
        crate::ship::shield_enquiry::install(freq, sc, cmds);
    } else if !config.shield_arcs.is_empty() {
        // Ships that declare `[[shield_arc]]` blocks without a
        // `[shields_console]` block (some legacy paths). Still build the
        // shield system from arcs, using default focus config.
        use crate::weapons::shield::ShieldSystem;
        let ship_wide = crate::weapons::shield::ShieldConfig::default();
        let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
        let mut shield_system = ShieldSystem::from_arcs(&arcs, &ship_wide);
        for facing in &mut shield_system.facings {
            facing.protection_fraction = 0.0;
        }
        let freq = config
            .shield_arcs
            .first()
            .map(|a| a.frequency)
            .unwrap_or(0.5);
        cmds.insert(crate::ship::shields::ShipShields(shield_system, freq));
    }
}
