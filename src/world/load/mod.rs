//! The world-loading sequence, owned in one place (issue #1213, Track 2 step A1).
//!
//! Every path that turns a world file into live content today does the same
//! handful of steps in the same order, spread across three call sites: the
//! browser bridge, `headless::app::build_headless_app`, and the additive
//! layer/scenario merge. Each reads the TOML, parses it into a
//! [`WorldConfig`](crate::world::config::WorldConfig), re-parses the raw
//! [`toml::Value`] the Rhai loader needs, (optionally) transforms that raw value,
//! validates the composition, compiles the scripts, and records what it read into
//! the [content ledger](crate::content_ledger). This module is that sequence
//! expressed once, as a function over a [`LoadRequest`].
//!
//! # A wrapper, deliberately
//!
//! This is a *thin wrapper over today's code paths*: it calls
//! [`parse_world`](crate::world::config::parse_world),
//! [`validate_composition`](crate::world::validate::validate_composition), and
//! [`load_world_scripts`](crate::world::script::load::load_world_scripts)
//! unchanged. Issue #1213 converts **zero** production call sites — later issues
//! (A2 headless, B5/B6 boot) adopt it — so it must not change any runtime
//! behaviour or digest. Nothing here parses a world a second way.
//!
//! # The ledger is data, not a side effect
//!
//! The one place today's boot paths differ from a pure function is the content
//! ledger: they `reset()` at the start of a load, `record(path, text)` each file,
//! and `freeze()` once the declared set is known. [`load`] does **not** freeze,
//! does not reset, and — since issue #1241 — does not record either. Every ledger
//! write it implies comes back as a [`LedgerPlan`] for the caller to apply and
//! then freeze in one documented order.
//!
//! The compiled-script digest was the exception until #1241: it was written from
//! inside the wrapped
//! [`load_world_scripts`](crate::world::script::load::load_world_scripts), so the
//! sentence above was true of everything you could see and false of one thing you
//! could not. It now rides out on [`LedgerPlan::digests`]. The content-ledger fold
//! is path-sorted and order-independent (see
//! [`crate::content_ledger::ContentLedger::fold`]), so applying records and
//! digests in any order yields a byte-identical frozen digest — which is what
//! makes moving the write from "during the load" to "at the caller's apply" a
//! refactor rather than a change.
//!
//! # The reader seam
//!
//! [`WorldReader`] abstracts "read the TOML at this path": [`FsReader`] on native,
//! [`WasmReader`] over the browser bridge's pending-fetch queue, and
//! [`MemoryReader`] for tests. It is deliberately **not** named `WorldSource` —
//! that name belongs to [`crate::world::validate::WorldSource`], the parsed
//! (path, toml, config) triple the composition validator borrows.

use std::collections::BTreeMap;
use std::fmt;

use crate::entities::config::EntityConfig;
use crate::entities::loader::TemplateLoader;
use crate::world::config::{parse_world, WorldConfig};
use crate::world::script::load::{load_world_scripts, CompiledScripts, ScriptResolver};
use crate::world::validate::{validate_composition, WorldFinding, WorldSource};

// ── The reader seam ─────────────────────────────────────────────────────────

/// A source of world TOML text, keyed by authored path.
///
/// The one thing a world load needs from its host that differs per target:
/// native reads the filesystem, the browser reads a JS-delivered fetch queue, a
/// test reads an in-memory map. Returns `None` when the path cannot be read (not
/// yet fetched, missing file, absent key) — the caller turns that into a
/// [`LoadError`].
pub trait WorldReader {
    /// Read the world TOML at `path`, or `None` if it cannot be read.
    fn read(&self, path: &str) -> Option<String>;
}

/// Native/headless reader: the world TOML off the filesystem.
///
/// Mirrors `world::server::load_scenario_toml_text`'s native arm
/// (`std::fs::read_to_string(path).ok()`), so a converted call site reads exactly
/// what it reads today.
#[cfg(not(target_arch = "wasm32"))]
pub struct FsReader;

