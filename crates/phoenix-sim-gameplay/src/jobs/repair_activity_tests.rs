use super::*;
use phoenix_model::activity::{ActivityOutcome, RepairAchievement};

#[test]
fn external_commitment_cannot_be_relabelled_as_an_accepted_internal_job() {
    let (mut teams, _, _, _) = fixture(4);
    assert!(teams.claim_external(0, "actual-ally".into()));
    let before = teams.save_state();
    assert_eq!(
        teams.dispatch_physical_job(
            0,
            SystemId("engine".into()),
            Some(RepairWorkTarget::Compartment {
                compartment: "front".into()
            }),
            "Front".into()
        ),
        None
    );
    assert_eq!(teams.save_state(), before);
}

fn physical_step(
    teams: &mut RepairTeams,
    damage: &mut DamageState,
    stock: &mut InfrastructureState,
    services: &BTreeMap<String, ResourceEndpoint>,
    deny_consume: bool,
) -> RepairTickResult {
    teams.tick_physical_with_expertise(
        0.25,
        damage,
        &BTreeSet::new(),
        &BTreeSet::new(),
        &[],
        |action, claim, recipe| {
            if deny_consume && action == RepairStockAction::Consume {
                return false;
            }
            stock.repair_parts(action, claim, recipe, services).is_ok()
        },
        |_, _| 1.0,
    )
}

#[test]
fn actual_capped_repair_result_survives_recall_and_settlement_is_separate() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    damage.compartments.get_mut("front").unwrap().hp = 95.0;
    let job = teams
        .dispatch_physical_job(
            0,
            SystemId("engine".into()),
            Some(RepairWorkTarget::Compartment {
                compartment: "front".into(),
            }),
            "Front".into(),
        )
        .unwrap();
    let mut achieved = None;
    for _ in 0..24 {
        let result = physical_step(&mut teams, &mut damage, &mut stock, &services, false);
        assert!(!result
            .outcomes
            .iter()
            .any(|(_, outcome)| matches!(outcome, ActivityOutcome::RepairSettled { .. })));
        if !result.outcomes.is_empty() {
            achieved = Some(result);
            break;
        }
    }
    let achieved = achieved.expect("real paid stage must complete");
    assert_eq!(achieved.outcomes.len(), 1);
    assert_eq!(achieved.outcomes[0].0, job);
    assert!(
        matches!(&achieved.outcomes[0].1,ActivityOutcome::RepairAchieved {remedy,result:RepairAchievement::Structure {restored_hp},..} if remedy=="structure" && *restored_hp==5.0)
    );
    assert_eq!(damage.compartments["front"].hp, 100.0);
    assert_eq!(stock.capacity_stock("parts").unwrap().level, 3);
    assert!(teams.recall(0));
    let mut settled = Vec::new();
    for _ in 0..24 {
        settled
            .extend(physical_step(&mut teams, &mut damage, &mut stock, &services, false).outcomes);
    }
    assert_eq!(
        settled,
        vec![(
            job,
            ActivityOutcome::RepairSettled {
                outcome: phoenix_model::job::JobOutcome::Cancelled
            }
        )]
    );
    assert_eq!(
        damage.compartments["front"].hp, 100.0,
        "recall never undoes achieved work"
    );
    assert_eq!(
        achieved.outcomes.len(),
        1,
        "historical result remains independent of later team row"
    );
    assert_ne!(
        teams
            .dispatch_physical_job(0, SystemId("scanner".into()), None, "Next".into())
            .unwrap(),
        job
    );
}

#[test]
fn rejected_actual_consume_produces_no_achievement_or_condition_write() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    damage.compartments.get_mut("front").unwrap().hp = 95.0;
    teams
        .dispatch_physical_job(
            0,
            SystemId("engine".into()),
            Some(RepairWorkTarget::Compartment {
                compartment: "front".into(),
            }),
            "Front".into(),
        )
        .unwrap();
    for _ in 0..24 {
        assert!(
            physical_step(&mut teams, &mut damage, &mut stock, &services, true)
                .outcomes
                .is_empty()
        );
        assert_eq!(damage.compartments["front"].hp, 95.0);
        assert_eq!(stock.capacity_stock("parts").unwrap().level, 4);
    }
}

#[test]
fn zero_representable_restoration_never_emits_a_positive_achievement() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    teams
        .damage_definition
        .as_mut()
        .unwrap()
        .config
        .remedies
        .get_mut("structure")
        .unwrap()
        .structural_hp = Some(1.0e-30);
    damage.compartments.get_mut("front").unwrap().hp = 95.0;
    teams
        .dispatch_physical_job(
            0,
            SystemId("engine".into()),
            Some(RepairWorkTarget::Compartment {
                compartment: "front".into(),
            }),
            "Front".into(),
        )
        .unwrap();
    for _ in 0..24 {
        let result = physical_step(&mut teams, &mut damage, &mut stock, &services, false);
        assert!(!result
            .outcomes
            .iter()
            .any(|(_, outcome)| matches!(outcome, ActivityOutcome::RepairAchieved { .. })));
        assert_eq!(damage.compartments["front"].hp, 95.0);
    }
}
