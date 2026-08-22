//! The pure, Bevy-free **scan derivation** (issue #1032, parent #851).
//!
//! Science needs something to do that is not reading back the label the author
//! wrote. A scan points the hull's sensor suite at a structure and returns what
//! that structure's **condition track actually says right now** — the number,
//! the operational flags and the capacity levels #1025 keeps. Change the
//! structure's condition and the scan says something different, and no author
//! touches a line of copy to make that happen.
//!
//! # THE READING IS A DERIVATION. THERE IS NO AUTHORED SCAN TEXT
//!
//! `pasm/spec/design/simulation-differentiation.yaml` states the principle —
//! "sensors reveal state rather than scenario text" — and then states the
//! failure mode it must not be satisfied by: *"scripted exposition dressed as
//! sensor output"*. That is a real temptation, because it is much easier to
//! author `scan_result = "The pylon is fractured."` than to compose a reading.
//! So this module is built so the easy thing has nowhere to live:
//!
//! * [`ScanSubject`] is **the whole input port**, and it has exactly two things
//!   in it — who the subject is, and its already-published condition track. It
//!   has no field for a result, a description, a summary or a narration, so
//!   there is nothing for authored prose to arrive on. Adding one would take a
//!   new field on this struct, in a diff, next to this paragraph.
//! * Every string on a [`ScanReading`] is either a **`strings.csv` id an author
//!   wrote against the thing it names** — the structure's own name, a
//!   threshold's `label`, a capacity's `label`, the fidelity band's `label` —
//!   or a machine code. No id here describes a *result*; they describe the
//!   quantities, and the quantities come from state.
//! * The only numbers on a reading are the subject's own condition fraction,
//!   its capacity levels, and its flag states — read through
//!   [`InfrastructureSnapshot`], which is minted from the live track.
//!
//! So a scan is a **composition of already-authored labels over live numbers**.
//! That is the distinction the differentiation spec is drawing, and it is the
//! one thing in this file that a reviewer should check first.
//!
//! # It is not a new door onto withheld truth either
//!
//! Issue #1030 closed a specific leak: a scenario that authors `[infrastructure]
//! publish = false` is saying "keep this structure's condition off every
//! console", and `InfrastructureSnapshot::from_state` is the single gate that
//! enforces it. A scan is a *console readout*, so it goes through that same
//! gate — [`ScanSubject::condition`] is a
//! [`SubjectCondition`](crate::dossier::SubjectCondition), the exact type the
//! dossier projection consumes and the only way to build one is from a
//! published snapshot. A withheld track therefore arrives here as `None` and
//! the scan is **refused** ([`ScanRefusal::NoReadableCondition`]) rather than
//! answered.
//!
//! That is deliberate and it is worth being explicit about, because "a scan
//! reads live authoritative state" could be read as licence to bypass the gate.
//! It is not. What this slice adds is a new **act** — the crew going and
//! getting a reading, at a fidelity their own choices decide — not a new door.
//! A scenario whose secret is meant to be findable publishes the track and
//! contradicts it in the briefing; a scenario that authors `publish = false`
//! has said the sensors get nothing, and they get nothing.
//!
//! # Fidelity is a decision the crew make, out of authored data
//!
//! A free, perfect reveal is not a decision. Every gate below is an authored
//! number on the hull's `[scan]` table, and every one of them is something the
//! crew can act on:
//!
//! * **Range** — `[[scan.band]]` blocks, finest first. The nearest band whose
//!   `max_range` still covers the subject is the one that answers; past the
//!   last band there is no answer at all. A coarse band reports condition
//!   rounded to a coarser step and may withhold the flag or capacity lists
//!   entirely, so *fly closer* buys a better reading.
//! * **Power** — `power_group` / `min_power_level`, read off the ship's own
//!   grid. Spending points on the sensor bus is a Power officer's call.
//! * **Interference** — `degraded_by` names region effects (the `[[region]]`
//!   vocabulary #1027's interrupt rules already use), and standing in one
//!   drops the reading `interference_bands` steps coarser. Waiting for the
//!   storm band to pass, or leaving it, is a Helm call.
//!
//! None of the three is a constant in this file. A hull that wants a single
//! perfect band authors one; a hull that wants a hard four-step ladder authors
//! four.
//!
//! Pure and Bevy-free: the adapter that gathers the live inputs and stores what
//! comes back is [`super::server`].

use serde::{Deserialize, Serialize};

use crate::dossier::SubjectCondition;
use crate::regions::effects::RegionEffectName;

// ── Authored TOML shape ──────────────────────────────────────────────────────

/// Default power group for a `[scan]` table that names none.
///
/// A TOML-parse fallback, the only kind of hardcoded gameplay value AGENTS.md
/// #11 sanctions. `shields` because that is the group #952 put in the sensor
/// bus's place when the three groups became `[helm, weapons, shields]` — a hull
/// whose sensor suite draws from somewhere else says so.
fn default_power_group() -> String {
    "shields".to_string()
}

