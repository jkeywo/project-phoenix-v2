use bevy::prelude::*;

use crate::lobby::{InboundMessage, OutboundMessage, Target, Sessions, WorldResource};
use crate::messages::{
    ClientMessage, Console, ModifierSlot, ServerMessage,
};
use crate::simulation::{Ship, Asteroid, AsteroidUuid, AsteroidDamage, SimOutbox};
use crate::torpedo::{TorpedoSystem, TorpedoConfig, TorpedoTubeId};
use crate::messages::TorpedoTube as MsgTorpedoTube;
use crate::ship_state::ShipState;
use crate::radar::WEAPONS_RADAR_RANGE;

// ── Beam constants ───────────────────────────────────────────────────────
const BEAM_DURATION_SECS: f32 = 6.0;
pub const BEAM_DAMAGE_PER_SEC: f32 = 5.0;
const BEAM_COOLDOWN_SECS: f32 = 6.0;

// ── Resources ─────────────────────────────────────────────────────────────

/// The currently locked target UUID on the Weapons console. `None` means no
/// lock is active.
#[derive(Resource, Default)]
pub struct WeaponsTarget(pub Option<String>);

/// Active phaser beam state. `target_uuid` is `Some` while a beam is firing.
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
#[derive(Resource, Default)]
pub struct ActiveBeam {
    pub target_uuid: Option<String>,
    pub remaining_secs: f32,
    pub damage_accumulator: f32,
    /// Which bank is firing this beam. `None` when no beam is active.
    pub bank: Option<crate::messages::PhaserBank>,
}

/// Post-beam cooldown. The weapons console is locked out for `BEAM_COOLDOWN_SECS`
/// after every beam end (natural, sever, or cancel).
#[derive(Resource, Default)]
pub struct PhaserCooldown {
    pub remaining_secs: f32,
}

impl PhaserCooldown {
    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }

    pub fn start(&mut self) {
        self.remaining_secs = BEAM_COOLDOWN_SECS;
    }

    pub fn tick(&mut self, dt: f32) {
        self.remaining_secs = (self.remaining_secs - dt).max(0.0);
    }
}

/// Current phaser firing mode (Auto or Manual), set by the Weapons console.
#[derive(Resource)]
pub struct CurrentPhaserMode(pub crate::messages::PhaserMode);

impl Default for CurrentPhaserMode {
    fn default() -> Self {
        Self(crate::messages::PhaserMode::Auto)
    }
}

/// Rendering config for the phaser beam (colour, max range).
/// Populated from ship entity TOML during world setup; defaults are used if
/// the TOML is absent.
#[derive(Resource, Clone, Debug)]
pub struct PhaserRenderConfig {
    /// RGBA beam colour in 0.0–1.0.
    pub beam_color: [f32; 4],
    /// Maximum beam range (world units); beam endpoint is clamped to this.
    pub beam_range: f32,
}

impl Default for PhaserRenderConfig {
    fn default() -> Self {
        Self {
            beam_color: crate::beam_render::DEFAULT_BEAM_COLOR,
            beam_range: 40.0,
        }
    }
}

/// Wraps the pure-Rust torpedo system so it can be used as a Bevy resource.
#[derive(Resource)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<PhaserRenderConfig>()
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())))
            .add_message::<AsteroidDestroyedVfx>()
            .add_systems(Update, (
                handle_set_target.in_set(crate::sim_sets::SimSet::Input),
                handle_fire_phaser.in_set(crate::sim_sets::SimSet::Input),
                handle_set_phaser_mode.in_set(crate::sim_sets::SimSet::Input),
                handle_set_phaser_frequency.in_set(crate::sim_sets::SimSet::Input),
                handle_fire_torpedo.in_set(crate::sim_sets::SimSet::Input),
            ))
            .add_systems(Update, (
                tick_active_beam.in_set(crate::sim_sets::SimSet::Physics),
                tick_torpedo_system.in_set(crate::sim_sets::SimSet::Physics),
            ));
    }
}

