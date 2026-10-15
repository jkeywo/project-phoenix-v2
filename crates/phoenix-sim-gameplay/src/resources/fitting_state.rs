use super::*;
use phoenix_model::damage::ComponentCondition;

fn condition_shape(state: &ComponentState) -> Result<(), String> {
    if !state.dose.is_finite()
        || state.dose < 0.0
        || state.last_source.as_ref().is_some_and(|s| {
            s.trim().is_empty() || s.len() > 256 || s.chars().any(char::is_control)
        })
        || (state.last_source.is_none()
            && (state.dose > 0.0 || state.condition != ComponentCondition::Healthy))
        || matches!(state.condition, ComponentCondition::Patched { factor } if !factor.is_finite() || factor <= 0.0 || factor > 1.0)
    {
        return Err("portable equipment condition is malformed".into());
    }
    Ok(())
}

impl CompiledFitting {
    pub fn validate_condition(&self, slot: &str, condition: &ComponentState) -> Result<(), String> {
        condition_shape(condition)?;
        let slot = self.slot(slot)?;
        let component = self
            .base
            .ship_config
            .as_ref()
            .and_then(|ship| ship.system(&slot.system))
            .and_then(|system| {
                system
                    .damage_components
                    .iter()
                    .find(|c| c.id == slot.operation_component)
            })
            .ok_or("fitting Operation hardware is absent")?;
        if let Some(emp) = &condition.emp {
            let policy = self
                .base
                .damage
                .as_ref()
                .and_then(|damage| damage.emp.as_ref())
                .ok_or("portable EMP has no compatible receiver policy")?;
            if !policy.bindings.contains(&component.binding) {
                return Err("portable EMP has an unsupported Operation binding".into());
            }
            emp.validate_shape(policy)?;
        }
        if condition.dose >= f64::from(component.fault_dose_threshold) {
            return Err("portable dose exceeds the supported Operation fault frontier".into());
        }
        if let ComponentCondition::Patched { factor } = condition.condition {
            let supported = component.patch_remedy.as_ref()
                .and_then(|id| self.base.damage.as_ref()?.remedies.get(id))
                .is_some_and(|recipe| matches!(recipe.outcome,
                    phoenix_model::damage::RemedyOutcome::Patch { factor: authored } if authored == factor));
            if !supported {
                return Err(
                    "portable patch must match the actual Operation hardware remedy".into(),
                );
            }
        }
        Ok(())
    }
}

impl FittingState {
    pub fn from_profile(profile: &CompiledFitting) -> Result<Self, String> {
        let mut state = Self {
            desired: profile
                .config
                .slots
                .iter()
                .map(|s| (s.id.clone(), s.default_equipment.clone()))
                .collect(),
            ..Default::default()
        };
        for item in &profile.config.assemblies {
            if state
                .assemblies
                .insert(item.id.clone(), item.clone())
                .is_some()
            {
                return Err("initial equipment assembly identity is duplicated".into());
            }
        }
        state.validate(profile)?;
        Ok(state)
    }

    pub fn installed(&self, slot: &str) -> Option<&EquipmentAssembly> {
        self.assemblies.values().find(|a| matches!(&a.location, EquipmentLocation::Installed { slot: target } if target == slot))
    }

    pub fn validate(&self, profile: &CompiledFitting) -> Result<(), String> {
        if self.desired.keys().cloned().collect::<BTreeSet<_>>()
            != profile
                .config
                .slots
                .iter()
                .map(|s| s.id.clone())
                .collect::<BTreeSet<_>>()
        {
            return Err("fitting desired choices must name exactly the fixed slot set".into());
        }
        for (slot, definition) in &self.desired {
            profile.compatible(slot, definition)?;
        }
        let mut installed = BTreeSet::new();
        let mut occupied: BTreeMap<&str, u64> = BTreeMap::new();
        for (id, item) in &self.assemblies {
            if id != &item.id || !super::fitting_compile::named(id) {
                return Err("equipment map key must match its unique actual identity".into());
            }
            let definition = profile.equipment(&item.definition)?;
            match &item.location {
                EquipmentLocation::Installed { slot } => {
                    profile.compatible(slot, &item.definition)?;
                    if !installed.insert(slot) || item.condition.is_some() {
                        return Err("one installed assembly per slot; installed condition belongs only to ShipDamage".into());
                    }
                }
                EquipmentLocation::Stored { locker } => {
                    let locker = profile.locker(locker)?;
                    let space = occupied.entry(&locker.id).or_default();
                    *space = space
                        .checked_add(u64::from(definition.storage_space))
                        .ok_or("equipment storage overflow")?;
                    if *space > u64::from(locker.space) {
                        return Err("equipment locker is full".into());
                    }
                    condition_shape(
                        item.condition
                            .as_ref()
                            .ok_or("stored assembly needs its actual condition")?,
                    )?;
                }
                EquipmentLocation::InWork { job, compartment } => {
                    if *job == 0
                        || !profile.base.interior.as_ref().is_some_and(|layout| {
                            layout.compartments.iter().any(|r| &r.id == compartment)
                        })
                    {
                        return Err("in-work assembly needs a real job and compartment".into());
                    }
                    condition_shape(
                        item.condition
                            .as_ref()
                            .ok_or("in-work assembly needs its actual condition")?,
                    )?;
                }
            }
            if let Some(condition) = &item.condition {
                let supported = profile
                    .config
                    .slots
                    .iter()
                    .filter(|s| profile.compatible(&s.id, &item.definition).is_ok())
                    .any(|slot| profile.validate_condition(&slot.id, condition).is_ok());
                if !supported {
                    return Err("portable condition has no compatible supported Operation hardware frontier".into());
                }
            }
        }
        Ok(())
    }

