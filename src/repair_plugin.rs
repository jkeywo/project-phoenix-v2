use bevy::prelude::*;

use crate::breakdown::BreakdownQueue;
use crate::lobby::{CurrentPhase, InboundMessage, Sessions, Target};
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage, Shape};
use crate::repair_teams::RepairTeams;
use crate::simulation::{ShipHullIntegrity, SimOutbox};
use crate::modifiers::ShipModifiers;
use crate::messages::ModifierSlot;

// ── Repair constants ──────────────────────────────────────────────────────────
/// HP restored per completed repair team.
pub const REPAIR_TEAM_HP: f32 = 10.0;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Bevy resource wrapping the pure `RepairTeams` state machine.
#[derive(Resource)]
pub struct ShipRepairTeams(pub RepairTeams);

/// Tracks the last-broadcast repair-icon state so the `broadcast_repair_icons`
/// system can send deltas (ClearRepairIcon for stale icons, ShowRepairIcon for
/// new/changed ones).
#[derive(Resource)]
pub struct RepairIconState {
    /// Map from console to the last shape sent to its holder.
    pub last_icons: std::collections::HashMap<Console, Shape>,
    pub(crate) rng: rand::rngs::SmallRng,
}

impl Default for RepairIconState {
    fn default() -> Self {
        use rand::SeedableRng;
        Self {
            last_icons: std::collections::HashMap::new(),
            rng: rand::rngs::SmallRng::from_os_rng(),
        }
    }
}

/// Bevy resource wrapping the breakdown queue.
#[derive(Resource)]
pub struct BreakdownQueueResource {
    pub queue: BreakdownQueue,
    /// Cumulative damage taken since game start (tracks 10-HP bucket crossings).
    pub cumulative_damage: f32,
    pub(crate) rng: rand::rngs::SmallRng,
}

impl Default for BreakdownQueueResource {
    fn default() -> Self {
        use rand::SeedableRng as _;
        Self {
            queue: BreakdownQueue::new(),
            cumulative_damage: 0.0,
            rng: rand::rngs::SmallRng::from_os_rng(),
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShipRepairTeams(RepairTeams::new()))
            .init_resource::<BreakdownQueueResource>()
            .init_resource::<RepairIconState>()
            .add_systems(Update, (
                handle_repair,
                tick_repair_teams,
                broadcast_repair_icons,
            ))
            .add_plugins(repair_state_broadcaster());
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `RepairState` producer.
///
/// Broadcasts `RepairState` at 10 Hz to the `Repair` console holder only.
/// Registered by [`RepairPlugin`].
pub fn repair_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(Console::Repair),
        Cadence::Hz(10.0),
        |world: &mut World| {
            use crate::messages::TeamSlot;
            let teams = world.resource::<ShipRepairTeams>();
            let breakdowns = world.resource::<BreakdownQueueResource>();

            let slots = teams.0.slots();
            let in_progress = slots.iter().any(|s| matches!(s, TeamSlot::Repairing { .. }));
            let penalty = slots.iter().any(|s| matches!(s, TeamSlot::Cooldown { .. }));
            let remaining_cooldown_secs = slots.iter().map(|s| match s {
                TeamSlot::Repairing { progress } => (1.0 - progress) * 30.0,
                TeamSlot::Cooldown { progress } => progress * 10.0,
                TeamSlot::Idle => 0.0,
            }).fold(0.0_f32, f32::max);

            let current_breakdown = breakdowns.queue.front().map(|entry| (entry.console.clone(), entry.shape));

            vec![ServerMessage::RepairState {
                remaining_cooldown_secs,
                in_progress,
                penalty,
                teams: *slots,
                current_breakdown,
            }]
        },
    )
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Handle `Repair { shape }` messages from the Repair console.
///
/// Validates: game is in-progress, sender holds `Console::Repair`.
/// - If no free team exists: message ignored.
/// - If queue head shape matches pressed shape: lowest-numbered free team
///   dispatched, breakdown popped from queue.
/// - If queue head shape does not match (or queue empty): lowest-numbered
///   free team penalised.
pub fn handle_repair(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut breakdowns: ResMut<BreakdownQueueResource>,
    mut teams: ResMut<ShipRepairTeams>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let pressed_shape = match &ev.msg {
            ClientMessage::Repair { shape } => *shape,
            _ => continue,
        };
        // Only the Repair console holder may send shape-matching presses.
        let Some(repair_token) = sessions.0.console_holder(Console::Repair) else {
            continue;
        };
        if ev.token.as_str() != repair_token {
            continue;
        }
        // Must have a free team to act.
        let Some(team_idx) = teams.0.lowest_free_team() else {
            continue;
        };
        // Check queue front shape (or empty queue).
        match breakdowns.queue.front() {
            Some(entry) if entry.shape == pressed_shape => {
                // Correct shape: dispatch team and pop breakdown.
                teams.0.dispatch(team_idx);
                breakdowns.queue.pop_front();
            }
            _ => {
                // Wrong shape or queue empty: penalise the free team.
                teams.0.penalise(team_idx);
            }
        }
    }
}

