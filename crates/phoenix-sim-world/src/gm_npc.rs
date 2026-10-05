use phoenix_sim_contracts::doctrine::DoctrineObjective;
use serde::Deserialize;
pub fn bounded_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

/// Targets are an explicit authored allow-list of NPC names or UUIDs. Neither
/// the browser nor the action can replace the profile's doctrine or eligibility.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawNpcDoctrinePaletteEntry {
    pub id: String,
    pub label: String,
    pub targets: Vec<String>,
    pub doctrine: Vec<DoctrineObjective>,
}

#[derive(Clone, Debug)]
pub struct NpcDoctrinePaletteEntry {
    pub id: String,
    pub label: String,
    pub targets: Vec<String>,
    pub doctrine: Vec<DoctrineObjective>,
    pub origin_layer: Option<String>,
}

pub fn parse_palette(
    raw: &[RawNpcDoctrinePaletteEntry],
) -> Result<Vec<NpcDoctrinePaletteEntry>, String> {
    let mut ids = std::collections::BTreeSet::new();
    raw.iter()
        .map(|entry| {
            if !bounded_id(&entry.id)
                || entry.label.trim().is_empty()
                || !ids.insert(&entry.id)
                || entry.targets.is_empty()
                || entry.targets.len() > 32
                || entry.targets.iter().any(|target| !bounded_id(target))
                || entry.doctrine.len() > 32
            {
                return Err("invalid or duplicate GM NPC doctrine palette identity/targets".into());
            }
            crate::entities::config::validate_doctrine_directives(&entry.doctrine)?;
            let mut doctrine_ids = std::collections::BTreeSet::new();
            for objective in &entry.doctrine {
                if !bounded_id(&objective.id)
                    || !doctrine_ids.insert(&objective.id)
                    || !objective.base_priority.is_finite()
                    || !objective.target_speed.is_finite()
                    || !(0.0..=1.0).contains(&objective.target_speed)
                    || !objective.maintain_range.is_finite()
                    || objective.maintain_range < 0.0
                    || objective.modifiers.iter().any(|m| {
                        !m.weight.is_finite() || m.threshold.is_some_and(|v| !v.is_finite())
                    })
                    || objective
                        .zero_gates
                        .iter()
                        .any(|g| g.threshold.is_some_and(|v| !v.is_finite()))
                {
                    return Err(
                        "GM NPC doctrine requires unique bounded objective ids and finite tuning"
                            .into(),
                    );
                }
            }
            Ok(NpcDoctrinePaletteEntry {
                id: entry.id.clone(),
                label: entry.label.clone(),
                targets: entry.targets.clone(),
                doctrine: entry.doctrine.clone(),
                origin_layer: None,
            })
        })
        .collect()
}
