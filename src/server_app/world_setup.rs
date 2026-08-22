//! World setup and world-entity construction (issue #1199).
//!
//! Public surface: the Startup/OnEnter spawn systems `setup_world`,
//! `spawn_game_start_entities`, `dump_tracked_entities`; the extracted
//! `spawn_anonymous_entities_internal`; the world-entity snapshot helpers
//! `upsert_world_entity` / `snapshot_from_entity_config`; and the player-ship
//! identity helpers (`player_hull_config`, `player_ship_identity`,
//! `player_spawn_rotation_yaw`). Re-exported through `crate::server_app`.
//!
//! Role: brings the authored world into being — the anonymous immediate
//! `[[entity]]` instances at Startup and the `GameStart` player ship (with its
//! full lobby-selected loadout) when the game enters `InProgress`.
//!
//! Load-bearing invariant: this is one half of the immediate spawn; the atomic
//! activation gate (`world_activation_blocked`) and the `setup_world` ordering
//! edges in registration keep it agreeing with `world::server::spawn_world_entities`
//! on which entries it owns and on the shared `WorldIdMint` order — the entity
//! mint order feeds the authoritative digest, so the spawn sequence is fixed.

use super::*;

/// Reconciles the live ECS entities with the `TrackedEntities` registry each tick.
pub(crate) fn upsert_world_entity(world: &mut WorldResource, snapshot: EntitySnapshot) {
    if let Some(existing) = world
        .0
        .entities
        .iter_mut()
        .find(|e| e.uuid == snapshot.uuid)
    {
        *existing = snapshot;
    } else {
        world.0.entities.push(snapshot);
    }
}

pub(crate) fn snapshot_from_entity_config(
    uuid: String,
    id: Option<String>,
    config: &crate::entities::config::EntityConfig,
    position: Vec3,
) -> EntitySnapshot {
    let mut snapshot = EntitySnapshot {
        uuid,
        id,
        // A ship's crew-facing PROPER NAME (issue: player-facing ship names)
        // when it authors one; otherwise `name`, which for a world instance is
        // the instance name. The proper name is a property of the hull and is
        // never overwritten by the spawn, so it survives the `name`-override a
        // world `[[entity]] name` performs for trigger targeting.
        name: config.display_name.clone().or_else(|| config.name.clone()),
        position: Some([position.x, position.y, position.z]),
        tags: config.tags.clone(),
        ..EntitySnapshot::default()
    };

    if let Some(radar) = &config.radar_appearance {
        if let Some(colour) = &radar.colour {
            if colour.len() >= 3 {
                snapshot.colour = Some([colour[0], colour[1], colour[2]]);
            }
        }
        if let Some(region_colour) = &radar.region_colour {
            if region_colour.len() >= 3 {
                snapshot.region_colour =
                    Some([region_colour[0], region_colour[1], region_colour[2]]);
            }
        }
        snapshot.radar_size = radar.size;
        snapshot.radar_icon = radar.icon.clone();
    }

    if let Some(collider) = &config.collider {
        if snapshot.radius.is_none() {
            snapshot.radius = Some(collider.radius);
        }
    }

    // Infrastructure condition + capacity (issue #1025). Built from the
    // authored table because this path mints the snapshot from the config,
    // before the entity exists — which is also the only moment its condition is
    // guaranteed to be its authored starting value.
    if let Some(infrastructure) = &config.infrastructure {
        snapshot.infrastructure = crate::core::messages::InfrastructureSnapshot::from_state(
            &crate::infrastructure::InfrastructureState::from_config(infrastructure),
        );
    }

    if let Some(target) = &config.target {
        snapshot.target_tags = target.tags.clone();
        snapshot.threat_level = Some(target.threat_level.as_str().to_string());
        snapshot.target_description = target.description.clone();
    }

    // Initial shield fraction (#471). When the entity has a `[shields]`
    // block, seed the snapshot at full HP. Per-tick updates flow through
    // `EntityStateSnapshot.shield_fraction` from `sim_state_broadcaster`.
    if config.shields_console.is_some() {
        snapshot.shield_fraction = Some(1.0);
    }

    snapshot
}

// ── World Setup ────────────────────────────────────────────────────────────
//
// Per PRD #341, asteroid-field entries and named `[[entity]]` instances are
// owned by `world::server::spawn_world_entities`. This `setup_world` system
// covers only:
//   * spawning *anonymous* immediate `[[entity]]` instances (e.g. stars,
//     planets) that aren't asteroid fields and don't carry a `name`.
//
// When no `WorldConfig` is loaded (native unit tests only — production
// always loads a world TOML via the WASM bridge) this is a no-op.
pub(crate) fn setup_world(
    mut commands: Commands,
    mut world: ResMut<WorldResource>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    let config_cache = crate::entities::config_cache::get_config_cache();
    spawn_anonymous_entities_internal(
        &mut commands,
        &mut world,
        &world_config,
        &config_cache,
        id_mint.as_deref(),
    );
}

/// Spawn the `setup_world`-owned anonymous immediate `[[entity]]` instances.
///
/// Returns the number spawned. Extracted from `setup_world` for the same reason
/// [`crate::world::server::spawn_immediate_entities_internal`] was extracted
/// from `spawn_world_entities`: the spawn logic is then testable on native with
/// a fixture `ConfigCache`, instead of depending on the process-global native
/// cache that `insert_native_config` warns is unsafe to touch from a unit test.
pub(crate) fn spawn_anonymous_entities_internal(
    commands: &mut Commands,
    world: &mut WorldResource,
    world_config: &crate::world::config::WorldConfig,
    config_cache: &crate::entities::config_cache::ConfigCache,
    id_mint: Option<&crate::world_id::WorldIdMint>,
) -> usize {
    // Atomic-activation guard (issues #750/#752/#906/#969/#973). This system owns one
    // half of the immediate spawn; `spawn_world_entities` owns the other, and
    // the two carry no ordering relationship. The loop below answers a failed
    // resolve by logging and `continue`ing, so without this gate an invalid
    // world still ships its stars, planets and nebulae while every named entity
    // and asteroid field silently vanishes — a *more* partial failure than the
    // single missing entity the gate exists to prevent, and a direct
    // contradiction of `world-content-lifecycle-state`. Same function, same
    // parsed config, so both halves always agree.
    if crate::world::server::world_activation_blocked(world_config, config_cache, "setup_world") {
        return 0;
    }

    // Pre-resolve named-entity positions so anonymous entries using
    // `relative_to` can be positioned (PRD #337).
    let named_positions = crate::world::config::build_named_entity_positions(world_config);
    let mut spawned = 0;

    for entity_inst in &world_config.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::Immediate {
            continue;
        }
        // Asteroid-field entries and named entries are owned by the unified
        // spawn pass in `world::server::spawn_world_entities`. Skip them to
        // avoid double-spawning.
        // Same lookup the unified half routes with, and the same one the spawn
        // below performs (issue #973 review): cache, then the host loader.
        // These two ownership predicates must never disagree — one that is
        // narrower than the spawn double-spawns or mis-routes an entry rather
        // than dropping it. See `entity_loader::template_is_asteroid_field`.
        let is_unified = crate::world::config::is_owned_by_unified_pipeline(entity_inst, |path| {
            crate::entities::loader::template_is_asteroid_field(
                path,
                config_cache,
                &crate::entities::loader::WasmTemplateLoader,
            )
        });
        if is_unified {
            continue;
        }

        let config = match crate::entities::loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entities::loader::WasmTemplateLoader,
        ) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "setup_world: failed to resolve entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        let uuid = crate::world_id::mint_id_with(id_mint, crate::world_id::IdNamespace::Entity);
        let pos = match crate::world::config::resolve_entity_position_with(
            entity_inst,
            &world_config.anchors,
            &named_positions,
        ) {
            Ok(p) => Vec3::new(p[0], p[1], p[2]),
            Err(e) => {
                bevy::log::error!("setup_world: {e}");
                continue;
            }
        };

        crate::entities::spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid.clone(),
            entity_inst.id.clone(),
        );
        upsert_world_entity(
            world,
            snapshot_from_entity_config(uuid, entity_inst.id.clone(), &config, pos),
        );
        spawned += 1;
    }

    spawned
}

