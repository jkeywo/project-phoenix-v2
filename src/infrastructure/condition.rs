//! Pure, Bevy-free infrastructure **condition** and **capacity** (issue #1025).
//!
//! Skyhooks, fuel depots and the rest of the authored world furniture a mission
//! is built around need two things the existing damage model does not give
//! them. First a *condition* track: structural health that is distinct from
//! `[hull]` (which is the thing weapons destroy) and from the per-system damage
//! tiers a crewed ship carries, because a structure degrades and gets patched up
//! over a mission rather than exploding once. Second one or more named
//! *capacities*: how much a depot moves in a transfer window, how many souls a
//! platform holds — authored numbers a consumer asks for instead of hard-coding
//! at the call site.
//!
//! Degradation is mechanically real. Authored thresholds on the condition track
//! flip named **operational flags** — `transfer_capable`, `docking_capable`,
//! whatever the scenario declares — and every mutating method returns the flags
//! that changed as a result, so the caller can mirror them wherever they need to
//! be observable. This module never reaches for the world flag store, the ECS,
//! or the wire; it owns the arithmetic and the edge detection and nothing else.
//! Its Bevy adapter is the sibling [`crate::infrastructure::server`].
//!
//! # Chatter (AC2)
//!
//! A threshold is **edge-triggered against the stored flag**, not level-computed
//! each tick, and its two edges sit at different values:
//!
//! * the flag falls when `fraction < fails_below`,
//! * and comes back only when `fraction >= restores_above`.
//!
//! `restores_above` defaults to `fails_below + hysteresis` (an authored band on
//! the `[infrastructure]` table), so a condition parked exactly on the boundary
//! — the storm ticking a point off, a repair team putting a point back — sits in
//! the dead band and reports no transition at all. Authoring
//! `restores_above = fails_below` opts out and is legal; the invariant the
//! validator enforces is only that the restore point is never *below* the
//! failure point, which would invert the band.

use serde::{Deserialize, Serialize};

// ── Authored TOML shape ──────────────────────────────────────────────────────

/// Default condition ceiling for a track that does not author one.
///
/// A TOML-parse fallback, which is the only kind of hardcoded gameplay value
/// AGENTS.md #11 sanctions. Round number, so an authored `fails_below = 0.4`
/// reads as "40 points left" without arithmetic.
fn default_condition_max() -> f32 {
    100.0
}

/// Default share of hull damage that also degrades condition.
///
/// `1.0` — a structure that declares `[infrastructure]` and then takes a phaser
/// to the spine has its condition fall point for point with its hull, which is
/// the behaviour an author who wrote the block down almost certainly meant. A
/// structure whose condition is purely script-driven authors `0.0`.
fn default_hull_damage_share() -> f32 {
    1.0
}

/// Default restore band added to every threshold that does not author its own
/// `restores_above`. See the module docs on chatter.
fn default_hysteresis() -> f32 {
    0.05
}

/// Default for [`InfrastructureConfig::publish`].
fn default_publish() -> bool {
    true
}