// ── Systems ─────────────────────────────────────────────────────────────────

fn handle_set_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    mut weapons_target: ResMut<WeaponsTarget>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
) {
    for ev in reader.read() {
        let ClientMessage::SetTarget { uuid } = &ev.msg else { continue };

        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }

        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let effective_weapons_range = WEAPONS_RADAR_RANGE * radar_range_mult;
        let asteroid = world.0.entities.iter().find(|a| &a.uuid == uuid);
        let locked = match asteroid {
            None => false,
            Some(a) => {
                let dx = a.x() - ship.x;
                let dz = a.z() - ship.z;
                dx * dx + dz * dz <= effective_weapons_range * effective_weapons_range
            }
        };

        if locked {
            weapons_target.0 = Some(uuid.clone());
        } else {
            weapons_target.0 = None;
        }

        outbox.0.push((Target::Token(ev.token.clone()), ServerMessage::TargetLock { uuid: uuid.clone(), locked }));
    }
}

fn handle_fire_phaser(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    cooldown: Res<PhaserCooldown>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
) {
    for ev in reader.read() {
        if !matches!(ev.msg, ClientMessage::FirePhaser) {
            continue;
        }
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        if cooldown.is_active() || beam.target_uuid.is_some() {
            continue;
        }
        let Some(target_uuid) = &weapons_target.0 else { continue };
        let Some(asteroid) = world.0.entities.iter().find(|a| &a.uuid == target_uuid) else {
            continue;
        };
        let effective_phaser_range = crate::radar::PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
        if !crate::radar::is_fire_ready_with_range(asteroid.x(), asteroid.z(), ship.x, ship.z, ship.yaw, effective_phaser_range) {
            continue;
        }

        if let Some(old_uuid) = beam.target_uuid.take() {
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid: old_uuid }));
        }

        let next_bank = match beam.bank {
            Some(crate::messages::PhaserBank::Port) => crate::messages::PhaserBank::Starboard,
            _ => crate::messages::PhaserBank::Port,
        };
        beam.target_uuid = Some(target_uuid.clone());
        beam.remaining_secs = BEAM_DURATION_SECS;
        beam.damage_accumulator = 0.0;
        beam.bank = Some(next_bank);

        outbox.0.push((Target::All, ServerMessage::BeamStarted { target_uuid: target_uuid.clone() }));
    }
}

fn handle_set_phaser_mode(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    for ev in reader.read() {
        let ClientMessage::SetPhaserMode { mode } = &ev.msg else { continue };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        phaser_mode.0 = *mode;
    }
}

fn handle_set_phaser_frequency(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    complexity: Res<crate::console_ai_plugin::ConsoleComplexityState>,
    mut ship: ResMut<ShipState>,
) {
    use crate::delegation::{is_sender_authorized, ComplexityContext, DelegatedControl};
    let ctx = ComplexityContext {
        tactical_is_low: complexity.is_low(&Console::Tactical),
    };
    for ev in reader.read() {
        let ClientMessage::SetPhaserFrequency { frequency } = &ev.msg else { continue };

        let sender_console = if sessions.0.console_holder(Console::Tactical) == Some(ev.token.as_str()) {
            Console::Tactical
        } else if sessions.0.console_holder(Console::Sensors) == Some(ev.token.as_str()) {
            Console::Sensors
        } else {
            continue;
        };

        if !is_sender_authorized(DelegatedControl::SetPhaserFrequency, &sender_console, &ctx) {
            continue;
        }

        ship.phaser_frequency = frequency.clamp(0.0, 1.0);
    }
}

fn to_tube_id(tube: MsgTorpedoTube) -> TorpedoTubeId {
    match tube {
        MsgTorpedoTube::ForePort => TorpedoTubeId::ForePort,
        MsgTorpedoTube::ForeStarboard => TorpedoTubeId::ForeStarboard,
        MsgTorpedoTube::Aft => TorpedoTubeId::Aft,
    }
}

fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
) {
    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else { continue };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        let tube_id = to_tube_id(*tube);
        let uuid = uuid::Uuid::new_v4().to_string();
        let launch_heading = ship.yaw;
        use crate::torpedo::LaunchResult;
        match torpedo_sys.0.launch(tube_id, uuid, ship.x, ship.z, launch_heading, target_uuid.clone()) {
            LaunchResult::Launched { uuid: launched_uuid } => {
                outbox.0.push((Target::All, ServerMessage::TorpedoLaunched {
                    uuid: launched_uuid,
                    tube: *tube,
                    x: ship.x,
                    z: ship.z,
                    heading: launch_heading,
                }));
            }
            LaunchResult::TubeNotLoaded | LaunchResult::NoTorpedoes => {}
        }
    }
}

fn tick_torpedo_system(
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    world: Res<WorldResource>,
    time: Res<Time>,
    mut outbox: ResMut<SimOutbox>,
) {
    let dt = time.delta_secs();
    let target_positions: std::collections::HashMap<String, (f32, f32)> = world.0.entities
        .iter()
        .map(|a| (a.uuid.clone(), (a.x(), a.z())))
        .collect();
    let result = torpedo_sys.0.tick(dt, &target_positions);
    for expired_uuid in result.expired {
        outbox.0.push((Target::All, ServerMessage::TorpedoDestroyed { uuid: expired_uuid }));
    }
}

