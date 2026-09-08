//! Live Bevy span producer for the headless harness. All clocks, buffers and
//! attribution live outside App; no simulation system can read them.
use super::phase::{self, FixedInvocation, Interval, ObservedSystem};
use crate::sim_sets::SimSet;
use bevy::log::{tracing, tracing_subscriber};
use bevy::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id},
    Subscriber,
};
use tracing_subscriber::{
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
    Layer,
};
use vellum_perf::Recorder;

const PHASES: [SimSet; 7] = [
    SimSet::Input,
    SimSet::Physics,
    SimSet::Damage,
    SimSet::Modifiers,
    SimSet::Publish,
    SimSet::PublishAggregate,
    SimSet::Broadcast,
];

/// Full names and multiplicity, not numeric ECS IDs. Also used by the external
/// measured/unmeasured proof to detect accidental graph changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScheduleObservation {
    pub systems: Vec<(String, Vec<String>)>,
    pub ambiguities: Vec<(String, String, Vec<String>)>,
}

/// Read only AFTER ordinary execution initialized the schedule. Eagerly
/// initializing before Startup could register component IDs earlier than an
/// unmeasured run, so this collector never does that.
pub fn observe_schedule(app: &App) -> Result<ScheduleObservation, String> {
    let schedule = app.get_schedule(FixedUpdate).ok_or("missing FixedUpdate")?;
    let systems: Vec<_> = schedule.systems().map_err(|e| format!("{e:?}"))?.collect();
    let names: HashMap<_, _> = systems
        .iter()
        .map(|(id, system)| (*id, system.name().as_string()))
        .collect();
    let mut rows = Vec::new();
    for (id, system) in systems {
        let mut phases = Vec::new();
        for phase in PHASES {
            match schedule.graph().systems_in_set(phase.intern()) {
                Ok(members) if members.contains(&id) => phases.push(format!("{phase:?}")),
                Ok(_) | Err(bevy::ecs::schedule::ScheduleError::SetNotFound) => {}
                Err(error) => return Err(format!("phase membership unavailable: {error:?}")),
            }
        }
        rows.push((system.name().as_string(), phases));
    }
    rows.sort();
    let mut ambiguities = Vec::new();
    for (a, b, access) in schedule.graph().conflicting_systems().iter() {
        let mut pair = [names[a].clone(), names[b].clone()];
        pair.sort();
        let mut access: Vec<_> = access
            .iter()
            .map(|id| {
                app.world()
                    .components()
                    .get_name(*id)
                    .map(|name| name.as_string())
                    .ok_or_else(|| format!("unknown component {id:?}"))
            })
            .collect::<Result<_, _>>()?;
        if access.is_empty() {
            access.push("<exclusive World access>".into());
        }
        access.sort();
        ambiguities.push((pair[0].clone(), pair[1].clone(), access));
    }
    ambiguities.sort();
    Ok(ScheduleObservation {
        systems: rows,
        ambiguities,
    })
}

fn attribution(snapshot: &ScheduleObservation) -> BTreeMap<String, Option<SimSet>> {
    let mut choices: BTreeMap<String, Vec<Option<SimSet>>> = BTreeMap::new();
    for (name, phases) in &snapshot.systems {
        let phase = if phases.len() == 1 {
            PHASES.into_iter().find(|p| format!("{p:?}") == phases[0])
        } else {
            None
        };
        choices.entry(name.clone()).or_default().push(phase);
    }
    choices
        .into_iter()
        .map(|(name, phases)| {
            let first = phases[0];
            (name, if phases.len() == 1 { first } else { None })
        })
        .collect()
}

