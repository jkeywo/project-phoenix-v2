//! Optional maintenance proposals and their immutable acceptance history.
//! Physical work stays in existing Situation, Repair and Logistics owners.
use super::{config::WorldConfig, server::WorldContentRuntime};
use phoenix_model::comms_request::{OpportunityDecision, OpportunityRef};
use phoenix_sim_world::world::planning::{opportunities::*, PlanChoice, PlanExecution};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunitySourceFailure {
    MissingUnit,
    Unresolved,
    Refused,
    Shape,
    NoNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpportunityState {
    Offered,
    SourceUnavailable {
        tick: u64,
        cause: OpportunitySourceFailure,
    },
    Pending {
        decision: OpportunityDecision,
        tick: u64,
    },
    Declined {
        tick: u64,
    },
    Refused {
        decision_tick: u64,
        tick: u64,
        reason: String,
    },
    Accepted {
        decision_tick: u64,
        input: MaintenanceOpportunityInput,
        choice: PlanChoice,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpportunityRecord {
    pub reference: OpportunityRef,
    pub discovered_tick: u64,
    pub input: MaintenanceOpportunityInput,
    pub proposal: MaintenanceSearchResult,
    pub state: OpportunityState,
}
pub struct PreparedOpportunityDecision<'a> {
    record: &'a mut OpportunityRecord,
    decision: OpportunityDecision,
    tick: u64,
}
fn reference_record<'a>(
    records: &'a [OpportunityRecord],
    reference: &OpportunityRef,
    actor: &str,
    now: u64,
) -> Result<&'a OpportunityRecord, String> {
    reference.validate()?;
    if reference.actor != actor {
        return Err("opportunity recipient differs from its actual actor".into());
    }
    let mut matching = records.iter().filter(|row| &row.reference == reference);
    let record = matching
        .next()
        .ok_or("unknown immutable opportunity context")?;
    if matching.next().is_some() || record.discovered_tick > now {
        return Err("duplicate or future opportunity context".into());
    }
    Ok(record)
}
pub fn validate_offered_opportunity(
    runtime: &WorldContentRuntime,
    reference: &OpportunityRef,
    actor: &str,
    now: u64,
) -> Result<(), String> {
    if !matches!(
        reference_record(&runtime.opportunities, reference, actor, now)?.state,
        OpportunityState::Offered
    ) {
        return Err("opportunity has already received a decision".into());
    }
    Ok(())
}
/// Actual final root failure, never a fabricated player decision. The caller
/// has discarded the root's effects and does not discard budget deferrals.
pub fn refuse_opportunity_source(
    runtime: &mut WorldContentRuntime,
    reference: &OpportunityRef,
    actor: &str,
    now: u64,
    cause: OpportunitySourceFailure,
) -> Result<(), String> {
    validate_offered_opportunity(runtime, reference, actor, now)?;
    let record = runtime
        .opportunities
        .iter_mut()
        .find(|row| &row.reference == reference)
        .expect("exact Offered record checked under exclusive borrow");
    record.state = OpportunityState::SourceUnavailable { tick: now, cause };
    Ok(())
}

pub fn prepare_opportunity_decision<'a>(
    runtime: &'a mut WorldContentRuntime,
    reference: &OpportunityRef,
    actor: &str,
    decision: OpportunityDecision,
    now: u64,
) -> Result<PreparedOpportunityDecision<'a>, String> {
    validate_offered_opportunity(runtime, reference, actor, now)?;
    let record = runtime
        .opportunities
        .iter_mut()
        .find(|row| &row.reference == reference)
        .expect("exact immutable offered record checked under exclusive borrow");
    Ok(PreparedOpportunityDecision {
        record,
        decision,
        tick: now,
    })
}
pub fn record_opportunity_decision(prepared: PreparedOpportunityDecision<'_>) {
    prepared.record.state = OpportunityState::Pending {
        decision: prepared.decision,
        tick: prepared.tick,
    };
}
pub fn validate_opportunity_reference(
    snapshot: &crate::snapshot::PhoenixSnapshot,
    reference: &OpportunityRef,
    actor: &str,
    now: u64,
) -> Result<(), String> {
    let records = &snapshot
        .scenario
        .as_ref()
        .ok_or("opportunity context lacks its World owner")?
        .opportunities;
    reference_record(records, reference, actor, now)?;
    Ok(())
}
pub fn validate_offered_opportunity_reference(
    snapshot: &crate::snapshot::PhoenixSnapshot,
    reference: &OpportunityRef,
    actor: &str,
    now: u64,
) -> Result<(), String> {
    let records = &snapshot
        .scenario
        .as_ref()
        .ok_or("opportunity context lacks its World owner")?
        .opportunities;
    if !matches!(
        reference_record(records, reference, actor, now)?.state,
        OpportunityState::Offered
    ) {
        return Err("pending offer context is no longer offered".into());
    }
    Ok(())
}

