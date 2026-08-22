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

use crate::entities::include_resolve::FragmentSource;
use crate::entities::loader::TemplateLoader;
use crate::world::config::{TriggerAction, WorldConfig, WorldEntity};

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
    /// `ambiguous-reference`, `invalid-qualified-reference`,
    /// `ambiguous-entity-reference`, `unresolved-relative-to`.
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

    fn warning(
        category: &'static str,
        file: &str,
        source_text: &str,
        reference: &str,
        message: String,
    ) -> Self {
        WorldFinding {
            severity: Severity::Warning,
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

/// Collect the set of entity reference ids declared by a world config: the
/// static `[[entity]]` names.
///
/// It also walked `SpawnEntity` trigger actions (which register into
/// `name_to_uuid` at runtime) until issue #985 deleted the `[[trigger]]`
/// front-end that authored them. A SCRIPTED `spawn_entity` names its entity in
/// a Rhai map this pass cannot read; `world::script::validate` is where a
/// scripted world's cross-references are checked.
fn declared_names(config: &WorldConfig) -> Vec<String> {
    config
        .entities
        .iter()
        .filter_map(|e| e.name.clone())
        .collect()
}

/// A single authored entity reference and a human-readable description of where
/// it came from (for finding messages).
///
/// Nothing constructs one since issue #985 — see [`collect_entity_references`]
/// for why the source went and why the shape is kept rather than unpicked.
#[allow(dead_code)]
struct EntityRef {
    reference: String,
    kind: &'static str,
}

/// Collect every authored entity-name reference in a world config.
///
/// It covered trigger conditions (destroy/attack/hail/region), objective
/// targets, and AI-state entity + target references — all of them reached
/// through `config.triggers`, which issue #985 deleted with the `[[trigger]]`
/// front-end. Nothing declarative is left to walk, so this returns empty and the
/// checks built on it are vacuous for every world; a scripted world's
/// cross-references are resolved by `world::script::validate` instead.
///
/// Kept rather than unpicked because the CHECKS it feeds — unresolved-reference
/// findings with their severities and messages — are the seam script-in-layers
/// (#1045) and any future authored-reference source plug into.
fn collect_entity_references(_config: &WorldConfig) -> Vec<EntityRef> {
    Vec::new()
}

/// Validate entity identity for one world: duplicate reference names in a single
/// namespace are errors (`world-entity-identity-state`).
///
/// A second, non-blocking pass covers the *positioning* namespace (issue #969).
/// A `relative_to` resolves against both authored identifiers — `id` as well as
/// `name` — so any spelling claimed by more than one `[[entity]]` is ambiguous
/// as a positioning reference. Two shapes reach it:
///
/// * one entity's `id` equal to another entity's `name`. Resolution is defined
///   (`name` wins, see [`crate::world::config::build_named_entity_positions`]),
///   but the author almost certainly meant one specific entity, and an `id`
///   added later can quietly take a reference off a `name` it used to miss.
/// * two entities sharing an `id`. Nothing decides between them but file order.
///
/// Warning, not error: neither shape is wrong until something references it,
/// `id` has never been required to be unique, and a mod pack may already ship
/// one. The duplicate **`name`** case stays an error — it breaks the
/// trigger/comms/objective namespace outright — and is reported there only, so
/// one collision never produces two findings.
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

    // How many entities claim each spelling through EITHER identifier. An
    // entity whose `id` and `name` are the same string claims it once — that is
    // one entity, and nothing about it is ambiguous.
    fn claimed_by(entity: &WorldEntity) -> Vec<&str> {
        let mut claimed: Vec<&str> = [entity.id.as_deref(), entity.name.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        claimed.dedup();
        claimed
    }
    let mut claimants: HashMap<&str, usize> = HashMap::new();
    for entity in entities {
        for spelling in claimed_by(entity) {
            *claimants.entry(spelling).or_insert(0) += 1;
        }
    }
    let mut warned: HashSet<&str> = HashSet::new();
    for entity in entities {
        for spelling in claimed_by(entity) {
            if claimants.get(spelling).copied().unwrap_or(0) < 2 {
                continue;
            }
            // Already an error in the reference namespace; don't say it twice.
            if seen.get(spelling).copied().unwrap_or(0) > 1 {
                continue;
            }
            if !warned.insert(spelling) {
                continue;
            }
            findings.push(WorldFinding::warning(
                "ambiguous-entity-reference",
                path,
                source_text,
                spelling,
                format!(
                    "'{spelling}' is claimed as the `id` or `name` of more than one \
                     [[entity]] in '{path}'; a `relative_to = \"{spelling}\"` resolves \
                     against exactly one of them (a `name` beats another entity's `id`, \
                     and otherwise the last declaration wins)"
                ),
            ));
        }
    }

    findings
}

/// Every action list authored in a world config, in a stable order.
///
/// It walked each `[[trigger]]`'s action array and every comms dialogue node's
/// response action lists; issue #985 deleted both front-ends, so there is
/// nothing authored left to walk and the objective/reference checks built on it
/// are vacuous for every world. Kept for [`collect_entity_references`]' reason:
/// the CHECKS are the seam a future authored-action source plugs into.
fn collect_action_lists(_config: &WorldConfig) -> Vec<&[TriggerAction]> {
    Vec::new()
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
///
/// # Vacuous since issue #985
///
/// Both rules read [`collect_action_lists`], whose only source was the
/// `[[trigger.action]]` and `[[comms.response.action]]` arrays. With those
/// parsers deleted it returns empty, so this finds nothing for any world — an
/// objective is declared by a script calling `ctx.effects.add_objective`, in a
/// Rhai map this pass cannot read. It is kept rather than unpicked for
/// [`collect_entity_references`]' reason: the RULES, their severities and their
/// messages are the seam a future authored-action source plugs into, and
/// `world::script::validate` is where a scripted world's cross-references are
/// checked today.
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

// ── Doctrine anchor references (issue #888) ──────────────────────────────────

/// One entity instance a world spawns: the label to name it by in a finding,
/// the template it is built from, and any inline `overrides` merged on top.
///
/// Covers both spawn paths — a static `[[entity]]` block and a `spawn_entity`
/// trigger/comms action — because both hand the same hull the same doctrine.
struct SpawnedInstance<'a> {
    /// Best-effort identity for diagnostics: the authored `name`, else the
    /// authored `id`, else the template path.
    label: String,
    template_path: &'a str,
    overrides: Option<&'a toml::Value>,
}

/// Every entity instance a world config spawns, static blocks first, then the
/// `spawn_entity` actions across every authored action list (triggers *and*
/// comms responses — `collect_action_lists` walks both).
fn collect_spawned_instances(config: &WorldConfig) -> Vec<SpawnedInstance<'_>> {
    let mut out: Vec<SpawnedInstance<'_>> = config
        .entities
        .iter()
        .map(|e| SpawnedInstance {
            label: e
                .name
                .clone()
                .or_else(|| e.id.clone())
                .unwrap_or_else(|| e.template_path.clone()),
            template_path: &e.template_path,
            overrides: e.overrides.as_ref(),
        })
        .collect();

    for actions in collect_action_lists(config) {
        for action in actions {
            if let TriggerAction::SpawnEntity {
                template_path,
                name,
                overrides,
                ..
            } = action
            {
                out.push(SpawnedInstance {
                    label: name.clone(),
                    template_path,
                    overrides: overrides.as_ref(),
                });
            }
        }
    }

    out
}

/// The anchor names one spawned instance's *effective* doctrine references,
/// paired with the directive kind that reads them.
///
/// Two deliberate choices:
///
/// * The doctrine read is the **effective** one — template plus any authored
///   `overrides` — because a scenario may add, retarget or clear doctrine
///   entries (`probe_artillery_standoff.toml` adds one by override). Judging
///   the raw template would validate content no scenario runs.
/// * Which fields count as anchors is asked of
///   [`crate::ai::core::parse_doctrine_directive`], the same function the
///   runtime flies, rather than re-derived from the `directive_*` field names.
///   A third copy of that table is how the courier's `directive_anchors`-on-a-
///   `Reach` survived in the first place.
///
/// A template that cannot be loaded or whose merge fails yields nothing: those
/// are *different* defects, and this validator must not turn either into a
/// spurious anchor complaint — nor block a world whose templates simply are not
/// reachable from wherever validation happens to run.
///
/// Since issue #973 both have an owner in the code rather than only in this
/// comment: [`validate_template_resolution_in`] reports them, as
/// `unresolvable-template` (on hosts whose loader can be authoritative about
/// absence) and `unmergeable-override` (on every host). So the `Err(_)` arm
/// below is now genuinely "somebody else's finding", not a swallowed failure.
/// This function's silence is unchanged — it is about anchors.
fn doctrine_anchor_refs(
    inst: &SpawnedInstance,
    loader: &dyn TemplateLoader,
) -> Vec<(String, &'static str)> {
    let Some(template) = loader.load_template(inst.template_path) else {
        return Vec::new();
    };
    let config = match inst.overrides {
        None => template,
        Some(overrides) => match crate::entities::loader::apply_overrides(&template, overrides) {
            Ok(merged) => merged,
            Err(_) => return Vec::new(),
        },
    };
    let Some(behaviour) = config.behaviour.as_ref() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in &behaviour.doctrine {
        match crate::ai::core::parse_doctrine_directive(entry) {
            crate::core::messages::AiDirective::Patrol { anchors, .. } => {
                out.extend(anchors.into_iter().map(|a| (a, "Patrol")));
            }
            crate::core::messages::AiDirective::Reach { anchor } => out.push((anchor, "Reach")),
            crate::core::messages::AiDirective::Retreat { anchor } => out.push((anchor, "Retreat")),
            _ => {}
        }
    }
    // An empty anchor name is the *field-name* defect, already rejected at
    // template load by `validate_doctrine_directives`; nothing to add here.
    out.retain(|(anchor, _)| !anchor.is_empty());
    out
}

/// The doctrine anchors ONE template references, for a host that fields a hull
/// the config walkers above cannot see (issue #984 review).
///
/// [`collect_spawned_instances`] only knows the *declarative* spawn sites —
/// `[[entity]]` blocks and `spawn_entity` trigger/comms actions. A hull spawned
/// from script, or dropped into a script-authored slot by the headless duel
/// harness (`--side-a`/`--side-b`), never appears there, so
/// [`validate_doctrine_anchors_in`] cannot judge it. Rather than let those hulls
/// escape the #888 guard entirely, the harness runs this against each hull it
/// fields and rejects an undeclared anchor itself — through the same
/// [`doctrine_anchor_refs`] table the load-time validator uses, so the two can
/// never disagree about which fields are anchors.
///
/// The template's OWN doctrine is judged, with no overrides applied: a slot
/// override adds entries (`merge_id_array` keeps every template entry the
/// override does not name by id), so the route the template arrives carrying is
/// present in the effective doctrine either way.
///
/// Returns `(anchor, directive-kind)` pairs, empty when the template cannot be
/// loaded — the same silence, for the same reason, as [`doctrine_anchor_refs`].
pub fn template_doctrine_anchors(
    template_path: &str,
    loader: &dyn TemplateLoader,
) -> Vec<(String, &'static str)> {
    doctrine_anchor_refs(
        &SpawnedInstance {
            label: String::new(),
            template_path,
            overrides: None,
        },
        loader,
    )
}

/// Reject a doctrine anchor that no world in the composition declares
/// (issue #888).
///
/// # Why this is an error and not a warning
///
/// Bare *entity* references that resolve nowhere are warnings (see
/// [`validate_composition`]) because the name may belong to an entity created
/// after load — `spawn_entity` registers fresh names into `name_to_uuid` at
/// runtime, so a linter cannot tell a typo from a forward reference.
///
/// **Anchors have no such runtime source.** The anchor table is parsed once
/// from `[anchors]` into `WorldConfig::anchors` and is never written again:
/// no system anywhere takes the config as `ResMut`, and no trigger action
/// declares an anchor. A doctrine anchor that misses at load misses on every
/// tick forever, so there is nothing for a warning to be tentative about — the
/// ship silently never pursues its goal, which is the same reads-as-nothing
/// failure the unvalidated `fact(...)` names keep producing. It fails the load.
///
/// # Resolution scope
///
/// Against the union of the anchor tables of the root world **and its layer
/// chain**, so a sub-world that spawns a hull steering to one of the base
/// world's anchors (the `btf_path_*` shape) does not false-positive. That union
/// is deliberately the *permissive* bound: doctrine anchors are looked up at
/// runtime in the base `WorldConfig` alone, so an anchor declared only by a
/// child would still miss. No shipped child world declares any anchor, so the
/// distinction is theoretical today; the union is chosen because a validator
/// that blocks activation must never be stricter than the composition contract
/// it validates.
fn validate_doctrine_anchors_in(
    src: &WorldSource,
    declared_anchors: &HashSet<&str>,
    loader: &dyn TemplateLoader,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let mut reported: HashSet<(String, String)> = HashSet::new();

    for inst in collect_spawned_instances(src.config) {
        for (anchor, kind) in doctrine_anchor_refs(&inst, loader) {
            if declared_anchors.contains(anchor.as_str()) {
                continue;
            }
            if !reported.insert((inst.label.clone(), anchor.clone())) {
                continue;
            }
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: "unresolved-anchor",
                message: format!(
                    "entity '{}' (template '{}') has a {kind} doctrine directive referencing \
                     anchor '{anchor}', which no world in the composition declares, in '{}'",
                    inst.label, inst.template_path, src.path
                ),
                source: SourceLocation {
                    file: src.path.clone(),
                    // The anchor name is absent from this world by definition,
                    // so point at the spawn site instead.
                    line: line_of(src.toml, &inst.label)
                        .or_else(|| line_of(src.toml, inst.template_path)),
                    reference: anchor.clone(),
                },
            });
        }
    }

    findings
}