pub(crate) fn player_spawn_rotation_yaw(rot: [f32; 3]) -> (bevy::math::Quat, f32) {
    let q = bevy::math::Quat::from_euler(bevy::math::EulerRot::YXZ, rot[1], rot[0], rot[2]);
    let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
    (q, yaw)
}

/// Compute the player ship's identity — the `player` tag and the `playerShip`
/// radar icon — to inject at the player game-start spawn.
///
/// This identity is deliberately NOT authored in the hull templates. If it
/// were, every world-spawned copy of the same hull (which spawns as an NPC)
/// would masquerade as the player: it would answer `player`-only radar filters
/// and draw with the player blip. Injecting here scopes the identity to the one
/// hull the local player actually flies.
///
/// Returns `(tags, radar)`: the template tags with `player` appended (keeping
/// `ship`, which player-ship selection keys off), and the template's radar
/// appearance with its icon forced to `playerShip` (colour/size preserved).
/// The caller re-inserts these onto the spawned entity; Bevy `insert` replaces,
/// so this overwrites the ordinary-ship sections `spawn_entity` set from the
/// template.
pub(crate) fn player_ship_identity(
    template_tags: &[String],
    template_radar: Option<&crate::entities::config::RadarAppearanceConfig>,
) -> (Vec<String>, crate::entities::config::RadarAppearanceConfig) {
    let mut tags = template_tags.to_vec();
    let player_tag = crate::entities::tags::EntityTag::Player.as_str();
    if !tags.iter().any(|t| t == player_tag) {
        tags.push(player_tag.to_string());
    }
    let mut radar =
        template_radar
            .cloned()
            .unwrap_or(crate::entities::config::RadarAppearanceConfig {
                icon: None,
                colour: None,
                size: None,
                region_colour: None,
            });
    radar.icon = Some(crate::entities::config::PLAYER_SHIP_RADAR_ICON.to_string());
    (tags, radar)
}

/// The config the player's game-start hull actually spawns with: the
/// **lobby-selected** hull, carrying the **world's own** `[[entity]]`
/// overrides.
///
/// # The two authorities, and why both have to survive
///
/// A world's `player-ship` row names two different things at once, and before
/// this function only the first survived:
///
/// * **Which hull.** The row's `template_path` is a placeholder — the lobby
///   picks the hull, and a player who selected the Destroyer must not spawn the
///   placeholder's weapons. `selected` wins that argument outright.
/// * **How THIS mission tunes it.** `[entity.overrides.*]` on that same row is
///   the world's per-instance intent — switch off the Comms AI for one mission,
///   nudge a doctrine priority, widen a radar. `resolve_entity_via` merged those
///   onto the row's template and the composition validator checked the result;
///   then the wholesale `selected.clone()` threw the merged document away and
///   every world's player-ship override became decorative. Found while
///   reviewing #1036: `probe_evidence.toml`'s `comms_console.ai.rule` override
///   was never the rule the run flew.
///
/// So the overrides are re-applied **onto the picked hull**, through
/// [`crate::entities::loader::apply_overrides`] — the same merge the validator
/// itself runs (`world::validate`) and the same one `resolve_entity_via`
/// performs for every other `[[entity]]`. One merge, one set of semantics; a
/// second implementation here would be a second set of answers about
/// `behaviour.doctrine` keying, `tags` replacing and the `_remove` tombstone.
///
/// # Semantics
///
/// * An override expresses the WORLD's intent, not the placeholder hull's, so it
///   applies to **whichever hull the lobby picked**. A world that tunes its
///   player ship tunes the ship the crew actually fly.
/// * An override naming a table the picked hull's template lacks follows the
///   **existing absent-table semantics**: the merge inserts it, and whether it
///   does anything is up to the system that reads it — a `[comms_console]`
///   block on a hull with no Comms system is silently inert. That is unchanged
///   here on purpose; making it loud is a separate task.
/// * A world with **no** overrides on that row (`overrides == None`) gets
///   `selected.clone()` — byte-for-byte the pre-fix path, which is what keeps
///   every shipped world spawning identically.
/// * A row that resolved without a lobby selection at all keeps `world_row`,
///   which `resolve_entity_via` has already merged.
///
/// # A merge failure spawns the ship anyway
///
/// The validator checked these overrides against the ROW's template, not against
/// the hull the lobby went on to pick, so a merge that was valid at validation
/// time can still fail here (an override reshaping a table the picked hull
/// declares differently). The ship is the one entity the session cannot do
/// without, so this logs and falls back to the unmodified selection — today's
/// behaviour — rather than dropping the player's hull.
pub(crate) fn player_hull_config(
    world_row: crate::entities::config::EntityConfig,
    overrides: Option<&toml::Value>,
    selected: Option<&crate::entities::config::EntityConfig>,
) -> crate::entities::config::EntityConfig {
    let Some(selected) = selected else {
        return world_row;
    };
    let Some(overrides) = overrides else {
        return selected.clone();
    };
    match crate::entities::loader::apply_overrides(selected, overrides) {
        Ok(merged) => merged,
        Err(e) => {
            bevy::log::error!(
                "player ship: the world's `player-ship` overrides do not merge onto the \
                 lobby-selected hull, spawning it untuned: {e}"
            );
            selected.clone()
        }
    }
}

