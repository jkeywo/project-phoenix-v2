//! Reading the host machine's OS accessibility preferences, and mapping them to
//! the DEFAULT layer a pane's private Accessibility profile initialises from
//! (issue #1127).
//!
//! # Why this exists
//!
//! A browser client reads the machine's accessibility preferences through
//! `matchMedia('(prefers-reduced-motion: reduce)')` and friends, and
//! `gui/accessibility-profile.js` folds those OS defaults *under* the player's
//! explicit choices. A native Ultralight pane loads that SAME page — but
//! Ultralight ships no OS-backed `matchMedia`, so every such query answers "no
//! preference" and the OS default layer would be silently empty on a native
//! bridge.
//!
//! So the host reads the OS itself and hands the page the same default layer a
//! browser would compute, by injecting `window.PhoenixOsAccessibilityDefaults`
//! into the pane document ([`os_defaults_script`], placed by
//! [`super::document::inject_os_accessibility_defaults`]). The page's
//! `osAccessibilityDefaults` reads that global as an override for the fields it
//! carries and falls back to `matchMedia` for the rest — the documented seam,
//! mirroring how `gui/rendezvous-transport.js` reads
//! `window.PhoenixTransportFactories`.
//!
//! # What crosses, and what emphatically does not (AC4)
//!
//! Only the DEFAULT layer crosses into the page, and it never leaves the
//! machine: it is the host telling its own pane's view what the OS prefers,
//! exactly as `matchMedia` tells a phone. Nothing here is a player SETTING, and
//! nothing here rides the `NativeTransport` seam to the simulation. The private
//! profile stays client-local in the pane's own `localStorage` (per-pane since
//! #1122), and the ONLY accessibility-derived thing that ever crosses the seam
//! is the anonymous ineligible-station set (`ReportStationEligibility`), exactly
//! as it is for a phone — a set of Station ids, never a setting, a diagnosis or
//! a reason.
//!
//! # Platform read
//!
//! Windows host builds use safe WinRT UISettings and AccessibilitySettings
//! bindings. Each property can fail independently; availability accompanies the
//! default layer so the page can explain a fallback. Objects are local to the
//! read and dropped immediately. The windows binding owns runtime activation.
//! Other platforms/builds explicitly report unavailable. Reads happen when a
//! document is created; crash recreation retains that document's defaults.

/// The three OS accessibility preferences a pane imports as its default layer.
///
/// The names mirror the browser's `matchMedia` vocabulary the page already
/// understands: `reduced_motion` ↔ `prefers-reduced-motion: reduce`,
/// `high_contrast` ↔ `prefers-contrast: more`, and `text_scale` a multiplier the
/// browser has no query for and Windows supplies. Booleans default to `false`
/// and the scale to `1.0` — "no preference stated".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OsAccessibilityPrefs {
    /// The OS asked for reduced motion (Windows: animations disabled).
    pub reduced_motion: bool,
    /// The OS is in a high-contrast mode (Windows: `HCF_HIGHCONTRASTON`).
    pub high_contrast: bool,
    /// The OS text-size multiplier (Windows "Make text bigger", `1.0` == 100%).
    pub text_scale: f32,
    /// Per-property read outcome; None for synthetic preferences.
    pub availability: Option<OsPreferenceAvailability>,
}

/// Availability stays machine-local beside the defaults, never in a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OsPreferenceAvailability {
    pub reduced_motion: bool,
    pub high_contrast: bool,
    pub text_scale: bool,
}

impl Default for OsAccessibilityPrefs {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            high_contrast: false,
            text_scale: 1.0,
            availability: None,
        }
    }
}

/// The floor/ceiling the page's `clampTextScale` uses, mirrored here so the
/// injected value is already sane before the page ever sees it.
const TEXT_SCALE_FLOOR: f32 = 0.5;
const TEXT_SCALE_CEIL: f32 = 2.0;

impl OsAccessibilityPrefs {
    /// Clamp the text scale to the same safe absolute range the page enforces,
    /// and drop a non-finite value back to the identity.
    fn sane_text_scale(&self) -> f32 {
        if !self.text_scale.is_finite() {
            return 1.0;
        }
        self.text_scale.clamp(TEXT_SCALE_FLOOR, TEXT_SCALE_CEIL)
    }
}

