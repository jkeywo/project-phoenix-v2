//! One Bevy installation path for authored ship capabilities and common ship state.
//! Crew selection and local/fleet identity belong to the GameStart adapter.

use super::config::EntityConfig;
use super::spawner::{EntityShipArcHull, HelmConsoleSection, WeaponsConsoleSection};
use bevy::prelude::*;

/// Resolved ship inputs. No equipment or player/NPC branch belongs in this seed.
pub(crate) struct ShipSpawnSeed {
    pub(crate) ship_config: crate::ship_plugin::ShipConfigComponent,
    pub(crate) control_sources: crate::ship_plugin::ShipSystemControlSources,
    pub(crate) active_ratings: crate::ship_plugin::ActiveStationRatings,
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
            initial_yaw: 0.0,
        }
    }
}

pub(super) fn install(
    config: &EntityConfig,
    position: Vec3,
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
        insert_power_multipliers_and_modifiers(config, build_power_multipliers(config), cmds);
        insert_ship_markers_and_trackers(cmds);
    }
    // These capabilities also belong to independently authored nonship entities.
    install_weapons(config, cmds);
    install_torpedoes(config, cmds);
    install_blasters(config, cmds);
    install_helm(config, is_ship, cmds);
    install_shields(config, cmds);
    install_arc_hull(config, cmds);
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
        initial_yaw,
    } = seed;
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
    cmds.insert(crate::gm_puppet::capability::NpcStationConfig(
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
/// optional inline power-allocation policy from `[power.ai_policy]`.
fn insert_power_state(
    config: &EntityConfig,
    power_group_seed: &[crate::modifiers::power_system::AuthoredPowerGroup],
    cmds: &mut EntityCommands,
) {
    // Per-entity power config (PRD #597 gap-4 closure). NPCs without a
    // `[power]` TOML block get `PowerConfigResource::default()` /
    // `PowerAiConfigResource::default()` so `translate_power_modifiers`,
    // `ai_power_allocation` (issue #693), and `tick_power_system` can
    // iterate every ship uniformly (`With<Ship>`) without an `is_npc`
    // fork. When the TOML supplies `[power]` / `[power.ai]`, those
    // values seed the components.
    let power_config = match &config.power {
        Some(pc) => {
            crate::ship::power::PowerConfigResource(crate::modifiers::power_system::PowerConfig {
                strike_reserve: pc.strike_reserve.clone(),
                capacity: pc.capacity,
                rates: pc.rates,
                sustainable_total: pc.sustainable_total,
                max_commanded_total: pc.max_commanded_total,
                emergency_threshold: pc.emergency_threshold,
            })
        }
        None => crate::ship::power::PowerConfigResource::default(),
    };
    cmds.insert(crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_authored_groups(
            &power_config.0,
            power_group_seed,
        ),
    ));
    cmds.insert(power_config);
    // Inline stateless Power allocation AI policy (issue #784) — from the
    // ship's `[power.ai_policy]` block. Since #885b stage 5d there is no
    // Rust-side synthesiser behind it: strict AI-declaration mode
    // (`AiDeclarationMode::DEFAULT`) rejects an AI-capable hull that omits
    // the block at load, so an AI-bearing entity always authors one and the
    // `None` arm is reached only by a config built in code. Nothing is
    // attached in that case — an undeclared system gets no automation, which
    // is PRD #774 US7's requirement. `to_policy` cannot fail here: the block
    // was validated in `EntityConfig::from_toml`.
    if let Some(ai) = config.power.as_ref().and_then(|pc| pc.ai_policy.as_ref()) {
        cmds.insert((
            crate::ship::power::PowerAiPolicy(ai.to_policy().unwrap_or_default()),
            // Carried from the SAME authored block (issue #889's
            // evaluate_every_ticks, wired at runtime): a resolved
            // `AiPolicy` alone forgets this field, so it rides alongside
            // as a sibling component.
            crate::ship::power::PowerAiCadence(ai.evaluate_every_ticks),
        ));
    }
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
    // Per-ship tuning is independent of whether shield equipment is present.
    cmds.insert(
        config
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai.as_ref())
            .map(|ai| crate::ship::shields::ShieldsAiConfigResource {
                damage_window_secs: ai.damage_window_secs,
                min_damage_window_secs: ai.min_damage_window_secs,
                damage_pct_threshold: ai.damage_pct_threshold,
                health_ratio_threshold: ai.health_ratio_threshold,
                ..Default::default()
            })
            .unwrap_or_default(),
    );
    // Shields focus AI policy (issue #783) — the inline stateless
    // `shield_focus` policy from the authored `[shields_console.ai_policy]`
    // block, so `ai_shield_focus` resolves a data-authored gate + reads the
    // authored windows/thresholds from the policy `param` map rather than the
    // retired `ai_cfg.*` reads. `to_policy` cannot fail here: the block was
    // validated in `EntityConfig::from_toml`.
    if let Some(ai) = config
        .shields_console
        .as_ref()
        .and_then(|sc| sc.ai_policy.as_ref())
    {
        cmds.insert(crate::ship::shields::ShieldsFocusAiPolicy(
            ai.to_policy().unwrap_or_default(),
        ));
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
        for (block, system_id) in [
            (hc.engines_ai.as_ref(), sr::helm_thrust_system_id()),
            (hc.steering_ai.as_ref(), sr::helm_steering_system_id()),
            (hc.lateral_ai.as_ref(), sr::lateral_thrust_system_id()),
            (hc.vertical_ai.as_ref(), sr::vertical_thrust_system_id()),
            (hc.impulse_ai.as_ref(), sr::helm_impulse_system_id()),
            (hc.boost_ai.as_ref(), sr::helm_boost_system_id()),
        ] {
            if let Some(ai) = block {
                fine_policies.insert(system_id, ai.to_policy().unwrap_or_default());
            }
        }
        cmds.insert(crate::ship::helm_ai::FineSystemAiPolicies(fine_policies));
    }
}