#[cfg(not(target_arch = "wasm32"))]
impl WorldReader for FsReader {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// Browser reader: resolve world TOML from the live overlay or durable
/// JS-delivered base cache, firing a fetch request when neither is present.
///
/// Mirrors `world::server::load_scenario_toml_text`'s wasm arm. Both
/// [`resolved_world_source`](crate::entities::config_cache::resolved_world_source) and
/// [`request_world_fetch`](crate::entities::config_cache::request_world_fetch) have native
/// no-op stubs, so this adapter compiles on every target (and simply reads
/// `None` off-browser) without a `cfg` gate of its own.
pub struct WasmReader;

impl WorldReader for WasmReader {
    fn read(&self, path: &str) -> Option<String> {
        crate::entities::config_cache::resolved_world_source(path).or_else(|| {
            crate::entities::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

/// Test reader: an in-memory `path -> TOML` map.
///
/// The single fixture source for [`load`]'s unit tests — a world composition
/// (root plus `extra_worlds` children) authored as literal strings, with no
/// filesystem or bridge involved.
pub struct MemoryReader(pub BTreeMap<String, String>);

impl MemoryReader {
    /// Build a reader from `(path, toml)` pairs.
    pub fn new<I, K, V>(entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        MemoryReader(
            entries
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl WorldReader for MemoryReader {
    fn read(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
}

/// Test loader: an in-memory `path -> EntityConfig` map, standing in for
/// [`crate::entities::loader::TemplateLoader`] the way [`MemoryReader`] stands
/// in for [`WorldReader`] (issue #1216).
///
/// Collapses twelve hand-rolled `impl TemplateLoader` fakes that had
/// accumulated across `entities::loader`, `headless::duel`,
/// `world::{mod_pack, validate, dispatch, spawn_origin}`'s test modules — most
/// of them the same "map of fixtures, authoritative about absence" shape, plus
/// three that instead pinned a *host's authority over absence*
/// ([`TemplateLoader::absence_is_final`], issue #973): a host still filling
/// (delivered some templates, but more may still arrive), a host that is
/// blind (delivers nothing and knows it), and a host that is empty yet
/// wrongly claims authority anyway. Those three are constructors here
/// ([`still_filling`](Self::still_filling), [`blind`](Self::blind),
/// [`authoritative_empty`](Self::authoritative_empty)) rather than separate
/// types, matched to the struct names the fakes they replace already used.
///
/// [`new`](Self::new) and [`from_toml`](Self::from_toml) are the plain
/// fixture-map constructors, answering `true` from `absence_is_final` — "the
/// map holds everything it will ever hold" — which is what most callers of
/// the fakes above wanted. Chain [`with_template`](Self::with_template) or
/// [`with_toml`](Self::with_toml) onto any constructor, including the
/// authority-pinning ones, to seed further entries.
#[derive(Debug, Clone)]
pub struct MemoryTemplateLoader {
    templates: BTreeMap<String, EntityConfig>,
    absence_is_final: bool,
}

/// An empty, authoritative loader — the same values [`new`](Self::new) gives
/// an empty entry list. NOT [`bool::default`]'s `false`: a fixture that
/// mentions no behaviour at all (e.g. `Fixture` structs elsewhere that
/// `#[derive(Default)]`) means "the map holds everything it will ever hold
/// (which is nothing)", not "still filling".
impl Default for MemoryTemplateLoader {
    fn default() -> Self {
        Self::empty(true)
    }
}

impl MemoryTemplateLoader {
    /// The shared empty base every constructor below builds on.
    fn empty(absence_is_final: bool) -> Self {
        MemoryTemplateLoader {
            templates: BTreeMap::new(),
            absence_is_final,
        }
    }

    /// Build a loader from `(path, EntityConfig)` pairs, authoritative about
    /// absence.
    pub fn new<I, K>(entries: I) -> Self
    where
        I: IntoIterator<Item = (K, EntityConfig)>,
        K: Into<String>,
    {
        MemoryTemplateLoader {
            templates: entries.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            absence_is_final: true,
        }
    }

    /// Build a loader from `(path, toml)` pairs, parsed eagerly and
    /// authoritative about absence like [`new`](Self::new). Panics if an
    /// entry does not parse — a bug in the fixture authoring it, not the
    /// thing under test.
    pub fn from_toml<I, K, V>(entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: AsRef<str>,
    {
        entries
            .into_iter()
            .fold(Self::empty(true), |loader, (path, toml)| {
                loader.with_toml(path, toml.as_ref())
            })
    }

    /// Delivered nothing yet, and not authoritative about it: a host still
    /// mid-preload, holding no hull in hand while other paths are in flight
    /// (issue #973 review, F6). Chain [`with_template`](Self::with_template)
    /// or [`with_toml`](Self::with_toml) to give it something it HAS
    /// delivered while it stays non-final.
    pub fn still_filling() -> Self {
        Self::empty(false)
    }

    /// Serves nothing, and knows it can serve nothing: the browser's blind
    /// answer (issue #973), for fixtures whose point is some *other* check
    /// and simply need a host that resolves no templates.
    pub fn blind() -> Self {
        Self::empty(false)
    }

    /// Serves nothing yet claims authority over absence anyway — what a
    /// native [`WasmTemplateLoader`](crate::entities::loader::WasmTemplateLoader)
    /// answers with an empty cache and nothing on disk (issue #973 review,
    /// F3).
    pub fn authoritative_empty() -> Self {
        Self::empty(true)
    }

    /// Insert one more pre-built template, builder-style.
    pub fn with_template(mut self, path: impl Into<String>, config: EntityConfig) -> Self {
        self.templates.insert(path.into(), config);
        self
    }

    /// Insert one more template parsed from raw TOML, builder-style. Panics
    /// if it does not parse.
    pub fn with_toml(self, path: impl Into<String>, toml: &str) -> Self {
        let path = path.into();
        let config = EntityConfig::from_toml(toml).unwrap_or_else(|e| {
            panic!("MemoryTemplateLoader fixture at {path:?} must parse: {e:?}")
        });
        self.with_template(path, config)
    }
}

impl TemplateLoader for MemoryTemplateLoader {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        self.templates.get(path).cloned()
    }

    /// The map holds everything it will ever hold: whatever was pinned at
    /// construction (see the constructors above).
    fn absence_is_final(&self) -> bool {
        self.absence_is_final
    }
}

// ── Request / policy ────────────────────────────────────────────────────────

/// How much of the load sequence to run for a [`LoadRequest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadPolicy {
    /// Full boot ingestion: parse the root and its static `extra_worlds`
    /// children, compile each world's own scripts, validate the composition,
    /// and gather every world record and script digest into the [`LedgerPlan`]
    /// for the caller to apply and freeze. The policy `boot::ingest_world` uses.
    Activate,
    /// Additive layer/scenario merge: parse the single world, record its TOML,
    /// and carry its compiled scripts as data — no whole-composition gate and no
    /// child recursion, because a layer merges into an already-active
    /// composition. The layer caller runs the narrow candidate gate before
    /// applying this result. The policy `scenario.rs` folds into (issue #1045
    /// plumbing).
    Merge,
    /// The pure kernel: [`parse_world`](crate::world::config::parse_world) only,
    /// no ledger and no scripts. The policy the manifest / mod-pack linters use
    /// to inspect a world without loading it.
    Inspect,
}

/// One world-load request: where to read from, how to resolve sibling scripts,
/// which [`LoadPolicy`] to run, and an optional pre-compile transform of the raw
/// world value.
pub struct LoadRequest<'a> {
    /// Authored path of the root world TOML (its content-ledger / snapshot key).
    pub path: String,
    /// The TOML reader for this target.
    pub reader: &'a dyn WorldReader,
    /// Resolver for a world's sibling `.rhai` scripts (the existing script-load
    /// seam). Unused under [`LoadPolicy::Inspect`].
    pub script_resolver: &'a dyn ScriptResolver,
    /// Which slice of the sequence to run.
    pub policy: LoadPolicy,
    /// Optional transform applied to the raw [`toml::Value`] **before** scripts
    /// are compiled — the seam `headless::duel::apply_duel_sides` rewrites the
    /// slot roster through. It touches only the raw value the script loader
    /// reads; the parsed [`WorldConfig`] is derived from the untouched text, so a
    /// transform never perturbs entity/anchor content. `Err` aborts the load with
    /// [`LoadError::TransformFailed`].
    pub raw_transform: Option<&'a dyn Fn(toml::Value) -> Result<toml::Value, String>>,
}

impl<'a> LoadRequest<'a> {
    /// A request with no [`raw_transform`](Self::raw_transform).
    pub fn new(
        path: impl Into<String>,
        reader: &'a dyn WorldReader,
        script_resolver: &'a dyn ScriptResolver,
        policy: LoadPolicy,
    ) -> Self {
        LoadRequest {
            path: path.into(),
            reader,
            script_resolver,
            policy,
            raw_transform: None,
        }
    }

    /// This request with a raw-value transform attached.
    pub fn with_transform(
        mut self,
        transform: &'a dyn Fn(toml::Value) -> Result<toml::Value, String>,
    ) -> Self {
        self.raw_transform = Some(transform);
        self
    }
}

// ── Result / ledger / error ─────────────────────────────────────────────────

/// The digest half of a [`LedgerPlan`], re-exported so both halves of the
/// vocabulary are found in one place. Defined in [`crate::content_ledger`]
/// because the script loader that produces it sits below this module.
pub use crate::content_ledger::LedgerDigest;

/// One content-ledger record a load produced by reading a file: the file's
/// authored path and the raw text read at it.
///
/// Applied by the caller as `content_ledger::record(&record.path, &record.text)`.
/// Its digest-carrying sibling is [`LedgerDigest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRecord {
    /// Authored path (the content-ledger key, before canonicalisation).
    pub path: String,
    /// The raw TOML text read at `path`.
    pub text: String,
}

/// The content-ledger writes a load gathered, returned as data.
///
/// [`load`] never touches the global ledger and never calls
/// [`freeze`](crate::content_ledger::freeze); it hands every write back so the
/// caller applies them (and freezes) in one place. Two halves, because the ledger
/// has two write primitives:
///
/// * [`records`](Self::records) — the world-TOML text the load read: the root and
///   every `extra_worlds` child.
/// * [`digests`](Self::digests) — a digest already computed below the load, which
///   today means the compiled script set's `content_hash` under
///   `<world path>#scripts`.
///
/// The digest half used to be a SIDE EFFECT, written from inside the wrapped
/// `load_world_scripts` while everything else round-tripped through here (issue
/// #1241). That was the one place `load` reached out and touched global state,
/// and it made "the ledger is returned as data" true of the sequence only if you
/// did not look inside it. Now every write a load implies leaves through this
/// type, so a caller that wants a load WITHOUT recording it — the pure layer
/// decision in `world::layers`, the `Inspect` linters — gets one by not applying
/// the plan, rather than by hoping nothing underneath recorded already.
///
/// The entity-template eager walk and the `freeze` itself still stay with the
/// caller. Application order does not matter: the ledger's fold is path-sorted
/// (see [`crate::content_ledger::ContentLedger::fold`]), so records and digests
/// interleaved any way round yield a byte-identical frozen digest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LedgerPlan {
    /// The `(path, text)` records to apply, in the order the loader read them
    /// (root first, then children). Order does not affect the folded digest.
    pub records: Vec<LedgerRecord>,
    /// The `(key, digest)` writes to apply. One entry per compiled script set
    /// that had any source at all; empty for a script-free world.
    pub digests: Vec<LedgerDigest>,
}

impl LedgerPlan {
    /// Apply every gathered write to the content ledger. The caller calls this
    /// (then [`crate::content_ledger::freeze`]) — [`load`] never does.
    pub fn apply(&self) {
        for record in &self.records {
            crate::content_ledger::record(&record.path, &record.text);
        }
        for digest in &self.digests {
            digest.apply();
        }
    }

    /// Whether this plan would write nothing.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.digests.is_empty()
    }
}

/// One fully-loaded world: its parsed config, compiled scripts, loaded children,
/// composition findings, and the ledger records the load read.
///
/// `findings` carries the **composition**-level findings (from
/// [`validate_composition`](crate::world::validate::validate_composition)); the
/// **script**-level findings ride in each world's own
/// [`CompiledScripts::findings`](crate::world::script::load::CompiledScripts). A
/// caller gating activation checks `loaded.findings`, the root `scripts`, and
/// every static child's `scripts`. Boot retains only the root set as the runtime
/// `PreCompiledScripts` resource; child sets exist here to validate and bind the
/// pre-freeze declared content before runtime layer activation recompiles them.
pub struct LoadedWorld {
    /// The parsed world configuration (from the untouched TOML text).
    pub config: WorldConfig,
    /// The compiled, validated script set, or `None` for a world with no
    /// `script` key or under [`LoadPolicy::Inspect`].
    pub scripts: Option<CompiledScripts>,
    /// The `extra_worlds` children, loaded and recorded (populated only under
    /// [`LoadPolicy::Activate`]).
    pub children: Vec<LoadedWorld>,
    /// Composition-level validation findings (empty except under
    /// [`LoadPolicy::Activate`]).
    pub findings: Vec<WorldFinding>,
    /// The content-ledger records this load read (empty under
    /// [`LoadPolicy::Inspect`]).
    pub ledger: LedgerPlan,
}

impl fmt::Debug for LoadedWorld {
    // Hand-rolled because [`CompiledScripts`] carries Rhai `AST`s and is not
    // `Debug`; the compiled set is shown as a presence marker so a `LoadedWorld`
    // (and any test asserting over one) can still be printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedWorld")
            .field("config", &self.config)
            .field("scripts", &self.scripts.as_ref().map(|_| "<compiled>"))
            .field("children", &self.children)
            .field("findings", &self.findings)
            .field("ledger", &self.ledger)
            .finish()
    }
}

/// Why a load could not produce a [`LoadedWorld`] at all.
///
/// Composition and script *validation* outcomes are **not** errors here — they
/// ride in [`LoadedWorld::findings`] / the compiled scripts, and the caller gates
/// activation on them. A `LoadError` is reserved for the failures that stop a
/// world being produced: an unreadable file, a TOML that will not parse, or a
/// failing transform — mirroring the boot paths, which hard-fail exactly those.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The reader returned `None` for the root world path.
    ReadFailed { path: String },
    /// [`parse_world`](crate::world::config::parse_world) rejected the root TOML.
    ParseFailed { path: String, message: String },
    /// The raw `toml::Value` re-parse (needed for the script seam) failed.
    RawParseFailed { path: String, message: String },
    /// The [`raw_transform`](LoadRequest::raw_transform) hook returned `Err`.
    TransformFailed { message: String },
    /// An `extra_worlds` child could not be read.
    ChildReadFailed { path: String },
    /// An `extra_worlds` child's TOML would not parse.
    ChildParseFailed { path: String, message: String },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::ReadFailed { path } => write!(f, "world {path:?} could not be read"),
            LoadError::ParseFailed { path, message } => {
                write!(f, "world {path:?} failed to parse: {message}")
            }
            LoadError::RawParseFailed { path, message } => {
                write!(
                    f,
                    "world {path:?} failed to re-parse as a TOML value: {message}"
                )
            }
            LoadError::TransformFailed { message } => {
                write!(f, "world raw-value transform failed: {message}")
            }
            LoadError::ChildReadFailed { path } => {
                write!(f, "extra_world {path:?} could not be read")
            }
            LoadError::ChildParseFailed { path, message } => {
                write!(f, "extra_world {path:?} failed to parse: {message}")
            }
        }
    }
}