/// The JavaScript that seeds the pane page's OS default layer.
///
/// One assignment to `window.PhoenixOsAccessibilityDefaults`, whose keys are
/// exactly the fields `gui/accessibility-profile.js`'s `readInjectedOsDefaults`
/// overlays: `reducedMotion`, `contrast` (Windows high-contrast → the browser's
/// `prefers-contrast: more`) and `textScale`. Injected into the pane document's
/// `<head>` before any `gui/` module evaluates, so the profile initialises from
/// it exactly as a browser initialises from `matchMedia`.
///
/// Deliberately hand-formatted rather than routed through `serde_json`: this is
/// a three-field, fixed-shape object, and `serde_json` is confined to
/// `core::codec` by the crate's first rule. Booleans render as JS `true`/`false`
/// and the clamped scale as a plain JS number literal.
///
/// # Injection-safety invariant
///
/// This string is interpolated verbatim into a `<script>` inside the pane's
/// HTML, so it is injection-safe *only because every field is a `bool` or a
/// finite, clamped number* — none can contain a quote, backslash, newline,
/// `</script>`, or `${`. If a live OS read ever adds a **string** field (a
/// locale label, a raw registry value, a mode name), it MUST be routed through
/// a JSON string escaper (and keep the `</script>` split-guard the
/// `the_script_is_a_single_well_formed_assignment` test pins) before it reaches
/// this `format!` — an unescaped string field here is an HTML/JS injection sink.
pub fn os_defaults_script(prefs: &OsAccessibilityPrefs) -> String {
    let availability = prefs
        .availability
        .map(|a| {
            format!(
                ",\"availability\":{{\"reducedMotion\":{},\"contrast\":{},\"textScale\":{}}}",
                a.reduced_motion, a.high_contrast, a.text_scale,
            )
        })
        .unwrap_or_default();
    format!(
        "window.PhoenixOsAccessibilityDefaults = \
         {{\"reducedMotion\":{},\"contrast\":{},\"textScale\":{}{}}};",
        prefs.reduced_motion,
        prefs.high_contrast,
        format_scale(prefs.sane_text_scale()),
        availability,
    )
}

/// Format a scale as a JS number literal with no trailing `.0` noise but always
/// a valid number (`1` and `1.25`, never `1.` or an empty string).
fn format_scale(scale: f32) -> String {
    let mut s = format!("{scale}");
    // `format!("{}", 1.0f32)` already yields "1"; this only guards a future
    // formatter change from emitting a trailing dot.
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

/// Map independently available OS properties to a finite default layer.
fn from_os_reads(
    animations: Option<bool>,
    contrast: Option<bool>,
    scale: Option<f64>,
) -> OsAccessibilityPrefs {
    let scale = scale.filter(|value| value.is_finite() && *value > 0.0);
    OsAccessibilityPrefs {
        reduced_motion: animations.is_some_and(|enabled| !enabled),
        high_contrast: contrast.unwrap_or(false),
        text_scale: scale
            .unwrap_or(1.0)
            .clamp(f64::from(TEXT_SCALE_FLOOR), f64::from(TEXT_SCALE_CEIL))
            as f32,
        availability: Some(OsPreferenceAvailability {
            reduced_motion: animations.is_some(),
            high_contrast: contrast.is_some(),
            text_scale: scale.is_some(),
        }),
    }
}

/// Read supported Windows preferences without retaining OS handles or authority.
/// Failure of one getter does not discard the other successful defaults.
#[cfg(all(target_os = "windows", feature = "host"))]
pub fn query_os_accessibility_prefs() -> OsAccessibilityPrefs {
    use windows::UI::ViewManagement::{AccessibilitySettings, UISettings};
    let ui = UISettings::new().ok();
    from_os_reads(
        ui.as_ref()
            .and_then(|settings| settings.AnimationsEnabled().ok()),
        AccessibilitySettings::new()
            .ok()
            .and_then(|settings| settings.HighContrast().ok()),
        ui.as_ref()
            .and_then(|settings| settings.TextScaleFactor().ok()),
    )
}

/// Unsupported targets/builds publish explicit unavailable status.
#[cfg(not(all(target_os = "windows", feature = "host")))]
pub fn query_os_accessibility_prefs() -> OsAccessibilityPrefs {
    from_os_reads(None, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
