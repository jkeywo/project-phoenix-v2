use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::asteroid_spawner::generate_donut_field;
use crate::breakdown::{breakdowns_from_damage, BreakdownQueue};
use crate::damage::{apply_damage_with_shields, collision_damage, HullIntegrity};
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions, Target, WorldResource};
use crate::shield::{attacker_bearing_relative, ShieldSystem};
use crate::map_config::MapConfig;
use crate::radar::WEAPONS_RADAR_RANGE;
use crate::radar::is_fire_ready;
use crate::messages::{
    AsteroidInfo, ClientMessage, Console, GamePhase, ServerMessage, ShieldFacingStatus, ViewMode,
};
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipState;
use crate::impulse::ImpulseState;

// ── Beam constants ────────────
const BEAM_DURATION_SECS: f32 = 6.0;
const BEAM_DAMAGE_PER_SEC: f32 = 5.0;
const BEAM_COOLDOWN_SECS: f32 = 6.0;

// ── Marker Components ────────
#[derive(Component)]
pub struct Ship;

#[derive(Component)]
pub struct Asteroid;

/// Stable UUID string identifying this asteroid entity (for targeting).
#[derive(Component, Clone)]
pub struct AsteroidUuid(pub String);

/// Tracks remaining HP for an asteroid entity (max and current = 30).
#[derive(Component)]
pub struct AsteroidDamage {
    pub max_hp: i32,
    pub current_hp: i32,
}

// ── Resources ────────────────
#[derive(Resource)]
struct SimBroadcastTimer(Timer);

#[derive(Resource)]
struct HelmInputTimer(Timer);

/// Ship-wide Hull Integrity (0–100). Tracked as a Bevy resource so systems
/// can read/write it independently of `ShipState`.
#[derive(Resource)]
pub struct ShipHullIntegrity(pub HullIntegrity);

/// The ship's shield system. Damage from collisions is routed through shields
/// first; only overflow passes through to the hull.
#[derive(Resource)]
pub struct ShipShields(pub ShieldSystem);

/// The ship's impulse drive state. Cancelled automatically when hull damage is taken.
#[derive(Resource)]
pub struct ShipImpulse(pub ImpulseState);

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
struct WorldSetupBroadcast {
    sent: bool,
}

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
}

/// Post-beam cooldown. The weapons console is locked out for `BEAM_COOLDOWN_SECS`
/// after every beam end (natural, sever, or cancel).
#[derive(Resource, Default)]
pub struct PhaserCooldown {
    pub remaining_secs: f32,
}

/// Current phaser firing mode (Auto or Manual), set by the Weapons console.
#[derive(Resource)]
pub struct CurrentPhaserMode(pub crate::messages::PhaserMode);

impl Default for CurrentPhaserMode {
    fn default() -> Self {
        Self(crate::messages::PhaserMode::Auto)
    }
}

// ── Repair constants ──────────
const REPAIR_DURATION_SECS: f32 = 30.0;
const REPAIR_HP_PER_SEC: f32 = 1.0 / 3.0; // +1 HP every 3 seconds
const REPAIR_MAX_HP: i32 = 10;
const REPAIR_PENALTY_SECS: f32 = 10.0;

/// Active repair action. `Some` while a repair is underway; the authorised
/// console is healing the ship. Resets to `None` when time expires or
/// `REPAIR_MAX_HP` is restored.
#[derive(Resource, Default)]
pub struct ActiveRepair {
    /// How many seconds remain in the current repair action.
    pub remaining_secs: f32,
    /// Fractional HP accumulator so 1 HP/3 s is applied accurately.
    pub hp_accumulator: f32,
    /// Total HP restored so far in this repair action (capped at REPAIR_MAX_HP).
    pub hp_restored: i32,
}

impl ActiveRepair {
    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }

    pub fn start(&mut self) {
        self.remaining_secs = REPAIR_DURATION_SECS;
        self.hp_accumulator = 0.0;
        self.hp_restored = 0;
    }
}

/// Per-token penalty cooldown. When a player presses Repair on an unauthorised
/// console, their token is entered here and locked out for `REPAIR_PENALTY_SECS`.
#[derive(Resource, Default)]
pub struct RepairPenalties(pub std::collections::HashMap<String, f32>);

impl RepairPenalties {
    pub fn is_penalised(&self, token: &str) -> bool {
        self.0.get(token).map_or(false, |&secs| secs > 0.0)
    }

    pub fn penalise(&mut self, token: &str) {
        self.0.insert(token.to_string(), REPAIR_PENALTY_SECS);
    }

    pub fn tick(&mut self, dt: f32) {
        for secs in self.0.values_mut() {
            *secs = (*secs - dt).max(0.0);
        }
    }

    pub fn remaining(&self, token: &str) -> f32 {
        self.0.get(token).copied().unwrap_or(0.0)
    }
}

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

/// Bevy resource wrapping the breakdown queue.
#[derive(Resource)]
pub struct BreakdownQueueResource {
    pub queue: BreakdownQueue,
    /// Cumulative damage taken since game start (tracks 10-HP bucket crossings).
    pub cumulative_damage: i32,
    rng: rand::rngs::SmallRng,
}

impl Default for BreakdownQueueResource {
    fn default() -> Self {
        use rand::SeedableRng as _;
        Self {
            queue: BreakdownQueue::new(),
            cumulative_damage: 0,
            rng: rand::rngs::SmallRng::from_os_rng(),
        }
    }
}

/// Remembers the most recent helm input so the 10 Hz physics tick can
/// keep applying it even when no new client message has arrived that tick.
#[derive(Resource, Default)]
struct LastHelmInput {
    thrust: f32,
    steering: f32,
}

/// Prevents `handle_collisions` from applying damage every frame while the
/// ship is in contact. After damage is applied once, a 1-second cooldown
/// suppresses further hits until the ship clears the obstacle.
#[derive(Resource, Default)]
struct CollisionCooldown {
    remaining_secs: f32,
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

// ── Plugin ───────────────────
pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RapierPhysicsPlugin::<()>::default())
            .add_message::<AsteroidDestroyedVfx>()
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(ImpulseState::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WorldSetupBroadcast>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<ActiveRepair>()
            .init_resource::<RepairPenalties>()
            .init_resource::<BreakdownQueueResource>()
            .init_resource::<LastHelmInput>()
            .init_resource::<CollisionCooldown>()
            .insert_resource(SimBroadcastTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .insert_resource(HelmInputTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_systems(Startup, setup_world)
            .add_systems(Update, (
                handle_toggle,
                handle_set_view,
                handle_set_target,
                handle_set_science_target,
                handle_fire_phaser,
                handle_set_phaser_mode,
                handle_repair,
                handle_impulse_messages,
                tick_active_beam,
                tick_repair,
                tick_shields,
                process_helm_inputs,
                sync_ship_position,
                handle_collisions,
                broadcast_sim_state,
                broadcast_weapons_update.after(broadcast_sim_state),
                broadcast_repair_state.after(broadcast_sim_state),
                broadcast_shield_status.after(broadcast_sim_state),
                broadcast_world_setup_on_start.after(crate::lobby::process_lobby),
            ));
    }
}

// ── Systems ──────────────────
fn handle_toggle(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if matches!(ev.msg, ClientMessage::ToggleRedAlert)
            && sessions.0.console_holder(Console::CaptainChair) == Some(ev.token.as_str())
        {
            ship.toggle_red_alert();
        }
    }
}

