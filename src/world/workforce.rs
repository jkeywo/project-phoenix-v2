//! The workforce register — who staffs a structure, and whether they are
//! working (issue #1035, parent #852 "Falling Skyway").
//!
//! A skyhook is not a machine that runs itself. Somebody authorises the pump,
//! somebody rides the climber down with a torch, somebody signs the transfer
//! off. This module is where a scenario says who those people are, which side
//! of a dispute they are on, and what they think of the crew asking them for
//! help.
//!
//! A world declares its sides in `[[workforce]]` blocks; a structure names the
//! side that staffs it on its own `[infrastructure] workforce = "…"`; and the
//! external-operations module asks one question of the pair — *are the people
//! who work this place out?* — through
//! [`OperationConditions::work_stopped`](crate::operations::OperationConditions::work_stopped).
//!
//! # THIS IS NOT A FACTION, AND IT IS NOT A SECOND ONE
//!
//! [`FactionConfig`](crate::ai::faction::FactionConfig) already answers *who
//! shoots whom*. A workforce answers *who turns up for work*, which is a
//! different question with a different lifetime: the Skyway workers and the
//! operator that employs them are both Federation, neither will ever fire on
//! the other, and the whole crisis is between them. Folding a strike into the
//! enemies list would have made "the line is down" mean "the depot is hostile",
//! which is the wrong thing on every console that reads a faction.
//!
//! So the two vocabularies stay apart, and a party that is both — the strike
//! committee is a hull with a faction *and* a side in the dispute — carries one
//! of each rather than one that does two jobs.
//!
//! # THE REGISTER IS A RECORD. IT DECIDES NOTHING
//!
//! Nothing here knows what a transfer is, what a repair rate is, or that
//! operations exist. It holds two facts per side — **are they out**, and **what
//! do they make of the crew** — and hands them to whoever asks. What a stoppage
//! *does* to a piece of work is authored on the hull's own
//! `[[operations.capability.interrupt]]` block, one rule per verb, because
//! "transfers are refused but repairs merely go slowly" is a designer's
//! judgement about the fiction and not a property of the word *strike*.
//!
//! That split is what makes the effects reversible for free: settle the strike
//! and the rule stops firing. There is no undo path to write, because nothing
//! was ever latched.
//!
//! # The mirror flags, and why they exist
//!
//! Every mutation also writes two ordinary world flags —
//! [`strike_flag`] and [`disposition_flag`], `workforce.<id>.on_strike` and
//! `workforce.<id>.disposition` — through the **existing**
//! [`ActionCmd::MutateFlag`](crate::world::dispatch::ActionCmd::MutateFlag)
//! path, exactly as a resolved commitment writes its campaign flag. Three
//! things fall out of that and none of them needed new machinery:
//!
//! * a script reads the state with the vocabulary it already has —
//!   `ctx.flags["workforce.skyway_workers.on_strike"]`;
//! * an `on_flag_cleared("workforce.skyway_workers.on_strike", …)` trigger
//!   chains off the settlement, so the negotiation slice (#1036) hangs its
//!   consequences off the strike ending without knowing this module exists;
//! * and the flags are in the save already, because the flag store is.
//!
//! The register itself is still the authority — the flags are a *mirror*, and
//! [`WorkforceRegister::apply`] returns the writes its caller must make so the
//! two cannot be updated in one place and forgotten in the other.

use serde::{Deserialize, Serialize};

/// The world flag mirroring `id`'s strike status: `workforce.<id>.on_strike`.
///
/// A free function rather than an inlined `format!` at each site, for
/// [`kept_flag`](crate::world::commitments::kept_flag)'s reason: the name is a
/// contract with scenario authors — an `on_flag_cleared` trigger is written
/// against this exact string — and a contract stated in two places can be
/// changed in one of them.
pub fn strike_flag(id: &str) -> String {
    format!("workforce.{id}.on_strike")
}

/// The world flag mirroring `id`'s disposition toward the crew:
/// `workforce.<id>.disposition`.
///
/// A counter rather than a boolean, because the flag store's counters are
/// `i64` and a disposition is a number a later beat compares against a
/// threshold. See [`Workforce::disposition`] for the scale.
pub fn disposition_flag(id: &str) -> String {
    format!("workforce.{id}.disposition")
}

