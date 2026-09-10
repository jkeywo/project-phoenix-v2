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
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn revision(&self) -> u64 {
        self.revision
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
mod tests {
    use super::*;

    #[test]
    fn revealed_lobby_clears_the_hud_without_covering_tiled_consoles() {
        use crate::core::messages::GamePhase;
        use crate::native_host::host_lobby::RevealState;

        let console_layer = 0;
        let native_scene_overlay_layer = 5;
        let mut reveal = RevealState::new();
        for phase in [GamePhase::InProgress, GamePhase::GameOver] {
            reveal.observe_phase(&phase);
            let in_play = hud_z_index(reveal.presence().composited);
            assert!(in_play > native_scene_overlay_layer);

            // F9 reveals the same lobby, including its Settings cog and popup.
            // Both textures remain visible; the passive HUD yields its layer.
            let with_chrome = hud_z_index(reveal.toggle().composited);
            assert!(with_chrome < LOBBY_Z_INDEX);
            assert!(LOBBY_Z_INDEX < console_layer);

            // Hiding chrome restores the HUD over scene UI and ending content.
            assert_eq!(hud_z_index(reveal.toggle().composited), in_play);
        }
    }

    #[test]
    fn unchanged_hud_input_keeps_its_revision_and_retained_command() {
        let mut cache = HudScriptCache::default();
        assert_eq!(cache.revision(), 0);
        assert!(cache.script().is_none());
        assert!(cache.update(r#"{"heading":90}"#));
        assert_eq!(cache.revision(), 1);
        let encoded = cache.script().unwrap().to_owned();
        assert!(!cache.update(r#"{"heading":90}"#));
        assert_eq!(cache.revision(), 1);
        assert_eq!(cache.script(), Some(encoded.as_str()));
        assert!(cache.update(r#"{"heading":91}"#));
        assert_eq!(cache.revision(), 2);
        assert_ne!(cache.script(), Some(encoded.as_str()));
    }

    #[test]
    fn the_effects_half_rides_the_same_retained_command_and_goes_first() {
        // One command, because the worker re-applies the retained one to a view
        // that was reloaded or re-created — two channels would be two things to
        // re-apply. Effects FIRST, so a document hearing both in one evaluation
        // has its bands in force before the alert class flips.
        let mut cache = HudScriptCache::default();
        assert!(cache.set_effects(Some(hud_effects_script(1.0, 0.0, 1.0))));
        let effects_only = cache
            .script()
            .expect("a display can be told about its effects before any state arrives");
        assert!(effects_only.starts_with("window.__phoenixSetHudEffects("));
        assert!(!effects_only.contains("__updateHud"));

        assert!(cache.update(r#"{"heading":90}"#));
        let both = cache.script().expect("both halves");
        let effects_at = both
            .find("__phoenixSetHudEffects")
            .expect("the effects half");
        let update_at = both.find("__updateHud").expect("the state half");
        assert!(
            effects_at < update_at,
            "bands land before the state: {both}"
        );

        // A frame that changes neither half sends nothing.
        let revision = cache.revision();
        assert!(!cache.set_effects(Some(hud_effects_script(1.0, 0.0, 1.0))));
        assert!(!cache.update(r#"{"heading":90}"#));
        assert_eq!(cache.revision(), revision);

        // A press that moves one control is a new revision, and the state half
        // survives it untouched.
        assert!(cache.set_effects(Some(hud_effects_script(1.0, 1.0, 1.0))));
        assert_ne!(cache.revision(), revision);
        assert!(cache.script().expect("still both").contains("__updateHud"));
    }

    #[test]
    fn flashes_off_reaches_the_overlay_document_as_a_band_it_can_read() {
        // The claim the native Display tab's flash control makes: choosing Off
        // stops the red-alert pulse the room is looking at. On native that pulse
        // is drawn by `gui/viewscreen-hud.html`, whose only handle is this push
        // — the `@media (prefers-reduced-motion)` rule beside it is dead in an
        // Ultralight view. The band name is the page's: `hud.effectBand` maps
        // 0 to "off", and `:root[data-flash="off"]` holds the glow still.
        let script = hud_effects_script(1.0, 0.0, 1.0);
        assert_eq!(
            script,
            "window.__phoenixSetHudEffects({\"shake\":1.0,\"flash\":0.0,\"decorativeMotion\":1.0})"
        );

        // The gentler stop is a number the vignette divides its own period by,
        // not a switch — so it has to survive as a number.
        assert!(hud_effects_script(0.3, 0.3, 0.4).contains("\"flash\":0.3"));
        assert!(hud_effects_script(0.3, 0.3, 0.4).contains("\"decorativeMotion\":0.4"));
    }

    #[test]
    fn an_intensity_that_is_not_a_number_reads_as_full_rather_than_off() {
        // A broken input must never silently take an effect away: the same
        // answer `clampEffectIntensity` gives in gui/visual-effects.js, and the
        // same one the page's own stamper gives.
        assert!(hud_effects_script(f32::NAN, 4.0, -1.0).contains("\"shake\":1.0"));
        assert!(hud_effects_script(f32::NAN, 4.0, -1.0).contains("\"flash\":1.0"));
        assert!(hud_effects_script(f32::NAN, 4.0, -1.0).contains("\"decorativeMotion\":0.0"));
    }
}
