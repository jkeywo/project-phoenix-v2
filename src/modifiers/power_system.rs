use crate::core::messages::PowerGroupId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A ship's saved reactor allocation, battery reserve and exhaustion lock.
///
/// These explicit scalars retain the existing `snapshot::PowerState` wire
/// contract. They are separate from runtime state and from the Power read
/// surface: snapshot orchestration owns entity identity and ordering, while
/// the reactor owns its projection and reinstatement.
///
/// Power-derived damage, motion and shield modifiers read these values on the
/// first resumed tick. An immediately equal world digest does not establish
/// that continuation: the reactor itself is not part of that digest fold.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerState {
    /// `(power group id, level)` in reactor insertion order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allocations: Vec<(String, u8)>,
    pub battery_charge: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

pub const HELM_POWER_GROUP: &str = "helm";
pub const WEAPONS_POWER_GROUP: &str = "weapons";
pub const SHIELDS_POWER_GROUP: &str = "shields";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerAllocationError {
    UnknownGroup(PowerGroupId),
}

/// The level a SCRIPTED power order asks a group to be commanded to (issue
/// #1398).
///
/// Two variants rather than a bare `u8` because a scenario script cannot know a
/// hull's authored numbers and must not have to. `ctx.effects.hold_fire(name)`
/// and `release_fire(name)` are the two verbs that carry this today: hold means
/// [`Self::Exact`]`(0)`, and release means "put it back where the hull says it
/// belongs", which is [`Self::AuthoredDefault`] — the group's
/// `[power_groups.<id>] default_level`, resolved by the applier against the
/// ship's own config rather than restated in the world file. A script that
/// guessed `2` would be authoring one hull's number into every scenario that
/// used the verb, which is exactly the hardcoding AGENTS.md rule 11 forbids.
///
/// [`Self::Exact`] is the general form the two verbs are special cases of. It is
/// what a future `set_power(entity, group, level)` verb would push, and it is
/// how `hold_fire` expresses itself — `Exact(0)`, one rung the applier clamps to
/// the group's own authored `min_level` like any other order.
///
/// Bevy-free and pure, beside the reactor vocabulary it speaks in: `world`'s
/// dispatch layer names it in [`crate::world::dispatch::ActionCmd`] and
/// `ship::power` resolves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptedPowerLevel {
    /// Command the group to this exact level. Clamped by the applier to
    /// `[floor_for(group), GROUP_LEVEL_MAX]` exactly as a console order is, so a
    /// hull whose `min_level` is 1 refuses `Exact(0)` and stays warm.
    Exact(u8),
    /// Command the group back to its hull's authored `default_level` — or to
    /// [`crate::ship::config::default_power_level`], the level a group with no
    /// `[power_groups.<id>]` block boots at, when the hull describes none.
    AuthoredDefault,
}

/// One queued scripted power order, resolved from an authored entity NAME to a
/// ship uuid by the dispatch applier and drained by
/// `crate::ship::power::drain_scripted_power_orders` (issue #1398).
///
/// The two-step shape is the one every other name-carrying scripted effect
/// already has (`PendingCivilianOrder`, the infrastructure adjustments): the
/// applier is where `name_to_uuid` lives and holds no entity query, and the
/// draining system is where the reactor lives and holds no name table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingGroupPower {
    /// The target ship's `EntityUuid`.
    pub uuid: String,
    /// The power group to command — `weapons` for both of today's verbs, but
    /// carried as a field rather than assumed, so a hull that authors its guns
    /// onto another group is one script verb away from being orderable.
    pub group: PowerGroupId,
    /// What to command it to.
    pub level: ScriptedPowerLevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerReadState {
    pub allocations: Vec<(PowerGroupId, u8)>,
    pub battery_charge: f32,
    /// True while the reactor is locked out after exhausting its battery — every
    /// group is taken down to level 1 and the allocation controls are frozen until
    /// the reserve recovers past [`PowerConfig::emergency_threshold`].
    pub locked: bool,
}

impl PowerReadState {
    pub fn level_for_group(&self, group: &PowerGroupId) -> Option<u8> {
        self.allocations
            .iter()
            .find(|(id, _)| id == group)
            .map(|(_, level)| *level)
    }
}

pub struct Channel1Read<'a> {
    state: &'a PowerReadState,
}

impl<'a> Channel1Read<'a> {
    pub fn new(state: &'a PowerReadState) -> Self {
        Self { state }
    }

    pub fn power_level(&self, group: &PowerGroupId) -> Option<u8> {
        self.state.level_for_group(group)
    }
}

/// The stable canonical order of the built-in power groups. The publisher
/// walks this order to build wire snapshots. Extending the list requires
/// touching the wire format and every filter path, so keep it minimal.
///
/// `shields` replaced `sensors` here in issue #952. Sensors had stopped buying
/// anything a reactor point is worth spending on once #955 decoupled weapon
/// reach from [`crate::core::messages::ModifierSlot::RadarRange`], so the third group
/// is one a reactor point is actually worth spending on — shields, not a radar
/// horizon.
pub const POWER_GROUP_ORDER: &[&str] =
    &[HELM_POWER_GROUP, WEAPONS_POWER_GROUP, SHIELDS_POWER_GROUP];

/// Pure `PowerSystem` state — keyed by [`PowerGroupId`] after issue #617.
///
/// The three canonical groups (`helm`, `weapons`, `shields`) are seeded at
/// construction so tests can rely on `level_for` returning `Some(2)` without
/// first calling `set_group_allocation`. Additional groups can be added by
/// TOML-driven config in future PRs.
///
/// # Per-group floors
///
/// Since issue #1395 the floor is the GROUP'S OWN, read off its authored
/// `[power_groups.<id>] min_level` and carried in `floors`. A hull that authors
/// `min_level = 0` for a group has said that group may be commanded COLD —
/// switched off outright rather than merely turned down — and every clamp in
/// the setter API honours that. [`GROUP_LEVEL_MIN`] is now the DEFAULT floor an
/// unauthored group takes, not a global one every group is held at.
///
/// # Exhaustion lock
///
/// `groups` holds what the reactor has been told to run each group at — by a
/// human Power operator or by `ai_power_allocation`, through the one admitted
/// `SetPowerGroupAllocation` applier. When the battery is drained to empty the
/// reactor browns out: every group is forced DOWN to level 1 and `locked` is
/// set, freezing the allocation controls until the reserve has recovered past
/// [`PowerConfig::emergency_threshold`]. A player who fails to manage power is
/// meant to feel that — there is no graceful per-group floor holding systems up.
///
/// Down, never up: a brownout is a loss of power, so it cannot be the thing
/// that switches a cold group back on. A group the crew (or a script) has taken
/// to 0 stays at 0 through the lock.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerSystem {
    /// Per-group allocation level. Values are clamped to
    /// `[floors[group], GROUP_LEVEL_MAX]` by the setter API; direct
    /// construction should preserve that invariant.
    groups: HashMap<PowerGroupId, u8>,
    /// Per-group commandable floor — the group's authored
    /// `[power_groups.<id>] min_level`, or [`GROUP_LEVEL_MIN`] for a group
    /// seeded without one (issue #1395).
    ///
    /// Separate from `groups` because it is CONFIG, not run state: it is seeded
    /// once from the hull and never moves again, which is why [`Self::restore`]
    /// rebuilds `groups` and `order` from a save but leaves this map alone —
    /// the save records what a run commanded, the hull records what the run was
    /// allowed to command.
    floors: HashMap<PowerGroupId, u8>,
    /// Insertion order of `groups`; walked by publishers so wire output is
    /// deterministic even when the HashMap iteration order isn't.
    order: Vec<PowerGroupId>,
    /// True while the reactor is locked out after a full brownout. Set by
    /// [`Self::tick`] when the battery hits zero, cleared once the charge climbs
    /// back to [`PowerConfig::emergency_threshold`]. While locked, `increase`
    /// and `decrease` are no-ops.
    locked: bool,
    /// The ship-wide allocation budget copied from its authored reactor config.
    max_commanded_total: u8,
    pub battery_charge: f32,
}

/// One authored power group as the reactor is SEEDED with it (issue #1395):
/// the group's id, the level it spawns at, and the floor no order may take it
/// below.
///
/// A named struct rather than a tuple because the floor is the whole point of
/// it. `(id, level)` was already ambiguous enough at a call site; `(id, level,
/// floor)` would be worse, and a caller that mixed the last two up would author
/// a group whose floor was 2 and whose boot level was 1 — a hull that spawns
/// under its own minimum and can never be commanded back down to where it
/// started. Named fields make that unwriteable.
///
/// Every field is read off the hull's `[power_groups.<id>]` block by
/// `ship::power::authored_power_group_seed`. There is nothing here a Rust
/// caller supplies that the TOML does not.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredPowerGroup {
    /// The group's id — the `[power_groups.<id>]` table name.
    pub id: PowerGroupId,
    /// The level the group boots at — its authored `default_level`.
    pub level: u8,
    /// The lowest level any operator, human or AI, may command this group to —
    /// its authored `min_level`. `0` means the group may be taken COLD.
    pub floor: u8,
}