    fn transaction<T>(
        &mut self,
        profile: &CompiledFitting,
        apply: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.validate(profile)?;
        let mut next = self.clone();
        let result = apply(&mut next)?;
        next.validate(profile)?;
        *self = next;
        Ok(result)
    }

    /// Changes intent only; no assembly, team or resource claim is created.
    pub fn request(
        &mut self,
        profile: &CompiledFitting,
        slot: &str,
        definition: &str,
    ) -> Result<(), String> {
        profile.compatible(slot, definition)?;
        self.transaction(profile, |next| {
            next.desired.insert(slot.into(), definition.into());
            Ok(())
        })
    }

    /// Same-room convenience for the original fitting adapter. Distant items
    /// must first be collected by the actual team at their locker.
    pub fn stored_to_work(
        &mut self,
        profile: &CompiledFitting,
        item: &str,
        job: u64,
        slot: &str,
    ) -> Result<(), String> {
        let room = profile.compartment(slot)?.to_string();
        self.transaction(profile, |next| {
            let item = next
                .assemblies
                .get_mut(item)
                .ok_or("equipment assembly is absent")?;
            profile.compatible(slot, &item.definition)?;
            let EquipmentLocation::Stored { locker } = &item.location else {
                return Err("equipment is already installed or committed to work".into());
            };
            if profile.locker(locker)?.compartment != room {
                return Err("equipment is not in the refit compartment; deliver it first".into());
            }
            item.location = EquipmentLocation::InWork {
                job,
                compartment: room,
            };
            Ok(())
        })
    }

    /// Caller proves the committed team is physically at this locker. The
    /// assembly changes owner/location once; its actual condition is retained.
    pub fn pick_up_from_locker(
        &mut self,
        profile: &CompiledFitting,
        item: &str,
        job: u64,
        actual_compartment: &str,
    ) -> Result<(), String> {
        self.transaction(profile, |next| {
            let item = next
                .assemblies
                .get_mut(item)
                .ok_or("equipment assembly is absent")?;
            let EquipmentLocation::Stored { locker } = &item.location else {
                return Err("equipment is already installed or committed to work".into());
            };
            if profile.locker(locker)?.compartment != actual_compartment {
                return Err("team is not at the actual equipment locker".into());
            }
            item.location = EquipmentLocation::InWork {
                job,
                compartment: actual_compartment.into(),
            };
            Ok(())
        })
    }

    /// Called only after this same team's real route reaches the new room.
    /// Moving a carried assembly does not install it or change its condition.
    pub fn move_in_work(
        &mut self,
        profile: &CompiledFitting,
        item: &str,
        job: u64,
        actual_compartment: &str,
    ) -> Result<(), String> {
        self.transaction(profile, |next| {
            let item = next.assemblies.get_mut(item).ok_or("equipment assembly is absent")?;
            if !matches!(&item.location, EquipmentLocation::InWork { job: owner, .. } if *owner == job) {
                return Err("assembly is not carried by this refit job".into());
            }
            item.location = EquipmentLocation::InWork { job, compartment: actual_compartment.into() };
            Ok(())
        })
    }

    /// Caller stages the matching ShipDamage removal in the same transaction.
    pub fn remove_installed(
        &mut self,
        profile: &CompiledFitting,
        slot: &str,
        job: u64,
        condition: ComponentState,
    ) -> Result<String, String> {
        profile.validate_condition(slot, &condition)?;
        let room = profile.compartment(slot)?.to_string();
        self.transaction(profile, |next| {
            let item = next.assemblies.values_mut().find(|a| matches!(&a.location, EquipmentLocation::Installed { slot: current } if current == slot))
                .ok_or("fitting slot is already vacant")?;
            item.location = EquipmentLocation::InWork { job, compartment: room };
            item.condition = Some(condition);
            Ok(item.id.clone())
        })
    }

    /// Returns the actual replacement condition to move into ShipDamage. There
    /// is no installed-item copy and no implicit condition repair.
    pub fn install_from_work(
        &mut self,
        profile: &CompiledFitting,
        slot: &str,
        item: &str,
        job: u64,
    ) -> Result<ComponentState, String> {
        let room = profile.compartment(slot)?.to_string();
        self.transaction(profile, |next| {
            if next.installed(slot).is_some() { return Err("fitting slot is occupied".into()); }
            let item = next.assemblies.get_mut(item).ok_or("equipment assembly is absent")?;
            profile.compatible(slot, &item.definition)?;
            if !matches!(&item.location, EquipmentLocation::InWork { job: owner, compartment } if *owner == job && compartment == &room) {
                return Err("replacement is not held by this job at the actual slot".into());
            }
            let condition = item.condition.take().ok_or("replacement condition is absent")?;
            profile.validate_condition(slot, &condition)?;
            item.location = EquipmentLocation::Installed { slot: slot.into() };
            Ok(condition)
        })
    }

    pub fn return_to_locker(
        &mut self,
        profile: &CompiledFitting,
        item: &str,
        job: u64,
        locker: &str,
    ) -> Result<(), String> {
        let room = profile.locker(locker)?.compartment.clone();
        self.transaction(profile, |next| {
            let item = next.assemblies.get_mut(item).ok_or("equipment assembly is absent")?;
            if !matches!(&item.location, EquipmentLocation::InWork { job: owner, compartment } if *owner == job && compartment == &room) {
                return Err("assembly is not held by this job at the receiving locker".into());
            }
            item.location = EquipmentLocation::Stored { locker: locker.into() };
            Ok(())
        })
    }
}
