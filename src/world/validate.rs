// World authoring validation (issue #750).
//
// Pure Rust module — no Bevy. Resolves every authored world reference across
// the effective composition (root world + additive `extra_worlds` + any
// caller-supplied child worlds) BEFORE the root world is activated, and reports
// **source-located** findings. Any error finding blocks activation of the
// entire root world; nothing spawns partially.
//
// # Identity vs. display (world-identity-contract)
//
// `WorldEntity.name` is the *unique authored reference id* — the only thing
// world references (triggers, comms senders, objective targets, qualified
// composition references) resolve against. `WorldEntity.display_name` is the
// separate player-facing text. Duplicate `name`s inside one namespace are an
// authoring error.
//
// # Qualified references (world-composition-contract)
//
// A world reference may be *bare* (`axiom_station`) or *qualified*:
//
// * `parent.<name>` climbs one composition layer; repeated `parent.` segments
//   climb further (`parent.parent.<name>`). Climbing past the root is an error.
// * `<alias>.<name>` qualifies a name into a named child world, where `<alias>`
//   is a declared child-world alias (the child TOML's file stem).
//
// Note the deliberate **syntax split**: composition *entity* references use the
// PASM dot syntax (`parent.<name>`), while the pre-existing layered *flag*
// resolver (`content::resolve_layer_prefix`) keeps its colon syntax
// (`parent:<flag>`) so shipped worlds (`btf_path_a.toml`,
// `btf_aphelion_protocol.toml`) that address flags across layers are untouched.
// Flags live in a layered `FlagStore`; entity names live in per-world
// namespaces — different resolution domains, different syntaxes.

use std::collections::{HashMap, HashSet};

use crate::world::config::{
    CommsDialogueNode, TriggerAction, TriggerCondition, WorldConfig, WorldEntity,
};

/// Severity of a [`WorldFinding`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Blocks activation of the entire root world.
    Error,
    /// Reported but non-blocking.
    Warning,
}

/// Where a finding originates: the source file, a best-effort 1-based line, and
/// the offending reference string (issue #750).
///
/// The line is derived by scanning the raw TOML for the reference string, so it
/// is "best effort": `None` when the reference cannot be located (e.g. a
/// synthesised reference or a name that also appears as a substring elsewhere).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    /// World file path the reference was authored in.
    pub file: String,
    /// 1-based line number, best-effort.
    pub line: Option<usize>,
    /// The offending reference string as authored.
    pub reference: String,
}

/// A single source-located validation finding
/// (`world-authoring-validation-state`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldFinding {
    pub severity: Severity,
    /// Short kebab-case category slug: `duplicate-name`, `unresolved-reference`,
    /// `ambiguous-reference`, `invalid-qualified-reference`.
    pub category: &'static str,
    pub message: String,
    pub source: SourceLocation,
}

impl WorldFinding {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    fn error(
        category: &'static str,
        file: &str,
        source_text: &str,
        reference: &str,
        message: String,
    ) -> Self {
        WorldFinding {
            severity: Severity::Error,
            category,
            message,
            source: SourceLocation {
                file: file.to_string(),
                line: line_of(source_text, reference),
                reference: reference.to_string(),
            },
        }
    }
}

/// True when any finding is an error — the atomic-activation gate.
pub fn has_error(findings: &[WorldFinding]) -> bool {
    findings.iter().any(WorldFinding::is_error)
}

/// Best-effort 1-based line lookup: the first line of `source` that contains
/// `needle`. Returns `None` when `needle` is empty or not present.
pub fn line_of(source: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    source
        .lines()
        .position(|l| l.contains(needle))
        .map(|i| i + 1)
}

/// One parsed world in the effective composition: its authored path, raw TOML
/// (for source-location lookup), and parsed config.
pub struct WorldSource<'a> {
    pub path: String,
    pub toml: &'a str,
    pub config: &'a WorldConfig,
}

impl<'a> WorldSource<'a> {
    pub fn new(path: impl Into<String>, toml: &'a str, config: &'a WorldConfig) -> Self {
        WorldSource {
            path: path.into(),
            toml,
            config,
        }
    }

