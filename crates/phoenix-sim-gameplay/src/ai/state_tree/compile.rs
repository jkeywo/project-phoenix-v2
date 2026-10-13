use super::schema::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeError(pub String);
impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for TreeError {}
pub(super) fn fail(message: impl Into<String>) -> TreeError {
    TreeError(message.into())
}
#[derive(Clone, Debug)]
pub struct Program {
    pub(super) root: String,
    pub(super) trees: BTreeMap<String, TreeDef>,
    pub(super) bindings: SupportedBindings,
}
impl Program {
    pub fn resolved(&self) -> AiTreeConfig {
        AiTreeConfig {
            root: self.root.clone(),
            tree: self.trees.values().cloned().collect(),
        }
    }
}
fn ids<'a>(values: impl Iterator<Item = &'a str>) -> Result<(), TreeError> {
    let mut seen = BTreeSet::new();
    for id in values {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || !seen.insert(id)
        {
            return Err(fail(format!("invalid or duplicate identity {id}")));
        }
    }
    Ok(())
}
fn merge<T: Clone>(base: &mut Vec<T>, incoming: &[T], id: impl Fn(&T) -> &str) {
    for value in incoming {
        if let Some(at) = base.iter().position(|old| id(old) == id(value)) {
            base[at] = value.clone();
        } else {
            base.push(value.clone());
        }
    }
}
fn resolve(
    id: &str,
    raw: &BTreeMap<String, TreeDef>,
    out: &mut BTreeMap<String, TreeDef>,
    stack: &mut Vec<String>,
) -> Result<TreeDef, TreeError> {
    if let Some(tree) = out.get(id) {
        return Ok(tree.clone());
    }
    if stack.len() >= MAX_DEPTH || stack.iter().any(|x| x == id) {
        return Err(fail("tree inheritance cycle/depth"));
    }
    let own = raw
        .get(id)
        .ok_or_else(|| fail(format!("unknown tree {id}")))?
        .clone();
    stack.push(id.into());
    let mut tree = if let Some(parent) = &own.extends {
        resolve(parent, raw, out, stack)?
    } else {
        own.clone()
    };
    if own.extends.is_some() {
        tree.id = own.id.clone();
        tree.parameters.extend(own.parameters);
        tree.memory.extend(own.memory);
        if own.initial.is_some() {
            tree.initial = own.initial;
        }
        for state in own.state {
            if let Some(old) = tree.state.iter_mut().find(|x| x.id == state.id) {
                if state.parent.is_some() {
                    old.parent = state.parent;
                }
                if state.initial_child.is_some() {
                    old.initial_child = state.initial_child;
                }
                merge(&mut old.task, &state.task, |v| &v.id);
                merge(&mut old.transition, &state.transition, |v| &v.id);
            } else {
                tree.state.push(state);
            }
        }
    }
    tree.extends = None;
    stack.pop();
    out.insert(id.into(), tree.clone());
    Ok(tree)
}
pub(super) fn path(tree: &TreeDef, leaf: &str) -> Result<Vec<String>, TreeError> {
    let mut path = Vec::new();
    let mut next = Some(leaf);
    while let Some(id) = next {
        if path.len() >= MAX_DEPTH || path.iter().any(|v| v == id) {
            return Err(fail("state hierarchy cycle/depth"));
        }
        let state = tree
            .state
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| fail(format!("unknown state {id}")))?;
        path.push(id.to_string());
        next = state.parent.as_deref();
    }
    path.reverse();
    Ok(path)
}
pub(super) fn descend(tree: &TreeDef, start: &str) -> Result<String, TreeError> {
    let mut current = start.to_string();
    for _ in 0..MAX_DEPTH {
        let state = tree
            .state
            .iter()
            .find(|s| s.id == current)
            .ok_or_else(|| fail("unknown initial state"))?;
        match &state.initial_child {
            Some(child) => current = child.clone(),
            None => return Ok(current),
        }
    }
    Err(fail("initial child depth"))
}
fn source(
    value: &BoolSource,
    tree: &TreeDef,
    bindings: &SupportedBindings,
) -> Result<(), TreeError> {
    let valid = match value {
        BoolSource::Literal { .. } => true,
        BoolSource::Fact { name } => bindings.facts.contains(name),
        BoolSource::Parameter { name } => tree.parameters.contains_key(name),
        BoolSource::Memory { name } => tree.memory.contains_key(name),
        BoolSource::Pending { channel } => bindings.channels.contains(channel),
        BoolSource::CallResult { task, .. } => tree.state.iter().any(|s| {
            s.task.iter().any(|t| {
                format!("{}/{}", s.id, t.id) == *task && matches!(t.action, TaskAction::Call { .. })
            })
        }),
    };
    if valid {
        Ok(())
    } else {
        Err(fail(format!("unsupported source {value:?} in {}", tree.id)))
    }
}
fn conditions(
    values: &[BoolCondition],
    tree: &TreeDef,
    bindings: &SupportedBindings,
) -> Result<(), TreeError> {
    if values.len() > MAX_CONDITIONS {
        return Err(fail("condition bound exceeded"));
    }
    for c in values {
        source(&c.value, tree, bindings)?;
    }
    Ok(())
}
fn budget(
    id: &str,
    trees: &BTreeMap<String, TreeDef>,
    stack: &mut Vec<String>,
) -> Result<usize, TreeError> {
    if stack.len() >= MAX_DEPTH || stack.iter().any(|v| v == id) {
        return Err(fail("reference cycle/depth"));
    }
    stack.push(id.into());
    let mut total = 1usize;
    for s in &trees[id].state {
        total += 1;
        for t in &s.transition {
            total += 1 + t.when.len();
        }
        for t in &s.task {
            total += 2 + t.when.len();
            if let TaskAction::Call { tree, parameters } = &t.action {
                total += parameters.len() + budget(tree, trees, stack)?;
            }
            if total > MAX_VISITS {
                return Err(fail("expanded execution budget exceeded"));
            }
        }
    }
    stack.pop();
    if total > MAX_VISITS {
        Err(fail("expanded execution budget exceeded"))
    } else {
        Ok(total)
    }
}
pub fn compile(config: &AiTreeConfig, bindings: &SupportedBindings) -> Result<Program, TreeError> {
    if config.tree.is_empty() || config.tree.len() > MAX_TREES {
        return Err(fail("tree count out of bounds"));
    }
    ids(config.tree.iter().map(|t| t.id.as_str()))?;
    for t in &config.tree {
        ids(t.state.iter().map(|s| s.id.as_str()))?;
        for s in &t.state {
            ids(s.task.iter().map(|n| n.id.as_str()))?;
            ids(s.transition.iter().map(|n| n.id.as_str()))?;
        }
    }
    let raw: BTreeMap<_, _> = config
        .tree
        .iter()
        .map(|t| (t.id.clone(), t.clone()))
        .collect();
    // Check every ancestry independently: resolved-cache hits must not hide depth.
    for id in raw.keys() {
        let mut seen = BTreeSet::new();
        let mut next = Some(id.as_str());
        while let Some(id) = next {
            if seen.len() >= MAX_DEPTH || !seen.insert(id) {
                return Err(fail("tree inheritance cycle/depth"));
            }
            next = raw
                .get(id)
                .ok_or_else(|| fail("unknown inherited tree"))?
                .extends
                .as_deref();
        }
    }
    let mut trees = BTreeMap::new();
    for id in raw.keys() {
        resolve(id, &raw, &mut trees, &mut Vec::new())?;
    }
    if !trees.contains_key(&config.root) {
        return Err(fail("root tree missing"));
    }
    for tree in trees.values() {
        if tree.state.is_empty()
            || tree.state.len() > MAX_STATES
            || tree.memory.len() > MAX_VARIABLES
            || tree.parameters.len() > MAX_VARIABLES
            || tree.state.iter().map(|s| s.task.len()).sum::<usize>() > MAX_TASKS
            || tree.state.iter().map(|s| s.transition.len()).sum::<usize>() > MAX_TRANSITIONS
        {
            return Err(fail("tree size bound exceeded"));
        }
        ids(tree.memory.keys().map(String::as_str))?;
        ids(tree.parameters.keys().map(String::as_str))?;
        let initial = tree
            .initial
            .as_deref()
            .ok_or_else(|| fail("initial state missing"))?;
        if path(tree, initial)?.len() != 1 {
            return Err(fail("initial state must be a root state"));
        }
        descend(tree, initial)?;
        for state in &tree.state {
            path(tree, &state.id)?;
            if let Some(child) = &state.initial_child {
                let child = tree
                    .state
                    .iter()
                    .find(|s| &s.id == child)
                    .ok_or_else(|| fail("initial child missing"))?;
                if child.parent.as_deref() != Some(&state.id) {
                    return Err(fail("initial child is not a direct child"));
                }
                descend(tree, &state.id)?;
            }
            for tr in &state.transition {
                conditions(&tr.when, tree, bindings)?;
                if let TransitionTarget::State { state } = &tr.target {
                    path(tree, state)?;
                }
            }
            for task in &state.task {
                conditions(&task.when, tree, bindings)?;
                match &task.action {
                    TaskAction::Write { channel, value } => {
                        if !bindings.channels.contains(channel) {
                            return Err(fail("unknown output channel"));
                        }
                        source(value, tree, bindings)?;
                    }
                    TaskAction::Remember { key, value } => {
                        if !tree.memory.contains_key(key) {
                            return Err(fail("unknown memory key"));
                        }
                        source(value, tree, bindings)?;
                    }
                    TaskAction::Call {
                        tree: callee,
                        parameters,
                    } => {
                        let callee = trees
                            .get(callee)
                            .ok_or_else(|| fail("unknown reference tree"))?;
                        for (key, value) in parameters {
                            if !callee.parameters.contains_key(key) {
                                return Err(fail("unknown reference parameter"));
                            }
                            source(value, tree, bindings)?;
                        }
                    }
                    TaskAction::Complete { .. } => {}
                }
            }
        }
    }
    for id in trees.keys() {
        budget(id, &trees, &mut Vec::new())?;
    }
    Ok(Program {
        root: config.root.clone(),
        trees,
        bindings: bindings.clone(),
    })
}
