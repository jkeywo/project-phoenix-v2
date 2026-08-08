use crate::messages::PowerGroupId;
use std::collections::HashMap;

pub const HELM_POWER_GROUP: &str = "helm";
pub const WEAPONS_POWER_GROUP: &str = "weapons";
pub const SHIELDS_POWER_GROUP: &str = "shields";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerAllocationError {
    UnknownGroup(PowerGroupId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerReadState {
    pub allocations: Vec<(PowerGroupId, u8)>,
    pub battery_charge: f32,
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
/// reach from [`crate::messages::ModifierSlot::RadarRange`], and the per-group
/// battery-floor ladder this issue introduces needs a group whose floor is
/// worth holding when everything else has been cut — which is shields, not a
/// radar horizon.
pub const POWER_GROUP_ORDER: &[&str] =
    &[HELM_POWER_GROUP, WEAPONS_POWER_GROUP, SHIELDS_POWER_GROUP];

/// The battery floor authored for one power group (issue #952).
///
/// `battery_pct` is a percentage of the reactor's own `capacity`, NOT an
/// absolute charge: a hull with `capacity = 90` and a `helm` floor of `50`
/// starts cutting helm at 45 charge. (Contrast [`PowerConfig::emergency_threshold`],
/// which has always been compared against `battery_charge` in absolute units.)
///
/// `min_level` is where the group lands while it is under its floor — the
/// group's own authored `[power_groups.<id>] min_level`.
///
/// A floor is a TWO-threshold shape, not one: it engages at `battery_pct` and
/// releases at `battery_pct + `[`PowerConfig::floor_release_margin_pct`]. See
/// that field for why a single threshold cannot work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PowerGroupFloor {
    /// Battery percentage (0–100) below which this group is held down.
    pub battery_pct: f32,
    /// Level the group is held at while under its floor.
    pub min_level: u8,
}

/// Pure `PowerSystem` state — keyed by [`PowerGroupId`] after issue #617.
///
/// The three canonical groups (`helm`, `weapons`, `shields`) are seeded at
/// construction so tests can rely on `level_for` returning `Some(2)` without
/// first calling `set_group_allocation`. Additional groups can be added by
/// TOML-driven config in future PRs.
///
/// # Commanded vs. effective level (issue #952)
///
/// `groups` holds what the reactor has been TOLD to run each group at — by a
/// human Power operator or by `ai_power_allocation`, through the one admitted
/// `SetPowerGroupAllocation` applier. `floored` holds the groups the battery is
/// currently too flat to sustain, at the level they are being held down to.
/// [`Self::level_for`] (and therefore every modifier, blackboard, and wire
/// snapshot) reads the EFFECTIVE level: the floor when one applies, the
/// commanded level otherwise.
///
/// Keeping the commanded level intact is what makes recovery free: nothing has
/// to re-issue a command when the battery climbs back over a group's floor —
/// the next `tick` simply stops holding it down and the group is back at what
/// its operator last asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerSystem {
    /// Per-group COMMANDED allocation level. Values are clamped to `[1, 4]` by
    /// the setter API; direct construction should preserve that invariant.
    groups: HashMap<PowerGroupId, u8>,
    /// Insertion order of `groups`; walked by publishers so wire output is
    /// deterministic even when the HashMap iteration order isn't.
    order: Vec<PowerGroupId>,
    /// Groups whose floor is currently ENGAGED — the battery-side half of the
    /// state, and the half the hysteresis lives in. A group enters when the
    /// charge falls under its `battery_pct` and leaves only when the charge
    /// climbs past `battery_pct + `[`PowerConfig::floor_release_margin_pct`].
    ///
    /// Deliberately separate from `floored` below. Engagement is a fact about
    /// the RESERVE; being held down is a fact about the reserve *and* the
    /// standing order. Fusing them would lose the hysteresis exactly where it
    /// is needed most: a group whose commanded level happens to sit at its own
    /// landing level is not "held", so on a fused set it would leave the band
    /// and be released at the bare threshold the moment its operator (human or
    /// policy) asked for more — which on the shipped hulls is precisely the
    /// boundary the AI reserve guards re-elevate at.
    under_floor: std::collections::HashSet<PowerGroupId>,
    /// Groups currently held below their commanded level because their floor is
    /// engaged, mapped to the level they are held at. Recomputed from scratch by
    /// every [`Self::tick`] as `under_floor` ∩ "the order asks for more".
    floored: HashMap<PowerGroupId, u8>,
    pub battery_charge: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerConfig {
    pub capacity: f32,
    pub rates: [f32; 6],
    /// Emergency reserve marker, in the same ABSOLUTE units as `capacity`.
    /// Published on `PowerBatteryBlackboard` (as a fraction of capacity) so the
    /// battery gauge can paint the reserve band. Since issue #952 removed the
    /// brownout lock it no longer gates anything in the simulation — the
    /// per-group [`PowerGroupFloor`] ladder decides who keeps power.
    pub emergency_threshold: f32,
    /// Per-group battery floors, keyed by power-group id (issue #952). A group
    /// with no entry here is never held down: it keeps its commanded level all
    /// the way to a flat battery.
    pub group_floors: HashMap<String, PowerGroupFloor>,
    /// How far above its own [`PowerGroupFloor::battery_pct`] the charge has to
    /// climb before a floored group is released, in the same percentage-of-
    /// capacity units. Authored as `[power] battery_floor_release_margin`.
    ///
    /// # Why a single threshold cannot work
    ///
    /// Flooring a group LOWERS the effective total, and the effective total is
    /// what [`Self::rates`] is indexed by — so the cut very often flips the
    /// battery rate's sign. A destroyer commanded `ops 1 / helm 3 / weapons 2 /
    /// shields 1 = 7` draws −2/s; cutting helm at 40 % leaves 6, which CHARGES
    /// at +2/s, which immediately puts the charge back over 40 %, which releases
    /// helm, which returns the draw to 7. With one threshold that loop runs
    /// once per FIXED TICK: `MaxSpeed`, `PhaserDamage` and
    /// [`Self::is_draining`] all toggle at 60 Hz, and every consumer that
    /// debounces on one of them — `ship::power::tick_power_brownout_advisory`
    /// most visibly — re-fires on alternating ticks. The pre-#952 model could
    /// not do this because it indexed `rates` off the COMMANDED total, which
    /// only ever moved at operator or AI cadence.
    ///
    /// The margin turns that into a relaxation cycle whose period the designer
    /// sets: the group is cut at the floor and released only at
    /// `floor + margin`, so the reserve has to genuinely recover the band
    /// before the draw comes back. It is the shape the retired brownout lock's
    /// `emergency_threshold` bought, restored per group.
    ///
    /// It is also what lets a floor be authored AT (or under) the same group's
    /// `[power.ai_policy.param] min_reserve_*` without the two racing: the
    /// policy re-elevates the moment the charge crosses its reserve, and the
    /// reactor keeps holding the group down through the margin band above it.
    pub floor_release_margin_pct: f32,
}

/// The level a group lands on when its hull describes no `[power_groups.*]`
/// entry for it — the level the runtime SEEDS such a group at
/// (`ship::config::default_power_level`), and the ×1.0 rung on every shipped
/// multiplier table.
///
/// A brownout takes back what was SPENT, not the system. Landing an
/// unauthored group on 1 instead would be inventing a debuff the hull never
/// wrote down, and it would put every AI-crewed NPC permanently below nominal
/// the moment a fight drained its reserve — a fleet-wide balance change wearing
/// a brownout's clothes. A hull that does want a group cut to 1 says so, by
/// authoring `[power_groups.<id>] min_level = 1`; the four Alliance hulls do
/// exactly that for `shields`.
///
/// Defined by CALLING [`crate::ship::config::default_power_level`] rather than
/// restating its value, so the two cannot drift: the whole justification above
/// is "the level the runtime seeds such a group at", which is that function's
/// return and nothing else.
pub const UNAUTHORED_FLOOR_LEVEL: u8 = crate::ship::config::default_power_level();

/// The lowest level any power group can be commanded to. Defined by CALLING
/// [`crate::ship::config::default_min_power_level`] — the parse default a
/// `[power_groups.<id>] min_level` gets — so the setter API's lower clamp and
/// the authoring default cannot drift apart.
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
pub const MAX_COMMANDED_TOTAL: u8 = 8;

/// The lowest total [`PowerConfig::rates`] describes a rate for: the table's
/// first entry is the battery rate at an EFFECTIVE total of `3` and its last is
/// the rate at [`MAX_COMMANDED_TOTAL`], which is why every hull authors exactly
/// six of them.
///
/// A DIFFERENT quantity from [`GROUP_LEVEL_MIN`], and not a budget of any kind:
/// it is the authoring shape of the `rates` table, the number the index is
/// offset by. It happens to equal the smallest total a three-group hull can
/// reach, which is where it came from; a four-group hull cannot go under 4 at
/// all. The clamp against it exists so a hull with fewer groups still indexes
/// the table rather than underflowing it.
pub const MIN_RATED_TOTAL: u8 = 3;

/// The parse-default release margin for [`PowerConfig::floor_release_margin_pct`],
/// in percentage points of capacity — the band a floored group's reserve has to
/// climb back through before the reactor lets it go.
///
/// Five points is roughly two seconds of recovery on the shipped reactors
/// (±2/s against capacities of 35–100), which is long enough that a brownout
/// reads as an event rather than as a flicker and short enough that the ship
/// keeps trying. Every shipped hull authors it explicitly in `[power]`.
pub const DEFAULT_FLOOR_RELEASE_MARGIN_PCT: f32 = 5.0;

/// The PARSE-DEFAULT battery-floor ladder: helm 50 % / weapons 25 % /
/// shields 5 %.
///
/// A TOML-parse default in the sense of AGENTS.md rule 11 — the value every
/// `[power.battery_floor]`-less hull is parsed with, and overridable per hull.
/// Shields sits lowest deliberately: it is the last thing a dying ship should
/// lose, so it holds while helm and weapons have already been cut.
///
/// These are the numbers issue #952 asked for, and the table a hull that
/// declares NO `[power.ai_policy.param]` reserves at all should fly: with
/// nothing else stopping it spending, the reactor is the only guard it has.
///
/// The shipped fleet authors `weapons` and `shields` at exactly these numbers
/// but `helm` at 40, and the difference is measured rather than aesthetic — see
/// the `[power.battery_floor]` note in
/// `assets/entities/fragments/ai/fleet_baseline.toml`. In short: a floor bites
/// only when it is reached while the operator is still COMMANDING above the
/// landing level, so `weapons` has to sit above the fleet's
/// `min_reserve_weapons` of 10 or `apply_battery_floors`' `held >= commanded`
/// skip fires and the ladder is inert on every AI-crewed hull; but `helm` at
/// the fleet's `min_reserve_helm` of 50 holds the group down through a band its
/// own crew has already decided it can afford, and measurably costs an
/// attack-pass destroyer its break-off.
///
/// The landing levels here are [`UNAUTHORED_FLOOR_LEVEL`], because this is the
/// table a hull that authors no `[power_groups.*]` at all gets. Where a hull
/// DOES author them, `ship::power::authored_power_group_floors` replaces these
/// with that group's own `min_level`.
pub fn default_group_floors() -> HashMap<String, PowerGroupFloor> {
    HashMap::from([
        (
            HELM_POWER_GROUP.to_string(),
            PowerGroupFloor {
                battery_pct: 50.0,
                min_level: UNAUTHORED_FLOOR_LEVEL,
            },
        ),
        (
            WEAPONS_POWER_GROUP.to_string(),
            PowerGroupFloor {
                battery_pct: 25.0,
                min_level: UNAUTHORED_FLOOR_LEVEL,
            },
        ),
        (
            SHIELDS_POWER_GROUP.to_string(),
            PowerGroupFloor {
                battery_pct: 5.0,
                min_level: UNAUTHORED_FLOOR_LEVEL,
            },
        ),
    ])
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            capacity: 100.0,
            rates: [6.0, 5.0, 4.0, 2.0, -2.0, -6.0],
            emergency_threshold: 25.0,
            group_floors: default_group_floors(),
            floor_release_margin_pct: DEFAULT_FLOOR_RELEASE_MARGIN_PCT,
        }
    }
}

