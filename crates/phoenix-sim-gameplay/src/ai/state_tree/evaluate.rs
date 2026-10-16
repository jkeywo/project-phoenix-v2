use super::{
    compile::{descend, fail, path},
    schema::*,
    Program, TreeError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<crate::ai::text_reading::TextReading>,
    pub root: Option<FrameRuntime>,
    pub last_tick: Option<u64>,
}
impl TreeRuntime {
    pub fn memory(&self, key: &str) -> Option<bool> {
        self.root.as_ref()?.memory.get(key).copied()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameRuntime {
    pub tree: String,
    pub active_leaf: String,
    pub memory: BTreeMap<String, bool>,
    pub calls: BTreeMap<String, FrameRuntime>,
    pub result: Option<Completion>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TraceEvent {
    WriteNumber {
        node: String,
        channel: String,
        value: f64,
    },
    Read {
        node: String,
        source: BoolSource,
        value: bool,
    },
    Write {
        node: String,
        channel: String,
        value: bool,
    },
    Remember {
        node: String,
        key: String,
        value: bool,
    },
    Transition {
        node: String,
        target: TransitionTarget,
    },
    Call {
        node: String,
        tree: String,
    },
    Complete {
        node: String,
        result: Completion,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct StepResult {
    pub intents: BTreeMap<String, bool>,
    pub numeric_intents: BTreeMap<String, f64>,
    pub trace: Vec<TraceEvent>,
}
fn frame(tree: &TreeDef) -> Result<FrameRuntime, TreeError> {
    Ok(FrameRuntime {
        tree: tree.id.clone(),
        active_leaf: descend(
            tree,
            tree.initial
                .as_deref()
                .ok_or_else(|| fail("missing initial"))?,
        )?,
        memory: tree.memory.clone(),
        calls: BTreeMap::new(),
        result: None,
    })
}
fn validate(
    frame: &FrameRuntime,
    expected: &str,
    program: &Program,
    depth: usize,
    count: &mut usize,
) -> Result<(), TreeError> {
    *count += 1;
    if depth >= MAX_DEPTH || *count > MAX_VISITS || frame.tree != expected {
        return Err(fail("invalid runtime frame"));
    }
    let tree = &program.trees[expected];
    let active = path(tree, &frame.active_leaf)?;
    if tree
        .state
        .iter()
        .any(|state| state.id == frame.active_leaf && state.initial_child.is_some())
    {
        return Err(fail("runtime active leaf requires initial-child descent"));
    }
    if frame.memory.keys().ne(tree.memory.keys())
        || frame.calls.len() > MAX_TASKS
        || (frame.result.is_some() && !frame.calls.is_empty())
    {
        return Err(fail("invalid runtime memory/calls"));
    }
    for (key, child) in &frame.calls {
        let mut found = None;
        for state in tree.state.iter().filter(|s| active.contains(&s.id)) {
            for task in &state.task {
                if format!("{}/{}", state.id, task.id) == *key {
                    if let TaskAction::Call { tree, .. } = &task.action {
                        found = Some(tree.as_str());
                    }
                }
            }
        }
        validate(
            child,
            found.ok_or_else(|| fail("inactive runtime call"))?,
            program,
            depth + 1,
            count,
        )?;
    }
    Ok(())
}
struct Context<'a> {
    program: &'a Program,
    facts: &'a BTreeMap<String, bool>,
    numeric_facts: &'a BTreeMap<String, f64>,
    pending: BTreeMap<String, bool>,
    intents: BTreeMap<String, bool>,
    numeric_intents: BTreeMap<String, f64>,
    trace: Vec<TraceEvent>,
    visits: usize,
}
impl Context<'_> {
    fn visit(&mut self) -> Result<(), TreeError> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            Err(fail("execution budget exceeded"))
        } else {
            Ok(())
        }
    }
    fn read(
        &mut self,
        value: &BoolSource,
        frame: &FrameRuntime,
        params: &BTreeMap<String, bool>,
        node: &str,
    ) -> Result<bool, TreeError> {
        self.visit()?;
        let result = match value {
            BoolSource::NumberCompare {
                name,
                comparison,
                value,
            } => comparison.compare(self.numeric_facts[name], *value),
            BoolSource::Literal { value } => *value,
            BoolSource::Fact { name } => self.facts[name],
            BoolSource::Parameter { name } => params[name],
            BoolSource::Memory { name } => frame.memory[name],
            BoolSource::Pending { channel } => self.pending[channel],
            BoolSource::CallResult { task, result } => frame
                .calls
                .get(task)
                .is_some_and(|f| f.result == Some(*result)),
        };
        self.trace.push(TraceEvent::Read {
            node: node.into(),
            source: value.clone(),
            value: result,
        });
        Ok(result)
    }
    fn guard(
        &mut self,
        conditions: &[BoolCondition],
        frame: &FrameRuntime,
        params: &BTreeMap<String, bool>,
        node: &str,
    ) -> Result<bool, TreeError> {
        for c in conditions {
            if self.read(&c.value, frame, params, node)? != c.equals {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn run(
        &mut self,
        frame: &mut FrameRuntime,
        params: &BTreeMap<String, bool>,
        prefix: &str,
    ) -> Result<(), TreeError> {
        self.visit()?;
        if frame.result.is_some() {
            return Ok(());
        }
        // Program is external immutable data, not part of the mutable execution context.
        let program = self.program;
        let tree = &program.trees[&frame.tree];
        let old = path(tree, &frame.active_leaf)?;
        let mut transition = None;
        'outer: for (depth, id) in old.iter().enumerate() {
            let state = tree.state.iter().find(|s| &s.id == id).unwrap();
            for tr in &state.transition {
                self.visit()?;
                let node = format!("{prefix}/{id}/{}", tr.id);
                if self.guard(&tr.when, frame, params, &node)? {
                    transition = Some((depth, node, tr.target.clone()));
                    break 'outer;
                }
            }
        }
        if let Some((depth, node, target)) = transition {
            self.trace.push(TraceEvent::Transition {
                node: node.clone(),
                target: target.clone(),
            });
            match target {
                TransitionTarget::Complete { result } => {
                    frame.result = Some(result);
                    frame.calls.clear();
                    self.trace.push(TraceEvent::Complete { node, result });
                    return Ok(());
                }
                TransitionTarget::State { state } => {
                    frame.active_leaf = descend(tree, &state)?;
                    let new = path(tree, &frame.active_leaf)?;
                    let keep: Vec<_> = old
                        .iter()
                        .zip(&new)
                        .take(depth)
                        .take_while(|(a, b)| a == b)
                        .map(|(a, _)| format!("{a}/"))
                        .collect();
                    frame
                        .calls
                        .retain(|key, _| keep.iter().any(|p| key.starts_with(p)));
                }
            }
        }
        for id in path(tree, &frame.active_leaf)? {
            self.visit()?;
            let state = tree.state.iter().find(|s| s.id == id).unwrap();
            for task in &state.task {
                self.visit()?;
                let key = format!("{id}/{}", task.id);
                let node = format!("{prefix}/{key}");
                if !self.guard(&task.when, frame, params, &node)? {
                    frame.calls.remove(&key);
                    continue;
                }
                match &task.action {
                    TaskAction::WriteNumber { channel, value } => {
                        self.numeric_intents.insert(channel.clone(), *value);
                        self.trace.push(TraceEvent::WriteNumber {
                            node,
                            channel: channel.clone(),
                            value: *value,
                        });
                    }
                    TaskAction::Write { channel, value } => {
                        let value = self.read(value, frame, params, &node)?;
                        self.pending.insert(channel.clone(), value);
                        self.intents.insert(channel.clone(), value);
                        self.trace.push(TraceEvent::Write {
                            node,
                            channel: channel.clone(),
                            value,
                        });
                    }
                    TaskAction::Remember { key, value } => {
                        let value = self.read(value, frame, params, &node)?;
                        frame.memory.insert(key.clone(), value);
                        self.trace.push(TraceEvent::Remember {
                            node,
                            key: key.clone(),
                            value,
                        });
                    }
                    TaskAction::Complete { result } => {
                        frame.result = Some(*result);
                        frame.calls.clear();
                        self.trace.push(TraceEvent::Complete {
                            node,
                            result: *result,
                        });
                        return Ok(());
                    }
                    TaskAction::Call {
                        tree: callee,
                        parameters,
                    } => {
                        let definition = &program.trees[callee];
                        let mut bound = definition.parameters.clone();
                        for (name, source) in parameters {
                            bound.insert(name.clone(), self.read(source, frame, params, &node)?);
                        }
                        let mut child = match frame.calls.remove(&key) {
                            Some(child) => child,
                            None => self::frame(definition)?,
                        };
                        self.trace.push(TraceEvent::Call {
                            node: node.clone(),
                            tree: callee.clone(),
                        });
                        self.run(&mut child, &bound, &node)?;
                        frame.calls.insert(key, child);
                    }
                }
            }
        }
        Ok(())
    }
}
/// Validate private continuation without evaluating tasks or producing effects.
pub fn validate_runtime(program: &Program, runtime: &TreeRuntime) -> Result<(), TreeError> {
    if let Some(reading) = &runtime.reading {
        reading
            .validate(reading.last_tick.unwrap_or(0))
            .map_err(fail)?;
    }
    if let Some(root) = &runtime.root {
        validate(root, &program.root, program, 0, &mut 0)?;
    } else if runtime.last_tick.is_some() {
        return Err(fail("missing runtime root"));
    }
    Ok(())
}

pub fn step(
    program: &Program,
    facts: &BTreeMap<String, bool>,
    current_channels: &BTreeMap<String, bool>,
    runtime: &mut TreeRuntime,
    tick: u64,
) -> Result<StepResult, TreeError> {
    step_with_numbers(
        program,
        facts,
        &BTreeMap::new(),
        current_channels,
        runtime,
        tick,
    )
}

pub fn step_with_numbers(
    program: &Program,
    facts: &BTreeMap<String, bool>,
    numeric_facts: &BTreeMap<String, f64>,
    current_channels: &BTreeMap<String, bool>,
    runtime: &mut TreeRuntime,
    tick: u64,
) -> Result<StepResult, TreeError> {
    if runtime.last_tick.is_some_and(|last| tick <= last) {
        return Err(fail("tree tick must advance"));
    }
    if facts.keys().ne(program.bindings.facts.iter())
        || current_channels.keys().ne(program.bindings.channels.iter())
        || numeric_facts.keys().ne(program.numeric.facts.iter())
        || numeric_facts.values().any(|value| !value.is_finite())
    {
        return Err(fail("input bindings do not match compiled interface"));
    }
    validate_runtime(program, runtime)?;
    let mut next = runtime.clone();
    let root = match next.root.as_mut() {
        Some(root) => root,
        None => next.root.insert(frame(&program.trees[&program.root])?),
    };
    let mut context = Context {
        program,
        facts,
        numeric_facts,
        pending: current_channels.clone(),
        intents: BTreeMap::new(),
        numeric_intents: BTreeMap::new(),
        trace: Vec::new(),
        visits: 0,
    };
    context.run(
        root,
        &program.trees[&program.root].parameters,
        &program.root,
    )?;
    next.last_tick = Some(tick);
    *runtime = next;
    Ok(StepResult {
        intents: context.intents,
        numeric_intents: context.numeric_intents,
        trace: context.trace,
    })
}
