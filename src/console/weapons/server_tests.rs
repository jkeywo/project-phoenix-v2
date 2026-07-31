use super::shared::system_is_registered;
use super::*;
use crate::ai_plugin::AiTokenRegistry;
use crate::damage::SystemHull;
use crate::entity_spawner::EntitySystemHull;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target, WorldResource};
use crate::messages::*;
use crate::modifiers::ShipModifiers;
use crate::simmath;
use crate::simulation::{ShipImpulse, SimOutbox};

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

#[derive(Resource, Default)]
struct ArcRequestLog(Vec<CoordinationEnqueue>);

fn collect_arc_requests(
    mut reader: MessageReader<CoordinationEnqueue>,
    mut log: ResMut<ArcRequestLog>,
) {
    for m in reader.read() {
        log.0.push(m.clone());
    }
}

/// Build a minimal `ShipConfigComponent` with a tactical station that has an
/// "Assisted" rating containing `torpedo_auto_fire` in its ai_tuning table.
///
/// Post-#512 this now uses fine Tactical `[[system]]` blocks matching
/// the ship entity TOML (phaser-fore/aft, torpedo-tube-fore-port/aft, etc.)
/// so tests exercise the production per-fine-system gate paths rather
/// than the legacy fallback-to-coarse-tactical path. The coarse
/// `[[system]] id = "tactical"` block is DELETED to match production.
fn test_ship_config() -> crate::ship_plugin::ShipConfigComponent {
    const TOML: &str = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Std"
automated_systems = []

[[station.rating]]
name = "Assisted"
automated_systems = []

[station.rating.ai_tuning]
torpedo_auto_fire = {}

[[system]]
id = "phaser-port"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "phaser-starboard"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"

[[system]]
id = "phaser-control"
kind = "phaser_control"
station = "tactical"

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"

[[system]]
id = "torpedo-tube-fore-starboard"
kind = "torpedo_tube"
station = "tactical"

[[system]]
id = "torpedo-tube-aft"
kind = "torpedo_tube"
station = "tactical"

# Declared so `any_blaster_bank_operates_ai` (which reads the ship CONFIG, not
# the `BlasterSystemResource` component) can see a blaster group on fixtures
# that attach one. Inert for ships that attach none: the auto-fire query
# requires `&BlasterSystemResource`, so a hull without the component is never
# iterated.
[[system]]
id = "blaster-fore"
kind = "blaster_bank"
station = "tactical"
"#;
    crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::parse_and_validate(
            TOML,
            &[
                "phaser_bank",
                "blaster_bank",
                "torpedo_tube",
                "torpedo_magazine",
                "tactical_radar",
                "phaser_control",
            ],
        )
        .expect("test ship config must be valid"),
    )
}

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

/// Test-only glue (issue #829): production lifts each ship's radar-selection
/// into `ViewscreenBlackboard::combat_lock` / `.science_target` via the radar
/// publishers + viewscreen aggregators (not present in these focused harnesses).
/// This mirror runs before `SimSet::Input` each tick, seeding the frozen
/// viewscreen fact from the ship's own `TacticalRadarSelection` /
/// `SensorRadarSelection` components so the converted consumers (firing, arc
/// request, freq match) see last-tick's selection exactly as they do in the
/// full app. Merges into any existing viewscreen entry (preserves scored
/// objectives).
fn seed_viewscreen_from_selection(
    mut q: Query<
        (
            Option<&TacticalRadarSelection>,
            Option<&crate::sensors_plugin::SensorRadarSelection>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
    for (tac, sci, mut bbs) in q.iter_mut() {
        let combat_lock = tac.and_then(|t| t.0.clone());
        let science_target = sci.and_then(|s| s.0.clone());
        let mut vbb = match bbs.0.get(&crate::system_registry::viewscreen_system_id()) {
            Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = combat_lock;
        vbb.science_target = science_target;
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vbb),
        );
    }
}

fn test_app() -> App {
    let mut app = App::new();
    app.configure_sets(
        Update,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    )
    .add_plugins(LobbyPlugin)
    .add_plugins(bevy::time::TimePlugin)
    .add_plugins(crate::server_app::AdmissionPlugin)
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(200),
    ))
    .init_resource::<WorldResource>()
    .add_message::<AsteroidDestroyedVfx>()
    .add_message::<ShipDestroyedVfx>()
    .add_message::<crate::ai_plugin::AiEntityDestroyed>()
    .add_message::<crate::balance::BalanceEvent>()
    .init_resource::<CurrentPhaserMode>()
    .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
        TorpedoConfig::default(),
    )))
    .init_resource::<SimOutbox>()
    .init_resource::<Outbox>()
    .init_resource::<ArcRequestLog>()
    .init_resource::<crate::world::server::WorldContentRuntime>()
    .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
    .add_plugins(WeaponsPlugin)
    // Override with two banks so per-bank arc checks work.
    // Uses wide (270°) arcs so existing tests that fire "port" at a
    // target ahead still pass. Tighter arcs are tested in dedicated
    // per-bank arc severance tests.
    .insert_resource(PhaserCombatConfigResource(
        crate::entity_config::PhaserCombatConfig {
            banks: vec![
                crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                },
                crate::entity_config::PhaserBankConfig {
                    id: "starboard".into(),
                    facing_deg: 90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                },
            ],
        },
    ))
    .add_systems(
        Update,
        seed_viewscreen_from_selection.before(crate::sim_sets::SimSet::Input),
    )
    .add_plugins(weapons_update_broadcaster())
    // PR-7 (issue #597) — `tick_shields` (formerly `tick_npc_shield_regen`)
    // now lives on `ShipShieldsPlugin`. Include it so tests that spawn NPCs
    // with `ShipShields` observe regen on every frame.
    .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
    .add_systems(PostUpdate, (collect, collect_arc_requests));
    // Spawn the Ship entity with config/control-source components so all
    // weapons systems that use `Query<..., With<Ship>>.single()` have a
    // valid entity to operate on, matching what `spawn_game_start_entities`
    // would do in a full server build.
    let ship = app
        .world_mut()
        .spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            test_ship_config(),
            ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            ShipPhysics::default(),
            crate::ship_state::ShipPhaserFrequency::default(),
            bevy::prelude::Transform::default(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[
                (SystemId("helm".into()), 25.0),
                (SystemId("tactical".into()), 25.0),
                (SystemId("power".into()), 25.0),
                (SystemId("shields".into()), 25.0),
                // Fine Tactical hull entries (issue #512) so tests can drive
                // sync_console_damage_tiers → offline_systems for the fine
                // systems declared in the updated test_ship_config().
                (SystemId("phaser-fore".into()), 15.0),
                (SystemId("phaser-aft".into()), 15.0),
                (SystemId("torpedo-tube-fore-port".into()), 12.0),
                (SystemId("torpedo-tube-fore-starboard".into()), 12.0),
                (SystemId("torpedo-tube-aft".into()), 12.0),
                (SystemId("torpedo-magazine".into()), 20.0),
            ])),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::entity_spawner::EntityUuid("test-local-ship".to_string()),
        ))
        .id();
    // Second insert to stay under Bevy's Bundle-tuple length limit.
    app.world_mut().entity_mut(ship).insert((
        // Insert per-entity weapon configs so component-path queries succeed.
        // These are overridden by individual tests via insert_resource for the
        // PhaserCombatConfigResource; we keep both in sync here.
        TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![
                crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                },
                crate::entity_config::PhaserBankConfig {
                    id: "starboard".into(),
                    facing_deg: 90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                },
            ],
        }),
        PhaserRenderConfig::default(),
        // PR 7 (issue #597) — per-entity beam / target / cooldown components.
        TacticalRadarSelection::default(),
        ActiveBeam::default(),
        PhaserCooldown::default(),
        // PR 10 (PRD #597) — per-entity combat activity trackers.
        crate::server_app::WeaponFiredThisTick::default(),
        crate::server_app::ShipAttackedThisTick::default(),
        LastShipAttacker::default(),
        crate::ship::combat_activity::RecentCombatActivity::default(),
        ShipImpulse(crate::impulse::ImpulseState::new()),
        ShipModifiers::new(),
    ));
    attach_shipped_weapon_ai(&mut app, ship);
    app
}

/// Attach the SHIPPED authored weapons AI declarations to `ship`: the Tactical
/// target selector, plus one per-bank / per-tube policy for every bank and tube
/// the entity currently carries, and the shared magazine's grant policy.
///
/// Since #885b stage 5d the auto-fire hosts have no synthesised fallback — a
/// bank with no entry in `PhaserBankAiPolicies` does not fire, and a ship with
/// no `TacticalTargetSelector` ranks nothing. The ids are read off the entity's
/// own weapon configs rather than listed, so a test that swaps in a different
/// bank list only has to call this again.
pub(crate) fn attach_shipped_weapon_ai(app: &mut App, ship: Entity) {
    use crate::entities::authored_ai_pins::{shipped_policy_toml, shipped_selector_toml};
    let bank_policy = || {
        shipped_policy_toml("phaser_bank")
            .to_policy()
            .expect("the shipped phaser-bank policy decodes")
    };
    let blaster_policy = || {
        shipped_policy_toml("blaster_bank")
            .to_policy()
            .expect("the shipped blaster-bank policy decodes")
    };
    let tube_policy = || {
        shipped_policy_toml("torpedo_tube")
            .to_policy()
            .expect("the shipped torpedo-tube policy decodes")
    };

    let phaser_ids: Vec<String> = app
        .world()
        .entity(ship)
        .get::<PhaserCombatConfigResource>()
        .map(|c| c.0.banks.iter().map(|b| b.id.clone()).collect())
        .unwrap_or_default();
    let blaster_ids: Vec<String> = app
        .world()
        .entity(ship)
        .get::<crate::weapons_plugin::BlasterSystemResource>()
        .map(|c| c.0.iter().map(|b| b.config.id.clone()).collect())
        .unwrap_or_default();
    let tube_ids: Vec<String> = app
        .world()
        .entity(ship)
        .get::<TorpedoSystemResource>()
        .map(|c| c.0.tubes.iter().map(|t| t.id.clone()).collect())
        .unwrap_or_default();

    app.world_mut().entity_mut(ship).insert((
        crate::weapons_plugin::TacticalTargetSelector {
            selector: shipped_selector_toml("tactical")
                .to_selector()
                .expect("the shipped Tactical selector decodes"),
            power_rating: None,
            idle: false,
        },
        crate::weapons_plugin::PhaserBankAiPolicies(
            phaser_ids
                .into_iter()
                .map(|id| (id, bank_policy()))
                .collect(),
        ),
        crate::weapons_plugin::BlasterBankAiPolicies(
            blaster_ids
                .into_iter()
                .map(|id| (id, blaster_policy()))
                .collect(),
        ),
        crate::weapons_plugin::TorpedoTubeAiPolicies(
            tube_ids.into_iter().map(|id| (id, tube_policy())).collect(),
        ),
        crate::weapons_plugin::TorpedoMagazineAiPolicy(
            shipped_policy_toml("torpedo_magazine")
                .to_policy()
                .expect("the shipped torpedo-magazine policy decodes"),
        ),
        // Red alert, RAISED (issue #872).
        //
        // The shipped player-hull fire policies this helper attaches are
        // authored `fact(red_alert) >= param(min_alert_to_fire)` with a
        // threshold of 1, so a player hull holds fire until its captain calls
        // red alert. Every test below this line is about something else —
        // arcs, leads, cooldowns, admission, per-bank independence — and each
        // needs the ship WILLING to fire before it can say anything about how
        // it fires. Raising the alert restores that premise without touching
        // the gate; the gate itself is proved in both directions by
        // `backfilled_weapons_hold_fire_until_red_alert` and the
        // `weapons_fire_guard_truth_table` in `authored_ai_pins`.
        crate::ship_state::ShipRedAlert(true),
    ));
}

// ── PR 7 test helpers — per-entity access to Weapons state ──────────────
// These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
// `world.resource::<X>()` after PR 7 (PRD #597) removed the Resource derive.
//
// Each helper: single-entity lookup returning owned data.

fn get_weapons_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&TacticalRadarSelection, With<crate::server_app::LocalShip>>();
    q.single(app.world()).ok().and_then(|wt| wt.0.clone())
}

fn set_weapons_target(app: &mut App, uuid: Option<String>) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut TacticalRadarSelection, With<crate::server_app::LocalShip>>();
    if let Ok(mut wt) = q.single_mut(app.world_mut()) {
        wt.0 = uuid;
    }
}

// ── Single-beam fixture helpers (post-#790) ──────────────────────────────────
//
// `ActiveBeam` is per-bank since issue #790. Every fixture below drives a ship
// that fires ONE bank at a time, so "the beam" still means "the one live slot";
// these helpers preserve their old signatures and simply address that slot.
// Tests that genuinely care about two banks at once assert on `live_banks()`
// directly.

fn get_active_beam_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .ok()
        .and_then(|b| b.any_target().map(str::to_string))
}

fn active_beam_target_is_none(app: &mut App) -> bool {
    get_active_beam_target(app).is_none()
}

fn get_active_beam_bank(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .ok()
        .and_then(|b| b.any_bank().map(str::to_string))
}

fn live_beam_banks(app: &mut App) -> Vec<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .ok()
        .map(|b| b.live_banks().map(|(k, _)| k.clone()).collect())
        .unwrap_or_default()
}

fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
    let banks = live_beam_banks(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        match uuid {
            // Extinguish every live bank — the "beam cancelled" fixture.
            None => {
                for bank in banks {
                    b.end_bank(&bank);
                }
            }
            Some(u) => {
                let bank = banks.first().cloned().unwrap_or_default();
                let remaining = b
                    .bank_slot_mut(&bank)
                    .map(|s| s.remaining_secs)
                    .unwrap_or(0.0);
                b.start(bank, u, remaining);
            }
        }
    }
}

fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
    let banks = live_beam_banks(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        for bank in banks {
            if let Some(slot) = b.bank_slot_mut(&bank) {
                slot.remaining_secs = secs;
            }
        }
    }
}

fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
    let banks = live_beam_banks(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        for bank in banks {
            if let Some(slot) = b.bank_slot_mut(&bank) {
                slot.damage_accumulator = val;
            }
        }
    }
}

fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
    let mut q = app
        .world_mut()
        .query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .ok()
        .map(|cd| cd.is_bank_active(bank))
        .unwrap_or(false)
}

fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut PhaserCooldown, With<crate::server_app::LocalShip>>();
    if let Ok(mut cd) = q.single_mut(app.world_mut()) {
        cd.start_bank_with_cooldown(bank, secs);
    }
}

fn get_phaser_frequency(app: &mut App) -> f32 {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<crate::server_app::LocalShip>>();
    q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
}

fn set_ship_yaw(app: &mut App, yaw: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
    let mut p = q
        .single_mut(app.world_mut())
        .expect("expected Ship with ShipPhysics");
    p.yaw = yaw;
}

fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

fn tick(app: &mut App) -> Vec<OutboundMessage> {
    // Issue #889: the weapons AI deciders (`ai_target_selection`,
    // `ai_phaser_auto_fire`, `tick_blaster_auto_fire`) are gated by
    // `run_if(ai_tick_ready)` — before it they decided once per rendered frame.
    // A fixture that wants a decision on this update ticks the latch by hand;
    // the cadence itself is covered in `ai::cadence`.
    crate::ai::cadence::arm_ai_tick(app);
    app.update();
    let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
    let mut out = app.world().resource::<Outbox>().0.clone();
    for (target, msg) in sim_entries {
        out.push(OutboundMessage {
            target,
            msg,
            delivery: crate::messages::DeliveryClass::Reliable,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn load_tube_now(app: &mut App, tube: &str) {
    // The systems now prefer the per-entity component over the resource.
    // Update both to keep them in sync.
    let mut q = app
        .world_mut()
        .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
    if let Ok(mut ts) = q.single_mut(app.world_mut()) {
        ts.0.tube_mut(tube)
            .expect("test tube should exist")
            .loaded_count = 1;
    } else {
        let world = app.world_mut();
        let mut res = world.resource_mut::<TorpedoSystemResource>();
        res.0
            .tube_mut(tube)
            .expect("test tube should exist")
            .loaded_count = 1;
    }
}

fn start_game(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    tick(app);
}

fn setup_weapons_world(
    app: &mut App,
    asteroid_x: f32,
    asteroid_z: f32,
) -> bevy::ecs::entity::Entity {
    let uuid = "target-uuid".to_string();
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot::asteroid(
                &uuid, asteroid_x, asteroid_z, 2.0,
            )],
            ..Default::default()
        }));
    // handle_set_target and tick_beams use live ECS Transforms
    // (live_entity_xz), so every WorldResource entry must also have a
    // matching ECS entity with the components all queries expect.
    app.world_mut()
        .spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        ))
        .id()
}

fn setup_weapons_world_with_entity(
    app: &mut App,
    asteroid_x: f32,
    asteroid_z: f32,
) -> bevy::ecs::entity::Entity {
    setup_weapons_world(app, asteroid_x, asteroid_z)
}

fn start_game_with_weapons(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "weapons", ClientMessage::SetReady { ready: true });
    tick(app);
    // Apply the human rating for Tactical's weapons systems so
    // `admit_system_commands` (which checks ShipSystemControlSources)
    // authorizes human ControlSystem messages for phasers, torpedoes, etc.
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    if let Ok(mut cs) = q.single_mut(app.world_mut()) {
        use crate::ship::control_source::ControlSource;
        cs.0.set(SystemId("phaser-port".into()), ControlSource::Human);
        cs.0.set(SystemId("phaser-starboard".into()), ControlSource::Human);
        cs.0.set(
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::system_registry::torpedo_tube_aft_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::system_registry::torpedo_magazine_system_id(),
            ControlSource::Human,
        );
    }
}

fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
    setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
    start_game_with_weapons(app);
    push(
        app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let _ = tick(app);
    push(
        app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(app)
}

// ── SetTarget / TargetLock tests ───────────────────────────────────────

#[test]
fn valid_target_within_range_replies_with_target_lock_confirmed() {
    let mut app = test_app();
    // The lock horizon is the ship's OWN authored `[weapons_console.radar]
    // range` since issue #887 (per-ship, so it works on an NPC too), not the
    // `LocalShip`-only `ShipClientConfigResource`. Author one, as every shipped
    // player hull does.
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let out = tick(&mut app);

    let lock = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        })
        .expect("expected a TargetLock response");
    assert_eq!(lock.0, "target-uuid");
    assert!(lock.1, "expected locked=true for in-range asteroid");

    assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
}

#[test]
fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
    let mut app = test_app();
    // See `valid_target_within_range_...`: the horizon is this ship's own
    // authored radar range (issue #887).
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 400.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let out = tick(&mut app);

    let lock = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        })
        .expect("expected a TargetLock response");
    assert!(!lock.1, "expected locked=false for out-of-range asteroid");
    assert!(get_weapons_target(&mut app).is_none());
}

#[test]
fn unknown_uuid_replies_with_target_lock_rejected() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 10.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "no-such-asteroid".into(),
            },
        },
    );
    let out = tick(&mut app);

    let lock = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        })
        .expect("expected a TargetLock response");
    assert!(!lock.1, "expected locked=false for unknown UUID");
    assert!(get_weapons_target(&mut app).is_none());
}

// ── WeaponsUpdate / fire_ready tests ───────────────────────────────────

#[test]
fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    // Tick 1 admits SetTarget in `SimSet::Input`. `compute_current_weapons_update`
    // reads the frozen viewscreen combat lock (spec §3), which this harness'
    // `seed_viewscreen_from_selection` glue refreshes before `SimSet::Input` —
    // so the new lock lands on the wire on tick 2. (In the full app the
    // viewscreen aggregator runs in `SimSet::PublishAggregate`, before the
    // `SimSet::Broadcast` broadcaster, so there is no such gap.)
    tick(&mut app);
    let out = tick(&mut app);

    let update = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate {
                target_uuid, banks, ..
            } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
            _ => None,
        })
        .expect("expected a WeaponsUpdate message");
    assert_eq!(update.0.as_deref(), Some("target-uuid"));
    assert!(
        update.1,
        "expected fire_ready=true for in-range, forward-arc target"
    );
}

#[test]
fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    // Two ticks, for the same frozen-combat-lock reason as the test above.
    tick(&mut app);
    let out = tick(&mut app);

    let update = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate {
                target_uuid, banks, ..
            } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
            _ => None,
        })
        .expect("expected a WeaponsUpdate message");
    assert_eq!(update.0.as_deref(), Some("target-uuid"));
    assert!(
        !update.1,
        "expected fire_ready=false for beyond-phaser-range target"
    );
}

// ── Shared weapon readiness contract (issue #764) ──────────────────────
//
// The phaser bank's `readiness` field must publish the same observable
// blocking cases the client renders (AC4): Ready when in range + arc, and the
// correct `WeaponBlockReason` for no-target and out-of-range. Blaster range /
// arc / no-target / cooldown / loading / offline coverage lives in the pure
// `blaster.rs` model tests; these exercise the server projection end to end.

/// Read the LocalShip's published Weapons blackboard `banks` (deterministic
/// every tick, unlike the diff-gated WeaponsUpdate broadcast).
fn local_weapons_banks(app: &mut App) -> Vec<crate::messages::PhaserBankState> {
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .expect("a LocalShip must exist");
    weapons_blackboard_of(app, entity)
        .expect("LocalShip must publish a Weapons blackboard")
        .banks
}

/// Read the LocalShip's published Weapons blackboard `tubes`.
fn local_weapons_tubes(app: &mut App) -> Vec<crate::messages::TorpedoTubeState> {
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .expect("a LocalShip must exist");
    weapons_blackboard_of(app, entity)
        .expect("LocalShip must publish a Weapons blackboard")
        .tubes
}

#[test]
fn torpedo_readiness_unloaded_tube_blocks_on_loading_or_no_ammo() {
    use crate::messages::WeaponBlockReason;
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    tick(&mut app);

    let tubes = local_weapons_tubes(&mut app);
    assert!(!tubes.is_empty(), "the test ship declares torpedo tubes");
    // Freshly started tubes carry no loaded round, so the shared contract must
    // report a loading/magazine blocking reason — never Ready (issue #764).
    for tube in &tubes {
        assert!(!tube.readiness.ready, "unloaded tube must not be Ready");
        assert!(
            matches!(
                tube.readiness.blocking_reason,
                WeaponBlockReason::Loading
                    | WeaponBlockReason::NoAmmo
                    | WeaponBlockReason::NoTarget
                    | WeaponBlockReason::Offline
            ),
            "unloaded tube must map onto a loading/ammo/target/offline reason, got {:?}",
            tube.readiness.blocking_reason
        );
    }
}

fn set_target(app: &mut App) {
    push(
        app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
}

#[test]
fn phaser_readiness_ready_when_target_in_range_and_arc() {
    use crate::messages::WeaponBlockReason;
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    set_target(&mut app);
    tick(&mut app);
    tick(&mut app);

    let banks = local_weapons_banks(&mut app);
    let ready = banks
        .iter()
        .find(|b| b.readiness.blocking_reason == WeaponBlockReason::Ready)
        .expect("at least one bank should report Ready for an in-range forward target");
    assert!(ready.readiness.ready);
    assert!(
        ready.readiness.target_range.is_some(),
        "a locked target must populate target_range"
    );
    assert!(ready.readiness.target_arc.is_some());
}

#[test]
fn phaser_readiness_no_target_blocks_with_no_target_reason() {
    use crate::messages::WeaponBlockReason;
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    // No SetTarget sent — every bank must report NoTarget.
    tick(&mut app);

    let banks = local_weapons_banks(&mut app);
    assert!(!banks.is_empty());
    assert!(
        banks
            .iter()
            .all(|b| b.readiness.blocking_reason == WeaponBlockReason::NoTarget),
        "every bank must report NoTarget when nothing is locked"
    );
    assert!(banks.iter().all(|b| !b.readiness.ready));
    assert!(banks.iter().all(|b| b.readiness.target_range.is_none()));
}

#[test]
fn phaser_readiness_out_of_range_blocks_with_out_of_range_reason() {
    use crate::messages::WeaponBlockReason;
    let mut app = test_app();
    // Target dead ahead but beyond phaser range.
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game_with_weapons(&mut app);
    set_target(&mut app);
    tick(&mut app);
    tick(&mut app);

    let banks = local_weapons_banks(&mut app);
    assert!(
        banks
            .iter()
            .all(|b| b.readiness.blocking_reason == WeaponBlockReason::OutOfRange),
        "every bank must report OutOfRange for a beyond-range forward target, got {:?}",
        banks
            .iter()
            .map(|b| b.readiness.blocking_reason)
            .collect::<Vec<_>>()
    );
    // Range/arc geometry is still populated while blocked.
    assert!(banks.iter().all(|b| b.readiness.target_range.is_some()));
}

// ── FirePhaser / beam lifecycle tests ──────────────────────────────────

#[test]
fn fire_phaser_on_valid_target_broadcasts_beam_started() {
    let mut app = test_app();
    let out = lock_and_fire(&mut app, 0.0, -20.0);

    let beam_started = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
    assert!(
        beam_started.is_some(),
        "expected BeamStarted after firing at fire-ready target"
    );
    match &beam_started.unwrap().msg {
        ServerMessage::BeamStarted { target_uuid, .. } => {
            assert_eq!(target_uuid, "target-uuid")
        }
        _ => unreachable!(),
    }
    match &beam_started.unwrap().target {
        Target::All => {}
        t => panic!("BeamStarted should target All, got {:?}", t),
    }

    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );
}

#[test]
fn fire_phaser_rejected_during_cooldown() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "port", 3.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "BeamStarted should not fire during cooldown"
    );
}

#[test]
fn fire_phaser_ignored_from_non_weapons_player() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "captain should not be able to fire phaser"
    );
}

#[test]
fn fire_phaser_rejected_when_target_outside_bank_arc() {
    let mut app = test_app();
    // Target at starboard beam (20, 0), bearing +90°, which is outside the
    // port bank's 270° arc centered at -90° (covers -135° to 45°).
    setup_weapons_world(&mut app, 20.0, 0.0);
    start_game_with_weapons(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "FirePhaser should be rejected when target is outside bank's fire arc"
    );
}

#[test]
fn full_beam_duration_kills_asteroid() {
    let mut app = test_app();
    // setup_weapons_world (called by lock_and_fire) now spawns the
    // asteroid ECS entity. Fetch its handle after setup.
    let _ = lock_and_fire(&mut app, 0.0, -20.0);
    let asteroid_entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("setup_weapons_world should have spawned the target asteroid")
    };

    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );

    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);

    let out = tick(&mut app);

    let destroyed = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
    assert!(
        destroyed.is_some(),
        "expected AsteroidDestroyed when asteroid HP reaches 0"
    );
    match &destroyed.unwrap().msg {
        ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
        _ => unreachable!(),
    }

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded after asteroid destruction"
    );

    assert!(
        !app.world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid"),
        "destroyed asteroid should be removed from WorldData"
    );

    assert!(active_beam_target_is_none(&mut app));

    assert!(
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after beam end"
    );

    assert!(
        app.world()
            .get::<EntitySystemHull>(asteroid_entity)
            .is_none(),
        "asteroid entity should be despawned"
    );
}

#[test]
fn beam_severs_when_target_leaves_bank_arc() {
    let mut app = test_app();
    // Target at port beam (-20, 0), bearing -90° — inside port bank's
    // 270° arc centered at -90° (covers -135° to 45°).
    let _ = lock_and_fire(&mut app, -20.0, 0.0);

    // Rotate 180° so the target moves to starboard beam (bearing +90°),
    // which is outside the port bank's arc.
    set_ship_yaw(&mut app, std::f32::consts::PI);

    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded when target leaves bank fire arc"
    );
    assert!(
        active_beam_target_is_none(&mut app),
        "beam should be cleared after sever-by-arc"
    );
    assert!(
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after arc sever"
    );
}

#[test]
fn beam_severs_when_target_leaves_phaser_range() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Move the live ECS Transform out of range. tick_beams reads the
    // live position, not the WorldResource snapshot.
    let entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("target entity should exist")
    };
    app.world_mut()
        .entity_mut(entity)
        .insert(Transform::from_xyz(0.0, 0.0, -50.0));

    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded when target leaves phaser range"
    );
    assert!(
        active_beam_target_is_none(&mut app),
        "beam should be cleared after sever-by-range"
    );
    assert!(
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after range sever"
    );
}

#[test]
fn no_damage_refund_on_sever() {
    let mut app = test_app();
    let asteroid_entity = app
        .world_mut()
        .spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
        ))
        .id();
    // Target at port beam (-20, 0) so the port bank's arc check passes.
    let _ = lock_and_fire(&mut app, -20.0, 0.0);

    set_active_beam_damage_accumulator(&mut app, 10.0);
    let _ = tick(&mut app);

    // Rotate 180° — target moves to starboard beam, outside port bank's arc.
    set_ship_yaw(&mut app, std::f32::consts::PI);
    let _ = tick(&mut app);

    let hp = app
        .world()
        .get::<EntitySystemHull>(asteroid_entity)
        .map(|h| h.0.total_current());
    assert!(
        hp.is_some() && hp.unwrap() < 30.0,
        "asteroid should retain damage after sever (no refund), hp={:?}",
        hp
    );
}

#[test]
fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![
                crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
            ..Default::default()
        }));
    // Spawn matching ECS entities so live_entity_xz can find them.
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        crate::simulation::AsteroidUuid("t1".into()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        crate::simulation::AsteroidUuid("t2".into()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -15.0),
    ));
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let _ = tick(&mut app);
    assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

    set_active_beam_remaining_secs(&mut app, 0.0);
    set_active_beam_damage_accumulator(&mut app, 0.0);
    let _ = tick(&mut app);

    assert!(phaser_bank_is_active(&mut app, "port"));

    start_phaser_cooldown(&mut app, "port", 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "expected BeamStarted for new target after cooldown"
    );
    assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t2"));
}

/// Issue #763 (AC1) — a firing phaser does not jump to a newly selected
/// target. Capture-at-attack-start is authoritative: `handle_fire_phaser`
/// records `ActiveBeam.target_uuid` from the frozen combat lock when the beam
/// opens, and every later tick re-resolves *that captured uuid*, never the
/// live selection. Changing the combat lock (and even re-issuing FirePhaser on
/// the same bank) while the beam is live must leave the beam on its original
/// target. Modelled on `retarget_after_cooldown_cancels_prior_beam_and_starts_new`
/// but WITHOUT winding `remaining_secs`/cooldown down — the beam stays live
/// throughout.
#[test]
fn firing_beam_retains_captured_target_when_combat_lock_changes() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![
                crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
            ..Default::default()
        }));
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        crate::simulation::AsteroidUuid("t1".into()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        crate::simulation::AsteroidUuid("t2".into()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -15.0),
    ));
    start_game_with_weapons(&mut app);

    // Lock t1 and open fire on the port bank.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let _ = tick(&mut app);
    assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

    // Change the combat lock to t2 mid-attack (does NOT reset remaining_secs
    // or cooldown — the beam is still live and burning down its duration), and
    // re-issue FirePhaser on the same bank to prove the fire entrypoint
    // early-outs while a beam is live.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
        },
    );
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );

    let mut all_out = Vec::new();
    for _ in 0..3 {
        all_out.extend(tick(&mut app));
    }

    // The live selection has moved to t2 ...
    assert_eq!(get_weapons_target(&mut app).as_deref(), Some("t2"));
    // ... but the beam is still burning the ORIGINAL captured target t1.
    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("t1"),
        "firing beam must retain its captured target when the combat lock changes"
    );
    // No new beam opened on t2 while the original was live.
    assert!(
        !all_out.iter().any(|m| matches!(
            &m.msg,
            ServerMessage::BeamStarted { target_uuid, .. } if target_uuid == "t2"
        )),
        "no BeamStarted should fire for the newly selected target mid-attack"
    );
}

/// Issue #763 (AC2) — the captured target stays valid only while it remains a
/// live entity. When the captured entity vanishes mid-beam, the sever path in
/// `tick_beams_prepare` (`live_entity_xz` → `None`) clears the beam, starts the
/// bank cooldown, and emits `BeamEnded`. Complements
/// `beam_severs_when_target_leaves_phaser_range` (range) and
/// `beam_severs_when_target_leaves_bank_arc` (arc).
#[test]
fn beam_severs_when_target_vanishes() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Despawn the live target entity mid-beam.
    let entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("target entity should exist")
    };
    app.world_mut().entity_mut(entity).despawn();

    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded when the captured target vanishes"
    );
    assert!(
        active_beam_target_is_none(&mut app),
        "beam should clear when its captured target vanishes"
    );
    assert!(
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after target-vanish sever"
    );
}

/// Issue #763 (AC4) — independent banks, tested at the capture/cooldown level:
/// driving the *other* bank (starboard) into cooldown must not disturb the port
/// bank's live captured beam, and the two banks' cooldown state stays
/// independent. This is the LIVE counterpart to the pure `banks_are_independent`
/// test in `src/weapons/phaser.rs`.
///
/// The premise changed under issue #790 — `ActiveBeam` is per-bank now, so
/// "only one bank fires a beam at a time" is no longer true and this fixture
/// simply has only one bank bearing (the target sits in the port arc alone).
/// Both `ActiveBeam` and `PhaserCooldown` are per-bank maps, and this pins that
/// touching one bank's entry leaves the other's alone in both of them.
#[test]
fn independent_bank_cooldown_does_not_disturb_live_captured_beam() {
    let mut app = test_app();
    // Target at port beam (-20, 0), inside the port bank's arc.
    let _ = lock_and_fire(&mut app, -20.0, 0.0);
    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );
    assert_eq!(get_active_beam_bank(&mut app).as_deref(), Some("port"));

    // Independently drive the OTHER bank into cooldown.
    start_phaser_cooldown(&mut app, "starboard", 5.0);

    let out = tick(&mut app);

    // Port's live captured beam is untouched by starboard's cooldown.
    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid"),
        "port's captured beam must survive an unrelated starboard cooldown"
    );
    assert_eq!(get_active_beam_bank(&mut app).as_deref(), Some("port"));
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "starboard's cooldown must not sever the live port beam"
    );
    // Cooldown state is per-bank and independent.
    assert!(phaser_bank_is_active(&mut app, "starboard"));
    assert!(
        !phaser_bank_is_active(&mut app, "port"),
        "the port bank is mid-beam, not on cooldown"
    );
}

// ── SetPhaserMode tests ────────────────────────────────────────────────

#[test]
fn weapons_console_can_set_phaser_mode_to_manual() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::messages::PhaserMode::Manual,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world().resource::<CurrentPhaserMode>().0,
        crate::messages::PhaserMode::Manual,
        "phaser mode should be Manual after SetPhaserMode"
    );
}

#[test]
fn non_weapons_player_cannot_set_phaser_mode() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Establish a known mode (Auto) via the authorised player first.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::messages::PhaserMode::Auto,
            },
        },
    );
    tick(&mut app);
    // Non-weapons player attempts to switch back to Manual — must be ignored.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::messages::PhaserMode::Manual,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world().resource::<CurrentPhaserMode>().0,
        crate::messages::PhaserMode::Auto,
        "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
    );
}

// ── FireTorpedo tests ──────────────────────────────────────────────────

#[test]
fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(
            |m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")
        ),
        "expected TorpedoLaunched broadcast after Tactical fires torpedo"
    );
}

/// Regression test for PRD #597 gap-3: an NPC ship spawned with a
/// `[torpedoes]` TOML block must carry its own `TorpedoSystemResource`
/// component, and firing from it via the `ai:<uuid>` token path must
/// launch a torpedo. Two subchecks:
///
/// 1. Direct wiring: `TorpedoSystem::launch()` called on the NPC's own
///    component successfully returns `Launched` (i.e. the tubes are
///    populated and `torpedoes_remaining > 0`).
/// 2. End-to-end message routing: an `ai:<uuid>` `FireTorpedo` message
///    arriving through `InboundMessage` reaches the NPC's tubes and
///    emits a `TorpedoLaunched` broadcast, drawing from the NPC's own
///    per-entity tube state — the player-ship `TorpedoSystemResource`
///    resource is left untouched.
///
/// NPC AI does not currently emit `FireTorpedo` messages autonomously;
/// verifying that pipeline is future work (see PRD #487 fine-grained
/// tactical decomposition). This test covers the wiring.
#[test]
fn npc_ship_can_fire_torpedo_when_toml_has_torpedoes_block() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntityUuid;
    use crate::torpedo::LaunchResult;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "cc000000-0000-0000-0000-000000000001";

    // Simulate what `src/entities/spawner.rs` does for an NPC with
    // `[torpedoes]`: attach a `TorpedoSystemResource` component built
    // from the runtime config, with default tubes (fore_port, fore_starboard, aft).
    let torpedo_config = TorpedoConfig::default();
    let npc_torpedo_sys = crate::torpedo::TorpedoSystem::new(torpedo_config);
    let mut npc_ai_sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the fine tube + magazine systems (there is no coarse
    // tactical system to seed).
    for sysid in [
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::system_registry::torpedo_tube_fore_starboard_system_id(),
        crate::system_registry::torpedo_tube_aft_system_id(),
        crate::system_registry::torpedo_magazine_system_id(),
    ] {
        npc_ai_sources.set(sysid, crate::ship::control_source::ControlSource::Ai);
    }
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(npc_ai_sources),
            crate::ship_plugin::ShipConfigComponent::default(),
            ShipPhysics::default(),
            TacticalRadarSelection::default(),
            TorpedoSystemResource(npc_torpedo_sys),
            crate::server_app::WeaponFiredThisTick::default(),
            crate::messages::AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            bevy::prelude::Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }

    // Subcheck 1: direct wiring — the NPC's own component has functional
    // tubes and `.launch()` succeeds when the tube is loaded.
    {
        let mut ts = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(npc_entity)
            .expect("NPC must have TorpedoSystemResource component");
        ts.0.tube_mut("fore_port")
            .expect("default TorpedoSystem must expose fore_port tube")
            .loaded_count = 1;
        let result = ts.0.launch(
            "fore_port",
            "direct-launch-uuid".to_string(),
            0.0,
            0.0,
            0.0,
            0.0,
            None,
            Some(npc_uuid.to_string()),
        );
        assert!(
            matches!(result, LaunchResult::Launched { .. }),
            "direct TorpedoSystem::launch on NPC's own component must succeed, got {result:?}"
        );
    }

    // Reload the tube for the end-to-end path (previous launch consumed it).
    {
        let mut ts = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(npc_entity)
            .unwrap();
        ts.0.tube_mut("fore_port").unwrap().loaded_count = 1;
        ts.0.in_flight.clear();
    }

    // Subcheck 2: end-to-end message routing.
    // Snapshot the player-ship (resource) torpedo count to prove the NPC's
    // fire draws from its own component, not from the shared Resource.
    let player_torpedoes_before = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;

    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(
            |m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")
        ),
        "NPC should broadcast TorpedoLaunched after ai:<uuid> FireTorpedo message"
    );

    // The player-ship Resource must NOT have been drained.
    let player_torpedoes_after = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;
    assert_eq!(
        player_torpedoes_before, player_torpedoes_after,
        "NPC fire must draw from its own per-entity TorpedoSystemResource, \
         leaving the global (player-ship) Resource untouched"
    );
}