#[derive(Clone, Debug)]
enum Kind {
    Schedule(String),
    System(String),
    Auxiliary,
}
#[derive(Default)]
struct NameField(String);
impl Visit for NameField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" {
            self.0 = value.into();
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "name" {
            self.0 = format!("{value:?}");
        }
    }
}
struct Entered {
    span: u64,
    at: Duration,
    kind: Kind,
    invocation: Option<u64>,
    nested: bool,
    ambiguous: bool,
}
struct RawSystem {
    name: String,
    interval: Interval,
    unattributed: bool,
    ambiguous: bool,
}
struct RawInvocation {
    id: u64,
    interval: Interval,
    systems: Vec<RawSystem>,
}
struct Active {
    id: u64,
    span: u64,
    at: Duration,
    nested: usize,
    systems: Vec<RawSystem>,
}
#[derive(Default)]
struct Buffer {
    kinds: HashMap<u64, Kind>,
    named_spans: BTreeMap<String, usize>,
    stacks: HashMap<ThreadId, Vec<Entered>>,
    active: Option<Active>,
    completed: Vec<RawInvocation>,
    next_id: u64,
    error: Option<String>,
    stopped: bool,
}
impl Buffer {
    fn register(&mut self, span: u64, kind: Kind) {
        if let Kind::System(name) = &kind {
            *self.named_spans.entry(name.clone()).or_default() += 1;
        }
        self.kinds.insert(span, kind);
    }
    fn close(&mut self, span: u64) {
        if let Some(Kind::System(name)) = self.kinds.remove(&span) {
            if let Some(count) = self.named_spans.get_mut(&name) {
                *count -= 1;
                if *count == 0 {
                    self.named_spans.remove(&name);
                }
            }
        }
    }
    fn fail(&mut self, message: &str) {
        if self.error.is_none() {
            self.error = Some(message.into());
        }
    }
    fn enter(&mut self, span: u64, thread: ThreadId, at: Duration) {
        if self.stopped {
            return;
        }
        let Some(kind) = self.kinds.get(&span).cloned() else {
            return;
        };
        if matches!(&kind, Kind::Schedule(name) if name == "FixedUpdate") {
            if self.active.is_some() {
                self.fail("nested/concurrent FixedUpdate capture is unsupported");
                return;
            }
            let id = self.next_id;
            self.next_id += 1;
            self.active = Some(Active {
                id,
                span,
                at,
                nested: 0,
                systems: Vec::new(),
            });
            self.stacks.entry(thread).or_default().push(Entered {
                span,
                at,
                kind,
                invocation: Some(id),
                nested: false,
                ambiguous: false,
            });
            return;
        }
        let ambiguous = matches!(&kind, Kind::System(name) if self.named_spans.get(name).is_some_and(|count| *count > 1));
        let invocation = self.active.as_ref().map(|active| active.id);
        let nested = self.active.as_ref().is_some_and(|active| active.nested > 0)
            || self.stacks.get(&thread).is_some_and(|stack| {
                stack.iter().any(|entered| {
                    invocation.is_some()
                        && entered.invocation == invocation
                        && matches!(entered.kind, Kind::System(_))
                })
            });
        if matches!(kind, Kind::Schedule(_) | Kind::Auxiliary) {
            if let Some(active) = &mut self.active {
                active.nested += 1;
            }
        }
        self.stacks.entry(thread).or_default().push(Entered {
            span,
            at,
            kind,
            invocation,
            nested,
            ambiguous,
        });
    }
    fn exit(&mut self, span: u64, thread: ThreadId, at: Duration) {
        if self.stopped {
            return;
        }
        if !self.kinds.contains_key(&span) {
            return;
        }
        let Some(entered) = self.stacks.get_mut(&thread).and_then(|stack| stack.pop()) else {
            self.fail("unpaired tracing span exit");
            return;
        };
        if entered.span != span {
            self.fail("non-LIFO tracing span exit");
            return;
        }
        match entered.kind {
            Kind::Schedule(ref name) if name == "FixedUpdate" => {
                let Some(active) = self.active.take() else {
                    self.fail("FixedUpdate exit without entry");
                    return;
                };
                if active.span != span || active.nested != 0 {
                    self.fail("unbalanced nested schedule at FixedUpdate exit");
                    return;
                }
                if self.completed.len() >= 4096 {
                    self.fail("phase capture was not drained between frames");
                    return;
                }
                self.completed.push(RawInvocation {
                    id: active.id,
                    interval: Interval {
                        start: active.at,
                        end: at,
                    },
                    systems: active.systems,
                });
            }
            Kind::System(name) => {
                if let Some(invocation) = entered.invocation {
                    if let Some(active) = &mut self.active {
                        if active.id == invocation {
                            active.systems.push(RawSystem {
                                name,
                                interval: Interval {
                                    start: entered.at,
                                    end: at,
                                },
                                unattributed: entered.nested,
                                ambiguous: entered.ambiguous,
                            });
                        } else {
                            self.fail("system crossed a FixedUpdate boundary");
                        }
                    } else {
                        self.fail("system exited after its FixedUpdate");
                    }
                }
            }
            Kind::Schedule(_) | Kind::Auxiliary => {
                if let Some(invocation) = entered.invocation {
                    if let Some(active) = &mut self.active {
                        if active.id == invocation && active.nested > 0 {
                            active.nested -= 1;
                        } else {
                            self.fail("nested schedule crossed a FixedUpdate boundary");
                        }
                    } else {
                        self.fail("nested schedule exited after FixedUpdate");
                    }
                }
            }
        }
    }
}