/// Spawn entities with `spawn_on = GameStart` (e.g. player ship) when the
/// game transitions to InProgress. Registered in `OnEnter(GamePhase::InProgress)`.
pub(crate) fn spawn_game_start_entities(
    mut commands: Commands,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending_ship_config: Option<ResMut<crate::ship_plugin::PendingShipConfig>>,
    selected_ship: Option<Res<crate::lobby::SelectedShipResource>>,
    mut sessions: Option<ResMut<crate::lobby::Sessions>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut has_spawned: Local<bool>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
) {
    if *has_spawned {
        return;
    }

    let mc = match world_config.as_deref() {
        Some(mc) => mc,
        None => return,
    };

    let config_cache = crate::entities::config_cache::get_config_cache();

    let mut ship_spawned = false;
    let named_positions = crate::world::config::build_named_entity_positions(mc);
    for entity_inst in &mc.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
            continue;
        }
        // Evaluate optional spawn predicate against the world flag store.
        if let Some(pred) = &entity_inst.when_predicate {
            let empty = crate::world::flags::FlagStore::new();
            let flags_ref = runtime.as_ref().map(|r| &r.flags).unwrap_or(&empty);
            if !pred.evaluate(&[flags_ref]) {
                continue;
            }
        }
        let config = match crate::entities::loader::resolve_entity_via(
            entity_inst,
            &config_cache,
            &crate::entities::loader::WasmTemplateLoader,
        ) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "Failed to resolve GameStart entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        // The player ship's full loadout (weapons, torpedoes, blasters, shields,
        // mesh, stations) must come from the lobby-selected ship template, not
        // the world's `[[entity]] player-ship` placeholder. The placeholder only
        // fixes spawn position; without this override a player who selects the
        // Destroyer still spawns the placeholder hull's weapons (e.g. the
        // cruiser's two phaser banks and no blasters). ShipConfigComponent is
        // already sourced from the selection (PendingShipConfig); this brings the
        // EntityConfig-derived systems into agreement. Matched on the same
        // predicate used below for the player-ship position/rotation/marker.
        //
        // The row's `[entity.overrides.*]` ride ALONG with the selection rather
        // than being discarded by it — see `player_hull_config` for why the
        // world's per-instance tuning outlives the hull swap, and for what a
        // world without overrides is guaranteed (nothing changes at all).
        let config = if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            player_hull_config(
                config,
                entity_inst.overrides.as_ref(),
                selected_ship
                    .as_ref()
                    .and_then(|sel| config_cache.get(&sel.0)),
            )
        } else {
            config
        };

        let uuid =
            crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);
        let pos = match crate::world::config::resolve_entity_position_with(
            entity_inst,
            &mc.anchors,
            &named_positions,
        ) {
            Ok(p) => Vec3::new(p[0], p[1], p[2]),
            Err(e) => {
                bevy::log::error!(
                    "Failed to resolve GameStart entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        // Override with player_spawn position when spawning the player ship
        // (issue #623).
        let pos = if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            if let Some(ref spawn) = mc.player_spawn {
                if let Some(ref anchor_name) = spawn.anchor {
                    match mc.anchors.get(anchor_name) {
                        Some(a) => Vec3::new(a[0], a[1], a[2]),
                        None => {
                            bevy::log::error!("player_spawn anchor '{}' not found", anchor_name);
                            pos
                        }
                    }
                } else if let Some(p) = spawn.position {
                    Vec3::new(p[0], p[1], p[2])
                } else {
                    pos
                }
            } else {
                pos
            }
        } else {
            pos
        };

        // Override with player_spawn rotation when spawning the player ship (issue #623).
        let player_spawn_rot: Option<bevy::math::Quat> =
            if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
                mc.player_spawn.as_ref().and_then(|s| s.rotation).map(|r| {
                    let (q, _) = player_spawn_rotation_yaw(r);
                    q
                })
            } else {
                None
            };

        let spawned = crate::entities::spawner::spawn_entity(
            &mut commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );

        // Apply rotation on the spawned entity's Transform
        if let Some(q) = player_spawn_rot {
            commands
                .entity(spawned)
                .insert(bevy::prelude::Transform::from_translation(pos).with_rotation(q));
        }

        // Extract yaw for ShipPhysicsComponent
        let initial_yaw = player_spawn_rot
            .map(|q| {
                let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
                yaw
            })
            .unwrap_or(0.0);

        // The first GameStart entity tagged "ship" gets the Ship marker and its
        // full lobby-selected loadout. Issue #1200 decomposes the ~820-line
        // configuration into per-concern builders under `configure_player_ship`;
        // the sequence and set of `.insert()` / `insert_resource` /
        // `remove_resource` calls is byte-for-byte the one that shipped inline,
        // which the archetype-order guard gates.
        if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            configure_player_ship(
                &mut commands,
                spawned,
                &config,
                pos,
                initial_yaw,
                &mut pending_ship_config,
                &mut sessions,
            );
            ship_spawned = true;
        }
    }

    *has_spawned = true;
}

