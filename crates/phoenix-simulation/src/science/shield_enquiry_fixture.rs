//! Advisory unit fixtures begin after acquisition; paid lifecycle is tested by focused_tests.
use crate::core::messages::{ScanBlackboard, ScanReadingSnapshot, SystemBlackboard};
use crate::server_app::ShipSystemBlackboards;

pub(crate) fn acquired(boards: &mut ShipSystemBlackboards, subject: &str, frequency: f32) {
    let owner = crate::ship::system_registry::sensors_system_id();
    let source = super::super::scan_blackboard_key_for(&owner);
    let finding =
        phoenix_model::shield_enquiry::ShieldFinding::acquire(frequency, Default::default(), 0.01)
            .unwrap();
    let board = boards.0.entry(source).or_insert_with(|| {
        SystemBlackboard::Scan(ScanBlackboard {
            owner: Some(owner),
            capable: true,
            ..Default::default()
        })
    });
    let SystemBlackboard::Scan(scan) = board else {
        panic!("qualified Scan fixture")
    };
    scan.history.push(ScanReadingSnapshot {
        aim: phoenix_model::focused_enquiry::FocusedAim::Shields,
        subject_uuid: subject.into(),
        taken_at_tick: scan.history.len() as u64,
        shield_finding: Some(finding),
        ..Default::default()
    });
}