    /// The child-world alias for this source: the file stem of its path
    /// (`assets/worlds/btf_path_a.toml` -> `btf_path_a`). Used to qualify
    /// references as `<alias>.<name>`.
    fn alias(&self) -> String {
        self.path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.path)
            .strip_suffix(".toml")
            .unwrap_or(&self.path)
            .to_string()
    }
}

/// A parsed qualified reference (`world-composition-contract`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QualifiedRef {
    /// A bare name with no qualifier: resolves in the referencing world's own
    /// namespace.
    Bare(String),
    /// `parent.` * depth + name: climbs `depth` composition layers.
    Parent { depth: usize, name: String },
    /// `<alias>.<name>`: resolves in the named child world's namespace.
    Child { alias: String, name: String },
    /// Structurally invalid (empty segment, e.g. `parent.` or trailing dot).
    Invalid(String),
}

/// Parse a world reference into its qualified form.
///
/// `child_aliases` is the set of declared child-world aliases; a leading
/// segment that matches one is treated as a `Child` qualifier. A leading
/// `parent` segment (dot syntax) is the reserved parent qualifier. Anything
/// else is a `Bare` name (dots are legal inside bare names — shipped worlds use
/// localization keys like `world.entity.axiom_station.name`).
pub fn parse_qualified_reference(reference: &str, child_aliases: &[String]) -> QualifiedRef {
    if let Some(rest) = reference.strip_prefix("parent.") {
        let mut depth = 1;
        let mut name = rest;
        while let Some(next) = name.strip_prefix("parent.") {
            depth += 1;
            name = next;
        }
        if name.is_empty() || name.starts_with("parent.") {
            return QualifiedRef::Invalid(reference.to_string());
        }
        return QualifiedRef::Parent {
            depth,
            name: name.to_string(),
        };
    }

    // `<alias>.<name>` only when the leading segment is a *declared* child
    // alias; otherwise the dots belong to a bare localization-key name.
    if let Some((head, tail)) = reference.split_once('.') {
        if child_aliases.iter().any(|a| a == head) {
            if tail.is_empty() {
                return QualifiedRef::Invalid(reference.to_string());
            }
            return QualifiedRef::Child {
                alias: head.to_string(),
                name: tail.to_string(),
            };
        }
    }

    QualifiedRef::Bare(reference.to_string())
}

/// Collect the set of entity reference ids declared by a world config: static
/// `[[entity]]` names plus names introduced by `SpawnEntity` trigger actions
/// (which register into `name_to_uuid` at runtime).
fn declared_names(config: &WorldConfig) -> Vec<String> {
    let mut names: Vec<String> = config
        .entities
        .iter()
        .filter_map(|e| e.name.clone())
        .collect();
    for trigger in &config.triggers {
        for action in &trigger.actions {
            if let TriggerAction::SpawnEntity { name, .. } = action {
                names.push(name.clone());
            }
        }
    }
    names
}

/// A single authored entity reference and a human-readable description of where
/// it came from (for finding messages).
struct EntityRef {
    reference: String,
    kind: &'static str,
}

/// Collect every authored entity-name reference in a world config.
///
/// Covers trigger conditions (destroy/attack/hail/region), objective targets,
/// and AI-state entity + target references. Comms *sender* identity is
/// deliberately out of scope here (issue #751 disambiguates whether a comms
/// `from` names an entity or is a plain display key); the objective *contract*
/// is #752. This only resolves references we can classify unambiguously today.
fn collect_entity_references(config: &WorldConfig) -> Vec<EntityRef> {
    let mut refs = Vec::new();
    let mut push = |reference: &str, kind: &'static str| {
        if !reference.is_empty() {
            refs.push(EntityRef {
                reference: reference.to_string(),
                kind,
            });
        }
    };

    for trigger in &config.triggers {
        match &trigger.condition {
            TriggerCondition::OnDestroyed { entity_name }
            | TriggerCondition::OnAttacked { entity_name }
            | TriggerCondition::OnHailed { entity_name }
            | TriggerCondition::OnEnteredRegion { entity_name }
            | TriggerCondition::OnExitedRegion { entity_name } => {
                push(entity_name, "trigger target")
            }
            _ => {}
        }
        for action in &trigger.actions {
            match action {
                TriggerAction::AddObjective { targets, .. } => {
                    for t in targets {
                        push(t, "objective target");
                    }
                }
                TriggerAction::SetAiState { entity, target, .. } => {
                    push(entity, "ai-state entity");
                    if let Some(t) = target {
                        push(t, "ai-state target");
                    }
                }
                TriggerAction::DestroyEntity { entity } => push(entity, "destroy target"),
                _ => {}
            }
        }
    }

    refs
}

