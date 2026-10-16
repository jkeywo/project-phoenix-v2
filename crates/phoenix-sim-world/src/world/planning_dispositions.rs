//! Bounded nonphysical monitoring of frozen Repair-Structure leaves.
use super::*;
use crate::world::activities::{ActivityRequest, SituationInstances};
use phoenix_model::{
    activity::RepairGoal,
    damage::RepairWorkTarget,
    observation::{ObservationSource, ObservationSubject, ObservationUncertainty},
};
impl Decomposition {
    fn validate_reading(
        &self,
        choice: &PlanChoice,
        node: &str,
        reading: &PlanNodeReading,
        now: u64,
    ) -> Result<(), String> {
        let leaf = self.leaves.get(node).ok_or("unknown disposition node")?;
        let ActivityRequest::Repair {
            owner,
            target: RepairWorkTarget::Compartment { .. },
            goal: RepairGoal::Structure,
            ..
        } = &leaf.request
        else {
            return Err("node dispositions support only compartment Structure repair".into());
        };
        let observation = &reading.observation;
        let tick = reading.tick().ok_or("node reading lacks receipt clock")?;
        if !observation.is_valid()
            || observation.provenance.kind != ObservationSource::Evidence
            || observation.provenance.system.as_ref() != Some(owner)
            || !observation.provenance.label.is_empty()
            || !matches!(observation.subject, ObservationSubject::System { .. })
            || observation.observed_tick != Some(tick)
            || tick < choice.chosen_tick
            || tick > now
            || observation.uncertainty != ObservationUncertainty::Qualitative
            || !reading.hp.is_finite()
            || !reading.max_hp.is_finite()
            || reading.max_hp <= 0.0
            || reading.hp < 0.0
            || reading.hp > reading.max_hp
            || reading.required_parts <= 0
            || reading.local_available.is_some_and(|amount| amount < 0)
        {
            return Err("invalid own-owner node observation or original clock".into());
        }
        let valid = match reading.reason {
            PlanNodeReason::RestoredExternally => reading.hp == reading.max_hp,
            PlanNodeReason::WorkInProgress => {
                reading.hp < reading.max_hp
                    && reading.other_work.as_ref().is_some_and(|work| {
                        work.id.0 > 0
                            && work.owner == *owner
                            && work.outcome.is_none()
                            && work.target_entity.is_none()
                            && matches!(
                                work.phase,
                                phoenix_model::job::JobPhase::Travelling
                                    | phoenix_model::job::JobPhase::Working
                            )
                    })
            }
            PlanNodeReason::PartsUnavailable => {
                reading.hp < reading.max_hp
                    && reading.other_work.is_none()
                    && reading
                        .local_available
                        .is_some_and(|amount| amount < reading.required_parts)
            }
            PlanNodeReason::RouteUnavailable => {
                reading.hp < reading.max_hp
                    && reading.other_work.is_none()
                    && reading
                        .local_available
                        .is_some_and(|amount| amount >= reading.required_parts)
                    && reading.route_available == Some(false)
            }
            PlanNodeReason::Available => {
                reading.hp < reading.max_hp
                    && reading.other_work.is_none()
                    && reading
                        .local_available
                        .is_some_and(|amount| amount >= reading.required_parts)
                    && reading.route_available == Some(true)
            }
        };
        if !valid {
            return Err("node reason disagrees with its retained permitted observation".into());
        }
        Ok(())
    }
    pub(super) fn validate_dispositions(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<(), String> {
        if choice.node_dispositions.len() > self.leaves.len()
            || (!choice.node_dispositions.is_empty() && choice.execution.is_primary())
        {
            return Err("unsupported node disposition execution".into());
        }
        for (node, record) in &choice.node_dispositions {
            self.validate_reading(choice, node, &record.first, now)?;
            self.validate_reading(choice, node, &record.latest, now)?;
            if record.first.reason == PlanNodeReason::Available
                || record.first.tick() > record.latest.tick()
                || record.first.terminal() && record.first != record.latest
                || record.latest.reason == PlanNodeReason::Available && !record.first.blocked()
                || self.dependencies.iter().any(|edge| &edge.from == node)
                || instances
                    .instances
                    .get(&self.leaves[node].id)
                    .is_some_and(|instance| {
                        instance.accepted_tick.is_some_and(|tick| {
                            record.latest.withheld() || tick < record.latest.tick().unwrap()
                        })
                    })
            {
                return Err(
                    "node disposition changes accepted work, a successor, or historical monitoring"
                        .into(),
                );
            }
            if let Some(receipt) = &record.consequence {
                if !record.latest.terminal()
                    || receipt.tick < record.latest.tick().unwrap()
                    || receipt.tick > now
                {
                    return Err("invalid changed-work consequence receipt".into());
                }
            }
        }
        Ok(())
    }
    /// Called only with an actual permitted owner reading. Never manufactures a
    /// Situation, Job, physical fact, release clock or successful consequence.
    pub fn observe_node(
        &self,
        choice: &mut PlanChoice,
        instances: &SituationInstances,
        node: &str,
        reading: PlanNodeReading,
        now: u64,
    ) -> Result<bool, String> {
        self.validate_history(choice, instances, now)?;
        self.validate_reading(choice, node, &reading, now)?;
        if choice.execution.is_primary()
            || self.dependencies.iter().any(|edge| edge.from == node)
            || instances
                .instances
                .get(&self.leaves[node].id)
                .is_some_and(|instance| instance.accepted.is_some())
        {
            return Err("monitor cannot replace accepted or unsupported future work".into());
        }
        if choice
            .node_dispositions
            .get(node)
            .is_some_and(|record| record.latest.terminal())
        {
            return Ok(false);
        }
        if reading.reason == PlanNodeReason::Available
            && !choice.node_dispositions.contains_key(node)
        {
            return Ok(false);
        }
        let mut candidate = choice.clone();
        if let Some(record) = candidate.node_dispositions.get_mut(node) {
            if record.latest.reason == reading.reason {
                return Ok(false);
            }
            record.latest = reading;
        } else {
            candidate.node_dispositions.insert(
                node.into(),
                PlanNodeDisposition {
                    first: reading.clone(),
                    latest: reading,
                    consequence: None,
                },
            );
        }
        self.validate_history(&candidate, instances, now)?;
        *choice = candidate;
        Ok(true)
    }
    pub fn finish_node_consequence(
        &self,
        choice: &mut PlanChoice,
        instances: &SituationInstances,
        node: &str,
        result: crate::world::activities::ScenarioCallResult,
        now: u64,
    ) -> Result<(), String> {
        self.validate_history(choice, instances, now)?;
        let row = choice
            .node_dispositions
            .get_mut(node)
            .ok_or("missing changed-work node")?;
        if !row.latest.terminal() || row.consequence.is_some() {
            return Err("changed-work callback is not pending".into());
        }
        row.consequence = Some(PlanNodeCallReceipt { tick: now, result });
        Ok(())
    }
}
impl PlanNodeReading {
    fn withheld(&self) -> bool {
        self.blocked() || self.terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::planning::opportunities::*;
    use phoenix_model::{identity::SystemId, messages::RepairTarget};
    fn fixture() -> (Decomposition, PlanChoice, SituationInstances) {
        fixture_supply(false)
    }
    fn fixture_supply(missing: bool) -> (Decomposition, PlanChoice, SituationInstances) {
        let world = crate::world::config::parse_world(include_str!(
            "../../../../tests/fixtures/optional-maintenance-world.toml"
        ))
        .unwrap();
        let (source, _) =
            CompiledMaintenanceSearch::load(world.opportunity_search.unwrap(), |_| {
                Some(
                    include_str!("../../../../tests/fixtures/maintenance-opportunity-domain.toml")
                        .into(),
                )
            })
            .unwrap();
        let input = MaintenanceOpportunityInput {
            compartment: "engineering".into(),
            hp: 90.0,
            max_hp: 100.0,
            remedy_hp: 10.0,
            required_parts: 1,
            local_available: if missing { 0 } else { 1 },
            local_headroom: 1,
            source_available_above_floor: 1,
            known: true,
            report: Some(
                phoenix_model::observation::ObservationMetadata::at(
                    1,
                    ObservationSubject::System {
                        id: SystemId("power".into()),
                    },
                    phoenix_model::observation::ObservationProvenance {
                        kind: ObservationSource::RepairTeam,
                        system: Some(source.config.repair_owner.clone()),
                        label: String::new(),
                    },
                    ObservationUncertainty::Qualitative,
                )
                .received(1),
            ),
            repair_idle: true,
            handling_idle: true,
            route_available: true,
            outstanding_supply: false,
        };
        let proposal = source.search(&input).unwrap().unwrap();
        let reference = phoenix_model::comms_request::OpportunityRef {
            source: source.config.id.clone(),
            ordinal: 1,
            actor: "actual-actor".into(),
        };
        let graph = source.derive(&reference, &input, &proposal).unwrap();
        let choice = PlanChoice {
            id: source.config.id.clone(),
            execution: PlanExecution::Optional {
                source: source.config.id.clone(),
                ordinal: 1,
            },
            required_commitment: Some("opportunity.maintenance.1".into()),
            root_arguments: Default::default(),
            chosen_tick: 10,
            actor_uuid: reference.actor,
            released: Default::default(),
            methods: Default::default(),
            refused_methods: Vec::new(),
            temporal: None,
            temporal_reconsiderations: Vec::new(),
            node_dispositions: Default::default(),
        };
        (graph, choice, SituationInstances::default())
    }
    fn reading(reason: PlanNodeReason, tick: u64) -> PlanNodeReading {
        PlanNodeReading {
            observation: phoenix_model::observation::ObservationMetadata::at(
                tick,
                ObservationSubject::System {
                    id: SystemId("power".into()),
                },
                phoenix_model::observation::ObservationProvenance {
                    kind: ObservationSource::Evidence,
                    system: Some(SystemId("repair".into())),
                    label: String::new(),
                },
                ObservationUncertainty::Qualitative,
            )
            .received(tick),
            hp: if reason == PlanNodeReason::RestoredExternally {
                100.0
            } else {
                90.0
            },
            max_hp: 100.0,
            local_available: Some(if reason == PlanNodeReason::PartsUnavailable {
                0
            } else {
                1
            }),
            required_parts: 1,
            route_available: Some(reason != PlanNodeReason::RouteUnavailable),
            other_work: None,
            reason,
        }
    }
    #[test]
    fn bounded_monitoring_clears_without_reserving_or_restarting_original_work() {
        let (graph, mut choice, mut instances) = fixture();
        let original = (
            choice.chosen_tick,
            choice.methods.clone(),
            choice.actor_uuid.clone(),
        );
        assert!(graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::PartsUnavailable, 11),
                11
            )
            .unwrap());
        assert!(graph
            .release(&mut choice, &mut instances, "actual-actor", 12)
            .unwrap()
            .is_empty());
        assert!(instances.is_empty());
        assert!(!graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::PartsUnavailable, 13),
                13
            )
            .unwrap());
        assert!(graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::Available, 14),
                14
            )
            .unwrap());
        assert_eq!(choice.node_dispositions["step-0"].first.tick(), Some(11));
        assert_eq!(choice.node_dispositions["step-0"].latest.tick(), Some(14));
        assert_eq!(
            graph
                .release(&mut choice, &mut instances, "actual-actor", 15)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            original,
            (
                choice.chosen_tick,
                choice.methods.clone(),
                choice.actor_uuid.clone()
            )
        );
        let request = RepairTarget::Work(RepairWorkTarget::Compartment {
            compartment: "engineering".into(),
        });
        assert!(instances
            .accept_repair(
                "actual-actor",
                &SystemId("repair".into()),
                0,
                &request,
                phoenix_model::job::JobId(1),
                16
            )
            .unwrap()
            .is_some());
        let before = choice.clone();
        assert!(graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::RestoredExternally, 17),
                17
            )
            .is_err());
        assert_eq!(
            choice, before,
            "accepted jobs never turn into externally changed work"
        );
        graph.validate_history(&choice, &instances, 17).unwrap();
    }
    #[test]
    fn terminal_change_is_immutable_and_never_manufactures_acceptance_or_achievement() {
        let (graph, mut choice, mut instances) = fixture();
        graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::RestoredExternally, 11),
                11,
            )
            .unwrap();
        let before = choice.clone();
        assert!(!graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::PartsUnavailable, 12),
                12
            )
            .unwrap());
        assert_eq!(before, choice);
        assert!(graph
            .release(&mut choice, &mut instances, "actual-actor", 12)
            .unwrap()
            .is_empty());
        assert!(graph
            .ready_requests(&choice, &instances, 12)
            .unwrap()
            .is_empty());
        assert_eq!(graph.settled_tick(&choice, &instances, 12).unwrap(), None);
        assert!(instances.is_empty());
        graph
            .finish_node_consequence(
                &mut choice,
                &instances,
                "step-0",
                crate::world::activities::ScenarioCallResult::Applied,
                13,
            )
            .unwrap();
        assert_eq!(
            graph.settled_tick(&choice, &instances, 13).unwrap(),
            Some(13)
        );
        assert!(
            instances.is_empty(),
            "application closes nonphysical history, not a fake Job"
        );
        assert!(graph
            .finish_node_consequence(
                &mut choice,
                &instances,
                "step-0",
                crate::world::activities::ScenarioCallResult::Applied,
                14
            )
            .is_err());
        let intent = graph.intent_requests(&choice, &instances, 14).unwrap();
        assert_eq!(
            intent[0].disposition.as_ref().unwrap().consequence_applied,
            Some(true)
        );
    }
    #[test]
    fn malformed_monitoring_refuses_before_mutating_frozen_history() {
        let (graph, mut choice, instances) = fixture();
        for fault in 0..6 {
            let mut observation = reading(PlanNodeReason::RestoredExternally, 11);
            match fault {
                0 => observation.hp = 99.0,
                1 => observation.max_hp = f64::NAN,
                2 => observation.observation.provenance.system = Some(SystemId("foreign".into())),
                3 => observation.observation.observed_tick = Some(12),
                4 => observation.observation.received_tick = Some(9),
                _ => observation.observation.provenance.kind = ObservationSource::RepairDiagnosis,
            };
            let before = choice.clone();
            assert!(
                graph
                    .observe_node(&mut choice, &instances, "step-0", observation, 11)
                    .is_err(),
                "fault{fault}"
            );
            assert_eq!(before, choice);
        }
    }
    #[test]
    fn changed_final_leaf_does_not_release_prior_physical_assignment_or_invent_delivery() {
        let (graph, mut choice, mut instances) = fixture_supply(true);
        graph
            .release(&mut choice, &mut instances, "actual-actor", 10)
            .unwrap();
        graph
            .observe_node(
                &mut choice,
                &instances,
                "step-1",
                reading(PlanNodeReason::RestoredExternally, 11),
                11,
            )
            .unwrap();
        graph
            .finish_node_consequence(
                &mut choice,
                &instances,
                "step-1",
                crate::world::activities::ScenarioCallResult::Applied,
                12,
            )
            .unwrap();
        assert_eq!(graph.settled_tick(&choice, &instances, 12).unwrap(), None);
        let ActivityRequest::Delivery {
            owner,
            source,
            destination,
            commodity,
            amount,
            ..
        } = graph.leaves["step-0"].request.clone()
        else {
            unreachable!()
        };
        instances
            .accept_delivery(
                "actual-actor",
                &owner,
                &source,
                &destination,
                &commodity,
                amount,
                phoenix_model::job::JobId(1),
                13,
            )
            .unwrap();
        assert_eq!(
            graph.settled_tick(&choice, &instances, 13).unwrap(),
            None,
            "actual unfinished prior owner remains committed"
        );
        let scope = instances
            .instances
            .values()
            .next()
            .unwrap()
            .accepted
            .clone()
            .unwrap();
        let fact = instances
            .fact(
                &scope,
                14,
                phoenix_model::activity::ActivityOutcome::Delivery {
                    source,
                    destination,
                    commodity,
                    amount,
                    returned: false,
                    outcome: phoenix_model::job::JobOutcome::Completed,
                },
            )
            .unwrap();
        instances.acknowledge(&fact, 14).unwrap();
        assert_eq!(
            graph.settled_tick(&choice, &instances, 14).unwrap(),
            None,
            "actual Scenario callback still must close"
        );
        let definitions = graph.definitions.values().cloned().collect::<Vec<_>>();
        for activation in instances.ready(&definitions) {
            instances
                .finish_activation(
                    &activation,
                    crate::world::activities::ScenarioCallResult::Applied,
                    15,
                    &definitions,
                )
                .unwrap();
        }
        assert_eq!(
            graph.settled_tick(&choice, &instances, 15).unwrap(),
            Some(15)
        );
        assert_eq!(
            instances.instances.len(),
            1,
            "terminal future Repair never becomes a fake physical leaf"
        );
    }
    #[test]
    fn withheld_released_leaf_does_not_claim_normal_manual_override() {
        let (graph, mut choice, mut instances) = fixture();
        graph
            .release(&mut choice, &mut instances, "actual-actor", 10)
            .unwrap();
        graph
            .observe_node(
                &mut choice,
                &instances,
                "step-0",
                reading(PlanNodeReason::PartsUnavailable, 11),
                11,
            )
            .unwrap();
        let excluded = std::collections::BTreeSet::from([graph.leaves["step-0"].id.clone()]);
        let request = RepairTarget::Work(RepairWorkTarget::Compartment {
            compartment: "engineering".into(),
        });
        assert!(instances
            .accept_repair_excluding(
                "actual-actor",
                &SystemId("repair".into()),
                0,
                &request,
                phoenix_model::job::JobId(1),
                12,
                &excluded
            )
            .unwrap()
            .is_none());
        assert!(instances
            .instances
            .values()
            .all(|row| row.accepted.is_none()));
        graph.validate_history(&choice, &instances, 12).unwrap();
    }
}
