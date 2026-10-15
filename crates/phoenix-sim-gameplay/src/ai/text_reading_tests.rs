use super::*;

#[test]
fn semantic_work_is_tick_based_replay_safe_and_json_continuable() {
    let mut held = None;
    assert!(!TextReading::observe(&mut held, "request/alpha", 2, 100, true).unwrap());
    assert!(!TextReading::observe(&mut held, "request/alpha", 2, 101, true).unwrap());
    let once = held.clone();
    assert!(!TextReading::observe(&mut held, "request/alpha", 2, 101, true).unwrap());
    assert_eq!(held, once);
    let mut restored = serde_json::from_str(&serde_json::to_string(&held).unwrap()).unwrap();
    for value in [&mut held, &mut restored] {
        assert!(TextReading::observe(value, "request/alpha", 2, 102, true).unwrap());
    }
    assert_eq!(held, restored);
}

#[test]
fn ownership_information_loss_and_skipped_time_do_not_earn_reading_work() {
    let mut held = None;
    TextReading::observe(&mut held, "known-finding", 2, 0, true).unwrap();
    TextReading::observe(&mut held, "known-finding", 2, 1, false).unwrap();
    assert!(!TextReading::observe(&mut held, "known-finding", 2, 2, true).unwrap());
    assert_eq!(held.as_ref().unwrap().completed_ticks, 1);
    assert!(!TextReading::observe(&mut held, "known-finding", 2, 200, true).unwrap());
    assert_eq!(held.as_ref().unwrap().completed_ticks, 1);
    assert!(TextReading::observe(&mut held, "known-finding", 2, 201, true).unwrap());
}

#[test]
fn revisions_restart_but_repeated_copies_do_not_and_zero_work_is_responsive() {
    let mut held = None;
    TextReading::observe(&mut held, "same-original", 2, 5, true).unwrap();
    TextReading::observe(&mut held, "same-original", 2, 6, true).unwrap();
    assert!(!TextReading::observe(&mut held, "changed-original", 2, 7, true).unwrap());
    assert_eq!(held.as_ref().unwrap().completed_ticks, 0);
    assert!(TextReading::observe(&mut held, "short-text", 0, 8, true).unwrap());
    let before = held.clone();
    assert!(TextReading::observe(&mut held, "future", 3, 7, true).is_err());
    assert_eq!(before, held);
    held.as_mut().unwrap().completed_ticks = 1;
    assert!(held.as_ref().unwrap().validate(8).is_err());
}