/// Default minimum power level for a scan. `1` — the floor every group is
/// clamped to anyway, so a hull that authors no power condition is not gated on
/// power at all rather than being gated on a number it never chose.
fn default_min_power_level() -> u8 {
    1
}

/// Default number of bands a scan drops when the operator is standing in an
/// authored interference effect. `1` — one step coarser, which is the smallest
/// move that is still visible on the readout.
fn default_interference_bands() -> u8 {
    1
}

/// Default for [`ScanBandConfig::report_thresholds`] /
/// [`ScanBandConfig::report_capacities`]: a band reports everything unless the
/// author takes something away. The finest band is the common case and it
/// should not have to say so twice.
fn default_report() -> bool {
    true
}

/// The `[scan]` table on a hull's entity TOML.
///
/// Absent for every hull that cannot scan, which is every hull shipped before
/// this existed. The mirror image of `[infrastructure]`: that table says what
/// can be *read off* an entity, this one says what a hull can read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanConfig {
    /// The power group the sensor suite draws on.
    #[serde(default = "default_power_group")]
    pub power_group: String,
    /// The minimum allocation level that group must hold for a scan to return
    /// anything. A whole level rather than a fraction, because that is the unit
    /// the power grid and its console are authored in.
    #[serde(default = "default_min_power_level")]
    pub min_power_level: u8,
    /// The fidelity ladder, **finest first**. A scan resolves to the first band
    /// whose `max_range` still covers the subject.
    #[serde(default, rename = "band", skip_serializing_if = "Vec::is_empty")]
    pub bands: Vec<ScanBandConfig>,
    /// Region effects that degrade this hull's sensors, by their authored name.
    /// Empty (the default) means nothing in the world interferes with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_by: Vec<RegionEffectName>,
    /// How many bands coarser a reading taken inside one of those effects is.
    /// Past the coarsest band the scan returns nothing at all.
    #[serde(default = "default_interference_bands")]
    pub interference_bands: u8,
}

impl Default for ScanConfig {
    /// Hand-written so it calls the same `default_*` fns serde does — two
    /// copies of these numbers could only ever drift apart.
    fn default() -> Self {
        Self {
            power_group: default_power_group(),
            min_power_level: default_min_power_level(),
            bands: Vec::new(),
            degraded_by: Vec::new(),
            interference_bands: default_interference_bands(),
        }
    }
}

/// One `[[scan.band]]` block: how good a reading is, and how far out it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanBandConfig {
    /// Machine name for the band (`"detailed"`, `"coarse"`). Never display
    /// text — it is what a test and a log name the band by, and what the wire
    /// carries as a code beside the label.
    pub id: String,
    /// `strings.csv` id for the crew-facing band name. Required, not optional:
    /// a fidelity the console cannot name is a fidelity the crew cannot act on,
    /// so there is no unlabelled-band case to fall back to.
    pub label: String,
    /// How far out this band still answers, in world units, centre to centre —
    /// the same measure comms range and an operation's `range` take.
    pub max_range: f32,
    /// The step the condition fraction is reported to. `0.01` reads out whole
    /// percent; `0.25` reads out quarters, which is what makes a distant scan
    /// visibly coarser than a close one **without a second copy of the number
    /// anywhere**.
    pub condition_step: f32,
    /// Whether this band resolves the subject's operational flags.
    #[serde(default = "default_report")]
    pub report_thresholds: bool,
    /// Whether this band resolves the subject's capacity levels.
    #[serde(default = "default_report")]
    pub report_capacities: bool,
}