/// The `[[route]]` id one spawned instance's `[civilian]` table names.
///
/// Same shape, and the same silences, as [`doctrine_anchor_refs`]: a template
/// that cannot be loaded or whose override merge fails yields nothing, because
/// both are *different* defects with their own findings.
fn civilian_route_ref(inst: &SpawnedInstance, loader: &dyn TemplateLoader) -> Option<String> {
    let template = loader.load_template(inst.template_path)?;
    let config = match inst.overrides {
        None => template,
        Some(overrides) => crate::entities::loader::apply_overrides(&template, overrides).ok()?,
    };
    config
        .civilian
        .as_ref()
        .and_then(|c| c.route.clone())
        .filter(|id| !id.is_empty())
}

/// Reject a `[[route]]` leg naming an anchor no world in the composition
/// declares (issue #1028).
///
/// The same argument as [`validate_doctrine_anchors_in`], which this sits
/// beside and shares its resolution scope with: the anchor table is parsed once
/// and never written again, so a leg that misses at load misses on every tick
/// forever and the hauler flying that lane silently skips it. There is nothing
/// for a warning to be tentative about.
fn validate_route_anchors_in(
    src: &WorldSource,
    declared_anchors: &HashSet<&str>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for route in &src.config.routes {
        for leg in &route.legs {
            if declared_anchors.contains(leg.anchor.as_str()) {
                continue;
            }
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: "unresolved-anchor",
                message: format!(
                    "civilian route '{}' has a leg referencing anchor '{}', which no \
                     world in the composition declares, in '{}'",
                    route.id, leg.anchor, src.path
                ),
                source: SourceLocation {
                    file: src.path.clone(),
                    line: line_of(src.toml, &leg.anchor).or_else(|| line_of(src.toml, &route.id)),
                    reference: leg.anchor.clone(),
                },
            });
        }
    }
    findings
}

/// Reject an entity whose `[civilian] route` names a lane no world in the
/// composition declares (issue #1028).
///
/// Routes are world data and civilians reference them by id, so this is the
/// same class of dangling reference as an unresolved anchor and gets the same
/// treatment. A civilian pointed at a lane that does not exist installs no
/// directive at all and holds station for the whole mission — visibly *there*,
/// and doing nothing, with no diagnostic anywhere.
fn validate_civilian_routes_in(
    src: &WorldSource,
    declared_routes: &HashSet<&str>,
    loader: &dyn TemplateLoader,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let mut reported: HashSet<(String, String)> = HashSet::new();
    for inst in collect_spawned_instances(src.config) {
        let Some(route) = civilian_route_ref(&inst, loader) else {
            continue;
        };
        if declared_routes.contains(route.as_str()) {
            continue;
        }
        if !reported.insert((inst.label.clone(), route.clone())) {
            continue;
        }
        findings.push(WorldFinding {
            severity: Severity::Error,
            category: "unresolved-route",
            message: format!(
                "entity '{}' (template '{}') is assigned civilian route '{route}', which \
                 no world in the composition declares, in '{}'",
                inst.label, inst.template_path, src.path
            ),
            source: SourceLocation {
                file: src.path.clone(),
                line: line_of(src.toml, &inst.label)
                    .or_else(|| line_of(src.toml, inst.template_path)),
                reference: route,
            },
        });
    }
    findings
}

/// Reject an entity template whose include closure cannot be composed
/// (issue #906).
///
/// # Why this joins the world-finding flow at all
///
/// The `includes` resolver (issue #869) has been on every production load path
/// since the commit that added it, but each host handled a composition failure
/// *privately*: `world::server::build_layer_config_cache` warned and skipped,
/// `entity_loader::FsTemplateLoader` returned `None`, the browser preload logged
/// to the JS console. The net effect was that a world with a broken include
/// silently lost an entity — nothing reached the editor's validation badge, and
/// nothing reached the atomic-activation gate ([`has_error`]). Composition is a
/// content error like any other, so it is reported like any other.
///
/// # Why it is an ERROR and never a warning
///
/// A template that declares `includes` has said it is incomplete on its own.
/// Spawning it from the fragments that happened to resolve would put a
/// half-assembled hull in the world, which is the one outcome composition must
/// never produce. [`crate::entities::include_resolve::IncludeError`] has no warning
/// severity for the same reason.
///
/// The finding's `source` is the resolver's own: the file that *declared* the
/// bad include, with a best-effort line, which is the file the author has to
/// edit. The world and entity that pulled it in are named in the message
/// instead, since one broken fragment can be reached from many worlds.
fn validate_template_composition_in(
    src: &WorldSource,
    fragments: &dyn FragmentSource,
    seen: &mut HashSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for inst in collect_spawned_instances(src.config) {
        // Key on the CANONICAL path, matching what `composition_finding` does
        // internally. Authored spellings of one template vary freely —
        // `./assets/x.toml`, `assets/./x.toml`, backslashes on Windows — and
        // keying on the raw string would let two spellings of one hull produce
        // two findings for one broken include, which is exactly the duplication
        // this set exists to prevent.
        if !seen.insert(crate::entities::include_resolve::canonical_template_path(
            inst.template_path,
        )) {
            continue;
        }
        let Some(mut finding) =
            crate::entities::include_resolve::composition_finding(inst.template_path, fragments)
        else {
            continue;
        };
        finding.message = format!(
            "entity '{}' (template '{}') could not be composed, from '{}': {}",
            inst.label, inst.template_path, src.path, finding.message
        );
        findings.push(finding);
    }
    findings
}

