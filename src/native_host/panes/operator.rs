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

/// The Authoring panel vocabulary of each stored layout version, mirroring
/// `gui/workshop-layout-model.js`. A tree is sanitized against the vocabulary
/// its own version had, so a panel registered later can never be read back out
/// of an older profile: it only ever enters through migration.
const WORKSHOP_PANELS_V2: &[&str] = &["files", "source", "inspector", "add", "recovery"];
const WORKSHOP_PANELS_V3: &[&str] = &[
    "files",
    "source",
    "inspector",
    "add",
    "recovery",
    "findings",
    "feedback",
    "dependencies",
    "settings",
];
const WORKSHOP_PANELS_V4: &[&str] = &[
    "files",
    "source",
    "inspector",
    "add",
    "recovery",
    "findings",
    "feedback",
    "dependencies",
    "settings",
    "models",
    "model-preview",
    "sound",
];
/// Panels registered after a stored version, with the group each joins on migration.
const WORKSHOP_ADDED_IN_V3: &[(&str, &str)] = &[
    ("dependencies", "files"),
    ("findings", "source"),
    ("feedback", "source"),
    ("settings", "inspector"),
];
const WORKSHOP_ADDED_IN_V4: &[(&str, &str)] = &[
    ("models", "inspector"),
    ("model-preview", "source"),
    ("sound", "inspector"),
];

fn workshop_panels_for(version: u64) -> &'static [&'static str] {
    match version {
        1 | 2 => WORKSHOP_PANELS_V2,
        3 => WORKSHOP_PANELS_V3,
        _ => WORKSHOP_PANELS_V4,
    }
}

fn workshop_panels_added_after(version: u64) -> Vec<(&'static str, &'static str)> {
    let mut added = Vec::new();
    if version < 3 {
        added.extend_from_slice(WORKSHOP_ADDED_IN_V3);
    }
    if version < 4 {
        added.extend_from_slice(WORKSHOP_ADDED_IN_V4);
    }
    added
}

fn known_panel(value: &Value, allowed: &[&str]) -> bool {
    value.as_str().is_some_and(|panel| allowed.contains(&panel))
}

const LIVE_PANELS_V1: &[&str] = &["roster", "readiness", "join", "manual-save"];
const LIVE_PANELS_V2: &[&str] = &[
    "roster",
    "readiness",
    "join",
    "manual-save",
    "mission",
    "comms",
    "activity",
    "journal",
    "session-history",
];
/// Panels registered after version 1, with the group each joins and how.
/// Comms opens a group BELOW the readiness panels: the record surfaces are one
/// reading surface on this desk and always were.
const LIVE_ADDED_IN_V2: &[(&str, &str, &str)] = &[
    ("mission", "roster", "tab"),
    ("comms", "roster", "bottom"),
    ("activity", "comms", "tab"),
    ("journal", "comms", "tab"),
    ("session-history", "comms", "tab"),
];

fn live_panels_for(version: u64) -> &'static [&'static str] {
    if version < 2 {
        LIVE_PANELS_V1
    } else {
        LIVE_PANELS_V2
    }
}