/// Tick repair teams each frame: advance progress, apply HP for completed repairs.
pub fn tick_repair_teams(
    time: Res<Time>,
    mut teams: ResMut<ShipRepairTeams>,
    mut hull: ResMut<ShipHullIntegrity>,
    phase: Res<CurrentPhase>,
    modifiers: Res<ShipModifiers>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
    let completed = teams.0.tick(dt * repair_mult);
    for _team_idx in completed {
        hull.0.restore(REPAIR_TEAM_HP);
    }
}

/// Broadcast `ShowRepairIcon` / `ClearRepairIcon` to console holders based
/// on the current breakdown queue state. Sends deltas only.
pub fn broadcast_repair_icons(
    sessions: Res<Sessions>,
    breakdowns: Res<BreakdownQueueResource>,
    phase: Res<CurrentPhase>,
    mut icon_state: ResMut<RepairIconState>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    // Debug: verify we can see the breakdown queue
    let _debug_count = breakdowns.queue.len();
    use crate::breakdown::ALL_CONSOLES;
    use rand::Rng;
    use std::collections::{HashMap, HashSet};

    let mut current: HashMap<Console, Shape> = HashMap::new();
    let mut damaged: HashSet<Console> = HashSet::new();

    for entry in breakdowns.queue.entries() {
        damaged.insert(entry.console.clone());
        current.insert(entry.console.clone(), entry.shape);
    }

    if !breakdowns.queue.is_empty() {
        let undamaged: Vec<&Console> = ALL_CONSOLES
            .iter()
            .filter(|c| !damaged.contains(c))
            .collect();
        if !undamaged.is_empty() {
            let idx = icon_state.rng.random_range(0..undamaged.len());
            let decoy = undamaged[idx].clone();
            let shape = match icon_state.rng.random_range(0..3) {
                0 => Shape::Square,
                1 => Shape::Triangle,
                _ => Shape::Circle,
            };
            current.insert(decoy, shape);
        }
    }

    for (console, _) in &icon_state.last_icons {
        if !current.contains_key(console) {
            if let Some(token) = sessions.0.console_holder(console.clone()) {
                outbox.0.push((Target::Token(token.to_string()), ServerMessage::ClearRepairIcon));
            }
        }
    }

    for (console, shape) in &current {
        if icon_state.last_icons.get(console) != Some(shape) {
            if let Some(token) = sessions.0.console_holder(console.clone()) {
                outbox.0.push((Target::Token(token.to_string()), ServerMessage::ShowRepairIcon { shape: *shape }));
            }
        }
    }

    icon_state.last_icons = current;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::HullIntegrity;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::simulation::{ShipImpulse, ShipShields};
    use crate::shield::ShieldSystem;

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
            .insert_resource(crate::ship_state::ShipState::new())
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_plugins(RepairPlugin)
            .add_plugins(repair_state_broadcaster())
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

    /// Set up a game with a captain, repair player, and a single breakdown
    /// with a known shape at the front. HP = 90.
    fn start_game_with_repair_shape(app: &mut App, shape: Shape) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "eng", ClientMessage::Identify { token: "eng".into(), name: "Bob".into() });
        tick(app);
        push(app, "eng", ClientMessage::SelectStation { station: "Repair".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);

        // Apply 10 damage so HP = 90.
        app.world_mut().resource_mut::<ShipHullIntegrity>().0.apply_damage(10.0);

        // Push a single breakdown with the requested shape and Repair console.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape,
            });
        }
    }

    /// Register captain, repair, helm, tactical, and power players, then start
    /// the game.
    fn start_game_with_repair_basic(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "eng", ClientMessage::Identify { token: "eng".into(), name: "Bob".into() });
        tick(app);
        push(app, "eng", ClientMessage::SelectStation { station: "Repair".into() });
        tick(app);
        push(app, "helm", ClientMessage::Identify { token: "helm".into(), name: "Hikaru".into() });
        tick(app);
        push(app, "helm", ClientMessage::SelectStation { station: "Helm".into() });
        tick(app);
        push(app, "tac", ClientMessage::Identify { token: "tac".into(), name: "Chekov".into() });
        tick(app);
        push(app, "tac", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "power", ClientMessage::Identify { token: "power".into(), name: "Monty".into() });
        tick(app);
        push(app, "power", ClientMessage::SelectStation { station: "Power".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        let _ = tick(app);
    }

    /// Helpers to check RepairTeams team state.
    fn team_is_repairing(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Repairing { .. })
    }

    fn team_is_cooldown(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Cooldown { .. })
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
    }

    /// Find the last ShowRepairIcon targeted to a given console's holder.
    fn last_icon_for(out: &[OutboundMessage], token: &str) -> Option<Shape> {
        out.iter().rev().find_map(|m| {
            if let Target::Token(t) = &m.target {
                if t == token {
                    if let ServerMessage::ShowRepairIcon { shape } = &m.msg {
                        return Some(*shape);
                    }
                }
            }
            None
        })
    }

    /// Check if ClearRepairIcon was sent to a given token.
    fn has_clear_for(out: &[OutboundMessage], token: &str) -> bool {
        out.iter().any(|m| {
            matches!(&m.target, Target::Token(t) if t == token) &&
            matches!(&m.msg, ServerMessage::ClearRepairIcon)
        })
    }

    // -- Shape-matching repair tests -----------------------------------------

    /// Non-Repair console holder sending `Repair { shape }` is ignored.
    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Captain (not Repair holder) presses a shape.
        push(&mut app, "captain", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_idle(&teams, 0), "team 0 should remain idle after non-Repair press");
        assert!(team_is_idle(&teams, 1), "team 1 should remain idle");
        assert!(team_is_idle(&teams, 2), "team 2 should remain idle");
    }

    /// Correct shape dispatches a team and pops the queue.
    #[test]
    fn correct_shape_dispatches_team_and_pops_queue() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Repair holder presses the matching shape.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_repairing(&teams, 0), "team 0 should be repairing after correct shape press");
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty(),
            "breakdown queue should be empty after correct shape repair");
    }

    /// Wrong shape penalises the lowest free team and leaves queue intact.
    #[test]
    fn wrong_shape_penalises_team_and_leaves_queue() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Repair holder presses the WRONG shape (Square, not Triangle).
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Square });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_cooldown(&teams, 0), "team 0 should be on cooldown after wrong shape press");
        assert_eq!(app.world().resource::<BreakdownQueueResource>().queue.len(), 1,
            "breakdown queue should be unchanged after wrong shape press");
    }

    /// All-busy teams: no free team → further presses are ignored.
    #[test]
    fn all_busy_teams_ignore_further_presses() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // First press: correct shape, dispatches team 0.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);
        assert!(team_is_repairing(&app.world().resource::<ShipRepairTeams>(), 0));

        // Push another breakdown.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            use crate::breakdown::BreakdownEntry;
            bd.queue.push_back(BreakdownEntry { console: Console::Repair, shape: Shape::Circle });
        }

        // Manually dispatch teams 1 and 2 so all three are busy.
        app.world_mut().resource_mut::<ShipRepairTeams>().0.dispatch(1);
        app.world_mut().resource_mut::<ShipRepairTeams>().0.dispatch(2);

        // Third press should be ignored (no free team).
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Circle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_repairing(&teams, 0));
        assert!(team_is_repairing(&teams, 1));
        assert!(team_is_repairing(&teams, 2));
        assert_eq!(app.world().resource::<BreakdownQueueResource>().queue.len(), 1,
            "breakdown queue should remain unchanged when all teams are busy");
    }

    /// Empty-queue press penalises the lowest free team.
    #[test]
    fn empty_queue_press_penalises_team() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Pop the queue so it's empty.
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty());

        // Repair holder presses any shape.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_cooldown(&teams, 0), "team 0 should be on cooldown after empty-queue press");
    }

    /// Repair team tick restores HP on completion.
    #[test]
    fn repair_team_completion_restores_hp() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        fn near(a: f32, b: f32) -> bool { (a - b).abs() < 1e-6 }

        let initial_hp = app.world().resource::<ShipHullIntegrity>().0.current(); // 90

        // Dispatch team 0 via correct shape press.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);
        assert!(team_is_repairing(&app.world().resource::<ShipRepairTeams>(), 0));

        // Advance team 0 to completion via the team's own tick method.
        let completed = app.world_mut().resource_mut::<ShipRepairTeams>().0.tick(30.0);
        assert_eq!(completed, vec![0], "team 0 should complete after 30s");

        // Manually apply HP as the system would: for each completed team, restore HP.
        for _ in completed {
            app.world_mut().resource_mut::<ShipHullIntegrity>().0.restore(REPAIR_TEAM_HP);
        }

        let hp_after = app.world().resource::<ShipHullIntegrity>().0.current();
        assert!(near(hp_after, initial_hp + REPAIR_TEAM_HP),
            "HP should increase by {} after repair team completion", REPAIR_TEAM_HP);
    }

    /// RepairState broadcast shows in_progress when team is repairing.
    #[test]
    fn repair_state_shows_in_progress() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        let out = tick(&mut app);

        let repair_state = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { in_progress: true, .. })
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(repair_state.is_some(),
            "RepairState with in_progress=true should be broadcast to repair console");
    }

    /// RepairState broadcast shows penalty when team is on cooldown.
    #[test]
    fn repair_state_shows_penalty() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Press wrong shape to penalise team 0.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Square });
        let out = tick(&mut app);

        let penalty_msg = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { penalty: true, .. })
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(penalty_msg.is_some(),
            "RepairState with penalty=true should be broadcast after wrong shape press");
    }

    // -- Repair icon broadcast tests -----------------------------------------

    #[test]
    fn push_assigns_real_icon_to_damaged_console() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Triangle,
            });
        }

        let out = tick(&mut app);

        let icon = last_icon_for(&out, "eng");
        assert_eq!(icon, Some(Shape::Triangle), "Repair holder should receive ShowRepairIcon with Triangle");
    }

    #[test]
    fn push_assigns_decoy_to_undamaged_console() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Triangle,
            });
        }

        let out = tick(&mut app);

        let decoy_tokens = ["helm", "tac", "power", "captain"];
        let has_decoy = decoy_tokens.iter().any(|t| last_icon_for(&out, t).is_some());
        assert!(has_decoy, "at least one undamaged console should receive a decoy ShowRepairIcon");
    }

    #[test]
    fn pop_clears_real_icon() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Square,
            });
        }
        let _ = tick(&mut app); // first tick sends ShowRepairIcon

        // Pop the breakdown.
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();
        let out = tick(&mut app);

        assert!(has_clear_for(&out, "eng"), "Repair holder should receive ClearRepairIcon after pop");
    }

    #[test]
    fn old_decoy_cleared_before_new_decoy_assigned() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);
        use rand::SeedableRng;

        // Manually set previous state: Repair has a real icon (Square),
        // Helm was the decoy (Triangle). Damaged = {Repair}.
        {
            let state = &mut app.world_mut().resource_mut::<RepairIconState>();
            state.last_icons.clear();
            state.last_icons.insert(Console::Repair, Shape::Square);
            state.last_icons.insert(Console::Helm, Shape::Triangle);
            state.rng = rand::rngs::SmallRng::seed_from_u64(0);
        }

        // Current queue: Repair (Square) only.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Square,
            });
        }

        let others: Vec<Console> = crate::breakdown::ALL_CONSOLES.iter()
            .filter(|c| **c != Console::Repair && **c != Console::CaptainChair && **c != Console::Helm)
            .cloned()
            .collect();
        for c in &others {
            app.world_mut().resource_mut::<BreakdownQueueResource>().queue.push_front(
                crate::breakdown::BreakdownEntry { console: c.clone(), shape: Shape::Circle },
            );
        }

        let out = tick(&mut app);
        let state = app.world().resource::<RepairIconState>();

        let helm_in_state = state.last_icons.contains_key(&Console::Helm);
        let captain_in_state = state.last_icons.contains_key(&Console::CaptainChair);
        assert!(helm_in_state || captain_in_state, "either Helm (old decoy) or Captain (new decoy) should be in state");
        if captain_in_state && !helm_in_state {
            assert!(has_clear_for(&out, "helm"), "Helm should receive ClearRepairIcon when replaced as decoy");
        }
    }

    #[test]
    fn empty_queue_clears_all_icons() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Square,
            });
        }
        let _ = tick(&mut app); // first tick sends icons

        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty());
        let out = tick(&mut app);

        assert!(has_clear_for(&out, "eng"), "Repair holder should be cleared when queue empties");

        let any_show = out.iter().any(|m| matches!(&m.msg, ServerMessage::ShowRepairIcon { .. }));
        assert!(!any_show, "no ShowRepairIcon should be sent when queue is empty");
    }

    #[test]
    fn no_undamaged_consoles_shows_no_decoy() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        // Fill all 5 ALL_CONSOLES with breakdowns (CaptainChair, Helm, Tactical, Repair, Power)
        for console in &crate::breakdown::ALL_CONSOLES {
            app.world_mut().resource_mut::<BreakdownQueueResource>().queue.push_back(
                crate::breakdown::BreakdownEntry {
                    console: console.clone(),
                    shape: Shape::Square,
                }
            );
        }

        let out = tick(&mut app);

        assert!(last_icon_for(&out, "captain").is_some(), "Captain should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "eng").is_some(), "Repair should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "helm").is_some(), "Helm should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "tac").is_some(), "Tactical should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "power").is_some(), "Power should receive ShowRepairIcon");

        let state = app.world().resource::<RepairIconState>();
        assert_eq!(state.last_icons.len(), 5, "only 5 damaged consoles should have icons, no decoy");
    }
}
