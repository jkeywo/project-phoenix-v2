//! Keeping the native bridge **setup** and its configured **pane layouts**
//! operable under the shared Accessibility profile (issue #1128) — **pure,
//! Bevy-free**.
//!
//! The four pieces this builds on are all pure models already: which monitor is
//! which and where each pane sits ([`super::bridge_profile`]), which pane an
//! input reaches and which one holds keyboard focus
//! ([`super::input_routing`]), which media device each surface uses
//! ([`super::bridge_media`]), and the OS accessibility default layer a pane
//! imports ([`super::panes::os_prefs`]). This module is the fifth: given a
//! resolved bridge and the shared Accessibility profile's supported extremes,
//! *does the configured layout keep every console's controls present and
//! reachable, is every setup action reachable without touch, and is keyboard
//! focus shown by shape rather than colour* — the arithmetic and the rules
//! behind acceptance criteria 1–4, decided here and checked by the ordinary
//! `cargo test` CI runs on no hardware.
//!
//! The one thing it cannot decide is whether real text on a real monitor at a
//! real scale actually reflows without a clipped pixel; that is the multi-monitor
//! walkthrough in `docs/acceptance/1128-accessibility.md` and the `#[ignore]`d
//! `tests/native_bridge_accessibility.rs`, exactly as #1123's real-window proof
//! is deferred to a machine with the displays.
//!
//! # The four models
//!
//! * **Reflow headroom** ([`PaneContentBox`], [`bridge_preserves_all_consoles`])
//!   — a pane's rectangle divided by its monitor scale is its console's
//!   **logical** box; text scaling multiplies the console's minimum operable box,
//!   and the pane must still hold it. This is acceptance criterion 1's "no
//!   overlap, no unreachable actions at the supported scaling extremes", reduced
//!   to one comparison per pane.
//! * **Focus order across monitors and split panes** ([`bridge_focus_order`]) —
//!   the global keyboard traversal order over every Station pane on every
//!   monitor, monitor by monitor in profile order and pane by pane within each
//!   monitor. Acceptance criterion 3's "including across monitors and split
//!   panes", as a deterministic sequence a [`FocusRing`] cycles.
//! * **Setup-action reachability** ([`SetupAction`], [`InputRoutes`]) — every
//!   display, pane, touch and media assignment action, and the input modalities
//!   that can perform it. Acceptance criterion 2's "all reachable by keyboard and
//!   mouse as well as touch": the invariant is that no action is touch-only.

use super::bridge_profile::{
    pane_rects, DisplayRole, MonitorIdentity, PaneRect, ResolvedBridge, ResolvedSurface,
};
use super::panes::os_prefs::OsAccessibilityPrefs;

// ── supported text-scale extremes ────────────────────────────────────────────

/// The smallest supported text-scale multiplier — the identity, "no scaling".
///
/// Mirrors `TEXT_SCALE_MIN` in `gui/accessibility-profile.js`, which is the
/// authority the player's slider and the CSS `--a11y-text-scale` var use. Kept
/// here so the reflow check reasons over the *same* range the page can actually
/// produce; [`the_supported_extremes_match_the_client`] is the drift guard.
pub const SUPPORTED_TEXT_SCALE_MIN: f64 = 1.0;

/// The largest supported text-scale multiplier. Mirrors `TEXT_SCALE_MAX` in
/// `gui/accessibility-profile.js`.
pub const SUPPORTED_TEXT_SCALE_MAX: f64 = 1.5;

// ── reflow headroom (acceptance criterion 1) ─────────────────────────────────

/// The smallest **logical** width a console keeps every control present and
/// reachable in, before columns can no longer sit side by side without
/// overlapping — a legibility bound, not a designer tunable.
///
/// Like [`super::bridge_profile::MAX_PANES_PER_STATION`] this is a property of
/// how the consoles are authored, not a value a scenario tunes: the client CSS is
/// mobile-first, its type ramp floors at `--text-min: 11px` and its control rows
/// carry fixed `min-width`s (`gui/tokens.css`, `gui/console.css`), so below a
/// phone-width logical box a row's columns overlap rather than shrink. `320`
/// logical pixels is the classic narrow-phone width the consoles already reflow
/// within; a pane narrower than this at a given text scale is the one case where
/// "no overlap" stops holding.
pub const MIN_CONSOLE_LOGICAL_WIDTH_PX: u32 = 320;

