#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_native_latch_carries_a_choice_and_a_reset() {
    use crate::server::bridge::{
        published_effect_intensities, set_native_effect_intensities, NATIVE_EFFECT_LATCH_TEST_LOCK,
    };

    // The latch is process-global, and the host-lobby test that seeds it
    // from a saved record shares it; one lock keeps the two off each other.
    let _serialised = NATIVE_EFFECT_LATCH_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Whole percent in, fractions out — the shape the setting already
    // crosses the page/host bridge in (issue #1428).
    set_native_effect_intensities(Some(30), Some(0), Some(40));
    let (shake, flash, decorative) = published_effect_intensities();
    assert!((shake.expect("a published shake") - 0.3).abs() < 1e-6);
    assert_eq!(
        flash,
        Some(0.0),
        "a published zero is a choice, not an absence"
    );
    // The third effect rides the same latch but is nobody's uniform: it is
    // the band the native HUD overlay's document is stamped with.
    assert!((decorative.expect("a published band") - 0.4).abs() < 1e-6);

    // A per-setting reset publishes nothing again, so the effect goes back
    // to following the machine rather than sticking at its last number.
    set_native_effect_intensities(None, None, None);
    assert_eq!(published_effect_intensities(), (None, None, None));
}