/// Configure the one GameStart player ship: resolve its lobby-selected hull
/// config, seed the boot ratings and the authored power-group reactor seed,
/// then run the per-concern builders below in the EXACT order their inserts
/// previously ran inline. Component-insertion order is archetype-creation
/// order, which the authoritative digest is sensitive to, so this split is
/// pure code motion (issue #1200).
fn configure_player_ship(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
    pos: Vec3,
    initial_yaw: f32,
    pending_ship_config: &mut Option<ResMut<crate::ship_plugin::PendingShipConfig>>,
    sessions: &mut Option<ResMut<crate::lobby::Sessions>>,
) {
    let ship_config = if let Some(pending) = pending_ship_config.as_mut() {
        let cfg = crate::ship_plugin::ShipConfigComponent(pending.0.clone());
        commands.remove_resource::<crate::ship_plugin::PendingShipConfig>();
        *pending_ship_config = None;
        cfg
    } else {
        crate::ship_plugin::load_ship_config_from_disk()
    };
    // Seed the reactor from the player ship's authored power groups
    // (issue #762) before `ship_config` is moved into the entity, so
    // authored groups beyond the canonical three (e.g. `ops`) are
    // allocatable. Empty for a config with no `[power_groups.*]`.
    let power_group_seed =
        crate::ship::power::authored_power_group_seed(&ship_config.0.power_groups);
    let (initial_control_sources, initial_active_ratings) = {
        // The shared boot-seeding path (issue #871) — the same
        // `seed_boot_ratings` `entities::spawner` calls for every other
        // hull. Only the per-station rating CHOICE differs here: this
        // path knows about lobby sessions, so a manned station boots on
        // the player's chosen complexity toggle instead of Backfill.
        match sessions.as_ref() {
            Some(sess) => {
                let manned: std::collections::HashSet<_> = sess
                    .0
                    .players()
                    .iter()
                    .filter(|p| p.connected)
                    .filter_map(|p| p.station.as_ref())
                    .collect();
                let (resolver, active_ratings) =
                    crate::ship::rating::seed_boot_ratings(&ship_config.0, |station| {
                        // Manned stations apply the player's
                        // lobby-chosen complexity toggle (if any), else
                        // the station's base (first) rating. Unmanned
                        // stations are fully AI-backfilled, as before.
                        if manned.contains(&station.id) {
                            sess.0
                                .pending_rating_for(&station.id)
                                .cloned()
                                .or_else(|| station.ratings.first().map(|r| r.name.clone()))
                                .unwrap_or_else(|| "Std".to_string())
                        } else {
                            crate::ship::rating::BACKFILL_RATING.to_string()
                        }
                    });
                (
                    crate::ship_plugin::ShipSystemControlSources(resolver),
                    crate::ship_plugin::ActiveStationRatings(active_ratings),
                )
            }
            // No lobby at all: leave both empty, exactly as before.
            None => (
                crate::ship_plugin::ShipSystemControlSources::default(),
                crate::ship_plugin::ActiveStationRatings::default(),
            ),
        }
    };
    if let Some(sess) = sessions.as_mut() {
        sess.0.clear_all_pending_ratings();
    }

    insert_player_core_bundle(
        commands,
        spawned,
        ship_config,
        initial_control_sources,
        initial_active_ratings,
        &power_group_seed,
        pos,
        initial_yaw,
    );
    insert_player_identity(commands, spawned, config);
    insert_player_repair_teams(commands, spawned, config);
    insert_player_shields(commands, spawned, config);
    insert_player_console_selectors(commands, spawned, config);
    insert_player_shields_damage_and_arc_hull(commands, spawned, config);
    insert_player_weapons_console(commands, spawned, config);
    insert_player_torpedoes(commands, spawned, config);
    insert_player_power_state(commands, spawned, config, &power_group_seed);
    insert_player_helm_configs(commands, spawned, config);
}

/// The player ship's marker + high-fidelity core component bundle: the `Ship`/
/// `LocalShip` markers, AI high-fidelity components, blackboards, the resolved
/// ship config, control sources and active ratings, physics, and the full set
/// of per-ship scratch/state components (issue #1200).
fn insert_player_core_bundle(
    commands: &mut Commands,
    spawned: Entity,
    ship_config: crate::ship_plugin::ShipConfigComponent,
    initial_control_sources: crate::ship_plugin::ShipSystemControlSources,
    initial_active_ratings: crate::ship_plugin::ActiveStationRatings,
    power_group_seed: &[(crate::core::messages::PowerGroupId, u8)],
    pos: Vec3,
    initial_yaw: f32,
) {
    commands
        .entity(spawned)
        .insert(Ship)
        .insert(LocalShip)
        // The player ship is permanently high-fidelity (`lod_ai_ships`
        // never evaluates `LocalShip`), so it takes the marker and the
        // components that travel with it from the SAME shared
        // definition the NPC promotion path uses. Spelling the set out
        // here again is how #785's RepairTargetSelector, #786's
        // CommsTargetSelector and #882's HelmBoostAiPolicyState each
        // silently missed the player ship.
        .insert(crate::ai::server::ai_high_fidelity_components())
        .insert(ShipSystemBlackboards::default())
        .insert(ship_config)
        .insert(initial_control_sources)
        .insert(initial_active_ratings)
        .insert(crate::ship_plugin::CoordinationQueue::default())
        .insert(crate::ship_plugin::PendingArcBearingRequest::default())
        .insert(crate::ship_plugin::DockingMotionIntent::default())
        .insert(crate::ship::shields::PendingShieldsThreatBearing::default())
        // Sensors→Tactical frequency advisory a backfilled Tactical
        // consumes off the channel-3 bus (issue #873).
        .insert(crate::ship_plugin::PendingTacticalFrequencyHint::default())
        // Per-ship intent-narration memory (issue #879). The player
        // ship is the one bridge with human seats to narrate TO, so
        // omitting it here — the failure mode #785/#786/#882/#885 each
        // shipped — would leave the whole feature dead on the only
        // hull it exists for.
        .insert(crate::ship_plugin::ShipIntentNarration::default())
        .insert(crate::core::messages::AdmittedCommands::default())
        .insert(ShipPhysicsComponent {
            x: pos.x,
            z: pos.z,
            yaw: initial_yaw,
            ..Default::default()
        })
        // Channel-3 Navigation→Helm clearance latch (issue #702).
        .insert(crate::ship_plugin::HelmWaypointClearance::default())
        // Per-objective route cursors. The player ship was missing
        // these — `entities/spawner.rs` inserted them for NPCs and this
        // path did not — which silently disabled AI patrol on the
        // player ship whenever an unmanned Helm backfilled to AI: with
        // no cursor component, `helm_patrol` had no route position to
        // steer from and `advance_objective_cursors` had nothing to
        // advance (issue #702).
        .insert(crate::ai::server::ObjectiveCursors::default())
        .insert(crate::console::weapons::TacticalRadarSelection::default())
        .insert(crate::console::weapons::ActiveBeam::default())
        .insert(crate::console::weapons::PhaserCooldown::default())
        .insert(crate::console::weapons::WeaponsArcRequestState::default())
        .insert(crate::ship::sensors::SensorRadarSelection::default())
        .insert(crate::ship::state::ShipRedAlert::default())
        .insert(crate::ship::state::ShipViewMode::default())
        .insert(crate::ship::state::ShipPhaserFrequency::default())
        .insert(crate::console::navigation::NavigationWaypoint::default())
        .insert(crate::ship::power::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_authored_groups(
                &crate::modifiers::power_system::PowerConfig::default(),
                power_group_seed,
            ),
        ))
        .insert(crate::ship_plugin::LastHelmInput::default())
        // Per-ship impulse drive state (audit follow-up). Every
        // ship carries its own; NPC ships get one via the spawner
        // too (both idle by default).
        .insert(crate::server_app::ShipImpulse::default())
        // Per-ship boost drive battery (audit follow-up). Every
        // ship carries its own; NPC ships get one via the spawner
        // (both empty by default).
        .insert(crate::server_app::ShipBoost::default())
        // Per-ship coordination bus state (audit follow-up). See
        // `entities/spawner.rs` for details.
        .insert(crate::ship::shields::ShieldsCoordinationState::default())
        .insert(crate::ship::sensors::SensorsFrequencyState::default())
        .insert(crate::ship::sensors::SensorsThreatState::default())
        .insert(crate::ship::power::PowerBrownoutState::default())
        // Per-entity CollisionCooldown so player and NPC ships each
        // have their own cooldown timer (PRD #597 PR-8).
        .insert(CollisionCooldown::default())
        // ShipModifiers as per-entity component (PR 6 — PRD #597; the
        // legacy Resource fallback was removed in issue #606). Every
        // ship — player and NPC — carries its own instance.
        .insert(crate::modifiers::ShipModifiers::new())
        // Combat activity state per-ship (PR 10 — PRD #597). Every
        // ship (player + NPC) tracks its own recent combat activity
        // + this-tick weapon-fired / attacked / last-attacker markers.
        .insert(crate::ship::combat_activity::RecentCombatActivity::default())
        .insert(WeaponFiredThisTick::default())
        .insert(ShipAttackedThisTick::default())
        .insert(crate::console::weapons::LastShipAttacker::default());
}

