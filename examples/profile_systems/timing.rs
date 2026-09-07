//! Named Bevy system wall spans for the profiling example (#1416/#1417).
//! Wraps existing systems without adding ordering/access edges. Bevy's debug
//! names are provided by the example's existing development dependencies.
//! Durations overlap across systems/threads and are not exclusive CPU/GPU time.
use bevy::ecs::{
    change_detection::{CheckChangeTicks, Tick},
    query::FilteredAccessSet,
    schedule::{graph::Direction, InternedSystemSet, NodeId, ScheduleGraph},
    system::{RunSystemError, ScheduleSystem, SystemParamValidationError, SystemStateFlags},
    world::{unsafe_world_cell::UnsafeWorldCell, DeferredWorld},
};
use bevy::prelude::*;
use bevy::utils::prelude::DebugName;
use bevy_rapier3d::prelude::PhysicsSet;
use project_phoenix::sim_sets::SimSet;
use serde::Serialize;
use std::{
    any::TypeId,
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Default, Clone, Serialize)]
pub(crate) struct Counts {
    pub calls: u64,
    pub nanos: u128,
    pub max_ns: u128,
    pub deferred_calls: u64,
    pub deferred_ns: u128,
    pub spans: Vec<Span>,
}
#[derive(Clone, Serialize)]
pub(crate) struct Span {
    pub update: u64,
    pub start_ns: u128,
    pub end_ns: u128,
    pub deferred: bool,
}
#[derive(Serialize)]
pub(crate) struct Row {
    pub schedule: String,
    pub category: String,
    pub node: String,
    pub name: String,
    pub counts: Mutex<Counts>,
}
pub(crate) struct Control {
    pub active: AtomicBool,
    pub update: AtomicU64,
    pub epoch: Instant,
    /// Native wall window shares the cadence collector's clock. Headless
    /// instead uses the explicit update-index `active` gate.
    pub window: Option<(Duration, Duration)>,
    /// Bounded raw trace per system. A truncated trace is explicitly labelled.
    pub span_limit: usize,
    pub truncated: AtomicBool,
}
impl Control {
    pub fn new(span_limit: usize) -> Self {
        Self {
            active: AtomicBool::new(false),
            update: AtomicU64::new(0),
            epoch: Instant::now(),
            window: None,
            span_limit,
            truncated: AtomicBool::new(false),
        }
    }
    pub fn active_at(&self, now: Instant) -> bool {
        self.window.map_or_else(
            || self.active.load(Ordering::Relaxed),
            |(start, end)| (start..end).contains(&now.duration_since(self.epoch)),
        )
    }
}
struct Timed {
    inner: ScheduleSystem,
    row: Arc<Row>,
    control: Arc<Control>,
}
impl Timed {
    fn record(&self, start: Option<(Instant, u64)>, deferred: bool) {
        if let Some((start, update)) = start {
            let ns = start.elapsed().as_nanos();
            let mut c = self.row.counts.lock().unwrap();
            if deferred {
                c.deferred_calls += 1;
                c.deferred_ns += ns;
            } else {
                c.calls += 1;
                c.nanos += ns;
                c.max_ns = c.max_ns.max(ns);
            }
            if c.spans.len() < self.control.span_limit {
                c.spans.push(Span {
                    update,
                    start_ns: start.duration_since(self.control.epoch).as_nanos(),
                    end_ns: start.duration_since(self.control.epoch).as_nanos() + ns,
                    deferred,
                });
            } else if self.control.span_limit > 0 {
                self.control.truncated.store(true, Ordering::Relaxed);
            }
        }
    }
    fn start(&self) -> Option<(Instant, u64)> {
        let now = Instant::now();
        self.control
            .active_at(now)
            .then(|| (now, self.control.update.load(Ordering::Relaxed)))
    }
}
impl System for Timed {
    type In = ();
    type Out = ();
    fn name(&self) -> DebugName {
        self.inner.name()
    }
    fn type_id(&self) -> TypeId {
        self.inner.type_id()
    }
    fn flags(&self) -> SystemStateFlags {
        self.inner.flags()
    }
    unsafe fn run_unsafe(
        &mut self,
        input: (),
        world: UnsafeWorldCell,
    ) -> Result<(), RunSystemError> {
        let start = self.start();
        // SAFETY: the wrapper forwards the inner system's access metadata,
        // flags and validation unchanged. The caller's System contract is the
        // same contract required by this exact inner call.
        let result = unsafe { self.inner.run_unsafe(input, world) };
        self.record(start, false);
        result
    }
    fn run_without_applying_deferred(
        &mut self,
        input: (),
        world: &mut World,
    ) -> Result<(), RunSystemError> {
        let start = self.start();
        let result = self.inner.run_without_applying_deferred(input, world);
        self.record(start, false);
        result
    }
    fn apply_deferred(&mut self, world: &mut World) {
        let start = self.start();
        self.inner.apply_deferred(world);
        self.record(start, true);
    }
    fn queue_deferred(&mut self, world: DeferredWorld) {
        self.inner.queue_deferred(world);
    }
    unsafe fn validate_param_unsafe(
        &mut self,
        world: UnsafeWorldCell,
    ) -> Result<(), SystemParamValidationError> {
        // SAFETY: no additional World access is introduced by the observer.
        unsafe { self.inner.validate_param_unsafe(world) }
    }
    fn initialize(&mut self, world: &mut World) -> FilteredAccessSet {
        self.inner.initialize(world)
    }
    fn check_change_tick(&mut self, check: CheckChangeTicks) {
        self.inner.check_change_tick(check);
    }
    fn default_system_sets(&self) -> Vec<InternedSystemSet> {
        self.inner.default_system_sets()
    }
    fn get_last_run(&self) -> Tick {
        self.inner.get_last_run()
    }
    fn set_last_run(&mut self, last_run: Tick) {
        self.inner.set_last_run(last_run);
    }
}
fn ancestors(graph: &ScheduleGraph, node: NodeId) -> HashSet<NodeId> {
    let mut found = HashSet::new();
    let mut pending = vec![node];
    while let Some(n) = pending.pop() {
        for parent in graph
            .hierarchy()
            .graph()
            .neighbors_directed(n, Direction::Incoming)
        {
            if found.insert(parent) {
                pending.push(parent);
            }
        }
    }
    found
}
pub(crate) fn instrument_world(world: &mut World, control: Arc<Control>) -> Vec<Arc<Row>> {
    let mut rows = Vec::new();
    let mut schedules = world.resource_mut::<bevy::ecs::schedule::Schedules>();
    for (label, schedule) in schedules.iter_mut() {
        let schedule_name = format!("{label:?}");
        // Exclude schedules whose systems invoke other schedules, avoiding inclusive double counts.
        let include = [
            "First",
            "PreUpdate",
            "FixedFirst",
            "FixedPreUpdate",
            "FixedUpdate",
            "FixedPostUpdate",
            "FixedLast",
            "Update",
            "PostUpdate",
            "Last",
            "Render",
            "ExtractSchedule",
        ]
        .contains(&schedule_name.as_str());
        if !include {
            continue;
        }
        let graph = schedule.graph_mut();
        let mut named_sets: Vec<(String, InternedSystemSet)> = [
            SimSet::Input,
            SimSet::Physics,
            SimSet::Damage,
            SimSet::Modifiers,
            SimSet::Publish,
            SimSet::PublishAggregate,
            SimSet::Broadcast,
        ]
        .into_iter()
        .map(|s| (format!("SimSet::{s:?}"), s.intern()))
        .collect();
        named_sets.extend(
            [
                PhysicsSet::SyncBackend,
                PhysicsSet::StepSimulation,
                PhysicsSet::Writeback,
            ]
            .into_iter()
            .map(|s| (format!("Rapier::{s:?}"), s.intern())),
        );
        named_sets.push((
            "Admission".into(),
            project_phoenix::command_admission::AdmissionSet.intern(),
        ));
        named_sets.push((
            "Lobby".into(),
            project_phoenix::lobby::server::LobbySystemSet.intern(),
        ));
        use bevy::render::RenderSystems;
        named_sets.extend(
            [
                RenderSystems::ExtractCommands,
                RenderSystems::PrepareAssets,
                RenderSystems::PrepareMeshes,
                RenderSystems::ManageViews,
                RenderSystems::QueueMeshes,
                RenderSystems::QueueSweep,
                RenderSystems::Queue,
                RenderSystems::PhaseSort,
                RenderSystems::PrepareResources,
                RenderSystems::PrepareResourcesCollectPhaseBuffers,
                RenderSystems::PrepareResourcesFlush,
                RenderSystems::PrepareBindGroups,
                RenderSystems::Prepare,
                RenderSystems::Render,
                RenderSystems::Cleanup,
                RenderSystems::PostCleanup,
            ]
            .into_iter()
            .map(|s| (format!("RenderSystems::{s:?}"), s.intern())),
        );
        let categories: Vec<_> = named_sets
            .into_iter()
            .filter_map(|(name, s)| {
                graph
                    .system_sets
                    .get_key(s)
                    .map(|key| (name, NodeId::Set(key)))
            })
            .collect();
        let keys: Vec<_> = graph.systems.iter().map(|(key, _, _)| key).collect();
        for key in keys {
            let above = ancestors(graph, NodeId::System(key));
            let category = categories
                .iter()
                .find(|(_, node)| above.contains(node))
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "Other".into());
            let slot = graph.systems.get_mut(key).unwrap();
            // Preserve the special marker's concrete identity and executor handling.
            if slot.system.type_id() == TypeId::of::<bevy::ecs::schedule::ApplyDeferred>() {
                continue;
            }
            let node = format!("{key:?}");
            let name = slot.system.name().to_string();
            assert!(
                !name.contains("Enable the debug"),
                "Named profiling requires Bevy debug names"
            );
            let row = Arc::new(Row {
                schedule: schedule_name.clone(),
                category,
                node,
                name,
                counts: Mutex::new(Counts::default()),
            });
            let inner =
                std::mem::replace(&mut slot.system, Box::new(IntoSystem::into_system(|| {})));
            slot.system = Box::new(Timed {
                inner,
                row: row.clone(),
                control: control.clone(),
            });
            rows.push(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Component)]
    struct Value(u32);
    #[derive(Resource, Default)]
    struct Pass(u32);
    #[derive(Resource, Default, Debug, PartialEq)]
    struct Observed(Vec<(u32, u32)>);

    fn populate(mut commands: Commands, pass: Res<Pass>) {
        if pass.0 == 0 {
            commands.spawn(Value(1));
        }
    }
    fn change(pass: Res<Pass>, mut values: Query<&mut Value>) {
        if pass.0 == 1 {
            for mut value in &mut values {
                value.0 += 3;
            }
        }
    }
    fn observe(pass: Res<Pass>, values: Query<&Value, Changed<Value>>, mut seen: ResMut<Observed>) {
        for value in &values {
            seen.0.push((pass.0, value.0));
        }
    }
    fn advance(world: &mut World) {
        world.resource_mut::<Pass>().0 += 1;
    }

    fn run(wrapped: bool) -> (Vec<(u32, u32)>, Vec<Arc<Row>>) {
        let mut app = App::new();
        app.init_resource::<Pass>()
            .init_resource::<Observed>()
            .add_systems(Update, (populate, change, observe, advance).chain());
        app.finish();
        app.cleanup();
        let control = Arc::new(Control::new(100));
        control.active.store(true, Ordering::Relaxed);
        let rows = if wrapped {
            instrument_world(app.world_mut(), control)
        } else {
            Vec::new()
        };
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<Pass>().0, 3);
        (app.world().resource::<Observed>().0.clone(), rows)
    }

    #[test]
    fn wrapping_preserves_deferred_visibility_change_ticks_and_exclusive_system_order() {
        let (control, _) = run(false);
        let (wrapped, rows) = run(true);
        assert_eq!(control, vec![(0, 1), (1, 4)]);
        assert_eq!(wrapped, control);
        for name in ["populate", "change", "observe", "advance"] {
            let row = rows.iter().find(|row| row.name.ends_with(name)).unwrap();
            assert_eq!(row.counts.lock().unwrap().calls, 3);
        }
        assert!(rows
            .iter()
            .any(|row| row.counts.lock().unwrap().deferred_calls > 0));
    }
}