fn handle_set_view(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if let ClientMessage::SetView { mode } = ev.msg.clone() {
            // Authorization is per-variant: Camera views are the captain's call,
            // Radar is the helm's call. A request from the wrong console is
            // silently ignored.
            let required = match &mode {
                ViewMode::Camera(_) => Console::CaptainChair,
                ViewMode::Radar => Console::Helm,
                ViewMode::ScienceRadar | ViewMode::SystemChart => Console::Science,
            };
            if sessions.0.console_holder(required) == Some(ev.token.as_str()) {
                ship.view_mode = mode;
            }
        }
    }
}

fn handle_set_target(
    mut reader: MessageReader<InboundMessage>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    mut weapons_target: ResMut<WeaponsTarget>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetTarget { uuid } = &ev.msg else { continue };

        // Only the Weapons console holder may lock a target.
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }

        // Validate: asteroid must exist in world data and be within WEAPONS_RADAR_RANGE.
        let asteroid = world.0.asteroids.iter().find(|a| &a.uuid == uuid);
        let locked = match asteroid {
            None => false,
            Some(a) => {
                let dx = a.x - ship.x;
                let dz = a.z - ship.z;
                dx * dx + dz * dz <= WEAPONS_RADAR_RANGE * WEAPONS_RADAR_RANGE
            }
        };

        if locked {
            weapons_target.0 = Some(uuid.clone());
        } else {
            // Rejection clears the visual lock.
            weapons_target.0 = None;
        }

        writer.write(OutboundMessage {
            target: Target::Token(ev.token.clone()),
            msg: ServerMessage::TargetLock { uuid: uuid.clone(), locked },
        });
    }
}

fn handle_set_science_target(
    mut reader: MessageReader<InboundMessage>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetScienceTarget { uuid } = &ev.msg else { continue };

        // Only the Science console holder may broadcast a target suggestion.
        if sessions.0.console_holder(Console::Science) != Some(ev.token.as_str()) {
            continue;
        }

        // Only broadcast if there is a Weapons console player to receive it.
        let Some(weapons_token) = sessions.0.console_holder(Console::Tactical) else {
            continue;
        };

        writer.write(OutboundMessage {
            target: Target::Token(weapons_token.to_string()),
            msg: ServerMessage::ScienceTargetSuggestion { uuid: uuid.clone() },
        });
    }
}

fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship: ResMut<ShipState>,
    phase: Res<CurrentPhase>,
    mut last_input: ResMut<LastHelmInput>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    // Only process if helm is occupied
    let helm_token = sessions.0.console_holder(Console::Helm);
    if helm_token.is_none() {
        return;
    }

    // Update the stored input from any messages that arrived this tick.
    // If none arrived, last_input retains its previous values so a steady
    // joystick position keeps applying thrust rather than decelerating.
    for ev in reader.read() {
        if ev.token != helm_token.unwrap() {
            continue;
        }
        if let ClientMessage::HelmInput { thrust: t, steering: s } = ev.msg {
            last_input.thrust = t;
            last_input.steering = s;
        }
    }

    // Compute physics — use the timer's nominal period, not the frame delta.
    // The timer fires every 100 ms; time.delta_secs() is only one frame (~16 ms).
    let dt = timer.0.duration().as_secs_f32();
    let state = ShipPhysicsState {
        x: ship.x,
        z: ship.z,
        yaw: ship.yaw,
        forward_speed: ship.forward_speed,
    };
    let input = ShipPhysicsInput { thrust: last_input.thrust, steering: last_input.steering };
    let config = ShipPhysicsConfig::new();
    let result = compute_physics(state, input, dt, &config);

    ship.x = result.x;
    ship.z = result.z;
    ship.yaw = result.yaw;
    ship.forward_speed = result.forward_speed;
}

fn sync_ship_position(
    ship: Res<ShipState>,
    mut ship_query: Query<&mut Transform, With<Ship>>,
) {
    let Ok(mut transform) = ship_query.single_mut() else {
        return;
    };

    transform.translation.x = ship.x;
    transform.translation.z = ship.z;
    transform.rotation = Quat::from_axis_angle(Vec3::Y, ship.yaw);
}

fn handle_collisions(
    time: Res<Time>,
    context: ReadRapierContext,
    ship_query: Query<Entity, With<Ship>>,
    asteroid_query: Query<(&Transform, &AsteroidUuid), With<Asteroid>>,
    mut ship: ResMut<ShipState>,
    mut hull: ResMut<ShipHullIntegrity>,
    mut shields: ResMut<ShipShields>,
    mut breakdowns: ResMut<BreakdownQueueResource>,
    mut cooldown: ResMut<CollisionCooldown>,
) {
    let dt = time.delta_secs();
    cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);

    let Ok(ctx) = context.single() else { return };
    let Ok(ship_entity) = ship_query.single() else { return };

    // Collect the first contact partner entity (if any).
    let contact = ctx.contact_pairs_with(ship_entity).next().map(|pair| {
        if pair.collider1() == Some(ship_entity) { pair.collider2() } else { pair.collider1() }
    }).flatten();

    if contact.is_some() {
        // Only apply damage once per contact event; skip while immune.
        if cooldown.remaining_secs > 0.0 {
            return;
        }
        let max_speed = ShipPhysicsConfig::new().max_speed;
        let damage = collision_damage(ship.forward_speed, max_speed);

        // Determine which shield facing absorbs the hit by finding the
        // attacker's world-space position and computing its bearing relative
        // to the ship's current yaw.
        let bearing = contact
            .and_then(|attacker_entity| {
                asteroid_query.get(attacker_entity).ok().map(|(t, _)| {
                    attacker_bearing_relative(
                        t.translation.x,
                        t.translation.z,
                        ship.x,
                        ship.z,
                        ship.yaw,
                    )
                })
            })
            .unwrap_or(0.0); // fallback: treat as fore hit

        // Route damage: shields absorb first, overflow goes to hull.
        let hull_damage = apply_damage_with_shields(damage, bearing, &mut shields.0, &mut hull.0);
        if hull_damage > 0 {
            let before = breakdowns.cumulative_damage;
            breakdowns.cumulative_damage += hull_damage;
            let new_count = breakdowns_from_damage(before, breakdowns.cumulative_damage);
            let BreakdownQueueResource { queue, rng, .. } = &mut *breakdowns;
            for _ in 0..new_count {
                queue.push_random(rng);
            }
        }
        ship.forward_speed = 0.0;
        cooldown.remaining_secs = 1.0;
    }
}

/// Tick shield regen and offline timers each frame.
fn tick_shields(time: Res<Time>, mut shields: ResMut<ShipShields>) {
    shields.0.tick(time.delta_secs());
}