/// Inject the player identity (`player` tag + `playerShip` radar icon) onto the
/// one ship the local player flies, overwriting the template's ordinary sections.
fn insert_player_identity(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // Inject player identity (the `player` tag + `playerShip` radar
    // icon) HERE, on the one ship the local player flies — not in the
    // hull template. The templates author only ordinary-ship identity
    // so that NPC copies of the same hull spawned into the world do not
    // masquerade as the player. `spawn_entity` already inserted the
    // template's ordinary `EntityTagsSection` / `RadarAppearanceSection`;
    // Bevy `insert` replaces, so re-inserting overwrites them. These
    // components feed the snapshot builders, so the injected tag/icon
    // reach clients (and the native radar's player dedup) before the
    // first broadcast.
    let (player_tags, player_radar) =
        player_ship_identity(&config.tags, config.radar_appearance.as_ref());
    commands
        .entity(spawned)
        .insert(EntityTagsSection(player_tags))
        .insert(RadarAppearanceSection(player_radar));
}

/// The player ship's repair teams, built from the `[repair]` block (issue #1200).
fn insert_player_repair_teams(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // The player ship's hull lives on its `EntitySystemHull`
    // component (PRD #581). All damage/repair paths write there
    // directly; the old `ShipHullIntegrity` resource was retired
    // in PRD #597 PR 10.
    // Ship-specific resource setup
    if let Some(hc) = &config.hull {
        let _hc = hc; // hull is set up via EntitySystemHull in the spawner
                      // [repair] block — overrides default RepairTimings if present.
                      // Absent block keeps the same defaults the hardcoded constants
                      // used to provide (5.0s travel, 0.5 HP/s repair rate).
        let repair = config.repair.as_ref();
        let team_count = repair
            .map(|rc| rc.repair_team_count as usize)
            .filter(|&n| n > 0)
            .unwrap_or(2);
        let timings = repair.map(|rc| rc.to_runtime()).unwrap_or_default();
        let teams = ShipRepairTeams(
            crate::modifiers::repair_teams::RepairTeams::new_with_timings(team_count, timings),
        );
        // Per-entity component only (issue #830 retired the global Resource).
        commands.entity(spawned).insert(teams);
    }
}

