//! Feed the host's gilrs gamepad state into the native console panes.
//!
//! Ultralight has no W3C Gamepad API — `pane_boot.js`'s `gamepad: false` was the
//! admission of that, so a gamepad plugged into a native console screen was
//! invisible to the page. This bridges Bevy's gilrs input (live for free on the
//! native Window host via `DefaultPlugins`) into each pane as a
//! `navigator.getGamepads()` snapshot in the W3C **standard** shape, so the
//! client page's existing `gui/gamepad-input.js` runtime — its selection,
//! bindings, neutral gates and hotplug-by-poll — works unchanged.
//!
//! A native pane is exactly one console (one station), so its runtime drives its
//! own station with no focus routing to resolve: the pad the operator SELECTED
//! for that screen always reaches the station on it. The host feeds every
//! connected pad to every pane; each pane picks its own selected slot.
//!
//! This module is the **pure** half — the Bevy-free shape of the snapshot, so
//! the wire form `gamepad-input.js` reads is unit-tested without gilrs. The
//! adapter that reads live pads and pushes the snapshot into each pane's view is
//! [`super::ultralight`].

use serde::Serialize;

/// The number of buttons the W3C "standard" mapping numbers (0–15).
pub const STANDARD_BUTTONS: usize = 16;

/// The number of axes the W3C "standard" mapping numbers (0–3): the two sticks.
pub const STANDARD_AXES: usize = 4;

/// One standard-mapped pad's live state, already in the W3C control order and
/// the W3C axis convention (Y positive is DOWN). Bevy-free so the wire shape is
/// tested without gilrs.
pub struct PadReading {
    /// Hardware descriptor for preference matching; identical devices remain ambiguous.
    pub id: String,
    /// The browser slot this pad occupies — the value the page's per-station
    /// selection setting stores, kept stable across frames by the adapter so a
    /// plugged-in pad keeps its slot for the session.
    pub slot: usize,
    /// Buttons 0–15 in W3C order, each `(pressed, analog value)`.
    pub buttons: [(bool, f32); STANDARD_BUTTONS],
    /// Axes 0–3 in W3C order (`left-x, left-y, right-x, right-y`).
    pub axes: [f32; STANDARD_AXES],
}

#[derive(Serialize)]
struct WireButton {
    pressed: bool,
    value: f32,
}

#[derive(Serialize)]
struct WirePad {
    id: String,
    index: usize,
    mapping: &'static str,
    connected: bool,
    buttons: Vec<WireButton>,
    axes: Vec<f32>,
}

/// Build the `navigator.getGamepads()` array for the pane shim: a slot-indexed
/// array with `null` in every empty slot, exactly the shape `enumerateGamepads`
/// / `gamepadBindingPressed` in `gui/gamepad-input.js` read (`pad.index`,
/// `pad.mapping === 'standard'`, `pad.buttons[i].pressed/value`, `pad.axes[i]`).
///
/// Returned as a JSON array literal — valid JavaScript — so the adapter can push
/// it straight into `window.__phoenixSetGamepads(<here>)` with no re-encoding.
pub fn gamepad_snapshot_json(pads: &[PadReading]) -> String {
    let Some(max_slot) = pads.iter().map(|p| p.slot).max() else {
        return "[]".to_string();
    };
    let mut slots: Vec<Option<WirePad>> = (0..=max_slot).map(|_| None).collect();
    for pad in pads {
        slots[pad.slot] = Some(WirePad {
            id: pad.id.clone(),
            index: pad.slot,
            mapping: "standard",
            connected: true,
            buttons: pad
                .buttons
                .iter()
                .map(|(pressed, value)| WireButton {
                    pressed: *pressed,
                    value: *value,
                })
                .collect(),
            axes: pad.axes.to_vec(),
        });
    }
    serde_json::to_string(&slots).unwrap_or_else(|_| "[]".to_string())
}

/// The latest script to hold on the pane thread. Before the first pad there is
/// nothing to publish. Afterwards an empty snapshot must remain held until a
/// new state arrives: clearing the slot on a faster main frame could coalesce
/// away the disconnect before any page had observed it.
pub fn held_gamepad_script(pads: &[PadReading], ever_connected: &mut bool) -> Option<String> {
    if pads.is_empty() && !*ever_connected {
        return None;
    }
    *ever_connected = true;
    Some(format!(
        "window.__phoenixSetGamepads({})",
        gamepad_snapshot_json(pads)
    ))
}

#[cfg(test)]
mod tests {
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
}
