//! The disposable iframe's narrow WASM edge. Captured assets are an explicit
//! per-App input; only clock requests and a status mirror use the local edge.
use super::{
    test_clock::{TestClock, TestClockPlugin, TestControls},
    test_protocol::{ControlRecord, Launch, TestStatus},
};
use bevy::prelude::*;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use wasm_bindgen::prelude::*;

pub(crate) struct BrowserTest {
    pub launch: Launch,
    pub assets: Arc<BTreeMap<String, Arc<[u8]>>>,
}

impl BrowserTest {
    pub fn capture(launch: &str, source: JsValue) -> Result<Self, JsValue> {
        let fail = |message: &str| JsValue::from_str(message);
        if crate::server::bridge::wasm_boot_profile() != "not-started" {
            return Err(fail("Workshop Test requires a fresh iframe"));
        }
        let launch = crate::core::codec::decode_workshop_test_launch(launch.as_bytes())
            .map_err(|e| fail(&e))?;
        if !source.is_object() || js_sys::Array::is_array(&source) {
            return Err(fail("Invalid Workshop Test source"));
        }
        let entries = js_sys::Object::entries(&source.unchecked_into());
        if entries.length() > 16384 {
            return Err(fail("Workshop Test source is too large"));
        }
        let mut assets = BTreeMap::new();
        let mut text = BTreeMap::new();
        let mut folded = BTreeSet::new();
        let mut length = 0usize;
        for entry in entries.iter() {
            let pair = js_sys::Array::from(&entry);
            let path = pair
                .get(0)
                .as_string()
                .ok_or_else(|| fail("Invalid Test source path"))?;
            if !(path == "scenarios.toml" || path.starts_with("assets/"))
                || path.contains(['\\', ':'])
                || path.chars().any(char::is_control)
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || !folded.insert(path.to_ascii_lowercase())
            {
                return Err(fail("Invalid Test source path"));
            }
            let value = pair.get(1);
            let bytes = if let Some(source) = value.as_string() {
                if path.ends_with(".toml") || path.ends_with(".rhai") {
                    text.insert(path.clone(), source.clone());
                }
                source.into_bytes()
            } else if value.is_instance_of::<js_sys::Uint8Array>() {
                let bytes = value.unchecked_into::<js_sys::Uint8Array>();
                if bytes.length() as usize > (512 * 1024 * 1024usize).saturating_sub(length) {
                    return Err(fail("Workshop Test source is too large"));
                }
                bytes.to_vec()
            } else {
                return Err(fail("Invalid Test source bytes"));
            };
            length = length.saturating_add(bytes.len());
            if length > 512 * 1024 * 1024 {
                return Err(fail("Workshop Test source is too large"));
            }
            assets.insert(path, Arc::from(bytes));
        }
        let report = super::test_source::validate_selection(text, &launch.selection);
        if !report.accepted {
            return Err(fail(
                &crate::core::codec::encode_workshop_validation(&report)
                    .map_err(|e| fail(&e.to_string()))?,
            ));
        }
        Ok(Self {
            launch,
            assets: Arc::new(assets),
        })
    }
}

#[derive(Default)]
struct Edge {
    commands: VecDeque<ControlRecord>,
    last_id: u64,
    status: Option<TestStatus>,
}
thread_local! { static EDGE: RefCell<Edge> = RefCell::new(Edge::default()); }

/// A Test may select any complete captured hull, including one outside the
/// world's player picker. Its include/rig closure still uses normal preload.
#[wasm_bindgen]
pub fn wasm_workshop_test_preload_ship(path: String, source: String) -> Result<JsValue, JsValue> {
    if crate::server::bridge::wasm_boot_profile() != "not-started"
        || !path.starts_with("assets/entities/")
        || !path.ends_with(".toml")
    {
        return Err(JsValue::from_str(
            "Workshop Test requires an unstarted captured hull",
        ));
    }
    crate::entities::config_cache::mark_entity_template(&path);
    crate::entities::config_cache::wasm_load_config(path, source)
}

#[derive(Resource)]
struct BrowserTestRun {
    launch: Launch,
    acknowledged: u64,
}

pub(crate) fn install(app: &mut App, launch: Launch) {
    use crate::authoritative::{DeclareState, StateClass};
    crate::sim_rng::install(
        app.world_mut(),
        crate::sim_rng::SimRng::new(launch.selection.seed, crate::sim_rng::SeedSource::Cli),
    );
    EDGE.with(|edge| edge.borrow_mut().status = Some(TestStatus::starting(&launch)));
    app.declare_state::<BrowserTestRun>(StateClass::Timer, "gm-milestone-integrated-workshop")
        .insert_resource(BrowserTestRun {
            launch,
            acknowledged: 0,
        })
        .add_plugins(TestClockPlugin)
        .insert_resource(crate::server::bridge::PendingForceStart(true))
        .add_systems(
            First,
            drain_controls.before(super::test_clock::apply_controls),
        )
        .add_systems(Last, publish_status.after(super::test_clock::finish_step));
}

fn drain_controls(mut run: ResMut<BrowserTestRun>, mut controls: ResMut<TestControls>) {
    EDGE.with(|edge| {
        for record in edge.borrow_mut().commands.drain(..) {
            run.acknowledged = record.id;
            controls.0.push_back(record.control);
        }
    });
}

fn publish_status(
    run: Res<BrowserTestRun>,
    clock: Res<TestClock>,
    tick: Res<crate::sim_tick::SimTick>,
    phase: Res<State<crate::core::messages::GamePhase>>,
) {
    use crate::core::messages::GamePhase;
    EDGE.with(|edge| {
        edge.borrow_mut().status = Some(TestStatus {
            running: true,
            starting: matches!(phase.get(), GamePhase::Lobby | GamePhase::Loading),
            paused: clock.paused,
            tick: tick.0,
            multiplier: clock.multiplier,
            acknowledged: run.acknowledged,
            error: None,
            revision: run.launch.revision.clone(),
            selection: run.launch.selection.clone(),
        })
    });
}

#[wasm_bindgen]
pub fn wasm_workshop_test_control(record: &str) -> Result<(), JsValue> {
    let record = crate::core::codec::decode_workshop_test_control(record)
        .map_err(|e| JsValue::from_str(&e))?;
    EDGE.with(|edge| {
        let mut edge = edge.borrow_mut();
        let Some(status) = edge.status.as_ref() else { return Err(JsValue::from_str("Workshop Test is not running")); };
        if record.id <= edge.last_id || edge.commands.len() >= 8
            || matches!(record.control, super::test_protocol::TestControl::Rate { multiplier } if !matches!(multiplier, 1 | 2 | 4 | 8))
            || matches!(record.control, super::test_protocol::TestControl::Step {} if !status.paused || status.starting) {
            return Err(JsValue::from_str("Workshop Test control refused"));
        }
        edge.last_id = record.id; edge.commands.push_back(record); Ok(())
    })
}

#[wasm_bindgen]
pub fn wasm_workshop_test_status() -> Result<Option<String>, JsValue> {
    EDGE.with(|edge| {
        edge.borrow()
            .status
            .as_ref()
            .map(crate::core::codec::encode_workshop_test_status)
            .transpose()
            .map_err(|e| JsValue::from_str(&e))
    })
}