impl ScanConfig {
    /// Refuse an authored table that could never produce a reading.
    ///
    /// Called from the entity-config deserialiser, beside `[infrastructure]`'s
    /// and `[operations]`', so a mistake is a load failure naming the file
    /// rather than a console that quietly answers nothing forever.
    pub fn validate(&self) -> Result<(), String> {
        if self.bands.is_empty() {
            return Err(
                "[scan] authors no [[scan.band]] — a hull that can scan needs at least one \
                 fidelity band, and a hull that cannot should omit the [scan] table"
                    .to_string(),
            );
        }
        let mut seen: Vec<&str> = Vec::new();
        let mut previous_range = 0.0_f32;
        for band in &self.bands {
            if band.id.trim().is_empty() {
                return Err("[[scan.band]] has an empty id".to_string());
            }
            if band.label.trim().is_empty() {
                return Err(format!(
                    "[[scan.band]] '{}' has an empty label — a band the console cannot name is \
                     a fidelity the crew cannot act on",
                    band.id
                ));
            }
            if seen.contains(&band.id.as_str()) {
                return Err(format!(
                    "[[scan.band]] '{}' is declared twice — the first would always win and the \
                     second would never be reachable",
                    band.id
                ));
            }
            seen.push(&band.id);
            if !positive(band.max_range) {
                return Err(format!(
                    "[[scan.band]] '{}' authors max_range = {} — a band with no reach can never \
                     answer",
                    band.id, band.max_range
                ));
            }
            if band.max_range <= previous_range {
                return Err(format!(
                    "[[scan.band]] '{}' authors max_range = {} at or inside the previous band's \
                     {} — bands are authored FINEST FIRST and their ranges must strictly \
                     increase, or the band behind this one could never be reached",
                    band.id, band.max_range, previous_range
                ));
            }
            previous_range = band.max_range;
            if !positive(band.condition_step) || band.condition_step > 1.0 {
                return Err(format!(
                    "[[scan.band]] '{}' authors condition_step = {} — it is a fraction of the \
                     condition ceiling and must be inside (0, 1]",
                    band.id, band.condition_step
                ));
            }
        }
        Ok(())
    }

    /// The band that answers at `distance`, as an index into [`Self::bands`],
    /// or `None` when the subject is past the coarsest band's reach.
    ///
    /// Finest first, so the first band that still covers the distance is the
    /// best available one.
    pub fn band_for(&self, distance: f32) -> Option<usize> {
        self.bands.iter().position(|b| distance <= b.max_range)
    }

    /// Whether any of the effects the operator is standing in is one this hull
    /// authored as interference.
    pub fn interfered_with_by(&self, effects: &[RegionEffectName]) -> bool {
        effects.iter().any(|e| self.degraded_by.contains(e))
    }
}

// ── The input port ───────────────────────────────────────────────────────────

/// Everything the derivation may see about the thing being scanned — **the
/// whole input port**.
///
/// The shortness is the point: see the module docs. There is no field for
/// authored result text because there is no authored result text. `mass`
/// (issue #1154) does not bend that rule — it is not a result either, it is a
/// number [`EntityConfig::mass`](crate::entities::config::EntityConfig::mass)
/// already carries on every entity, unconditionally, before anyone scans
/// anything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanSubject {
    /// The subject entity's UUID.
    pub uuid: String,
    /// `strings.csv` id for its crew-facing name, or empty. The subject's own
    /// authored name — the same id the radar and the dossier already show.
    pub name: String,
    /// The subject's **published** condition track, already through #1025's
    /// `publish` gate and already paired with the labels the scenario authored
    /// for its thresholds and capacities.
    ///
    /// The same type the dossier projection consumes, on purpose: the only way
    /// to build one is
    /// [`SubjectCondition::from_published`](crate::dossier::SubjectCondition::from_published),
    /// whose input is `InfrastructureSnapshot::from_state`'s `Option`. A
    /// withheld track cannot reach this field, so a scan cannot report one.
    pub condition: Option<SubjectCondition>,
    /// The subject's authored mass (issue #1154), in the game's own mass
    /// unit — carried straight off its `EntityMass` component. Never `None`
    /// and never zero: every entity has one, whether an author chose it or it
    /// took the documented parse-time default.
    pub mass: f32,
}

/// This tick's real conditions, as the adapter reads them off the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanConditions {
    /// Centre-to-centre distance from the scanning hull to the subject.
    pub distance: f32,
    /// The allocation level the hull's authored `power_group` is holding.
    pub power_level: u8,
    /// Whether the grid is in its exhaustion lock.
    pub power_locked: bool,
    /// Which authored region effects the scanning hull is standing in, in a
    /// fixed order.
    pub region_effects: Vec<RegionEffectName>,
}

// ── The refusal vocabulary ───────────────────────────────────────────────────

/// Why a scan returned nothing.
///
/// Closed and typed, for [`EvidenceProvenance`](crate::dossier::EvidenceProvenance)'s
/// reason: each has a `strings.csv` row the console resolves, and a fifth kind
/// would need one too. A refusal is information — "there is nothing there to
/// read" and "you are too far out" send the crew to different consoles — so it
/// is never collapsed into a silent empty readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanRefusal {
    /// The hull authored no `[scan]` table at all.
    NotCapable,
    /// Nothing in the world answers to the uuid the command carried.
    NoSuchTarget,
    /// The subject has no readable condition track: either it authors no
    /// `[infrastructure]` block (a rock, a warship, a beacon) or it authors
    /// `publish = false`. **One reason for both**, deliberately: a scenario
    /// that keeps a structure's condition back must not be betrayed by a
    /// refusal that distinguishes "nothing to read" from "something I am not
    /// telling you", which is a leak spelled as an error message.
    NoReadableCondition,
    /// The subject is past the coarsest authored band.
    OutOfRange,
    /// The hull's authored power group is below its authored minimum, or the
    /// grid is locked out.
    Underpowered,
    /// An authored interference effect pushed the reading past the coarsest
    /// band, so the suite is returning noise.
    Blinded,
}

