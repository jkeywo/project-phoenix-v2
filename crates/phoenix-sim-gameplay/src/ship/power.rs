use crate::core::messages::{
    CoordinationPayload, PowerBatteryBlackboard, PowerBlackboard, PowerGroupEntry, PowerGroupId,
    PowerReactorBlackboard, SystemBlackboard, SystemId,
};
use crate::modifiers::power_system::{
    power_level_for_group, AuthoredPowerGroup, PowerConfig, PowerSystem, HELM_POWER_GROUP,
    POWER_GROUP_ORDER, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
};
use crate::ship_plugin::CoordinationEnqueue;
use bevy::prelude::*;

/// Wraps the pure-Rust power system so it can be used as a Bevy resource.
///
/// Derives both `Resource` (existing player-ship singleton) and `Component`
/// (per-entity path after issue #594 unification).
#[derive(Resource, Component, Clone)]
pub struct ShipPowerSystem(pub PowerSystem);

/// Wraps the power config for the ship's power system.
///
/// Dual-derives `Resource` (legacy global fallback used by tests) and
/// `Component` (per-entity storage on each ship — PR 6 migration, see PRD #597).
#[derive(Resource, Component, Default, Clone)]
pub struct PowerConfigResource(pub PowerConfig);

/// Per-group power multiplier configuration: `[f32; 4]` indexed by level-1
/// (index 0 = level 1, index 3 = level 4). Defaults give `[-0.5, 0.0, 0.25, 0.5]`
/// for every canonical power group unless overridden in the ship TOML.
///
/// After issue #617 the map is keyed by [`PowerGroupId`] rather than `Console`.
///
/// Dual-derives `Resource` (legacy global fallback used by tests) and
/// `Component` (per-entity storage on each ship — PR 6 migration, see PRD #597).
#[derive(Resource, Component, Clone, Debug)]
pub struct PowerMultiplierResource {
    pub multipliers: std::collections::HashMap<PowerGroupId, [f32; 4]>,
}

impl Default for PowerMultiplierResource {
    fn default() -> Self {
        let defaults = [-0.5, 0.0, 0.25, 0.5];
        let mut multipliers = std::collections::HashMap::new();
        for &name in POWER_GROUP_ORDER {
            multipliers.insert(PowerGroupId(name.to_string()), defaults);
        }
        Self { multipliers }
    }
}

/// Per-ship inline stateless Power allocation policy (issue #784).
///
/// From the ship's `[power.ai_policy]` block when authored, otherwise the
/// canonical [`crate::entities::config::default_power_ai_config`] policy
/// (which reproduces the retired stateful engine's helm←thrust / weapons←red
/// alert behaviour with reserve guards). Read by
/// `console_ai::server::ai_power_allocation`, which for each of the ship's
/// AUTHORED power groups resolves that group's channel over an immutable
/// per-tick fact snapshot and emits the winning `set_power_group_allocation`
/// verb's absolute level as an admitted `SetPowerGroupAllocation` — the same
/// admitted command a human Power operator sends.
///
/// This RETIRES the bespoke stateful `PowerAiConfigResource` +
/// `ShipPowerAiState` (`EngageState` hysteresis) from #762: the decision is now
/// a pure function of the per-tick snapshot, with no private timer state
/// (AGENTS.md rule #7). Attached to every ship at spawn (player + NPC).
#[derive(Component, Clone, Debug, Default)]
pub struct PowerAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship multiplier on the shared AI base cadence for `[power.ai_policy]`
/// (issue #889's PASM-tracked runtime gap: `evaluate_every_ticks` was parsed
/// and validated but no host read it). `ai_power_allocation` decides on every
/// Nth arm of the shared `ai_tick_ready` latch rather than every arm.
///
/// A sibling component to [`PowerAiPolicy`] rather than a field ON it (or on
/// the shared [`crate::ai::policy::AiPolicy`] type itself): dozens of call
/// sites across the crate construct an `AiPolicy` by literal, and this way
/// wiring Power's cadence costs one small component instead of touching every
/// one of them, most of which do not (yet) need per-host cadence at all.
///
/// `1` — the parse default, and what every shipped hull authors today — means
/// "every arm", identical to behaviour before this component existed.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PowerAiCadence(pub u32);