/// Default disposition for a side that authors none.
///
/// `50` — the midpoint of the authored 0–100 scale. A side nobody has written
/// an opinion for regards the crew as neither friend nor obstacle, which is the
/// only honest reading of silence. A TOML-parse fallback, the one kind of
/// hardcoded gameplay value AGENTS.md #11 sanctions.
fn default_disposition() -> i64 {
    50
}

/// One `[[workforce]]` block: a body of people a scenario can put out on
/// strike.
///
/// Authored data only. The *live* status is
/// [`WorkforceRegister`](crate::world::workforce::WorkforceRegister), armed
/// from this list on the first simulation tick of the mission, the same way a
/// `[[deadline]]` is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workforce {
    /// Stable id, unique within a world. A structure names this on its
    /// `[infrastructure] workforce`, a script names it in
    /// `ctx.effects.settle_strike("…")`, and it is the `<id>` in
    /// [`strike_flag`]/[`disposition_flag`].
    pub id: String,
    /// `strings.csv` id for the crew-facing name of this side. Never English
    /// (AGENTS.md rule 11).
    ///
    /// Optional in the same sense a deadline's `label` is: a side the mission
    /// never names on a console owes the crew no name. A dispute the crew are
    /// negotiating has one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Whether this side is out **when the mission opens**.
    ///
    /// Authored rather than scripted on tick one for the reason a skyhook's
    /// starting condition is: the crew arrive into a situation that was already
    /// happening, and a world that had to script its own opening state would be
    /// describing a change nobody was there for.
    #[serde(default)]
    pub on_strike: bool,
    /// What this side makes of the crew, `0..=100`: `0` is *you are the enemy*,
    /// `100` is *whatever you need*. The midpoint is
    /// [`default_disposition`].
    ///
    /// A number rather than a named ladder because it is a quantity later beats
    /// compare against their own authored thresholds — and because the
    /// negotiation (#1036) moves it by an authored amount rather than by
    /// stepping a rung. Nothing in *this* slice branches on it; it is the state
    /// the acts after it read.
    #[serde(default = "default_disposition")]
    pub disposition: i64,
}

/// The lowest and highest a disposition may be authored or moved to.
///
/// Public because the negotiation slice sets dispositions and should clamp to
/// the same scale rather than restate it.
pub const DISPOSITION_MIN: i64 = 0;
/// See [`DISPOSITION_MIN`].
pub const DISPOSITION_MAX: i64 = 100;

impl Workforce {
    /// Reject a `[[workforce]]` block that cannot mean anything.
    ///
    /// Called at world-parse time so a typo is a load error naming the field,
    /// not a side whose strike silently never binds to anything.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("[[workforce]] needs a non-empty id — a structure names \
                        its workforce by it, and an empty one can never be named"
                .to_string());
        }
        if !(DISPOSITION_MIN..=DISPOSITION_MAX).contains(&self.disposition) {
            return Err(format!(
                "[[workforce]] {} disposition must be between {DISPOSITION_MIN} and \
                 {DISPOSITION_MAX}, got {}",
                self.id, self.disposition
            ));
        }
        Ok(())
    }
}

/// What a script asked of one side.
///
/// Three verbs rather than one setter taking a struct, for the reason the
/// infrastructure vocabulary has `repair_` and `damage_` rather than a signed
/// `adjust_`: the direction lives in the name, so a scenario cannot end a
/// strike by getting a boolean the wrong way round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkforceMutation {
    /// They walk out. Work at every structure they staff stops.
    CallStrike,
    /// They go back. Everything the stoppage was gating resumes, because
    /// nothing about it was latched.
    Settle,
    /// Move what they make of the crew to an absolute value, clamped to
    /// [`DISPOSITION_MIN`]`..=`[`DISPOSITION_MAX`].
    ///
    /// Absolute rather than a delta because a negotiation's outcome is a
    /// *position* — "they will work with you now" — and two beats that each
    /// nudged by five would compose into a number no author chose.
    SetDisposition(i64),
}

/// One flag write a mutation asks its caller to make, so the mirror cannot
/// drift from the register.
///
/// Returned rather than performed because this module holds no flag store and
/// must not: a pure record that reached for the world's flags would be the
/// second thing that owns them, and flag transitions are the trigger
/// pipeline's to emit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagMirror {
    /// The flag name — [`strike_flag`] or [`disposition_flag`].
    pub name: String,
    /// The value to write, absolutely.
    pub value: i64,
}

