//! Bounded, read-only recovery facts shared by browser and native operators.
//! No command payloads, crew tokens or reconnect capabilities cross this view.
use serde_json::{json, Value};

use super::{
    recovery::RecoveryDiagnostic, FleetRoster, HostLossRecord, SlotRecoveryRecord,
    SlotRecoveryResult,
};

pub fn recovery_status(
    losses: &[HostLossRecord],
    divergence: Option<&RecoveryDiagnostic>,
    replacement: Option<&SlotRecoveryRecord>,
    roster: Option<&FleetRoster>,
) -> Value {
    let losses: Vec<_> = losses
        .iter()
        .rev()
        .take(32)
        .rev()
        .map(|record| json!({"slot":record.slot.0,"tick":record.tick}))
        .collect();
    let divergence = divergence.map(|record| {
        json!({
            "observer":record.observer.0, "divergence_tick":record.divergence_tick,
            "boundary_tick":record.boundary_tick, "leader":record.leader.map(|slot|slot.0),
            "recovering":record.recovering.iter().map(|slot|slot.0).collect::<Vec<_>>(),
            "result": match &record.result {
                super::recovery::RecoveryResult::Led { .. } => "led",
                super::recovery::RecoveryResult::Witnessed => "witnessed",
                super::recovery::RecoveryResult::Recovered { .. } => "recovered",
                super::recovery::RecoveryResult::NoSafeLeader { .. } => "no-safe-leader",
                super::recovery::RecoveryResult::NoValidRecord { .. } => "no-valid-record",
            },
        })
    });
    let replacement = replacement.map(|record| {
        json!({
            "slot":record.slot.0, "leader":record.leader.0, "boundary_tick":record.boundary,
            "claim_seq":record.claim_seq,
            "result": match &record.result {
                SlotRecoveryResult::Led { .. } => "led",
                SlotRecoveryResult::Witnessed => "witnessed",
                SlotRecoveryResult::Recovered { .. } => "recovered",
                SlotRecoveryResult::NoValidRecord { .. } => "no-valid-record",
            },
        })
    });
    let ships: Vec<_> = roster
        .map(|roster| {
            roster
                .ships()
                .iter()
                .map(|ship| json!({"slot":ship.host.0,"crewed":!ship.crew.is_empty()}))
                .collect()
        })
        .unwrap_or_default();
    json!({"losses":losses,"divergence":divergence,"replacement":replacement,"ships":ships})
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