impl ScanRefusal {
    /// Every refusal, in the order a console legend would read them.
    pub const ALL: [Self; 6] = [
        Self::NotCapable,
        Self::NoSuchTarget,
        Self::NoReadableCondition,
        Self::OutOfRange,
        Self::Underpowered,
        Self::Blinded,
    ];

    /// The `strings.csv` id the console resolves. A `match` rather than a
    /// composed `format!("scan.refusal.{…}")`, because a composed id is
    /// invisible to `scripts/check-strings.mjs` and would let a refusal ship
    /// with no row behind it.
    pub fn string_id(self) -> &'static str {
        match self {
            Self::NotCapable => "scan.refusal.not_capable",
            Self::NoSuchTarget => "scan.refusal.no_such_target",
            Self::NoReadableCondition => "scan.refusal.no_readable_condition",
            Self::OutOfRange => "scan.refusal.out_of_range",
            Self::Underpowered => "scan.refusal.underpowered",
            Self::Blinded => "scan.refusal.blinded",
        }
    }
}

// ── The reading ──────────────────────────────────────────────────────────────

/// What a scan came back with.
///
/// A reading is stamped with the tick it was taken on and is **not** recomputed
/// afterwards: the crew read the structure as it stood when they looked at it,
/// at the fidelity their range and their power bought them. That is what makes
/// it comparable against a dossier later, and what makes moving closer and
/// scanning again a thing worth doing.
///
/// Every string is a `strings.csv` id; no English crosses the wire.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanReading {
    /// The subject's UUID — the row key a dossier comparison joins on.
    pub subject_uuid: String,
    /// `strings.csv` id for the subject's crew-facing name, or empty.
    #[serde(default)]
    pub subject_name: String,
    /// The machine code of the band that answered.
    pub band: String,
    /// `strings.csv` id for that band's crew-facing name.
    pub band_label: String,
    /// The `SimTick` the reading was taken on.
    pub taken_at_tick: u64,
    /// Structural condition as a fraction of the subject's authored ceiling,
    /// **rounded to the answering band's step**. The whole fidelity model, in
    /// one number: the underlying value is the live track's, and the coarseness
    /// is the band's.
    pub condition_fraction: f32,
    /// The step it was rounded to, so the console can say how precise this is
    /// rather than implying a precision the band never had.
    pub condition_step: f32,
    /// The subject's authored mass (issue #1154), in the game's own mass
    /// unit, verbatim — the fidelity ladder never coarsens it, unlike
    /// `condition_fraction`, because it is content identity rather than a
    /// live measurement: the same number regardless of which band answered.
    /// `#[serde(default)]` so a save written before this field existed still
    /// deserialises (at `0.0`, on a reading taken before #1154) rather than
    /// refusing to load.
    #[serde(default)]
    pub mass: f32,
    /// `(label id, held)` for each operational flag the subject authored a
    /// label for. Empty when the answering band does not resolve flags.
    #[serde(default)]
    pub flags: Vec<(String, bool)>,
    /// `(label id, level)` for each capacity the subject authored a label for.
    /// Empty when the answering band does not resolve capacities.
    #[serde(default)]
    pub capacities: Vec<(String, i64)>,
}

// ── The mirror flag: that the crew went and looked ───────────────────────────