/// One side's live state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkforceRecord {
    /// The authored id.
    pub id: String,
    /// The authored `strings.csv` label, carried through unchanged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Whether they are out **right now**.
    #[serde(default)]
    pub on_strike: bool,
    /// What they make of the crew right now.
    #[serde(default)]
    pub disposition: i64,
}

/// Every side of every dispute this world declared, in authored order.
///
/// A `Vec` rather than a map, for
/// [`DeadlineTable`](crate::world::deadlines::DeadlineTable)'s reason: the
/// order is the world file's, it is the order a debrief reads them in, and a
/// `HashMap`'s iteration order must never reach a payload or a fold.
///
/// Lives as a **field** on
/// [`WorldContentRuntime`](crate::world::server::WorldContentRuntime), beside
/// the deadline table and the commitments ledger, so this slice registers no
/// new component and no new resource — see the baseline note in
/// `tests/authoritative_state_enumeration.rs`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkforceRegister {
    /// The sides, in authored order.
    #[serde(default)]
    pub records: Vec<WorkforceRecord>,
    /// Whether [`arm`](Self::arm) has run for this mission.
    ///
    /// In the payload for [`DeadlineTable`]'s reason: a resumed mission that
    /// forgot it had armed would re-arm on its first tick and put a settled
    /// strike straight back on, which is the loudest possible way to lose a
    /// negotiation the crew already won.
    ///
    /// [`DeadlineTable`]: crate::world::deadlines::DeadlineTable
    #[serde(default)]
    pub armed: bool,
}

impl WorkforceRegister {
    /// Whether this world declared any side at all — the payload's skip
    /// predicate, and the early-out every caller takes.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The record for `id`, or `None`.
    pub fn get(&self, id: &str) -> Option<&WorkforceRecord> {
        self.records.iter().find(|r| r.id == id)
    }

    /// Whether `id` is out right now.
    ///
    /// `false` for a side this world never declared, which is the load-bearing
    /// default: an entity template may name a workforce that a given world has
    /// no dispute about (the same depot template ships in five scenarios), and
    /// the honest reading of "this world has never heard of those people" is
    /// that work there carries on.
    pub fn on_strike(&self, id: &str) -> bool {
        self.get(id).is_some_and(|r| r.on_strike)
    }

    /// What `id` makes of the crew, or `None` for a side this world never
    /// declared.
    pub fn disposition(&self, id: &str) -> Option<i64> {
        self.get(id).map(|r| r.disposition)
    }

    /// Build the live register from the authored table, returning the mirror
    /// flags the caller must write.
    ///
    /// Idempotent through [`Self::armed`]: a second call does nothing and
    /// returns no writes, so a resumed mission cannot be put back on strike by
    /// the system that armed it in the first place.
    pub fn arm(&mut self, authored: &[Workforce]) -> Vec<FlagMirror> {
        if self.armed || authored.is_empty() {
            self.armed = true;
            return Vec::new();
        }
        self.records = authored
            .iter()
            .map(|w| WorkforceRecord {
                id: w.id.clone(),
                label: w.label.clone(),
                on_strike: w.on_strike,
                disposition: w.disposition.clamp(DISPOSITION_MIN, DISPOSITION_MAX),
            })
            .collect();
        self.armed = true;
        self.records
            .iter()
            .flat_map(|r| {
                [
                    FlagMirror {
                        name: strike_flag(&r.id),
                        value: i64::from(r.on_strike),
                    },
                    FlagMirror {
                        name: disposition_flag(&r.id),
                        value: r.disposition,
                    },
                ]
            })
            .collect()
    }