/// The player ship's shields: the arc/shield system from `[shields_console]` /
/// `[[shield_arc]]`, the shields AI config (component + dual-written resource),
/// and the optional focus AI policy.
fn insert_player_shields(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // Apply shield focus config + base shield-system values from TOML if present.
    // Post-#514: the `[shields_console.base]` sub-block still holds
    // ship-wide defaults (max_hp, regen_per_sec, offline_duration)
    // consumed as fallbacks by each `[[shield_arc]]` block. When
    // shield_arcs are declared the runtime is built via
    // `ShieldSystem::from_arcs`; otherwise fall back to
    // `ShieldSystem::new` with historical evenly-spaced facings.
    if let Some(sc) = &config.shields_console {
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
        let mut shields = ShipShields(shield_system, freq);
        shields.0.focus_config = crate::weapons::shield::ShieldFocusConfig {
            bonus_max_hp: sc.focus_bonus_max_hp,
            bonus_regen: sc.focus_bonus_regen,
            penalty_max_hp: sc.focus_penalty_max_hp,
            penalty_regen: sc.focus_penalty_regen,
            decay_rate: sc.focus_decay_rate,
            focused_damage_multiplier: sc.focus_focused_damage_multiplier,
            unfocused_damage_multiplier: sc.focus_unfocused_damage_multiplier,
        };
        commands.entity(spawned).insert(shields);
    } else if !config.shield_arcs.is_empty() {
        let ship_wide = crate::weapons::shield::ShieldConfig::default();
        let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
        let freq = config
            .shield_arcs
            .first()
            .map(|a| a.frequency)
            .unwrap_or(0.5);
        commands.entity(spawned).insert(ShipShields(
            ShieldSystem::from_arcs(&arcs, &ship_wide),
            freq,
        ));
    } else {
        // Default shields on the ship entity when no TOML shields_console block.
        commands
            .entity(spawned)
            .insert(ShipShields(ShieldSystem::default(), 0.5));
    }

    // Shields AI config — loaded from [shields_console.ai] if present,
    // otherwise falls back to ShieldsAiConfigResource defaults. The
    // per-entity Component is what `operate_shields_ai` and
    // `emit_shields_coordination` read; the global Resource is a
    // dual-write with no remaining readers (issue #738).
    let ai_cfg = config
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
        .unwrap_or_default();
    commands.entity(spawned).insert(ai_cfg.clone());
    commands.insert_resource(ai_cfg);

    // Shields focus AI policy (issue #783) — player-ship half of the
    // per-entity pattern in `spawner.rs`. The authored
    // `[shields_console.ai_policy]` block drives `ai_shield_focus`'s gate
    // and supplies the authored windows/thresholds. Since #885b stage 5d
    // there is no Rust-side synthesiser behind it: strict AI-declaration
    // mode rejects an AI-capable hull that omits the block at load, so an
    // unauthored policy means no component and no automation rather than
    // one invented in Rust (PRD #774 US7). Validated already in
    // `EntityConfig::from_toml`.
    if let Some(ai) = config
        .shields_console
        .as_ref()
        .and_then(|sc| sc.ai_policy.as_ref())
    {
        commands
            .entity(spawned)
            .insert(crate::ship::shields::ShieldsFocusAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
    }
}

/// The player ship's console AI selectors and policies: sensors AI config +
/// selector, tactical / navigation selectors, comms hail selector + response
/// policy, repair selector, captain AI policy, and the helm fine-system map.
fn insert_player_console_selectors(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // Sensors AI config — the player-ship half of the same per-entity
    // pattern (issue #738 follow-up). `tick_frequency_hint_high_fidelity` reads only the
    // Component, and the spawner attaches one to every entity with a
    // `[behaviour]` block; without this, `[sensors_console.ai]` authored
    // on a player-class ship was silently ignored end to end. Behaviour-
    // neutral for every ship TOML in `assets/` today: none declares the
    // section, and the fallback the reader already used is this same
    // parse-time default.
    commands.entity(spawned).insert(
        config
            .sensors_console
            .as_ref()
            .and_then(|sc| sc.ai.as_ref())
            .map(|ai| crate::ship::sensors::SensorsAiConfigResource {
                frequency_hint_delay_secs: ai.frequency_hint_delay_secs,
            })
            .unwrap_or_default(),
    );

    // Sensors target selector (issue #776) — the player-ship half of the
    // per-entity pattern. The authored `[sensors_console.selector]` block
    // drives `operate_sensors_ai`'s ranking under Backfill. No
    // synthesised stand-in since #885b stage 5d. Validated already in
    // `EntityConfig::from_toml`.
    if let Some(s) = config
        .sensors_console
        .as_ref()
        .and_then(|sc| sc.selector.as_ref())
    {
        commands
            .entity(spawned)
            .insert(crate::ship::sensors::SensorsTargetSelector {
                selector: s.to_selector().unwrap_or_default(),
                power_rating: config.power_rating.map(|r| r as f32),
            });
    }

    // Tactical target selector (issue #777) — the player-ship half of
    // the per-entity pattern. The authored `[weapons_console.selector]`
    // block drives `ai_target_selection`'s ranking under Backfill.
    if let Some(s) = config
        .weapons_console
        .as_ref()
        .and_then(|wc| wc.selector.as_ref())
    {
        commands
            .entity(spawned)
            .insert(crate::console::weapons::TacticalTargetSelector {
                selector: s.to_selector().unwrap_or_default(),
                power_rating: config.power_rating.map(|r| r as f32),
                // AC6 (issue #781): explicit radar idle from `[weapons_console]
                // selector_idle`, else baseline (radar runs its selector).
                idle: config
                    .weapons_console
                    .as_ref()
                    .map(|wc| wc.selector_idle)
                    .unwrap_or(false),
            });
    }

    // Navigation target selector (issue #778) — the player-ship half of
    // the per-entity pattern. The authored
    // `[navigation_console.selector]` block drives
    // `operate_navigation_ai`'s ranking under Backfill.
    if let Some(s) = config
        .navigation_console
        .as_ref()
        .and_then(|nc| nc.selector.as_ref())
    {
        commands
            .entity(spawned)
            .insert(crate::console::navigation::NavigationTargetSelector {
                selector: s.to_selector().unwrap_or_default(),
                power_rating: config.power_rating.map(|r| r as f32),
            });
    }

    // Comms hail selector + dialogue-response policy (issue #786) — the
    // player-ship half of the per-entity pattern in `spawner.rs`, and
    // the ONLY half that can ever run: both `operate_comms_ai` and
    // `operate_comms_response_ai` are filtered `With<LocalShip>`, and
    // the spawner never spawns the player ship. Without this,
    // `[comms_console.selector]` / `[comms_console.ai]` parsed,
    // validated, and were then silently ignored (the host's tick-local
    // canonical default always won), and `self_fact(power_rating)` /
    // `fact(power_rating)` were permanently ABSENT — the #779
    // empty-facts failure mode. Both are resolved by the same shared
    // helper the spawner calls, so the two paths cannot drift.
    let (comms_selector, comms_response_policy, comms_response_cadence) =
        crate::console::comms::server::comms_console_ai_components(config);
    if let Some(sel) = comms_selector {
        commands.entity(spawned).insert(sel);
    }
    if let Some(policy) = comms_response_policy {
        commands.entity(spawned).insert(policy);
    }
    if let Some(cadence) = comms_response_cadence {
        commands.entity(spawned).insert(cadence);
    }

    // Repair target selector (issue #785) — same player-ship gap. Less
    // severe than Comms because `operate_repair_ai`'s host is
    // `With<Ship>`, so spawner-built NPCs already carried one; but the
    // PLAYER ship never goes through the spawner, so an authored
    // `[repair.selector]` on a player-class hull was ignored and
    // `self_fact(power_rating)` was absent there too. Same shape as the
    // spawner's insert; the block is already validated in
    // `EntityConfig::from_toml`.
    if let Some(s) = config.repair.as_ref().and_then(|rc| rc.selector.as_ref()) {
        commands
            .entity(spawned)
            .insert(crate::console::repair::server::RepairTargetSelector {
                selector: s.to_selector().unwrap_or_default(),
                power_rating: config.power_rating.map(|r| r as f32),
            });
    }

    // Captain AI policy (issue #775) — the player ship half of the
    // per-entity pattern above. The authored `[captain_console.ai]` block
    // drives `operate_captain_ai`. Validated already in
    // `EntityConfig::from_toml`.
    if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
        commands
            .entity(spawned)
            .insert(crate::console::captain::server::CaptainAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
    }

    // Helm fine-system AI policies (issues #779/#780, collapsed by
    // #1209) — player-ship half of the per-entity pattern in
    // `spawner.rs`. The authored `[helm_console.*_ai]` blocks resolve into
    // ONE keyed `FineSystemAiPolicies` map, one entry per authored block,
    // driving `ai_helm_thrust` / `ai_helm_steering` / the secondary hosts.
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
        commands
            .entity(spawned)
            .insert(crate::ship::helm_ai::FineSystemAiPolicies(fine_policies));
    }
}

/// The player ship's shields damage history and per-arc hull HP (`EntityShipArcHull`).
fn insert_player_shields_damage_and_arc_hull(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // Shields damage history — per-ship Component tracking HP deltas
    // for the AI damage-concentration algorithm. Initialised empty; resized
    // lazily by operate_shields_ai to match the ship's arc count.
    commands
        .entity(spawned)
        .insert(crate::ship::shields::ShieldsDamageHistory::default());

    // Per-arc hull HP (issue #514). Attach `EntityShipArcHull`
    // alongside the shield system so `sync_console_damage_tiers`
    // can flip the fine `shield-arc-<id>` SystemIds into
    // `offline_systems` when an arc's hull HP drops into the
    // Disabled/Destroyed tier.
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
            commands
                .entity(spawned)
                .insert(crate::entities::spawner::EntityShipArcHull(
                    crate::ship::damage::ShipArcHull::from_entries(arc_entries),
                ));
        }
    }
}