/// The smallest **logical** height a console stays operable in. Below this even a
/// scrolled console has too little viewport to show a header and an action row at
/// once.
///
/// Unlike the width bound this is a **fixed floor**, not scaled by the text
/// multiplier: the consoles scroll vertically (`overflow-y: auto` in
/// `gui/console.css`), so larger text producing more vertical content is absorbed
/// by scrolling and every action stays reachable — "no unreachable actions" holds
/// by scroll. Horizontal overflow is the one that collides columns, which is why
/// only [`MIN_CONSOLE_LOGICAL_WIDTH_PX`] grows with the text scale.
pub const MIN_CONSOLE_LOGICAL_HEIGHT_PX: u32 = 320;

/// A console pane's box in its own **logical (page CSS) pixels** — the pane's
/// physical rectangle divided by its monitor's scale factor, which is the box the
/// Ultralight view actually lays the console out in (see
/// [`super::input_routing`]'s coordinate-space note).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneContentBox {
    pub logical_width: f64,
    pub logical_height: f64,
}

impl PaneContentBox {
    /// The logical box of one pane rectangle on a monitor of `scale_factor`.
    ///
    /// A non-positive scale is treated as `1.0`, matching
    /// [`super::input_routing`]'s guard, so a malformed geometry degrades to
    /// physical-equals-logical rather than dividing by zero.
    pub fn of(rect: &PaneRect, scale_factor: f64) -> Self {
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        Self {
            logical_width: rect.width as f64 / scale,
            logical_height: rect.height as f64 / scale,
        }
    }

    /// Whether this box still holds a console's minimum operable area at
    /// `text_scale` — acceptance criterion 1, one comparison.
    ///
    /// Text scaling enlarges every string at once (`--a11y-text-scale` multiplies
    /// the console root font-size). The failure mode that scaling drives is
    /// **horizontal**: a row of fixed-`min-width` columns whose labels grow until
    /// they collide, because a row does not scroll sideways. So the **width**
    /// demand grows with the text multiplier, while the **height** demand is a
    /// fixed floor — vertical growth is absorbed by the consoles' `overflow-y:
    /// auto` scroll, keeping every action reachable (see the two constants). A
    /// negative or non-finite scale is clamped to zero width demand, so a garbage
    /// value never reports a false failure.
    pub fn preserves_console_at_scale(&self, text_scale: f64) -> bool {
        let s = if text_scale.is_finite() && text_scale > 0.0 {
            text_scale
        } else {
            0.0
        };
        self.logical_width >= MIN_CONSOLE_LOGICAL_WIDTH_PX as f64 * s
            && self.logical_height >= MIN_CONSOLE_LOGICAL_HEIGHT_PX as f64
    }

    /// Whether the console is preserved across the whole supported text-scale
    /// range. Because the demand grows monotonically with the scale, checking the
    /// [maximum](SUPPORTED_TEXT_SCALE_MAX) is sufficient; this states the intent
    /// at the call site.
    pub fn preserves_console_across_supported_scaling(&self) -> bool {
        self.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MAX)
    }
}

/// One Station pane's home for the accessibility checks: which monitor it is on,
/// who sits at it, its rectangle and its monitor scale — everything the reflow
/// and focus-order checks need, with no Bevy and no window.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusablePane {
    /// The monitor this pane is composited on.
    pub monitor: MonitorIdentity,
    /// The participant label — the `--pane <NAME>` name that ties this pane to a
    /// profile Station slot (see [`super::bridge_profile::PaneSlot`]).
    pub label: String,
    /// The pane's rectangle in its monitor's physical pixels.
    pub rect: PaneRect,
    /// The monitor's scale factor, for turning the rectangle into a logical box.
    pub scale_factor: f64,
}

impl FocusablePane {
    /// This pane's console box in logical pixels.
    pub fn content_box(&self) -> PaneContentBox {
        PaneContentBox::of(&self.rect, self.scale_factor)
    }