/// Seed the immutable per-tick fact snapshot the Power allocation policy
/// resolves each authored power group's channel against (issue #784).
///
/// Pure and Bevy-free (AGENTS.md rule #10): the host reads live state and passes
/// primitives in. Seeds the BROAD fact set so authored guards can read ship,
/// threat, objective, and system facts — but the canonical default policy only
/// needs `battery_pct`, `thrust`, and `red_alert`. The #779 empty-facts lesson:
/// a `fact(...)` guard validates but never fires unless the host seeds the fact,
/// so this is THE piece that makes the reserve guard (and every other) live.
///
/// SHIP facts: `battery_pct`, `thrust`, `red_alert`, per-group `power_<group>`
/// current level, and `total_allocation`. THREAT: `secs_since_combat` (absent
/// when the ship has no combat history) and `nearest_enemy_dist` (absent when
/// none is known). OBJECTIVE: `has_destroy_objective` (`1.0`/`0.0`). SYSTEM:
/// `offline_system_count`.
#[allow(clippy::too_many_arguments)]
pub fn seed_power_facts(
    power: &PowerSystem,
    battery_pct: f32,
    thrust: f32,
    red_alert: bool,
    secs_since_combat: Option<f32>,
    nearest_enemy_dist: Option<f32>,
    has_destroy_objective: bool,
    offline_system_count: u32,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set(
        crate::entities::config::POWER_BATTERY_PCT_FACT,
        battery_pct as f64,
    );
    facts.set(crate::entities::config::POWER_THRUST_FACT, thrust as f64);
    facts.set(
        crate::entities::config::POWER_RED_ALERT_FACT,
        if red_alert { 1.0 } else { 0.0 },
    );
    // Per-group current level, keyed `power_<group>`, plus the ship-wide total.
    for (id, level) in power.iter() {
        facts.set(&format!("power_{}", id.0), level as f64);
    }
    facts.set_fact(fid::TOTAL_ALLOCATION, power.total() as f64);
    // THREAT: only set when known, so an absent-fact guard reads "no threat".
    if let Some(s) = secs_since_combat {
        facts.set_fact(fid::SECS_SINCE_COMBAT, s as f64);
    }
    if let Some(d) = nearest_enemy_dist {
        facts.set_fact(fid::NEAREST_ENEMY_DIST, d as f64);
    }
    // OBJECTIVE + SYSTEM.
    facts.set_fact(
        fid::HAS_DESTROY_OBJECTIVE,
        if has_destroy_objective { 1.0 } else { 0.0 },
    );
    facts.set_fact(fid::OFFLINE_SYSTEM_COUNT, offline_system_count as f64);
    seed_strike_reserve_facts(&mut facts, power);
    facts
}

/// Shared, read-only readings for Power allocation and Helm transitions. These
/// are the same charge and toggle that the human consoles receive; no AI reserve.
pub fn seed_strike_reserve_facts(facts: &mut crate::world::flags::AiFacts, power: &PowerSystem) {
    use crate::entities::ai_flag_hosts as fid;
    if let Some(boost) = power.strike_boost() {
        facts.set_fact(fid::STRIKE_RESERVE_CHARGE, power.battery_charge() as f64);
        facts.set_fact(
            fid::STRIKE_BOOST_ENABLED,
            if boost.enabled { 1.0 } else { 0.0 },
        );
    }
}

/// Debounce state for power brownout coordination advisories (issue #678).
///
/// Tracks which power groups have already been notified of a brownout
/// condition so the advisory only fires once per transition into brownout.
/// Cleared when the group exits the brownout condition, allowing re-fire
/// on subsequent brownout cycles.
#[derive(Component, Default, Clone)]
pub struct PowerBrownoutState {
    /// Group id strings (e.g. "weapons", "helm", "shields") that are
    /// currently in a notified-brownout state.
    pub notified_groups: std::collections::HashSet<String>,
    /// Set by [`tick_power_system`] from
    /// [`crate::modifiers::power_system::PowerSystem::tick`]'s return whenever
    /// the reactor's exhaustion lock changed state this tick, and consumed (and
    /// cleared) by [`tick_power_brownout_advisory`] later in the same tick.
    ///
    /// This is the edge-triggered half of the advisory (issue #678). The
    /// debounce below is level-triggered off the reserve's direction; the edge
    /// re-arms it the tick the reactor locks out so the brownout announces
    /// itself to Helm or Tactical.
    pub locked_changed: bool,
}

