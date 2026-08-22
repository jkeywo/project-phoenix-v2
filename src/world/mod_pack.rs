// Host mod-pack upload validation (issue #760).
//
// Pure Rust module — no Bevy, no wasm_bindgen, and no host I/O: every source of
// content OUTSIDE the uploaded archive arrives through an injected seam
// (`resolve_base` for base content text, a `TemplateLoader` for parsed entity
// templates). That is what keeps the module natively testable, and it is not
// decorative — reaching for a host default instead is precisely how the
// composition check first came to validate a pack against content the pack does
// not contain (see `PackFragments` below). Consumes the store-only ZIP the
// editor mod-pack exporter (issue #759, `editor/mod-pack-export.js`) produces
// and validates it *atomically*: the whole pack is accepted only when every
// step passes. On any failure — malformed archive, off-whitelist path, missing
// manifest, unparseable TOML, invalid manifest entry, or an unresolved
// composition reference — the function returns error findings and NOTHING is
// applied (AC1).
//
// The archive reader mirrors `readStoreZip`/`crc32` in
// `editor/mod-pack-export.js` byte-for-byte (there is no zip crate in
// Cargo.toml, and a store-only reader is small enough to audit). The semantic
// validation deliberately REUSES the existing pure validators rather than
// forking them:
//   * `world::manifest::{parse_manifest, validate_manifest}` for the required
//     `scenarios.toml` manifest, resolving each root world against BOTH the
//     pack contents and base content;
//   * `world::validate::{validate_composition_with_fragments, has_error}` for
//     every manifest-listed world's authored references, and for the `includes`
//     closure of every entity template those worlds spawn (issue #906).
//
// Acceptance is gated on `has_error` (definite errors block; warnings are
// non-blocking, consistent with #757/#759). The Bevy/wasm adapter that turns a
// browser upload into a call here — and populates the session overlay on
// success — lives in `server::bridge` + `entities::config_cache`, keeping this
// module a pure, natively-testable core.

use std::collections::BTreeMap;

use crate::entities::config_cache::ActivePack;
use crate::entities::include_resolve::FragmentSource;
use crate::entities::loader::TemplateLoader;
use crate::world::config::parse_world;
use crate::world::manifest::{
    parse_pack_manifest, validate_manifest, ContentIdentity, SUPPORTED_PACK_FORMAT,
};
use crate::world::script::load::{compile_scripts, lift_world_scripts, ScriptResolver};
use crate::world::validate::{
    has_error, validate_composition_with_fragments, Severity, SourceLocation, WorldFinding,
    WorldSource,
};
use vellum_script::ScriptSource;

/// The manifest path a mod pack always carries (top-level in the archive).
/// Mirrors `MANIFEST_PATH` in `editor/mod-pack-export.js`. Structural, not a
/// gameplay value.
pub const MANIFEST_PATH: &str = "scenarios.toml";

/// Supported authored directory prefixes a mod pack file may sit directly
/// under. Mirrors `ALLOWED_DIR_PREFIXES` in `editor/mod-pack-export.js`.
const ALLOWED_DIR_PREFIXES: [&str; 4] = [
    "assets/worlds/",
    "assets/entities/",
    "assets/factions/",
    "assets/models/",
];

/// Whether `path` is a world path allowed to be a manifest root world. Mirrors
/// `isWorldContentPath` in `editor/mod-pack-export.js`.
pub fn is_world_content_path(path: &str) -> bool {
    path.starts_with("assets/worlds/")
        && path.ends_with(".toml")
        && path.len() > "assets/worlds/".len() + ".toml".len()
        && !path.contains("..")
}