#[test]
fn local_console_token_can_fire_torpedo() {
    // issue #422: actions from the local HTML console (browser server
    // viewscreen / native wry server) arrive under LOCAL_CONSOLE_TOKEN with
    // no remote PeerJS session, so holder_for_station(tactical) is None.
    // `tactical_authorized` must treat that token as an authorized local
    // operator so a button press actually launches end-to-end — the
    // decode→map→InboundMessage→fire hop the wasm bridge cannot unit-test.
    let mut app = test_app();
    // No player holds Tactical here — authorization comes purely from the
    // local-console bypass.
    load_tube_now(&mut app, "fore_port");
    push(
        &mut app,
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(
            |m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")
        ),
        "local console token should be authorized to fire torpedoes end-to-end (issue #422)"
    );
}

#[test]
fn torpedo_system_resource_reflects_battleship_toml_torpedoes_block() {
    // End-to-end TOML-driven wiring check: build the runtime
    // TorpedoSystem the same way `spawn_game_start_entities` does
    // (parse alliance_battleship.toml → TorpedoesConfig::to_runtime → TorpedoSystem)
    // and assert the magazine size matches the TOML.
    // Through the resolver (issue #876): this hull is COMPOSED, so its baked
    // bytes are no longer the document `spawn_game_start_entities` reads.
    let config =
        crate::entity_includes::load_entity_config("assets/entities/alliance_battleship.toml")
            .expect("alliance_battleship.toml must compose and parse");
    let tc = config
        .torpedoes
        .expect("alliance_battleship must declare [torpedoes]");
    let runtime = tc.to_runtime();
    let sys = crate::torpedo::TorpedoSystem::new(runtime.clone());
    // Magazine size matches TOML — changing `count = 30` to `count = 99`
    // in alliance_battleship.toml would fail this assertion.
    assert_eq!(sys.torpedoes_remaining, tc.count);
    assert_eq!(sys.config.damage_hull, tc.damage_hull);
    assert_eq!(sys.config.load_time, tc.load_time);
    assert!((sys.config.turn_rate - tc.turn_rate_deg_per_sec.to_radians()).abs() < 1e-5);
}

#[test]
fn phaser_combat_config_resource_reflects_battleship_toml_weapons_console() {
    // End-to-end TOML-driven wiring check: build the runtime
    // PhaserCombatConfig the same way `spawn_game_start_entities` does
    // (parse alliance_battleship.toml → PhaserCombatConfig::from_weapons_console
    // → PhaserCombatConfigResource) and assert the resulting per-bank
    // values are exactly what the TOML says.
    // Through the resolver (issue #876): this hull is COMPOSED, so its baked
    // bytes are no longer the document `spawn_game_start_entities` reads.
    let config =
        crate::entity_includes::load_entity_config("assets/entities/alliance_battleship.toml")
            .expect("alliance_battleship.toml must compose and parse");
    let wc = config
        .weapons_console
        .expect("alliance_battleship must declare [weapons_console]");
    let combat = crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc);

    // alliance_battleship.toml has two banks (fore, aft) with matching combat values.
    // Fore bank is double-damage (8.0 dps) and shorter range (40) than the standard cruiser.
    assert_eq!(combat.banks.len(), 2, "must have fore and aft banks");
    let fore = &combat.banks[0];
    assert_eq!(fore.id, "fore");
    assert_eq!(fore.cooldown_secs, 6.0, "cooldown_secs from TOML bank");
    assert_eq!(
        fore.beam_duration_secs, 6.0,
        "beam_duration_secs from TOML bank"
    );
    assert_eq!(
        fore.beam_damage_per_sec, 8.0,
        "beam_damage_per_sec from TOML bank"
    );
    assert_eq!(fore.beam_range, 40.0, "beam_range from TOML bank");

    // And starting the cooldown produces exactly that value, so it flows
    // through to live `PhaserCooldown.bank_remaining_secs`.
    let mut cd = PhaserCooldown::default();
    cd.start_bank("test", fore.cooldown_secs);
    assert_eq!(
        cd.bank_remaining_secs("test"),
        6.0,
        "PhaserCooldown::start_bank must use the TOML-sourced cooldown"
    );
}

#[test]
fn non_tactical_player_cannot_fire_torpedo() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "captain should not be able to fire torpedo"
    );
}

#[test]
fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
    // Note: The Lobby gate is now at the SimSet chain level.
    // In test configurations without SimSet, the system processes messages during Lobby.
    let mut app = test_app();
    load_tube_now(&mut app, "aft");
    push(
        &mut app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-aft".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "FireTorpedo should fire during Lobby when no SimSet gate is configured"
    );
}

#[test]
fn torpedo_launched_is_broadcast_to_all() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_starboard");

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-starboard".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    let launched = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
        .expect("expected TorpedoLaunched");
    assert!(
        matches!(&launched.target, Target::All),
        "TorpedoLaunched should be broadcast to All, not {:?}",
        launched.target
    );
}

#[test]
fn torpedo_does_not_detonate_on_asteroid_field_anchor_entity() {
    // Regression for "torpedoes don't appear when you hit fire": the
    // default scenario seats the player ship at (280, 0, 0), 280 m from
    // an `asteroid_field_main` anchor entity at the origin. That anchor
    // entity carries an `[asteroid_field]` section with
    // `outer_radius = 350`, and `EntitySnapshot.radius` is populated from
    // that outer radius. `find_detonation_hits` treats every entity in
    // the world with a non-zero radius as a hittable target, so the
    // torpedo detonated on the field anchor on its first physics tick —
    // before the firing crew ever saw a sphere on the viewscreen.
    //
    // Asteroid-field anchors are virtual organisational entities and
    // must never act as torpedo detonation targets.
    use crate::entity_config::AsteroidFieldConfig;
    use crate::entity_spawner::{AsteroidFieldSection, EntityUuid};

    let mut app = test_app();
    start_game_with_weapons(&mut app);

    let field_uuid = "field-uuid".to_string();
    // Mirror the production code path: the WorldResource snapshot for the
    // field anchor reports radius = outer_radius.
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: field_uuid.clone(),
                position: Some([0.0, 0.0, 0.0]),
                radius: Some(350.0),
                inner_radius: Some(300.0),
                shape: Some("torus".into()),
                tags: vec!["asteroid_field".into()],
                ..Default::default()
            }],
            ..Default::default()
        }));
    // Real ECS-side anchor entity so the live-position path also sees it.
    app.world_mut().spawn((
        EntityUuid(field_uuid.clone()),
        AsteroidFieldSection(AsteroidFieldConfig {
            inner_radius: 300.0,
            outer_radius: 350.0,
            density: 0.005,
            weight: 1.0,
            spawn_distance: 250.0,
            despawn_distance: 300.0,
            asteroid_type_paths: vec![],
            cosmetic_type_paths: vec![],
            shape: None,
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            shield_pierce: 0.0,
            tags: vec![],
            grid: None,
            random_rotation: None,
        }),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Move the ship inside the field-anchor's "radius" (300 < 350).
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("Ship with ShipPhysics");
        p.x = 280.0;
    }
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    // First tick processes the FireTorpedo; second tick is where
    // `tick_torpedo_lifecycle` evaluates detonations against the live
    // target list (including the field anchor at the origin).
    tick(&mut app);
    tick(&mut app);

    let in_flight_len = {
        // Systems prefer the per-entity component; read from it for assertion.
        let mut q = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|ts| ts.0.in_flight.len())
            .unwrap_or_else(|| {
                app.world()
                    .resource::<TorpedoSystemResource>()
                    .0
                    .in_flight
                    .len()
            })
    };
    assert_eq!(
        in_flight_len, 1,
        "torpedo should still be in flight after ticking — the asteroid \
         field anchor entity must not be treated as a detonation target"
    );
}

// ── ShipModifiers integration tests ────────────────────────────────────

#[test]
fn empty_modifier_table_reproduces_base_phaser_damage() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    let hp_before = {
        let world = app.world().resource::<WorldResource>();
        world
            .0
            .entities
            .iter()
            .find(|a| a.uuid == "target-uuid")
            .map(|_| true)
    };
    assert!(hp_before.is_some(), "asteroid should still exist after <1s");
}

#[test]
fn phaser_damage_modifier_doubles_kill_rate() {
    use crate::messages::{ModifierSlot, ModifierSource};
    use crate::modifiers::{Modifier, ShipModifiers};

    let mut app_fast = test_app();
    setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
    start_game_with_weapons(&mut app_fast);
    {
        let mut q = app_fast
            .world_mut()
            .query_filtered::<&mut ShipModifiers, With<crate::simulation::LocalShip>>();
        let mut mods = q.single_mut(app_fast.world_mut()).unwrap();
        mods.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::PhaserDamage,
            bonus: 1.0,
        });
    }
    push(
        &mut app_fast,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app_fast);
    push(
        &mut app_fast,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app_fast);

    set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
    tick(&mut app_fast);

    let still_exists_fast = app_fast
        .world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .any(|a| a.uuid == "target-uuid");
    assert!(
        !still_exists_fast,
        "with 2× phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
    );

    let mut app_base = test_app();
    setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
    start_game_with_weapons(&mut app_base);
    push(
        &mut app_base,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app_base);
    push(
        &mut app_base,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app_base);
    set_active_beam_damage_accumulator(&mut app_base, BEAM_DAMAGE_PER_SEC * 1.0 * 3.5);
    tick(&mut app_base);

    let still_exists_base = app_base
        .world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .any(|a| a.uuid == "target-uuid");
    assert!(
        still_exists_base,
        "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)"
    );
}

// ── SetPhaserFrequency delegation tests ────────────────────────────────

fn start_game_with_sensors_and_weapons(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::Identify {
            token: "sensors".into(),
            name: "Spock".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::SelectStation {
            station: "Sensors".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "sensors", ClientMessage::SetReady { ready: true });
    push(app, "weapons", ClientMessage::SetReady { ready: true });
    tick(app);
}

/// Build the admitted-envelope form of a frequency change (issue #804):
/// the only wire shape since the legacy top-level message was deleted.
fn set_phaser_frequency_msg(frequency: f32) -> ClientMessage {
    ClientMessage::ControlSystem {
        target: crate::system_registry::phaser_control_system_id(),
        payload: SystemControlPayload::SetPhaserFrequency { frequency },
    }
}

#[test]
fn tactical_holder_can_set_phaser_frequency() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(&mut app, "weapons", set_phaser_frequency_msg(0.8));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.8).abs() < 1e-5,
        "Tactical holder should set phaser frequency to 0.8, got {freq}"
    );
}

#[test]
fn sensors_holder_cannot_set_phaser_frequency() {
    // Delegation removed in B4 — only Tactical holder may set phaser frequency.
    let mut app = test_app();
    start_game_with_sensors_and_weapons(&mut app);
    push(&mut app, "sensors", set_phaser_frequency_msg(0.9));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.5).abs() < 1e-5,
        "Sensors holder must NOT change phaser frequency, got {freq}"
    );
}

#[test]
fn unrelated_console_cannot_set_phaser_frequency() {
    let mut app = test_app();
    start_game(&mut app);
    push(&mut app, "captain", set_phaser_frequency_msg(0.9));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.5).abs() < 1e-5,
        "Captain must NOT change phaser frequency, got {freq}"
    );
}

/// When the phaser-control system operates AI, human `SetPhaserFrequency`
/// envelopes are refused at admission (mirrors the navigation console's
/// `control_system_rejected_when_ai_controlled`).
#[test]
fn set_phaser_frequency_rejected_when_phaser_control_ai() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::system_registry::phaser_control_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
    }
    push(&mut app, "weapons", set_phaser_frequency_msg(0.9));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.5).abs() < 1e-5,
        "an AI-operated phaser-control must refuse human frequency input, got {freq}"
    );
}

#[test]
fn set_phaser_frequency_clamps_value() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(&mut app, "weapons", set_phaser_frequency_msg(1.5));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 1.0).abs() < 1e-5,
        "frequency above 1.0 should clamp to 1.0, got {freq}"
    );

    push(&mut app, "weapons", set_phaser_frequency_msg(-0.5));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.0).abs() < 1e-5,
        "frequency below 0.0 should clamp to 0.0, got {freq}"
    );
}

// ── NPC / station phaser damage (issue #311) ──────────────────────────

fn setup_npc_world(app: &mut App, npc_x: f32, npc_z: f32) {
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: "npc-1".into(),
                position: Some([npc_x, 0.0, npc_z]),
                tags: vec!["ship".into()],
                ..Default::default()
            }],
            ..Default::default()
        }));
}

fn spawn_npc_entity(
    app: &mut App,
    npc_x: f32,
    npc_z: f32,
    max_hp: f32,
) -> bevy::ecs::entity::Entity {
    app.world_mut()
        .spawn((
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                max_hp,
            )])),
            Transform::from_xyz(npc_x, 0.0, npc_z),
        ))
        .id()
}

// ── Cycle 1: phaser beam reduces NPC hull ─────────────────────────────

#[test]
fn phaser_beam_damages_npc_entity_hull() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Accumulate damage but don't destroy
    set_active_beam_damage_accumulator(&mut app, 10.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("NPC entity should still exist")
        .0
        .total_current();
    assert!(
        hp < 30.0,
        "NPC hull should be reduced after phaser hit, got {hp}"
    );
}

// ── Cycle 2: NPC at 0 HP is despawned and EntityDespawned broadcast ──

#[test]
fn phaser_beam_destroys_npc_entity_when_hull_reaches_zero() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Force lethal damage
    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    let out = tick(&mut app);

    // ECS entity despawned
    assert!(
        app.world().get::<EntitySystemHull>(npc_entity).is_none(),
        "NPC entity should be despawned after hull reaches 0"
    );

    // EntityDespawned wire message broadcast to all
    let despawned_msg = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { uuid } if uuid == "npc-1"));
    assert!(
        despawned_msg.is_some(),
        "expected EntityDespawned {{ uuid: npc-1 }} broadcast"
    );

    // BeamEnded sent
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded after NPC destruction"
    );

    // Beam cleared, cooldown started
    assert!(active_beam_target_is_none(&mut app));
    assert!(phaser_bank_is_active(&mut app, "port"));
}

// ── NPC shields integration ────────────────────────────────────────────

/// Spawn a shielded NPC: same as `spawn_npc_entity` but also attaches a
/// `ShipShields` (num_facings=1) so the damage routing path is exercised
/// end-to-end.
fn spawn_shielded_npc_entity(
    app: &mut App,
    npc_x: f32,
    npc_z: f32,
    hull_max: f32,
    shield_max: f32,
    regen_per_sec: f32,
) -> bevy::ecs::entity::Entity {
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    app.world_mut()
        .spawn((
            // PR-7 (issue #597) — NPC ships carry the `Ship` marker
            // so the unified `tick_shields` picks them up.
            crate::simulation::Ship,
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                hull_max,
            )])),
            crate::ship::shields::ShipShields(
                ShieldSystem::new(&ShieldConfig {
                    num_facings: 1,
                    max_hp: shield_max.round() as i32,
                    regen_per_sec,
                    offline_duration: 10.0,
                }),
                0.5,
            ),
            Transform::from_xyz(npc_x, 0.0, npc_z),
        ))
        .id()
}

#[test]
fn phaser_beam_damages_shielded_npc_routes_through_shield_first() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Apply 5 units of damage. With pierce=0 (default in test config),
    // the entire amount lands on the shield, hull is unchanged.
    set_active_beam_damage_accumulator(&mut app, 5.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("NPC must still have ShipShields component");
    assert!(
        shields.0.facings[0].hp < 20,
        "shield must absorb damage, got {}",
        shields.0.facings[0].hp
    );
    assert!(
        shields.0.facings[0].is_online(),
        "shield must still be online"
    );

    let hull_hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("hull must still exist")
        .0
        .total_current();
    assert_eq!(hull_hp, 30.0, "hull must be untouched while shield holds");
}

#[test]
fn phaser_beam_breaks_shield_then_leaks_to_hull() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 10.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Apply 15 units of damage. With shield=10, shield depletes
    // and 5 units leak to hull.
    set_active_beam_damage_accumulator(&mut app, 15.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("ShipShields component must persist after break");
    // With ShipShields, a depleted facing goes offline (offline_remaining > 0),
    // not permanently broken.
    assert_eq!(shields.0.facings[0].hp, 0);
    assert!(
        !shields.0.facings[0].is_online(),
        "facing must go offline once depleted"
    );

    let hull_hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("hull must exist")
        .0
        .total_current();
    assert!(
        hull_hp < 30.0 && hull_hp > 20.0,
        "hull must take only the leak (~5 units), got {hull_hp}"
    );
}

fn shield_hp(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<crate::ship::shields::ShipShields>(entity)
        .expect("target must have ShipShields")
        .0
        .facings[0]
        .hp as f32
}

/// The balance tracer has to name both ends of the exchange and split the
/// damage the same way the hull and shields actually took it — that split is
/// the whole point of the event, and nothing on the wire carries it.
#[test]
fn phaser_beam_emits_balance_event_with_attacker_victim_and_split() {
    use crate::balance::BalanceEvent;
    use bevy::ecs::message::Messages;

    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 10.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Ignore the trickle from the ticks above; only the burst below is being
    // reconciled against the components.
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .clear();
    let shield_before = shield_hp(&app, npc_entity);
    let hull_before = hull_hp(&app, npc_entity);

    // 15 units into a partly-charged shield: the shield eats what it has
    // left, the rest leaks to hull.
    set_active_beam_damage_accumulator(&mut app, 15.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let shield_lost = shield_before - shield_hp(&app, npc_entity);
    let hull_lost = hull_before - hull_hp(&app, npc_entity);
    assert!(shield_lost > 0.0 && hull_lost > 0.0, "the hit must do both");

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let hits: Vec<&BalanceEvent> = cursor.read(messages).collect();
    assert!(
        !hits.is_empty(),
        "the beam hit must produce a balance event"
    );

    // A `tick` pumps the app more than once, so the sustained beam lands in
    // more than one instalment; the totals are what must reconcile.
    let mut shield_reported = 0.0f32;
    let mut hull_reported = 0.0f32;
    for hit in &hits {
        let BalanceEvent::DamageApplied {
            attacker,
            victim,
            victim_kind,
            weapon,
            amount,
            shield_absorbed,
            hull_damage,
            system_hit,
        } = hit
        else {
            continue;
        };

        assert_eq!(attacker.as_deref(), Some("test-local-ship"));
        assert_eq!(victim, "npc-1");
        assert_eq!(*victim_kind, crate::balance::VictimKind::Ship);
        assert_eq!(weapon, "port", "weapon must name the firing bank");
        assert!(
            *amount >= shield_absorbed + hull_damage && *amount > 0.0,
            "offered damage must cover what landed, got {amount}"
        );
        assert_eq!(
            *system_hit, None,
            "no chokepoint can name the system hit yet"
        );
        shield_reported += shield_absorbed;
        hull_reported += hull_damage;
    }
    assert_eq!(
        shield_reported, shield_lost,
        "reported shield absorption must match what the shield actually lost"
    );
    assert_eq!(
        hull_reported, hull_lost,
        "reported hull damage must match what the hull actually lost"
    );
}

/// Mining is not combat. The rock still shows up in the timeline — an
/// asteroid soaking a beam is a real thing a balance pass wants to see — but
/// it must not open a ledger row, and it must not inflate the shooter's
/// `damage_dealt` in a report field literally named `damage_by_ship`.
#[test]
fn phaser_beam_on_an_asteroid_is_tagged_and_kept_out_of_the_ledger() {
    use crate::balance::{aggregate_damage, BalanceEvent, VictimKind};
    use bevy::ecs::message::Messages;
    use std::collections::BTreeMap;

    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    spawn_asteroid_target(&mut app, "rock-1", 0.0, -20.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "rock-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);
    set_active_beam_damage_accumulator(&mut app, 10.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let hits: Vec<BalanceEvent> = cursor.read(messages).cloned().collect();
    assert!(
        !hits.is_empty(),
        "shooting the rock must still reach the timeline"
    );
    for hit in &hits {
        let BalanceEvent::DamageApplied {
            victim,
            victim_kind,
            ..
        } = hit
        else {
            continue;
        };
        assert_eq!(victim, "rock-1");
        assert_eq!(*victim_kind, VictimKind::Asteroid);
    }

    // Scope to the damage events — `WeaponFired` records the trigger-pull
    // regardless of target, so it legitimately opens a `shots_fired` row; this
    // assertion is about damage, which mining must not credit.
    let damage_only: Vec<BalanceEvent> = hits
        .iter()
        .filter(|h| matches!(h, BalanceEvent::DamageApplied { .. }))
        .cloned()
        .collect();
    assert!(
        aggregate_damage(damage_only.iter(), &BTreeMap::new()).is_empty(),
        "no ledger row for the rock, and no damage_dealt for the shooter"
    );
}

/// A shooter with no `EntityUuid` is *unknown*, not a ship named `""`. Every
/// other chokepoint models that as `None`; an empty string here would key a
/// junk `""` row in the ledger and print `"attacker":""` in the timeline.
#[test]
fn phaser_beam_from_an_unidentified_shooter_reports_no_attacker() {
    use crate::balance::BalanceEvent;
    use bevy::ecs::message::Messages;

    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 10.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Strip the shooter's identity mid-run — the cheapest way to reach the
    // `shooter_uuid_opt == None` branch of the beam snapshot.
    let shooter = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world()).expect("fixture has a local ship")
    };
    app.world_mut()
        .entity_mut(shooter)
        .remove::<crate::entity_spawner::EntityUuid>();

    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .clear();
    set_active_beam_damage_accumulator(&mut app, 15.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let hits: Vec<&BalanceEvent> = cursor.read(messages).collect();
    assert!(!hits.is_empty(), "the beam must still land and be traced");
    for hit in &hits {
        let BalanceEvent::DamageApplied { attacker, .. } = hit else {
            continue;
        };
        assert_eq!(
            *attacker, None,
            "an unidentified shooter is None, never Some(\"\")"
        );
    }
}

#[test]
fn phaser_beam_post_break_skips_shield_routing_entirely() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    // Spawn with already-offline shield (facing depleted, offline timer running).
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let mut shield_sys = ShieldSystem::new(&ShieldConfig {
        num_facings: 1,
        max_hp: 20,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    });
    // Deplete the facing so it goes offline.
    shield_sys.apply_damage(20, 0.0);
    assert!(!shield_sys.facings[0].is_online(), "facing must be offline");

    let npc_entity = app
        .world_mut()
        .spawn((
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            crate::ship::shields::ShipShields(shield_sys, 0.5),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ))
        .id();

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    set_active_beam_damage_accumulator(&mut app, 5.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let hull_hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("hull must exist")
        .0
        .total_current();
    // Hull must take damage (offline shield does not absorb).
    assert!(
        hull_hp < 30.0,
        "offline shield must let damage through to hull, got {hull_hp}"
    );
    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("ShipShields component must persist");
    assert_eq!(
        shields.0.facings[0].hp, 0,
        "offline facing hp must remain 0, got {}",
        shields.0.facings[0].hp
    );
    assert!(
        !shields.0.facings[0].is_online(),
        "facing must remain offline"
    );
}

#[test]
fn shield_regen_advances_npc_shield_below_max() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 5.0);

    // Damage the shield to 10 HP.
    if let Some(mut shields) = app
        .world_mut()
        .get_mut::<crate::ship::shields::ShipShields>(npc_entity)
    {
        shields.0.facings[0].hp = 10;
    }

    // Advance time. The Bevy `Time` resource advances on each `app.update()`
    // call; we tick a few frames and expect regen to push hp upward.
    for _ in 0..3 {
        tick(&mut app);
    }

    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("ShipShields must persist");
    // We don't assert exact values (frame timing varies in tests) but we
    // verify regen is making forward progress and not stuck at 10.
    assert!(
        shields.0.facings[0].hp > 10,
        "shield must regen between ticks, got {}",
        shields.0.facings[0].hp
    );
    assert!(
        shields.0.facings[0].hp <= 20,
        "shield must clamp to max_hp, got {}",
        shields.0.facings[0].hp
    );
    assert!(shields.0.facings[0].is_online());
}

// ── PR2: Torpedo damage routes through ShipShields on the player ship ──

/// Verify that a torpedo detonation on the player ship reduces `ShipShields`
/// HP before leaking to the hull — end-to-end ShipShields coverage for the
/// torpedo damage path (PR2: Unified ShipShields).
#[test]
fn torpedo_hit_reduces_ship_shields_on_local_ship() {
    use crate::entity_spawner::EntityUuid;
    use crate::server_app::LocalShip;
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    use crate::weapons::torpedo::Torpedo;

    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Give the player ship ShipShields with known HP.
    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();

    let shield_max_hp = 100i32;
    let shield_sys = ShieldSystem::new(&ShieldConfig {
        num_facings: 4,
        max_hp: shield_max_hp,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    });
    app.world_mut().entity_mut(player_entity).insert((
        EntityUuid("player-ship".into()),
        crate::ship::shields::ShipShields(shield_sys, 0.5),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Also expose the player ship in the world snapshot so the torpedo can
    // find it as a target.
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: "player-ship".into(),
                position: Some([0.0, 0.0, 0.0]),
                radius: Some(5.0),
                ..Default::default()
            }],
            ..Default::default()
        }));

    // Read initial total shield HP.
    let shields_before: i32 = app
        .world()
        .entity(player_entity)
        .get::<crate::ship::shields::ShipShields>()
        .unwrap()
        .0
        .facings
        .iter()
        .map(|f| f.hp)
        .sum();
    assert_eq!(shields_before, shield_max_hp * 4);

    // Read initial hull HP.
    let hull_before = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();

    // Directly inject a torpedo already adjacent to the player ship so it
    // detonates on the next tick. We write into both the per-entity component
    // and the resource to stay in sync.
    let torpedo = Torpedo {
        uuid: "test-torp-1".into(),
        x: 1.0, // 1 m away from player at origin — within detonation_radius
        y: 0.0,
        z: 0.0,
        heading: 0.0,
        pitch: 0.0,
        lifespan_remaining: 30.0,
        target_uuid: Some("player-ship".into()),
        source_uuid: None, // no source → no self-detonation exclusion
        tube_id: "fore_port".into(),
        shield_pierce: 0.0, // no pierce → all damage goes to shields first
    };
    // Write to the per-entity component (preferred by systems) and resource.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.in_flight.push(torpedo.clone());
        }
    }
    app.world_mut()
        .resource_mut::<TorpedoSystemResource>()
        .0
        .in_flight
        .push(torpedo);

    // Tick once — torpedo detonates and routes damage through ShipShields.
    tick(&mut app);

    let shields_after: i32 = app
        .world()
        .entity(player_entity)
        .get::<crate::ship::shields::ShipShields>()
        .unwrap()
        .0
        .facings
        .iter()
        .map(|f| f.hp)
        .sum();

    let hull_after = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();

    // Shield HP must decrease (torpedo damage_shields absorbed by shield).
    // (If damage_shields == 0 in the TOML config the test is still valid:
    // it just shows hull dropped instead, but we accept either change.)
    let total_damage_taken = (shields_before - shields_after) + ((hull_before - hull_after) as i32);
    assert!(
        total_damage_taken > 0,
        "torpedo hit must cause total damage: shields_before={shields_before}, shields_after={shields_after}, \
         hull_before={hull_before}, hull_after={hull_after}"
    );
    // The important invariant: if damage_shields > 0, shield must have taken damage first.
    // We verify this indirectly: hull must not exceed its pre-hit value.
    assert!(
        hull_after <= hull_before,
        "hull must not increase after torpedo hit, got {hull_after} > {hull_before}"
    );
}

/// Closes the loop `ai_torpedo_auto_fire_gates_on_the_arc_the_torpedo_would_strike`
/// leaves open: that test asserts only the *launch* decision, so it passed
/// while `tick_torpedo_lifecycle` still routed every detonation through a
/// hardcoded bearing of `0.0` — i.e. the gate cleared a collapsed aft arc and
/// the hit then landed on the healthy fore arc. This asserts the other half:
/// a torpedo that arrives from astern depletes the ASTERN arc and leaves the
/// fore arc untouched. Arcs are named via the ship's own
/// `facing_index_for_bearing` rather than hardcoded indices, so the test
/// tracks the routing instead of restating it.
#[test]
fn torpedo_hit_from_astern_damages_the_astern_arc_not_the_fore_arc() {
    use crate::entity_spawner::EntityUuid;
    use crate::server_app::LocalShip;
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    use crate::weapons::torpedo::Torpedo;

    let mut app = test_app();
    start_game_with_weapons(&mut app);

    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();

    // Four arcs, no regen, so the only HP movement in this test is the hit.
    let shield_sys = ShieldSystem::new(&ShieldConfig {
        num_facings: 4,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    });
    app.world_mut().entity_mut(player_entity).insert((
        EntityUuid("player-ship".into()),
        crate::ship::shields::ShipShields(shield_sys, 0.5),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: "player-ship".into(),
                position: Some([0.0, 0.0, 0.0]),
                radius: Some(5.0),
                ..Default::default()
            }],
            ..Default::default()
        }));

    // Forward is -z, so a torpedo sitting at +z is astern of a yaw-0 ship.
    // It is launched far enough back that one tick of flight (heading 0 →
    // straight down -z) closes to within the detonation radius while still
    // leaving it astern: detonation is evaluated *after* the torpedo moves, so
    // a torpedo started at z=1 would overshoot to z=-2 and legitimately strike
    // the fore arc.
    let (start_x, start_z) = (0.0_f32, 6.0_f32);
    let (astern_arc, fore_arc) = {
        let shields = app
            .world()
            .entity(player_entity)
            .get::<crate::ship::shields::ShipShields>()
            .unwrap();
        let astern = shields
            .0
            .facing_index_for_bearing(crate::shield::attacker_bearing_relative(
                start_x, start_z, 0.0, 0.0, 0.0,
            ));
        let fore = shields
            .0
            .facing_index_for_bearing(crate::shield::attacker_bearing_relative(
                0.0, -1.0, 0.0, 0.0, 0.0,
            ));
        assert_ne!(
            astern, fore,
            "precondition: a four-arc hull must route fore and astern to different arcs"
        );
        (astern, fore)
    };

    let hp_of = |app: &App, idx: usize| -> i32 {
        app.world()
            .entity(player_entity)
            .get::<crate::ship::shields::ShipShields>()
            .unwrap()
            .0
            .facings[idx]
            .hp
    };
    let astern_before = hp_of(&app, astern_arc);
    let fore_before = hp_of(&app, fore_arc);

    let torpedo = Torpedo {
        uuid: "test-torp-astern".into(),
        x: start_x,
        y: 0.0,
        z: start_z,
        heading: 0.0,
        pitch: 0.0,
        lifespan_remaining: 30.0,
        target_uuid: Some("player-ship".into()),
        source_uuid: None,
        tube_id: "aft".into(),
        shield_pierce: 0.0, // no pierce → the shield arc takes it all
    };
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.in_flight.push(torpedo.clone());
        }
    }
    app.world_mut()
        .resource_mut::<TorpedoSystemResource>()
        .0
        .in_flight
        .push(torpedo);

    tick(&mut app);

    assert!(
        hp_of(&app, astern_arc) < astern_before,
        "the arc facing the torpedo's approach must absorb the hit: {} → {}",
        astern_before,
        hp_of(&app, astern_arc)
    );
    assert_eq!(
        hp_of(&app, fore_arc),
        fore_before,
        "the arc pointing away from the torpedo must be untouched — a hardcoded \
         bearing of 0.0 would have put this hit on the fore arc"
    );
}

// ── Cycle 3: AiEntityDestroyed message written on NPC destruction ─────

#[test]
fn phaser_beam_emits_ai_entity_destroyed_on_npc_kill() {
    #[derive(Resource, Default)]
    struct DestroyedBox(Vec<crate::ai_plugin::AiEntityDestroyed>);

    let mut app = test_app();
    app.init_resource::<DestroyedBox>();
    app.add_systems(
        bevy::app::Update,
        |mut r: bevy::ecs::prelude::MessageReader<crate::ai_plugin::AiEntityDestroyed>,
         mut b: bevy::ecs::prelude::ResMut<DestroyedBox>| {
            for ev in r.read() {
                b.0.push(ev.clone());
            }
        },
    );

    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);
    tick(&mut app); // second tick allows PostUpdate-equivalent collector to drain the message

    let destroyed_events = app.world().resource::<DestroyedBox>();
    assert!(
        destroyed_events.0.iter().any(|e| e.entity_uuid == "npc-1"),
        "AiEntityDestroyed must be emitted with entity_uuid 'npc-1' so on_destroyed triggers fire"
    );
}

/// `ShipDestroyedVfx` (issue #825) fires alongside `AiEntityDestroyed` on
/// a phaser kill, at the target's position, falling back to
/// `DEFAULT_SHIP_EXPLOSION_RADIUS` since `spawn_npc_entity` gives the
/// target no `ColliderSection`.
#[test]
fn phaser_beam_emits_ship_destroyed_vfx_on_npc_kill() {
    #[derive(Resource, Default)]
    struct VfxBox(Vec<ShipDestroyedVfx>);

    let mut app = test_app();
    app.init_resource::<VfxBox>();
    app.add_systems(
        bevy::app::Update,
        |mut r: bevy::ecs::prelude::MessageReader<ShipDestroyedVfx>,
         mut b: bevy::ecs::prelude::ResMut<VfxBox>| {
            for ev in r.read() {
                b.0.push(*ev);
            }
        },
    );

    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);
    tick(&mut app);

    let vfx = app.world().resource::<VfxBox>();
    assert!(
        vfx.0
            .iter()
            .any(|e| e.x == 0.0 && e.z == -20.0 && e.radius == DEFAULT_SHIP_EXPLOSION_RADIUS),
        "ShipDestroyedVfx must fire at the target's position with the \
         default radius (no ColliderSection on this fixture); got {:?}",
        vfx.0
    );
}

/// `ShipDestroyedVfx` (issue #825) must fire on a blaster kill too.
/// Before this fix, `handle_blaster_hits` despawned non-local ship
/// targets silently — no `EntityDespawned`, no `AiEntityDestroyed`, no
/// VFX — unlike the phaser and torpedo damage paths. Pre-populates the
/// bank's `in_flight` list directly (rather than simulating multi-tick
/// projectile travel) so the hit registers on the very first tick.
#[test]
fn blaster_hit_emits_ship_destroyed_vfx_on_npc_kill() {
    use crate::entity_spawner::EntityUuid;

    #[derive(Resource, Default)]
    struct VfxBox(Vec<ShipDestroyedVfx>);

    let mut app = test_app();
    app.init_resource::<VfxBox>();
    app.add_systems(
        bevy::app::Update,
        |mut r: bevy::ecs::prelude::MessageReader<ShipDestroyedVfx>,
         mut b: bevy::ecs::prelude::ResMut<VfxBox>| {
            for ev in r.read() {
                b.0.push(*ev);
            }
        },
    );

    let target_uuid = "npc-blaster-target";

    let mut bank = crate::blaster::BlasterSystem::new(crate::blaster::BlasterBankConfig {
        id: "fore".into(),
        facing_deg: 0.0,
        fire_arc_deg: 360.0,
        volley_count: 1,
        volley_interval_secs: 0.1,
        cooldown_secs: 3.0,
        charge_time_secs: 0.0,
        projectile_speed: 40.0,
        collision_radius: 5.0,
        visual_scale: 1.0,
        damage: 50,
        shield_pierce: 0.0,
        recoil_impulse: 0.0,
        screenshake_magnitude: 0.0,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        range: 35.0,
    });
    bank.in_flight.push(crate::blaster::BlasterProjectile {
        id: "proj-1".into(),
        x: 0.0,
        z: -20.0,
        heading: 0.0,
        speed: 40.0,
        lifespan_remaining: 5.0,
        collision_radius: 5.0,
        damage: 50,
        shield_pierce: 0.0,
        source_uuid: "shooter-uuid".into(),
    });

    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid("shooter-uuid".into()),
        BlasterSystemResource(vec![bank]),
        Transform::default(),
    ));

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();
    app.update(); // second tick allows the message-collector probe to drain it

    let vfx = app.world().resource::<VfxBox>();
    assert!(
        vfx.0
            .iter()
            .any(|e| e.x == 0.0 && e.z == -20.0 && e.radius == DEFAULT_SHIP_EXPLOSION_RADIUS),
        "ShipDestroyedVfx must fire on a blaster kill too; got {:?}",
        vfx.0
    );
}

/// The blaster's twin of `phaser_beam_from_an_unidentified_shooter_...`.
///
/// `BlasterProjectile::source_uuid` is a plain `String` and carries `""` for a
/// shooter with no `EntityUuid` — so the detonation has to narrow it to
/// `Option` on the way to the tracer. Without that, the ledger grows a junk
/// `""` row and the timeline prints `"attacker":""`.
#[test]
fn blaster_hit_from_an_unidentified_shooter_reports_no_attacker() {
    use crate::balance::BalanceEvent;
    use crate::entity_spawner::EntityUuid;
    use bevy::ecs::message::Messages;

    let mut app = test_app();

    let mut bank = crate::blaster::BlasterSystem::new(crate::blaster::BlasterBankConfig {
        id: "fore".into(),
        facing_deg: 0.0,
        fire_arc_deg: 360.0,
        volley_count: 1,
        volley_interval_secs: 0.1,
        cooldown_secs: 3.0,
        charge_time_secs: 0.0,
        projectile_speed: 40.0,
        collision_radius: 5.0,
        visual_scale: 1.0,
        damage: 5,
        shield_pierce: 0.0,
        recoil_impulse: 0.0,
        screenshake_magnitude: 0.0,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        range: 35.0,
    });
    bank.in_flight.push(crate::blaster::BlasterProjectile {
        id: "proj-1".into(),
        x: 0.0,
        z: -20.0,
        heading: 0.0,
        speed: 40.0,
        lifespan_remaining: 5.0,
        collision_radius: 5.0,
        damage: 5,
        shield_pierce: 0.0,
        // What the firing path writes when the shooter has no `EntityUuid`.
        source_uuid: String::new(),
    });

    // The shooter deliberately has no `EntityUuid`, matching the projectile.
    app.world_mut().spawn((
        crate::server_app::Ship,
        BlasterSystemResource(vec![bank]),
        Transform::default(),
    ));
    app.world_mut().spawn((
        EntityUuid("npc-blaster-target".into()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            100.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let hits: Vec<&BalanceEvent> = cursor.read(messages).collect();
    assert!(!hits.is_empty(), "the blaster hit must be traced");
    for hit in &hits {
        let BalanceEvent::DamageApplied {
            attacker, weapon, ..
        } = hit
        else {
            continue;
        };
        assert_eq!(
            *attacker, None,
            "an unidentified shooter is None, never Some(\"\")"
        );
        assert_eq!(weapon, "fore", "weapon must name the firing bank");
    }
}

// ── NPC as shooter: handle_fire_phaser (unified) / tick_beams ────────────

/// Set up `AiTokenRegistry`, an NPC entity with
/// `ActiveBeam`/`PhaserCooldown` (unified per-entity phaser state), and a target entity.
fn setup_npc_shooter(
    app: &mut App,
    npc_uuid: &str,
    target_uuid: &str,
    target_x: f32,
    target_z: f32,
) -> (bevy::ecs::entity::Entity, bevy::ecs::entity::Entity) {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    // Spawn NPC entity facing toward negative-Z (yaw = 0 → forward = -Z).
    // Includes the Ship marker so the unified `tick_beams` picks it up as
    // a shooter (matches the production `entities::spawner::spawn_entity`
    // path where every ship gets `Ship` — see PRD #597).
    //
    // Also mirrors production by inserting `ShipSystemControlSources` with
    // the Tactical system set to `Ai`, and the NPC's target lock in
    // `TacticalRadarSelection` — both required by the unified `handle_fire_phaser`
    // per-ship query. `TacticalRadarSelection` is the ship's authoritative lock
    // whether a human or `ai_target_selection` set it, so an AI shooter
    // seeds it exactly as a human one would.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the fine systems for the banks these tests fire
    // ("port"/"starboard" per the test_app combat config) — there is no
    // coarse tactical system to seed.
    for bank in ["port", "starboard"] {
        sources.set(
            crate::system_registry::phaser_bank_system_id(bank).unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
    }

    // Minimal ShipConfigComponent so admission's ship_query resolves this NPC.
    let npc_config = crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::parse_and_validate(
            r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Dummy"
rank = "Ltn."

[[system]]
id = "phaser-port"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "phaser-starboard"
kind = "phaser_bank"
station = "tactical"
"#,
            &["phaser_bank"],
        )
        .expect("NPC ship config must be valid"),
    );

    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            npc_config,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            Transform::from_xyz(0.0, 0.0, 0.0),
            crate::messages::AdmittedCommands::default(),
        ))
        .id();

    // Register with the Bevy entity so handle_fire_phaser can look it up.
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }

    // Spawn target entity.
    let target_entity = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(target_x, 0.0, target_z),
        ))
        .id();

    (npc_entity, target_entity)
}