/// Validate entity identity for one world: duplicate reference names in a single
/// namespace are errors (`world-entity-identity-state`).
pub fn validate_entity_identity(
    path: &str,
    source_text: &str,
    entities: &[WorldEntity],
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for entity in entities {
        if let Some(name) = entity.name.as_deref() {
            *seen.entry(name).or_insert(0) += 1;
        }
    }
    // Deterministic order: iterate entities, emit once per duplicate name.
    let mut reported: HashMap<&str, bool> = HashMap::new();
    for entity in entities {
        if let Some(name) = entity.name.as_deref() {
            if seen.get(name).copied().unwrap_or(0) > 1 && !reported.contains_key(name) {
                reported.insert(name, true);
                findings.push(WorldFinding::error(
                    "duplicate-name",
                    path,
                    source_text,
                    name,
                    format!(
                        "duplicate entity reference name '{name}' in '{path}'; \
                         reference names must be unique within a world"
                    ),
                ));
            }
        }
    }
    findings
}

/// Every action list authored in a world config, in a stable order: each
/// trigger's action list, then every comms dialogue node's response action
/// lists (recursing through follow-ups and the template root follow-up).
///
/// Objective declarations and references live in these lists whether they were
/// authored on a world trigger or a comms response, so the objective validator
/// walks all of them in one place.
fn collect_action_lists(config: &WorldConfig) -> Vec<&[TriggerAction]> {
    let mut lists: Vec<&[TriggerAction]> = Vec::new();
    for trigger in &config.triggers {
        lists.push(&trigger.actions);
    }
    for template in &config.comms {
        collect_comms_node_action_lists(&template.node, &mut lists);
        if let Some(root_fu) = &template.root_follow_up {
            collect_comms_node_action_lists(root_fu, &mut lists);
        }
    }
    lists
}

fn collect_comms_node_action_lists<'a>(
    node: &'a CommsDialogueNode,
    lists: &mut Vec<&'a [TriggerAction]>,
) {
    for response in &node.responses {
        lists.push(&response.actions);
        if let Some(follow_up) = &response.follow_up {
            collect_comms_node_action_lists(follow_up, lists);
        }
    }
}

/// Collect every objective id declared via `add_objective` across all of a
/// world config's action lists (triggers + comms).
fn collect_objective_declarations(config: &WorldConfig) -> HashSet<&str> {
    let mut declared = HashSet::new();
    for actions in collect_action_lists(config) {
        for action in actions {
            if let TriggerAction::AddObjective { id, .. } = action {
                declared.insert(id.as_str());
            }
        }
    }
    declared
}