/// Whether `path` is a content path a mod pack is allowed to include. Mirrors
/// `isAllowedContentPath` in `editor/mod-pack-export.js`: the manifest is
/// allowed on its own; every other file must sit directly under a supported
/// `assets/*` directory, carry a real file name, and contain no path traversal
/// or backslash.
///
/// A supported authored file is a `.toml` under one of [`ALLOWED_DIR_PREFIXES`],
/// OR a `.rhai` script directly under `assets/worlds/` (issue #988) — the exact
/// sibling layout `world::script::load` resolves a world's `script = "..."`
/// declaration to. The extension is NOT the trust boundary: a `.rhai` is
/// admitted here only structurally, then COMPILED under the deny-by-default
/// sandbox by [`validate_pack_scripts`], which is what actually gates it.
pub fn is_allowed_content_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.contains("..") || path.contains('\\') {
        return false;
    }
    if path == MANIFEST_PATH {
        return true;
    }
    // Rhai scripts sit beside the world that loads them: a sibling
    // `assets/worlds/*.rhai`, and nowhere else (issue #988).
    if path.ends_with(".rhai") {
        return match path.strip_prefix("assets/worlds/") {
            Some(name) => name.len() > ".rhai".len() && !name.contains('/'),
            None => false,
        };
    }
    if !path.ends_with(".toml") {
        return false;
    }
    for prefix in ALLOWED_DIR_PREFIXES {
        if let Some(name) = path.strip_prefix(prefix) {
            // Directly under the prefix (no further nesting) and a real name.
            return name.len() > ".toml".len() && !name.contains('/');
        }
    }
    false
}

// ── CRC-32 (IEEE) ────────────────────────────────────────────────────────────

/// CRC-32 (IEEE polynomial 0xedb88320) of `bytes`. Mirrors `crc32` in
/// `editor/mod-pack-export.js`.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xffff_ffff
}

// ── Store-only ZIP reader ─────────────────────────────────────────────────────

const LOCAL_FILE_HEADER_SIG: u32 = 0x0403_4b50;

fn read_u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    let slice = bytes.get(at..end)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    let slice = bytes.get(at..end)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Read a store-only ZIP produced by `createStoreZip` (issue #759) into an
/// ordered map of `path -> text`. Verifies each entry's stored CRC and rejects
/// any entry that is not compression method 0 (store). Returns `Err` on a
/// malformed archive. Mirrors `readStoreZip` in `editor/mod-pack-export.js`.
///
/// Iteration stops at the first bytes that are not a local file header (the
/// central directory / EOCD), matching the JS reader.
pub fn read_store_zip(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    let mut pos = 0usize;

    while pos + 4 <= bytes.len() && read_u32_le(bytes, pos) == Some(LOCAL_FILE_HEADER_SIG) {
        let method = read_u16_le(bytes, pos + 8).ok_or("truncated local file header")?;
        let crc = read_u32_le(bytes, pos + 14).ok_or("truncated local file header")?;
        let comp_size = read_u32_le(bytes, pos + 18).ok_or("truncated local file header")? as usize;
        let name_len = read_u16_le(bytes, pos + 26).ok_or("truncated local file header")? as usize;
        let extra_len = read_u16_le(bytes, pos + 28).ok_or("truncated local file header")? as usize;
        let name_start = pos + 30;
        let data_start = name_start + name_len + extra_len;

        if method != 0 {
            return Err(format!("unsupported compression method {method}"));
        }

        let name_bytes = bytes
            .get(name_start..name_start + name_len)
            .ok_or("truncated file name")?;
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| "file name is not valid UTF-8".to_string())?
            .to_string();

        let data = bytes
            .get(data_start..data_start + comp_size)
            .ok_or("truncated file data")?;
        if crc32(data) != crc {
            return Err(format!("CRC mismatch for {name:?}"));
        }
        let text = std::str::from_utf8(data)
            .map_err(|_| format!("file {name:?} is not valid UTF-8"))?
            .to_string();

        files.insert(name, text);
        pos = data_start + comp_size;
    }

    Ok(files)
}

// ── Atomic validation ─────────────────────────────────────────────────────────