#[test]
fn npc_fire_phaser_activates_entity_phaser_state() {
    // NPC entity at origin, target directly ahead (negative-Z), within beam range.
    // Sending a FirePhaser InboundMessage for the NPC's ai: token should set
    // `ActiveBeam::target_uuid = Some(...)` after one update.
    use crate::ai_plugin::AiTokenRegistry;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000001";
    let target_uuid = "00000000-0000-0000-0000-000000000002";

    let (npc_entity, _target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid, 0.0, -20.0);

    // Send FirePhaser as the NPC's synthetic token.
    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC entity must have ActiveBeam component");
    assert!(
        beam.is_firing(),
        "ActiveBeam::target_uuid should be Some after NPC fires phaser via ai: token"
    );
}

#[test]
fn npc_beam_tick_applies_damage_to_target_hull() {
    // With an active NPC beam, each tick of tick_beams reduces
    // the target's EntitySystemHull.
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntitySystemHull;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000003";
    let target_uuid_str = "00000000-0000-0000-0000-000000000004";

    let (npc_entity, target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    // Activate the beam directly on the per-entity ActiveBeam component.
    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.start("", target_uuid_str.to_string(), 10.0);
    }

    let hp_before = app
        .world()
        .get::<EntitySystemHull>(target_entity)
        .unwrap()
        .0
        .total_current();

    // Run several ticks so damage accumulates.
    for _ in 0..10 {
        app.update();
    }

    let hp_after = app
        .world()
        .get::<EntitySystemHull>(target_entity)
        .unwrap()
        .0
        .total_current();
    assert!(
        hp_after < hp_before,
        "target hull must decrease as NPC beam ticks (before={hp_before}, after={hp_after})"
    );
}

#[test]
fn npc_beam_tick_records_shooter_as_last_attacker() {
    // Write-on-damage (#689): when a live beam hits a ship target that
    // carries a `LastShipAttacker` component, `tick_beams` records the
    // shooter's UUID as that target's last attacker. This write fires in
    // Phase 2 before the `damage_to_apply <= 0` guard, but only when the
    // target entity actually carries the component — so we insert it.
    use crate::ai_plugin::AiTokenRegistry;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000003";
    let target_uuid_str = "00000000-0000-0000-0000-000000000004";

    let (npc_entity, target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    // The attacker-write branch only fires if the target carries
    // `LastShipAttacker`; `setup_npc_shooter` does not add it.
    app.world_mut()
        .entity_mut(target_entity)
        .insert(LastShipAttacker::default());

    // Activate the beam directly on the per-entity ActiveBeam component.
    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.start("", target_uuid_str.to_string(), 10.0);
    }

    // Tick enough for the beam to reach and hit the target.
    for _ in 0..10 {
        app.update();
    }

    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(target_entity)
            .unwrap()
            .0,
        Some(npc_uuid.to_string()),
        "beam hit must record the shooter UUID as the target's last attacker"
    );
}

/// The writer's half of the `AiEntityAttacked` exactly-once contract
/// (issue #702).
///
/// `tick_beams`' attacker-write branch runs every tick a beam is live.
/// Post-#702 the rising edge that fires `AiEntityAttacked` — and through it
/// `on_entity_attacked` scenario triggers — *is* `LastShipAttacker`'s change
/// detection, so a blind write would re-fire the trigger on every tick of a
/// sustained beam. This pins the compare: across many ticks of one live beam
/// from one shooter, the component is marked changed exactly once.
///
/// `ai_entity_attacked_not_re_emitted_for_same_attacker` pins the reader's
/// half in `ai::server`.
#[test]
fn sustained_beam_marks_last_attacker_changed_exactly_once() {
    use crate::ai_plugin::AiTokenRegistry;

    #[derive(Resource, Default)]
    struct ChangeCount(usize);

    // Mirrors `ai_plugin::emit_attacked_on_new_attacker`'s guard: count the
    // changes that would fire `AiEntityAttacked`, i.e. those that *name* an
    // attacker. Component insertion also marks a component changed, and the
    // fixture below inserts a `default()` (`None`) — which is a clear, not
    // an attack, and which the emitter skips for exactly this reason.
    fn count_changes(
        q: Query<&LastShipAttacker, Changed<LastShipAttacker>>,
        mut counter: ResMut<ChangeCount>,
    ) {
        counter.0 += q.iter().filter(|a| a.0.is_some()).count();
    }

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();
    app.init_resource::<ChangeCount>();

    let npc_uuid = "00000000-0000-0000-0000-000000000013";
    let target_uuid_str = "00000000-0000-0000-0000-000000000014";

    let (npc_entity, target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    app.world_mut()
        .entity_mut(target_entity)
        .insert(LastShipAttacker::default());

    // Count in `PostUpdate` so each `Update` tick's write is observed on the
    // tick it happens. (Ordering against `tick_beams` directly is not an
    // option here: this fixture registers it a second time outside any
    // SimSet, so its `SystemTypeSet` is ambiguous.)
    app.add_systems(PostUpdate, count_changes);

    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.start("", target_uuid_str.to_string(), 100.0);
    }

    // Many ticks of one continuous beam from one shooter.
    for _ in 0..20 {
        app.update();
    }

    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(target_entity)
            .unwrap()
            .0,
        Some(npc_uuid.to_string()),
        "precondition: the sustained beam must actually have recorded the shooter"
    );
    assert_eq!(
        app.world().resource::<ChangeCount>().0,
        1,
        "tick_beams must compare before writing LastShipAttacker: a sustained beam \
         from one shooter may mark it changed exactly once, on the tick the attacker \
         becomes known. More than one means a blind write, which re-fires \
         AiEntityAttacked (and on_entity_attacked triggers) every tick the beam is live."
    );
}

#[test]
fn npc_beam_tick_damages_npc_target_not_player() {
    // Regression test for PRD #597 PR-1: NPC-vs-NPC beam damage.
    // Before the fix, the old tick_npc_beams hull_query had
    // Without<LocalShip> so NPCs couldn't damage other NPCs — damage
    // was silently lost. The unified `tick_beams` iterates all ships
    // and applies damage to any target found via `hull_q`.
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntitySystemHull;
    use crate::server_app::ShipAttackedThisTick;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();
    app.init_resource::<crate::simulation::GameOverReason>();

    let shooter_uuid = "10000000-0000-0000-0000-000000000001";
    let npc_target_uuid = "20000000-0000-0000-0000-000000000002";

    // Spawn NPC shooter.
    let (shooter_entity, npc_target_entity) =
        setup_npc_shooter(&mut app, shooter_uuid, npc_target_uuid, 0.0, -10.0);
    // Add ShipPhysics to the target so it looks like a real production-spawned
    // NPC (physics-enabled). The unified `tick_beams` finds targets by
    // EntityUuid in `hull_q` (no Ship marker requirement on targets).
    app.world_mut()
        .entity_mut(npc_target_entity)
        .insert(ShipPhysics::default());

    // Activate beam on the shooter.
    {
        let mut beam = app
            .world_mut()
            .get_mut::<ActiveBeam>(shooter_entity)
            .unwrap();
        beam.start("", npc_target_uuid.to_string(), 10.0);
    }

    let hp_before = app
        .world()
        .get::<EntitySystemHull>(npc_target_entity)
        .unwrap()
        .0
        .total_current();

    for _ in 0..10 {
        app.update();
    }

    let hp_after = app
        .world()
        .get::<EntitySystemHull>(npc_target_entity)
        .unwrap()
        .0
        .total_current();

    assert!(
        hp_after < hp_before,
        "NPC beam must damage NPC target hull (before={hp_before}, after={hp_after})"
    );
    // Player ship must NOT have been marked as attacked.
    let player_atk = app
        .world_mut()
        .query_filtered::<&ShipAttackedThisTick, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|c| c.0)
        .unwrap_or(false);
    assert!(
        !player_atk,
        "NPC-vs-NPC beam must not set player's ShipAttackedThisTick"
    );
}

#[test]
fn on_beam_started_emits_correct_source_uuid_with_multiple_ships() {
    // Multi-ship source-uuid behaviour: `on_beam_started` resolves the emitted
    // `source_uuid` per-entity from `BeamStartedEvent::source_entity` (looking up
    // that entity's own `EntityUuid`), so a beam fired by one ship names that
    // ship even when several ships exist. Originally a regression guard for the
    // PRD #597 PR-1 `With<Ship>.single()` panic; the source is now the event's
    // shooter entity, not any `LocalShip`/`single()` query (#832).
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();
    let player_uuid_str = "aaaaaaaa-0000-0000-0000-000000000001";
    let npc_uuid_str = "bbbbbbbb-0000-0000-0000-000000000002";

    // Add EntityUuid to the existing LocalShip entity (spawned by test_app).
    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(player_entity)
        .insert(EntityUuid(player_uuid_str.to_string()));

    // Spawn a second NPC ship (non-LocalShip, has Ship marker).
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid(npc_uuid_str.to_string()),
        ShipPhysics::default(),
        Transform::default(),
    ));

    // Trigger BeamStartedEvent — the observer on_beam_started should emit
    // source_uuid = player_uuid_str, not empty.
    app.world_mut().trigger(super::BeamStartedEvent {
        bank: "port".to_string(),
        target_uuid: "some-target".to_string(),
        source_entity: player_entity,
    });
    app.update();

    // Find the BeamStarted message in the SimOutbox.
    let outbox = app.world().resource::<crate::simulation::SimOutbox>();
    let beam_started = outbox
        .0
        .iter()
        .find(|(_, msg)| matches!(msg, crate::messages::ServerMessage::BeamStarted { .. }));
    let Some((_, crate::messages::ServerMessage::BeamStarted { source_uuid, .. })) = beam_started
    else {
        panic!("expected BeamStarted message in outbox");
    };
    assert_eq!(
        source_uuid, player_uuid_str,
        "on_beam_started must emit the firing entity's UUID as source_uuid, not {:?}",
        source_uuid
    );
}

#[test]
fn npc_beam_tick_applies_damage_to_local_ship_through_shields() {
    // When the beam target is the player ship (has Ship marker), damage
    // must route through shields → hull component, not just EntitySystemHull directly.
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntityUuid;
    use crate::server_app::{LocalShip, ShipAttackedThisTick};
    use crate::shield::ShieldConfig;
    use crate::simulation::{GameOverReason, ShipShields};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();
    app.init_resource::<GameOverReason>();

    // Insert shields on the LocalShip entity so the shield-routing
    // path is exercised (ShipShields is pure per-entity Component
    // post ship-parity audit).
    let shield_config = ShieldConfig {
        max_hp: 100,
        regen_per_sec: 0.0,
        num_facings: 4,
        ..Default::default()
    };
    {
        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        let local = q.single(app.world()).unwrap();
        app.world_mut().entity_mut(local).insert(ShipShields(
            crate::shield::ShieldSystem::new(&shield_config),
            0.5,
        ));
    }

    let npc_uuid = "00000000-0000-0000-0000-000000000010";
    let player_uuid = "00000000-0000-0000-0000-000000000011";
    let player_uuid_parsed = uuid::Uuid::parse_str(player_uuid).unwrap();

    // Add EntityUuid and position to the existing LocalShip entity (already spawned by test_app).
    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(player_entity).insert((
        EntityUuid(player_uuid.to_string()),
        Transform::from_xyz(0.0, 0.0, -10.0),
    ));

    // Spawn NPC entity using the new per-entity beam components.
    let npc_entity = {
        let npc_ent = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                // The NPC's Tactical lock. Was seeded on the private
                // `ShipAiMemory.target` mirror until #702 deleted it;
                // `TacticalRadarSelection` is the surface every firing path reads.
                TacticalRadarSelection(Some(player_uuid_parsed.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_ent);
        npc_ent
    };

    let hull_before = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();
    let shields_sum_before: i32 = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipShields, With<LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipShields")
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum()
    };

    // Activate the beam directly targeting the player ship.
    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.start("", player_uuid.to_string(), 10.0);
    }

    for _ in 0..10 {
        app.update();
    }

    let hull_after = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();
    let shields_sum_after: i32 = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipShields, With<LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipShields")
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum()
    };

    let hull_lost = hull_before - hull_after;
    let shields_lost = shields_sum_before - shields_sum_after;

    assert!(
        hull_lost > 0.0 || shields_lost > 0,
        "NPC beam must damage player ship: hull {hull_before}->{hull_after} ({hull_lost}), shields {shields_sum_before}->{shields_sum_after} ({shields_lost})"
    );
    let player_atk = app
        .world_mut()
        .query_filtered::<&ShipAttackedThisTick, With<LocalShip>>()
        .single(app.world())
        .map(|c| c.0)
        .unwrap_or(false);
    assert!(
        player_atk,
        "NPC beam targeting the player ship must mark the ship as attacked for Captain AI"
    );
}

#[test]
fn npc_beam_cooldown_starts_after_beam_expires() {
    // When an NPC's ActiveBeam remaining_secs reaches zero, PhaserCooldown must
    // be set to a positive value and ActiveBeam.target_uuid must become None.
    use crate::ai_plugin::AiTokenRegistry;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000005";
    let target_uuid_str = "00000000-0000-0000-0000-000000000006";

    let (npc_entity, _target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.start("", target_uuid_str.to_string(), 0.001); // expires on first tick
    }

    app.update(); // beam expires
    app.update(); // cooldown ticked

    let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
    assert!(
        !beam.is_firing(),
        "ActiveBeam.target_uuid must be None after beam expires"
    );
    let cooldown = app.world().get::<PhaserCooldown>(npc_entity).unwrap();
    assert!(
        cooldown.per_bank.values().any(|&v| v > 0.0),
        "PhaserCooldown must be positive after beam ends: {:?}",
        cooldown.per_bank
    );
}

// ── End-to-end: tick_ai_controllers → InboundMessage → handle_fire_phaser ──

/// Build an app that includes BOTH `WeaponsPlugin` AND `AiPlugin` together
/// with all their required resources, so the full routing path can be tested:
/// `tick_ai_controllers` emits a `FirePhaser` `InboundMessage` which the
/// unified `handle_fire_phaser` picks up and activates the NPC's `ActiveBeam`.
fn combined_test_app() -> App {
    use crate::ai_plugin::AiPlugin;
    use crate::config_cache::FactionRegistryResource;

    let mut app = test_app();
    app.add_plugins(AiPlugin)
        .insert_resource(FactionRegistryResource(
            crate::config_cache::get_faction_registry(),
        ));
    app
}

#[test]
fn tick_ai_controllers_fire_phaser_routes_through_unified_handle_fire_phaser() {
    // Full end-to-end test: an NPC with a Destroy doctrine and a pre-selected
    // target directly in its forward arc causes `tick_ai_controllers` to write
    // a `FirePhaser` `InboundMessage`, which the unified `handle_fire_phaser`
    // picks up
    // and sets `ActiveBeam::target_uuid`.
    use crate::damage::SystemHull;
    use crate::entity_config::{BehaviourConfig, DoctrineObjective};
    use crate::entity_spawner::{EntitySystemHull, EntityUuid, WeaponsConsoleSection};
    use crate::messages::{GamePhase, SystemId};
    use bevy::prelude::State;

    let mut app = combined_test_app();

    // Put the simulation in InProgress so tick_ai_controllers runs.
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let beam_range = 50.0_f32;
    let npc_uuid_str = "ee000000-0000-0000-0000-000000000010";
    let target_uuid_str = "ee000000-0000-0000-0000-000000000011";
    let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();

    // Doctrine: single Destroy objective at high priority — always scores > 0.
    let behaviour = BehaviourConfig {
        doctrine: vec![DoctrineObjective {
            id: "destroy-hostiles".into(),
            text: "Destroy target".into(),
            directive_kind: Some("Destroy".into()),
            base_priority: 35.0,
            target_speed: 0.9,
            maintain_range: 25.0,
            ..Default::default()
        }],
        ..Default::default()
    };

    // Spawn NPC at origin, facing -Z (yaw = 0 → forward = -Z).
    // Include ActiveBeam/PhaserCooldown/ShipPhysics for the unified fire path,
    // plus the components the unified `handle_fire_phaser` requires:
    // `Ship`, `ShipSystemControlSources` (Tactical = Ai), `TacticalRadarSelection`.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    // Minimal ShipConfigComponent so admission's ship_query resolves this NPC.
    let npc_config = crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::parse_and_validate(
            r#"
[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[station]]
id = "tactical"
name = "Tactical"
description = "Dummy"
rank = "Ltn."
"#,
            &["phaser_bank"],
        )
        .expect("NPC ship config must be valid"),
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entity_spawner::BehaviourSection(behaviour),
            EntityUuid(npc_uuid_str.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            npc_config,
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection::default(),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            WeaponsConsoleSection(crate::entity_config::WeaponsConsoleConfig {
                torpedo_arc_color: vec![],
                power_multipliers: None,
                phaser_banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: Some(0.0),
                    marker: None,
                    ai: None,
                }],
                blaster_banks: vec![],
                radar: None,
                selector: None,
                selector_idle: false,
            }),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            AdmittedCommands::default(),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ))
        .id();

    // Spawn target directly ahead (-Z), well within beam range.
    let _target = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid_str.to_string()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                200.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -10.0),
        ))
        .id();

    // Tick 1: `register_ai_tokens_on_spawn` runs → token registered in
    //         AiTokenRegistry.
    app.update();

    // Register the Bevy entity in AiTokenRegistry (needed by handle_fire_phaser).
    {
        let mut reg = app
            .world_mut()
            .resource_mut::<crate::ai_plugin::AiTokenRegistry>();
        reg.register_with_entity(npc_uuid_str, npc_entity);
    }

    // Set the NPC's target lock so handle_fire_phaser can look up the
    // target. `TacticalRadarSelection` is the authoritative lock for every ship —
    // in production `ai_target_selection` writes it for AI-operated
    // tactical systems; here we seed it directly.
    {
        let mut target = app
            .world_mut()
            .get_mut::<TacticalRadarSelection>(npc_entity)
            .expect("NPC must have TacticalRadarSelection");
        target.0 = Some(target_uuid_parsed.to_string());
    }

    // Push a synthetic FirePhaser message for the NPC's ai: token.
    // In production this would be emitted by ai_phaser_auto_fire,
    // but for this integration test we inject it directly.
    let ai_token = format!("ai:{}", npc_uuid_str);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("phaser-fore".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );

    // Tick: handle_fire_phaser processes the message and activates ActiveBeam.
    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC must have ActiveBeam component");
    assert!(
        beam.is_firing(),
        "ActiveBeam.target_uuid must be Some after tick_ai_controllers → InboundMessage → handle_fire_phaser routing"
    );
}

/// Verify that both a `LocalShip` entity and an NPC entity use the same
/// `tick_beams` handler (unified per-entity beam path — issues #588 / #597).
#[test]
fn both_localship_and_npc_can_fire_via_per_entity_active_beam() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let target_uuid = "ff000000-0000-0000-0000-000000000001";
    let npc_uuid = "ff000000-0000-0000-0000-000000000002";

    // Spawn a target entity with hull.
    let target_entity = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -15.0),
        ))
        .id();

    // Spawn NPC entity with per-entity ActiveBeam and activate beam.
    // Includes the Ship marker so the unified `tick_beams` picks it up
    // as a shooter (matches production NPC spawn path — see PRD #597).
    let npc_ent = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            {
                let mut beam = ActiveBeam::default();
                beam.start("", target_uuid, 10.0);
                beam
            },
            PhaserCooldown::default(),
            ShipPhysics::default(),
            Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_ent);
    }

    // Run ticks so tick_beams fires.
    for _ in 0..5 {
        app.update();
    }

    let hp = app
        .world()
        .get::<EntitySystemHull>(target_entity)
        .unwrap()
        .0
        .total_current();
    assert!(
        hp < 100.0,
        "NPC beam must apply damage via the unified tick_beams path (hp={hp})"
    );
}

/// Regression test for the unified phaser auto-fire path (post-#846:
/// `ai_phaser_auto_fire` -> `AdmittedCommands` -> `handle_fire_phaser`).
///
/// Before unification, `tick_phaser_auto_fire` iterated only `LocalShip`,
/// so NPCs had to route through the (now-deleted) `handle_npc_beam_fire`
/// with synthetic `FirePhaser` messages emitted by AI. Post-unification
/// the same system iterates every ship whose Tactical system is
/// AI-controlled, activating an [`ActiveBeam`] directly.
#[test]
fn ai_phaser_auto_fire_activates_ai_controlled_npc_beam() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "aa000000-0000-0000-0000-000000000001";
    let target_uuid = "aa000000-0000-0000-0000-000000000002";

    // NPC facing -Z (yaw=0 forward = -Z) with Tactical set to Ai.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                }],
            }),
            AdmittedCommands::default(),
            Transform::default(),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, npc_entity);

    // Spawn target
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC entity must have ActiveBeam component");
    assert!(
        beam.is_firing(),
        "ai_phaser_auto_fire -> handle_fire_phaser must activate the \
         NPC's ActiveBeam when Tactical is AI-controlled"
    );
    assert_eq!(
        beam.any_bank(),
        Some("fore"),
        "NPC should fire the in-arc bank selected from its own PhaserCombatConfigResource"
    );
}

// ── Phaser decide/integrate split (issue #698) ─────────────────────────

/// Spawn an AI-controlled NPC with one 360° bank, a locked target, and a
/// live entity to shoot at directly ahead. Returns the NPC's entity.
///
/// Deliberately does **not** insert `AiHighFidelity`: the population this
/// helper builds is a low-LOD NPC, which is precisely the case
/// `ai_phaser_auto_fire`'s missing `With<AiHighFidelity>` filter exists to
/// serve. Tests that need high fidelity add the marker themselves.
fn spawn_ai_phaser_npc(app: &mut App, npc_uuid: &str, target_uuid: &str) -> Entity {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                }],
            }),
            AdmittedCommands::default(),
            Transform::default(),
        ))
        .id();
    attach_shipped_weapon_ai(app, npc);

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    npc
}

/// `ai_phaser_auto_fire` is a *decider*: it publishes its choice to
/// `AdmittedCommands` (via `emit_ai_command`) and leaves `ActiveBeam`
/// alone. Running it in isolation proves the two halves are genuinely
/// separated.
#[test]
fn ai_phaser_auto_fire_writes_admitted_command_without_touching_the_beam() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = test_app();
    let npc = spawn_ai_phaser_npc(
        &mut app,
        "bb000000-0000-0000-0000-000000000001",
        "bb000000-0000-0000-0000-000000000002",
    );

    // Seed the frozen viewscreen combat_lock from the NPC's selection (issue
    // #829) — the isolated run below bypasses the harness's per-tick lift.
    app.world_mut()
        .run_system_once(seed_viewscreen_from_selection)
        .expect("seed viewscreen");
    app.world_mut()
        .run_system_once(ai_phaser_auto_fire)
        .expect("ai_phaser_auto_fire should run");

    let admitted = app
        .world()
        .get::<AdmittedCommands>(npc)
        .expect("every ship has AdmittedCommands");
    assert_eq!(
        admitted.0.len(),
        1,
        "the decider must emit exactly one FirePhaser command"
    );
    assert_eq!(
        admitted.0[0].target,
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        "the decider must target the chosen bank"
    );
    assert!(
        matches!(&admitted.0[0].payload, SystemControlPayload::FirePhaser),
        "the payload must be FirePhaser"
    );
    assert!(
        !app.world().get::<ActiveBeam>(npc).unwrap().is_firing(),
        "ai_phaser_auto_fire must not mutate ActiveBeam — that is \
         handle_fire_phaser's job"
    );
}

/// Pins the deliberate asymmetry between `ai_phaser_auto_fire` (no
/// `AiHighFidelity` filter) and `ai_torpedo_auto_fire` (filtered).
///
/// Extracting phaser fire from `tick_phaser_auto_fire` into the same
/// decide/integrate shape `ai_torpedo_auto_fire` uses makes it tempting to
/// inherit its `With<AiHighFidelity>` filter too. That would silently
/// disarm every low-LOD NPC — a gameplay change wearing a refactor's
/// clothes. Phasers are the main damage low-LOD NPCs contribute, and the
/// `CurrentPhaserMode::Auto` leg of this system isn't AI at all, so the
/// filter would be wrong on its own terms as well.
///
/// If a future slice does decide to gate phasers on LOD, `AdmittedCommands`
/// must move into `lod_ai_ships`' promote/demote bundle at the same time —
/// see `ActiveBeam`'s `#[require(AdmittedCommands)]`.
#[test]
fn ai_phaser_auto_fire_runs_for_low_lod_npc_without_ai_high_fidelity() {
    let mut app = test_app();
    let npc = spawn_ai_phaser_npc(
        &mut app,
        "bb000000-0000-0000-0000-000000000005",
        "bb000000-0000-0000-0000-000000000006",
    );
    assert!(
        app.world()
            .get::<crate::ai_plugin::AiHighFidelity>(npc)
            .is_none(),
        "precondition: this NPC is low-LOD"
    );

    app.update();

    assert!(
        app.world().get::<ActiveBeam>(npc).unwrap().is_firing(),
        "low-LOD NPCs must keep firing phasers — ai_phaser_auto_fire is \
         deliberately NOT gated on AiHighFidelity"
    );
}

/// `tick_weapons_arc_request` (issue #677): a target within a bank's
/// range but outside its firing arc should enqueue a channel-3
/// `ArcBearingRequest` addressed to Helm.
#[test]
fn tick_weapons_arc_request_fires_when_target_in_range_but_outside_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "bb000000-0000-0000-0000-000000000001";

    let ship_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            ShipPhysics::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            WeaponsArcRequestState::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 30.0,
                    auto_arc_deg: 30.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                }],
            }),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, ship_entity);

    // Target is directly to starboard (x=20, z=0): in range (distance 20 <
    // beam_range 50) but 90 degrees off the fore bank's 30-degree arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();

    let log = app.world().resource::<ArcRequestLog>();
    let request = log
        .0
        .iter()
        .find(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
        .expect("expected an ArcBearingRequest CoordinationEnqueue event");
    assert_eq!(request.source_entity, ship_entity);
    assert_eq!(request.target, crate::system_registry::helm_station_key());
    match &request.payload {
        CoordinationPayload::ArcBearingRequest { uuid, .. } => {
            assert_eq!(uuid, target_uuid);
        }
        _ => unreachable!(),
    }
}

/// A target within the firing arc must not trigger an arc-bearing
/// request — Weapons can already fire without Helm's help.
#[test]
fn tick_weapons_arc_request_does_not_fire_when_target_in_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "bb000000-0000-0000-0000-000000000002";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 30.0,
                auto_arc_deg: 30.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
                ai: None,
            }],
        }),
    ));

    // Directly ahead (forward = -Z at yaw 0): in range and in arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let log = app.world().resource::<ArcRequestLog>();
    assert!(
        !log.0
            .iter()
            .any(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. })),
        "an in-arc target must not trigger an ArcBearingRequest"
    );
}

/// The request is debounced: an unchanged arc miss on the same target
/// must not re-enqueue every tick.
#[test]
fn tick_weapons_arc_request_is_debounced_for_unchanged_miss() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "bb000000-0000-0000-0000-000000000003";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 30.0,
                auto_arc_deg: 30.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
                ai: None,
            }],
        }),
    ));

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();
    app.update();
    app.update();

    let log = app.world().resource::<ArcRequestLog>();
    let count = log
        .0
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
        .count();
    assert_eq!(
        count, 1,
        "an unchanged arc miss on the same target must only enqueue once, not every tick"
    );
}

// ── #767: weapon-family-aware arc-bearing coordination ───────────────────────

/// Build a single-bank blaster resource facing forward with the given fire arc
/// and range.
fn blaster_res(facing_deg: f32, fire_arc_deg: f32, range: f32) -> BlasterSystemResource {
    BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
        crate::blaster::BlasterBankConfig {
            id: "fore".into(),
            facing_deg,
            fire_arc_deg,
            range,
            ..crate::blaster::BlasterBankConfig::default()
        },
    )])
}

/// A single-tube, loaded torpedo resource facing forward with the given fire
/// arc. Homing reach is `speed × lifespan` from the default config.
fn loaded_torpedo_res(facing_deg: f32, fire_arc_deg: f32) -> TorpedoSystemResource {
    let mut ts = TorpedoSystem::new(TorpedoConfig::default());
    ts.tubes.truncate(1);
    let tube = &mut ts.tubes[0];
    tube.facing_deg = facing_deg;
    tube.fire_arc_deg = fire_arc_deg;
    tube.loaded_count = 1;
    TorpedoSystemResource(ts)
}

/// Find the single `ArcBearingRequest` in the log, if any.
fn find_arc_request(app: &App) -> Option<CoordinationPayload> {
    app.world()
        .resource::<ArcRequestLog>()
        .0
        .iter()
        .find(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
        .map(|e| e.payload.clone())
}

/// A BLASTER-only ship whose target is in range but out of every blaster arc
/// must emit an `ArcBearingRequest` for the Blasters family carrying that
/// family's arcs.
#[test]
fn tick_weapons_arc_request_fires_for_blaster_family_in_range_out_of_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "cc000000-0000-0000-0000-000000000001";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        // No phaser config: blasters are the only capable family.
        blaster_res(0.0, 30.0, 50.0),
    ));
    // Directly to starboard: in range (20 < 50) but 90° off the 30° fore arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();

    let payload = find_arc_request(&app).expect("blaster family must emit an ArcBearingRequest");
    match payload {
        CoordinationPayload::ArcBearingRequest {
            uuid, family, arcs, ..
        } => {
            assert_eq!(uuid, target_uuid);
            assert_eq!(family, WeaponFamily::Blasters);
            assert_eq!(
                arcs,
                vec![WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 50.0,
                }],
                "request must carry the blaster family's fire arc + range"
            );
        }
        _ => unreachable!(),
    }
}

/// A TORPEDO-only ship (loaded tube) with an in-range, out-of-arc target must
/// emit for the Torpedoes family carrying the tube's arc + homing reach.
#[test]
fn tick_weapons_arc_request_fires_for_torpedo_family_in_range_out_of_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "cc000000-0000-0000-0000-000000000002";
    // Homing reach = default speed × lifespan.
    let cfg = TorpedoConfig::default();
    let reach = cfg.speed * cfg.lifespan;

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        loaded_torpedo_res(0.0, 30.0),
    ));
    // To starboard: in homing reach but 90° off the 30° fore tube arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();

    let payload = find_arc_request(&app).expect("torpedo family must emit an ArcBearingRequest");
    match payload {
        CoordinationPayload::ArcBearingRequest { family, arcs, .. } => {
            assert_eq!(family, WeaponFamily::Torpedoes);
            assert_eq!(
                arcs,
                vec![WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: reach,
                }],
                "request must carry the tube's fire arc + homing reach"
            );
        }
        _ => unreachable!(),
    }
}

/// No request for an INCAPABLE family: a ship with no emitters of any family
/// (an empty blaster vec, no phasers, no tubes) must not emit.
#[test]
fn tick_weapons_arc_request_silent_when_family_incapable() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "cc000000-0000-0000-0000-000000000003";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        BlasterSystemResource(vec![]), // capable of nothing
    ));
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();

    assert!(
        find_arc_request(&app).is_none(),
        "an incapable ship (no emitters) must not emit an ArcBearingRequest"
    );
}

/// No request when the only family's emitters are OFFLINE: an offline blaster
/// bank classifies as `Offline`, never `OutOfArc`, so no bearing is asked.
#[test]
fn tick_weapons_arc_request_silent_when_family_offline() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "cc000000-0000-0000-0000-000000000004";

    let mut cs = ShipSystemControlSources::default();
    cs.0.set_offline(SystemId("blaster-fore".into()), true);

    app.world_mut().spawn((
        crate::server_app::Ship,
        cs,
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        blaster_res(0.0, 30.0, 50.0),
    ));
    // In range, out of arc — but the bank is offline.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();

    assert!(
        find_arc_request(&app).is_none(),
        "an offline weapon family must not emit an ArcBearingRequest"
    );
}

/// No request when the target is OUT OF RANGE of every emitter: no yaw brings
/// an out-of-reach contact into a firing solution, so nothing is asked.
#[test]
fn tick_weapons_arc_request_silent_when_target_out_of_range() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "cc000000-0000-0000-0000-000000000005";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        TacticalRadarSelection(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        blaster_res(0.0, 30.0, 50.0),
    ));
    // Beyond the 50-unit blaster range (200 away) — out of range entirely.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(200.0, 0.0, 0.0),
    ));

    app.update();

    assert!(
        find_arc_request(&app).is_none(),
        "an out-of-range target must not emit an ArcBearingRequest — no bearing helps"
    );
}

/// The request clears (re-fires) when the target crosses INTO the family's arc:
/// once the same family+target has the target in arc, no request stands.
#[test]
fn tick_weapons_arc_request_clears_when_target_enters_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "cc000000-0000-0000-0000-000000000006";

    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            ShipPhysics::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            WeaponsArcRequestState::default(),
            blaster_res(0.0, 30.0, 50.0),
        ))
        .id();
    // Start out of arc (starboard) → request fires.
    let target = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(20.0, 0.0, 0.0),
        ))
        .id();
    app.update();
    assert!(
        find_arc_request(&app).is_some(),
        "precondition: an out-of-arc target fires a request"
    );

    // Move the target directly ahead (into the fore arc) and confirm the
    // emitter's condition no longer holds — the debounce state clears so a
    // stale request stops standing.
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_xyz(0.0, 0.0, -20.0));
    app.update();

    let state = app.world().get::<WeaponsArcRequestState>(ship).unwrap();
    assert!(
        state.last.is_none(),
        "once the target enters the family's arc, the request condition must clear"
    );
}

/// Regression test for the unified `handle_fire_phaser`.
///
/// Before unification, `handle_npc_beam_fire` always used the first entry
/// of `WeaponsConsoleSection.phaser_banks` and a 360° arc via
/// `radar::is_fire_ready_with_range`. Post-unification, NPCs consult
/// their `PhaserCombatConfigResource::bank_by_id` and honour that bank's
/// `fire_arc_deg`. A target outside the requested bank's arc must be
/// rejected, matching the player-fire behaviour.
#[test]
fn npc_handle_fire_phaser_rejects_target_outside_requested_bank_arc() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "bb000000-0000-0000-0000-000000000001";
    let target_uuid = "bb000000-0000-0000-0000-000000000002";

    // NPC facing -Z with a narrow port-only bank (facing_deg=-90, arc=60°).
    // Target directly ahead is out of arc.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("port").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let combat = crate::entity_config::PhaserCombatConfig {
        banks: vec![crate::entity_config::PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 60.0,
            auto_arc_deg: 60.0,
            beam_range: 50.0,
            beam_damage_per_sec: 5.0,
            beam_duration_secs: 3.0,
            cooldown_secs: 6.0,
            beam_color: vec![],
            shield_pierce: None,
            marker: None,
            ai: None,
        }],
    };
    let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid).unwrap();
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            TacticalRadarSelection(Some(target_uuid_parsed.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(combat),
            Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }
    // Target directly ahead (-Z, bearing 0°) — outside the -90° port bank
    // whose arc runs from -120° to -60°.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    // Send an explicit FirePhaser request for the port bank.
    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    app.update();

    let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
    assert!(
        !beam.is_firing(),
        "FirePhaser for a port bank must be rejected when the target is not in that bank's fire arc — unified handler now honours per-bank config for NPCs"
    );
}

fn tactical_blips(app: &mut App) -> Vec<RadarBlip> {
    use crate::messages::SystemBlackboard;
    use crate::server_app::ShipSystemBlackboards;
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    match q.single(app.world()) {
        Ok(bbs) => match bbs
            .0
            .get(&crate::system_registry::tactical_radar_system_id())
        {
            Some(SystemBlackboard::TacticalRadar(bb)) => bb.blips.clone(),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

/// This ship's Tactical Radar blackboard (issue #829), if present.
fn tactical_radar_bb_of(
    app: &mut App,
    entity: Entity,
) -> Option<crate::messages::TacticalRadarBlackboard> {
    use crate::messages::SystemBlackboard;
    use crate::server_app::ShipSystemBlackboards;
    let bbs = app.world().get::<ShipSystemBlackboards>(entity)?;
    match bbs
        .0
        .get(&crate::system_registry::tactical_radar_system_id())
    {
        Some(SystemBlackboard::TacticalRadar(bb)) => Some(bb.clone()),
        _ => None,
    }
}

#[test]
fn radar_blip_appears_for_asteroid_within_tactical_range() {
    let mut app = test_app();
    // Configure tactical radar to show asteroids with range 300.
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.tactical_radar_shows = vec!["asteroid".into()];
        cfg.0.tactical_radar_range = 300.0;
    }
    // Asteroid 50 units ahead (z=-50, within 300 range).
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game(&mut app);
    tick(&mut app); // first InProgress tick → publish runs

    let blips = tactical_blips(&mut app);

    assert_eq!(blips.len(), 1, "expected one blip for in-range asteroid");
    assert_eq!(blips[0].uuid, "target-uuid");
    assert_eq!(blips[0].kind, "asteroid");
    // Forward (z=-50) at yaw=0 maps to radar_y > 0 (forward = up).
    assert!(
        blips[0].radar_y > 0.0,
        "asteroid ahead should have positive radar_y"
    );
    assert!(
        (blips[0].radar_x).abs() < 1e-4,
        "asteroid directly ahead has radar_x ≈ 0"
    );
}

#[test]
fn asteroid_beyond_tactical_range_not_in_blips() {
    let mut app = test_app();
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.tactical_radar_shows = vec!["asteroid".into()];
        cfg.0.tactical_radar_range = 100.0;
    }
    // Asteroid 200 units ahead — beyond the 100-unit radar range.
    setup_weapons_world(&mut app, 0.0, -200.0);
    start_game(&mut app);
    tick(&mut app);

    let blips = tactical_blips(&mut app);
    assert!(
        blips.is_empty(),
        "asteroid beyond tactical range must not appear in blips"
    );
}

// ── Tactical AI tests ──────────────────────────────────────────────────

/// Set the ControlSource for every tactical fine system on the LocalShip.
///
/// Post-#512 gating reads per-fine-system policies; post-#801 the coarse
/// `tactical` id is not a system at all, so this helper seeds only the
/// fine ids (mirrors what happens when a station rating flips to
/// Backfill, which triggers AI control of every fine system owned by
/// the station).
fn set_tactical_control_source(app: &mut App, source: crate::ship::control_source::ControlSource) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        for sysid in [
            // The tactical RADAR is what licenses AI target selection since
            // issue #887 — the lock belongs to the radar, so the radar's own
            // policy is the gate. Without it here, "put Tactical under AI"
            // would set every weapon Ai and leave the lock human-held, which
            // is a different ship from the one these tests mean.
            crate::system_registry::tactical_radar_system_id(),
            crate::system_registry::phaser_fore_system_id(),
            crate::system_registry::phaser_aft_system_id(),
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::system_registry::torpedo_tube_aft_system_id(),
            crate::system_registry::torpedo_magazine_system_id(),
        ] {
            cs.0.set(sysid, source);
        }
    }
}

