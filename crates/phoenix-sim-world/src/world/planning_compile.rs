use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainSource {
    pub path: String,
    pub text: String,
}
#[derive(Clone, Debug)]
pub struct CompiledPlanning {
    pub(super) config: PlanningConfig,
    pub(super) libraries: BTreeMap<String, DomainLibrary>,
}
pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}
fn asset_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.starts_with('/')
        && path.ends_with(".toml")
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
fn unique_ids<'a>(values: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .all(|id| identifier(id) && seen.insert(id))
}
fn validate_imports(imports: &[DomainImport]) -> Result<(), String> {
    if imports.len() > MAX_LIBRARIES
        || !unique_ids(imports.iter().map(|row| row.namespace.as_str()))
        || imports.iter().any(|row| {
            !asset_path(&row.path)
                || row.names.is_empty()
                || !unique_ids(row.names.iter().map(String::as_str))
        })
    {
        return Err("planning imports require distinct namespaces, canonical relative TOML paths and explicit distinct exported names".into());
    }
    Ok(())
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
pub(super) fn validate_value(value: &Value) -> Result<(), String> {
    let valid = match value {
        Value::Actor(value) => text(value),
        Value::System(value) => identifier(&value.0),
        Value::Team(_) | Value::RepairGoal(_) => true,
        Value::Store(value) | Value::Commodity(value) => identifier(value),
        Value::Quantity(value) => *value > 0,
        Value::RepairTarget(phoenix_model::damage::RepairWorkTarget::Compartment {
            compartment,
        }) => identifier(compartment),
        Value::RepairTarget(phoenix_model::damage::RepairWorkTarget::Component {
            system,
            component,
        }) => identifier(&system.0) && identifier(component),
        Value::RepairRequest(request) => match request {
            phoenix_model::messages::RepairTarget::External => false,
            phoenix_model::messages::RepairTarget::Core => true,
            phoenix_model::messages::RepairTarget::System(value) => identifier(&value.0),
            phoenix_model::messages::RepairTarget::Station(value) => identifier(&value.0),
            phoenix_model::messages::RepairTarget::Console(value) => identifier(&value.0),
            phoenix_model::messages::RepairTarget::Work(value) => {
                validate_value(&Value::RepairTarget(value.clone())).is_ok()
            }
        },
    };
    if valid {
        Ok(())
    } else {
        Err("invalid typed planning argument value".into())
    }
}
fn validate_parameters(parameters: &[Parameter]) -> Result<(), String> {
    if parameters.len() > 32 || !unique_ids(parameters.iter().map(|row| row.name.as_str())) {
        return Err("task parameters require bounded distinct typed names".into());
    }
    Ok(())
}
fn template_arguments(request: &RequestTemplate) -> Vec<(&Argument, ParameterKind)> {
    use ParameterKind as Kind;
    match request {
        RequestTemplate::Communication {
            actor,
            owner,
            recipient,
            ..
        } => vec![
            (actor, Kind::Actor),
            (owner, Kind::System),
            (recipient, Kind::Actor),
        ],
        RequestTemplate::Repair {
            actor,
            owner,
            team,
            request,
            target,
            goal,
        } => vec![
            (actor, Kind::Actor),
            (owner, Kind::System),
            (team, Kind::Team),
            (request, Kind::RepairRequest),
            (target, Kind::RepairTarget),
            (goal, Kind::RepairGoal),
        ],
        RequestTemplate::Delivery {
            actor,
            owner,
            source,
            destination,
            commodity,
            amount,
        } => vec![
            (actor, Kind::Actor),
            (owner, Kind::System),
            (source, Kind::Store),
            (destination, Kind::Store),
            (commodity, Kind::Commodity),
            (amount, Kind::Quantity),
        ],
    }
}
fn validate_argument(
    argument: &Argument,
    expected: ParameterKind,
    parameters: &[Parameter],
) -> Result<(), String> {
    let kind = match argument {
        Argument::Literal { value } => {
            validate_value(value)?;
            value.kind()
        }
        Argument::Parameter { name } => {
            parameters
                .iter()
                .find(|row| &row.name == name)
                .ok_or_else(|| format!("unknown task parameter {name:?}"))?
                .kind
        }
    };
    if kind != expected {
        return Err(format!(
            "planning parameter type {kind:?} does not match {expected:?}"
        ));
    }
    Ok(())
}

impl CompiledPlanning {
    pub fn config(&self) -> &PlanningConfig {
        &self.config
    }
    pub fn libraries(&self) -> impl Iterator<Item = (&str, &DomainLibrary)> {
        self.libraries
            .iter()
            .map(|(path, library)| (path.as_str(), library))
    }

    /// Read only explicit immutable source. The caller applies returned records
    /// to its existing content ledger; this pure owner never mutates that ledger.
    pub fn load(
        config: PlanningConfig,
        mut read: impl FnMut(&str) -> Option<String>,
    ) -> Result<(Self, Vec<DomainSource>), String> {
        if !identifier(&config.id) || config.arguments.len() > 32 {
            return Err("invalid planning root identity/arguments".into());
        }
        validate_imports(&config.imports)?;
        for value in config.arguments.values() {
            validate_value(value)?;
        }
        let mut libraries = BTreeMap::new();
        let mut sources = Vec::new();
        let mut stack = Vec::new();
        fn visit(
            path: &str,
            read: &mut impl FnMut(&str) -> Option<String>,
            libraries: &mut BTreeMap<String, DomainLibrary>,
            sources: &mut Vec<DomainSource>,
            stack: &mut Vec<String>,
        ) -> Result<(), String> {
            if stack.iter().any(|row| row == path) {
                return Err(format!("cyclic planning import {path:?}"));
            }
            if libraries.contains_key(path) {
                return Ok(());
            }
            if stack.len() >= MAX_IMPORT_DEPTH || libraries.len() + stack.len() >= MAX_LIBRARIES {
                return Err("planning import bound exhausted".into());
            }
            let text =
                read(path).ok_or_else(|| format!("planning library {path:?} is unavailable"))?;
            let library: DomainLibrary = toml::from_str(&text)
                .map_err(|error| format!("planning library {path:?}: {error}"))?;
            if library.domain.format != DOMAIN_FORMAT
                || !identifier(&library.domain.id)
                || library.tasks.is_empty()
                || !unique_ids(library.tasks.iter().map(Task::id))
                || library.domain.exports.is_empty()
                || !unique_ids(library.domain.exports.iter().map(String::as_str))
                || library
                    .domain
                    .requires
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != library.domain.requires.len()
            {
                return Err(format!("invalid planning library header/tasks {path:?}"));
            }
            validate_imports(&library.imports)?;
            stack.push(path.to_owned());
            for import in &library.imports {
                visit(&import.path, read, libraries, sources, stack)?;
            }
            stack.pop();
            sources.push(DomainSource {
                path: path.into(),
                text,
            });
            libraries.insert(path.into(), library);
            Ok(())
        }
        for import in &config.imports {
            visit(
                &import.path,
                &mut read,
                &mut libraries,
                &mut sources,
                &mut stack,
            )?;
        }
        let compiled = Self { config, libraries };
        compiled.validate()?;
        Ok((compiled, sources))
    }
    pub(super) fn resolve(
        &self,
        caller: Option<&str>,
        reference: &str,
    ) -> Result<TaskIdentity, String> {
        if let Some((namespace, task)) = reference.split_once("::") {
            if !identifier(namespace) || !identifier(task) {
                return Err(format!("invalid task reference {reference:?}"));
            }
            let imports = caller.map_or(&self.config.imports, |path| &self.libraries[path].imports);
            let import = imports
                .iter()
                .find(|row| row.namespace == namespace && row.names.iter().any(|name| name == task))
                .ok_or_else(|| format!("task {reference:?} was not explicitly imported"))?;
            let library = self
                .libraries
                .get(&import.path)
                .ok_or("missing imported planning library")?;
            if !library.domain.exports.iter().any(|name| name == task)
                || !library.tasks.iter().any(|row| row.id() == task)
            {
                return Err(format!("task {reference:?} is not exported"));
            }
            Ok(TaskIdentity {
                library_path: import.path.clone(),
                task: task.into(),
            })
        } else {
            let path = caller.ok_or("planning root must name an explicitly imported task")?;
            if !identifier(reference)
                || !self.libraries[path]
                    .tasks
                    .iter()
                    .any(|task| task.id() == reference)
            {
                return Err(format!("unknown local task {reference:?}"));
            }
            Ok(TaskIdentity {
                library_path: path.into(),
                task: reference.into(),
            })
        }
    }
    pub(super) fn task(&self, id: &TaskIdentity) -> &Task {
        self.libraries[&id.library_path]
            .tasks
            .iter()
            .find(|task| task.id() == id.task)
            .expect("validated planning task")
    }
    fn validate(&self) -> Result<(), String> {
        if let Some(policy) = &self.config.temporal {
            policy.validate()?;
        }
        let count = self
            .libraries
            .values()
            .map(|library| {
                library.tasks.len()
                    + library.definitions.len()
                    + library
                        .tasks
                        .iter()
                        .map(|task| match task {
                            Task::Primitive { .. } => 0,
                            Task::Compound { methods, .. } => {
                                methods.len()
                                    + methods
                                        .iter()
                                        .map(|method| method.steps.len() + method.edges.len())
                                        .sum::<usize>()
                            }
                        })
                        .sum::<usize>()
            })
            .sum::<usize>();
        if count > MAX_DEFINITIONS {
            return Err("planning definition bound exhausted".into());
        }
        for (path, library) in &self.libraries {
            if !unique_ids(library.definitions.iter().map(|row| row.id.as_str())) {
                return Err("duplicate/invalid domain Scenario definition".into());
            }
            for definition in &library.definitions {
                definition.validate()?;
            }
            for export in &library.domain.exports {
                if !library.tasks.iter().any(|task| task.id() == export) {
                    return Err(format!("unresolved domain export {export:?}"));
                }
            }
            for import in &library.imports {
                for name in &import.names {
                    self.resolve(Some(path), &format!("{}::{name}", import.namespace))?;
                }
            }
            for task in &library.tasks {
                validate_parameters(task.parameters())?;
                match task {
                    Task::Primitive {
                        definition,
                        request,
                        forecast,
                        ..
                    } => {
                        if let RequestTemplate::Communication { channel, .. } = request {
                            if !phoenix_model::comms_channel::channel_id(channel) {
                                return Err(
                                    "communication primitive requires a supported authored channel"
                                        .into(),
                                );
                            }
                        }
                        if let Some(forecast) = forecast {
                            forecast.validate()?;
                        }
                        if !library.definitions.iter().any(|row| &row.id == definition) {
                            return Err("primitive references absent Scenario definition".into());
                        }
                        let family = match request {
                            RequestTemplate::Communication { .. } => {
                                OperatorFamily::CommunicationReportStatus
                            }
                            RequestTemplate::Repair { .. } => OperatorFamily::RepairInternal,
                            RequestTemplate::Delivery { .. } => OperatorFamily::DeliveryLocal,
                        };
                        if !library.domain.requires.contains(&family) {
                            return Err(
                                "operator family missing from domain compatibility requirements"
                                    .into(),
                            );
                        }
                        for (argument, kind) in template_arguments(request) {
                            validate_argument(argument, kind, task.parameters())?;
                        }
                    }
                    Task::Compound { methods, .. } => {
                        if methods.is_empty()
                            || methods.len() > MAX_DEFINITIONS
                            || !unique_ids(methods.iter().map(|method| method.id.as_str()))
                        {
                            return Err("compound requires bounded distinct methods".into());
                        }
                        for method in methods {
                            if method.steps.is_empty()
                                || method.steps.len() > MAX_EXPANDED_NODES
                                || !unique_ids(method.steps.iter().map(|step| step.id.as_str()))
                            {
                                return Err("method requires bounded distinct calls".into());
                            }
                            for step in &method.steps {
                                let target = self.resolve(Some(path), &step.task)?;
                                let target = self.task(&target);
                                if step.arguments.len() != target.parameters().len() {
                                    return Err("method call parameter set mismatch".into());
                                }
                                for parameter in target.parameters() {
                                    validate_argument(
                                        step.arguments
                                            .get(&parameter.name)
                                            .ok_or("missing method call parameter")?,
                                        parameter.kind,
                                        task.parameters(),
                                    )?;
                                }
                            }
                            if method.edges.len() > MAX_EXPANDED_NODES {
                                return Err("method dependency bound exhausted".into());
                            }
                        }
                    }
                }
            }
        }
        for import in &self.config.imports {
            for name in &import.names {
                self.resolve(None, &format!("{}::{name}", import.namespace))?;
            }
        }
        let root = self.resolve(None, &self.config.task)?;
        let task = self.task(&root);
        if self.config.arguments.len() != task.parameters().len() {
            return Err("root argument set mismatch".into());
        }
        for parameter in task.parameters() {
            let value = self
                .config
                .arguments
                .get(&parameter.name)
                .ok_or("missing root argument")?;
            if value.kind() != parameter.kind {
                return Err("root argument type mismatch".into());
            }
        }
        // Reject recursion in every imported definition, including an unused
        // alternative; runtime bounds never make a cyclic source acceptable.
        fn acyclic(
            owner: &CompiledPlanning,
            id: TaskIdentity,
            active: &mut BTreeSet<TaskIdentity>,
            done: &mut BTreeSet<TaskIdentity>,
        ) -> Result<(), String> {
            if done.contains(&id) {
                return Ok(());
            }
            if !active.insert(id.clone()) {
                return Err("cyclic HTN task definitions".into());
            }
            if let Task::Compound { methods, .. } = owner.task(&id) {
                for method in methods {
                    for step in &method.steps {
                        acyclic(
                            owner,
                            owner.resolve(Some(&id.library_path), &step.task)?,
                            active,
                            done,
                        )?;
                    }
                }
            }
            active.remove(&id);
            done.insert(id);
            Ok(())
        }
        let mut done = BTreeSet::new();
        for (path, library) in &self.libraries {
            for task in &library.tasks {
                acyclic(
                    self,
                    TaskIdentity {
                        library_path: path.clone(),
                        task: task.id().into(),
                    },
                    &mut BTreeSet::new(),
                    &mut done,
                )?;
            }
        }
        Ok(())
    }
}
