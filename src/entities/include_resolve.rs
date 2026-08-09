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
// ## Why include resolution is UPSTREAM of `resolve_entity_via`, not inside it
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
//    of the entity-instance resolver (`resolve_entity` then, `resolve_entity_via`
//    since #973) so that the instance merge could be reused on its own.
//    Pushing a second, per-template concern back in would re-fuse them.
//
// Both layers share the same merge FUNCTION
// (`entity_override::merge_entity_config_toml_with`) and differ only in the
// `MergePolicy` they pass it — see "Array semantics" below. Cardinality and
// input form differ too.
//
// ## Merge order
//
// Depth-first, in declared order. Each fragment is merged into the accumulator,
// and the declaring template is merged **last**, so the includer always wins.
// A template that includes `[a, b]` resolves as
// `((a's own closure) ⊕ (b's own closure)) ⊕ self`.
//
// ## Array semantics (issue #911 — this SUPERSEDES #869's "everything else
// replaces wholesale")
//
// This layer merges under `MergePolicy::ComposeFragments`, the instance-override
// layer under `MergePolicy::InstanceOverride`. That is the seam #869 did not
// have, and the reason it put array extension out of scope: with one shared
// rule, letting a hull EXTEND a fragment's `[[system]]` suite would have
// silently let every world override extend it too.
//
// What a fragment author writes, and what it does:
//
// * **Extend** — declare an entry with an `id` (or, for `[[station.rating]]`, a
//   `name`) that no earlier fragment used. It is APPENDED. This is the case
//   #911 exists for: "the library's systems, plus two of my own", with no new
//   syntax and no marker.
// * **Replace one entry** — declare an entry whose key MATCHES an inherited
//   one. It deep-merges into it, **at the inherited entry's position**, so
//   fields you do not mention survive and `[[shield_arc]]`'s load-bearing order
//   is preserved.
// * **Remove one entry** — declare `{ id = "…", _remove = true }`. The
//   inherited entry is dropped and the tombstone itself never reaches the
//   resolved document (the marker is stripped exactly as `includes` is, because
//   `EntityConfig` is `deny_unknown_fields`). A tombstone matching nothing is a
//   no-op.
// * **Clear the whole list** — author an empty array (`doctrine = []`,
//   `system = []`). Unchanged from #869, and it still beats the element-wise
//   rules.
// * **Leave it alone** — omit the key. An absent key never reaches the merge.
//
// Which arrays reconcile is one table, `entity_override::COMPOSE_KEYED_ARRAYS`,
// keyed on the dotted, index-free path: `system`/`station`/`shield_arc`/
// `weapons_console.{phaser,blaster}_banks`/`torpedoes.tubes`/
// `behaviour.doctrine` by `id`, and `station.rating` by `name`. **Provenance
// below reads the SAME table** — see `record_leaves`.
//
// `tags` has no key (bare strings), so it UNIONS here and REPLACES at the
// instance layer; that asymmetry is deliberate and is what the policy seam is
// for. Arrays with no stable identity — `*.ai.rule`, `*_ai.state[].transition`,
// `*.selector.score`, `hull.system_hull` — keep replacing
// wholesale. **A fragment contributing an AI policy contributes it WHOLE**;
// that is the intended granularity.
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
use crate::entity_override::{ArrayRule, MergePolicy};
use crate::world::validate::{line_of, Severity, SourceLocation, WorldFinding};

/// The authored key that lists a template's ordered includes.
pub const INCLUDES_KEY: &str = "includes";

/// The layer this resolver merges at (issue #911). Fragments compose; a world's
/// `[[entity]].overrides` do not — see [`MergePolicy`].
const POLICY: MergePolicy = MergePolicy::ComposeFragments;

// ── Source of template text ──────────────────────────────────────────────────

/// Where the resolver gets template text from, keyed by canonical path.
///
/// This trait is the ONLY seam through which text enters the resolver: the
/// resolver functions themselves ([`resolve_template`], [`preload_step`] and the
/// merge below them) never touch a filesystem or a config cache. Individual
/// *implementations* of the trait certainly do — `FsFragmentSource` reads the
/// disk and [`HostFragmentSource`] reads the config cache as well — which is
/// exactly the point of putting the seam here.
///
/// Object-safe (`&self`, no generics) so callers hold a `&dyn FragmentSource`,
/// exactly like [`crate::entity_loader::TemplateLoader`].
///
/// A `None` means "not available *yet*" as much as "does not exist"; which of
/// those it is depends on the caller's [`MissingPolicy`] and on
/// [`FragmentSource::absence_is_final`].
pub trait FragmentSource {
    /// The raw TOML text at `path`, or `None` when it cannot be served.
    fn read(&self, path: &str) -> Option<String>;

    /// Whether a `None` from [`FragmentSource::read`] is the FINAL answer.
    ///
    /// `true` for a source that already holds everything it will ever hold: the
    /// filesystem, a fixture map, a mod-pack overlay. A fragment it cannot
    /// serve does not exist, and a validator may say so.
    ///
    /// `false` for a source that fills INCREMENTALLY, where absence means "not
    /// delivered yet". The browser is the one that matters: raw templates
    /// arrive asynchronously, so a root can be in hand a whole layer-load
    /// before the fragment it includes. A validator that read that race as a
    /// fault would condemn a perfectly good world — see
    /// [`composition_finding`].
    ///
    /// # Why this has NO default
    ///
    /// A default could only be `true`, and `true` is the answer that fails
    /// destructively: an incrementally-filling source that omits the method
    /// gets its in-flight fragments read as faults, and the world blanks
    /// permanently, because the layer is marked loaded and never retried. The
    /// safe answer must not be reachable by omission.
    ///
    /// It also has to be a compile-time obligation rather than a tested one.
    /// The dangerous case — someone deletes
    /// [`HostFragmentSource::absence_is_final`] — is invisible to a native
    /// suite, because `true` IS the native answer, so CI would stay green
    /// while the browser broke. With no default, that deletion is a build
    /// error on both targets instead.
    fn absence_is_final(&self) -> bool;
}

impl FragmentSource for std::collections::HashMap<String, String> {
    fn read(&self, path: &str) -> Option<String> {
        self.get(path).cloned()
    }

    /// A fixture map is complete by construction: what it lacks does not exist.
    fn absence_is_final(&self) -> bool {
        true
    }
}

impl FragmentSource for BTreeMap<String, String> {
    fn read(&self, path: &str) -> Option<String> {
        self.get(path).cloned()
    }

    /// A fixture map — or an uploaded mod pack's file set, which is fully in
    /// hand before validation begins — is complete by construction.
    fn absence_is_final(&self) -> bool {
        true
    }
}

/// Filesystem adapter for the pure resolver above.
///
/// One of the two I/O-touching items in this file (see also
/// [`HostFragmentSource`]), mirroring how `entity_loader::FsTemplateLoader` sits
/// beside the pure resolution in `loader.rs`. The mod-pack overlay is consulted FIRST so an uploaded pack's
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

    /// The filesystem is authoritative and the overlay is installed whole, so
    /// a fragment neither can serve genuinely does not exist.
    fn absence_is_final(&self) -> bool {
        true
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
/// Leaf paths are dotted (`hull.hull_integrity`). Every array the merge
/// reconciles by key is addressed by that key rather than by index, because
/// positions are not stable across a merge:
///
/// * `behaviour.doctrine[id=destroy-hostiles].base_priority`
/// * `system[id=helm-thrust].ai_only`
/// * `station[id=bridge].rating[name=tactical].level`
///
/// Every other array is a merge leaf, so it is recorded at its own path with no
/// element addressing — `tags`, `captain_console.ai.rule`, and a cleared
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
        record_leaves("", "", value, &step, &mut self.fields);
        self.order.push(step);
    }
}