/// The `[infrastructure]` table on an entity TOML.
///
/// Every field is optional; an entity that omits the table entirely behaves
/// exactly as it did before this existed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfrastructureConfig {
    /// Condition ceiling in condition points.
    #[serde(default = "default_condition_max")]
    pub condition_max: f32,
    /// Starting condition in points. `None` starts the structure intact, at
    /// `condition_max` — a mission that opens on an already-battered skyhook
    /// authors the lower number here rather than scripting a hit on tick one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<f32>,
    /// Authored decay in condition points per second, applied every tick with
    /// no further prompting. `0.0` (the default) means the structure only moves
    /// when something moves it.
    #[serde(default)]
    pub decay_per_sec: f32,
    /// Condition points lost per point of hull lost. See
    /// [`default_hull_damage_share`].
    #[serde(default = "default_hull_damage_share")]
    pub hull_damage_share: f32,
    /// Restore band added to every threshold that does not author its own
    /// `restores_above`, as a fraction of `condition_max`.
    #[serde(default = "default_hysteresis")]
    pub hysteresis: f32,
    /// Whether the condition/capacity block reaches the wire payload at all.
    /// `true` by default; a scenario that wants a structure's condition kept
    /// off every console authors `false`. There is no third state — what this
    /// module holds IS the truth, and a dossier's contradicting account is
    /// authored elsewhere rather than hidden in here.
    #[serde(default = "default_publish")]
    pub publish: bool,
    /// The `[[workforce]]` id of the people who staff this structure
    /// (issue #1035), or `None` for a structure nobody works — a nav beacon, an
    /// automated relay, a rock.
    ///
    /// A *machine* id in the scenario's namespace, like a capacity id: it is
    /// matched against the world's authored
    /// [`Workforce::id`](crate::world::workforce::Workforce::id) and never
    /// rendered. Naming a workforce a given world has no dispute about is
    /// legal and means exactly what it says — the same depot template ships in
    /// five scenarios and only one of them has a strike — so this is
    /// deliberately not cross-checked at load. See
    /// [`WorkforceRegister::on_strike`](crate::world::workforce::WorkforceRegister::on_strike).
    ///
    /// What a stoppage *does* to crew-owned tractor, dock, umbilical or repair
    /// systems is not implicit in this association. The workforce register
    /// publishes its state for the scenario to gate objectives, dialogue and
    /// consequences; physical systems continue to obey their own authored
    /// eligibility unless the scenario changes the state they read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workforce: Option<String>,
    /// Named capacities, in authored order.
    #[serde(default, rename = "capacity", skip_serializing_if = "Vec::is_empty")]
    pub capacities: Vec<CapacityConfig>,
    /// Degradation thresholds, in authored order.
    #[serde(default, rename = "threshold", skip_serializing_if = "Vec::is_empty")]
    pub thresholds: Vec<ThresholdConfig>,
}

impl Default for InfrastructureConfig {
    /// Hand-written so it calls the same `default_*` fns serde does — two
    /// copies of these numbers could only ever drift apart.
    fn default() -> Self {
        Self {
            condition_max: default_condition_max(),
            condition: None,
            decay_per_sec: 0.0,
            hull_damage_share: default_hull_damage_share(),
            hysteresis: default_hysteresis(),
            publish: default_publish(),
            workforce: None,
            capacities: Vec::new(),
            thresholds: Vec::new(),
        }
    }
}

/// One `[[infrastructure.capacity]]` block: a named authored quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityConfig {
    /// Identifier a consumer asks by (`transfer_throughput`, `berths`). Not
    /// display text — it never reaches a player-facing surface as prose. It is
    /// also the counter name the adapter mirrors this capacity onto in the
    /// world flag store, which is what makes the number readable from a script
    /// predicate; the authored id is therefore the scenario's namespace, the
    /// same way a threshold's `flag` is.
    pub id: String,
    /// The authored quantity.
    ///
    /// A whole number, because every capacity this vocabulary is for is a
    /// count — souls held, units moved per window, berths — and because the
    /// world flag store the adapter mirrors it onto is an `i64` counter store.
    /// A float here would either round on the way out or give scripts a
    /// second, lossier answer than the wire's.
    ///
    /// Since #1027 this is the **starting level** rather than a fixed number: a
    /// `transfer` operation moves it. `ceiling` below is what it may grow back
    /// to.
    pub amount: i64,
    /// `strings.csv` id for a crew-facing label, when the scenario wants this
    /// capacity readable on a dossier (issue #1030).
    ///
    /// Optional, and its absence is a decision rather than an oversight: `id`
    /// above is explicitly a machine name and must never be shown as prose, so a
    /// capacity reaches a fact sheet only when an author has written down what
    /// to call it. That is the second of the two gates the dossier projection
    /// applies — the first is `publish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// The most this capacity can ever hold (issue #1027). `None` — the
    /// default, and what every capacity authored before transfers existed
    /// says — means the starting `amount` is also the ceiling, so a depot
    /// authored full can be emptied and refilled to exactly where it began and
    /// no further.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<i64>,
}