/// The world flag mirroring that a reading of the structure authored as
/// `[[entity]] id = "<id>"` has come back at least once: `scan.<id>.taken`.
///
/// A free function rather than an inlined `format!` at the one write site, for
/// [`strike_flag`](crate::world::workforce::strike_flag)'s reason: the name is a
/// contract with scenario authors — an `on_flag_set("scan.depot_ladder_b.taken",
/// …)` trigger is written against this exact string — and a contract stated in
/// two places can be changed in one of them.
///
/// # Keyed on the world's own `id`, not on the minted UUID
///
/// A [`ScanReading`] joins on `subject_uuid`, which is the UUID
/// `spawn_world_entities` mints — a value no author has ever seen and none can
/// type. A flag is read from a scenario, so it is keyed on the handle a scenario
/// *wrote*: the `[[entity]] id`, carried at runtime as
/// [`EntityId`](crate::entities::spawner::EntityId), and already the way #1029
/// matches a promise's party onto its dossier. A structure that authors no `id`
/// is one no scenario can name at all, so nothing is mirrored for it.
///
/// # Why a flag exists at all, when the reading is already stored
///
/// [`ScanReading`] is on the scanning hull's own record, where a console reads
/// it. A **scenario** needs a different question answered: *did this crew ever
/// go and look at that structure?* Issue #1038's scan-versus-dossier beat turns
/// on it, and neither of the two things script could already reach answers it —
/// a timer says how long the mission has run, and the subject's own condition
/// flags say what is true whether anyone looked or not.
///
/// So this is a **mirror**, in the sense #1025's threshold flags and #1035's
/// workforce flags are: the reading stays the authority, the flag is a
/// derived-and-latched restatement of one bit of it, written at the same site
/// that takes the reading so the two cannot drift. Three things fall out of
/// spelling it as an ordinary world flag rather than as new machinery:
///
/// * a script reads it with the vocabulary it already has —
///   `ctx.flags["scan.depot_ladder_b.taken"]`;
/// * an `on_flag_set` trigger chains off the moment the reading lands, so a
///   scenario hangs a beat on the crew's own act without a new trigger kind;
/// * and it is in the save already, because the flag store is.
///
/// # It LATCHES, and that is the claim it is making
///
/// [`ShipScanRecord::last`](super::server::ShipScanRecord::last) holds one
/// reading and the next scan replaces it — a console shows what is on screen
/// now. This flag says something the console does not: that the crew *have
/// read* this structure. Scanning something else afterwards does not unlearn
/// it, exactly as [`EvidenceLog`](crate::dossier::EvidenceLog) never forgets a
/// finding, so the flag is raised once and never cleared. A refusal raises
/// nothing: being told there is nothing to read is not having read it.
pub fn scanned_flag(entity_id: &str) -> String {
    format!("scan.{entity_id}.taken")
}

/// Round a fraction to a band's reporting step.
///
/// `+ - * /` and `round` only — every one of them IEEE-754 exact on every
/// target, so two peers reading the same structure through the same band report
/// the same number (see `src/simmath.rs` for why that matters and which
/// functions are NOT safe here).
pub fn quantise(fraction: f32, step: f32) -> f32 {
    let clamped = fraction.clamp(0.0, 1.0);
    if !positive(step) {
        return clamped;
    }
    ((clamped / step).round() * step).clamp(0.0, 1.0)
}

// Fail-closed on an incomparable value: a NaN range or step refuses rather
// than passes, which is why this is spelled via partial_cmp instead of
// `x > 0.0` under a negation (the lint), matching operations::hold::within.
fn positive(x: f32) -> bool {
    matches!(x.partial_cmp(&0.0), Some(std::cmp::Ordering::Greater))
}

