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
    pub amount: i64,
}

/// One `[[infrastructure.threshold]]` block: an operational flag and the
/// condition fractions at which it falls and returns.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdConfig {
    /// The operational flag this threshold owns. The author's namespace: two
    /// entities declaring the same name are declaring the same flag.
    pub flag: String,
    /// Condition fraction below which the flag falls.
    pub fails_below: f32,
    /// Condition fraction at or above which the flag returns. `None` resolves
    /// to `fails_below + hysteresis`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restores_above: Option<f32>,
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
            if !(0.0..=1.0).contains(&threshold.fails_below) {
                return Err(format!(
                    "[[infrastructure.threshold]] {} fails_below must be a condition FRACTION in \
                     0.0..=1.0, got {}",
                    threshold.flag, threshold.fails_below
                ));
            }
            if let Some(restore) = threshold.restores_above {
                if !(0.0..=1.0).contains(&restore) {
                    return Err(format!(
                        "[[infrastructure.threshold]] {} restores_above must be a condition \
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
    /// Condition fraction below which the flag falls.
    pub fails_below: f32,
    /// Condition fraction at or above which the flag returns.
    pub restores_above: f32,
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
    thresholds: Vec<ResolvedThreshold>,
    /// Held state per threshold, parallel to `thresholds`.
    held: Vec<bool>,
    capacities: Vec<CapacityConfig>,
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
                fails_below: t.fails_below,
                restores_above: t
                    .restores_above
                    .unwrap_or_else(|| (t.fails_below + config.hysteresis).min(1.0)),
            })
            .collect();
        let fraction = fraction_of(current, max);
        let held = thresholds
            .iter()
            .map(|t| fraction >= t.fails_below)
            .collect();
        Self {
            max,
            current,
            decay_per_sec: config.decay_per_sec,
            hull_damage_share: config.hull_damage_share,
            publish: config.publish,
            thresholds,
            held,
            capacities: config.capacities.clone(),
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

    /// The aggregate hull total recorded by the last [`Self::observe_hull`],
    /// or `None` before the first. Read by the adapter so a structure whose
    /// hull has not moved is not marked changed for nothing.
    pub fn last_observed_hull(&self) -> Option<f32> {
        self.last_hull
    }

    /// The authored amount for a named capacity, or `None` when the structure
    /// never declared one by that name.
    pub fn capacity(&self, id: &str) -> Option<i64> {
        self.capacities
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.amount)
    }

    /// Every authored capacity, in authored order.
    pub fn capacities(&self) -> &[CapacityConfig] {
        &self.capacities
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
        let fraction = fraction_of(self.current, self.max);
        let mut changes = Vec::new();
        for (index, threshold) in self.thresholds.iter().enumerate() {
            let Some(held) = self.held.get_mut(index) else {
                continue;
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A depot with one capacity and one `transfer_capable` threshold that
    /// fails below 40 % and, with the default 0.05 band, returns at 45 %.
    fn depot() -> InfrastructureConfig {
        InfrastructureConfig {
            condition_max: 100.0,
            capacities: vec![CapacityConfig {
                id: "transfer_throughput".to_string(),
                amount: 40,
            }],
            thresholds: vec![ThresholdConfig {
                flag: "depot_transfer_capable".to_string(),
                fails_below: 0.4,
                restores_above: None,
            }],
            ..Default::default()
        }
    }

    // ── AC1: an omitted table changes nothing; an authored one parses ──

    #[test]
    fn an_authored_table_takes_the_documented_defaults_for_everything_it_omits() {
        let parsed: InfrastructureConfig = toml::from_str("").expect("an empty table is legal");
        assert_eq!(
            parsed,
            InfrastructureConfig::default(),
            "serde's defaults and the hand-written Default impl must agree — two copies of \
             these numbers could only drift"
        );
        assert_eq!(
            parsed.condition_max, 100.0,
            "the condition ceiling falls back to the documented parse default"
        );
        assert!(
            parsed.thresholds.is_empty() && parsed.capacities.is_empty(),
            "a table that declares neither thresholds nor capacities declares neither"
        );
    }

    #[test]
    fn the_authored_vocabulary_round_trips_through_toml() {
        let authored = r#"
condition_max = 200.0
condition = 150.0
decay_per_sec = 0.5
hull_damage_share = 0.25
hysteresis = 0.1
publish = false

[[capacity]]
id = "berths"
amount = 12

[[threshold]]
flag = "docking_capable"
fails_below = 0.3
restores_above = 0.6
"#;
        let parsed: InfrastructureConfig = toml::from_str(authored).expect("the vocabulary parses");
        assert_eq!(parsed.condition_max, 200.0);
        assert_eq!(parsed.condition, Some(150.0));
        assert_eq!(parsed.decay_per_sec, 0.5);
        assert_eq!(parsed.hull_damage_share, 0.25);
        assert_eq!(parsed.hysteresis, 0.1);
        assert!(!parsed.publish);
        assert_eq!(
            parsed.capacities.len(),
            1,
            "[[capacity]] is a repeated block"
        );
        assert_eq!(
            parsed.thresholds.len(),
            1,
            "[[threshold]] is a repeated block"
        );
        parsed.validate().expect("the authored table is valid");
    }

    #[test]
    fn an_unknown_key_is_a_parse_error_rather_than_a_silently_ignored_typo() {
        let err = toml::from_str::<InfrastructureConfig>("conditon_max = 50.0")
            .expect_err("a misspelt key must not be swallowed");
        assert!(
            err.to_string().contains("conditon_max"),
            "the error must name the offending key, got {err}"
        );
    }

    // ── AC2: thresholds flip in BOTH directions, with a dead band ──

    #[test]
    fn a_threshold_falls_on_the_way_down_and_returns_on_the_way_up() {
        let mut state = InfrastructureState::from_config(&depot());
        assert_eq!(
            state.flag("depot_transfer_capable"),
            Some(true),
            "an intact depot starts capable"
        );

        let falling = state.degrade(65.0);
        assert_eq!(
            falling,
            vec![FlagChange {
                flag: "depot_transfer_capable".to_string(),
                raised: false
            }],
            "dropping to 35 % crosses fails_below (40 %) and reports the flag falling"
        );
        assert_eq!(state.flag("depot_transfer_capable"), Some(false));

        let rising = state.repair(15.0);
        assert_eq!(
            rising,
            vec![FlagChange {
                flag: "depot_transfer_capable".to_string(),
                raised: true
            }],
            "climbing to 50 % clears restores_above (45 %) and reports the flag returning"
        );
        assert_eq!(state.flag("depot_transfer_capable"), Some(true));
    }

    #[test]
    fn a_condition_parked_inside_the_hysteresis_band_reports_no_transition() {
        let mut state = InfrastructureState::from_config(&depot());
        state.degrade(65.0);
        assert_eq!(state.flag("depot_transfer_capable"), Some(false));

        // 35 % → 42 %: past fails_below, short of restores_above. This is the
        // exact value a hovering structure sits at, and it must be silent.
        let changes = state.repair(7.0);
        assert!(
            changes.is_empty(),
            "a value inside the dead band must report nothing, got {changes:?}"
        );
        assert_eq!(
            state.flag("depot_transfer_capable"),
            Some(false),
            "…and must stay down until the restore point is actually reached"
        );
    }

    #[test]
    fn a_point_of_damage_and_a_point_of_repair_on_the_boundary_do_not_chatter() {
        let mut state = InfrastructureState::from_config(&depot());
        for lap in 0..8 {
            // Park the depot back on the failure line, capable, for each lap.
            state.set_condition(100.0);
            state.set_condition(40.0);
            assert_eq!(
                state.flag("depot_transfer_capable"),
                Some(true),
                "lap {lap}: sitting exactly on fails_below is still capable — the flag falls \
                 BELOW it"
            );
            let down = state.degrade(1.0);
            assert_eq!(
                down.len(),
                1,
                "lap {lap}: the first point below the line is a genuine crossing"
            );
            let up = state.repair(1.0);
            assert!(
                up.is_empty(),
                "lap {lap}: putting that one point back lands in the dead band, so the flag \
                 stays down instead of flapping once per tick, got {up:?}"
            );
        }
    }

    #[test]
    fn an_authored_restore_point_overrides_the_default_band() {
        let mut config = depot();
        config.thresholds[0].restores_above = Some(0.9);
        let mut state = InfrastructureState::from_config(&config);
        state.degrade(70.0);
        assert_eq!(state.flag("depot_transfer_capable"), Some(false));
        assert!(
            state.repair(50.0).is_empty(),
            "80 % is above the default band but below the authored 90 % restore point"
        );
        assert_eq!(
            state.repair(10.0),
            vec![FlagChange {
                flag: "depot_transfer_capable".to_string(),
                raised: true
            }],
            "…and 90 % is the authored restore point exactly, so the flag returns there"
        );
    }

    #[test]
    fn a_structure_that_starts_degraded_starts_with_its_flag_already_down() {
        let mut config = depot();
        config.condition = Some(10.0);
        let state = InfrastructureState::from_config(&config);
        assert_eq!(
            state.flag("depot_transfer_capable"),
            Some(false),
            "flags are level-evaluated once at construction, so a mission opening on a wrecked \
             skyhook opens with the flag down rather than flipping it on tick one"
        );
        assert_eq!(
            state.initial_flags(),
            vec![FlagChange {
                flag: "depot_transfer_capable".to_string(),
                raised: false
            }],
            "…and the starting set is phrased as changes so the caller mirrors it through the \
             same path it mirrors later edges"
        );
    }

    // ── AC4: repair raises condition, incrementally ──

    #[test]
    fn a_repair_can_arrive_in_arbitrarily_small_increments() {
        let mut config = depot();
        config.condition = Some(30.0);
        let mut state = InfrastructureState::from_config(&config);
        assert_eq!(
            state.flag("depot_transfer_capable"),
            Some(false),
            "precondition: 30 % starts below the 40 % failure point"
        );
        let mut crossings = 0;
        for _ in 0..400 {
            // A timed operation's per-tick slice: 0.05 points at a time.
            crossings += state.repair(0.05).len();
        }
        // Within a float's worth of 50: four hundred `f32` additions of 0.05 do
        // not land on the number a single `+ 20.0` would, and the tolerance
        // says so rather than pretending otherwise.
        assert!(
            (state.condition() - 50.0).abs() < 0.01,
            "four hundred slices of 0.05 must add up to the 20 points a single jump would, \
             got {}",
            state.condition()
        );
        assert_eq!(
            crossings, 1,
            "the crossing is reported exactly once, on the slice that carries the condition \
             over the 45 % restore point — not once per slice above it, and not never"
        );
    }

    #[test]
    fn condition_is_clamped_at_both_ends() {
        let mut state = InfrastructureState::from_config(&depot());
        state.repair(1_000.0);
        assert_eq!(
            state.condition(),
            100.0,
            "a repair cannot exceed the ceiling"
        );
        state.degrade(1_000.0);
        assert_eq!(
            state.condition(),
            0.0,
            "damage cannot drive condition negative"
        );
        assert_eq!(state.condition_fraction(), 0.0);
    }

    #[test]
    fn a_degrade_refuses_a_negative_amount_instead_of_repairing() {
        let mut state = InfrastructureState::from_config(&depot());
        assert!(state.degrade(-50.0).is_empty());
        assert_eq!(
            state.condition(),
            100.0,
            "a negative degrade is ignored — the sign convention is in the method name, and a \
             caller that got it wrong must not silently heal the structure"
        );
        assert!(state.repair(-50.0).is_empty());
        assert_eq!(state.condition(), 100.0);
    }

    // ── AC3-adjacent: damage drives condition down ──

    #[test]
    fn the_first_hull_observation_only_records_the_baseline() {
        let mut state = InfrastructureState::from_config(&depot());
        assert!(state.observe_hull(200.0).is_empty());
        assert_eq!(
            state.condition(),
            100.0,
            "a structure that spawns with 200 hull has not just taken 200 points of damage"
        );
    }

    #[test]
    fn a_hull_drop_is_booked_as_condition_damage_at_the_authored_share() {
        let mut config = depot();
        config.hull_damage_share = 0.5;
        let mut state = InfrastructureState::from_config(&config);
        state.observe_hull(200.0);
        assert!(
            state.observe_hull(120.0).is_empty(),
            "80 hull → 40 condition"
        );
        assert_eq!(
            state.condition(),
            60.0,
            "the authored share is what converts hull points into condition points"
        );
        // 60 more hull points off is 30 more condition points: 30/100 = 30 %,
        // the first reading strictly below the authored 40 %.
        let changes = state.observe_hull(60.0);
        assert_eq!(
            changes,
            vec![FlagChange {
                flag: "depot_transfer_capable".to_string(),
                raised: false
            }],
            "…and a hit that takes condition through a threshold reports the crossing, so the \
             damage path and the script path flip the same flag by the same rule"
        );
    }

    #[test]
    fn a_share_of_zero_leaves_condition_entirely_to_the_scenario() {
        let mut config = depot();
        config.hull_damage_share = 0.0;
        let mut state = InfrastructureState::from_config(&config);
        state.observe_hull(200.0);
        state.observe_hull(1.0);
        assert_eq!(
            state.condition(),
            100.0,
            "a structure whose condition is purely script-driven authors share = 0 and is not \
             quietly degraded by the hull it happens to also carry"
        );
    }

    #[test]
    fn a_hull_that_climbs_does_not_repair_condition() {
        let mut state = InfrastructureState::from_config(&depot());
        state.observe_hull(100.0);
        state.observe_hull(60.0);
        assert_eq!(state.condition(), 60.0);
        assert!(state.observe_hull(100.0).is_empty());
        assert_eq!(
            state.condition(),
            60.0,
            "condition is its own track: only the repair hooks raise it, so a hull restored by \
             some other means does not silently un-degrade the structure"
        );
    }

    // ── AC5: capacity is readable without hard-coding the number ──

    #[test]
    fn a_consumer_can_ask_a_depot_how_much_it_moves() {
        let state = InfrastructureState::from_config(&depot());
        assert_eq!(
            state.capacity("transfer_throughput"),
            Some(40),
            "the authored number is the answer"
        );
        assert_eq!(
            state.capacity("berths"),
            None,
            "…and a capacity the structure never declared reads as absent rather than zero, so \
             a consumer can tell 'holds nobody' apart from 'does not do berths'"
        );
    }

    #[test]
    fn capacity_does_not_move_when_condition_does() {
        let mut state = InfrastructureState::from_config(&depot());
        state.degrade(90.0);
        assert_eq!(
            state.capacity("transfer_throughput"),
            Some(40),
            "capacity is an authored property of the structure. A scenario that wants a \
             battered depot to move less reads the condition and decides that itself, rather \
             than having an implicit curve applied underneath it."
        );
    }

    // ── AC6: nothing here is a hidden-truth field ──

    #[test]
    fn every_readable_field_is_one_the_scenario_authored() {
        let config = depot();
        let state = InfrastructureState::from_config(&config);
        assert_eq!(state.condition(), config.condition_max);
        assert_eq!(
            state.flags(),
            vec![("depot_transfer_capable", true)],
            "the flag set is exactly the authored thresholds"
        );
        assert_eq!(state.capacities().len(), config.capacities.len());
        assert!(
            state.publishes(),
            "publication is authored, defaulting to on; a scenario that wants a structure's \
             condition off every console says so"
        );
    }

    // ── Validation ──

    #[test]
    fn validation_rejects_the_tables_that_cannot_mean_anything() {
        let cases: Vec<(InfrastructureConfig, &str)> = vec![
            (
                InfrastructureConfig {
                    condition_max: 0.0,
                    ..Default::default()
                },
                "condition_max",
            ),
            (
                InfrastructureConfig {
                    condition: Some(500.0),
                    ..Default::default()
                },
                "condition",
            ),
            (
                InfrastructureConfig {
                    decay_per_sec: -1.0,
                    ..Default::default()
                },
                "decay_per_sec",
            ),
            (
                InfrastructureConfig {
                    hull_damage_share: -1.0,
                    ..Default::default()
                },
                "hull_damage_share",
            ),
            (
                InfrastructureConfig {
                    thresholds: vec![ThresholdConfig {
                        flag: "a".to_string(),
                        fails_below: 40.0,
                        restores_above: None,
                    }],
                    ..Default::default()
                },
                "FRACTION",
            ),
            (
                InfrastructureConfig {
                    thresholds: vec![ThresholdConfig {
                        flag: "a".to_string(),
                        fails_below: 0.5,
                        restores_above: Some(0.2),
                    }],
                    ..Default::default()
                },
                "inverts",
            ),
            (
                InfrastructureConfig {
                    thresholds: vec![
                        ThresholdConfig {
                            flag: "a".to_string(),
                            fails_below: 0.5,
                            restores_above: None,
                        },
                        ThresholdConfig {
                            flag: "a".to_string(),
                            fails_below: 0.2,
                            restores_above: None,
                        },
                    ],
                    ..Default::default()
                },
                "twice",
            ),
            (
                InfrastructureConfig {
                    capacities: vec![
                        CapacityConfig {
                            id: "a".to_string(),
                            amount: 1,
                        },
                        CapacityConfig {
                            id: "a".to_string(),
                            amount: 2,
                        },
                    ],
                    ..Default::default()
                },
                "twice",
            ),
        ];
        for (config, expected) in cases {
            let err = config
                .validate()
                .expect_err("this table is not authorable and must be refused");
            assert!(
                err.contains(expected),
                "the refusal must name what is wrong ({expected}), got {err}"
            );
        }
    }

    #[test]
    fn the_shipped_shape_of_a_valid_table_passes_validation() {
        depot()
            .validate()
            .expect("the exemplar depot is authorable");
        InfrastructureConfig::default()
            .validate()
            .expect("an empty [infrastructure] table is authorable");
    }

    // ── Serde round-trip (the snapshot payload leans on this) ──

    #[test]
    fn live_state_round_trips_through_serde_with_its_flag_edges_intact() {
        let mut state = InfrastructureState::from_config(&depot());
        state.degrade(65.0);
        let bytes = toml::to_string(&state).expect("the live track serialises");
        let restored: InfrastructureState = toml::from_str(&bytes).expect("…and comes back");
        assert_eq!(
            restored, state,
            "a resumed structure must remember its condition AND which flags are currently \
             down — restoring the number alone would re-fire the crossing on the next tick"
        );
        assert_eq!(restored.flag("depot_transfer_capable"), Some(false));
    }
}