/// One capacity as the **live** track carries it (issue #1027).
///
/// The resolved sibling of [`CapacityConfig`], for the reason
/// [`ResolvedThreshold`] is [`ThresholdConfig`]'s: the authored table says what
/// a structure starts with, and the live one has to say what it has now and
/// what it could hold. Keeping the ceiling resolved here rather than as an
/// `Option` means a level that has moved cannot be mistaken for the ceiling it
/// was authored at.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCapacity {
    /// The authored id — the scenario's namespace, and the world-flag counter
    /// name this capacity is mirrored onto.
    pub id: String,
    /// How much is there now.
    pub level: i64,
    /// The most it can hold.
    pub ceiling: i64,
    /// `strings.csv` id for a crew-facing label, carried through from
    /// [`CapacityConfig::label`] so the dossier projection (issue #1030) reads
    /// the LIVE capacity without a side-trip back to the authored table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl ResolvedCapacity {
    /// How much more the ceiling would still admit. Never negative, so a
    /// structure authored above its own ceiling reads as simply full rather
    /// than as owing capacity.
    pub fn headroom(&self) -> i64 {
        self.ceiling.saturating_sub(self.level).max(0)
    }
}

/// One queued move of a named capacity (issue #1027).
///
/// Queued rather than applied where it is decided, for the reason
/// [`ConditionAdjustment`] is: every move of a structure's published numbers
/// goes through [`crate::infrastructure::tick_infrastructure_condition`], which
/// is the one place that re-publishes the counter a scenario predicate reads.
/// A transfer writing the component directly would move the goods and leave the
/// script store saying they were still where they started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityAdjustment {
    /// The structure's `EntityUuid`.
    pub uuid: String,
    /// The `[[infrastructure.capacity]]` id being moved.
    pub capacity: String,
    /// How much to add. Negative takes it away.
    pub delta: i64,
}

