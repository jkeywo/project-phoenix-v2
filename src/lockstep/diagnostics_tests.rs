use super::*;
use crate::command_admission::HostSlot;
#[test]
fn reports_applied_boundaries_and_restore_result_without_transfer_payloads() {
    let records: Vec<_> = (1..=40)
        .map(|tick| HostLossRecord {
            slot: HostSlot(2),
            tick,
        })
        .collect();
    let replacement = SlotRecoveryRecord {
        slot: HostSlot(2),
        leader: HostSlot(1),
        boundary: 90,
        claim_seq: 4,
        result: SlotRecoveryResult::Recovered {
            record_tick: 90,
            digest: u64::MAX,
        },
    };
    let status = recovery_status(&records, None, Some(&replacement), None);
    assert_eq!(status["losses"].as_array().unwrap().len(), 32);
    assert_eq!(status["losses"][0]["tick"], 9);
    assert_eq!(status["replacement"]["result"], "recovered");
    assert_eq!(status["replacement"]["boundary_tick"], 90);
    assert!(status["replacement"].get("digest").is_none());
}
