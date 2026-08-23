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

use std::borrow::Cow;
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
/// Covers every spawn path — a static `[[entity]]` block, a `spawn_entity`
/// trigger/comms action, and a `ctx.effects.spawn_entity` call inside a script
/// (issue #1046) — because they all hand the same hull the same doctrine.
struct SpawnedInstance<'a> {
    /// Best-effort identity for diagnostics: the authored `name`, else the
    /// authored `id`, else the template path.
    label: String,
    /// Borrowed for a declarative instance; owned for a script-scanned one,
    /// which reads a literal out of a script body and has nothing to borrow.
    template_path: Cow<'a, str>,
    /// The instance `overrides` table, when this validator can see one.
    ///
    /// Always `None` for a script-authored spawn — see
    /// [`collect_spawned_instances`].
    overrides: Option<&'a toml::Value>,
    /// This spawn passes an `overrides` map the validator cannot READ (issue
    /// #1046). Always false for a declarative instance, whose table is TOML and
    /// is merged for real.
    ///
    /// Judging the template's own content is then a statement about a hull that
    /// may not be the hull that spawns, so a check which would otherwise error
    /// softens to a warning. `overrides: None` beside `unreadable_overrides:
    /// true` is the honest pair: nothing to merge, and a reason the merged
    /// result is unknowable.
    unreadable_overrides: bool,
}

