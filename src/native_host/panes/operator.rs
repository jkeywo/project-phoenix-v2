//! Private operator preferences and controller leases. Never simulation messages.
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::registry::PaneId;

const PROFILE_LIMIT: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct NativeOperators {
    pub scope: Option<String>,
    pub root: Option<PathBuf>,
    pub replies: BTreeMap<PaneId, Vec<String>>,
    leases: BTreeMap<usize, (PaneId, String)>,
    devices: BTreeMap<usize, String>,
}

impl NativeOperators {
    pub fn configure(&mut self, scope: &str) -> bool {
        if self.scope.as_deref() == Some(scope) {
            return false;
        }
        self.scope = Some(scope.to_owned());
        self.root = directories::BaseDirs::new().map(|base| {
            base.data_dir()
                .join("ProjectPhoenix")
                .join("operator-profiles")
        });
        self.leases.clear();
        true
    }

    pub fn close(&mut self, pane: PaneId) {
        self.leases.retain(|_, (owner, _)| *owner != pane);
        self.replies.remove(&pane);
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        // Encoding bytes preserves distinct labels and cannot introduce path components.
        let encode = |value: &str| {
            value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        Some(
            self.root
                .as_ref()?
                .join(encode(self.scope.as_ref()?))
                .join(format!("{}.json", encode(name))),
        )
    }

    pub fn handle(&mut self, pane: PaneId, name: &str, record: &str) -> bool {
        if record.len() > PROFILE_LIMIT {
            return false;
        }
        let Ok(raw) = serde_json::from_str::<Value>(record) else {
            return false;
        };
        if raw["type"] != "NativeOperator" {
            return false;
        }
        let mut reply = json!({ "operation": raw["operation"], "status": "ok" });
        match raw["operation"].as_str() {
            Some("load") => {
                let loaded = self
                    .path(name)
                    .ok_or_else(|| "Profile storage is unavailable".to_owned())
                    .and_then(|path| match std::fs::read_to_string(path) {
                        Ok(text) if text.len() <= PROFILE_LIMIT => {
                            sanitize_profile(&text).map(Some)
                        }
                        Ok(_) => Err("Stored profile is too large".to_owned()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(e) => Err(e.to_string()),
                    });
                match loaded {
                    Ok(profile) => {
                        reply["profile"] = profile.map(Value::String).unwrap_or(Value::Null)
                    }
                    Err(error) => {
                        reply["status"] = json!("error");
                        reply["error"] = json!(error);
                    }
                }
            }
            Some("save") => {
                let result = raw["profile"]
                    .as_str()
                    .ok_or_else(|| "Expected profile JSON".to_owned())
                    .and_then(sanitize_profile)
                    .and_then(|profile| {
                        let path = self
                            .path(name)
                            .ok_or_else(|| "Profile storage is unavailable".to_owned())?;
                        super::super::layout_store::write_atomically(&path, &profile)
                            .map_err(|e| e.to_string())
                    });
                if let Err(error) = result {
                    reply["status"] = json!("error");
                    reply["error"] = json!(error);
                }
            }
            Some("select") => {
                let selected = raw["index"].as_u64().and_then(|n| usize::try_from(n).ok());
                if let Some(slot) = selected {
                    if !self.devices.contains_key(&slot)
                        || self
                            .leases
                            .get(&slot)
                            .is_some_and(|(owner, _)| *owner != pane)
                    {
                        reply["status"] = json!("refused");
                    } else {
                        self.leases.retain(|_, (owner, _)| *owner != pane);
                        self.leases.insert(slot, (pane, name.to_owned()));
                    }
                } else if raw["index"].is_null() {
                    self.leases.retain(|_, (owner, _)| *owner != pane);
                } else {
                    reply["status"] = json!("refused");
                }
            }
            _ => return true,
        }
        let queue = self.replies.entry(pane).or_default();
        // Preferences are latest-state operations. Bound a page that never drains.
        if queue.len() == 32 {
            queue.remove(0);
        }
        queue.push(format!("window.__phoenixOperatorReply({reply})"));
        true
    }

    pub fn observe(&mut self, script: &str) {
        let Some(pads) = snapshot(script) else {
            return;
        };
        let devices: BTreeMap<_, _> = pads
            .iter()
            .filter_map(|pad| {
                Some((
                    usize::try_from(pad["index"].as_u64()?).ok()?,
                    pad["id"].as_str()?.to_owned(),
                ))
            })
            .collect();
        self.leases.retain(|slot, _| {
            self.devices.get(slot) == devices.get(slot) && devices.contains_key(slot)
        });
        self.devices = devices;
    }

    pub fn snapshot_for(&self, script: &str, pane: PaneId) -> String {
        let Some(mut pads) = snapshot(script) else {
            return script.to_owned();
        };
        for pad in pads.iter_mut().filter(|pad| !pad.is_null()) {
            let slot = pad["index"].as_u64().unwrap_or(u64::MAX) as usize;
            let lease = self.leases.get(&slot);
            let owned = lease.is_some_and(|(owner, _)| *owner == pane);
            pad["nativeOwned"] = json!(owned);
            pad["available"] = json!(lease.is_none() || owned);
            if let Some((_, name)) = lease {
                pad["assignedTo"] = json!(name);
            }
            if !owned {
                if let Some(buttons) = pad["buttons"].as_array_mut() {
                    for button in buttons {
                        *button = json!({"pressed": false, "value": 0});
                    }
                }
                if let Some(axes) = pad["axes"].as_array_mut() {
                    axes.fill(json!(0));
                }
            }
        }
        format!("window.__phoenixSetGamepads({})", Value::Array(pads))
    }
}

fn snapshot(script: &str) -> Option<Vec<Value>> {
    serde_json::from_str(
        script
            .strip_prefix("window.__phoenixSetGamepads(")?
            .strip_suffix(')')?,
    )
    .ok()
}

fn fields(value: &Value, names: &[&str]) -> Value {
    Value::Object(
        names
            .iter()
            .filter_map(|name| {
                value
                    .get(*name)
                    .filter(|v| !v.is_object() && !v.is_array())
                    .map(|v| ((*name).to_owned(), v.clone()))
            })
            .collect(),
    )
}

/// Apply the same field boundary as operator-profile.js before any disk write.
fn sanitize_profile(text: &str) -> Result<String, String> {
    if text.len() > PROFILE_LIMIT {
        return Err("Profile is too large".into());
    }
    let raw: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if raw["kind"] != "project-phoenix/operator-profile" || raw["version"] != 1 {
        return Err("Unsupported operator profile".into());
    }
    let mut safe = fields(&raw, &["kind", "version"]);
    safe["accessibility"] = json!({
        "presentation": fields(&raw["accessibility"]["presentation"], &["textScale", "contrast", "reducedMotion"]),
        "assistance": fields(&raw["accessibility"]["assistance"], &["helm.course-keeping", "tactical.target-selection", "sensors.contact-triage", "comms.dialogue-timing"]),
    });
    safe["bindings"] = json!({});
    if let Some(bindings) = raw["bindings"].as_object() {
        for (id, slots) in bindings.iter().take(512) {
            if let Some(slots) = slots.as_array() {
                safe["bindings"][id] = Value::Array(
                    slots
                        .iter()
                        .take(2)
                        .map(|binding| {
                            if binding.is_null() {
                                Value::Null
                            } else {
                                fields(
                                    binding,
                                    &[
                                        "type",
                                        "input",
                                        "control",
                                        "direction",
                                        "threshold",
                                        "code",
                                        "ctrlKey",
                                        "shiftKey",
                                        "altKey",
                                        "metaKey",
                                    ],
                                )
                            }
                        })
                        .collect(),
                );
            }
        }
    }
    safe["gamepad"] = fields(&raw["gamepad"], &["preferredSlot", "hideTouchControls"]);
    safe["gamepad"]["preferredDevice"] = if raw["gamepad"]["preferredDevice"].is_null() {
        Value::Null
    } else {
        fields(&raw["gamepad"]["preferredDevice"], &["id", "mapping"])
    };
    safe["gamepad"]["tuning"] = json!({});
    if let Some(tuning) = raw["gamepad"]["tuning"].as_object() {
        for (id, value) in tuning.iter().take(512) {
            safe["gamepad"]["tuning"][id] = fields(value, &["deadzone", "inverted"]);
        }
    }
    safe["feedback"] = fields(&raw["feedback"], &["vibration", "semanticCues"]);
    safe["gmConfirmations"] = json!({});
    if let Some(confirmations) = raw["gmConfirmations"].as_object() {
        for (id, value) in confirmations.iter().take(512) {
            if matches!(
                value.as_str(),
                Some("immediate" | "confirm" | "confirm-preview")
            ) {
                safe["gmConfirmations"][id] = value.clone();
            }
        }
    }
    serde_json::to_string_pretty(&safe).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("phoenix-operator-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
    const PADS: &str = "window.__phoenixSetGamepads([{\"index\":0,\"id\":\"pad\",\"buttons\":[{\"pressed\":true,\"value\":1}],\"axes\":[1]}])";
    #[test]
    fn host_excludes_other_consoles_and_neutralizes_unowned_snapshots() {
        let mut state = NativeOperators::default();
        state.observe(PADS);
        state.handle(
            PaneId(1),
            "helm",
            r#"{"type":"NativeOperator","operation":"select","index":0}"#,
        );
        state.handle(
            PaneId(2),
            "tactical",
            r#"{"type":"NativeOperator","operation":"select","index":0}"#,
        );
        let owned = snapshot(&state.snapshot_for(PADS, PaneId(1))).unwrap();
        let other = snapshot(&state.snapshot_for(PADS, PaneId(2))).unwrap();
        assert_eq!(owned[0]["axes"][0], 1);
        assert_eq!(other[0]["axes"][0], 0);
        assert_eq!(other[0]["assignedTo"], "helm");
        assert_eq!(other[0]["available"], false);
        state.close(PaneId(1));
        assert_eq!(
            snapshot(&state.snapshot_for(PADS, PaneId(2))).unwrap()[0]["available"],
            true
        );
    }
    #[test]
    fn disconnect_releases_controller_and_reconnect_needs_a_new_claim() {
        let mut state = NativeOperators::default();
        state.observe(PADS);
        state.handle(
            PaneId(1),
            "helm",
            r#"{"type":"NativeOperator","operation":"select","index":0}"#,
        );
        state.observe("window.__phoenixSetGamepads([])");
        state.observe(PADS);
        assert_eq!(
            snapshot(&state.snapshot_for(PADS, PaneId(1))).unwrap()[0]["nativeOwned"],
            false
        );
    }
    #[test]
    fn preferences_round_trip_by_hull_and_label_without_credentials() {
        let dir = Scratch::new();
        let mut state = NativeOperators {
            root: Some(dir.0.clone()),
            scope: Some("cruiser".into()),
            ..Default::default()
        };
        let profile = json!({"kind":"project-phoenix/operator-profile", "version":1,
            "token":"secret", "gamepad":{"preferredDevice":{"id":"pad", "mapping":"standard", "token":"secret"}, "hideTouchControls":false},
            "bindings":{"helm.thrust":[{"type":"gamepad", "input":"axis", "control":"left-stick-y", "token":"secret"}, null]}});
        state.handle(
            PaneId(1),
            "helm",
            &json!({"type":"NativeOperator", "operation":"save", "profile":profile.to_string()})
                .to_string(),
        );
        let text = std::fs::read_to_string(state.path("helm").unwrap()).unwrap();
        assert!(!text.contains("secret"));
        state.handle(
            PaneId(2),
            "helm",
            r#"{"type":"NativeOperator","operation":"load"}"#,
        );
        assert!(state.replies[&PaneId(2)][0].contains("left-stick-y"));
        assert_ne!(state.path("../helm"), state.path("helm"));
        state.scope = Some("destroyer".into());
        assert!(!state.path("helm").unwrap().exists());
    }

    #[test]
    fn corrupt_and_unwritable_profiles_report_errors_without_breaking_controllers() {
        let dir = Scratch::new();
        let mut state = NativeOperators {
            root: Some(dir.0.clone()),
            scope: Some("cruiser".into()),
            ..Default::default()
        };
        let path = state.path("helm").unwrap();
        super::super::super::layout_store::write_atomically(&path, "corrupt").unwrap();
        state.handle(
            PaneId(1),
            "helm",
            r#"{"type":"NativeOperator","operation":"load"}"#,
        );
        assert!(state.replies[&PaneId(1)][0].contains("error"));
        let blocked = dir.0.join("blocked");
        std::fs::write(&blocked, "a file cannot be a settings directory").unwrap();
        state.root = Some(blocked);
        state.handle(
            PaneId(1),
            "helm",
            &json!({"type":"NativeOperator", "operation":"save", "profile":
            json!({"kind":"project-phoenix/operator-profile", "version":1}).to_string()})
            .to_string(),
        );
        assert!(state.replies[&PaneId(1)][1].contains("error"));
        state.observe(PADS);
        state.handle(
            PaneId(1),
            "helm",
            r#"{"type":"NativeOperator","operation":"select","index":0}"#,
        );
        assert_eq!(
            snapshot(&state.snapshot_for(PADS, PaneId(1))).unwrap()[0]["nativeOwned"],
            true
        );
    }
}
