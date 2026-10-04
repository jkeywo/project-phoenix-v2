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
//! activation gate (`world_activation_blocked`) and the shared World materialization
//! chain keep it agreeing with `world::server::spawn_world_entities`
//! on which entries it owns and on the shared `WorldIdMint` order — the entity
//! mint order feeds the authoritative digest, so the spawn sequence is fixed.

use super::*;

/// The identities attached to authored `GameStart` rows in the run that booted.
///
/// Kept as derived boot metadata so [`crate::snapshot::capture`] can persist the
/// exact pre-restore roster without marking entities or changing their
/// archetypes. The vector contains only rows that actually spawned (a false
/// `when` predicate contributes nothing), in authored row order.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GameStartEntityUuids(pub(crate) Vec<crate::snapshot::GameStartEntityUuid>);

/// GameStart identities a compatible saved run requires on a fresh boot.
///
/// This is a transient, target-neutral boot resource: browser and native resume
/// stage the same value after the full save gate, and the shared spawn system
/// consumes it once. It is deliberately not part of snapshot restore itself —
/// by then every captured entity row is already keyed by these UUIDs and it is
/// too late to repair a freshly minted identity.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResumeGameStartEntityUuids(Vec<crate::snapshot::GameStartEntityUuid>);

/// Stage the validated GameStart identity part of a saved boot record.
pub(crate) fn stage_resume_game_start_entity_uuids(
    world: &mut World,
    boot: &crate::snapshot::BootIdentity,
) {
    world.insert_resource(ResumeGameStartEntityUuids(
        boot.game_start_entity_uuids.clone(),
    ));
}

fn select_game_start_entity_uuid(
    resume: Option<&ResumeGameStartEntityUuids>,
    authored_index: u32,
    registered_uuid: Option<&String>,
    minted_uuid: String,
) -> String {
    resume
        .and_then(|saved| {
            saved
                .0
                .binary_search_by_key(&authored_index, |row| row.authored_index)
                .ok()
                .map(|index| saved.0[index].entity_uuid.clone())
        })
        .or_else(|| registered_uuid.cloned())
        .unwrap_or(minted_uuid)
}

