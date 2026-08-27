//! The campaign projection (issue #867, parent #848).
//!
//! What one finished mission hands to the next, folded out of the save the
//! mission left behind. Pure, Bevy-free, and — the property the whole issue
//! turns on — **narrow**: it takes a whole authoritative world snapshot and
//! returns only facts a campaign is allowed to remember.
//!
//! # The rule: the OUTPUT is the declaration
//!
//! [`crate::dossier::projection`] keeps hidden truth out by narrowing its
//! *input port* — a fact with no field to arrive through cannot leak. This
//! projection cannot do that, and the reason is in its acceptance criteria: it
//! is handed a `vellum-save` snapshot, whole, because that is the artifact a
//! finished mission produces. So the gate moves to the other end.
//!
//! [`CampaignFacts`] has a field for each declared family of cross-mission fact
//! — the mission and how it ended, its handoff tallies, the promises it settled,
//! what its crew found out, where it stands with the parties it dealt with, the
//! named things still standing, and what happened to the structures — **and no
//! field for anything else**. There is nowhere for a hull fraction, a beam's
//! remaining seconds, a tube's load timer, an asteroid, an RNG position or a
//! mid-flight torpedo to go. Carrying one into the next mission would take a new
//! field on that struct, in a diff, next to this paragraph.
//!
//! That is why the exclusion test does not enumerate what is left out. It varies
//! the transient state — mauls the ships, arms their weapons, moves them, fills
//! the belt with rocks — and asserts the facts are **unchanged**. A list of
//! excluded fields would go stale the first time a new component was added; a
//! claim that nothing but the declared families can move the output does not.
//!
//! # The vocabulary is not this module's to invent
//!
//! The named cross-mission facts a mission writes are
//! `campaign-flag-handoff-state` in
//! `pasm/spec/architecture/world-files.yaml` (issue #1043): ordinary counters in
//! the world `FlagStore`, under a `campaign.<mission>.<family>.<fact>` prefix,
//! written once at a mission's close. This module is that record's **consumer**
//! and adds no second declaration: [`CampaignFacts::tallies`] carries those
//! counters through verbatim, under their authored names, in sorted order.
//!
//! The prefix is the contract there, so the prefix is the filter here. What this
//! module contributes is the *rest* of the handoff — the promises, findings,
//! standing, assets and published structures that are already authoritative records rather
//! than counters, and which a later mission would otherwise have to re-derive
//! from a flag someone remembered to write.
//!
//! # Identity is the authored NAME, never the uuid
//!
//! A uuid is minted per run (`crate::world_id`), so the skyhook in the mission
//! that ends is not the skyhook in the mission that follows even when the
//! fiction says it is. Every identity that leaves here is therefore the name a
//! scenario author wrote — `name_to_uuid`'s key, resolved backwards — and a fact
//! about an entity with no authored name is dropped rather than carried under a
//! number the next mission cannot match. That is the same call
//! [`crate::world::commitments::Commitment::made_to`] already makes for a
//! promise, made for the same reason and one layer out.
//!
//! # No I/O, and no opinion about where the save came from
//!
//! [`project`] takes `&StoredRun` and returns a value. It opens nothing, reads
//! no clock, and cannot tell whether the snapshot arrived from `LocalStorage`,
//! a file, or the `TransferStore` an import travelled through (issue #866) —
//! which is the point: a campaign is continuity between missions, not between
//! storage backends.

use serde::{Deserialize, Serialize};

use crate::snapshot::{PhoenixSnapshot, ScenarioState, StoredRun};
use crate::world::commitments::CommitmentState;
use crate::world::flags::FlagStore;

/// The prefix a cross-mission counter is written under (issue #1043).
///
/// `campaign.` and nothing narrower: the family after it is
/// `<mission>.<family>.<fact>`, and this module deliberately does not know which
/// missions exist. A campaign that adds a mission adds names, not code here.
pub const CAMPAIGN_FLAG_PREFIX: &str = "campaign.";

/// The shape of [`CampaignFacts`] this build produces.
///
/// Carried in the value rather than kept as a private constant because these
/// facts are meant to *outlive* the mission that made them: a campaign runner
/// holding a projection from an older build needs to know which vocabulary it
/// is holding, and the answer must survive being written down.
///
/// `1` — issue #867's original set: mission, outcome, tallies, commitments,
/// evidence, standing, assets, structures.
pub const CAMPAIGN_FACTS_VERSION: u32 = 1;