/// Reconstruct frozen proposals/accepted children before any restore writes.
/// Original discovery inputs are checked against immutable recipe limits;
/// current usefulness and current stock never replace accepted history.
pub(crate) fn validate_history(
    world: &bevy::prelude::World,
    snapshot: &crate::snapshot::PhoenixSnapshot,
) -> Result<
    Vec<(
        PlanChoice,
        phoenix_sim_world::world::planning::Decomposition,
    )>,
    String,
> {
    let Some(stored) = &snapshot.scenario else {
        return Ok(Vec::new());
    };
    if stored.opportunities.is_empty() {
        if stored
            .situations
            .instances
            .keys()
            .any(|id| id.starts_with("htn-optional/"))
            || stored
                .commitments
                .records
                .iter()
                .any(|row| row.id.starts_with("opportunity."))
        {
            return Err(
                "orphan optional execution or generated promise lacks its immutable record".into(),
            );
        }
        return Ok(Vec::new());
    }
    let base = world
        .get_resource::<WorldConfig>()
        .ok_or("optional history lacks accepted World source")?;
    let Some(source) = &base.compiled_opportunity_search else {
        return if stored.opportunities.is_empty() {
            Ok(Vec::new())
        } else {
            Err("optional history lacks accepted operator source".into())
        };
    };
    if stored.opportunities.len() > 8 {
        return Err("optional history exceeds its retained bound".into());
    }
    let actor = base
        .name_to_uuid
        .get(&source.config.actor)
        .ok_or("optional source actor lacks accepted identity")?;
    if let Some(comms) = &snapshot.comms {
        for open in &comms.pending_opens {
            if let Some(reference) = &open.opportunity {
                if reference.source != source.config.id
                    || open.script_path != source.config.offer_script
                    || open.root_fn != source.config.offer_function
                    || open.thread_id.as_deref() != Some(reference.instance_id().as_str())
                {
                    return Err(
                        "pending opportunity changed its accepted script/root/source binding"
                            .into(),
                    );
                }
            }
        }
        for dialogue in &comms.dialogues {
            if dialogue.script.opportunity.is_some()
                && dialogue.script.script_path != source.config.offer_script
            {
                return Err("opportunity reply changed its accepted script unit".into());
            }
        }
    }
    let recipe =
        crate::snapshot::activities::accepted_recipe(world, snapshot, &source.config.actor)?;
    let mut frontier = match (&base.compiled_planning, &stored.plan_choice) {
        (Some(compiled), Some(choice)) => compiled
            .validate_history(choice, &stored.situations, snapshot.tick)?
            .settled_tick(choice, &stored.situations, snapshot.tick)?,
        (None, None) => Some(0),
        _ => None,
    };
    let mut graphs = Vec::new();
    for (index, record) in stored.opportunities.iter().enumerate() {
        record.reference.validate()?;
        if record.reference.source != source.config.id
            || record.reference.ordinal != index as u16 + 1
            || &record.reference.actor != actor
            || record.discovered_tick > snapshot.tick
            || frontier.is_none_or(|tick| tick >= record.discovered_tick)
        {
            return Err(
                "optional discovery replaces source, actor, ordinal or prior settlement".into(),
            );
        }
        phoenix_content_preparation::world::opportunities::validate_input(
            source,
            &recipe,
            &record.input,
        )?;
        source.derive(&record.reference, &record.input, &record.proposal)?;
        if record.input.report.as_ref().is_none_or(|report| {
            report
                .received_tick
                .is_none_or(|tick| tick > record.discovered_tick)
        }) {
            return Err("discovery report did not precede its original offer".into());
        }

        let valid_tick = |tick: u64| record.discovered_tick <= tick && tick <= snapshot.tick;
        let promise_id = format!(
            "opportunity.{}.{}",
            record.reference.source, record.reference.ordinal
        );
        let promises = stored
            .commitments
            .records
            .iter()
            .filter(|row| row.id == promise_id)
            .collect::<Vec<_>>();
        // Retained originating presentation cannot contradict the historical
        // decision. Inbox pruning does not erase the immutable World record,
        // and this check never re-runs historical scripted effects.
        if let Some(comms) = &snapshot.comms {
            let originating = comms
                .inbox
                .iter()
                .filter(|message| {
                    message.thread_id == record.reference.instance_id()
                        && message.sender_uuid == "_self"
                        && message
                            .recipient_ship
                            .as_ref()
                            .is_some_and(|ship| ship.0 == record.reference.actor)
                        && !message.responses.is_empty()
                })
                .collect::<Vec<_>>();
            let selected = originating.iter().any(|message| {
                message
                    .selected_response
                    .is_some_and(|index| index < message.responses.len())
            });
            let decided = matches!(
                record.state,
                OpportunityState::Pending { .. }
                    | OpportunityState::Declined { .. }
                    | OpportunityState::Refused { .. }
                    | OpportunityState::Accepted { .. }
            );
            if (!originating.is_empty() && decided && !selected)
                || (matches!(record.state, OpportunityState::Offered) && selected)
            {
                return Err(
                    "optional decision contradicts its retained originating Comms response".into(),
                );
            }
        }
        frontier = match &record.state {
            OpportunityState::Offered => {
                let envelopes = snapshot.comms.as_ref().map_or(0, |comms| {
                    comms
                        .pending_opens
                        .iter()
                        .filter(|open| open.opportunity.as_ref() == Some(&record.reference))
                        .count()
                        + comms
                            .dialogues
                            .iter()
                            .filter(|dialogue| {
                                dialogue.script.opportunity.as_ref() == Some(&record.reference)
                            })
                            .count()
                });
                if envelopes != 1 {
                    return Err(
                        "offered maintenance lost its unique pending or active Comms envelope"
                            .into(),
                    );
                }
                None
            }
            OpportunityState::SourceUnavailable { tick, .. } if valid_tick(*tick) => {
                if snapshot.comms.as_ref().is_some_and(|comms| {
                    comms
                        .pending_opens
                        .iter()
                        .any(|open| open.opportunity.as_ref() == Some(&record.reference))
                        || comms.dialogues.iter().any(|dialogue| {
                            dialogue.script.opportunity.as_ref() == Some(&record.reference)
                        })
                }) {
                    return Err(
                        "unavailable source retains a supposedly answerable offer envelope".into(),
                    );
                }
                Some(*tick)
            }
            OpportunityState::Pending { tick, .. } if valid_tick(*tick) => None,
            OpportunityState::Declined { tick } if valid_tick(*tick) => Some(*tick),
            OpportunityState::Refused {
                decision_tick,
                tick,
                reason,
            } if valid_tick(*decision_tick)
                && *decision_tick <= *tick
                && *tick <= snapshot.tick
                && !reason.trim().is_empty()
                && reason.len() <= 2048
                && !reason.chars().any(char::is_control) =>
            {
                Some(*tick)
            }
            OpportunityState::Accepted {
                decision_tick,
                input,
                choice,
            } => {
                if !valid_tick(*decision_tick)
                    || *decision_tick > choice.chosen_tick
                    || choice.chosen_tick > snapshot.tick
                    || choice.id != source.config.id
                    || choice.actor_uuid != *actor
                    || choice.required_commitment.as_deref() != Some(promise_id.as_str())
                    || choice.execution
                        != (PlanExecution::Optional {
                            source: source.config.id.clone(),
                            ordinal: record.reference.ordinal,
                        })
                    || promises.len() != 1
                {
                    return Err(
                        "accepted optional binding or original decision clocks disagree".into(),
                    );
                }
                let promise = promises[0];
                if promise.made_at_tick != choice.chosen_tick
                    || promise.made_to != *actor
                    || promise.terms != source.config.terms
                    || promise.resolves_when != source.config.resolves_when
                    || !matches!(
                        (promise.state, promise.resolved_at_tick),
                        (crate::world::commitments::CommitmentState::Open, None)
                    ) && !matches!((promise.state,promise.resolved_at_tick),(crate::world::commitments::CommitmentState::Kept|crate::world::commitments::CommitmentState::Broken,Some(tick)) if choice.chosen_tick<=tick && tick<=snapshot.tick)
                {
                    return Err(
                        "optional promise lost its exact atomic acceptance interval or terms"
                            .into(),
                    );
                }
                phoenix_content_preparation::world::opportunities::validate_input(
                    source, &recipe, input,
                )?;
                if input.report.as_ref().is_none_or(|report| {
                    report
                        .received_tick
                        .is_none_or(|tick| tick > choice.chosen_tick)
                }) {
                    return Err("admission report follows original acceptance".into());
                }
                let graph = source.derive(&record.reference, input, &record.proposal)?;
                graph.validate_history(choice, &stored.situations, snapshot.tick)?;
                let settled = graph.settled_tick(choice, &stored.situations, snapshot.tick)?;
                graphs.push((choice.clone(), graph));
                settled
            }
            _ => return Err("invalid optional decision state or original clocks".into()),
        };
        if !matches!(record.state, OpportunityState::Accepted { .. }) && !promises.is_empty() {
            return Err("unaccepted opportunity has a generated promise".into());
        }
    }
    let expected = graphs
        .iter()
        .flat_map(|(choice, graph)| {
            choice
                .released
                .keys()
                .map(|node| graph.leaves[node].id.as_str())
        })
        .collect::<std::collections::BTreeSet<_>>();
    if stored
        .situations
        .instances
        .keys()
        .any(|id| id.starts_with("htn-optional/") && !expected.contains(id.as_str()))
    {
        return Err("orphan optional leaf lacks its original accepted execution record".into());
    }
    for promise in &stored.commitments.records {
        if promise.id.starts_with("opportunity.")
            && !stored.opportunities.iter().any(|row| {
                format!(
                    "opportunity.{}.{}",
                    row.reference.source, row.reference.ordinal
                ) == promise.id
                    && matches!(row.state, OpportunityState::Accepted { .. })
            })
        {
            return Err("orphan generated opportunity promise".into());
        }
    }
    Ok(graphs)
}
fn choice(record: &OpportunityRecord, now: u64) -> PlanChoice {
    PlanChoice {
        id: record.reference.source.clone(),
        required_commitment: Some(format!(
            "opportunity.{}.{}",
            record.reference.source, record.reference.ordinal
        )),
        execution: PlanExecution::Optional {
            source: record.reference.source.clone(),
            ordinal: record.reference.ordinal,
        },
        root_arguments: BTreeMap::new(),
        chosen_tick: now,
        actor_uuid: record.reference.actor.clone(),
        released: BTreeMap::new(),
        methods: BTreeMap::new(),
        refused_methods: Vec::new(),
        temporal: None,
        temporal_reconsiderations: Vec::new(),
    }
}
/// Conservative first join: prior work must actually settle and callbacks end.
fn settled_frontier(
    runtime: &WorldContentRuntime,
    base: &WorldConfig,
    source: &CompiledMaintenanceSearch,
    now: u64,
) -> Result<Option<u64>, String> {
    let mut latest = 0;
    match (&base.compiled_planning, &runtime.plan_choice) {
        (Some(compiled), Some(choice)) => {
            let graph = compiled.validate_history(choice, &runtime.situations, now)?;
            let Some(tick) = graph.settled_tick(choice, &runtime.situations, now)? else {
                return Ok(None);
            };
            latest = latest.max(tick);
        }
        (Some(_), None) => return Ok(None),
        (None, Some(_)) => return Err("primary work has no accepted source".into()),
        (None, None) => {}
    }
    for record in &runtime.opportunities {
        match &record.state {
            OpportunityState::Accepted { input, choice, .. } => {
                let graph = source.derive(&record.reference, input, &record.proposal)?;
                let Some(tick) = graph.settled_tick(choice, &runtime.situations, now)? else {
                    return Ok(None);
                };
                latest = latest.max(tick);
            }
            OpportunityState::Declined { tick }
            | OpportunityState::Refused { tick, .. }
            | OpportunityState::SourceUnavailable { tick, .. } => latest = latest.max(*tick),
            OpportunityState::Offered | OpportunityState::Pending { .. } => return Ok(None),
        }
    }
    Ok(Some(latest))
}
/// Advance already accepted children through the exact common decomposition
/// release transaction. No search, deadline reset or method swap participates.
pub(crate) fn advance(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    actor: &str,
    now: u64,
) -> Result<(), String> {
    let Some(source) = &base.compiled_opportunity_search else {
        return Ok(());
    };
    for index in 0..runtime.opportunities.len() {
        let record = runtime.opportunities[index].clone();
        if record.reference.actor != actor {
            continue;
        }
        let OpportunityState::Accepted {
            input, mut choice, ..
        } = record.state
        else {
            continue;
        };
        let graph = source.derive(&record.reference, &input, &record.proposal)?;
        graph.release(&mut choice, &mut runtime.situations, actor, now)?;
        if let OpportunityState::Accepted { choice: stored, .. } =
            &mut runtime.opportunities[index].state
        {
            *stored = choice;
        }
    }
    Ok(())
}
/// The caller supplies its actual finite maintenance projection immediately
/// before admission. Every mutation is staged, including the existing Promise.
pub(crate) fn consider(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    actor: &str,
    now: u64,
    inputs: &[MaintenanceOpportunityInput],
) -> Result<Option<OpportunityRef>, String> {
    let Some(source) = &base.compiled_opportunity_search else {
        return Ok(None);
    };
    if base
        .name_to_uuid
        .get(&source.config.actor)
        .map(String::as_str)
        != Some(actor)
    {
        return Ok(None);
    }
    for index in 0..runtime.opportunities.len() {
        let record = runtime.opportunities[index].clone();
        if record.reference.actor != actor {
            continue;
        }
        let OpportunityState::Pending { decision, tick } = record.state else {
            continue;
        };
        if decision == OpportunityDecision::Decline {
            runtime.opportunities[index].state = OpportunityState::Declined { tick };
            continue;
        }
        let admission = inputs
            .iter()
            .find(|input| input.compartment == record.input.compartment)
            .filter(|input| source.search(input).ok().flatten().as_ref() == Some(&record.proposal))
            .cloned();
        let Some(input) = admission else {
            runtime.opportunities[index].state = OpportunityState::Refused {
                decision_tick: tick,
                tick: now,
                reason:
                    "the actual maintenance need or available resources changed before acceptance"
                        .into(),
            };
            continue;
        };
        let mut selected = choice(&record, now);
        let graph = source.derive(&record.reference, &input, &record.proposal)?;
        let mut instances = runtime.situations.clone();
        let mut commitments = runtime.commitments.clone();
        commitments
            .record(
                selected
                    .required_commitment
                    .as_deref()
                    .expect("generated promise"),
                actor,
                &source.config.terms,
                &source.config.resolves_when,
                now,
            )
            .map_err(|_| "generated opportunity promise identity already exists")?;
        if let Err(reason) = graph.release(&mut selected, &mut instances, actor, now) {
            runtime.opportunities[index].state = OpportunityState::Refused {
                decision_tick: tick,
                tick: now,
                reason,
            };
            continue;
        }
        let mut definitions = runtime.situation_definitions.clone();
        for definition in graph.definitions.values() {
            if definitions
                .iter()
                .any(|existing| existing.id == definition.id)
            {
                return Err("duplicate optional Scenario definition".into());
            }
            definitions.push(definition.clone());
        }
        runtime.situations = instances;
        runtime.commitments = commitments;
        runtime.situation_definitions = definitions;
        runtime.opportunities[index].state = OpportunityState::Accepted {
            decision_tick: tick,
            input,
            choice: selected,
        };
    }
    if runtime.opportunities.len() >= 8 {
        return Ok(None);
    }
    let Some(settled) = settled_frontier(runtime, base, source, now)? else {
        return Ok(None);
    };
    if settled >= now {
        return Ok(None);
    }
    let mut sorted = inputs.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.compartment.cmp(&right.compartment));
    if sorted.len() > 16 {
        return Err("maintenance discovery candidate bound exceeded".into());
    }
    for input in sorted {
        if runtime.opportunities.iter().any(|row| row.input == *input) {
            continue;
        }
        let Some(proposal) = source.search(input)? else {
            continue;
        };
        let reference = OpportunityRef {
            source: source.config.id.clone(),
            ordinal: runtime.opportunities.len() as u16 + 1,
            actor: actor.into(),
        };
        // Reserve the same global history capacity before making any offer.
        if runtime.situations.instances.len() + proposal.steps.len()
            > phoenix_sim_world::world::activities::MAX_SITUATIONS
        {
            return Ok(None);
        }
        runtime.opportunities.push(OpportunityRecord {
            reference: reference.clone(),
            discovered_tick: now,
            input: input.clone(),
            proposal,
            state: OpportunityState::Offered,
        });
        return Ok(Some(reference));
    }
    Ok(None)
}