impl Default for PowerSystem {
    fn default() -> Self {
        Self::seeded_with_defaults(100.0)
    }
}

impl PowerSystem {
    pub fn new(config: &PowerConfig) -> Self {
        Self::seeded_with_defaults(config.capacity)
    }

    /// Internal helper: construct a PowerSystem with the three canonical
    /// groups pre-seeded at level 2 and the requested battery charge.
    fn seeded_with_defaults(battery_charge: f32) -> Self {
        let mut groups = HashMap::with_capacity(3);
        let mut order = Vec::with_capacity(3);
        for &name in POWER_GROUP_ORDER {
            let id = PowerGroupId(name.to_string());
            groups.insert(id.clone(), 2u8);
            order.push(id);
        }
        Self {
            groups,
            order,
            under_floor: std::collections::HashSet::new(),
            floored: HashMap::new(),
            battery_charge,
        }
    }

    /// Construct a `PowerSystem` seeded from a ship's authored power groups
    /// (issue #762). Each `(group, level)` is inserted at the given level,
    /// clamped to `[1, 4]`, in the order supplied — so a ship that authors an
    /// `ops` group beyond the canonical three gets it seeded and therefore
    /// allocatable (otherwise `set_group_allocation` returns `UnknownGroup`
    /// and any authored rule targeting it silently no-ops).
    ///
    /// Falls back to [`Self::seeded_with_defaults`] (the canonical `helm` /
    /// `weapons` / `shields` at level 2) when `groups` is empty, so ships and
    /// fixtures without a `[power_groups.*]` block are unchanged.
    pub fn from_authored_groups(battery_charge: f32, groups: &[(PowerGroupId, u8)]) -> Self {
        if groups.is_empty() {
            return Self::seeded_with_defaults(battery_charge);
        }
        let mut map = HashMap::with_capacity(groups.len());
        let mut order = Vec::with_capacity(groups.len());
        for (id, level) in groups {
            if map.contains_key(id) {
                continue;
            }
            map.insert(id.clone(), (*level).clamp(GROUP_LEVEL_MIN, GROUP_LEVEL_MAX));
            order.push(id.clone());
        }
        Self {
            groups: map,
            order,
            under_floor: std::collections::HashSet::new(),
            floored: HashMap::new(),
            battery_charge,
        }
    }