/// Reject an `[[entity]]` (or `spawn_entity` action) that
/// [`crate::entities::loader::resolve_entity_via`] would return `Err` for — the
/// template does not resolve, or its `overrides` do not merge (issue #973).
///
/// # Why this is an ERROR and never a warning
///
/// `resolve_entity_via` returns `Err` for both, and **every spawn caller logs
/// and `continue`s** — `world::server::spawn_immediate_entities_internal`
/// twice, `setup_world` and `spawn_game_start_entities` in `server_app` once
/// each. So a world naming a template that does not resolve loads
/// "successfully" and is simply missing entities. That is how #954's hull
/// relocation surfaced: the scenario ran, spawned no hostiles, and the only
/// signal was a log line the determinism guard happened to catch. An author who
/// wrote a `template_path` asked for that entity; a world without it is a
/// different world, not a degraded one.
///
/// # The two failure shapes, and why both live here
///
/// This validator's contract is "everything `resolve_entity_via` can refuse",
/// not "the lookup". The lookup was the only half #973 first covered, which
/// left the override half producing the very silent drop the issue condemns:
/// an `overrides` table that fails the strict re-parse of the merged document
/// (`entity_loader::apply_overrides`) or carries a `_remove` tombstone
/// (issue #911, rejected outright by `reject_unhonoured_removals`) still cost
/// exactly one entity, with the rest of the world spawning around the hole.
///
/// 1. **`unresolvable-template`** — the loader cannot serve the template.
///    `missing` and `malformed` are deliberately one finding, because
///    `load_template` collapses them and because the *world* consequence is
///    identical. That is a narrower policy than the preload's warn-and-skip for
///    an unparseable template, and deliberately so: the preload walks a whole
///    directory, where one bad cosmetic asteroid must not stop a combat test,
///    whereas this walks only templates a world explicitly named.
/// 2. **`unmergeable-override`** — the template resolved, but this instance's
///    `overrides` do not apply to it.
///
/// # What the merge finding costs a TRIGGER spawn, stated exactly
///
/// The two shapes reach the two spawn origins differently, and conflating them
/// would be the kind of "this is covered" claim that stops the next reader
/// looking. For a static `[[entity]]`, both shapes are the same silent drop:
/// `resolve_entity_via` returns `Err` and the caller logs and `continue`s.
/// For a `spawn_entity` **trigger action**, only the lookup drops the entity.
/// `world::dispatch::dispatch_spawn_entity` runs its own merge — the same
/// `merge_entity_config_toml` + strict re-parse, so it reaches the same verdict
/// — but answers a failure by keeping the template and pushing a warning, so
/// the entity does spawn, wearing none of the override.
///
/// It is still an error here, deliberately. The consequence differs; the
/// authoring defect does not, and neither does the signal. A hull that flies
/// the doctrine its author meant to replace is not a degraded version of the
/// authored world, it is a different one, and today the only thing that says so
/// is a log line that fails nothing — which is the whole complaint #973 is
/// about. Nothing can make the merge succeed later, on any host, so there is
/// nothing for a warning to be tentative about.
///
/// # Host gating applies to the FIRST shape only
///
/// `relative_to` resolves entirely within the parsed [`WorldConfig`] in hand,
/// so every host can decide it completely (see [`validate_relative_to_in`]).
/// A template *path* cannot be decided without asking somebody for the
/// template, and not every host can answer; the condition is named rather than
/// implied by a `cfg`: [`TemplateLoader::absence_is_final`], the exact twin of
/// [`crate::entities::include_resolve::FragmentSource::absence_is_final`] one layer down.
///
/// * **Hard-fails**: native hosts — headless (`FsTemplateLoader`, and
///   `WasmTemplateLoader` through its filesystem fallback) and every fixture
///   loader, all of which hold everything they will ever hold.
/// * **Stays silent**: the browser. `WasmTemplateLoader` on `wasm32` serves the
///   preloaded config cache, which fills one delivery at a time while the
///   runtime layer load spawns the moment a layer's TOML arrives. Reading that
///   race as "the template does not exist" would blank the whole world, and
///   permanently, since the layer is marked loaded and never retried.
///
/// A failed **merge** is not host-gated, and must not be: once the template is
/// in hand the merge is decided entirely from content in hand, on any target.
/// There is no "not yet" to confuse it with — a later delivery cannot change
/// the answer, because the template it would deliver is the one already merged
/// against. Gating it on `absence_is_final` would leave the browser running the
/// exact silent drop this validator exists to refuse.
///
/// # Why "validation passed" means "the spawn will resolve it"
///
/// The loader handed here is the one the spawn will consult —
/// [`crate::entities::loader::SpawnTemplateLoader`] over the very
/// `ConfigCache` the spawn reads (see `world::server::world_activation_blocked`).
/// Before #973 the two disagreed: validation asked `WasmTemplateLoader`
/// (filesystem fallback on native) while spawning asked the cache alone, so on
/// native validation could pass on a template the spawn could not find.
///
/// # Deduplication
///
/// The two shapes dedupe differently, because their defects live at different
/// cardinalities. A missing template is a property of the *hull*, deduped by
/// canonical path across the whole composition (`seen`) — `./assets/x.toml` and
/// `assets/x.toml` are one hull, and one hull is one finding, matching
/// [`validate_template_composition_in`]. A failed merge is a property of the
/// *instance*: two `[[entity]]` blocks on the same hull carry different
/// `overrides` tables, so each is reported on its own.
fn validate_template_resolution_in(
    src: &WorldSource,
    loader: &dyn TemplateLoader,
    seen: &mut HashSet<String>,
) -> Vec<WorldFinding> {
    // The authority condition, read once for the whole world rather than per
    // entity: a host that cannot distinguish "absent" from "not yet" reports no
    // *presence* finding at all. It does not gate the merge check below.
    let absence_is_final = loader.absence_is_final();

    let mut findings = Vec::new();
    for inst in collect_spawned_instances(src.config) {
        let Some(template) = loader.load_template(inst.template_path) else {
            if !absence_is_final {
                continue;
            }
            if !seen.insert(crate::entities::include_resolve::canonical_template_path(
                inst.template_path,
            )) {
                continue;
            }
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: "unresolvable-template",
                message: format!(
                    "entity '{}' names template '{}', which this host cannot resolve \
                     (missing, unreadable, or invalid), in '{}'; every spawn caller \
                     would log and skip it, leaving the world silently short of that \
                     entity",
                    inst.label, inst.template_path, src.path
                ),
                source: SourceLocation {
                    file: src.path.clone(),
                    line: line_of(src.toml, inst.template_path)
                        .or_else(|| line_of(src.toml, &inst.label)),
                    reference: inst.template_path.to_string(),
                },
            });
            continue;
        };

        // The template resolved. The other half of what the spawn does with it
        // is the instance merge — the same call, through the same function.
        let Some(overrides) = inst.overrides else {
            continue;
        };
        match crate::entities::loader::apply_overrides(&template, overrides) {
            Ok(_merged) => {
                // The merge succeeded, but "succeeded" only means the merged
                // document re-parses — see `validate_override_table_presence`
                // for the quieter defect that survives a clean merge.
                findings.extend(validate_override_table_presence(
                    &inst, &template, overrides, src,
                ));
            }
            Err(e) => {
                findings.push(WorldFinding {
                    severity: Severity::Error,
                    category: "unmergeable-override",
                    message: format!(
                        "entity '{}' carries an 'overrides' table that does not apply to its \
                         template '{}', in '{}': {e}; every spawn caller would log and skip \
                         it, leaving the world silently short of that entity",
                        inst.label, inst.template_path, src.path
                    ),
                    source: SourceLocation {
                        file: src.path.clone(),
                        line: line_of(src.toml, &inst.label)
                            .or_else(|| line_of(src.toml, inst.template_path)),
                        reference: inst.template_path.to_string(),
                    },
                });
            }
        }
    }
    findings
}

/// Warn when an instance `overrides` table names a top-level TABLE the
/// resolved template does not declare (issue #1043).
///
/// # The foot-gun
///
/// [`crate::entities::loader::apply_overrides`] deep-merges an override onto the
/// template's serialised document (`EntityConfig::to_toml_value`); when the
/// template's own field is `None`, its key is simply absent from that
/// document, so the merge does not refuse the override or merge it into
/// anything — it inserts the override's table **fresh**, and the merged
/// document still parses. The load succeeds either way, so nothing here is an
/// `unmergeable-override`.
///
/// Whether that freshly-inserted table then does anything is entirely up to
/// whatever system reads it, and a template that never declared the table
/// typically has nothing wired up to read it. That is precisely how #1043
/// lost a debugging pass: a `[behaviour]` override on a hauler template with
/// no `[behaviour]` of its own parsed cleanly and changed nothing anyone could
/// see. `player_hull_config` (`src/server_app.rs`) documents the exact same
/// "existing absent-table semantics" for the player-ship row and deliberately
/// leaves them unchanged — "making it loud is a separate task". This is that
/// task, at the one place both spawn paths (`resolve_entity_via` and
/// `player_hull_config`) actually share: the merge itself.
///
/// # Why a WARNING and not an ERROR
///
/// Unlike `unmergeable-override`, nothing here is refused: the entity spawns
/// whole, carrying the override's data, and a template gaining the table later
/// is a legitimate, unremarkable edit — the doctrine anchor/route checks this
/// sits beside are errors because a miss can never resolve on any later load;
/// this one already might, on the very next template edit. The author still
/// needs to see it, because today it is silent.
///
/// # Only TABLES, not scalars
///
/// A scalar override (`hull_id`, `power_rating`, `class`, ...) on a field the
/// template leaves unset is ordinary authoring, not this foot-gun: a scalar is
/// read directly wherever it is read, with no separate system that has to
/// already be wired up to consume it. Only a nested `toml::Value::Table`
/// override — a config *section* — carries the "attaches to nothing" risk, so
/// only those are checked. `WorldEntity`'s own `transform` (position/rotation)
/// lives as a sibling field beside `overrides`, never inside it, so there is no
/// override-only key here that could false-positive against this rule; every
/// key that reaches this function is a real `EntityConfig` field name, or the
/// merge above would already have refused it as `unmergeable-override`.
///
/// # Deriving the table set from the struct, not a hardcoded list
///
/// The set of "tables the template declares" is read off the SAME
/// `EntityConfig::to_toml_value()` document `apply_overrides` merges onto —
/// whatever key serde actually emitted for this resolved (fragment-composed)
/// template — rather than a hardcoded field-name list. That keeps this check
/// honest against `EntityConfig` as it evolves: a renamed or newly-added
/// section is covered for free, and nothing here has to be told about
/// `workforce` or the next table by name.
fn validate_override_table_presence(
    inst: &SpawnedInstance,
    template: &crate::entities::config::EntityConfig,
    overrides: &toml::Value,
    src: &WorldSource,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let Some(override_table) = overrides.as_table() else {
        return findings;
    };
    // Serialising a second time (`apply_overrides` above already did this
    // once, successfully) is the price of keeping this check independent of
    // that function's return value, which discards the intermediate template
    // table. `to_toml_value` is a cheap, pure, in-memory serialise, and
    // validation is not a hot path.
    let Ok(template_value) = template.to_toml_value() else {
        return findings;
    };
    let Some(template_table) = template_value.as_table() else {
        return findings;
    };

    for (key, value) in override_table {
        if !matches!(value, toml::Value::Table(_)) {
            continue;
        }
        if template_table.contains_key(key) {
            continue;
        }
        findings.push(WorldFinding {
            severity: Severity::Warning,
            category: "override-absent-table",
            message: format!(
                "entity '{}' overrides '[{key}]', which template '{}' does not declare \
                 after fragment composition, in '{}'; the merge inserts the table fresh \
                 rather than refusing it, so the load succeeds but nothing the template \
                 already wires up reads it — the override may attach to nothing until \
                 the template gains that table itself",
                inst.label, inst.template_path, src.path
            ),
            source: SourceLocation {
                file: src.path.clone(),
                line: line_of(src.toml, key)
                    .or_else(|| line_of(src.toml, &inst.label))
                    .or_else(|| line_of(src.toml, inst.template_path)),
                reference: key.clone(),
            },
        });
    }
    findings
}