/// Take one reading, or say why there is none.
///
/// Deterministic and total: the same subject at the same distance with the same
/// power and the same interference always returns the same reading, and every
/// path that cannot return one names a reason.
///
/// The order the gates are tested in is the order the crew can do something
/// about them, hardest first: a subject with nothing readable on it is not a
/// problem helm can fly out of, but range, power and weather all are.
pub fn derive(
    config: &ScanConfig,
    subject: &ScanSubject,
    conditions: &ScanConditions,
    now_tick: u64,
) -> Result<ScanReading, ScanRefusal> {
    let Some(condition) = subject.condition.as_ref() else {
        return Err(ScanRefusal::NoReadableCondition);
    };
    if conditions.power_locked || conditions.power_level < config.min_power_level {
        return Err(ScanRefusal::Underpowered);
    }
    let Some(index) = config.band_for(conditions.distance) else {
        return Err(ScanRefusal::OutOfRange);
    };
    let index = if config.interfered_with_by(&conditions.region_effects) {
        index.saturating_add(usize::from(config.interference_bands))
    } else {
        index
    };
    let Some(band) = config.bands.get(index) else {
        return Err(ScanRefusal::Blinded);
    };

    Ok(ScanReading {
        subject_uuid: subject.uuid.clone(),
        subject_name: subject.name.clone(),
        band: band.id.clone(),
        band_label: band.label.clone(),
        taken_at_tick: now_tick,
        condition_fraction: quantise(condition.condition_fraction, band.condition_step),
        condition_step: band.condition_step,
        mass: subject.mass,
        flags: if band.report_thresholds {
            condition.flags.clone()
        } else {
            Vec::new()
        },
        capacities: if band.report_capacities {
            condition.capacities.clone()
        } else {
            Vec::new()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::InfrastructureSnapshot;

    /// The mirror flag's spelling is a **contract with scenario authors** —
    /// `falling_skyway.toml` and `probe_scandiff.toml` write
    /// `on_flag_set("scan.depot_ladder_b.taken", …)` against it — so it is
    /// pinned here rather than left to be re-read out of a `format!`.
    #[test]
    fn the_scanned_flag_is_named_after_the_subject_it_was_read_from() {
        assert_eq!(scanned_flag("depot_ladder_b"), "scan.depot_ladder_b.taken");
        assert_ne!(
            scanned_flag("depot_ladder_a"),
            scanned_flag("depot_ladder_b"),
            "two structures do not share one 'somebody scanned something' bit"
        );
    }

    fn band(id: &str, max_range: f32, step: f32) -> ScanBandConfig {
        ScanBandConfig {
            id: id.to_string(),
            label: format!("world.probe.band.{id}.label"),
            max_range,
            condition_step: step,
            report_thresholds: true,
            report_capacities: true,
        }
    }

    /// A two-rung ladder: whole percent inside 500 units, quarters out to 3000.
    fn suite() -> ScanConfig {
        ScanConfig {
            power_group: "shields".into(),
            min_power_level: 2,
            bands: vec![
                band("detailed", 500.0, 0.01),
                ScanBandConfig {
                    report_capacities: false,
                    ..band("coarse", 3000.0, 0.25)
                },
            ],
            degraded_by: vec![RegionEffectName::NebulaFog],
            interference_bands: 1,
        }
    }

    fn depot(fraction: f32) -> ScanSubject {
        ScanSubject {
            uuid: "depot-1".into(),
            name: "world.probe.entity.depot.name".into(),
            condition: Some(SubjectCondition {
                condition_fraction: fraction,
                flags: vec![("world.probe.threshold.transfer.label".into(), false)],
                capacities: vec![("world.probe.capacity.berths.label".into(), 4)],
            }),
            mass: 180_000.0,
        }
    }

    fn at(distance: f32) -> ScanConditions {
        ScanConditions {
            distance,
            power_level: 2,
            power_locked: false,
            region_effects: Vec::new(),
        }
    }

    /// **AC2, the derivation-purity claim, as a property of one function.**
    ///
    /// The same subject at the same range, scanned twice with a different
    /// condition in between, reads out differently — and nothing about the
    /// config, the labels or this test changed to make it happen.
    #[test]
    fn moving_the_subjects_condition_moves_the_reading_with_no_other_input_changed() {
        let config = suite();
        let before = derive(&config, &depot(0.82), &at(100.0), 10).expect("a reading");
        let after = derive(&config, &depot(0.31), &at(100.0), 20).expect("a reading");

        assert_eq!(before.condition_fraction, 0.82);
        assert_eq!(after.condition_fraction, 0.31);
        assert_ne!(
            before.condition_fraction, after.condition_fraction,
            "the readout is a derivation of the track, not a string chosen for it"
        );
        assert_eq!(
            (before.band.as_str(), after.band.as_str()),
            ("detailed", "detailed"),
            "…and everything else about the two scans is identical"
        );
    }

    /// **AC5.** The same structure, the same power, the same weather — twice as
    /// far out and the reading is measurably coarser: a rounder number, and the
    /// capacity list withdrawn because the authored band says so.
    #[test]
    fn a_distant_scan_reads_coarser_than_a_close_one_off_the_authored_bands() {
        let config = suite();
        let close = derive(&config, &depot(0.37), &at(400.0), 1).expect("a reading");
        let far = derive(&config, &depot(0.37), &at(2_400.0), 1).expect("a reading");

        assert_eq!(close.band, "detailed");
        assert_eq!(close.condition_fraction, 0.37);
        assert_eq!(close.capacities.len(), 1);

        assert_eq!(far.band, "coarse");
        assert_eq!(
            far.condition_fraction, 0.25,
            "0.37 rounded to the coarse band's authored quarter-steps"
        );
        assert!(
            far.capacities.is_empty(),
            "the coarse band authored report_capacities = false, so the berth count is not \
             something this reading claims to know"
        );
        assert_eq!(
            far.flags.len(),
            1,
            "…while the flags it DOES author still come through"
        );
        assert_eq!(far.condition_step, 0.25, "and it says how precise it is");
    }

    /// Issue #1154: mass is content identity, not a live measurement, so
    /// unlike `condition_fraction` the fidelity ladder never touches it — a
    /// coarse reading from 2,400 units out reports the exact same mass as a
    /// detailed one from 400.
    #[test]
    fn mass_rides_through_the_reading_unrounded_at_every_fidelity() {
        let config = suite();
        let close = derive(&config, &depot(0.37), &at(400.0), 1).expect("a reading");
        let far = derive(&config, &depot(0.37), &at(2_400.0), 1).expect("a reading");

        assert_eq!(close.mass, 180_000.0);
        assert_eq!(
            far.mass, 180_000.0,
            "mass does not get coarser with range the way condition_fraction does"
        );
    }

    /// Past the coarsest band there is no reading at all — the ladder has an
    /// end, and it is the authored one.
    #[test]
    fn past_the_last_bands_reach_the_scan_is_refused_as_out_of_range() {
        assert_eq!(
            derive(&suite(), &depot(0.5), &at(3_000.1), 1),
            Err(ScanRefusal::OutOfRange)
        );
        assert!(
            derive(&suite(), &depot(0.5), &at(3_000.0), 1).is_ok(),
            "and the boundary itself still answers — max_range is inclusive"
        );
    }

    /// **AC1's refusal half.** A target with no condition track is refused with
    /// a reason rather than answered with an empty readout.
    #[test]
    fn a_target_with_no_condition_track_is_refused_with_a_reason() {
        let rock = ScanSubject {
            uuid: "rock-7".into(),
            name: String::new(),
            condition: None,
            mass: 500.0,
        };
        assert_eq!(
            derive(&suite(), &rock, &at(100.0), 1),
            Err(ScanRefusal::NoReadableCondition)
        );
    }

    /// **The leak rule, restated for this module.** A structure the scenario
    /// keeps off the wire cannot be built into a subject that has a condition,
    /// because the only constructor for one is #1025's publish gate — so the
    /// scan refuses it, and refuses it with the SAME reason a bare rock gets.
    ///
    /// The identical reason is the load-bearing half: a refusal that said
    /// "withheld" would leak the existence of the secret to anyone who scanned.
    #[test]
    fn a_withheld_condition_track_cannot_be_scanned_and_does_not_announce_itself() {
        use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

        let hidden = InfrastructureState::from_config(&InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(31.0),
            publish: false,
            ..InfrastructureConfig::default()
        });
        assert!(
            InfrastructureSnapshot::from_state(&hidden).is_none(),
            "the publish gate is #1025's, and this derivation is downstream of it"
        );

        let sealed = ScanSubject {
            uuid: "sealed-1".into(),
            name: "world.probe.entity.sealed.name".into(),
            condition: InfrastructureSnapshot::from_state(&hidden)
                .as_ref()
                .map(|published| SubjectCondition::from_published(published, |_| None, |_| None)),
            mass: 90_000.0,
        };
        let refusal = derive(&suite(), &sealed, &at(100.0), 1).expect_err("no reading");
        assert_eq!(
            refusal,
            ScanRefusal::NoReadableCondition,
            "the same answer an unreadable rock gets — a scan that distinguished the two \
             would betray the secret by the shape of its error"
        );
    }

    /// Power is a live gate, and it refuses rather than degrading: a suite
    /// under its authored minimum is not returning a worse answer, it is
    /// returning none.
    #[test]
    fn a_suite_below_its_authored_power_level_or_behind_a_locked_grid_returns_nothing() {
        let config = suite();
        let mut brownout = at(100.0);
        brownout.power_level = 1;
        assert_eq!(
            derive(&config, &depot(0.5), &brownout, 1),
            Err(ScanRefusal::Underpowered)
        );

        let mut locked = at(100.0);
        locked.power_locked = true;
        assert_eq!(
            derive(&config, &depot(0.5), &locked, 1),
            Err(ScanRefusal::Underpowered),
            "the exhaustion lock is the same answer as being under the floor"
        );
    }

    /// Interference drops the reading a band, out of authored data on both
    /// ends: the effect names the hull declared, and the number of steps it
    /// declared. An effect the hull did NOT declare changes nothing.
    #[test]
    fn standing_in_an_authored_interference_effect_drops_the_reading_a_band() {
        let config = suite();
        let mut fogged = at(100.0);
        fogged.region_effects = vec![RegionEffectName::NebulaFog];
        let reading = derive(&config, &depot(0.37), &fogged, 1).expect("still a reading");
        assert_eq!(
            reading.band, "coarse",
            "inside the fog the close-range scan reads out at the next band down"
        );
        assert_eq!(reading.condition_fraction, 0.25);

        let mut irrelevant = at(100.0);
        irrelevant.region_effects = vec![RegionEffectName::SlowZone];
        assert_eq!(
            derive(&config, &depot(0.37), &irrelevant, 1)
                .expect("a reading")
                .band,
            "detailed",
            "a hazard this hull never declared as interference is not interference"
        );
    }

    /// Interference that pushes past the coarsest band blinds the suite — a
    /// refusal, not a silently empty reading.
    #[test]
    fn interference_past_the_last_band_blinds_the_suite() {
        let config = suite();
        let mut fogged = at(2_400.0);
        fogged.region_effects = vec![RegionEffectName::NebulaFog];
        assert_eq!(
            derive(&config, &depot(0.5), &fogged, 1),
            Err(ScanRefusal::Blinded)
        );
    }

    /// The reading is stamped with the tick it was taken on, which is what
    /// makes it a reading rather than a live gauge.
    #[test]
    fn a_reading_carries_the_tick_it_was_taken_on() {
        let reading = derive(&suite(), &depot(0.5), &at(100.0), 4_242).expect("a reading");
        assert_eq!(reading.taken_at_tick, 4_242);
        assert_eq!(reading.subject_uuid, "depot-1");
        assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
        assert_eq!(reading.band_label, "world.probe.band.detailed.label");
    }

    /// Quantisation is arithmetic, and it is stable at both ends of the range.
    #[test]
    fn quantisation_rounds_to_the_step_and_clamps_to_the_track() {
        assert_eq!(quantise(0.37, 0.25), 0.25);
        assert_eq!(quantise(0.38, 0.25), 0.5);
        assert_eq!(quantise(1.0, 0.25), 1.0);
        assert_eq!(quantise(0.0, 0.25), 0.0);
        assert_eq!(quantise(-0.4, 0.25), 0.0, "a track cannot read below empty");
        assert_eq!(quantise(1.4, 0.25), 1.0, "…or above full");
        assert_eq!(
            quantise(0.37, 0.0),
            0.37,
            "a step of zero reports the value unchanged rather than dividing by it"
        );
    }

    /// The authored vocabulary round-trips through TOML, and an unknown key is
    /// a parse error rather than a silently ignored typo.
    #[test]
    fn the_authored_table_round_trips_and_refuses_unknown_keys() {
        let toml = r#"
power_group = "shields"
min_power_level = 2
degraded_by = ["nebula_fog"]
interference_bands = 1

[[band]]
id = "detailed"
label = "world.probe.band.detailed.label"
max_range = 500.0
condition_step = 0.01

[[band]]
id = "coarse"
label = "world.probe.band.coarse.label"
max_range = 3000.0
condition_step = 0.25
report_capacities = false
"#;
        let parsed: ScanConfig = toml::from_str(toml).expect("the authored shape parses");
        parsed.validate().expect("and validates");
        assert_eq!(parsed, suite());

        let typo: Result<ScanConfig, _> = toml::from_str("min_power_lvl = 2\n");
        assert!(typo.is_err(), "a mistyped key is refused, not ignored");
    }

    /// A table that omits everything optional takes the documented defaults.
    #[test]
    fn an_authored_table_takes_the_documented_defaults_for_everything_it_omits() {
        let parsed: ScanConfig = toml::from_str(
            r#"
[[band]]
id = "only"
label = "world.probe.band.only.label"
max_range = 900.0
condition_step = 0.05
"#,
        )
        .expect("parses");
        assert_eq!(parsed.power_group, "shields");
        assert_eq!(parsed.min_power_level, 1);
        assert_eq!(parsed.interference_bands, 1);
        assert!(parsed.degraded_by.is_empty());
        assert!(parsed.bands[0].report_thresholds);
        assert!(parsed.bands[0].report_capacities);
        parsed.validate().expect("and validates");
    }

    /// Every author mistake that would otherwise show up as a console which
    /// quietly never answers is a load failure naming the band.
    #[test]
    fn validation_refuses_a_ladder_that_could_never_answer() {
        let empty = ScanConfig::default();
        assert!(empty
            .validate()
            .expect_err("no bands")
            .contains("no [[scan.band]]"));

        let mut duplicate = suite();
        duplicate.bands[1].id = "detailed".into();
        assert!(duplicate
            .validate()
            .expect_err("duplicate id")
            .contains("declared twice"));

        let mut unreachable = suite();
        unreachable.bands[1].max_range = 200.0;
        assert!(unreachable
            .validate()
            .expect_err("descending ranges")
            .contains("FINEST FIRST"));

        let mut nameless = suite();
        nameless.bands[0].label = "  ".into();
        assert!(nameless
            .validate()
            .expect_err("no label")
            .contains("empty label"));

        let mut silly_step = suite();
        silly_step.bands[0].condition_step = 0.0;
        assert!(silly_step
            .validate()
            .expect_err("zero step")
            .contains("condition_step"));

        let mut no_reach = suite();
        no_reach.bands[0].max_range = 0.0;
        assert!(no_reach
            .validate()
            .expect_err("zero range")
            .contains("no reach"));
    }

    /// The refusal vocabulary is closed, distinct, and every member has a
    /// literal `strings.csv` id the checker can find.
    #[test]
    fn every_refusal_has_its_own_string_id() {
        let mut ids: Vec<&str> = ScanRefusal::ALL.iter().map(|r| r.string_id()).collect();
        assert_eq!(ids.len(), 6);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 6, "the six ids are distinct");
        for refusal in ScanRefusal::ALL {
            assert!(refusal.string_id().starts_with("scan.refusal."));
        }
    }

    /// A refusal serialises under its script name, so a save and a payload
    /// spell it identically.
    #[test]
    fn a_refusal_serialises_under_its_snake_case_name() {
        assert_eq!(
            serde_json::to_string(&ScanRefusal::NoReadableCondition).unwrap(),
            "\"no_readable_condition\""
        );
    }
}