impl PowerBrownoutState {
    /// Read the authoritative advisory debounce for continuation projection.
    pub fn continuation(&self) -> (&std::collections::HashSet<String>, bool) {
        (&self.notified_groups, self.locked_changed)
    }

    /// Replace bootstrap debounce state with a restored continuation.
    pub fn replace_continuation(
        &mut self,
        notified_groups: std::collections::HashSet<String>,
        locked_changed: bool,
    ) {
        self.notified_groups = notified_groups;
        self.locked_changed = locked_changed;
    }
}

/// Maps a canonical power group id string to the `strings.csv` id for its
/// display label. Anything unknown falls back to `power.group.unknown`.
///
/// **This function is the live path, for every hull.** It is the sole producer
/// of [`PowerGroupEntry::label`] and of [`CoordinationPayload::PowerBrownout`]'s
/// label, and nothing in `gui/` reads a `power_groups` block. Per issue #977 it
/// emits a `strings.csv` id, never composed English: the blackboard and the
/// coordination payload both cross `localiseTree` at the wire boundary, which
/// resolves the id to "HELM" / "WEAPONS" / "SHIELDS" client-side. The authored
/// `[power_groups.<id>] label` fields are real ids too and covered by
/// `scripts/check-strings.mjs`; wiring them up means carrying the ship's
/// authored labels into `publish_power_blackboard`, with these as the fallback
/// for a hull that authors none.
pub fn power_group_label(group_id: &str) -> &'static str {
    match group_id {
        HELM_POWER_GROUP => "power.group.helm",
        WEAPONS_POWER_GROUP => "power.group.weapons",
        SHIELDS_POWER_GROUP => "power.group.shields",
        _ => "power.group.unknown",
    }
}

/// Returns the current power level for `group` from the `PowerSystem`.
///
/// Delegates to [`power_level_for_group`]; kept as a thin wrapper for the
/// legacy call-site shape.
pub fn power_level_for(ps: &PowerSystem, group: &PowerGroupId) -> u8 {
    power_level_for_group(ps, group)
}

/// Build a deterministic seed list for
/// [`PowerSystem::from_authored_groups`](crate::modifiers::power_system::PowerSystem::from_authored_groups)
/// from a ship's authored `[power_groups.*]` config (issue #762).
///
/// The canonical groups (`helm`, `weapons`, `shields`) come first in their
/// stable [`POWER_GROUP_ORDER`], then any extra authored groups (e.g. `ops`)
/// sorted by id, each seeded at its authored `default_level` AND its authored
/// `min_level` (issue #1395 — the floor the reactor stores and clamps every
/// later order to, so a hull that authors `min_level = 0` really can have that
/// group commanded cold). Returns an empty vec when there are no authored
/// groups so the caller falls back to the canonical default seeding (unchanged
/// behaviour for NPCs / fixtures without a `[power_groups.*]` block).
pub fn authored_power_group_seed(
    power_groups: &std::collections::HashMap<PowerGroupId, crate::ship::config::PowerGroupConfig>,
) -> Vec<AuthoredPowerGroup> {
    if power_groups.is_empty() {
        return Vec::new();
    }
    fn entry(id: PowerGroupId, cfg: &crate::ship::config::PowerGroupConfig) -> AuthoredPowerGroup {
        AuthoredPowerGroup {
            id,
            level: cfg.default_level,
            floor: cfg.min_level,
        }
    }
    let mut seed: Vec<AuthoredPowerGroup> = Vec::with_capacity(power_groups.len());
    for &name in POWER_GROUP_ORDER {
        let id = PowerGroupId(name.to_string());
        if let Some(cfg) = power_groups.get(&id) {
            seed.push(entry(id, cfg));
        }
    }
    let mut extra: Vec<(&PowerGroupId, &crate::ship::config::PowerGroupConfig)> = power_groups
        .iter()
        .filter(|(id, _)| !POWER_GROUP_ORDER.contains(&id.0.as_str()))
        .collect();
    extra.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (id, cfg) in extra {
        seed.push(entry(id.clone(), cfg));
    }
    seed
}