/// Composition findings for one world's entity templates, for callers that hold
/// a single [`WorldConfig`] rather than a whole effective composition — the
/// Bevy `Startup` spawn gate in `world::server`.
///
/// `path`/`toml` are accepted for symmetry with [`validate_entity_identity`] and
/// [`validate_objectives`]; the spawn gate passes `""` for both, exactly as it
/// does there. Findings carry the resolver's own source location regardless.
pub fn validate_template_composition(
    path: &str,
    toml: &str,
    config: &WorldConfig,
    fragments: &dyn FragmentSource,
) -> Vec<WorldFinding> {
    let src = WorldSource::new(path, toml, config);
    let mut seen = HashSet::new();
    validate_template_composition_in(&src, fragments, &mut seen)
}

/// Reject an `[[entity]]` whose `transform.relative_to` names nothing this
/// world can position it against (issue #969).
///
/// # Why this is an ERROR and never a warning
///
/// Every spawn caller resolves a position and, on failure, logs and `continue`s
/// — so an unresolvable `relative_to` degrades to *the entity is absent*, with
/// the rest of the world spawning happily around the hole. That is precisely
/// how `combat_test.toml`'s ice moon went missing for three weeks: nothing the
/// scenario asserts on noticed a moon that was never there. An author who wrote
/// `relative_to` asked for a position they cannot compute themselves; refusing
/// to place the entity at all is not a degraded answer to that, it is a
/// different world.
///
/// # Why it needs no host gating
///
/// Unlike the template-shaped validators above ([`doctrine_anchor_refs`],
/// [`validate_template_composition_in`]), which stay quiet about templates the
/// host's loader cannot reach, `relative_to` resolves **entirely within the
/// parsed [`WorldConfig`] in hand** — no template load, no filesystem, no
/// preload cache. Every host that can parse the world can decide this
/// completely, so the browser host is as authoritative as the native one and
/// the error is unconditional.
///
/// # Agreement with the runtime
///
/// The check asks
/// [`crate::world::config::build_named_entity_positions`] — the very table the
/// spawners look references up in — so "validation passed" means "every
/// `relative_to` resolves at spawn time", with no second opinion to drift. The
/// message then inspects the entity list to say *why* it missed, since the three
/// causes want different fixes: an unknown reference, a base that is itself
/// `relative_to`-positioned (chains are unsupported by design), or a base whose
/// own anchor does not resolve.
fn validate_relative_to_in(src: &WorldSource) -> Vec<WorldFinding> {
    let resolvable = crate::world::config::build_named_entity_positions(src.config);
    let mut findings = Vec::new();

    for ent in &src.config.entities {
        let Some(reference) = ent.transform.as_ref().and_then(|t| t.relative_to.as_ref()) else {
            continue;
        };
        if resolvable.contains_key(reference.as_str()) {
            continue;
        }

        // Which authored identifiers name the reference at all, ignoring
        // whether that entity's own position resolved.
        let base = src.config.entities.iter().find(|e| {
            e.id.as_deref() == Some(reference.as_str())
                || e.name.as_deref() == Some(reference.as_str())
        });
        let why = match base {
            None => format!("no entity in this world declares '{reference}' as its `id` or `name`"),
            Some(b)
                if b.transform
                    .as_ref()
                    .and_then(|t| t.relative_to.as_ref())
                    .is_some() =>
            {
                format!(
                    "entity '{reference}' is itself positioned with `relative_to`, and \
                     relative-to-relative chains are not supported"
                )
            }
            Some(_) => format!(
                "entity '{reference}' exists but its own position does not resolve \
                 (check its `anchor`)"
            ),
        };

        let label = ent
            .name
            .clone()
            .or_else(|| ent.id.clone())
            .unwrap_or_else(|| ent.template_path.clone());
        findings.push(WorldFinding {
            severity: Severity::Error,
            category: "unresolved-relative-to",
            message: format!(
                "entity '{label}' (template '{}') is positioned \
                 `relative_to = \"{reference}\"`, which does not resolve in '{}': {why}",
                ent.template_path, src.path
            ),
            source: SourceLocation {
                file: src.path.clone(),
                // The authored spelling first (the line the author edits),
                // then the bare quoted reference for compact/odd spacing,
                // then the spawn site.
                line: line_of(src.toml, &format!("relative_to = \"{reference}\""))
                    .or_else(|| line_of(src.toml, &format!("relative_to=\"{reference}\"")))
                    .or_else(|| line_of(src.toml, ent.template_path.as_str())),
                reference: reference.clone(),
            },
        });
    }

    findings
}

/// `relative_to` findings for one world, for callers that hold a single
/// [`WorldConfig`] rather than a whole effective composition — the Bevy
/// `Startup` spawn gate in `world::server`.
///
/// `path`/`toml` are accepted for symmetry with [`validate_entity_identity`];
/// the spawn gate passes `""` for both, exactly as it does there.
pub fn validate_relative_to(path: &str, toml: &str, config: &WorldConfig) -> Vec<WorldFinding> {
    validate_relative_to_in(&WorldSource::new(path, toml, config))
}