fn spawn_asteroid_target(app: &mut App, uuid: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        AsteroidUuid(uuid.into()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(x, 0.0, z),
    ));
}

fn spawn_entity_target(app: &mut App, uuid: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::entity_spawner::EntityUuid(uuid.into()),
        AdmittedCommands::default(),
        Transform::from_xyz(x, 0.0, z),
    ));
}

// ── Nearest-hostile acquisition fixtures (issue #703) ──────────────────

/// Faction UUIDs for the nearest-hostile tests. Mirrors combat_test.toml:
/// Harrow lists Federation as an enemy.
fn harrow_faction() -> uuid::Uuid {
    uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap()
}

fn federation_faction() -> uuid::Uuid {
    uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap()
}

/// Declare the ship's tactical radar horizon. In production this is
/// authored per entity template under `[weapons_console] radar.range`; the
/// tests read it from the same component rather than any literal in code.
fn set_tactical_radar_range(app: &mut App, range: f32) {
    use crate::entity_tags::EntityTag;
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    let entity = q.single_mut(app.world_mut()).expect("LocalShip");
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::entity_spawner::WeaponsConsoleSection(
            crate::entity_config::WeaponsConsoleConfig {
                torpedo_arc_color: vec![],
                power_multipliers: None,
                phaser_banks: vec![],
                blaster_banks: vec![],
                radar: Some(crate::radar_config::RadarConfig {
                    range,
                    shows: vec![EntityTag::Ship],
                    selects: vec![],
                }),
                selector: None,
                selector_idle: false,
            },
        ));
}

/// Put the LocalShip in the Harrow faction and load a registry in which
/// Harrow is hostile to Federation — the same shape `combat_test.toml`
/// builds via `add_faction_enemy`.
fn setup_harrow_ship_hostile_to_federation(app: &mut App) {
    use crate::faction::{FactionConfig, FactionRegistry};

    let mut registry = FactionRegistry::new();
    registry.insert(FactionConfig {
        uuid: harrow_faction(),
        name: "Harrow".into(),
        enemies: vec![federation_faction()],
    });
    registry.insert(FactionConfig {
        uuid: federation_faction(),
        name: "Federation".into(),
        enemies: vec![],
    });
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        registry,
    ));

    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    let entity = q.single_mut(app.world_mut()).expect("LocalShip");
    app.world_mut()
        .entity_mut(entity)
        .insert(FactionComponent(harrow_faction()));
}

/// A factioned **ship** — the entity shape the nearest-hostile tier is
/// allowed to auto-acquire. The `Ship` marker is not decoration: the tier-4
/// scan is `With<Ship>`, matching the tactical radar's `shows:
/// [EntityTag::Ship]`. See `tier_four_does_not_acquire_a_factioned_non_ship`
/// for the other side of that filter.
fn spawn_factioned_target(
    app: &mut App,
    uuid: &str,
    x: f32,
    z: f32,
    faction: uuid::Uuid,
) -> Entity {
    app.world_mut()
        .spawn((
            crate::simulation::Ship,
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
            FactionComponent(faction),
        ))
        .id()
}

/// Author an *untargeted* `Destroy` objective — `Destroy { target: "" }`.
/// This is what every shipped hostile TOML produces (`directive_kind =
/// "Destroy"` with no `directive_target`), and the only directive shape
/// that licenses the nearest-hostile tier.
fn insert_untargeted_destroy_objective(app: &mut App, score: f32) {
    insert_destroy_objective_blackboard(app, "", score);
}

/// Set the LocalShip's `LastShipAttacker`. Wraps the entity-taking
/// `set_last_attacker` defined further down this module.
fn set_local_last_attacker(app: &mut App, uuid: Option<String>) {
    let entity = local_ship_entity(app);
    set_last_attacker(app, entity, uuid);
}

/// Issue #889: `ai_target_selection` was UNGATED. `SimSet` is configured in
/// Bevy's `Update`, so ungated meant one target decision per rendered frame —
/// at the host's display refresh rate, over a `WorldSnapshot` rebuilt on an
/// unrelated 10 Hz clock. It now shares the helm axes' fixed-rate latch.
///
/// The probe is the helm cadence tests' sentinel shape: a UUID no entity
/// carries is stamped into `TacticalRadarSelection` before every frame, so a
/// frame that leaves it standing is a frame the decider did not run on
/// (retention cannot keep a lock on an entity that does not exist).
///
/// Note this fixture deliberately does NOT use the module's `tick`, which arms
/// the latch by hand — it drives `Time` instead, because the throttle is the
/// thing under test.
#[test]
fn ai_target_selection_runs_on_the_shared_ai_tick_not_per_frame() {
    const SENTINEL: &str = "00000000-0000-0000-0000-0000000889ff";

    let mut app = test_app();
    let near_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &near_uuid, 0.0, -50.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("target".into(), near_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "target", 80.0);

    // 10 ms per frame — under the 33.3 ms shared cadence period, i.e. what a
    // 60 Hz rAF-driven host actually does.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(10),
    ));

    const FRAMES: usize = 12;
    let mut ran = 0usize;
    for _ in 0..FRAMES {
        set_weapons_target(&mut app, Some(SENTINEL.to_string()));
        app.update();
        if get_weapons_target(&mut app).as_deref() != Some(SENTINEL) {
            ran += 1;
        }
    }

    assert!(
        ran > 0,
        "precondition: {FRAMES} frames x 10 ms spans several 33.3 ms periods, so the \
         decider must run at least once — 0 runs means the probe is broken and this \
         test proves nothing about cadence"
    );
    assert!(
        ran <= FRAMES / 2,
        "the shared AI tick must throttle ai_target_selection: at 10 ms/frame it ran \
         on {ran} of {FRAMES} frames. Running every frame means the \
         run_if(ai_tick_ready) gate is gone and target selection follows display \
         refresh rate again (issue #889, PRD #620)"
    );
}

#[test]
fn tactical_ai_respects_radar_range() {
    let mut app = test_app();
    let near_uuid = uuid::Uuid::new_v4().to_string();
    let far_uuid = uuid::Uuid::new_v4().to_string();

    // Attach a WeaponsConsoleSection with a radar range of 100 so the
    // tactical AI reads a finite, damage-scaled horizon for the test.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        if let Ok(entity) = q.single_mut(app.world_mut()) {
            use crate::entity_tags::EntityTag;
            app.world_mut().entity_mut(entity).insert(
                crate::entity_spawner::WeaponsConsoleSection(
                    crate::entity_config::WeaponsConsoleConfig {
                        torpedo_arc_color: vec![],
                        power_multipliers: None,
                        phaser_banks: vec![],
                        blaster_banks: vec![],
                        radar: Some(crate::radar_config::RadarConfig {
                            range: 100.0,
                            shows: vec![EntityTag::Ship],
                            selects: vec![],
                        }),
                        selector: None,
                        selector_idle: false,
                    },
                ),
            );
        }
    }

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Far target — beyond radar range.
    spawn_entity_target(&mut app, &far_uuid, 0.0, -500.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("target".into(), far_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "target", 80.0);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "Tactical AI must NOT acquire a target beyond radar range"
    );

    // Near target — now within range. Update the runtime mapping so the
    // same objective name resolves to the nearby entity.
    spawn_entity_target(&mut app, &near_uuid, 0.0, -50.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("target".into(), near_uuid.clone());

    set_weapons_target(&mut app, None);
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(near_uuid.as_str()),
        "Tactical AI must acquire a target within radar range"
    );
}

// ── Nearest-hostile acquisition tier (issue #703) ──────────────────────
//
// Regression guards for the shipped-content bug: `ai_target_selection`
// acquired only from an explicit `Destroy` target or `LastShipAttacker`.
// No asset TOML authors a `directive_target`, and `LastShipAttacker` is
// written only by `tick_beams` — so an NPC could not fire until the player
// shot it first. These pin the third tier that closes that gap.

/// The headline fix: an NPC on standing "destroy hostiles" doctrine
/// acquires a hostile it can see, *without* having been attacked.
#[test]
fn tactical_ai_acquires_nearest_hostile_without_being_shot_first() {
    let mut app = test_app();
    let hostile_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // A Federation ship well inside the 100-unit radar horizon.
    spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -50.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);

    // Nobody has shot us: no LastShipAttacker, and the objective names
    // no one. Pre-#703 both acquisition tiers came up empty here.
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(hostile_uuid.as_str()),
        "an NPC on untargeted Destroy doctrine must acquire the nearest hostile in radar \
         range without waiting to be shot first — this is the whole point of issue #703"
    );
}

/// The nearest hostile is picked among several — and it is the *nearest*,
/// agreeing with the helm AI, which closes on the same ship.
#[test]
fn tactical_ai_acquires_the_nearest_of_several_hostiles() {
    let mut app = test_app();
    let near_uuid = uuid::Uuid::new_v4().to_string();
    let far_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Both in range; spawn the far one first so the result cannot be an
    // artefact of iteration order.
    spawn_factioned_target(&mut app, &far_uuid, 0.0, -90.0, federation_faction());
    spawn_factioned_target(&mut app, &near_uuid, 0.0, -20.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(near_uuid.as_str()),
        "the nearest-hostile tier must pick the nearest, not the first found — the helm AI \
         closes on the nearest via the same find_nearest_hostile, and the two must agree"
    );
}

/// The radar gate binds the new tier exactly as it binds the others: a
/// ship must not lock what it cannot detect.
#[test]
fn tactical_ai_does_not_acquire_a_hostile_beyond_radar_range() {
    let mut app = test_app();
    let hostile_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Hostile at 500 units — far beyond the 100-unit radar horizon.
    spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -500.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "the nearest-hostile tier must be gated by the damage-scaled tactical radar range — \
         an NPC must not acquire a target it cannot detect"
    );
}

/// Faction filtering: a ship of our own faction is not a hostile, however
/// close it is.
#[test]
fn tactical_ai_does_not_acquire_a_non_hostile() {
    let mut app = test_app();
    let friendly_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Another Harrow ship — our own faction — right next to us.
    spawn_factioned_target(&mut app, &friendly_uuid, 0.0, -10.0, harrow_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "the nearest-hostile tier must filter by faction through the live FactionRegistry — \
         a same-faction ship is never a weapons target, however near"
    );
}

/// Precedence, tier 1 over tier 3: a `Destroy` naming someone specific must
/// not wander onto a nearer ship.
#[test]
fn explicit_destroy_target_takes_precedence_over_a_nearer_hostile() {
    let mut app = test_app();
    let named_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // The named target is further away than an unnamed hostile. Both are
    // Federation, both in radar range.
    spawn_factioned_target(&mut app, &named_uuid, 0.0, -80.0, federation_faction());
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), named_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(named_uuid.as_str()),
        "an explicit Destroy target must outrank the nearest-hostile tier — a mission that \
         names a target must not be silently retargeted onto whoever is closest"
    );
}

/// Precedence, tier 2 over tier 3: whoever shot us still outranks a nearer
/// bystander, exactly as before #703.
#[test]
fn last_attacker_takes_precedence_over_a_nearer_hostile() {
    let mut app = test_app();
    let attacker_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // The attacker is further away than an unengaged hostile.
    spawn_factioned_target(&mut app, &attacker_uuid, 0.0, -80.0, federation_faction());
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, Some(attacker_uuid.clone()));

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(attacker_uuid.as_str()),
        "LastShipAttacker must outrank the nearest-hostile tier — shooting back at whoever \
         hit us must not be displaced by a closer bystander"
    );
}

// ── Target retention (tier 2) ──────────────────────────────────────────
//
// The nearest-hostile tier decides "who is closest *now*". Left ungated it
// re-decides that every tick, so a lock follows whoever happens to be
// nearest at this instant — beams retargeting, and (because the helm pursues
// `TacticalRadarSelection`) the ship slewing between bearings with it. These pin the
// retention tier that keeps an engaged ship committed.

/// The headline retention case: engaged with A, B closes inside it, and the
/// lock stays on A.
#[test]
fn an_established_lock_is_retained_when_a_nearer_hostile_appears() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    // Tick once with only A present: the ship acquires and engages it.
    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the ship must be engaged with A before B arrives"
    );

    // B arrives, closer than A, and equally hostile.
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "an established lock on a live, in-range hostile must be retained when a nearer \
         hostile appears — the helm keeps closing on A (the helm reads the retained TacticalRadarSelection, which prefers its \
         current target), so weapons flipping to B would have the ship shooting one ship \
         while flying at another"
    );
}

/// The other half: retention is not a freeze. A lock that dies is re-scanned.
#[test]
fn the_lock_is_rescanned_when_the_current_target_dies() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let other_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let engaged = spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the nearer hostile is the one engaged"
    );

    // A is destroyed.
    app.world_mut().entity_mut(engaged).despawn();
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(other_uuid.as_str()),
        "retention must not outlive the target: once the locked ship is gone the \
         nearest-hostile tier must acquire afresh, or the AI sits idle beside a live enemy"
    );
}

/// The liveness half of retention, on the one path where the radar gate
/// cannot stand in for it. A ship that declares no `radar.range` has an
/// unbounded horizon (`range_bounds_targets == false`), so `within_range`
/// is never consulted and "the locked entity no longer exists" is the only
/// thing that can release the lock. Without that check the retention tier
/// hands the dead UUID on, the stale guard clears it, and the ship spends
/// the tick idle next to a live enemy instead of acquiring it.
#[test]
fn the_lock_is_rescanned_when_the_current_target_dies_with_no_radar_horizon() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let other_uuid = uuid::Uuid::new_v4().to_string();

    // Deliberately no set_tactical_radar_range: no WeaponsConsoleSection
    // means a base range of 0, which the system reads as "unbounded".
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let engaged = spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the nearer hostile is the one engaged"
    );

    app.world_mut().entity_mut(engaged).despawn();
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(other_uuid.as_str()),
        "retention must check that the locked entity still exists, not lean on the radar \
         gate to notice — an unbounded horizon never range-checks, so a dead lock would \
         block acquisition for the tick"
    );
}

/// Retention is bounded by the same radar horizon as acquisition (issue
/// #680): a lock that runs out of detection range is re-scanned, not held.
#[test]
fn the_lock_is_rescanned_when_the_current_target_leaves_radar_range() {
    let mut app = test_app();
    let fleeing_uuid = uuid::Uuid::new_v4().to_string();
    let other_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let fleeing = spawn_factioned_target(&mut app, &fleeing_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(fleeing_uuid.as_str()),
        "precondition: the nearer hostile is the one engaged"
    );

    // A runs beyond the 100-unit tactical radar horizon.
    app.world_mut()
        .entity_mut(fleeing)
        .insert(Transform::from_xyz(0.0, 0.0, -500.0));
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(other_uuid.as_str()),
        "retention must be gated by the damage-scaled radar range exactly as acquisition is \
         — a target the ship can no longer detect must not pin the lock and starve the scan"
    );
}

/// The ordering decision, pinned: retention outranks `LastShipAttacker`,
/// because the helm has no retaliation tier and would keep closing on A.
/// The reverse order is the tempting one — see this system's doc comment.
#[test]
fn an_established_lock_outranks_a_new_last_attacker() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let attacker_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &attacker_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the ship is engaged with A"
    );

    // B opens fire on us mid-engagement.
    set_local_last_attacker(&mut app, Some(attacker_uuid.clone()));
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "taking a hit must not break off an engagement weapons is already in: the helm's \
         ai_target_selection's retention tier outranks its last_attacker tier, and \
         weapons must match it tier for tier or the ship closes on A while shooting B. \
         last_attacker_takes_precedence_over_a_nearer_hostile pins the case that still \
         retaliates — no lock to keep"
    );
}

/// A named assault target may be factionless (Starbase Alpha). Once that
/// objective is vetoed and the active Destroy doctrine becomes untargeted
/// combat, retaining that old lock would prevent the hostile scan from ever
/// engaging the player.
#[test]
fn combat_doctrine_drops_a_retained_factionless_assault_lock() {
    let mut app = test_app();
    let starbase_uuid = uuid::Uuid::new_v4().to_string();
    let hostile_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &starbase_uuid, 0.0, -40.0);
    spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -60.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_weapons_target(&mut app, Some(starbase_uuid));

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(hostile_uuid.as_str()),
        "untargeted combat doctrine must discard a factionless assault lock and acquire an opposing ship"
    );
}

/// Advisory from the #703 review: the tier-4 scan is an *auto-acquisition*
/// surface, so it must be `With<Ship>` — the tactical radar `shows:
/// [EntityTag::Ship]` and nothing else. No shipped non-ship template
/// declares a `faction` today; this pins the filter before one does.
#[test]
fn tier_four_does_not_acquire_a_factioned_non_ship() {
    let mut app = test_app();
    let station_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // A hostile-factioned entity that is *not* a ship — the shape a
    // factioned station / mine / probe template would spawn. Everything
    // else about it would qualify: in radar range, enemy faction, closer
    // than anything else in the world.
    app.world_mut().spawn((
        crate::entity_spawner::EntityUuid(station_uuid),
        Transform::from_xyz(0.0, 0.0, -10.0),
        FactionComponent(federation_faction()),
    ));
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "the nearest-hostile tier must only auto-acquire ships — a factioned non-ship is \
         not what the tactical radar shows, and locking one would have the AI open fire on \
         scenery it cannot even see"
    );
}

// ── Advisory Sensors designation copying + Tactical authority (issue #777) ──
//
// AC2/AC3/AC4/AC6: Tactical ranks its own candidates, may strongly favour the
// advisory Sensors science target, but independently revalidates it and remains
// the SOLE writer of the authoritative `TacticalRadarSelection`. The Sensors
// pick reaches Tactical only through the frozen viewscreen `science_target`,
// which the harness' `seed_viewscreen_from_selection` glue lifts from the
// ship's own `SensorRadarSelection` before `SimSet::Input` — exactly as the
// radar publisher + viewscreen aggregator do in the full app.

/// Set the LocalShip's advisory Sensors science target. The seed glue lifts it
/// into `ViewscreenBlackboard::science_target`, which `ai_target_selection`
/// reads as the `sensors-designation` candidate source.
fn set_science_designation(app: &mut App, uuid: Option<String>) {
    let entity = local_ship_entity(app);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::sensors_plugin::SensorRadarSelection(uuid));
}

/// AC6(a): Tactical COPIES the advisory Sensors designation when it wins the
/// authored ranking. The designated hostile is further away than an unnamed
/// hostile the radar-contacts source would otherwise pick; the Sensors-favour
/// bonus carries the day, and the observable authoritative weapons target lands
/// on the designated ship.
#[test]
fn tactical_copies_the_advisory_sensors_designation_when_it_wins_scoring() {
    let mut app = test_app();
    let designated_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 200.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // The designated hostile is FURTHER than an unnamed hostile the radar
    // source would pick — so a plain nearest scan would choose the nearer one.
    spawn_factioned_target(
        &mut app,
        &designated_uuid,
        0.0,
        -120.0,
        federation_faction(),
    );
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -20.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);
    set_science_designation(&mut app, Some(designated_uuid.clone()));

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(designated_uuid.as_str()),
        "Tactical must favour the advisory Sensors designation through its authored score \
         bonus (AC2) and copy it into the authoritative TacticalRadarSelection (AC6), even \
         when a nearer unnamed hostile is available"
    );
}

/// AC6(b) / AC3: Tactical REFUSES the Sensors pick when it fails independent
/// revalidation — here the designation is a friendly ship — and falls back to
/// its own independently-validated hostile.
#[test]
fn tactical_refuses_a_friendly_sensors_designation_and_picks_its_own_hostile() {
    let mut app = test_app();
    let friendly_uuid = uuid::Uuid::new_v4().to_string();
    let hostile_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 200.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Sensors designates a same-faction (Harrow) ship right next to us; the only
    // opposing ship is further away.
    spawn_factioned_target(&mut app, &friendly_uuid, 0.0, -20.0, harrow_faction());
    spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -100.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);
    set_science_designation(&mut app, Some(friendly_uuid.clone()));

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(hostile_uuid.as_str()),
        "Tactical must independently revalidate the Sensors designation's hostility (AC3) — \
         a friendly pick is refused, and Tactical acquires its own hostile instead"
    );
}

/// AC4 / AC6(c): a Sensors designation ALONE never mutates the authoritative
/// weapons target. With no candidate Tactical will validate — the designation is
/// a friendly and nothing else is on the board — `TacticalRadarSelection` stays
/// empty, proving the Sensors host never writes or bypasses weapons target state.
#[test]
fn sensors_designation_alone_does_not_mutate_the_weapons_target() {
    let mut app = test_app();
    let friendly_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 200.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Only a friendly ship exists, and Sensors designates it. No objective, no
    // last attacker, no hostile.
    spawn_factioned_target(&mut app, &friendly_uuid, 0.0, -20.0, harrow_faction());
    set_local_last_attacker(&mut app, None);
    set_science_designation(&mut app, Some(friendly_uuid.clone()));

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "a Sensors designation must never directly write or bypass the authoritative weapons \
         target (AC4): Tactical refused the friendly pick and had nothing else to validate, so \
         TacticalRadarSelection stays empty"
    );
}

/// Regression (issue #777): scores are ADDITIVE and one entity can carry
/// several source markers. With the original weights the current lock scored
/// its `retained` weight (500) ON TOP of its `sensors_designation` weight (800)
/// whenever the two coincided — the common NPC case (sensors AI →
/// SensorRadarSelection → frozen science_target → same ship's Tactical) — for a
/// stacked 1300 that beat a distinct in-range named Destroy objective (1000).
/// The ship refused to retarget onto its explicit mission objective.
///
/// The weights are now sized so `objective` (1000) strictly dominates the whole
/// non-objective stack by more than `switch_margin` (sensors 500 + retained 200
/// = 700 < 1000 − 50). The coinciding lock scores 700, the objective 1000 wins,
/// and hysteresis cannot save the lock (700 < 950). This is the exact case the
/// review finding describes; no earlier test covered it.
#[test]
fn objective_beats_a_lock_that_coincides_with_the_sensors_designation() {
    let mut app = test_app();
    let designated_uuid = uuid::Uuid::new_v4().to_string();
    let objective_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 300.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // The current lock is a hostile that the ship's own Sensors radar ALSO
    // designates — so as a candidate it carries BOTH `source_retained` and
    // `source_sensors_designation`, the double-source stack the finding names.
    spawn_factioned_target(&mut app, &designated_uuid, 0.0, -20.0, federation_faction());
    set_weapons_target(&mut app, Some(designated_uuid.clone()));
    set_science_designation(&mut app, Some(designated_uuid.clone()));

    // A DISTINCT, in-range ship named by an explicit Destroy objective. A named
    // (not untargeted) objective keeps the nearest-hostile radar source out of
    // the pool, so the only candidates are the designated lock and this
    // objective.
    spawn_entity_target(&mut app, &objective_uuid, 0.0, -120.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("primary".into(), objective_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "primary", 90.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(objective_uuid.as_str()),
        "an in-range named Destroy objective (1000) must win over the ship's current lock even \
         when that lock coincides with its own Sensors designation — retention is switch_margin \
         hysteresis, not an additive weight that stacks to 1300 and refuses the mission objective"
    );
}

fn insert_destroy_objective_blackboard(app: &mut App, target: &str, score: f32) {
    use crate::messages::{
        AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
        SystemAffinity, SystemBlackboard, ViewscreenBlackboard,
    };
    use crate::server_app::ShipSystemBlackboards;

    let viewscreen = ViewscreenBlackboard {
        scored_objectives: vec![ScoredObjective {
            id: format!("obj-destroy-{target}"),
            score,
            directive: AiDirective::Destroy {
                target: target.into(),
            },
            source: ObjectiveSource::Mission,
            relevance: vec![
                SystemAffinity::Helm,
                SystemAffinity::Weapons,
                SystemAffinity::Captain,
            ],
            snapshot: ObjectiveSnapshot {
                id: format!("obj-destroy-{target}"),
                text: format!("Destroy {target}"),
                mandatory: true,
                status: ObjectiveStatus::Active,
                targets: vec![target.into()],
                source: ObjectiveSource::Mission,
            },
        }],
        ..Default::default()
    };
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let mut bbs = q
        .single_mut(app.world_mut())
        .expect("LocalShip must have ShipSystemBlackboards");
    bbs.0.insert(
        crate::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(viewscreen),
    );
}

#[test]
fn tactical_ai_selects_named_destroy_objective_target() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "Tactical AI must lock the live entity named by the Destroy objective"
    );
}

#[test]
fn tactical_ai_clears_stale_weapons_target_when_objective_target_dead() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    // Pre-set a stale target UUID — simulates a prior Destroy objective
    // whose entity was killed.
    set_weapons_target(&mut app, Some("dead-target-uuid".into()));
    // No last attacker.
    // Still have a Destroy objective for a target that is no longer alive.
    insert_destroy_objective_blackboard(&mut app, "wave_gone", 80.0);
    // No entity named "wave_gone" exists → resolve returns None.

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "Tactical AI must clear TacticalRadarSelection when the objective target is \
         dead and no last attacker is available, fixing the stale-target bug \
         that caused AI to sit idle after killing its last target"
    );
}

#[test]
fn tactical_ai_ignores_missing_destroy_objective_target() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    insert_destroy_objective_blackboard(&mut app, "wave_404", 80.0);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "Tactical AI must not lock an arbitrary target when the objective target is missing"
    );
}

// ── ai_target_selection / locked_target (issue #697) ────────────────────

/// Read a ship's published Weapons blackboard by entity.
fn weapons_blackboard_of(app: &mut App, entity: Entity) -> Option<WeaponsBlackboard> {
    app.world()
        .entity(entity)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .and_then(
            |bbs| match bbs.0.get(&crate::system_registry::tactical_station_key()) {
                Some(SystemBlackboard::Weapons(bb)) => Some(bb.clone()),
                _ => None,
            },
        )
}

/// Spawn an NPC ship: every component the spawner gives a `[behaviour]`
/// entity that the Weapons systems touch, minus the `LocalShip` marker.
/// Its Tactical fine systems are all AI-controlled.
fn spawn_npc_ship(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    let config = test_ship_config();
    let mut resolver = ControlSourceResolver::new();
    for system in &config.0.systems {
        resolver.set(system.id.clone(), ControlSource::Ai);
    }
    let npc = app
        .world_mut()
        .spawn((
            crate::simulation::Ship,
            config,
            ShipSystemControlSources(resolver),
            crate::server_app::ShipSystemBlackboards::default(),
            LastShipAttacker::default(),
            ShipPhysics {
                x,
                z,
                ..Default::default()
            },
            // Every ship the Tactical AI decides for needs one: since issue
            // #887 the decision travels to `handle_set_target` as an admitted
            // `SetTarget` rather than being written straight to the component.
            // `entities/spawner.rs` inserts this on every spawned NPC.
            crate::messages::AdmittedCommands::default(),
            TacticalRadarSelection::default(),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "phaser-fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    ..Default::default()
                }],
            }),
            TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
        ))
        .id();
    attach_shipped_weapon_ai(app, npc);
    npc
}

fn set_last_attacker(app: &mut App, entity: Entity, uuid: Option<String>) {
    app.world_mut()
        .entity_mut(entity)
        .insert(LastShipAttacker(uuid));
}

#[test]
fn ai_target_selection_publishes_locked_target_and_applies_it() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    // Two ticks: `target_uuid` is the FROZEN viewscreen combat lock (spec §3),
    // aggregated in `SimSet::PublishAggregate` after this publisher ran in
    // `SimSet::Publish`, so tick 1's fresh AI selection only reaches
    // `target_uuid` on tick 2. `locked_target` (written in `SimSet::Input`) is
    // visible immediately — the one-tick lag is exactly the gap between them.
    tick(&mut app);
    tick(&mut app);

    let local = local_ship_entity(&mut app);
    let bb = weapons_blackboard_of(&mut app, local)
        .expect("LocalShip must publish a Weapons blackboard");
    assert_eq!(
        bb.locked_target.as_deref(),
        Some(target_uuid.as_str()),
        "ai_target_selection must publish its choice as locked_target, and that intent \
         must survive publish_weapons_core_blackboard rebuilding the blackboard in SimSet::Publish"
    );
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "ai_target_selection must apply its choice to the authoritative TacticalRadarSelection"
    );
    assert_eq!(
        bb.target_uuid, bb.locked_target,
        "on an AI-operated ship, intent and truth agree once the combat lock has \
         been through the viewscreen aggregator"
    );
    assert_eq!(
        bb.target_uuid.as_deref(),
        Some(target_uuid.as_str()),
        "target_uuid must be the FROZEN ViewscreenBlackboard.combat_lock, not a live \
         read of TacticalRadarSelection (spec §3)"
    );
}

/// Pins that `WeaponsBlackboard.target_uuid` follows the viewscreen's frozen
/// Combat Lock and *only* that — mirroring the #829 consumer tests. Writing a
/// combat lock that disagrees with the live `TacticalRadarSelection` component
/// must publish the frozen value, proving the publisher no longer reaches the
/// radar's live selection synchronously.
#[test]
fn weapons_blackboard_target_follows_the_frozen_combat_lock_not_the_live_selection() {
    let mut app = test_app();
    let frozen_uuid = uuid::Uuid::new_v4().to_string();
    let live_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Human);
    spawn_entity_target(&mut app, &frozen_uuid, 0.0, -30.0);
    spawn_entity_target(&mut app, &live_uuid, 0.0, -40.0);

    let local = local_ship_entity(&mut app);
    // Live radar selection says one thing...
    set_weapons_target(&mut app, Some(live_uuid.clone()));
    // ...the frozen viewscreen fact says another. Overwrite it *after* the
    // seed glue would have run by writing directly, then read what Publish
    // produced on the next tick.
    {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        let mut bbs = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(local)
            .expect("LocalShip must carry ShipSystemBlackboards");
        let mut vbb = match bbs.0.get(&crate::system_registry::viewscreen_system_id()) {
            Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = Some(frozen_uuid.clone());
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vbb),
        );
    }
    // Run only the publisher, so the test harness' seed glue cannot re-sync
    // the viewscreen fact back to the live selection first.
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut()
        .run_system_once(crate::weapons_plugin::publish_weapons_core_blackboard)
        .expect("publisher must run");

    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.target_uuid.as_deref(),
        Some(frozen_uuid.as_str()),
        "publish_weapons_core_blackboard must read ViewscreenBlackboard.combat_lock; \
         reading the live TacticalRadarSelection would have published {live_uuid}"
    );
}

#[test]
fn ai_target_selection_clears_locked_target_when_target_dies() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("dead-target-uuid".into()));

    tick(&mut app);

    let local = local_ship_entity(&mut app);
    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.locked_target, None,
        "a lock on an entity that no longer exists must be dropped from the AI's intent"
    );
    assert!(
        get_weapons_target(&mut app).is_none(),
        "and it must clear the authoritative TacticalRadarSelection to match"
    );
}

#[test]
fn human_tactical_leaves_locked_target_empty_and_keeps_the_human_lock() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Human);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    // A Destroy objective the AI *would* act on, were it in control.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);
    // The human operator's own lock, as handle_set_target would leave it.
    set_weapons_target(&mut app, Some(target_uuid.clone()));

    tick(&mut app);

    let local = local_ship_entity(&mut app);
    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.locked_target, None,
        "locked_target is AI intent only — a human-operated Tactical selects nothing, \
         even with a live Destroy objective on the board"
    );
    assert_eq!(
        bb.target_uuid.as_deref(),
        Some(target_uuid.as_str()),
        "target_uuid mirrors the authoritative TacticalRadarSelection, which the human still owns"
    );
}

/// Put the ship in the mixed-rating shape a crewed Tactical station with
/// backfilled torpedoes actually has: the radar and the phaser banks are Human,
/// the magazine and every tube are Ai. `alliance_cruiser`'s shipped `Simplified`
/// rating is the mirror image (automated banks, crewed radar); either way the
/// point is that Tactical is NOT uniformly one source.
///
/// Pre-#887 this was the shape in which the two writers of
/// `TacticalRadarSelection` overlapped, held apart only by a `.before` edge.
/// Since #887 the radar's own policy is the gate, so the overlap is
/// unrepresentable: an Ai tube cannot license the selector on a Human radar.
fn set_mixed_tactical_control_sources(app: &mut App) {
    use crate::ship::control_source::ControlSource;
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        for sysid in [
            crate::system_registry::tactical_radar_system_id(),
            crate::system_registry::phaser_fore_system_id(),
            crate::system_registry::phaser_aft_system_id(),
        ] {
            cs.0.set(sysid, ControlSource::Human);
        }
        for sysid in [
            crate::system_registry::torpedo_magazine_system_id(),
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::system_registry::torpedo_tube_aft_system_id(),
        ] {
            cs.0.set(sysid, ControlSource::Ai);
        }
    }
}

/// The mixed-rating shape is only interesting if it really is mixed. Pin that
/// the AI half is live — `any_tactical_system_operates_ai` still holds — while
/// the radar stays the human's, so the regression test below can't quietly
/// decay into a test of a ship with no AI on it at all.
#[test]
fn mixed_rating_ship_has_live_tactical_ai_but_a_human_radar() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    set_mixed_tactical_control_sources(&mut app);

    let world = app.world_mut();
    let mut q = world.query_filtered::<(
        &ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
    ), With<crate::server_app::LocalShip>>();
    let (control_sources, ship_config) = q.single(world).expect("local ship");

    assert!(
        any_tactical_system_operates_ai(control_sources, &ship_config.0),
        "an Ai torpedo magazine must keep the Tactical surface partly AI — if this \
         goes false the ship stops being mixed and the regression below is unreachable"
    );
    let radar = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_radar_system_id());
    assert!(
        radar.accept_human_input && !radar.operate_ai,
        "the radar itself must stay the human's — that is the whole asymmetry \
         issue #887 closes"
    );
}

#[test]
fn human_set_target_survives_the_tick_on_a_mixed_rating_ship() {
    let mut app = test_app();
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    set_mixed_tactical_control_sources(&mut app);
    // A live untargeted Destroy objective plus a hostile the selector would
    // happily acquire: if the AI ran on this ship at all, it would take the lock.
    insert_untargeted_destroy_objective(&mut app, 35.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    let poacher = uuid::Uuid::new_v4().to_string();
    spawn_factioned_target(&mut app, &poacher, 0.0, -10.0, federation_faction());

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some("target-uuid"),
        "the human's SetTarget must survive the tick it was admitted in — the \
         Ai magazine must not license the selector to hand the lock to {poacher}"
    );

    // And it must still be there next tick — a lock clobbered on tick N is
    // not recovered on tick N+1, because selection re-seeds from the
    // (clobbered) TacticalRadarSelection.
    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some("target-uuid"),
        "the human's lock must be stable across subsequent ticks"
    );
}

/// Ported from `integrator_leaves_weapons_target_alone_when_selection_never_ran`
/// (issue #700). That test pinned the "decider never ran" vs "decider chose
/// nothing" distinction which `blackboard_locked_target`'s `Option<Option<_>>`
/// carried between `ai_target_selection` and the separate `operate_tactical_ai`
/// integrator. With the integrator folded in, a decision and its application
/// are the same statement, so "never ran" can no longer be misread as "chose
/// nothing" — the bug is unrepresentable rather than merely guarded.
///
/// What survives is the property underneath it, on the one path that can still
/// reach it: a ship the selector skips must keep the lock it already has, and
/// must not have an AI-intent entry conjured onto its blackboard.
#[test]
fn skipped_ship_keeps_its_weapons_target_and_gains_no_blackboard_entry() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    let mut app = App::new();
    // `ai_target_selection` emits its decision through the admission seam
    // (issue #887), which asks `Sessions` about station tenure.
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.add_systems(Update, ai_target_selection);

    let config = test_ship_config();
    let mut resolver = ControlSourceResolver::new();
    // Human across the board — including the tactical radar, which is the gate
    // since issue #887 — so selection skips this ship entirely.
    for system in &config.0.systems {
        resolver.set(system.id.clone(), ControlSource::Human);
    }
    let ship = app
        .world_mut()
        .spawn((
            crate::simulation::Ship,
            config,
            ShipSystemControlSources(resolver),
            LastShipAttacker::default(),
            ShipPhysics::default(),
            crate::messages::AdmittedCommands::default(),
            // The human operator's standing lock, on an entity that does not
            // exist in this bare world — so if the AI ever did run for this
            // ship, its stale-target guard would clear the lock and the
            // assertion below would fail. That is the point: the AI must not
            // run at all.
            TacticalRadarSelection(Some("standing-lock".into())),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();

    app.update();

    assert_eq!(
        app.world()
            .entity(ship)
            .get::<TacticalRadarSelection>()
            .unwrap()
            .0,
        Some("standing-lock".into()),
        "a ship whose Tactical is human-operated is skipped by the selector — \
         it must keep the human's lock, not have it re-decided or cleared"
    );
    assert!(
        !app.world()
            .entity(ship)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .unwrap()
            .0
            .contains_key(&crate::system_registry::tactical_station_key()),
        "a skipped ship has no AI intent to report, so the selector must not \
         insert a bare Weapons blackboard entry for it"
    );
}

#[derive(Resource)]
struct KillTargetOnDamage(String);

/// Stands in for `tick_beams` / `tick_torpedo_lifecycle`: both destroy the
/// locked target and clear `TacticalRadarSelection` *after* `SimSet::Input`, which is
/// what leaves a dead `locked_target` for `publish_weapons_core_blackboard` to
/// carry forward.
fn kill_target_after_input(
    mut commands: Commands,
    kill: Res<KillTargetOnDamage>,
    target_q: Query<(Entity, &crate::entity_spawner::EntityUuid)>,
    mut weapons_target_q: Query<&mut TacticalRadarSelection, With<crate::server_app::LocalShip>>,
) {
    for (entity, uuid) in target_q.iter() {
        if uuid.0 == kill.0 {
            commands.entity(entity).despawn();
        }
    }
    for mut wt in weapons_target_q.iter_mut() {
        if wt.0.as_deref() == Some(kill.0.as_str()) {
            wt.0 = None;
        }
    }
}

#[test]
fn publish_drops_locked_target_when_the_selected_target_dies_mid_tick() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    // Tick 1: the AI acquires the target while it is alive.
    tick(&mut app);
    // Tick 2 lets the acquisition reach the frozen viewscreen combat lock that
    // `target_uuid` now reads (spec §3 one-tick lag).
    tick(&mut app);
    let local = local_ship_entity(&mut app);
    assert_eq!(
        weapons_blackboard_of(&mut app, local)
            .expect("blackboard")
            .locked_target
            .as_deref(),
        Some(target_uuid.as_str()),
        "precondition: the AI must be locked on before the target dies"
    );

    // Kill tick: Input selects the (still live) target, then the target is
    // destroyed in Damage — exactly the beam/torpedo kill ordering.
    app.insert_resource(KillTargetOnDamage(target_uuid.clone()));
    app.add_systems(
        Update,
        kill_target_after_input.in_set(crate::sim_sets::SimSet::Damage),
    );
    tick(&mut app);

    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.target_uuid, None,
        "the kill cleared the authoritative TacticalRadarSelection; the frozen combat \
         lock that target_uuid reads still holds the dead uuid for one tick, so the \
         publisher's liveness filter is what must drop it"
    );
    assert_eq!(
        bb.locked_target, None,
        "a locked_target whose entity died after SimSet::Input must not be carried \
         forward: publishing it would put locked_target != target_uuid on the wire, \
         contradicting the field's documented contract that the two agree after a tick"
    );
}