/// Handle `FirePhaser` messages from the Weapons console.
///
/// Validates: sender is Weapons holder, game is in-progress, no active cooldown,
/// a locked target exists, and that target is currently fire-ready.
/// On success, starts a new beam (cancelling any active beam first) and broadcasts
/// `BeamStarted` to all players.
fn handle_fire_phaser(
    mut reader: MessageReader<InboundMessage>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    cooldown: ResMut<PhaserCooldown>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if !matches!(ev.msg, ClientMessage::FirePhaser) {
            continue;
        }
        // Only the Weapons console holder may fire.
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        // Reject if on cooldown or a beam is already active.
        if cooldown.is_active() || beam.target_uuid.is_some() {
            continue;
        }
        // Need a locked target.
        let Some(target_uuid) = &weapons_target.0 else { continue };
        // Target must still exist in world data and be fire-ready.
        let Some(asteroid) = world.0.asteroids.iter().find(|a| &a.uuid == target_uuid) else {
            continue;
        };
        if !is_fire_ready(asteroid.x, asteroid.z, ship.x, ship.z, ship.yaw) {
            continue;
        }

        // If another beam was active (shouldn't happen with cooldown enforcement,
        // but guard defensively), end it first.
        if let Some(old_uuid) = beam.target_uuid.take() {
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            writer.write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::BeamEnded { target_uuid: old_uuid },
            });
        }

        // Start new beam.
        beam.target_uuid = Some(target_uuid.clone());
        beam.remaining_secs = BEAM_DURATION_SECS;
        beam.damage_accumulator = 0.0;

        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamStarted { target_uuid: target_uuid.clone() },
        });
    }
}

/// Handle `SetPhaserMode` messages from the Weapons console.
///
/// Only the Tactical console holder may change the phaser mode.
fn handle_set_phaser_mode(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetPhaserMode { mode } = &ev.msg else { continue };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        phaser_mode.0 = *mode;
    }
}

/// Handle `Repair` messages from any console player.
///
/// Validates: game is in-progress, sender holds a console.
/// - If the sender's console is the `authorized_repair_console` and no repair is
///   already active and they are not penalised: start a 30-second repair action.
/// - Otherwise: apply a 30-second penalty cooldown to that player (ignored if
///   they are already penalised or a repair is in progress for them).
fn handle_repair(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    breakdowns: Res<BreakdownQueueResource>,
    mut repair: ResMut<ActiveRepair>,
    mut penalties: ResMut<RepairPenalties>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let repair_console = match &ev.msg {
            ClientMessage::Repair { console } => console.clone(),
            _ => continue,
        };
        let token = ev.token.as_str();
        // Sender must actually hold the console they claim to be pressing.
        let sender_consoles = sessions.0.players()
            .iter()
            .find(|p| p.token == token)
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        if !sender_consoles.contains(&repair_console) {
            continue;
        }
        // Ignore presses during an active penalty for this player.
        if penalties.is_penalised(token) {
            continue;
        }
        // Check if the specific console pressed is the authorized one.
        let authorized = breakdowns.queue.front().cloned();
        let is_authorized = authorized.as_ref().map_or(false, |auth| *auth == repair_console);

        if is_authorized && !repair.is_active() {
            repair.start();
        } else if !is_authorized && authorized.is_some() {
            penalties.penalise(token);
        }
        // No breakdowns pending, or repair already active → silent no-op.
    }
}

/// Handle `StartImpulseCharge` and `CancelImpulse` messages from helm/science.
/// Also cancels impulse whenever the hull takes damage this frame.
fn handle_impulse_messages(
    mut reader: MessageReader<InboundMessage>,
    mut impulse: ResMut<ShipImpulse>,
    phase: Res<CurrentPhase>,
    hull: Res<ShipHullIntegrity>,
    mut last_hull_hp: Local<i32>,
) {
    // Initialise on first call.
    if *last_hull_hp == 0 && hull.0.current() == 100 {
        *last_hull_hp = 100;
    }

    // Cancel impulse if hull HP decreased since last frame.
    let current_hp = hull.0.current();
    if current_hp < *last_hull_hp {
        impulse.0.cancel_charge();
    }
    *last_hull_hp = current_hp;

    if phase.0 != GamePhase::InProgress {
        return;
    }

    for msg in reader.read() {
        match &msg.msg {
            ClientMessage::StartImpulseCharge => {
                impulse.0.start_charge();
            }
            ClientMessage::CancelImpulse => {
                impulse.0.cancel_charge();
            }
            _ => {}
        }
    }
}

/// Tick the active repair each frame: restore HP, advance breakdown queue on
/// completion.
fn tick_repair(
    time: Res<Time>,
    mut repair: ResMut<ActiveRepair>,
    mut penalties: ResMut<RepairPenalties>,
    mut hull: ResMut<ShipHullIntegrity>,
    mut breakdowns: ResMut<BreakdownQueueResource>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    // Tick per-token penalty timers.
    penalties.tick(dt);

    if !repair.is_active() {
        return;
    }

    repair.hp_accumulator += REPAIR_HP_PER_SEC * dt;
    let hp_to_apply = repair.hp_accumulator.floor() as i32;
    if hp_to_apply > 0 {
        repair.hp_accumulator -= hp_to_apply as f32;
        let remaining_budget = REPAIR_MAX_HP - repair.hp_restored;
        let actual = hp_to_apply.min(remaining_budget);
        if actual > 0 {
            hull.0.restore(actual);
            repair.hp_restored += actual;
        }
    }

    repair.remaining_secs -= dt;
    if repair.remaining_secs <= 0.0 || repair.hp_restored >= REPAIR_MAX_HP {
        // Repair complete — advance the breakdown queue.
        breakdowns.queue.pop_front();
        repair.remaining_secs = 0.0;
        repair.hp_accumulator = 0.0;
        repair.hp_restored = 0;
    }
}

/// Broadcast `RepairState` at 10 Hz to every console holder.
fn broadcast_repair_state(
    timer: Res<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    repair: Res<ActiveRepair>,
    penalties: Res<RepairPenalties>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.just_finished() {
        return;
    }

    use crate::messages::Console;
    let all_consoles = [Console::CaptainChair, Console::Helm, Console::Tactical, Console::Engineering];
    for console in &all_consoles {
        let Some(token) = sessions.0.console_holder(console.clone()) else { continue };
        let penalty_remaining = penalties.remaining(token);
        let (remaining_cooldown_secs, in_progress, penalty) = if repair.is_active() {
            (repair.remaining_secs, true, false)
        } else if penalty_remaining > 0.0 {
            (penalty_remaining, false, true)
        } else {
            (0.0, false, false)
        };
        writer.write(OutboundMessage {
            target: Target::Token(token.to_string()),
            msg: ServerMessage::RepairState { remaining_cooldown_secs, in_progress, penalty },
        });
    }
}

