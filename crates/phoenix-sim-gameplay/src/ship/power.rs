use crate::core::messages::{CoordinationPayload, PowerGroupId};
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
/// of the allocation entry label and of [`CoordinationPayload::PowerBrownout`]'s
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
    mut ships: Query<(
        &crate::core::messages::AdmittedCommands,
        &mut ShipPowerSystem,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    use crate::core::messages::SystemControlPayload as Control;
    for (admitted, mut power) in &mut ships {
        for cmd in admitted.for_target(crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID) {
            let mut candidate = power.0.clone();
            let result: Result<(), String> = match &cmd.payload {
                Control::SetPowerGroupAllocation { group, level } => candidate
                    .set_group_allocation(group, *level)
                    .map_err(|error| format!("{error:?}")),
                Control::SetEnergyGeneration { enabled } => {
                    candidate.set_generation_enabled(*enabled);
                    Ok(())
                }
                Control::SetEnergyReactor {
                    output_per_sec,
                    cooling_per_sec,
                } => candidate
                    .set_output(*output_per_sec)
                    .and_then(|()| candidate.set_reactor_cooling(*cooling_per_sec)),
                Control::SetEnergyGroup {
                    group,
                    flow_per_sec,
                    cooling_per_sec,
                } => candidate
                    .set_flow(group, *flow_per_sec)
                    .and_then(|()| candidate.set_cooling(group, *cooling_per_sec)),
                Control::SetEnergySystemFlow {
                    system,
                    flow_per_sec,
                } => candidate.set_fine_flow(system, *flow_per_sec),
                Control::SetEnergyMode {
                    mode,
                    cooling_usage,
                } => {
                    candidate.set_mode(*mode);
                    candidate.set_cooling_usage(*cooling_usage);
                    Ok(())
                }
                Control::SetEnergyPriority { group, priority } => {
                    reorder_energy_group(&mut candidate, group, *priority)
                }
                _ => continue,
            };
            if result.is_ok() {
                power.0 = candidate;
            }
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                cmd,
                if result.is_ok() {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

/// A priority command moves one sibling in a stable list, rather than creating
/// ambiguous equal priorities or silently reordering unrelated subgroups.
fn reorder_energy_group(
    power: &mut PowerSystem,
    group: &PowerGroupId,
    requested: u16,
) -> Result<(), String> {
    let parent = power
        .config()
        .groups
        .iter()
        .find(|entry| &entry.id == group)
        .ok_or("unknown energy group")?
        .parent
        .clone();
    let mut siblings: Vec<_> = power
        .config()
        .groups
        .iter()
        .filter(|entry| entry.parent == parent)
        .map(|entry| entry.id.clone())
        .collect();
    siblings.sort_by(|a, b| {
        (power.network().groups[a].priority, a).cmp(&(power.network().groups[b].priority, b))
    });
    siblings.retain(|id| id != group);
    siblings.insert(usize::from(requested).min(siblings.len()), group.clone());
    for (rank, id) in siblings.iter().enumerate() {
        power.set_priority(
            id,
            u16::try_from(rank).map_err(|_| "too many priority siblings")?,
        )?;
    }
    Ok(())
}

// Energy routing and real-stock integration live in Simulation's owning Power adapter.

/// Map a power group to its explicit Coordination destination. Multi-instance
/// Shields has no coarse System, so it resolves the shared `shield_arc` owner.
fn addresses_for_power_group(
    config: &crate::ship::config::ShipConfig,
    power: &PowerSystem,
    group: &str,
) -> Vec<crate::core::messages::CoordinationAddress> {
    let mut addresses = Vec::new();
    for system in &config.systems {
        let mut cursor = system.power_group.as_ref();
        let mut affected = false;
        for _ in 0..=power.config().groups.len() {
            let Some(id) = cursor else {
                break;
            };
            if id.0 == group {
                affected = true;
                break;
            }
            cursor = power
                .config()
                .groups
                .iter()
                .find(|row| &row.id == id)
                .and_then(|row| row.parent.as_ref());
        }
        if affected {
            if let Some(address) = crate::ship::coordination::address_for_system(config, &system.id)
            {
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
        }
    }
    addresses
}

/// Report new incoming-flow cuts to every affected Console, including fine
/// groups and groups spanning several seats. Reserves and standing orders remain.
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
        let cuts: std::collections::HashSet<_> = power
            .0
            .network()
            .groups
            .iter()
            .filter(|(_, group)| group.cut)
            .map(|(id, _)| id.0.clone())
            .collect();
        let mut newly_cut: Vec<_> = cuts
            .difference(&brownout_state.notified_groups)
            .cloned()
            .collect();
        newly_cut.sort();
        brownout_state.notified_groups = cuts;

        let sender_origin = control_sources
            .0
            .source_for(&crate::ship::system_registry::power_reactor_system_id());

        for id in newly_cut {
            let group_id = crate::core::messages::PowerGroupId(id);
            let level = 0;
            for address in addresses_for_power_group(&ship_config.0, &power.0, &group_id.0) {
                writer.write(CoordinationEnqueue {
                    source_entity: entity,
                    sender_origin,
                    address,
                    payload: CoordinationPayload::PowerBrownout {
                        group: group_id.0.clone(),
                        label: ship_config
                            .0
                            .power_groups
                            .get(&group_id)
                            .map(|group| group.label.clone())
                            .unwrap_or_else(|| power_group_label(&group_id.0).to_string()),
                        allocated_level: level,
                    },
                    presentation: crate::core::messages::CoordinationPresentation::new(
                        "coordination.power_brownout.title",
                        "coordination.power_brownout.body",
                    )
                    .with_title_param(
                        "label",
                        ship_config
                            .0
                            .power_groups
                            .get(&group_id)
                            .map(|group| group.label.as_str())
                            .unwrap_or_else(|| power_group_label(&group_id.0)),
                    )
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