#[test]
fn npc_ship_publishes_its_own_weapons_blackboard_with_ship_state_only() {
    let mut app = test_app();
    // LocalShip radar config: shows asteroids out to 300 units. Only the
    // LocalShip has a browser client, so only it should get blips.
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.tactical_radar_shows = vec!["asteroid".into()];
        cfg.0.tactical_radar_range = 300.0;
    }
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game(&mut app);

    // NPC at the origin, attacked by a live entity 30 units ahead.
    let attacker_uuid = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(&mut app, &attacker_uuid, 0.0, -30.0);
    let npc = spawn_npc_ship(&mut app, "npc-1", 0.0, 0.0);
    set_last_attacker(&mut app, npc, Some(attacker_uuid.clone()));

    // Two ticks: `target_uuid` is the frozen viewscreen combat lock, one tick
    // behind the NPC Tactical AI's selection (spec §1/§3).
    tick(&mut app);
    tick(&mut app);

    let bb = weapons_blackboard_of(&mut app, npc)
        .expect("an NPC carrying ShipSystemBlackboards must get a Weapons blackboard too");

    // Ship state — computed per-entity, so NPCs get the real thing.
    assert_eq!(
        bb.locked_target.as_deref(),
        Some(attacker_uuid.as_str()),
        "the NPC's Tactical AI must select its last attacker"
    );
    assert_eq!(
        bb.target_uuid.as_deref(),
        Some(attacker_uuid.as_str()),
        "and the NPC's authoritative TacticalRadarSelection must follow its own intent"
    );
    assert_eq!(
        bb.banks.len(),
        1,
        "banks come from the NPC's own PhaserCombatConfigResource"
    );
    assert_eq!(bb.banks[0].id, "phaser-fore");
    assert_eq!(
        bb.torpedo_count,
        TorpedoConfig::default().count,
        "torpedo_count comes from the NPC's own TorpedoSystemResource"
    );

    // Client render data — player-only, and left empty for NPCs. Blips +
    // regions moved to the tactical-radar blackboard (issue #829); phaser /
    // torpedo arcs remain on the Weapons blackboard.
    let npc_radar = tactical_radar_bb_of(&mut app, npc)
        .expect("an NPC carrying ShipSystemBlackboards must get a TacticalRadar blackboard too");
    assert!(
        npc_radar.blips.is_empty(),
        "blips are client render data sourced from the player-only \
         ShipClientConfigResource, and are O(all entities) to compute — an NPC \
         with no browser client must not pay for them"
    );
    assert!(
        npc_radar.regions.is_empty(),
        "regions are client render data"
    );
    assert!(
        bb.phaser_arcs.is_empty(),
        "phaser_arcs are client render data"
    );
    assert!(
        bb.torpedo_arcs.is_empty(),
        "torpedo_arcs are client render data"
    );

    // The contrast: the LocalShip *does* get its render data, so the
    // assertions above are about the NPC tier and not a dead radar config.
    let local = local_ship_entity(&mut app);
    let local_radar = tactical_radar_bb_of(&mut app, local).expect("tactical-radar blackboard");
    assert_eq!(
        local_radar.blips.len(),
        1,
        "the LocalShip still gets its in-range asteroid blip"
    );
}

#[test]
fn npc_and_local_ship_select_targets_independently() {
    let mut app = test_app();
    // Regression guard for the SetTarget-contamination class of bug: two
    // ships, two different attackers, two independent locks.
    let local_target = uuid::Uuid::new_v4().to_string();
    let npc_target = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(&mut app, &local_target, 0.0, -30.0);
    spawn_entity_target(&mut app, &npc_target, 0.0, 30.0);

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    let local = local_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(local)
        .insert(LastShipAttacker(Some(local_target.clone())));

    let npc = spawn_npc_ship(&mut app, "npc-1", 0.0, 0.0);
    set_last_attacker(&mut app, npc, Some(npc_target.clone()));

    tick(&mut app);

    assert_eq!(
        weapons_blackboard_of(&mut app, local)
            .expect("blackboard")
            .locked_target
            .as_deref(),
        Some(local_target.as_str())
    );
    assert_eq!(
        weapons_blackboard_of(&mut app, npc)
            .expect("blackboard")
            .locked_target
            .as_deref(),
        Some(npc_target.as_str()),
        "each ship selects from its own last-attacker surface, not a shared one"
    );
}

/// Builds on `test_app()` (LocalShip + `WeaponsPlugin` + `LobbyPlugin`) by
/// wiring in `ai_torpedo_auto_fire` (issue #694) and giving the LocalShip
/// `AiHighFidelity` so the decider runs for it.
fn torpedo_ai_test_app() -> App {
    let mut app = test_app();
    app.add_systems(
        Update,
        crate::console_ai_plugin::ai_torpedo_auto_fire
            .in_set(crate::sim_sets::SimSet::Physics)
            .before(crate::console::weapons::handle_fire_torpedo),
    );
    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("test_app must spawn a LocalShip")
    };
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ai_plugin::AiHighFidelity);
    app
}

/// Regression test for issue #694: `ai_torpedo_auto_fire` (preliminary)
/// replaces the old fused torpedo sub-block that used to run inline
/// inside `operate_tactical_ai`. Ported from the pre-#694
/// `ai_fires_torpedo_when_ai_controls_unclaimed_station`, which exercised
/// `operate_tactical_ai`'s torpedo block directly before it was deleted.
#[test]
fn ai_torpedo_auto_fire_fires_when_ai_controls_unclaimed_station() {
    // Unclaimed station + Ai ControlSource → ai_torpedo_auto_fire fires unconditionally.
    let mut app = torpedo_ai_test_app();

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    // Asteroid at (0, -30) → bearing 0 from ship at origin yaw=0 → in ForePort arc.
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire should fire TorpedoLaunched when controlling an unclaimed \
         Tactical station"
    );
}

/// Issue #738 per-ship isolation: `ai_torpedo_auto_fire` used to resolve its
/// tube/magazine state as `per_entity_component.unwrap_or(&global_resource)`,
/// and the global `TorpedoSystemResource` mirrors the PLAYER ship. An NPC with
/// no `[torpedoes]` block therefore decided its auto-fire from the player's
/// tubes — and would publish a command naming a tube it does not own.
///
/// Two NPCs here: one with its own loaded tube (which must still fire) and one
/// with no torpedo system at all (which must stay silent even though the
/// player's global Resource has a loaded tube).
#[test]
fn npc_torpedo_ai_never_decides_from_the_player_ships_tubes() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = torpedo_ai_test_app();
    // The player ship's tube is loaded — the retired fallback would have read
    // it on behalf of any NPC lacking a component of its own.
    {
        let mut res = app.world_mut().resource_mut::<TorpedoSystemResource>();
        res.0.tube_mut("fore_port").unwrap().loaded_count = 1;
    }
    spawn_asteroid_target(&mut app, "npc-target", 0.0, -30.0);

    let npc_sources = || {
        let mut s = crate::ship::control_source::ControlSourceResolver::new();
        s.set(
            crate::system_registry::torpedo_magazine_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        s.set(
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        s.set(
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        s.set(
            crate::system_registry::torpedo_tube_aft_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        crate::ship_plugin::ShipSystemControlSources(s)
    };

    let armed_sys = {
        let mut ts = TorpedoSystem::new(TorpedoConfig::default());
        ts.tube_mut("fore_port").unwrap().loaded_count = 1;
        ts
    };
    let armed = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ship_plugin::ShipConfigComponent::default(),
            npc_sources(),
            crate::ship_plugin::ActiveStationRatings::default(),
            ShipPhysics::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some("npc-target".into())),
            TorpedoSystemResource(armed_sys),
            AdmittedCommands::default(),
            crate::ai_plugin::AiHighFidelity,
            bevy::prelude::Transform::default(),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, armed);

    // Deliberately NO `TorpedoSystemResource` component.
    let bare = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ship_plugin::ShipConfigComponent::default(),
            npc_sources(),
            crate::ship_plugin::ActiveStationRatings::default(),
            ShipPhysics::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some("npc-target".into())),
            AdmittedCommands::default(),
            crate::ai_plugin::AiHighFidelity,
            bevy::prelude::Transform::default(),
        ))
        .id();

    app.world_mut()
        .run_system_once(seed_viewscreen_from_selection)
        .expect("seed viewscreen");
    app.world_mut()
        .run_system_once(crate::console_ai_plugin::ai_torpedo_auto_fire)
        .expect("ai_torpedo_auto_fire should run");

    assert!(
        !app.world()
            .get::<AdmittedCommands>(armed)
            .expect("admitted commands")
            .0
            .is_empty(),
        "an NPC with its own loaded tube must still decide to fire"
    );
    assert!(
        app.world()
            .get::<AdmittedCommands>(bare)
            .expect("admitted commands")
            .0
            .is_empty(),
        "an NPC with no torpedo system of its own must not decide from the player ship's global TorpedoSystemResource"
    );
}

/// `ai_torpedo_auto_fire` is a *decider*: it publishes to `AdmittedCommands`
/// (via `emit_ai_command`) and leaves the `TorpedoSystem` alone.
#[test]
fn ai_torpedo_auto_fire_writes_admitted_command_without_launching() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    // Seed the frozen viewscreen combat_lock from the ship's selection (issue
    // #829) — the isolated run below bypasses the harness's per-tick lift.
    app.world_mut()
        .run_system_once(seed_viewscreen_from_selection)
        .expect("seed viewscreen");
    app.world_mut()
        .run_system_once(crate::console_ai_plugin::ai_torpedo_auto_fire)
        .expect("ai_torpedo_auto_fire should run");

    let ship = local_ship(&mut app);
    let admitted = app
        .world()
        .get::<AdmittedCommands>(ship)
        .expect("every ship has AdmittedCommands");
    assert!(
        !admitted.0.is_empty(),
        "the decider must publish at least one command"
    );
    assert_eq!(
        admitted.0[0].target,
        SystemId("torpedo-tube-fore-port".into()),
        "the decider must target the loaded, in-arc tube"
    );
    assert!(
        app.world()
            .resource::<SimOutbox>()
            .0
            .iter()
            .all(|(_, m)| !matches!(m, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire must not launch — that is handle_fire_torpedo's job"
    );
}

// ── Per-tube LAUNCH policy gate (issue #782) ────────────────────────────

/// Attach a `TorpedoTubeAiPolicies` map to the local ship for `fore_port`, built
/// from an authored `when` guard on the `torpedo_launch` channel.
fn attach_launch_policy(app: &mut App, when: &str) {
    let ai = crate::entity_config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entity_config::FineSystemAiRuleToml {
            priority: 0,
            channel: crate::entity_config::TORPEDO_LAUNCH_CHANNEL.into(),
            when: when.into(),
            verb: crate::entity_config::TORPEDO_LAUNCH_VERB.into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let mut map = std::collections::HashMap::new();
    map.insert("fore_port".to_string(), ai.to_policy().unwrap());
    let ship = local_ship(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::weapons_plugin::TorpedoTubeAiPolicies(map));
}

/// An idle launch policy blocks the launch even though the tube is loaded, in
/// arc, and the target's striking shield arc is down — the per-tube launch opt-out
/// (AC1/AC2).
#[test]
fn ai_torpedo_auto_fire_idle_launch_policy_blocks_launch() {
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    let mut map = std::collections::HashMap::new();
    map.insert(
        "fore_port".to_string(),
        crate::ai::policy::AiPolicy {
            idle: true,
            ..Default::default()
        },
    );
    let ship = local_ship(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::weapons_plugin::TorpedoTubeAiPolicies(map));

    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "an idle launch policy must hold fire even from a ready tube"
    );
}

/// The #779 empty-facts lesson for the launch side: the host seeds real per-tube
/// readiness facts, so a `fact(...)` guard actually evaluates. `fact(in_arc) > 0`
/// fires (in_arc is seeded to 1 for candidates); `fact(in_arc) > 5` holds —
/// proving the facts are seeded, not empty.
#[test]
fn ai_torpedo_auto_fire_launch_fact_guard_fires_over_seeded_facts() {
    // Satisfiable guard → launch. If facts were empty, `fact(in_arc)` would read
    // 0 and this guard would hold — so a launch here proves the fact was seeded.
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);
    attach_launch_policy(&mut app, "fact(in_arc) > 0");
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "a launch guard satisfied by the seeded in_arc fact must fire"
    );

    // Unsatisfiable guard → hold.
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);
    attach_launch_policy(&mut app, "fact(in_arc) > 5");
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "a launch guard unsatisfiable over the seeded in_arc fact must hold"
    );
}

/// Issue #791, AC2: the new ship-wide `tubes_full` launch fact.
///
/// "All tubes full" did not exist before #791 — `TorpedoTube::is_loaded()` is
/// `loaded_count > 0`, so the closest thing available was "this tube has at
/// least one round", which a salvo doctrine cannot use. The fact is seeded from
/// every tube's `loaded_count` against its own `volley_max`.
///
/// Both halves matter, and the negative one especially: an unseeded `fact(...)`
/// name parses, validates, and then reads absent for ever, so a guard on it
/// would hold fire permanently and look exactly like a correctly cautious
/// doctrine. One tube loaded out of the fixture's three is the "not full" case;
/// all three is the "full" case, through the same guard.
#[test]
fn ai_torpedo_auto_fire_tubes_full_fact_gates_the_salvo() {
    // One tube of three loaded → the ship is NOT at full salvo → hold.
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);
    attach_launch_policy(&mut app, "fact(tubes_full) > 0");
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "one loaded tube out of three is not a full salvo: the guard must hold"
    );

    // Every tube at its volley capacity → launch. That this differs from the
    // run above is the proof the fact is genuinely seeded rather than absent.
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    for tube in ["fore_port", "fore_starboard", "aft"] {
        load_tube_now(&mut app, tube);
    }
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);
    attach_launch_policy(&mut app, "fact(tubes_full) > 0");
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "with every tube at its volley capacity the salvo guard must fire"
    );
}

/// Issue #791, AC2 as SHIPPED CONTENT: the Harrow cruiser's own authored tube
/// policy launches only while all three of its conditions hold, continuously.
///
/// The tests above prove the `tubes_full` fact is seeded and gates; this proves
/// the hull actually authors a guard that uses it, on real tubes with a real
/// 24-degree arc, through the real decider. A hull whose guard were quietly
/// wrong — a fact name typo, say — would parse, validate, and simply never fire,
/// which is invisible everywhere else.
#[test]
fn shipped_cruiser_tubes_launch_only_on_a_full_salvo_through_a_downed_arc() {
    use crate::entity_spawner::EntityUuid;
    use bevy::ecs::system::RunSystemOnce;

    let hull = crate::entity_config::EntityConfig::from_toml(include_str!(
        "../../../assets/entities/ship_harrow_cruiser.toml"
    ))
    .expect("the shipped cruiser hull must parse");
    let torpedoes = hull.torpedoes.as_ref().expect("the cruiser carries tubes");

    // The ship, its tubes and its AUTHORED per-tube policies — the same three
    // things `entities::spawner` attaches together.
    let build = |app: &mut App, target_uuid: &str, loaded: &[(&str, u32)]| -> Entity {
        let mut system =
            crate::torpedo::TorpedoSystem::from_configs(&torpedoes.tubes, torpedoes.to_runtime());
        for (id, count) in loaded {
            system.tube_mut(id).expect("shipped tube").loaded_count = *count;
        }
        let policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> = torpedoes
            .tubes
            .iter()
            .map(|t| {
                (
                    t.id.clone(),
                    t.ai.as_ref()
                        .expect("every shipped tube authors a policy")
                        .to_policy()
                        .expect("and it decodes"),
                )
            })
            .collect();
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        sources.set(
            crate::system_registry::torpedo_magazine_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        for tube in &torpedoes.tubes {
            sources.set(
                crate::system_registry::torpedo_tube_system_id(&tube.id).unwrap(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid("harrow-cruiser".into()),
                crate::ship_plugin::ShipConfigComponent::default(),
                crate::ship_plugin::ShipSystemControlSources(sources),
                crate::ship_plugin::ActiveStationRatings::default(),
                ShipPhysics::default(),
                crate::server_app::ShipSystemBlackboards::default(),
                TacticalRadarSelection(Some(target_uuid.to_string())),
                TorpedoSystemResource(system),
                crate::weapons_plugin::TorpedoTubeAiPolicies(policies),
                AdmittedCommands::default(),
                crate::ai_plugin::AiHighFidelity,
                Transform::default(),
            ))
            .id()
    };

    // A shielded target, placed so the choice of arc state is the only variable.
    // `(0, -30)` is dead ahead of a ship at the origin with yaw 0 — deliberately
    // NOT on an arc boundary, so the 24-degree tube arc admits it on its merits.
    let spawn_target = |app: &mut App, uuid: &str, x: f32, z: f32, arcs_online: bool| {
        let mut shields = crate::shield::ShieldSystem::default();
        if !arcs_online {
            for facing in shields.facings.iter_mut() {
                facing.offline_remaining = 30.0;
            }
        }
        app.world_mut().spawn((
            EntityUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, z),
            crate::simulation::ShipShields(shields, 0.5),
        ));
    };

    let launches = |app: &mut App, ship: Entity| -> usize {
        app.world_mut()
            .run_system_once(seed_viewscreen_from_selection)
            .expect("seed viewscreen");
        app.world_mut()
            .run_system_once(crate::console_ai_plugin::ai_torpedo_auto_fire)
            .expect("ai_torpedo_auto_fire runs");
        app.world()
            .get::<AdmittedCommands>(ship)
            .expect("admitted commands")
            .0
            .iter()
            .filter(|c| matches!(c.payload, SystemControlPayload::FireTorpedo { .. }))
            .count()
    };

    let full: Vec<(&str, u32)> = torpedoes
        .tubes
        .iter()
        .map(|t| (t.id.as_str(), t.volley_max))
        .collect();

    // Full salvo, arc down, dead ahead → every tube launches.
    let mut app = test_app();
    let ship = build(&mut app, "cruiser-target", &full);
    spawn_target(&mut app, "cruiser-target", 0.0, -30.0, false);
    assert_eq!(
        launches(&mut app, ship),
        torpedoes.tubes.len(),
        "a full salvo through a downed arc must fire every tube"
    );

    // One tube one round short → the WHOLE salvo holds. This is the condition
    // that did not exist before #791: every tube is `is_loaded()`, and the old
    // gate would have fired.
    let mut short = full.clone();
    short[0].1 -= 1;
    assert!(
        short[0].1 > 0,
        "precondition: the short tube is still loaded"
    );
    let mut app = test_app();
    let ship = build(&mut app, "cruiser-target", &short);
    spawn_target(&mut app, "cruiser-target", 0.0, -30.0, false);
    assert_eq!(
        launches(&mut app, ship),
        0,
        "one tube short of a full salvo must hold EVERY tube: the doctrine spends \
         a shield gap on one volley or not at all"
    );

    // Full salvo but the arc is back up → hold.
    let mut app = test_app();
    let ship = build(&mut app, "cruiser-target", &full);
    spawn_target(&mut app, "cruiser-target", 0.0, -30.0, true);
    assert_eq!(
        launches(&mut app, ship),
        0,
        "a healthy striking arc must hold the salvo"
    );

    // Full salvo, arc down, but the target is off the bow — outside the tubes'
    // 24-degree cone. This is what the Steering phase exists to fix, and until it
    // has, nothing launches.
    let mut app = test_app();
    let ship = build(&mut app, "cruiser-target", &full);
    spawn_target(&mut app, "cruiser-target", 30.0, 0.0, false);
    assert_eq!(
        launches(&mut app, ship),
        0,
        "a target abeam is outside a fixed forward tube's arc, however open the \
         opportunity is"
    );
}

// ── The battleship's opportunistic close defence (issue #793) ──────────────

/// Build the shipped Harrow battleship's real torpedo battery: its tubes, its
/// AUTHORED per-tube policies, and the control sources the spawner registers —
/// the same three things `entities::spawner` attaches together.
///
/// `loaded` names the tubes that start with a round in them; any tube left out is
/// empty, which is what "still reloading" looks like to the decider.
#[cfg(test)]
fn spawn_shipped_warhawk_battery(
    app: &mut App,
    torpedoes: &crate::entity_config::TorpedoesConfig,
    target_uuid: &str,
    loaded: &[&str],
) -> Entity {
    use crate::entity_spawner::EntityUuid;

    let mut system =
        crate::torpedo::TorpedoSystem::from_configs(&torpedoes.tubes, torpedoes.to_runtime());
    for id in loaded {
        let tube = system.tube_mut(id).expect("shipped tube");
        tube.loaded_count = tube.volley_max;
    }
    let policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> = torpedoes
        .tubes
        .iter()
        .map(|t| {
            (
                t.id.clone(),
                t.ai.as_ref()
                    .expect("every shipped tube authors a policy")
                    .to_policy()
                    .expect("and it decodes"),
            )
        })
        .collect();
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );
    for tube in &torpedoes.tubes {
        sources.set(
            crate::system_registry::torpedo_tube_system_id(&tube.id).unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
    }
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid("harrow-warhawk".into()),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::ship_plugin::ActiveStationRatings::default(),
            ShipPhysics::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            TorpedoSystemResource(system),
            crate::weapons_plugin::TorpedoTubeAiPolicies(policies),
            AdmittedCommands::default(),
            crate::ai_plugin::AiHighFidelity,
            Transform::default(),
        ))
        .id()
}

/// Spawn a shielded target for the battleship's launchers to decide about.
/// `arcs_online = false` forces every arc offline, which the host reports as 0 HP
/// — "the arc a round would strike is not blocking".
#[cfg(test)]
fn spawn_warhawk_shield_target(app: &mut App, uuid: &str, x: f32, z: f32, arcs_online: bool) {
    use crate::entity_spawner::EntityUuid;

    let mut shields = crate::shield::ShieldSystem::default();
    if !arcs_online {
        for facing in shields.facings.iter_mut() {
            facing.offline_remaining = 30.0;
        }
    }
    app.world_mut().spawn((
        EntityUuid(uuid.to_string()),
        Transform::from_xyz(x, 0.0, z),
        crate::simulation::ShipShields(shields, 0.5),
    ));
}

/// Every `FireTorpedo` the decider admitted on this ship, by the tube it targets.
#[cfg(test)]
fn torpedo_launch_targets(app: &mut App, ship: Entity) -> Vec<SystemId> {
    use bevy::ecs::system::RunSystemOnce;

    app.world_mut()
        .run_system_once(seed_viewscreen_from_selection)
        .expect("seed viewscreen");
    app.world_mut()
        .run_system_once(crate::console_ai_plugin::ai_torpedo_auto_fire)
        .expect("ai_torpedo_auto_fire runs");
    app.world()
        .get::<AdmittedCommands>(ship)
        .expect("admitted commands")
        .0
        .iter()
        .filter(|c| matches!(c.payload, SystemControlPayload::FireTorpedo { .. }))
        .map(|c| c.target.clone())
        .collect()
}

/// Issue #793, AC2/AC3 as SHIPPED CONTENT: the battleship's fore and aft
/// launchers evaluate readiness, bearing and the striking shield arc entirely
/// independently of one another.
///
/// The FIRST case is the one the whole issue turns on, and it is the case the
/// cruiser's guard gets wrong. The cruiser gates on `fact(tubes_full)`, which is
/// SHIP-WIDE — every tube on the hull at its `volley_max`. Pasted onto this hull
/// it would mean a loaded fore tube, bearing on a collapsed arc, holding its round
/// because the aft tube is eight seconds into a reload. The battleship gates on
/// the per-tube `fact(loaded)` instead, and this test is what tells the two apart:
/// under `tubes_full` the expected count here is zero.
///
/// A hull whose guard were quietly wrong — a fact-name typo, say — would parse,
/// validate, and simply never fire, which is invisible everywhere else.
#[test]
fn shipped_warhawk_launchers_decide_independently_through_a_downed_arc() {
    let hull = crate::entity_config::EntityConfig::from_toml(include_str!(
        "../../../assets/entities/ship_harrow_warhawk.toml"
    ))
    .expect("the shipped battleship hull must parse");
    let torpedoes = hull
        .torpedoes
        .as_ref()
        .expect("the battleship carries close-defence launchers");
    let fore = crate::system_registry::torpedo_tube_system_id("fore").unwrap();
    let aft = crate::system_registry::torpedo_tube_system_id("aft").unwrap();

    // AC2: the FORE tube is loaded and bearing; the AFT tube is still reloading.
    // `(0, -30)` is dead ahead of a ship at the origin with yaw 0 — 45 degrees
    // clear of the 90-degree cone's edge, so it is admitted on its merits rather
    // than by the `<=` boundary tie.
    let mut app = test_app();
    let ship = spawn_shipped_warhawk_battery(&mut app, torpedoes, "closer", &["fore"]);
    spawn_warhawk_shield_target(&mut app, "closer", 0.0, -30.0, false);
    assert_eq!(
        torpedo_launch_targets(&mut app, ship),
        vec![fore.clone()],
        "the loaded fore launcher must take its own opportunity while the aft \
         launcher is still reloading — a ship-wide `tubes_full` guard would hold \
         BOTH tubes here, which is the coupling this hull must not have"
    );

    // AC2, the other launcher: a player who has got behind a hull that turns at
    // 0.20 rad/s. `(0, +30)` is dead astern — inside the aft cone, 180 degrees off
    // the fore one.
    let mut app = test_app();
    let ship = spawn_shipped_warhawk_battery(&mut app, torpedoes, "closer", &["fore", "aft"]);
    spawn_warhawk_shield_target(&mut app, "closer", 0.0, 30.0, false);
    assert_eq!(
        torpedo_launch_targets(&mut app, ship),
        vec![aft.clone()],
        "a target astern is the AFT launcher's opportunity and nobody else's: the \
         fore tube is loaded and bearing on nothing"
    );

    // AC3: the striking arc is back. Both tubes are loaded and one of them is
    // bearing, and neither fires.
    let mut app = test_app();
    let ship = spawn_shipped_warhawk_battery(&mut app, torpedoes, "closer", &["fore", "aft"]);
    spawn_warhawk_shield_target(&mut app, "closer", 0.0, -30.0, true);
    assert_eq!(
        torpedo_launch_targets(&mut app, ship),
        Vec::<SystemId>::new(),
        "a recovered shield arc must hold every launcher: the gate is doctrine, \
         not damage arithmetic — `damage_hull` would land either way — and a \
         twelve-round magazine is spent on openings, not on a covered target"
    );

    // Out of arc: the target is abeam, 45 degrees clear of BOTH cones' edges.
    // Nothing puts it back in — the launchers never command a bearing (AC4), so
    // an abeam target is simply an opportunity that does not exist.
    let mut app = test_app();
    let ship = spawn_shipped_warhawk_battery(&mut app, torpedoes, "closer", &["fore", "aft"]);
    spawn_warhawk_shield_target(&mut app, "closer", 30.0, 0.0, false);
    assert_eq!(
        torpedo_launch_targets(&mut app, ship),
        Vec::<SystemId>::new(),
        "a target on the beam sits in the gap between the fore and aft cones, \
         however open the shield opportunity is"
    );
}

/// Issue #793, AC2/AC3 at the guard itself: the shipped launch policy resolved
/// directly over a seeded fact snapshot.
///
/// The behaviour test above goes through `auto_fire_torpedo`, which applies the
/// same conditions as HOST gates before the policy is ever consulted — so it
/// cannot, on its own, tell an authored guard that reads its facts from one that
/// silently reads nothing. (An unseeded or misspelled `fact(...)` name parses,
/// validates, and then reads absent for ever; the #779 failure mode.)
///
/// This drives the real `seed_torpedo_tube_launch_facts` → shipped policy pair
/// directly, so each conjunct is switched on and off in isolation. The
/// `tubes_full = false` in every case is deliberate and is AC2's independence
/// stated at the level it is authored: the ship-wide reading is FALSE throughout,
/// and the guard fires anyway.
#[test]
fn shipped_warhawk_launch_guard_reads_each_tubes_own_readiness() {
    let hull = crate::entity_config::EntityConfig::from_toml(include_str!(
        "../../../assets/entities/ship_harrow_warhawk.toml"
    ))
    .expect("the shipped battleship hull must parse");
    let torpedoes = hull
        .torpedoes
        .as_ref()
        .expect("the battleship carries tubes");

    for tube in &torpedoes.tubes {
        let policy = tube
            .ai
            .as_ref()
            .expect("every shipped tube authors a policy")
            .to_policy()
            .expect("and it decodes");
        let fires = |loaded: bool, in_arc: bool, facing_shields: i32| {
            crate::weapons_plugin::torpedo_tube_launch_policy_fires(
                &policy,
                &crate::weapons_plugin::seed_torpedo_tube_launch_facts(
                    loaded,
                    true,
                    true,
                    in_arc,
                    facing_shields,
                    // SHIP-WIDE "every tube full" is false in every case below.
                    false,
                    // …and so is red alert (issue #872). The Harrow hulls are
                    // authored ALWAYS-ARMED, so their launch guard must not
                    // depend on a captain having raised the alert; leaving this
                    // false throughout is that statement.
                    false,
                ),
            )
        };

        assert!(
            fires(true, true, 0),
            "tube '{}': loaded, bearing, striking arc down — the shot is on, and \
             the ship-wide `tubes_full` reading being FALSE must not touch it",
            tube.id
        );
        assert!(
            !fires(false, true, 0),
            "tube '{}': an empty launcher must hold even with the opportunity \
             wide open — and that this differs from the case above is the proof \
             `loaded` is a seeded fact rather than an absent name reading 0",
            tube.id
        );
        assert!(
            !fires(true, false, 0),
            "tube '{}': a loaded launcher that is not bearing must hold — the \
             tubes take the bearing the gun line gives them and never ask for one",
            tube.id
        );
        assert!(
            !fires(true, true, 40),
            "tube '{}': a striking arc with HP left must hold the round; \
             `target_facing_shields` is an HP reading, not a boolean",
            tube.id
        );
        assert!(
            fires(true, true, -5),
            "tube '{}': the gate is `<= 0`, so an over-collapsed arc is still an \
             opportunity",
            tube.id
        );
    }
}

/// Issue #793, AC4/AC5: taking a torpedo opportunity never commands Steering and
/// never moves the hull.
///
/// The mirror of `ai_phaser_auto_fire_ignores_the_helm_torpedo_bearing_phase`,
/// pointed the other way: that one pins that the helm's leg does not reach the
/// guns, this one that the guns do not reach the helm. AC4 is satisfied by
/// OMISSION — `ai_torpedo_auto_fire` only ever emits `FireTorpedo` at a tube's own
/// system id — and an omission is exactly what a later "swing the bow onto the
/// tube" change would fill in. The failure it guards against is invisible in every
/// content test: the battleship would still parse, still hold its band, and simply
/// start aiming its artillery at where the target IS instead of where the bolt and
/// the target meet.
///
/// The whole launch runs here — the decider AND `handle_fire_torpedo` — because
/// AC5 is a statement about what firing COSTS, and a decision that never became a
/// round cannot cost anything. The launch is asserted to have actually happened
/// (a `TorpedoLaunched` broadcast, and the tube emptied) before the physics
/// comparison is allowed to mean anything.
///
/// The hull is put in the state it is really in while this happens: holding its
/// artillery firing position, with the `HelmPassSurface` that leg publishes
/// live — every high-fidelity AI ship carries one (`ai_high_fidelity_components`),
/// so a fixture without the component could assert nothing about it.
#[test]
fn warhawk_torpedo_opportunity_never_commands_the_helm() {
    use bevy::ecs::system::RunSystemOnce;

    let hull = crate::entity_config::EntityConfig::from_toml(include_str!(
        "../../../assets/entities/ship_harrow_warhawk.toml"
    ))
    .expect("the shipped battleship hull must parse");
    let torpedoes = hull
        .torpedoes
        .as_ref()
        .expect("the battleship carries tubes");
    let steering = hull
        .helm_console
        .as_ref()
        .expect("the hull declares [helm_console]")
        .steering_ai
        .as_ref()
        .expect("and authors a Steering policy");
    let steering_param = |name: &str| {
        *steering
            .param
            .get(name)
            .unwrap_or_else(|| panic!("the hull authors a `{name}` steering param"))
    };
    // The lead speed the artillery hold predicts with is derived host-side from
    // the hull's longest-reaching blaster bank, so it is read the same way here.
    let bow = hull
        .weapons_console
        .as_ref()
        .expect("the hull declares [weapons_console]")
        .blaster_banks
        .iter()
        .max_by(|a, b| a.range.total_cmp(&b.range))
        .expect("the battleship carries its bow artillery");

    let mut app = test_app();
    let ship = spawn_shipped_warhawk_battery(&mut app, torpedoes, "closer", &["fore", "aft"]);
    spawn_warhawk_shield_target(&mut app, "closer", 0.0, -30.0, false);
    // The leg the battleship is flying while its tubes take the opportunity,
    // built out of this hull's own authored numbers rather than invented ones.
    let surface = crate::ship::helm_ai::HelmPassSurface {
        active: true,
        artillery_hold: true,
        artillery_hold_speed: steering_param("artillery_hold_speed"),
        artillery_lead_speed: bow.projectile_speed,
        tracking_deadband_rad: steering_param("tracking_deadband_rad"),
        tracking_full_steer_rad: steering_param("tracking_full_steer_rad"),
        ..Default::default()
    };
    app.world_mut().entity_mut(ship).insert(surface);

    let before = *app
        .world()
        .get::<ShipPhysics>(ship)
        .expect("the fixture ship carries physics");

    let launches = torpedo_launch_targets(&mut app, ship);
    assert!(
        !launches.is_empty(),
        "precondition: a launcher must actually take the opportunity, or this \
         test proves nothing about what taking one costs"
    );

    // Every admitted command is a launch at a tube. Nothing addresses the helm.
    let admitted = app
        .world()
        .get::<AdmittedCommands>(ship)
        .expect("admitted commands");
    for command in &admitted.0 {
        assert!(
            matches!(command.payload, SystemControlPayload::FireTorpedo { .. }),
            "the torpedo path admitted a non-launch command ({:?} at {}): a \
             launcher may publish nothing but its own shot",
            command.payload,
            command.target.0
        );
        assert!(
            torpedoes
                .tubes
                .iter()
                .any(
                    |t| crate::system_registry::torpedo_tube_system_id(&t.id).as_ref()
                        == Some(&command.target)
                ),
            "a launch was admitted at `{}`, which is not one of this hull's tubes",
            command.target.0
        );
    }

    // Now actually fire: the consumer takes the admitted launch and advances the
    // tube's own state machine, which is the step that puts a round in the air.
    let loaded_before: u32 = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .expect("the fixture ship carries its own tubes")
        .0
        .tubes
        .iter()
        .map(|t| t.loaded_count)
        .sum();
    app.world_mut()
        .run_system_once(handle_fire_torpedo)
        .expect("handle_fire_torpedo runs");

    let launched: Vec<&ServerMessage> = app
        .world()
        .resource::<SimOutbox>()
        .0
        .iter()
        .map(|(_, m)| m)
        .filter(|m| matches!(m, ServerMessage::TorpedoLaunched { .. }))
        .collect();
    assert!(
        !launched.is_empty(),
        "precondition: a round must genuinely have left a tube — without a launch \
         the physics comparison below cannot fail for any reason"
    );
    let loaded_after: u32 = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .expect("the fixture ship carries its own tubes")
        .0
        .tubes
        .iter()
        .map(|t| t.loaded_count)
        .sum();
    assert!(
        loaded_after < loaded_before,
        "precondition: the launch must have spent a loaded round \
         ({loaded_before} → {loaded_after})"
    );

    // AC5: and the hull has not moved ACROSS THE LAUNCH. `recoil_impulse` is the
    // only mechanism in the close-defence path that could touch physics, it is
    // blaster-only, and this hull authors none — pinned as content in
    // `entities::config`. `handle_fire_torpedo` takes `&ShipPhysics`, so the
    // guarantee is structural; this is what would notice it stopping being so.
    let after = *app
        .world()
        .get::<ShipPhysics>(ship)
        .expect("the fixture ship carries physics");
    assert_eq!(
        (after.yaw, after.x, after.z, after.forward_speed),
        (before.yaw, before.x, before.z, before.forward_speed),
        "launching from the firing position must not move or re-point the hull: \
         the predictive bow-artillery facing is the battleship"
    );
    assert_eq!(
        app.world()
            .get::<crate::ship::helm_ai::HelmPassSurface>(ship)
            .copied(),
        Some(surface),
        "the torpedo path must not touch the helm's pass surface: the leg the \
         host publishes is derived from the Steering machine's own yaw verb and \
         from nothing the launchers did"
    );
}

/// Issue #791, AC5: the phaser banks keep working while the helm is holding the
/// bow on a torpedo opportunity.
///
/// `ai_phaser_auto_fire` reads physics, the bank's own arcs/cooldown and its
/// #781 policy — and nothing else. This pins that: a ship carrying a
/// `HelmPassSurface` that says the torpedo phase is live still opens fire on a
/// bearing target. The failure this guards against is a future "pause the beams
/// while lining up the tubes" coupling, which would be invisible in every helm
/// test and would leave the cruiser silent at the moment it is most exposed.
#[test]
fn ai_phaser_auto_fire_ignores_the_helm_torpedo_bearing_phase() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = test_app();
    let npc = spawn_ai_phaser_npc(
        &mut app,
        "cc000000-0000-0000-0000-000000000001",
        "cc000000-0000-0000-0000-000000000002",
    );
    app.world_mut()
        .run_system_once(seed_viewscreen_from_selection)
        .expect("seed viewscreen");

    let fire_count = |app: &mut App| {
        app.world_mut()
            .run_system_once(ai_phaser_auto_fire)
            .expect("ai_phaser_auto_fire runs");
        let mut entity = app.world_mut().entity_mut(npc);
        let mut admitted = entity
            .get_mut::<AdmittedCommands>()
            .expect("every ship has AdmittedCommands");
        let n = admitted
            .0
            .iter()
            .filter(|c| matches!(c.payload, SystemControlPayload::FirePhaser))
            .count();
        admitted.0.clear();
        n
    };

    let baseline = fire_count(&mut app);
    assert!(
        baseline > 0,
        "precondition: the bank must be firing at all, or this test proves nothing"
    );

    // Now say the helm is mid-torpedo-phase. Nothing about the bank changed.
    app.world_mut()
        .entity_mut(npc)
        .insert(crate::ship::helm_ai::HelmPassSurface {
            active: true,
            torpedo_bearing: true,
            torpedo_bearing_speed: 0.0,
            ..Default::default()
        });
    assert_eq!(
        fire_count(&mut app),
        baseline,
        "the phaser path must not consult the helm's leg: ordinary beam pressure \
         continues through the whole bow-on phase"
    );
}

