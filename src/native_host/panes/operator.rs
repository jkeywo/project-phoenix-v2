//! Private operator preferences and controller leases. Never simulation messages.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{json, Value};

use super::registry::PaneId;

const PROFILE_LIMIT: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct NativeOperators {
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

    /// A replacement view inherits active ownership before its page loads.
    /// Called while the pane bus holds its lifecycle mutex, so no competing
    /// console can acquire the controller between closing and rebuilding.
    pub fn transfer(&mut self, previous: PaneId, replacement: PaneId) {
        for (owner, _) in self.leases.values_mut() {
            if *owner == previous {
                *owner = replacement;
            }
        }
        self.replies.remove(&previous);
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

fn workshop_panel(value: &Value) -> bool {
    matches!(
        value.as_str(),
        Some("files" | "source" | "inspector" | "add" | "recovery")
    )
}

fn sanitize_workshop_node(
    value: &Value,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Option<Value>, ()> {
    if depth > 5 {
        return Err(());
    }
    let Some(node_type) = value["type"].as_str() else {
        return Ok(None);
    };
    match node_type {
        "tabs" => {
            let Some(raw_tabs) = value["tabs"].as_array() else {
                return Ok(None);
            };
            let mut tabs = Vec::new();
            for panel in raw_tabs.iter().filter(|panel| workshop_panel(panel)) {
                let panel = panel.as_str().unwrap();
                if seen.insert(panel.to_owned()) {
                    tabs.push(json!(panel));
                }
            }
            if tabs.is_empty() {
                return Ok(None);
            }
            let active = value
                .get("active")
                .filter(|active| tabs.contains(active))
                .cloned()
                .unwrap_or_else(|| tabs[0].clone());
            Ok(Some(
                json!({"type": "tabs", "tabs": tabs, "active": active}),
            ))
        }
        "split" if matches!(value["axis"].as_str(), Some("horizontal" | "vertical")) => {
            let Some(raw_children) = value["children"].as_array() else {
                return Ok(None);
            };
            let mut children = Vec::new();
            for child in raw_children.iter().take(5) {
                if let Some(child) = sanitize_workshop_node(child, seen, depth + 1)? {
                    children.push(child);
                }
            }
            if children.is_empty() {
                return Ok(None);
            }
            if children.len() == 1 {
                return Ok(children.pop());
            }
            let supplied = value["sizes"].as_array();
            let sizes: Vec<_> = (0..children.len())
                .map(|index| {
                    supplied
                        .and_then(|sizes| sizes.get(index))
                        .and_then(Value::as_f64)
                        .filter(|size| size.is_finite() && *size > 0.0)
                        .unwrap_or(1.0)
                })
                .map(|size| json!(size))
                .collect();
            Ok(Some(
                json!({"type": "split", "axis": value["axis"], "sizes": sizes, "children": children}),
            ))
        }
        _ => Ok(None),
    }
}

fn default_authoring_layout() -> Value {
    json!({
        "version": 2,
        "root": {"type":"split", "axis":"horizontal", "sizes":[22,56,22], "children":[
            {"type":"tabs", "tabs":["files"], "active":"files"},
            {"type":"tabs", "tabs":["source"], "active":"source"},
            {"type":"tabs", "tabs":["inspector","add","recovery"], "active":"inspector"}
        ]},
        "floats": [], "closed": [], "selected": "source"
    })
}

fn first_visible(node: &Value, floats: &[Value]) -> Option<Value> {
    match node["type"].as_str() {
        Some("tabs") => node.get("active").cloned(),
        Some("split") => node["children"]
            .as_array()?
            .iter()
            .find_map(|child| first_visible(child, &[])),
        _ => floats.first().and_then(|entry| entry.get("panel")).cloned(),
    }
}

fn sanitize_authoring_layout(value: &Value) -> Option<Value> {
    if value["version"] != 1 && value["version"] != 2 {
        return None;
    }
    let mut seen = BTreeSet::new();
    let root = match value.get("root")? {
        Value::Null => Value::Null,
        root => match sanitize_workshop_node(root, &mut seen, 0) {
            Ok(Some(root)) => root,
            Ok(None) => Value::Null,
            Err(()) => return Some(default_authoring_layout()),
        },
    };
    let mut floats = Vec::new();
    for entry in value["floats"].as_array().into_iter().flatten() {
        let Some(panel) = entry["panel"]
            .as_str()
            .filter(|_| workshop_panel(&entry["panel"]))
        else {
            continue;
        };
        if !seen.insert(panel.to_owned()) {
            continue;
        }
        let number = |name: &str, fallback: f64| entry[name].as_f64().unwrap_or(fallback);
        floats.push(json!({
            "panel": panel,
            "x": number("x", 12.0).max(0.0),
            "y": number("y", 12.0).max(0.0),
            "width": number("width", 420.0).max(240.0),
            "height": number("height", 360.0).max(180.0),
        }));
        if floats.len() == 5 {
            break;
        }
    }
    if !value["root"].is_null() && root.is_null() && floats.is_empty() {
        return Some(default_authoring_layout());
    }
    let mut closed = Vec::new();
    for panel in value["closed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|panel| workshop_panel(panel))
    {
        let panel = panel.as_str().unwrap();
        if seen.insert(panel.to_owned()) {
            closed.push(json!(panel));
        }
    }
    for panel in ["files", "source", "inspector", "add", "recovery"] {
        if seen.insert(panel.to_owned()) {
            closed.push(json!(panel));
        }
    }
    let selected = value
        .get("selected")
        .filter(|panel| workshop_panel(panel) && !closed.contains(panel))
        .cloned()
        .or_else(|| first_visible(&root, &floats))
        .unwrap_or_else(|| json!("files"));
    let mut layout = json!({"version": 2, "root": root, "floats": floats, "closed": closed, "selected": selected});
    if value["version"] == 1 {
        layout = migrate_authoring_layout(layout);
    }
    Some(layout)
}

fn add_workshop_tab(node: &mut Value, target: &str, panel: &str) -> bool {
    match node["type"].as_str() {
        Some("tabs")
            if node["tabs"].as_array().is_some_and(|tabs| {
                tabs.iter()
                    .any(|candidate| candidate.as_str() == Some(target))
            }) =>
        {
            node["tabs"].as_array_mut().unwrap().push(json!(panel));
            node["active"] = json!(panel);
            true
        }
        Some("split") => node["children"].as_array_mut().is_some_and(|children| {
            children
                .iter_mut()
                .any(|child| add_workshop_tab(child, target, panel))
        }),
        _ => false,
    }
}

fn remove_workshop_panel(node: &Value, panel: &str) -> Value {
    match node["type"].as_str() {
        Some("tabs") => {
            let tabs: Vec<_> = node["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|tab| tab.as_str() != Some(panel))
                .cloned()
                .collect();
            if tabs.is_empty() {
                Value::Null
            } else {
                let active = node["active"]
                    .as_str()
                    .filter(|active| tabs.iter().any(|tab| tab.as_str() == Some(active)))
                    .map_or_else(|| tabs[0].clone(), Value::from);
                json!({"type":"tabs", "tabs":tabs, "active":active})
            }
        }
        Some("split") => {
            let sizes = node["sizes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let survivors: Vec<_> = node["children"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, child)| {
                    let child = remove_workshop_panel(child, panel);
                    (!child.is_null()).then(|| {
                        (
                            child,
                            sizes.get(index).and_then(Value::as_f64).unwrap_or(1.0),
                        )
                    })
                })
                .collect();
            match survivors.len() {
                0 => Value::Null,
                1 => survivors[0].0.clone(),
                _ => {
                    let previous_total: f64 = sizes.iter().filter_map(Value::as_f64).sum();
                    let surviving_total: f64 = survivors.iter().map(|(_, size)| size).sum();
                    json!({
                        "type":"split",
                        "axis":node["axis"],
                        "sizes":survivors.iter().map(|(_, size)| size * previous_total / surviving_total).collect::<Vec<_>>(),
                        "children":survivors.into_iter().map(|(child, _)| child).collect::<Vec<_>>()
                    })
                }
            }
        }
        _ => Value::Null,
    }
}

fn dock_workshop_tab(layout: &mut Value, panel: &str, target: &str) {
    let original = layout.clone();
    layout["root"] = remove_workshop_panel(&layout["root"], panel);
    layout["floats"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["panel"].as_str() != Some(panel));
    layout["closed"]
        .as_array_mut()
        .unwrap()
        .retain(|value| value.as_str() != Some(panel));
    if add_workshop_tab(&mut layout["root"], target, panel) {
        return;
    }
    let Some(index) = layout["floats"].as_array().and_then(|floats| {
        floats
            .iter()
            .position(|entry| entry["panel"].as_str() == Some(target))
    }) else {
        *layout = original;
        return;
    };
    layout["floats"].as_array_mut().unwrap().remove(index);
    let joined = json!({"type":"tabs", "tabs":[target, panel], "active":panel});
    layout["root"] = if layout["root"].is_null() {
        joined
    } else {
        json!({"type":"split", "axis":"horizontal", "sizes":[3,1], "children":[layout["root"].take(), joined]})
    };
}

fn migrate_authoring_layout(mut layout: Value) -> Value {
    let selected = layout["selected"].clone();
    let target = if !layout["closed"]
        .as_array()
        .is_some_and(|closed| closed.contains(&json!("inspector")))
    {
        Some("inspector".to_owned())
    } else {
        first_visible(
            &layout["root"],
            layout["floats"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )
        .and_then(|panel| panel.as_str().map(str::to_owned))
    };
    let Some(target) = target else {
        return default_authoring_layout();
    };
    for panel in ["add", "recovery"] {
        dock_workshop_tab(&mut layout, panel, &target);
    }
    layout["selected"] = selected;
    layout
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
    // Portable private audio choices, never hardware routes or cue records.
    // Keep absent audio absent so the JS owner can perform its legacy migration.
    if raw["audio"].is_object() {
        safe["audio"] = json!({"version": 1, "mono": raw["audio"]["mono"].as_bool().unwrap_or(false),
            "reducedRange": raw["audio"]["reducedRange"].as_bool().unwrap_or(false), "mix": {}, "cues": {}});
        for bus in [
            "master",
            "music",
            "ambience",
            "effects",
            "alerts",
            "interface",
        ] {
            let value = &raw["audio"]["mix"][bus];
            safe["audio"]["mix"][bus] = json!({
                "level": value["level"].as_f64().filter(|v| v.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0),
                "muted": value["muted"].as_bool().unwrap_or(false),
            });
        }
        for (cue, default) in [
            ("clicks", true),
            ("refused", true),
            ("timedOut", true),
            ("applied", false),
            ("pending", false),
            ("actionable", true),
        ] {
            safe["audio"]["cues"][cue] =
                json!(raw["audio"]["cues"][cue].as_bool().unwrap_or(default));
        }
    }
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
    if let Some(layout) = sanitize_authoring_layout(&raw["authoringLayout"]) {
        safe["authoringLayout"] = layout;
    }
    serde_json::to_string_pretty(&safe).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Random UUIDs isolate host-local temporary test directories.
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
            "bindings":{"helm.thrust":[{"type":"gamepad", "input":"axis", "control":"left-stick-y", "token":"secret"}, null]},
            "audio":{"version":1,"mono":true,"reducedRange":true,"output":"secret","history":["secret"],
                "mix":{"master":{"level":0.12,"muted":true},"music":{"level":0.5}},
                "cues":{"applied":true,"unknown":"secret"}},
            "authoringLayout":{"version":1,"root":{"type":"tabs","tabs":["source"],"active":"source","unsafe":"secret"},"floats":[],"closed":["files","inspector"],"selected":"source","unsafe":"secret"}});
        state.handle(
            PaneId(1),
            "helm",
            &json!({"type":"NativeOperator", "operation":"save", "profile":profile.to_string()})
                .to_string(),
        );
        let text = std::fs::read_to_string(state.path("helm").unwrap()).unwrap();
        assert!(!text.contains("secret"));
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            saved["audio"]["mix"]["master"],
            json!({"level":0.12,"muted":true})
        );
        assert_eq!(
            saved["audio"]["mix"]["music"],
            json!({"level":0.5,"muted":false})
        );
        assert_eq!(saved["audio"]["cues"]["applied"], true);
        assert_eq!(saved["audio"]["mono"], true);
        assert_eq!(saved["audio"]["reducedRange"], true);
        assert_eq!(saved["authoringLayout"]["selected"], "source");
        assert!(saved["authoringLayout"].get("unsafe").is_none());
        state.handle(
            PaneId(2),
            "helm",
            r#"{"type":"NativeOperator","operation":"load"}"#,
        );
        assert!(state.replies[&PaneId(2)][0].contains("left-stick-y"));
        assert!(state.replies[&PaneId(2)][0].contains("0.12"));
        assert_ne!(state.path("../helm"), state.path("helm"));
        state.scope = Some("destroyer".into());
        assert!(!state.path("helm").unwrap().exists());
    }

    #[test]
    fn over_depth_authoring_layout_is_rejected_as_a_whole() {
        let mut root = json!({"type":"tabs","tabs":["source"],"active":"source"});
        for _ in 0..6 {
            root = json!({"type":"split","axis":"horizontal","sizes":[1],"children":[root]});
        }
        let profile = json!({
            "kind":"project-phoenix/operator-profile",
            "version":1,
            "authoringLayout":{
                "version":2,
                "root":{
                    "type":"split","axis":"horizontal","sizes":[1,1],
                    "children":[
                        {"type":"tabs","tabs":["files"],"active":"files"},
                        root
                    ]
                },
                "floats":[],"closed":[],"selected":"files"
            }
        });

        let saved: Value =
            serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
        assert_eq!(saved["authoringLayout"], default_authoring_layout());
    }

    #[test]
    fn missing_authoring_root_is_rejected_for_default_recovery() {
        let profile = json!({
            "kind":"project-phoenix/operator-profile",
            "version":1,
            "authoringLayout":{"version":1,"floats":[],"closed":[],"selected":"source"}
        });

        let saved: Value =
            serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
        assert!(saved.get("authoringLayout").is_none());
    }

    #[test]
    fn malformed_authoring_root_is_rejected_for_default_recovery() {
        let profile = json!({
            "kind":"project-phoenix/operator-profile",
            "version":1,
            "authoringLayout":{
                "version":2,
                "root":{"type":"unknown"},
                "floats":[],"closed":[],"selected":"source"
            }
        });

        let saved: Value =
            serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
        assert_eq!(saved["authoringLayout"], default_authoring_layout());
    }

    #[test]
    fn explicit_null_authoring_root_preserves_all_panels_closed_state() {
        let profile = json!({
            "kind":"project-phoenix/operator-profile",
            "version":1,
            "authoringLayout":{
                "version":2,
                "root":null,
                "floats":[],
                "closed":["files","source","inspector","add","recovery"],
                "selected":"source"
            }
        });

        let saved: Value =
            serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
        assert_eq!(saved["authoringLayout"]["selected"], "files");
    }

    #[test]
    fn authoring_layout_normalization_matches_browser_global_deduplication() {
        let layout = json!({
            "version":2,
            "root":null,
            "floats":[
                {"panel":"source"}, {"panel":"source"}, {"panel":"source"},
                {"panel":"inspector","x":30,"y":40}, {"panel":"files"}
            ],
            "closed":["source","inspector","files"], "selected":"inspector"
        });

        assert_eq!(
            sanitize_authoring_layout(&layout).unwrap(),
            json!({
                "version":2,
                "root":null,
                "floats":[
                    {"panel":"source","x":12.0,"y":12.0,"width":420.0,"height":360.0},
                    {"panel":"inspector","x":30.0,"y":40.0,"width":420.0,"height":360.0},
                    {"panel":"files","x":12.0,"y":12.0,"width":420.0,"height":360.0}
                ],
                "closed":["add","recovery"], "selected":"inspector"
            })
        );
    }

    #[test]
    fn legacy_authoring_layout_adds_placement_only_for_lifecycle_panels() {
        let layout = json!({
            "version":1,
            "root":{"type":"tabs","tabs":["files","inspector"],"active":"files"},
            "floats":[],"closed":["source"],"selected":"files",
            "recovery":{"draft":"must not persist"}
        });

        let migrated = sanitize_authoring_layout(&layout).unwrap();
        assert_eq!(migrated["version"], 2);
        assert_eq!(
            migrated["root"]["tabs"],
            json!(["files", "inspector", "add", "recovery"])
        );
        assert_eq!(migrated["selected"], "files");
        assert!(migrated.get("recovery").is_none());
    }

    #[test]
    fn legacy_authoring_layout_rehomes_crafted_lifecycle_panels_without_duplicates() {
        let layout = json!({
            "version":1,
            "root":{"type":"split","axis":"horizontal","sizes":[10,30,60],"children":[
                {"type":"tabs","tabs":["files","add"],"active":"add"},
                {"type":"tabs","tabs":["source"],"active":"source"},
                {"type":"tabs","tabs":["inspector"],"active":"inspector"}
            ]},
            "floats":[{"panel":"recovery","x":7,"y":9,"width":300,"height":200}],
            "closed":[],"selected":"source"
        });

        assert_eq!(
            sanitize_authoring_layout(&layout).unwrap(),
            json!({
                "version":2,
                "root":{"type":"split","axis":"horizontal","sizes":[10.0,30.0,60.0],"children":[
                    {"type":"tabs","tabs":["files"],"active":"files"},
                    {"type":"tabs","tabs":["source"],"active":"source"},
                    {"type":"tabs","tabs":["inspector","add","recovery"],"active":"recovery"}
                ]},
                "floats":[],"closed":[],"selected":"source"
            })
        );
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