/// Walk one fragment's contribution, recording who authored each leaf.
///
/// # Two paths, and why
///
/// `prefix` is the PROVENANCE address, which carries element keys
/// (`station[id=bridge].rating[name=tactical]`). `merge_path` is the dotted,
/// index-free path the MERGE judges by (`station.rating`). They have to be
/// separate strings because the identity table is keyed on the second, and
/// consulting it with the first would never match.
///
/// # Why this must read the merge's own table (issue #911)
///
/// Before #911 this function carried its own copy of the keyed-array knowledge
/// — a two-arm `match` on `behaviour.state` / `behaviour.doctrine`. The moment
/// the merge started reconciling `[[system]]` by `id`, that copy would have
/// gone stale in the most damaging direction: a merged-in `[[system]]` array
/// recorded as a wholesale leaf makes [`insert_leaf`]'s `retain` prune EVERY
/// field an earlier fragment contributed to that array, so provenance would
/// confidently report a system suite as authored by whichever fragment touched
/// it last. Both now read [`MergePolicy::array_rule`].
fn record_leaves(
    prefix: &str,
    merge_path: &str,
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
                if key == crate::entity_override::REMOVE_KEY {
                    // The tombstone marker is authoring metadata, stripped from
                    // the resolved document; it is not a field anyone authored.
                    continue;
                }
                let path = join_field(prefix, key);
                let merge_child = if merge_path.is_empty() {
                    key.clone()
                } else {
                    format!("{merge_path}.{key}")
                };
                record_leaves(&path, &merge_child, child, step, out);
            }
        }
        toml::Value::Array(items) => {
            match POLICY.array_rule(merge_path) {
                // A keyed array reconciles element-by-element, so record the
                // elements and leave siblings from earlier fragments intact.
                ArrayRule::Keyed(key) if !items.is_empty() => {
                    out.remove(prefix);
                    for (index, element) in items.iter().enumerate() {
                        let addressed = element
                            .get(key)
                            .and_then(|v| v.as_str())
                            .map(|id| format!("{prefix}[{key}={id}]"))
                            .unwrap_or_else(|| format!("{prefix}[{index}]"));
                        if crate::entity_override::is_removal(element) {
                            // A removal is the opposite of authoring: prune the
                            // entry an earlier fragment recorded rather than
                            // claiming it.
                            out.retain(|k, _| k != &addressed && !is_descendant(k, &addressed));
                            continue;
                        }
                        record_leaves(&addressed, merge_path, element, step, out);
                    }
                }
                // Everything else is a merge leaf — replaced or unioned
                // wholesale — including an authored empty array, which is how a
                // fragment CLEARS a list.
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

    // Strip the `_remove` tombstone for the same reason `take_includes` strips
    // `includes`: an authoring marker must not exist at runtime, and
    // `EntityConfig` is `deny_unknown_fields`.
    //
    // `merge_entity_config_toml_with` already strips under this policy, and
    // every composed closure ends in a merge, so the ONLY document this catches
    // is an UNCOMPOSED root that authors a tombstone — no accumulator, no
    // merge, nothing to strip it. That document is an authoring error (there is
    // nothing to remove), but it must not be one that leaks a marker into
    // `value` while `toml` is served verbatim from `root_text`. Pinned by
    // `an_uncomposed_template_never_leaks_a_tombstone_into_its_value`.
    let value = crate::entity_override::strip_removals(
        &ctx.accumulator
            .unwrap_or_else(|| toml::Value::Table(toml::value::Table::new())),
    );
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
    // The merge is fallible for exactly one reason (issue #911): a `_remove`
    // tombstone reaching a policy that does not honour it. `POLICY` here is
    // `ComposeFragments`, which DOES honour it, so this cannot fail today. It is
    // mapped onto the include chain rather than unwrapped so that changing
    // `POLICY` yields a located diagnostic instead of a panic in the resolver.
    let merged = match ctx.accumulator.take() {
        None => Ok(value.clone()),
        Some(accumulated) => {
            crate::entity_override::merge_entity_config_toml_with(&accumulated, &value, POLICY)
        }
    };
    let merged = match merged {
        Ok(merged) => merged,
        Err(message) => {
            let mut chain = ctx.stack.clone();
            ctx.stack.pop();
            chain.push(path.to_string());
            return Err(IncludeError::new(
                "include-invalid-template",
                chain,
                SourceLocation {
                    file: path.to_string(),
                    line: line_of(&text, crate::entity_override::REMOVE_KEY),
                    reference: path.to_string(),
                },
                message,
            ));
        }
    };
    ctx.accumulator = Some(merged);
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

// ── Composition as a world finding (issue #906) ──────────────────────────────

/// The fragment source a HOST resolves against, on either target.
///
/// Mirrors [`crate::entity_loader::WasmTemplateLoader`]'s three-step lookup, one
/// layer lower down (raw text rather than parsed configs):
///
/// 1. the session mod-pack overlay, so an uploaded pack's fragment wins;
/// 2. the raw templates the host has already delivered — on WASM this is the
///    *only* source, since there is no filesystem;
/// 3. the filesystem, on native.
///
/// Compiles on both targets so callers can name it unconditionally, which is
/// what lets [`crate::world::validate::validate_composition`] carry a default
/// source without a `cfg` split at every call site.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostFragmentSource;

impl FragmentSource for HostFragmentSource {
    fn read(&self, path: &str) -> Option<String> {
        if let Some(text) = crate::config_cache::mod_pack_overlay_get(path) {
            return Some(text);
        }
        if let Some(text) = crate::config_cache::raw_template_text(path) {
            return Some(text);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::read_to_string(path).ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Native: the filesystem is authoritative, so absence is final.
    ///
    /// Browser: it is NOT. `RAW_TEMPLATE_TOML` fills one delivery at a time, so
    /// a fragment this source cannot serve may simply be still in flight. The
    /// initial preload waits for the queue to drain before it declares itself
    /// complete, but the runtime layer load does not: `world::server` builds the
    /// layer cache and spawns the moment the layer TOML arrives, while that
    /// layer's entity templates were only just queued. Treating that race as a
    /// composition fault would blank the entire world — permanently, since the
    /// layer is marked loaded and never retried.
    fn absence_is_final(&self) -> bool {
        !cfg!(target_arch = "wasm32")
    }
}

/// Compose `path` for VALIDATION, reporting a composition failure as a
/// [`WorldFinding`] the world-finding flow can gate on (issue #906).
///
/// Every load path already resolves includes; each one handled a failure
/// privately (warn-and-skip, `None`, a `BuildError`, a JS console error), so a
/// world with a broken include quietly lost an entity instead of failing
/// validation. This is the seam that turns the `IncludeError`'s own finding —
/// which already carries the right severity, category and source line — into
/// something [`crate::world::validate::has_error`] sees.
///
/// # What is deliberately NOT reported
///
/// * **A root template the source cannot serve at all.** A missing template is
///   a different defect with its own diagnostics (dispatch warns, the
///   `[[entity]]` loader errors), and a validator that runs where the content
///   is not reachable — a test fixture path, a browser before preload — must
///   not manufacture an error out of its own blindness. Same policy as
///   `world::validate::doctrine_anchor_refs`.
/// * **A fragment the source cannot serve YET.** The same blindness one level
///   down, and the reason this function walks the closure with
///   [`preload_step`] rather than [`resolve_template`]. A source that fills
///   incrementally — the browser's raw-template channel — routinely holds a
///   root before the fragment it includes, and reading that race as a fault
///   would fail validation for the whole world and lose every entity in it. A
///   source whose absence IS final ([`FragmentSource::absence_is_final`], true
///   for the filesystem and for every fixture map) still reports the missing
///   fragment, with the declaring file and line.
/// * **A root template that is not valid TOML on its own.** That is the plain
///   parse error every host has always skipped with a warning (one bad
///   cosmetic asteroid must not stop a combat test); it is not a *composition*
///   failure and this function must not silently promote it to one. Detected
///   as `include-parse` whose chain is the root alone — an unparseable
///   *fragment* always has the includer above it in the chain.
/// * **A resolved document that fails `EntityConfig` validation when nothing
///   was composed.** Same reason. When something WAS composed it is reported,
///   because the offending combination then exists in no single authored file
///   — exactly the failure mode composition introduces, and the same asymmetry
///   `headless::app::preload_entity_templates` already applies.
pub fn composition_finding(path: &str, source: &dyn FragmentSource) -> Option<WorldFinding> {
    let root = canonical_template_path(path);
    // Blindness is not a finding — see the doc above.
    source.read(&root)?;

    // `preload_step`, not `resolve_template`: absence must stay separable from
    // the faults. Cycles, malformed `includes` declarations and unparseable
    // fragments all still error under this policy, so no real diagnostic is
    // traded away for the separation.
    let resolved = match preload_step(&root, source) {
        Ok(PreloadStep::Ready(resolved)) => *resolved,
        Ok(PreloadStep::AwaitingIncludes(_)) => {
            if !source.absence_is_final() {
                // Still in flight — see the doc above.
                return None;
            }
            // Absence is final, so the fragment genuinely does not exist.
            // Re-walk under the failing policy purely to get the located
            // `include-missing` finding, which names the file that DECLARED the
            // bad include rather than just the paths still outstanding.
            return resolve_template(&root, source)
                .err()
                .map(|e| finding_of(&e));
        }
        Err(e) => {
            if e.chain.len() == 1 && e.category() == "include-parse" {
                return None;
            }
            return Some(finding_of(&e));
        }
    };
    if !resolved.is_composed() {
        return None;
    }
    resolved.parse().err().map(|e| finding_of(&e))
}

/// The error's own finding, with the include chain folded into the message so a
/// reader of the validation badge sees which fragment chain reached the fault.
fn finding_of(e: &IncludeError) -> WorldFinding {
    let mut finding = e.finding.clone();
    finding.message = format!("{} [include chain: {}]", finding.message, e.chain_display());
    finding
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

    /// `behaviour.state` was reconciled by `name` before #911; it is not any
    /// more, and it must not be.
    ///
    /// The FSM was dissolved in #572: `BehaviourConfig` is
    /// `deny_unknown_fields` with no `state` field, so a resolved document
    /// carrying `[[behaviour.state]]` cannot parse and no shipped hull or
    /// fragment has one. #911 retired the special case rather than generalising
    /// a corpse. The `name`-keyed MECHANISM is alive and tested through
    /// `[[station.rating]]` — see `nested_arrays_reconcile_under_a_composed_chain`.
    ///
    /// Re-pointed rather than deleted so the retirement is a checked claim: if
    /// `state` ever comes back, this fails and sends the author to the identity
    /// table.
    #[test]
    fn state_is_retired_and_no_longer_merges_by_name_across_fragments() {
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
        assert_eq!(
            states.len(),
            1,
            "an array with no identity entry replaces wholesale"
        );
        assert_eq!(states[0].get("name").unwrap().as_str(), Some("patrol"));
        assert!(
            EntityConfig::from_toml(&r.toml).is_err(),
            "and the resolved document does not parse either way — which is why \
             there was nothing to generalise"
        );
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

    fn strings_at(r: &ResolvedTemplate, key: &str) -> Vec<String> {
        r.value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn ids_at(r: &ResolvedTemplate, key: &str) -> Vec<String> {
        r.value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// #869 asserted that `tags` REPLACED between fragments. #911 changes that
    /// deliberately: `tags` has no key, so at the compose layer it can only
    /// union, and union is what a fragment library needs.
    ///
    /// This is a behaviour change confined to the compose layer.
    /// `entity_override::instance_override_tags_replace_they_do_not_union` pins
    /// the other half, which is the one three shipped worlds depend on.
    #[test]
    fn tags_union_between_fragments() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/base.toml", "tags = [\"ship\", \"npc\"]\n"),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\ntags = [\"npc\", \"scenery\"]\n",
                ),
            ],
        );
        assert_eq!(
            strings_at(&r, "tags"),
            vec!["ship", "npc", "scenery"],
            "the hull ADDS to the fragment's tags; a tag both declare is not \
             duplicated"
        );
    }

    /// …and an authored empty array is still a fragment's lever to clear them.
    #[test]
    fn a_fragment_authoring_empty_tags_clears_them() {
        let r = resolve(
            "e/hull.toml",
            &[
                ("e/base.toml", "tags = [\"ship\", \"npc\"]\n"),
                ("e/hull.toml", "includes = [\"base.toml\"]\ntags = []\n"),
            ],
        );
        assert!(strings_at(&r, "tags").is_empty());
    }

    /// Arrays with no stable identity keep replacing wholesale between
    /// fragments. A fragment contributing an AI policy contributes it WHOLE.
    #[test]
    fn keyless_arrays_still_replace_wholesale_between_fragments() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    "[[captain_console.ai.rule]]\nchannel = \"a\"\npriority = 1\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\n[[captain_console.ai.rule]]\nchannel = \"b\"\npriority = 2\n",
                ),
            ],
        );
        let rules = r
            .value
            .get("captain_console")
            .unwrap()
            .get("ai")
            .unwrap()
            .get("rule")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].get("channel").unwrap().as_str(), Some("b"));
    }

    // ── Array extension across a composed chain (issue #911) ─────────────────

    /// The issue in one test: "the library's systems, plus two of my own."
    #[test]
    fn a_hull_extends_a_fragments_system_suite_instead_of_replacing_it() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/systems.toml",
                    "[[system]]\nid = \"helm-thrust\"\nkind = \"helm_thrust\"\n\
                     [[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"systems.toml\"]\n\
                     [[system]]\nid = \"phaser-dorsal\"\nkind = \"phaser_bank\"\n",
                ),
            ],
        );
        assert_eq!(
            ids_at(&r, "system"),
            vec!["helm-thrust", "power-reactor", "phaser-dorsal"],
            "a hull needing one extra system no longer has to restate the suite \
             — and so no longer silently opts out of future library changes"
        );
    }

    /// Replace-in-place and remove, through a THREE-deep chain, so the rules
    /// are shown to survive an intermediate fragment rather than only working
    /// between a hull and its direct include.
    #[test]
    fn a_composed_chain_specialises_and_removes_inherited_entries() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/library.toml",
                    "[[system]]\nid = \"helm-thrust\"\nkind = \"helm_thrust\"\nai_only = true\n\
                     [[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n\
                     [[system]]\nid = \"legacy-probe\"\nkind = \"sensor_probe\"\n",
                ),
                (
                    "e/class.toml",
                    "includes = [\"library.toml\"]\n\
                     [[system]]\nid = \"legacy-probe\"\n_remove = true\n\
                     [[system]]\nid = \"phaser-dorsal\"\nkind = \"phaser_bank\"\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"class.toml\"]\n\
                     [[system]]\nid = \"helm-thrust\"\nai_only = false\n",
                ),
            ],
        );
        assert_eq!(
            ids_at(&r, "system"),
            vec!["helm-thrust", "power-reactor", "phaser-dorsal"],
            "the mid fragment's removal and append both survive to the hull"
        );
        let thrust = &r.value.get("system").unwrap().as_array().unwrap()[0];
        assert_eq!(thrust.get("ai_only").unwrap().as_bool(), Some(false));
        assert_eq!(
            thrust.get("kind").unwrap().as_str(),
            Some("helm_thrust"),
            "a key only the library declared survives two levels of merge"
        );
        assert!(
            !r.toml.contains(crate::entity_override::REMOVE_KEY),
            "the tombstone marker must never reach the resolved document"
        );
    }

    /// A tombstone in the FIRST fragment of a closure never meets a merge — it
    /// is the value the accumulator is seeded with. It must still be stripped,
    /// which is what the resolver's own strip site is for.
    #[test]
    fn an_unmatched_tombstone_never_reaches_the_resolved_document() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/frag.toml",
                    "[[system]]\nid = \"never-declared\"\n_remove = true\n\
                     [[system]]\nid = \"real\"\nkind = \"power_reactor\"\n",
                ),
                ("e/hull.toml", "includes = [\"frag.toml\"]\nclass = \"x\"\n"),
            ],
        );
        assert_eq!(ids_at(&r, "system"), vec!["real"]);
        assert!(!r.toml.contains(crate::entity_override::REMOVE_KEY));
        assert!(!r
            .value
            .to_string()
            .contains(crate::entity_override::REMOVE_KEY));
    }

    /// The one document the merge cannot clean: an UNCOMPOSED root with a
    /// tombstone. It never meets an accumulator, so the resolver's own strip
    /// site is the only thing standing between it and `value`.
    ///
    /// Authoring a tombstone here is a mistake (there is nothing inherited to
    /// remove), and the resolved `toml` is served verbatim from `root_text`, so
    /// `parse()` rejects it loudly. What must NOT happen is `value` and `toml`
    /// disagreeing about whether the marker is there.
    #[test]
    fn an_uncomposed_template_never_leaks_a_tombstone_into_its_value() {
        let body = "class = \"solo\"\n[[system]]\nid = \"ghost\"\n_remove = true\n";
        let r = resolve("e/hull.toml", &[("e/hull.toml", body)]);
        assert!(!r.is_composed());
        assert!(
            !r.value
                .to_string()
                .contains(crate::entity_override::REMOVE_KEY),
            "no `_remove` may survive into the resolved value, composed or not"
        );
        assert_eq!(
            r.toml, body,
            "an uncomposed template is still served verbatim — byte-identity is \
             not traded away for the strip"
        );
        assert!(
            r.parse().is_err(),
            "and the verbatim bytes still carry the marker, so the mistake is \
             rejected rather than silently absorbed"
        );
    }

    /// Nested arrays under a composed chain: `[[station.rating]]` reconciles by
    /// `name` INSIDE a `[[station]]` reconciled by `id`.
    #[test]
    fn nested_arrays_reconcile_under_a_composed_chain() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/frag.toml",
                    "[[station]]\nid = \"bridge\"\n\
                     [[station.rating]]\nname = \"helm\"\nlevel = 1\n\
                     [[station.rating]]\nname = \"tactical\"\nlevel = 1\n\
                     [[station]]\nid = \"engineering\"\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"frag.toml\"]\n\
                     [[station]]\nid = \"bridge\"\n\
                     [[station.rating]]\nname = \"tactical\"\nlevel = 3\n",
                ),
            ],
        );
        assert_eq!(ids_at(&r, "station"), vec!["bridge", "engineering"]);
        let ratings = r.value.get("station").unwrap().as_array().unwrap()[0]
            .get("rating")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(ratings.len(), 2, "the unmentioned rating survives");
        assert_eq!(ratings[0].get("level").unwrap().as_integer(), Some(1));
        assert_eq!(ratings[1].get("level").unwrap().as_integer(), Some(3));
    }

    /// `[[shield_arc]]` order is load-bearing (`ShieldSystem::from_arcs` maps
    /// arcs positionally; the FIRST arc's frequency seeds the ship-wide shield
    /// frequency). Stated as a guarantee of composition, not left to chance.
    #[test]
    fn shield_arc_order_survives_composition() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/frag.toml",
                    "[[shield_arc]]\nid = \"fore\"\nfrequency = 1.0\n\
                     [[shield_arc]]\nid = \"aft\"\nfrequency = 2.0\n",
                ),
                (
                    // Specialises the FIRST arc: an override that only touched
                    // the last one would pass even if matched entries moved.
                    "e/hull.toml",
                    "includes = [\"frag.toml\"]\n\
                     [[shield_arc]]\nid = \"fore\"\nfrequency = 9.0\n\
                     [[shield_arc]]\nid = \"dorsal\"\nfrequency = 5.0\n",
                ),
            ],
        );
        assert_eq!(
            ids_at(&r, "shield_arc"),
            vec!["fore", "aft", "dorsal"],
            "specialised arcs hold their template position; new arcs append AFTER"
        );
        let arcs = r.value.get("shield_arc").unwrap().as_array().unwrap();
        assert_eq!(
            arcs[0].get("frequency").unwrap().as_float(),
            Some(9.0),
            "the ship-wide shield frequency is seeded from whichever arc is \
             FIRST, so composition must not reorder them"
        );
        assert_eq!(arcs[1].get("id").unwrap().as_str(), Some("aft"));
    }

    /// Provenance is driven from the SAME identity table as the merge. If it
    /// were not, `system` would be recorded as a wholesale leaf and
    /// `insert_leaf`'s prune would erase every field the library fragment
    /// contributed to the systems it did not touch.
    #[test]
    fn provenance_addresses_every_reconciled_array_by_key_not_just_doctrine() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/frag.toml",
                    "[[system]]\nid = \"helm-thrust\"\nkind = \"helm_thrust\"\nai_only = true\n\
                     [[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"frag.toml\"]\n[[system]]\nid = \"helm-thrust\"\nai_only = false\n",
                ),
            ],
        );
        assert_eq!(
            r.provenance
                .origin("system[id=helm-thrust].ai_only")
                .expect("the hull's specialisation is recorded")
                .source,
            "e/hull.toml"
        );
        assert_eq!(
            r.provenance
                .origin("system[id=helm-thrust].kind")
                .expect("a key the hull never mentioned is still recorded")
                .source,
            "e/frag.toml",
            "if provenance recorded `system` as a wholesale leaf, this field \
             would have been pruned"
        );
        assert_eq!(
            r.provenance
                .origin("system[id=power-reactor].kind")
                .expect("an untouched sibling is still recorded")
                .source,
            "e/frag.toml"
        );
    }

    /// A removal is the opposite of authoring: provenance must stop reporting
    /// fields of an entry that no longer exists.
    #[test]
    fn provenance_prunes_an_entry_a_later_fragment_removed() {
        let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/frag.toml",
                    "[[system]]\nid = \"legacy\"\nkind = \"sensor_probe\"\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"frag.toml\"]\n[[system]]\nid = \"legacy\"\n_remove = true\n",
                ),
            ],
        );
        assert!(ids_at(&r, "system").is_empty());
        assert!(
            r.provenance.origin("system[id=legacy].kind").is_none(),
            "a removed entry must not still report a field that no longer exists"
        );
        assert!(
            r.provenance.origin("system[id=legacy]._remove").is_none(),
            "the marker is authoring metadata, not an authored field"
        );
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

    /// The `[[mesh.lod]]` relocation guard (issue #914) runs on the RESOLVED
    /// document, so a fragment library entry that still authors the banned
    /// location is caught exactly like a shipped hull would be — it cannot
    /// hide behind composition and slip an old-style ladder into every hull
    /// that includes it.
    #[test]
    fn a_fragment_carrying_relocated_mesh_lod_is_rejected_with_the_targeted_message() {
        const FRAGMENT: &str = r#"
[mesh]
model = "assets/models/rock.glb"
variant = "small"
shape = "sphere"
colour = [0.5, 0.5, 0.5]
radius = 2.0

[[mesh.lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
"#;
        let resolved = resolve(
            "e/hull.toml",
            &[
                ("e/frag.toml", FRAGMENT),
                ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
            ],
        );
        let err = resolved
            .parse()
            .expect_err("[[mesh.lod]] authored by a fragment must still be rejected");
        assert_eq!(err.category(), "include-invalid-template");
        assert!(
            err.message().contains("assets/models/rock.small.toml"),
            "the error must name the sidecar the chain moved to; got: {}",
            err.message()
        );
        assert!(
            err.message().contains("[[lod]]"),
            "the error must name the new block; got: {}",
            err.message()
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

    // ── The shipped tree (issue #906) ────────────────────────────────────────
    //
    // The byte-stability tests above prove the MECHANISM: `resolve_with` hands
    // back the root text untouched when nothing was composed, so an uncomposed
    // template is never round-tripped through `toml::to_string`. These prove
    // the same thing over the content that actually ships, and pin the one
    // condition under which the `include_str!` sites that bake hull bytes into
    // the binary are allowed to stay as they are.
    #[cfg(not(target_arch = "wasm32"))]
    mod shipped_tree {
        use super::*;

        /// Every shipped hull: the `.toml` files directly in
        /// `assets/entities/`, repo-relative with forward slashes (the form
        /// every template path is authored in).
        ///
        /// Deliberately NOT recursive. `assets/entities/fragments/` is the
        /// fragment tree — nothing there is spawnable, and it already holds a
        /// composed mechanism fixture (`composed_escort.toml`), so a recursive
        /// walk would assert byte-identity over the one file that must not have
        /// it.
        fn shipped_templates() -> Vec<String> {
            let dir = std::path::Path::new("assets/entities");
            let mut out: Vec<String> = std::fs::read_dir(dir)
                .expect("assets/entities must be readable")
                .map(|e| e.expect("readable dir entry").path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect();
            out.sort();
            out
        }

        /// Dotted provenance prefixes a HULL must author for itself, whatever
        /// it composes. Matched on segment boundaries, so `power.capacity` does
        /// not also claim `power.ai_policy`.
        ///
        /// This is the hazard list: every authoring surface the spawner gates a
        /// real capability on, where composition bringing a parent table into
        /// existence merely by authoring a child of it would hand an includer
        /// equipment it never authored. See
        /// [`the_composed_destroyer_takes_only_ai_policy_from_its_fragments`]
        /// for what each entry does when a fragment supplies it.
        ///
        /// Module-level rather than inline in that test (issue #875 review) so
        /// the shipped-tree walk can apply it to EVERY composed hull. The
        /// destroyer's own test keeps the parts that only make sense for one
        /// hull — the reactor values, the coverage floor, player-flyability.
        const HULL_OWNED: [&str; 17] = [
            "tags",
            "faction",
            "system",
            "station",
            "shield_arc",
            "hull",
            "mesh",
            "collider",
            "comms",
            "torpedoes",
            "weapons_console.phaser_banks",
            "weapons_console.blaster_banks",
            "shields_console.base",
            "repair.repair_team_count",
            // The reactor scalars, per prefix rather than as a bare `power`:
            // `fleet_baseline.toml` really does carry `capacity = 90`,
            // `rates` and `emergency_threshold = 22` so that its
            // `[power.ai_policy]` has a table to sit in, and none of the three
            // has a parse-time default. A future hull that includes that
            // fragment and authors no `[power]` of its own would silently
            // inherit a 90-capacity reactor — the exact silent-equipment class
            // this list exists to catch. Bare `power` cannot be used:
            // `power.ai_policy` is precisely what a hull is MEANT to take from
            // the fragment library.
            "power.capacity",
            "power.rates",
            "power.emergency_threshold",
        ];

        /// Whether a provenance path falls under `prefix`, on segment
        /// boundaries: `system[id=helm-thrust].ai_only` starts with `system`
        /// and then a delimiter; `systems_foo` must not match `system`.
        fn owned(path: &str, prefix: &str) -> bool {
            path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
        }

        /// Assert that no fragment authored any hazard-list surface of
        /// `resolved`, whose root template is `path`.
        ///
        /// Returns how many fields it checked, so a caller can pin that the
        /// provenance addressing has not changed shape underneath it.
        fn assert_no_fragment_supplied_equipment(path: &str, resolved: &ResolvedTemplate) -> usize {
            let mut checked = 0usize;
            for (field, origin) in resolved.provenance.fields() {
                if !HULL_OWNED.iter().any(|p| owned(field, p)) {
                    continue;
                }
                checked += 1;
                assert_eq!(
                    origin.source, path,
                    "`{field}` in the resolved {path} was authored by {}, not by \
                     the hull. A shared AI-policy fragment must never contribute \
                     an authoring surface the spawner gates a capability on — \
                     see `the_composed_destroyer_takes_only_ai_policy_from_its_fragments` \
                     for what each one does. Include chain: {:?}",
                    origin.source, origin.chain
                );
            }
            checked
        }

        /// A directory walk over every shipped template, asserting the property
        /// that actually holds of each: an UNCOMPOSED hull resolves
        /// byte-identically to its own file contents, and a COMPOSED one
        /// resolves to a document that loads.
        ///
        /// No snapshot files: for the uncomposed majority the file IS the
        /// expectation, and for a composed hull the expectation is that the
        /// resolved document — not the file — is what the game can spawn.
        ///
        /// # Why this is now a partition (issue #875)
        ///
        /// Until the player destroyer composed the fragment library, no shipped
        /// hull declared `includes` and byte-identity held over the whole tree.
        /// This test's own failure message said what to do the day one did:
        /// drop it from the byte walk and assert on the resolved document
        /// instead. That is what the two arms below are.
        ///
        /// Neither arm is weaker than what it replaced. The uncomposed arm is
        /// the same assertion over the same files. The composed arm is
        /// STRONGER than byte-identity would have been, because byte-identity
        /// never said the document parses — and a composed hull is exactly
        /// where a merge can produce a document that no author ever read.
        #[test]
        fn every_shipped_template_resolves_to_its_own_bytes() {
            let templates = shipped_templates();
            assert!(
                templates.len() > 20,
                "the walk found only {} templates — it is not reaching the shipped \
                 content it is supposed to be guarding",
                templates.len()
            );
            let mut uncomposed = 0usize;
            let mut composed: Vec<&String> = Vec::new();
            for path in &templates {
                let on_disk = std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("{path} must be readable: {e}"));
                let resolved =
                    resolve_from_disk(path).unwrap_or_else(|e| panic!("{path} must resolve: {e}"));
                if resolved.is_composed() {
                    composed.push(path);
                    // The resolved document, not the file, is what spawns — so
                    // that is what has to be a valid entity. `EntityConfig` is
                    // `deny_unknown_fields`, so this also proves the `includes`
                    // key never survives resolution.
                    EntityConfig::from_toml(&resolved.toml).unwrap_or_else(|e| {
                        panic!(
                            "{path} is composed, and its RESOLVED document must load: {e}\n\
                             The file on disk is only half the hull; read the merge, \
                             not the file."
                        )
                    });
                    assert_ne!(
                        resolved.toml, on_disk,
                        "{path} declares `includes` but resolved to its own bytes \
                         unchanged, so the fragments it names contributed nothing. \
                         Either the include list is dead or the merge stopped working."
                    );
                    // And the hazard list (issue #875 review): "parses and
                    // differs" is a weak thing to say about a composed hull
                    // next to the strong provenance pin the destroyer gets from
                    // its own test. Applying that pin here means the NEXT hull
                    // to compose inherits it on the day it declares `includes`,
                    // rather than the day someone remembers to write it a test.
                    let checked = assert_no_fragment_supplied_equipment(path, &resolved);
                    // The pin has to have addressed SOMETHING. A composed hull
                    // whose provenance keys changed shape would iterate nothing
                    // and pass vacuously — the guard would report success on the
                    // day it stopped guarding. Every hull authors systems, a
                    // hull and a faction, so a real one clears this by two
                    // orders of magnitude; the floor is deliberately low enough
                    // that a small composed hull does not have to be exempted.
                    assert!(
                        checked > 10,
                        "only {checked} hull-owned fields were checked in {path}, \
                         so the provenance addressing has changed shape and this \
                         guard is matching (almost) nothing"
                    );
                    continue;
                }
                uncomposed += 1;
                assert_eq!(
                    resolved.toml, on_disk,
                    "{path} no longer resolves to its own bytes but does not declare \
                     `includes` either, so the resolver is rewriting a template it \
                     was asked only to pass through. That is a resolver bug, not a \
                     composition: `resolve_with` hands back the root text untouched \
                     when nothing was composed, and this is the assertion that says so."
                );
            }
            // The byte-identity arm must still be doing real work. If the whole
            // tree ever became composed this test would be asserting nothing
            // about pass-through, which is the property the resolver is most
            // likely to break silently.
            //
            // A RATIO, not a floor. An absolute `uncomposed > 20` is satisfied
            // by 21 pass-through files however large the tree grows, so a fleet
            // that went two-thirds composed would keep this arm nominally alive
            // while it had stopped describing the tree at all. Requiring the
            // pass-through arm to cover the MAJORITY ties the guard to the
            // shape of the content rather than to today's file count — and the
            // day the majority genuinely composes (issue #878), this failing is
            // the prompt to retire the arm rather than to raise a number.
            assert!(
                uncomposed * 2 > templates.len(),
                "only {uncomposed} of {} shipped templates are uncomposed, so the \
                 pass-through arm of this walk no longer covers most of the tree \
                 and has stopped meaningfully guarding it. Composed: {composed:?}",
                templates.len()
            );
        }

        /// **Issue #875 AC4, as provenance: fragments apply only to the systems
        /// a hull actually owns.**
        ///
        /// The player destroyer takes AI POLICY from the fragment library and
        /// nothing else. Every authoring surface the spawner gates a real
        /// capability on must still be authored by the hull itself.
        ///
        /// # Why provenance rather than a shape assertion
        ///
        /// The failure this guards is not "the hull came out wrong" — it is "a
        /// fragment quietly supplied something", which a shape assertion cannot
        /// tell from the hull supplying it. Composition brings a parent table
        /// into existence merely by authoring a child of it, and Rust gates on
        /// the parent, so a shared AI fragment is one careless line away from
        /// handing every hull that includes it equipment it never authored:
        ///
        /// * `tags` UNIONS at compose (unlike at the instance-override layer),
        ///   so a fragment carrying `"npc"` would flip a player hull out of
        ///   `marker_validate::is_player_flyable` — silently, since nothing
        ///   about the resolved document would look wrong;
        /// * a bare `[torpedoes.ai]` creates `[torpedoes]`, i.e. a torpedo
        ///   system with zero tubes;
        /// * `[shields_console.base]` or a `[[shield_arc]]` gives a hull
        ///   shields;
        /// * `repair_team_count` hands teams to a hull that must not have them;
        /// * `[[hull.system_hull]]` is NOT a keyed array, so a fragment touching
        ///   it REPLACES all fourteen of this hull's entries.
        ///
        /// [`HULL_OWNED`] is that hazard list, turned into an assertion. A
        /// future fragment that starts contributing one of these fails with the
        /// field named — here for this hull, and in
        /// [`every_shipped_template_resolves_to_its_own_bytes`] for every hull
        /// that composes later. What stays here is what only makes sense for
        /// one hull: the reactor VALUES, the coverage floor, player-flyability.
        #[test]
        fn the_composed_destroyer_takes_only_ai_policy_from_its_fragments() {
            const HULL: &str = "assets/entities/alliance_destroyer.toml";
            // The reactor scalars have no parse-time defaults, and
            // `fleet_baseline.toml` also carries them (at other values) so that
            // its `[power.ai_policy]` has a table to sit in. The hull's own must
            // win, or this ship silently gains the fragment's reactor.
            const HULL_REACTOR: [(&str, f64); 2] = [
                ("power.capacity", 70.0),
                ("power.emergency_threshold", 20.0),
            ];

            let resolved = resolve_from_disk(HULL).expect("the player destroyer must resolve");
            assert!(
                resolved.is_composed(),
                "this test is about what COMPOSITION contributed; if the hull \
                 stopped declaring `includes` it proves nothing"
            );

            let checked = assert_no_fragment_supplied_equipment(HULL, &resolved);
            assert!(
                checked > 100,
                "only {checked} hull-owned fields were checked, so the provenance \
                 addressing has changed shape and this guard is matching nothing"
            );

            for (path, want) in HULL_REACTOR {
                let origin = resolved
                    .provenance
                    .origin(path)
                    .unwrap_or_else(|| panic!("`{path}` must be authored somewhere"));
                assert_eq!(
                    origin.source, HULL,
                    "`{path}` came from {} — the hull's own reactor must win over \
                     any a fragment carries",
                    origin.source
                );
                let config = resolved.parse().expect("the resolved hull must parse");
                let power = config.power.as_ref().expect("the hull authors [power]");
                let got = match path {
                    "power.capacity" => power.capacity as f64,
                    _ => power.emergency_threshold as f64,
                };
                assert_eq!(got, want, "`{path}` resolved to the wrong value");
            }

            // The shape the provenance check exists to protect, stated once so a
            // reader can see what "the systems a hull actually owns" means here.
            let config = resolved.parse().expect("the resolved hull must parse");
            assert!(
                crate::entities::marker_validate::is_player_flyable(&config),
                "the player destroyer must still be player-flyable — this is what \
                 a fragment carrying `tags = [\"npc\"]` would silently break"
            );
            assert_eq!(config.tags, vec!["ship".to_string()]);
            let ship = config
                .ship_config
                .as_ref()
                .expect("the hull declares [[system]] blocks");
            let stations: Vec<&str> = ship.stations.iter().map(|s| s.id.0.as_str()).collect();
            assert_eq!(
                stations,
                vec!["captain", "helm", "tactical", "engineering"],
                "four seats, exactly as authored — no fragment adds or removes one"
            );
            let arcs: Vec<&str> = config
                .shield_arcs
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                arcs,
                vec!["fore", "aft"],
                "TWO arcs, not four. A shared shields fragment that assumed the \
                 four-arc layout would add two this hull never authored."
            );
        }

        /// Where a baked asset path sits in the tree.
        ///
        /// `assets/entities/*.toml` is a spawnable HULL; anything under
        /// `assets/entities/fragments/` is a FRAGMENT, which is not spawnable
        /// and is the thing hulls compose FROM. The scan below reports the two
        /// separately, because "this file bakes a composed hull" and "this file
        /// bakes a fragment that has itself grown includes" send an author to
        /// completely different fixes.
        fn is_fragment(asset_path: &str) -> bool {
            asset_path.starts_with("assets/entities/fragments/")
        }

        /// The literal inside `include_str!( … "…" )`, and how far past it to
        /// resume scanning, given the text immediately after `include_str!(`.
        ///
        /// `None` when the macro's argument is not a plain string literal — a
        /// `concat!`, a `const`, a nested macro — none of which bake a path
        /// this scan can name.
        fn baked_literal(after_open: &str) -> Option<(&str, usize)> {
            let quote = after_open.find(|c: char| !c.is_whitespace())?;
            if after_open.as_bytes()[quote] != b'"' {
                return None;
            }
            let body = &after_open[quote + 1..];
            let end = body.find('"')?;
            Some((&body[..end], quote + 1 + end))
        }

        /// Shipped entity-asset paths baked into the binary by `include_str!`,
        /// paired with the source file that bakes them.
        ///
        /// Tolerates rustfmt's wrapping: a long path is routinely pushed onto
        /// the line after `include_str!(`, so the scan skips whitespace before
        /// expecting the opening quote. Searching for the contiguous bytes
        /// `include_str!("` instead would silently miss every wrapped site —
        /// and it is the wrapped ones, being the long paths, that are most
        /// likely to be assets.
        ///
        /// Walks BOTH crate source roots. `tests/` is not decoration: the
        /// headless runner's integration tests bake a hull with
        /// `include_str!("../assets/entities/…")` exactly as `src/` does, and a
        /// scan that only saw `src/` would leave those sites unenumerated — the
        /// AC asks for every site to be named or excused, and a site the scan
        /// cannot see is neither. Any future source root (`benches/`,
        /// `examples/`) belongs in this list for the same reason.
        const SOURCE_ROOTS: [&str; 2] = ["src", "tests"];

        /// Alongside the sites, returns how many `.rs` files the WALK ITSELF
        /// read under each of `SOURCE_ROOTS` — the "did the scan reach this
        /// root at all" reading, which has to come from this walk rather than
        /// a second, separately-written one: a standalone directory walk would
        /// prove only that the root exists and holds `.rs` files, not that the
        /// enumeration above ever looked at them. It would pass unchanged if
        /// `SOURCE_ROOTS` were trimmed to `["src"]`, or if this walk grew a bug
        /// that returned early. Threading the count through the same recursion
        /// the sites come from ties the "reached" evidence to the thing it is
        /// evidence for.
        fn include_str_baked_hulls() -> (Vec<(String, String)>, HashMap<&'static str, usize>) {
            fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>, read: &mut usize) {
                let entries = std::fs::read_dir(dir)
                    .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
                for entry in entries {
                    let path = entry.expect("readable dir entry").path();
                    if path.is_dir() {
                        walk(&path, out, read);
                        continue;
                    }
                    if path.extension().is_none_or(|e| e != "rs") {
                        continue;
                    }
                    *read += 1;
                    let file = path.to_string_lossy().replace('\\', "/");
                    let src = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("{file} must be readable: {e}"));
                    let mut rest = src.as_str();
                    while let Some(i) = rest.find("include_str!(") {
                        rest = &rest[i + "include_str!(".len()..];
                        // Not a plain string literal (a `concat!`, a macro
                        // argument): nothing to bake, and `rest` has already
                        // advanced, so the walk cannot stall here.
                        let Some((literal, consumed)) = baked_literal(rest) else {
                            continue;
                        };
                        // The literal is relative to the declaring FILE; the
                        // repo-relative form is whatever follows the assets
                        // prefix, which is unambiguous.
                        if let Some(j) = literal.find("assets/entities/") {
                            out.push((file.clone(), literal[j..].to_string()));
                        }
                        rest = &rest[consumed..];
                    }
                }
            }
            let mut out = Vec::new();
            let mut read_per_root: HashMap<&'static str, usize> = HashMap::new();
            for root in SOURCE_ROOTS {
                let mut read = 0usize;
                walk(std::path::Path::new(root), &mut out, &mut read);
                read_per_root.insert(root, read);
            }
            out.sort();
            out.dedup();
            (out, read_per_root)
        }

        /// THE EXCUSE for the `include_str!` sites, recorded where a future
        /// author trips over it (issue #906).
        ///
        /// `include_str!` bakes a hull's bytes into the binary at COMPILE time.
        /// There is no seam at which resolution could run: the resolver needs a
        /// fragment source at runtime, so a baked template can never see a
        /// resolved document. Migrating every site would mean turning each into
        /// a disk load inside the test body — a large mechanical change to
        /// tests that mostly assert on ONE authored field of ONE hull and are
        /// all correct today.
        ///
        /// So they are excused, on a condition this test enforces: **every hull
        /// reached by an `include_str!` must be uncomposed.** While that holds,
        /// the baked bytes and the resolved document are the same text (proved
        /// by `every_shipped_template_resolves_to_its_own_bytes` above) and the
        /// excuse costs nothing. The moment #875/#878 compose one of these
        /// hulls, this test names the exact sites that must move — strictly
        /// better than a frozen list, because it covers `include_str!` sites
        /// added after this was written too.
        #[test]
        fn include_str_baked_hulls_are_all_uncomposed() {
            let (baked, read_per_root) = include_str_baked_hulls();
            // The floor is a "did the scan actually run" check, not a budget. It
            // stood at 20 until issue #878 composed the five Harrow hulls and
            // moved every site that baked one onto the resolving load path — a
            // little over half the sites in the tree, and exactly the migration
            // this test's own doc comment predicted. Lower it again only
            // alongside another such migration, never to make a red run green.
            assert!(
                baked.len() >= 8,
                "the source scan found only {} baked hull sites — it has stopped \
                 finding them, so it is guarding nothing",
                baked.len()
            );
            // Every source root must actually be REACHED by the SCAN ITSELF, or
            // the enumeration this AC rests on is silently partial. `tests/` is
            // the one that was missed first time round: `tests/headless_runner.rs`
            // baked a hull through `../assets/entities/…`, and a src-only scan
            // would have excused a site it had never looked at.
            //
            // Asserted on the walk's own per-root read count (from
            // `include_str_baked_hulls`) rather than on a baked site being found
            // there, because issue #878 composed the five Harrow hulls and
            // `tests/headless_runner.rs`'s two sites — both Harrow — moved onto
            // the resolving load path. A root with no baked site left is not a
            // root the scan cannot see, and conflating the two would have this
            // guard fail for the very migration it exists to demand. It is also
            // asserted on the SAME walk rather than a second, independently
            // written directory count: a re-implemented walk would prove the
            // root has `.rs` files, not that this scan reaches them — trimming
            // `SOURCE_ROOTS` to `["src"]` would still pass that.
            //
            // Spelled out as literals rather than read from `SOURCE_ROOTS`:
            // deriving them would let the guard shrink in step with the thing it
            // is guarding, which is exactly the regression to catch.
            for root in ["src", "tests"] {
                assert!(
                    read_per_root.get(root).is_some_and(|&n| n > 0),
                    "the scan itself read zero .rs files under {root}/ — either the \
                     root has been renamed and SOURCE_ROOTS is stale, or the walk \
                     never reached it, so the scan is looking at nothing there"
                );
            }
            assert!(
                baked.iter().any(|(site, _)| site.starts_with("src/")),
                "no baked `include_str!` site was found under src/ — the scan has \
                 stopped parsing. Sites found: {:?}",
                baked.iter().map(|(s, _)| s).collect::<Vec<_>>()
            );
            let mut composed: Vec<String> = Vec::new();
            for (site, asset) in &baked {
                let resolved = resolve_from_disk(asset)
                    .unwrap_or_else(|e| panic!("{site} bakes {asset}, which must resolve: {e}"));
                if resolved.is_composed() {
                    // Naming the KIND matters: a composed hull sends the author
                    // to the resolved document, a composed fragment sends them
                    // to the fragment tree. Reporting a fragment as a hull is
                    // the wrong diagnosis.
                    let kind = if is_fragment(asset) {
                        "composed fragment"
                    } else {
                        "composed hull"
                    };
                    composed.push(format!("{site} bakes {kind} {asset}"));
                }
            }
            assert!(
                composed.is_empty(),
                "these `include_str!` sites bake an entity asset that is now COMPOSED, \
                 so they assert on unresolved text. Replace each with a disk load \
                 through `entity_includes::load_entity_config` (or `resolve_from_disk` \
                 where the raw text is needed for line lookups). A `composed fragment` \
                 is a different diagnosis from a `composed hull`: the fragment tree \
                 has grown a level, so check what ELSE includes it before changing \
                 the site:\n{}",
                composed.join("\n")
            );
        }

        /// **The RUNTIME twin of [`include_str_baked_hulls_are_all_uncomposed`].**
        ///
        /// That test catches a hull whose bytes are baked at COMPILE time and
        /// then parsed as if they were the whole document. It cannot see the
        /// other half of the same mistake: text read at RUN time and parsed the
        /// same way. No source scan can enumerate those — the path is a variable,
        /// not a literal — so they have to be caught by driving the entry point.
        ///
        /// There is one such entry point over shipped hulls, and it broke twice.
        /// `server::bridge::validate_ship_stations` is the browser's pre-start
        /// ship gate: `server.html` fetches the chosen hull and hands the gate
        /// the path and those raw bytes, and nothing may start until it passes.
        /// While it parsed the fetched text directly, the first hull to declare
        /// `includes` became unselectable in the browser — with the native and
        /// headless suites, and every other CI gate, staying green, because the
        /// only host that reads a hull as raw text is the browser. The failure
        /// surfaced as a crashed page in Playwright.
        ///
        /// So: every crewed shipped hull, through the real gate, over exactly
        /// what the page passes it. A hull with `[[station]]` blocks is one a
        /// host can pick, and a hull a host can pick must boot.
        ///
        /// # What makes this non-vacuous
        ///
        /// The `composed` count. Over an all-uncomposed tree this test passes
        /// just as happily against a gate that parses the raw text — it would
        /// report success on exactly the tree where it guards nothing. Requiring
        /// at least one COMPOSED crewed hull ties the guard to the thing it is
        /// guarding: the difference between the authored file and the document
        /// the game runs.
        #[test]
        fn every_shipped_hull_passes_the_browser_station_gate() {
            let mut crewed: Vec<&String> = Vec::new();
            let mut composed = 0usize;
            let templates = shipped_templates();
            for path in &templates {
                let raw = std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("{path} must be readable: {e}"));
                let resolved =
                    resolve_from_disk(path).unwrap_or_else(|e| panic!("{path} must resolve: {e}"));
                // Read the crewed-hull predicate off the RESOLVED document, not
                // off a parse: a template that fails to parse must reach the
                // gate and be reported by it, not skipped by this filter.
                if resolved.value.get("station").is_none() {
                    continue;
                }
                crewed.push(path);
                if resolved.is_composed() {
                    composed += 1;
                }
                crate::server::bridge::validate_ship_stations(path, &raw).unwrap_or_else(|e| {
                    panic!(
                        "{path} declares `[[station]]`, so a host can select it and the \
                         browser's pre-start gate must accept it. Driven over exactly what \
                         `server.html` passes — the template path and the RAW bytes at that \
                         path — it did not: {e}\n\
                         If this says `unknown field `includes``, the gate is parsing the \
                         authored file instead of resolving it: the file on disk is only \
                         half a composed hull."
                    )
                });
            }
            assert!(
                crewed.len() >= 8,
                "only {} shipped templates declare `[[station]]`, so this walk has \
                 stopped finding the crewed hulls it is supposed to be driving \
                 through the gate: {crewed:?}",
                crewed.len()
            );
            assert!(
                composed > 0,
                "none of the {} crewed hulls this gate accepted is COMPOSED, so a gate \
                 that parsed the raw authored text would pass this test unchanged and \
                 it has stopped guarding anything. Composition of a shipped hull is \
                 what it exists to catch.",
                crewed.len()
            );
        }

        /// A fragment is not a hull, and the scan must not call one the other.
        ///
        /// `src/world/validate.rs` bakes `fragments/ai/fleet_baseline.toml`,
        /// which is a FRAGMENT. If a fragment ever grows its own `includes`,
        /// reporting it as a composed *hull* would send the author looking for
        /// a spawnable template that does not exist — the wrong diagnosis, and
        /// the wrong fix.
        #[test]
        fn the_scan_tells_a_fragment_apart_from_a_hull() {
            assert!(is_fragment(
                "assets/entities/fragments/ai/fleet_baseline.toml"
            ));
            assert!(!is_fragment("assets/entities/alliance_cruiser.toml"));
            let (baked, _read_per_root) = include_str_baked_hulls();
            let fragments: Vec<&(String, String)> =
                baked.iter().filter(|(_, a)| is_fragment(a)).collect();
            assert!(
                !fragments.is_empty(),
                "no baked site reaches the fragment tree any more, so the \
                 hull/fragment distinction in the failure message is guarding \
                 nothing — check the scan is still parsing before removing it"
            );
            assert!(
                fragments.len() < baked.len(),
                "the scan must still reach hulls too, or `is_fragment` has \
                 stopped discriminating"
            );
        }

        /// rustfmt wraps a long `include_str!` path onto the next line, and the
        /// scan above must still see it.
        ///
        /// This is the mechanism in isolation, pinned synthetically rather than
        /// against the real tree. The wrapped form is not hypothetical — issue
        /// #878 composed the five Harrow hulls and moved every site that baked
        /// one onto the resolving load path, and those long
        /// `"../../assets/entities/ship_harrow_*.toml"` literals were exactly the
        /// ones rustfmt had wrapped — but a synthetic fixture needs no wrapped
        /// site to exist in the tree at all, so this stays load-bearing even if
        /// the real tree later converges back to all-contiguous sites.
        #[test]
        fn the_scan_reads_a_wrapped_include_str_literal() {
            let one_line = "include_str!(\"../../assets/entities/x.toml\")";
            let wrapped = "include_str!(\n            \"../../assets/entities/x.toml\"\n        )";
            for src in [one_line, wrapped] {
                let after = &src[src.find("include_str!(").expect("the macro") + 13..];
                let (literal, _) = baked_literal(after)
                    .unwrap_or_else(|| panic!("the scan must read the literal out of: {src:?}"));
                assert_eq!(literal, "../../assets/entities/x.toml");
            }
            assert!(
                baked_literal("concat!(\"a\", \"b\"))").is_none(),
                "a non-literal argument bakes no path this scan can name"
            );
        }
    }

    // ── Composition as a world finding (issue #906) ──────────────────────────

    mod composition_findings {
        use super::*;
        use crate::world::validate::{has_error, Severity};

        fn source(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(p, t)| (p.to_string(), t.to_string()))
                .collect()
        }

        #[test]
        fn a_missing_fragment_becomes_an_error_finding_naming_the_declaring_file() {
            let src = source(&[(
                "e/hull.toml",
                "includes = [\"absent.toml\"]\nname = \"H\"\n",
            )]);
            let f = composition_finding("e/hull.toml", &src).expect("a finding");
            assert_eq!(f.severity, Severity::Error);
            assert_eq!(f.category, "include-missing");
            assert_eq!(
                f.source.file, "e/hull.toml",
                "the finding names the file that DECLARED the bad include"
            );
            assert_eq!(f.source.line, Some(1));
            assert!(f.message.contains("include chain"), "{}", f.message);
            assert!(has_error(&[f]), "an error finding must gate activation");
        }

        #[test]
        fn a_cycle_becomes_an_error_finding() {
            let src = source(&[
                ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
                ("e/frag.toml", "includes = [\"hull.toml\"]\n"),
            ]);
            let f = composition_finding("e/hull.toml", &src).expect("a finding");
            assert_eq!(f.category, "include-cycle");
        }

        #[test]
        fn a_malformed_includes_declaration_becomes_an_error_finding() {
            let src = source(&[("e/hull.toml", "includes = 7\n")]);
            let f = composition_finding("e/hull.toml", &src).expect("a finding");
            assert_eq!(f.category, "include-malformed");
            assert_eq!(f.source.file, "e/hull.toml");
        }

        #[test]
        fn a_composed_document_that_is_not_a_valid_entity_becomes_an_error_finding() {
            let src = source(&[
                ("e/frag.toml", "not_a_real_key = 1\n"),
                ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
            ]);
            let f = composition_finding("e/hull.toml", &src).expect("a finding");
            assert_eq!(
                f.category, "include-invalid-template",
                "the offending combination exists in no single authored file"
            );
        }

        #[test]
        fn a_template_the_source_cannot_serve_is_not_a_composition_finding() {
            let src = source(&[]);
            assert!(
                composition_finding("e/hull.toml", &src).is_none(),
                "a validator must not manufacture an error out of its own blindness — \
                 a missing template has its own diagnostics"
            );
        }

        #[test]
        fn an_uncomposed_template_that_is_not_valid_toml_is_not_a_composition_finding() {
            let src = source(&[("e/hull.toml", "this is not toml\n")]);
            assert!(
                composition_finding("e/hull.toml", &src).is_none(),
                "a plain parse error keeps its historical skip-with-warning; it is not \
                 a composition failure"
            );
        }

        #[test]
        fn an_uncomposed_template_that_is_not_a_valid_entity_is_not_a_composition_finding() {
            let src = source(&[("e/hull.toml", "not_a_real_key = 1\n")]);
            assert!(composition_finding("e/hull.toml", &src).is_none());
        }

        #[test]
        fn an_unparseable_fragment_is_a_composition_finding() {
            let src = source(&[
                ("e/frag.toml", "this is not toml\n"),
                ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
            ]);
            let f = composition_finding("e/hull.toml", &src).expect("a finding");
            assert_eq!(f.category, "include-parse");
        }

        /// The host source's answer to `absence_is_final` is target-dependent,
        /// and the browser answer is the one that matters — so it is also the
        /// one `cargo test` can never observe.
        ///
        /// The structural guard against losing that answer is that
        /// [`FragmentSource::absence_is_final`] has no default: delete
        /// `HostFragmentSource`'s override and the crate stops compiling, on
        /// both targets. This test pins the *values* on top of that. The native
        /// arm runs in CI; the wasm arm is compiled only under `wasm32` and so
        /// runs only under a wasm test runner — it is written as an assertion
        /// rather than a comment so that if this crate ever grows one, the
        /// claim is already being checked rather than merely described.
        #[test]
        fn the_host_source_answers_absence_by_target() {
            #[cfg(not(target_arch = "wasm32"))]
            assert!(
                HostFragmentSource.absence_is_final(),
                "on native the filesystem is authoritative, so a fragment that \
                 cannot be read genuinely does not exist and validation may say so"
            );
            #[cfg(target_arch = "wasm32")]
            assert!(
                !HostFragmentSource.absence_is_final(),
                "in the browser the raw-template channel fills one delivery at a \
                 time, so an unread fragment may still be in flight; calling that \
                 final blanks the world permanently"
            );
        }

        /// A source that fills INCREMENTALLY — the browser's raw-template
        /// channel, where a root can be in hand a whole layer-load before the
        /// fragment it includes.
        struct StillFilling(HashMap<String, String>);

        impl FragmentSource for StillFilling {
            fn read(&self, path: &str) -> Option<String> {
                self.0.get(path).cloned()
            }
            fn absence_is_final(&self) -> bool {
                false
            }
        }

        fn still_filling(pairs: &[(&str, &str)]) -> StillFilling {
            StillFilling(source(pairs))
        }

        /// The wasm hazard, stated directly: a fragment that has not been
        /// delivered YET is not a fault.
        ///
        /// If this reported, `has_error` would gate the world, and
        /// `spawn_immediate_entities_internal` would return zero entities — for
        /// a world whose only sin is that its fragments are still arriving. The
        /// runtime layer load never retries, so the loss would be permanent
        /// rather than a frame of lag.
        #[test]
        fn a_fragment_that_has_not_arrived_yet_is_not_a_composition_finding() {
            let src =
                still_filling(&[("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n")]);
            assert!(
                composition_finding("e/hull.toml", &src).is_none(),
                "a source that is still filling must not have its own race read \
                 back to it as a broken include"
            );
        }

        /// …and the blindness is not permanent: once the fragment lands, the
        /// same pair is composed and validated like any other.
        #[test]
        fn the_same_pair_validates_once_the_fragment_arrives() {
            let good = still_filling(&[
                ("e/frag.toml", "class = \"cruiser\"\n"),
                ("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n"),
            ]);
            assert!(composition_finding("e/hull.toml", &good).is_none());

            let bad = still_filling(&[
                ("e/frag.toml", "not_a_real_key = 1\n"),
                ("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n"),
            ]);
            let f = composition_finding("e/hull.toml", &bad)
                .expect("a delivered fragment is judged, not excused");
            assert_eq!(f.category, "include-invalid-template");
        }

        /// Deferring on ABSENCE must not defer on the faults. A cycle is a
        /// fault no delivery can fix, and it is still reported from a source
        /// that is still filling.
        #[test]
        fn the_real_faults_are_still_reported_while_a_source_is_still_filling() {
            let cyclic = still_filling(&[
                ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
                ("e/frag.toml", "includes = [\"hull.toml\"]\n"),
            ]);
            assert_eq!(
                composition_finding("e/hull.toml", &cyclic)
                    .expect("a cycle is not a delivery race")
                    .category,
                "include-cycle"
            );

            let malformed = still_filling(&[("e/hull.toml", "includes = 7\n")]);
            assert_eq!(
                composition_finding("e/hull.toml", &malformed)
                    .expect("a malformed declaration is not a delivery race")
                    .category,
                "include-malformed"
            );

            let unparseable = still_filling(&[
                ("e/frag.toml", "this is not toml\n"),
                ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
            ]);
            assert_eq!(
                composition_finding("e/hull.toml", &unparseable)
                    .expect("a delivered but unparseable fragment is not a delivery race")
                    .category,
                "include-parse"
            );
        }

        /// The default the other way round: a source that already holds
        /// everything it will ever hold — the filesystem, every fixture map —
        /// still reports a genuinely missing fragment, with the declaring file
        /// and line intact.
        #[test]
        fn a_source_whose_absence_is_final_still_reports_the_missing_fragment() {
            let src = source(&[(
                "e/hull.toml",
                "includes = [\"absent.toml\"]\nname = \"H\"\n",
            )]);
            assert!(
                src.absence_is_final(),
                "a fixture map holds everything it will ever hold"
            );
            let f = composition_finding("e/hull.toml", &src).expect("a finding");
            assert_eq!(f.category, "include-missing");
            assert_eq!(f.source.file, "e/hull.toml");
            assert_eq!(f.source.line, Some(1));
        }

        #[test]
        fn a_template_that_composes_cleanly_produces_no_finding() {
            let src = source(&[
                ("e/frag.toml", "class = \"cruiser\"\n"),
                ("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n"),
            ]);
            assert!(composition_finding("e/hull.toml", &src).is_none());
        }
    }
}