impl std::error::Error for LoadError {}

// ── The load sequence ───────────────────────────────────────────────────────

/// Run the world-loading sequence described by `request`.
///
/// Wraps today's code paths; see the [module docs](self) for the ledger and
/// no-behaviour-change contract. Dispatches on [`LoadRequest::policy`]:
///
/// * [`Inspect`](LoadPolicy::Inspect) — read + `parse_world`. No scripts, no
///   ledger, no children.
/// * [`Merge`](LoadPolicy::Merge) — read + `parse_world`, record the TOML, and
///   compile scripts (carried, not activated). No whole-composition gate or
///   children; the additive-layer caller runs its narrow candidate gate.
/// * [`Activate`](LoadPolicy::Activate) — the full sequence: read + `parse_world`,
///   record, load every `extra_worlds` child, compile the root and each child's
///   own scripts, validate composition against those exact source sets, and
///   return them all. The optional raw transform applies to the root only.
pub fn load(request: LoadRequest) -> Result<LoadedWorld, LoadError> {
    let LoadRequest {
        path,
        reader,
        script_resolver,
        policy,
        raw_transform,
    } = request;

    let text = read_root(reader, &path)?;
    let config = parse_root(&path, &text)?;

    match policy {
        LoadPolicy::Inspect => Ok(LoadedWorld {
            config,
            scripts: None,
            children: Vec::new(),
            findings: Vec::new(),
            ledger: LedgerPlan::default(),
        }),
        LoadPolicy::Merge => {
            let scripts = compile_scripts(&path, &text, script_resolver, raw_transform)?;
            let digests = script_digests(&scripts);
            Ok(LoadedWorld {
                config,
                scripts,
                children: Vec::new(),
                findings: Vec::new(),
                ledger: LedgerPlan {
                    records: vec![LedgerRecord {
                        path: path.clone(),
                        text,
                    }],
                    digests,
                },
            })
        }
        LoadPolicy::Activate => {
            let mut records = vec![LedgerRecord {
                path: path.clone(),
                text: text.clone(),
            }];

            // Read, parse, compile and record each `extra_worlds` child. Static
            // children need their own compiled set HERE, before the content-ledger
            // freeze: their literal sibling-script spawns are part of the declared
            // template set even though runtime layer activation later compiles the
            // child again into its independently owned registrations. The root's
            // raw transform is deliberately not inherited — duel-side rewriting is
            // a root harness seam, not a transform over every supporting world.
            //
            // The `(path, text)` pairs are owned here so the borrowed
            // `WorldSource`s below outlive the composition call, exactly as
            // `build_headless_app` owned its `child_owned` triples.
            let mut children: Vec<LoadedWorld> = Vec::new();
            let mut child_sources: Vec<(String, String)> = Vec::new();
            for child_path in &config.extra_worlds {
                let child_text =
                    reader
                        .read(child_path)
                        .ok_or_else(|| LoadError::ChildReadFailed {
                            path: child_path.clone(),
                        })?;
                let child_config =
                    parse_world(&child_text).map_err(|e| LoadError::ChildParseFailed {
                        path: child_path.clone(),
                        message: e,
                    })?;
                let child_scripts =
                    compile_scripts(child_path, &child_text, script_resolver, None)?;
                records.push(LedgerRecord {
                    path: child_path.clone(),
                    text: child_text.clone(),
                });
                child_sources.push((child_path.clone(), child_text));
                children.push(LoadedWorld {
                    config: child_config,
                    scripts: child_scripts,
                    children: Vec::new(),
                    findings: Vec::new(),
                    ledger: LedgerPlan::default(),
                });
            }

            // Compile ONCE before validation so the root source can expose the
            // exact inline/sibling script-spawn set to the composition gate.
            // This stays after child ingestion, preserving the prior error and
            // ledger order; the same owned value is returned below and supplies
            // the unchanged digest/runtime handoff.
            let scripts = compile_scripts(&path, &text, script_resolver, raw_transform)?;

            // Atomic composition validation over root + children. Findings (not
            // errors) — the caller gates activation on them.
            let mut root_src = WorldSource::new(path.clone(), &text, &config);
            if let Some(compiled) = scripts.as_ref() {
                root_src = root_src.with_resolved_script_spawns(&compiled.spawned_templates);
            }
            let child_srcs: Vec<WorldSource> = children
                .iter()
                .zip(child_sources.iter())
                .map(|(child, (child_path, child_text))| {
                    let mut source =
                        WorldSource::new(child_path.clone(), child_text.as_str(), &child.config);
                    if let Some(compiled) = child.scripts.as_ref() {
                        source = source.with_resolved_script_spawns(&compiled.spawned_templates);
                    }
                    source
                })
                .collect();
            let findings = validate_composition(&root_src, &child_srcs);
            drop(child_srcs);
            drop(root_src);

            let mut digests = script_digests(&scripts);
            for child in &children {
                digests.extend(script_digests(&child.scripts));
            }

            Ok(LoadedWorld {
                config,
                scripts,
                children,
                findings,
                ledger: LedgerPlan { records, digests },
            })
        }
    }
}