    /// Whether this pane preserves its console across the supported text-scale
    /// range.
    pub fn preserves_console(&self) -> bool {
        self.content_box()
            .preserves_console_across_supported_scaling()
    }
}

/// The global keyboard-focus traversal order over every Station pane on every
/// monitor — acceptance criterion 3, "across monitors and split panes".
///
/// Monitor by monitor in the profile's own order, then pane by pane within each
/// monitor in [`pane_rects`] order (left-to-right for a side-by-side split,
/// top-to-bottom for a stacked one). The viewscreen has no panes and contributes
/// nothing. This is the sequence a [`FocusRing`] cycles with Ctrl+Tab, laid out
/// so the very first assertion — that it lists every configured pane exactly once
/// — is what proves no pane is unreachable by keyboard.
pub fn bridge_focus_order(resolved: &ResolvedBridge) -> Vec<FocusablePane> {
    let mut order = Vec::new();
    for surface in resolved.stations() {
        push_station_panes(surface, &mut order);
    }
    order
}

/// The panes of one resolved Station surface, in split order.
fn push_station_panes(surface: &ResolvedSurface, out: &mut Vec<FocusablePane>) {
    let DisplayRole::Station { split, panes } = &surface.role else {
        return;
    };
    let rects = pane_rects(&surface.geometry, *split, panes.len());
    for (slot, rect) in panes.iter().zip(rects) {
        out.push(FocusablePane {
            monitor: surface.identity.clone(),
            label: slot.label.clone(),
            rect,
            scale_factor: surface.geometry.scale_factor,
        });
    }
}

/// Whether every configured pane, across every monitor and split, preserves its
/// console across the supported text-scale range — acceptance criterion 1 over a
/// whole resolved bridge.
///
/// `true` when the bridge has no Station panes at all (nothing to fail); the
/// callers that care whether a bridge is *configured* check that separately.
pub fn bridge_preserves_all_consoles(resolved: &ResolvedBridge) -> bool {
    bridge_focus_order(resolved)
        .iter()
        .all(FocusablePane::preserves_console)
}

// ── setup-action reachability (acceptance criterion 2) ───────────────────────

/// A bridge setup action whose reachability by keyboard, mouse and touch
/// acceptance criterion 2 gates.
///
/// Every one of these is an *assignment*: which role a display has, which panes a
/// Station shows, which monitor a touch device drives, and which media device a
/// surface uses. In this host all four are performed the same way — by editing
/// the bridge profile TOML and running `--setup` to check it — which is why none
/// is touch-only and each carries an equivalent keyboard and mouse route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupAction {
    /// Assign a monitor the viewscreen or a Station role
    /// (`[[display]] role = …`).
    DisplayRole,
    /// Assign a Station's console panes and their split
    /// (`[[display.pane]]`, `split = …`).
    PaneAssignment,
    /// Map a touch input device to the monitor it drives (`[[touch]]`).
    TouchMapping,
    /// Assign a surface's camera, microphone(s) and output(s) (`[[media]]`).
    MediaAssignment,
}

/// Which input modalities can perform a [`SetupAction`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputRoutes {
    /// Reachable from the keyboard (editing the profile, running `--setup`).
    pub keyboard: bool,
    /// Reachable with the mouse (a text editor, a file picker, clicking the
    /// pane/console controls the profile lays out).
    pub mouse: bool,
    /// Reachable by touch (a touch-first affordance), where one exists.
    pub touch: bool,
}

impl InputRoutes {
    /// Whether the action is reachable by keyboard **and** mouse — the guarantee
    /// acceptance criterion 2 asks of every setup action.
    pub fn keyboard_and_mouse(&self) -> bool {
        self.keyboard && self.mouse
    }

    /// Whether the action is touch-**only** — reachable by touch but not by
    /// keyboard or mouse. Acceptance criterion 2 forbids this for every action.
    pub fn is_touch_only(&self) -> bool {
        self.touch && !(self.keyboard || self.mouse)
    }
}

impl SetupAction {
    /// Every setup action acceptance criterion 2 covers.
    pub const ALL: [SetupAction; 4] = [
        SetupAction::DisplayRole,
        SetupAction::PaneAssignment,
        SetupAction::TouchMapping,
        SetupAction::MediaAssignment,
    ];

