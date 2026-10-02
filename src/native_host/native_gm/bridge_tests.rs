use super::*;
use crate::native_host::panes::PaneSurfaceError;
#[test]
#[allow(clippy::disallowed_methods)] // Unique test-only filesystem sandbox, never simulation identity.
fn native_gm_audio_profile_reloads_on_its_host_scope_without_crew_identity() {
    let root = std::env::temp_dir().join(format!(
        "phoenix-gm-private-profile-{}",
        uuid::Uuid::new_v4()
    ));
    let bridge = NativeGmBridge::default();
    bridge.set_operator_scope("cruiser");
    bridge.lock().operators.root = Some(root.clone());
    bridge.activate(PaneId(1));
    let mut surface = Surface::default();
    surface
        .records
        .push(include_str!("../../../tests/fixtures/native-private-profile-save.json").into());
    bridge.pump(PaneId(1), &mut surface);
    assert!(bridge.take_records().is_empty());
    assert_eq!(
        bridge.lock().operators.scope.as_deref(),
        Some("native-gm:cruiser")
    );
    // A fresh process-style bridge uses the same established operator store.
    let relaunched = NativeGmBridge::default();
    relaunched.set_operator_scope("cruiser");
    relaunched.lock().operators.root = Some(root.clone());
    relaunched.activate(PaneId(2));
    surface
        .records
        .push(r#"{"type":"NativeOperator","operation":"load"}"#.into());
    relaunched.pump(PaneId(2), &mut surface);
    relaunched.pump(PaneId(2), &mut surface);
    let reply: serde_json::Value = surface
        .scripts
        .iter()
        .rev()
        .filter_map(|script| {
            let json = script
                .strip_prefix("window.__phoenixOperatorReply(")?
                .strip_suffix(')')?;
            serde_json::from_str::<serde_json::Value>(json).ok()
        })
        .find(|reply| reply["operation"] == "load")
        .expect("the real bridge returned the loaded operator profile");
    assert_eq!(reply["status"], "ok");
    let profile: serde_json::Value =
        serde_json::from_str(reply["profile"].as_str().unwrap()).unwrap();
    assert_eq!(profile["audio"]["mix"]["master"]["level"], 0.23);
    assert_eq!(profile["audio"]["mix"]["master"]["muted"], true);
    assert_eq!(profile["audio"]["cues"]["applied"], true);
    // A literal same-name crew operator files under its ordinary hull scope.
    let mut crew = crate::native_host::panes::operator::NativeOperators::default();
    crew.configure("cruiser");
    crew.root = Some(root.clone());
    crew.handle(
        PaneId(3),
        "native-gm",
        r#"{"type":"NativeOperator","operation":"load"}"#,
    );
    assert!(crew.replies[&PaneId(3)][0].contains(r#""profile":null"#));
    relaunched.set_operator_scope("courier");
    relaunched.lock().operators.root = Some(root.clone());
    surface.scripts.clear();
    surface
        .records
        .push(r#"{"type":"NativeOperator","operation":"load"}"#.into());
    relaunched.pump(PaneId(2), &mut surface);
    relaunched.pump(PaneId(2), &mut surface);
    assert!(surface
        .scripts
        .iter()
        .any(|script| script.contains(r#""profile":null"#)));
    assert!(!surface.scripts.iter().any(|script| script.contains("0.23")));
    assert!(root.starts_with(std::env::temp_dir()));
    std::fs::remove_dir_all(root).unwrap();
}
#[derive(Default)]
struct Surface {
    scripts: Vec<String>,
    records: Vec<String>,
    refuse: bool,
}
impl PaneSurface for Surface {
    fn load(&mut self, _: &str) -> Result<(), PaneSurfaceError> {
        Ok(())
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
        if self.refuse {
            return Err(PaneSurfaceError::Script("loading".into()));
        }
        self.scripts.push(script.into());
        Ok(())
    }
    fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.records)
    }
}
#[test]
fn replacement_surface_replays_latest_projection_and_rejects_old_surface_records() {
    let bridge = NativeGmBridge::default();
    bridge.publish("gm_session", "{\"paused\":true}".into());
    bridge.activate(PaneId(10));
    let mut surface = Surface::default();
    assert_eq!(bridge.pump(PaneId(10), &mut surface), 1);
    assert_eq!(bridge.pump(PaneId(10), &mut surface), 0);
    bridge.activate(PaneId(11));
    surface.records.push("stale action".into());
    assert_eq!(bridge.pump(PaneId(10), &mut surface), 0);
    assert!(bridge.take_records().is_empty());
    assert_eq!(bridge.pump(PaneId(11), &mut surface), 1);
    assert!(surface.scripts.last().unwrap().contains("paused"));
}
#[test]
fn load_retry_preserves_projection_and_failure_edge_survives_rebuild() {
    let bridge = NativeGmBridge::default();
    bridge.activate(PaneId(1));
    bridge.publish("metadata", "{}".into());
    let mut surface = Surface {
        refuse: true,
        ..Default::default()
    };
    assert_eq!(bridge.pump(PaneId(1), &mut surface), 0);
    surface.refuse = false;
    assert_eq!(bridge.pump(PaneId(1), &mut surface), 1);
    bridge.fault();
    bridge.activate(PaneId(2));
    assert!(bridge.take_failure());
    assert!(!bridge.take_failure());
    assert!(!bridge.live());
}

#[test]
fn reliable_overflow_faults_the_whole_batch_and_accepts_no_suffix() {
    for records in [
        vec!["action".into(); MAX_RECORDS + 1],
        vec!["x".repeat(MAX_RECORD_BYTES + 1)],
        vec!["x".repeat(MAX_RECORD_BYTES); 5],
    ] {
        let bridge = NativeGmBridge::default();
        bridge.activate(PaneId(1));
        bridge.mark_live();
        let mut surface = Surface {
            records,
            ..Default::default()
        };
        bridge.pump(PaneId(1), &mut surface);
        assert!(bridge.failed());
        assert!(bridge.take_failure());
        assert!(!bridge.live());
        assert!(bridge.take_records().is_empty());
        surface.records.push("suffix action".into());
        bridge.pump(PaneId(1), &mut surface);
        assert!(bridge.take_records().is_empty());
    }
}
