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

#[derive(Debug, Default)]
pub(crate) struct HudScriptCache {
    json: Option<String>,
    script: Option<String>,
    revision: u64,
}

impl HudScriptCache {
    pub fn update(&mut self, json: &str) -> bool {
        if self.json.as_deref() == Some(json) {
            return false;
        }
        self.script = Some(crate::core::codec::encode_hud_update_script(json));
        self.json = Some(json.to_owned());
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }
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
}
