//! HUD command revisions and placement relative to revealed host chrome.
//!
//! The main world owns this cache. The worker owns each view's last successful
//! application, so a new/revealed/resized view can request the retained command
//! without making the producer encode it again.

/// The lobby stays behind tiled consoles (their default UI layer is zero),
/// matching the input router's console-first hit order.
pub(super) const LOBBY_Z_INDEX: i32 = -1;

/// The passive HUD must not cover the interactive lobby's Settings or rows.
/// CSS z-index inside the lobby cannot lift controls above another UI texture.
/// When chrome yields, restore the HUD above the native radar/comms overlays.
pub(super) fn hud_z_index(lobby_composited: bool) -> i32 {
    if lobby_composited {
        LOBBY_Z_INDEX - 1
    } else {
        20
    }
}

/// The retained command for the viewscreen HUD overlay: what the surface shows,
/// and how much of it is allowed to move.
///
/// ONE command carrying both halves rather than two channels, because the
/// worker re-applies the retained command to a view that was reloaded, revealed
/// or re-created, and two channels would mean two things to re-apply and an
/// order to get wrong. The effects statement is emitted FIRST so a document that
/// hears both in the same evaluation has its bands in force before the alert
/// class flips — the vignette never gets a frame of unstamped pulse.
#[derive(Debug, Default)]
pub(crate) struct HudScriptCache {
    json: Option<String>,
    effects: Option<String>,
    reading: Option<String>,
    script: Option<String>,
    revision: u64,
}

impl HudScriptCache {
    pub fn update(&mut self, json: &str) -> bool {
        if self.json.as_deref() == Some(json) {
            return false;
        }
        self.json = Some(json.to_owned());
        self.recompose();
        true
    }

    /// Retain `statement` as the effects half of the command (issue #1428).
    ///
    /// Answers `false` — and leaves the revision alone — when nothing changed,
    /// so a display whose settings nobody touches costs one string compare a
    /// frame and no pushes at all.
    pub fn set_effects(&mut self, statement: Option<String>) -> bool {
        if self.effects == statement {
            return false;
        }
        self.effects = statement;
        self.recompose();
        true
    }

    fn recompose(&mut self) {
        let update = self
            .json
            .as_deref()
            .map(crate::core::codec::encode_hud_update_script);
        self.script = match (self.effects.as_deref(), update) {
            (None, None) => None,
            (Some(effects), None) => Some(format!("{effects};")),
            (None, Some(update)) => Some(update),
            (Some(effects), Some(update)) => Some(format!("{effects};{update}")),
        };
        if let Some(reading) = &self.reading {
            self.script = Some(format!(
                "{reading};{}",
                self.script.as_deref().unwrap_or_default()
            ));
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_reading(&mut self, statement: Option<String>) -> bool {
        if self.reading == statement {
            return false;
        }
        self.reading = statement;
        self.recompose();
        true
    }

    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }
}

/// The JavaScript that stamps this display's three effect bands on the HUD
/// overlay document's root (issue #1428).
///
/// One call to `window.__phoenixSetHudEffects`, whose three keys are exactly the
/// ones `gui/visual-effects.js` names (`EFFECT_IDS`) and whose values are the
/// RESOLVED `0..=1` intensities `ViewscreenMotion` holds — the operator's stored
/// choice where they made one, and otherwise what following this machine's
/// motion preference means. Resolved by that one system rather than again here,
/// so the glow on the overlay and the shader behind it cannot disagree about
/// what the room chose.
///
/// Why the overlay needs telling at all: it is a THIRD document
/// (`gui/viewscreen-hud.html`), served statically and opened as its own surface,
/// so the presentation script injected into the lobby document's head never
/// reaches it — and an Ultralight view answers no `prefers-reduced-motion`
/// query, which leaves the page's own `@media` rule dead on native. Without this
/// push, choosing Flashes = Off on the native Display tab reached the shader but
/// not the vignette the room is looking at.
///
/// # Injection-safety invariant
///
/// The string is evaluated as script in that document, so it is injection-safe
/// **only because every interpolated value is a finite number clamped into
/// `0..=1`** — none can carry a quote, a backslash or a newline. The same
/// warning sits over `viewscreen_presentation::presentation_script`, and for the
/// same reason.
pub(super) fn hud_effects_script(shake: f32, flash: f32, decorative_motion: f32) -> String {
    format!(
        "window.__phoenixSetHudEffects({{\"shake\":{},\"flash\":{},\"decorativeMotion\":{}}})",
        effect_literal(shake),
        effect_literal(flash),
        effect_literal(decorative_motion),
    )
}

/// One intensity as a JS number literal: clamped into `0..=1`, and at the two
/// decimals the whole-percent record actually has, so the literal is short and
/// exact (`0.3`, never `0.30000001`). A value that is not a number at all reads
/// as FULL — a broken input must never silently take an effect away.
fn effect_literal(intensity: f32) -> String {
    let clamped = if intensity.is_finite() {
        intensity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut text = format!("{clamped:.2}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.push('0');
    }
    text
}

#[cfg(test)]
#[path = "hud_tests.rs"]
mod tests;