/// The result of validating an uploaded mod pack.
///
/// `findings` is empty (and never contains an error) when the pack is accepted.
/// `files` is the exact-path -> TOML map of supported authored files the
/// session overlay should install; `manifest_toml` is the pack's raw
/// `scenarios.toml`. Both are only meaningful when the pack is accepted (the
/// caller must gate on [`is_accepted`]).
#[derive(Debug, Default)]
pub struct ValidatedModPack {
    pub findings: Vec<WorldFinding>,
    pub files: BTreeMap<String, String>,
    pub manifest_toml: String,
}

impl ValidatedModPack {
    /// True when no finding is an error — the atomic-acceptance gate (AC1).
    pub fn is_accepted(&self) -> bool {
        !has_error(&self.findings)
    }
}

/// Build a single archive-scoped finding (no line lookup — the archive is not a
/// single source file).
fn archive_finding(
    severity: Severity,
    category: &'static str,
    reference: &str,
    message: String,
) -> WorldFinding {
    WorldFinding {
        severity,
        category,
        message,
        source: SourceLocation {
            file: MANIFEST_PATH.to_string(),
            line: None,
            reference: reference.to_string(),
        },
    }
}

/// An archive-scoped ERROR finding (blocks acceptance).
fn archive_error(category: &'static str, reference: &str, message: String) -> WorldFinding {
    archive_finding(Severity::Error, category, reference, message)
}

/// The raw-TOML source an uploaded pack's entity composition resolves against
/// (issue #906, #987).
///
/// Pack files FIRST, then whatever the injected `resolve_base` closure serves —
/// which [`validate_mod_pack`] composes as the already-active overlay stack
/// (newest active first) THEN base content (issue #987). Same order, and for the
/// same reason, as the world resolution above: a pack that carries a fragment
/// must be validated against the fragment it carries, and a fragment an EARLIER
/// active pack supplies must resolve too.
///
/// The alternative — letting the composition check fall back to the host's
/// default fragment source — reads the session overlay (not installed yet: the
/// whole point of atomic validation is that nothing is applied until the pack
/// passes), then the host's raw templates, then disk. A pack whose hull includes
/// a fragment carried INSIDE the pack would then be rejected for an
/// `include-missing` that is untrue, and on wasm the same case would be deferred
/// and never checked at all.
struct PackFragments<'a, F: Fn(&str) -> Option<String>> {
    files: &'a BTreeMap<String, String>,
    resolve_base: &'a F,
}

impl<F: Fn(&str) -> Option<String>> FragmentSource for PackFragments<'_, F> {
    fn read(&self, path: &str) -> Option<String> {
        self.files
            .get(path)
            .cloned()
            .or_else(|| (self.resolve_base)(path))
    }

    /// Final. Upload validation is a one-shot decision over an archive that is
    /// wholly in hand, and the caller's `resolve_base` is expected to be able to
    /// see base content by the time a pack can be uploaded (it is a
    /// pre-scenario action, after the base preload has drained). Deferring here
    /// would mean accepting a pack whose composition was never checked.
    fn absence_is_final(&self) -> bool {
        true
    }
}