/// Apply admitted settings to the reactor owner. Ownership and knowledge remain
/// Console-scoped; displaying another consumer never admits its commands.
pub fn handle_power_messages(
    mut ships: Query<(&crate::core::messages::AdmittedCommands, &mut ShipPowerSystem)>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    use crate::core::messages::SystemControlPayload as Control;
    for (admitted, mut power) in &mut ships {
        for cmd in admitted.for_target(crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID) {
            let mut candidate = power.0.clone();
            let result: Result<(), String> = match &cmd.payload {
                Control::SetPowerGroupAllocation { group, level } => candidate.set_group_allocation(group, *level).map_err(|error| format!("{error:?}")),
                Control::SetEnergyReactor { output_per_sec, cooling_per_sec } => candidate.set_output(*output_per_sec).and_then(|()| candidate.set_reactor_cooling(*cooling_per_sec)),
                Control::SetEnergyGroup { group, flow_per_sec, cooling_per_sec } => candidate.set_flow(group, *flow_per_sec).and_then(|()| candidate.set_cooling(group, *cooling_per_sec)),
                Control::SetEnergySystemFlow { system, flow_per_sec } => candidate.set_fine_flow(system, *flow_per_sec),
                Control::SetEnergyMode { mode, cooling_usage } => { candidate.set_mode(*mode); candidate.set_cooling_usage(*cooling_usage); Ok(()) },
                Control::SetEnergyPriority { group, priority } => reorder_energy_group(&mut candidate, group, *priority),
                _ => continue,
            };
            if result.is_ok() { power.0 = candidate; }
            crate::command_admission::finish_admitted_action_feedback(&mut outbound, cmd,
                if result.is_ok() { crate::core::messages::ActionFeedbackOutcome::Applied }
                else { crate::core::messages::ActionFeedbackOutcome::Refused });
        }
    }
}

/// A priority command moves one sibling in a stable list, rather than creating
/// ambiguous equal priorities or silently reordering unrelated subgroups.
fn reorder_energy_group(power: &mut PowerSystem, group: &PowerGroupId, requested: u16) -> Result<(), String> {
    let parent = power.config().groups.iter().find(|entry| &entry.id == group)
        .ok_or("unknown energy group")?.parent.clone();
    let mut siblings: Vec<_> = power.config().groups.iter().filter(|entry| entry.parent == parent)
        .map(|entry| entry.id.clone()).collect();
    siblings.sort_by(|a, b| (power.network().groups[a].priority, a).cmp(&(power.network().groups[b].priority, b)));
    siblings.retain(|id| id != group);
    siblings.insert(usize::from(requested).min(siblings.len()), group.clone());
    for (rank, id) in siblings.iter().enumerate() { power.set_priority(id, u16::try_from(rank).map_err(|_| "too many priority siblings")?)?; }
    Ok(())
}

// Energy routing and real-stock integration live in Simulation's owning Power adapter.

/// Map a power group to its explicit Coordination destination. Multi-instance
/// Shields has no coarse System, so it resolves the shared `shield_arc` owner.
fn address_for_power_group(
    config: &crate::ship::config::ShipConfig,
    group: &str,
) -> Option<crate::core::messages::CoordinationAddress> {
    match group {
        WEAPONS_POWER_GROUP => crate::ship::coordination::address_for_system(
            config,
            &crate::ship::system_registry::tactical_radar_system_id(),
        ),
        HELM_POWER_GROUP => crate::ship::coordination::address_for_system(
            config,
            &crate::ship::system_registry::helm_steering_system_id(),
        ),
        SHIELDS_POWER_GROUP => crate::ship::coordination::address_for_system_kind(
            config,
            crate::ship::system_registry::SHIELD_ARC_KIND,
        ),
        _ => None,
    }
}