/// Tick the active beam each frame: apply damage, check sever conditions
/// (arc, range, target destroyed), and handle natural expiry.
///
/// When the beam ends (any cause), starts the post-beam cooldown and broadcasts
/// `BeamEnded`. If the target asteroid reaches 0 HP, also broadcasts
/// `AsteroidDestroyed` and removes it from `WorldData`.
fn tick_active_beam(
    time: Res<Time>,
    mut beam: ResMut<ActiveBeam>,
    mut cooldown: ResMut<PhaserCooldown>,
    mut writer: MessageWriter<OutboundMessage>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    ship: Res<ShipState>,
    mut world: ResMut<WorldResource>,
    mut asteroid_query: Query<(Entity, &AsteroidUuid, &mut AsteroidDamage)>,
    mut commands: Commands,
    mut destroyed_asteroids: ResMut<crate::asteroid_lifecycle::DestroyedAsteroids>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    let dt = time.delta_secs();

    // Tick cooldown regardless of beam state.
    cooldown.tick(dt);

    let Some(target_uuid) = beam.target_uuid.clone() else {
        return;
    };

    // Check sever: target no longer exists in world data.
    let asteroid_info = world.0.asteroids.iter().find(|a| a.uuid == target_uuid).cloned();
    let Some(info) = asteroid_info else {
        // Target was already destroyed (e.g., double-tick race). End beam silently.
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamEnded { target_uuid },
        });
        return;
    };

    // Check sever: out of range or out of arc.
    if !is_fire_ready(info.x, info.z, ship.x, ship.z, ship.yaw) {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamEnded { target_uuid },
        });
        return;
    }

    // Apply damage proportionally to elapsed time.
    beam.damage_accumulator += BEAM_DAMAGE_PER_SEC * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        // Find the asteroid entity and apply damage.
        let mut destroyed = false;
        for (entity, uuid_comp, mut dmg) in asteroid_query.iter_mut() {
            if uuid_comp.0 == target_uuid {
                dmg.current_hp = (dmg.current_hp - damage_to_apply).max(0);
                if dmg.current_hp == 0 {
                    destroyed = true;
                    commands.entity(entity).despawn();
                }
                break;
            }
        }

 if destroyed {
 // Add to destroyed set to prevent respawning
 destroyed_asteroids.0.insert(target_uuid.clone());

 // Remove from world data.
 world.0.asteroids.retain(|a| a.uuid != target_uuid);

 // Fire VFX event with the asteroid's last known position so the
 // renderer can play the destruction ripple. `info` holds the position
 // captured before the retain() call above.
 vfx_events.write(AsteroidDestroyedVfx { x: info.x, z: info.z });

 beam.target_uuid = None;
 beam.remaining_secs = 0.0;
 beam.damage_accumulator = 0.0;
 cooldown.start();

 writer.write(OutboundMessage {
 target: Target::All,
 msg: ServerMessage::AsteroidDestroyed { uuid: target_uuid.clone() },
 });
 writer.write(OutboundMessage {
 target: Target::All,
 msg: ServerMessage::BeamEnded { target_uuid },
 });
 return;
        }
    }

    // Tick beam duration.
    beam.remaining_secs -= dt;
    if beam.remaining_secs <= 0.0 {
        // Natural expiry.
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamEnded { target_uuid },
        });
    }
}

fn broadcast_sim_state(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    ship: Res<ShipState>,
    hull: Res<ShipHullIntegrity>,
    breakdowns: Res<BreakdownQueueResource>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if timer.0.tick(time.delta()).just_finished() {
        let authorized = breakdowns.queue.front().cloned();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::SimState { snapshot: ship.snapshot(hull.0.current(), authorized) },
        });
    }
}

/// Broadcast `ShieldStatus` to all players at 10 Hz.
fn broadcast_shield_status(
    timer: Res<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    shields: Res<ShipShields>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.just_finished() {
        return;
    }
    let facings = shields.0.snapshot().into_iter().map(|s| ShieldFacingStatus {
        label: s.label,
        hp: s.hp,
        max_hp: s.max_hp,
        online: s.online,
        offline_remaining: s.offline_remaining,
    }).collect();
    writer.write(OutboundMessage {
        target: Target::All,
        msg: ServerMessage::ShieldStatus { facings },
    });
}

/// Broadcast `WeaponsUpdate` to the Weapons console player at 10 Hz.
///
/// Reuses `SimBroadcastTimer`; after the timer ticks in `broadcast_sim_state`
/// the `just_finished()` flag is still `true` for the remainder of the frame
/// because `Repeating` timers latch it until the next `tick`.
fn broadcast_weapons_update(
    timer: Res<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    weapons_target: Res<WeaponsTarget>,
    cooldown: Res<PhaserCooldown>,
    beam: Res<ActiveBeam>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.just_finished() {
        return;
    }
    let Some(weapons_token) = sessions.0.console_holder(Console::Tactical) else {
        return;
    };

    let fire_ready = match &weapons_target.0 {
        None => false,
        Some(uuid) => {
            world.0.asteroids.iter()
                .find(|a| &a.uuid == uuid)
                .map(|a| is_fire_ready(a.x, a.z, ship.x, ship.z, ship.yaw))
                .unwrap_or(false)
        }
    };

    writer.write(OutboundMessage {
        target: Target::Token(weapons_token.to_string()),
        msg: ServerMessage::WeaponsUpdate {
            target_uuid: weapons_target.0.clone(),
            fire_ready,
            on_cooldown: cooldown.is_active() || beam.target_uuid.is_some(),
        },
    });
}

/// Emit a single `WorldSetup` broadcast the first frame the game enters
/// `InProgress`. Stays silent in Lobby and on subsequent in-game ticks.
fn broadcast_world_setup_on_start(
    mut writer: MessageWriter<OutboundMessage>,
    world: Res<WorldResource>,
    phase: Res<CurrentPhase>,
    mut state: ResMut<WorldSetupBroadcast>,
) {
    if phase.0 != GamePhase::InProgress || state.sent {
        return;
    }
    writer.write(OutboundMessage {
        target: Target::All,
        msg: ServerMessage::WorldSetup { world: world.0.clone() },
    });
    state.sent = true;
}

// ── World Setup ──────────────
fn setup_world(
    commands: Commands,
    meshes: ResMut<Assets<Mesh>>,
    materials: ResMut<Assets<StandardMaterial>>,
    world: ResMut<WorldResource>,
) {
    // Try to get the preloaded map config and config cache
    if let Some(map_config) = crate::config_cache::get_map_config() {
        let config_cache = crate::config_cache::get_config_cache();
        setup_world_from_config(commands, meshes, materials, world, map_config, config_cache);
    } else {
        // Fallback: use hardcoded behavior
        setup_world_hardcoded(commands, meshes, materials, world);
    }
}