/// Every entity instance a world config spawns: static `[[entity]]` blocks,
/// then the `spawn_entity` actions of every authored action list, then every
/// literal `spawn_entity` its inline scripts name (issue #1046).
///
/// # How a validator sees inside a script, and what it cannot see
///
/// The third source is a STATIC scan
/// ([`crate::world::config::script_spawned_templates`]) for literal
/// `template_path: "…"` entries, and the choice of mechanism was made for it
/// twice already: `validate_flag_opassign` (issue #994) and
/// `validate_on_pick_fns` (issue #984) both wanted to read inside script bodies,
/// and both settled on a lexical scan because a true `AST::walk` over
/// `Stmt`/`Expr` needs Rhai's `internals` feature, which this build does not
/// enable, and `vellum_script` exposes no walk helper. A handler body never runs
/// at load either — `Engine::run_ast` executes only a unit's top level — so what
/// the source says literally is the whole of what is knowable here.
///
/// A COMPUTED path is therefore invisible, and stays legal. `duel.toml` ships
/// one: its single authored `spawn_slot(ctx, name, template, …)` body passes
/// `template_path: template`, because the hull comes from `--side-a`/`--side-b`
/// at run time and cannot be written down in the world. A gate that demanded a
/// literal, or a declaration of every path a script might reach for, would
/// either refuse that world or force it to declare what it does not know. The
/// asymmetry the sibling passes chose applies unchanged: for a LOAD-TIME gate a
/// false positive that blocks a legitimate world is strictly worse than a missed
/// catch. The backstop is at spawn time, and issue #1046 made it loud —
/// `dispatch_spawn_entity` reports an unresolvable template on
/// `DispatchResult::override_failures` (ERROR) rather than the warn-level
/// channel it shared with routine misses.
///
/// # Overrides, and the finding that has to soften because of them
///
/// A script instance carries `overrides: None`, because its map is Rhai built at
/// CALL time out of values a load-time pass has not got. For
/// [`validate_template_resolution_in`] that is simply correct: there is no table
/// to merge, so the merge half does not run and the path half — which is a
/// literal or nothing — runs in full.
///
/// [`doctrine_anchor_refs`] is the hard case, and the first cut of this change
/// got it wrong in a way worth recording. It judges the TEMPLATE's own doctrine,
/// on the reasoning that an override cannot REMOVE what the template authored
/// (`behaviour.doctrine` reconciles by id), so every anchor the template names
/// survives the merge. The entry survives; the ANCHOR REFERENCE does not have
/// to. An override may restate an entry by id with `directive_kind = "None"`,
/// and a directive of kind None reads no anchors at all — which is not a corner
/// case but the shipped idiom, documented at length in both worlds that use it:
///
/// * `combat_test.toml`'s wave 8 spawns `ship_harrow_patrol.toml`, whose
///   `patrol-ironveil` entry names `ironveil_patrol_a`/`_b`; the picket line
///   those anchors belonged to was deleted in #960 and the spawn stands the
///   entry down rather than declaring a route nothing flies.
/// * `probe_artillery_standoff.toml` does the same to `patrol-warhawk`.
///
/// Judged on the template alone, both come out as unresolved anchors, and both
/// would have been blocked by an ERROR — including the one selectable scenario.
/// So the severity follows what can be SEEN: the scan reports whether the spawn
/// map passes an `overrides` key at all
/// ([`crate::world::config::ScriptSpawnRef::overrides_passed`]), and
/// [`validate_doctrine_anchors_in`] errors only when it does not. With no
/// override the template's doctrine IS the effective doctrine and the check is
/// exactly as sound as for a declarative instance; with one, the same finding is
/// a warning that names what it cannot see.
///
/// What stays invisible either way: an anchor an override *introduces*, and a
/// `directive_target` it retargets. That is the script surface's share of the
/// "computed is invisible" bargain above, and it fails at runtime as it always
/// did.
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
            template_path: Cow::Borrowed(e.template_path.as_str()),
            overrides: e.overrides.as_ref(),
            unreadable_overrides: false,
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
                    template_path: Cow::Borrowed(template_path.as_str()),
                    overrides: overrides.as_ref(),
                    unreadable_overrides: false,
                });
            }
        }
    }

    for spawn in crate::world::config::script_spawned_templates(config) {
        out.push(SpawnedInstance {
            // No `name` to reach for: the scan sees one map entry, not the map.
            // The template path is the identity a finding can offer, and it is
            // also what `line_of` locates — an inline `[script]` body IS a slice
            // of the world TOML, so the spawn site is findable in the source the
            // finding names.
            label: spawn.template_path.clone(),
            template_path: Cow::Owned(spawn.template_path),
            overrides: None,
            unreadable_overrides: spawn.overrides_passed,
        });
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
    let Some(template) = loader.load_template(&inst.template_path) else {
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
            template_path: Cow::Borrowed(template_path),
            overrides: None,
            unreadable_overrides: false,
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
            // A spawn whose `overrides` map this validator cannot read is a
            // statement about the TEMPLATE, not about the hull that arrives
            // (issue #1046). Standing the offending entry down in that map is
            // the shipped answer to this very finding — `combat_test.toml`'s
            // `wave_8_overrides()` and `probe_artillery_standoff.toml`'s literal
            // both do it, and both say so at length — so erroring here would
            // block two working worlds for content that is already correct.
            // Reported all the same, because the other half of the time it is a
            // real unresolved anchor and nothing else will say so.
            let (severity, message) = if inst.unreadable_overrides {
                (
                    Severity::Warning,
                    format!(
                        "script spawn of template '{}' has a {kind} doctrine directive \
                         referencing anchor '{anchor}', which no world in the composition \
                         declares, in '{}'. The spawn passes an `overrides` map this \
                         validator cannot read, so the entry may already be stood down \
                         there (`directive_kind = \"None\"` and every field its old kind \
                         owned) — if it is not, this anchor resolves to nothing on every \
                         tick and the hull silently never pursues it",
                        inst.template_path, src.path
                    ),
                )
            } else {
                (
                    Severity::Error,
                    format!(
                        "entity '{}' (template '{}') has a {kind} doctrine directive \
                         referencing anchor '{anchor}', which no world in the composition \
                         declares, in '{}'",
                        inst.label, inst.template_path, src.path
                    ),
                )
            };
            findings.push(WorldFinding {
                severity,
                category: "unresolved-anchor",
                message,
                source: SourceLocation {
                    file: src.path.clone(),
                    // The anchor name is absent from this world by definition,
                    // so point at the spawn site instead.
                    line: line_of(src.toml, &inst.label)
                        .or_else(|| line_of(src.toml, &inst.template_path)),
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
    let template = loader.load_template(&inst.template_path)?;
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
                    .or_else(|| line_of(src.toml, &inst.template_path)),
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
            &inst.template_path,
        )) {
            continue;
        }
        let Some(mut finding) =
            crate::entities::include_resolve::composition_finding(&inst.template_path, fragments)
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
        let Some(template) = loader.load_template(&inst.template_path) else {
            if !absence_is_final {
                continue;
            }
            if !seen.insert(crate::entities::include_resolve::canonical_template_path(
                &inst.template_path,
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
                    line: line_of(src.toml, &inst.template_path)
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
                            .or_else(|| line_of(src.toml, &inst.template_path)),
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
                    .or_else(|| line_of(src.toml, &inst.template_path)),
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
#[path = "validate_tests.rs"]
mod tests;
