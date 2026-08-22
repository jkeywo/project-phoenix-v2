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

use crate::entities::config::EntityConfig;
use crate::entities::entity_override::{ArrayRule, MergePolicy};
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
/// exactly like [`crate::entities::loader::TemplateLoader`].
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
        crate::entities::config_cache::mod_pack_overlay_get(path)
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
                if key == crate::entities::entity_override::REMOVE_KEY {
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
                        if crate::entities::entity_override::is_removal(element) {
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
    let value = crate::entities::entity_override::strip_removals(
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
        Some(accumulated) => crate::entities::entity_override::merge_entity_config_toml_with(
            &accumulated,
            &value,
            POLICY,
        ),
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
                    line: line_of(&text, crate::entities::entity_override::REMOVE_KEY),
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
/// Mirrors [`crate::entities::loader::WasmTemplateLoader`]'s three-step lookup, one
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
        if let Some(text) = crate::entities::config_cache::mod_pack_overlay_get(path) {
            return Some(text);
        }
        if let Some(text) = crate::entities::config_cache::raw_template_text(path) {
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
#[path = "include_resolve_tests.rs"]
mod tests;
