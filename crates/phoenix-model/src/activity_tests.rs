use super::*;
#[test]
fn received_and_returned_are_distinct_closed_historical_facts() {
    let fact = ActivityFact {
        instance: "deliver-one".into(),
        scope: ActivityJob {
            actor: "ship".into(),
            owner: SystemId("handling".into()),
            job: JobId(3),
        }
        .into(),
        tick: 10,
        outcome: ActivityOutcome::Delivery {
            source: "main".into(),
            destination: "local".into(),
            commodity: "parts".into(),
            amount: 4,
            returned: false,
            outcome: JobOutcome::Completed,
        },
    };
    fact.validate(10).unwrap();
    let saved = serde_json::to_string(&fact).unwrap();
    assert_eq!(serde_json::from_str::<ActivityFact>(&saved).unwrap(), fact);
    let mut returned = fact.clone();
    if let ActivityOutcome::Delivery {
        returned,
        destination,
        outcome,
        ..
    } = &mut returned.outcome
    {
        *returned = true;
        *destination = "main".into();
        *outcome = JobOutcome::Cancelled;
    }
    returned.validate(10).unwrap();
    assert_ne!(fact, returned);
    assert!(returned.validate(9).is_err());
    if let ActivityOutcome::Delivery { amount, .. } = &mut returned.outcome {
        *amount = 0;
    }
    assert!(returned.validate(10).is_err());
    assert!(serde_json::from_str::<ActivityFact>(
        &saved.replace("\"tick\":10", "\"tick\":10,\"forged\":true")
    )
    .is_err());
}

#[test]
fn execution_identity_is_closed_and_communication_cannot_impersonate_a_physical_job() {
    let physical: ActivityExecution = ActivityJob {
        actor: "ship".into(),
        owner: SystemId("repair".into()),
        job: JobId(9),
    }
    .into();
    assert_eq!(physical.physical_job(), Some(JobId(9)));
    physical.validate(10).unwrap();
    let mut value = serde_json::to_value(&physical).unwrap();
    value["identity"]["recipient"] = "forged".into();
    assert!(serde_json::from_value::<ActivityExecution>(value).is_err());
    let communication = ActivityExecution {
        actor: "ship".into(),
        owner: SystemId("comms".into()),
        identity: ActivityExecutionIdentity::Communication {
            origin_id: "actual-origin".into(),
            recipient: "port".into(),
            emitted_tick: 10,
        },
    };
    assert_eq!(communication.physical_job(), None);
    communication.validate(10).unwrap();
    assert!(communication.validate(9).is_err());
    let fact = ActivityFact {
        instance: "repair".into(),
        scope: communication,
        tick: 10,
        outcome: ActivityOutcome::RepairSettled {
            outcome: JobOutcome::Cancelled,
        },
    };
    assert!(fact.validate(10).is_err());
}
