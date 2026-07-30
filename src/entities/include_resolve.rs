// Composable entity templates (issue #869).
//
// Pure module: no I/O, no Bevy, no thread-locals in the resolver itself. It
// turns one entity template plus its ordered `includes` into ONE final TOML
// document, and hands back **provenance** saying which fragment authored each
// field and through which include chain.
//
// # Where this sits
//
// ```text
//   fragment TOML ─┐
//   fragment TOML ─┼─ resolve_template ─→ one resolved TOML ─→ EntityConfig::from_toml
//   hull TOML ─────┘   (once per TEMPLATE)                         │
//                                                                  ▼
//                                    entity_loader::apply_overrides (once per INSTANCE)
// ```
//
// ## Why include resolution is UPSTREAM of `resolve_entity`, not inside it
//
// The PRD left this open. It stays upstream, for three reasons that are not
// preference:
//
// 1. **`EntityConfig` is `deny_unknown_fields`.** An `includes` key cannot
//    survive into a parsed config, so folding resolution into
//    `entity_loader::apply_overrides` — which starts from an already-parsed
//    `EntityConfig` — would require adding an `includes` field to
//    `EntityConfig` purely to carry authoring metadata into the runtime, then
//    scrubbing it again. Includes would then literally exist at runtime, which
//    is the one thing #869 forbids. Resolution here operates on raw TOML text
//    and the key never reaches the struct.
// 2. **Cardinality.** Includes resolve once per *template* (cached); overrides
//    apply once per *instance*. `apply_overrides` pays a
//    `EntityConfig → toml::Value → merge → String → EntityConfig` round trip
//    because its input is already a struct. Include resolution starts from
//    text, so it merges as `toml::Value` and parses **exactly once**, at the
//    end. Conflating them would multiply that round trip by every spawned
//    instance for no gain.
// 3. **`67c31b9e` already split the two concerns** — `apply_overrides` came out
//    of `resolve_entity` so that the instance merge could be reused on its own.
//    Pushing a second, per-template concern back in would re-fuse them.
//
// Both layers share the SAME merge (`entity_override::merge_entity_config_toml`);
// only the cardinality and the input form differ.
//
// ## Merge order
//
// Depth-first, in declared order. Each fragment is merged into the accumulator,
// and the declaring template is merged **last**, so the includer always wins.
// A template that includes `[a, b]` resolves as
// `((a's own closure) ⊕ (b's own closure)) ⊕ self`.
//
// The merge is `merge_entity_config_toml`, so every rule it documents holds
// between fragments too, including the one added by `68bda1be`: a fragment that
// authors `doctrine = []` **clears** whatever earlier fragments contributed
// (that is a fragment's only subtractive lever), while a fragment that omits
// the key leaves the accumulator alone, and a fragment that authors a non-empty
// `doctrine` merges by `id` into what came before.
//
// One authoring consequence worth stating plainly, because it is the trap
// #878's migration will meet first: every array OTHER than those two replaces
// wholesale — `tags`, `[[station]]`, `[[system]]`, `[[shield_arc]]`,
// `[[weapons_console.phaser_banks]]`, and the rest. A hull that includes a
// systems fragment and then declares its own `[[system]]` blocks replaces the
// fragment's whole suite rather than adding to it. That is `merge_toml`'s
// long-standing array rule, shared with instance overrides; widening it would
// change override semantics too, and is deliberately not in this issue's scope.
//
// ## Paths
//
// Include paths are resolved **relative to the declaring template** and
// lexically canonicalised (`\` → `/`, `.` and `..` collapsed). Canonicalisation
// is lexical rather than `std::fs::canonicalize` on purpose: it must produce the
// same key on WASM, where there is no filesystem, as it does natively, and the
// keys double as config-cache keys and as the cycle-detection identity.

use std::collections::BTreeMap;

use crate::entity_config::EntityConfig;
use crate::world::validate::{line_of, Severity, SourceLocation, WorldFinding};

/// The authored key that lists a template's ordered includes.
pub const INCLUDES_KEY: &str = "includes";

/// `behaviour.state` reconciles by `name` in the merge, so provenance addresses
/// its elements by name rather than by index.
const STATE_ARRAY: &str = "behaviour.state";
/// `behaviour.doctrine` reconciles by `id` in the merge, so provenance
/// addresses its elements by id rather than by index.
const DOCTRINE_ARRAY: &str = "behaviour.doctrine";

// ── Source of template text ──────────────────────────────────────────────────

/// Where the resolver gets template text from, keyed by canonical path.
///
/// Pure by contract — the resolver never touches a filesystem or a config cache
/// itself. Object-safe (`&self`, no generics) so callers hold a
/// `&dyn FragmentSource`, exactly like [`crate::entity_loader::TemplateLoader`].
///
/// A `None` means "not available *yet*" as much as "does not exist"; which of
/// those it is depends on the caller's [`MissingPolicy`].
pub trait FragmentSource {
    /// The raw TOML text at `path`, or `None` when it cannot be served.
    fn read(&self, path: &str) -> Option<String>;
}

impl FragmentSource for std::collections::HashMap<String, String> {
    fn read(&self, path: &str) -> Option<String> {
        self.get(path).cloned()
    }
}

impl FragmentSource for BTreeMap<String, String> {
    fn read(&self, path: &str) -> Option<String> {
        self.get(path).cloned()
    }
}

/// Filesystem adapter for the pure resolver above.
///
/// The one I/O-touching item in this file, mirroring how
/// `entity_loader::FsTemplateLoader` sits beside the pure resolution in
/// `loader.rs`. The mod-pack overlay is consulted FIRST so an uploaded pack's
/// fragment wins over the shipped one, matching how every other content channel
/// resolves an authored path (issue #760 AC2, #869 US7).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct FsFragmentSource;

#[cfg(not(target_arch = "wasm32"))]
impl FragmentSource for FsFragmentSource {
    fn read(&self, path: &str) -> Option<String> {
        crate::config_cache::mod_pack_overlay_get(path)
            .or_else(|| std::fs::read_to_string(path).ok())
    }
}

// ── Errors ───────────────────────────────────────────────────────────────────

/// A composition failure, carrying the include chain that reached it.
///
/// Reuses `world::validate`'s finding vocabulary — [`Severity`],
/// [`SourceLocation`], the kebab-case `category` slug — so composition errors
/// read like every other content error. Every one of these is an ERROR: cycles,
/// missing fragments, unparseable fragments, malformed `includes` declarations
/// and invalid resolved templates all block the load. There is no warning
/// severity here by design; a partially composed entity must never spawn.
///
/// Boxed: composition sits on the `Result` of every load path, and a fat
/// `Err` variant would cost every successful load too (`clippy::result_large_err`).
/// [`Deref`](std::ops::Deref) keeps `err.chain` / `err.finding` reading as
/// plain fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludeError(Box<IncludeErrorDetail>);