/// The PARSED entity templates an uploaded pack validates against: the pack's
/// OWN `assets/entities/*.toml` first, then the caller's loader (issue #973
/// review, F3).
///
/// # Why this exists rather than a note telling callers to be careful
///
/// The pack's session overlay is deliberately NOT installed while the pack is
/// being judged — that is what atomic validation means — so a host loader
/// cannot see the hulls the pack carries. [`validate_mod_pack`] used to document
/// that as an obligation on its callers: do not pass a loader claiming
/// [`TemplateLoader::absence_is_final`] unless it can serve the pack's own
/// hulls. A future native caller writing the obvious thing —
/// [`crate::entities::loader::WasmTemplateLoader`], the same type the documented
/// wasm caller passes — gets `true` on native and would reject **every valid
/// pack** with bogus `unresolvable-template` errors. Prose on a `pub fn` is the
/// weakest possible guard against that, and this module's own tests could never
/// catch it: they all pass a loader answering `false`, so the native suite only
/// ever exercised the safe arm.
///
/// So the obligation is discharged here instead of asked for: whatever loader
/// arrives, the pack's own files are served in front of it.
///
/// Composition matches [`PackFragments`] exactly — pack first, then base — and
/// resolves *through* it, so a pack hull including a fragment the pack also
/// carries composes from the pack's copy. When the pack holds the path its
/// answer is final: falling through to the host on a composition failure would
/// mask a broken pack hull behind a shipped one of the same name.
struct PackTemplates<'a, F: Fn(&str) -> Option<String>> {
    files: &'a BTreeMap<String, String>,
    fragments: &'a PackFragments<'a, F>,
    host: &'a dyn TemplateLoader,
}

impl<F: Fn(&str) -> Option<String>> TemplateLoader for PackTemplates<'_, F> {
    fn load_template(&self, path: &str) -> Option<crate::entities::config::EntityConfig> {
        // Both spellings, because a world may name `./assets/entities/x.toml`
        // for a pack entry keyed `assets/entities/x.toml`.
        let canonical = crate::entities::include_resolve::canonical_template_path(path);
        if self.files.contains_key(path) || self.files.contains_key(&canonical) {
            return crate::entities::include_resolve::resolve_template(path, self.fragments)
                .ok()?
                .parse()
                .ok();
        }
        self.host.load_template(path)
    }

    /// The host's answer, for the same reason
    /// [`crate::entities::loader::SpawnTemplateLoader`]'s is: serving the pack's
    /// files ADDS to what the host can see, and adding cannot make a blind host
    /// authoritative about everything it is still missing.
    fn absence_is_final(&self) -> bool {
        self.host.absence_is_final()
    }
}

