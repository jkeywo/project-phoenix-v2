//! Read-only settled-interval measurements, private to one disposable child.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReferenceProfile {
    pub workload: Workload,
    pub warmup_intervals: u32,
    pub phases: Vec<ReferencePhase>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReferencePhase {
    pub activity: Activity,
    pub intervals: u32,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Activity {
    Engagement,
    Recovery,
}
impl ReferenceProfile {
    pub fn validate(&self, workload: Workload) -> Result<(), String> {
        let total: u64 = self.phases.iter().map(|p| u64::from(p.intervals)).sum();
        if self.workload != workload
            || self.warmup_intervals > 600
            || self.phases.is_empty()
            || self.phases.len() > 8
            || self.phases.iter().any(|p| p.intervals == 0)
            || total > 3600
            || total + u64::from(self.warmup_intervals) > 4200
            || !self
                .phases
                .iter()
                .any(|p| p.activity == Activity::Engagement)
            || (workload == Workload::BurstRecovery
                && self
                    .phases
                    .last()
                    .is_none_or(|p| p.activity != Activity::Recovery))
        {
            return Err("Reference workload has unsupported phases or interval bounds".into());
        }
        Ok(())
    }
    fn activity(&self, measured: u32) -> Option<Activity> {
        let mut end = 0;
        for phase in &self.phases {
            end += phase.intervals;
            if measured < end {
                return Some(phase.activity);
            }
        }
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BufferMeasurement {
    pub initial_energy: f64,
    pub final_energy: f64,
    pub minimum_energy: f64,
    pub maximum_heat: f64,
    pub thermal_trips: u32,
    pub thermal_recoveries: u32,
    pub paid_energy: f64,
    pub active_intervals: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChartSample {
    pub tick: u64,
    pub measured_intervals: u32,
    pub reserve: f64,
    pub maximum_heat: f64,
    pub generated_energy: f64,
    pub paid_energy: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RehearsalResult {
    pub revision: String,
    pub hull_content_id: String,
    pub workload_revision: String,
    pub run_generation: u64,
    pub selection: super::super::test_protocol::TestSelection,
    pub choices: BTreeMap<String, String>,
    pub profile: ReferenceProfile,
    pub fixed_period_secs: f64,
    pub initialized_tick: u64,
    pub last_measured_tick: Option<u64>,
    pub measured_intervals: u32,
    pub complete: bool,
    pub insufficient_activity: bool,
    pub error: Option<String>,
    pub buffers: BTreeMap<String, BufferMeasurement>,
    pub authored_initial_power: crate::modifiers::power_system::PowerState,
    pub nominal_equipment: BTreeMap<String, phoenix_model::fitting::EquipmentHardware>,
    pub initial_coolant: BTreeMap<String, f64>,
    pub final_coolant: BTreeMap<String, f64>,
    pub authored_initial_coolant: BTreeMap<String, f64>,
    pub authored_initial_ammunition: BTreeMap<String, i64>,
    pub generated_energy: f64,
    pub flow_cut_intervals: u32,
    pub engagement_intervals: u32,
    pub achieved_engagement_intervals: u32,
    pub recovery_intervals: u32,
    pub charts: Vec<ChartSample>,
    pub subject_structural_hp_lost: f64,
    pub subject_retired_or_lost: bool,
}

#[derive(Resource)]
pub struct Measurements {
    pub result: RehearsalResult,
    elapsed: u32,
    last_sample: Option<u64>,
    thermal: BTreeMap<String, bool>,
    subject: Entity,
    previous_subject_hp: f64,
}

fn coolant(
    world: &World,
    entity: Entity,
    power: &ShipPowerSystem,
) -> Result<BTreeMap<String, f64>, String> {
    let services = world
        .get::<crate::resources::server::ResourceServices>(entity)
        .ok_or("Actual coolant service links are absent")?;
    let bindings = crate::ship::energy_runtime::coolant_bindings(&power.0, services)?;
    let infra = world
        .get::<crate::infrastructure::InfrastructureCondition>(entity)
        .ok_or("Actual coolant Stocks are absent")?;
    let state = power.0.capture_continuation();
    let mut out = BTreeMap::new();
    for id in std::iter::once(bindings.reactor).chain(bindings.consumers.into_values()) {
        let stock = infra
            .0
            .capacity_stock(&id)
            .ok_or("Actual coolant capacity is absent")?;
        out.insert(
            id.clone(),
            stock.level as f64 + state.coolant_credit.get(&id).copied().unwrap_or(0.0),
        );
    }
    Ok(out)
}

pub(super) fn initial(
    world: &World,
    entity: Entity,
    power: &ShipPowerSystem,
) -> Result<Measurements, String> {
    let input = world.resource::<PreparedRehearsal>();
    let state = power.0.capture_continuation();
    let profile = world
        .get::<CompiledShipFitting>(entity)
        .ok_or("Actual fitting profile is absent")?;
    let nominal_equipment = input
        .choices
        .iter()
        .map(|(slot, def)| {
            profile
                .0
                .equipment(def)
                .map(|equipment| (slot.clone(), equipment.hardware.clone()))
        })
        .collect::<Result<_, _>>()?;
    let mut subjects = world
        .try_query::<(
            Entity,
            &crate::entities::spawner::EntityId,
            &crate::ship::compartment_damage::ShipDamage,
        )>()
        .ok_or("Reference subject schema is absent")?;
    let subjects: Vec<_> = subjects
        .iter(world)
        .filter(|(_, id, _)| id.0 == "rehearsal-subject")
        .map(|(entity, _, damage)| (entity, damage.structural_totals().0))
        .collect();
    let [(subject, subject_hp)] = subjects.as_slice() else {
        return Err("Reference requires exactly one actual finite subject".into());
    };
    let buffers: BTreeMap<_, _> = std::iter::once(("reactor".to_string(), state.network.reactor))
        .chain(
            state
                .network
                .consumers
                .iter()
                .map(|(id, b)| (id.0.clone(), *b)),
        )
        .map(|(id, b)| {
            (
                id,
                BufferMeasurement {
                    initial_energy: f64::from(b.energy),
                    final_energy: f64::from(b.energy),
                    minimum_energy: f64::from(b.energy),
                    maximum_heat: f64::from(b.heat),
                    thermal_trips: 0,
                    thermal_recoveries: 0,
                    paid_energy: 0.0,
                    active_intervals: 0,
                },
            )
        })
        .collect();
    let thermal = std::iter::once(("reactor".to_string(), state.network.reactor.thermal_stopped))
        .chain(
            state
                .network
                .consumers
                .iter()
                .map(|(id, b)| (id.0.clone(), b.thermal_stopped)),
        )
        .collect();
    let coolant = coolant(world, entity, power)?;
    let mut ammunition = BTreeMap::new();
    if let Some(torpedoes) = world.get::<crate::console::weapons::TorpedoSystemResource>(entity) {
        ammunition.insert("torpedo-magazine".into(), torpedoes.0.magazine.level);
        for tube in &torpedoes.0.tubes {
            ammunition.insert(
                format!("torpedo-tube/{}", tube.id.0),
                i64::from(tube.loaded_count),
            );
        }
    }
    Ok(Measurements {
        result: RehearsalResult {
            revision: input.revision.clone(),
            hull_content_id: input.request.plan.content_id.clone(),
            workload_revision: input.workload_revision.clone(),
            run_generation: input.run_generation,
            selection: input.selection.clone(),
            choices: input.choices.clone(),
            profile: input.profile.clone(),
            fixed_period_secs: world.resource::<Time<Fixed>>().timestep().as_secs_f64(),
            initialized_tick: world.resource::<crate::sim_tick::SimTick>().0,
            last_measured_tick: None,
            measured_intervals: 0,
            complete: false,
            insufficient_activity: false,
            error: None,
            buffers,
            authored_initial_power: state,
            nominal_equipment,
            initial_coolant: coolant.clone(),
            authored_initial_coolant: coolant.clone(),
            authored_initial_ammunition: ammunition,
            final_coolant: coolant,
            generated_energy: 0.0,
            flow_cut_intervals: 0,
            engagement_intervals: 0,
            achieved_engagement_intervals: 0,
            recovery_intervals: 0,
            charts: Vec::new(),
            subject_structural_hp_lost: 0.0,
            subject_retired_or_lost: false,
        },
        elapsed: 0,
        last_sample: None,
        thermal,
        subject: *subject,
        previous_subject_hp: *subject_hp,
    })
}

fn settled(world: &World) -> bool {
    world.get_resource::<crate::effect_queue::EffectQueue<crate::infrastructure::CapacityAdjustment>>()
        .is_some_and(|queue| queue.0.is_empty())
}

pub(super) fn sample(world: &mut World) {
    let Some(input) = world.get_resource::<PreparedRehearsal>() else {
        return;
    };
    let Some(entity) = input.target else {
        return;
    };
    let tick = world.resource::<crate::sim_tick::SimTick>().0;
    let Some(mut samples) = world.remove_resource::<Measurements>() else {
        return;
    };
    let active = world
        .get_resource::<State<crate::core::messages::GamePhase>>()
        .is_some_and(|phase| *phase.get() == crate::core::messages::GamePhase::InProgress)
        && !world
            .resource::<crate::gm::actions::action::SimulationPaused>()
            .0;
    if !samples.result.complete
        && samples.result.error.is_none()
        && world
            .get_resource::<State<crate::core::messages::GamePhase>>()
            .is_some_and(|phase| *phase.get() == crate::core::messages::GamePhase::GameOver)
    {
        samples.result.error = Some("Reference ended before the requested window completed".into());
        world
            .resource_mut::<super::super::test_clock::TestControls>()
            .0
            .push_back(super::super::test_protocol::TestControl::Pause {});
    }
    if samples.result.complete
        || samples.result.error.is_some()
        || !active
        || tick <= samples.result.initialized_tick
        || samples.last_sample == Some(tick)
    {
        world.insert_resource(samples);
        return;
    }
    samples.last_sample = Some(tick);
    let observed = observe(world, entity, &mut samples, tick);
    if let Err(error) = observed {
        samples.result.error = Some(error);
        if samples.result.error.as_deref() == Some("result-too-large") {
            samples.result.charts.clear();
        }
    }
    if samples.result.complete || samples.result.error.is_some() {
        // The next real frame consumes this ordinary private clock request;
        // remaining ticks in a multi-step frame cannot extend the frozen result.
        world
            .resource_mut::<super::super::test_clock::TestControls>()
            .0
            .push_back(super::super::test_protocol::TestControl::Pause {});
    }
    world.insert_resource(samples);
}

fn observe(
    world: &World,
    entity: Entity,
    samples: &mut Measurements,
    tick: u64,
) -> Result<(), String> {
    if !settled(world) {
        return Err("Reference result refused: resource effects are not settled".into());
    }
    let power = world
        .get::<ShipPowerSystem>(entity)
        .ok_or("Reference ship was lost before the requested window completed")?;
    let period = world.resource::<Time<Fixed>>().timestep().as_secs_f64();
    if period != samples.result.fixed_period_secs {
        return Err("Reference fixed period changed during measurement".into());
    }
    samples.elapsed += 1;
    if let Some(damage) = world.get::<crate::ship::compartment_damage::ShipDamage>(samples.subject)
    {
        let hp = damage.structural_totals().0;
        if samples.elapsed > samples.result.profile.warmup_intervals {
            samples.result.subject_structural_hp_lost +=
                (samples.previous_subject_hp - hp).max(0.0);
        }
        samples.previous_subject_hp = hp;
    } else {
        samples.result.subject_retired_or_lost = true;
    }
    if samples.elapsed <= samples.result.profile.warmup_intervals {
        let state = power.0.capture_continuation();
        for (id, b) in std::iter::once(("reactor".to_string(), state.network.reactor)).chain(
            state
                .network
                .consumers
                .iter()
                .map(|(id, b)| (id.0.clone(), *b)),
        ) {
            let row = samples
                .result
                .buffers
                .get_mut(&id)
                .ok_or("Reference energy hardware changed")?;
            row.initial_energy = f64::from(b.energy);
            row.final_energy = f64::from(b.energy);
            row.minimum_energy = f64::from(b.energy);
            row.maximum_heat = f64::from(b.heat);
            samples.thermal.insert(id, b.thermal_stopped);
        }
        samples.result.initial_coolant = coolant(world, entity, power)?;
        samples.result.final_coolant = samples.result.initial_coolant.clone();
        return Ok(());
    }
    let index = samples.result.measured_intervals;
    let Some(activity) = samples.result.profile.activity(index) else {
        return Ok(());
    };
    let state = power.0.capture_continuation();
    let mut paid_total = 0.0;
    let mut reserve = 0.0;
    let mut max_heat: f64 = 0.0;
    for (id, b) in std::iter::once(("reactor".to_string(), state.network.reactor)).chain(
        state
            .network
            .consumers
            .iter()
            .map(|(id, b)| (id.0.clone(), *b)),
    ) {
        let row = samples
            .result
            .buffers
            .get_mut(&id)
            .ok_or("Reference energy hardware changed")?;
        let paid = if id == "reactor" {
            0.0
        } else {
            power
                .0
                .emitted_work(&crate::core::messages::SystemId(id.clone()))
        };
        row.final_energy = f64::from(b.energy);
        row.minimum_energy = row.minimum_energy.min(f64::from(b.energy));
        row.maximum_heat = row.maximum_heat.max(f64::from(b.heat));
        let was_stopped = samples
            .thermal
            .insert(id, b.thermal_stopped)
            .unwrap_or(false);
        row.thermal_trips += u32::from(!was_stopped && b.thermal_stopped);
        row.thermal_recoveries += u32::from(was_stopped && !b.thermal_stopped);
        row.paid_energy += paid;
        row.active_intervals += u32::from(paid > 0.0);
        paid_total += paid;
        reserve += f64::from(b.energy);
        max_heat = max_heat.max(f64::from(b.heat));
    }
    samples.result.generated_energy += f64::from(state.network.output_actual_per_sec) * period;
    samples.result.flow_cut_intervals += u32::from(state.network.groups.values().any(|g| g.cut));
    let beam_work = power
        .0
        .emitted_work(&crate::core::messages::SystemId("phaser-fore".into()))
        > 0.0;
    match activity {
        Activity::Engagement => {
            samples.result.engagement_intervals += 1;
            samples.result.achieved_engagement_intervals += u32::from(beam_work);
        }
        Activity::Recovery => samples.result.recovery_intervals += 1,
    }
    samples.result.final_coolant = coolant(world, entity, power)?;
    samples.result.measured_intervals += 1;
    samples.result.last_measured_tick = Some(tick);
    // Fixed stride gives <=121 samples for the bounded 3600-interval window,
    // plus the exact final boundary, without renderer-dependent downsampling.
    let complete = samples
        .result
        .profile
        .activity(samples.result.measured_intervals)
        .is_none();
    if index % 30 == 0 || complete {
        samples.result.charts.push(ChartSample {
            tick,
            measured_intervals: samples.result.measured_intervals,
            reserve,
            maximum_heat: max_heat,
            generated_energy: samples.result.generated_energy,
            paid_energy: paid_total,
        });
    }
    samples.result.complete = complete;
    samples.result.insufficient_activity =
        complete && samples.result.achieved_engagement_intervals == 0;
    if serde_json::to_vec(&samples.result)
        .map_err(|e| e.to_string())?
        .len()
        > 32_768
    {
        return Err("result-too-large".into());
    }
    Ok(())
}

pub fn result(world: &World) -> Option<RehearsalResult> {
    world
        .get_resource::<Measurements>()
        .map(|samples| samples.result.clone())
}
