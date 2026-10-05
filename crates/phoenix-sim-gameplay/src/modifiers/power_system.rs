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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike_boost: Option<crate::modifiers::strike_reserve::StrikeBoost>,
    /// `(power group id, level)` in reactor insertion order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allocations: Vec<(String, u8)>,
    pub battery_charge: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

pub const HELM_POWER_GROUP: &str = "helm";

pub const SHIELDS_POWER_GROUP: &str = "shields";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerAllocationError {
    UnknownGroup(PowerGroupId),
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
    reserve_group: Option<PowerGroupId>,
    strike_boost: Option<crate::modifiers::strike_reserve::StrikeBoost>,
    strike_weapons:
        std::collections::BTreeMap<String, crate::modifiers::strike_reserve::StrikeWeaponConfig>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrikeReserveConfig {
    /// Basic Backfill re-enable threshold in stored charge units. None leaves
    /// boost decisions with an operator; spending always uses the same command.
    #[serde(default)]
    pub ai_enable_at: Option<f32>,
    pub group: String,
    pub units_per_level: f32,
    #[serde(default)]
    pub weapons:
        std::collections::BTreeMap<String, crate::modifiers::strike_reserve::StrikeWeaponConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerConfig {
    /// An explicitly allocated group stores generation for attacks.
    pub strike_reserve: Option<StrikeReserveConfig>,
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
            strike_reserve: None,
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
            strike_boost: config.strike_reserve.as_ref().map(|_| Default::default()),
            strike_weapons: config
                .strike_reserve
                .as_ref()
                .map(|r| r.weapons.clone())
                .unwrap_or_default(),
            reserve_group: config
                .strike_reserve
                .as_ref()
                .map(|r| PowerGroupId(r.group.clone())),
            battery_charge: if config.strike_reserve.is_some() {
                0.0
            } else {
                config.capacity
            },
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
            strike_boost: config.strike_reserve.as_ref().map(|_| Default::default()),
            strike_weapons: config
                .strike_reserve
                .as_ref()
                .map(|r| r.weapons.clone())
                .unwrap_or_default(),
            reserve_group: config
                .strike_reserve
                .as_ref()
                .map(|r| PowerGroupId(r.group.clone())),
            battery_charge: if config.strike_reserve.is_some() {
                0.0
            } else {
                config.capacity
            },
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
            strike_boost: self.strike_boost.clone(),
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
        if self.reserve_group.is_some() {
            self.strike_boost = Some(saved.strike_boost.clone().unwrap_or_default());
        }
    }

    pub fn strike_boost(&self) -> Option<&crate::modifiers::strike_reserve::StrikeBoost> {
        self.strike_boost.as_ref()
    }

    pub fn strike_read(
        &self,
        config: &PowerConfig,
    ) -> Option<crate::core::messages::StrikeReserveBlackboard> {
        self.strike_boost()
            .map(|boost| crate::core::messages::StrikeReserveBlackboard {
                charge: self.battery_charge,
                capacity: config.capacity,
                charging: self.is_charging(config),
                enabled: boost.enabled,
                depleted: boost.depleted,
            })
    }

    pub fn set_strike_boost(&mut self, enabled: bool) -> bool {
        let Some(boost) = self.strike_boost.as_mut() else {
            return false;
        };
        boost.set(enabled);
        true
    }

    /// Call once at the actual attack boundary, after every no-fire gate.
    pub fn fire_strike_weapon(&mut self, system_id: &str) -> f32 {
        let Some(boost) = self.strike_boost.as_mut() else {
            return 1.0;
        };
        boost.fire(&mut self.battery_charge, self.strike_weapons.get(system_id))
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
        if let Some(reserve) = &config.strike_reserve {
            return f32::from(self.level_for(&PowerGroupId(reserve.group.clone())))
                * reserve.units_per_level;
        }
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
        if config.strike_reserve.is_some() {
            self.battery_charge = (self.battery_charge
                + self.battery_rate(config).max(0.0) * dt.max(0.0))
            .clamp(0.0, config.capacity);
            return false;
        }
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
/// The authored reserve charging group is not equipment. Zero means no
/// generation assigned to storage, so its own Backfill rule may restart it.
/// The exception is identified by the authored policy, never a hull name.
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
        // Zero charging is not cold equipment: Backfill may explicitly restart
        // storage through its authored allocation rule after a full reserve.
        .filter(|b| {
            power.has_group(&b.group)
                && (!power.is_group_cold(&b.group)
                    || power.reserve_group.as_ref() == Some(&b.group))
        })
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
#[path = "power_system_tests.rs"]
mod tests;

pub use phoenix_sim_contracts::power::*;