/// The body of an [`IncludeError`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludeErrorDetail {
    /// Root template first, offending template last.
    pub chain: Vec<String>,
    /// Source-located finding, always [`Severity::Error`].
    pub finding: WorldFinding,
}

impl std::ops::Deref for IncludeError {
    type Target = IncludeErrorDetail;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IncludeError {
    /// Kebab-case category slug: `include-cycle`, `include-missing`,
    /// `include-parse`, `include-malformed`, `include-invalid-template`.
    pub fn category(&self) -> &'static str {
        self.finding.category
    }

    /// The human-readable explanation, without the chain suffix.
    pub fn message(&self) -> &str {
        &self.finding.message
    }

    /// `a.toml -> b.toml -> c.toml`
    pub fn chain_display(&self) -> String {
        self.chain.join(" -> ")
    }

    fn new(
        category: &'static str,
        chain: Vec<String>,
        source: SourceLocation,
        message: String,
    ) -> Self {
        IncludeError(Box::new(IncludeErrorDetail {
            chain,
            finding: WorldFinding {
                severity: Severity::Error,
                category,
                message,
                source,
            },
        }))
    }
}

impl std::fmt::Display for IncludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.finding.category, self.finding.message)?;
        write!(f, " [include chain: {}]", self.chain_display())?;
        match self.finding.source.line {
            Some(line) => write!(f, " (at {}:{line})", self.finding.source.file),
            None => write!(f, " (in {})", self.finding.source.file),
        }
    }
}

impl std::error::Error for IncludeError {}

// ── Provenance ───────────────────────────────────────────────────────────────

/// One template's contribution to the resolved document, in merge order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeStep {
    /// Canonical path of the contributing template.
    pub source: String,
    /// Include chain from the root template down to `source`, inclusive.
    pub chain: Vec<String>,
}

/// Which template last authored a given field, and how the resolver got there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldOrigin {
    /// Canonical path of the template that authored this field last — i.e. the
    /// one that won.
    pub source: String,
    /// Include chain from the root template down to `source`, inclusive.
    pub chain: Vec<String>,
}

/// Which include contributed each field of a resolved template, and the chain
/// that reached it.
///
/// This is the piece #869 calls out as worth building for later extraction to
/// the fleet: `vellum-compose` has composition but no provenance. It is kept
/// deliberately free of phoenix concepts — a field is a dotted path string, a
/// source is a path string — so lifting it needs no phoenix types.
///
/// # Field addressing
///
/// Leaf paths are dotted (`hull.hull_integrity`). The two arrays the merge
/// reconciles by key are addressed by that key rather than by index, because
/// their positions are not stable across a merge:
///
/// * `behaviour.state[name=patrol].target_speed`
/// * `behaviour.doctrine[id=destroy-hostiles].base_priority`
///
/// Every other array is a merge leaf (arrays replace wholesale), so it is
/// recorded at its own path with no element addressing — `tags`, and a cleared
/// `behaviour.doctrine` (`= []`) alike.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    order: Vec<MergeStep>,
    fields: BTreeMap<String, FieldOrigin>,
}

impl Provenance {
    /// Every contributing template in the order it was merged: fragments first
    /// (depth-first, declared order), the declaring template last.
    pub fn merge_order(&self) -> &[MergeStep] {
        &self.order
    }

    /// Canonical paths of the contributing templates, in merge order.
    pub fn sources(&self) -> Vec<&str> {
        self.order.iter().map(|s| s.source.as_str()).collect()
    }

    /// Who authored `field` in the resolved document, or `None` when no
    /// template set it.
    pub fn origin(&self, field: &str) -> Option<&FieldOrigin> {
        self.fields.get(field)
    }

    /// Every recorded field path with its origin, in sorted path order.
    pub fn fields(&self) -> impl Iterator<Item = (&String, &FieldOrigin)> {
        self.fields.iter()
    }

    /// How many distinct leaf fields the resolved document carries.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Record everything `value` authored, attributing it to `step`.
    fn record(&mut self, value: &toml::Value, step: MergeStep) {
        record_leaves("", value, &step, &mut self.fields);
        self.order.push(step);
    }
}

fn record_leaves(
    prefix: &str,
    value: &toml::Value,
    step: &MergeStep,
    out: &mut BTreeMap<String, FieldOrigin>,
) {
    let origin = || FieldOrigin {
        source: step.source.clone(),
        chain: step.chain.clone(),
    };
    match value {
        toml::Value::Table(table) => {
            if table.is_empty() {
                if !prefix.is_empty() {
                    insert_leaf(prefix, origin(), out);
                }
                return;
            }
            // A table at this path supersedes any scalar previously recorded
            // there, but its children are merged individually so they are NOT
            // pruned.
            out.remove(prefix);
            for (key, child) in table {
                let path = join_field(prefix, key);
                record_leaves(&path, child, step, out);
            }
        }
        toml::Value::Array(items) => {
            let element_key = match prefix {
                STATE_ARRAY => Some("name"),
                DOCTRINE_ARRAY => Some("id"),
                _ => None,
            };
            match element_key {
                // A keyed array reconciles element-by-element, so record the
                // elements and leave siblings from earlier fragments intact.
                Some(key) if !items.is_empty() => {
                    out.remove(prefix);
                    for (index, element) in items.iter().enumerate() {
                        let addressed = element
                            .get(key)
                            .and_then(|v| v.as_str())
                            .map(|id| format!("{prefix}[{key}={id}]"))
                            .unwrap_or_else(|| format!("{prefix}[{index}]"));
                        record_leaves(&addressed, element, step, out);
                    }
                }
                // Everything else replaces wholesale — including an authored
                // empty array, which is how a fragment CLEARS a list.
                _ => insert_leaf(prefix, origin(), out),
            }
        }
        _ => insert_leaf(prefix, origin(), out),
    }
}

/// Record a leaf, dropping anything previously recorded beneath it — the value
/// at `prefix` replaced that subtree wholesale.
fn insert_leaf(prefix: &str, origin: FieldOrigin, out: &mut BTreeMap<String, FieldOrigin>) {
    out.retain(|key, _| !is_descendant(key, prefix));
    out.insert(prefix.to_string(), origin);
}

fn is_descendant(key: &str, prefix: &str) -> bool {
    key.len() > prefix.len()
        && key.starts_with(prefix)
        && matches!(key.as_bytes()[prefix.len()], b'.' | b'[')
}

fn join_field(prefix: &str, key: &str) -> String {
    let key = if key.contains(['.', '[', ']', '=', ' ']) {
        format!("\"{key}\"")
    } else {
        key.to_string()
    };
    if prefix.is_empty() {
        key
    } else {
        format!("{prefix}.{key}")
    }
}

// ── Resolved output ──────────────────────────────────────────────────────────