/// Emit power brownout coordination advisories when the reactor EXHAUSTS —
/// battery charge hits zero and [`PowerSystem::tick`] takes every group DOWN to
/// [`GROUP_LEVEL_MIN`] and locks the reactor.
///
/// This is the brownout, and the only one: the ship actually lost power and
/// every system that was running reset to 1. (A group somebody had already
/// switched off is left cold rather than raised — see `PowerSystem::tick`.) It is emphatically NOT a "reserve running low"
/// warning — a draining-but-managed reactor (the AI shed ladder doing its job
/// on the way down, which is the normal state of any ship holding elevated
/// power in combat) is expected and says nothing. Firing on mere drain
/// spammed every red-alert fight, loudest on the player's own ship, for a
/// condition that never reached the lock the ladder exists to avoid.
///
/// Driven by [`PowerBrownoutState::locked_changed`] — the lock-changed edge
/// [`tick_power_system`] forwards from [`PowerSystem::tick`]. The edge fires on
/// BOTH lock and unlock; only the INTO-locked direction (`power.0.locked()`) is
/// a brownout. On lock every group's owning station is told (helm → Helm,
/// weapons → Tactical, shields → Shields) and the announced set is recorded in
/// `notified_groups` for the intent narration to read while the lock persists;
/// on recovery the set is cleared so a later exhaustion re-announces.
///
/// # `sender_origin`
///
/// Resolved from the ship's own `power-reactor` control source — the allocation
/// surface, i.e. the system whose state *is* the brownout, and the same
/// representative `handle_power_messages` admits against. Until issue #873 this
/// was hardcoded to `ControlSource::Ai`, which is a routing-tag lie in the
/// opposite direction from the emit-side branches that issue removed: a
/// human-operated Power console's advisory claimed AI origin, so
/// `route_coordination` raised a popup at a human Helm/Tactical where two humans
/// on the same bridge should simply talk to each other (Suppress). Nothing here
/// branches on the value — it is stamped and forgotten, exactly as
/// `detect_damage_tier_crossings` stamps its own.
pub fn tick_power_brownout_advisory(
    mut ships: Query<
        (
            Entity,
            &ShipPowerSystem,
            &mut PowerBrownoutState,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, power, mut brownout_state, control_sources, ship_config) in ships.iter_mut() {
        // Consume this tick's lock-changed edge. Only a transition INTO the
        // locked state is a brownout; an unlock (recovery) consumes the edge and
        // clears the announced set so the next exhaustion re-announces.
        if !std::mem::take(&mut brownout_state.locked_changed) {
            continue;
        }
        let cuts: std::collections::HashSet<_> = power.0.network().groups.iter()
            .filter(|(_, group)| group.cut).map(|(id, _)| id.0.clone()).collect();
        let mut newly_cut: Vec<_> = cuts.difference(&brownout_state.notified_groups).cloned().collect();
        newly_cut.sort();
        brownout_state.notified_groups = cuts;

        let sender_origin = control_sources
            .0
            .source_for(&crate::ship::system_registry::power_reactor_system_id());

        for id in newly_cut {
            let group_id = crate::core::messages::PowerGroupId(id);
            let level = 0;
            if let Some(address) = address_for_power_group(&ship_config.0, &group_id.0) {
                writer.write(CoordinationEnqueue {
                    source_entity: entity,
                    sender_origin,
                    address,
                    payload: CoordinationPayload::PowerBrownout {
                        group: group_id.0.clone(),
                        label: power_group_label(&group_id.0).to_string(),
                        allocated_level: level,
                    },
                    presentation: crate::core::messages::CoordinationPresentation::new(
                        "coordination.power_brownout.title",
                        "coordination.power_brownout.body",
                    )
                    .with_title_param("label", power_group_label(&group_id.0))
                    .with_body_param("level", level),
                    sender_label: crate::ship::coordination::CHATTER_SENDER_POWER.to_string(),
                    sender_system: crate::ship::system_registry::power_reactor_system_id(),
                });
            }
        }
    }
}