/// One promise a mission settled, or left open.
///
/// Keyed by the authored id and the party it was made to — both strings a script
/// wrote, neither a handle. `terms` is a `strings.csv` id, as it is in the
/// ledger: a campaign carries the promise, not a translation of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignPromise {
    pub id: String,
    pub made_to: String,
    pub terms: String,
    /// `kept` / `broken` / `open`, by the ledger's own name for it.
    pub state: String,
}

/// One thing the crew found out, carried forward by the authored name of its
/// subject.
///
/// The log stores a subject **uuid** — deliberately, because a finding is about
/// the specific thing that was examined. That is the right key inside a mission
/// and the wrong one between missions, so it is resolved back through
/// `name_to_uuid` here; a finding whose subject has no authored name does not
/// travel, because the next mission has no way to say what it was about.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignFinding {
    /// The authored entity name the finding is about.
    pub subject: String,
    /// `strings.csv` id for what was learned.
    pub text: String,
    /// How the crew learned it, by the provenance's own name.
    pub provenance: String,
}

/// Where the mission left the crew with one party it dealt with.
///
/// Reputation, in the only form this game actually holds one: a
/// `[[workforce]]` side's disposition toward the crew, plus whether the dispute
/// was still on when the mission ended. Derived from nothing — the register is
/// authoritative state a mission moves by settling or failing to settle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignStanding {
    /// The authored side id.
    pub party: String,
    /// What they make of the crew, on the register's own scale.
    pub disposition: i64,
    /// Whether they were still out when the mission closed.
    pub on_strike: bool,
}

/// A named thing that was still on the board when the mission ended.
///
/// "Reusable asset" in the campaign sense: a hull or a structure the next
/// mission can be authored to expect, referred to by the name the last one used.
/// `template` is the entity template a *runtime* spawn was made from (issue
/// #863's [`crate::world::spawn_origin::SpawnOrigin`]) and `None` for an
/// authored `[[entity]]`, whose template the next world file names for itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignAsset {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// What a mission did to a structure.
///
/// The condition track's own reading, plus the operational flags it was holding
/// when the lights went out — which is what a later mission needs to open on a
/// depot that is still limping rather than on the one the world file authors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CampaignStructure {
    pub name: String,
    /// Condition as a fraction of its maximum, `0.0..=1.0`.
    pub condition: f32,
    /// `(operational flag, whether it was holding)`, sorted by flag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<(String, bool)>,
}

/// Everything one mission hands to the next, and nothing else.
///
/// See the module docs: the field list **is** the declaration of what a campaign
/// may remember, and the exclusion of transient combat state is the absence of
/// anywhere to put it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CampaignFacts {
    /// [`CAMPAIGN_FACTS_VERSION`] as of the build that produced this value.
    ///
    /// `#[serde(default)]` like every field here, so a projection written down
    /// by an older build still reads back — as version `0`, which is the honest
    /// answer for a value that predates the field rather than a guess at which
    /// vocabulary it used.
    #[serde(default)]
    pub version: u32,
    /// `Run::scenario` — the mission this is the memory of.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mission: String,
    /// The outcome label the run ended on, or `None` for a save taken before the
    /// end. A campaign is entitled to know a mission was left unfinished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// The `campaign.*` counters, verbatim and sorted by name — issue #1043's
    /// handoff record, carried rather than re-interpreted.
    ///
    /// `i64`, because that is what a `FlagStore` counter IS and what a later
    /// mission's `counter(name)` predicate compares against. Reading them as
    /// anything else would be re-interpreting a record this module has just
    /// claimed only to carry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tallies: Vec<(String, i64)>,
    /// Promises, in the order they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commitments: Vec<CampaignPromise>,
    /// Findings, in the order they were learned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<CampaignFinding>,
    /// Standing with each party, in authored order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standing: Vec<CampaignStanding>,
    /// Named things still standing, sorted by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<CampaignAsset>,
    /// Structures and what was done to them, sorted by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structures: Vec<CampaignStructure>,
}

impl CampaignFacts {
    /// Look one handoff counter up by its authored name.
    ///
    /// The read a later mission's `counter(name)` predicate is the equivalent
    /// of. `0` for a name the mission never wrote, which is the same answer the
    /// flag store gives — an unwritten counter is zero, not an error, and #1043's
    /// exclusivity invariant is what makes a *family* legible rather than each
    /// member's presence.
    pub fn tally(&self, name: &str) -> i64 {
        self.tallies
            .iter()
            .find(|(id, _)| id == name)
            .map_or(0, |(_, value)| *value)
    }