    /// Total EFFECTIVE allocation across all groups — the draw the battery is
    /// actually carrying, so a browned-out group stops costing what it is no
    /// longer being given. This is what [`Self::tick`] indexes `rates` with,
    /// and therefore what makes a bottomed-out ship recharge at all.
    pub fn total(&self) -> u8 {
        self.order.iter().map(|id| self.level_for(id)).sum()
    }

    /// Total COMMANDED allocation — what the reactor has been asked to run,
    /// ignoring any battery floor currently holding a group down. This is the
    /// quantity the 8-point budget is spent against, so a brownout cannot be
    /// used as headroom to over-commit and then snap past the cap on recovery.
    pub fn commanded_total(&self) -> u8 {
        self.groups.values().copied().sum()
    }

    /// Current EFFECTIVE level for the given power group: the battery floor
    /// while one is holding the group down, otherwise the commanded level.
    /// Returns `0` for groups the system does not know about (matches the
    /// historical `power_level_for_console` fallback for non-powered consoles).
    pub fn level_for(&self, group: &PowerGroupId) -> u8 {
        self.floored
            .get(group)
            .copied()
            .or_else(|| self.groups.get(group).copied())
            .unwrap_or(0)
    }

    /// Level the group has been COMMANDED to run at, ignoring any battery
    /// floor. Returns `0` for unknown groups.
    pub fn commanded_level_for(&self, group: &PowerGroupId) -> u8 {
        self.groups.get(group).copied().unwrap_or(0)
    }

    /// True while `group` is being held below its commanded level by its
    /// authored battery floor (issue #952).
    pub fn is_floored(&self, group: &PowerGroupId) -> bool {
        self.floored.contains_key(group)
    }

    /// True if the system tracks the given power group.
    pub fn has_group(&self, group: &PowerGroupId) -> bool {
        self.groups.contains_key(group)
    }

