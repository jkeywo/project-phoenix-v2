//! Latest HUD command, encoded only when its source value changes.
//!
//! The main world owns this cache. The worker owns each view's last successful
//! application, so a new/revealed/resized view can request the retained command
//! without making the producer encode it again.

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