struct SpanLayer {
    epoch: Instant,
    buffer: Arc<Mutex<Buffer>>,
}
impl<S: Subscriber> Layer<S> for SpanLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _: Context<'_, S>) {
        let mut name = NameField::default();
        attrs.record(&mut name);
        let kind = match attrs.metadata().name() {
            "schedule" => Kind::Schedule(name.0),
            "system" => Kind::System(name.0),
            _ => Kind::Auxiliary,
        };
        self.buffer.lock().unwrap().register(id.into_u64(), kind);
    }
    fn on_enter(&self, id: &Id, _: Context<'_, S>) {
        let at = self.epoch.elapsed();
        self.buffer
            .lock()
            .unwrap()
            .enter(id.into_u64(), std::thread::current().id(), at);
    }
    fn on_exit(&self, id: &Id, _: Context<'_, S>) {
        let at = self.epoch.elapsed();
        self.buffer
            .lock()
            .unwrap()
            .exit(id.into_u64(), std::thread::current().id(), at);
    }
    fn on_close(&self, id: Id, _: Context<'_, S>) {
        self.buffer.lock().unwrap().close(id.into_u64());
    }
}

#[derive(Default, Debug, Serialize)]
pub struct PhaseCoverage {
    pub samples: Vec<InvocationCoverage>,
    pub invocations: usize,
    pub observed_intervals: usize,
    pub unattributed_names: BTreeMap<String, usize>,
    pub nested_or_auxiliary_intervals: usize,
    pub schedule: Option<ScheduleObservation>,
}

/// Raw coverage retained alongside aggregate metric percentiles. Milliseconds
/// are evidence, never test thresholds or authoritative values.
#[derive(Debug, Serialize)]
pub struct InvocationCoverage {
    pub id: u64,
    pub elapsed_ms: f64,
    pub unobserved_ms: f64,
    pub phases: Vec<(String, f64, f64, usize)>,
    pub unattributed: Option<(f64, f64, usize)>,
}

