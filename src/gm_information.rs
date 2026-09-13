//! Per-observer reported information. Ghosts never become simulation entities.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod reports;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ContactInformationChange {
    SetGhost {
        id: String,
        palette: String,
        position_mm: [i32; 3],
    },
    RemoveGhost {
        id: String,
    },
    SetReportPolicy {
        target: String,
        policy: reports::ReportPolicy,
    },
    ClearReportPolicy {
        target: String,
    },
}
impl ContactInformationChange {
    pub fn target(&self) -> &str {
        match self {
            Self::SetGhost { id, .. } | Self::RemoveGhost { id } => id,
            Self::SetReportPolicy { target, .. } | Self::ClearReportPolicy { target } => target,
        }
    }
    pub fn bounded(&self) -> bool {
        let id = |v: &str| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control);
        id(self.target())
            && match self {
                Self::SetGhost { palette, .. } => id(palette),
                Self::SetReportPolicy { policy, .. } => policy.changes_report(),
                _ => true,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhostContact {
    pub id: String,
    pub palette: String,
    pub label: String,
    pub position_mm: [i32; 3],
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactInformation {
    pub ghosts: BTreeMap<String, BTreeMap<String, GhostContact>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reports: reports::Reports,
}
impl ContactInformation {
    pub fn is_empty(&self) -> bool {
        self.ghosts.is_empty() && self.reports.is_empty()
    }
}

pub fn ghost_uuid(observer: &str, id: &str) -> String {
    format!("__gm_ghost:{observer}:{id}")
}

/// Shared live positional-cue audience gate. Sensors presentation cannot expose
/// current geometry from a concealed or manipulated report. Consumed cues are
/// never held for later playback; the ordinary camera's public geometry remains
/// independent of the Sensors picture. Caller supplies the effective M8 view.
pub fn suppresses_spatial_cue(
    state: &ContactInformation,
    overrides: &crate::gm_contact::ContactOverrides,
    observer: &str,
    source: &str,
    view: &crate::core::messages::ViewMode,
) -> bool {
    matches!(
        view,
        crate::core::messages::ViewMode::ScienceRadar
            | crate::core::messages::ViewMode::SensorsRadar
    ) && (crate::gm_contact::mode(overrides, observer, source)
        == crate::gm_contact::ContactMode::Conceal
        || reports::contains(&state.reports, observer, source))
}

/// Caller verifies the observing FleetSlot and any real policy target. Palette resolution and absolute
/// no-op semantics are shared by admitted GM actions and mission dispatch.
pub fn apply_change(
    runtime: &mut crate::world::server::WorldContentRuntime,
    observer: &str,
    change: &ContactInformationChange,
) -> Result<bool, &'static str> {
    if !change.bounded() {
        return Err("invalid-contact-information");
    }
    match change {
        ContactInformationChange::SetReportPolicy { target, policy } => Ok(reports::set(
            &mut runtime.contact_information.reports,
            observer,
            target,
            Some(policy.clone()),
        )),
        ContactInformationChange::ClearReportPolicy { target } => Ok(reports::set(
            &mut runtime.contact_information.reports,
            observer,
            target,
            None,
        )),
        ContactInformationChange::SetGhost {
            id,
            palette,
            position_mm,
        } => {
            let entry = crate::gm_spawn::palette_entry(&runtime.gm_palette, palette)
                .ok_or("unknown-contact-palette")?;
            let ghost = GhostContact {
                id: id.clone(),
                palette: palette.clone(),
                label: entry.label.clone(),
                position_mm: *position_mm,
            };
            let rows = runtime
                .contact_information
                .ghosts
                .entry(observer.into())
                .or_default();
            if rows.get(id) == Some(&ghost) {
                return Ok(false);
            }
            rows.insert(id.clone(), ghost);
            Ok(true)
        }
        ContactInformationChange::RemoveGhost { id } => {
            let Some(rows) = runtime.contact_information.ghosts.get_mut(observer) else {
                return Ok(false);
            };
            let changed = rows.remove(id).is_some();
            if rows.is_empty() {
                runtime.contact_information.ghosts.remove(observer);
            }
            Ok(changed)
        }
    }
}

/// Pure projection uses a reserved identity and a basic icon only. No real
/// faction, geometry, capability, physical target or entity is manufactured.
pub fn ghost_snapshots(
    state: &ContactInformation,
    observer: &str,
) -> Vec<crate::core::messages::EntitySnapshot> {
    state
        .ghosts
        .get(observer)
        .into_iter()
        .flat_map(|rows| rows.values())
        .map(|ghost| crate::core::messages::EntitySnapshot {
            uuid: ghost_uuid(observer, &ghost.id),
            name: Some(ghost.label.clone()),
            position: Some(ghost.position_mm.map(|value| value as f32 / 1000.0)),
            tags: vec![crate::gm_contact::BASIC_RADAR_TAG.into()],
            radar_icon: Some(crate::gm_contact::BASIC_RADAR_ICON.into()),
            radar_size: Some(4.0),
            ..Default::default()
        })
        .collect()
}

pub fn apply_scenario_command(
    world: &mut bevy::prelude::World,
    observer: &str,
    change: &ContactInformationChange,
) -> Result<bool, &'static str> {
    let live = world
        .query::<(
            &crate::entities::spawner::EntityUuid,
            bevy::prelude::Has<crate::lockstep::FleetSlotOf>,
            bevy::prelude::Has<crate::server_app::Ship>,
        )>()
        .iter(world)
        .any(|(id, fleet, ship)| id.0 == observer && fleet && ship);
    if !live {
        return Err("unknown-contact-observer");
    }
    if matches!(
        change,
        ContactInformationChange::SetReportPolicy { .. }
            | ContactInformationChange::ClearReportPolicy { .. }
    ) && (change.target() == observer
        || !world
            .query::<&crate::entities::spawner::EntityUuid>()
            .iter(world)
            .any(|id| id.0 == change.target()))
    {
        return Err("unknown-contact-target");
    }
    let mut runtime = world
        .get_resource_mut::<crate::world::server::WorldContentRuntime>()
        .ok_or("world-unavailable")?;
    apply_change(&mut runtime, observer, change)
}

/// Use the existing Reveal floor to clamp a false point to the same radar edge.
pub fn viewscreen_ghosts(
    state: &ContactInformation,
    observer: &str,
    x: f32,
    z: f32,
    range: f32,
) -> Vec<crate::core::messages::EntitySnapshot> {
    let sources = ghost_snapshots(state, observer);
    let modes = sources
        .iter()
        .map(|entity| (entity.uuid.clone(), crate::gm_contact::ContactMode::Reveal))
        .collect();
    let mut projected =
        crate::gm_contact::viewscreen_contacts(&sources, &modes, x, z, range, &Default::default());
    for (entity, source) in projected.iter_mut().zip(sources) {
        entity.name = source.name;
    }
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spatial_cues_follow_the_effective_sensors_picture_without_inventing_delayed_audio() {
        use crate::core::messages::ViewMode;
        let mut state = ContactInformation::default();
        reports::set(
            &mut state.reports,
            "a",
            "b",
            Some(reports::ReportPolicy {
                delay_ticks: 4,
                position_step_mm: 0,
                hide_identity: false,
            }),
        );
        let mut overrides = Default::default();
        for view in [ViewMode::SensorsRadar, ViewMode::ScienceRadar] {
            assert!(suppresses_spatial_cue(&state, &overrides, "a", "b", &view));
            assert!(!suppresses_spatial_cue(
                &state, &overrides, "other", "b", &view
            ));
        }
        assert!(!suppresses_spatial_cue(
            &state,
            &overrides,
            "a",
            "b",
            &ViewMode::Camera(Default::default())
        ));
        state.reports.clear();
        assert!(!suppresses_spatial_cue(
            &state,
            &overrides,
            "a",
            "b",
            &ViewMode::SensorsRadar
        ));
        crate::gm_contact::set(
            &mut overrides,
            "a",
            "b",
            crate::gm_contact::ContactMode::Conceal,
        );
        assert!(suppresses_spatial_cue(
            &state,
            &overrides,
            "a",
            "b",
            &ViewMode::SensorsRadar
        ));
    }
    #[test]
    fn ghost_changes_have_a_binary_roundtrip_and_strict_authored_shape() {
        let change = ContactInformationChange::SetGhost {
            id: "echo".into(),
            palette: "cargo".into(),
            position_mm: [1000, 0, -2000],
        };
        let codec = vellum_digest::ShareCodec::new("GM-INFORMATION-TEST-");
        let bytes = codec.encode(&change).unwrap();
        assert_eq!(
            codec.decode::<ContactInformationChange>(&bytes).unwrap(),
            change
        );
        let raw: crate::world::config::RawActionEntry = toml::from_str(r#"
            type = "set_contact_information"
            entity = "observer"
            contact_information = { set_ghost = { id = "echo", palette = "cargo", position_mm = [1000, 0, -2000] } }
        "#).unwrap();
        assert_eq!(
            crate::world::config::parse_action_entry(&raw).unwrap(),
            crate::world::config::TriggerAction::SetContactInformation {
                ship: "observer".into(),
                change
            }
        );
        assert!(toml::from_str::<ContactInformationChange>(r#"set_ghost = { id = "echo", palette = "cargo", position_mm = [1,2,3], physical = true }"#).is_err());
    }
}