/// The player ship's weapons console: phaser render + combat config (each a
/// component + dual-written resource), per-bank phaser/blaster AI policies, and
/// the ship-level weapons doctrine.
fn insert_player_weapons_console(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    if let Some(wc) = &config.weapons_console {
        let first_bank = wc.phaser_banks.first();
        let beam_color = crate::weapons::beam_render::resolve_beam_color(
            first_bank.map(|b| &b.beam_color).unwrap_or(&vec![]),
        );
        let beam_range = first_bank
            .map(|b| {
                if b.beam_range > 0.0 {
                    b.beam_range
                } else {
                    40.0
                }
            })
            .unwrap_or(40.0);
        let render_cfg = PhaserRenderConfig {
            beam_color,
            beam_range,
        };
        // Insert as per-entity component AND global resource (dual-write migration).
        commands.entity(spawned).insert(render_cfg.clone());
        commands.insert_resource(render_cfg);

        // Player phaser combat tuning — overrides the default
        // PhaserCombatConfig that WeaponsPlugin installed. The
        // [weapons_console] block already carries `beam_range`,
        // `beam_damage_per_sec`, `beam_duration_secs`, and
        // `cooldown_secs`; before this slice those were only
        // honoured by the NPC phaser path. Now the player path
        // also reads them via the PhaserCombatConfig resource.
        let combat_cfg = crate::console::weapons::PhaserCombatConfigResource(
            crate::entities::config::PhaserCombatConfig::from_weapons_console(wc),
        );
        // Insert as per-entity component AND global resource (dual-write migration).
        commands.entity(spawned).insert(combat_cfg.clone());
        commands.insert_resource(combat_cfg);

        // Per-bank phaser / blaster open-fire AI policies (issue #781) —
        // the player-ship half of `spawner.rs`'s per-weapon maps, added
        // by #885b stage 5d.
        //
        // THIS PATH DID NOT EXIST BEFORE. Until the synthesisers were
        // deleted the player ship carried no per-bank map at all and
        // `ai_phaser_auto_fire` / `ai_blaster_auto_fire` silently fell
        // back to a Rust-side default on every tick — the same "the
        // player ship goes through a second attachment path" omission
        // that bit #785, #786 and #882. Attaching the authored maps here
        // is what keeps a backfilled player ship firing exactly as it
        // did, now from its own TOML.
        let phaser_bank_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> =
            wc.phaser_banks
                .iter()
                .filter_map(|b| {
                    let ai = b.ai.as_ref()?;
                    Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                })
                .collect();
        commands
            .entity(spawned)
            .insert(crate::console::weapons::PhaserBankAiPolicies(
                phaser_bank_policies,
            ));
        // The ship-level WEAPONS DOCTRINE (issue #956) — the player-ship
        // half of `spawner.rs`'s attachment. Without it the player hull
        // would carry no family order at all and would never ask Helm to
        // turn a gun onto its target, which is exactly the
        // player-path-forgotten failure #785/#786/#882 each shipped.
        if let Some(ai) = wc.ai.as_ref() {
            commands
                .entity(spawned)
                .insert(crate::console::weapons::WeaponsDoctrineAiPolicy(
                    ai.to_policy().unwrap_or_default(),
                ));
        }
        if !wc.blaster_banks.is_empty() {
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
            commands
                .entity(spawned)
                .insert(crate::console::weapons::BlasterBankAiPolicies(
                    blaster_bank_policies,
                ));
        }
    } else {
        // No [weapons_console] block — insert defaults so the entity-component
        // path always finds a value on the LocalShip entity.
        commands
            .entity(spawned)
            .insert(crate::console::weapons::PhaserCombatConfigResource::default());
        commands
            .entity(spawned)
            .insert(PhaserRenderConfig::default());
    }
}

/// The player ship's torpedo system from `[torpedoes]` (component + dual-written
/// resource) plus the per-tube and shared-magazine AI policies.
fn insert_player_torpedoes(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // [torpedoes] block — builds the TorpedoSystem from TOML config.
    // Inserted as per-entity component AND global resource (dual-write
    // migration). NPC ships with a [torpedoes] block also get their own
    // TorpedoSystemResource component via `entities::spawner::spawn_entity`
    // (see #597 PR-3 and the audit follow-up); `tick_torpedo_lifecycle`
    // iterates `With<Ship>` so both paths advance the same way.
    if let Some(tc) = &config.torpedoes {
        let runtime_config = tc.to_runtime();
        let torpedo_system = if !tc.tubes.is_empty() {
            crate::weapons::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
        } else {
            crate::weapons::torpedo::TorpedoSystem::new(runtime_config)
        };
        let torpedo_res = crate::console::weapons::TorpedoSystemResource(torpedo_system);
        // Insert as per-entity component AND global resource (dual-write migration).
        commands.insert_resource(torpedo_res.clone());
        commands.entity(spawned).insert(torpedo_res);

        // Per-tube load/launch + shared-magazine grant AI policies
        // (issue #782) — the player-ship half of `spawner.rs`'s maps,
        // added by #885b stage 5d for the same reason as the phaser and
        // blaster maps above: before it the player ship carried neither,
        // and the torpedo hosts fell back to a Rust-side default every
        // tick.
        let tube_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> = tc
            .tubes
            .iter()
            .filter_map(|t| {
                let ai = t.ai.as_ref()?;
                Some((t.id.clone(), ai.to_policy().unwrap_or_default()))
            })
            .collect();
        commands
            .entity(spawned)
            .insert(crate::console::weapons::TorpedoTubeAiPolicies(
                tube_policies,
            ));
        if let Some(ai) = tc.ai.as_ref() {
            commands
                .entity(spawned)
                .insert(crate::console::weapons::TorpedoMagazineAiPolicy(
                    ai.to_policy().unwrap_or_default(),
                ));
        }
    }
}