fn resume_game_start_row_inclusion(
    resume: Option<&ResumeGameStartEntityUuids>,
    authored_index: u32,
) -> Option<bool> {
    resume.map(|saved| {
        saved
            .0
            .binary_search_by_key(&authored_index, |row| row.authored_index)
            .is_ok()
    })
}

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
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    script_gate: Option<Res<crate::world::server::ScriptActivationGate>>,
) {
    if script_gate.is_some_and(|gate| gate.blocks_spawn("setup_world")) {
        return;
    }
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
    id_mint: Option<&crate::world_id::EntityMint>,
) -> usize {
    // Atomic-activation guard (issues #750/#752/#906/#969/#973). This system owns one
    // half of the immediate spawn; `spawn_world_entities` owns the other, and
    // both run in the ordered World materialization pass. Each half still
    // guards activation: the loop below answers a failed
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

        let uuid =
            crate::world_id::mint_live_id_with(id_mint, crate::world_id::IdNamespace::Entity);
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

        let spawned_entity = crate::entities::spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid.clone(),
            entity_inst.id.clone(),
        );
        commands
            .entity(spawned_entity)
            .insert(crate::entities::spawner::EntityTemplatePath::new(
                &entity_inst.template_path,
            ));
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
    mut runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    mut has_spawned: Local<bool>,
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    frozen_ship_slots: Option<Res<crate::ship_slots::FrozenShipSlots>>,
    gm_join_bootstrap: Option<Res<crate::gm_join::GmJoinBootstrap>>,
    fleet_session: Option<Res<crate::lockstep::FleetLockstep>>,
    resume_game_start_uuids: Option<Res<ResumeGameStartEntityUuids>>,
) {
    if *has_spawned {
        return;
    }

    let mc = match world_config.as_deref() {
        Some(mc) => mc,
        None => return,
    };

    let config_cache = crate::entities::config_cache::get_config_cache();

    // The frozen fleet (issue #1116). One ship for a lone host — the default,
    // and the shipped single-player case — or one per host in a fleet, taking
    // the world's GameStart `ship` rows in slot order. An app with no roster
    // resource at all (a bare fixture) behaves as a fleet of one.
    let solo_roster = crate::lockstep::FleetRoster::default();
    let roster = roster
        .as_deref()
        .or_else(|| {
            gm_join_bootstrap
                .as_deref()
                .map(|bootstrap| bootstrap.topology())
        })
        .unwrap_or(&solo_roster);
    let launch_ship_count = frozen_ship_slots
        .as_ref()
        .map_or_else(|| roster.len(), |slots| slots.0.len());
    let mut player_ships_spawned = 0usize;
    let mut launched_slot_ids = Vec::new();
    let mut authored_ship_row_index = 0usize;
    let mut game_start_entity_uuids = Vec::new();
    let named_positions = crate::world::config::build_named_entity_positions(mc);
    for (authored_index, entity_inst) in mc.entities.iter().enumerate() {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
            continue;
        }
        let authored_index = u32::try_from(authored_index)
            .expect("a WorldConfig cannot contain more than u32::MAX entity rows");
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
        let is_authored_ship_row = config.tags.iter().any(|tag| tag == "ship");
        let authored_slot = is_authored_ship_row
            .then(|| mc.ship_slots.get(authored_ship_row_index))
            .flatten();
        if is_authored_ship_row {
            authored_ship_row_index += 1;
        }
        // A resume reproduces the original GameStart row set recorded in its
        // authored-index map. Fresh lobby timing may leave flags at different
        // values, so re-evaluating `when` here could omit a saved live entity or
        // add one the original run never spawned. A normal boot keeps the exact
        // old predicate path.
        if let Some(included) =
            resume_game_start_row_inclusion(resume_game_start_uuids.as_deref(), authored_index)
        {
            if !included {
                continue;
            }
        } else if let Some(pred) = &entity_inst.when_predicate {
            let empty = crate::world::flags::FlagStore::new();
            let flags_ref = runtime.as_ref().map(|r| &r.flags).unwrap_or(&empty);
            if !pred.evaluate(&[flags_ref]) {
                continue;
            }
        }
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
        // Is this row one of the fleet's player ships? The world authors as
        // many `ship`-tagged GameStart rows as the largest fleet it supports,
        // and the roster decides how many of them are crewed hulls rather than
        // ordinary NPCs — so a two-ship world played solo spawns one player
        // ship and one NPC, exactly as it did before the fleet existed.
        let frozen_slot = if let Some(slot) = authored_slot {
            frozen_ship_slots
                .as_ref()
                .and_then(|frozen| frozen.0.iter().find(|row| row.slot_id == slot.id))
        } else if is_authored_ship_row {
            frozen_ship_slots
                .as_ref()
                .and_then(|frozen| frozen.0.get(player_ships_spawned))
        } else {
            None
        };
        // In an authored slot world, a frozen omission is the `Absent` policy,
        // not an NPC fallback. Skip that slot's own GameStart row completely.
        if authored_slot.is_some() && frozen_ship_slots.is_some() && frozen_slot.is_none() {
            continue;
        }
        let is_fleet_ship = if authored_slot.is_some() && frozen_ship_slots.is_some() {
            frozen_slot.is_some()
        } else {
            player_ships_spawned < launch_ship_count && is_authored_ship_row
        };
        // A frozen authored claim names the ship whose original crew owns this
        // view. The default solo roster has no authored slot, so assigning it
        // positionally would make an earlier Backfill slot local as well as the
        // claimed slot (notably in a disposable Workshop Test controlling wing).
        // A real fleet roster carries authored ids and matches those instead.
        let has_frozen_claim = frozen_ship_slots
            .as_ref()
            .is_some_and(|slots| slots.0.iter().any(|slot| slot.claimant.is_some()));
        let fleet_ship = if is_fleet_ship && authored_slot.is_some() && has_frozen_claim {
            frozen_slot.and_then(|slot| {
                roster
                    .ships()
                    .iter()
                    .find(|ship| ship.authored_slot_id.as_deref() == Some(slot.slot_id.as_str()))
            })
        } else if is_fleet_ship {
            roster.ship(player_ships_spawned)
        } else {
            None
        };
        // Which hull this slot flies: the roster's choice for a fleet member,
        // and this host's own lobby selection for a lone host (`ship_path` is
        // `None` there, which is what keeps the solo spawn byte-identical).
        let hull_path: Option<String> = frozen_slot
            .map(|slot| slot.hull.clone())
            .or_else(|| fleet_ship.and_then(|ship| ship.ship_path.clone()))
            .or_else(|| selected_ship.as_ref().map(|sel| sel.0.clone()));
        let config = if is_fleet_ship {
            player_hull_config(
                config,
                entity_inst.overrides.as_ref(),
                hull_path.as_deref().and_then(|path| config_cache.get(path)),
            )
        } else {
            config
        };

        // Always consume the ordinary mint, including on resume. Its counter is
        // authoritative state and the fresh bootstrap must take the identical
        // draw it would have taken without a saved run. The value attached to a
        // saved authored GameStart row takes precedence; on a normal boot, a
        // named row uses the UUID Startup already registered in `name_to_uuid`.
        // That is the same identity guarantee the Immediate named path makes:
        // trigger/GM lookups must resolve to the entity that actually spawned.
        // A row skipped by its `when` predicate consumes neither a mint nor a
        // saved identity.
        let minted_uuid = crate::world_id::mint_live_id_with(
            id_mint.as_deref(),
            crate::world_id::IdNamespace::Entity,
        );
        let registered_uuid = entity_inst
            .name
            .as_ref()
            .and_then(|name| mc.name_to_uuid.get(name));
        let uuid = select_game_start_entity_uuid(
            resume_game_start_uuids.as_deref(),
            authored_index,
            registered_uuid,
            minted_uuid,
        );
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
        // `[player_spawn]` places the FIRST fleet ship only. Every later slot
        // takes its own authored `[[entity]] transform`, which is how a world
        // gives a fleet distinct starting positions — one anchor cannot place
        // two hulls.
        let pos = if is_fleet_ship && player_ships_spawned == 0 {
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

        // Each crew slot keeps its authored heading. The first slot's legacy
        // player_spawn override still wins when present (issue #623).
        let player_spawn_rot: Option<bevy::math::Quat> =
            if is_fleet_ship && player_ships_spawned == 0 {
                mc.player_spawn.as_ref().and_then(|s| s.rotation).map(|r| {
                    let (q, _) = player_spawn_rotation_yaw(r);
                    q
                })
            } else {
                None
            }
            .or_else(|| {
                entity_inst
                    .transform
                    .as_ref()
                    .filter(|_| is_fleet_ship)
                    .map(|transform| transform.quat())
            });

        // Extract yaw for ShipPhysicsComponent
        let initial_yaw = player_spawn_rot
            .map(|q| {
                let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
                yaw
            })
            .unwrap_or(0.0);

        let placement = if is_fleet_ship {
            let synthetic_host = fleet_ship
                .map(|ship| ship.host)
                .or_else(|| {
                    frozen_slot.map(|slot| {
                        if slot.claimant.is_some() {
                            crate::command_admission::HostSlot::SOLO
                        } else {
                            crate::command_admission::HostSlot(
                                u32::try_from(player_ships_spawned + 1)
                                    .expect("ship-slot count fits a u32"),
                            )
                        }
                    })
                })
                .unwrap_or(crate::command_admission::HostSlot::SOLO);
            let crew = fleet_ship.map_or(&[][..], |ship| ship.crew.as_slice());
            Some(FleetPlacement {
                host: synthetic_host,
                is_local: if fleet_ship.is_some() {
                    roster.is_local(synthetic_host)
                } else {
                    frozen_slot.is_some_and(|slot| slot.claimant.is_some())
                },
                uses_live_sessions: if fleet_ship.is_some() {
                    crate::lockstep::uses_live_sessions(
                        roster,
                        fleet_session.is_some(),
                        synthetic_host,
                    )
                } else {
                    frozen_slot.is_some_and(|slot| slot.claimant.is_some())
                },
                crew,
                hull_path: hull_path.as_deref(),
                authored_slot_id: frozen_slot
                    .map(|slot| slot.slot_id.as_str())
                    .or_else(|| fleet_ship.and_then(|ship| ship.authored_slot_id.as_deref())),
            })
        } else {
            None
        };
        let seed = placement.as_ref().map(|placement| {
            prepare_ship_seed(
                &mut commands,
                initial_yaw,
                &mut pending_ship_config,
                &mut sessions,
                placement,
                &config_cache,
            )
        });

        let spawned = crate::entities::spawner::spawn_entity_with_ship_seed(
            &mut commands,
            &config,
            pos,
            uuid.clone(),
            entity_inst.id.clone(),
            seed,
        );
        let spawned_template = hull_path
            .as_deref()
            .filter(|_| is_fleet_ship)
            .unwrap_or(&entity_inst.template_path);
        commands
            .entity(spawned)
            .insert(crate::entities::spawner::EntityTemplatePath::new(
                spawned_template,
            ));
        game_start_entity_uuids.push(crate::snapshot::GameStartEntityUuid {
            authored_index,
            entity_uuid: uuid,
        });

        // The authored narrative mark (issue #1338), attached on this path for
        // the same reason `world::server::spawn_immediate_entities_internal`
        // attaches it on its own: the payload is the WORLD's authored `name` —
        // the unique reference id triggers, comms and objectives address this
        // instance by — and the entity template knows nothing about it.
        //
        // Both spawn timings mark, because `narrative = true` says nothing
        // about WHEN the hull enters the world. A story hull that the world
        // holds back until the game starts (`spawn_on = "game_start"`) is still
        // a story hull, and marking only the Immediate branch would drop its
        // spawn and death from the timeline with no warning anywhere — the one
        // failure PRD #1337's "authored, never inferred" rule cannot absorb,
        // because the author believes they marked it. A nameless row cannot be
        // marked (there is no id to carry); `world::validate` warns about that
        // combination at load rather than leaving it silent here.
        if entity_inst.narrative {
            if let Some(name) = entity_inst.name.as_ref() {
                commands
                    .entity(spawned)
                    .insert(crate::core::narrative::NarrativeMark(name.clone()));
            }
        }

        // Apply rotation on the spawned entity's Transform
        if let Some(q) = player_spawn_rot {
            commands
                .entity(spawned)
                .insert(bevy::prelude::Transform::from_translation(pos).with_rotation(q));
        }

        if let Some(placement) = &placement {
            if let Some(slot) = authored_slot {
                launched_slot_ids.push(slot.id.clone());
            }
            insert_fleet_metadata(&mut commands, spawned, &config, placement);
            publish_compatibility_ship_resources(&mut commands, spawned);
            player_ships_spawned += 1;
        }
    }

    // A resumed run keeps the ORIGINAL mapping even when this fresh bootstrap
    // skipped a conditional row or the saved entity had already been destroyed.
    // The restore may despawn fresh surplus entities, but later saves still need
    // the run's complete boot identity rather than a reconstruction of this
    // particular bootstrap attempt.
    let persistent_game_start_uuids = resume_game_start_uuids
        .as_deref()
        .map(|saved| saved.0.clone())
        .unwrap_or(game_start_entity_uuids);
    commands.insert_resource(GameStartEntityUuids(persistent_game_start_uuids));
    // Scenario scripts may scale their first encounter from the fleet that
    // actually launched. Freeze the fact after all GameStart rows were applied;
    // a later disconnect/backfill does not change the authored mission plan.
    // Slotless legacy worlds retain their old flag store and digest.
    if !mc.ship_slots.is_empty() {
        if let Some(runtime) = runtime.as_mut() {
            runtime.flags.set_flag_value(
                "fleet.initial_player_ships",
                i64::try_from(player_ships_spawned).expect("ship slot count fits i64"),
            );
            for slot_id in launched_slot_ids {
                runtime
                    .flags
                    .set_flag_value(&format!("fleet.slot.{slot_id}.present"), 1);
            }
        }
    }
    if resume_game_start_uuids.is_some() {
        commands.remove_resource::<ResumeGameStartEntityUuids>();
    }
    *has_spawned = true;
}

