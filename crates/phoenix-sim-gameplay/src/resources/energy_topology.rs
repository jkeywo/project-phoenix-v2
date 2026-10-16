//! Derived indices for one immutable Power definition. Not simulation state.
//! Raw Network APIs retain their full checks and scan-based reference path.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompiledEnergyDefinition {
    config: EnergyConfigSection,
    ship: ShipConfig,
    pub(super) group_indices: BTreeMap<PowerGroupId, usize>,
    pub(super) consumer_indices: BTreeMap<SystemId, usize>,
    pub(super) roots: Vec<PowerGroupId>,
    pub(super) children: Vec<Vec<PowerGroupId>>,
    pub(super) group_consumers: Vec<Vec<usize>>,
    consumer_groups: Vec<Option<usize>>,
    pub(super) consumer_paths: Vec<Vec<usize>>,
}

impl CompiledEnergyDefinition {
    pub(crate) fn compile(
        config: EnergyConfigSection,
        ship: ShipConfig,
        state: &EnergyNetworkState,
    ) -> Result<Self, String> {
        // No trusted constructor from raw definitions: this includes hierarchy,
        // declaration and cost validation before any index can be consulted.
        state.validate_against(&config, &ship)?;
        let group_indices: BTreeMap<_, _> = config
            .groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.id.clone(), index))
            .collect();
        let consumer_indices = config
            .consumers
            .iter()
            .enumerate()
            .map(|(index, consumer)| (consumer.system.clone(), index))
            .collect();
        let roots = config
            .groups
            .iter()
            .filter(|group| group.parent.is_none())
            .map(|group| group.id.clone())
            .collect();
        let mut children = vec![Vec::new(); config.groups.len()];
        for group in &config.groups {
            if let Some(parent) = &group.parent {
                children[group_indices[parent]].push(group.id.clone());
            }
        }
        let mut group_consumers = Vec::new();
        for group in &config.groups {
            // Preserve raw routing's `any`, including its treatment of repeated
            // System rows. Priority/fine allocation below use the original first
            // matching System instead. Accepted content normally has unique IDs.
            let mut consumers: Vec<_> = config
                .consumers
                .iter()
                .enumerate()
                .filter(|(_, consumer)| {
                    ship.systems.iter().any(|system| {
                        system.id == consumer.system
                            && system.power_group.as_ref() == Some(&group.id)
                    })
                })
                .map(|(index, _)| index)
                .collect();
            consumers.sort_by(|a, b| {
                config.consumers[*a]
                    .system
                    .cmp(&config.consumers[*b].system)
            });
            group_consumers.push(consumers);
        }
        let consumer_groups: Vec<_> = config
            .consumers
            .iter()
            .map(|consumer| {
                ship.systems
                    .iter()
                    .find(|system| system.id == consumer.system)
                    .and_then(|system| system.power_group.as_ref())
                    .map(|id| group_indices[id])
            })
            .collect();
        let consumer_paths = consumer_groups
            .iter()
            .map(|group| {
                let mut path = Vec::new();
                let mut current = *group;
                while let Some(index) = current {
                    path.push(index);
                    current = config.groups[index]
                        .parent
                        .as_ref()
                        .map(|id| group_indices[id]);
                }
                path.reverse();
                path
            })
            .collect();
        Ok(Self {
            config,
            ship,
            group_indices,
            consumer_indices,
            roots,
            children,
            group_consumers,
            consumer_groups,
            consumer_paths,
        })
    }

    pub(crate) fn config(&self) -> &EnergyConfigSection {
        &self.config
    }
    pub(crate) fn ship(&self) -> &ShipConfig {
        &self.ship
    }

    /// Every mutable constraint remains checked on both sides of a tick. Only
    /// immutable declarations/cycles/costs were discharged by compile(). Reserve
    /// validation still uses the unchanged EnergyState API, including its config
    /// checks; this optimization is limited to network topology.
    pub(super) fn validate_state(&self, state: &EnergyNetworkState) -> Result<(), String> {
        let config = &self.config;
        state.reactor.validate_against(&config.reactor.reserve)?;
        bounded(
            "output target",
            state.output_target_per_sec,
            config.reactor.max_output_per_sec,
        )?;
        bounded(
            "output actual",
            state.output_actual_per_sec,
            config.reactor.max_output_per_sec,
        )?;
        bounded(
            "reactor cooling",
            state.reactor_cooling_per_sec,
            config.reactor.max_cooling_per_sec,
        )?;
        if state.reactor.thermal_stopped && state.output_actual_per_sec != 0.0 {
            return Err("thermally stopped reactor cannot report actual output".into());
        }
        if state.groups.keys().ne(self.group_indices.keys())
            || state.consumers.keys().ne(self.consumer_indices.keys())
        {
            return Err("energy state does not match authored group/consumer identities".into());
        }
        for (index, group) in config.groups.iter().enumerate() {
            let current = &state.groups[&group.id];
            bounded(
                "desired flow",
                current.desired_flow_per_sec,
                group.max_flow_per_sec,
            )?;
            bounded(
                "effective flow",
                current.effective_flow_per_sec,
                group.max_flow_per_sec,
            )?;
            bounded(
                "desired cooling",
                current.desired_cooling_per_sec,
                group.max_cooling_per_sec,
            )?;
            bounded(
                "effective cooling",
                current.effective_cooling_per_sec,
                group.max_cooling_per_sec,
            )?;
            bounded(
                "restore readiness",
                current.restore_ready_secs,
                group.restore_delay_secs,
            )?;
            if current.cut && current.effective_flow_per_sec != 0.0 {
                return Err("cut group cannot have effective incoming flow".into());
            }
            let child_flow = self.children[index]
                .iter()
                .map(|id| state.groups[id].effective_flow_per_sec as f64)
                .sum::<f64>();
            let child_cooling = self.children[index]
                .iter()
                .map(|id| state.groups[id].effective_cooling_per_sec as f64)
                .sum::<f64>();
            if child_flow > current.effective_flow_per_sec as f64
                || child_cooling > current.effective_cooling_per_sec as f64
            {
                return Err("saved child allocations exceed parent budget".into());
            }
        }
        for consumer in &config.consumers {
            state.consumers[&consumer.system].validate_against(&consumer.reserve)?;
        }
        for (system, flow) in &state.fine_flows {
            let index = *self
                .consumer_indices
                .get(system)
                .ok_or("fine flow has unknown consumer")?;
            let consumer = &config.consumers[index];
            bounded("fine flow", *flow, consumer.reserve.max_intake_per_sec)?;
            let group =
                self.consumer_groups[index].ok_or("fine flow consumer has no allocation group")?;
            bounded("fine flow", *flow, config.groups[group].max_flow_per_sec)?;
        }
        Ok(())
    }
}
