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

/// Best-effort OS locale. Native browser engines can also supply
/// `navigator.language`; this seed takes precedence when the host environment
/// declares a language and otherwise leaves that browser value in charge.
pub fn query_os_locale() -> Option<String> {
    #[cfg(all(target_os = "windows", feature = "host"))]
    if let Ok(languages) = windows::System::UserProfile::GlobalizationPreferences::Languages() {
        if let Ok(language) = languages.GetAt(0) {
            if let Some(locale) = normalise_os_locale(&language.to_string()) {
                return Some(locale);
            }
        }
    }
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find_map(|value| normalise_os_locale(&value))
}

fn normalise_os_locale(value: &str) -> Option<String> {
    let raw = value.split(['.', '@']).next()?.replace('_', "-");
    (raw != "C"
        && raw != "POSIX"
        && !raw.is_empty()
        && raw.len() <= 35
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then_some(raw)
}

/// Injection safe because normalise_os_locale accepts ASCII language-tag
/// characters only; no quote, slash, newline or script terminator can enter.
pub fn os_locale_script(locale: Option<&str>) -> String {
    let value = locale
        .and_then(normalise_os_locale)
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "null".to_owned());
    format!("window.PhoenixOsLocale = {value};")
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
#[path = "os_prefs_tests.rs"]
mod tests;