/// One entity template with its include closure merged in.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTemplate {
    /// Canonical path of the declaring (root) template.
    pub path: String,
    /// The resolved document as TOML text — the sole input to
    /// `EntityConfig::from_toml`. Byte-stable: the same inputs always render
    /// the same bytes, regardless of the order the fragments were delivered in.
    ///
    /// A template that declares no includes anywhere in its closure yields its
    /// **original text verbatim**, so nothing about an uncomposed template
    /// changes — including the raw-text line lookups that marker validation and
    /// `line_of` do.
    pub toml: String,
    /// The resolved document as a value, with every `includes` key removed.
    pub value: toml::Value,
    /// Which template authored each field, and through which chain.
    pub provenance: Provenance,
}

impl ResolvedTemplate {
    /// True when at least one fragment contributed — i.e. the template declares
    /// includes, or something it includes does.
    pub fn is_composed(&self) -> bool {
        self.provenance.order.len() > 1
    }

    /// Parse the resolved document. Only the fully resolved template is ever
    /// validated; a parse failure names the whole include chain, because the
    /// offending combination may exist in no single authored file.
    pub fn parse(&self) -> Result<EntityConfig, IncludeError> {
        EntityConfig::from_toml(&self.toml).map_err(|e| {
            IncludeError::new(
                "include-invalid-template",
                self.provenance
                    .sources()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                SourceLocation {
                    file: self.path.clone(),
                    line: None,
                    reference: self.path.clone(),
                },
                format!("resolved template is not a valid entity: {e}"),
            )
        })
    }
}

/// What a preload host should do next with one template.
#[derive(Clone, Debug, PartialEq)]
pub enum PreloadStep {
    /// Every include is available; the template is fully resolved.
    Ready(Box<ResolvedTemplate>),
    /// These canonical paths must be fetched before the template can resolve.
    /// Deduplicated, in first-encountered order.
    AwaitingIncludes(Vec<String>),
}

/// How the resolver treats a fragment the source cannot serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MissingPolicy {
    /// A fragment that cannot be read is a load error (native, where the
    /// filesystem is authoritative).
    Fail,
    /// A fragment that cannot be read is not an error yet — collect it as
    /// something still to fetch (the browser preload, where fragments arrive
    /// asynchronously).
    Collect,
}

// ── Path canonicalisation ────────────────────────────────────────────────────

/// Canonicalise a template path for use as a cache key, a diagnostic, and a
/// cycle-detection identity: `\` → `/`, `.` dropped, `..` collapsed.
///
/// Lexical on purpose — see the module header.
pub fn canonical_template_path(path: &str) -> String {
    normalise_segments(&path.replace('\\', "/"))
}

/// Resolve an authored include reference against the template that declared it.
///
/// Returns `None` for a reference that is not resolvable relative to the
/// declarer: empty, root-absolute (`/...`), or drive-absolute (`C:\...`).
pub fn canonical_include_path(declaring_path: &str, include: &str) -> Option<String> {
    let include = include.trim().replace('\\', "/");
    if include.is_empty() || include.starts_with('/') {
        return None;
    }
    // `C:/...` — an absolute path on Windows.
    let bytes = include.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return None;
    }
    let declaring = declaring_path.replace('\\', "/");
    let dir = match declaring.rfind('/') {
        Some(i) => &declaring[..i],
        None => "",
    };
    let joined = if dir.is_empty() {
        include
    } else {
        format!("{dir}/{include}")
    };
    Some(normalise_segments(&joined))
}

fn normalise_segments(path: &str) -> String {
    let leading_slash = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => match out.last() {
                Some(&last) if last != ".." => {
                    out.pop();
                }
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    let joined = out.join("/");
    if leading_slash {
        format!("/{joined}")
    } else {
        joined
    }
}

// ── The resolver ─────────────────────────────────────────────────────────────

/// Resolve `root_path` and its whole include closure into one document.
///
/// Every include must be readable; a fragment the source cannot serve is a load
/// error, as are cycles, unparseable fragments and malformed `includes`
/// declarations. Nothing is validated here beyond TOML well-formedness — call
/// [`ResolvedTemplate::parse`] to validate the composed result.
pub fn resolve_template(
    root_path: &str,
    source: &dyn FragmentSource,
) -> Result<ResolvedTemplate, IncludeError> {
    match resolve_with(root_path, source, MissingPolicy::Fail)? {
        PreloadStep::Ready(resolved) => Ok(*resolved),
        // Unreachable under `Fail`: a missing fragment has already errored.
        PreloadStep::AwaitingIncludes(missing) => Err(IncludeError::new(
            "include-missing",
            vec![canonical_template_path(root_path)],
            SourceLocation {
                file: canonical_template_path(root_path),
                line: None,
                reference: missing.first().cloned().unwrap_or_default(),
            },
            format!("unresolved include(s): {}", missing.join(", ")),
        )),
    }
}

/// The preload contract: resolve as far as the delivered fragments allow, and
/// otherwise report exactly which paths the host still has to fetch.
///
/// This is the one entry point both hosts share. The browser calls it as each
/// fetched template arrives; native tests call it to assert the same closure
/// walk without a browser. Cycles and malformed declarations still fail here —
/// only *absence* is treated as "not yet".
pub fn preload_step(
    root_path: &str,
    source: &dyn FragmentSource,
) -> Result<PreloadStep, IncludeError> {
    resolve_with(root_path, source, MissingPolicy::Collect)
}

struct Ctx<'a> {
    source: &'a dyn FragmentSource,
    policy: MissingPolicy,
    stack: Vec<String>,
    accumulator: Option<toml::Value>,
    provenance: Provenance,
    missing: Vec<String>,
    /// Text of the root template, kept verbatim so an uncomposed template
    /// renders byte-identically to what the author wrote.
    root_text: String,
}

fn resolve_with(
    root_path: &str,
    source: &dyn FragmentSource,
    policy: MissingPolicy,
) -> Result<PreloadStep, IncludeError> {
    let root = canonical_template_path(root_path);
    let mut ctx = Ctx {
        source,
        policy,
        stack: Vec::new(),
        accumulator: None,
        provenance: Provenance::default(),
        missing: Vec::new(),
        root_text: String::new(),
    };
    visit(&mut ctx, &root, None, true)?;

    if !ctx.missing.is_empty() {
        return Ok(PreloadStep::AwaitingIncludes(ctx.missing));
    }

    let value = ctx
        .accumulator
        .unwrap_or_else(|| toml::Value::Table(toml::value::Table::new()));
    let composed = ctx.provenance.order.len() > 1;
    let toml_text = if composed {
        toml::to_string(&value).map_err(|e| {
            IncludeError::new(
                "include-invalid-template",
                ctx.provenance
                    .sources()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                SourceLocation {
                    file: root.clone(),
                    line: None,
                    reference: root.clone(),
                },
                format!("resolved template could not be re-serialised: {e}"),
            )
        })?
    } else {
        ctx.root_text
    };

    Ok(PreloadStep::Ready(Box::new(ResolvedTemplate {
        path: root,
        toml: toml_text,
        value,
        provenance: ctx.provenance,
    })))
}