    /// Insertion-ordered iteration over `(&PowerGroupId, effective level)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&PowerGroupId, u8)> {
        self.order.iter().map(move |id| (id, self.level_for(id)))
    }

    pub fn read_state(&self) -> PowerReadState {
        PowerReadState {
            allocations: self
                .order
                .iter()
                .map(|id| (id.clone(), self.level_for(id)))
                .collect(),
            battery_charge: self.battery_charge,
        }
    }

    /// Set the COMMANDED allocation for a specific power group to `level`,
    /// clamped to `[1, 4]`. Delta is applied one step at a time via `increase` /
    /// `decrease` so the `commanded_total() <= 8` invariant is honoured.
    ///
    /// Deliberately measured against the commanded level, not the effective
    /// one: a browned-out group must still accept the command that says where
    /// it goes once the battery is back, otherwise recovery would need whoever
    /// issued it to notice and re-issue.
    pub fn set_group_allocation(
        &mut self,
        group: &PowerGroupId,
        level: u8,
    ) -> Result<(), PowerAllocationError> {
        if !self.groups.contains_key(group) {
            return Err(PowerAllocationError::UnknownGroup(group.clone()));
        }
        let current = self.commanded_level_for(group);
        let target_level = level.clamp(GROUP_LEVEL_MIN, GROUP_LEVEL_MAX);
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

    /// Increase the commanded allocation for `group` by 1. Clamped to `4` per
    /// group and to `8` for the commanded total.
    pub fn increase(&mut self, group: &PowerGroupId) {
        if self.commanded_total() >= MAX_COMMANDED_TOTAL {
            return;
        }
        if let Some(v) = self.groups.get_mut(group) {
            if *v < GROUP_LEVEL_MAX {
                *v += 1;
            }
        }
    }

    /// Decrease the commanded allocation for `group` by 1. Clamped to `1` per
    /// group.
    pub fn decrease(&mut self, group: &PowerGroupId) {
        if let Some(v) = self.groups.get_mut(group) {
            if *v > GROUP_LEVEL_MIN {
                *v -= 1;
            }
        }
    }

    /// The reactor's current net battery rate, in charge units per second, at
    /// the effective total allocation. Negative means the ship is spending its
    /// reserve faster than the reactor makes it.
    pub fn battery_rate(&self, config: &PowerConfig) -> f32 {
        let total = self.total().clamp(MIN_RATED_TOTAL, MAX_COMMANDED_TOTAL) as usize;
        config.rates[total - MIN_RATED_TOTAL as usize]
    }

    /// True when the battery is falling at the current draw. Published so the
    /// gauge can say whether the reserve is filling or emptying — the honest
    /// successor to the `locked` flag issue #952 retired, which said only that
    /// the reserve had already run out.
    pub fn is_draining(&self, config: &PowerConfig) -> bool {
        self.battery_rate(config) < 0.0
    }

    /// True when the battery is actually FILLING at the current draw.
    ///
    /// Deliberately not `!is_draining()`: a hull may author a rate of exactly
    /// `0.0` for some total (the courier did, at the very rung its own floors
    /// settled it on), and at that total the reserve is frozen — neither
    /// emptying nor filling. Painting the gauge's pulsing CHARGING indicator
    /// from the negation would claim a recovery that is never going to arrive,
    /// which is the most misleading thing a battery readout can say.
    pub fn is_charging(&self, config: &PowerConfig) -> bool {
        self.battery_rate(config) > 0.0
    }

    /// Advance the simulation by `dt` seconds: integrate the battery from the
    /// current EFFECTIVE total allocation, then re-derive which groups their
    /// authored battery floor is holding down (issue #952).
    ///
    /// There is no brownout lock any more. A ship that flattens its battery
    /// does not have every group slammed to 1 and frozen there until some
    /// absolute recovery threshold — each group is simply held at its own
    /// authored `min_level` for as long as the battery sits under its own
    /// authored percentage, and released once the charge climbs back through
    /// that percentage plus [`PowerConfig::floor_release_margin_pct`], once no
    /// rung above it is engaged either. Every ladder in the fleet descends
    /// helm → weapons → shields, so falling, helm is cut first, weapons next,
    /// and shields holds longest — which is the whole point of the ordering.
    /// Recovering, the ladder releases from the TOP: a cut rung waits for the
    /// rungs above it, so the reserve climbs on the deep-cut draw instead of
    /// re-imposing it, and the lowest-floor group is the LAST one back. See
    /// [`Self::apply_battery_floors`] for why the release margin alone cannot
    /// buy that.
    ///
    /// Returns `true` if the set of floored groups changed this tick.
    /// `ship::power::tick_power_system` forwards that edge into
    /// `PowerBrownoutState::floors_changed`, which
    /// `ship::power::tick_power_brownout_advisory` consumes later the same tick
    /// to re-arm its debounce — so a group the reactor has just cut announces
    /// itself to Helm or Tactical instead of being swallowed by a debounce that
    /// only watches the reserve's direction.
    pub fn tick(&mut self, dt: f32, config: &PowerConfig) -> bool {
        let rate = self.battery_rate(config);
        self.battery_charge = (self.battery_charge + rate * dt).clamp(0.0, config.capacity);
        self.apply_battery_floors(config)
    }

    /// Re-derive [`Self::floored`] from the current charge. Returns `true` when
    /// the set (or a held level within it) changed.
    ///
    /// TWO thresholds, not one. A group not currently held is cut when the
    /// charge falls BELOW its `battery_pct`; a group already held is released
    /// only once the charge climbs to `battery_pct + `
    /// [`PowerConfig::floor_release_margin_pct`]. Between the two it simply
    /// stays as it is. Without that band the cut would chatter at tick rate,
    /// because cutting a group lowers the effective total and the effective
    /// total is what sets the sign of the battery rate — see the field's docs.
    ///
    /// # The ladder releases from the TOP, and the margin alone cannot do it
    ///
    /// The band is necessary but not sufficient, because of WHAT it releases
    /// into. The charge a failing reactor can actually reach is set by the
    /// LOWEST engaged floor, since that is the cut deep enough to flip the
    /// `rates` rung positive. Release that rung the moment it clears its own
    /// band and the draw goes straight back up, capping the reserve in a limit
    /// cycle a few points wide — well under every HIGHER rung's release
    /// threshold, which is then unreachable for the rest of the encounter. On
    /// the shipped destroyer (`rates = [5,4,3,2,-2,-5]`, capacity 70) an
    /// officer commanding the legal `ops 1 / helm 3 / weapons 3 / shields 1`
    /// parked at exactly that: weapons cycling across 25–30 % at ±2/s with zero
    /// net drift, helm latched down at its `min_level` against a release
    /// threshold of 45 % it could never reach.
    ///
    /// So a rung stays cut while any rung ABOVE it is still engaged. The
    /// reserve holds the deep-cut draw all the way up through the highest
    /// engaged rung's release band, and everything returns together — because
    /// the floors descend and the margin is uniform, clearing the top rung's
    /// threshold clears every lower one's by construction.
    ///
    /// This inverts what an earlier revision documented. Falling, the ship
    /// still loses helm first, then its guns, and keeps its screens longest —
    /// that half is unchanged, and it is the half a crew feels. Recovering, the
    /// groups come back in that SAME order rather than the reverse: the
    /// lowest-floor group (shields on every shipped hull) is the last one
    /// released, not the first. "Shields comes back first" was only ever true
    /// of a ladder that never finished recovering.
    ///
    /// Blocking is keyed on ENGAGEMENT (`under_floor`), not on being held
    /// (`floored`). A rung whose commanded level already sits at its landing
    /// level takes nothing from the ship, but the reserve is still under its
    /// threshold, and that is the fact the ladder is climbing. Keying on
    /// `floored` instead would leave every AI-crewed hull in the limit cycle,
    /// since the policy's own reserve guard has usually given the helm point
    /// back before the charge reaches helm's floor.
    ///
    /// Pinned by
    /// `ship::power::tests::a_human_commanded_destroyer_climbs_back_out_of_its_own_floor_ladder`.
    fn apply_battery_floors(&mut self, config: &PowerConfig) -> bool {
        let battery_pct = if config.capacity > 0.0 {
            (self.battery_charge / config.capacity) * 100.0
        } else {
            0.0
        };
        let margin = config.floor_release_margin_pct.max(0.0);

        // Step 1 — the battery-side question, with the hysteresis: which floors
        // are ENGAGED. A group not currently engaged engages when the charge
        // falls under its own threshold; one that is stays engaged until the
        // charge clears that threshold PLUS the margin AND no rung above it is
        // still engaged.
        //
        // Walked from the top of the ladder down, off `order` rather than the
        // HashMap's keys, so the result does not depend on hash iteration
        // order. `sort_by` is stable, so equal floors keep `order`'s sequence;
        // they also share a release threshold, so neither can block the other
        // into a state it would not have reached alone.
        let mut ladder: Vec<(&PowerGroupId, &PowerGroupFloor)> = self
            .order
            .iter()
            // No authored floor: this group is never cut back. `ops` and any
            // other bespoke group a hull invents sit here by default.
            .filter_map(|id| config.group_floors.get(id.0.as_str()).map(|f| (id, f)))
            .collect();
        ladder.sort_by(|a, b| {
            b.1.battery_pct
                .partial_cmp(&a.1.battery_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut engaged: std::collections::HashSet<PowerGroupId> = std::collections::HashSet::new();
        let mut higher_rung_engaged = false;
        for (id, floor) in ladder {
            let is_engaged = if self.under_floor.contains(id) {
                battery_pct < floor.battery_pct + margin || higher_rung_engaged
            } else {
                battery_pct < floor.battery_pct
            };
            if is_engaged {
                engaged.insert(id.clone());
                higher_rung_engaged = true;
            }
        }
        self.under_floor = engaged;

        // Step 2 — the order-side question: of the engaged floors, which are
        // actually holding a group BELOW what it has been told to run at.
        let mut next: HashMap<PowerGroupId, u8> = HashMap::new();
        for (id, commanded) in self.groups.iter() {
            if !self.under_floor.contains(id) {
                continue;
            }
            let Some(floor) = config.group_floors.get(id.0.as_str()) else {
                continue;
            };
            let held = floor.min_level.clamp(GROUP_LEVEL_MIN, GROUP_LEVEL_MAX);
            // A group already resting AT or BELOW its floor level is not
            // recorded at all. The brownout has nothing to take from it, and
            // recording it anyway would both raise a group the brownout is
            // meant to be taking power AWAY from and fire this tick's change
            // edge for a transition no reader could observe. `>=` rather than
            // `>` for the second half of that: a hull authoring `min_level = 2`
            // and resting the group at 2 is simply unaffected.
            if held >= *commanded {
                continue;
            }
            next.insert(id.clone(), held);
        }
        let changed = next != self.floored;
        self.floored = next;
        changed
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
    /// The `priority` of the authored rule that won this group's channel. The
    /// PRIMARY ordering key, and the whole of the "priority is data-authored"
    /// requirement: a designer who wants weapons served before helm when the
    /// budget is short raises that rule's `priority` in the hull file.
    ///
    /// No Rust list outranks a preference the hull expressed. Both ordering
    /// keys on this struct are authored, and the caller's own order — which is
    /// [`PowerSystem::iter`]'s, i.e. [`POWER_GROUP_ORDER`] first and then
    /// alphabetically, via `ship::power::authored_power_group_seed` — breaks
    /// only the ties the hull left identical on BOTH keys: equal rule
    /// priorities AND neither group named in `[power.battery_floor]`. See the
    /// sort site in [`plan_allocation`], which says the same thing from the
    /// other end.
    pub rule_priority: i32,
    /// This group's `[power.battery_floor]` percentage, `None` when the hull
    /// authors no floor for it.
    ///
    /// The SECONDARY key, used only to break a tie the designer left. A floor
    /// is the hull's statement of the order it gives groups up as the reserve
    /// falls — highest floor cut first — so a LOWER floor means "keep this one
    /// longer", and an absent floor ("never cut at all") is the strongest such
    /// statement there is. Spending a scarce point in that same order is the
    /// hull's own answer rather than an invented one, and on the shipped fleet
    /// (helm 40 / weapons 25 / shields 5, every elevation rule at priority 10)
    /// it is the only authored ranking that exists.
    pub floor_pct: Option<f32>,
}

/// Distribute the reactor's allocation budget across the groups that bid for it
/// (issue #959), returning the levels to COMMAND, in the order they must be
/// applied.
///
/// # The bug this replaces
///
/// The AI decider used to resolve each group's channel in isolation and emit
/// that group's absolute target with no idea what the others had asked for.
/// [`PowerSystem::increase`] refuses past [`MAX_COMMANDED_TOTAL`] SILENTLY and
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
///   may be cut; `ops` on the Alliance hulls sits here.
/// * Every bidding group is guaranteed [`GROUP_LEVEL_MIN`], because that is the
///   floor the setter API clamps to and no distribution can go under it.
/// * What is left over — `MAX_COMMANDED_TOTAL - reserved - one per bidder` — is
///   the DISCRETIONARY budget, handed out in authored-priority order until it
///   runs out. A group that cannot be paid in full lands as high as the budget
///   reaches, never at a level the applier would refuse.
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
    // out of the budget arithmetic too.
    let mut ranked: Vec<&AllocationBid> =
        bids.iter().filter(|b| power.has_group(&b.group)).collect();

    // Groups nothing bid for hold what they were last commanded to, and that
    // holding costs budget.
    let reserved: u16 = power
        .order
        .iter()
        .filter(|id| !ranked.iter().any(|b| &b.group == *id))
        .map(|id| power.commanded_level_for(id) as u16)
        .sum();

    // Authored order: rule priority first (higher wins), then the hull's own
    // floor ladder (lower floor — kept longer — wins, absent floor first).
    // `sort_by` is stable, so a tie on BOTH authored keys falls back to the
    // caller's order, which is `PowerSystem::iter()`'s. That last fallback is
    // determinism, not a design opinion: it only decides between groups the
    // hull has ranked identically.
    ranked.sort_by(|a, b| {
        b.rule_priority.cmp(&a.rule_priority).then_with(|| {
            let key = |f: &Option<f32>| f.unwrap_or(f32::NEG_INFINITY);
            key(&a.floor_pct)
                .partial_cmp(&key(&b.floor_pct))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    let mins = ranked.len() as u16 * GROUP_LEVEL_MIN as u16;
    let mut spare = (MAX_COMMANDED_TOTAL as u16).saturating_sub(reserved + mins);

    let mut planned: Vec<(PowerGroupId, u8)> = Vec::with_capacity(ranked.len());
    for bid in ranked {
        let ceiling = bid.max_level.clamp(GROUP_LEVEL_MIN, GROUP_LEVEL_MAX);
        let want = bid.want.clamp(GROUP_LEVEL_MIN, ceiling);
        let asked = (want - GROUP_LEVEL_MIN) as u16;
        let granted = asked.min(spare);
        spare -= granted;
        planned.push((bid.group.clone(), GROUP_LEVEL_MIN + granted as u8));
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

    fn helm() -> PowerGroupId {
        PowerGroupId(HELM_POWER_GROUP.into())
    }
    fn weapons() -> PowerGroupId {
        PowerGroupId(WEAPONS_POWER_GROUP.into())
    }
    fn shields() -> PowerGroupId {
        PowerGroupId(SHIELDS_POWER_GROUP.into())
    }

    /// A config with the shipped floor ladder but a battery that never moves,
    /// for tests that want to place the charge by hand and tick once.
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

    /// The 8-point budget is spent against the COMMANDED total, not the
    /// effective one, so a brownout cannot be used as headroom.
    ///
    /// This replaces `increase_when_locked_is_noop`, whose premise (a `locked`
    /// flag that froze every allocation control) went away with the brownout
    /// lock in issue #952. The invariant worth keeping from it is that some
    /// battery state can refuse an increase — but the refusal that survives is
    /// the budget one, and it must NOT get quietly looser while groups are
    /// floored or the reactor would snap past 8 the instant the battery
    /// recovers.
    #[test]
    fn budget_cap_is_measured_against_commanded_not_floored_levels() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.set_group_allocation(&weapons(), 3).unwrap();
        // Flatten the battery: every group is under its floor.
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.total(), 6, "helm and weapons held back at their floors");
        assert_eq!(ps.commanded_total(), 8, "commands are untouched");

        ps.increase(&shields());
        ps.battery_charge = 100.0;
        ps.tick(1.0, &config);
        assert_eq!(
            ps.total(),
            8,
            "the increase must have been refused while floored: recovering to a \
             total of 9 would put the reactor over a cap nothing else enforces"
        );
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

    /// A floored group still takes commands; they land on the commanded level
    /// and take effect on recovery.
    ///
    /// This replaces `decrease_when_locked_is_noop`. Its premise was that a
    /// flat battery froze the Power console's controls outright — the operator
    /// could not even give power away. That is the wrong half of the lock to
    /// have kept: refusing input on a dying ship is a UI dead end, and it made
    /// recovery depend on somebody noticing the unlock and re-commanding.
    #[test]
    fn a_floored_group_still_accepts_commands() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.level_for(&helm()), 2, "helm is at its floor");

        ps.set_group_allocation(&helm(), 4).unwrap();
        assert_eq!(ps.level_for(&helm()), 2, "still held down while flat");
        assert_eq!(ps.commanded_level_for(&helm()), 4);

        ps.battery_charge = 100.0;
        ps.tick(1.0, &config);
        assert_eq!(
            ps.level_for(&helm()),
            4,
            "the command taken while floored applies the moment the floor lifts"
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

    // ── battery floors (issue #952) ───────────────────────────────────────

    /// **The ladder.** As the battery falls, helm is cut at 50 %, weapons at
    /// 25 %, and shields holds all the way down to 5 %.
    ///
    /// Each group lands on its floor LEVEL, which for a system seeded from the
    /// canonical trio is `UNAUTHORED_FLOOR_LEVEL` — nominal. A brownout takes
    /// back what was spent, so the groups tested here are spent up first.
    ///
    /// This replaces `exhaustion_forces_consoles_to_one_and_locks`, which
    /// asserted the binary model: nothing happened at all until the battery hit
    /// zero, and then EVERY group was slammed to 1 in the same instant. That
    /// assertion is now wrong in three ways — degradation starts long before
    /// zero, it never arrives all at once, and where it lands is authored.
    #[test]
    fn groups_are_cut_in_floor_order_as_the_battery_falls() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.set_group_allocation(&weapons(), 3).unwrap();

        let read = |ps: &PowerSystem| {
            (
                ps.level_for(&helm()),
                ps.level_for(&weapons()),
                ps.level_for(&shields()),
            )
        };

        // 60 %: everyone is above their floor.
        ps.battery_charge = 60.0;
        ps.tick(1.0, &config);
        assert_eq!(read(&ps), (3, 3, 2));

        // 40 %: helm (50) is under; weapons (25) and shields (5) are not.
        ps.battery_charge = 40.0;
        ps.tick(1.0, &config);
        assert_eq!(read(&ps), (2, 3, 2));

        // 10 %: weapons joins helm; shields still holds.
        ps.battery_charge = 10.0;
        ps.tick(1.0, &config);
        assert_eq!(read(&ps), (2, 2, 2));

        // 1 %: shields is under its floor too — and, resting at the floor
        // level already, has nothing left to give. That is the point of the
        // ordering: the last group down loses nothing.
        ps.battery_charge = 1.0;
        ps.tick(1.0, &config);
        assert_eq!(read(&ps), (2, 2, 2));
    }

    /// A ship that bottoms out sits at its floors — not at a frozen level-1
    /// lock — and is still a functioning reactor.
    #[test]
    fn a_flat_battery_settles_at_the_floors_with_no_lock() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);

        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&shields()), 2);
        assert!(ps.is_floored(&helm()));
        // The commanded allocation survives the brownout intact.
        assert_eq!(ps.commanded_level_for(&helm()), 4);
    }

    /// A hull that AUTHORS `min_level = 1` for a group gets a group cut to 1 —
    /// the landing level is data, and `UNAUTHORED_FLOOR_LEVEL` is only what
    /// stands in when a hull says nothing.
    #[test]
    fn an_authored_min_level_of_one_really_does_cut_the_group_to_one() {
        let mut config = still_config();
        config.group_floors.insert(
            SHIELDS_POWER_GROUP.to_string(),
            PowerGroupFloor {
                battery_pct: 5.0,
                min_level: 1,
            },
        );
        let mut ps = PowerSystem::default();
        ps.battery_charge = 1.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.level_for(&shields()), 1);
        assert_eq!(ps.level_for(&helm()), 2, "helm still lands on nominal");
    }

    /// Floors release from the TOP of the ladder as the battery climbs, and
    /// each group comes straight back to what it was last commanded at.
    ///
    /// This test used to assert the opposite order — weapons back at 30 %,
    /// helm later at 80 % — and that order is exactly the defect
    /// `apply_battery_floors` now documents: releasing the lowest engaged rung
    /// first restores the draw that was emptying the battery, so on a live
    /// reactor (this fixture's rates are all zero, so it cannot see that) the
    /// reserve never climbs to the higher rung's band at all.
    /// `ship::power::tests::a_human_commanded_destroyer_climbs_back_out_of_its_own_floor_ladder`
    /// flies the shipped destroyer through that excursion for real.
    #[test]
    fn recovery_refills_groups_as_the_battery_climbs_back() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.set_group_allocation(&weapons(), 3).unwrap();
        let read = |ps: &PowerSystem| {
            (
                ps.level_for(&helm()),
                ps.level_for(&weapons()),
                ps.level_for(&shields()),
            )
        };

        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert_eq!(read(&ps), (2, 2, 2));

        // Above weapons' own 25+5 release band, but helm's rung (50) is still
        // engaged above it — so the guns wait. Releasing them here is what put
        // the ship back on the draw that stopped it ever reaching helm's band.
        ps.battery_charge = 30.0;
        ps.tick(1.0, &config);
        assert_eq!(
            read(&ps),
            (2, 2, 2),
            "a rung stays cut while a rung above it is still engaged"
        );

        // Clear of helm's 50+5, and because the floors descend and the margin is
        // uniform, that clears every lower rung's band too: the whole ladder
        // returns together.
        ps.battery_charge = 56.0;
        ps.tick(1.0, &config);
        assert_eq!(read(&ps), (3, 3, 2));
    }

    /// The effective total is what the battery is charged for, so a browned-out
    /// ship actually recovers instead of sitting flat for ever.
    #[test]
    fn flooring_groups_drops_the_draw_so_the_battery_recharges() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.set_group_allocation(&weapons(), 2).unwrap();
        ps.set_group_allocation(&shields(), 2).unwrap();
        assert_eq!(ps.commanded_total(), 8);
        ps.battery_charge = 2.0;

        // total 8 → -6/s, so this tick flattens the battery and floors helm
        // back to nominal (weapons and shields are resting there already).
        ps.tick(1.0, &config);
        assert_eq!(ps.battery_charge, 0.0);
        assert_eq!(ps.total(), 6);
        // Next tick draws at the floored total of 6 → +2/s.
        ps.tick(1.0, &config);
        assert!(
            (ps.battery_charge - 2.0).abs() < 0.001,
            "got {}",
            ps.battery_charge
        );
    }

    /// Floors are a PERCENTAGE of the hull's own capacity, unlike
    /// `emergency_threshold`, which has always been absolute.
    #[test]
    fn floors_scale_with_the_hulls_authored_capacity() {
        let config = PowerConfig {
            capacity: 90.0,
            rates: [0.0; 6],
            ..PowerConfig::default()
        };
        let mut ps = PowerSystem::new(&config);
        ps.set_group_allocation(&helm(), 3).unwrap();
        // 46/90 = 51.1 % — above helm's 50 % floor even though it is well
        // under the 50 an absolute reading would have compared against.
        ps.battery_charge = 46.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.level_for(&helm()), 3);
        // 44/90 = 48.9 % — under it.
        ps.battery_charge = 44.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.level_for(&helm()), 2);
    }

    /// A group with no authored floor is never cut — `ops` and friends keep
    /// their commanded level to a flat battery.
    #[test]
    fn a_group_with_no_authored_floor_is_never_cut() {
        let config = still_config();
        let ops = PowerGroupId("ops".into());
        let mut ps = PowerSystem::from_authored_groups(100.0, &[(helm(), 3), (ops.clone(), 3)]);
        ps.battery_charge = 0.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.level_for(&helm()), 2, "helm has a floor and is cut");
        assert_eq!(ps.level_for(&ops), 3, "ops has none and is left alone");
    }

    /// **A floor does not chatter, because cutting a group flips the sign of
    /// the battery rate.**
    ///
    /// The failure mode this pins is not hypothetical arithmetic: flooring
    /// LOWERS the effective total, `tick` indexes `rates` by the effective
    /// total, and on every shipped reactor the rungs either side of the resting
    /// total have opposite signs. So the cut immediately starts recharging the
    /// battery back through the very threshold that made it, and with a single
    /// threshold the group is released on the next tick, re-raising the draw,
    /// re-crossing the threshold, for ever — at 60 Hz. Everything downstream of
    /// the effective level toggles with it: `MaxSpeed`, `PhaserDamage`,
    /// `is_draining`, and `tick_power_brownout_advisory`'s debounce, which
    /// would re-send `CoordinationPayload::PowerBrownout` to Helm and Tactical
    /// on alternating ticks.
    #[test]
    fn a_floored_group_is_not_released_until_the_charge_clears_the_margin() {
        let config = PowerConfig::default(); // helm floor 50, margin 5
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap(); // total 7 → -2/s
        ps.battery_charge = 49.0;

        ps.tick(1.0, &config); // 47 — under the floor
        assert!(ps.is_floored(&helm()));
        assert_eq!(ps.total(), 6, "the cut dropped the draw onto a +2/s rung");

        // Charging back at +2/s. Crossing the bare floor must NOT release it:
        // that is exactly where the single-threshold shape oscillates.
        for expected in [49.0, 51.0, 53.0] {
            ps.tick(1.0, &config);
            assert!((ps.battery_charge - expected).abs() < 1e-4);
            assert!(
                ps.is_floored(&helm()),
                "helm released at {}% — back to a tick-rate flip-flop",
                ps.battery_charge
            );
        }

        ps.tick(1.0, &config); // 55 — clears floor + margin
        assert!(!ps.is_floored(&helm()));
        assert_eq!(ps.level_for(&helm()), 3, "and comes back at the command");
    }

    /// The margin is a live, authored knob: at `0.0` the floor collapses back
    /// to the single-threshold shape (cut and release at the same percentage),
    /// which is the degenerate case the shipped default deliberately avoids.
    #[test]
    fn a_zero_release_margin_collapses_the_two_thresholds_into_one() {
        let config = PowerConfig {
            floor_release_margin_pct: 0.0,
            ..PowerConfig::default()
        };
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.battery_charge = 49.0;

        ps.tick(1.0, &config); // 47 → floored
        assert!(ps.is_floored(&helm()));
        ps.tick(1.0, &config); // 49 → still under 50
        assert!(ps.is_floored(&helm()));
        ps.tick(1.0, &config); // 51 → released the moment it clears the floor
        assert!(!ps.is_floored(&helm()));
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

    /// The tick's return value is the floor-set edge, replacing the old
    /// `tick_returns_true_when_lock_changes`. The lock it reported no longer
    /// exists; the transition worth hanging an advisory off does.
    #[test]
    fn tick_returns_true_when_the_floored_set_changes() {
        let config = still_config();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.battery_charge = 40.0;
        assert!(ps.tick(1.0, &config), "helm crossed under its floor");
        assert!(!ps.tick(1.0, &config), "nothing changed");
        ps.battery_charge = 80.0;
        assert!(ps.tick(1.0, &config), "helm was released");
    }

    // ── configurable constructor ──────────────────────────────────────────

    #[test]
    fn custom_config() {
        let config = PowerConfig {
            capacity: 50.0,
            rates: [1.0, 1.0, 1.0, -1.0, -2.0, -3.0],
            emergency_threshold: 10.0,
            group_floors: default_group_floors(),
            floor_release_margin_pct: DEFAULT_FLOOR_RELEASE_MARGIN_PCT,
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

    fn ops() -> PowerGroupId {
        PowerGroupId("ops".into())
    }

    /// A bid at the shipped fleet's ordering: every elevation rule authored at
    /// priority 10, tie-broken by the hull's own floor ladder.
    fn bid(
        group: PowerGroupId,
        want: u8,
        rule_priority: i32,
        floor_pct: Option<f32>,
    ) -> AllocationBid {
        AllocationBid {
            group,
            want,
            max_level: crate::ship::config::default_max_power_level(),
            rule_priority,
            floor_pct,
        }
    }

    /// The four-group Alliance shape: `ops` outside the canonical trio, seeded
    /// at 1, with nothing in the policy bidding for it.
    fn alliance_reactor() -> PowerSystem {
        PowerSystem::from_authored_groups(
            90.0,
            &[(helm(), 2), (weapons(), 2), (shields(), 1), (ops(), 1)],
        )
    }

    /// **Allocate within the max.** The shipped combat-stations allocation —
    /// helm 3 / weapons 3 against `ops` 1 and `shields` 1 — spends the budget
    /// exactly, and every planned level is inside the group's own ceiling.
    #[test]
    fn plan_allocation_spends_the_budget_without_exceeding_it() {
        let ps = alliance_reactor();
        let plan = plan_allocation(
            &ps,
            &[
                bid(helm(), 3, 10, Some(40.0)),
                bid(weapons(), 3, 10, Some(25.0)),
            ],
        );

        let mut after = ps.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(after.commanded_level_for(&helm()), 3);
        assert_eq!(after.commanded_level_for(&weapons()), 3);
        assert_eq!(after.commanded_total(), MAX_COMMANDED_TOTAL);
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
        let mut ps = alliance_reactor();
        ps.set_group_allocation(&weapons(), 1).unwrap();
        let capped = AllocationBid {
            max_level: 2,
            ..bid(weapons(), 4, 10, Some(25.0))
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
        let ps = alliance_reactor();
        let plan = plan_allocation(
            &ps,
            &[
                bid(helm(), 4, 5, Some(40.0)),
                bid(weapons(), 4, 30, Some(25.0)),
                bid(shields(), 4, 20, Some(5.0)),
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
            after.commanded_level_for(&ops()),
            1,
            "un-bid groups are reserved"
        );
        assert_eq!(after.commanded_total(), MAX_COMMANDED_TOTAL);
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

    /// A tie the designer left is broken by the hull's own `[power.battery_floor]`
    /// ladder — lower floor (kept longer as the reserve falls) served first —
    /// and NOT by any ordering baked into Rust.
    #[test]
    fn plan_allocation_breaks_a_priority_tie_on_the_authored_floor_ladder() {
        let ps = alliance_reactor();
        // Both at priority 10, both asking for 4, only 4 discretionary points.
        let plan = plan_allocation(
            &ps,
            &[
                bid(helm(), 4, 10, Some(40.0)),
                bid(weapons(), 4, 10, Some(25.0)),
            ],
        );
        let mut after = ps.clone();
        for (group, level) in &plan {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(
            after.commanded_level_for(&weapons()),
            4,
            "weapons' floor of 25 is under helm's 40, so the hull keeps it \
             longer and it is served first"
        );
        assert_eq!(after.commanded_level_for(&helm()), 2);

        // Invert the ladder in the DATA alone and the order inverts with it.
        let inverted = plan_allocation(
            &ps,
            &[
                bid(helm(), 4, 10, Some(25.0)),
                bid(weapons(), 4, 10, Some(40.0)),
            ],
        );
        let mut after = ps.clone();
        for (group, level) in &inverted {
            after.set_group_allocation(group, *level).unwrap();
        }
        assert_eq!(after.commanded_level_for(&helm()), 4);
        assert_eq!(after.commanded_level_for(&weapons()), 2);
    }

    /// **No re-emit stall.** Re-planning against the reactor the previous plan
    /// produced returns NOTHING — the decision has settled, so the host emits
    /// nothing and admission stays quiet. This is the invariant the old
    /// per-group emit could not hold: its refused command was re-issued on
    /// every decision arm for ever.
    #[test]
    fn plan_allocation_settles_and_stops_emitting() {
        let mut ps = alliance_reactor();
        let bids = [
            bid(helm(), 4, 10, Some(40.0)),
            bid(weapons(), 4, 10, Some(25.0)),
            bid(shields(), 4, 10, Some(5.0)),
        ];

        let first = plan_allocation(&ps, &bids);
        assert!(!first.is_empty(), "the first arm has work to do");
        for (group, level) in &first {
            ps.set_group_allocation(group, *level).unwrap();
        }
        assert!(ps.commanded_total() <= MAX_COMMANDED_TOTAL);

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
        let mut ps = alliance_reactor();
        ps.set_group_allocation(&helm(), 4).unwrap();
        assert_eq!(ps.commanded_total(), MAX_COMMANDED_TOTAL);

        // The policy now wants the two swapped over.
        let plan = plan_allocation(
            &ps,
            &[
                bid(helm(), 2, 10, Some(40.0)),
                bid(weapons(), 4, 20, Some(25.0)),
            ],
        );
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
        let ps = alliance_reactor();
        let plan = plan_allocation(
            &ps,
            &[
                bid(PowerGroupId("life-support".into()), 4, 100, None),
                bid(weapons(), 4, 10, Some(25.0)),
            ],
        );
        assert_eq!(plan, vec![(weapons(), 4)]);
    }

    /// **The qualifier on "the returned total fits."** A hull whose
    /// `[power_groups.*] default_level` values already sum past
    /// [`MAX_COMMANDED_TOTAL`] is not something this function can undo, and
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
            90.0,
            &[
                (helm(), 2),
                (weapons(), 2),
                (shields(), 2),
                (ops(), 2),
                (life_support.clone(), 2),
            ],
        );
        assert!(over.commanded_total() > MAX_COMMANDED_TOTAL);

        // Only helm bids. The other four hold 8 between them, so there is no
        // discretionary budget at all and nothing licenses cutting them.
        let plan = plan_allocation(&over, &[bid(helm(), 4, 10, Some(40.0))]);
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
        let all_bid: Vec<AllocationBid> = over
            .iter()
            .map(|(id, _)| bid(id.clone(), 4, 10, None))
            .collect();
        let mut after_all = over.clone();
        for (group, level) in plan_allocation(&over, &all_bid) {
            after_all.set_group_allocation(&group, level).unwrap();
        }
        assert_eq!(after_all.commanded_total(), MAX_COMMANDED_TOTAL);
    }
}