/// Validate an uploaded mod-pack ZIP atomically (issue #760, AC1).
///
/// `base_content` is the host's declared content identity (the `[content]` block
/// of the base `assets/scenarios.toml`), against which the pack's
/// `[pack.requires]` clause is checked. It is injected rather than read from a
/// host default here — the same seam discipline as `resolve_base` — so this
/// module keeps no host dependency of its own (issue #986).
///
/// The pack's `[pack]` identity header is judged BEFORE any content validation:
/// a missing header (`missing-pack-header`) or a `format` above
/// [`SUPPORTED_PACK_FORMAT`] (`unsupported-pack-format`) rejects the pack
/// immediately, so a future-format pack can never bury its one real
/// incompatibility under a wall of content findings. An empty `id`
/// (`invalid-pack-id`) and a content-identity mismatch (`pack-content-mismatch`)
/// are also reported; validation is atomic, so any of these blocks the whole
/// pack.
///
/// `zip_bytes` is the raw uploaded archive. `resolve_base` resolves an authored
/// path against BASE content (returning `None` when the host has not fetched
/// it) — worlds for the manifest, and raw entity/fragment TOML for include
/// resolution — so a manifest root world may resolve either inside the pack or
/// against shipped content, and a pack hull may include a shipped fragment.
/// `template_loader` supplies PARSED entity templates for the reference checks
/// that need them (doctrine anchors, and issue #973's template-resolution
/// check); it is injected rather than defaulted so this module keeps no host
/// dependency of its own.
///
/// **Any loader is safe to pass, including an authoritative one** (issue #973
/// review, F3). It is wrapped in [`PackTemplates`], which serves the pack's own
/// hulls in front of it — see that type for why the constraint is structural
/// rather than a note asking callers to be careful. The only production caller
/// is `bridge::wasm_add_mod_pack`, which is `wasm32`-only and passes
/// `WasmTemplateLoader` (`false` in the browser, so the presence check is inert
/// there); a native caller may now pass the same type and get the answer it
/// expects. The residual gap that leaves is stated on
/// [`crate::world::validate::activation_findings`]'s presence check: on a host
/// that is *not* authoritative, a pack naming a hull it does not carry is
/// caught at spawn, not at upload.
///
/// Composition references are validated per manifest-listed world against the
/// pack + `active` stack + base worlds, and each spawned template's `includes`
/// closure against the pack's own files + the active stack + base
/// ([`PackFragments`]).
///
/// `active` is the ALREADY-INSTALLED overlay stack (oldest → newest), so a new
/// pack's composition resolves against packs loaded before it: the ordered
/// precedence is CANDIDATE pack → `active` stack (newest active first) → base
/// (issue #987). It also drives two multi-pack findings: a `duplicate-pack-id`
/// error when the candidate's `[pack] id` is already active (the stack keys packs
/// by id), and a non-blocking `overlapping-pack-path` WARNING naming the winner
/// (this candidate, loaded latest) and the shadowed loser for each authored path
/// the candidate shares with an active pack.
///
/// The returned [`ValidatedModPack`] carries error findings on any failure and
/// the overlay files + manifest on success; the caller pushes the pack onto the
/// overlay stack only when [`ValidatedModPack::is_accepted`] holds.
pub fn validate_mod_pack(
    zip_bytes: &[u8],
    base_content: &ContentIdentity,
    resolve_base: impl Fn(&str) -> Option<String>,
    template_loader: &dyn TemplateLoader,
    active: &[ActivePack],
) -> ValidatedModPack {
    // 1. Parse the store ZIP — a malformed / non-store / CRC-mismatched archive
    //    rejects the whole pack.
    let files = match read_store_zip(zip_bytes) {
        Ok(files) => files,
        Err(e) => {
            return ValidatedModPack {
                findings: vec![archive_error(
                    "invalid-archive",
                    "",
                    format!("mod pack archive could not be read: {e}"),
                )],
                ..Default::default()
            };
        }
    };

    let mut findings = Vec::new();

    // 2. Require the manifest FIRST — the pack identity header is read from it,
    //    and the header gate (step 3) runs before any content or path check so
    //    an unsupported future format is not buried under those.
    let Some(manifest_toml) = files.get(MANIFEST_PATH).cloned() else {
        findings.push(archive_error(
            "missing-manifest",
            MANIFEST_PATH,
            format!("mod pack is missing its required {MANIFEST_PATH} manifest"),
        ));
        return ValidatedModPack {
            findings,
            ..Default::default()
        };
    };

    // 3. Parse the manifest ([pack] header + [[scenario]] entries) and gate on
    //    the pack identity BEFORE any content or path validation (issue #986).
    let pack_manifest = match parse_pack_manifest(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            findings.push(archive_error(
                "unparseable-manifest",
                MANIFEST_PATH,
                format!("mod pack {MANIFEST_PATH} is not valid TOML: {e}"),
            ));
            return ValidatedModPack {
                findings,
                ..Default::default()
            };
        }
    };

    // 3a. A missing header or unsupported format rejects immediately — nothing
    //     further is worth checking, and a wall of content errors against a
    //     format this host cannot read correctly would only mislead.
    let Some(pack) = pack_manifest.pack.as_ref() else {
        findings.push(archive_error(
            "missing-pack-header",
            MANIFEST_PATH,
            format!("mod pack {MANIFEST_PATH} has no required [pack] identity table"),
        ));
        return ValidatedModPack {
            findings,
            ..Default::default()
        };
    };
    if pack.format > SUPPORTED_PACK_FORMAT {
        findings.push(archive_error(
            "unsupported-pack-format",
            MANIFEST_PATH,
            format!(
                "mod pack declares [pack] format {} but this host supports at most {SUPPORTED_PACK_FORMAT}",
                pack.format
            ),
        ));
        return ValidatedModPack {
            findings,
            ..Default::default()
        };
    }

    // 3b. Identity + compatibility findings that still let content validation
    //     run (atomic acceptance blocks the pack regardless): an empty id, and a
    //     content-identity mismatch against the injected base.
    if pack.id.trim().is_empty() {
        findings.push(archive_error(
            "invalid-pack-id",
            MANIFEST_PATH,
            "mod pack [pack] id is empty or whitespace".to_string(),
        ));
    }
    if pack.requires.content_id.as_deref() != Some(base_content.id.as_str())
        || pack.requires.content_epoch != Some(base_content.epoch)
    {
        findings.push(archive_error(
            "pack-content-mismatch",
            MANIFEST_PATH,
            format!(
                "mod pack requires content id {:?} epoch {:?}, but this host provides id {:?} epoch {}",
                pack.requires.content_id,
                pack.requires.content_epoch,
                base_content.id,
                base_content.epoch,
            ),
        ));
    }

    // 3c. Multi-pack stack findings (issue #987). A duplicate pack id is a hard
    //     ERROR — the overlay stack keys packs by id, so two packs with the same
    //     id could never both be addressed. An authored path this candidate
    //     shares with an already-active pack is a non-blocking WARNING naming the
    //     winner (this candidate, loaded latest) and the shadowed loser.
    if !pack.id.trim().is_empty() && active.iter().any(|p| p.id == pack.id) {
        findings.push(archive_error(
            "duplicate-pack-id",
            MANIFEST_PATH,
            format!("a mod pack with id {:?} is already active", pack.id),
        ));
    }
    for path in files.keys() {
        if path == MANIFEST_PATH {
            continue;
        }
        for active_pack in active {
            if active_pack.files.contains_key(path) {
                findings.push(archive_finding(
                    Severity::Warning,
                    "overlapping-pack-path",
                    path,
                    format!(
                        "mod pack {:?} overrides path {path:?} also provided by active pack {:?} — {:?} wins",
                        pack.id, active_pack.id, pack.id
                    ),
                ));
            }
        }
    }

    // 4. Path whitelist — any file outside the supported authored paths (or a
    //    traversal attempt) rejects the whole pack.
    for path in files.keys() {
        if !is_allowed_content_path(path) {
            findings.push(archive_error(
                "disallowed-path",
                path,
                format!("mod pack path {path:?} is not a supported authored path"),
            ));
        }
    }

    // 5. Parse every non-manifest TOML (worlds are re-parsed by the validators
    //    below; this catches unparseable entity/faction/model files too). A
    //    `.rhai` entry is not TOML — it is compiled instead, in step 5a.
    for (path, text) in &files {
        if path == MANIFEST_PATH || path.ends_with(".rhai") {
            continue;
        }
        if let Err(e) = toml::from_str::<toml::Value>(text) {
            findings.push(archive_error(
                "unparseable-content",
                path,
                format!("mod pack file {path:?} is not valid TOML: {e}"),
            ));
        }
    }

    // 5a. Compile every script the pack carries under the SAME deny-by-default
    //     vellum sandbox M1's loader uses (issue #988): a `.rhai` that fails to
    //     compile, or reaches for a denied capability, rejects the whole pack
    //     atomically. Reconciles #856's "packs contain no executable code" — the
    //     sandbox profile, not the extension, is the trust boundary.
    findings.extend(validate_pack_scripts(&files));

    // 6. Validate the manifest, resolving worlds against pack THEN the active
    //    stack THEN base (issue #987 precedence: candidate → active → base).
    let manifest = pack_manifest.manifest;
    // The active stack, newest active first, falling through to base content.
    // This is what a NEW pack composes against for anything it does not carry
    // itself, so a fragment an EARLIER active pack supplies resolves here.
    let resolve_beneath = |path: &str| -> Option<String> {
        active
            .iter()
            .rev()
            .find_map(|p| p.files.get(path).cloned())
            .or_else(|| resolve_base(path))
    };
    let resolve_beneath = &resolve_beneath;
    let resolve = |path: &str| files.get(path).cloned().or_else(|| resolve_beneath(path));
    // Bind a shared reference so the same resolver serves both the manifest
    // validation here and the per-world composition checks below (`&F: Fn` is
    // Copy, so this passes by value without moving the closure).
    let resolve = &resolve;
    findings.extend(validate_manifest(&manifest, &manifest_toml, resolve));

    // Raw fragment text and parsed templates, candidate-first then the active
    // stack then base (issue #906, #973 review F3, #987). Built once, outside the
    // per-world loop: they depend only on the archive + active stack, and
    // `PackTemplates` borrows the fragment source.
    let pack_fragments = PackFragments {
        files: &files,
        resolve_base: resolve_beneath,
    };
    let pack_templates = PackTemplates {
        files: &files,
        fragments: &pack_fragments,
        host: template_loader,
    };

    // 7. Composition references for every manifest-listed root world that
    //    resolves + parses (unresolved / unparseable worlds are already
    //    reported by validate_manifest above).
    for entry in &manifest.scenarios {
        let world_path = entry.world.trim();
        if world_path.is_empty() {
            continue;
        }
        let Some(world_toml) = resolve(world_path) else {
            continue;
        };
        let Ok(root_config) = parse_world(&world_toml) else {
            continue;
        };

        // Resolve declared extra_worlds children from pack + base.
        let mut child_tomls: Vec<(String, String)> = Vec::new();
        for child_path in &root_config.extra_worlds {
            if let Some(child_toml) = resolve(child_path) {
                child_tomls.push((child_path.clone(), child_toml));
            }
        }
        let child_configs: Vec<(String, String, _)> = child_tomls
            .into_iter()
            .filter_map(|(p, toml)| parse_world(&toml).ok().map(|c| (p, toml, c)))
            .collect();

        let root_src = WorldSource::new(world_path, &world_toml, &root_config);
        let child_srcs: Vec<WorldSource> = child_configs
            .iter()
            .map(|(p, toml, c)| WorldSource::new(p.clone(), toml, c))
            .collect();
        findings.extend(validate_composition_with_fragments(
            &root_src,
            &child_srcs,
            &pack_templates,
            &pack_fragments,
        ));
    }

    // 8. On success, hand back the supported authored files (excluding the
    //    manifest itself) for the session overlay.
    let overlay_files: BTreeMap<String, String> = files
        .iter()
        .filter(|(p, _)| p.as_str() != MANIFEST_PATH)
        .map(|(p, t)| (p.clone(), t.clone()))
        .collect();

    ValidatedModPack {
        findings,
        files: overlay_files,
        manifest_toml,
    }
}