/// The world flag mirroring the hull the crew fly: `weapons_cold.own_ship`.
///
/// Keyed by ROLE rather than by an authored reference name, because the
/// player's hull is not required to declare one — `falling_skyway.toml` gives
/// its `player-ship` an `id` and no `name`, so there is no name to key on and a
/// scenario still has to be able to ask the question.
///
/// Replaces `weapons_hold.own_ship` (issue #1041) key for key: what moved is the
/// state behind it, from a hidden captain's boolean to the ship's own reactor.
pub const OWN_SHIP_WEAPONS_COLD_FLAG: &str = "weapons_cold.own_ship";

/// The world flag mirroring a named ship's cold weapons group:
/// `weapons_cold.<name>`.
///
/// The naming imitates issue #1035's `workforce.<id>.on_strike` deliberately:
/// authoritative state lives in the ship's own [`ShipPowerSystem`], the flag is
/// a MIRROR of it, and scenario script reads the mirror. `name` is the entity's
/// authored reference name — the same string `on_destroyed(...)` and `hail(...)`
/// take.
pub fn weapons_cold_flag(name: &str) -> String {
    format!("weapons_cold.{name}")
}

pub fn publish_power_blackboard(
    power_res: Option<Res<ShipPowerSystem>>,
    config_res: Option<Res<PowerConfigResource>>,
    multipliers_res: Option<Res<PowerMultiplierResource>>,
    ship_q: Query<
        (
            &ShipPowerSystem,
            Option<&PowerConfigResource>,
            Option<&PowerMultiplierResource>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    // The authored hull, for each group's `min_level` (issue #1004). Read via
    // its OWN query for the same reason `control_sources_q` below is: the
    // primary `ship_q` requires a per-entity `ShipPowerSystem`, so a LocalShip
    // running off the global resources would silently contribute nothing and
    // every group would publish the fallback floor instead of its authored one.
    ship_config_q: Query<
        &crate::ship_plugin::ShipConfigComponent,
        With<crate::server_app::LocalShip>,
    >,
    control_sources_q: Query<
        &crate::ship_plugin::ShipSystemControlSources,
        With<crate::server_app::LocalShip>,
    >,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    use crate::ship::system_registry::{
        POWER_BATTERY_SYSTEM_ID, POWER_REACTOR_SYSTEM_ID, POWER_SYSTEM_ID,
    };

    // Prefer per-entity components on LocalShip; fall back to global Resources.
    let entity_view = ship_q.single().ok();
    let power_default;
    let power: &ShipPowerSystem = match entity_view.map(|(p, _, _)| p) {
        Some(p) => p,
        None => match power_res.as_deref() {
            Some(p) => p,
            None => {
                power_default =
                    ShipPowerSystem(crate::modifiers::power_system::PowerSystem::default());
                &power_default
            }
        },
    };
    let config_default;
    let config: &PowerConfigResource = match entity_view.and_then(|(_, c, _)| c) {
        Some(c) => c,
        None => match config_res.as_deref() {
            Some(c) => c,
            None => {
                config_default = PowerConfigResource::default();
                &config_default
            }
        },
    };
    let multipliers_default;
    let multipliers: &PowerMultiplierResource = match entity_view.and_then(|(_, _, m)| m) {
        Some(m) => m,
        None => match multipliers_res.as_deref() {
            Some(m) => m,
            None => {
                multipliers_default = PowerMultiplierResource::default();
                &multipliers_default
            }
        },
    };
    // Per-fine-system online state, derived from offline_systems on the
    // LocalShip's control sources. Read via a separate query so the
    // fine-system online flags survive test setups that spawn a LocalShip
    // with `ShipSystemControlSources` but no per-entity `ShipPowerSystem`
    // component (the primary `ship_q` above requires the latter).
    let control_sources = control_sources_q.single().ok();
    let reactor_id = crate::ship::system_registry::power_reactor_system_id();
    let battery_id = crate::ship::system_registry::power_battery_system_id();
    let reactor_online = control_sources
        .map(|cs| !cs.0.is_offline(&reactor_id))
        .unwrap_or(true);
    let battery_online = control_sources
        .map(|cs| !cs.0.is_offline(&battery_id))
        .unwrap_or(true);

    let ship_config = ship_config_q.single().ok();

    let entries: Vec<PowerGroupEntry> = power
        .0
        .iter()
        .map(|(gid, _)| gid.clone())
        .filter(|gid| {
            multipliers.multipliers.contains_key(gid)
                || config
                    .0
                    .strike_reserve
                    .as_ref()
                    .is_some_and(|r| r.group == gid.0)
        })
        .map(|gid| {
            let max_level = multipliers
                .multipliers
                .get(&gid)
                .map(|arr| arr.len() as u8)
                .unwrap_or(4);
            // This group's own authored floor — the lowest level it may be
            // commanded to (issue #1004, made a real engine clamp by #1395). A
            // hull that declares no `[power_groups.*]` block has none to read,
            // so the parse default stands in, which is also the floor
            // `PowerSystem` seeds. Read off the hull rather than off the
            // multiplier table, since the table's LENGTH is a ceiling and says
            // nothing about where the rungs start.
            //
            // The same number the reactor is holding: `PowerSystem::floor_for`
            // was seeded from this very field at spawn. Published from the
            // config rather than from the reactor because the blackboard is
            // built from the hull throughout, and the two cannot disagree.
            let min_level = ship_config
                .and_then(|sc| sc.0.power_groups.get(&gid))
                .map(|g| g.min_level)
                .unwrap_or_else(crate::ship::config::default_min_power_level);
            PowerGroupEntry {
                id: gid.0.clone(),
                label: ship_config
                    .filter(|_| {
                        config
                            .0
                            .strike_reserve
                            .as_ref()
                            .is_some_and(|r| r.group == gid.0)
                    })
                    .and_then(|sc| sc.0.power_groups.get(&gid))
                    .map(|group| group.label.clone())
                    .unwrap_or_else(|| power_group_label(gid.0.as_str()).into()),
                level: power_level_for(&power.0, &gid),
                // The standing order, so the panel's +/- can step from what was
                // ASKED for rather than from what a battery floor has left the
                // group running at (issue #952).
                commanded_level: power.0.commanded_level_for(&gid),
                min_level,
                max_level,
            }
        })
        .collect();

    let bb = PowerBlackboard {
        energy: None,
        strike_reserve: power.0.strike_read(&config.0),
        groups: entries,
        total: power.0.total(),
        total_max: power.0.max_commanded_total(),
        battery_charge: power.0.battery_charge(),
        battery_max: config.0.capacity,
        draining: power.0.is_draining(&config.0),
        charging: power.0.is_charging(&config.0),
        locked: power.0.locked(),
    };
    // Fine blackboards (issue #513) — reactor owns the allocation surface,
    // battery owns the emergency-reserve pool. Emitted alongside the legacy
    // coarse `Power` blackboard so downstream JS panels can pick either or both.
    let reactor_bb = PowerReactorBlackboard {
        total_allocation: power.0.total(),
        max_allocation: power.0.max_commanded_total(),
        is_online: reactor_online,
        draining: power.0.is_draining(&config.0),
    };
    let emergency_threshold = if config.0.capacity > 0.0 {
        (config.0.emergency_threshold / config.0.capacity).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let battery_bb = PowerBatteryBlackboard {
        charge: power.0.battery_charge(),
        capacity: config.0.capacity,
        is_online: battery_online,
        emergency_threshold,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(POWER_SYSTEM_ID.to_string()),
            SystemBlackboard::Power(bb),
        );
        bbs.0.insert(
            SystemId(POWER_REACTOR_SYSTEM_ID.to_string()),
            SystemBlackboard::PowerReactor(reactor_bb),
        );
        bbs.0.insert(
            SystemId(POWER_BATTERY_SYSTEM_ID.to_string()),
            SystemBlackboard::PowerBattery(battery_bb),
        );
    }
}