/// Sole owner of a process-global subscriber, created before App construction.
/// It holds no App/World reference. The global layer is disabled when dropped.
pub struct PhaseProfiler {
    buffer: Arc<Mutex<Buffer>>,
    coverage: PhaseCoverage,
}
impl PhaseProfiler {
    pub fn install(log_spec: &str) -> Result<Self, String> {
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let layer = SpanLayer {
            epoch: Instant::now(),
            buffer: buffer.clone(),
        }
        .with_filter(tracing_subscriber::filter::FilterFn::new(|meta| {
            meta.is_span()
                && meta.target().starts_with("bevy_ecs::")
                && matches!(
                    meta.name(),
                    "schedule" | "system" | "check_conditions" | "system_commands"
                )
        }));
        // Match the normal headless LogPlugin's INFO level + warn-prefixed
        // explicit filter, including RUST_LOG precedence, but filter formatting
        // independently so it cannot turn off the measurement layer.
        let fallback = if log_spec.is_empty() {
            "info,warn".into()
        } else {
            format!("info,warn,{log_spec}")
        };
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::builder().parse_lossy(fallback));
        let formatting = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(filter);
        tracing_subscriber::registry()
            .with(layer)
            .with(formatting)
            .try_init()
            .map_err(|e| format!("phase profiling must install before the process logger: {e}"))?;
        Ok(Self {
            buffer,
            coverage: PhaseCoverage::default(),
        })
    }

    pub fn drain_after_frame(&mut self, app: &App, recorder: &mut Recorder) -> Result<(), String> {
        let (raw, span_counts) = {
            let mut buffer = self.buffer.lock().unwrap();
            if let Some(error) = &buffer.error {
                return Err(error.clone());
            }
            if buffer.active.is_some() {
                return Err("frame ended inside FixedUpdate".into());
            }
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for kind in buffer.kinds.values() {
                if let Kind::System(name) = kind {
                    *counts.entry(name.clone()).or_default() += 1;
                }
            }
            (std::mem::take(&mut buffer.completed), counts)
        };
        if raw.is_empty() {
            return Ok(());
        }
        let snapshot = observe_schedule(app)?;
        if self
            .coverage
            .schedule
            .as_ref()
            .is_some_and(|previous| *previous != snapshot)
        {
            return Err(
                "FixedUpdate graph changed during this capture; cannot silently mix attribution"
                    .into(),
            );
        }
        let membership = attribution(&snapshot);
        self.coverage.schedule = Some(snapshot);
        let invocations: Vec<_> = raw
            .into_iter()
            .map(|invocation| {
                let systems = invocation
                    .systems
                    .into_iter()
                    .map(|system| {
                        self.coverage.observed_intervals += 1;
                        let phase = if system.unattributed
                            || system.ambiguous
                            || span_counts.get(&system.name) != Some(&1)
                        {
                            None
                        } else {
                            membership.get(&system.name).copied().flatten()
                        };
                        if phase.is_none() {
                            *self
                                .coverage
                                .unattributed_names
                                .entry(system.name)
                                .or_default() += 1;
                        }
                        if system.unattributed {
                            self.coverage.nested_or_auxiliary_intervals += 1;
                        }
                        ObservedSystem {
                            interval: system.interval,
                            phase,
                        }
                    })
                    .collect();
                FixedInvocation {
                    id: invocation.id,
                    interval: invocation.interval,
                    systems,
                }
            })
            .collect();
        let reduced =
            phase::sample_phase_timings(recorder, &invocations).map_err(|e| e.to_string())?;
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        self.coverage
            .samples
            .extend(reduced.into_iter().map(|invocation| {
                InvocationCoverage {
                    id: invocation.id,
                    elapsed_ms: ms(invocation.elapsed),
                    unobserved_ms: ms(invocation.unobserved),
                    phases: invocation
                        .phases
                        .into_iter()
                        .map(|phase| {
                            (
                                format!("{:?}", phase.phase),
                                ms(phase.timing.observed),
                                ms(phase.timing.envelope),
                                phase.timing.intervals,
                            )
                        })
                        .collect(),
                    unattributed: invocation
                        .unattributed
                        .map(|timing| (ms(timing.observed), ms(timing.envelope), timing.intervals)),
                }
            }));
        self.coverage.invocations += invocations.len();
        Ok(())
    }
    pub fn coverage(&self) -> &PhaseCoverage {
        &self.coverage
    }
}
impl Drop for PhaseProfiler {
    fn drop(&mut self) {
        self.buffer.lock().unwrap().stopped = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }
    fn buffer() -> Buffer {
        let mut buffer = Buffer::default();
        buffer.kinds.insert(1, Kind::Schedule("FixedUpdate".into()));
        buffer.kinds.insert(2, Kind::System("outer".into()));
        buffer.kinds.insert(3, Kind::System("inner".into()));
        buffer.kinds.insert(4, Kind::Auxiliary);
        buffer
    }
    #[test]
    fn duplicate_or_composed_names_are_not_guessed_into_a_phase() {
        let snapshot = ScheduleObservation {
            systems: vec![
                ("one".into(), vec!["Input".into()]),
                ("duplicate".into(), vec!["Input".into()]),
                ("duplicate".into(), vec!["Physics".into()]),
                ("same-phase".into(), vec!["Input".into()]),
                ("same-phase".into(), vec!["Input".into()]),
                ("multi-set".into(), vec!["Input".into(), "Physics".into()]),
                ("unowned".into(), vec![]),
            ],
            ambiguities: vec![],
        };
        let map = attribution(&snapshot);
        assert_eq!(map["one"], Some(SimSet::Input));
        for name in ["duplicate", "same-phase", "multi-set", "unowned"] {
            assert_eq!(map[name], None);
        }
        assert!(!map.contains_key("composed-child-not-in-schedule"));
    }
    #[test]
    fn paired_worker_and_nested_spans_retain_real_invocation_boundaries() {
        let main = std::thread::current().id();
        let worker = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        let mut buffer = buffer();
        buffer.enter(1, main, at(0));
        buffer.enter(2, worker, at(1));
        buffer.enter(3, worker, at(2));
        buffer.exit(3, worker, at(3));
        buffer.exit(2, worker, at(4));
        buffer.enter(4, main, at(5));
        buffer.enter(3, main, at(6));
        buffer.exit(3, main, at(7));
        buffer.exit(4, main, at(8));
        buffer.exit(1, main, at(10));
        assert!(buffer.error.is_none());
        let result = &buffer.completed[0];
        assert_eq!(
            result.interval,
            Interval {
                start: at(0),
                end: at(10)
            }
        );
        assert_eq!(result.systems.len(), 3);
        assert!(result.systems[0].unattributed);
        assert!(!result.systems[1].unattributed);
        assert!(result.systems[2].unattributed);
    }
    #[test]
    fn the_outer_fixed_main_system_is_not_a_nested_fixed_update_system() {
        let thread = std::thread::current().id();
        let mut buffer = buffer();
        buffer.register(5, Kind::System("fixed-main-runner".into()));
        buffer.enter(5, thread, at(0));
        buffer.enter(1, thread, at(1));
        buffer.enter(2, thread, at(2));
        buffer.exit(2, thread, at(3));
        buffer.exit(1, thread, at(4));
        buffer.exit(5, thread, at(5));
        assert!(buffer.error.is_none());
        assert!(!buffer.completed[0].systems[0].unattributed);
    }

    #[test]
    fn a_temporary_duplicate_span_cannot_be_attributed_after_it_closes() {
        let thread = std::thread::current().id();
        let mut buffer = buffer();
        buffer.register(6, Kind::System("duplicate".into()));
        buffer.register(7, Kind::System("duplicate".into()));
        buffer.enter(1, thread, at(0));
        buffer.enter(7, thread, at(1));
        buffer.exit(7, thread, at(2));
        buffer.close(7);
        buffer.exit(1, thread, at(3));
        assert!(buffer.completed[0].systems[0].ambiguous);
        assert_eq!(buffer.named_spans["duplicate"], 1);
    }

    #[test]
    fn unbalanced_spans_fail_instead_of_inventing_complete_samples() {
        let thread = std::thread::current().id();
        let mut buffer = buffer();
        buffer.enter(1, thread, at(0));
        buffer.enter(2, thread, at(1));
        buffer.exit(1, thread, at(2));
        assert!(buffer.error.is_some());
        assert!(buffer.completed.is_empty());
    }
}