impl AuthoredPowerGroup {
    /// A seed entry for a fixture that authors no `[power_groups.*]` floor: the
    /// group takes [`GROUP_LEVEL_MIN`], exactly as the TOML parse default would
    /// give it. Used by tests and by code building a config in Rust; production
    /// spawns go through `ship::power::authored_power_group_seed`, which reads
    /// the hull's real `min_level`.
    pub fn at_default_floor(id: PowerGroupId, level: u8) -> Self {
        Self {
            id,
            level,
            floor: GROUP_LEVEL_MIN,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerConfig {
    pub capacity: f32,
    pub rates: [f32; 6],
    /// Highest allocation total that must leave the reserve non-draining.
    pub sustainable_total: u8,
    /// Ship-wide allocation ceiling enforced by human and AI commands.
    pub max_commanded_total: u8,
    /// Emergency recovery threshold, in the same ABSOLUTE units as `capacity`.
    /// Once the reactor has locked out at a flat battery it stays locked until
    /// the charge climbs back to this level, at which point the allocation
    /// controls unfreeze. Also published on `PowerBatteryBlackboard` (as a
    /// fraction of capacity) so the battery gauge can paint the reserve band.
    pub emergency_threshold: f32,
}

/// The DEFAULT lowest level a power group can be commanded to — the floor a
/// group takes when its hull authors no `[power_groups.<id>] min_level`.
/// Defined by CALLING [`crate::ship::config::default_min_power_level`] — the
/// parse default that field gets — so the seeded floor and the authoring
/// default cannot drift apart.
///
/// Since issue #1395 this is no longer a global clamp. A group may author its
/// own `min_level`, including `0`, and [`PowerSystem`] stores and clamps to
/// that; see [`PowerSystem::floor_for`]. What this constant still is, on its
/// own account, is the level the exhaustion lock forces every group DOWN to —
/// the one rung a brownout leaves running.
pub const GROUP_LEVEL_MIN: u8 = crate::ship::config::default_min_power_level();

/// The highest level any power group can be commanded to, whatever its own
/// `max_level` says. Defined by CALLING
/// [`crate::ship::config::default_max_power_level`], whose docs already name it
/// "the ceiling the allocation API clamps every group to".
pub const GROUP_LEVEL_MAX: u8 = crate::ship::config::default_max_power_level();

/// The reactor's ship-wide allocation budget: the COMMANDED total across every
/// group that [`PowerSystem::increase`] refuses to go past.
///
/// Not a new number — this is the `8` that has always been inline in
/// `increase`, lifted out so every site that spends against it reads ONE value:
/// `increase`'s refusal, [`plan_allocation`]'s budget, and the top rung of the
/// [`PowerConfig::rates`] table that [`PowerSystem::battery_rate`] and
/// [`PowerSystem::tick`] index. Issue #959: the applier enforced the budget
/// silently and the AI decider had no idea the budget existed, so a policy
/// whose per-group targets summed past it had the excess dropped without error
/// and re-asked for on every decision arm, for ever. [`plan_allocation`] closes
/// that by spending against this const before anything is emitted.
///
/// Still a Rust constant rather than a `[power]` field, and deliberately so
/// for now: [`PowerSystem::set_group_allocation`] and [`PowerSystem::increase`]
/// take no [`PowerConfig`], so making the budget per-hull is a signature change
/// across every caller of the allocation API rather than a tuning change.
/// Authoring it is a separate piece of work; stating it once is the
/// precondition for that work, not a substitute for it.
impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            capacity: 100.0,
            rates: [6.0, 5.0, 4.0, 2.0, -2.0, -6.0],
            sustainable_total: 6,
            max_commanded_total: 8,
            emergency_threshold: 25.0,
        }
    }
}

impl PowerConfig {
    /// The lowest allocation represented by the rate ladder. Its final rung is
    /// always the authored command ceiling.
    pub fn minimum_rated_total(&self) -> u8 {
        self.max_commanded_total
            .saturating_sub(self.rates.len().saturating_sub(1) as u8)
    }
}

impl Default for PowerSystem {
    fn default() -> Self {
        Self::new(&PowerConfig::default())
    }
}

impl PowerSystem {
    pub fn new(config: &PowerConfig) -> Self {
        Self::seeded_with_defaults(config)
    }

    /// Internal helper: construct a PowerSystem with the three canonical
    /// groups pre-seeded at level 2 and the requested battery charge.
    fn seeded_with_defaults(config: &PowerConfig) -> Self {
        let mut groups = HashMap::with_capacity(3);
        let mut floors = HashMap::with_capacity(3);
        let mut order = Vec::with_capacity(3);
        for &name in POWER_GROUP_ORDER {
            let id = PowerGroupId(name.to_string());
            groups.insert(id.clone(), 2u8);
            // No hull to read: the canonical trio takes the parse default
            // floor, which is what a `[power_groups.*]`-less TOML would give
            // them anyway.
            floors.insert(id.clone(), GROUP_LEVEL_MIN);
            order.push(id);
        }
        Self {
            groups,
            floors,
            order,
            locked: false,
            max_commanded_total: config.max_commanded_total,
            battery_charge: config.capacity,
        }
    }

    /// Construct a `PowerSystem` seeded from a ship's authored power groups
    /// (issue #762). Each entry is inserted at its authored level, clamped to
    /// `[floor, GROUP_LEVEL_MAX]`, in the order supplied — so a ship that
    /// authors an extra group beyond the canonical three gets it seeded and
    /// therefore allocatable (otherwise `set_group_allocation` returns
    /// `UnknownGroup` and any authored rule targeting it silently no-ops).
    ///
    /// Each entry also carries the group's own [`AuthoredPowerGroup::floor`],
    /// which is STORED (issue #1395): it is the floor every later clamp in this
    /// type reads, so a hull authoring `min_level = 0` really can be commanded
    /// cold. Taking the floor here rather than deriving it later is what makes
    /// the seed the single place a hull's authoring enters the reactor.
    ///
    /// Falls back to [`Self::seeded_with_defaults`] (the canonical `helm` /
    /// `weapons` / `shields` at level 2, all at [`GROUP_LEVEL_MIN`]) when
    /// `groups` is empty, so ships and fixtures without a `[power_groups.*]`
    /// block are unchanged.
    pub fn from_authored_groups(config: &PowerConfig, groups: &[AuthoredPowerGroup]) -> Self {
        if groups.is_empty() {
            return Self::seeded_with_defaults(config);
        }
        let mut map = HashMap::with_capacity(groups.len());
        let mut floors = HashMap::with_capacity(groups.len());
        let mut order = Vec::with_capacity(groups.len());
        for group in groups {
            if map.contains_key(&group.id) {
                continue;
            }
            let floor = group.floor.min(GROUP_LEVEL_MAX);
            map.insert(group.id.clone(), group.level.clamp(floor, GROUP_LEVEL_MAX));
            floors.insert(group.id.clone(), floor);
            order.push(group.id.clone());
        }
        Self {
            groups: map,
            floors,
            order,
            locked: false,
            max_commanded_total: config.max_commanded_total,
            battery_charge: config.capacity,
        }
    }

    /// Total allocation across all groups — the draw the battery is carrying.
    /// This is what [`Self::tick`] indexes `rates` with.
    pub fn total(&self) -> u8 {
        self.groups.values().copied().sum()
    }

    /// Alias of [`Self::total`]. Retained for the budget-planner call sites
    /// (issue #959): with the exhaustion lock there is no floored-vs-commanded
    /// distinction, so the commanded total and the effective total are the same.
    pub fn commanded_total(&self) -> u8 {
        self.total()
    }

    /// Current level for the given power group. Returns `0` for groups the
    /// system does not know about (matches the historical
    /// `power_level_for_console` fallback for non-powered consoles).
    pub fn level_for(&self, group: &PowerGroupId) -> u8 {
        self.groups.get(group).copied().unwrap_or(0)
    }

    /// Alias of [`Self::level_for`], retained for the budget-planner call sites
    /// (issue #959). Returns `0` for unknown groups.
    pub fn commanded_level_for(&self, group: &PowerGroupId) -> u8 {
        self.level_for(group)
    }

    /// True while the reactor is locked out after exhausting its battery.
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// The allocation ceiling copied from the ship's authored reactor config.
    pub fn max_commanded_total(&self) -> u8 {
        self.max_commanded_total
    }

    /// True if the system tracks the given power group.
    pub fn has_group(&self, group: &PowerGroupId) -> bool {
        self.groups.contains_key(group)
    }

    /// The lowest level `group` may be commanded to — its authored
    /// `[power_groups.<id>] min_level` (issue #1395).
    ///
    /// Returns [`GROUP_LEVEL_MIN`] for a group the reactor does not track, so a
    /// caller reading a floor before checking [`Self::has_group`] gets the
    /// conservative answer rather than a licence to switch something off.
    pub fn floor_for(&self, group: &PowerGroupId) -> u8 {
        self.floors.get(group).copied().unwrap_or(GROUP_LEVEL_MIN)
    }

