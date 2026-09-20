//! The disposable iframe's narrow WASM edge. Captured assets are an explicit
//! per-App input; only clock requests and a status mirror use the local edge.
use super::{
    test_clock::{TestClock, TestClockPlugin, TestControls},
    test_protocol::{ControlRecord, Launch, TestStatus},
};
use bevy::prelude::*;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
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
        let captured = super::captured_source::capture(source, "Workshop Test", "Test")
            .map_err(|message| fail(&message))?;
        let (assets, text) = (captured.assets, captured.text);
        // The captured factions ARE the Test's registry (issue #1474): a
        // faction the draft deleted is gone, one it added is there, and the
        // compiled-in set never fills a gap. Validation already refuses an
        // unparsable file; this keeps that belt on the launch itself.
        let mut factions = Vec::new();
        for (path, source) in &text {
            if path.starts_with("assets/factions/") && path.ends_with(".toml") {
                factions.push(
                    crate::ai::faction::parse_faction_config(source)
                        .map_err(|error| fail(&format!("{path}: {error}")))?,
                );
            }
        }
        let report = super::test_source::validate_selection(text, &launch.selection);
        if !report.accepted {
            return Err(fail(
                &crate::core::codec::encode_workshop_validation(&report)
                    .map_err(|e| fail(&e.to_string()))?,
            ));
        }
        crate::entities::config_cache::replace_faction_registry(factions);
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
        .insert_resource(super::test_trace::TestTrace::default())
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
    view: Res<super::test_view::TestViewState>,
    trace: Res<super::test_trace::TestTrace>,
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
            view: view.requested.clone(),
            ships: view.ships.clone(),
            trace: trace.records(),
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
            || matches!(record.control, super::test_protocol::TestControl::Step {} if !status.paused || status.starting)
            // A view names a ship the run actually has, or the launched one.
            // An unknown id is refused here rather than silently drawing
            // whatever was already on screen.
            || matches!(&record.control, super::test_protocol::TestControl::View {
                view: super::test_protocol::TestView::Ship { entity: Some(entity) }
            } if !status.ships.iter().any(|ship| &ship.entity == entity)) {
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