/// Which fleet slot a GameStart player ship is being built for, and everything
/// about that slot the builders below need (issue #1116).
///
/// A struct rather than five loose arguments because four of the five are only
/// meaningful together: "this is slot 2's ship, it is not this host's, the
/// fleet agreed its Helm is crewed at Std, and it flies this hull" is one fact.
pub(crate) struct FleetPlacement<'a> {
    /// The host that flies this ship.
    pub host: crate::command_admission::HostSlot,
    /// Whether that host is this one — the whole of what `LocalShip` means.
    pub is_local: bool,
    /// Whether this ship takes its crew and pending Ratings from the App's live
    /// `Sessions`. This is true for the local ship in a solo or independently
    /// restored saved fleet, and false for every ship in active lockstep.
    pub uses_live_sessions: bool,
    /// The frozen crewing of this ship, `(station, rating)`.
    pub crew: &'a [(crate::core::messages::StationId, String)],
    /// The hull this slot flies, as an entity-template path.
    pub hull_path: Option<&'a str>,
    /// Scenario-authored ship-slot identity. Unlike `host`, this is stable
    /// content vocabulary and may be used by objective recipient selectors.
    pub authored_slot_id: Option<&'a str>,
}

/// Resolve a GameStart ship's final topology, crew ratings and initial heading
/// before the shared installer constructs any runtime state.
///
/// # Seeding a ship whose crew is on another machine (issue #1116)
///
/// The pre-#1116 seeding reads local `Sessions` — which stations have a
/// connected player, and what complexity Rating each of them chose. That is the
/// right answer for a ship run by this App alone and the wrong one for an active
/// fleet: `Sessions` describes only THIS host's crew, so slot 2's ship would boot
/// fully AI-backfilled here and human-crewed on the machine its crew is sitting
/// at. The two hosts would then run different AI on the same hull from tick zero.
///
/// An active `FleetLockstep` therefore seeds every ship — its own included —
/// from the roster's frozen crewing, which every host received identically when
/// the roster froze. With no lockstep session, the local ship uses live
/// `Sessions`; any retained remote saved ships use their empty frozen crew and
/// start on Backfill.
fn prepare_ship_seed(
    commands: &mut Commands,
    initial_yaw: f32,
    pending_ship_config: &mut Option<ResMut<crate::ship_plugin::PendingShipConfig>>,
    sessions: &mut Option<ResMut<crate::lobby::Sessions>>,
    placement: &FleetPlacement<'_>,
    config_cache: &crate::entities::config_cache::ConfigCache,
) -> crate::entities::ship_spawn::ShipSpawnSeed {
    // `PendingShipConfig` is this host's own lobby selection, so it belongs to
    // this host's own ship and nothing else. A fleet member's hull comes from
    // the template the roster named — the same template every host in the fleet
    // resolved, which is what makes the stations, systems and Ratings below
    // identical everywhere.
    let ship_config = if placement.is_local {
        if let Some(pending) = pending_ship_config.as_mut() {
            let cfg = crate::ship_plugin::ShipConfigComponent(pending.0.clone());
            commands.remove_resource::<crate::ship_plugin::PendingShipConfig>();
            *pending_ship_config = None;
            cfg
        } else {
            crate::ship_plugin::load_ship_config_from_disk()
        }
    } else {
        placement
            .hull_path
            .and_then(|path| config_cache.get(path))
            .and_then(|entity| entity.ship_config.clone())
            .map(crate::ship_plugin::ShipConfigComponent)
            .unwrap_or_else(|| {
                bevy::log::error!(
                    "fleet {}: no station-bearing hull at {:?}; falling back to \
                     the default so the ship still exists",
                    placement.host.slot_id(),
                    placement.hull_path,
                );
                crate::ship_plugin::load_ship_config_from_disk()
            })
    };
    let (initial_control_sources, initial_active_ratings) = if !placement.uses_live_sessions {
        // An active fleet, or a remote ship in an independent saved fleet:
        // seed from the frozen roster. Active peers therefore agree exactly;
        // the standalone remote roster is empty and yields AI Backfill.
        let (resolver, active_ratings) =
            crate::ship::rating::seed_boot_ratings(&ship_config.0, |station| {
                placement
                    .crew
                    .iter()
                    .find(|(id, _)| *id == station.id)
                    .map(|(_, rating)| rating.clone())
                    .unwrap_or_else(|| crate::ship::rating::BACKFILL_RATING.to_string())
            });
        (
            crate::ship_plugin::ShipSystemControlSources(resolver),
            crate::ship_plugin::ActiveStationRatings(active_ratings),
        )
    } else {
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
    // Pending ratings belong only to this App's local lobby. A remote ship may
    // spawn first (when restoring slot 2), so it must not consume them before
    // the local ship reads them. Active fleets still discard the now-obsolete
    // local lobby choices when their local frozen-roster ship is configured.
    if placement.is_local {
        if let Some(sess) = sessions.as_mut() {
            sess.0.clear_all_pending_ratings();
        }
    }

    crate::entities::ship_spawn::ShipSpawnSeed {
        ship_config,
        control_sources: initial_control_sources,
        active_ratings: initial_active_ratings,
        initial_yaw,
    }
}

/// Fleet identity and fidelity; authored capabilities are already installed.
fn insert_fleet_metadata(
    commands: &mut Commands,
    spawned: Entity,
    config: &crate::entities::config::EntityConfig,
    placement: &FleetPlacement<'_>,
) {
    let mut ship = commands.entity(spawned);
    // LocalShip selects the host's presentation; all fleet hulls share core state.
    if placement.is_local {
        ship.insert(LocalShip);
    }
    if let Some(slot_id) = placement.authored_slot_id {
        ship.insert(crate::ship_slots::AuthoredShipSlotId(slot_id.to_string()));
    }
    ship
        // Which host flies this hull. On every host in the fleet, for every
        // fleet ship — so an NPC and a peer's player ship are told apart by a
        // component rather than by absence, and a peer's `ShipKey`-routed
        // command has something to be answered by.
        .insert(crate::lockstep::FleetSlotOf(placement.host))
        // The three components #984 made `LocalShip` `#[require]`. They are
        // inserted HERE, in the spawn burst, on EVERY fleet ship — the local
        // one and the peers' alike (issue #1116). Two reasons, and both are
        // load-bearing:
        //
        //   * a mid-run `Commands::insert` of `HumanSeekingHosts` moved the
        //     authoritative digest on `duel` and `rng_coverage` (#984/#1051),
        //     so the resolver must never have to create them; and
        //   * making their PRESENCE follow `LocalShip` would give two hosts
        //     different component sets on the same hull from tick zero, which
        //     is the cross-host asymmetry `LocalShip`'s docs now forbid.
        //
        // Every fleet ship needs them on their merits too: a peer's ship has
        // human-seeking systems that must resolve the same way on every host,
        // or one host's AI operates a console another host's crew is sitting at.
        .insert(crate::ship_plugin::HumanSeekingHosts::default())
        .insert(crate::ship_plugin::VisitingStationHosts::default())
        .insert(crate::ship_plugin::ScenarioDetailFloor::default())
        // The player ship is permanently high-fidelity (`lod_ai_ships`
        // never evaluates `LocalShip`), so it takes the marker and the
        // components that travel with it from the SAME shared
        // definition the NPC promotion path uses. Spelling the set out
        // here again is how #785's RepairTargetSelector, #786's
        // CommsTargetSelector and #882's HelmBoostAiPolicyState each
        // silently missed the player ship.
        .insert(crate::ai::server::ai_high_fidelity_components());
    insert_player_identity(commands, spawned, config);
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

/// Compatibility resources retain their existing per-fleet-row publication order.
/// Components are constructed once by ship_spawn; this queued command reads them
/// after installation, before the next fleet row's publication.
fn publish_compatibility_ship_resources(commands: &mut Commands, spawned: Entity) {
    commands.queue(move |world: &mut World| {
        macro_rules! publish {
            ($ty:ty) => {
                if let Some(value) = world.get::<$ty>(spawned).cloned() {
                    world.insert_resource(value);
                }
            };
        }
        publish!(crate::ship::shields::ShieldsAiConfigResource);
        publish!(PhaserRenderConfig);
        publish!(crate::console::weapons::PhaserCombatConfigResource);
        publish!(crate::console::weapons::TorpedoSystemResource);
        publish!(PowerConfigResource);
        publish!(PowerMultiplierResource);
        publish!(crate::ship_plugin::ShipPhysicsConfigResource);
        publish!(crate::ship_plugin::BankConfigResource);
    });
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

#[cfg(test)]
#[path = "world_setup_game_start_identity_tests.rs"]
mod game_start_identity_tests;

#[cfg(test)]
#[path = "world_setup_ship_spawn_tests.rs"]
mod ship_spawn_tests;