    /// True when this reactor tracks `group` and it is currently at level 0 —
    /// COLD, switched off rather than turned down (issue #1395).
    ///
    /// Guarded on [`Self::has_group`] deliberately. [`Self::level_for`] returns
    /// `0` for a group the reactor has never heard of, so the bare level test
    /// would read every hull with no weapons group as having cold weapons —
    /// the exact opposite of the answer a fire gate or a sensor readout wants
    /// from it.
    pub fn is_group_cold(&self, group: &PowerGroupId) -> bool {
        self.has_group(group) && self.level_for(group) == 0
    }

    /// Insertion-ordered iteration over `(&PowerGroupId, level)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&PowerGroupId, u8)> {
        self.order.iter().map(move |id| (id, self.level_for(id)))
    }

    /// Overwrite the whole reactor state from a snapshot (issue #997).
    ///
    /// Reinstates each group at its stored level in the stored order, the
    /// battery charge, and the lock — the three things a run *changed* that the
    /// per-tick recompute cannot re-derive. `snapshot::restore` needs this
    /// because the `PhaserDamage`/`MaxSpeed`/`ShieldRegen` modifiers are
    /// recomputed every tick from these levels, so a resumed ship whose reactor
    /// came back at the seeded default (every group at 2) fires, steers and
    /// regenerates at a different intensity than the live one on the very first
    /// tick after a restore — a small, silent, per-ship divergence a digest
    /// match at the instant of restore cannot see, because the digest folds
    /// `ShipPhysics` and hull, not the reactor.
    ///
    /// Writes the fields directly rather than routing through
    /// [`Self::set_group_allocation`]: that path clamps to the ship-wide budget
    /// and no-ops while `locked`, so it could neither reinstate a legally-reached
    /// over-budget transient nor set the levels of a locked-out reactor. A
    /// restore reinstates a state the run already reached under those rules; it
    /// is not commanding a new one.
    ///
    /// Clamps to the group's OWN floor rather than to [`GROUP_LEVEL_MIN`]
    /// (issue #1395). The global clamp was the resume bug: a ship whose crew had
    /// taken weapons cold came back at level 1 with live guns, because the one
    /// line that reinstated the save disagreed with the hull about what the
    /// lowest legal level was. The floors themselves are NOT restored — they are
    /// the hull's, seeded at spawn by [`Self::from_authored_groups`] before any
    /// save is laid over the entity, and a save that could rewrite them would be
    /// a save that could re-author the ship.
    pub fn restore(
        &mut self,
        allocations: &[(PowerGroupId, u8)],
        battery_charge: f32,
        locked: bool,
    ) {
        self.groups.clear();
        self.order.clear();
        for (id, level) in allocations {
            if self.groups.contains_key(id) {
                continue;
            }
            let floor = self.floor_for(id);
            self.groups
                .insert(id.clone(), (*level).clamp(floor, GROUP_LEVEL_MAX));
            self.order.push(id.clone());
        }
        self.battery_charge = battery_charge;
        self.locked = locked;
    }

    pub fn read_state(&self) -> PowerReadState {
        PowerReadState {
            allocations: self
                .order
                .iter()
                .map(|id| (id.clone(), self.level_for(id)))
                .collect(),
            battery_charge: self.battery_charge,
            locked: self.locked,
        }
    }

    /// Capture the saved projection in the reactor's own allocation order.
    /// Default values are still a complete replacement of bootstrap state.
    pub fn capture_continuation(&self) -> PowerState {
        PowerState {
            allocations: self
                .iter()
                .map(|(id, level)| (id.0.clone(), level))
                .collect(),
            battery_charge: self.battery_charge,
            locked: self.locked(),
        }
    }

    /// Reinstate a saved frontier using the reactor's existing restore rules:
    /// authored group floors survive, duplicate ids keep their first position,
    /// and the saved lock does not prevent reinstating its own allocations.
    pub fn restore_continuation(&mut self, saved: &PowerState) {
        let allocations: Vec<_> = saved
            .allocations
            .iter()
            .map(|(id, level)| (PowerGroupId(id.clone()), *level))
            .collect();
        self.restore(&allocations, saved.battery_charge, saved.locked);
    }

    /// Set the allocation for a specific power group to `level`, clamped to
    /// `[floor_for(group), GROUP_LEVEL_MAX]`. Delta is applied one step at a
    /// time via `increase` / `decrease` so the `total() <= 8` and `locked`
    /// invariants are honoured.
    ///
    /// The lower clamp is the GROUP'S floor since issue #1395, so a group whose
    /// hull authored `min_level = 0` can be ordered cold and one that did not
    /// still cannot.
    pub fn set_group_allocation(
        &mut self,
        group: &PowerGroupId,
        level: u8,
    ) -> Result<(), PowerAllocationError> {
        if !self.groups.contains_key(group) {
            return Err(PowerAllocationError::UnknownGroup(group.clone()));
        }
        let current = self.commanded_level_for(group);
        let target_level = level.clamp(self.floor_for(group), GROUP_LEVEL_MAX);
        if target_level > current {
            for _ in 0..(target_level - current) {
                self.increase(group);
            }
        } else if target_level < current {
            for _ in 0..(current - target_level) {
                self.decrease(group);
            }
        }
        Ok(())
    }

    /// Increase the allocation for `group` by 1. Clamped to `4` per group and
    /// to `8` for the total. No-op when the reactor is locked.
    pub fn increase(&mut self, group: &PowerGroupId) {
        if self.locked || self.total() >= self.max_commanded_total {
            return;
        }
        if let Some(v) = self.groups.get_mut(group) {
            if *v < GROUP_LEVEL_MAX {
                *v += 1;
            }
        }
    }

    /// Decrease the allocation for `group` by 1. Clamped to that group's own
    /// authored floor ([`Self::floor_for`], issue #1395) — `0` for a group its
    /// hull says may be taken cold. No-op when the reactor is locked.
    pub fn decrease(&mut self, group: &PowerGroupId) {
        if self.locked {
            return;
        }
        let floor = self.floor_for(group);
        if let Some(v) = self.groups.get_mut(group) {
            if *v > floor {
                *v -= 1;
            }
        }
    }

    /// The reactor's current net battery rate, in charge units per second, at
    /// the total allocation. Negative means the ship is spending its reserve
    /// faster than the reactor makes it.
    pub fn battery_rate(&self, config: &PowerConfig) -> f32 {
        let minimum = config.minimum_rated_total();
        let total = self.total().clamp(minimum, self.max_commanded_total) as usize;
        config.rates[total - minimum as usize]
    }

    /// True when the battery is falling at the current draw. Published so the
    /// gauge can say whether the reserve is filling or emptying.
    pub fn is_draining(&self, config: &PowerConfig) -> bool {
        self.battery_rate(config) < 0.0
    }

    /// True when the battery is actually FILLING at the current draw.
    ///
    /// Deliberately not `!is_draining()`: a hull may author a rate of exactly
    /// `0.0` for some total, and at that total the reserve is frozen — neither
    /// emptying nor filling. Painting the gauge's pulsing CHARGING indicator
    /// from the negation would claim a recovery that is never going to arrive,
    /// which is the most misleading thing a battery readout can say.
    pub fn is_charging(&self, config: &PowerConfig) -> bool {
        self.battery_rate(config) > 0.0
    }

    /// Advance the simulation by `dt` seconds. Integrates the battery from the
    /// current total allocation, then handles exhaustion (every group forced
    /// DOWN to 1 and the reactor locked) and recovery (unlock once the charge
    /// climbs back to [`PowerConfig::emergency_threshold`]).
    ///
    /// There is no graceful per-group floor. A ship that flattens its battery
    /// browns out completely: the player who let it happen loses the lot until
    /// the reserve recovers. `ship::power::tick_power_system` forwards the
    /// returned lock-changed edge into `PowerBrownoutState`, which
    /// `ship::power::tick_power_brownout_advisory` consumes later the same tick.
    ///
    /// Returns `true` if the `locked` state changed this tick.
    pub fn tick(&mut self, dt: f32, config: &PowerConfig) -> bool {
        let prev_locked = self.locked;
        let rate = self.battery_rate(config);
        self.battery_charge = (self.battery_charge + rate * dt).clamp(0.0, config.capacity);

        if self.battery_charge <= 0.0 {
            for v in self.groups.values_mut() {
                // `min`, not assignment (issue #1395). A brownout is a LOSS of
                // power; it takes every group down to the one rung the reserve
                // can still carry, and a group already at 0 is below that
                // already. Slamming it to 1 would have the reactor switching
                // the guns back on at the exact moment it lost the battery,
                // undoing a standing order nobody had cancelled.
                *v = (*v).min(GROUP_LEVEL_MIN);
            }
            self.locked = true;
        } else if self.locked && self.battery_charge >= config.emergency_threshold {
            self.locked = false;
        }

        self.locked != prev_locked
    }
}

/// Free function preserving the historical `power_level_for_console`
/// signature but keyed on `PowerGroupId`. Returns `0` for unknown groups.
pub fn power_level_for_group(ps: &PowerSystem, group: &PowerGroupId) -> u8 {
    ps.level_for(group)
}

