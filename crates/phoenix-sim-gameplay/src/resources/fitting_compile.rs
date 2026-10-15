use super::*;
use crate::ship::system_registry::{phaser_bank_system_id, PHASER_BANK_KIND, POWER_REACTOR_KIND};
use phoenix_model::damage::DamageBinding;

pub(super) fn named(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'-')
}
fn resource(id: &str) -> bool {
    named(&id.replace('_', "-"))
}
fn finite(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}
fn positive(value: f32) -> bool {
    finite(value) && value > 0.0
}

impl CompiledFitting {
    pub fn compile(base: &EntityConfig) -> Result<Option<Self>, String> {
        super::validate_refit_facility(base)?;
        let Some(config) = &base.fitting else {
            return Ok(None);
        };
        if config.slots.is_empty()
            && config.equipment.is_empty()
            && config.lockers.is_empty()
            && config.assemblies.is_empty()
        {
            return Ok(None);
        }
        if config.slots.is_empty() || config.equipment.is_empty() {
            return Err("fitting requires actual slots and equipment definitions".into());
        }
        let ship = base
            .ship_config
            .as_ref()
            .ok_or("fitting requires actual ship Systems")?;
        let layout = base
            .interior
            .as_ref()
            .ok_or("fitting requires physical compartments")?;
        let energy = base
            .energy
            .as_ref()
            .ok_or("fitting requires canonical energy hardware")?;
        base.damage
            .as_ref()
            .ok_or("fitting requires the canonical physical condition owner")?;
        let mut slot_ids = BTreeSet::new();
        let mut system_ids = BTreeSet::new();
        let mut compartments = BTreeMap::new();
        for slot in &config.slots {
            if !named(&slot.id)
                || !named(&slot.interface)
                || slot.size == 0
                || !named(&slot.operation_component)
                || !slot_ids.insert(&slot.id)
                || !system_ids.insert(&slot.system)
            {
                return Err("fitting slots need unique identities/Systems and positive typed interfaces/sizes".into());
            }
            let system = ship
                .system(&slot.system)
                .ok_or("fitting slot references an absent System")?;
            let kind = match slot.family {
                EquipmentFamily::Beam => PHASER_BANK_KIND,
                EquipmentFamily::Reactor => POWER_REACTOR_KIND,
            };
            if system.kind != kind || system.console.is_none() {
                return Err("fitting slot must retain its real owned System family".into());
            }
            let component = system
                .damage_components
                .iter()
                .find(|c| c.id == slot.operation_component)
                .ok_or("fitting requires explicit replaceable Operation hardware")?;
            if component.binding != DamageBinding::Operation {
                return Err(
                    "fitting cannot replace fixed intake/cooling links as Operation hardware"
                        .into(),
                );
            }
            let room = layout
                .placements
                .get(&slot.system)
                .ok_or("fitting System has no physical placement")?;
            if !layout.compartments.iter().any(|r| &r.id == room) {
                return Err("fitting slot references an absent compartment".into());
            }
            if slot.family == EquipmentFamily::Beam
                && !energy.consumers.iter().any(|c| c.system == slot.system)
            {
                return Err("fitting beam needs its existing local-energy consumer".into());
            }
            if slot.family == EquipmentFamily::Reactor
                && slot.system != crate::ship::system_registry::power_reactor_system_id()
            {
                return Err(
                    "the supported reactor fitting adapter requires the canonical reactor System"
                        .into(),
                );
            }
            compartments.insert(slot.id.clone(), room.clone());
        }
        let mut equipment_ids = BTreeSet::new();
        for equipment in &config.equipment {
            if !named(&equipment.id)
                || !named(&equipment.interface)
                || equipment.size == 0
                || equipment.storage_space == 0
                || !equipment_ids.insert(&equipment.id)
                || !positive(equipment.work.work_secs)
                || !positive(equipment.work.manual_work_fraction)
                || equipment.work.manual_work_fraction > 1.0
            {
                return Err("equipment needs unique IDs, positive space/size and finite work/manual fraction in (0,1]".into());
            }
            let mut requirements = BTreeSet::new();
            for part in &equipment.work.requirements {
                if !resource(&part.resource)
                    || !resource(&part.service)
                    || part.amount <= 0
                    || !requirements.insert((&part.resource, &part.service))
                {
                    return Err(
                        "fitting parts require distinct named positive resource/service amounts"
                            .into(),
                    );
                }
                let endpoint = super::super::service::resolve_endpoint(
                    &part.service,
                    &base.resource_services,
                    &Default::default(),
                )?;
                match endpoint {
                    super::super::service::ResourceEndpoint::Capacity { id } => {
                        if !base
                            .infrastructure
                            .as_ref()
                            .is_some_and(|i| i.capacities.iter().any(|c| c.id == id))
                        {
                            return Err(
                                "fitting parts need an actual finite local capacity/service".into(),
                            );
                        }
                        if base.logistics.as_ref().is_some_and(|p| {
                            p.stores.iter().any(|s| {
                                s.endpoint
                                    == (super::super::service::ResourceEndpoint::Capacity {
                                        id: id.clone(),
                                    })
                                    && s.resource != part.resource
                            })
                        }) {
                            return Err(
                                "fitting part service relabels an actual managed resource".into()
                            );
                        }
                    }
                    _ => {
                        return Err(
                            "fitting parts must use an actual capacity, not ammunition".into()
                        )
                    }
                }
            }
            match &equipment.hardware {
                EquipmentHardware::Beam(h) => {
                    h.impact.validate()?;
                    if ![
                        h.beam_range,
                        h.auto_arc_deg,
                        h.beam_damage_per_sec,
                        h.beam_duration_secs,
                    ]
                    .into_iter()
                    .all(positive)
                        || ![h.cooldown_secs, h.continuous_cost_per_sec]
                            .into_iter()
                            .all(finite)
                        || !h.cycle_jitter.is_finite()
                        || !(0.0..1.0).contains(&h.cycle_jitter)
                        || !h.shield_pierce.is_finite()
                        || !(0.0..=1.0).contains(&h.shield_pierce)
                    {
                        return Err("beam fitting hardware requires finite supported range/cycle/work parameters".into());
                    }
                }
                EquipmentHardware::Reactor(h) => {
                    if equipment.work.class != RefitClass::Facility
                        || ![
                            h.sustainable_output_per_sec,
                            h.max_output_per_sec,
                            h.ramp_per_sec,
                        ]
                        .into_iter()
                        .all(positive)
                        || h.sustainable_output_per_sec > h.max_output_per_sec
                        || ![h.charge_heat_per_unit, h.discharge_heat_per_unit]
                            .into_iter()
                            .all(finite)
                        || energy.reactor.initial_output_per_sec > h.max_output_per_sec
                    {
                        return Err("reactor profiles require facility work, finite generation/ramp and supported initial output".into());
                    }
                }
            }
        }
        let mut locker_ids = BTreeSet::new();
        for locker in &config.lockers {
            if !named(&locker.id)
                || !locker_ids.insert(&locker.id)
                || locker.space == 0
                || !layout
                    .compartments
                    .iter()
                    .any(|room| room.id == locker.compartment)
            {
                return Err("equipment lockers need unique IDs, positive finite allocated space and an actual compartment".into());
            }
        }
        let profile = Self {
            config: config.clone(),
            base: std::sync::Arc::new(base.clone()),
            compartments,
        };
        for slot in &config.slots {
            profile.compatible(&slot.id, &slot.default_equipment)?;
            for equipment in &config.equipment {
                if profile.compatible(&slot.id, &equipment.id).is_ok() {
                    profile.validate_adapter(slot, equipment)?;
                }
            }
            if profile.base_hardware(slot)? != profile.equipment(&slot.default_equipment)?.hardware
            {
                return Err(format!(
                    "fitting default {} must reproduce the existing hardware of {}",
                    slot.default_equipment, slot.system.0
                ));
            }
        }
        // Runtime prefixes the local authored key with "eq-<UUID>-". Leave
        // that identity space without allowing unbounded saved item keys.
        if config.assemblies.iter().any(|item| item.id.len() > 88) {
            return Err(
                "authored assembly IDs must fit 88 bytes before the ship UUID prefix".into(),
            );
        }
        let state = FittingState::from_profile(&profile)?;
        for slot in &config.slots {
            if state
                .installed(&slot.id)
                .is_none_or(|a| a.definition != slot.default_equipment)
            {
                return Err("each authored fitting slot requires exactly its real default installed assembly".into());
            }
        }
        if state
            .assemblies
            .values()
            .any(|a| matches!(a.location, EquipmentLocation::InWork { .. }))
        {
            return Err("authored initial equipment cannot invent in-progress work".into());
        }
        Ok(Some(profile))
    }