/// One `[[infrastructure.threshold]]` block: an operational flag and the
/// condition fractions at which it falls and returns.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdConfig {
    /// The operational flag this threshold owns. The author's namespace: two
    /// entities declaring the same name are declaring the same flag.
    pub flag: String,
    /// Optional capacity whose live fraction this threshold reads instead of
    /// the condition track (issue #1135). The capacity must be declared on this
    /// same infrastructure block. Omitting it retains the original condition-
    /// backed threshold semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<String>,
    /// Source fraction below which the flag falls. The source is the condition
    /// track by default, or `capacity / capacity.ceiling` when `capacity` is
    /// authored above.
    pub fails_below: f32,
    /// Source fraction at or above which the flag returns. `None` resolves to
    /// `fails_below + hysteresis`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restores_above: Option<f32>,
    /// `strings.csv` id for a crew-facing label, when the scenario wants this
    /// operational flag readable on a dossier (issue #1030). Optional for
    /// [`CapacityConfig::label`]'s reason: `flag` is a machine name in the
    /// author's namespace, not display text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl InfrastructureConfig {
    /// Reject an `[infrastructure]` table that cannot mean anything.
    ///
    /// Called at entity-config parse time so a typo is a load error naming the
    /// field, not a structure that silently never degrades.
    pub fn validate(&self) -> Result<(), String> {
        if !self.condition_max.is_finite() || self.condition_max <= 0.0 {
            return Err(format!(
                "[infrastructure] condition_max must be a positive finite number, got {}",
                self.condition_max
            ));
        }
        if let Some(start) = self.condition {
            if !start.is_finite() || start < 0.0 || start > self.condition_max {
                return Err(format!(
                    "[infrastructure] condition must be between 0 and condition_max ({}), got {start}",
                    self.condition_max
                ));
            }
        }
        if !self.decay_per_sec.is_finite() || self.decay_per_sec < 0.0 {
            return Err(format!(
                "[infrastructure] decay_per_sec must be a non-negative finite number, got {}",
                self.decay_per_sec
            ));
        }
        if !self.hull_damage_share.is_finite() || self.hull_damage_share < 0.0 {
            return Err(format!(
                "[infrastructure] hull_damage_share must be a non-negative finite number, got {}",
                self.hull_damage_share
            ));
        }
        if !self.hysteresis.is_finite() || self.hysteresis < 0.0 {
            return Err(format!(
                "[infrastructure] hysteresis must be a non-negative finite number, got {}",
                self.hysteresis
            ));
        }
        if let Some(workforce) = &self.workforce {
            if workforce.trim().is_empty() {
                return Err(
                    "[infrastructure] workforce must be a non-empty [[workforce]] id, or be \
                     omitted entirely for a structure nobody works"
                        .to_string(),
                );
            }
        }
        for (index, capacity) in self.capacities.iter().enumerate() {
            if capacity.id.trim().is_empty() {
                return Err("[[infrastructure.capacity]] needs a non-empty id".to_string());
            }
            if capacity.amount < 0 {
                return Err(format!(
                    "[[infrastructure.capacity]] {} amount must be a non-negative whole number, got {}",
                    capacity.id, capacity.amount
                ));
            }
            if let Some(ceiling) = capacity.ceiling {
                if ceiling < capacity.amount {
                    return Err(format!(
                        "[[infrastructure.capacity]] {} has ceiling {ceiling} below its starting \
                         amount {} — a structure cannot begin holding more than it can hold",
                        capacity.id, capacity.amount
                    ));
                }
            }
            if self.capacities[..index].iter().any(|c| c.id == capacity.id) {
                return Err(format!(
                    "[[infrastructure.capacity]] id {} is declared twice — a consumer asking for it \
                     would get whichever came first",
                    capacity.id
                ));
            }
        }
        for (index, threshold) in self.thresholds.iter().enumerate() {
            if threshold.flag.trim().is_empty() {
                return Err("[[infrastructure.threshold]] needs a non-empty flag".to_string());
            }
            if let Some(capacity) = &threshold.capacity {
                if capacity.trim().is_empty() {
                    return Err(format!(
                        "[[infrastructure.threshold]] {} capacity must be a non-empty id",
                        threshold.flag
                    ));
                }
                if !self.capacities.iter().any(|c| c.id == *capacity) {
                    return Err(format!(
                        "[[infrastructure.threshold]] {} reads capacity '{capacity}', but this \
                         entity declares no [[infrastructure.capacity]] with that id",
                        threshold.flag
                    ));
                }
            }
            if !(0.0..=1.0).contains(&threshold.fails_below) {
                return Err(format!(
                    "[[infrastructure.threshold]] {} fails_below must be a source FRACTION in \
                     0.0..=1.0, got {}",
                    threshold.flag, threshold.fails_below
                ));
            }
            if let Some(restore) = threshold.restores_above {
                if !(0.0..=1.0).contains(&restore) {
                    return Err(format!(
                        "[[infrastructure.threshold]] {} restores_above must be a source \
                         FRACTION in 0.0..=1.0, got {restore}",
                        threshold.flag
                    ));
                }
                if restore < threshold.fails_below {
                    return Err(format!(
                        "[[infrastructure.threshold]] {} restores_above ({restore}) is below \
                         fails_below ({}) — that inverts the hysteresis band",
                        threshold.flag, threshold.fails_below
                    ));
                }
            }
            if self.thresholds[..index]
                .iter()
                .any(|t| t.flag == threshold.flag)
            {
                return Err(format!(
                    "[[infrastructure.threshold]] flag {} is declared twice on one entity",
                    threshold.flag
                ));
            }
        }
        Ok(())
    }
}

// ── Runtime state ────────────────────────────────────────────────────────────

/// A threshold with its restore point resolved against the authored hysteresis.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolvedThreshold {
    /// The operational flag this threshold owns.
    pub flag: String,
    /// Optional capacity whose live fraction this threshold reads. `None`
    /// means the entity's condition track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<String>,
    /// Source fraction below which the flag falls.
    pub fails_below: f32,
    /// Source fraction at or above which the flag returns.
    pub restores_above: f32,
    /// The authored crew-facing label, carried through unchanged — see
    /// [`ThresholdConfig::label`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One condition adjustment queued by a scripted repair/damage effect, already
/// resolved to its target entity's UUID.
///
/// Queued rather than applied where it is authored so that **every** condition
/// move — decay, damage, script — goes through the one system that owns the
/// flag edges. An effect that wrote the component directly would flip a
/// threshold nobody was listening at, and the world flag store would never hear
/// about it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConditionAdjustment {
    /// The target entity's `EntityUuid`.
    pub uuid: String,
    /// Condition points to add (negative degrades, positive repairs).
    pub delta: f32,
}

/// One operational flag that changed state during a mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagChange {
    /// The authored flag name.
    pub flag: String,
    /// `true` when the flag is now held (the structure is capable again),
    /// `false` when it just fell.
    pub raised: bool,
}