fn setup_world_from_config(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut world: ResMut<WorldResource>,
    map_config: MapConfig,
    config_cache: crate::config_cache::ConfigCache,
) {
    
    // Spawn stars as unlit emissive sphere meshes
    for star in &map_config.stars {
        let star_mesh = meshes.add(Sphere { radius: star.radius });
        // Convert RGB vec to Color
        let star_color = Color::srgb(star.colour[0], star.colour[1], star.colour[2]);
        let star_mat = materials.add(StandardMaterial {
            base_color: star_color,
            emissive: LinearRgba::from(star_color) * 2.0,
            ..default()
        });
        commands.spawn((
            Mesh3d(star_mesh),
            MeshMaterial3d(star_mat),
            Transform::from_xyz(star.position[0], star.position[1], star.position[2]),
        ));
    }
    
    // Spawn planets as standard lit sphere meshes
    for planet in &map_config.planets {
        let planet_mesh = meshes.add(Sphere { radius: planet.radius });
        let planet_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(planet.colour[0], planet.colour[1], planet.colour[2]),
            ..default()
        });
        commands.spawn((
            Mesh3d(planet_mesh),
            MeshMaterial3d(planet_mat),
            Transform::from_xyz(planet.position[0], planet.position[1], planet.position[2]),
        ));
    }
    
    // Spawn starfield skybox
    // Procedural points: many small unlit white spheres at radius ~2000
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        unlit: true,
        ..default()
    });
    let star_mesh = meshes.add(Sphere { radius: 1.0 });
    let star_count = 400u32;
    let radius = 2000.0_f32;
    for i in 0..star_count {
        // Deterministic pseudo-random unit vector via golden-spiral on a sphere.
        let frac = (i as f32 + 0.5) / star_count as f32;
        let phi = (1.0 - 2.0 * frac).acos();
        let theta = std::f32::consts::PI * (1.0 + 5_f32.sqrt()) * i as f32;
        let x = phi.sin() * theta.cos() * radius;
        let y = phi.sin() * theta.sin() * radius;
        let z = phi.cos() * radius;
        // Hash for size variation
        let h = ((i.wrapping_mul(2654435761)) ^ 0xDEADBEEF) % 100;
        let scale = 1.5 + (h as f32) / 25.0; // 1.5..5.5
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_xyz(x, y, z).with_scale(Vec3::splat(scale)),
        ));
    }
    
    // Spawn player ship from EntityConfig
    if let Some(ship_config) = config_cache.get("assets/entities/player_ship.toml") {
        // Get collider config
        let collider_radius = ship_config.collider.as_ref().map(|c| c.radius).unwrap_or(3.0);
        let collider_half_height = ship_config.collider.as_ref().map(|c| c.length / 2.0).unwrap_or(3.0);
        
            // Get hull integrity
            let hull_integrity = ship_config.hull.as_ref().map(|h| h.hull_integrity).unwrap_or(100);
            
            // Spawn ship - store hull integrity in resource
            commands.insert_resource(ShipHullIntegrity(HullIntegrity::with_hp(hull_integrity)));
        
        commands.spawn((
            Ship,
            Transform::default(),
            RigidBody::KinematicPositionBased,
            Collider::capsule_y(collider_half_height, collider_radius),
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));
    } else {
        // Fallback if no ship config
        // Fallback hull integrity
        commands.insert_resource(ShipHullIntegrity(HullIntegrity::new()));
        
        commands.spawn((
            
            Ship,
            Transform::default(),
            RigidBody::KinematicPositionBased,
            Collider::capsule_y(3.0, 6.0),
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));
    }
    
    // Spawn asteroids from asteroid fields
    let mut all_asteroid_infos = Vec::new();
    
    for (field_idx, field) in map_config.asteroid_fields.iter().enumerate() {
        let donut_result = generate_donut_field(
            field.inner_radius,
            field.outer_radius,
            field.density,
            field_idx as u64,
            &field.asteroid_type_paths,
            &field.cosmetic_type_paths,
        );
        
        // Generate UUIDs for this field
        let uuids = crate::asteroid_spawner::generate_donut_uuids(
            field.inner_radius,
            field.outer_radius,
            field.density,
            field_idx as u64,
            donut_result.spawns.len(),
        );
        
        // Spawn gameplay asteroids
        let asteroid_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.4, 0.35, 0.3),
            ..default()
        });
        let asteroid_mesh = meshes.add(Sphere { radius: 2.0 });
        
        let mut gameplay_count = 0;
        for spawn in &donut_result.spawns {
            // Check if this is a gameplay type
            if field.asteroid_type_paths.contains(&spawn.config_path) {
                gameplay_count += 1;
                commands.spawn((
                    Asteroid,
                    AsteroidUuid(uuids[gameplay_count - 1].clone()),
                    AsteroidDamage { max_hp: 30, current_hp: 30 },
                    Mesh3d(asteroid_mesh.clone()),
                    MeshMaterial3d(asteroid_mat.clone()),
                    Transform::from_xyz(spawn.x, 0.0, spawn.z),
                    Collider::ball(2.0),
                    RigidBody::Fixed,
                ));
                
                all_asteroid_infos.push(AsteroidInfo {
                    uuid: uuids[gameplay_count - 1].clone(),
                    x: spawn.x,
                    z: spawn.z,
                    radius: 2.0,
                    tags: vec!["asteroid".to_string()],
                });
            }
        }
        
        // Spawn cosmetic asteroids above/below the play plane
        let cosmetic_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.35, 0.3, 0.28),
            perceptual_roughness: 0.95,
            ..default()
        });
        
        let mut cosmetic_start_idx = gameplay_count;
        for (i, spawn) in donut_result.spawns.iter().enumerate() {
            if field.cosmetic_type_paths.contains(&spawn.config_path) {
                let idx = i;
                let h = ((idx as u32).wrapping_mul(2654435761)) ^ 0x9E3779B9;
                let above = (h & 1) == 0;
                let mag = 10.0 + ((h >> 1) % 5000) as f32 / 100.0; // 10..60
                let y = if above { mag } else { -mag };
                let radius = 0.5 + ((h >> 13) % 250) as f32 / 100.0; // 0.5..3.0
                let mesh = meshes.add(Sphere { radius });
                commands.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(cosmetic_mat.clone()),
                    Transform::from_xyz(spawn.x, y, spawn.z),
                ));
                
                all_asteroid_infos.push(AsteroidInfo {
                    uuid: uuids[cosmetic_start_idx].clone(),
                    x: spawn.x,
                    z: spawn.z,
                    radius,
                    tags: vec!["asteroid".to_string()],
                });
                cosmetic_start_idx += 1;
            }
        }
    }
    
    // Record asteroid layout
    world.0.asteroids = all_asteroid_infos;
}

/// Fallback world setup with hardcoded values for development/testing
fn setup_world_hardcoded(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    _world: ResMut<WorldResource>,
) {

    // ── Starfield skybox ───────────────────────────────────────────────
    // Procedural points: many small unlit white spheres at radius ~2000
    // around the origin. Cheap and works on WebGL2.
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        unlit: true,
        ..default()
    });
    let star_mesh = meshes.add(Sphere { radius: 1.0 });
    let star_count = 400u32;
    let radius = 2000.0_f32;
    for i in 0..star_count {
        // Deterministic pseudo-random unit vector via golden-spiral on a sphere.
        let frac = (i as f32 + 0.5) / star_count as f32;
        let phi = (1.0 - 2.0 * frac).acos();
        let theta = std::f32::consts::PI * (1.0 + 5_f32.sqrt()) * i as f32;
        let x = phi.sin() * theta.cos() * radius;
        let y = phi.sin() * theta.sin() * radius;
        let z = phi.cos() * radius;
        // Hash for size variation
        let h = ((i.wrapping_mul(2654435761)) ^ 0xDEADBEEF) % 100;
        let scale = 1.5 + (h as f32) / 25.0; // 1.5..5.5
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_xyz(x, y, z).with_scale(Vec3::splat(scale)),
        ));
    }

    // Spawn ship — kinematic so we drive position directly from ShipState;
    // collision events fire so handle_collisions can zero velocity on impact.
    commands.spawn((
        Ship,
        Transform::default(),
        RigidBody::KinematicPositionBased,
        Collider::capsule_y(3.0, 6.0),
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));
}

// ── Tests ────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, InboundMessage, OutboundMessage};
    use crate::messages::*;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