/// One group's claim on the reactor budget for a single allocation decision
/// (issue #959) — what an authored rule asked for, and the authored data that
/// decides who gets served when the budget cannot pay for everything.
///
/// Every field is READ OFF THE HULL'S OWN TOML. There is no field here a Rust
/// caller could use to override the authored config, and no ordering the caller
/// supplies beyond the one [`PowerSystem`] already publishes.
#[derive(Clone, Debug, PartialEq)]
pub struct AllocationBid {
    /// The power group this bid is for.
    pub group: PowerGroupId,
    /// Absolute level the winning `[[power.ai_policy.rule]]` asked this group
    /// to hold.
    pub want: u8,
    /// This group's own ceiling — `[power_groups.<id>] max_level`, or its parse
    /// default for a hull that authors no such block.
    pub max_level: u8,
    /// This group's own floor — `[power_groups.<id>] min_level`, or its parse
    /// default for a hull that authors no such block (issue #1395).
    ///
    /// The planner guarantees every bidder its floor before it hands out a
    /// single discretionary point, so this is what a bidding group costs the
    /// budget just by being in the running. A group authored at `0` therefore
    /// costs nothing to keep in the plan, which is the arithmetic that lets a
    /// hull carry a coldable weapons group without shrinking what the rest of
    /// the reactor can be asked for.
    pub floor: u8,
    /// The `priority` of the authored rule that won this group's channel. The
    /// ordering key, and the whole of the "priority is data-authored"
    /// requirement: a designer who wants weapons served before helm when the
    /// budget is short raises that rule's `priority` in the hull file.
    ///
    /// No Rust list outranks a preference the hull expressed. A tie on priority
    /// falls back to the caller's own order — [`PowerSystem::iter`]'s, i.e.
    /// [`POWER_GROUP_ORDER`] first and then alphabetically, via
    /// `ship::power::authored_power_group_seed` — which is determinism, not a
    /// design opinion: it only decides between groups the hull ranked
    /// identically. See the sort site in [`plan_allocation`].
    pub rule_priority: i32,
}

/// A bid's effective floor: its authored `min_level`, held under its own
/// ceiling so a hull that authored the pair the wrong way round cannot make the
/// planner's `want - floor` arithmetic underflow.
fn bid_floor(bid: &AllocationBid) -> u8 {
    bid.floor
        .min(bid.max_level.clamp(GROUP_LEVEL_MIN, GROUP_LEVEL_MAX))
}

/// The level [`plan_allocation`] GUARANTEES a bidder before it hands out a
/// single discretionary point — and therefore both what the bid costs the
/// budget and how far rationing can cut it.
///
/// The group's own floor, except that a bid asking for a WARM level is
/// guaranteed at least [`GROUP_LEVEL_MIN`] (issue #1395). Rationing hands a
/// group as much as the budget reaches, and for a coldable group "as much as
/// the budget reaches" could reach zero — which would have the planner switch
/// weapons off because the reactor was busy, and then, by the cold rule in
/// [`plan_allocation`], leave them off. Restraint is an order somebody gives,
/// not a rounding outcome, so the two halves of the rule are symmetric: the AI
/// never RAISES a cold group, and it never PARKS one cold either. A bid that
/// explicitly asks for its floor still lands there — an authored rule that bids
/// 0 gets 0, and costs the budget nothing.
fn bid_guarantee(bid: &AllocationBid) -> u8 {
    bid_want(bid).min(bid_floor(bid).max(GROUP_LEVEL_MIN))
}

/// What a bid is asking for, clamped into its group's own authored range.
fn bid_want(bid: &AllocationBid) -> u8 {
    bid.want.clamp(
        bid_floor(bid),
        bid.max_level.clamp(GROUP_LEVEL_MIN, GROUP_LEVEL_MAX),
    )
}

