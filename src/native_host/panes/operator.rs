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
                    .and_then(|path| match std::fs::read_to_string(&path) {
                        Ok(text) if text.len() <= PROFILE_LIMIT => {
                            let profile = sanitize_profile(&text)?;
                            let reset = serde_json::from_str::<Value>(&text)
                                .ok()
                                .and_then(|raw| raw["liveLayout"]["version"].as_u64())
                                .is_some_and(|version| version < 19);
                            if reset {
                                // Backup and replacement are one atomic private write.
                                // An unwritable preference directory cannot disable the desk.
                                if let Err(error) =
                                    crate::native_file::write_preferences(&path, &profile)
                                {
                                    bevy::log::warn!(
                                        "Could not persist GM layout migration: {error}"
                                    );
                                }
                            }
                            Ok(Some(profile))
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
                        crate::native_file::write_preferences(&path, &profile)
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
// Vocabulary order is record order; migration order is deliberately separate.
// id, introduction version, migration order, preferred target, default column.
const WORKSHOP_INTRODUCTIONS: &[(&str, u64, usize, &str, usize)] = &[
    ("files", 2, 0, "", 0),
    ("source", 2, 0, "", 1),
    ("inspector", 2, 0, "", 2),
    ("add", 2, 0, "", 2),
    ("recovery", 2, 0, "", 2),
    ("findings", 3, 1, "source", 1),
    ("feedback", 3, 2, "source", 1),
    ("dependencies", 3, 0, "files", 0),
    ("settings", 3, 3, "inspector", 2),
    ("models", 4, 4, "inspector", 2),
    ("model-preview", 4, 5, "source", 1),
    ("sound", 4, 6, "inspector", 2),
    ("changes", 5, 7, "files", 0),
    ("definitions", 6, 8, "inspector", 2),
    ("composition", 7, 9, "files", 0),
    ("entity", 8, 10, "inspector", 2),
    ("presets", 9, 11, "files", 0),
    ("scripts", 10, 12, "source", 1),
];
const WORKSHOP_TEST_PANELS: &[&str] = &["test-controls", "test-viewscreen", "test-trace"];

fn workshop_panels_for(version: u64) -> Vec<&'static str> {
    WORKSHOP_INTRODUCTIONS
        .iter()
        .filter(|(_, introduced, _, _, _)| *introduced <= version.clamp(2, 10))
        .map(|(panel, _, _, _, _)| *panel)
        .collect()
}

fn workshop_panels_added_after(version: u64) -> Vec<(&'static str, &'static str)> {
    let mut records: Vec<_> = WORKSHOP_INTRODUCTIONS
        .iter()
        .filter(|(_, introduced, _, _, _)| *introduced > version.max(2))
        .collect();
    records.sort_by_key(|(_, _, order, _, _)| *order);
    records
        .into_iter()
        .map(|(panel, _, _, target, _)| (*panel, *target))
        .collect()
}

fn known_panel(value: &Value, allowed: &[&str]) -> bool {
    value.as_str().is_some_and(|panel| allowed.contains(&panel))
}

const LIVE_PANELS: &[&str] = &[
    "roster",
    "readiness",
    "join",
    "manual-save",
    "mission",
    "comms",
    "activity",
    "journal",
    "session-history",
    "map",
    "attention",
    "workload",
    "widgets",
    "health",
    "station",
    "station-console",
    "presentation",
    "audition",
    "source-link",
    "spawn",
    "inspector",
    "checkpoint",
    "restore",
    "contact",
    "npc",
    "misclassify",
    "report-policy",
    "system",
    "effect",
    "despawn",
    "faction",
    "objective",
    "entity-fields",
    "world-fields",
    "hull-fields",
    "region-fields",
    "presentation-fields",
];
/// Complex actions the operator opens, fills in and finishes. A DOCKED one is a
/// tool kept to hand and comes back empty; a FLOATING one is a draft and is not
/// restored at all. Mirrors LIVE_TEMPORARY_PANELS in gui/live-layout-model.js.
const LIVE_TEMPORARY_PANELS: &[&str] = &[
    "spawn",
    "restore",
    "misclassify",
    "report-policy",
    "effect",
    "manual-save",
    "contact",
    "npc",
    "system",
    "despawn",
    "faction",
    "entity-fields",
    "world-fields",
    "hull-fields",
    "region-fields",
    "presentation-fields",
];

/// Panels the operator may not close. The attention region renders connection
/// and recovery banners verbatim and health is the table behind them: a Game
/// Master must not be able to hide a failure from themselves, whichever
/// mechanism does the hiding. Mirrors LIVE_PINNED_PANELS in
/// gui/live-layout-model.js.
const LIVE_PINNED_PANELS: &[&str] = &[];

/// Put a pinned panel back into the first group of a stored tree that claimed
/// it was closed. Mirrors `placeInFirstGroup` in gui/dock-layout-model.js.
fn place_in_first_group(node: &mut Value, panel: &str) {
    match node["type"].as_str() {
        Some("tabs") => {
            if let Some(tabs) = node["tabs"].as_array_mut() {
                tabs.push(json!(panel));
            }
        }
        Some("split") => {
            if let Some(first) = node["children"].as_array_mut().and_then(|c| c.first_mut()) {
                place_in_first_group(first, panel);
            }
        }
        _ => *node = json!({"type":"tabs", "tabs":[panel], "active":panel}),
    }
}

fn sanitize_placement_node(
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
                if let Some(child) = sanitize_placement_node(child, seen, depth + 1, allowed)? {
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
    let children: Vec<_> = ["files", "source", "inspector"]
        .into_iter()
        .enumerate()
        .map(|(column, active)| {
            let tabs: Vec<_> = WORKSHOP_INTRODUCTIONS
                .iter()
                .filter(|(_, _, _, _, home)| *home == column)
                .map(|(panel, _, _, _, _)| *panel)
                .collect();
            json!({"type":"tabs", "tabs":tabs, "active":active})
        })
        .collect();
    json!({"version":10, "root":{"type":"split", "axis":"horizontal",
        "sizes":[22,56,22], "children":children}, "floats":[], "closed":[], "selected":"source"})
}

fn default_test_layout() -> Value {
    json!({
        "version": 2,
        "root": {"type":"split", "axis":"horizontal", "sizes":[25,50,25], "children":[
            {"type":"tabs", "tabs":["test-controls"], "active":"test-controls"},
            {"type":"tabs", "tabs":["test-viewscreen"], "active":"test-viewscreen"},
            {"type":"tabs", "tabs":["test-trace"], "active":"test-trace"}
        ]},
        "floats": [], "closed": [], "selected": "test-viewscreen"
    })
}

fn default_live_layout() -> Value {
    let open = ["roster", "map", "inspector", "activity"];
    let closed: Vec<_> = LIVE_PANELS.iter().filter(|id| !open.contains(id)).collect();
    json!({
        "version": 19,
        "root": {"type":"split", "axis":"vertical", "sizes":[4,1], "children":[
            {"type":"split", "axis":"horizontal", "sizes":[22,52,26], "children":[
                {"type":"tabs", "tabs":["roster"], "active":"roster"},
                {"type":"tabs", "tabs":["map"], "active":"map"},
                {"type":"tabs", "tabs":["inspector"], "active":"inspector"}
            ]},
            {"type":"tabs", "tabs":["activity"], "active":"activity"}
        ]},
        "floats": [],
        "closed": closed,
        "selected": "roster"
    })
}

fn sanitize_live_layout(value: &Value) -> Option<Value> {
    let stored = value["version"].as_u64().filter(|v| (1..=19).contains(v))?;
    if stored < 19 {
        return Some(default_live_layout());
    }
    let allowed = LIVE_PANELS;
    let mut layout = match sanitize_placement(value, allowed, LIVE_TEMPORARY_PANELS, "roster") {
        Ok(layout) => layout,
        Err(PlacementError::Invalid) => return None,
        Err(PlacementError::Reset) => return Some(default_live_layout()),
    };
    // A version that registered only a temporary panel places nothing, so the
    // stamp is written here rather than inside the placement pass.
    layout["version"] = default_live_layout()["version"].clone();
    // Retire BEFORE the pinned repair, as the browser does: its current-registry
    // sanitize drops a retired panel (and collapses the group it emptied) and
    // only then puts a pinned panel back into the first group. Repairing first
    // would land the pinned panels in a group about to be emptied and keep it.
    retire_unregistered_live_panels(&mut layout);
    repair_pinned_live_panels(&mut layout);
    record_unplaced_live_panels(&mut layout);
    Some(layout)
}

/// Prune unknown panels before repairing pinned and unplaced current panels.
fn retire_unregistered_live_panels(layout: &mut Value) {
    let current = LIVE_PANELS;
    let known = |panel: &Value| panel.as_str().is_some_and(|p| current.contains(&p));
    fn prune(node: &Value, known: &dyn Fn(&Value) -> bool) -> Option<Value> {
        match node["type"].as_str() {
            Some("tabs") => {
                let tabs: Vec<Value> = node["tabs"]
                    .as_array()?
                    .iter()
                    .filter(|tab| known(tab))
                    .cloned()
                    .collect();
                if tabs.is_empty() {
                    return None;
                }
                let active = if tabs.contains(&node["active"]) {
                    node["active"].clone()
                } else {
                    tabs[0].clone()
                };
                Some(json!({"type":"tabs", "tabs":tabs, "active":active}))
            }
            Some("split") => {
                let mut children = Vec::new();
                let mut sizes = Vec::new();
                for child in node["children"].as_array()?.iter() {
                    if let Some(kept) = prune(child, known) {
                        // The browser reads the size at the KEPT child's index,
                        // not the original one, so this does the same.
                        let size = node["sizes"]
                            .get(children.len())
                            .filter(|size| size.as_f64().is_some_and(|s| s > 0.0))
                            .cloned()
                            .unwrap_or_else(|| json!(1));
                        children.push(kept);
                        sizes.push(size);
                    }
                }
                match children.len() {
                    0 => None,
                    1 => children.pop(),
                    _ => Some(json!({"type":"split", "axis":node["axis"].clone(),
                        "sizes":sizes, "children":children})),
                }
            }
            _ => None,
        }
    }
    if !layout["root"].is_null() {
        layout["root"] = prune(&layout["root"], &known).unwrap_or(Value::Null);
    }
    if let Some(floats) = layout["floats"].as_array_mut() {
        floats.retain(|entry| known(&entry["panel"]));
    }
    if let Some(closed) = layout["closed"].as_array_mut() {
        closed.retain(|panel| known(panel));
    }
    if !known(&layout["selected"]) {
        let floats: Vec<Value> = layout["floats"].as_array().cloned().unwrap_or_default();
        layout["selected"] =
            first_visible(&layout["root"], &floats).unwrap_or_else(|| json!("roster"));
    }
}

/// A panel the CURRENT registry has that migration did not place — a temporary
/// one, which migration never places — is recorded as closed. Without this a
/// layout stored before it was registered would come back claiming neither open
/// nor closed, which the browser model does not do.
fn record_unplaced_live_panels(layout: &mut Value) {
    let mut placed = BTreeSet::new();
    collect_live_panels(&layout["root"], &mut placed);
    for entry in layout["floats"].as_array().into_iter().flatten() {
        if let Some(panel) = entry["panel"].as_str() {
            placed.insert(panel.to_owned());
        }
    }
    for panel in layout["closed"].as_array().into_iter().flatten() {
        if let Some(panel) = panel.as_str() {
            placed.insert(panel.to_owned());
        }
    }
    let closed = layout["closed"].as_array_mut().unwrap();
    for panel in LIVE_PANELS {
        if !placed.contains(*panel) {
            closed.push(json!(panel));
        }
    }
}

fn collect_live_panels(node: &Value, out: &mut BTreeSet<String>) {
    match node["type"].as_str() {
        Some("tabs") => {
            for tab in node["tabs"].as_array().into_iter().flatten() {
                if let Some(tab) = tab.as_str() {
                    out.insert(tab.to_owned());
                }
            }
        }
        Some("split") => {
            for child in node["children"].as_array().into_iter().flatten() {
                collect_live_panels(child, out);
            }
        }
        _ => {}
    }
}

/// A pinned panel that arrived closed - from a hand-edited or older profile -
/// is put back rather than honoured: closing it is not a choice this surface
/// offers, so a stored tree claiming it was closed is not one to trust.
fn repair_pinned_live_panels(layout: &mut Value) {
    for panel in LIVE_PINNED_PANELS {
        let Some(index) = layout["closed"]
            .as_array()
            .and_then(|closed| closed.iter().position(|v| v.as_str() == Some(*panel)))
        else {
            continue;
        };
        layout["closed"].as_array_mut().unwrap().remove(index);
        place_in_first_group(&mut layout["root"], panel);
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

#[derive(Debug)]
enum PlacementError {
    Invalid,
    Reset,
}

/// Shared bounded placement pass. Context adapters own versions and migration.
fn sanitize_placement(
    value: &Value,
    allowed: &[&str],
    excluded_floats: &[&str],
    fallback_selection: &str,
) -> Result<Value, PlacementError> {
    let mut seen = BTreeSet::new();
    let root = match value.get("root").ok_or(PlacementError::Invalid)? {
        Value::Null => Value::Null,
        root => match sanitize_placement_node(root, &mut seen, 0, allowed) {
            Ok(Some(root)) => root,
            Ok(None) => Value::Null,
            Err(()) => return Err(PlacementError::Reset),
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
        if excluded_floats.contains(&panel) || !seen.insert(panel.to_owned()) {
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
        return Err(PlacementError::Reset);
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
        .unwrap_or_else(|| json!(fallback_selection));
    Ok(json!({"root": root, "floats": floats, "closed": closed, "selected": selected}))
}

fn sanitize_authoring_layout(value: &Value) -> Option<Value> {
    let stored = value["version"].as_u64().filter(|v| (1..=10).contains(v))?;
    let allowed = workshop_panels_for(stored);
    let added = workshop_panels_added_after(stored);
    let mut layout = match sanitize_placement(value, &allowed, &[], "files") {
        Ok(layout) => layout,
        Err(PlacementError::Invalid) => return None,
        Err(PlacementError::Reset) => return Some(default_authoring_layout()),
    };
    let version = if stored == 1 { 2 } else { stored };
    layout["version"] = json!(version);
    if !added.is_empty() {
        layout = migrate_authoring_layout(layout, stored, &added, &value["closed"]);
    }
    Some(layout)
}

fn sanitize_test_layout(value: &Value) -> Option<Value> {
    let stored = value["version"].as_u64()?;
    if !matches!(stored, 1 | 2) {
        return None;
    }
    let allowed = WORKSHOP_TEST_PANELS;
    let mut layout = match sanitize_placement(value, allowed, &[], "test-controls") {
        Ok(layout) => layout,
        Err(PlacementError::Invalid) => return None,
        Err(PlacementError::Reset) => return Some(default_test_layout()),
    };
    layout["version"] = json!(2);
    if stored == 1 {
        let selected = layout["selected"].clone();
        dock_workshop_panel(&mut layout, "test-trace", "test-viewscreen", "right");
        layout["selected"] = selected;
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
    // Taken from the default rather than written again: the Live migration
    // already derives it this way, and the one place that spelled it out as a
    // literal is the one place a version bump forgot to update (issue #1471).
    layout["version"] = default_authoring_layout()["version"].clone();
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
    // A native pane's language is presentation state on this operator's own
    // host-backed profile. Keep it out of command and simulation records.
    if let Some(locale) = raw["locale"].as_str().filter(|value| {
        !value.is_empty()
            && value.len() <= 35
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        safe["locale"] = json!(locale);
    }
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
    if let Some(layout) = sanitize_test_layout(&raw["testLayout"]) {
        safe["testLayout"] = layout;
    }
    if let Some(layout) = sanitize_live_layout(&raw["liveLayout"]) {
        safe["liveLayout"] = layout;
    }
    let previous = if raw["previousLiveLayout"].is_object() {
        &raw["previousLiveLayout"]
    } else if raw["liveLayout"]["version"]
        .as_u64()
        .is_some_and(|v| v < 19)
    {
        &raw["liveLayout"]
    } else {
        &Value::Null
    };
    if previous.is_object() {
        let mut previous = previous.clone();
        previous["version"] = json!(19);
        if let Some(layout) = sanitize_live_layout(&previous) {
            safe["previousLiveLayout"] = layout;
        }
    }
    safe["gmDensity"] = json!(if raw["gmDensity"] == "touch" {
        "touch"
    } else {
        "compact"
    });
    serde_json::to_string_pretty(&safe).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Random UUIDs isolate host-local temporary test directories.
#[path = "operator_tests.rs"]
mod tests;