// ── Private helpers (each wraps one of today's steps) ────────────────────────

/// The ledger writes a compiled script set implies (issue #1241).
///
/// One entry, or none for a world that lifted no source. Kept as a `Vec` rather
/// than an `Option` because an Activate load concatenates the root and every
/// scripted child's record into one caller-applied plan.
fn script_digests(scripts: &Option<CompiledScripts>) -> Vec<LedgerDigest> {
    scripts
        .as_ref()
        .and_then(|s| s.ledger_digest.clone())
        .into_iter()
        .collect()
}

fn read_root(reader: &dyn WorldReader, path: &str) -> Result<String, LoadError> {
    reader.read(path).ok_or_else(|| LoadError::ReadFailed {
        path: path.to_string(),
    })
}

fn parse_root(path: &str, text: &str) -> Result<WorldConfig, LoadError> {
    parse_world(text).map_err(|e| LoadError::ParseFailed {
        path: path.to_string(),
        message: e,
    })
}

/// Build the raw [`toml::Value`] the script loader reads, apply the optional
/// transform, and compile the world's scripts — or `Ok(None)` for a world with no
/// `script` key. Mirrors `build_headless_app`'s script gate: config comes from
/// the untouched text (the caller already parsed it); the transform rewrites only
/// this raw value.
fn compile_scripts(
    path: &str,
    text: &str,
    resolver: &dyn ScriptResolver,
    raw_transform: Option<&dyn Fn(toml::Value) -> Result<toml::Value, String>>,
) -> Result<Option<CompiledScripts>, LoadError> {
    let mut raw: toml::Value = toml::from_str(text).map_err(|e| LoadError::RawParseFailed {
        path: path.to_string(),
        message: e.to_string(),
    })?;

    if let Some(transform) = raw_transform {
        raw = transform(raw).map_err(|message| LoadError::TransformFailed { message })?;
    }

    if raw.get("script").is_none() {
        return Ok(None);
    }

    Ok(Some(load_world_scripts(path, &raw, resolver)))
}

#[cfg(test)]
mod tests;