// ── Pack script compilation (issue #988) ─────────────────────────────────────

/// A [`ScriptResolver`] over an uploaded pack's OWN files, so a world's
/// `script = "sibling.rhai"` resolves to the `.rhai` the pack carries. The
/// upload-time twin of the overlay-backed resolver a live session uses
/// ([`crate::entities::config_cache::OverlayScriptResolver`]); here the archive is wholly
/// in hand, so absence is final and a missing sibling is a real error.
struct PackScriptFiles<'a> {
    files: &'a BTreeMap<String, String>,
}

impl ScriptResolver for PackScriptFiles<'_> {
    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
}

/// Whether a sandbox compile error names a statically denied capability.
///
/// The deny-by-default profile refuses `eval` at PARSE time (`disable_symbol`),
/// so that one denial surfaces as a compile error rather than a top-level run
/// error; the exact wording is rhai's, so we match the capability name it
/// reports. This only refines the DIAGNOSTIC category — the pack is rejected
/// whichever bucket the finding lands in — so the trust boundary never rests on
/// this string match.
fn names_denied_capability(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("eval") || m.contains("import") || m.contains("module") || m.contains("timestamp")
}

/// Translate a [`compile_scripts`] finding into a pack-scoped one (issue #988).
///
/// The loader reports parse and top-level-run failures under its own categories;
/// a pack reports them as `unparseable-script` / `denied-script-capability`. A
/// top-level RUN failure under the deny-by-default profile always means the
/// script reached for something the sandbox refuses (an `import` the dummy
/// resolver rejects, the absent wall-clock `timestamp`), so it is a denied
/// capability. A COMPILE failure is a plain syntax error — `unparseable-script`
/// — UNLESS it names a statically denied capability (`eval`), the one denial
/// visible before the top level runs. Any other finding is passed through
/// unchanged so nothing silently vanishes.
fn map_script_finding(f: WorldFinding) -> WorldFinding {
    let category = match f.category {
        "script-parse-error" => {
            if names_denied_capability(&f.message) {
                "denied-script-capability"
            } else {
                "unparseable-script"
            }
        }
        "script-runtime-error" => "denied-script-capability",
        _ => return f,
    };
    archive_error(category, &f.source.reference, f.message)
}