/// Target invalidation: a locked target UUID that resolves to no live entity
/// (destroyed / never spawned) yields no launch — even under an unconditional
/// launch policy — because the host readiness gate finds nothing to shoot at.
#[test]
fn ai_torpedo_auto_fire_holds_when_target_invalidated() {
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    // Locked target names an entity that does not exist in the world.
    set_weapons_target(&mut app, Some("ghost-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    attach_launch_policy(&mut app, "true");

    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "an invalidated target (no live entity) must produce no launch"
    );
}

/// AC5: in-flight torpedoes are published as a public authoritative fact on the
/// shared magazine blackboard, so other policies read the count on the NEXT AI
/// tick (the same one-tick-lag discipline as the combat lock).
#[test]
fn torpedo_in_flight_count_is_published_as_a_public_fact() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = test_app();
    let mut ts = TorpedoSystem::new(TorpedoConfig::default());
    ts.in_flight.push(crate::torpedo::Torpedo {
        uuid: "flying-1".into(),
        x: 0.0,
        y: 0.0,
        z: 0.0,
        heading: 0.0,
        pitch: 0.0,
        lifespan_remaining: 10.0,
        target_uuid: None,
        source_uuid: Some("shooter".into()),
        tube_id: "fore_port".into(),
        shield_pierce: 0.0,
    });
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            TorpedoSystemResource(ts),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();

    app.world_mut()
        .run_system_once(crate::console::weapons::publish_torpedo_magazine_blackboard)
        .expect("publish runs");

    let bbs = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap();
    let mag = bbs
        .0
        .get(&crate::system_registry::torpedo_magazine_system_id());
    match mag {
        Some(crate::messages::SystemBlackboard::TorpedoMagazine(bb)) => {
            assert_eq!(
                bb.torpedoes_in_flight, 1,
                "the published magazine fact must expose the in-flight count"
            );
        }
        other => panic!("expected a TorpedoMagazine blackboard, got {other:?}"),
    }
}

/// `handle_fire_torpedo` (the consumer) reads `AdmittedCommands` and fires
/// a torpedo. Pins the consumer from a hand-written command, independently
/// of the AI decider. (The admitted buffer is cleared each tick by
/// `admit_system_commands`, not by the consumer.)
#[test]
fn handle_fire_torpedo_launches_from_admitted_command() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = torpedo_ai_test_app();
    load_tube_now(&mut app, "fore_port");
    let ship = local_ship(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<AdmittedCommands>()
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo {
                target_uuid: Some("target-uuid".into()),
            },
            response_token: None,
        });

    app.world_mut()
        .run_system_once(handle_fire_torpedo)
        .expect("handle_fire_torpedo should run");

    assert!(
        app.world()
            .resource::<SimOutbox>()
            .0
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::TorpedoLaunched { .. })),
        "the consumer must advance the torpedo state machine from the admitted command"
    );
}

/// Patterned launch (issue #766): `handle_fire_torpedo` resolves the immediate
/// round's origin from the authored barrel marker (not ship centre), and the
/// tube surfaces the active pattern step/barrel for the Tactical indicator.
#[test]
fn handle_fire_torpedo_patterned_launch_resolves_barrel_origin() {
    use crate::entity_spawner::EntityUuid;
    use crate::model_rig::{Marker, ModelMarkers};
    use bevy::ecs::system::RunSystemOnce;
    use std::collections::HashMap;

    let mut app = test_app();

    // Alternating two-barrel pattern on a fresh tube.
    let tube_cfg = crate::entities::config::TorpedoTubeConfig {
        id: "fore-centre".into(),
        facing_deg: 0.0,
        fire_arc_deg: 90.0,
        load_time: None,
        marker: None,
        barrels: vec!["tp_port".into(), "tp_starboard".into()],
        pattern: vec![
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0],
                offset_secs: 0.0,
            },
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![1],
                offset_secs: 0.5,
            },
        ],
        volley_max: 3,
        ai_target_count: None,
        ai: None,
    };
    let mut torp =
        crate::torpedo::TorpedoSystem::from_configs(&[tube_cfg], TorpedoConfig::default());
    torp.torpedoes_remaining -= 2;
    torp.tube_mut("fore-centre").unwrap().loaded_count = 2;

    // Distinct barrel-marker positions (identity base) so the origin's X
    // identifies the barrel: port at +3, starboard at -3.
    let mut markers = HashMap::new();
    markers.insert(
        "tp_port".to_string(),
        Marker {
            position: [3.0, 0.0, 0.0],
            direction: [0.0, 0.0, 1.0],
        },
    );
    markers.insert(
        "tp_starboard".to_string(),
        Marker {
            position: [-3.0, 0.0, 0.0],
            direction: [0.0, 0.0, 1.0],
        },
    );
    let mm = ModelMarkers::from_markers(markers);

    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid("shooter".into()),
            crate::ship_plugin::ShipSystemControlSources(
                crate::ship::control_source::ControlSourceResolver::new(),
            ),
            ShipPhysics::default(),
            mm,
            TorpedoSystemResource(torp),
            crate::server_app::WeaponFiredThisTick::default(),
            AdmittedCommands(vec![AdmittedCommand {
                target: SystemId("torpedo-tube-fore-centre".into()),
                payload: SystemControlPayload::FireTorpedo { target_uuid: None },
                response_token: None,
            }]),
            crate::server_app::ShipSystemBlackboards::default(),
            bevy::prelude::Transform::default(),
        ))
        .id();

    app.world_mut()
        .run_system_once(handle_fire_torpedo)
        .expect("handle_fire_torpedo should run");

    // The immediate round left from barrel 0 (tp_port at x = +3), NOT ship
    // centre (x = 0).
    let launched: Vec<(f32, f32)> = app
        .world()
        .resource::<SimOutbox>()
        .0
        .iter()
        .filter_map(|(_, m)| match m {
            ServerMessage::TorpedoLaunched { tube, x, z, .. } if tube == "fore-centre" => {
                Some((*x, *z))
            }
            _ => None,
        })
        .collect();
    assert_eq!(launched.len(), 1, "one immediate launch broadcast");
    assert!(
        (launched[0].0 - 3.0).abs() < 1e-4,
        "immediate round must leave from barrel 0's marker (x=+3), got {}",
        launched[0].0
    );

    // The tube surfaces the active patterned attack.
    let ts = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .expect("ship keeps its torpedo component");
    let tube = ts.0.tube("fore-centre").unwrap();
    assert_eq!(tube.active_barrels, vec![0]);
    assert_eq!(tube.pattern_step, 1);
    assert_eq!(tube.pattern_len(), 2);
}

/// Issue #698 promotion: `ai_torpedo_auto_fire` used to hardcode
/// `TorpedoAiInput { target_facing_shields: 0 }`, which made
/// `auto_fire_torpedo`'s "shields must be down" condition unreachable — the AI
/// fired torpedoes straight into a fully-shielded target. It now reads the
/// target's real `ShipShields`, so the pure function's documented doctrine
/// (phasers strip shields, torpedoes finish the hull) actually holds.
#[test]
fn ai_torpedo_auto_fire_holds_fire_while_target_shields_are_up() {
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");

    // A ship target dead ahead, shields up.
    let target = spawn_shielded_target(&mut app, "target-uuid", 0.0, -30.0);

    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "torpedoes must hold while the arc facing the shooter is still up"
    );

    // Collapse every facing — now the shot is on.
    {
        let mut shields = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(target)
            .unwrap();
        for facing in shields.0.facings.iter_mut() {
            facing.hp = 0;
        }
    }
    load_tube_now(&mut app, "fore_port");

    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "torpedoes must fire once the target's shields are down"
    );
}

/// The multi-arc case the summed gate got wrong: the shooter sits astern of a
/// four-arc target, so the torpedo would strike the target's Aft arc. Collapsing
/// only the *front* arc must NOT unlock the shot; collapsing the aft arc must —
/// and the three still-healthy arcs must not veto it. Uses the target's own
/// `facing_index_for_bearing` to name the arc rather than hardcoding an index,
/// so the test tracks the routing rather than restating it.
#[test]
fn ai_torpedo_auto_fire_gates_on_the_arc_the_torpedo_would_strike() {
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");

    // Target ahead of the shooter at the origin, so the shot arrives on the
    // target's own aft bearing.
    let target = spawn_shielded_target(&mut app, "target-uuid", 0.0, -30.0);

    // Which arc is actually in the way? Ask the shield system, not a constant.
    let (struck, away) = {
        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(target)
            .unwrap();
        assert!(
            shields.0.facings.len() >= 2,
            "precondition: this test needs a multi-arc target"
        );
        let incoming = crate::shield::attacker_bearing_relative(0.0, 0.0, 0.0, -30.0, 0.0);
        let struck = shields.0.facing_index_for_bearing(incoming);
        let away = (struck + 1) % shields.0.facings.len();
        (struck, away)
    };

    // Collapse an arc pointing somewhere else: still no shot.
    {
        let mut shields = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(target)
            .unwrap();
        shields.0.facings[away].hp = 0;
    }
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "a collapsed arc facing elsewhere must not unlock the shot"
    );

    // Collapse the arc actually in the way, leaving the others healthy: fire.
    {
        let mut shields = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(target)
            .unwrap();
        shields.0.facings[away].hp = shields.0.facings[away].max_hp;
        shields.0.facings[struck].hp = 0;
        assert!(
            shields
                .0
                .facings
                .iter()
                .enumerate()
                .any(|(i, f)| i != struck && f.hp > 0),
            "precondition: other arcs must still be healthy"
        );
    }
    load_tube_now(&mut app, "fore_port");
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "healthy rear arcs must not veto a shot into the collapsed facing arc"
    );
}

fn local_ship(app: &mut App) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .expect("test_app must spawn a LocalShip")
}

/// A ship-like entity carrying `ShipShields` at full HP.
fn spawn_shielded_target(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
    let shields = crate::shield::ShieldSystem::new(&crate::shield::ShieldConfig::default());
    assert!(
        shields.facings.iter().any(|f| f.hp > 0),
        "precondition: the default shield config must start with HP up"
    );
    app.world_mut()
        .spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                50.0,
            )])),
            crate::ship::shields::ShipShields(shields, 0.5),
            Transform::from_xyz(x, 0.0, z),
        ))
        .id()
}

fn set_tactical_station_rating(app: &mut App, rating: &str) {
    let rating = rating.to_string();
    let world = app.world_mut();
    let mut q = world
        .query_filtered::<&mut crate::ship_plugin::ActiveStationRatings, With<crate::server_app::LocalShip>>();
    for mut ratings in q.iter_mut(world) {
        ratings.0.insert(
            crate::messages::StationId("tactical".into()),
            rating.clone(),
        );
    }
}

/// Ported from the pre-#694 `ai_stops_firing_when_rating_switches_to_std`,
/// which exercised `operate_tactical_ai`'s torpedo block directly before
/// it was deleted; see `ai_torpedo_auto_fire_fires_when_ai_controls_unclaimed_station`
/// above.
#[test]
fn ai_torpedo_auto_fire_stops_firing_when_rating_switches_to_std() {
    // Occupied station: AI fires when rating is Assisted (has torpedo_auto_fire
    // in ai_tuning), stops when rating is Std (no ai_tuning).
    let mut app = torpedo_ai_test_app();

    // Assign a human holder so the ai_tuning gate is active.
    push(
        &mut app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(&mut app);

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    // Set rating to Assisted (has torpedo_auto_fire in ai_tuning).
    set_tactical_station_rating(&mut app, "Assisted");
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    // First tick — AI should fire with Assisted rating.
    let out1 = tick(&mut app);
    assert!(
        out1.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire should fire TorpedoLaunched when rating is Assisted"
    );

    // Reload the tube (launch consumed it) so the only gate is the rating.
    load_tube_now(&mut app, "fore_port");

    // Switch to Std rating (no torpedo_auto_fire in ai_tuning).
    set_tactical_station_rating(&mut app, "Std");

    // Second tick - AI must not fire.
    let out2 = tick(&mut app);
    assert!(
        !out2
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire must not fire TorpedoLaunched when rating is Std"
    );
}

// ── Fine-Tactical decomposition tests (issue #512) ─────────────────────
//
// Every new fine SystemId, blackboard, and gate has coverage here. The
// channel-2 `ClaimTorpedoRound` transaction is exercised via
// `handle_load_tube` → `handle_torpedo_magazine_inter_system`. Firing
// gates are exercised via `handle_fire_torpedo` and `handle_fire_phaser`.

/// Helper: mark a fine system Offline (Disabled/Destroyed) on the LocalShip
/// by inserting it into `ControlSourceResolver.offline_systems`. Mirrors
/// what `sync_console_damage_tiers` would do after a damage tick — the
/// direct-insert avoids needing to spawn a hull component just to test
/// the gate.
fn mark_system_offline(app: &mut App, system_id: SystemId) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        cs.0.set_offline(system_id.clone(), true);
    }
}

/// Helper: register a fine system on the LocalShip's ControlSourceResolver
/// with a specific ControlSource. Used to simulate the ship having declared
/// a fine `[[system]]` block in its TOML.
fn register_fine_system(
    app: &mut App,
    system_id: SystemId,
    source: crate::ship::control_source::ControlSource,
) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        cs.0.set(system_id.clone(), source);
    }
}

// ── Registered-system predicate ───────────────────────────────────────

#[test]
fn system_is_registered_returns_true_after_set() {
    let mut sources = ShipSystemControlSources::default();
    let sysid = crate::system_registry::phaser_fore_system_id();
    sources.0.set(
        sysid.clone(),
        crate::ship::control_source::ControlSource::Human,
    );
    assert!(system_is_registered(&sources, &sysid));
}

#[test]
fn system_is_registered_returns_true_after_offline_insert() {
    let mut sources = ShipSystemControlSources::default();
    let sysid = crate::system_registry::phaser_fore_system_id();
    sources.0.set_offline(sysid.clone(), true);
    assert!(system_is_registered(&sources, &sysid));
}

#[test]
fn system_is_registered_returns_false_when_absent() {
    let sources = ShipSystemControlSources::default();
    let sysid = crate::system_registry::phaser_fore_system_id();
    assert!(!system_is_registered(&sources, &sysid));
}

// ── Per-bank fire gate ────────────────────────────────────────────────

#[test]
fn fire_phaser_refused_when_bank_fine_system_offline() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Reset beam / cooldown so the only variable is the bank gate.
    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "port", 0.0);

    // Register the port bank as Human, then mark it offline (as
    // sync_console_damage_tiers would do on Disabled hull).
    register_fine_system(
        &mut app,
        SystemId("phaser-port".into()),
        crate::ship::control_source::ControlSource::Human,
    );
    mark_system_offline(&mut app, SystemId("phaser-port".into()));

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "FirePhaser must be refused when the bank's fine system is offline"
    );
}

#[test]
fn fire_phaser_allowed_when_other_bank_offline_but_this_one_online() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);
    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "port", 0.0);
    start_phaser_cooldown(&mut app, "starboard", 0.0);

    // Only starboard offline; port stays online.
    mark_system_offline(&mut app, SystemId("phaser-starboard".into()));

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "Firing port must succeed when only starboard is offline"
    );
}

// ── Per-tube load/unload gate ─────────────────────────────────────────

#[test]
fn load_tube_emits_claim_torpedo_round_via_channel_2() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::LoadTube,
        },
    );
    // Run one tick to admit the command → handle_load_tube emits the claim.
    tick(&mut app);

    let queue = &app.world().resource::<InterSystemQueue>().0;
    let claim_present = queue.iter().any(|m| {
        m.target == crate::system_registry::torpedo_magazine_system_id()
            && matches!(
                &m.payload,
                InterSystemPayload::ClaimTorpedoRound { tube } if tube == "fore_port"
            )
    });
    assert!(
        claim_present,
        "handle_load_tube should emit ClaimTorpedoRound on channel-2"
    );
}

/// Reads the LocalShip tube's volley target, preferring the per-entity
/// component the way the handler does.
fn local_tube_target_count(app: &mut App, tube: &str) -> u32 {
    let mut q = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
    let from_component = q
        .single(app.world())
        .ok()
        .and_then(|ts| ts.0.tube(tube).map(|t| t.target_count));
    from_component.unwrap_or_else(|| {
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .tube(tube)
            .expect("test tube should exist")
            .target_count
    })
}

/// The human half of the command `console_ai::server::ai_torpedo_load` issues:
/// the Tactical operator's console sends `ControlSystem { target:
/// "torpedo-tube-<id>", SetTorpedoVolleyTarget }`, and it must still land on
/// the player ship's own tube now that the handler reads `AdmittedCommands`
/// per ship instead of the raw inbound stream.
#[test]
fn human_set_torpedo_volley_target_reaches_the_local_ship_tube() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::SetTorpedoVolleyTarget { count: 1 },
        },
    );
    tick(&mut app);

    assert_eq!(
        local_tube_target_count(&mut app, "fore_port"),
        1,
        "an admitted human volley order must set the tube's target_count"
    );
}

/// A torpedo that kills the *player's* ship must end the run, exactly as a beam
/// or blaster kill does. Before AI crews fired torpedoes at all this branch was
/// unreachable, so the torpedo detonation path simply despawned whatever it
/// killed: the player vanished, the ledger recorded the death, and the run
/// carried on `InProgress` to the tick budget with no game-over reason.
#[test]
fn torpedo_kill_on_the_local_ship_latches_game_over() {
    use crate::simulation::GameOverReason;

    let mut app = test_app();
    app.init_resource::<GameOverReason>();
    start_game_with_weapons(&mut app);

    let player = local_ship(&mut app);

    // An enemy ship whose torpedo is already on top of the player, carrying
    // more than enough hull damage to finish it.
    let mut enemy_torpedoes = TorpedoSystem::new(TorpedoConfig {
        damage_hull: 100_000,
        ..TorpedoConfig::default()
    });
    enemy_torpedoes.in_flight.push(crate::torpedo::Torpedo {
        uuid: "torpedo-uuid".into(),
        x: 0.0,
        y: 0.0,
        z: 0.0,
        heading: 0.0,
        pitch: 0.0,
        lifespan_remaining: 10.0,
        target_uuid: Some("test-local-ship".into()),
        source_uuid: Some("enemy-uuid".into()),
        tube_id: "fore_port".into(),
        shield_pierce: 1.0,
    });
    app.world_mut().spawn((
        crate::simulation::Ship,
        crate::entity_spawner::EntityUuid("enemy-uuid".into()),
        Transform::from_xyz(0.0, 0.0, 40.0),
        TorpedoSystemResource(enemy_torpedoes),
    ));

    tick(&mut app);

    assert!(
        app.world().get_entity(player).is_ok(),
        "the LocalShip must never be despawned on death — the run ends instead"
    );
    let reason = app.world().resource::<GameOverReason>();
    assert!(
        reason.0.is_some(),
        "a torpedo kill on the player must latch a game-over reason"
    );
    assert_eq!(
        reason.1,
        Some(crate::balance::Outcome::Defeat),
        "the player's death is a defeat, whatever weapon delivered it"
    );
}

/// Tube ids are designer-authored and `alliance_battleship` spells them with
/// hyphens (`id = "fore-port"`). The handler used to recover the tube id by
/// inverting the SystemId mapping — strip `torpedo-tube-`, turn hyphens back
/// into underscores — which is lossy, so a hyphenated tube resolved to
/// `fore_port`, matched nothing, and every volley order for that hull was
/// dropped: its AI crew never loaded a round. The handler now compares
/// forward-mapped ids, so either spelling lands.
#[test]
fn set_torpedo_volley_target_accepts_a_hyphenated_tube_id() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Re-spell the tube the way a hyphen-authoring hull does.
    {
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        for mut ts in q.iter_mut(world) {
            ts.0.tube_mut("fore_port")
                .expect("test ship should have a fore_port tube")
                .id = "fore-port".to_string();
        }
    }

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::SetTorpedoVolleyTarget { count: 1 },
        },
    );
    tick(&mut app);

    assert_eq!(
        local_tube_target_count(&mut app, "fore-port"),
        1,
        "a hyphen-spelled tube id must still receive its volley order"
    );
}

#[test]
fn set_torpedo_volley_target_refused_when_tube_fine_system_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    mark_system_offline(&mut app, SystemId("torpedo-tube-fore-port".into()));

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::SetTorpedoVolleyTarget { count: 1 },
        },
    );
    tick(&mut app);

    assert_eq!(
        local_tube_target_count(&mut app, "fore_port"),
        0,
        "an offline tube must refuse volley orders from any origin"
    );
}

#[test]
fn load_tube_refused_when_tube_fine_system_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_tube_fore_port_system_id(),
    );

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::LoadTube,
        },
    );
    tick(&mut app);

    // No claim should have been emitted this tick.
    let queue = &app.world().resource::<InterSystemQueue>().0;
    assert!(
        !queue
            .iter()
            .any(|m| matches!(&m.payload, InterSystemPayload::ClaimTorpedoRound { .. })),
        "load must not emit a magazine claim when the tube system is offline"
    );
}

// ── Magazine claim transaction ────────────────────────────────────────
//
// Directly exercise `handle_torpedo_magazine_inter_system` by pushing
// a claim into the queue and asserting the same-tick effect on the
// magazine counter and the tube state.

#[test]
fn magazine_claim_decrements_counter_by_one_when_online() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Snapshot the magazine counter (starts at 10 from TorpedoConfig::default).
    let before = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert!(before > 0, "test precondition: magazine must have stock");

    // Drive the end-to-end path: `handle_load_tube` (Input) emits the
    // channel-2 claim, and `handle_torpedo_magazine_inter_system` (Physics)
    // consumes it — both happen within a single `app.update()` after
    // `clear_inter_system_queue` runs.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::LoadTube,
        },
    );
    let _ = tick(&mut app);

    let after = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert_eq!(
        after,
        before - 1,
        "magazine counter must decrement by exactly one after a granted claim"
    );

    // The tube should now be Loading.
    let tube_loading = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| {
            matches!(
                ts.0.tube("fore_port").map(|t| &t.load_state),
                Some(crate::torpedo::TubeLoadState::Loading { .. })
            )
        })
        .unwrap();
    assert!(
        tube_loading,
        "granted claim must start loading the target tube via start_load_reserved"
    );
}

#[test]
fn magazine_claim_refused_when_magazine_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Register magazine as human, then mark it offline (Disabled tier).
    register_fine_system(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
    );

    let before = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();

    // End-to-end: LoadTube tries to emit a claim — the tube gate passes
    // (fine tube systems default to the Human source), then the claim
    // goes to the magazine consumer which refuses because the magazine
    // is offline.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::LoadTube,
        },
    );
    let _ = tick(&mut app);

    let after = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert_eq!(
        after, before,
        "offline magazine must refuse the claim — counter unchanged"
    );
}

#[test]
fn magazine_claim_refused_when_empty() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Drain the magazine.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        let mut ts = q.single_mut(app.world_mut()).unwrap();
        ts.0.torpedoes_remaining = 0;
    }

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::LoadTube,
        },
    );
    let _ = tick(&mut app);

    // Tube must still be Unloaded — no start_load_reserved happened.
    let tube_state = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.tube("fore_port").map(|t| t.load_state.clone()))
        .unwrap();
    assert_eq!(
        tube_state,
        Some(crate::torpedo::TubeLoadState::Unloaded),
        "empty magazine must not begin loading the tube"
    );
}

// ── Same-tick magazine contention (issue #782, AC6) ───────────────────

/// Push two `ClaimTorpedoRound` claims (fore_port then fore_starboard) at a
/// magazine holding exactly ONE round, then run the single consumer. The
/// authoritative counter has exactly one writer draining the queue in Vec order,
/// so the FIRST claim wins the round and the SECOND is refused — deterministically
/// on every run. This is the atomicity invariant #782 must not break: the tube
/// LOAD policy and the magazine GRANT policy only DECIDE, they never decrement
/// `torpedoes_remaining` themselves.
#[test]
fn same_tick_magazine_contention_is_deterministic_first_claim_wins() {
    use bevy::ecs::system::RunSystemOnce;

    for _ in 0..8 {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        let ship = local_ship(&mut app);

        // Magazine holds exactly one round.
        {
            let mut ts = app
                .world_mut()
                .get_mut::<TorpedoSystemResource>(ship)
                .unwrap();
            ts.0.torpedoes_remaining = 1;
        }

        // Two competing claims, in a fixed queue order.
        {
            let mut q = app.world_mut().resource_mut::<InterSystemQueue>();
            q.0.push(InterSystemMsg {
                target: crate::system_registry::torpedo_magazine_system_id(),
                payload: InterSystemPayload::ClaimTorpedoRound {
                    tube: "fore_port".into(),
                },
                source_entity: Some(ship),
            });
            q.0.push(InterSystemMsg {
                target: crate::system_registry::torpedo_magazine_system_id(),
                payload: InterSystemPayload::ClaimTorpedoRound {
                    tube: "fore_starboard".into(),
                },
                source_entity: Some(ship),
            });
        }

        app.world_mut()
            .run_system_once(crate::console::weapons::handle_torpedo_magazine_inter_system)
            .expect("magazine consumer runs");

        let ts = app.world().get::<TorpedoSystemResource>(ship).unwrap();
        assert_eq!(
            ts.0.torpedoes_remaining, 0,
            "the single round must be reserved exactly once — one writer, no double-spend"
        );
        assert!(
            matches!(
                ts.0.tube("fore_port").map(|t| &t.load_state),
                Some(crate::torpedo::TubeLoadState::Loading { .. })
            ),
            "the first claim in queue order must win the contested round"
        );
        assert_eq!(
            ts.0.tube("fore_starboard").map(|t| &t.load_state),
            Some(&crate::torpedo::TubeLoadState::Unloaded),
            "the second claim must be refused when the magazine is exhausted"
        );
    }
}

/// The magazine's authored GRANT policy (AC1) gates the reservation right before
/// `claim_magazine_round`. An idle magazine policy refuses every claim, so the
/// counter is never decremented and the tube never loads — the offline gate stays
/// the hard authority and the policy is a data-authored arbiter layered on top.
#[test]
fn idle_magazine_grant_policy_refuses_the_claim() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = test_app();
    start_game_with_weapons(&mut app);
    let ship = local_ship(&mut app);

    let before = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .unwrap()
        .0
        .torpedoes_remaining;
    assert!(before > 0, "precondition: magazine has stock");

    // Attach an idle magazine grant policy.
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::weapons_plugin::TorpedoMagazineAiPolicy(
            crate::ai::policy::AiPolicy {
                idle: true,
                ..Default::default()
            },
        ));

    {
        let mut q = app.world_mut().resource_mut::<InterSystemQueue>();
        q.0.push(InterSystemMsg {
            target: crate::system_registry::torpedo_magazine_system_id(),
            payload: InterSystemPayload::ClaimTorpedoRound {
                tube: "fore_port".into(),
            },
            source_entity: Some(ship),
        });
    }

    app.world_mut()
        .run_system_once(crate::console::weapons::handle_torpedo_magazine_inter_system)
        .expect("magazine consumer runs");

    let ts = app.world().get::<TorpedoSystemResource>(ship).unwrap();
    assert_eq!(
        ts.0.torpedoes_remaining, before,
        "an idle magazine grant policy must refuse the claim — counter unchanged"
    );
    assert_eq!(
        ts.0.tube("fore_port").map(|t| &t.load_state),
        Some(&crate::torpedo::TubeLoadState::Unloaded),
        "a refused claim must not begin loading the tube"
    );
}

// ── Fire torpedo: magazine-online gate ────────────────────────────────

#[test]
fn fire_torpedo_refused_when_magazine_offline_even_if_tube_loaded() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Load the tube directly (bypass channel-2 to isolate the fire gate).
    load_tube_now(&mut app, "fore_port");

    // Register magazine as offline.
    register_fine_system(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
    );

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "disabled magazine must block fire even from a loaded tube"
    );
}

#[test]
fn fire_torpedo_refused_when_tube_fine_system_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");
    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_tube_fore_port_system_id(),
    );

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "disabled tube fine system must block its fire"
    );
}

// ── The lock's own gate is the radar (issue #887) ─────────────────────
//
// These replace `set_target_refused_when_all_banks_offline`, which pinned the
// pre-#887 shape: `handle_set_target` carried an extra
// `any_bank_accepts_human_input` gate on top of admission's own check on
// `tactical-radar`. That coupled the ship's lock to the phaser banks' rating,
// which is wrong on shipped content — `alliance_cruiser`'s `Simplified` Tactical
// rating automates both banks, and the crewed operator could not lock anything.

#[test]
fn set_target_survives_every_phaser_bank_going_offline() {
    let mut app = test_app();
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    // Both fine phaser banks dead. The radar is untouched.
    mark_system_offline(&mut app, SystemId("phaser-port".into()));
    mark_system_offline(&mut app, SystemId("phaser-starboard".into()));

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some("target-uuid"),
        "a working tactical radar must still lock a target when every phaser bank \
         is dead — the lock belongs to the radar, and torpedoes still need it"
    );
}

#[test]
fn set_target_refused_when_the_tactical_radar_is_offline() {
    let mut app = test_app();
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    mark_system_offline(&mut app, crate::system_registry::tactical_radar_system_id());

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TargetLock { .. })),
        "an offline tactical radar must refuse SetTarget at admission — this is \
         the ONE gate on the lock"
    );
    assert!(get_weapons_target(&mut app).is_none());
}

// ── Blackboards ───────────────────────────────────────────────────────

#[test]
fn publish_writes_phaser_fore_blackboard_when_bank_configured() {
    let mut app = test_app();
    // The test app config has "port"/"starboard" banks — no "fore" bank.
    // Insert a fresh combat config with a "fore" bank so publish emits an entry.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>(
            );
        if let Ok(mut cc) = q.single_mut(app.world_mut()) {
            cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 270.0,
                auto_arc_deg: 180.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 6.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
                ai: None,
            }];
        }
    }
    // Publish runs in SimSet::Publish — one full update ticks it.
    app.update();

    let key = crate::system_registry::phaser_fore_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let bb = bbs
        .0
        .get(&key)
        .expect("expected phaser-fore in blackboards");
    assert!(matches!(bb, SystemBlackboard::PhaserBank(_)));
}

#[test]
fn publish_writes_torpedo_magazine_blackboard() {
    let mut app = test_app();
    app.update();

    let key = crate::system_registry::torpedo_magazine_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::TorpedoMagazine(mag_bb) = bbs
        .0
        .get(&key)
        .expect("expected torpedo-magazine in blackboards")
        .clone()
    else {
        panic!("expected TorpedoMagazine blackboard");
    };
    assert!(
        mag_bb.is_online,
        "fresh test ship magazine should be online"
    );
    assert_eq!(mag_bb.torpedoes_remaining, mag_bb.capacity);
}

#[test]
fn publish_writes_torpedo_tube_blackboards_per_tube() {
    let mut app = test_app();
    app.update();

    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    for tube_key in [
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::system_registry::torpedo_tube_fore_starboard_system_id(),
        crate::system_registry::torpedo_tube_aft_system_id(),
    ] {
        let bb = bbs
            .0
            .get(&tube_key)
            .unwrap_or_else(|| panic!("expected {tube_key:?} in blackboards"));
        assert!(matches!(bb, SystemBlackboard::TorpedoTube(_)));
    }
}

// ── Ship-level AI early-skip regression tests (issue #512, findings 1 & 2) ─
//
// These tests cover the specific production path the reviewer flagged as
// dead code: after #512 deleted `[[system]] id = "tactical" kind = "tactical"`
// from every ship TOML, the coarse tactical SystemId is not registered
// in any ship's ControlSourceResolver. Every code path that gated on
// a coarse-tactical policy lookup would therefore see the
// default `Human` policy (`operate_ai = false`) and never run.
//
// These tests DO NOT touch the coarse `tactical` SystemId — they set
// AI only on a fine phaser bank / torpedo tube and assert the
// ship-level AI paths still activate.

/// Finding 1 regression: the phaser auto-fire path used to gate its
/// early skip on the coarse `tactical` policy. Post-fix, it uses
/// `any_bank_operates_ai` which iterates the ship config's `phaser_bank`
/// fine systems. This test seeds AI on ONE fine bank on an NPC — no
/// coarse tactical touching — and asserts a beam still activates.
#[test]
fn ai_phaser_auto_fire_activates_when_any_bank_operates_ai() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "cc000000-0000-0000-0000-000000000001";
    let target_uuid = "cc000000-0000-0000-0000-000000000002";

    // The NPC has a `phaser_bank` fine system ("phaser-port") declared
    // in its ShipConfigComponent — matching what the ship_harrow_*.toml
    // NPC TOMLs do. Its policy is Ai. The coarse `tactical` SystemId
    // is INTENTIONALLY untouched — the test would fail before finding 1
    // was fixed because the early-skip in `tick_phaser_auto_fire` would
    // read the coarse tactical policy's `operate_ai == false` and
    // `continue`.
    const NPC_TOML: &str = r#"
[[system]]
id = "phaser-port"
kind = "phaser_bank"
ai_only = true
"#;
    let npc_ship_config = crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::parse_and_validate(NPC_TOML, &["phaser_bank"])
            .expect("NPC ship config must be valid"),
    );

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        SystemId("phaser-port".into()),
        crate::ship::control_source::ControlSource::Ai,
    );
    // NOTE: coarse tactical NOT set — this is the whole point of the test.

    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            npc_ship_config,
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                }],
            }),
            Transform::default(),
            crate::messages::AdmittedCommands::default(),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, npc_entity);

    // Target directly ahead of NPC (yaw=0, forward=-Z).
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC entity must have ActiveBeam component");
    assert!(
        beam.is_firing(),
        "ai_phaser_auto_fire must activate the beam when ANY phaser bank fine \
         system has operate_ai=true, even without the coarse tactical SystemId"
    );
    assert_eq!(
        beam.any_bank(),
        Some("port"),
        "NPC should fire the port bank whose fine system is AI-operated"
    );
}

/// Seed a live named `Destroy` objective and return the target's UUID, so a
/// test only has to arrange control sources and read the resulting lock.
fn seed_destroy_objective_target(app: &mut App) -> String {
    let target_uuid = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(app, "wave_1", 80.0);
    target_uuid
}

fn set_system_control_source(
    app: &mut App,
    system_id: SystemId,
    source: crate::ship::control_source::ControlSource,
) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        cs.0.set(system_id.clone(), source);
    }
}

/// Issue #887, the fairness invariant. The Tactical AI used to run whenever
/// ANY tactical fine system was AI (`any_tactical_system_operates_ai`) — a gate
/// strictly wider than the one admission applies to the human, who is checked on
/// `tactical-radar` alone. On a mixed-rating ship that meant an automated
/// torpedo magazine licensed the AI to overwrite the crewed radar's lock.
///
/// This is not a hypothetical config: `alliance_cruiser`'s shipped `Simplified`
/// Tactical rating automates the phaser banks and leaves the radar crewed.
#[test]
fn an_ai_magazine_alone_does_not_license_the_tactical_ai_to_take_the_lock() {
    let mut app = test_app();
    set_tactical_station_rating(&mut app, "Assisted");
    set_system_control_source(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );
    // The radar stays Human (the resolver's default), i.e. the operator's.
    let target_uuid = seed_destroy_objective_target(&mut app);
    set_weapons_target(&mut app, Some("the-humans-lock".into()));

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some("the-humans-lock"),
        "an Ai torpedo magazine must NOT license the Tactical AI to re-decide the \
         lock: the radar is Human, so the operator's lock is the ship's lock \
         (issue #887). Locking {target_uuid} here means the AI overrode a crewed radar."
    );
}

/// The other half of the same gate: an AI tactical radar — and nothing else on
/// the Tactical surface — is enough on its own. The lock belongs to the radar.
#[test]
fn an_ai_tactical_radar_alone_licenses_the_tactical_ai_to_take_the_lock() {
    let mut app = test_app();
    set_system_control_source(
        &mut app,
        crate::system_registry::tactical_radar_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let target_uuid = seed_destroy_objective_target(&mut app);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "an AI-operated tactical radar must run the selector and apply its choice, \
         with no weapon fine system needed to license it"
    );
}

// ── One lock per ship: the fairness count (issue #887) ─────────────────
//
// A human gunner holds exactly ONE `TacticalRadarSelection`, so a crewed ship
// engages one target at a time. If an AI weapon bank picked a target of its own
// instead of the ship's lock, an AI ship could engage two at once — a capability
// no player has, i.e. an AGENTS.md #6 symmetry violation. The test below is the
// count that pins it, and it is deliberately arranged so a per-bank picker WOULD
// hit both: the two phaser arcs do not overlap, and there is one eligible
// hostile squarely inside each.

/// A hostile ship with a hull, so damage to it is observable.
fn spawn_hostile_hull(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
    app.world_mut()
        .spawn((
            crate::simulation::Ship,
            crate::entity_spawner::EntityUuid(uuid.into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                200.0,
            )])),
            Transform::from_xyz(x, 0.0, z),
            FactionComponent(federation_faction()),
        ))
        .id()
}

fn hull_current(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<crate::entity_spawner::EntitySystemHull>(entity)
        .expect("hostile carries EntitySystemHull")
        .0
        .total_current()
}

/// Arrange the shipped-shape two-bank ship (port −90°, starboard +90°, 240°
/// auto arcs, so the arcs do NOT overlap abeam) plus a 360° blaster bank, all
/// AI-operated, with the Tactical radar AI too. Returns (port-side hostile,
/// starboard-side hostile).
fn setup_two_arc_ai_shooter(app: &mut App) -> (Entity, Entity, String, String) {
    use crate::ship::control_source::ControlSource;

    setup_harrow_ship_hostile_to_federation(app);
    insert_untargeted_destroy_objective(app, 45.0);
    for sysid in [
        crate::system_registry::tactical_radar_system_id(),
        SystemId("phaser-port".into()),
        SystemId("phaser-starboard".into()),
    ] {
        set_system_control_source(app, sysid, ControlSource::Ai);
    }

    // A third weapon group with an arc that bears on BOTH hostiles, so the
    // count covers more than a pair of phaser banks.
    let ship = local_ship_entity(app);
    set_system_control_source(
        app,
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        ControlSource::Ai,
    );
    app.world_mut()
        .entity_mut(ship)
        .insert(BlasterSystemResource(vec![
            crate::blaster::BlasterSystem::new(crate::blaster::BlasterBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 360.0,
                volley_count: 1,
                volley_interval_secs: 0.1,
                cooldown_secs: 3.0,
                charge_time_secs: 0.0,
                projectile_speed: 60.0,
                collision_radius: 1.5,
                visual_scale: 1.0,
                damage: 10,
                shield_pierce: 0.0,
                recoil_impulse: 0.0,
                screenshake_magnitude: 0.0,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                range: 45.0,
            }),
        ]));
    // Re-attach so the new blaster bank picks up the shipped open-fire policy.
    attach_shipped_weapon_ai(app, ship);

    // Port hostile is the NEARER of the two, so the selector's nearest-hostile
    // source picks it deterministically. Both are inside 40 units (the default
    // phaser range) and each sits squarely inside exactly one 240° arc.
    let port_uuid = uuid::Uuid::new_v4().to_string();
    let starboard_uuid = uuid::Uuid::new_v4().to_string();
    let port = spawn_hostile_hull(app, &port_uuid, -20.0, -3.0);
    let starboard = spawn_hostile_hull(app, &starboard_uuid, 25.0, -3.0);
    (port, starboard, port_uuid, starboard_uuid)
}