/// Distribute the reactor's allocation budget across the groups that bid for it
/// (issue #959), returning the levels to COMMAND, in the order they must be
/// applied.
///
/// # The bug this replaces
///
/// The AI decider used to resolve each group's channel in isolation and emit
/// that group's absolute target with no idea what the others had asked for.
/// [`PowerSystem::increase`] refuses past its authored ceiling SILENTLY and
/// drops the surplus with no error, so a policy whose targets summed past the
/// budget got some groups served, the rest left where they were — and, because
/// the decider only skips an emit when the commanded level already MATCHES its
/// target, the unserved ones were re-asked for on every decision arm for the
/// rest of the encounter. A cap refusal that neither the policy nor any log
/// could observe, and an admitted command re-issued for ever.
///
/// # What this does instead
///
/// * Groups with no bid are RESERVED at their current commanded level. The
///   policy has authored no verb for them, so there is nothing that says they
///   may be cut; this preserves an authored auxiliary group a policy does not
///   bid for.
/// * A group that is currently COLD is treated as one of those — its bid is
///   DROPPED (issue #1395). See "The AI never raises a cold group" below.
/// * Every bidding group is guaranteed [`AllocationBid::floor`] — its own
///   authored `min_level`, not the global [`GROUP_LEVEL_MIN`], since #1395.
///   Reading the global here would have the planner spend a point on a group
///   whose hull says it needs none, and refuse a rule that asked for 0. The one
///   qualification is that a bid for a WARM level is guaranteed at least
///   `GROUP_LEVEL_MIN`, so rationing can never be the thing that switches a
///   coldable group off: see [`bid_guarantee`].
/// * What is left over — `max_commanded_total - reserved - each bidder's
///   guarantee` — is the DISCRETIONARY budget, handed out in authored-priority
///   order until it runs out. A group that cannot be paid in full lands as high
///   as the budget reaches, never at a level the applier would refuse.
/// * Each grant is capped by that group's own authored `max_level` as well as
///   by [`GROUP_LEVEL_MAX`], so a bid over the hull's ceiling is trimmed here
///   rather than silently trimmed by the applier and re-emitted.
///
/// The returned total therefore never RISES above the budget, and fits outright
/// whenever the reactor was already inside it — which is every shipped hull.
/// Nothing the plan emits can be refused, and a settled ship stops emitting
/// entirely.
///
/// The qualifier is real rather than defensive. [`PowerSystem::from_authored_groups`]
/// clamps each group to `[GROUP_LEVEL_MIN, GROUP_LEVEL_MAX]` but never checks
/// their SUM, and nothing validates the `[power_groups.*] default_level` total
/// at load either — so a hull authoring five groups at `default_level = 2`
/// spawns already commanded to 10. Whether this function can recover from that
/// turns on who bids. If every group bids it can: `reserved` is 0, five
/// minimums leave three discretionary points, and the plan lands on 8. If the
/// groups holding the overspend do NOT bid it cannot — four un-bid groups at 2
/// make `reserved + mins` 9 between them, `spare` saturates to `0`, the one
/// bidder is planned at [`GROUP_LEVEL_MIN`], and the total stays at 9. The
/// policy authored no verb for those four, and nothing here licenses cutting a
/// group its own hull never offered up.
///
/// The SAFETY property survives that intact — a plan carrying no increase has
/// nothing for the applier to refuse and nothing to re-emit next arm — but
/// "fits" is not the word for it. Adding the load-time guard the stronger claim
/// assumes is a separate piece of work.
///
/// # The AI never raises a cold group
///
/// **Decision (issue #1395).** A group at level 0 is not a group running low;
/// it is a group somebody switched OFF, and switching it back on is an order,
/// not a default. So a bid for a cold group is dropped here and the group falls
/// into the reserved set at 0, costing the budget nothing.
///
/// Without this the feature does not exist. Every Alliance hull ships a
/// priority-0 weapons rule in `fragments/ai/fleet_baseline.toml` that bids
/// level 2 on any tick the battery is healthy — an unconditional baseline, not
/// a reaction to anything — so a cold weapons group would be planned straight
/// back to 2 on the next decision arm, whoever had cold it and however
/// deliberately. Restraint would last one tick.
///
/// Nothing is stranded by it: warming a group is a COMMAND, and commands do not
/// come through the planner. A human Power officer's `SetPowerGroupAllocation`
/// and a scenario script's power order both reach
/// [`PowerSystem::set_group_allocation`] directly through
/// `ship::power::handle_power_messages`, which reads the group's floor and not
/// this function. The rule is that the AI's standing policy may not undo a cold
/// order, not that a cold group can never be warmed.
///
/// And the planner cannot walk a group into that state behind the rule's back:
/// see [`bid_guarantee`] for the other half, which keeps rationing from ever
/// landing a warm bid on 0.
///
/// # Why the order of the returned commands matters
///
/// `ship::power::handle_power_messages` applies admitted commands one at a
/// time, and [`PowerSystem::increase`] tests the budget against the total AT
/// THAT MOMENT. A plan that ends at exactly the cap can still be refused
/// halfway through if an increase is applied before the decrease that pays for
/// it. So every decrease is returned first: after them the total is at its
/// lowest, and the increases then climb monotonically to a final total already
/// known to fit. No-ops (a group already commanded to its planned level) are
/// dropped, which is what keeps admission quiet on a settled ship.
pub fn plan_allocation(power: &PowerSystem, bids: &[AllocationBid]) -> Vec<(PowerGroupId, u8)> {
    // Bids for groups the reactor does not track would be rejected by
    // `set_group_allocation` as `UnknownGroup`; dropping them here keeps them
    // out of the budget arithmetic too. A bid for a COLD group is dropped for
    // the reason the doc comment states: the group then holds at 0 through the
    // reservation path below, at no cost to the budget.
    let mut ranked: Vec<&AllocationBid> = bids
        .iter()
        .filter(|b| power.has_group(&b.group) && !power.is_group_cold(&b.group))
        .collect();

    // Groups nothing bid for hold what they were last commanded to, and that
    // holding costs budget.
    let reserved: u16 = power
        .order
        .iter()
        .filter(|id| !ranked.iter().any(|b| &b.group == *id))
        .map(|id| power.commanded_level_for(id) as u16)
        .sum();

    // Authored order: rule priority (higher wins). `sort_by` is stable, so a
    // tie on priority falls back to the caller's order, which is
    // `PowerSystem::iter()`'s. That fallback is determinism, not a design
    // opinion: it only decides between groups the hull has ranked identically.
    ranked.sort_by_key(|b| std::cmp::Reverse(b.rule_priority));

    // What each bid is GUARANTEED, off that group's own authored floor rather
    // than off one global number: what a group costs the budget just for being
    // in the running is what its hull says its lowest legal level is.
    let mins: u16 = ranked.iter().map(|b| bid_guarantee(b) as u16).sum();
    let mut spare = (power.max_commanded_total as u16).saturating_sub(reserved + mins);

    let mut planned: Vec<(PowerGroupId, u8)> = Vec::with_capacity(ranked.len());
    for bid in ranked {
        let guaranteed = bid_guarantee(bid);
        let asked = (bid_want(bid) - guaranteed) as u16;
        let granted = asked.min(spare);
        spare -= granted;
        planned.push((bid.group.clone(), guaranteed + granted as u8));
    }

    // Decreases first (see the doc comment): both halves keep the authored
    // order within themselves because `partition` is stable.
    let (down, up): (Vec<_>, Vec<_>) = planned
        .into_iter()
        .filter(|(id, level)| *level != power.commanded_level_for(id))
        .partition(|(id, level)| *level < power.commanded_level_for(id));
    down.into_iter().chain(up).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;

    #[test]
    fn continuation_retains_existing_wire_fields_and_allocation_order() {
        let wire = r#"(allocations:[("weapons",3),("helm",1),("shields",2)],battery_charge:12.5,locked:true)"#;
        let saved: PowerState = ron::from_str(wire).unwrap();
        let mut reactor = PowerSystem::default();
        reactor.restore_continuation(&saved);
        assert_eq!(reactor.capture_continuation(), saved);
        assert_eq!(
            ron::to_string(&reactor.capture_continuation()).unwrap(),
            wire
        );
        let old_default: PowerState = ron::from_str("(battery_charge:0.0)").unwrap();
        assert_eq!(old_default, PowerState::default());
    }

    #[test]
    fn continuation_replaces_bootstrap_and_preserves_exhaustion_and_recovery_ticks() {
        let config = PowerConfig::default();
        for (allocations, charge, locked) in [
            (
                vec![(weapons(), 4), (helm(), 3), (shields(), 1)],
                0.001,
                false,
            ),
            (vec![(shields(), 1), (weapons(), 1), (helm(), 1)], 0.0, true),
            (
                vec![(helm(), 2), (weapons(), 2), (shields(), 2)],
                config.capacity,
                false,
            ),
        ] {
            let mut live = PowerSystem::new(&config);
            live.restore(&allocations, charge, locked);
            let saved = live.capture_continuation();
            let mut resumed = PowerSystem::new(&config);
            resumed.restore(
                &[(helm(), 4), (shields(), 4), (weapons(), 4)],
                13.0,
                !locked,
            );
            assert_ne!(resumed.capture_continuation(), saved);
            resumed.restore_continuation(&saved);
            assert_eq!(resumed.capture_continuation(), saved);
            for _ in 0..200 {
                assert_eq!(resumed.tick(0.1, &config), live.tick(0.1, &config));
                assert_eq!(resumed.capture_continuation(), live.capture_continuation());
            }
        }
    }

    fn helm() -> PowerGroupId {
        PowerGroupId(HELM_POWER_GROUP.into())
    }
    fn weapons() -> PowerGroupId {
        PowerGroupId(WEAPONS_POWER_GROUP.into())
    }
    fn shields() -> PowerGroupId {
        PowerGroupId(SHIELDS_POWER_GROUP.into())
    }

    /// A config with a battery that never moves on its own, for tests that
    /// want to place the charge by hand and tick once.
    fn still_config() -> PowerConfig {
        PowerConfig {
            rates: [0.0; 6],
            ..PowerConfig::default()
        }
    }

    #[test]
    fn defaults() {
        let ps = PowerSystem::default();
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&shields()), 2);
        assert_eq!(ps.battery_charge, 100.0);
    }

    #[test]
    fn increase_helm() {
        let mut ps = PowerSystem::default();
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 3);
    }

    #[test]
    fn increase_weapons() {
        let mut ps = PowerSystem::default();
        ps.increase(&weapons());
        assert_eq!(ps.level_for(&weapons()), 3);
    }

    #[test]
    fn increase_shields() {
        let mut ps = PowerSystem::default();
        ps.increase(&shields());
        assert_eq!(ps.level_for(&shields()), 3);
    }

    #[test]
    fn increase_at_four_is_noop() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 4);
    }

    #[test]
    fn increase_at_total_cap_eight_is_noop() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.set_group_allocation(&weapons(), 3).unwrap();
        // shields is 2 → total = 8
        assert_eq!(ps.total(), 8);
        ps.increase(&shields());
        assert_eq!(ps.level_for(&shields()), 2);
    }

    /// While the reactor is locked out after a brownout, `increase` is a no-op:
    /// the operator cannot spend power the reserve cannot pay for.
    #[test]
    fn increase_when_locked_is_noop() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        // Flatten the battery: the reactor locks and forces every group to 1.
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert!(ps.locked());
        assert_eq!(ps.total(), 3, "every group slammed to 1");

        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 1, "locked reactor refuses the spend");
    }

    #[test]
    fn decrease_helm() {
        let mut ps = PowerSystem::default();
        ps.decrease(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
    }

    #[test]
    fn decrease_weapons() {
        let mut ps = PowerSystem::default();
        ps.decrease(&weapons());
        assert_eq!(ps.level_for(&weapons()), 1);
    }

    #[test]
    fn decrease_shields() {
        let mut ps = PowerSystem::default();
        ps.decrease(&shields());
        assert_eq!(ps.level_for(&shields()), 1);
    }

    #[test]
    fn decrease_at_one_is_noop() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 1).unwrap();
        ps.decrease(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
    }

    /// While locked, `decrease` is a no-op too: the allocation controls are
    /// frozen outright until the reserve recovers.
    #[test]
    fn decrease_when_locked_is_noop() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert!(ps.locked());
        assert_eq!(
            ps.level_for(&helm()),
            1,
            "helm slammed to 1 by the brownout"
        );

        ps.decrease(&helm());
        assert_eq!(
            ps.level_for(&helm()),
            1,
            "locked reactor refuses the change"
        );
    }

    // ── tick ──────────────────────────────────────────────────────────────

    #[test]
    fn tick_discharges_above_base() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        // total = 8 → rate = -6.0/s
        ps.battery_charge = 100.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 94.0).abs() < 0.001);
    }

    #[test]
    fn alliance_reactor_has_six_free_pips_and_refuses_a_ninth() {
        let config = PowerConfig {
            capacity: 70.0,
            rates: [5.0, 4.0, 3.0, 2.0, -2.0, -5.0],
            sustainable_total: 6,
            max_commanded_total: 8,
            emergency_threshold: 20.0,
        };
        let mut power = PowerSystem::new(&config);
        assert_eq!(power.total(), 6);
        assert!(power.is_charging(&config));

        power.increase(&helm());
        assert_eq!(power.total(), 7);
        assert!(power.is_draining(&config));
        power.increase(&weapons());
        assert_eq!(power.total(), 8);
        assert!(power.is_draining(&config));
        power.increase(&shields());
        assert_eq!(power.total(), 8, "a ninth pip is refused");
    }

    #[test]
    fn tick_recharges_below_base() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 6 → rate = 2.0/s
        ps.battery_charge = 50.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 52.0).abs() < 0.001);
    }

    #[test]
    fn tick_cannot_overcharge_beyond_capacity() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 3 → rate = 6.0/s
        ps.set_group_allocation(&helm(), 1).unwrap();
        ps.set_group_allocation(&weapons(), 1).unwrap();
        ps.set_group_allocation(&shields(), 1).unwrap();
        ps.battery_charge = 99.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 100.0).abs() < 0.001);
    }

    // ── exhaustion lock ───────────────────────────────────────────────────

    /// **Exhaustion.** Nothing degrades until the battery hits zero; then every
    /// group is taken down to 1 in the same instant and the reactor locks. There
    /// is no graceful per-group floor — a player who drains the reserve loses
    /// the lot. Replaces the issue-#952 floor ladder this reverts.
    ///
    /// Down, never up: `exhaustion_forces_one_but_never_raises_a_cold_group`
    /// below covers the direction this one cannot see, since every group here
    /// starts warm.
    #[test]
    fn exhaustion_forces_groups_to_one_and_locks() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.set_group_allocation(&weapons(), 3).unwrap();

        // Above zero: the standing order is untouched, whatever the charge.
        ps.battery_charge = 5.0;
        assert!(!ps.tick(1.0, &config));
        assert_eq!(ps.level_for(&helm()), 3);
        assert_eq!(ps.level_for(&weapons()), 3);
        assert!(!ps.locked());

        // Zero: the whole reactor browns out at once.
        ps.battery_charge = 0.0;
        assert!(ps.tick(1.0, &config), "the lock engaged");
        assert_eq!(ps.level_for(&helm()), 1);
        assert_eq!(ps.level_for(&weapons()), 1);
        assert_eq!(ps.level_for(&shields()), 1);
        assert!(ps.locked());
    }

    // ── Issue #1395: a group its hull authored at min_level 0 ────────────────

    /// The Alliance shape since #1395: weapons may be taken cold, helm and
    /// shields may not.
    fn reactor_with_coldable_weapons() -> PowerSystem {
        PowerSystem::from_authored_groups(
            &PowerConfig::default(),
            &[
                seed(helm(), 2),
                coldable_seed(weapons(), 2),
                seed(shields(), 2),
            ],
        )
    }

    /// **The floor is the group's own.** A hull that authored `min_level = 0`
    /// for weapons can have that group ordered to 0; one that authored 1 for
    /// helm cannot, and the same command against helm stops at 1.
    #[test]
    fn a_group_authored_at_zero_can_be_commanded_cold_and_its_neighbours_cannot() {
        let mut ps = reactor_with_coldable_weapons();
        assert_eq!(ps.floor_for(&weapons()), 0);
        assert_eq!(ps.floor_for(&helm()), GROUP_LEVEL_MIN);

        ps.set_group_allocation(&weapons(), 0).unwrap();
        assert_eq!(ps.level_for(&weapons()), 0);
        assert!(ps.is_group_cold(&weapons()));

        ps.set_group_allocation(&helm(), 0).unwrap();
        assert_eq!(
            ps.level_for(&helm()),
            1,
            "helm authored a floor of 1 and the same order is clamped to it"
        );
        assert!(!ps.is_group_cold(&helm()));
    }

    /// `is_group_cold` is about a group this reactor HAS. `level_for` returns 0
    /// for a group it has never heard of, so the bare level test would read
    /// every hull without a weapons group as having cold weapons — the exact
    /// opposite of what a fire gate wants to hear.
    #[test]
    fn an_untracked_group_is_not_cold() {
        let ps = reactor_with_coldable_weapons();
        let unknown = PowerGroupId("tractor".into());
        assert_eq!(ps.level_for(&unknown), 0);
        assert!(!ps.is_group_cold(&unknown));
        assert_eq!(
            ps.floor_for(&unknown),
            GROUP_LEVEL_MIN,
            "and its floor reads as the conservative default, not as 0"
        );
    }

    /// `decrease` walks a coldable group all the way off, one step at a time,
    /// and stops there.
    #[test]
    fn decrease_stops_at_the_groups_own_floor() {
        let mut ps = reactor_with_coldable_weapons();
        for _ in 0..4 {
            ps.decrease(&weapons());
        }
        assert_eq!(ps.level_for(&weapons()), 0, "weapons authored a floor of 0");

        for _ in 0..4 {
            ps.decrease(&shields());
        }
        assert_eq!(
            ps.level_for(&shields()),
            1,
            "shields authored a floor of 1 and holds there"
        );
    }

    /// **Exhaustion takes power away; it does not hand it out.** The lock still
    /// forces every warm group to 1, but a group the crew switched off stays
    /// off — otherwise a flat battery would be the thing that put the guns back
    /// on, undoing a standing order nobody had cancelled.
    #[test]
    fn exhaustion_forces_one_but_never_raises_a_cold_group() {
        let config = still_config();
        let mut ps = reactor_with_coldable_weapons();
        ps.set_group_allocation(&weapons(), 0).unwrap();
        ps.set_group_allocation(&helm(), 4).unwrap();

        ps.battery_charge = 0.0;
        assert!(ps.tick(1.0, &config), "the lock engaged");
        assert_eq!(ps.level_for(&helm()), 1, "a warm group is forced down to 1");
        assert_eq!(ps.level_for(&shields()), 1);
        assert_eq!(ps.level_for(&weapons()), 0, "a cold group is left cold");
        assert!(ps.is_group_cold(&weapons()));
        assert!(ps.locked());
    }

    /// **The resume bug.** `restore` clamped to the global minimum, so a ship
    /// saved with its weapons cold came back at level 1 with live guns. It now
    /// clamps to the group's own floor.
    #[test]
    fn restore_reinstates_a_cold_group() {
        let mut ps = reactor_with_coldable_weapons();
        assert_eq!(
            ps.level_for(&weapons()),
            2,
            "precondition: the fresh reactor is warm, so what comes back below \
             cannot be a bootstrap coincidence"
        );

        ps.restore(&[(helm(), 3), (weapons(), 0), (shields(), 2)], 40.0, false);

        assert_eq!(ps.level_for(&weapons()), 0);
        assert!(ps.is_group_cold(&weapons()));
        assert_eq!(ps.level_for(&helm()), 3);
        assert_eq!(ps.battery_charge, 40.0);
    }

    /// The floors are the HULL's, not the save's. A payload carrying a level
    /// under a group's authored floor is still lifted to it — the ship the
    /// resume boots is the ship the file describes, and a save cannot re-author
    /// what an officer is allowed to command.
    #[test]
    fn restore_still_lifts_a_group_below_its_own_floor() {
        let mut ps = reactor_with_coldable_weapons();
        ps.restore(&[(helm(), 0), (weapons(), 0), (shields(), 2)], 40.0, false);
        assert_eq!(ps.level_for(&helm()), 1, "helm's authored floor is 1");
        assert_eq!(ps.level_for(&weapons()), 0, "weapons' authored floor is 0");
    }

    /// Recovery: once locked, the reactor stays locked until the charge climbs
    /// back to `emergency_threshold`, and only then do the controls unfreeze.
    #[test]
    fn recovery_unlocks_at_the_emergency_threshold() {
        let config = still_config(); // emergency_threshold 25
        let mut ps = PowerSystem::default();
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert!(ps.locked());

        // Below the threshold: still locked, controls still frozen.
        ps.battery_charge = 20.0;
        assert!(
            !ps.tick(1.0, &config),
            "no edge — still under the threshold"
        );
        assert!(ps.locked());
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 1, "frozen below the threshold");

        // At the threshold: the lock releases.
        ps.battery_charge = 25.0;
        assert!(ps.tick(1.0, &config), "the lock released");
        assert!(!ps.locked());
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 2, "controls live again");
    }

    /// Dropping to 1 across the board lowers the draw, so a bottomed-out reactor
    /// recharges instead of sitting flat for ever.
    #[test]
    fn a_locked_reactor_recharges_on_the_minimum_draw() {
        let config = PowerConfig::default(); // rates[0] (total 3) = +6/s
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap(); // total 8 → -6/s
        ps.battery_charge = 2.0;

        // This tick flattens the battery and locks: groups go to 1, total 3.
        ps.tick(1.0, &config);
        assert_eq!(ps.battery_charge, 0.0);
        assert!(ps.locked());
        assert_eq!(ps.total(), 3);
        // Next tick draws at the locked total of 3 → +6/s.
        ps.tick(1.0, &config);
        assert!(
            (ps.battery_charge - 6.0).abs() < 0.001,
            "got {}",
            ps.battery_charge
        );
    }

    /// `is_charging` is not `!is_draining`: at a rate of exactly zero the
    /// reserve is frozen, and the gauge must claim neither.
    #[test]
    fn a_zero_rate_is_neither_draining_nor_charging() {
        let config = PowerConfig {
            rates: [3.0, 2.0, 1.0, 0.0, -1.0, -3.0],
            ..PowerConfig::default()
        };
        let ps = PowerSystem::default(); // total 6 → rates[3] = 0.0
        assert_eq!(ps.total(), 6);
        assert!(!ps.is_draining(&config));
        assert!(
            !ps.is_charging(&config),
            "a frozen reserve must not paint the pulsing CHARGING indicator"
        );
    }

    /// The tick's return value is the lock-changed edge, which
    /// `ship::power::tick_power_brownout_advisory` hangs its debounce off.
    #[test]
    fn tick_returns_true_when_lock_changes() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.battery_charge = 0.0;
        assert!(ps.tick(1.0, &config), "locked out this tick");
        assert!(!ps.tick(1.0, &config), "still locked — no edge");
        ps.battery_charge = 100.0;
        assert!(ps.tick(1.0, &config), "unlocked this tick");
    }

    // ── configurable constructor ──────────────────────────────────────────

    #[test]
    fn custom_config() {
        let config = PowerConfig {
            capacity: 50.0,
            rates: [1.0, 1.0, 1.0, -1.0, -2.0, -3.0],
            sustainable_total: 5,
            max_commanded_total: 8,
            emergency_threshold: 10.0,
        };
        let ps = PowerSystem::new(&config);
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&shields()), 2);
        assert!((ps.battery_charge - 50.0).abs() < 0.001);
    }

    /// Re-authored from `increase_on_unknown_group_is_noop`, which asserted
    /// that `"shields"` was an unknown group. That is exactly backwards since
    /// issue #952: `shields` IS one of the three canonical groups now, and
    /// `sensors` is the one that no longer exists. Left as it was the test
    /// would have gone on passing while asserting nothing — the increase it
    /// aimed at an "unknown" group would have quietly moved a real one, and the
    /// three follow-up assertions read the two groups it did not touch.
    #[test]
    fn increase_on_unknown_group_is_noop() {
        let mut ps = PowerSystem::default();
        // Neither sensors nor navigation are seeded as power groups.
        ps.increase(&PowerGroupId("sensors".into()));
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&shields()), 2);

        ps.increase(&PowerGroupId("navigation".into()));
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&shields()), 2);
        assert_eq!(ps.total(), 6, "no stray group was created either");
    }

    #[test]
    fn set_group_allocation_updates_named_group() {
        let mut ps = PowerSystem::default();

        ps.set_group_allocation(&weapons(), 3).unwrap();

        assert_eq!(ps.level_for(&weapons()), 3);
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&shields()), 2);
    }

    #[test]
    fn set_group_allocation_rejects_unknown_group() {
        let mut ps = PowerSystem::default();
        let group = PowerGroupId("life-support".into());

        assert_eq!(
            ps.set_group_allocation(&group, 3),
            Err(PowerAllocationError::UnknownGroup(group))
        );
        assert_eq!(ps.total(), 6);
    }

    #[test]
    fn channel_1_read_exposes_power_without_mutation_access() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        let state = ps.read_state();
        let channel_1 = Channel1Read::new(&state);

        assert_eq!(channel_1.power_level(&helm()), Some(4));
        assert_eq!(channel_1.power_level(&PowerGroupId("unknown".into())), None);
        assert_eq!(ps.level_for(&helm()), 4);
    }

    // ── Budget-aware allocation planning (issue #959) ────────────────────────

    fn auxiliary() -> PowerGroupId {
        PowerGroupId("auxiliary".into())
    }

    /// A bid at the shipped fleet's ordering: every elevation rule authored at
    /// priority 10, ties falling back to the caller's group order. The group
    /// authors no floor of its own, so it takes the parse default.
    fn bid(group: PowerGroupId, want: u8, rule_priority: i32) -> AllocationBid {
        AllocationBid {
            group,
            want,
            max_level: crate::ship::config::default_max_power_level(),
            floor: GROUP_LEVEL_MIN,
            rule_priority,
        }
    }

    /// The same bid for a group whose hull authored `min_level = 0` — one that
    /// may be taken cold (issue #1395).
    fn coldable_bid(group: PowerGroupId, want: u8, rule_priority: i32) -> AllocationBid {
        AllocationBid {
            floor: 0,
            ..bid(group, want, rule_priority)
        }
    }

    /// A seed entry for a group that authors no floor.
    fn seed(group: PowerGroupId, level: u8) -> AuthoredPowerGroup {
        AuthoredPowerGroup::at_default_floor(group, level)
    }

    /// A seed entry for a group whose hull authored `min_level = 0`.
    fn coldable_seed(group: PowerGroupId, level: u8) -> AuthoredPowerGroup {
        AuthoredPowerGroup {
            id: group,
            level,
            floor: 0,
        }
    }

    /// The four-group Alliance shape: `ops` outside the canonical trio, seeded
    /// at 1, with nothing in the policy bidding for it.
    fn reactor_with_auxiliary_group() -> PowerSystem {
        PowerSystem::from_authored_groups(
            &PowerConfig::default(),
            &[
                seed(helm(), 2),
                seed(weapons(), 2),
                seed(shields(), 1),
                seed(auxiliary(), 1),
            ],
        )
    }

    /// **Allocate within the max.** The shipped combat-stations allocation —
    /// helm 3 / weapons 3 against `ops` 1 and `shields` 1 — spends the budget
    /// exactly, and every planned level is inside the group's own ceiling.
    #[test]
    fn plan_allocation_spends_the_budget_without_exceeding_it() {
        let ps = reactor_with_auxiliary_group();
        let plan = plan_allocation(&ps, &[bid(helm(), 3, 10), bid(weapons(), 3, 10)]);

        let mut after = ps.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(after.commanded_level_for(&helm()), 3);
        assert_eq!(after.commanded_level_for(&weapons()), 3);
        assert_eq!(after.commanded_total(), ps.max_commanded_total());
        assert!(
            plan.iter().all(|(_, l)| *l <= GROUP_LEVEL_MAX),
            "no planned level may exceed the per-group ceiling"
        );
    }

    /// A bid over the group's OWN authored `max_level` is trimmed by the
    /// planner, not by the applier. Trimming it downstream is what made the
    /// difference invisible: the applier's clamp is silent, so the decider went
    /// on asking for a level the hull had already ruled out.
    #[test]
    fn plan_allocation_trims_a_bid_to_the_groups_authored_max_level() {
        let mut ps = reactor_with_auxiliary_group();
        ps.set_group_allocation(&weapons(), 1).unwrap();
        let capped = AllocationBid {
            max_level: 2,
            ..bid(weapons(), 4, 10)
        };
        assert_eq!(
            plan_allocation(&ps, std::slice::from_ref(&capped)),
            vec![(weapons(), 2)],
            "the bid asked for 4 and the hull's own max_level is 2"
        );

        // And once it is there, the trimmed bid settles: nothing further is
        // planned, so the ceiling cannot become a re-emit loop of its own.
        ps.set_group_allocation(&weapons(), 2).unwrap();
        assert!(plan_allocation(&ps, &[capped]).is_empty());
    }

    /// **Budget collision.** Three groups asking for more than the reactor can
    /// pay for are rationed in AUTHORED priority order, and the plan still
    /// fits: the top-priority bid is paid in full, the next takes what is left,
    /// the last lands on its minimum. Nothing is refused by the applier.
    #[test]
    fn plan_allocation_rations_a_budget_collision_by_authored_priority() {
        let ps = reactor_with_auxiliary_group();
        let plan = plan_allocation(
            &ps,
            &[
                bid(helm(), 4, 5),
                bid(weapons(), 4, 30),
                bid(shields(), 4, 20),
            ],
        );

        let mut after = ps.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        // ops holds 1 (nothing bid for it); 7 points are left for three groups
        // that each need at least 1, so 4 are discretionary: weapons (30) takes
        // 3, shields (20) takes the last 1, helm (5) gets none.
        assert_eq!(after.commanded_level_for(&weapons()), 4);
        assert_eq!(after.commanded_level_for(&shields()), 2);
        assert_eq!(after.commanded_level_for(&helm()), 1);
        assert_eq!(
            after.commanded_level_for(&auxiliary()),
            1,
            "un-bid groups are reserved"
        );
        assert_eq!(after.commanded_total(), ps.max_commanded_total());
        // And every level the plan asked for is the level the reactor actually
        // holds — the whole point: no silent refusal anywhere in the plan.
        for (group, level) in &plan {
            assert_eq!(
                after.commanded_level_for(group),
                *level,
                "{} was planned at {level} and the applier refused it",
                group.0
            );
        }
    }

    /// An equal-priority tie falls back to the caller's deterministic group
    /// order (`POWER_GROUP_ORDER`: helm before weapons), not to any preference
    /// baked into the planner. With the battery floors reverted there is no
    /// secondary authored key left, so this fallback is the whole tie-break.
    #[test]
    fn plan_allocation_breaks_a_priority_tie_on_the_callers_group_order() {
        let ps = reactor_with_auxiliary_group();
        // Both at priority 10, both asking for 4, only 4 discretionary points.
        let plan = plan_allocation(&ps, &[bid(helm(), 4, 10), bid(weapons(), 4, 10)]);
        let mut after = ps.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(
            after.commanded_level_for(&helm()),
            4,
            "helm precedes weapons in POWER_GROUP_ORDER, so the tie serves it first"
        );
        assert_eq!(after.commanded_level_for(&weapons()), 2);
    }

    /// **No re-emit stall.** Re-planning against the reactor the previous plan
    /// produced returns NOTHING — the decision has settled, so the host emits
    /// nothing and admission stays quiet. This is the invariant the old
    /// per-group emit could not hold: its refused command was re-issued on
    /// every decision arm for ever.
    #[test]
    fn plan_allocation_settles_and_stops_emitting() {
        let mut ps = reactor_with_auxiliary_group();
        let bids = [
            bid(helm(), 4, 10),
            bid(weapons(), 4, 10),
            bid(shields(), 4, 10),
        ];

        let first = plan_allocation(&ps, &bids);
        assert!(!first.is_empty(), "the first arm has work to do");
        for (group, level) in &first {
            ps.set_group_allocation(group, *level).unwrap();
        }
        assert!(ps.commanded_total() <= ps.max_commanded_total());

        for arm in 0..5 {
            let again = plan_allocation(&ps, &bids);
            assert!(
                again.is_empty(),
                "arm {arm} re-emitted {again:?} after the allocation had settled"
            );
        }
    }

    /// Decreases are ordered ahead of increases, because the applier tests the
    /// budget one command at a time. A plan that ends at the cap is refused
    /// halfway through if the increase lands before the decrease that pays for
    /// it — which is the silent refusal wearing a different hat.
    #[test]
    fn plan_allocation_orders_decreases_before_the_increases_they_pay_for() {
        // Commanded at the cap already: helm 4 / weapons 2 / shields 1 / ops 1.
        let mut ps = reactor_with_auxiliary_group();
        ps.set_group_allocation(&helm(), 4).unwrap();
        assert_eq!(ps.commanded_total(), ps.max_commanded_total());

        // The policy now wants the two swapped over.
        let plan = plan_allocation(&ps, &[bid(helm(), 2, 10), bid(weapons(), 4, 20)]);
        assert_eq!(
            plan.first().map(|(g, l)| (g.0.as_str(), *l)),
            Some((HELM_POWER_GROUP, 2)),
            "the decrease must come first: {plan:?}"
        );

        // Apply in the planned order through the real applier semantics.
        for (group, level) in &plan {
            ps.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(
            ps.commanded_level_for(&weapons()),
            4,
            "the increase was paid for"
        );
        assert_eq!(ps.commanded_level_for(&helm()), 2);
    }

    /// A group the reactor does not track is dropped rather than charged to the
    /// budget — the applier would reject it as `UnknownGroup`, and counting it
    /// would starve a real group of a point that was never spent.
    #[test]
    fn plan_allocation_ignores_a_bid_for_an_untracked_group() {
        let ps = reactor_with_auxiliary_group();
        let plan = plan_allocation(
            &ps,
            &[
                bid(PowerGroupId("life-support".into()), 4, 100),
                bid(weapons(), 4, 10),
            ],
        );
        assert_eq!(plan, vec![(weapons(), 4)]);
    }

    // ── Issue #1395: the planner and a coldable group ────────────────────────

    /// **The AI never raises a cold group.** Every Alliance hull ships an
    /// unconditional priority-0 weapons rule bidding level 2; without this the
    /// planner would warm a cold weapons group on the very next decision arm
    /// and restraint would last one tick.
    #[test]
    fn plan_allocation_never_raises_a_cold_group() {
        let mut ps = reactor_with_coldable_weapons();
        ps.set_group_allocation(&weapons(), 0).unwrap();

        let plan = plan_allocation(&ps, &[coldable_bid(weapons(), 2, 0), bid(helm(), 2, 10)]);
        assert!(
            !plan.iter().any(|(id, _)| id == &weapons()),
            "the cold group is not in the plan at all: {plan:?}"
        );
        assert!(plan.is_empty(), "and helm was already at 2: {plan:?}");
    }

    /// The freed point is genuinely freed. A cold group is reserved at 0 rather
    /// than at the global minimum, so the rest of the reactor may spend what it
    /// is not using — helm to its ceiling on a budget that could not otherwise
    /// have paid for it.
    #[test]
    fn a_cold_group_hands_its_point_back_to_the_budget() {
        let mut ps = reactor_with_coldable_weapons();
        ps.set_group_allocation(&weapons(), 0).unwrap();

        let plan = plan_allocation(&ps, &[bid(helm(), 4, 10), bid(shields(), 4, 10)]);
        let mut after = ps.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(after.level_for(&helm()), 4);
        assert_eq!(after.level_for(&shields()), 4);
        assert_eq!(after.level_for(&weapons()), 0);
        assert_eq!(
            after.commanded_total(),
            8,
            "the whole 8-point budget went to the two warm groups"
        );
    }

    /// **The other half of the rule: the AI never PARKS a group cold either.**
    ///
    /// Rationing hands a group as much as the budget reaches, and for a group
    /// whose floor is 0 that could reach zero. Here helm and shields hold the
    /// whole 8-point budget between them — the over-authored shape the test
    /// below documents, and the only way a bidder's discretionary share can
    /// saturate to nothing — and weapons bids 3 with not a point left to pay
    /// for it. It holds the 1 it has rather than being cut to 0. Otherwise a
    /// busy reactor would switch the guns off, and the cold rule above would
    /// then keep them off for the rest of the encounter.
    #[test]
    fn plan_allocation_rations_a_coldable_group_down_to_one_not_to_zero() {
        let ps = PowerSystem::from_authored_groups(
            &PowerConfig::default(),
            &[
                seed(helm(), 4),
                coldable_seed(weapons(), 1),
                seed(shields(), 4),
            ],
        );
        assert_eq!(ps.commanded_total(), 9, "deliberately over the 8-point cap");

        let plan = plan_allocation(&ps, &[coldable_bid(weapons(), 3, 0)]);
        assert_eq!(
            plan,
            vec![],
            "no spare to grant, and the guarantee holds weapons at the 1 it \
             already has rather than cutting it to 0: {plan:?}"
        );
    }

    /// A rule that ASKS for 0 is served, because that is an authored decision
    /// rather than a rounding outcome — and it costs the budget nothing, so the
    /// point it gives up is available to the group bidding beside it.
    #[test]
    fn plan_allocation_serves_a_rule_that_bids_a_group_cold() {
        let ps = reactor_with_coldable_weapons();
        let plan = plan_allocation(&ps, &[coldable_bid(weapons(), 0, 10), bid(helm(), 4, 5)]);
        assert_eq!(
            plan,
            vec![(weapons(), 0), (helm(), 4)],
            "the decrease is ordered first, as ever, and helm gets the point"
        );
    }

    /// A group whose hull authored a floor ABOVE the global minimum is
    /// guaranteed its own floor, not 1 — the same rule read from the other end.
    /// Helm here holds 4 and shields 3, leaving one discretionary point for an
    /// `ops` group authored at `min_level = 2` bidding 4: it lands on 3.
    #[test]
    fn plan_allocation_guarantees_a_group_its_own_raised_floor() {
        let mut ps = PowerSystem::from_authored_groups(
            &PowerConfig::default(),
            &[
                seed(helm(), 2),
                seed(shields(), 2),
                AuthoredPowerGroup {
                    id: auxiliary(),
                    level: 2,
                    floor: 2,
                },
            ],
        );
        ps.set_group_allocation(&helm(), 4).unwrap();
        assert_eq!(ps.commanded_total(), 8);

        let raised = AllocationBid {
            floor: 2,
            ..bid(auxiliary(), 4, 10)
        };
        let plan = plan_allocation(&ps, &[raised]);
        assert_eq!(
            plan,
            vec![],
            "helm 4 + shields 2 reserved is 6; ops is guaranteed its own floor of \
             2, which is exactly what it already holds: {plan:?}"
        );

        ps.set_group_allocation(&shields(), 1).unwrap();
        let raised = AllocationBid {
            floor: 2,
            ..bid(auxiliary(), 4, 10)
        };
        let plan = plan_allocation(&ps, &[raised]);
        assert_eq!(
            plan,
            vec![(auxiliary(), 3)],
            "one point freed, one point granted on top of the authored floor"
        );
    }

    /// **The qualifier on "the returned total fits."** A hull whose
    /// `[power_groups.*] default_level` values already sum past
    /// the authored `max_commanded_total` is not something this function can undo, and
    /// nothing rejects that authoring at load — so the doc claim is "never
    /// rises above the budget", not "always fits", and this is the case that
    /// makes the difference.
    ///
    /// What IS guaranteed on such a reactor is that the plan never makes the
    /// overspend worse: `reserved + mins` is already over budget, `spare`
    /// saturates to 0, and the plan carries decreases only — so the applier has
    /// nothing to refuse and there is nothing to re-emit next arm.
    #[test]
    fn plan_allocation_cannot_rescue_a_reactor_authored_over_its_own_budget() {
        let life_support = PowerGroupId("life-support".into());
        // Five groups seeded at 2: an authoring nothing rejects, and a commanded
        // total of 10 against a budget of 8.
        let over = PowerSystem::from_authored_groups(
            &PowerConfig::default(),
            &[
                seed(helm(), 2),
                seed(weapons(), 2),
                seed(shields(), 2),
                seed(auxiliary(), 2),
                seed(life_support.clone(), 2),
            ],
        );
        assert!(over.commanded_total() > over.max_commanded_total());

        // Only helm bids. The other four hold 8 between them, so there is no
        // discretionary budget at all and nothing licenses cutting them.
        let plan = plan_allocation(&over, &[bid(helm(), 4, 10)]);
        assert_eq!(plan, vec![(helm(), GROUP_LEVEL_MIN)]);
        assert!(
            plan.iter()
                .all(|(id, level)| *level <= over.commanded_level_for(id)),
            "an over-budget reactor must never be handed an increase"
        );

        let mut after = over.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(
            after.commanded_total(),
            9,
            "the plan lowers the overspend as far as it may and no further"
        );

        // The other arm, and the reason the claim is conditional rather than
        // simply false: when EVERY group bids there is nothing held outside the
        // plan, and the same reactor does land inside the budget.
        let all_bid: Vec<AllocationBid> =
            over.iter().map(|(id, _)| bid(id.clone(), 4, 10)).collect();
        let mut after_all = over.clone();
        for (group, level) in plan_allocation(&over, &all_bid) {
            after_all.set_group_allocation(&group, level).unwrap();
        }
        assert_eq!(after_all.commanded_total(), after_all.max_commanded_total());
    }
}