/// Validate objective declarations and references for one world config against
/// the set of objective ids declared across the whole effective composition
/// (`objective-authoring-validation`, issue #752).
///
/// Two error rules, both deliberately **precise** so every shipped world keeps
/// validating clean:
///
/// * **Duplicate declaration** — the same objective id declared more than once
///   within a *single* action list (one trigger's actions, or one comms
///   response's actions). Both would run on a single fire, so the second is a
///   guaranteed dead no-op (`ObjectiveManager::add` silently ignores a repeated
///   id). Re-declaring an id across *separate*, mutually-exclusive branches —
///   e.g. `btf_path_a`'s two `obj-rescue-varen` arms, or an objective offered by
///   two alternative comms responses — is legitimate authoring and is NOT
///   flagged.
/// * **Unresolved reference** — a `complete_objective` / `fail_objective` id
///   that no `add_objective` anywhere in the composition declares. The
///   transition targets an objective that can never exist.
///
/// Objective *targets* are entity references, not objective ids; they are
/// resolved (as warnings for unresolved bare names) by `collect_entity_references`
/// and are deliberately not escalated to errors here — shipped worlds legitimately
/// target localization-key names and runtime-spawned entities.
pub fn validate_objectives_in(
    path: &str,
    source_text: &str,
    config: &WorldConfig,
    composition_declared: &HashSet<&str>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let action_lists = collect_action_lists(config);

    // Duplicate declaration within a single action list.
    for actions in &action_lists {
        let mut seen_here: HashSet<&str> = HashSet::new();
        for action in *actions {
            if let TriggerAction::AddObjective { id, .. } = action {
                if !seen_here.insert(id.as_str()) {
                    findings.push(WorldFinding::error(
                        "duplicate-objective-id",
                        path,
                        source_text,
                        id,
                        format!(
                            "objective id '{id}' is declared more than once in a single \
                             action list in '{path}'; the repeat is a dead no-op"
                        ),
                    ));
                }
            }
        }
    }

    // Unresolved complete/fail reference to an undeclared objective.
    for actions in &action_lists {
        for action in *actions {
            if let TriggerAction::CompleteObjective { id } | TriggerAction::FailObjective { id } =
                action
            {
                if !composition_declared.contains(id.as_str()) {
                    findings.push(WorldFinding::error(
                        "unresolved-objective-reference",
                        path,
                        source_text,
                        id,
                        format!(
                            "objective transition references id '{id}' which no \
                             add_objective declares, in '{path}'"
                        ),
                    ));
                }
            }
        }
    }

    findings
}

/// Convenience wrapper validating a single world config in isolation (the
/// Bevy `Startup` spawn gate): objective references resolve against that world's
/// own declarations only.
pub fn validate_objectives(
    path: &str,
    source_text: &str,
    config: &WorldConfig,
) -> Vec<WorldFinding> {
    let declared = collect_objective_declarations(config);
    validate_objectives_in(path, source_text, config, &declared)
}