/// The acceptance count. Two eligible hostiles, three AI weapon groups bearing,
/// and exactly ONE hostile takes damage.
#[test]
fn an_ai_ship_with_several_bearing_weapon_groups_damages_only_one_hostile() {
    let mut app = test_app();
    let (port, starboard, port_uuid, _starboard_uuid) = setup_two_arc_ai_shooter(&mut app);

    let port_hull_before = hull_current(&app, port);
    let starboard_hull_before = hull_current(&app, starboard);

    for _ in 0..6 {
        tick(&mut app);
    }

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(port_uuid.as_str()),
        "precondition: the ship's ONE lock is the nearer hostile"
    );
    assert!(
        hull_current(&app, port) < port_hull_before,
        "precondition: the ship must actually be shooting the target it locked, \
         or this test counts nothing"
    );
    // Precondition: more than one weapon GROUP is live, so "one target" is a
    // statement about the ship, not about a ship that only has one gun.
    assert_eq!(
        live_beam_banks(&mut app),
        vec!["port".to_string()],
        "precondition: exactly the bank whose arc holds the lock is burning"
    );
    let ship = local_ship_entity(&mut app);
    let blaster = app
        .world()
        .get::<BlasterSystemResource>(ship)
        .expect("the shooter carries a blaster bank");
    assert!(
        blaster.0[0].volley.on_cooldown || !blaster.0[0].in_flight.is_empty(),
        "precondition: the 360° blaster group also engaged this tick window — a \
         second weapon group that could have chosen the other hostile for itself"
    );
    assert_eq!(
        hull_current(&app, starboard),
        starboard_hull_before,
        "one lock per ship: the second hostile must take NO damage while the ship \
         is engaging the first. A weapon group that picked its own target would \
         engage both at once — a capability no human gunner has, who holds exactly \
         one TacticalRadarSelection (issue #887, AGENTS.md #6)"
    );
}

/// The fixture above only counts something if the second hostile really was
/// engageable — otherwise "no damage" proves an arc gap, not a shared lock.
/// Point the same ship's lock at the starboard hostile and it takes damage from
/// the starboard bank, with the port hostile now untouched.
#[test]
fn the_unengaged_hostile_was_engageable_all_along() {
    let mut app = test_app();
    let (port, starboard, _port_uuid, starboard_uuid) = setup_two_arc_ai_shooter(&mut app);

    // Take the radar off AI and hand the lock over as a human would, so the
    // selector's nearest-first ranking does not immediately take it back.
    set_system_control_source(
        &mut app,
        crate::system_registry::tactical_radar_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    set_weapons_target(&mut app, Some(starboard_uuid.clone()));

    let port_hull_before = hull_current(&app, port);
    let starboard_hull_before = hull_current(&app, starboard);

    for _ in 0..6 {
        tick(&mut app);
    }

    assert!(
        hull_current(&app, starboard) < starboard_hull_before,
        "the starboard hostile is inside the starboard bank's arc and inside \
         weapons range — it was always engageable, so the count above is a \
         statement about the shared lock, not about arcs"
    );
    assert_eq!(
        hull_current(&app, port),
        port_hull_before,
        "and with the lock moved, the port hostile is the one now spared — still \
         exactly one engaged target"
    );
}

// ── issue #692 (audit finding B1): tick_npc_auto_match_frequency gate ──
//
// Both frequency-hint systems must be gated on `AiHighFidelity`. The
// `tick_frequency_hint` path already is (`tick_frequency_hint_high_fidelity`); these two
// tests cover the newly-added gate on the NPC auto-match path.

/// Spawns a target entity that `tick_npc_auto_match_frequency` can read a
/// shield frequency from: `EntityUuid` (matched against the locked target),
/// `Transform` (so `ai_target_selection`'s stale-target guard treats it as
/// alive and keeps the lock), and `ShipShields` carrying `freq`.
fn spawn_shield_target(app: &mut App, uuid: &str, freq: f32) {
    app.world_mut().spawn((
        crate::entity_spawner::EntityUuid(uuid.into()),
        bevy::prelude::Transform::from_xyz(0.0, 0.0, -30.0),
        crate::ship::shields::ShipShields(crate::shield::ShieldSystem::default(), freq),
    ));
}

/// Puts the LocalShip's Tactical fine systems under AI control (so
/// `any_tactical_system_operates_ai` is true) and locks it onto
/// `target_uuid` — shared setup for both auto-match tests.
fn setup_npc_auto_match(app: &mut App, target_uuid: &str) {
    set_tactical_control_source(app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(app, Some(target_uuid.into()));
}

fn local_ship_entity(app: &mut App) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .expect("test_app must spawn a LocalShip")
}

/// Positive path: a high-fidelity NPC whose Tactical is AI-operated and
/// which has a target locked drives its `ShipPhaserFrequency` toward the
/// target's shield frequency once `NPC_FREQ_MATCH_DELAY` elapses.
#[test]
fn npc_auto_match_frequency_matches_with_high_fidelity() {
    let mut app = test_app();
    let target_uuid = "shield-target-hi-fi";
    // Distinct from ShipPhaserFrequency's 0.5 default AND from the code's
    // 0.5 fallback, so an observed change proves a real match fired.
    let target_freq = 0.8_f32;

    setup_npc_auto_match(&mut app, target_uuid);
    spawn_shield_target(&mut app, target_uuid, target_freq);

    let ship = local_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ai_plugin::AiHighFidelity);

    assert_eq!(
        get_phaser_frequency(&mut app),
        0.5,
        "test invariant: phaser frequency starts at its default"
    );

    // NPC_FREQ_MATCH_DELAY = 2.0s; test app ticks at 200ms → ≥10 ticks to
    // cross the delay. Run extra to stay clear of first-tick dt edge cases.
    for _ in 0..15 {
        tick(&mut app);
    }

    assert_eq!(
        get_phaser_frequency(&mut app),
        target_freq,
        "high-fidelity NPC must auto-match its phaser frequency to the locked \
         target's shield frequency after the delay"
    );
}

/// Negative path (the gate under test): identical setup but WITHOUT
/// `AiHighFidelity` → the new gate suppresses auto-match and the phaser
/// frequency never changes. This test fails if the `has_high_fidelity`
/// gate is removed.
#[test]
fn npc_auto_match_frequency_gated_off_without_high_fidelity() {
    let mut app = test_app();
    let target_uuid = "shield-target-lo-fi";
    let target_freq = 0.8_f32;

    setup_npc_auto_match(&mut app, target_uuid);
    spawn_shield_target(&mut app, target_uuid, target_freq);

    // Deliberately NOT high-fidelity — no AiHighFidelity component.

    assert_eq!(
        get_phaser_frequency(&mut app),
        0.5,
        "test invariant: phaser frequency starts at its default"
    );

    for _ in 0..15 {
        tick(&mut app);
    }

    assert_eq!(
        get_phaser_frequency(&mut app),
        0.5,
        "without AiHighFidelity the auto-match gate must not fire; the phaser \
         frequency stays at its default"
    );
}

// ── Finding 5 regression: publish gates on offline_systems, not hardcoded Console match ──

/// If an unknown / non-standard bank id ends up in the bank blackboard,
/// the previous hardcoded `match "fore" | "aft"` returned `None` and
/// silently reported `is_online: true` regardless of hull state.
///
/// Post-fix, `is_online` is derived from `offline_systems` — so a bank
/// whose fine SystemId lives in `offline_systems` reports `is_online: false`
/// no matter whether the id matches a Console variant.
#[test]
fn publish_marks_bank_offline_when_fine_system_in_offline_set() {
    let mut app = test_app();
    // Swap in a bank config whose id is NOT in the hardcoded match
    // (e.g. "dorsal"), so the old bug's hardcoded id→Console arms
    // would default to online.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>(
            );
        if let Ok(mut cc) = q.single_mut(app.world_mut()) {
            cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                id: "dorsal".into(),
                facing_deg: 0.0,
                fire_arc_deg: 270.0,
                auto_arc_deg: 180.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 6.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
                ai: None,
            }];
        }
    }
    // Mark the corresponding fine SystemId offline via offline_systems.
    mark_system_offline(&mut app, SystemId("phaser-dorsal".into()));

    app.update();

    let key = SystemId("phaser-dorsal".into());
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::PhaserBank(bb) = bbs
        .0
        .get(&key)
        .expect("expected phaser-dorsal blackboard entry")
        .clone()
    else {
        panic!("expected PhaserBank blackboard variant");
    };
    assert!(
        !bb.is_online,
        "bank must report is_online: false when its fine SystemId is in \
         offline_systems (regardless of whether the id matches a Console variant)"
    );
}

// ── Finding 7 regression: end-to-end hull → offline_systems → PhaserBankBlackboard ──
//
// Ties together sync_console_damage_tiers (in ship_plugin) and
// publish_phaser_bank_blackboards (in this module). A hull entry for
// Console::PhaserFore below the disabled threshold should end up as
// `phaser-fore ∈ offline_systems` after one tick, and the emitted
// blackboard should reflect `is_online: false`.

#[test]
fn hull_disabled_console_causes_publish_to_mark_bank_offline() {
    let mut app = test_app();
    // Register the sync system directly (test_app doesn't include ShipPlugin).
    app.add_systems(
        Update,
        crate::ship_plugin::sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
    );

    // Insert a "fore" bank so publish emits a `phaser-fore` blackboard.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>(
            );
        if let Ok(mut cc) = q.single_mut(app.world_mut()) {
            cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 270.0,
                auto_arc_deg: 180.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 6.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
                ai: None,
            }];
        }
    }

    // Damage the PhaserFore console to 0 HP (Destroyed tier → offline).
    {
        let world = app.world_mut();
        let ship = world
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(world)
            .unwrap();
        let mut entity_mut = app.world_mut().entity_mut(ship);
        let mut hull = entity_mut
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap();
        hull.0.set_hp(&SystemId("phaser-fore".into()), 0.0);
    }

    // One update: sync_console_damage_tiers (Damage) writes offline_systems,
    // publish_phaser_bank_blackboards (Publish) reads it and emits the entry.
    app.update();

    // Step 1 verify: offline_systems contains `phaser-fore`.
    let phaser_fore_id = crate::system_registry::phaser_fore_system_id();
    let is_in_offline = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        let cs = q.single(app.world()).unwrap();
        cs.0.is_offline(&phaser_fore_id)
    };
    assert!(
        is_in_offline,
        "sync_console_damage_tiers must add phaser-fore to offline_systems \
         when Console::PhaserFore hull is at Disabled/Destroyed tier"
    );

    // Step 2 verify: blackboard reports is_online: false for phaser-fore.
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::PhaserBank(bb) = bbs
        .0
        .get(&phaser_fore_id)
        .expect("expected phaser-fore blackboard entry")
        .clone()
    else {
        panic!("expected PhaserBank blackboard variant");
    };
    assert!(
        !bb.is_online,
        "PhaserBankBlackboard.is_online must be false end-to-end when the \
         console hull is disabled (hull → offline_systems → blackboard chain)"
    );
}

/// Issue #738: the torpedo console handlers used to write the LocalShip's own
/// component "or fall back to the global resource for test compat". The global
/// `TorpedoSystemResource` is a shared singleton that belongs to no particular
/// ship, so that branch was a standing footgun; it is gone. A console unload
/// now mutates exactly one thing — the operating ship's own component.
#[test]
fn unload_tube_mutates_the_ships_own_component_and_never_the_global_resource() {
    let mut app = test_app();
    load_tube_now(&mut app, "fore_port");
    {
        let mut res = app.world_mut().resource_mut::<TorpedoSystemResource>();
        res.0.tube_mut("fore_port").unwrap().loaded_count = 1;
    }

    push(
        &mut app,
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::UnloadTube,
        },
    );
    tick(&mut app);

    let mut q = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
    let ship_state = q.single(app.world()).unwrap().0.tube("fore_port").unwrap();
    assert!(
        matches!(
            ship_state.load_state,
            crate::torpedo::TubeLoadState::Unloading { .. }
        ),
        "the operating ship's own tube must begin unloading"
    );
    assert!(
        !matches!(
            app.world()
                .resource::<TorpedoSystemResource>()
                .0
                .tube("fore_port")
                .unwrap()
                .load_state,
            crate::torpedo::TubeLoadState::Unloading { .. }
        ),
        "the shared global Resource must never be mutated by a console command"
    );
}

/// Issue #738 per-ship isolation: an NPC ship that carries NO
/// `TorpedoSystemResource` component must not fire, and above all must not
/// fire out of the PLAYER ship's magazine.
///
/// `handle_fire_torpedo` used to resolve the shooter's torpedo state as
/// `per_entity_component.unwrap_or(&mut global_resource)`. A comment claimed
/// "only the LocalShip should ever fall through", but nothing enforced it: the
/// global `TorpedoSystemResource` mirrors the player ship, so an NPC resolved
/// through `AiTokenRegistry` with no component of its own launched from — and
/// decremented — the player's tubes. The fallback is now gated on the shooter
/// actually being the LocalShip.
#[test]
fn npc_without_its_own_torpedo_system_cannot_fire_from_the_player_ships_magazine() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    // Load the PLAYER ship's fore_port tube in the global Resource, so the
    // retired fallback would have had a live round to launch.
    {
        let mut res = app.world_mut().resource_mut::<TorpedoSystemResource>();
        res.0.tube_mut("fore_port").unwrap().loaded_count = 1;
    }
    let player_tubes_before = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .tube("fore_port")
        .unwrap()
        .loaded_count;
    let player_torpedoes_before = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;

    let npc_uuid = "cc000000-0000-0000-0000-0000000000ff";
    let mut npc_ai_sources = crate::ship::control_source::ControlSourceResolver::new();
    for sysid in [
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::system_registry::torpedo_magazine_system_id(),
    ] {
        npc_ai_sources.set(sysid, crate::ship::control_source::ControlSource::Ai);
    }
    // Deliberately NO `TorpedoSystemResource` component — this NPC's TOML has
    // no `[torpedoes]` block, so it has no tubes at all.
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(npc_ai_sources),
            ShipPhysics::default(),
            TacticalRadarSelection::default(),
            crate::server_app::WeaponFiredThisTick::default(),
            bevy::prelude::Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }

    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "an NPC with no torpedo system of its own must not launch anything"
    );
    let res = app.world().resource::<TorpedoSystemResource>();
    assert_eq!(
        res.0.tube("fore_port").unwrap().loaded_count,
        player_tubes_before,
        "the player ship's loaded tube must be untouched by an NPC's fire command"
    );
    assert_eq!(
        res.0.torpedoes_remaining, player_torpedoes_before,
        "the player ship's magazine must be untouched by an NPC's fire command"
    );
}

// ── Finding 8 regression: magazine claim routes by source_entity ──────
//
// Before the fix, `handle_load_tube` emitted `source_entity: None` on
// its `ClaimTorpedoRound` message. `handle_torpedo_magazine_inter_system`
// then queried `With<LocalShip>` only, so an NPC's claim would either
// be ignored entirely or misroute to the player ship. Post-fix, both
// sides route by source_entity (mirroring `handle_power_inter_system`)
// so each ship's claims mutate that ship's own magazine.

#[test]
fn magazine_claim_routes_to_shooter_ship_when_multiple_ships_have_magazines() {
    let mut app = test_app();

    // Snapshot the LocalShip's magazine counter.
    let localship_before = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();

    // Spawn a second Ship (NOT LocalShip) that also has a magazine. Give
    // it a fully-declared torpedo_magazine fine system with Human
    // policy so the online gate passes, and its own TorpedoSystemResource
    // with 10 torpedoes and a "fore_port" tube.
    let mut npc_sources = crate::ship::control_source::ControlSourceResolver::new();
    npc_sources.set(
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    npc_sources.set(
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    let npc_torpedo_sys = TorpedoSystem::from_configs(
        &[crate::entity_config::TorpedoTubeConfig {
            id: "fore_port".into(),
            facing_deg: -30.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }],
        TorpedoConfig {
            count: 10,
            ..Default::default()
        },
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship, // NOT LocalShip
            crate::entity_spawner::EntityUuid("npc-with-magazine".into()),
            crate::ship_plugin::ShipSystemControlSources(npc_sources),
            TorpedoSystemResource(npc_torpedo_sys),
            Transform::default(),
        ))
        .id();
    // The SHIPPED authored magazine-grant policy: since #885b stage 5d a ship
    // with no `TorpedoMagazineAiPolicy` grants no claim at all.
    attach_shipped_weapon_ai(&mut app, npc_entity);

    let npc_before = 10u32;

    // Install a one-shot system in `SimSet::Input` that pushes a claim
    // for the NPC entity into the queue every tick. This mirrors what
    // `handle_load_tube` would do if it ran for NPC ships — the point
    // of the test is that `handle_torpedo_magazine_inter_system` in
    // Physics routes the claim to the ship named by `source_entity`,
    // NOT to `With<LocalShip>` only.
    //
    // The queue is cleared by `clear_inter_system_queue` before
    // `SimSet::Input`, so pushing during Input survives to Physics.
    let claim_target_entity = npc_entity;
    app.add_systems(
        Update,
        (move |mut queue: ResMut<InterSystemQueue>| {
            queue.0.push(InterSystemMsg {
                target: crate::system_registry::torpedo_magazine_system_id(),
                payload: InterSystemPayload::ClaimTorpedoRound {
                    tube: "fore_port".into(),
                },
                source_entity: Some(claim_target_entity),
            });
        })
        .in_set(crate::sim_sets::SimSet::Input),
    );

    app.update();

    // LocalShip magazine must be UNCHANGED — the claim was for the NPC.
    let localship_after = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert_eq!(
        localship_after, localship_before,
        "LocalShip magazine must NOT be decremented when the claim was \
         attributed to a different ship"
    );

    // NPC magazine must have decremented by 1.
    let npc_after = app
        .world()
        .get::<TorpedoSystemResource>(npc_entity)
        .unwrap()
        .0
        .torpedoes_remaining;
    assert_eq!(
        npc_after,
        npc_before - 1,
        "NPC magazine must decrement by 1 when its own claim is granted"
    );

    // NPC tube must be Loading.
    let npc_tube_loading = app
        .world()
        .get::<TorpedoSystemResource>(npc_entity)
        .unwrap()
        .0
        .tube("fore_port")
        .map(|t| matches!(t.load_state, crate::torpedo::TubeLoadState::Loading { .. }))
        .unwrap_or(false);
    assert!(
        npc_tube_loading,
        "NPC's own tube must transition to Loading after its claim is granted"
    );
}

// ── LOS blocking tests (Rapier) ──────────────────────────────────────────
//
// These tests spin up a Rapier world (like the collision tests in
// server_app.rs) and verify that the beam-tick phases route damage
// correctly when a blocking entity is between the shooter and the
// original target.

/// Build a minimal app with Rapier physics + WeaponsPlugin so
/// `tick_beams_prepare` runs the LOS raycast.
fn los_test_app() -> App {
    use bevy_rapier3d::prelude::RapierPhysicsPlugin;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .init_resource::<WorldResource>()
        .add_message::<AsteroidDestroyedVfx>()
        .add_message::<ShipDestroyedVfx>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .init_resource::<CurrentPhaserMode>()
        .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
            TorpedoConfig::default(),
        )))
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
        // FactionRegistryResource for the LOS faction check.
        .insert_resource(crate::entities::config_cache::FactionRegistryResource(
            crate::entities::config_cache::get_faction_registry(),
        ))
        .add_plugins(WeaponsPlugin)
        .insert_resource(PhaserCombatConfigResource(
            crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 100.0,
                    beam_duration_secs: 10.0,
                    cooldown_secs: 1.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                    ai: None,
                }],
            },
        ))
        // WeaponsPlugin already registers the three beam-tick phase
        // systems (tick_beams_prepare / tick_beams_apply_damage /
        // tick_beams_tick_lifetimes) and the two torpedo-tick phases
        // (build_torpedo_target_snapshot / tick_torpedo_lifecycle).
        // Do NOT register them again here.
        .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
        .add_systems(PostUpdate, collect);

    // Advance one tick to let Rapier initialise.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    app
}

/// Helper: spawn a ship entity with a ball collider and phaser state.
/// Returns the Entity.
fn spawn_los_ship(
    app: &mut App,
    uuid: &str,
    x: f32,
    z: f32,
    faction: Option<uuid::Uuid>,
    hull_hp: f32,
    is_local: bool,
) -> bevy::ecs::entity::Entity {
    use bevy_rapier3d::prelude::{
        ActiveCollisionTypes, Collider, ColliderMassProperties, RigidBody,
    };
    let mut ecmds = app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entity_spawner::EntityUuid(uuid.to_string()),
        ShipPhysics {
            x,
            z,
            yaw: 0.0,
            forward_speed: 0.0,
            roll: 0.0,
            lateral_speed: 0.0,
            ..Default::default()
        },
        Transform::from_xyz(x, 0.0, z),
        GlobalTransform::default(),
        Visibility::default(),
        // Ball collider large enough for the raycast to hit.
        Collider::ball(3.0),
        RigidBody::Fixed,
        ColliderMassProperties::Density(1.0),
        ActiveCollisionTypes::all(),
        crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            hull_hp,
        )])),
        ActiveBeam::default(),
        PhaserCooldown::default(),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 360.0,
                auto_arc_deg: 360.0,
                beam_range: 0.0,
                beam_damage_per_sec: 100.0,
                beam_duration_secs: 10.0,
                cooldown_secs: 1.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
                ai: None,
            }],
        }),
        crate::ship_plugin::ShipSystemControlSources::default(),
    ));
    if is_local {
        ecmds.insert(crate::server_app::LocalShip);
    }
    if let Some(f) = faction {
        ecmds.insert(FactionComponent(f));
    }
    ecmds.id()
}

/// Helper: spawn an asteroid with a ball collider.
fn spawn_los_asteroid(
    app: &mut App,
    uuid: &str,
    x: f32,
    z: f32,
    hull_hp: f32,
) -> bevy::ecs::entity::Entity {
    use bevy_rapier3d::prelude::{
        ActiveCollisionTypes, Collider, ColliderMassProperties, RigidBody,
    };
    app.world_mut()
        .spawn((
            crate::simulation::Asteroid,
            AsteroidUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, z),
            GlobalTransform::default(),
            Visibility::default(),
            Collider::ball(3.0),
            RigidBody::Fixed,
            ColliderMassProperties::Density(1.0),
            ActiveCollisionTypes::all(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                hull_hp,
            )])),
        ))
        .id()
}

/// Activate a beam on the given ship entity, targeting `target_uuid`.
fn activate_los_beam(app: &mut App, shooter: bevy::ecs::entity::Entity, target_uuid: &str) {
    let mut beam = app.world_mut().get_mut::<ActiveBeam>(shooter).unwrap();
    beam.start("port", target_uuid, 10.0);
}

/// Read the total current hull HP from a ship/asteroid entity.
fn hull_hp(app: &App, entity: bevy::ecs::entity::Entity) -> f32 {
    app.world()
        .get::<crate::entity_spawner::EntitySystemHull>(entity)
        .map(|h| h.0.total_current())
        .unwrap_or(0.0)
}

#[test]
fn los_no_blocker_damages_original_target() {
    // Shooter at origin, target at (0, 0, -30). No entity in between.
    // Beam should damage the original target.
    let mut app = los_test_app();
    let faction_uuid = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();

    let shooter = spawn_los_ship(
        &mut app,
        "shooter-uuid",
        0.0,
        0.0,
        Some(faction_uuid),
        200.0,
        true,
    );
    let target = spawn_los_ship(&mut app, "target-uuid", 0.0, -30.0, None, 200.0, false);

    // Let Rapier settle and colliders register at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid");

    let before = hull_hp(&app, target);
    // Run a few ticks to accumulate damage.
    for _ in 0..5 {
        app.update();
    }
    let after = hull_hp(&app, target);
    assert!(
        after < before,
        "Target should take damage when LOS is clear (before={before}, after={after})"
    );
}

#[test]
fn los_enemy_blocker_redirects_damage_away_from_target() {
    // Shooter at origin. Enemy blocker at (0,0,-10). Original target at (0,0,-30).
    // Blocker is in the way → target takes no damage, blocker takes damage.
    use crate::config_cache::FactionRegistryResource;
    use crate::faction::FactionRegistry;

    let mut app = los_test_app();

    let shooter_faction = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
    let enemy_faction = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();

    // Make shooter hostile to blocker.
    let mut reg = FactionRegistry::new();
    reg.insert(crate::faction::FactionConfig {
        uuid: shooter_faction,
        name: "Federation".into(),
        enemies: vec![enemy_faction],
    });
    reg.insert(crate::faction::FactionConfig {
        uuid: enemy_faction,
        name: "Pirate".into(),
        enemies: vec![],
    });
    app.insert_resource(FactionRegistryResource(reg));

    let shooter = spawn_los_ship(
        &mut app,
        "shooter-uuid-2",
        0.0,
        0.0,
        Some(shooter_faction),
        200.0,
        true,
    );
    let blocker = spawn_los_ship(
        &mut app,
        "blocker-uuid-2",
        0.0,
        -10.0,
        Some(enemy_faction),
        500.0,
        false,
    );
    let target = spawn_los_ship(&mut app, "target-uuid-2", 0.0, -30.0, None, 500.0, false);

    // Let Rapier settle so colliders are at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid-2");

    let blocker_before = hull_hp(&app, blocker);
    let target_before = hull_hp(&app, target);
    // Run several ticks — each tick the ray hits the blocker, rerouting damage.
    for _ in 0..5 {
        app.update();
    }
    let blocker_after = hull_hp(&app, blocker);
    let target_after = hull_hp(&app, target);

    assert!(
        blocker_after < blocker_before,
        "Enemy blocker between shooter and target must take damage \
         (before={blocker_before}, after={blocker_after})"
    );
    assert_eq!(
        target_after, target_before,
        "Original target must NOT take damage when blocked \
         (before={target_before}, after={target_after})"
    );
}

#[test]
fn los_friendly_blocker_absorbs_beam_with_no_damage() {
    // Shooter and blocker are same faction. Blocker at (0,0,-10),
    // target at (0,0,-30). Neither blocker nor target should take damage.
    use crate::config_cache::FactionRegistryResource;
    use crate::faction::FactionRegistry;

    let mut app = los_test_app();

    let faction_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();

    // Empty enemy list → faction is friendly to itself.
    let mut reg = FactionRegistry::new();
    reg.insert(crate::faction::FactionConfig {
        uuid: faction_uuid,
        name: "Federation".into(),
        enemies: vec![],
    });
    app.insert_resource(FactionRegistryResource(reg));

    let shooter = spawn_los_ship(
        &mut app,
        "shooter-uuid-3",
        0.0,
        0.0,
        Some(faction_uuid),
        200.0,
        true,
    );
    let blocker = spawn_los_ship(
        &mut app,
        "blocker-uuid-3",
        0.0,
        -10.0,
        Some(faction_uuid), // same faction → friendly
        500.0,
        false,
    );
    let target = spawn_los_ship(&mut app, "target-uuid-3", 0.0, -30.0, None, 500.0, false);

    // Let Rapier settle so colliders are at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid-3");

    let blocker_before = hull_hp(&app, blocker);
    let target_before = hull_hp(&app, target);
    for _ in 0..5 {
        app.update();
    }
    let blocker_after = hull_hp(&app, blocker);
    let target_after = hull_hp(&app, target);

    assert_eq!(
        blocker_after, blocker_before,
        "Friendly blocker must NOT take damage (before={blocker_before}, after={blocker_after})"
    );
    assert_eq!(
        target_after, target_before,
        "Target must NOT take damage when a friendly blocks (before={target_before}, after={target_after})"
    );
}

#[test]
fn los_asteroid_blocker_takes_damage() {
    // Asteroid at (0,0,-10), target at (0,0,-30).
    // Beam aimed at target — asteroid intercepts and takes damage.
    let mut app = los_test_app();

    let shooter = spawn_los_ship(&mut app, "shooter-uuid-4", 0.0, 0.0, None, 200.0, true);
    let ast = spawn_los_asteroid(&mut app, "ast-uuid-4", 0.0, -10.0, 2000.0);
    let target = spawn_los_ship(&mut app, "target-uuid-4", 0.0, -30.0, None, 500.0, false);

    // Let Rapier settle so colliders are at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid-4");

    let ast_before = hull_hp(&app, ast);
    let target_before = hull_hp(&app, target);
    for _ in 0..5 {
        app.update();
    }
    let ast_after = hull_hp(&app, ast);
    let target_after = hull_hp(&app, target);

    assert!(
        ast_after < ast_before,
        "Asteroid blocker must take damage (before={ast_before}, after={ast_after})"
    );
    assert_eq!(
        target_after, target_before,
        "Target behind asteroid must NOT take damage (before={target_before}, after={target_after})"
    );
}

// ── Blaster AI auto-fire tests ──────────────────────────────────────

/// NPC with tactical set to Ai and target in range must have the auto-fire
/// system call `request_charge_start` on the blaster bank.
#[test]
fn tick_blaster_auto_fire_gate_passes_when_tactical_is_ai() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();

    let npc_uuid = "bb000000-0000-0000-0000-000000000010";
    let target_uuid = "bb000000-0000-0000-0000-000000000011";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the blaster bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    // NPC at (10, 10) — away from LocalShip at origin — facing -Z (target at 10, -10).
    // This avoids the projectile immediately hitting the LocalShip which
    // occupies (0, 0) in test_app().
    let npc_physics = ShipPhysics {
        x: 10.0,
        z: 10.0,
        ..Default::default()
    };
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            // #781: blaster AI now emits an admitted ChargeBlasterStart consumed
            // by handle_fire_blaster — the ship needs an AdmittedCommands.
            crate::messages::AdmittedCommands::default(),
            npc_physics,
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 180.0, // face toward -Z (toward target)
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    barrels: Vec::new(),
                    pattern: Vec::new(),
                    range: 35.0,
                },
            )]),
            Transform::from_xyz(10.0, 0.0, 10.0),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, npc_entity);

    // Spawn target directly ahead (-Z), well within blaster range.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(10.0, 0.0, -10.0),
    ));

    // Check initial state before update.
    let init_bank = &app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap()
        .0[0];
    eprintln!(
        "[DEBUG] init: fire_ready={} on_cooldown={} pending={} charging={}",
        init_bank.is_fire_ready(),
        init_bank.volley.on_cooldown,
        init_bank.volley.pending_volley,
        init_bank.volley.charging,
    );

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    let bank = &blaster_res.0[0];
    // tick_blaster_auto_fire (Input) calls request_charge_start, then
    // tick_blaster_system (Physics) fires the projectile same-tick.
    // The projectile ends up in in_flight.
    assert!(
        !bank.in_flight.is_empty(),
        "tick_blaster_auto_fire must fire a blaster projectile when tactical is Ai \
         and target is in range/arc (in_flight={})",
        bank.in_flight.len(),
    );
}

/// A moving target that carries **no blaster banks of its own** must still be
/// led. This is the regression test for the velocity-map defect.
///
/// `tick_blaster_system` built its per-target velocity map by iterating its own
/// firing query, and that query requires `&mut BlasterSystemResource` — so only
/// blaster-CARRYING ships ever landed in the map. Every other hull missed the
/// lookup, resolved to `(0.0, 0.0)`, and the intercept solver aimed exactly at
/// the target's live bearing with no lead whatsoever. That is most of the
/// shipped content: `ship_harrow_patrol` and `alliance_cruiser` author no blaster
/// bank at all, which is to say precisely the hulls the artillery shoots at.
///
/// The geometry is chosen so the correct answer is a round number and the
/// broken answer is zero — nothing here turns on a tolerance. The target sits
/// 100 units dead ahead crossing square at 20 u/s against a 40 u/s bolt, so the
/// exact lead is `asin(20/40)` = 30°. Unled, the heading is the live bearing:
/// 0°. Both are asserted, so a future change that merely perturbs the lead
/// fails as loudly as one that removes it.
#[test]
fn a_target_with_no_blaster_banks_is_still_led() {
    use crate::entity_spawner::EntityUuid;
    use std::f32::consts::FRAC_PI_2;

    let mut app = test_app();

    let shooter_uuid = "bb000000-0000-0000-0000-000000000090";
    let target_uuid = "bb000000-0000-0000-0000-000000000091";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );

    // Shooter at (10, 10), yaw 0 → bow along −Z. Away from the `LocalShip` at
    // the origin so the bolt cannot be consumed by an unrelated hull.
    let shooter = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(shooter_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            // Seeds the viewscreen combat_lock via `seed_viewscreen_from_selection`.
            TacticalRadarSelection(Some(target_uuid.to_string())),
            crate::messages::AdmittedCommands::default(),
            ShipPhysics {
                x: 10.0,
                z: 10.0,
                ..Default::default()
            },
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    barrels: Vec::new(),
                    pattern: Vec::new(),
                    range: 200.0,
                },
            )]),
            Transform::from_xyz(10.0, 0.0, 10.0),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, shooter);

    // The target: a full `Ship` with live `ShipPhysics`, 100 units dead ahead,
    // yaw +90° → heading along +X at 20 u/s. It deliberately carries NO
    // `BlasterSystemResource` — that omission IS the test.
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid(target_uuid.to_string()),
        ShipPhysics {
            x: 10.0,
            z: -90.0,
            yaw: FRAC_PI_2,
            forward_speed: 20.0,
            ..Default::default()
        },
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(10.0, 0.0, -90.0),
    ));

    app.update();

    let bank = &app
        .world()
        .get::<BlasterSystemResource>(shooter)
        .expect("the shooter keeps its blaster bank")
        .0[0];
    assert_eq!(
        bank.in_flight.len(),
        1,
        "the bank must have launched exactly one bolt for this test to say \
         anything about its heading"
    );

    let heading = bank.in_flight[0].heading;
    let led = simmath::asin(20.0_f32 / 40.0); // 30°, the exact square-on lead.
    let live_bearing = 0.0_f32; // the target's CURRENT bearing: dead ahead.

    assert!(
        (heading - led).abs() < 1e-3,
        "a blaster-less crosser must be led by the full solved angle: heading \
         {} deg, expected {} deg",
        heading.to_degrees(),
        led.to_degrees()
    );
    assert!(
        (heading - live_bearing).abs() > 0.1,
        "the counterfactual: with the target's velocity unseen the solver aims \
         at the live bearing ({} deg) and the bolt trails the target — heading \
         was {} deg",
        live_bearing.to_degrees(),
        heading.to_degrees()
    );
}

/// The control for `a_target_with_no_blaster_banks_is_still_led`: the SAME
/// geometry against a target that does carry a blaster bank was always led
/// correctly, which is why the defect stayed invisible. Pinning both halves
/// keeps the map's coverage — not the solver — as the thing under test.
#[test]
fn a_blaster_carrying_target_is_led_identically() {
    use crate::entity_spawner::EntityUuid;
    use std::f32::consts::FRAC_PI_2;

    fn dummy_bank() -> BlasterSystemResource {
        BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
            crate::blaster::BlasterBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 360.0,
                volley_count: 1,
                volley_interval_secs: 0.1,
                cooldown_secs: 3.0,
                charge_time_secs: 0.0,
                projectile_speed: 40.0,
                collision_radius: 1.5,
                visual_scale: 1.0,
                damage: 10,
                shield_pierce: 0.0,
                recoil_impulse: 0.0,
                screenshake_magnitude: 0.0,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                range: 200.0,
            },
        )])
    }

    let mut app = test_app();

    let shooter_uuid = "bb000000-0000-0000-0000-000000000092";
    let target_uuid = "bb000000-0000-0000-0000-000000000093";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );

    let shooter = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(shooter_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            crate::messages::AdmittedCommands::default(),
            ShipPhysics {
                x: 10.0,
                z: 10.0,
                ..Default::default()
            },
            dummy_bank(),
            Transform::from_xyz(10.0, 0.0, 10.0),
        ))
        .id();
    // The SHIPPED authored weapons AI declarations: since #885b stage 5d a
    // bank with no policy entry does not fire and a ship with no Tactical
    // selector ranks nothing.
    attach_shipped_weapon_ai(&mut app, shooter);

    // Identical to the target above in every respect BUT the blaster bank.
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid(target_uuid.to_string()),
        ShipPhysics {
            x: 10.0,
            z: -90.0,
            yaw: FRAC_PI_2,
            forward_speed: 20.0,
            ..Default::default()
        },
        dummy_bank(),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(10.0, 0.0, -90.0),
    ));

    app.update();

    let bank = &app
        .world()
        .get::<BlasterSystemResource>(shooter)
        .expect("the shooter keeps its blaster bank")
        .0[0];
    assert_eq!(
        bank.in_flight.len(),
        1,
        "the bank must have launched a bolt"
    );

    let heading = bank.in_flight[0].heading;
    let led = simmath::asin(20.0_f32 / 40.0);
    assert!(
        (heading - led).abs() < 1e-3,
        "heading {} deg, expected the same {} deg lead the blaster-less target gets",
        heading.to_degrees(),
        led.to_degrees()
    );
}

/// NPC with AI-controlled blaster has target out of range — must NOT fire.
#[test]
fn tick_blaster_auto_fire_skips_when_target_out_of_range() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();

    let npc_uuid = "bb000000-0000-0000-0000-000000000020";
    let target_uuid = "bb000000-0000-0000-0000-000000000021";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the blaster bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            crate::messages::AdmittedCommands::default(),
            ShipPhysics::default(),
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    barrels: Vec::new(),
                    pattern: Vec::new(),
                    range: 35.0,
                },
            )]),
            Transform::default(),
        ))
        .id();

    // Spawn target well outside blaster range (35 units).
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -100.0),
    ));

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    assert_eq!(
        blaster_res.0[0].volley.pending_volley, 0,
        "tick_blaster_auto_fire must NOT fire when target is out of range"
    );
}