/// Where an include was authored, for source-located diagnostics.
struct Decl<'a> {
    file: &'a str,
    text: &'a str,
    reference: &'a str,
}

fn visit(
    ctx: &mut Ctx<'_>,
    path: &str,
    decl: Option<Decl<'_>>,
    is_root: bool,
) -> Result<(), IncludeError> {
    if ctx.stack.iter().any(|p| p == path) {
        let mut chain = ctx.stack.clone();
        chain.push(path.to_string());
        return Err(IncludeError::new(
            "include-cycle",
            chain.clone(),
            source_location(&decl, path),
            format!(
                "include cycle: {path} is already being resolved further up the chain \
                 ({})",
                chain.join(" -> ")
            ),
        ));
    }

    let Some(text) = ctx.source.read(path) else {
        if ctx.policy == MissingPolicy::Collect && !is_root {
            if !ctx.missing.iter().any(|p| p == path) {
                ctx.missing.push(path.to_string());
            }
            return Ok(());
        }
        let mut chain = ctx.stack.clone();
        chain.push(path.to_string());
        return Err(IncludeError::new(
            "include-missing",
            chain,
            source_location(&decl, path),
            format!("included template not found: {path}"),
        ));
    };

    let mut value: toml::Value = toml::from_str(&text).map_err(|e| {
        let mut chain = ctx.stack.clone();
        chain.push(path.to_string());
        IncludeError::new(
            "include-parse",
            chain,
            SourceLocation {
                file: path.to_string(),
                line: None,
                reference: path.to_string(),
            },
            format!("template is not valid TOML: {e}"),
        )
    })?;

    if is_root {
        ctx.root_text = text.clone();
    }

    let includes = take_includes(ctx, &mut value, path, &text)?;

    ctx.stack.push(path.to_string());
    for reference in &includes {
        let Some(child) = canonical_include_path(path, reference) else {
            let mut chain = ctx.stack.clone();
            ctx.stack.pop();
            chain.push(reference.clone());
            return Err(IncludeError::new(
                "include-malformed",
                chain,
                SourceLocation {
                    file: path.to_string(),
                    line: line_of(&text, reference),
                    reference: reference.clone(),
                },
                format!(
                    "include {reference:?} is not resolvable relative to {path} — \
                     include paths are relative to the declaring template and must \
                     not be absolute"
                ),
            ));
        };
        let declared = Decl {
            file: path,
            text: &text,
            reference,
        };
        if let Err(e) = visit(ctx, &child, Some(declared), false) {
            ctx.stack.pop();
            return Err(e);
        }
    }

    // The declaring template merges LAST, so the includer always wins.
    let step = MergeStep {
        source: path.to_string(),
        chain: ctx.stack.clone(),
    };
    ctx.accumulator = Some(match ctx.accumulator.take() {
        None => value.clone(),
        Some(accumulated) => crate::entity_override::merge_entity_config_toml(&accumulated, &value),
    });
    ctx.provenance.record(&value, step);
    ctx.stack.pop();
    Ok(())
}

fn take_includes(
    ctx: &Ctx<'_>,
    value: &mut toml::Value,
    path: &str,
    text: &str,
) -> Result<Vec<String>, IncludeError> {
    let malformed = |message: String, reference: &str| {
        let mut chain = ctx.stack.clone();
        chain.push(path.to_string());
        IncludeError::new(
            "include-malformed",
            chain,
            SourceLocation {
                file: path.to_string(),
                line: line_of(text, INCLUDES_KEY),
                reference: reference.to_string(),
            },
            message,
        )
    };

    let Some(table) = value.as_table_mut() else {
        return Ok(Vec::new());
    };
    match table.remove(INCLUDES_KEY) {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in &items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(malformed(
                            format!(
                                "`{INCLUDES_KEY}` must be an array of template paths; \
                                 found a non-string entry"
                            ),
                            INCLUDES_KEY,
                        ));
                    }
                }
            }
            Ok(out)
        }
        Some(_) => Err(malformed(
            format!("`{INCLUDES_KEY}` must be an array of template paths"),
            INCLUDES_KEY,
        )),
    }
}

fn source_location(decl: &Option<Decl<'_>>, path: &str) -> SourceLocation {
    match decl {
        Some(d) => SourceLocation {
            file: d.file.to_string(),
            line: line_of(d.text, d.reference),
            reference: d.reference.to_string(),
        },
        None => SourceLocation {
            file: path.to_string(),
            line: None,
            reference: path.to_string(),
        },
    }
}

// ── Native convenience ───────────────────────────────────────────────────────

/// Resolve a template off disk (mod-pack overlay first). Native only — the
/// browser resolves out of its preload cache via [`preload_step`].
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_from_disk(path: &str) -> Result<ResolvedTemplate, IncludeError> {
    resolve_template(path, &FsFragmentSource)
}