/// Validate the effective composition (`world-authoring-validation-state`).
///
/// Runs per-world identity checks, then resolves every authored entity
/// reference across `root` + `children`:
///
/// * duplicate reference name in a namespace -> `duplicate-name` error
/// * `parent.<name>` climbing past the root, or resolving to a name absent in
///   the target layer -> `unresolved-reference` error
/// * `<alias>.<name>` with an unknown alias handled as bare; a known alias with
///   a name absent in that child -> `unresolved-reference` error
/// * malformed qualifier (`parent.`, trailing dot) -> `invalid-qualified-reference`
/// * a bare name present in more than one child namespace (and not in the
///   referencing world's own namespace) -> `ambiguous-reference` error
///
/// Bare references that resolve nowhere are **not** errored (they may name
/// runtime-spawned or engine-provided entities); they are reported as warnings
/// so shipped worlds keep activating while authors still get feedback.
pub fn validate_composition(root: &WorldSource, children: &[WorldSource]) -> Vec<WorldFinding> {
    let mut findings = Vec::new();

    // Per-world identity (duplicate names).
    findings.extend(validate_entity_identity(
        &root.path,
        root.toml,
        &root.config.entities,
    ));
    for child in children {
        findings.extend(validate_entity_identity(
            &child.path,
            child.toml,
            &child.config.entities,
        ));
    }

    // Namespaces, keyed by child alias for `<alias>.<name>` resolution.
    let root_names = declared_names(root.config);
    let child_aliases: Vec<String> = children.iter().map(|c| c.alias()).collect();
    let child_namespaces: HashMap<String, Vec<String>> = children
        .iter()
        .map(|c| (c.alias(), declared_names(c.config)))
        .collect();

    // The chain of ancestor namespaces a `parent.` climb walks: index 0 is the
    // referencing world itself, index 1 its parent, etc. In the additive
    // `extra_worlds`/root model every child's single parent is the root, so the
    // climb chain for a child is [child, root]; the root's is [root].
    let resolve_all = |ref_world_names: &[String], ancestors: &[&[String]], src: &WorldSource| {
        let mut out = Vec::new();
        for EntityRef { reference, kind } in collect_entity_references(src.config) {
            match parse_qualified_reference(&reference, &child_aliases) {
                QualifiedRef::Invalid(r) => out.push(WorldFinding::error(
                    "invalid-qualified-reference",
                    &src.path,
                    src.toml,
                    &r,
                    format!(
                        "invalid qualified reference '{r}' ({kind}) in '{}'",
                        src.path
                    ),
                )),
                QualifiedRef::Parent { depth, name } => match ancestors.get(depth) {
                    None => out.push(WorldFinding::error(
                        "unresolved-reference",
                        &src.path,
                        src.toml,
                        &reference,
                        format!(
                            "qualified reference '{reference}' ({kind}) climbs past the \
                                     root world in '{}'",
                            src.path
                        ),
                    )),
                    Some(layer_names) => {
                        if !layer_names.iter().any(|n| n == &name) {
                            out.push(WorldFinding::error(
                                "unresolved-reference",
                                &src.path,
                                src.toml,
                                &reference,
                                format!(
                                    "qualified reference '{reference}' ({kind}) resolves \
                                             to no entity in the target layer, from '{}'",
                                    src.path
                                ),
                            ));
                        }
                    }
                },
                QualifiedRef::Child { alias, name } => {
                    let ok = child_namespaces
                        .get(&alias)
                        .map(|ns| ns.iter().any(|n| n == &name))
                        .unwrap_or(false);
                    if !ok {
                        out.push(WorldFinding::error(
                            "unresolved-reference",
                            &src.path,
                            src.toml,
                            &reference,
                            format!(
                                "qualified reference '{reference}' ({kind}) names no entity \
                                     in child world '{alias}', from '{}'",
                                src.path
                            ),
                        ));
                    }
                }
                QualifiedRef::Bare(name) => {
                    if ref_world_names.iter().any(|n| n == &name) {
                        continue; // resolves in own namespace
                    }
                    // Ambiguous if declared in more than one *other* namespace.
                    let matching: Vec<&String> = child_namespaces
                        .iter()
                        .filter(|(_, ns)| ns.iter().any(|n| n == &name))
                        .map(|(alias, _)| alias)
                        .collect();
                    if matching.len() > 1 {
                        out.push(WorldFinding::error(
                            "ambiguous-reference",
                            &src.path,
                            src.toml,
                            &name,
                            format!(
                                "reference '{name}' ({kind}) is ambiguous: declared in \
                                     child worlds {matching:?}; qualify it as <child>.{name}"
                            ),
                        ));
                    } else if matching.is_empty() {
                        // Resolves nowhere — non-blocking warning.
                        out.push(WorldFinding {
                            severity: Severity::Warning,
                            category: "unresolved-reference",
                            message: format!(
                                "reference '{name}' ({kind}) resolves to no declared entity \
                                     in '{}'",
                                src.path
                            ),
                            source: SourceLocation {
                                file: src.path.clone(),
                                line: line_of(src.toml, &name),
                                reference: name.clone(),
                            },
                        });
                    }
                    // Exactly one other namespace: resolvable, no finding.
                }
            }
        }
        out
    };

    // Objective declarations/references (issue #752). Resolve `complete`/`fail`
    // references against the union of objectives declared anywhere in the
    // effective composition, since they share a single `ObjectiveManager`.
    let mut composition_declared = collect_objective_declarations(root.config);
    for child in children {
        composition_declared.extend(collect_objective_declarations(child.config));
    }
    findings.extend(validate_objectives_in(
        &root.path,
        root.toml,
        root.config,
        &composition_declared,
    ));
    for child in children {
        findings.extend(validate_objectives_in(
            &child.path,
            child.toml,
            child.config,
            &composition_declared,
        ));
    }

    // Root references: ancestor chain is just [root].
    let root_ancestors: Vec<&[String]> = vec![root_names.as_slice()];
    findings.extend(resolve_all(&root_names, &root_ancestors, root));

    // Child references: ancestor chain is [child, root].
    for child in children {
        let names = declared_names(child.config);
        let ancestors: Vec<&[String]> = vec![names.as_slice(), root_names.as_slice()];
        findings.extend(resolve_all(&names, &ancestors, child));
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::parse_world;

    fn cfg(toml: &str) -> WorldConfig {
        parse_world(toml).expect("fixture parses")
    }

    // ── name vs display_name roles (AC1) ─────────────────────────────────────

    #[test]
    fn name_is_reference_display_name_is_player_text() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/station.toml"
name = "axiom_station"
display_name = "Axiom Station"
"#;
        let c = cfg(toml);
        // Reference id used by name_to_uuid / composition:
        assert_eq!(c.entities[0].name.as_deref(), Some("axiom_station"));
        // Player-facing text, independent:
        assert_eq!(c.entities[0].display_text(), "Axiom Station");
    }

    #[test]
    fn display_text_falls_back_to_name_when_absent() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/station.toml"
name = "world.entity.axiom_station.name"
"#;
        let c = cfg(toml);
        assert_eq!(c.entities[0].display_name, None);
        // Falls back to the name reference id (a localization key) — the
        // pre-#750 conflated behaviour is preserved for existing worlds.
        assert_eq!(
            c.entities[0].display_text(),
            "world.entity.axiom_station.name"
        );
    }

    // ── duplicate reference name (AC2, source-located) ───────────────────────

    #[test]
    fn duplicate_reference_name_is_source_located_error() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/a.toml"