fn sanitize_workshop_node(
    value: &Value,
    seen: &mut BTreeSet<String>,
    depth: usize,
    allowed: &[&str],
) -> Result<Option<Value>, ()> {
    if depth > allowed.len() {
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
            for panel in raw_tabs.iter().filter(|panel| known_panel(panel, allowed)) {
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
            for child in raw_children.iter().take(allowed.len()) {
                if let Some(child) = sanitize_workshop_node(child, seen, depth + 1, allowed)? {
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
        "version": 4,
        "root": {"type":"split", "axis":"horizontal", "sizes":[22,56,22], "children":[
            {"type":"tabs", "tabs":["files","dependencies"], "active":"files"},
            {"type":"tabs", "tabs":["source","findings","feedback","model-preview"], "active":"source"},
            {"type":"tabs", "tabs":["inspector","add","recovery","settings","models","sound"], "active":"inspector"}
        ]},
        "floats": [], "closed": [], "selected": "source"
    })
}

fn default_live_layout() -> Value {
    json!({
        "version": 2,
        "root": {"type":"split", "axis":"vertical", "sizes":[1.0,1.0], "children":[
            {"type":"tabs", "tabs":["roster","readiness","join","manual-save","mission"], "active":"roster"},
            {"type":"tabs", "tabs":["comms","activity","journal","session-history"], "active":"comms"}
        ]},
        "floats": [], "closed": [], "selected": "roster"
    })
}

fn sanitize_live_layout(value: &Value) -> Option<Value> {
    let stored = value["version"].as_u64().filter(|v| (1..=2).contains(v))?;
    let allowed = live_panels_for(stored);
    let mut seen = BTreeSet::new();
    let root = match value.get("root")? {
        Value::Null => Value::Null,
        root => match sanitize_live_node(root, &mut seen, 0, allowed) {
            Ok(Some(root)) => root,
            Ok(None) => Value::Null,
            Err(()) => return Some(default_live_layout()),
        },
    };
    let mut floats = Vec::new();
    for entry in value["floats"].as_array().into_iter().flatten() {
        let Some(panel) = entry["panel"]
            .as_str()
            .filter(|_| known_panel(&entry["panel"], allowed))
        else {
            continue;
        };
        if !seen.insert(panel.to_owned()) {
            continue;
        }
        let number = |name: &str, fallback: f64| entry[name].as_f64().unwrap_or(fallback);
        floats.push(
            json!({"panel":panel, "x":number("x",12.0).max(0.0), "y":number("y",12.0).max(0.0),
            "width":number("width",420.0).max(240.0), "height":number("height",360.0).max(180.0)}),
        );
        if floats.len() == allowed.len() {
            break;
        }
    }
    if !value["root"].is_null() && root.is_null() && floats.is_empty() {
        return Some(default_live_layout());
    }
    let mut closed = Vec::new();
    for panel in value["closed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|panel| known_panel(panel, allowed))
    {
        let id = panel.as_str().unwrap();
        if seen.insert(id.to_owned()) {
            closed.push(json!(id));
        }
    }
    for panel in allowed {
        if seen.insert((*panel).to_owned()) {
            closed.push(json!(panel));
        }
    }
    let selected = value
        .get("selected")
        .filter(|panel| known_panel(panel, allowed) && !closed.contains(panel))
        .cloned()
        .or_else(|| first_visible(&root, &floats))
        .unwrap_or_else(|| json!("roster"));
    let mut layout = json!({"version":stored, "root":root, "floats":floats, "closed":closed, "selected":selected});
    if stored < 2 {
        layout = migrate_live_layout(layout);
    }
    Some(layout)
}

/// Place the panels registered after the stored version, exactly as
/// `gui/dock-layout-migration.js` does.
fn migrate_live_layout(mut layout: Value) -> Value {
    let selected = layout["selected"].clone();
    let mut previous_actives = BTreeMap::new();
    collect_workshop_actives(&layout["root"], &mut previous_actives);
    layout["closed"]
        .as_array_mut()
        .unwrap()
        .extend(LIVE_ADDED_IN_V2.iter().map(|(panel, _, _)| json!(panel)));
    layout["version"] = json!(2);
    for (panel, preferred, placement) in LIVE_ADDED_IN_V2 {
        add_migration_panel_at(&mut layout, panel, preferred, &Value::Null, placement);
    }
    let added: Vec<&str> = LIVE_ADDED_IN_V2
        .iter()
        .map(|(panel, _, _)| *panel)
        .collect();
    settle_new_groups(&mut layout["root"], &added);
    restore_workshop_actives(&mut layout["root"], &previous_actives);
    layout["selected"] = selected;
    layout
}

/// A group made entirely of panels this migration introduced shows its FIRST
/// panel, not whichever one happened to be docked last.
fn settle_new_groups(node: &mut Value, added: &[&str]) {
    match node["type"].as_str() {
        Some("tabs") => {
            let all_new = node["tabs"].as_array().is_some_and(|tabs| {
                tabs.iter()
                    .all(|tab| tab.as_str().is_some_and(|tab| added.contains(&tab)))
            });
            if all_new {
                if let Some(first) = node["tabs"]
                    .as_array()
                    .and_then(|tabs| tabs.first())
                    .cloned()
                {
                    node["active"] = first;
                }
            }
        }
        Some("split") => {
            if let Some(children) = node["children"].as_array_mut() {
                for child in children {
                    settle_new_groups(child, added);
                }
            }
        }
        _ => {}
    }
}

fn sanitize_live_node(
    value: &Value,
    seen: &mut BTreeSet<String>,
    depth: usize,
    allowed: &[&str],
) -> Result<Option<Value>, ()> {
    if depth > allowed.len() {
        return Err(());
    }
    match value["type"].as_str() {
        Some("tabs") => {
            let Some(raw) = value["tabs"].as_array() else {
                return Ok(None);
            };
            let tabs: Vec<_> = raw
                .iter()
                .filter(|panel| known_panel(panel, allowed))
                .filter_map(|panel| {
                    let id = panel.as_str()?;
                    seen.insert(id.to_owned()).then(|| json!(id))
                })
                .collect();
            if tabs.is_empty() {
                return Ok(None);
            }
            let active = value
                .get("active")
                .filter(|active| tabs.contains(active))
                .cloned()
                .unwrap_or_else(|| tabs[0].clone());
            Ok(Some(json!({"type":"tabs", "tabs":tabs, "active":active})))
        }
        Some("split") if matches!(value["axis"].as_str(), Some("horizontal" | "vertical")) => {
            let Some(raw) = value["children"].as_array() else {
                return Ok(None);
            };
            let mut children = Vec::new();
            for child in raw.iter().take(allowed.len()) {
                if let Some(child) = sanitize_live_node(child, seen, depth + 1, allowed)? {
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
                        .and_then(|v| v.get(index))
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .unwrap_or(1.0)
                })
                .collect();
            Ok(Some(
                json!({"type":"split", "axis":value["axis"], "sizes":sizes, "children":children}),
            ))
        }
        _ => Ok(None),
    }
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
    let stored = value["version"].as_u64().filter(|v| (1..=4).contains(v))?;
    let allowed = workshop_panels_for(stored);
    let added = workshop_panels_added_after(stored);
    let mut seen = BTreeSet::new();
    let root = match value.get("root")? {
        Value::Null => Value::Null,
        root => match sanitize_workshop_node(root, &mut seen, 0, allowed) {
            Ok(Some(root)) => root,
            Ok(None) => Value::Null,
            Err(()) => return Some(default_authoring_layout()),
        },
    };
    let mut floats = Vec::new();
    for entry in value["floats"].as_array().into_iter().flatten() {
        let Some(panel) = entry["panel"]
            .as_str()
            .filter(|_| known_panel(&entry["panel"], allowed))
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
        if floats.len() == allowed.len() {
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
        .filter(|panel| known_panel(panel, allowed))
    {
        let panel = panel.as_str().unwrap();
        if seen.insert(panel.to_owned()) {
            closed.push(json!(panel));
        }
    }
    for panel in allowed {
        if seen.insert((*panel).to_owned()) {
            closed.push(json!(panel));
        }
    }
    let selected = value
        .get("selected")
        .filter(|panel| known_panel(panel, allowed) && !closed.contains(panel))
        .cloned()
        .or_else(|| first_visible(&root, &floats))
        .unwrap_or_else(|| json!("files"));
    let version = if stored == 1 { 2 } else { stored };
    let mut layout = json!({"version": version, "root": root, "floats": floats, "closed": closed, "selected": selected});
    if !added.is_empty() {
        layout = migrate_authoring_layout(layout, stored, &added, &value["closed"]);
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

/// Split the group holding `target` so `panel` becomes its neighbour, mirroring
/// the `dock` transition in gui/dock-layout-model.js.
fn split_workshop_group(
    node: &mut Value,
    target: &str,
    panel: &str,
    axis: &str,
    before: bool,
) -> bool {
    match node["type"].as_str() {
        Some("tabs")
            if node["tabs"].as_array().is_some_and(|tabs| {
                tabs.iter()
                    .any(|candidate| candidate.as_str() == Some(target))
            }) =>
        {
            let existing = node.take();
            let added = json!({"type":"tabs", "tabs":[panel], "active":panel});
            let children = if before {
                json!([added, existing])
            } else {
                json!([existing, added])
            };
            *node = json!({"type":"split", "axis":axis, "sizes":[1.0,1.0], "children":children});
            true
        }
        Some("split") => node["children"].as_array_mut().is_some_and(|children| {
            children
                .iter_mut()
                .any(|child| split_workshop_group(child, target, panel, axis, before))
        }),
        _ => false,
    }
}

fn dock_workshop_panel(layout: &mut Value, panel: &str, target: &str, placement: &str) {
    if placement == "tab" {
        dock_workshop_tab(layout, panel, target);
        return;
    }
    let axis = if matches!(placement, "left" | "right") {
        "horizontal"
    } else {
        "vertical"
    };
    let before = matches!(placement, "left" | "top");
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
    if !split_workshop_group(&mut layout["root"], target, panel, axis, before) {
        *layout = original;
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

fn workshop_node_contains(node: &Value, panel: &str) -> bool {
    match node["type"].as_str() {
        Some("tabs") => node["tabs"]
            .as_array()
            .is_some_and(|tabs| tabs.iter().any(|tab| tab.as_str() == Some(panel))),
        Some("split") => node["children"].as_array().is_some_and(|children| {
            children
                .iter()
                .any(|child| workshop_node_contains(child, panel))
        }),
        _ => false,
    }
}

fn add_migration_panel(layout: &mut Value, panel: &str, preferred: &str, preserved_closed: &Value) {
    add_migration_panel_at(layout, panel, preferred, preserved_closed, "tab");
}

fn add_migration_panel_at(
    layout: &mut Value,
    panel: &str,
    preferred: &str,
    preserved_closed: &Value,
    placement: &str,
) {
    if preserved_closed
        .as_array()
        .is_some_and(|closed| closed.iter().any(|value| value.as_str() == Some(panel)))
    {
        return;
    }
    let Some(closed_index) = layout["closed"].as_array().and_then(|closed| {
        closed
            .iter()
            .position(|value| value.as_str() == Some(panel))
    }) else {
        return;
    };
    if let Some(target) = preferred_workshop_target(layout, preferred) {
        dock_workshop_panel(layout, panel, &target, placement);
        return;
    }
    if layout["floats"].as_array().is_none_or(Vec::is_empty) {
        return;
    }
    layout["closed"]
        .as_array_mut()
        .unwrap()
        .remove(closed_index);
    layout["root"] = json!({"type":"tabs", "tabs":[panel], "active":panel});
}

fn collect_workshop_actives(node: &Value, out: &mut BTreeMap<String, String>) {
    match node["type"].as_str() {
        Some("tabs") => {
            let Some(active) = node["active"].as_str() else {
                return;
            };
            for panel in node["tabs"].as_array().into_iter().flatten() {
                if let Some(panel) = panel.as_str() {
                    out.insert(panel.to_owned(), active.to_owned());
                }
            }
        }
        Some("split") => {
            for child in node["children"].as_array().into_iter().flatten() {
                collect_workshop_actives(child, out);
            }
        }
        _ => {}
    }
}

fn restore_workshop_actives(node: &mut Value, previous: &BTreeMap<String, String>) {
    match node["type"].as_str() {
        Some("tabs") => {
            let active = node["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|panel| {
                    previous
                        .get(panel.as_str()?)
                        .filter(|active| {
                            node["tabs"].as_array().is_some_and(|tabs| {
                                tabs.iter()
                                    .any(|candidate| candidate.as_str() == Some(active))
                            })
                        })
                        .cloned()
                });
            if let Some(active) = active {
                node["active"] = json!(active);
            }
        }
        Some("split") => {
            for child in node["children"].as_array_mut().into_iter().flatten() {
                restore_workshop_actives(child, previous);
            }
        }
        _ => {}
    }
}

fn migrate_authoring_layout(
    mut layout: Value,
    stored_version: u64,
    added: &[(&str, &str)],
    preserved_closed: &Value,
) -> Value {
    let selected = layout["selected"].clone();
    let mut previous_actives = BTreeMap::new();
    collect_workshop_actives(&layout["root"], &mut previous_actives);
    layout["closed"]
        .as_array_mut()
        .unwrap()
        .extend(added.iter().map(|(panel, _)| json!(panel)));
    if stored_version == 1 {
        for panel in ["add", "recovery"] {
            add_migration_panel(&mut layout, panel, "inspector", preserved_closed);
        }
    }
    layout["version"] = json!(4);
    for (panel, preferred) in added {
        add_migration_panel(&mut layout, panel, preferred, &Value::Null);
    }
    restore_workshop_actives(&mut layout["root"], &previous_actives);
    layout["selected"] = selected;
    layout
}

fn preferred_workshop_target(layout: &Value, preferred: &str) -> Option<String> {
    if workshop_node_contains(&layout["root"], preferred) {
        Some(preferred.to_owned())
    } else {
        first_visible(&layout["root"], &[]).and_then(|panel| panel.as_str().map(str::to_owned))
    }
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
        "presentation": fields(&raw["accessibility"]["presentation"], &[
            "textScale", "contrast", "reducedMotion", "shake", "flash", "decorativeMotion"
        ]),
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
    if let Some(layout) = sanitize_live_layout(&raw["liveLayout"]) {
        safe["liveLayout"] = layout;
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
            "accessibility":{"presentation":{"textScale":1.25,"contrast":"on","reducedMotion":"off","shake":0.2,"flash":0.4,"decorativeMotion":"default","unsafe":"secret"},
                "assistance":{"helm.course-keeping":"request"}},
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
        assert_eq!(
            saved["accessibility"]["presentation"],
            json!({
                "textScale":1.25,"contrast":"on","reducedMotion":"off",
                "shake":0.2,"flash":0.4,"decorativeMotion":"default"
            })
        );
        assert_eq!(
            saved["accessibility"]["assistance"],
            json!({"helm.course-keeping":"request"})
        );
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
    fn live_layout_is_sanitized_separately_from_authoring_layout() {
        let profile = json!({
            "kind":"project-phoenix/operator-profile", "version":1,
            "authoringLayout":default_authoring_layout(),
            "liveLayout":{
                "version":1,
                "root":{"type":"tabs","tabs":["roster","roster","unsafe"],"active":"unsafe"},
                "floats":[{"panel":"join","x":30,"unsafe":"secret"}],
                "closed":["manual-save"], "selected":"unsafe", "unsafe":"secret"
            },
            "reconnectCredential":"secret"
        });
        let saved: Value =
            serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
        assert_eq!(saved["authoringLayout"]["selected"], "source");
        assert_eq!(
            saved["authoringLayout"]["root"]["children"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        // The stored layout is version 1, so it arrives migrated: the record
        // panels open their own group below the readiness panels, exactly as
        // gui/live-layout-model.js places them.
        assert_eq!(
            saved["liveLayout"],
            json!({
                "version":2,
                "root":{"type":"split", "axis":"vertical", "sizes":[1.0,1.0], "children":[
                    {"type":"tabs", "tabs":["roster","mission"], "active":"roster"},
                    {"type":"tabs", "tabs":["comms","activity","journal","session-history"], "active":"comms"}
                ]},
                "floats":[{"panel":"join","x":30.0,"y":12.0,"width":420.0,"height":360.0}],
                "closed":["manual-save","readiness"], "selected":"roster"
            })
        );
        assert!(saved.get("reconnectCredential").is_none());
        assert!(!saved.to_string().contains("secret"));
    }

    #[test]
    fn a_stored_live_layout_cannot_name_a_panel_version_one_never_registered() {
        // v1 had no record vocabulary: these must enter through migration only.
        let layout = json!({
            "version":1,
            "root":{"type":"tabs","tabs":["roster","comms","journal"],"active":"journal"},
            "floats":[{"panel":"activity","x":7,"y":9,"width":300,"height":200}],
            "closed":["readiness"], "selected":"journal"
        });

        let migrated = sanitize_live_layout(&layout).unwrap();
        assert_eq!(migrated["version"], 2);
        assert_eq!(migrated["floats"], json!([]));
        assert_eq!(migrated["selected"], "roster");
        assert_eq!(
            migrated["closed"],
            json!(["readiness", "join", "manual-save"])
        );
        assert_eq!(
            migrated["root"]["children"][0]["tabs"],
            json!(["roster", "mission"])
        );
        assert_eq!(
            migrated["root"]["children"][1],
            json!({"type":"tabs", "tabs":["comms","activity","journal","session-history"], "active":"comms"})
        );
    }

    #[test]
    fn a_current_live_layout_keeps_its_arrangement_and_drops_unknown_fields() {
        let layout = json!({
            "version":2,
            "root":{"type":"tabs","tabs":["comms","journal","unsafe"],"active":"journal","unsafe":"secret"},
            "floats":[{"panel":"roster","x":7,"y":9,"width":300,"height":200,"unsafe":"secret"}],
            "closed":["readiness","join","manual-save","mission","activity","session-history"],
            "selected":"journal", "unsafe":"secret"
        });

        assert_eq!(
            sanitize_live_layout(&layout).unwrap(),
            json!({
                "version":2,
                "root":{"type":"tabs", "tabs":["comms","journal"], "active":"journal"},
                "floats":[{"panel":"roster","x":7.0,"y":9.0,"width":300.0,"height":200.0}],
                "closed":["readiness","join","manual-save","mission","activity","session-history"],
                "selected":"journal"
            })
        );
    }

    #[test]
    fn a_v1_live_layout_that_closed_a_panel_keeps_it_closed() {
        let layout = json!({
            "version":1,
            "root":{"type":"tabs","tabs":["roster","readiness"],"active":"readiness"},
            "floats":[], "closed":["join","manual-save"], "selected":"readiness"
        });

        let migrated = sanitize_live_layout(&layout).unwrap();
        assert_eq!(migrated["closed"], json!(["join", "manual-save"]));
        assert_eq!(
            migrated["root"]["children"][0],
            json!({"type":"tabs", "tabs":["roster","readiness","mission"], "active":"readiness"})
        );
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
        assert_eq!(saved["authoringLayout"]["version"], 4);
        assert!(saved["authoringLayout"]["root"].is_null());
        assert_eq!(saved["authoringLayout"]["floats"], json!([]));
        assert_eq!(
            saved["authoringLayout"]["closed"],
            json!([
                "files",
                "source",
                "inspector",
                "add",
                "recovery",
                "dependencies",
                "findings",
                "feedback",
                "settings",
                "models",
                "model-preview",
                "sound"
            ])
        );
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

        let repaired = sanitize_authoring_layout(&layout).unwrap();
        assert_eq!(repaired["version"], 4);
        assert_eq!(repaired["selected"], "inspector");
        assert_eq!(repaired["closed"], json!(["add", "recovery"]));
        fn placements(node: &Value, panel: &str) -> usize {
            match node["type"].as_str() {
                Some("tabs") => node["tabs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|candidate| candidate.as_str() == Some(panel))
                    .count(),
                Some("split") => node["children"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|child| placements(child, panel))
                    .sum(),
                _ => 0,
            }
        }
        for panel in [
            "source",
            "inspector",
            "files",
            "dependencies",
            "findings",
            "feedback",
            "settings",
            "models",
            "model-preview",
            "sound",
        ] {
            let count = placements(&repaired["root"], panel)
                + repaired["floats"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|entry| entry["panel"].as_str() == Some(panel))
                    .count();
            assert_eq!(count, 1);
        }
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
        assert_eq!(migrated["version"], 4);
        assert_eq!(
            migrated["root"]["tabs"],
            json!([
                "files",
                "inspector",
                "add",
                "recovery",
                "dependencies",
                "findings",
                "feedback",
                "settings",
                "models",
                "model-preview",
                "sound"
            ])
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
                "version":4,
                "root":{"type":"split","axis":"horizontal","sizes":[10.0,30.0,60.0],"children":[
                    {"type":"tabs","tabs":["files","add","dependencies"],"active":"add"},
                    {"type":"tabs","tabs":["source","findings","feedback","model-preview"],"active":"source"},
                    {"type":"tabs","tabs":["inspector","settings","models","sound"],"active":"inspector"}
                ]},
                "floats":[{"panel":"recovery","x":7.0,"y":9.0,"width":300.0,"height":200.0}],
                "closed":[],"selected":"source"
            })
        );
    }

    #[test]
    fn legacy_authoring_layouts_preserve_floating_preferred_targets() {
        for version in [1, 2] {
            let floats = json!([
                {"panel":"files","x":13.0,"y":17.0,"width":301.0,"height":211.0},
                {"panel":"source","x":41.0,"y":47.0,"width":503.0,"height":307.0}
            ]);
            let layout = json!({
                "version":version,
                "root":{"type":"split","axis":"vertical","sizes":[17,83],"children":[
                    {"type":"tabs","tabs":["inspector"],"active":"inspector"},
                    {"type":"tabs","tabs":["recovery"],"active":"recovery"}
                ]},
                "floats":floats, "closed":["add"], "selected":"source"
            });

            assert_eq!(
                sanitize_authoring_layout(&layout).unwrap(),
                json!({
                    "version":4,
                    "root":{"type":"split","axis":"vertical","sizes":[17.0,83.0],"children":[
                        {"type":"tabs","tabs":["inspector","dependencies","findings","feedback","settings","models","model-preview","sound"],"active":"inspector"},
                        {"type":"tabs","tabs":["recovery"],"active":"recovery"}
                    ]},
                    "floats":floats, "closed":["add"], "selected":"source"
                })
            );
        }
    }

    #[test]
    fn all_closed_legacy_authoring_layouts_remain_all_closed() {
        for version in [1, 2] {
            let layout = json!({
                "version":version, "root":null, "floats":[],
                "closed":["files","source","inspector","add","recovery"], "selected":"source"
            });
            let migrated = sanitize_authoring_layout(&layout).unwrap();
            assert_eq!(migrated["version"], 4);
            assert!(migrated["root"].is_null());
            assert_eq!(migrated["floats"], json!([]));
            assert_eq!(
                migrated["closed"],
                json!([
                    "files",
                    "source",
                    "inspector",
                    "add",
                    "recovery",
                    "dependencies",
                    "findings",
                    "feedback",
                    "settings",
                    "models",
                    "model-preview",
                    "sound"
                ])
            );
        }
    }

    #[test]
    fn current_authoring_layout_preserves_registered_panels_and_drops_unknown_fields() {
        let layout = json!({
            "version":4,
            "root":{"type":"tabs","tabs":["source","findings","feedback","dependencies","settings","models","model-preview","sound","unsafe"],"active":"feedback","unsafe":"secret"},
            "floats":[{"panel":"files","x":7,"y":9,"width":300,"height":200,"unsafe":"secret"}],
            "closed":["inspector","add","recovery"],"selected":"feedback","unsafe":"secret"
        });

        assert_eq!(
            sanitize_authoring_layout(&layout).unwrap(),
            json!({
                "version":4,
                "root":{"type":"tabs","tabs":["source","findings","feedback","dependencies","settings","models","model-preview","sound"],"active":"feedback"},
                "floats":[{"panel":"files","x":7.0,"y":9.0,"width":300.0,"height":200.0}],
                "closed":["inspector","add","recovery"],"selected":"feedback"
            })
        );
    }

    #[test]
    fn stored_v3_authoring_layout_registers_media_panels_without_reopening_a_closed_panel() {
        let layout = json!({
            "version":3,
            "root":{"type":"split","axis":"horizontal","sizes":[22,56,22],"children":[
                {"type":"tabs","tabs":["files","dependencies"],"active":"files"},
                {"type":"tabs","tabs":["source","findings"],"active":"source"},
                {"type":"tabs","tabs":["inspector","add","recovery"],"active":"inspector"}
            ]},
            "floats":[],"closed":["feedback","settings"],"selected":"source"
        });

        assert_eq!(
            sanitize_authoring_layout(&layout).unwrap(),
            json!({
                "version":4,
                "root":{"type":"split","axis":"horizontal","sizes":[22.0,56.0,22.0],"children":[
                    {"type":"tabs","tabs":["files","dependencies"],"active":"files"},
                    {"type":"tabs","tabs":["source","findings","model-preview"],"active":"source"},
                    {"type":"tabs","tabs":["inspector","add","recovery","models","sound"],"active":"inspector"}
                ]},
                "floats":[],"closed":["feedback","settings"],"selected":"source"
            })
        );
    }

    #[test]
    fn a_stored_layout_cannot_name_a_panel_its_own_version_never_registered() {
        // v3 had no media vocabulary: these must enter through migration only.
        let layout = json!({
            "version":3,
            "root":{"type":"tabs","tabs":["source","model-preview","sound"],"active":"model-preview"},
            "floats":[{"panel":"models","x":7,"y":9,"width":300,"height":200}],
            "closed":[],"selected":"model-preview"
        });

        let migrated = sanitize_authoring_layout(&layout).unwrap();
        assert_eq!(migrated["version"], 4);
        assert_eq!(migrated["floats"], json!([]));
        assert_eq!(migrated["selected"], "source");
        assert_eq!(migrated["root"]["active"], "source");
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