/// Resolve a template off disk and parse it. The single native entry point for
/// "give me the `EntityConfig` this path denotes", include closure and all.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_entity_config(path: &str) -> Result<EntityConfig, IncludeError> {
    resolve_from_disk(path)?.parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn src(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn resolve(root: &str, pairs: &[(&str, &str)]) -> ResolvedTemplate {
        resolve_template(root, &src(pairs)).expect("fixture must resolve")
    }

    // ── Ordered precedence ───────────────────────────────────────────────────

    #[test]
    fn includer_wins_over_its_fragment() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    "class = \"escort\"\n[hull]\nhull_integrity = 100.0\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\n[hull]\nhull_integrity = 500.0\n",
                ),
            ],
        );
        assert_eq!(
            r.value
                .get("hull")
                .unwrap()
                .get("hull_integrity")
                .unwrap()
                .as_float(),
            Some(500.0),
            "the declaring template merges last, so it wins"
        );
        assert_eq!(
            r.value.get("class").unwrap().as_str(),
            Some("escort"),
            "a field only the fragment sets survives"
        );
    }

    #[test]
    fn later_include_wins_over_earlier_include() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/a.toml", "class = \"a\"\nhull_id = \"from-a\"\n"),
                ("e/b.toml", "class = \"b\"\n"),
                ("e/hull.toml", "includes = [\"a.toml\", \"b.toml\"]\n"),
            ],
        );
        assert_eq!(
            r.value.get("class").unwrap().as_str(),
            Some("b"),
            "includes merge in declared order, so the later one wins"
        );
        assert_eq!(
            r.value.get("hull_id").unwrap().as_str(),
            Some("from-a"),
            "a field the later include does not mention keeps the earlier value"
        );
    }

    /// The mutation guard for precedence: if the includer were merged FIRST
    /// (or the include list were walked in reverse), one of these two
    /// assertions has to break. They pin opposite ends of the order.
    #[test]
    fn precedence_order_is_fragments_then_declarer() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/a.toml",
                    "class = \"a\"\nhull_id = \"a\"\npower_rating = 1\n",
                ),
                ("e/b.toml", "class = \"b\"\nhull_id = \"b\"\n"),
                (
                    "e/hull.toml",
                    "includes = [\"a.toml\", \"b.toml\"]\nclass = \"self\"\n",
                ),
            ],
        );
        assert_eq!(r.value.get("class").unwrap().as_str(), Some("self"));
        assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("b"));
        assert_eq!(r.value.get("power_rating").unwrap().as_integer(), Some(1));
        assert_eq!(
            r.provenance.sources(),
            vec!["e/a.toml", "e/b.toml", "e/hull.toml"],
            "merge order must be depth-first in declared order, declarer last"
        );
    }

    // ── Nested includes ──────────────────────────────────────────────────────

    #[test]
    fn nested_includes_resolve_depth_first() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/deep.toml", "class = \"deep\"\nhull_id = \"deep\"\n"),
                (
                    "e/mid.toml",
                    "includes = [\"deep.toml\"]\nclass = \"mid\"\n",
                ),
                ("e/hull.toml", "includes = [\"mid.toml\"]\n"),
            ],
        );
        assert_eq!(
            r.provenance.sources(),
            vec!["e/deep.toml", "e/mid.toml", "e/hull.toml"],
            "a fragment's own includes are merged before the fragment itself"
        );
        assert_eq!(r.value.get("class").unwrap().as_str(), Some("mid"));
        assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("deep"));
    }

    #[test]
    fn a_fragment_included_twice_is_merged_twice_not_rejected() {
        // A diamond is legal: two fragments may both build on a common base.
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/base.toml", "class = \"base\"\n"),
                ("e/a.toml", "includes = [\"base.toml\"]\nhull_id = \"a\"\n"),
                ("e/b.toml", "includes = [\"base.toml\"]\npower_rating = 2\n"),
                ("e/hull.toml", "includes = [\"a.toml\", \"b.toml\"]\n"),
            ],
        );
        assert_eq!(r.value.get("class").unwrap().as_str(), Some("base"));
        assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("a"));
        assert_eq!(r.value.get("power_rating").unwrap().as_integer(), Some(2));
    }

    // ── Relative paths ───────────────────────────────────────────────────────

    #[test]
    fn include_paths_resolve_relative_to_the_declaring_template() {
        let r = resolve(
            "assets/entities/hull.toml",
            &[
                ("assets/entities/frag/a.toml", "class = \"a\"\n"),
                (
                    "assets/entities/shared/b.toml",
                    // relative to `assets/entities/frag/`, NOT to the root hull
                    "class = \"b\"\nhull_id = \"b\"\n",
                ),
                (
                    "assets/entities/hull.toml",
                    "includes = [\"frag/a.toml\", \"./shared/b.toml\"]\n",
                ),
            ],
        );
        assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("b"));
        assert_eq!(
            r.provenance.sources(),
            vec![
                "assets/entities/frag/a.toml",
                "assets/entities/shared/b.toml",
                "assets/entities/hull.toml"
            ]
        );
    }

    #[test]
    fn a_nested_fragment_resolves_its_own_includes_relative_to_itself() {
        let r = resolve(
            "assets/entities/hull.toml",
            &[
                ("assets/shared/core.toml", "class = \"core\"\n"),
                (
                    "assets/entities/frag/mid.toml",
                    "includes = [\"../../shared/core.toml\"]\nhull_id = \"mid\"\n",
                ),
                (
                    "assets/entities/hull.toml",
                    "includes = [\"frag/mid.toml\"]\n",
                ),
            ],
        );
        assert_eq!(r.value.get("class").unwrap().as_str(), Some("core"));
        assert_eq!(
            r.provenance.sources()[0],
            "assets/shared/core.toml",
            "`..` must be collapsed against the DECLARING fragment's directory"
        );
    }

    #[test]
    fn canonical_include_path_collapses_dot_segments() {
        assert_eq!(
            canonical_include_path("a/b/hull.toml", "./frag/../frag/x.toml").as_deref(),
            Some("a/b/frag/x.toml")
        );
        assert_eq!(
            canonical_include_path("a/b/hull.toml", "..\\shared\\x.toml").as_deref(),
            Some("a/shared/x.toml"),
            "backslashes are normalised so a Windows-authored path resolves identically"
        );
    }

    #[test]
    fn canonical_include_path_rejects_absolute_references() {
        assert!(canonical_include_path("a/hull.toml", "/etc/passwd.toml").is_none());
        assert!(canonical_include_path("a/hull.toml", "C:\\x\\y.toml").is_none());
        assert!(canonical_include_path("a/hull.toml", "   ").is_none());
    }

    // ── Named / id array behaviour ───────────────────────────────────────────

    #[test]
    fn doctrine_merges_by_id_across_fragments() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    r#"
[behaviour]
[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 40.0
[[behaviour.doctrine]]
id = "hold-station"
base_priority = 10.0
"#,
                ),
                (
                    "e/hull.toml",
                    r#"
includes = ["base.toml"]
[[behaviour.doctrine]]
id = "destroy-hostiles"
base_priority = 90.0
"#,
                ),
            ],
        );
        let doctrine = r
            .value
            .get("behaviour")
            .unwrap()
            .get("doctrine")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(doctrine.len(), 2, "unmentioned entries survive the merge");
        assert_eq!(
            doctrine[0].get("base_priority").unwrap().as_float(),
            Some(90.0),
            "the includer's entry replaces the fragment's by id"
        );
        assert_eq!(
            doctrine[0].get("directive_kind").unwrap().as_str(),
            Some("Destroy"),
            "keys the includer did not mention survive"
        );
    }

    #[test]
    fn state_merges_by_name_across_fragments() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    r#"
[behaviour]
[[behaviour.state]]
name = "patrol"
target_speed = 0.5
[[behaviour.state]]
name = "idle"
target_speed = 0.0
"#,
                ),
                (
                    "e/hull.toml",
                    r#"
includes = ["base.toml"]
[[behaviour.state]]
name = "patrol"
target_speed = 0.9
"#,
                ),
            ],
        );
        let states = r
            .value
            .get("behaviour")
            .unwrap()
            .get("state")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].get("target_speed").unwrap().as_float(), Some(0.9));
        assert_eq!(states[1].get("target_speed").unwrap().as_float(), Some(0.0));
    }

    /// `68bda1be`'s empty-array rule has to mean something coherent between
    /// fragments too: an authored empty array CLEARS, an omitted key does not.
    #[test]
    fn a_fragment_authoring_an_empty_doctrine_clears_what_came_before() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/armed.toml",
                    r#"