name = "outpost"

[[entity]]
template_path = "assets/entities/b.toml"
name = "outpost"
"#;
        let c = cfg(toml);
        let findings = validate_entity_identity("root.toml", toml, &c.entities);
        assert_eq!(findings.len(), 1, "one finding per duplicated name");
        let f = &findings[0];
        assert!(f.is_error());
        assert_eq!(f.category, "duplicate-name");
        assert_eq!(f.source.file, "root.toml");
        assert_eq!(f.source.reference, "outpost");
        // Best-effort line points at the *first* occurrence of the name.
        assert_eq!(f.source.line, Some(4));
    }

    // ── invalid qualified reference (AC2) ────────────────────────────────────

    #[test]
    fn malformed_parent_qualifier_is_invalid() {
        assert_eq!(
            parse_qualified_reference("parent.", &[]),
            QualifiedRef::Invalid("parent.".to_string())
        );
        assert_eq!(
            parse_qualified_reference("parent.parent.", &[]),
            QualifiedRef::Invalid("parent.parent.".to_string())
        );
    }

    #[test]
    fn parent_and_child_qualifiers_parse() {
        assert_eq!(
            parse_qualified_reference("parent.axiom", &[]),
            QualifiedRef::Parent {
                depth: 1,
                name: "axiom".to_string()
            }
        );
        assert_eq!(
            parse_qualified_reference("parent.parent.axiom", &[]),
            QualifiedRef::Parent {
                depth: 2,
                name: "axiom".to_string()
            }
        );
        let aliases = vec!["btf_path_a".to_string()];
        assert_eq!(
            parse_qualified_reference("btf_path_a.ironveil", &aliases),
            QualifiedRef::Child {
                alias: "btf_path_a".to_string(),
                name: "ironveil".to_string()
            }
        );
        // Localization-key names keep their dots — not a child qualifier.
        assert_eq!(
            parse_qualified_reference("world.entity.axiom.name", &aliases),
            QualifiedRef::Bare("world.entity.axiom.name".to_string())
        );
    }

    // ── parent.<name> resolution (AC3) ───────────────────────────────────────

    #[test]
    fn parent_reference_resolves_to_root_entity() {
        let root = cfg(r#"
[[entity]]
template_path = "assets/entities/a.toml"
name = "axiom"
"#);
        // Child references parent.axiom in a trigger target — resolves to root.
        let child = cfg(r#"
[[entity]]
template_path = "assets/entities/b.toml"
name = "child_ship"

[[trigger]]
condition = "on_destroyed"
entity = "parent.axiom"
"#);
        let root_src = WorldSource::new("root.toml", "", &root);
        let child_toml = "entity = \"parent.axiom\"";
        let child_src = WorldSource::new("assets/worlds/child.toml", child_toml, &child);
        let findings = validate_composition(&root_src, &[child_src]);
        assert!(
            !has_error(&findings),
            "parent.axiom resolves to root: {findings:?}"
        );
    }

    #[test]
    fn parent_reference_past_root_is_unresolved_error() {
        let root = cfg("");
        let child = cfg(r#"
[[trigger]]
condition = "on_destroyed"
entity = "parent.parent.axiom"
"#);
        let root_src = WorldSource::new("root.toml", "", &root);
        let child_toml = "entity = \"parent.parent.axiom\"";
        let child_src = WorldSource::new("child.toml", child_toml, &child);
        let findings = validate_composition(&root_src, &[child_src]);
        let err = findings
            .iter()
            .find(|f| f.is_error())
            .expect("climbing past root errors");
        assert_eq!(err.category, "unresolved-reference");
        assert_eq!(err.source.reference, "parent.parent.axiom");
        assert_eq!(err.source.line, Some(1));
    }

    // ── child_world.<name> resolution (AC3) ──────────────────────────────────

    #[test]
    fn child_world_reference_resolves_and_detects_unknown() {
        let root = cfg(r#"
[[entity]]
template_path = "assets/entities/a.toml"
name = "flagship"

[[trigger]]
condition = "on_destroyed"
entity = "child_a.ironveil"

[[trigger]]
condition = "on_destroyed"
entity = "child_a.ghost"
"#);
        let child = cfg(r#"
[[entity]]
template_path = "assets/entities/b.toml"
name = "ironveil"
"#);
        let root_toml = "entity = \"child_a.ironveil\"\nentity = \"child_a.ghost\"".to_string();
        let root_src = WorldSource::new("assets/worlds/root.toml", &root_toml, &root);
        let child_src = WorldSource::new("assets/worlds/child_a.toml", "", &child);
        let findings = validate_composition(&root_src, &[child_src]);
        // ironveil resolves; ghost does not.
        let errs: Vec<_> = findings.iter().filter(|f| f.is_error()).collect();
        assert_eq!(
            errs.len(),
            1,
            "only child_a.ghost is unresolved: {findings:?}"
        );
        assert_eq!(errs[0].source.reference, "child_a.ghost");
        assert_eq!(errs[0].category, "unresolved-reference");
    }

    // ── ambiguous reference (AC2) ────────────────────────────────────────────

    #[test]
    fn bare_reference_in_two_children_is_ambiguous() {
        let root = cfg(r#"
[[trigger]]
condition = "on_destroyed"
entity = "raider"
"#);
        let a = cfg(r#"
[[entity]]
template_path = "assets/entities/x.toml"
name = "raider"
"#);
        let b = cfg(r#"
[[entity]]
template_path = "assets/entities/y.toml"
name = "raider"
"#);
        let root_src = WorldSource::new("root.toml", "entity = \"raider\"", &root);
        let a_src = WorldSource::new("child_a.toml", "", &a);
        let b_src = WorldSource::new("child_b.toml", "", &b);
        let findings = validate_composition(&root_src, &[a_src, b_src]);
        let err = findings
            .iter()
            .find(|f| f.category == "ambiguous-reference")
            .expect("raider is ambiguous across children");
        assert!(err.is_error());
        assert_eq!(err.source.reference, "raider");
    }

    // ── unresolved bare reference is a non-blocking warning ──────────────────

    #[test]
    fn unresolved_bare_reference_is_warning_not_error() {
        let root = cfg(r#"
[[trigger]]
condition = "on_destroyed"
entity = "phantom"
"#);
        let root_src = WorldSource::new("root.toml", "entity = \"phantom\"", &root);
        let findings = validate_composition(&root_src, &[]);
        assert!(!has_error(&findings), "bare unresolved must not block");
        assert!(findings
            .iter()
            .any(|f| f.severity == Severity::Warning && f.source.reference == "phantom"));
    }

    // ── objective authoring validation (AC2, issue #752) ─────────────────────

    #[test]
    fn duplicate_objective_id_in_one_action_list_is_error() {
        let toml = r#"
[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-dup"
  text = "First"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-dup"
  text = "Second"
"#;
        let c = cfg(toml);
        let findings = validate_objectives("root.toml", toml, &c);
        let err = findings
            .iter()
            .find(|f| f.category == "duplicate-objective-id")
            .expect("duplicate id in one action list must error");
        assert!(err.is_error());
        assert_eq!(err.source.reference, "obj-dup");
    }

    #[test]
    fn same_objective_id_across_separate_triggers_is_allowed() {
        // Mutually-exclusive branches re-declaring the same id (the shipped
        // `btf_path_a` pattern) must NOT be flagged.
        let toml = r#"
[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-rescue"
  text = "Case A"

[[trigger]]
condition = "on_flag_set"
name      = "ready"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-rescue"
  text = "Case B"
"#;
        let c = cfg(toml);
        let findings = validate_objectives("root.toml", toml, &c);
        assert!(
            !has_error(&findings),
            "cross-branch id reuse must not error: {findings:?}"
        );
    }

    #[test]
    fn complete_objective_referencing_undeclared_id_is_error() {
        let toml = r#"
[[trigger]]
condition = "on_destroyed"
entity    = "raider"

  [[trigger.action]]
  type = "complete_objective"
  id   = "obj-ghost"
"#;
        let c = cfg(toml);
        let findings = validate_objectives("root.toml", toml, &c);
        let err = findings
            .iter()
            .find(|f| f.category == "unresolved-objective-reference")
            .expect("complete of an undeclared objective must error");
        assert!(err.is_error());
        assert_eq!(err.source.reference, "obj-ghost");
    }

    #[test]
    fn complete_objective_referencing_declared_id_resolves() {
        let toml = r#"
[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-1"
  text = "Do it"

[[trigger]]
condition = "on_destroyed"
entity    = "raider"

  [[trigger.action]]
  type = "complete_objective"
  id   = "obj-1"
"#;
        let c = cfg(toml);
        let findings = validate_objectives("root.toml", toml, &c);
        assert!(
            !has_error(&findings),
            "complete of a declared objective must resolve: {findings:?}"
        );
    }

    #[test]
    fn objective_declared_in_root_resolves_a_child_reference() {
        // Composition-wide resolution: a child's complete_objective resolves
        // against an objective the root declares (one shared ObjectiveManager).
        let root = cfg(r#"
[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-shared"
  text = "Shared"
"#);
        let child = cfg(r#"
[[trigger]]
condition = "on_destroyed"
entity    = "raider"

  [[trigger.action]]
  type = "complete_objective"
  id   = "obj-shared"
"#);
        let root_src = WorldSource::new("root.toml", "", &root);
        let child_src = WorldSource::new("assets/worlds/child.toml", "", &child);
        let findings = validate_composition(&root_src, &[child_src]);
        assert!(
            !has_error(&findings),
            "child complete resolves against root declaration: {findings:?}"
        );
    }

    #[test]
    fn duplicate_objective_id_blocks_composition_activation() {
        let toml = r#"
[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "dup"
  text = "One"

  [[trigger.action]]
  type = "add_objective"
  id   = "dup"
  text = "Two"
"#;
        let c = cfg(toml);
        let src = WorldSource::new("root.toml", toml, &c);
        let findings = validate_composition(&src, &[]);
        assert!(
            has_error(&findings),
            "a duplicate objective declaration must block activation"
        );
    }

    // ── shipped worlds validate clean (regression guard) ─────────────────────

    #[test]
    fn shipped_root_worlds_have_no_composition_errors() {
        // Iterate every shipped world so the atomic-activation gate can never
        // silently start rejecting real content as the catalog grows.
        let mut checked = 0;
        for entry in std::fs::read_dir("assets/worlds").expect("worlds dir readable") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let path_str = path.to_string_lossy().replace('\\', "/");
            let toml = std::fs::read_to_string(&path).expect("shipped world readable");
            let config = parse_world(&toml).expect("shipped world parses");
            let src = WorldSource::new(&path_str, &toml, &config);
            let findings = validate_composition(&src, &[]);
            let errors: Vec<_> = findings.iter().filter(|f| f.is_error()).collect();
            assert!(
                errors.is_empty(),
                "shipped world {path_str} must not error: {errors:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected at least one shipped world to validate"
        );
    }
}
