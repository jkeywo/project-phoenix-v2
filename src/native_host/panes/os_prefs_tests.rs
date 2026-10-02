use super::*;

#[test]
fn os_locale_is_normalised_for_native_page_injection() {
    assert_eq!(normalise_os_locale("de_DE.UTF-8"), Some("de-DE".into()));
    assert_eq!(normalise_os_locale("C"), None);
    assert_eq!(
        os_locale_script(Some("de-DE")),
        "window.PhoenixOsLocale = \"de-DE\";"
    );
    assert_eq!(
        os_locale_script(Some("de';alert(1)")),
        "window.PhoenixOsLocale = null;"
    );
}

#[test]
fn independent_reads_preserve_success_and_report_unavailable_fields() {
    let prefs = from_os_reads(Some(false), None, Some(1.5));
    assert!(prefs.reduced_motion);
    assert!(!prefs.high_contrast);
    assert_eq!(prefs.text_scale, 1.5);
    assert_eq!(
        prefs.availability,
        Some(OsPreferenceAvailability {
            reduced_motion: true,
            high_contrast: false,
            text_scale: true,
        })
    );
    assert!(os_defaults_script(&prefs).contains(
        "\"availability\":{\"reducedMotion\":true,\"contrast\":false,\"textScale\":true}"
    ));
    let neutral = from_os_reads(Some(true), Some(false), Some(1.0));
    assert!(!neutral.reduced_motion);
    assert!(neutral.availability.unwrap().high_contrast);
}

#[test]
fn unavailable_and_invalid_scale_are_honest_finite_fallbacks() {
    for value in [
        None,
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(0.0),
        Some(-1.0),
    ] {
        let prefs = from_os_reads(None, None, value);
        assert_eq!(prefs.text_scale, 1.0);
        assert!(!prefs.availability.unwrap().text_scale);
    }
    assert_eq!(from_os_reads(None, None, Some(2.25)).text_scale, 2.0);
    assert!(
        from_os_reads(None, None, Some(2.25))
            .availability
            .unwrap()
            .text_scale
    );
}

#[test]
fn the_default_states_no_preference() {
    let p = OsAccessibilityPrefs::default();
    assert!(!p.reduced_motion);
    assert!(!p.high_contrast);
    assert_eq!(p.text_scale, 1.0);
}

#[test]
fn the_default_script_matches_a_browser_with_no_preferences() {
    // A pane on a machine with nothing set must present the SAME default
    // layer a browser computes from a silent matchMedia: no motion/contrast
    // preference and the identity text scale.
    assert_eq!(
        os_defaults_script(&OsAccessibilityPrefs::default()),
        "window.PhoenixOsAccessibilityDefaults = \
             {\"reducedMotion\":false,\"contrast\":false,\"textScale\":1};"
    );
}

#[test]
fn the_script_carries_each_preference_under_the_key_the_page_reads() {
    // The keys are the page's `readInjectedOsDefaults` vocabulary:
    // reducedMotion, contrast (high-contrast → prefers-contrast: more), and
    // textScale. High contrast maps onto `contrast`, NOT a fourth key.
    let prefs = OsAccessibilityPrefs {
        reduced_motion: true,
        high_contrast: true,
        text_scale: 1.25,
        ..Default::default()
    };
    assert_eq!(
        os_defaults_script(&prefs),
        "window.PhoenixOsAccessibilityDefaults = \
             {\"reducedMotion\":true,\"contrast\":true,\"textScale\":1.25};"
    );
}

#[test]
fn the_injected_text_scale_is_clamped_to_the_pages_own_safe_range() {
    // The page clamps too, but a value already out of range in the served
    // body is confusing to read and needless; clamp at the source.
    let big = OsAccessibilityPrefs {
        text_scale: 9.0,
        ..OsAccessibilityPrefs::default()
    };
    assert!(os_defaults_script(&big).contains("\"textScale\":2"));
    let tiny = OsAccessibilityPrefs {
        text_scale: 0.01,
        ..OsAccessibilityPrefs::default()
    };
    assert!(os_defaults_script(&tiny).contains("\"textScale\":0.5"));
    let nan = OsAccessibilityPrefs {
        text_scale: f32::NAN,
        ..OsAccessibilityPrefs::default()
    };
    // Never "NaN" (not a JS literal) — a non-finite scale is the identity.
    assert!(os_defaults_script(&nan).contains("\"textScale\":1"));
    assert!(!os_defaults_script(&nan).contains("NaN"));
}

#[test]
fn the_script_is_a_single_well_formed_assignment() {
    // It is injected into a classic <script> in the pane document, so it has
    // to be one statement with no stray quote that could close the element.
    let s = os_defaults_script(&OsAccessibilityPrefs {
        reduced_motion: true,
        high_contrast: false,
        text_scale: 1.5,
        ..Default::default()
    });
    assert!(s.starts_with("window.PhoenixOsAccessibilityDefaults = {"));
    assert!(s.ends_with("};"));
    assert!(!s.contains("</script"));
    assert_eq!(s.matches(';').count(), 1);
}

#[test]
fn a_query_never_panics_and_stays_in_range() {
    // A live read can be unavailable, but must never create invalid CSS/JS.
    let p = query_os_accessibility_prefs();
    eprintln!("OS_ACCESSIBILITY_READ {p:?}");
    assert!(p.text_scale.is_finite());
    assert!(p.sane_text_scale() >= TEXT_SCALE_FLOOR);
    assert!(p.sane_text_scale() <= TEXT_SCALE_CEIL);
}