/// Inserts the [`PowerMultiplierResource`] built by [`build_power_multipliers`],
/// the empty per-entity [`ShipModifiers`] cache, and — only when the config
/// declares repair TEAMS — the [`ShipRepairTeams`].
fn insert_power_multipliers_and_modifiers(
    config: &EntityConfig,
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
            cmds.insert(crate::console::repair::server::ShipRepairTeams(
                crate::modifiers::repair_teams::RepairTeams::new_with_timings(
                    repair_cfg.repair_team_count as usize,
                    timings,
                ),
            ));
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
    } else if !config.shield_arcs.is_empty() {
        // Ships that declare `[[shield_arc]]` blocks without a
        // `[shields_console]` block (some legacy paths). Still build the
        // shield system from arcs, using default focus config.
        use crate::weapons::shield::ShieldSystem;
        let ship_wide = crate::weapons::shield::ShieldConfig::default();
        let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
        let shield_system = ShieldSystem::from_arcs(&arcs, &ship_wide);
        let freq = config
            .shield_arcs
            .first()
            .map(|a| a.frequency)
            .unwrap_or(0.5);
        cmds.insert(crate::ship::shields::ShipShields(shield_system, freq));
    }
}

fn install_arc_hull(config: &EntityConfig, cmds: &mut EntityCommands) {
    // Per-arc hull HP (issue #514) — populated from `[[shield_arc]].hull_max_hp`
    // and companion threshold/debuff fields. Attaches `EntityShipArcHull`
    // alongside the shield system so `sync_console_damage_tiers` can route arc
    // damage → offline_systems per-arc. Skipped when no arc declares hull HP.
    if !config.shield_arcs.is_empty() {
        let arc_entries: Vec<(String, crate::ship::damage::ArcHullEntry)> = config
            .shield_arcs
            .iter()
            .filter(|a| a.hull_max_hp > 0.0)
            .map(|a| {
                (
                    a.id.clone(),
                    crate::ship::damage::ArcHullEntry {
                        current: a.hull_max_hp,
                        max: a.hull_max_hp,
                        tier_config: crate::ship::damage::ConsoleTierConfig {
                            damaged_threshold_pct: a.hull_damaged_threshold_pct,
                            disabled_threshold_pct: a.hull_disabled_threshold_pct,
                            debuff_magnitude: a.hull_debuff_magnitude,
                        },
                    },
                )
            })
            .collect();
        if !arc_entries.is_empty() {
            cmds.insert(EntityShipArcHull(
                crate::ship::damage::ShipArcHull::from_entries(arc_entries),
            ));
        }
    }
}