    /// Whether a promise with this id came out kept.
    pub fn kept(&self, id: &str) -> bool {
        self.commitments
            .iter()
            .any(|promise| promise.id == id && promise.state == "kept")
    }

    /// Whether a promise with this id came out broken.
    pub fn broken(&self, id: &str) -> bool {
        self.commitments
            .iter()
            .any(|promise| promise.id == id && promise.state == "broken")
    }

    /// The condition a named structure was left in, or `None` if the mission
    /// had no such structure.
    pub fn condition_of(&self, name: &str) -> Option<f32> {
        self.structures
            .iter()
            .find(|structure| structure.name == name)
            .map(|structure| structure.condition)
    }
}

/// Fold a finished mission's save into what the campaign remembers.
///
/// Pure. Takes the stored run because the mission's own name lives on the
/// envelope (`Run::scenario`) rather than in the payload, and returns a value
/// built entirely from the two.
///
/// A run with no snapshot — a recording rather than a save — projects to the
/// defaults with the mission name filled in. That is not an error: a mission
/// nobody saved left nothing behind, and saying so is more useful than refusing.
pub fn project(run: &StoredRun) -> CampaignFacts {
    let mut facts = CampaignFacts {
        version: CAMPAIGN_FACTS_VERSION,
        mission: run.scenario.clone(),
        ..CampaignFacts::default()
    };
    let Some(snapshot) = run.snapshot.as_ref().map(|s| &s.state) else {
        return facts;
    };

    facts.outcome = snapshot
        .game_over
        .as_ref()
        .and_then(|(_, outcome)| outcome.clone());
    facts.tallies = campaign_tallies(snapshot);

    let Some(scenario) = snapshot.scenario.as_ref() else {
        // A payload with no scenario state is a bare-`App` capture (the fixtures
        // this crate's unit tests build). The counters above still travelled,
        // because they live on the payload's own flag store; everything below
        // needs the scenario's records and there are none.
        return facts;
    };

    facts.commitments = promises(scenario);
    facts.evidence = findings(scenario);
    facts.standing = standing(scenario);
    facts.assets = assets(snapshot, scenario);
    facts.structures = structures(snapshot, scenario);
    facts
}

