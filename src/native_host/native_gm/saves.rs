//! Private GM access to the host's existing local save catalogue.
use super::{bridge::NativeGmBridge, NativeGmSurface};
use crate::save_slots_store::SaveSlotService;
use bevy::prelude::*;
use serde::Serialize;

#[derive(Serialize)]
pub struct SaveRow {
    slot_id: String,
    display_name: String,
    kind: &'static str,
    scenario: Option<String>,
    capture_tick: Option<String>,
    preflight: crate::gm_checkpoint::CandidatePreflight,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum SaveValue {
    Slot(String),
    Rows(Vec<SaveRow>),
}

#[derive(Serialize)]
pub struct SaveReply {
    id: String,
    value: Option<SaveValue>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct SaveOutcome {
    pub(super) slot: String,
    tick: u64,
    ok: bool,
    error: Option<String>,
}

pub(super) fn request(
    world: &mut World,
    bridge: &NativeGmBridge,
    id: String,
    operation: &str,
    name: Option<String>,
) {
    if id.len() > 128 {
        return;
    }
    let answer: Result<SaveValue, String> = (|| {
        if operation == "create" {
            if world
                .resource::<State<crate::core::messages::GamePhase>>()
                .get()
                != &crate::core::messages::GamePhase::InProgress
            {
                return Err("Saves are available while the game is running".into());
            }
            let name = name
                .filter(|name| !name.trim().is_empty() && name.len() <= 1024)
                .ok_or("Enter a save name")?;
            return crate::save_slots_store::request_named_manual_save(world, name)
                .map(SaveValue::Slot)
                .map_err(str::to_owned);
        }
        if operation != "list" {
            return Err("Unknown save operation".into());
        }
        let service = world
            .get_resource::<SaveSlotService>()
            .ok_or("Local save store is unavailable")?;
        let versions = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());
        let scenario = world
            .get_resource::<crate::save_slots_lifecycle::SaveScenario>()
            .map(|value| value.0.as_str())
            .unwrap_or_default();
        let roster = world
            .get_resource::<crate::lockstep::FleetRoster>()
            .cloned()
            .unwrap_or_default();
        let hull = world.get_resource::<crate::lobby::SelectedShipResource>();
        let live = crate::gm_checkpoint::LiveSeating::from_roster(
            scenario,
            &roster,
            hull.map(|value| value.0.as_str()),
        );
        let rows = service
            .list_for_loaded_scenario(&versions, scenario)
            .map_err(|error| format!("{error:?}"))?;
        Ok(SaveValue::Rows(
            rows.iter()
                .map(|row| SaveRow {
                    slot_id: row.slot_id.clone(),
                    display_name: row.display_name.clone(),
                    kind: if row.slot_id == crate::save_slots::AUTOSAVE_SLOT {
                        "autosave"
                    } else {
                        "manual"
                    },
                    scenario: row.record.as_ref().map(|record| record.scenario.clone()),
                    capture_tick: row
                        .record
                        .as_ref()
                        .map(|record| record.capture_tick.to_string()),
                    preflight: crate::gm_checkpoint::preflight(&live, row),
                })
                .collect(),
        ))
    })();
    let reply = match answer {
        Ok(value) => SaveReply {
            id,
            value: Some(value),
            error: None,
        },
        Err(error) => SaveReply {
            id,
            value: None,
            error: Some(error),
        },
    };
    if let Ok(json) = crate::core::codec::encode_native_gm_save_reply(&reply) {
        bridge.publish("save_reply", json);
    }
}

pub fn publish_outcomes(surface: Res<NativeGmSurface>, service: Option<Res<SaveSlotService>>) {
    let Some(service) = service else {
        return;
    };
    let mut outcomes: Vec<_> = service
        .outcomes()
        .filter_map(|outcome| {
            let crate::save_slots::CaptureSlot::Manual(slot) = &outcome.decision.slot else {
                return None;
            };
            Some(SaveOutcome {
                slot: slot.clone(),
                tick: outcome.decision.tick,
                ok: outcome.result.is_ok(),
                error: outcome
                    .result
                    .as_ref()
                    .err()
                    .map(|error| format!("{error:?}")),
            })
        })
        .collect();
    outcomes.extend(service.manual_refusals().map(|refusal| SaveOutcome {
        slot: refusal.slot_id.clone(),
        tick: refusal.tick,
        ok: false,
        error: Some(format!("{:?}", refusal.reason)),
    }));
    surface.bridge.retain_save_outcomes(outcomes);
}

#[cfg(test)]
#[path = "saves_tests.rs"]
mod tests;