    /// Apply one mutation, returning the mirror flag write it asks for.
    ///
    /// `None` — nothing happened, nothing to write — for a side this world
    /// never declared, and for a mutation that changed nothing (settling a
    /// side that is already working). A no-op rather than an error, the way a
    /// deadline's second cancel is: the state a caller reads back says which it
    /// is, and a scenario that can reach the same beat twice should not have to
    /// guard it.
    pub fn apply(&mut self, id: &str, mutation: WorkforceMutation) -> Option<FlagMirror> {
        let record = self.records.iter_mut().find(|r| r.id == id)?;
        match mutation {
            WorkforceMutation::CallStrike | WorkforceMutation::Settle => {
                let out = mutation == WorkforceMutation::CallStrike;
                if record.on_strike == out {
                    return None;
                }
                record.on_strike = out;
                Some(FlagMirror {
                    name: strike_flag(id),
                    value: i64::from(out),
                })
            }
            WorkforceMutation::SetDisposition(value) => {
                let value = value.clamp(DISPOSITION_MIN, DISPOSITION_MAX);
                if record.disposition == value {
                    return None;
                }
                record.disposition = value;
                Some(FlagMirror {
                    name: disposition_flag(id),
                    value,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored() -> Vec<Workforce> {
        vec![
            Workforce {
                id: "skyway_workers".into(),
                label: "world.probe.workforce.workers.label".into(),
                on_strike: true,
                disposition: 30,
            },
            Workforce {
                id: "havelock_operations".into(),
                label: "world.probe.workforce.operator.label".into(),
                on_strike: false,
                disposition: 45,
            },
        ]
    }

    fn armed() -> WorkforceRegister {
        let mut register = WorkforceRegister::default();
        register.arm(&authored());
        register
    }

    // ── AC1: two sides, an explicit status and a per-side disposition ────────

    #[test]
    fn arming_takes_both_sides_status_and_disposition_from_the_world() {
        let register = armed();
        assert_eq!(
            register
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["skyway_workers", "havelock_operations"],
            "in authored order — the world file's order is the one a debrief reads"
        );
        assert!(register.on_strike("skyway_workers"));
        assert!(
            !register.on_strike("havelock_operations"),
            "the operator is not the one who walked out; a slice that could not tell \
             the two sides apart could not have a dispute"
        );
        assert_eq!(register.disposition("skyway_workers"), Some(30));
        assert_eq!(register.disposition("havelock_operations"), Some(45));
    }

    #[test]
    fn a_side_this_world_never_declared_is_working_and_has_no_opinion() {
        let register = armed();
        assert!(
            !register.on_strike("dockers_of_some_other_mission"),
            "a depot template naming a workforce the world has no dispute about must \
             keep working — the template ships in every scenario, the dispute does not"
        );
        assert_eq!(register.disposition("dockers_of_some_other_mission"), None);
    }

    #[test]
    fn arming_writes_both_mirror_flags_for_every_side() {
        let mut register = WorkforceRegister::default();
        let writes = register.arm(&authored());
        assert_eq!(
            writes,
            vec![
                FlagMirror {
                    name: "workforce.skyway_workers.on_strike".into(),
                    value: 1
                },
                FlagMirror {
                    name: "workforce.skyway_workers.disposition".into(),
                    value: 30
                },
                FlagMirror {
                    name: "workforce.havelock_operations.on_strike".into(),
                    value: 0
                },
                FlagMirror {
                    name: "workforce.havelock_operations.disposition".into(),
                    value: 45
                },
            ],
            "the mirror is written by the caller, from writes this module names, so \
             the flag a script reads cannot drift from the record"
        );
    }

    #[test]
    fn arming_twice_changes_nothing_and_asks_for_no_writes() {
        let mut register = armed();
        register.apply("skyway_workers", WorkforceMutation::Settle);
        assert!(!register.on_strike("skyway_workers"));

        assert!(
            register.arm(&authored()).is_empty(),
            "a resumed mission's first tick must not put a settled strike back on"
        );
        assert!(!register.on_strike("skyway_workers"));
    }

    #[test]
    fn a_world_that_declares_no_side_arms_to_nothing() {
        let mut register = WorkforceRegister::default();
        assert!(register.arm(&[]).is_empty());
        assert!(register.is_empty());
        assert!(
            register.armed,
            "and it counts as armed, so the system that arms it stops looking"
        );
    }

    // ── AC4: reversible, both ways, with the mirror following ────────────────

    #[test]
    fn settling_a_strike_clears_the_status_and_its_flag() {
        let mut register = armed();
        assert_eq!(
            register.apply("skyway_workers", WorkforceMutation::Settle),
            Some(FlagMirror {
                name: "workforce.skyway_workers.on_strike".into(),
                value: 0
            })
        );
        assert!(!register.on_strike("skyway_workers"));
    }

    #[test]
    fn a_settled_side_can_walk_out_again() {
        let mut register = armed();
        register.apply("skyway_workers", WorkforceMutation::Settle);
        assert_eq!(
            register.apply("skyway_workers", WorkforceMutation::CallStrike),
            Some(FlagMirror {
                name: "workforce.skyway_workers.on_strike".into(),
                value: 1
            }),
            "nothing about a stoppage is latched, in either direction"
        );
        assert!(register.on_strike("skyway_workers"));
    }

    #[test]
    fn a_mutation_that_changes_nothing_writes_nothing() {
        let mut register = armed();
        assert_eq!(
            register.apply("skyway_workers", WorkforceMutation::CallStrike),
            None,
            "they are already out"
        );
        assert_eq!(
            register.apply("havelock_operations", WorkforceMutation::Settle),
            None,
            "and they never left"
        );
        assert_eq!(
            register.apply("nobody", WorkforceMutation::Settle),
            None,
            "an unknown side settles nothing and writes no flag"
        );
    }

    #[test]
    fn disposition_moves_absolutely_and_clamps_to_the_authored_scale() {
        let mut register = armed();
        assert_eq!(
            register.apply("skyway_workers", WorkforceMutation::SetDisposition(80)),
            Some(FlagMirror {
                name: "workforce.skyway_workers.disposition".into(),
                value: 80
            })
        );
        assert_eq!(register.disposition("skyway_workers"), Some(80));

        register.apply("skyway_workers", WorkforceMutation::SetDisposition(9_000));
        assert_eq!(
            register.disposition("skyway_workers"),
            Some(DISPOSITION_MAX),
            "a runaway negotiation cannot push a side off its own scale"
        );
        register.apply("skyway_workers", WorkforceMutation::SetDisposition(-40));
        assert_eq!(
            register.disposition("skyway_workers"),
            Some(DISPOSITION_MIN)
        );
    }

    #[test]
    fn the_two_facts_move_independently() {
        // A side can go back to work still resenting the crew, and can walk out
        // while thinking well of them. Folding status and disposition into one
        // number would make both of those unsayable.
        let mut register = armed();
        register.apply("skyway_workers", WorkforceMutation::Settle);
        assert!(!register.on_strike("skyway_workers"));
        assert_eq!(register.disposition("skyway_workers"), Some(30));

        register.apply("havelock_operations", WorkforceMutation::SetDisposition(10));
        assert!(!register.on_strike("havelock_operations"));
        assert_eq!(register.disposition("havelock_operations"), Some(10));
    }

    // ── Authoring guards ─────────────────────────────────────────────────────

    #[test]
    fn an_empty_id_is_refused_at_load() {
        let workforce = Workforce {
            id: "  ".into(),
            ..Default::default()
        };
        assert!(workforce.validate().unwrap_err().contains("non-empty id"));
    }

    #[test]
    fn a_disposition_off_the_scale_is_refused_at_load() {
        for value in [-1, 101] {
            let workforce = Workforce {
                id: "skyway_workers".into(),
                disposition: value,
                ..Default::default()
            };
            let err = workforce.validate().expect_err("off the 0..=100 scale");
            assert!(err.contains("disposition"), "{err}");
            assert!(err.contains("skyway_workers"), "the error names it: {err}");
        }
        assert!(Workforce {
            id: "skyway_workers".into(),
            disposition: DISPOSITION_MAX,
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn an_unauthored_disposition_defaults_to_the_midpoint() {
        let parsed: Workforce = toml::from_str(r#"id = "skyway_workers""#).expect("parses");
        assert_eq!(parsed.disposition, default_disposition());
        assert!(
            !parsed.on_strike,
            "a side nobody said was out is at work — the block exists to declare a \
             dispute, not to imply one"
        );
        assert!(parsed.label.is_empty());
    }

    #[test]
    fn the_register_round_trips_through_serialization() {
        let mut register = armed();
        register.apply("skyway_workers", WorkforceMutation::Settle);
        register.apply("havelock_operations", WorkforceMutation::SetDisposition(12));

        let json = serde_json::to_string(&register).expect("serialises");
        let restored: WorkforceRegister = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(
            restored, register,
            "every field a run moves — both statuses, both dispositions and the armed \
             latch — comes back"
        );
    }
}