/// The `campaign.*` counters, sorted, from the BASE world's flag store.
///
/// Base only, deliberately: a layer's store is a sub-world's private bookkeeping
/// that unloads with it, and #1043 writes the handoff into the base store at the
/// mission's close. A layer that wrote a `campaign.` name would be writing into
/// a book that gets closed.
fn campaign_tallies(snapshot: &PhoenixSnapshot) -> Vec<(String, i64)> {
    let Some(flags) = snapshot.flags.as_ref() else {
        return Vec::new();
    };
    let mut rows: Vec<(String, i64)> = flags
        .iter()
        .filter(|(name, _)| name.starts_with(CAMPAIGN_FLAG_PREFIX))
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn promises(scenario: &ScenarioState) -> Vec<CampaignPromise> {
    scenario
        .commitments
        .records
        .iter()
        .map(|promise| CampaignPromise {
            id: promise.id.clone(),
            made_to: promise.made_to.clone(),
            terms: promise.terms.clone(),
            state: match promise.state {
                CommitmentState::Kept => "kept",
                CommitmentState::Broken => "broken",
                CommitmentState::Open => "open",
            }
            .to_string(),
        })
        .collect()
}

fn findings(scenario: &ScenarioState) -> Vec<CampaignFinding> {
    scenario
        .evidence
        .entries
        .iter()
        .filter_map(|entry| {
            Some(CampaignFinding {
                subject: authored_name(scenario, &entry.subject_uuid)?,
                text: entry.text.clone(),
                provenance: entry.provenance.as_str().to_string(),
            })
        })
        .collect()
}

fn standing(scenario: &ScenarioState) -> Vec<CampaignStanding> {
    scenario
        .workforce
        .records
        .iter()
        .map(|record| CampaignStanding {
            party: record.id.clone(),
            disposition: record.disposition,
            on_strike: record.on_strike,
        })
        .collect()
}

fn assets(snapshot: &PhoenixSnapshot, scenario: &ScenarioState) -> Vec<CampaignAsset> {
    let mut rows: Vec<CampaignAsset> = snapshot
        .entities
        .iter()
        .filter_map(|entity| {
            Some(CampaignAsset {
                name: authored_name(scenario, &entity.uuid)?,
                template: entity
                    .spawn
                    .as_ref()
                    .map(|origin| origin.template_path.clone()),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

fn structures(snapshot: &PhoenixSnapshot, scenario: &ScenarioState) -> Vec<CampaignStructure> {
    let mut rows: Vec<CampaignStructure> = snapshot
        .entities
        .iter()
        .filter_map(|entity| {
            let condition = entity.infrastructure.as_ref()?;
            // `publish = false` is the infrastructure vocabulary's existing
            // declaration that this is a private, scenario-local ledger. It
            // remains authoritative mission state, but it is not a durable
            // structure a campaign may project.
            if !condition.publishes() {
                return None;
            }
            let mut flags: Vec<(String, bool)> = condition
                .flags()
                .into_iter()
                .map(|(flag, held)| (flag.to_string(), held))
                .collect();
            flags.sort_by(|a, b| a.0.cmp(&b.0));
            Some(CampaignStructure {
                name: authored_name(scenario, &entity.uuid)?,
                condition: condition.condition_fraction(),
                flags,
            })
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// Turn a campaign's memory back into the counters a later mission reads
/// (issue #867's handoff half).
///
/// The other end of the seam. A mission's `when = "counter(...) > 0"` predicates
/// and its script's `ctx.flags[...]` reads are the ONE consumer shape a
/// cross-mission fact has (`campaign-flag-handoff-state`), so "configuring a
/// later mission" means seeding its base flag store — and this is that, as a
/// pure function of the facts.
///
/// # It seeds only names that already exist, and that is the whole restraint
///
/// Two families go in, and both are somebody else's vocabulary:
///
/// * every `campaign.*` tally, verbatim, under the name the writing mission
///   wrote (issue #1043);
/// * `commitment.<id>.kept` / `.broken`, through
///   [`crate::world::commitments::kept_flag`] and
///   [`crate::world::commitments::broken_flag`] — the same two functions the
///   commitments vocabulary writes with inside a mission.
///
/// Standing, assets and structures are deliberately NOT seeded, and the reason
/// is the rule this module is a consumer of rather than an author of: there is
/// no declared flag name for "the riggers think well of us" or "the skyhook came
/// out at 45%", and minting one here would be exactly the parallel declaration
/// #1043 refuses — a registry of family names kept in a second place, out of step
/// with the scenario that reads it. Those facts travel as DATA on
/// [`CampaignFacts`], for a campaign runner to feed into a later mission's
/// entity overrides, which is a different consumption shape and not a flag.
///
/// A later mission that wants one of them as a counter should have the mission
/// that produced it write the counter, under the prefix, at its close — which is
/// what #1043 already says.
///
/// # A `campaign.` name is script-readable, not predicate-readable
///
/// Worth knowing before authoring the mission that consumes this, because the
/// failure is a load error rather than a wrong answer: the predicate lexer's
/// identifiers are `[A-Za-z_][A-Za-z0-9_:-]*` (`crate::world::flags`), so a
/// DOTTED name cannot appear inside `counter(...)` or `flag(...)` in a `when`
/// clause at all. The read that works is a script's
/// `ctx.flags["campaign.skyway.strike.negotiated"]`, which is what
/// `falling_skyway` uses on its own record and what this store answers. Both
/// families seeded here are dotted, so both are script reads.
pub fn seed_flags(facts: &CampaignFacts) -> FlagStore {
    let mut store = FlagStore::new();
    for (name, value) in &facts.tallies {
        store.set_flag_value(name, *value);
    }
    for promise in &facts.commitments {
        match promise.state.as_str() {
            "kept" => {
                store.set_flag(&crate::world::commitments::kept_flag(&promise.id));
            }
            "broken" => {
                store.set_flag(&crate::world::commitments::broken_flag(&promise.id));
            }
            // An OPEN promise seeds nothing, and the silence is the answer: the
            // mission ended without settling it, so neither flag is true and a
            // later mission asking "did they keep their word?" gets `false`
            // rather than an invented third name.
            _ => {}
        }
    }
    store
}

/// Resolve a run-scoped uuid back to the name a scenario author wrote.
///
/// The reverse of `name_to_uuid`, which is a name→uuid map because that is the
/// direction a running mission resolves in. Walking it backwards is O(n) over a
/// roster of tens; a second index would be a second thing to keep in step for a
/// fold that happens once per mission.
///
/// `None` for a uuid no name answers to, and the caller drops the row: an
/// unnamed entity is one the next mission cannot ask about.
fn authored_name(scenario: &ScenarioState, uuid: &str) -> Option<String> {
    scenario
        .name_to_uuid
        .iter()
        .find(|(_, id)| id == uuid)
        .map(|(name, _)| name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dossier::evidence::{EvidenceLog, EvidenceProvenance};
    use crate::snapshot::{run_for, EntityState, PhoenixSnapshot, StoredRun};
    use crate::world::commitments::{CommitmentLedger, CommitmentOutcome};
    use crate::world::flags::FlagStore;
    use crate::world::workforce::{WorkforceRecord, WorkforceRegister};
    use vellum_save::Versions;

    const MISSION: &str = "assets/worlds/probe_campaign.toml";
    const SKYHOOK: &str = "world.probe.entity.skyhook.name";
    const SKYHOOK_UUID: &str = "00000000-0000-8000-8000-000000000001";
    const TENDER: &str = "world.probe.entity.tender.name";
    const TENDER_UUID: &str = "00000000-0000-8000-8000-000000000002";

    /// A payload shaped like the end of a mission that did all six kinds of
    /// thing a campaign remembers.
    fn finished_payload() -> PhoenixSnapshot {
        let mut flags = FlagStore::new();
        flags.set_flag_value("campaign.probe.strike.negotiated", 1);
        flags.set_flag_value("campaign.probe.casualties.total", 4);
        flags.set_flag_value("campaign.probe.passage.taken", 2);
        // Not a handoff fact: an ordinary mission-local counter, written by the
        // same store and left behind by the same run.
        flags.set_flag_value("skyway_records_diff_found", 1);

        // Through the ledger's own vocabulary rather than by hand-building
        // records: a promise this module reports as kept is one the ledger
        // agreed to resolve.
        let mut commitments = CommitmentLedger::default();
        commitments
            .record(
                "safe_passage",
                "the committee",
                "world.probe.commitment.safe_passage.terms",
                "",
                10,
            )
            .expect("a fresh id");
        commitments
            .record(
                "berth_for_havelock",
                "havelock",
                "world.probe.commitment.berth.terms",
                "",
                20,
            )
            .expect("a fresh id");
        commitments.resolve("safe_passage", CommitmentOutcome::Kept, 900);
        commitments.resolve("berth_for_havelock", CommitmentOutcome::Broken, 900);

        let mut evidence = EvidenceLog::default();
        evidence.append(
            SKYHOOK_UUID,
            "world.probe.evidence.ladder_b",
            EvidenceProvenance::Scan,
            120,
        );
        // A finding about something no authored name answers to — a rock, a
        // wave NPC, anything the next mission cannot ask after.
        evidence.append(
            "00000000-0000-8000-8000-0000000000ff",
            "world.probe.evidence.nobody",
            EvidenceProvenance::Dialogue,
            130,
        );

        let scenario = ScenarioState {
            name_to_uuid: vec![
                (SKYHOOK.to_string(), SKYHOOK_UUID.to_string()),
                (TENDER.to_string(), TENDER_UUID.to_string()),
            ],
            commitments,
            evidence,
            workforce: WorkforceRegister {
                records: vec![WorkforceRecord {
                    id: "riggers".into(),
                    label: "world.probe.workforce.riggers.label".into(),
                    on_strike: false,
                    disposition: 2,
                }],
                armed: true,
            },
            ..ScenarioState::default()
        };

        PhoenixSnapshot {
            tick: 900,
            flags: Some(flags),
            scenario: Some(scenario),
            game_over: Some((Some("mission_complete".into()), Some("victory".into()))),
            entities: vec![
                EntityState {
                    uuid: SKYHOOK_UUID.to_string(),
                    infrastructure: Some(skyhook_condition()),
                    ..EntityState::default()
                },
                EntityState {
                    uuid: TENDER_UUID.to_string(),
                    spawn: Some(crate::world::spawn_origin::SpawnOrigin {
                        template_path: "assets/entities/alliance_cruiser.toml".into(),
                        name: TENDER.into(),
                        position: [0.0, 0.0, 0.0],
                        ..crate::world::spawn_origin::SpawnOrigin::default()
                    }),
                    ..EntityState::default()
                },
                // Unnamed, and therefore not a campaign asset: a wave NPC the
                // scenario spawned and never named.
                EntityState {
                    uuid: "00000000-0000-8000-8000-0000000000aa".to_string(),
                    ..EntityState::default()
                },
            ],
            ..PhoenixSnapshot::default()
        }
    }

    /// A structure knocked down to half its track, holding one flag and having
    /// dropped the other.
    fn skyhook_condition() -> crate::infrastructure::InfrastructureState {
        use crate::infrastructure::condition::{InfrastructureConfig, ThresholdConfig};
        let mut state =
            crate::infrastructure::InfrastructureState::from_config(&InfrastructureConfig {
                condition: Some(100.0),
                condition_max: 100.0,
                thresholds: vec![
                    ThresholdConfig {
                        flag: "lift_capable".into(),
                        capacity: None,
                        fails_below: 0.6,
                        restores_above: None,
                        label: None,
                    },
                    ThresholdConfig {
                        flag: "tether_stable".into(),
                        capacity: None,
                        fails_below: 0.2,
                        restores_above: None,
                        label: None,
                    },
                ],
                ..InfrastructureConfig::default()
            });
        state.set_condition(45.0);
        state
    }

    fn mission_local_condition() -> crate::infrastructure::InfrastructureState {
        crate::infrastructure::InfrastructureState::from_config(
            &crate::infrastructure::condition::InfrastructureConfig {
                publish: false,
                ..crate::infrastructure::condition::InfrastructureConfig::default()
            },
        )
    }

    fn stored(payload: PhoenixSnapshot) -> StoredRun {
        run_for(
            payload,
            0,
            42,
            MISSION,
            Versions::new(crate::snapshot::SNAPSHOT_FORMAT, "0.1", 0),
        )
    }

    // ── Inclusion ────────────────────────────────────────────────────────────

    #[test]
    fn the_mission_and_how_it_ended_travel() {
        let facts = project(&stored(finished_payload()));
        assert_eq!(facts.version, CAMPAIGN_FACTS_VERSION);
        assert_eq!(facts.mission, MISSION);
        assert_eq!(facts.outcome.as_deref(), Some("victory"));
    }

    #[test]
    fn the_handoff_counters_travel_verbatim_and_sorted() {
        let facts = project(&stored(finished_payload()));
        assert_eq!(
            facts.tallies,
            vec![
                ("campaign.probe.casualties.total".to_string(), 4),
                ("campaign.probe.passage.taken".to_string(), 2),
                ("campaign.probe.strike.negotiated".to_string(), 1),
            ],
            "issue #1043's names, carried rather than re-interpreted — and a \
             mission-local counter written by the same store is not one of them"
        );
        assert_eq!(facts.tally("campaign.probe.casualties.total"), 4);
        assert_eq!(
            facts.tally("skyway_records_diff_found"),
            0,
            "a name outside the prefix is not a handoff fact, and reads as \
             unwritten rather than as itself"
        );
    }

    #[test]
    fn promises_evidence_standing_assets_and_structures_all_travel() {
        let facts = project(&stored(finished_payload()));

        assert!(facts.kept("safe_passage"));
        assert!(facts.broken("berth_for_havelock"));
        assert_eq!(facts.commitments[0].made_to, "the committee");
        assert_eq!(
            facts.commitments[0].terms, "world.probe.commitment.safe_passage.terms",
            "the terms travel as the strings id the ledger holds, not as prose"
        );

        assert_eq!(facts.evidence.len(), 1);
        assert_eq!(facts.evidence[0].subject, SKYHOOK);
        assert_eq!(facts.evidence[0].provenance, "scan");

        assert_eq!(facts.standing.len(), 1);
        assert_eq!(facts.standing[0].party, "riggers");
        assert_eq!(facts.standing[0].disposition, 2);
        assert!(!facts.standing[0].on_strike);

        assert_eq!(
            facts
                .assets
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            vec![SKYHOOK, TENDER],
            "sorted by the authored name, which is the only identity that \
             survives a mission boundary"
        );
        assert_eq!(
            facts.assets[1].template.as_deref(),
            Some("assets/entities/alliance_cruiser.toml"),
            "a runtime spawn carries the template it was made from (#863); an \
             authored entity does not, because the next world file names its own"
        );
        assert_eq!(facts.assets[0].template, None);

        assert_eq!(facts.structures.len(), 1);
        assert_eq!(facts.structures[0].name, SKYHOOK);
        assert!((facts.structures[0].condition - 0.45).abs() < 1e-5);
        assert_eq!(
            facts.structures[0].flags,
            vec![
                ("lift_capable".to_string(), false),
                ("tether_stable".to_string(), true),
            ],
            "the operational flags as the mission left them — which is what a \
             later mission opens on, rather than what its own world file authors"
        );
    }

    #[test]
    fn unpublished_mission_local_infrastructure_is_not_a_campaign_structure() {
        let mut payload = finished_payload();
        payload.entities[0].infrastructure = Some(mission_local_condition());

        let facts = project(&stored(payload));

        assert!(
            facts.assets.iter().any(|asset| asset.name == SKYHOOK),
            "publish=false narrows only the infrastructure projection; it does not broadly erase a real named asset"
        );
        assert!(
            facts.structures.is_empty(),
            "the infrastructure vocabulary already marks this ledger private to the mission, so it must not become campaign structure state"
        );
    }

    // ── Exclusion ────────────────────────────────────────────────────────────

    /// **The exclusion claim, stated as an invariant rather than as a list.**
    ///
    /// Everything transient is varied at once — hull, alert, helm axes, weapon
    /// machines, physics, the asteroid belt, the RNG, the tick — and the facts
    /// must be byte-identical. A test that enumerated excluded FIELDS would go
    /// stale the first time a component was added; this one cannot, because it
    /// says nothing about which fields exist.
    #[test]
    fn transient_combat_state_cannot_reach_the_facts() {
        let calm = finished_payload();
        let quiet = project(&stored(calm.clone()));

        let mut fought = calm;
        fought.tick = 4_000;
        fought.rng = None;
        fought.asteroids = vec![crate::snapshot::AsteroidState {
            uuid: "rock-1".into(),
            translation: [10.0, 0.0, 20.0],
            ..crate::snapshot::AsteroidState::default()
        }];
        fought.collisions = vec![crate::snapshot::CollisionRecord {
            tick: 3_900,
            sim_t: 65.0,
            victim: SKYHOOK_UUID.into(),
            victim_is_asteroid: false,
            amount: 40.0,
            shield_absorbed: 10.0,
            hull_damage: 30.0,
        }];
        for entity in &mut fought.entities {
            entity.physics = Some([1.0, 2.0, 3.0, 0.5, 40.0, 0.1, 0.0, 0.0]);
            entity.hull = Some(vec![("captain".to_string(), 3.0, 500.0)]);
            entity.red_alert = Some(true);
            entity.weapons_hold = Some(true);
            entity.control = Some(crate::snapshot::ControlState {
                thrust: 1.0,
                steering: -1.0,
                ..crate::snapshot::ControlState::default()
            });
            entity.weapons = Some(crate::snapshot::WeaponState {
                beams: vec![("fore".to_string(), "x".to_string(), 1.5, 0.25, 6.0)],
                ..crate::snapshot::WeaponState::default()
            });
        }

        assert_eq!(
            project(&stored(fought)),
            quiet,
            "a mauled, shooting, moving world at a different tick hands the next \
             mission exactly what a quiet one does — the facts have nowhere for \
             any of it to arrive"
        );
    }

    /// An entity with no authored name is not an asset, however real it is.
    #[test]
    fn an_unnamed_entity_is_not_carried_under_a_number() {
        let facts = project(&stored(finished_payload()));
        assert_eq!(facts.assets.len(), 2);
        assert!(
            !facts
                .assets
                .iter()
                .any(|asset| asset.name.contains("0000000000aa")),
            "a uuid is minted per run, so carrying one forward names nothing in \
             the mission that reads it"
        );
    }

    // ── Stable identity ──────────────────────────────────────────────────────

    /// **The identity claim.** The same mission run twice mints different uuids
    /// for the same authored things, and the facts must not notice.
    #[test]
    fn the_same_mission_run_twice_projects_the_same_identities() {
        let first = project(&stored(finished_payload()));

        // A second run of the same content: same authored names, every uuid
        // different — which is what a re-run actually produces, because the mint
        // is tick-and-sequence scoped rather than content-scoped.
        let mut second_payload = finished_payload();
        let remap = |uuid: &str| format!("11111111-{}", &uuid[9..]);
        if let Some(scenario) = second_payload.scenario.as_mut() {
            for (_, uuid) in &mut scenario.name_to_uuid {
                *uuid = remap(uuid);
            }
            for entry in &mut scenario.evidence.entries {
                entry.subject_uuid = remap(&entry.subject_uuid);
            }
        }
        for entity in &mut second_payload.entities {
            entity.uuid = remap(&entity.uuid);
        }

        let second = project(&stored(second_payload));
        assert_eq!(
            second, first,
            "every identity that leaves the projection is an authored name, so \
             two runs of the same mission hand the campaign the same facts"
        );
    }

    // ── Version and defaults ─────────────────────────────────────────────────

    #[test]
    fn a_run_with_no_snapshot_projects_to_defaults_with_the_mission_named() {
        let mut run = stored(finished_payload());
        run.snapshot = None;
        let facts = project(&run);
        assert_eq!(facts.mission, MISSION);
        assert_eq!(facts.version, CAMPAIGN_FACTS_VERSION);
        assert_eq!(facts.tallies, Vec::new());
        assert_eq!(facts.commitments, Vec::new());
        assert_eq!(facts.outcome, None);
    }

    /// A payload with no scenario record — a bare-`App` capture — still hands
    /// over its counters, because those live on the payload's own flag store.
    #[test]
    fn a_payload_with_no_scenario_record_still_carries_its_counters() {
        let mut payload = finished_payload();
        payload.scenario = None;
        let facts = project(&stored(payload));
        assert_eq!(facts.tallies.len(), 3);
        assert!(facts.commitments.is_empty());
        assert!(facts.evidence.is_empty());
        assert!(facts.structures.is_empty());
    }

    /// Facts written down by a build that predates the version field read back
    /// as version 0 — an honest "I do not know which vocabulary this is" rather
    /// than a guess that it is this one.
    #[test]
    fn facts_from_before_the_version_field_read_back_as_version_zero() {
        let older: CampaignFacts = ron::from_str("(mission: \"old.toml\")").expect("parses");
        assert_eq!(older.version, 0);
        assert_eq!(older.mission, "old.toml");
        assert!(older.tallies.is_empty());
    }

    // ── The handoff, as a later mission reads it ─────────────────────────────

    /// **The consumption shape.** Seeded facts answer the reads a later mission
    /// is authored with — which is the only thing "configuring a later mission"
    /// can mean without building one.
    ///
    /// Through `counter`/`flag` rather than through a parsed `when` predicate,
    /// and that is not a shortcut: see [`seed_flags`]. A `campaign.` name has a
    /// dot in it and the predicate lexer's identifiers do not, so the read a
    /// later mission actually performs is a script's `ctx.flags["campaign.…"]`
    /// — which is exactly this call, and exactly what `falling_skyway` itself
    /// does with the record it wrote.
    #[test]
    fn seeded_facts_answer_the_reads_a_later_mission_makes() {
        let facts = project(&stored(finished_payload()));
        let seeded = seed_flags(&facts);

        // A next mission opening differently because the strike was settled at
        // the table rather than forced.
        assert!(seeded.counter("campaign.probe.strike.negotiated") > 0);
        assert_eq!(seeded.counter("campaign.probe.strike.forced"), 0);
        // …and because of what it cost.
        assert_eq!(seeded.counter("campaign.probe.casualties.total"), 4);
        // The promises, through the commitments vocabulary's OWN flag names —
        // the same two a mission writes with while it is running.
        assert!(seeded.flag(&crate::world::commitments::kept_flag("safe_passage")));
        assert!(seeded.flag(&crate::world::commitments::broken_flag(
            "berth_for_havelock"
        )));
        assert!(!seeded.flag(&crate::world::commitments::broken_flag("safe_passage")));
    }

    /// An unsettled promise seeds neither flag, so a later mission asking
    /// whether the crew kept their word gets `false` rather than a third name.
    #[test]
    fn an_open_promise_seeds_no_flag_either_way() {
        let mut facts = project(&stored(finished_payload()));
        facts.commitments[0].state = "open".to_string();
        let seeded = seed_flags(&facts);
        assert!(!seeded.flag(&crate::world::commitments::kept_flag("safe_passage")));
        assert!(!seeded.flag(&crate::world::commitments::broken_flag("safe_passage")));
    }

    /// Seeding invents no names: everything in the store came from a tally the
    /// last mission wrote or from the commitments vocabulary.
    #[test]
    fn seeding_writes_only_names_that_already_had_owners() {
        let facts = project(&stored(finished_payload()));
        let seeded = seed_flags(&facts);
        for (name, _) in seeded.iter() {
            assert!(
                name.starts_with(CAMPAIGN_FLAG_PREFIX) || name.starts_with("commitment."),
                "`{name}` is a name this module minted — which is the parallel                  declaration issue #1043 refuses. Standing, assets and structures                  travel as data, not as invented counters"
            );
        }
        assert!(
            !seeded.iter().any(|(name, _)| name.contains("riggers")),
            "the workforce standing is real and is NOT seeded, because no              declared flag name owns it"
        );
    }

    /// And a projection round-trips, because a campaign runner has to be able to
    /// write one down between missions.
    #[test]
    fn facts_round_trip_through_ron() {
        let facts = project(&stored(finished_payload()));
        let text = ron::ser::to_string(&facts).expect("serialises");
        let back: CampaignFacts = ron::from_str(&text).expect("parses back");
        assert_eq!(back, facts);
    }
}