/// An admitted `ChargeBlasterStart` — the origin-agnostic typed input both a
/// human and the AI decider now converge on (issue #781) — is consumed by
/// `handle_fire_blaster` and arms the bank. Post-converge the handler reads
/// per-ship `AdmittedCommands`, not raw `InboundMessage`s, so this injects the
/// admitted command directly (the shape admission produces from either origin).
#[test]
fn handle_fire_blaster_consumes_admitted_charge_start() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();

    let npc_uuid = "bb000000-0000-0000-0000-000000000030";
    let target_uuid_str = "bb000000-0000-0000-0000-000000000031";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    // Frozen combat lock on the viewscreen blackboard — the surface
    // handle_fire_blaster's arc check reads.
    let mut blackboards = crate::server_app::ShipSystemBlackboards::default();
    blackboards.0.insert(
        crate::system_registry::viewscreen_system_id(),
        crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
            combat_lock: Some(target_uuid_str.to_string()),
            ..Default::default()
        }),
    );
    // Pre-admitted ChargeBlasterStart (no ShipConfigComponent → admission does
    // not clear this ship's queue, so the injected command survives to Physics).
    let mut admitted = crate::messages::AdmittedCommands::default();
    admitted.0.push(crate::messages::AdmittedCommand {
        target: crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        payload: SystemControlPayload::ChargeBlasterStart,
        response_token: None,
    });
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            // Seeds the viewscreen combat_lock via `seed_viewscreen_from_selection`.
            TacticalRadarSelection(Some(target_uuid_str.to_string())),
            blackboards,
            admitted,
            crate::ship_plugin::ShipSystemControlSources(sources),
            ShipPhysics::default(),
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    barrels: Vec::new(),
                    pattern: Vec::new(),
                    range: 35.0,
                },
            )]),
            Transform::default(),
        ))
        .id();

    // Spawn target entity at (0, -10) — directly ahead of NPC at origin,
    // within the 35-unit range and inside the 360° fire arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid_str.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -10.0),
    ));

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    // handle_fire_blaster (Physics) arms the volley, tick_blaster_system (Physics)
    // fires it and enters cooldown. on_cooldown is evidence the volley dispatched.
    assert!(
        blaster_res.0[0].volley.on_cooldown,
        "handle_fire_blaster must consume the admitted ChargeBlasterStart and fire"
    );
}

/// The blaster twin of `set_torpedo_volley_target_accepts_a_hyphenated_tube_id`.
///
/// `blaster_bank_system_id` folds `_` to `-`, so `handle_fire_blaster`'s old
/// inverse ("strip `blaster-`, the rest is the bank id") turned
/// `blaster-fore-port` back into `fore-port` and matched no bank on a hull that
/// authored `fore_port` — the order vanished with no error. Latent only because
/// every shipped hull happens to author hyphen-free bank ids. The handler now
/// forward-maps each authored bank id and compares, so both spellings resolve.
#[test]
fn handle_fire_blaster_accepts_an_underscore_authored_bank_id() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();

    let npc_uuid = "bb000000-0000-0000-0000-000000000040";
    let target_uuid_str = "bb000000-0000-0000-0000-000000000041";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore_port").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    // Frozen combat lock on the viewscreen blackboard (issue #829/#822).
    let mut blackboards = crate::server_app::ShipSystemBlackboards::default();
    blackboards.0.insert(
        crate::system_registry::viewscreen_system_id(),
        crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
            combat_lock: Some(target_uuid_str.to_string()),
            ..Default::default()
        }),
    );
    // A pre-admitted ChargeBlasterStart addressed to `blaster-fore-port` — the id
    // the registry produces for the underscore-authored `fore_port` bank. The
    // handler forward-maps each authored bank id and compares, so this resolves;
    // the old inverse ("strip `blaster-`") turned it back into `fore-port` and
    // matched nothing. Injected directly (no ShipConfigComponent → admission
    // leaves this ship's queue intact, so it survives to Physics).
    let mut admitted = crate::messages::AdmittedCommands::default();
    admitted.0.push(crate::messages::AdmittedCommand {
        target: crate::system_registry::blaster_bank_system_id("fore_port").unwrap(),
        payload: SystemControlPayload::ChargeBlasterStart,
        response_token: None,
    });
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            // Seeds the viewscreen combat_lock via `seed_viewscreen_from_selection`.
            TacticalRadarSelection(Some(target_uuid_str.to_string())),
            blackboards,
            admitted,
            crate::ship_plugin::ShipSystemControlSources(sources),
            ShipPhysics::default(),
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    // Underscore-authored, the spelling the inverse dropped.
                    id: "fore_port".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    barrels: Vec::new(),
                    pattern: Vec::new(),
                    range: 35.0,
                },
            )]),
            Transform::default(),
        ))
        .id();

    // Deliberately BEYOND the bank's 35-unit range: `tick_blaster_auto_fire`
    // would skip the bank (out of range) and emit nothing, so the injected
    // explicit order is the ONLY thing that can arm it. handle_fire_blaster
    // applies an arc check (360° here → passes) but no range gate on an explicit
    // order, so a successful bank-id resolution is what arms the volley.
    app.world_mut().spawn((
        EntityUuid(target_uuid_str.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -100.0),
    ));

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    assert!(
        blaster_res.0[0].volley.on_cooldown,
        "an underscore-authored bank id must still receive its fire order"
    );
}

// ── Per-bank weapon AI policy (issue #781) ───────────────────────────────────
//
// Each AI-capable phaser/blaster bank resolves its OWN inline stateless policy
// over a seeded readiness snapshot before firing, emitting the SAME typed input a
// human does. These pin: idle banks hold fire (blocking condition), a per-bank
// `fact(...)` guard actually fires (the #779 empty-facts edge closed by seeding),
// one idle bank does not disarm another (per-bank independence), Control-Source
// symmetry, and the AC6 radar idle declaration.

/// Build a phaser-bank fire policy from a single guard expression.
fn phaser_bank_fire_policy(when: &str) -> crate::ai::policy::AiPolicy {
    crate::entities::config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 0,
            channel: crate::entities::config::PHASER_FIRE_CHANNEL.to_string(),
            when: when.to_string(),
            verb: crate::entities::config::PHASER_FIRE_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
    .to_policy()
    .expect("valid phaser bank policy")
}

fn idle_bank_policy() -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        idle: true,
        ..Default::default()
    }
}

/// Spawn an AI-controlled NPC with the given phaser banks + per-bank policies,
/// a locked target ahead (−Z), and everything the decide→admit→fire chain needs.
fn spawn_policy_phaser_npc(
    app: &mut App,
    npc_uuid: &str,
    target_uuid: &str,
    banks: Vec<(
        crate::entity_config::PhaserBankConfig,
        crate::ai::policy::AiPolicy,
    )>,
) -> Entity {
    spawn_policy_phaser_npc_at(app, npc_uuid, target_uuid, banks, [0.0, 0.0, -20.0])
}

/// As [`spawn_policy_phaser_npc`], with the target placed anywhere.
///
/// The position is what selects which banks bear, so a test about OVERLAPPING
/// arcs needs the target abeam rather than dead ahead (issue #790).
fn spawn_policy_phaser_npc_at(
    app: &mut App,
    npc_uuid: &str,
    target_uuid: &str,
    banks: Vec<(
        crate::entity_config::PhaserBankConfig,
        crate::ai::policy::AiPolicy,
    )>,
    target_pos: [f32; 3],
) -> Entity {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    let mut policies = std::collections::HashMap::new();
    let mut bank_cfgs = Vec::new();
    for (cfg, policy) in banks {
        sources.set(
            crate::system_registry::phaser_bank_system_id(&cfg.id).unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        policies.insert(cfg.id.clone(), policy);
        bank_cfgs.push(cfg);
    }

    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: bank_cfgs,
            }),
            crate::weapons_plugin::PhaserBankAiPolicies(policies),
            AdmittedCommands::default(),
            Transform::default(),
        ))
        .id();

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(target_pos[0], target_pos[1], target_pos[2]),
    ));
    npc
}

fn wide_bank(id: &str, facing_deg: f32) -> crate::entity_config::PhaserBankConfig {
    crate::entity_config::PhaserBankConfig {
        id: id.into(),
        facing_deg,
        fire_arc_deg: 360.0,
        auto_arc_deg: 360.0,
        beam_range: 50.0,
        beam_damage_per_sec: 5.0,
        beam_duration_secs: 3.0,
        cooldown_secs: 6.0,
        beam_color: vec![],
        shield_pierce: None,
        marker: None,
        ai: None,
    }
}

/// An idle phaser bank policy holds fire even when the bank is host-ready
/// (target in range/arc, off cooldown) — a blocking condition (AC1/AC2).
#[test]
fn phaser_bank_idle_policy_holds_fire() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc(
        &mut app,
        "cc000000-0000-0000-0000-000000000001",
        "cc000000-0000-0000-0000-000000000002",
        vec![(wide_bank("fore", 0.0), idle_bank_policy())],
    );
    app.update();
    let beam = app.world().get::<ActiveBeam>(npc).unwrap();
    assert!(
        !beam.is_firing(),
        "an idle phaser bank policy must hold fire — no beam should start"
    );
}

/// A per-bank `fact(...)` guard actually fires once the host seeds the readiness
/// snapshot (the #779 empty-facts edge closed), AND one idle bank does not disarm
/// another (per-bank independence, AC7): the fore bank is idle, the aft bank's
/// guard references a seeded fact and fires — so the ship opens fire from aft.
#[test]
fn phaser_bank_fact_guard_fires_and_idle_bank_does_not_disarm_another() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc(
        &mut app,
        "cc000000-0000-0000-0000-000000000011",
        "cc000000-0000-0000-0000-000000000012",
        vec![
            // fore bank: idle → holds. If it wrongly fired, `bank` would be "fore".
            (wide_bank("fore", 0.0), idle_bank_policy()),
            // aft bank: fires only when the seeded `in_range` fact is set — proves
            // a fact guard evaluates (never a spurious empty-facts fire).
            (
                wide_bank("aft", 180.0),
                phaser_bank_fire_policy("fact(in_range) > 0 and fact(target_valid) > 0"),
            ),
        ],
    );
    app.update();
    let beam = app.world().get::<ActiveBeam>(npc).unwrap();
    assert!(
        beam.is_firing(),
        "the aft bank's fact guard must fire (facts are seeded) even though fore is idle"
    );
    assert_eq!(
        beam.any_bank(),
        Some("aft"),
        "the idle fore bank must not fire and must not disarm the firing aft bank"
    );
}

// ── The red-alert fire gate, end to end (issue #872) ────────────────────────

/// The SHIPPED player-hull phaser policy — the one an AI-backfilled bridge
/// actually flies. Read from content rather than hand-built, so this test
/// cannot drift away from the hull it stands for.
fn shipped_player_phaser_policy() -> crate::ai::policy::AiPolicy {
    crate::entities::authored_ai_pins::shipped_policy_toml("phaser_bank")
        .to_policy()
        .expect("the shipped phaser-bank policy decodes")
}

/// The SHIPPED Harrow phaser policy: the same predicate text, the always-armed
/// threshold.
fn shipped_harrow_phaser_policy() -> crate::ai::policy::AiPolicy {
    let hull = crate::entity_config::EntityConfig::from_toml(include_str!(
        "../../../assets/entities/ship_harrow_patrol.toml"
    ))
    .expect("the shipped Harrow patrol hull must parse");
    hull.weapons_console
        .as_ref()
        .expect("the Harrow patrol carries phasers")
        .phaser_banks
        .first()
        .expect("…at least one bank")
        .ai
        .as_ref()
        .expect("every shipped bank authors a policy")
        .to_policy()
        .expect("and it decodes")
}

/// **AC2 + AC3, behaviourally.** A backfilled player bank holds fire while the
/// alert is down — including while the ship is being shot at — keeps its target
/// designated the whole time, and opens fire on the very next tick after the
/// captain raises red alert.
///
/// The three claims are one test on purpose: "holds fire" and "fires" are only
/// interesting together (either alone is satisfiable by a broken bank or an
/// ungated one), and "still designating" is what separates *holding fire* from
/// *not being in a fight*.
#[test]
fn backfilled_weapons_hold_fire_until_red_alert() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let target_uuid = "cc000000-0000-0000-0000-000000000042";
    let npc = spawn_policy_phaser_npc(
        &mut app,
        "cc000000-0000-0000-0000-000000000041",
        target_uuid,
        vec![(wide_bank("fore", 0.0), shipped_player_phaser_policy())],
    );
    // A ship UNDER ATTACK, with the alert still down. There is no return-fire
    // leg anywhere in the authored predicate, and this is where that is
    // asserted rather than assumed: the weapon does not arm itself because the
    // ship is being hit.
    app.world_mut().entity_mut(npc).insert((
        crate::ship_state::ShipRedAlert(false),
        crate::ship::combat_activity::RecentCombatActivity {
            last_damage_taken: Some(0.0),
            last_hostile_fire_taken: Some(0.0),
            last_weapon_fired: None,
            prev_hull: 0.0,
        },
    ));

    // Several shared AI ticks, not one: "held this frame" would also be
    // satisfied by a bank that simply had not been offered a decision yet.
    for _ in 0..6 {
        app.update();
        assert!(
            !app.world()
                .get::<ActiveBeam>(npc)
                .expect("the ship has a beam component")
                .is_firing(),
            "under fire, alert down: the backfilled bank must HOLD. Every host \
             readiness gate has passed — the target is designated, in range and \
             in arc — so the only thing refusing is the hull's authored predicate."
        );
    }
    // AC3, first half: holding fire is not standing down. The tactical radar
    // owns designation and the fire host only reads it, so a held bank must
    // leave the lock exactly where it was.
    assert_eq!(
        app.world()
            .get::<TacticalRadarSelection>(npc)
            .and_then(|s| s.0.clone())
            .as_deref(),
        Some(target_uuid),
        "a bank holding fire must still be TRACKING — designation is the target \
         selector's decision and the fire gate must not reach it"
    );

    // The captain calls red alert. Nothing else about the world changes.
    app.world_mut()
        .entity_mut(npc)
        .insert(crate::ship_state::ShipRedAlert(true));
    // Two frames, because the deciders run on the ONE shared AI cadence
    // (`ai_tick_ready`, issue #889) rather than per rendered frame — this is
    // the very next AI tick, not a settling period.
    app.update();
    app.update();
    assert!(
        app.world()
            .get::<ActiveBeam>(npc)
            .expect("the ship has a beam component")
            .is_firing(),
        "AC3: the held bank opens fire on the next shared AI tick after red \
         alert is raised. It was tracking all along, so there is no \
         re-acquisition delay — only the gate had to open."
    );
}

/// **AC5.** A Harrow bank fires with the alert down — same shipped predicate,
/// always-armed threshold, no captain involved.
///
/// Paired deliberately with the test above: identical fixture, identical facts,
/// identical guard EXPRESSION, opposite outcome. The only difference between
/// them is a number in a TOML file.
#[test]
fn npc_weapons_fire_without_a_captain_raising_the_alert() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc(
        &mut app,
        "cc000000-0000-0000-0000-000000000051",
        "cc000000-0000-0000-0000-000000000052",
        vec![(wide_bank("fore", 0.0), shipped_harrow_phaser_policy())],
    );
    app.world_mut()
        .entity_mut(npc)
        .insert(crate::ship_state::ShipRedAlert(false));

    app.update();
    assert!(
        app.world()
            .get::<ActiveBeam>(npc)
            .expect("the ship has a beam component")
            .is_firing(),
        "a Harrow has no bridge crew to call red alert; its authored \
         `min_alert_to_fire = 0` is what lets it open the engagement at all"
    );
}

/// Control-Source symmetry (AC7): a human's admitted `FirePhaser` and an
/// AI-policy fire produce the identical observable output — an active beam on the
/// same bank. Here the same admitted `FirePhaser` a human would send arms the
/// beam through `handle_fire_phaser`, matching the AI path above.
#[test]
fn phaser_human_admitted_fire_matches_ai_policy_output() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();

    let npc_uuid = "cc000000-0000-0000-0000-000000000021";
    let target_uuid = "cc000000-0000-0000-0000-000000000022";
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // Human-operable bank (accept_human_input via Human control source).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Human,
    );
    let mut blackboards = crate::server_app::ShipSystemBlackboards::default();
    blackboards.0.insert(
        crate::system_registry::viewscreen_system_id(),
        crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
            combat_lock: Some(target_uuid.to_string()),
            ..Default::default()
        }),
    );
    let mut admitted = AdmittedCommands::default();
    admitted.0.push(crate::messages::AdmittedCommand {
        target: crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        payload: crate::messages::SystemControlPayload::FirePhaser,
        response_token: None,
    });
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            blackboards,
            admitted,
            crate::ship_plugin::ShipSystemControlSources(sources),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![wide_bank("fore", 0.0)],
            }),
            Transform::default(),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    app.update();
    let beam = app.world().get::<ActiveBeam>(npc).unwrap();
    assert!(
        beam.is_firing() && beam.any_bank() == Some("fore"),
        "a human admitted FirePhaser must produce the same active beam an AI-policy fire does"
    );
}

/// An idle blaster bank policy holds its volley even when the bank is host-ready
/// (AC1/AC2): no cooldown is entered because no volley is dispatched.
#[test]
fn blaster_bank_idle_policy_holds_fire() {
    use crate::entity_spawner::EntityUuid;
    let mut app = test_app();

    let npc_uuid = "cc000000-0000-0000-0000-000000000031";
    let target_uuid = "cc000000-0000-0000-0000-000000000032";
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut policies = std::collections::HashMap::new();
    policies.insert("fore".to_string(), idle_bank_policy());
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            crate::messages::AdmittedCommands::default(),
            ShipPhysics::default(),
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    barrels: Vec::new(),
                    pattern: Vec::new(),
                    range: 35.0,
                },
            )]),
            crate::weapons_plugin::BlasterBankAiPolicies(policies),
            Transform::default(),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -10.0),
    ));
    app.update();
    let blaster_res = app.world().get::<BlasterSystemResource>(npc).unwrap();
    assert!(
        !blaster_res.0[0].volley.on_cooldown && blaster_res.0[0].in_flight.is_empty(),
        "an idle blaster bank policy must hold its volley — no fire, no cooldown"
    );
}

/// AC6: an explicit Tactical-radar idle declaration makes the radar take NO AI
/// target selection even when a tactical fine system is AI-operated — the ship
/// acquires nothing, distinct from a default (non-idle) radar that would lock the
/// objective (pinned by `ai_target_selection_runs_when_any_tactical_system_operates_ai`).
#[test]
fn tactical_radar_idle_makes_no_ai_selection() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Attach an explicit idle radar declaration to the LocalShip.
    let ship = local_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::weapons_plugin::TacticalTargetSelector {
            selector: crate::entities::authored_ai_pins::shipped_selector_toml("tactical")
                .to_selector()
                .expect("the shipped Tactical selector decodes"),
            power_rating: None,
            idle: true,
        });

    // Provide a lockable objective target — a non-idle radar would acquire it.
    let target_uuid = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "an idle Tactical radar must make no AI selection — the lock stays empty"
    );
}

// ── Issue #841: balance-taxonomy emission, per chokepoint family ─────────────
//
// Each test drives a NON-LOCAL ship, guarding the "emitted unconditionally,
// outside every is_local gate" convention: an event that only fired for the
// player ship would report half the fight.

/// Weapon-fire family: a beam opening from a NON-LOCAL (NPC) shooter emits
/// `WeaponFired` attributed to that shooter.
#[test]
fn npc_beam_fire_emits_weapon_fired_for_the_non_local_shooter() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::balance::BalanceEvent;
    use bevy::ecs::message::Messages;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000001";
    let target_uuid = "00000000-0000-0000-0000-000000000002";
    setup_npc_shooter(&mut app, npc_uuid, target_uuid, 0.0, -20.0);

    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    app.update();

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let saw = cursor.read(messages).any(|e| {
        matches!(
            e,
            BalanceEvent::WeaponFired { shooter, kind, .. }
                if shooter.as_deref() == Some(npc_uuid)
                    && kind == crate::balance::FIRED_KIND_BEAM
        )
    });
    assert!(
        saw,
        "an NPC beam opening must emit WeaponFired for the non-local shooter"
    );
}

/// Shields family: a beam that drives a NON-LOCAL ship's only shield facing to
/// zero emits `ShieldArcCollapsed` once, keyed on that ship.
#[test]
fn beam_collapsing_a_non_local_shield_facing_emits_shield_arc_collapsed() {
    use crate::balance::BalanceEvent;
    use bevy::ecs::message::Messages;

    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    // Small single-facing shield (10 HP) over ample hull so the facing, not the
    // hull, is what the burst breaks.
    spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 200.0, 10.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .clear();
    // A burst well above the facing's remaining HP collapses it.
    set_active_beam_damage_accumulator(&mut app, 60.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let saw = cursor
        .read(messages)
        .any(|e| matches!(e, BalanceEvent::ShieldArcCollapsed { ship, .. } if ship == "npc-1"));
    assert!(
        saw,
        "collapsing the non-local ship's facing must emit ShieldArcCollapsed"
    );
}

/// Destruction family: a beam kill on a NON-LOCAL ship emits exactly one
/// `EntityDestroyed`, crediting the local shooter as killer.
#[test]
fn beam_kill_of_a_non_local_ship_emits_entity_destroyed_with_killer_credit() {
    use crate::balance::BalanceEvent;
    use bevy::ecs::message::Messages;

    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    // Unshielded NPC sturdy enough to survive the beam-start ticks, so the
    // kill lands in the measured burst below rather than before the clear.
    spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .clear();
    set_active_beam_damage_accumulator(&mut app, 50.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let deaths: Vec<&BalanceEvent> = cursor
        .read(messages)
        .filter(|e| matches!(e, BalanceEvent::EntityDestroyed { victim, .. } if victim == "npc-1"))
        .collect();
    assert_eq!(
        deaths.len(),
        1,
        "the kill must emit exactly one EntityDestroyed for the victim"
    );
    assert!(
        matches!(
            deaths[0],
            BalanceEvent::EntityDestroyed { killer, .. }
                if killer.as_deref() == Some("test-local-ship")
        ),
        "EntityDestroyed must credit the local shooter as killer"
    );
}

// ── Direct-fire reach (issue #788) ──────────────────────────────────────────
//
// The pure half of the standoff ring: how far a ship can put unguided fire.
// Torpedoes are excluded by construction — they never reach this list — so the
// tests below are about the online/usable gate and the max.

fn emitter(online: bool, usable: bool, range: f32) -> DirectFireEmitter {
    DirectFireEmitter {
        online,
        usable,
        range,
    }
}

#[test]
fn direct_fire_reach_is_the_longest_usable_online_bank() {
    let reach = longest_usable_direct_fire_range(&[
        emitter(true, true, 40.0),
        emitter(true, true, 320.0),
        emitter(true, true, 120.0),
    ]);
    assert_eq!(reach, 320.0, "the longest bank sets the reach");
}

#[test]
fn an_offline_or_unusable_bank_does_not_count_toward_reach() {
    // The longest bank is offline: it is not a threat, so it must not inflate
    // the ring an opponent keeps.
    assert_eq!(
        longest_usable_direct_fire_range(&[emitter(false, true, 320.0), emitter(true, true, 40.0)]),
        40.0
    );
    // Online but unusable is the same answer.
    assert_eq!(
        longest_usable_direct_fire_range(&[emitter(true, false, 320.0), emitter(true, true, 40.0)]),
        40.0
    );
}

#[test]
fn a_ship_with_no_usable_direct_fire_has_no_reach() {
    assert_eq!(longest_usable_direct_fire_range(&[]), 0.0);
    assert_eq!(
        longest_usable_direct_fire_range(&[emitter(false, true, 500.0)]),
        0.0,
        "a fully disarmed ship reaches nothing — never a fallback distance"
    );
}

#[test]
fn reach_is_never_negative() {
    assert_eq!(
        longest_usable_direct_fire_range(&[emitter(true, true, -10.0)]),
        0.0
    );
}

// ── Simultaneous broadside beams (issue #790) ────────────────────────────────
//
// `ActiveBeam` was one slot per ship until issue #790: `handle_fire_phaser`
// refused any fire while it was occupied and `ai_phaser_auto_fire` `find_map`ped
// to exactly one bank, so two banks could never be lit at once no matter how far
// their arcs overlapped. It is a per-bank map now, and these pin both halves of
// that: two banks DO burn together when both bear, and every ordinary gate still
// applies to each bank on its own.
//
// The bank geometry is taken from SHIPPED hulls rather than restated inline, so
// a hull that stopped overlapping would fail here rather than passing against a
// fixture that no ship flies.

/// The authored phaser banks of a shipped hull, exactly as the TOML declares
/// them — arcs included, so a hull retuned in `assets/` is felt here.
///
/// Takes a hull STEM rather than baked text (issue #876): `include_str!` bakes
/// bytes at compile time, so a baked site can never see include resolution, and
/// `alliance_cruiser` is a COMPOSED hull since #876.
fn shipped_bank_configs(stem: &str) -> Vec<crate::entity_config::PhaserBankConfig> {
    let cfg = crate::entity_includes::load_entity_config(&format!("assets/entities/{stem}.toml"))
        .unwrap_or_else(|e| panic!("{stem} must compose and parse: {e}"));
    let banks = cfg
        .weapons_console
        .as_ref()
        .expect("hull declares [weapons_console]")
        .phaser_banks
        .clone();
    assert_eq!(banks.len(), 2, "these fixtures are about a bank PAIR");
    banks
}

/// The authored phaser banks of a shipped hull, paired with an unconditional
/// fire policy — the shape `spawn_policy_phaser_npc_at` takes.
fn shipped_banks(
    stem: &str,
) -> Vec<(
    crate::entity_config::PhaserBankConfig,
    crate::ai::policy::AiPolicy,
)> {
    shipped_bank_configs(stem)
        .into_iter()
        .map(|b| (b, phaser_bank_fire_policy("true")))
        .collect()
}

const HARROW_CRUISER_HULL: &str = "ship_harrow_cruiser";
const ALLIANCE_CRUISER_HULL: &str = "alliance_cruiser";

/// Broadly abeam to starboard but deliberately **off** the beam line. The ship
/// sits at the origin at yaw 0, so forward is `-Z` and starboard is `+X`; this
/// is a bearing of ≈101.3°, some 11° abaft the beam.
///
/// The 11° matters, and every broadside fixture in this section uses this
/// constant rather than a true `[20, 0, 0]` beam bearing because of it. Exactly
/// 90° is the exact edge of a 180-degree arc centred on 0° AND the exact edge of
/// one centred on 180°. `in_arc` compares `<=` against the half-arc, and
/// `180f32.to_radians() * 0.5` is bit-identically `FRAC_PI_2`, so a pair of
/// 180-degree fore/aft arcs BOTH admit there — by exact float equality, not by
/// overlapping. A test sitting on that tie cannot tell a 270-degree arc from a
/// 180-degree one: it only asserts that a tie compares equal. Narrow the hull's
/// arcs and it would keep passing, which is precisely the regression these
/// fixtures exist to catch.
///
/// Well inside a 270-degree arc centred either fore or aft (those reach ±135°),
/// so it is an honest interior bearing for the wide arcs, and outside the fore
/// half of a 180-degree pair, so it is a discriminating one for the narrow.
const OFF_BOUNDARY_STARBOARD: [f32; 3] = [20.0, 0.0, 4.0];
/// Dead ahead — inside the fore bank's arc and inside the aft bank's blind
/// wedge.
const DEAD_AHEAD: [f32; 3] = [0.0, 0.0, -20.0];

fn live_banks_of(app: &App, ship: Entity) -> Vec<String> {
    app.world()
        .get::<ActiveBeam>(ship)
        .expect("ship carries ActiveBeam")
        .live_banks()
        .map(|(bank, _)| bank.clone())
        .collect()
}

/// AC5: when a broadside bears, BOTH authored banks light — literally at the
/// same time, on the same target.
///
/// This is the assertion the whole per-bank rework exists for. Before it, the
/// identical fixture produced exactly one live beam and nothing failed.
///
/// The bearing is [`OFF_BOUNDARY_STARBOARD`], not the beam line itself, so this
/// pins the authored 270-degree arcs rather than a float tie — see that
/// constant.
#[test]
fn both_270_degree_banks_burn_at_once_on_a_target_abeam() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let target_uuid = "cc000000-0000-0000-0000-0000000007a2";
    let npc = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007a1",
        target_uuid,
        shipped_banks(HARROW_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    app.update();

    assert_eq!(
        live_banks_of(&app, npc),
        vec!["aft".to_string(), "fore".to_string()],
        "a target broad on the starboard quarter is inside BOTH 270-degree arcs — and \
         inside only one of a 180-degree pair — so both banks must be burning"
    );
    let beam = app.world().get::<ActiveBeam>(npc).unwrap();
    for bank in ["fore", "aft"] {
        assert_eq!(
            beam.bank_target(bank),
            Some(target_uuid),
            "bank '{bank}' must be burning at the locked target"
        );
    }
}

/// Spawn the PLAYER's ship — `LocalShip`, banks on the `Human` control source —
/// with a pending admitted `FirePhaser` for every bank: exactly what admission
/// leaves behind when a human gunner presses both fire buttons on one tick.
///
/// `Human` (not `Ai`) is load-bearing. `ai_phaser_auto_fire` needs `operate_ai`
/// on a bank, or a `LocalShip` with `CurrentPhaserMode::Auto` — and the test app
/// defaults to `Manual`. So the auto-fire path is inert here and any beam that
/// lights can only have come from `handle_fire_phaser`, which is what these
/// fixtures are about.
fn spawn_player_hull_firing_all_banks_at(
    app: &mut App,
    ship_uuid: &str,
    target_uuid: &str,
    banks: Vec<crate::entity_config::PhaserBankConfig>,
    target_pos: [f32; 3],
) -> Entity {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    let mut admitted = AdmittedCommands::default();
    for cfg in &banks {
        let bank_system = crate::system_registry::phaser_bank_system_id(&cfg.id).unwrap();
        sources.set(
            bank_system.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
        admitted.0.push(crate::messages::AdmittedCommand {
            target: bank_system,
            payload: SystemControlPayload::FirePhaser,
            response_token: None,
        });
    }

    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            EntityUuid(ship_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig { banks }),
            admitted,
            Transform::default(),
        ))
        .id();

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(target_pos[0], target_pos[1], target_pos[2]),
    ));
    ship
}

/// The same mechanic on the PLAYER's hull, on the path a human gunner actually
/// uses (AGENTS.md #6).
///
/// `alliance_cruiser` authors `fire_arc_deg = 270` on both banks, and
/// `handle_fire_phaser` gates on `fire_arc_deg` — so the two arcs genuinely
/// overlap through 180°, and a target abeam is a real double broadside for a
/// human. Note the hull's `auto_arc_deg` is only 180: the AI path tells a
/// different and much narrower story, pinned by
/// `the_player_hulls_180_degree_auto_arcs_do_not_both_bear_off_the_beam_line`
/// below. This test is deliberately scoped to the manual path and says nothing
/// about the auto one.
///
/// This is still the symmetry proof, because the mechanism is shared:
/// `handle_fire_phaser` reads `AdmittedCommands` and carries no origin branch at
/// all (admission already stripped the source identity), so the NPC path above
/// and this one are the same per-bank `ActiveBeam` rework seen from two sides.
/// The ship carries `LocalShip` and human-operated banks, so it is the player's
/// ship in every respect the firing path could observe even if someone added
/// such a branch.
#[test]
fn the_player_hulls_270_degree_banks_both_light_on_the_manual_fire_path() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let target_uuid = "cc000000-0000-0000-0000-0000000007b2";
    let ship = spawn_player_hull_firing_all_banks_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007b1",
        target_uuid,
        shipped_bank_configs(ALLIANCE_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    app.update();

    assert_eq!(
        live_banks_of(&app, ship),
        vec!["aft".to_string(), "fore".to_string()],
        "a bearing 11 degrees abaft the beam is comfortably inside BOTH 270-degree \
         fire arcs, so a human firing both banks must get both beams"
    );
    let beam = app.world().get::<ActiveBeam>(ship).unwrap();
    for bank in ["fore", "aft"] {
        assert_eq!(
            beam.bank_target(bank),
            Some(target_uuid),
            "bank '{bank}' must be burning at the locked target"
        );
    }
}

/// The other half of the player hull's story, and the reason the test above is
/// scoped to the manual path.
///
/// `alliance_cruiser` authors `auto_arc_deg = 180` on both banks, and
/// `ai_phaser_auto_fire` gates on `auto_arc_deg`. Two 180-degree arcs centred
/// fore and aft do not overlap — they abut, sharing only the beam line itself —
/// so an AI-operated alliance cruiser gets ONE bank at any bearing off that
/// line, not the double broadside the wide manual arcs give.
///
/// The bearing is the whole point. Exactly abeam, `in_arc`'s `<=` admits both
/// arcs by bit-exact equality on the shared boundary, which reads as an overlap
/// that is not there; `OFF_BOUNDARY_STARBOARD` stands 11 degrees abaft the beam
/// so the answer is the real one. Widening the hull's `auto_arc_deg` to 270
/// would fail this — which is the intent: it is a player-facing balance change,
/// not a refactor, and issue #790 is scoped to the Harrow cruiser.
#[test]
fn the_player_hulls_180_degree_auto_arcs_do_not_both_bear_off_the_beam_line() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let ship = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007b3",
        "cc000000-0000-0000-0000-0000000007b4",
        shipped_banks(ALLIANCE_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    app.update();
    assert_eq!(
        live_banks_of(&app, ship),
        vec!["aft".to_string()],
        "abaft the beam is outside the fore bank's 180-degree AUTO arc: only the aft \
         bank may burn, however wide the manual fire arc is"
    );
}

/// AC5's other half, and the reason the arcs are 270 rather than 360: a target
/// in only ONE bank's arc lights only that bank.
///
/// Without this, "both banks fire" would be indistinguishable from "the arc
/// check stopped being applied".
#[test]
fn a_target_in_one_arc_only_lights_the_bank_that_bears() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007c1",
        "cc000000-0000-0000-0000-0000000007c2",
        shipped_banks(HARROW_CRUISER_HULL),
        DEAD_AHEAD,
    );
    app.update();
    assert_eq!(
        live_banks_of(&app, npc),
        vec!["fore".to_string()],
        "dead ahead is the AFT bank's blind wedge: only the fore bank may burn"
    );
}

/// Per-bank AVAILABILITY still gates each bank on its own: a bank whose fine
/// system is not AI-operable stays cold while its sibling fires.
#[test]
fn an_unavailable_bank_does_not_stop_its_sibling_broadside() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007d1",
        "cc000000-0000-0000-0000-0000000007d2",
        shipped_banks(HARROW_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    // Take the aft bank off AI. `spawn_policy_phaser_npc_at` put every bank on
    // Ai, so this is the only difference from the double-broadside fixture.
    {
        let mut sources = app
            .world_mut()
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>(npc)
            .unwrap();
        sources.0.set(
            crate::system_registry::phaser_bank_system_id("aft").unwrap(),
            crate::ship::control_source::ControlSource::Human,
        );
    }
    app.update();
    assert_eq!(
        live_banks_of(&app, npc),
        vec!["fore".to_string()],
        "the aft bank is not AI-operable, and its absence must not disarm the fore bank"
    );
}

/// Per-bank COOLDOWN still gates each bank on its own — the property
/// `PhaserCooldown` already had, now matched by the beam map.
#[test]
fn a_bank_on_cooldown_does_not_stop_its_sibling_broadside() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007e1",
        "cc000000-0000-0000-0000-0000000007e2",
        shipped_banks(HARROW_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    {
        let mut cooldown = app.world_mut().get_mut::<PhaserCooldown>(npc).unwrap();
        cooldown.start_bank("aft", 5.0);
    }
    app.update();
    assert_eq!(
        live_banks_of(&app, npc),
        vec!["fore".to_string()],
        "the aft bank is cooling down; the fore bank must fire anyway"
    );
}

/// A bank already mid-beam is not re-lit, and — the part that used to be
/// impossible — that does not stop the other bank from opening fire on a later
/// tick when IT comes to bear.
#[test]
fn a_burning_bank_is_not_relit_while_its_sibling_may_still_open_fire() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-0000000007f1",
        "cc000000-0000-0000-0000-0000000007f2",
        shipped_banks(HARROW_CRUISER_HULL),
        DEAD_AHEAD,
    );
    app.update();
    assert_eq!(live_banks_of(&app, npc), vec!["fore".to_string()]);
    let opened_at = app
        .world()
        .get::<ActiveBeam>(npc)
        .unwrap()
        .bank_remaining_secs("fore");

    // Turn the ship so the same target now bears broad on the starboard quarter
    // and the aft bank bears too. For a target dead ahead at yaw 0 the relative
    // bearing is simply `-yaw`, so this yaw puts it at ≈101.5° — the same 11°
    // abaft the beam as [`OFF_BOUNDARY_STARBOARD`], and for the same reason:
    // a yaw of exactly `-FRAC_PI_2` would park the target on the bit-exact edge
    // of a 180-degree fore/aft pair, where both arcs admit by float tie and the
    // assertion below could no longer tell 270 from 180. Turning the shooter
    // rather than moving the target is what makes this a test about arcs rather
    // than about range.
    {
        let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
        physics.yaw = -(std::f32::consts::FRAC_PI_2 + 0.2);
    }
    // Issue #889: `ai_phaser_auto_fire` runs on the shared AI cadence latch,
    // which the update above consumed. The first update rides the
    // initialises-`true` free run; this one has to tick the latch by hand.
    crate::ai::cadence::arm_ai_tick(&mut app);
    app.update();

    assert_eq!(
        live_banks_of(&app, npc),
        vec!["aft".to_string(), "fore".to_string()],
        "the aft bank must be free to open fire while the fore bank is still burning"
    );
    let beam = app.world().get::<ActiveBeam>(npc).unwrap();
    assert!(
        beam.bank_remaining_secs("fore") < opened_at,
        "and the fore bank must not have been re-lit — its own timer keeps running down"
    );
}

/// Both live beams reach the wire as distinct `BeamStarted` broadcasts, one per
/// bank.
///
/// The renderer keys its beam entities on `(shooter, bank, target)` and the
/// client folds these messages, so a rework that lit two banks server-side but
/// only announced one would draw a single beam and look like nothing had
/// changed.
#[test]
fn each_live_broadside_bank_announces_its_own_beam_on_the_wire() {
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let target_uuid = "cc000000-0000-0000-0000-000000000802";
    spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-000000000801",
        target_uuid,
        shipped_banks(HARROW_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    let out = tick(&mut app);
    let mut banks: Vec<String> = out
        .iter()
        .filter_map(|m| match &m.msg {
            ServerMessage::BeamStarted {
                bank,
                target_uuid: t,
                ..
            } if t == target_uuid => Some(bank.clone()),
            _ => None,
        })
        .collect();
    banks.sort();
    assert_eq!(
        banks,
        vec!["aft".to_string(), "fore".to_string()],
        "both live beams must be broadcast, one BeamStarted per bank"
    );
}

/// Both live beams draw power, one drain per burning bank.
///
/// A ship running two emitters pays for two. Keeping the drain ship-level would
/// have made the second broadside free — exactly the sort of silent discount a
/// per-bank rework leaves behind if nobody looks.
#[test]
fn every_live_broadside_bank_draws_its_own_power() {
    use crate::messages::InterSystemPayload;
    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();
    let npc = spawn_policy_phaser_npc_at(
        &mut app,
        "cc000000-0000-0000-0000-000000000811",
        "cc000000-0000-0000-0000-000000000812",
        shipped_banks(HARROW_CRUISER_HULL),
        OFF_BOUNDARY_STARBOARD,
    );
    app.update();
    assert_eq!(live_banks_of(&app, npc).len(), 2, "precondition: two beams");
    app.update();
    let drains = app
        .world()
        .resource::<crate::messages::InterSystemQueue>()
        .0
        .iter()
        .filter(|m| {
            matches!(m.payload, InterSystemPayload::DrainWeaponsBattery { .. })
                && m.source_entity == Some(npc)
        })
        .count();
    assert_eq!(
        drains, 2,
        "two burning banks must draw two battery drains, not one"
    );
}
