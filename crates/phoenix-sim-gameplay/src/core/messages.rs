//! Game adapters for the shared wire vocabulary.
pub use phoenix_sim_contracts::messages::*;

// ── Inter-system command channel (issue #559) ─────────────────────────────────

/// Project published infrastructure state into the shared wire vocabulary.
pub fn infrastructure_snapshot_from_state(
    state: &crate::infrastructure::InfrastructureState,
) -> Option<InfrastructureSnapshot> {
    if !state.publishes() {
        return None;
    }
    Some(InfrastructureSnapshot {
        condition_fraction: state.condition_fraction(),
        flags: state
            .flags()
            .into_iter()
            .map(|(flag, held)| (flag.to_string(), held))
            .collect(),
        capacities: state
            .capacities()
            .iter()
            .map(|c| (c.id.clone(), c.level))
            .collect(),
    })
}

pub fn scan_reading_snapshot_from_reading(
    reading: &crate::science::ScanReading,
) -> ScanReadingSnapshot {
    ScanReadingSnapshot {
        shield_finding: reading.shield_finding.clone(),
        aim: reading.aim,
        intrusive: reading.intrusive,
        quantities: reading.quantities.clone(),
        forecast: reading.forecast.clone(),
        observation: Some(phoenix_model::observation::ObservationMetadata::at(
            reading.taken_at_tick,
            reading
                .contact_id
                .as_ref()
                .map(
                    |contact| phoenix_model::observation::ObservationSubject::Entity {
                        uuid: contact.clone(),
                    },
                )
                .unwrap_or_default(),
            phoenix_model::observation::ObservationProvenance {
                kind: phoenix_model::observation::ObservationSource::Scan,
                system: None,
                label: String::new(),
            },
            reading.condition_step.map_or(Default::default(), |step| {
                phoenix_model::observation::ObservationUncertainty::Quantized {
                    field: "condition_fraction".into(),
                    step,
                }
            }),
        )),
        subject_uuid: reading.contact_id.clone().unwrap_or_default(),
        subject_name: reading.subject_name.clone(),
        band: reading.band.clone(),
        band_label: reading.band_label.clone(),
        taken_at_tick: reading.taken_at_tick,
        condition_fraction: reading.condition_fraction,
        condition_step: reading.condition_step,
        mass: reading.mass,
        mass_class: reading.mass_class.clone(),
        mass_class_label: reading.mass_class_label.clone(),
        debris: reading.debris.clone(),
        flags: reading.flags.clone(),
        capacities: reading.capacities.clone(),
    }
}

impl From<&crate::weapons::arc_geometry::WeaponArcSector> for HostileWeaponArc {
    fn from(s: &crate::weapons::arc_geometry::WeaponArcSector) -> Self {
        Self {
            bearing_deg: s.bearing_deg,
            half_angle_deg: s.half_angle_deg,
            range: s.range,
        }
    }
}