/// Active beam tick handler for weapons plugin integration tests
/// to reference when building their test app.
fn tick_active_beam(
    time: Res<Time>,
    mut beam: ResMut<ActiveBeam>,
    mut cooldown: ResMut<PhaserCooldown>,
    ship: Res<ShipState>,
    mut world: ResMut<WorldResource>,
    mut asteroid_query: Query<(Entity, &AsteroidUuid, &mut AsteroidDamage)>,
    mut commands: Commands,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
) {

    let dt = time.delta_secs();
    cooldown.tick(dt);

    let Some(target_uuid) = beam.target_uuid.clone() else {
        return;
    };

    let asteroid_info = world.0.entities.iter().find(|a| a.uuid == target_uuid).cloned();
    let Some(info) = asteroid_info else {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
        return;
    };

    let effective_phaser_range = crate::radar::PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
    if !crate::radar::is_fire_ready_with_range(info.x(), info.z(), ship.x, ship.z, ship.yaw, effective_phaser_range) {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
        return;
    }

    beam.damage_accumulator += BEAM_DAMAGE_PER_SEC * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        let mut destroyed = false;
        for (entity, uuid_comp, mut dmg) in asteroid_query.iter_mut() {
            if uuid_comp.0 == target_uuid {
                dmg.current_hp = (dmg.current_hp - damage_to_apply).max(0);
                if dmg.current_hp == 0 {
                    destroyed = true;
                    commands.entity(entity).despawn();
                }
            }
        }

        if destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            vfx_events.write(AsteroidDestroyedVfx { x: info.x(), z: info.z() });

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start();

            outbox.0.push((Target::All, ServerMessage::AsteroidDestroyed { uuid: target_uuid.clone() }));
            outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
            return;
        }
    }

    beam.remaining_secs -= dt;
    if beam.remaining_secs <= 0.0 {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────

pub fn weapons_update_broadcaster() -> crate::core::broadcast::SimBroadcaster {
    crate::core::broadcast::SimBroadcaster::new().register(
        crate::core::broadcast::Audience::Holding(Console::Tactical),
        crate::core::broadcast::Cadence::Hz(10.0),
        |world: &mut World| {
            let ship = world.resource::<ShipState>();
            let world_res = world.resource::<WorldResource>();
            let weapons_target = world.resource::<WeaponsTarget>();
            let cooldown = world.resource::<PhaserCooldown>();
            let beam = world.resource::<ActiveBeam>();
            let torpedo_sys = world.resource::<TorpedoSystemResource>();
            let modifiers = world.resource::<crate::modifiers::ShipModifiers>();

            let effective_phaser_range = crate::radar::PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
            let fire_ready = match &weapons_target.0 {
                None => false,
                Some(uuid) => {
                    world_res.0.entities.iter()
                        .find(|a| &a.uuid == uuid)
                        .map(|a| crate::radar::is_fire_ready_with_range(a.x(), a.z(), ship.x, ship.z, ship.yaw, effective_phaser_range))
                        .unwrap_or(false)
                }
            };

            let ts = &torpedo_sys.0;
            vec![ServerMessage::WeaponsUpdate {
                target_uuid: weapons_target.0.clone(),
                fire_ready,
                on_cooldown: cooldown.is_active() || beam.target_uuid.is_some(),
                torpedo_count: ts.torpedoes_remaining,
                fore_port_loaded: ts.fore_port.is_loaded(),
                fore_port_reload_secs: ts.fore_port.reload_remaining,
                fore_starboard_loaded: ts.fore_starboard.is_loaded(),
                fore_starboard_reload_secs: ts.fore_starboard.reload_remaining,
                aft_loaded: ts.aft.is_loaded(),
                aft_reload_secs: ts.aft.reload_remaining,
            }]
        },
    )
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::HullIntegrity;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::{EntitySnapshot, WorldData, *};
    use crate::modifiers::ShipModifiers;
    use crate::simulation::{ShipHullIntegrity, ShipImpulse, SimOutbox};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .add_message::<AsteroidDestroyedVfx>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .insert_resource(ShipModifiers::new())
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())))
            .init_resource::<crate::console_ai_plugin::ConsoleComplexityState>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_plugins(WeaponsPlugin)
            .add_systems(Update, (
                tick_active_beam,
                tick_torpedo_system,
            ))
            .add_plugins(weapons_update_broadcaster())
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        app.world_mut().insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot::asteroid("target-uuid", asteroid_x, asteroid_z, 2.0)],
        }));
    }

    fn setup_weapons_world_with_entity(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> bevy::ecs::entity::Entity {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            crate::simulation::AsteroidDamage { max_hp: 30, current_hp: 30 },
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        )).id()
    }

    fn start_game_with_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        push(app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(app);
        push(app, "weapons", ClientMessage::FirePhaser);
        tick(app)
    }

    // ── SetTarget / TargetLock tests ───────────────────────────────────────

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 80.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "no-such-asteroid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    // ── WeaponsUpdate / fire_ready tests ───────────────────────────────────

    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(update.1, "expected fire_ready=true for in-range, forward-arc target");
    }

    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(!update.1, "expected fire_ready=false for beyond-phaser-range target");
    }

    // ── FirePhaser / beam lifecycle tests ──────────────────────────────────

    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out.iter().find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(beam_started.is_some(), "expected BeamStarted after firing at fire-ready target");
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid } => assert_eq!(target_uuid, "target-uuid"),
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = None;
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 3.0;

        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown");
    }

    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(&mut app, "captain", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser");
    }

    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc");
    }

    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();
        let asteroid_entity = app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            crate::simulation::AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );

        {
            let mut b = app.world_mut().resource_mut::<ActiveBeam>();
            b.damage_accumulator = 30.0;
            b.remaining_secs = 5.0;
        }

        let out = tick(&mut app);

        let destroyed = out.iter().find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(destroyed.is_some(), "expected AsteroidDestroyed when asteroid HP reaches 0");
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction");

        assert!(
            !app.world().resource::<WorldResource>().0.entities.iter().any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after beam end");

        assert!(app.world().get::<crate::simulation::AsteroidDamage>(asteroid_entity).is_none(),
            "asteroid entity should be despawned");
    }

    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after arc sever");
    }

    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        app.world_mut().resource_mut::<WorldResource>().0.entities[0].position = Some([0.0, 0.0, -50.0]);

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after range sever");
    }

    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        let asteroid_entity = app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            crate::simulation::AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 10.0;
        let _ = tick(&mut app);

        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app.world().get::<crate::simulation::AsteroidDamage>(asteroid_entity)
            .map(|d| d.current_hp);
        assert!(
            hp.is_some() && hp.unwrap() < 30,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();
        app.world_mut().insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![
                crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
        }));
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t1".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let _ = tick(&mut app);
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t1"));

        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 0.0;
        let _ = tick(&mut app);

        assert!(app.world().resource::<PhaserCooldown>().is_active());

        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 0.0;

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t2".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown");
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t2"));
    }

    // ── SetPhaserMode tests ────────────────────────────────────────────────

    #[test]
    fn weapons_console_can_set_phaser_mode_to_manual() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserMode { mode: crate::messages::PhaserMode::Manual });
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
        push(&mut app, "captain", ClientMessage::SetPhaserMode { mode: crate::messages::PhaserMode::Manual });
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

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForePort,
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube: crate::messages::TorpedoTube::ForePort, .. })),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
        );
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForePort,
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level.
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        push(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::Aft,
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should fire during Lobby when no SimSet gate is configured"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForeStarboard,
            target_uuid: None,
        });
        let out = tick(&mut app);

        let launched = out.iter().find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
            .expect("expected TorpedoLaunched");
        assert!(
            matches!(&launched.target, Target::All),
            "TorpedoLaunched should be broadcast to All, not {:?}", launched.target
        );
    }

    // ── ShipModifiers integration tests ────────────────────────────────────

    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        tick(&mut app);

        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world.0.entities.iter().find(|a| a.uuid == "target-uuid").map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::modifiers::{Modifier, ShipModifiers};
        use crate::messages::{ModifierSlot, ModifierSource};

        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        {
            let mut mods = app_fast.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0,
            });
        }
        start_game_with_weapons(&mut app_fast);
        push(&mut app_fast, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app_fast);
        push(&mut app_fast, "weapons", ClientMessage::FirePhaser);
        tick(&mut app_fast);

        {
            let mut beam = app_fast.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 2.0 * 3.5;
        }
        tick(&mut app_fast);

        let still_exists_fast = app_fast.world().resource::<WorldResource>()
            .0.entities.iter().any(|a| a.uuid == "target-uuid");
        assert!(!still_exists_fast, "with 2× phaser damage modifier, asteroid should be destroyed after 3.5s of beam");

        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(&mut app_base, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app_base);
        push(&mut app_base, "weapons", ClientMessage::FirePhaser);
        tick(&mut app_base);
        {
            let mut beam = app_base.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 1.0 * 3.5;
        }
        tick(&mut app_base);

        let still_exists_base = app_base.world().resource::<WorldResource>()
            .0.entities.iter().any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    // ── SetPhaserFrequency delegation tests ────────────────────────────────

    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(app);
        push(app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: 0.8 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.8).abs() < 1e-5, "Tactical holder should set phaser frequency to 0.8, got {freq}");
    }

    #[test]
    fn sensors_holder_can_set_phaser_frequency_when_tactical_is_low() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        app.world_mut()
            .resource_mut::<crate::console_ai_plugin::ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        push(&mut app, "sensors", ClientMessage::SetPhaserFrequency { frequency: 0.3 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.3).abs() < 1e-5, "Sensors holder should set phaser frequency when Tactical is Low, got {freq}");
    }

    #[test]
    fn sensors_holder_cannot_set_phaser_frequency_when_tactical_is_full() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        push(&mut app, "sensors", ClientMessage::SetPhaserFrequency { frequency: 0.9 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.5).abs() < 1e-5, "Sensors holder must NOT change phaser frequency when Tactical is Full, got {freq}");
    }

    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetPhaserFrequency { frequency: 0.9 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.5).abs() < 1e-5, "Captain must NOT change phaser frequency, got {freq}");
    }

    #[test]
    fn set_phaser_frequency_clamps_value() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: 1.5 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 1.0).abs() < 1e-5, "frequency above 1.0 should clamp to 1.0, got {freq}");

        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: -0.5 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.0).abs() < 1e-5, "frequency below 0.0 should clamp to 0.0, got {freq}");
    }
}