/// The live condition + capacity track for one entity.
///
/// Fields are private because the flag edges are only correct when every
/// condition move goes through one of the mutators below; a caller that could
/// write `current` directly could skip a crossing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InfrastructureState {
    max: f32,
    current: f32,
    decay_per_sec: f32,
    hull_damage_share: f32,
    publish: bool,
    /// The `[[workforce]]` id staffing this structure (issue #1035), carried
    /// through from the authored table and snapshot as the structure's party
    /// association. Current crew-owned external systems do not infer a hidden
    /// slowdown or refusal from it; scenarios read the workforce register's
    /// published flags and author the consequence explicitly.
    ///
    /// Authored and immutable, like `publish`: nothing moves it, and a
    /// structure does not change hands mid-mission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workforce: Option<String>,
    thresholds: Vec<ResolvedThreshold>,
    /// Held state per threshold, parallel to `thresholds`.
    held: Vec<bool>,
    capacities: Vec<ResolvedCapacity>,
    /// The entity's aggregate hull total as of the last [`Self::observe_hull`]
    /// call. Kept here rather than in a side table so it snapshots and resumes
    /// with the rest of the track — a resumed structure that forgot it would
    /// book its entire remaining hull as fresh damage on the next tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_hull: Option<f32>,
}

impl InfrastructureState {
    /// Build the live track from an authored (already validated) table.
    ///
    /// Flags start level-evaluated against the starting condition, so a mission
    /// opening on an already-degraded structure opens with the matching flags
    /// already down rather than flipping them on tick one.
    pub fn from_config(config: &InfrastructureConfig) -> Self {
        let max = config.condition_max;
        let current = config.condition.unwrap_or(max).clamp(0.0, max);
        let thresholds: Vec<ResolvedThreshold> = config
            .thresholds
            .iter()
            .map(|t| ResolvedThreshold {
                flag: t.flag.clone(),
                capacity: t.capacity.clone(),
                fails_below: t.fails_below,
                restores_above: t
                    .restores_above
                    .unwrap_or_else(|| (t.fails_below + config.hysteresis).min(1.0)),
                label: t.label.clone(),
            })
            .collect();
        let capacities: Vec<ResolvedCapacity> = config
            .capacities
            .iter()
            .map(|c| ResolvedCapacity {
                id: c.id.clone(),
                level: c.amount,
                // An unauthored ceiling is the starting amount: every
                // capacity written before transfers existed means "this is
                // what the depot holds", and that reading has to keep
                // working unchanged.
                ceiling: c.ceiling.unwrap_or(c.amount).max(c.amount),
                label: c.label.clone(),
            })
            .collect();
        let held = thresholds
            .iter()
            .map(|t| threshold_fraction(t, current, max, &capacities) >= t.fails_below)
            .collect();
        Self {
            max,
            current,
            decay_per_sec: config.decay_per_sec,
            hull_damage_share: config.hull_damage_share,
            publish: config.publish,
            workforce: config.workforce.clone(),
            thresholds,
            held,
            capacities,
            last_hull: None,
        }
    }

    /// Current condition in points.
    pub fn condition(&self) -> f32 {
        self.current
    }

    /// Condition ceiling in points.
    pub fn condition_max(&self) -> f32 {
        self.max
    }

    /// Current condition as a fraction of the ceiling, clamped to `0.0..=1.0`.
    /// A track with no ceiling reads as fully intact rather than dividing by
    /// zero.
    pub fn condition_fraction(&self) -> f32 {
        fraction_of(self.current, self.max)
    }

    /// Authored decay in condition points per second.
    pub fn decay_per_sec(&self) -> f32 {
        self.decay_per_sec
    }

    /// Condition points lost per point of hull lost.
    pub fn hull_damage_share(&self) -> f32 {
        self.hull_damage_share
    }

    /// Whether this track reaches the wire payload.
    pub fn publishes(&self) -> bool {
        self.publish
    }

    /// The `[[workforce]]` id staffing this structure (issue #1035), or `None`
    /// for one nobody works.
    ///
    /// Deliberately NOT on the wire. Which side of a dispute staffs a depot is
    /// something the crew learn by talking to people, and putting it on the
    /// published condition block would have every console read the answer off a
    /// panel before the negotiation started. What the crew *do* see is whichever
    /// stoppage consequence the scenario authors onto Comms or the dossier.
    pub fn workforce(&self) -> Option<&str> {
        self.workforce.as_deref()
    }