    fn validate_adapter(
        &self,
        slot: &FittingSlotConfig,
        equipment: &EquipmentDefinition,
    ) -> Result<(), String> {
        if let EquipmentHardware::Beam(h) = &equipment.hardware {
            let bank = self
                .base
                .weapons_console
                .as_ref()
                .and_then(|w| {
                    w.phaser_banks
                        .iter()
                        .find(|b| phaser_bank_system_id(&b.id).as_ref() == Some(&slot.system))
                })
                .ok_or("fitting beam has no actual bank strategy")?;
            if h.auto_arc_deg > bank.fire_arc_deg {
                return Err("beam sidegrade auto arc exceeds the fixed mount fire arc".into());
            }
        }
        Ok(())
    }
    fn base_hardware(&self, slot: &FittingSlotConfig) -> Result<EquipmentHardware, String> {
        let energy = self
            .base
            .energy
            .as_ref()
            .ok_or("fitting energy is absent")?;
        Ok(match slot.family {
            EquipmentFamily::Beam => {
                let bank = self
                    .base
                    .weapons_console
                    .as_ref()
                    .and_then(|w| {
                        w.phaser_banks
                            .iter()
                            .find(|b| phaser_bank_system_id(&b.id).as_ref() == Some(&slot.system))
                    })
                    .ok_or("fitting beam has no actual bank strategy")?;
                let consumer = energy
                    .consumers
                    .iter()
                    .find(|c| c.system == slot.system)
                    .ok_or("beam reserve is absent")?;
                EquipmentHardware::Beam(BeamHardware {
                    beam_range: bank.beam_range,
                    auto_arc_deg: bank.auto_arc_deg,
                    beam_damage_per_sec: bank.beam_damage_per_sec,
                    beam_duration_secs: bank.beam_duration_secs,
                    cooldown_secs: bank.cooldown_secs,
                    cycle_jitter: bank.cycle_jitter,
                    shield_pierce: bank.shield_pierce.unwrap_or(0.0),
                    impact: bank.impact,
                    continuous_cost_per_sec: consumer.continuous_cost_per_sec,
                })
            }
            EquipmentFamily::Reactor => EquipmentHardware::Reactor(ReactorHardware {
                sustainable_output_per_sec: energy.reactor.sustainable_output_per_sec,
                max_output_per_sec: energy.reactor.max_output_per_sec,
                ramp_per_sec: energy.reactor.ramp_per_sec,
                charge_heat_per_unit: energy.reactor.reserve.charge_heat_per_unit,
                discharge_heat_per_unit: energy.reactor.reserve.discharge_heat_per_unit,
            }),
        })
    }

