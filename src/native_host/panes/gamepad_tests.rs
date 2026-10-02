use super::*;

fn pad(slot: usize) -> PadReading {
    PadReading {
        id: "Test controller".into(),
        slot,
        buttons: [(false, 0.0); STANDARD_BUTTONS],
        axes: [0.0; STANDARD_AXES],
    }
}

#[test]
fn unplug_remains_a_held_empty_snapshot_across_quiet_main_frames() {
    let mut connected = false;
    assert_eq!(held_gamepad_script(&[], &mut connected), None);
    assert!(held_gamepad_script(&[pad(0)], &mut connected).is_some());
    for _ in 0..4 {
        assert_eq!(
            held_gamepad_script(&[], &mut connected).as_deref(),
            Some("window.__phoenixSetGamepads([])")
        );
    }
}

#[test]
fn no_pads_is_an_empty_array() {
    assert_eq!(gamepad_snapshot_json(&[]), "[]");
}

#[test]
fn a_pad_lands_at_its_slot_index_with_the_standard_mapping() {
    let mut p = pad(0);
    p.buttons[0] = (true, 1.0); // face-bottom pressed
    p.axes[0] = 0.5; // left-stick-x
    let json = gamepad_snapshot_json(&[p]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = value.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let pad0 = &arr[0];
    assert_eq!(pad0["index"], 0);
    assert_eq!(pad0["mapping"], "standard");
    assert_eq!(pad0["connected"], true);
    assert_eq!(pad0["buttons"].as_array().unwrap().len(), STANDARD_BUTTONS);
    assert_eq!(pad0["buttons"][0]["pressed"], true);
    assert_eq!(pad0["buttons"][0]["value"], 1.0);
    assert_eq!(pad0["axes"].as_array().unwrap().len(), STANDARD_AXES);
    assert_eq!(pad0["axes"][0], 0.5);
}

#[test]
fn empty_slots_between_pads_are_null_so_getgamepads_indexes_by_slot() {
    // A pad in slot 2 with 0 and 1 empty: the array is [null, null, pad],
    // which is how `navigator.getGamepads()` reports slots and how
    // `latestSnapshot[selection.index]` indexes them.
    let json = gamepad_snapshot_json(&[pad(2)]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = value.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr[0].is_null());
    assert!(arr[1].is_null());
    assert_eq!(arr[2]["index"], 2);
}
