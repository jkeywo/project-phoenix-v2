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
#[path = "projection_tests.rs"]
mod tests;