    /// Resolve only supported actual capability parameters. Do not reparse this
    /// effective view as a new authored hull: the fixed graph remains unchanged.
    pub fn effective_config(&self, state: &FittingState) -> Result<EntityConfig, String> {
        state.validate(self)?;
        let mut result = self.base.as_ref().clone();
        for slot in &self.config.slots {
            let Some(assembly) = state.installed(&slot.id) else {
                if slot.family == EquipmentFamily::Beam {
                    result
                        .weapons_console
                        .as_mut()
                        .ok_or("beam strategies missing")?
                        .phaser_banks
                        .retain(|b| phaser_bank_system_id(&b.id).as_ref() != Some(&slot.system));
                }
                continue;
            };
            match &self.equipment(&assembly.definition)?.hardware {
                EquipmentHardware::Beam(h) => {
                    let bank = result
                        .weapons_console
                        .as_mut()
                        .and_then(|w| {
                            w.phaser_banks.iter_mut().find(|b| {
                                phaser_bank_system_id(&b.id).as_ref() == Some(&slot.system)
                            })
                        })
                        .ok_or("beam strategy missing")?;
                    bank.beam_range = h.beam_range;
                    bank.auto_arc_deg = h.auto_arc_deg;
                    bank.beam_damage_per_sec = h.beam_damage_per_sec;
                    bank.beam_duration_secs = h.beam_duration_secs;
                    bank.cooldown_secs = h.cooldown_secs;
                    bank.cycle_jitter = h.cycle_jitter;
                    bank.shield_pierce = Some(h.shield_pierce);
                    bank.impact = h.impact;
                    result
                        .energy
                        .as_mut()
                        .ok_or("energy missing")?
                        .consumers
                        .iter_mut()
                        .find(|c| c.system == slot.system)
                        .ok_or("beam reserve missing")?
                        .continuous_cost_per_sec = h.continuous_cost_per_sec;
                }
                EquipmentHardware::Reactor(h) => {
                    let reactor = &mut result.energy.as_mut().ok_or("energy missing")?.reactor;
                    reactor.sustainable_output_per_sec = h.sustainable_output_per_sec;
                    reactor.max_output_per_sec = h.max_output_per_sec;
                    reactor.ramp_per_sec = h.ramp_per_sec;
                    reactor.reserve.charge_heat_per_unit = h.charge_heat_per_unit;
                    reactor.reserve.discharge_heat_per_unit = h.discharge_heat_per_unit;
                }
            }
        }
        Ok(result)
    }
    /// Vacancy is physical unavailability, not an operator intent or empty buffer.
    pub fn unavailable_systems(&self, state: &FittingState) -> Result<BTreeSet<SystemId>, String> {
        state.validate(self)?;
        Ok(self
            .config
            .slots
            .iter()
            .filter(|slot| state.installed(&slot.id).is_none())
            .map(|slot| slot.system.clone())
            .collect())
    }
}