/// Compile every Rhai script an uploaded pack carries, atomically (issue #988).
///
/// Packs MAY carry `.rhai`; the deny-by-default vellum sandbox profile — NOT the
/// file extension — is the trust boundary (reconciles #856). Two sources reach
/// the gate, exactly as they reach a live world through `world::script::load`:
///
///   * a standalone `assets/worlds/*.rhai` file the pack carries, and
///   * an inline `[script.*]` table (or a `script = "sibling.rhai"` reference)
///     in a pack-carried world.
///
/// Both are lifted to [`ScriptSource`]s and compiled through the SAME
/// [`compile_scripts`] a shipped world uses, so the trust boundary is literally
/// M1's gate rather than a re-implementation of it. Sources are keyed by path in
/// a `BTreeMap` so a sibling that a world both carries AND references is compiled
/// once, mirroring the loader's single AST map. A script that fails to compile is
/// `unparseable-script`; one that reaches for a denied capability (eval,
/// import/module resolve, the wall clock) is `denied-script-capability`. Either
/// is a definite error, so acceptance (gated on [`has_error`]) rejects the whole
/// pack.
fn validate_pack_scripts(files: &BTreeMap<String, String>) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    // One source set, keyed by path (a sibling `.rhai` is both a pack file AND
    // the target of a world's `script = "..."`).
    let mut sources: BTreeMap<String, String> = BTreeMap::new();

    // Standalone `.rhai` files. A script referenced by no world is still
    // compiled here — the host gates capability; the editor gates reference.
    for (path, text) in files {
        if path.ends_with(".rhai") {
            sources.entry(path.clone()).or_insert_with(|| text.clone());
        }
    }

    // Inline `[script.*]` blocks + `script = "sibling.rhai"` references in every
    // pack-carried world, lifted exactly as the loader does.
    let resolver = PackScriptFiles { files };
    for (path, text) in files {
        if !is_world_content_path(path) {
            continue;
        }
        let Ok(world) = toml::from_str::<toml::Value>(text) else {
            // Unparseable worlds are already reported by step 5 / validate_manifest.
            continue;
        };
        let (lifted, lift_findings) = lift_world_scripts(path, &world, &resolver);
        findings.extend(lift_findings);
        for s in lifted {
            sources.entry(s.path).or_insert(s.source);
        }
    }

    if sources.is_empty() {
        return findings;
    }

    let source_vec: Vec<ScriptSource> = sources
        .into_iter()
        .map(|(path, source)| ScriptSource { path, source })
        .collect();
    let compiled = compile_scripts(&source_vec);
    findings.extend(compiled.findings.into_iter().map(map_script_finding));
    findings
}

#[cfg(test)]
#[path = "mod_pack_tests.rs"]
mod tests;
