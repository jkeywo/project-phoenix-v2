//! World-less fleet staging. One owner allocates generations and carries
//! adoption, control projections and completion through their transitions.
use super::{
    queue_fleet_lobby_input_bounded, rebind_fleet_lobby_projections, FleetLobbyInput,
    PendingFleetAdoption, PendingFleetJoin, PendingFleetLobbyInput, MAX_FLEET_LOBBY_INPUTS,
};
use crate::lockstep::{FleetJoinStatus, FleetJoinStatusKind};
use std::collections::VecDeque;

pub(super) struct FleetStaging {
    pub generation: u64,
    pub status: FleetJoinStatus,
    pub adoptions: VecDeque<PendingFleetAdoption>,
    inputs: VecDeque<PendingFleetLobbyInput>,
    pub managed: Option<bool>,
    pub validation: Option<bool>,
}

impl Default for FleetStaging {
    fn default() -> Self {
        Self {
            generation: 0,
            status: FleetJoinStatus {
                generation: 0,
                status: FleetJoinStatusKind::Idle,
                reason: None,
            },
            adoptions: VecDeque::new(),
            inputs: VecDeque::new(),
            managed: None,
            validation: None,
        }
    }
}

impl FleetStaging {
    fn advance(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.status = FleetJoinStatus {
            generation: self.generation,
            status: FleetJoinStatusKind::Pending,
            reason: None,
        };
    }

    pub fn join(&mut self, roster_json: &str) -> String {
        self.advance();
        if matches!(self.adoptions.back(), Some(PendingFleetAdoption::Join(_))) {
            self.adoptions.pop_back();
        }
        if crate::core::codec::decode_fleet_roster(roster_json).is_none() {
            self.complete(self.generation, false, "fleet-roster-unreadable");
            return "fleet-roster-unreadable".to_string();
        }
        rebind_fleet_lobby_projections(
            &mut self.inputs,
            self.generation,
            self.managed,
            self.validation,
        );
        self.adoptions
            .push_back(PendingFleetAdoption::Join(PendingFleetJoin {
                generation: self.generation,
                roster_json: roster_json.to_string(),
            }));
        self.generation.to_string()
    }

    pub fn leave(&mut self) -> String {
        self.advance();
        self.adoptions.clear();
        self.adoptions.push_back(PendingFleetAdoption::Leave {
            generation: self.generation,
        });
        self.generation.to_string()
    }

    pub fn queue(&mut self, input: FleetLobbyInput) -> bool {
        queue_fleet_lobby_input_bounded(
            &mut self.inputs,
            self.generation,
            input,
            MAX_FLEET_LOBBY_INPUTS,
        )
    }

    pub fn complete(&mut self, generation: u64, accepted: bool, refusal: &str) {
        if self.status.generation == generation {
            self.status = FleetJoinStatus {
                generation,
                status: if accepted {
                    FleetJoinStatusKind::Accepted
                } else {
                    FleetJoinStatusKind::Refused
                },
                reason: (!accepted).then(|| refusal.to_string()),
            };
        }
    }

    pub fn retry(&mut self, generation: u64, inputs: VecDeque<FleetLobbyInput>) {
        for input in inputs.into_iter().rev() {
            self.inputs
                .push_front(PendingFleetLobbyInput { generation, input });
        }
    }

    pub fn take_inputs(&mut self, adoption: &FleetJoinStatus) -> Option<VecDeque<FleetLobbyInput>> {
        let inputs = std::mem::take(&mut self.inputs)
            .into_iter()
            .filter(|row| row.generation == adoption.generation)
            .map(|row| row.input)
            .collect();
        match adoption.status {
            FleetJoinStatusKind::Pending => {
                self.retry(adoption.generation, inputs);
                None
            }
            FleetJoinStatusKind::Refused => None,
            _ => Some(inputs),
        }
    }
}

#[cfg(test)]
#[path = "fleet_staging_tests.rs"]
mod tests;
