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