[behaviour]
waypoint_arrival_radius = 20.0
[[behaviour.doctrine]]
id = "destroy-hostiles"
base_priority = 40.0
"#,
                ),
                (
                    "e/hull.toml",
                    "includes = [\"armed.toml\"]\nbehaviour = { doctrine = [] }\n",
                ),
            ],
        );
        let doctrine = r
            .value
            .get("behaviour")
            .unwrap()
            .get("doctrine")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(
            doctrine.is_empty(),
            "an explicitly authored empty array is a fragment's only subtractive lever"
        );
        assert_eq!(
            r.value
                .get("behaviour")
                .unwrap()
                .get("waypoint_arrival_radius")
                .unwrap()
                .as_float(),
            Some(20.0),
            "clearing one list must not disturb the rest of the block"
        );
    }

    #[test]
    fn omitting_doctrine_leaves_the_fragments_list_alone() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/armed.toml",
                    "[behaviour]\n[[behaviour.doctrine]]\nid = \"kill\"\nbase_priority = 40.0\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"armed.toml\"]\n[behaviour]\nwaypoint_arrival_radius = 5.0\n",
                ),
            ],
        );
        let doctrine = r
            .value
            .get("behaviour")
            .unwrap()
            .get("doctrine")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(doctrine.len(), 1, "an absent key never reaches the merge");
    }

    #[test]
    fn plain_arrays_replace_wholesale_between_fragments() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/base.toml", "tags = [\"ship\", \"npc\"]\n"),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\ntags = [\"scenery\"]\n",
                ),
            ],
        );
        let tags: Vec<&str> = r
            .value
            .get("tags")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(tags, vec!["scenery"]);
    }

    // ── Cycles and missing paths ─────────────────────────────────────────────

    #[test]
    fn a_direct_cycle_is_a_load_error() {
        let err = resolve_template(
            "e/a.toml",
            &src(&[
                ("e/a.toml", "includes = [\"b.toml\"]\n"),
                ("e/b.toml", "includes = [\"a.toml\"]\n"),
            ]),
        )
        .expect_err("a cycle must not resolve");
        assert_eq!(err.category(), "include-cycle");
        assert_eq!(
            err.chain,
            vec!["e/a.toml", "e/b.toml", "e/a.toml"],
            "the error must name the chain that closed the loop"
        );
        assert!(err.finding.is_error(), "a cycle is never a warning");
    }

    #[test]
    fn a_self_include_is_a_load_error() {
        let err = resolve_template(
            "e/a.toml",
            &src(&[("e/a.toml", "includes = [\"a.toml\"]\n")]),
        )
        .expect_err("a self-include must not resolve");
        assert_eq!(err.category(), "include-cycle");
        assert_eq!(err.chain, vec!["e/a.toml", "e/a.toml"]);
    }

    #[test]
    fn a_cycle_through_relative_paths_is_still_detected() {
        // The two references spell the same file differently; canonicalisation
        // is what makes them one identity for cycle detection.
        let err = resolve_template(
            "e/a.toml",
            &src(&[
                ("e/a.toml", "includes = [\"./frag/b.toml\"]\n"),
                ("e/frag/b.toml", "includes = [\"../a.toml\"]\n"),
            ]),
        )
        .expect_err("a cycle spelled through `.`/`..` must still be caught");
        assert_eq!(err.category(), "include-cycle");
    }

    #[test]
    fn a_missing_include_is_a_load_error_naming_the_chain() {
        let err = resolve_template(
            "e/a.toml",
            &src(&[
                ("e/a.toml", "includes = [\"mid.toml\"]\n"),
                ("e/mid.toml", "includes = [\"gone.toml\"]\n"),
            ]),
        )
        .expect_err("a missing fragment must not resolve");
        assert_eq!(err.category(), "include-missing");
        assert_eq!(err.chain, vec!["e/a.toml", "e/mid.toml", "e/gone.toml"]);
        assert_eq!(
            err.finding.source.file, "e/mid.toml",
            "the diagnostic points at the file that DECLARED the bad include"
        );
        assert!(err.to_string().contains("include chain"));
    }

    #[test]
    fn a_fragment_that_is_not_valid_toml_is_a_load_error() {
        let err = resolve_template(
            "e/a.toml",
            &src(&[
                ("e/a.toml", "includes = [\"bad.toml\"]\n"),
                ("e/bad.toml", "this is not = = toml\n"),
            ]),
        )
        .expect_err("an unparseable fragment must not resolve");
        assert_eq!(err.category(), "include-parse");
        assert_eq!(err.chain, vec!["e/a.toml", "e/bad.toml"]);
    }

    #[test]
    fn a_malformed_includes_declaration_is_a_load_error() {
        for body in ["includes = \"base.toml\"\n", "includes = [3]\n"] {
            let err = resolve_template("e/a.toml", &src(&[("e/a.toml", body)]))
                .expect_err("`includes` must be an array of path strings");
            assert_eq!(err.category(), "include-malformed", "for body {body:?}");
        }
    }

    #[test]
    fn an_absolute_include_is_a_load_error() {
        let err = resolve_template(
            "e/a.toml",
            &src(&[("e/a.toml", "includes = [\"/etc/hull.toml\"]\n")]),
        )
        .expect_err("absolute include paths are not resolvable relative to the declarer");
        assert_eq!(err.category(), "include-malformed");
    }

    #[test]
    fn a_resolved_template_that_is_not_a_valid_entity_is_a_load_error() {
        // A `Patrol` doctrine carrying `directive_anchors` is valid; so is a
        // bare `Destroy`. Reconciling them by id produces a `Destroy` directive
        // that still carries the Patrol-only field — which nothing rejects
        // until the RESOLVED document is validated, because the offending
        // combination exists in neither authored file.
        const FRAGMENT: &str = r#"
[behaviour]
[[behaviour.doctrine]]
id = "patrol-lane"
directive_kind = "Patrol"
directive_anchors = ["alpha"]
base_priority = 10.0
"#;
        const HULL: &str = r#"
[behaviour]
[[behaviour.doctrine]]
id = "patrol-lane"
directive_kind = "Destroy"
"#;
        // Lenient: these two snippets are doctrine fixtures, not hulls, and
        // the point is the RESOLVED document's doctrine reconciliation. Strict
        // AI-declaration mode would reject both for the fifteen declarations
        // neither was ever meant to carry — see `EntityConfig::from_toml_in_mode`.
        let lenient = crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient;
        assert!(
            EntityConfig::from_toml_in_mode(FRAGMENT, lenient).is_ok(),
            "fragment is valid alone"
        );
        assert!(
            EntityConfig::from_toml_in_mode(HULL, lenient).is_ok(),
            "hull is valid alone"
        );

        let hull_with_include = format!("includes = [\"base.toml\"]\n{HULL}");
        let resolved = resolve(
            "e/hull.toml",
            &[
                ("e/base.toml", FRAGMENT),
                ("e/hull.toml", hull_with_include.as_str()),
            ],
        );
        let err = resolved
            .parse()
            .expect_err("an invalid RESOLVED template must be rejected");
        assert_eq!(err.category(), "include-invalid-template");
        assert_eq!(
            err.chain,
            vec!["e/base.toml", "e/hull.toml"],
            "the error names every template that contributed"
        );
    }

    // ── Provenance ───────────────────────────────────────────────────────────

    #[test]
    fn provenance_names_the_fragment_that_authored_each_field() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/base.toml", "class = \"escort\"\nhull_id = \"BASE\"\n"),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\nhull_id = \"NCC-1\"\n",
                ),
            ],
        );
        assert_eq!(r.provenance.origin("class").unwrap().source, "e/base.toml");
        assert_eq!(
            r.provenance.origin("hull_id").unwrap().source,
            "e/hull.toml",
            "the LAST author of a field is the one that won the merge"
        );
        assert!(r.provenance.origin("power_rating").is_none());
    }

    #[test]
    fn provenance_records_the_chain_that_reached_each_source() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/deep.toml", "class = \"deep\"\n"),
                (
                    "e/mid.toml",
                    "includes = [\"deep.toml\"]\nhull_id = \"mid\"\n",
                ),
                ("e/hull.toml", "includes = [\"mid.toml\"]\n"),
            ],
        );
        assert_eq!(
            r.provenance.origin("class").unwrap().chain,
            vec!["e/hull.toml", "e/mid.toml", "e/deep.toml"],
            "the chain runs root-first down to the authoring fragment"
        );
        assert_eq!(
            r.provenance.origin("hull_id").unwrap().chain,
            vec!["e/hull.toml", "e/mid.toml"]
        );
    }

    #[test]
    fn provenance_addresses_reconciled_array_elements_by_key() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    r#"