    /// A short operator label for the setup report and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            SetupAction::DisplayRole => "display role assignment",
            SetupAction::PaneAssignment => "pane assignment",
            SetupAction::TouchMapping => "touch-device mapping",
            SetupAction::MediaAssignment => "media-device assignment",
        }
    }

    /// The input routes this action is reachable by.
    ///
    /// All four assignments live in the profile TOML and the `--setup`/`--profile`
    /// flow, so all four are reachable by keyboard (editing, the CLI) and by mouse
    /// (an editor, a file picker, clicking the laid-out controls). None is a
    /// touch-first affordance today, so `touch` is `false` — which is exactly what
    /// acceptance criterion 2 wants: a setup action must never be reachable *only*
    /// by touch. If a touch-first setup surface is ever added, it sets `touch`
    /// here and the keyboard/mouse routes above are what keep the invariant.
    pub fn routes(self) -> InputRoutes {
        InputRoutes {
            keyboard: true,
            mouse: true,
            touch: false,
        }
    }
}

/// Whether every setup action is reachable by keyboard and mouse and none is
/// touch-only — acceptance criterion 2 over the whole action set.
pub fn every_setup_action_is_keyboard_and_mouse_reachable() -> bool {
    SetupAction::ALL
        .iter()
        .all(|a| a.routes().keyboard_and_mouse() && !a.routes().is_touch_only())
}

// ── the accessibility half of the --setup report ─────────────────────────────

/// Render the accessibility section of the `--setup` report (issue #1128).
///
/// Appended to the display and media halves so an operator running `--setup` — a
/// keyboard/CLI route, itself part of acceptance criterion 2 — sees, per
/// configured Station pane, whether it preserves its console at the supported
/// text-scale extremes, and sees the OS accessibility preferences the panes and
/// the focus reticle will start from. Operator-facing diagnostic text, like the
/// rest of the report; the player-visible accessibility labels live in the client
/// and its `strings.csv`.
///
/// `resolved` is `None` when no profile was supplied or it did not resolve; the
/// section then says only what the OS prefers, because there is no configured
/// layout to check.
pub fn render_accessibility_setup_report(
    resolved: Option<&ResolvedBridge>,
    prefs: &OsAccessibilityPrefs,
) -> String {
    let mut out = String::new();
    out.push_str("\nAccessibility:\n");
    out.push_str(&format!(
        "  OS defaults: text scale {}, contrast {}, reduced motion {}\n",
        prefs.text_scale,
        on_off(prefs.high_contrast),
        on_off(prefs.reduced_motion),
    ));
    out.push_str(&format!(
        "  Supported text scaling: {}x to {}x\n",
        SUPPORTED_TEXT_SCALE_MIN, SUPPORTED_TEXT_SCALE_MAX,
    ));
    if let Some(availability) = prefs.availability {
        for (name, available) in [
            ("text size", availability.text_scale),
            ("contrast", availability.high_contrast),
            ("motion", availability.reduced_motion),
        ] {
            if !available {
                out.push_str(&format!(
                    "  OS {name}: unavailable; using the standard default.\n"
                ));
            }
        }
    }

    let Some(resolved) = resolved else {
        out.push_str("  No profile resolved; no pane layout to check.\n");
        return out;
    };

    let panes = bridge_focus_order(resolved);
    if panes.is_empty() {
        out.push_str("  No Station panes configured.\n");
        return out;
    }

    out.push_str(&format!(
        "  Keyboard focus order across {} pane(s):\n",
        panes.len()
    ));
    for (i, pane) in panes.iter().enumerate() {
        let b = pane.content_box();
        let ok = pane.preserves_console();
        out.push_str(&format!(
            "    {}. {} on {} — {}x{} logical — text scaling {}\n",
            i + 1,
            pane.label,
            pane.monitor,
            b.logical_width as u32,
            b.logical_height as u32,
            if ok {
                "preserved to the supported maximum"
            } else {
                "TOO SMALL at the supported maximum: content would overlap or clip"
            },
        ));
    }
    out
}

fn on_off(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}

#[cfg(test)]
#[path = "setup_accessibility_tests.rs"]
mod tests;