/// The player ship's power state: the reactor `ShipPowerSystem` seeded from the
/// authored groups, the power config (component + resource), the inline power
/// allocation AI policy, and the per-group power multipliers.
fn insert_player_power_state(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
    power_group_seed: &[(crate::core::messages::PowerGroupId, u8)],
) {
    // Power config — unconditionally insert as per-entity Component
    // so systems that iterate `With<Ship>` always see a value on
    // the player ship (matching NPCs, which spawner.rs always
    // inserts a defaulted `PowerConfigResource` for). Dual-writes
    // the global Resource for legacy readers.
    let power_config = if let Some(pc) = &config.power {
        PowerConfigResource(crate::modifiers::power_system::PowerConfig {
            capacity: pc.capacity,
            rates: pc.rates,
            sustainable_total: pc.sustainable_total,
            max_commanded_total: pc.max_commanded_total,
            emergency_threshold: pc.emergency_threshold,
        })
    } else {
        PowerConfigResource::default()
    };
    commands
        .entity(spawned)
        .insert(crate::ship::power::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_authored_groups(
                &power_config.0,
                power_group_seed,
            ),
        ));
    commands.entity(spawned).insert(power_config.clone());
    commands.insert_resource(power_config);

    // Inline stateless Power allocation AI policy (issue #784) — from the
    // authored `[power.ai_policy]` block, so `ai_power_allocation`
    // iterating `With<Ship>` sees the ship's own policy. `to_policy`
    // cannot fail: validated at load.
    if let Some(ai) = config.power.as_ref().and_then(|pc| pc.ai_policy.as_ref()) {
        commands.entity(spawned).insert((
            crate::ship::power::PowerAiPolicy(ai.to_policy().unwrap_or_default()),
            // Carried from the SAME authored block (issue #889's
            // evaluate_every_ticks, wired at runtime): a resolved
            // `AiPolicy` alone forgets this field, so it rides
            // alongside as a sibling component.
            crate::ship::power::PowerAiCadence(ai.evaluate_every_ticks),
        ));
    }

    // Power multipliers
    let defaults = [-0.5, 0.0, 0.25, 0.5];
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
            // shields_console power drives ModifierSlot::ShieldRegen (#952)
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    commands.insert_resource(PowerMultiplierResource {
        multipliers: multipliers.clone(),
    });
    // Insert as per-entity component AND global resource (dual-write migration — PR 6).
    commands
        .entity(spawned)
        .insert(PowerMultiplierResource { multipliers });
}

/// The player ship's helm configs from `[helm_console]`: physics, impulse, boost
/// (each disabled/defaulted when its table is absent), and bank config.
fn insert_player_helm_configs(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
) {
    // Ship physics config from [helm_console] TOML, or default
    let physics_cfg =
        config
            .helm_console
            .as_ref()
            .map(|hc| crate::ship::physics::ShipPhysicsConfig {
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
                // Vertical axis (issue #744): no dedicated helm_console
                // TOML yet, so take the ShipPhysicsConfig defaults.
                ..crate::ship::physics::ShipPhysicsConfig::new()
            });
    let physics_cfg_resource = crate::ship_plugin::ShipPhysicsConfigResource(
        // `ShipPhysicsConfig::default()` is defined as `Self::new()`, so this is
        // byte-identical to the pre-#1200 `unwrap_or(ShipPhysicsConfig::new())`.
        physics_cfg.unwrap_or_default(),
    );
    commands.insert_resource(physics_cfg_resource.clone());
    commands.entity(spawned).insert(physics_cfg_resource);

    // Impulse config from [helm_console] TOML, or default
    let impulse_steering = config
        .helm_capability
        .as_ref()
        .map(|cap| cap.impulse.steering_multiplier)
        .unwrap_or(0.0);
    let impulse_cfg = config
        .helm_console
        .as_ref()
        .map(|hc| crate::ship_plugin::ImpulseConfigResource {
            charge_duration: hc.impulse_charge_duration,
            speed_multiplier: hc.impulse_speed_multiplier,
            acceleration_multiplier: hc.impulse_acceleration_multiplier,
            engage_distance: hc.impulse_engage_distance,
            cancel_distance: hc.impulse_cancel_distance,
            steering_multiplier: impulse_steering,
        })
        .unwrap_or_default();
    commands.entity(spawned).insert(impulse_cfg);

    // Boost config from [helm_console.boost] TOML. Absent table ⇒
    // feature disabled (default component has `enabled: false`).
    let boost_cfg = config
        .helm_console
        .as_ref()
        .and_then(|hc| hc.boost.as_ref())
        .map(|b| crate::ship_plugin::BoostConfigResource {
            enabled: true,
            multiplier: b.multiplier,
            steering_multiplier: b.steering_multiplier,
            active_duration: b.active_duration,
            recharge_duration: b.recharge_duration,
        })
        .unwrap_or_default();
    commands.entity(spawned).insert(boost_cfg);

    // Bank config from [helm_console] TOML, or default
    let bank_cfg = config
        .helm_console
        .as_ref()
        .map(|hc| crate::ship_plugin::BankConfigResource {
            max_bank_deg: hc.max_bank_deg,
            bank_lerp_rate: hc.bank_lerp_rate,
        })
        .unwrap_or_default();
    commands.insert_resource(bank_cfg.clone());
    commands.entity(spawned).insert(bank_cfg);
}

/// Diagnostic: dump every tracked entity's components on InProgress start.
/// Helps debug missing raider or other invisible NPC issues.
pub(crate) fn dump_tracked_entities(
    query: Query<(
        &EntityUuid,
        Option<&EntityName>,
        Option<&EntityId>,
        &Transform,
        Option<&MeshSection>,
        Option<&EntityTagsSection>,
        Option<&RadarAppearanceSection>,
        Option<&BehaviourSection>,
        Option<&FactionComponent>,
    )>,
) {
    bevy::log::info!("=== ENTITY DUMP (InProgress start) ===");
    let mut count = 0u32;
    for (uuid, name, id, transform, mesh, tags, radar, behaviour, faction) in &query {
        count += 1;
        let label = name
            .map(|n| n.0.clone())
            .or_else(|| id.map(|i| i.0.clone()))
            .unwrap_or_else(|| "?".to_string());
        let pos = format!(
            "[{:.1}, {:.1}, {:.1}]",
            transform.translation.x, transform.translation.y, transform.translation.z
        );
        let has_mesh = if mesh.is_some() { "MESH" } else { "no-mesh" };
        let tags_str = tags
            .map(|t| format!("tags={:?}", t.0))
            .unwrap_or_else(|| "no-tags".to_string());
        let has_radar = if radar.is_some() { "RADAR" } else { "no-radar" };
        let has_ai = if behaviour.is_some() { "AI" } else { "no-ai" };
        let fac = faction
            .map(|f| format!("faction={}", f.0))
            .unwrap_or_else(|| "no-faction".to_string());
        bevy::log::info!(
            "  ENTTY uuid={} label={} pos={} {} {} {} {} {}",
            &uuid.0[..uuid.0.len().min(8)],
            label,
            pos,
            has_mesh,
            tags_str,
            has_radar,
            has_ai,
            fac
        );
    }
    bevy::log::info!("=== ENTITY DUMP END ({} entities) ===", count);
}