    /// The aggregate hull total recorded by the last [`Self::observe_hull`],
    /// or `None` before the first. Read by the adapter so a structure whose
    /// hull has not moved is not marked changed for nothing.
    pub fn last_observed_hull(&self) -> Option<f32> {
        self.last_hull
    }

    /// The current level of a named capacity, or `None` when the structure
    /// never declared one by that name.
    pub fn capacity(&self, id: &str) -> Option<i64> {
        self.capacities.iter().find(|c| c.id == id).map(|c| c.level)
    }

    /// A named capacity whole — its level and its ceiling (issue #1027), which
    /// is what an Umbilical flow needs in order to ask whether this end has
    /// room.
    pub fn capacity_reading(&self, id: &str) -> Option<&ResolvedCapacity> {
        self.capacities.iter().find(|c| c.id == id)
    }

    /// Every capacity, in authored order.
    pub fn capacities(&self) -> &[ResolvedCapacity] {
        &self.capacities
    }

    /// Move a named capacity by `delta`, clamped to `0..=ceiling`
    /// (issue #1027).
    ///
    /// Returns the level it actually landed on, or `None` when this structure
    /// declares no such capacity. Clamping rather than refusing, because the
    /// caller has already asked whether the move fits, and this is the backstop
    /// that keeps a depot's published number honest if two moves ever land in
    /// one tick.
    ///
    /// A plain capacity remains a published quantity rather than an event. When
    /// an authored infrastructure threshold explicitly names this capacity,
    /// crossing that threshold returns the corresponding [`FlagChange`] just
    /// like a condition-backed threshold does.
    pub fn adjust_capacity(&mut self, id: &str, delta: i64) -> Option<(i64, Vec<FlagChange>)> {
        let capacity = self.capacities.iter_mut().find(|c| c.id == id)?;
        capacity.level = capacity
            .level
            .saturating_add(delta)
            .clamp(0, capacity.ceiling);
        let level = capacity.level;
        let changes = self.recompute_flags();
        Some((level, changes))
    }

    /// The resolved thresholds, in authored order.
    pub fn thresholds(&self) -> &[ResolvedThreshold] {
        &self.thresholds
    }

    /// The current state of one named operational flag, or `None` when this
    /// structure declares no threshold by that name.
    pub fn flag(&self, flag: &str) -> Option<bool> {
        self.thresholds
            .iter()
            .position(|t| t.flag == flag)
            .and_then(|index| self.held.get(index).copied())
    }

    /// Every operational flag and its current state, in authored order.
    pub fn flags(&self) -> Vec<(&str, bool)> {
        self.thresholds
            .iter()
            .zip(self.held.iter())
            .map(|(t, held)| (t.flag.as_str(), *held))
            .collect()
    }

    /// Every flag's *starting* state, phrased as changes so a caller can mirror
    /// the whole set on spawn through the same path it mirrors later edges.
    pub fn initial_flags(&self) -> Vec<FlagChange> {
        self.thresholds
            .iter()
            .zip(self.held.iter())
            .map(|(t, held)| FlagChange {
                flag: t.flag.clone(),
                raised: *held,
            })
            .collect()
    }

    /// Move the condition by `delta` points (negative degrades, positive
    /// repairs), clamp into `0.0..=max`, and return the operational flags that
    /// changed as a result, in authored order.
    ///
    /// This is the one mutator; `degrade`, `repair` and `set_condition` are
    /// spellings of it. A timed operation raises condition by calling it once
    /// per tick with a small positive delta — nothing here assumes a repair
    /// arrives in a single jump.
    pub fn apply_delta(&mut self, delta: f32) -> Vec<FlagChange> {
        if !delta.is_finite() || delta == 0.0 {
            return Vec::new();
        }
        self.set_condition((self.current + delta).clamp(0.0, self.max))
    }

    /// Lower the condition by `amount` points. A negative or non-finite amount
    /// is ignored rather than secretly repairing.
    pub fn degrade(&mut self, amount: f32) -> Vec<FlagChange> {
        if !amount.is_finite() || amount <= 0.0 {
            return Vec::new();
        }
        self.apply_delta(-amount)
    }