[behaviour]
[[behaviour.doctrine]]
id = "kill"
base_priority = 40.0
directive_kind = "Destroy"
"#,
                ),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\n[[behaviour.doctrine]]\nid = \"kill\"\nbase_priority = 90.0\n",
                ),
            ],
        );
        assert_eq!(
            r.provenance
                .origin("behaviour.doctrine[id=kill].base_priority")
                .unwrap()
                .source,
            "e/hull.toml",
            "the overriding priority is attributed to the hull"
        );
        assert_eq!(
            r.provenance
                .origin("behaviour.doctrine[id=kill].directive_kind")
                .unwrap()
                .source,
            "e/base.toml",
            "a key the hull never mentioned stays attributed to the fragment"
        );
    }

    #[test]
    fn provenance_drops_fields_a_later_fragment_replaced_wholesale() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    "[behaviour]\n[[behaviour.doctrine]]\nid = \"kill\"\nbase_priority = 40.0\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\nbehaviour = { doctrine = [] }\n",
                ),
            ],
        );
        assert!(
            r.provenance
                .origin("behaviour.doctrine[id=kill].base_priority")
                .is_none(),
            "a cleared list must not still report a field that no longer exists"
        );
        assert_eq!(
            r.provenance.origin("behaviour.doctrine").unwrap().source,
            "e/hull.toml",
            "the clear itself is attributed to the fragment that authored it"
        );
    }

    #[test]
    fn provenance_of_an_uncomposed_template_is_the_template_itself() {
        let r = resolve("e/hull.toml", &[("e/hull.toml", "class = \"solo\"\n")]);
        assert!(!r.is_composed());
        assert_eq!(r.provenance.sources(), vec!["e/hull.toml"]);
        assert_eq!(
            r.provenance.origin("class").unwrap().chain,
            vec!["e/hull.toml"]
        );
    }

    // ── Byte stability ───────────────────────────────────────────────────────

    #[test]
    fn an_uncomposed_template_resolves_to_its_own_bytes_verbatim() {
        let body = "# a comment\nclass  =  \"solo\"\n\n[hull]\nhull_integrity = 10.0\n";
        let r = resolve("e/hull.toml", &[("e/hull.toml", body)]);
        assert_eq!(
            r.toml, body,
            "a template with no includes must not be reformatted — its raw text is \
             what marker validation and line lookups read"
        );
    }

    #[test]
    fn resolution_is_byte_stable_across_runs_and_delivery_order() {
        let pairs = [
            ("e/a.toml", "class = \"a\"\nhull_id = \"a\"\n"),
            ("e/b.toml", "power_rating = 3\nclass = \"b\"\n"),
            (
                "e/hull.toml",
                "includes = [\"a.toml\", \"b.toml\"]\nname = \"H\"\n",
            ),
        ];
        let first = resolve("e/hull.toml", &pairs);
        // A different insertion order into the source map — the hash map's
        // iteration order changes, the resolved bytes must not.
        let mut reordered: Vec<(&str, &str)> = pairs.to_vec();
        reordered.reverse();
        let second = resolve("e/hull.toml", &reordered);
        assert_eq!(first.toml, second.toml);
        assert_eq!(
            first.toml,
            resolve("e/hull.toml", &pairs).toml,
            "resolving the same inputs twice must produce identical bytes"
        );
        assert!(first.is_composed());
        assert!(
            !first.toml.contains(INCLUDES_KEY),
            "the resolved document must not carry the authoring key into the runtime"
        );
    }

    #[test]
    fn the_resolved_document_never_carries_an_includes_key() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/deep.toml", "class = \"deep\"\n"),
                ("e/mid.toml", "includes = [\"deep.toml\"]\n"),
                ("e/hull.toml", "includes = [\"mid.toml\"]\n"),
            ],
        );
        assert!(r.value.get(INCLUDES_KEY).is_none());
        assert!(!r.toml.contains(INCLUDES_KEY));
    }

    // ── Preload contract (the shape both hosts share) ────────────────────────

    #[test]
    fn preload_step_reports_the_paths_still_to_fetch() {
        let delivered = src(&[(
            "e/hull.toml",
            "includes = [\"frag/a.toml\", \"frag/b.toml\"]\n",
        )]);
        let step = preload_step("e/hull.toml", &delivered).expect("not an error, just pending");
        assert_eq!(
            step,
            PreloadStep::AwaitingIncludes(vec![
                "e/frag/a.toml".to_string(),
                "e/frag/b.toml".to_string()
            ]),
            "the host is told the CANONICAL paths to fetch, in declared order"
        );
    }

    #[test]
    fn preload_step_walks_the_closure_one_layer_at_a_time() {
        let mut delivered = src(&[("e/hull.toml", "includes = [\"mid.toml\"]\n")]);
        let PreloadStep::AwaitingIncludes(fetch) = preload_step("e/hull.toml", &delivered).unwrap()
        else {
            panic!("expected a pending step");
        };
        assert_eq!(fetch, vec!["e/mid.toml"]);

        delivered.insert(
            "e/mid.toml".into(),
            "includes = [\"deep.toml\"]\nclass = \"mid\"\n".into(),
        );
        let PreloadStep::AwaitingIncludes(fetch) = preload_step("e/hull.toml", &delivered).unwrap()
        else {
            panic!("the transitive include must be discovered once its parent lands");
        };
        assert_eq!(fetch, vec!["e/deep.toml"]);

        delivered.insert("e/deep.toml".into(), "hull_id = \"deep\"\n".into());
        let PreloadStep::Ready(resolved) = preload_step("e/hull.toml", &delivered).unwrap() else {
            panic!("with every fragment delivered the template must resolve");
        };
        assert_eq!(
            resolved.value.get("hull_id").unwrap().as_str(),
            Some("deep")
        );
        assert_eq!(resolved.value.get("class").unwrap().as_str(), Some("mid"));
    }

    #[test]
    fn preload_step_still_rejects_a_cycle() {
        let delivered = src(&[
            ("e/a.toml", "includes = [\"b.toml\"]\n"),
            ("e/b.toml", "includes = [\"a.toml\"]\n"),
        ]);
        let err = preload_step("e/a.toml", &delivered)
            .expect_err("absence is 'not yet'; a cycle is never fetchable");
        assert_eq!(err.category(), "include-cycle");
    }

    #[test]
    fn preload_step_reports_a_missing_root_as_an_error_not_a_fetch() {
        let err = preload_step("e/gone.toml", &src(&[]))
            .expect_err("the root was just delivered by the host; its absence is a bug");
        assert_eq!(err.category(), "include-missing");
    }

    #[test]
    fn preload_step_is_ready_immediately_for_an_uncomposed_template() {
        let delivered = src(&[("e/hull.toml", "class = \"solo\"\n")]);
        let PreloadStep::Ready(resolved) = preload_step("e/hull.toml", &delivered).unwrap() else {
            panic!("a template with no includes needs no extra fetches");
        };
        assert!(!resolved.is_composed());
    }

    // ── Filesystem adapter + the shipped fixtures ────────────────────────────

    #[cfg(not(target_arch = "wasm32"))]
    mod on_disk {
        use super::*;

        const COMPOSED: &str = "assets/entities/fragments/composed_escort.toml";
        const CORE: &str = "assets/entities/fragments/npc_escort_core.toml";
        const CAPTAIN: &str = "assets/entities/fragments/ai/captain_red_alert_aggressive.toml";
        /// The fourteen other ship-level AI declarations, which every AI-bearing
        /// hull has owed since #885b stage 5d made strict mode the default.
        const BASELINE: &str = "assets/entities/fragments/ai/fleet_baseline.toml";

        #[test]
        fn the_composed_fixture_hull_resolves_off_disk() {
            let resolved = resolve_from_disk(COMPOSED).expect("fixture hull must resolve");
            assert_eq!(
                resolved.provenance.sources(),
                vec![CAPTAIN, BASELINE, CORE, COMPOSED],
                "depth-first in declared order, declaring hull last"
            );
        }

        #[test]
        fn the_composed_fixture_hull_parses_as_an_entity() {
            let config = load_entity_config(COMPOSED).expect("resolved fixture must be valid");
            let ship = config
                .ship_config
                .as_ref()
                .expect("the systems fragment must supply a ship_config");
            assert!(
                ship.systems.iter().any(|s| s.kind == "helm_thrust"),
                "the shared fragment's system suite must reach the resolved hull"
            );
            assert!(
                config
                    .captain_console
                    .as_ref()
                    .and_then(|c| c.ai.as_ref())
                    .is_some(),
                "the nested AI fragment's captain policy must reach the resolved hull"
            );
        }

        #[test]
        fn the_fixture_hull_specialises_the_fragments_doctrine_by_id() {
            let config = load_entity_config(COMPOSED).expect("resolved fixture must be valid");
            let behaviour = config.behaviour.expect("fragment supplies [behaviour]");
            let doctrine = behaviour
                .doctrine
                .iter()
                .find(|d| d.id == "destroy-hostiles")
                .expect("the shared fragment's doctrine must survive");
            assert!(
                (doctrine.base_priority - 90.0).abs() < 1e-6,
                "the hull's by-id specialisation must beat the fragment's base_priority, \
                 got {}",
                doctrine.base_priority
            );
            assert_eq!(
                doctrine.directive_kind.as_deref(),
                Some("Destroy"),
                "a key the hull never mentioned comes from the fragment"
            );
        }

        #[test]
        fn provenance_attributes_the_fixture_fields_to_the_right_files() {
            let resolved = resolve_from_disk(COMPOSED).expect("fixture hull must resolve");
            let p = &resolved.provenance;
            assert_eq!(
                p.origin("behaviour.doctrine[id=destroy-hostiles].base_priority")
                    .expect("doctrine priority is recorded")
                    .source,
                COMPOSED
            );
            assert_eq!(
                p.origin("behaviour.doctrine[id=destroy-hostiles].directive_kind")
                    .expect("directive kind is recorded")
                    .source,
                CORE
            );
            assert_eq!(
                p.origin("captain_console.ai.rule")
                    .expect("the captain rule list is recorded")
                    .chain,
                vec![COMPOSED, CORE, CAPTAIN],
                "the chain must show the AI fragment was reached THROUGH the core fragment"
            );
        }

        /// The fragments are partial by design — none of them is a valid entity
        /// on its own, which is exactly why they live outside `assets/entities/`
        /// where every "shipped template still loads" test would try to parse
        /// them.
        #[test]
        fn the_fragments_live_outside_the_shipped_template_directory() {
            for path in [CORE, CAPTAIN] {
                assert!(
                    std::path::Path::new(path).exists(),
                    "{path} must exist on disk"
                );
                let dir = std::path::Path::new(path).parent().unwrap();
                assert_ne!(
                    dir,
                    std::path::Path::new("assets/entities"),
                    "a fragment in assets/entities/ would be scanned as a shipped hull"
                );
            }
        }

        /// "Resolution must be identical on native and WASM", made checkable:
        /// the browser's incremental delivery of the SAME files must produce
        /// the same bytes as reading them straight off disk.
        #[test]
        fn the_incremental_walk_and_the_filesystem_walk_agree_byte_for_byte() {
            let native = resolve_from_disk(COMPOSED).expect("fixture resolves off disk");

            let mut delivered: HashMap<String, String> = HashMap::new();
            delivered.insert(
                COMPOSED.to_string(),
                std::fs::read_to_string(COMPOSED).unwrap(),
            );
            let mut rounds = 0;
            let browser = loop {
                rounds += 1;
                assert!(rounds < 16, "the closure walk must terminate");
                match preload_step(COMPOSED, &delivered).expect("no composition error") {
                    PreloadStep::Ready(resolved) => break *resolved,
                    PreloadStep::AwaitingIncludes(paths) => {
                        for path in paths {
                            let body = std::fs::read_to_string(&path)
                                .unwrap_or_else(|e| panic!("the resolver asked for {path}: {e}"));
                            delivered.insert(path, body);
                        }
                    }
                }
            };
            assert_eq!(browser.toml, native.toml);
            assert_eq!(browser.provenance, native.provenance);
        }

        #[test]
        fn fs_fragment_source_misses_return_none_rather_than_panicking() {
            assert!(FsFragmentSource
                .read("assets/entities/fragments/definitely_absent.toml")
                .is_none());
        }
    }
}