fn test_app() -> App {
    let mut app = App::new();
    // Use a 1-nanosecond timer so that any non-zero time delta finishes
    // the broadcast cycle, letting tests observe the snapshot after a
    // couple of update ticks.
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .insert_resource(ShipState::new())
        .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
        .insert_resource(ShipShields(ShieldSystem::default()))
        .insert_resource(ShipImpulse(ImpulseState::new()))
        .init_resource::<WorldResource>()
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<WeaponsTarget>()
        .init_resource::<ActiveBeam>()
        .add_message::<AsteroidDestroyedVfx>()
        .init_resource::<PhaserCooldown>()
        .init_resource::<CurrentPhaserMode>()
        .init_resource::<ActiveRepair>()
        .init_resource::<RepairPenalties>()
        .init_resource::<BreakdownQueueResource>()
        .init_resource::<crate::asteroid_lifecycle::DestroyedAsteroids>()
        .insert_resource(SimBroadcastTimer(Timer::new(
            std::time::Duration::from_nanos(1), TimerMode::Repeating)))
        .init_resource::<Outbox>()
        .add_systems(Update, (handle_set_view, handle_set_target, handle_set_science_target, handle_fire_phaser, handle_set_phaser_mode, handle_repair, handle_impulse_messages, tick_active_beam, tick_repair, broadcast_sim_state, broadcast_weapons_update.after(broadcast_sim_state), broadcast_repair_state.after(broadcast_sim_state), broadcast_shield_status.after(broadcast_sim_state), broadcast_world_setup_on_start.after(crate::lobby::process_lobby)))
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
        let msgs = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    fn start_game(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn set_view_during_lobby_is_ignored() {
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(&mut app);
        // Still in Lobby — game not started
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn non_captain_set_view_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "crew", ClientMessage::Identify { token: "crew".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "crew", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn captain_set_view_changes_direction() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
        );
    }

    fn start_game_with_helm(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "helm", ClientMessage::Identify { token: "helm".into(), name: "Bob".into() });
        tick(app);
        push(app, "helm", ClientMessage::SelectConsole { console: Console::Helm });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn start_game_with_science(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "science", ClientMessage::Identify { token: "science".into(), name: "Spock".into() });
        tick(app);
        push(app, "science", ClientMessage::SelectConsole { console: Console::Science });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn science_can_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_science(&mut app);
        push(&mut app, "science", ClientMessage::SetView { mode: ViewMode::ScienceRadar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::ScienceRadar
        );
    }

    #[test]
    fn science_can_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_science(&mut app);
        push(&mut app, "science", ClientMessage::SetView { mode: ViewMode::SystemChart });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::SystemChart
        );
    }

    #[test]
    fn non_science_cannot_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_science(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::ScienceRadar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn helm_can_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
    }

    #[test]
    fn captain_cannot_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Captain has no authority over Radar; request is silently dropped.
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn helm_cannot_switch_view_to_camera() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn sim_state_broadcast_carries_ship_position_and_view_mode() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Move the ship and switch to radar
        {
            let mut ship = app.world_mut().resource_mut::<ShipState>();
            ship.x = 12.0;
            ship.z = -3.5;
            ship.yaw = 1.25;
        }
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        // Ensure Time has accumulated some real delta and the broadcast fires.
        // Two prior ticks have already advanced TimePlugin's clock; a fresh
        // tick now sees a non-zero delta, finishing the 1-ns broadcast timer.
        let out = tick(&mut app);

        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");

        assert_eq!(snap.ship_x, 12.0);
        assert_eq!(snap.ship_z, -3.5);
        assert_eq!(snap.ship_yaw, 1.25);
        assert_eq!(snap.view_mode, ViewMode::Radar);
    }

    #[test]
    fn world_setup_is_broadcast_once_after_start_game() {
        let mut app = test_app();
        // Pre-populate world data so the broadcast has something to emit.
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![AsteroidInfo { uuid: "test-uuid".into(), x: 5.0, z: -1.0, radius: 2.0, tags: vec![] }],
            asteroid_fields: vec![],
        }));

        // Bring the game up to the point of pressing StartGame
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "A".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(&mut app);
        // The StartGame tick should produce the WorldSetup broadcast
        push(&mut app, "captain", ClientMessage::StartGame);
        let start_out = tick(&mut app);

        let world_setups: Vec<_> = start_out.iter().filter(|m|
            matches!(&m.msg, ServerMessage::WorldSetup { .. })
        ).collect();
        assert_eq!(world_setups.len(), 1, "expected exactly one WorldSetup on the StartGame tick");
        match &world_setups[0].msg {
            ServerMessage::WorldSetup { world } => {
                assert_eq!(world.asteroids.len(), 1);
                assert_eq!(world.asteroids[0].x, 5.0);
            }
            _ => unreachable!(),
        }
        match &world_setups[0].target {
            crate::lobby::Target::All => {}
            t => panic!("WorldSetup should target All, got {:?}", t),
        }

        // Subsequent ticks must not re-broadcast WorldSetup
        let later = tick(&mut app);
        assert!(!later.iter().any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should only fire once per game");
    }

    #[test]
    fn world_setup_is_not_broadcast_during_lobby() {
        let mut app = test_app();
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![AsteroidInfo { uuid: "test-uuid".into(), x: 0.0, z: 0.0, radius: 2.0, tags: vec![] }],
            asteroid_fields: vec![],
        }));
        // Identify and select a console but don't start the game.
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "A".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        let out = tick(&mut app);
        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should not be broadcast in the Lobby phase");
    }

    #[test]
    fn hull_integrity_starts_at_100_and_appears_in_sim_snapshot() {
        let mut app = test_app();
        start_game(&mut app);
        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert_eq!(snap.hull_integrity, 100);
    }

    #[test]
    fn direct_damage_reduces_hull_integrity_in_broadcast() {
        let mut app = test_app();
        start_game(&mut app);

        // Directly apply damage to the resource (simulates collision at ~half speed).
        app.world_mut()
            .resource_mut::<ShipHullIntegrity>()
            .0.apply_damage(10);

        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert_eq!(snap.hull_integrity, 90);
    }

    #[test]
    fn taking_25hp_damage_enqueues_2_breakdowns_and_snapshot_shows_first() {
        let mut app = test_app();
        start_game(&mut app);

        // Apply 25 HP of damage directly in 10-HP bucket tracking terms,
        // mimicking how handle_collisions would do it via breakdowns_from_damage.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            let before = bd.cumulative_damage; // 0
            bd.cumulative_damage += 25;
            let new_count = breakdowns_from_damage(before, bd.cumulative_damage);
            assert_eq!(new_count, 2, "25 HP should create exactly 2 breakdowns");
            let BreakdownQueueResource { queue, rng, .. } = &mut *bd;
            for _ in 0..new_count {
                queue.push_random(rng);
            }
        }

        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");

        // Queue has 2 entries; snapshot shows the front (not None).
        assert!(
            snap.authorized_repair_console.is_some(),
            "snapshot should show the authorized repair console"
        );
        // Verify queue length via resource.
        let bd = app.world().resource::<BreakdownQueueResource>();
        assert_eq!(bd.queue.len(), 2, "2 breakdowns should be queued");
        assert_eq!(
            snap.authorized_repair_console.as_ref(),
            bd.queue.front(),
            "snapshot authorized_repair_console matches queue front"
        );
    }

    #[test]
    fn advancing_queue_exposes_next_breakdown() {
        let mut app = test_app();
        start_game(&mut app);

        // Seed 2 breakdowns.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.cumulative_damage = 25;
            let BreakdownQueueResource { queue, rng, .. } = &mut *bd;
            queue.push_random(rng);
            queue.push_random(rng);
        }

        // Capture the first (front) entry.
        let first = app.world().resource::<BreakdownQueueResource>().queue.front().cloned();

        // Pop the front (simulating a successful repair).
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();

        // Second entry is now the front.
        let second = app.world().resource::<BreakdownQueueResource>().queue.front().cloned();

        assert!(second.is_some(), "second breakdown should now be front");
        assert_ne!(first, second, "consecutive entries are different consoles");
    }

    // ── SetTarget / TargetLock tests ──────────────────────────────────

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "target-uuid".into(),
                x: asteroid_x,
                z: asteroid_z,
                radius: 2.0,
                tags: vec![],
            }],
            asteroid_fields: vec![],
        }));
    }

    fn start_game_with_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectConsole { console: Console::Tactical });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        // Asteroid at (30, 0) — 30 units from ship origin, within 60-unit range.
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

        // Server state should record the lock.
        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        // Asteroid at (80, 0) — 80 units away, outside 60-unit Weapons range.
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

    // ── WeaponsUpdate / fire_ready tests ──────────────────────────────────────

    /// Target locked, within 40-unit phaser range, in forward arc → fire_ready = true.
    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        // Ship at origin, yaw=0 (facing -Z). Asteroid at (0, -20): directly ahead, 20 units away.
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        // Lock the target
        let _ = tick(&mut app);
        // Now run another tick to get a WeaponsUpdate
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(update.1, "expected fire_ready=true for in-range, forward-arc target");
    }

    /// Target locked but beyond 40-unit phaser range (within 60u lock range) → fire_ready = false.
    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        // Ship at origin, yaw=0. Asteroid at (0, -50): directly ahead, 50 units — within lock range
        // (60u) but outside phaser range (40u).
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

    // ── FirePhaser / beam lifecycle tests ──────────────────────────────────────

    /// Helper: lock target then fire phaser; returns messages from the fire tick.
    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        // Lock
        push(app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(app);
        // Fire
        push(app, "weapons", ClientMessage::FirePhaser);
        tick(app)
    }

    /// Firing at a fire-ready target broadcasts BeamStarted to all.
    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (yaw=0 → facing -Z → asteroid at (0,-20)).
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

        // ActiveBeam resource should be populated.
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );
    }

    /// FirePhaser is silently ignored when the phaser is on cooldown.
    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Manually put the cooldown into active state (simulating a beam just ended).
        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = None;
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 3.0;

        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown");
    }

    /// Non-weapons player cannot fire.
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

    /// When the beam fires at a target outside the 180° arc, it is rejected.
    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        // Yaw=0 means ship faces -Z. Asteroid at (0, +20) is directly behind — in rear arc.
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        // Lock (within 60u range) — lock doesn't require arc.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        // Fire — rejected because target is behind.
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc");
    }

    /// A 6-second natural beam kills the asteroid (5 HP/s × 6s = 30 HP total).
    ///
    /// The test accelerates time by manipulating the beam state directly
    /// after confirming the beam started, then runs ticks with large deltas.
    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();

        // Spawn an asteroid entity with full HP so tick_active_beam can find it.
        let asteroid_entity = app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();

        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Verify beam started.
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );

        // Fast-forward: accumulate 30 damage via the damage_accumulator.
        // Set accumulator to 30.0 so all damage applies in one tick.
        {
            let mut b = app.world_mut().resource_mut::<ActiveBeam>();
            b.damage_accumulator = 30.0;
            b.remaining_secs = 5.0; // still "ongoing"
        }

        let out = tick(&mut app);

        // Asteroid destroyed message should be present.
        let destroyed = out.iter().find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(destroyed.is_some(), "expected AsteroidDestroyed when asteroid HP reaches 0");
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        // BeamEnded also broadcast.
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction");

        // Asteroid no longer in world data.
        assert!(
            !app.world().resource::<WorldResource>().0.asteroids.iter().any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        // Beam resource cleared.
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        // Cooldown started.
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after beam end");

        // The entity should be despawned.
        assert!(app.world().get::<AsteroidDamage>(asteroid_entity).is_none(),
            "asteroid entity should be despawned");
    }

    /// Beam severs when ship rotates target out of the 180° forward arc.
    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Now rotate ship so the asteroid is behind it (yaw = π → facing +Z, asteroid at (0,-20) is behind).
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after arc sever");
    }

    /// Beam severs when the target moves beyond 40-unit phaser range.
    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move asteroid position in WorldData to 50 units away (out of 40u range).
        app.world_mut().resource_mut::<WorldResource>().0.asteroids[0].z = -50.0;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after range sever");
    }

    /// No damage refund on sever — whatever HP was dealt is permanent.
    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        let asteroid_entity = app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();

        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Apply partial damage via accumulator.
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 10.0;
        let _ = tick(&mut app);

        // Now sever by rotating ship.
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app.world().get::<AsteroidDamage>(asteroid_entity)
            .map(|d| d.current_hp);
        assert!(
            hp.is_some() && hp.unwrap() < 30,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    /// A fresh FirePhaser after cooldown on a new locked target cancels any
    /// active beam and starts a new one.
    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();

        // Set up two asteroids.
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![
                AsteroidInfo { uuid: "t1".into(), x: 0.0, z: -20.0, radius: 2.0, tags: vec![] },
                AsteroidInfo { uuid: "t2".into(), x: 0.0, z: -15.0, radius: 2.0, tags: vec![] },
            ],
            asteroid_fields: vec![],
        }));
        start_game_with_weapons(&mut app);

        // Lock and fire at t1.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t1".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let _ = tick(&mut app);
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t1"));

        // Natural beam expiry: set remaining to 0.
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        // Zero damage accumulator so no destruction fires.
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 0.0;
        let _ = tick(&mut app); // beam ends, cooldown starts

        // Cooldown should be active.
        assert!(app.world().resource::<PhaserCooldown>().is_active());

        // Force cooldown to expire.
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 0.0;

        // Lock and fire at t2.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t2".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown");
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t2"));
    }

    // ── Repair helpers ────────────────────────────────────────────────────

    /// Set up a game with a captain, engineering player, enqueue one breakdown
    /// targeting Engineering, and apply 10 HP damage so HP = 90.
    fn start_game_with_breakdown_for_engineering(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "eng", ClientMessage::Identify { token: "eng".into(), name: "Bob".into() });
        tick(app);
        push(app, "eng", ClientMessage::SelectConsole { console: Console::Engineering });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);

        // Apply 10 damage so HP = 90.
        app.world_mut().resource_mut::<ShipHullIntegrity>().0.apply_damage(10);

        // Force the breakdown queue to have Engineering at the front using a seeded
        // RNG that deterministically yields Engineering first.
        {
            use rand::SeedableRng as _;
            let mut bdr = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bdr.rng = rand::rngs::SmallRng::seed_from_u64(0);
        }
        // Push entries until Engineering is at the front.
        loop {
            let front = {
                let bdr = app.world().resource::<BreakdownQueueResource>();
                bdr.queue.front().cloned()
            };
            if front == Some(Console::Engineering) {
                break;
            }
            // If front is None or wrong, push + pop until Engineering.
            {
                let bdr = app.world_mut().resource_mut::<BreakdownQueueResource>();
                let BreakdownQueueResource { queue, rng, .. } = &mut *bdr.into_inner();
                if front.is_some() {
                    queue.pop_front();
                }
                queue.push_random(rng);
            }
        }
    }

    #[test]
    fn authorized_repair_press_starts_repair_action() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        // Engineering player presses Repair.
        push(&mut app, "eng", ClientMessage::Repair { console: Console::Engineering });
        tick(&mut app);

        assert!(app.world().resource::<ActiveRepair>().is_active(),
            "repair should be active after authorized press");
    }

    #[test]
    fn unauthorized_repair_press_starts_penalty_not_repair() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        // Captain presses Repair — they hold CaptainChair, not Engineering.
        push(&mut app, "captain", ClientMessage::Repair { console: Console::CaptainChair });
        tick(&mut app);

        assert!(!app.world().resource::<ActiveRepair>().is_active(),
            "repair should NOT be active after unauthorized press");
        assert!(app.world().resource::<RepairPenalties>().is_penalised("captain"),
            "captain should have penalty after unauthorized press");
    }

    #[test]
    fn repair_during_cooldown_is_ignored() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        // Captain presses once — gets penalty.
        push(&mut app, "captain", ClientMessage::Repair { console: Console::CaptainChair });
        tick(&mut app);
        assert!(app.world().resource::<RepairPenalties>().is_penalised("captain"));

        // Captain presses again — should still be penalised (no change).
        push(&mut app, "captain", ClientMessage::Repair { console: Console::CaptainChair });
        tick(&mut app);
        // Penalty remaining should still be close to 10s (only a tiny dt elapsed).
        let remaining = app.world().resource::<RepairPenalties>().remaining("captain");
        assert!(remaining > 5.0,
            "penalty should still be near 10s, got {remaining}");
    }

    #[test]
    fn authorized_repair_restores_hp_over_time() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        push(&mut app, "eng", ClientMessage::Repair { console: Console::Engineering });
        tick(&mut app);

        let initial_hp = app.world().resource::<ShipHullIntegrity>().0.current();

        // Simulate 3 seconds of repair (should restore 1 HP).
        app.world_mut().resource_mut::<ActiveRepair>().hp_accumulator = 0.999; // just under 1
        // Manually advance time is hard in Bevy tests; instead inject hp_accumulator directly.
        app.world_mut().resource_mut::<ActiveRepair>().hp_accumulator = 1.0;
        tick(&mut app);

        let hp_after = app.world().resource::<ShipHullIntegrity>().0.current();
        assert_eq!(hp_after, initial_hp + 1, "HP should increase by 1 after accumulator reaches 1.0");
    }

    #[test]
    fn repair_completion_advances_breakdown_queue() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        push(&mut app, "eng", ClientMessage::Repair { console: Console::Engineering });
        tick(&mut app);

        // Fast-complete the repair: set hp_restored to the cap so the next tick
        // triggers queue advancement.
        app.world_mut().resource_mut::<ActiveRepair>().hp_restored = REPAIR_MAX_HP;
        tick(&mut app);

        // Queue should be empty after the single breakdown is resolved.
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty(),
            "breakdown queue should be empty after repair completion");
        assert!(!app.world().resource::<ActiveRepair>().is_active(),
            "repair should no longer be active after completion");
    }

    #[test]
    fn repair_state_broadcast_sent_to_engineering_console() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        push(&mut app, "eng", ClientMessage::Repair { console: Console::Engineering });
        let out = tick(&mut app);

        let repair_state = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { in_progress: true, .. })
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(repair_state.is_some(),
            "RepairState with in_progress=true should be broadcast to engineering");
    }

    #[test]
    fn penalty_repair_state_broadcast_to_penalised_player() {
        let mut app = test_app();
        start_game_with_breakdown_for_engineering(&mut app);

        push(&mut app, "captain", ClientMessage::Repair { console: Console::CaptainChair });
        let out = tick(&mut app);

        let penalty_msg = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { penalty: true, .. })
                && matches!(&m.target, Target::Token(t) if t == "captain")
        });
        assert!(penalty_msg.is_some(),
            "RepairState with penalty=true should be broadcast to the penalised captain");
    }

    // ── SetPhaserMode tests ────────────────────────────────────────────────────

    /// The Weapons console holder can change the phaser mode to Manual.
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

    /// A non-Weapons player cannot change the phaser mode.
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

    // ── SetScienceTarget / ScienceTargetSuggestion tests ─────────────────

    fn start_game_with_science_and_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "science", ClientMessage::Identify { token: "science".into(), name: "Spock".into() });
        tick(app);
        push(app, "science", ClientMessage::SelectConsole { console: Console::Science });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectConsole { console: Console::Tactical });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    // ── Impulse Drive / Damage Cancellation tests ────────────────────────

    fn start_game_with_helm_and_science(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "helm", ClientMessage::Identify { token: "helm".into(), name: "Hikaru".into() });
        tick(app);
        push(app, "helm", ClientMessage::SelectConsole { console: Console::Helm });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn hull_damage_cancels_charging_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        // Helm begins charging impulse.
        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        // Verify impulse is now charging.
        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            crate::impulse::ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );

        // Direct hull damage (simulates a collision landing hull damage).
        app.world_mut()
            .resource_mut::<ShipHullIntegrity>()
            .0.apply_damage(10);
        tick(&mut app);

        // Impulse should have been cancelled.
        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            crate::impulse::ImpulsePhase::Idle,
            "impulse charge should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn hull_damage_cancels_active_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        // Force impulse to Active by directly mutating the resource.
        {
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(crate::impulse::IMPULSE_CHARGE_DURATION);
        }
        assert!(app.world().resource::<ShipImpulse>().0.is_active(),
            "impulse should be active before damage");

        // Apply hull damage.
        app.world_mut()
            .resource_mut::<ShipHullIntegrity>()
            .0.apply_damage(10);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            crate::impulse::ImpulsePhase::Idle,
            "active impulse should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn no_hull_damage_does_not_cancel_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        // No damage applied — tick without damage.
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            crate::impulse::ImpulsePhase::Charging,
            "impulse should still be charging when no damage occurred"
        );
    }

    #[test]
    fn start_impulse_charge_message_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            crate::impulse::ImpulsePhase::Charging,
        );
    }

    #[test]
    fn cancel_impulse_message_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);
        push(&mut app, "helm", ClientMessage::CancelImpulse);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            crate::impulse::ImpulsePhase::Idle,
        );
    }

    #[test]
    fn science_set_science_target_broadcasts_suggestion_to_weapons() {
        let mut app = test_app();
        start_game_with_science_and_weapons(&mut app);

        push(&mut app, "science", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        let suggestion = out.iter().find_map(|m| match &m.msg {
            ServerMessage::ScienceTargetSuggestion { uuid } => Some(uuid.clone()),
            _ => None,
        }).expect("expected a ScienceTargetSuggestion message");
        assert_eq!(suggestion, "asteroid-42");

        // Should be targeted to Weapons console player only.
        let suggestion_msg = out.iter().find(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. }))
            .unwrap();
        assert!(
            matches!(&suggestion_msg.target, Target::Token(t) if t == "weapons"),
            "ScienceTargetSuggestion should be sent only to Weapons console"
        );
    }

    #[test]
    fn non_science_player_cannot_send_science_target() {
        let mut app = test_app();
        start_game_with_science_and_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. })),
            "non-Science player should not be able to send ScienceTargetSuggestion"
        );
    }

    #[test]
    fn set_science_target_ignored_in_lobby() {
        let mut app = test_app();
        push(&mut app, "science", ClientMessage::Identify { token: "science".into(), name: "Spock".into() });
        tick(&mut app);
        push(&mut app, "science", ClientMessage::SelectConsole { console: Console::Science });
        tick(&mut app);

        push(&mut app, "science", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. })),
            "SetScienceTarget should be ignored during Lobby phase"
        );
    }
}