    /// Raise the condition by `amount` points. A negative or non-finite amount
    /// is ignored rather than secretly degrading.
    pub fn repair(&mut self, amount: f32) -> Vec<FlagChange> {
        if !amount.is_finite() || amount <= 0.0 {
            return Vec::new();
        }
        self.apply_delta(amount)
    }

    /// Fold this tick's aggregate hull total in, booking any DROP since the
    /// previous observation as condition damage at the authored
    /// `hull_damage_share`.
    ///
    /// # Why a level observation rather than a damage hook
    ///
    /// Weapons already damage non-ship entities: the beam, torpedo and blaster
    /// systems all query `EntitySystemHull` with no `With<Ship>` filter, so a
    /// station takes fire today. Region damage zones and collisions are
    /// ship-gated, and further hazards are still to come. Watching the hull
    /// LEVEL catches every one of those without a single edit to any damage
    /// site — present or future — and without a second opinion about how much
    /// damage was dealt. It is the same shape `collect_world_events` already
    /// uses to derive `HullDroppedBelow` from consecutive samples.
    ///
    /// The first observation only records: a structure that spawns with 200
    /// hull has not just taken 200 points of damage.
    pub fn observe_hull(&mut self, total: f32) -> Vec<FlagChange> {
        if !total.is_finite() {
            return Vec::new();
        }
        let previous = self.last_hull.replace(total);
        let Some(previous) = previous else {
            return Vec::new();
        };
        let lost = previous - total;
        if lost <= 0.0 {
            // A hull that was repaired (or held still) does not repair
            // condition: structural condition is its own track, raised only by
            // the repair hooks below. Nothing to book either way.
            return Vec::new();
        }
        self.degrade(lost * self.hull_damage_share)
    }

    /// Set the condition outright, clamped into `0.0..=max`, returning the
    /// flags that changed.
    pub fn set_condition(&mut self, value: f32) -> Vec<FlagChange> {
        if !value.is_finite() {
            return Vec::new();
        }
        self.current = value.clamp(0.0, self.max);
        self.recompute_flags()
    }

    /// Edge-detect every threshold against the stored flag state. See the
    /// module docs for why this is edge-triggered rather than level-computed.
    fn recompute_flags(&mut self) -> Vec<FlagChange> {
        let fractions: Vec<f32> = self
            .thresholds
            .iter()
            .map(|threshold| {
                threshold_fraction(threshold, self.current, self.max, &self.capacities)
            })
            .collect();
        let mut changes = Vec::new();
        for (index, threshold) in self.thresholds.iter().enumerate() {
            let Some(held) = self.held.get_mut(index) else {
                continue;
            };
            let fraction = fractions[index];
            if *held && fraction < threshold.fails_below {
                *held = false;
                changes.push(FlagChange {
                    flag: threshold.flag.clone(),
                    raised: false,
                });
            } else if !*held && fraction >= threshold.restores_above {
                *held = true;
                changes.push(FlagChange {
                    flag: threshold.flag.clone(),
                    raised: true,
                });
            }
        }
        changes
    }
}

/// `current / max`, clamped, with a zero ceiling reading as fully intact.
fn fraction_of(current: f32, max: f32) -> f32 {
    if max <= 0.0 {
        return 1.0;
    }
    (current / max).clamp(0.0, 1.0)
}

/// Read one threshold's configured source as a normalized fraction. A capacity
/// with a zero ceiling reads empty, not intact: unlike a condition track, zero
/// capacity is a real authored "nothing can be transferred" state.
fn threshold_fraction(
    threshold: &ResolvedThreshold,
    condition: f32,
    condition_max: f32,
    capacities: &[ResolvedCapacity],
) -> f32 {
    let Some(id) = threshold.capacity.as_deref() else {
        return fraction_of(condition, condition_max);
    };
    capacities
        .iter()
        .find(|capacity| capacity.id == id)
        .map(|capacity| {
            if capacity.ceiling <= 0 {
                0.0
            } else {
                (capacity.level as f32 / capacity.ceiling as f32).clamp(0.0, 1.0)
            }
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "condition_tests.rs"]
mod tests;