/// Every finding that blocks activation of a root world at Bevy `Startup`.
///
/// # Why this is one function and not a list each caller assembles
///
/// The immediate `[[entity]]` spawn is split across **two** `Startup` systems
/// with no ordering relationship between them: `spawn_world_entities` takes
/// asteroid fields and named entries, `setup_world` takes the anonymous
/// non-asteroid remainder (stars, planets, nebulae — see
/// [`crate::world::config::is_owned_by_unified_pipeline`]). Both answer a
/// failure to resolve an entity by logging and `continue`ing, so a gate on one
/// of them alone does not buy atomicity — it converts "one entity missing" into
/// "one half of the world missing", which is strictly worse and contradicts
/// `world-content-lifecycle-state` ("Failed validation leaves no partial
/// root-world content active"). Both systems consult this, so a world that
/// fails validation spawns nothing at all whichever of them runs first.
///
/// The headless build stays atomic for a different reason: it validates the
/// whole composition at build time and aborts before `Startup` runs at all.
///
/// `fragments` is the include-fragment source; every caller in the Bevy app
/// passes [`crate::entities::include_resolve::HostFragmentSource`], and tests pass a
/// fixture.
///
/// `templates` is the parsed-template source (issue #973). The Bevy caller
/// passes [`crate::entities::loader::SpawnTemplateLoader`] built over the exact
/// `ConfigCache` the spawn about to be gated will read, so this gate answers
/// the question that spawn will ask rather than a similar one.
pub fn activation_findings(
    config: &WorldConfig,
    fragments: &dyn FragmentSource,
    templates: &dyn TemplateLoader,
) -> Vec<WorldFinding> {
    // Entity identity (issue #750): duplicate reference names.
    let mut findings = validate_entity_identity("", "", &config.entities);
    // Objective authoring (issue #752): duplicate declarations within a single
    // action list, or complete/fail references to objectives no add_objective
    // declares.
    findings.extend(validate_objectives("", "", config));
    // Entity-template composition (issue #906): a template whose `includes`
    // closure is broken — a cycle, a missing fragment, a malformed `includes`
    // list, or a composed document that does not validate.
    findings.extend(validate_template_composition("", "", config, fragments));
    // Positioning references (issue #969): a `relative_to` naming nothing this
    // world can position the entity against.
    findings.extend(validate_relative_to("", "", config));
    // Template resolution (issue #973): anything `resolve_entity_via` would
    // refuse — a `template_path` the spawn's own loader cannot resolve (on a
    // host whose loader can be authoritative about that), or an `overrides`
    // table that does not merge onto the template it names (on every host).
    let src = WorldSource::new("", "", config);
    let mut seen = HashSet::new();
    findings.extend(validate_template_resolution_in(&src, templates, &mut seen));
    findings
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
/// so shipped worlds keep activating while authors still get feedback. Doctrine
/// *anchors* are the deliberate exception — see
/// [`validate_doctrine_anchors_in`].
///
/// Entity templates are resolved through the standard
/// [`crate::entities::loader::WasmTemplateLoader`] (preloaded config cache first,
/// filesystem fallback on native). Callers holding content the loader cannot
/// see — a mod pack's own `assets/entities/*.toml`, say — use
/// [`validate_composition_with`].
pub fn validate_composition(root: &WorldSource, children: &[WorldSource]) -> Vec<WorldFinding> {
    validate_composition_with(root, children, &crate::entities::loader::WasmTemplateLoader)
}

/// [`validate_composition`] with an explicit entity-template source.
///
/// Include fragments still come from the host source
/// ([`crate::entities::include_resolve::HostFragmentSource`]) — a `TemplateLoader` serves
/// parsed configs and cannot serve raw fragment text. Use
/// [`validate_composition_with_fragments`] to control both.
pub fn validate_composition_with(
    root: &WorldSource,
    children: &[WorldSource],
    template_loader: &dyn TemplateLoader,
) -> Vec<WorldFinding> {
    validate_composition_with_fragments(
        root,
        children,
        template_loader,
        &crate::entities::include_resolve::HostFragmentSource,
    )
}

/// [`validate_composition_with`] with an explicit *fragment* source as well —
/// the raw-TOML channel include resolution reads (issue #906).
pub fn validate_composition_with_fragments(
    root: &WorldSource,
    children: &[WorldSource],
    template_loader: &dyn TemplateLoader,
    fragments: &dyn FragmentSource,
) -> Vec<WorldFinding> {
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

    // Doctrine anchor references (issue #888). Resolve against the union of
    // the anchor tables declared across the effective composition, so a layer
    // whose ships steer to the base world's anchors does not false-positive.
    let mut declared_anchors: HashSet<&str> =
        root.config.anchors.keys().map(String::as_str).collect();
    for child in children {
        declared_anchors.extend(child.config.anchors.keys().map(String::as_str));
    }
    findings.extend(validate_doctrine_anchors_in(
        root,
        &declared_anchors,
        template_loader,
    ));
    for child in children {
        findings.extend(validate_doctrine_anchors_in(
            child,
            &declared_anchors,
            template_loader,
        ));
    }

    // Civilian route references (issue #1028), resolved against the same two
    // unions for the same reason: a route's legs are anchors, and a hauler
    // spawned by a layer may fly a lane the base world declares.
    findings.extend(validate_route_anchors_in(root, &declared_anchors));
    for child in children {
        findings.extend(validate_route_anchors_in(child, &declared_anchors));
    }
    let mut declared_routes: HashSet<&str> =
        root.config.routes.iter().map(|r| r.id.as_str()).collect();
    for child in children {
        declared_routes.extend(child.config.routes.iter().map(|r| r.id.as_str()));
    }
    findings.extend(validate_civilian_routes_in(
        root,
        &declared_routes,
        template_loader,
    ));
    for child in children {
        findings.extend(validate_civilian_routes_in(
            child,
            &declared_routes,
            template_loader,
        ));
    }

    // `relative_to` positioning references (issue #969). Resolved PER WORLD,
    // not against the composition union: the runtime lookup table
    // (`build_named_entity_positions`) is built from one `WorldConfig`, so a
    // cross-world `relative_to` would not resolve at spawn time either.
    // Matching that exactly is what lets a pass here promise a spawn.
    findings.extend(validate_relative_to_in(root));
    for child in children {
        findings.extend(validate_relative_to_in(child));
    }

    // Entity-template composition (issue #906). Deduplicated across the WHOLE
    // effective composition: a hull spawned by both the root and a child has
    // one broken include, not two.
    let mut composed_seen: HashSet<String> = HashSet::new();
    findings.extend(validate_template_composition_in(
        root,
        fragments,
        &mut composed_seen,
    ));
    for child in children {
        findings.extend(validate_template_composition_in(
            child,
            fragments,
            &mut composed_seen,
        ));
    }

    // Entity-template RESOLUTION (issue #973): a `template_path` the host's
    // loader cannot resolve at all, or an `overrides` table that will not merge
    // onto it. The presence half is deduplicated across the composition on the
    // same reasoning as above, and with its own set — one hull can be both
    // absent from the template loader and reachable in the fragment source, or
    // vice versa. The merge half is per instance and not deduplicated.
    let mut resolve_seen: HashSet<String> = HashSet::new();
    findings.extend(validate_template_resolution_in(
        root,
        template_loader,
        &mut resolve_seen,
    ));
    for child in children {
        findings.extend(validate_template_resolution_in(
            child,
            template_loader,
            &mut resolve_seen,
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
    use crate::world::load::MemoryTemplateLoader;

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

    // Fixtures below that assert on the ABSENCE of errors name templates that
    // are really on disk (issue #973): a `template_path` that resolves nowhere
    // is itself an error now, on any host whose loader is authoritative, and
    // `validate_composition`'s default loader is authoritative on native.

    // ── child_world.<name> resolution (AC3) ──────────────────────────────────

    // ── ambiguous reference (AC2) ────────────────────────────────────────────

    // ── unresolved bare reference is a non-blocking warning ──────────────────

    // ── objective authoring validation (AC2, issue #752) ─────────────────────

    // ── doctrine anchor references (issue #888) ──────────────────────────────

    /// Entity-template source for the doctrine-anchor fixtures: a fixed
    /// path → TOML map, so they never reach the filesystem or the process-wide
    /// config cache. Authoritative about absence (issue #973) — the map holds
    /// everything it will ever hold, the same answer the `HashMap`
    /// [`FragmentSource`] gives one layer down.
    fn fake_templates(entries: &[(&str, &str)]) -> MemoryTemplateLoader {
        MemoryTemplateLoader::from_toml(entries.iter().copied())
    }

    /// A loader that has DELIVERED its templates but is still filling — the
    /// browser mid-preload, holding the hull in hand while other paths are in
    /// flight (issue #973 review, F6).
    ///
    /// The one fixture that can tell the two halves of
    /// [`validate_template_resolution_in`] apart: absence is *not* final here,
    /// so the presence check stays silent, but the merge check must not, since
    /// a template in hand decides its own merge on any target.
    fn still_filling(entries: &[(&str, &str)]) -> MemoryTemplateLoader {
        entries.iter().fold(
            MemoryTemplateLoader::still_filling(),
            |loader, (path, toml)| loader.with_toml(*path, toml),
        )
    }

    /// A loader that can serve nothing AND knows it cannot: the browser's
    /// answer, which a native suite cannot otherwise reach (issue #973).
    ///
    /// Used by the fixtures below that are about some *other* check and whose
    /// worlds name templates no source in the test holds. Before #973 every
    /// loader was implicitly blind in this way, which is why those fixtures
    /// were written with paths that resolve nowhere.
    fn blind_templates() -> MemoryTemplateLoader {
        MemoryTemplateLoader::blind()
    }

    /// The ship-level AI declarations an AI-bearing hull owes, appended to the
    /// fixtures below (issue #885b stage 5d).
    ///
    /// Strict AI-declaration mode makes a `[behaviour]` hull that omits any of
    /// them fail to load, and `entity_override::apply_overrides` re-parses the
    /// MERGED document strictly — so an override-carrying fixture has to be a
    /// hull a real world could ship, not a doctrine snippet. Taken verbatim from
    /// the shared fragment rather than restated, so it cannot drift from it.
    const BASELINE_AI: &str =
        include_str!("../../assets/entities/fragments/ai/fleet_baseline.toml");

    /// The one declaration the shared fragment deliberately leaves to its
    /// includer (see that file's header).
    const CAPTAIN_AI: &str = r#"
[captain_console.ai]
param = { combat_window_secs = 10.0 }

[[captain_console.ai.rule]]
priority = 10
channel = "red_alert"
when = "fact(secs_since_combat) < param(combat_window_secs)"
verb = "set_red_alert"
value = true

[[captain_console.ai.rule]]
priority = 0
channel = "red_alert"
when = "true"
verb = "set_red_alert"
value = false
"#;

    /// A hull that patrols two named anchors.
    const PATROLLER: &str = r#"
name = "entity.patroller.name"

[behaviour]

[[behaviour.doctrine]]
id                = "patrol-route"
text              = "entity.patroller.doctrine.patrol.text"
directive_kind    = "Patrol"
directive_anchors = ["route_a", "route_b"]
directive_loop    = true
base_priority     = 20.0
"#;

    /// The patroller hull's authored `[behaviour]` document, shared by
    /// [`patroller_templates`] (authoritative) and the
    /// `an_unmergeable_override_is_reported_even_where_absence_is_not_final`
    /// test below (same content, non-final absence).
    fn patroller_toml() -> String {
        format!("{PATROLLER}\n{CAPTAIN_AI}\n{BASELINE_AI}")
    }

    fn patroller_templates() -> MemoryTemplateLoader {
        fake_templates(&[("assets/entities/patroller.toml", &patroller_toml())])
    }

    #[test]
    fn doctrine_anchor_declared_nowhere_is_rejected() {
        let root = cfg(r#"
[[entity]]
template_path = "assets/entities/patroller.toml"
name = "ashrender"
"#);
        let root_toml = "name = \"ashrender\"";
        let src = WorldSource::new("assets/worlds/scenario.toml", root_toml, &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());

        let errs: Vec<_> = findings
            .iter()
            .filter(|f| f.category == "unresolved-anchor")
            .collect();
        assert_eq!(
            errs.len(),
            2,
            "one error per unresolved route waypoint: {findings:?}"
        );
        assert!(
            has_error(&findings),
            "an unresolved anchor blocks the world"
        );
        for (err, anchor) in errs.iter().zip(["route_a", "route_b"]) {
            assert_eq!(err.source.reference, anchor);
            assert_eq!(err.source.file, "assets/worlds/scenario.toml");
            // The message has to name all three so an author can act on it
            // without opening the entity template.
            assert!(err.message.contains("ashrender"), "{}", err.message);
            assert!(err.message.contains(anchor), "{}", err.message);
            assert!(
                err.message.contains("assets/worlds/scenario.toml"),
                "{}",
                err.message
            );
        }
    }

    #[test]
    fn doctrine_anchor_the_world_declares_resolves() {
        let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]

[[entity]]
template_path = "assets/entities/patroller.toml"
name = "ashrender"
"#);
        let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(
            !has_error(&findings),
            "declared anchors resolve: {findings:?}"
        );
    }

    /// The case that makes the check non-trivial: a sub-world spawns a hull
    /// whose route is declared by the world it layers onto (the `btf_path_*`
    /// shape). A per-file linter would report two typos here.
    #[test]
    fn layered_world_inheriting_base_anchors_does_not_false_positive() {
        let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]
"#);
        let child = cfg(r#"
[[entity]]
template_path = "assets/entities/patroller.toml"
name = "reinforcement"
"#);
        let root_src = WorldSource::new("assets/worlds/base.toml", "", &root);
        let child_src = WorldSource::new("assets/worlds/layer.toml", "", &child);
        let findings = validate_composition_with(&root_src, &[child_src], &patroller_templates());
        assert!(
            !has_error(&findings),
            "a layer inherits its base world's anchors: {findings:?}"
        );
    }

    /// The lever the shipped worlds use to say "this hull has no patrol here"
    /// (`before_the_fire.toml`, `probe_artillery_standoff.toml`): the doctrine
    /// read is the *effective* one, so a by-id override that stands the
    /// directive down resolves clean.
    #[test]
    fn override_standing_the_directive_down_resolves() {
        let root = cfg(r#"
[[entity]]
template_path = "assets/entities/patroller.toml"
name = "ashrender"
overrides = { behaviour = { doctrine = [
  { id = "patrol-route", directive_kind = "None", directive_anchors = [], directive_loop = false },
] } }
"#);
        let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(
            !has_error(&findings),
            "a stood-down directive references no anchor: {findings:?}"
        );
    }

    /// The converse: an override that *introduces* a directive is checked too,
    /// so a scenario cannot smuggle in an unresolvable anchor by override.
    #[test]
    fn override_introducing_a_reach_anchor_is_validated() {
        let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]

[[entity]]
template_path = "assets/entities/patroller.toml"
name = "courier"
overrides = { behaviour = { doctrine = [
  { id = "deliver", directive_kind = "Reach", directive_anchor = "destination", base_priority = 90.0 },
] } }
"#);
        let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        let err = findings
            .iter()
            .find(|f| f.category == "unresolved-anchor")
            .expect("an override-introduced Reach anchor must resolve too");
        assert_eq!(err.source.reference, "destination");
        assert!(err.message.contains("Reach"), "{}", err.message);
    }

    /// A template the loader cannot serve must not turn into an ANCHOR
    /// complaint — the validator cannot know what doctrine a template it never
    /// read declares, and inventing one would send an author hunting for an
    /// anchor bug that is really a path bug.
    ///
    /// Since #973 it is not silent either: the same world now carries an
    /// `unresolvable-template` error, which is the finding that actually names
    /// the defect. This test pins the split, not the silence.
    #[test]
    fn unloadable_template_produces_no_anchor_finding() {
        let root = cfg(r#"
[[entity]]
template_path = "assets/entities/nowhere.toml"
name = "ghost"
"#);
        let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(
            !findings.iter().any(|f| f.category == "unresolved-anchor"),
            "{findings:?}"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unresolvable-template" && f.is_error()),
            "the defect is reported as what it is, by the check that owns it \
             (issue #973): {findings:?}"
        );
    }

    // ── civilian routes (issue #1028) ────────────────────────────────────────

    /// **AC1.** A route leg naming an anchor nobody declares blocks the world.
    ///
    /// Routes go through the same gate doctrine anchors do, and for the same
    /// reason: the anchor table is parsed once and never written again, so a
    /// leg that misses at load misses forever and the hauler flying that lane
    /// silently skips it.
    #[test]
    fn a_route_leg_naming_an_undeclared_anchor_is_rejected() {
        let root = cfg(r#"
[anchors]
depot_north = [10.0, 0.0, 20.0]

[[route]]
id = "depot_run"

[[route.leg]]
anchor = "depot_north"

[[route.leg]]
anchor = "depot_south"
"#);
        let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        let err = findings
            .iter()
            .find(|f| f.category == "unresolved-anchor")
            .expect("the undeclared leg anchor must be an error");
        assert_eq!(err.source.reference, "depot_south");
        assert!(err.message.contains("depot_run"), "{}", err.message);
        assert!(err.is_error(), "it blocks activation rather than warning");
    }

    /// …and a route whose every leg resolves is silent, including one whose
    /// anchors are declared by a *sibling layer* rather than by its own file.
    #[test]
    fn a_route_resolving_against_the_composition_is_accepted() {
        let root = cfg(r#"
[anchors]
depot_north = [10.0, 0.0, 20.0]
"#);
        let layer = cfg(r#"
[[route]]
id = "depot_run"

[[route.leg]]
anchor = "depot_north"
"#);
        let root_src = WorldSource::new("assets/worlds/base.toml", "", &root);
        let layer_src = WorldSource::new("assets/worlds/layer.toml", "", &layer);
        let findings = validate_composition_with(&root_src, &[layer_src], &patroller_templates());
        assert!(
            !findings.iter().any(|f| f.category == "unresolved-anchor"),
            "a lane may cross the base world's anchors: {findings:?}"
        );
    }

    /// **AC1.** A civilian assigned a lane nobody declares blocks the world too
    /// — otherwise it spawns, installs no directive, and holds station for the
    /// whole mission with no diagnostic anywhere.
    #[test]
    fn a_civilian_assigned_an_undeclared_route_is_rejected() {
        let templates = fake_templates(&[(
            "assets/entities/hauler.toml",
            &format!(
                "{PATROLLER}
[civilian]
route = \"storm_detour\"
{CAPTAIN_AI}
{BASELINE_AI}"
            ),
        )]);
        let root = cfg(r#"
[anchors]
depot_north = [10.0, 0.0, 20.0]

[[route]]
id = "depot_run"

[[route.leg]]
anchor = "depot_north"

[[entity]]
template_path = "assets/entities/hauler.toml"
name = "kestrel"
"#);
        let src = WorldSource::new("assets/worlds/scenario.toml", "name = \"kestrel\"", &root);
        let findings = validate_composition_with(&src, &[], &templates);
        let err = findings
            .iter()
            .find(|f| f.category == "unresolved-route")
            .expect("an unresolved civilian route assignment must be an error");
        assert_eq!(err.source.reference, "storm_detour");
        assert!(err.message.contains("kestrel"), "{}", err.message);

        // The control: the same world with the lane declared is clean.
        let mut with_lane = root.clone();
        with_lane.routes[0].id = "storm_detour".into();
        let src = WorldSource::new("assets/worlds/scenario.toml", "", &with_lane);
        assert!(
            !validate_composition_with(&src, &[], &templates)
                .iter()
                .any(|f| f.category == "unresolved-route"),
            "a civilian pointed at a declared lane is silent"
        );
    }

    // ── relative_to positioning references (issue #969) ──────────────────────

    /// The failure case the issue asks for: an entity positioned against a
    /// reference nothing declares must FAIL VALIDATION, not spawn-loop past a
    /// log line and leave the world one entity short.
    #[test]
    fn an_unresolvable_relative_to_blocks_activation_instead_of_dropping_the_entity() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
transform = { position = [400.0, 0.0, 400.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
transform = { relative_to = "erth", offset = [60.0, 0.0, 30.0] }
"#);
        let toml = r#"transform = { relative_to = "erth", offset = [60.0, 0.0, 30.0] }"#;
        let src = WorldSource::new("assets/worlds/typo.toml", toml, &config);
        let findings = validate_relative_to_in(&src);
        let err = findings
            .iter()
            .find(|f| f.category == "unresolved-relative-to")
            .expect("a typo'd relative_to must be reported");
        assert!(err.is_error(), "must block activation, not warn");
        assert_eq!(err.source.reference, "erth");
        assert_eq!(
            err.source.line,
            Some(1),
            "the finding points at the authored reference"
        );
        assert!(has_error(&findings));
    }

    /// It reaches the composition gate, not just the helper.
    #[test]
    fn an_unresolvable_relative_to_is_reported_by_validate_composition() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
transform = { relative_to = "nobody", offset = [1.0, 0.0, 0.0] }
"#);
        let src = WorldSource::new("assets/worlds/typo.toml", "", &config);
        let findings = validate_composition(&src, &[]);
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unresolved-relative-to" && f.is_error()),
            "{findings:?}"
        );
    }

    /// Both resolution directions pass validation, since the runtime table they
    /// are checked against is built over the whole file before anything moves.
    #[test]
    fn relative_to_declared_earlier_or_later_validates_clean() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "forward-moon"
transform = { relative_to = "planet", offset = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "planet"
name = "world.entity.earth.name"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_ice.toml"
id = "backward-moon"
transform = { relative_to = "world.entity.earth.name", offset = [0.0, 0.0, 5.0] }
"#);
        let src = WorldSource::new("assets/worlds/both.toml", "", &config);
        assert!(
            validate_relative_to_in(&src).is_empty(),
            "a reference by `id` (forward) or by `name` (backward) both resolve"
        );
    }

    /// A chain reads as "missing entity" to the lookup table but is a distinct
    /// authoring mistake, so the message says which one it is.
    #[test]
    fn a_relative_to_chain_is_named_as_a_chain_not_a_missing_entity() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "planet"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "moon"
transform = { relative_to = "planet", offset = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/station_axiom.toml"
id = "outpost"
transform = { relative_to = "moon", offset = [0.0, 1.0, 0.0] }
"#);
        let src = WorldSource::new("assets/worlds/chain.toml", "", &config);
        let findings = validate_relative_to_in(&src);
        assert_eq!(findings.len(), 1, "only the chained entity errors");
        assert!(
            findings[0].message.contains("chains are not supported"),
            "{}",
            findings[0].message
        );
    }

    /// No template is loaded to decide any of this, so the check is the same on
    /// a host whose template loader can see nothing — the browser included.
    /// (Contrast `unloadable_template_produces_no_anchor_finding` above.)
    #[test]
    fn relative_to_validation_needs_no_template_loader() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/nowhere.toml"
id = "ghost"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/also-nowhere.toml"
id = "tag-along"
transform = { relative_to = "ghost", offset = [1.0, 0.0, 0.0] }
"#);
        let src = WorldSource::new("assets/worlds/unloadable.toml", "", &config);
        assert!(
            validate_relative_to_in(&src).is_empty(),
            "unloadable is fine"
        );
    }

    // ── ambiguous positioning spellings (issue #969) ─────────────────────────

    /// The one shape in which admitting `id` as a positioning key can re-point a
    /// reference that already worked: entity A holds `foo` as its `name`,
    /// entity B later claims `foo` as its `id`. It still resolves — to A, by
    /// rule — but the author who added B may well have meant B, so say so.
    /// Warning, not error: nothing is broken until something references it.
    #[test]
    fn an_id_shadowing_another_entitys_name_warns_without_blocking() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/station_axiom.toml"
name = "beacon"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "beacon"
transform = { position = [2.0, 0.0, 0.0] }
"#);
        let findings = validate_entity_identity("root.toml", "", &config.entities);
        let f = findings
            .iter()
            .find(|f| f.category == "ambiguous-entity-reference")
            .expect("the shadowed spelling must be reported");
        assert!(!f.is_error(), "ambiguity is a warning, not a block");
        assert_eq!(f.source.reference, "beacon");
        assert!(!has_error(&findings), "the world still activates");
    }

    /// Two entities sharing an `id` have no principled winner at all — only file
    /// order — so the same warning covers them.
    #[test]
    fn a_duplicate_id_warns_that_a_relative_to_would_be_ambiguous() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "twin"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "twin"
transform = { position = [2.0, 0.0, 0.0] }
"#);
        let findings = validate_entity_identity("root.toml", "", &config.entities);
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.category == "ambiguous-entity-reference")
                .count(),
            1,
            "one spelling, one finding: {findings:?}"
        );
        assert!(!has_error(&findings));
    }

    /// A duplicate `name` is already an error in the reference namespace. It
    /// must not also produce the positioning warning — one collision, one
    /// finding, and the error is the one worth reading.
    #[test]
    fn a_duplicate_name_errors_once_and_is_not_also_warned_about() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
name = "outpost"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
name = "outpost"
transform = { position = [2.0, 0.0, 0.0] }
"#);
        let findings = validate_entity_identity("root.toml", "", &config.entities);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, "duplicate-name");
    }

    /// An entity whose `id` and `name` are the same string claims that spelling
    /// once. That is one entity, and nothing about it is ambiguous.
    #[test]
    fn an_entity_whose_id_equals_its_own_name_is_not_ambiguous() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
name = "earth"
transform = { position = [1.0, 0.0, 0.0] }
"#);
        assert!(
            validate_entity_identity("root.toml", "", &config.entities).is_empty(),
            "a single entity cannot collide with itself"
        );
    }

    // ── the shared Startup activation gate (issue #969) ──────────────────────

    /// [`activation_findings`] is what both immediate-spawn systems consult, so
    /// a `relative_to` failure has to reach it — not only `validate_composition`,
    /// which the browser host never calls.
    #[test]
    fn activation_findings_carries_the_relative_to_error() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
transform = { relative_to = "nobody", offset = [1.0, 0.0, 0.0] }
"#);
        let findings = activation_findings(
            &config,
            &crate::entities::include_resolve::HostFragmentSource,
            &crate::entities::loader::WasmTemplateLoader,
        );
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unresolved-relative-to" && f.is_error()),
            "{findings:?}"
        );
        assert!(has_error(&findings));
    }

    // ── Template composition joins the finding flow (issue #906) ─────────────

    fn fragments(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    fn one_entity_world(template_path: &str, name: &str) -> String {
        format!("[[entity]]\ntemplate_path = \"{template_path}\"\nname = \"{name}\"\n")
    }

    /// The whole point of the issue: a world whose entity template cannot be
    /// composed FAILS VALIDATION instead of quietly losing the entity.
    #[test]
    fn a_missing_fragment_blocks_activation_instead_of_dropping_the_entity() {
        let world_toml = one_entity_world("assets/entities/broken.toml", "broken_one");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let source = fragments(&[(
            "assets/entities/broken.toml",
            "includes = [\"fragments/absent.toml\"]\nname = \"B\"\n",
        )]);

        let findings =
            validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);

        assert!(
            has_error(&findings),
            "a broken include must gate activation: {findings:?}"
        );
        let f = findings
            .iter()
            .find(|f| f.category == "include-missing")
            .unwrap_or_else(|| panic!("expected an include-missing finding: {findings:?}"));
        assert_eq!(
            f.source.file, "assets/entities/broken.toml",
            "the finding names the file that DECLARED the bad include"
        );
        assert_eq!(f.source.line, Some(1));
        assert!(
            f.message.contains("broken_one") && f.message.contains("assets/worlds/w.toml"),
            "the message names the entity and the world that pulled it in: {}",
            f.message
        );
    }

    #[test]
    fn an_include_cycle_blocks_activation() {
        let world_toml = one_entity_world("assets/entities/a.toml", "looper");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let source = fragments(&[
            ("assets/entities/a.toml", "includes = [\"b.toml\"]\n"),
            ("assets/entities/b.toml", "includes = [\"a.toml\"]\n"),
        ]);

        let findings =
            validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);
        assert!(findings.iter().any(|f| f.category == "include-cycle"));
        assert!(has_error(&findings));
    }

    #[test]
    fn a_malformed_includes_declaration_blocks_activation() {
        let world_toml = one_entity_world("assets/entities/bad.toml", "bad_one");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let source = fragments(&[("assets/entities/bad.toml", "includes = \"not-an-array\"\n")]);

        let findings =
            validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);
        assert!(findings.iter().any(|f| f.category == "include-malformed"));
        assert!(has_error(&findings));
    }

    /// A hull spawned by both layers has ONE broken include, not two.
    #[test]
    fn one_broken_template_is_reported_once_across_the_composition() {
        let root_toml = one_entity_world("assets/entities/broken.toml", "root_one");
        let root = cfg(&root_toml);
        let child_toml = one_entity_world("assets/entities/broken.toml", "child_one");
        let child = cfg(&child_toml);
        let root_src = WorldSource::new("assets/worlds/root.toml", &root_toml, &root);
        let child_src = WorldSource::new("assets/worlds/child.toml", &child_toml, &child);
        let source = fragments(&[(
            "assets/entities/broken.toml",
            "includes = [\"fragments/absent.toml\"]\n",
        )]);

        let findings = validate_composition_with_fragments(
            &root_src,
            &[child_src],
            &patroller_templates(),
            &source,
        );
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.category.starts_with("include-"))
                .count(),
            1,
            "{findings:?}"
        );
    }

    /// Two authored SPELLINGS of one path are still one broken hull.
    ///
    /// Authors write `./assets/…` as readily as `assets/…`, and the resolver
    /// canonicalises before it does anything, so both spellings reach the same
    /// template and produce the same fault. Deduplicating on the raw string
    /// would report it twice — the same duplication
    /// `one_broken_template_is_reported_once_across_the_composition` rules out
    /// for two worlds, arriving instead through the back door of punctuation.
    #[test]
    fn two_spellings_of_one_template_are_reported_once() {
        let world_toml = format!(
            "{}{}",
            one_entity_world("assets/entities/broken.toml", "plain"),
            one_entity_world("./assets/entities/broken.toml", "dotted"),
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let source = fragments(&[(
            "assets/entities/broken.toml",
            "includes = [\"fragments/absent.toml\"]\n",
        )]);

        let findings =
            validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.category.starts_with("include-"))
                .count(),
            1,
            "one broken hull, spelled two ways, is one finding: {findings:?}"
        );
    }

    /// A template the fragment source cannot see is NOT a composition error —
    /// a validator must not manufacture one out of its own blindness.
    ///
    /// The template loader is [`blind_templates`] rather than the fixture map
    /// so the claim stays about *composition*: an authoritative loader would
    /// (correctly, since #973) report the same world as
    /// `unresolvable-template`, and this test would then be asserting two
    /// things at once.
    #[test]
    fn a_template_the_source_cannot_see_produces_no_composition_finding() {
        let world_toml = one_entity_world("assets/entities/nowhere.toml", "ghost");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let source = fragments(&[]);

        let findings = validate_composition_with_fragments(&src, &[], &blind_templates(), &source);
        assert!(!findings.iter().any(|f| f.category.starts_with("include-")));
        assert!(!has_error(&findings), "{findings:?}");
    }

    #[test]
    fn a_template_that_composes_cleanly_produces_no_composition_finding() {
        let world_toml = one_entity_world("assets/entities/good.toml", "good_one");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let source = fragments(&[
            ("assets/entities/hull_core.toml", "class = \"escort\"\n"),
            (
                "assets/entities/good.toml",
                "includes = [\"hull_core.toml\"]\nname = \"G\"\n",
            ),
        ]);

        // Blind loader for the same reason as the test above: `good.toml` is a
        // fragment-source fixture, not a template the loader holds.
        let findings = validate_composition_with_fragments(&src, &[], &blind_templates(), &source);
        assert!(!findings.iter().any(|f| f.category.starts_with("include-")));
        assert!(!has_error(&findings), "{findings:?}");
    }

    // ── Template presence (issue #973) ───────────────────────────────────────

    /// The defect the issue is about: a world naming a template that does not
    /// resolve used to load "successfully" and simply spawn nothing, because
    /// every spawn caller logs the resolve failure and `continue`s. On a host
    /// whose loader is authoritative it now FAILS VALIDATION, loudly, before
    /// anything spawns.
    #[test]
    fn an_unresolvable_template_blocks_activation_on_an_authoritative_host() {
        let world_toml = one_entity_world("assets/entities/nowhere.toml", "ghost");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        // `patroller_templates()` holds one template and knows it holds
        // everything it will ever hold — a fixture map, like the filesystem, is
        // authoritative about absence.
        let templates = patroller_templates();
        assert!(templates.absence_is_final(), "precondition");

        let findings = validate_composition_with(&src, &[], &templates);
        let err = findings
            .iter()
            .find(|f| f.category == "unresolvable-template")
            .unwrap_or_else(|| panic!("expected an unresolvable-template finding: {findings:?}"));
        assert!(err.is_error(), "it must block activation, not warn");
        assert!(has_error(&findings));
        assert_eq!(err.source.reference, "assets/entities/nowhere.toml");
        assert_eq!(err.source.file, "assets/worlds/w.toml");
        assert_eq!(
            err.source.line,
            Some(2),
            "the finding points at the authored template_path"
        );
        // The message has to name the entity and the world, so an author can
        // act on it without opening every world file.
        assert!(err.message.contains("ghost"), "{}", err.message);
        assert!(
            err.message.contains("assets/entities/nowhere.toml"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("assets/worlds/w.toml"),
            "{}",
            err.message
        );
    }

    /// The other half, and the half that keeps the shipped client working: a
    /// host that cannot tell "absent" from "not yet" reports NOTHING.
    ///
    /// This is the browser. Its preloaded config cache fills one delivery at a
    /// time while the runtime layer load spawns the moment a layer's TOML
    /// arrives, so a template the loader cannot serve at validation time
    /// routinely resolves fine a moment later. Erroring there would blank the
    /// whole world, permanently — the layer is marked loaded and never retried.
    #[test]
    fn an_unresolvable_template_is_silent_on_a_host_that_cannot_be_authoritative() {
        let world_toml = one_entity_world("assets/entities/nowhere.toml", "ghost");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        assert!(!blind_templates().absence_is_final(), "precondition");

        let findings = validate_composition_with(&src, &[], &blind_templates());
        assert!(
            !findings
                .iter()
                .any(|f| f.category == "unresolvable-template"),
            "a host that cannot see the template must not manufacture an error \
             out of its own blindness: {findings:?}"
        );
        assert!(!has_error(&findings), "{findings:?}");
    }

    /// One hull, spelled two ways, is one finding — the same canonicalisation
    /// `two_spellings_of_one_template_are_reported_once` pins for composition.
    #[test]
    fn two_spellings_of_one_absent_template_are_reported_once() {
        let world_toml = format!(
            "{}{}",
            one_entity_world("assets/entities/nowhere.toml", "plain"),
            one_entity_world("./assets/entities/nowhere.toml", "dotted"),
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.category == "unresolvable-template")
                .count(),
            1,
            "{findings:?}"
        );
    }

    /// The gate the Bevy `Startup` spawn consults must carry it too, not only
    /// `validate_composition` — the browser host never calls the latter, and
    /// on native the two immediate-spawn systems are what actually drop the
    /// entity. (Same reasoning as
    /// `activation_findings_carries_the_relative_to_error`.)
    #[test]
    fn activation_findings_carries_the_unresolvable_template_error() {
        let config = cfg(r#"
[[entity]]
template_path = "assets/entities/nowhere.toml"
name = "ghost"
"#);
        let findings = activation_findings(
            &config,
            &crate::entities::include_resolve::HostFragmentSource,
            &patroller_templates(),
        );
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unresolvable-template" && f.is_error()),
            "{findings:?}"
        );

        // …and stays silent through the same gate on a blind host.
        let findings = activation_findings(
            &config,
            &crate::entities::include_resolve::HostFragmentSource,
            &blind_templates(),
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.category == "unresolvable-template"),
            "{findings:?}"
        );
    }

    // ── Override resolution (issue #973 review, F6) ──────────────────────────

    /// A world file for one entity on the patroller hull, carrying `overrides`.
    fn overridden_entity_world(name: &str, overrides: &str) -> String {
        format!(
            "[[entity]]\ntemplate_path = \"assets/entities/patroller.toml\"\n\
             name = \"{name}\"\noverrides = {overrides}\n"
        )
    }

    /// The other half of the silent drop #973 is about, and the half its first
    /// pass missed. `resolve_entity_via` returns `Err` for a failed
    /// `apply_overrides` exactly as it does for a template it cannot find, and
    /// every spawn caller answers `Err` the same way: log and `continue`. So an
    /// `overrides` table that does not merge cost precisely one entity, with
    /// the rest of the world spawning around the hole.
    ///
    /// The `_remove` tombstone is issue #911's case: subtractive at a layer
    /// that does not honour subtraction, so `reject_unhonoured_removals`
    /// refuses it outright rather than letting it look like it worked.
    #[test]
    fn an_override_that_does_not_merge_blocks_activation() {
        let world_toml = overridden_entity_world(
            "ghost",
            "{ behaviour = { doctrine = [{ id = \"patrol\", _remove = true }] } }",
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        let findings = validate_composition_with(&src, &[], &patroller_templates());
        let err = findings
            .iter()
            .find(|f| f.category == "unmergeable-override")
            .unwrap_or_else(|| panic!("expected an unmergeable-override finding: {findings:?}"));
        assert!(err.is_error(), "it must block activation, not warn");
        assert!(has_error(&findings));
        assert!(err.message.contains("ghost"), "{}", err.message);
        assert!(
            err.message.contains("assets/entities/patroller.toml"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("assets/worlds/w.toml"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("_remove"),
            "the merge's own reason has to reach the author: {}",
            err.message
        );
    }

    /// The other failure shape of the same call: the merged document is
    /// well-formed TOML but no longer a valid `EntityConfig`, so the strict
    /// re-parse in `apply_overrides` refuses it.
    #[test]
    fn an_override_that_breaks_the_merged_documents_types_blocks_activation() {
        let world_toml = overridden_entity_world("ghost", "{ tags = \"not-an-array\" }");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unmergeable-override" && f.is_error()),
            "{findings:?}"
        );
    }

    /// **Not** host-gated, unlike the presence half — and this is the test that
    /// says so.
    ///
    /// A host that cannot be authoritative about absence can still be
    /// authoritative about a merge: once the template is in hand, no later
    /// delivery can change the answer, because the template a delivery would
    /// bring is the one already merged against. Gating this on
    /// `absence_is_final` would leave the browser running the exact silent drop
    /// the validator exists to refuse.
    #[test]
    fn an_unmergeable_override_is_reported_even_where_absence_is_not_final() {
        let world_toml = overridden_entity_world(
            "ghost",
            "{ behaviour = { doctrine = [{ id = \"patrol\", _remove = true }] } }",
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        let loader = still_filling(&[("assets/entities/patroller.toml", &patroller_toml())]);
        assert!(!loader.absence_is_final(), "precondition");
        assert!(
            loader
                .load_template("assets/entities/patroller.toml")
                .is_some(),
            "precondition: this host HAS the hull, it is merely still filling"
        );

        let findings = validate_composition_with(&src, &[], &loader);
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unmergeable-override" && f.is_error()),
            "a merge decided entirely from content in hand must be decided: {findings:?}"
        );
        // …while the presence half stays silent on the same host, which is what
        // keeps the browser's world from being blanked by a template in flight.
        let absent = one_entity_world("assets/entities/nowhere.toml", "not-yet");
        let absent_cfg = cfg(&absent);
        let absent_src = WorldSource::new("assets/worlds/w.toml", &absent, &absent_cfg);
        let findings = validate_composition_with(&absent_src, &[], &loader);
        assert!(
            !findings
                .iter()
                .any(|f| f.category == "unresolvable-template"),
            "{findings:?}"
        );
    }

    /// A failed merge is a property of the INSTANCE, not the hull, so two
    /// entries on one template each get their own finding — the deliberate
    /// difference from `unresolvable-template`, which is deduped by canonical
    /// path (`two_spellings_of_one_absent_template_are_reported_once`).
    #[test]
    fn two_instances_of_one_hull_each_report_their_own_broken_override() {
        let world_toml = format!(
            "{}{}",
            overridden_entity_world("first", "{ tags = \"not-an-array\" }"),
            overridden_entity_world("second", "{ tags = 7 }"),
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.category == "unmergeable-override")
                .count(),
            2,
            "one finding per instance, because each carries its own table: {findings:?}"
        );
    }

    /// The Bevy `Startup` gate carries the merge half too, for the same reason
    /// it carries the presence half: the browser never calls
    /// `validate_composition`, and on native the two immediate-spawn systems
    /// are what actually drop the entity.
    #[test]
    fn activation_findings_carries_the_unmergeable_override_error() {
        let config = cfg(&overridden_entity_world(
            "ghost",
            "{ behaviour = { doctrine = [{ id = \"patrol\", _remove = true }] } }",
        ));
        let findings = activation_findings(
            &config,
            &crate::entities::include_resolve::HostFragmentSource,
            &patroller_templates(),
        );
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unmergeable-override" && f.is_error()),
            "{findings:?}"
        );
    }

    // ── Override-absent-table warning (issue #1043) ──────────────────────────

    /// The #1043 foot-gun itself: an override names a top-level table
    /// (`[civilian]`) the resolved template does not declare at all —
    /// `patroller.toml` has no `[civilian]` anywhere in its fixture TOML. The
    /// merge still succeeds (it inserts the table fresh), so this is NOT an
    /// `unmergeable-override`; it is the quieter defect that survives a clean
    /// merge, and is why this check exists at all.
    #[test]
    fn override_targeting_an_absent_template_table_warns() {
        let world_toml = overridden_entity_world("ghost", "{ civilian = { route = \"lane-a\" } }");
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        let findings = validate_composition_with(&src, &[], &patroller_templates());
        let warning = findings
            .iter()
            .find(|f| f.category == "override-absent-table")
            .unwrap_or_else(|| panic!("expected an override-absent-table finding: {findings:?}"));
        assert!(!warning.is_error(), "the foot-gun is silent, not blocking");
        assert!(warning.message.contains("ghost"), "{}", warning.message);
        assert!(warning.message.contains("civilian"), "{}", warning.message);
        assert!(
            warning.message.contains("assets/entities/patroller.toml"),
            "{}",
            warning.message
        );
    }

    /// The other side of the same fixture: `patroller.toml` DOES declare
    /// `[behaviour]` (it is a patrolling hull — see the `PATROLLER` const), so
    /// an override that adds a second doctrine entry merges into a real table
    /// and must not warn.
    ///
    /// Declares `[anchors] route_a / route_b` — the same pair `PATROLLER`'s own
    /// baked-in `patrol-route` doctrine already names — so this world is clean
    /// under `validate_doctrine_anchors_in` too and the "no errors" assertion
    /// below is not contaminated by a pre-existing, unrelated anchor gap in the
    /// fixture.
    #[test]
    fn override_targeting_a_declared_template_table_does_not_warn() {
        let world_toml = format!(
            "{}\n[anchors]\nroute_a = [10.0, 0.0, 20.0]\nroute_b = [30.0, 0.0, 40.0]\n",
            overridden_entity_world(
                "ghost",
                "{ behaviour = { doctrine = [{ id = \"reinforce\", text = \"x\", \
                 directive_kind = \"Reach\", directive_anchor = \"route_a\" }] } }",
            )
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(
            !findings
                .iter()
                .any(|f| f.category == "override-absent-table"),
            "'[behaviour]' is declared by the template, so this must not warn: {findings:?}"
        );
        assert!(
            !findings.iter().any(WorldFinding::is_error),
            "and this override is well-formed, so nothing should block activation either: \
             {findings:?}"
        );
    }

    /// A warning is exactly that: the world still activates. The deliberate
    /// contrast with `an_override_that_does_not_merge_blocks_activation`, which
    /// is the same fixture shape one severity up.
    ///
    /// The override is an EMPTY `[civilian]` table — no `route` — so this stays
    /// clean under `validate_civilian_routes_in` too (an authored route id that
    /// resolves nowhere is its own, separate, unresolved-route error, not this
    /// check's concern). `[anchors] route_a / route_b` is declared for the same
    /// reason as the sibling "does not warn" test: `PATROLLER`'s own baked-in
    /// doctrine names them regardless of what this test overrides.
    #[test]
    fn override_absent_table_warning_does_not_block_activation() {
        let world_toml = format!(
            "{}\n[anchors]\nroute_a = [10.0, 0.0, 20.0]\nroute_b = [30.0, 0.0, 40.0]\n",
            overridden_entity_world("ghost", "{ civilian = {} }")
        );
        let config = cfg(&world_toml);
        let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(
            findings
                .iter()
                .any(|f| f.category == "override-absent-table"),
            "precondition: the warning itself must still be present: {findings:?}"
        );
        assert!(
            !has_error(&findings),
            "a warning must never block activation: {findings:?}"
        );
    }

    /// The Bevy `Startup` gate carries the warning too, for the same reason it
    /// carries every other half of this function's findings: the browser never
    /// calls `validate_composition`.
    #[test]
    fn activation_findings_carries_the_override_absent_table_warning() {
        let config = cfg(&overridden_entity_world(
            "ghost",
            "{ civilian = { route = \"lane-a\" } }",
        ));
        let findings = activation_findings(
            &config,
            &crate::entities::include_resolve::HostFragmentSource,
            &patroller_templates(),
        );
        assert!(
            findings
                .iter()
                .any(|f| f.category == "override-absent-table" && !f.is_error()),
            "{findings:?}"
        );
    }

    /// `probe_evidence.toml` and `probe_corroborate.toml` author a
    /// `comms_console.ai.rule` override — live today via `player_hull_config`
    /// (`src/server_app.rs`) — that must merge onto a table already declared by
    /// the shared AI fragment library, not an absent one (issue #1036/#1043). A
    /// `override-absent-table` warning for either world means that fix
    /// regressed. The general `override-absent-table` case (an authoring slip
    /// shaped like the hauler `[behaviour]` #1043 found by hand) is surfaced to
    /// the world author at load time by `validate_template_resolution_in`
    /// itself, not pinned here.
    #[test]
    fn evidence_and_corroborate_overrides_never_warn_override_absent_table() {
        let loader = crate::entities::loader::WasmTemplateLoader;
        for name in ["probe_evidence.toml", "probe_corroborate.toml"] {
            let path_str = format!("assets/worlds/{name}");
            let toml = std::fs::read_to_string(&path_str).expect("shipped world readable");
            let config = parse_world(&toml).expect("shipped world parses");
            let src = WorldSource::new(&path_str, &toml, &config);
            let mut seen = HashSet::new();
            let findings = validate_template_resolution_in(&src, &loader, &mut seen);
            assert!(
                !findings
                    .iter()
                    .any(|f| f.category == "override-absent-table"),
                "{path_str}'s `comms_console.ai.rule` override must merge onto a declared \
                 table (issue #1036/#1043) — a warning here means the player-ship fix regressed: \
                 {findings:?}"
            );
        }
    }
}
