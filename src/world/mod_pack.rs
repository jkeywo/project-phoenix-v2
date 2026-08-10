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

use crate::entity_includes::FragmentSource;
use crate::entity_loader::TemplateLoader;
use crate::world::config::parse_world;
use crate::world::manifest::{
    parse_pack_manifest, validate_manifest, ContentIdentity, SUPPORTED_PACK_FORMAT,
};
use crate::world::validate::{
    has_error, validate_composition_with_fragments, Severity, SourceLocation, WorldFinding,
    WorldSource,
};

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
/// `assets/*` directory, end in `.toml`, carry a real file name, and contain
/// no path traversal or backslash.
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

/// Build a single archive-scoped error finding (no line lookup — the archive is
/// not a single source file).
fn archive_error(category: &'static str, reference: &str, message: String) -> WorldFinding {
    WorldFinding {
        severity: Severity::Error,
        category,
        message,
        source: SourceLocation {
            file: MANIFEST_PATH.to_string(),
            line: None,
            reference: reference.to_string(),
        },
    }
}

/// The raw-TOML source an uploaded pack's entity composition resolves against
/// (issue #906).
///
/// Pack files FIRST, then base content through the caller's resolver — the same
/// order, and for the same reason, as the world resolution above: a pack that
/// carries a fragment must be validated against the fragment it carries.
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
/// [`crate::entity_loader::WasmTemplateLoader`], the same type the documented
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
    fn load_template(&self, path: &str) -> Option<crate::entity_config::EntityConfig> {
        // Both spellings, because a world may name `./assets/entities/x.toml`
        // for a pack entry keyed `assets/entities/x.toml`.
        let canonical = crate::entity_includes::canonical_template_path(path);
        if self.files.contains_key(path) || self.files.contains_key(&canonical) {
            return crate::entity_includes::resolve_template(path, self.fragments)
                .ok()?
                .parse()
                .ok();
        }
        self.host.load_template(path)
    }

    /// The host's answer, for the same reason
    /// [`crate::entity_loader::SpawnTemplateLoader`]'s is: serving the pack's
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
/// is `bridge::wasm_validate_mod_pack`, which is `wasm32`-only and passes
/// `WasmTemplateLoader` (`false` in the browser, so the presence check is inert
/// there); a native caller may now pass the same type and get the answer it
/// expects. The residual gap that leaves is stated on
/// [`crate::world::validate::activation_findings`]'s presence check: on a host
/// that is *not* authoritative, a pack naming a hull it does not carry is
/// caught at spawn, not at upload.
///
/// Composition references are validated per manifest-listed world against the
/// pack + base worlds, and each spawned template's `includes` closure against
/// the pack's own files + base ([`PackFragments`]).
///
/// The returned [`ValidatedModPack`] carries error findings on any failure and
/// the overlay files + manifest on success; the caller applies the overlay only
/// when [`ValidatedModPack::is_accepted`] holds.
pub fn validate_mod_pack(
    zip_bytes: &[u8],
    base_content: &ContentIdentity,
    resolve_base: impl Fn(&str) -> Option<String>,
    template_loader: &dyn TemplateLoader,
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

    // 4. Path whitelist — any file outside the supported authored paths (or a
    //    traversal attempt) rejects the whole pack.
    for path in files.keys() {
        if !is_allowed_content_path(path) {
            findings.push(archive_error(
                "disallowed-path",
                path,
                format!("mod pack path {path:?} is not a supported authored TOML path"),
            ));
        }
    }

    // 5. Parse every non-manifest TOML (worlds are re-parsed by the validators
    //    below; this catches unparseable entity/faction/model files too).
    for (path, text) in &files {
        if path == MANIFEST_PATH {
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

    // 6. Validate the manifest, resolving worlds against pack THEN base.
    let manifest = pack_manifest.manifest;
    let resolve = |path: &str| files.get(path).cloned().or_else(|| resolve_base(path));
    // Bind a shared reference so the same resolver serves both the manifest
    // validation here and the per-world composition checks below (`&F: Fn` is
    // Copy, so this passes by value without moving the closure).
    let resolve = &resolve;
    findings.extend(validate_manifest(&manifest, &manifest_toml, resolve));

    // Raw fragment text and parsed templates, both pack-first (issue #906 and
    // #973 review F3). Built once, outside the per-world loop: they depend only
    // on the archive, and `PackTemplates` borrows the fragment source.
    let pack_fragments = PackFragments {
        files: &files,
        resolve_base: &resolve_base,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Store ZIP writer (test-only twin of createStoreZip) ──────────────────

    /// Minimal store-only ZIP writer, the inverse of [`read_store_zip`], shaped
    /// exactly like `createStoreZip` in `editor/mod-pack-export.js` so the
    /// reader is exercised against the real archive layout.
    fn create_store_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut local: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        let mut offset: u32 = 0;

        for (path, text) in entries {
            let name = path.as_bytes();
            let data = text.as_bytes();
            let crc = crc32(data);

            let mut lh = Vec::new();
            lh.extend_from_slice(&LOCAL_FILE_HEADER_SIG.to_le_bytes());
            lh.extend_from_slice(&20u16.to_le_bytes()); // version needed
            lh.extend_from_slice(&0u16.to_le_bytes()); // flags
            lh.extend_from_slice(&0u16.to_le_bytes()); // method: store
            lh.extend_from_slice(&0u16.to_le_bytes()); // mod time
            lh.extend_from_slice(&0x21u16.to_le_bytes()); // mod date
            lh.extend_from_slice(&crc.to_le_bytes());
            lh.extend_from_slice(&(data.len() as u32).to_le_bytes());
            lh.extend_from_slice(&(data.len() as u32).to_le_bytes());
            lh.extend_from_slice(&(name.len() as u16).to_le_bytes());
            lh.extend_from_slice(&0u16.to_le_bytes()); // extra len
            lh.extend_from_slice(name);
            lh.extend_from_slice(data);

            let mut ch = Vec::new();
            ch.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            ch.extend_from_slice(&20u16.to_le_bytes());
            ch.extend_from_slice(&20u16.to_le_bytes());
            ch.extend_from_slice(&0u16.to_le_bytes());
            ch.extend_from_slice(&0u16.to_le_bytes());
            ch.extend_from_slice(&0u16.to_le_bytes());
            ch.extend_from_slice(&0x21u16.to_le_bytes());
            ch.extend_from_slice(&crc.to_le_bytes());
            ch.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ch.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ch.extend_from_slice(&(name.len() as u16).to_le_bytes());
            ch.extend_from_slice(&0u16.to_le_bytes()); // extra
            ch.extend_from_slice(&0u16.to_le_bytes()); // comment
            ch.extend_from_slice(&0u16.to_le_bytes()); // disk
            ch.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            ch.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            ch.extend_from_slice(&offset.to_le_bytes());
            ch.extend_from_slice(name);

            offset += lh.len() as u32;
            local.extend_from_slice(&lh);
            central.extend_from_slice(&ch);
        }

        let central_size = central.len() as u32;
        let central_offset = offset;
        let mut out = local;
        out.extend_from_slice(&central);
        // EOCD
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&central_size.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn simple_world(title: &str) -> String {
        format!("[global]\ntitle = \"{title}\"\n")
    }

    /// The base content identity the test packs declare `[pack.requires]`
    /// against, mirrored by [`base_identity`] below.
    const TEST_CONTENT_ID: &str = "phoenix-base";
    const TEST_CONTENT_EPOCH: i64 = 1;

    fn base_identity() -> ContentIdentity {
        ContentIdentity {
            id: TEST_CONTENT_ID.to_string(),
            epoch: TEST_CONTENT_EPOCH,
        }
    }

    /// A manifest carrying a well-formed `[pack]` header (matching
    /// [`base_identity`]) plus one `[[scenario]]` — the shape every pack now
    /// requires (issue #986). The identity-rejection tests below author their
    /// own headers instead.
    fn manifest_for(id: &str, world: &str) -> String {
        format!(
            "[pack]\n\
             format = 1\n\
             id = \"test-pack\"\n\
             version = \"1.0.0\"\n\
             name = \"Test Pack\"\n\n\
             [pack.requires]\n\
             content_id = \"{TEST_CONTENT_ID}\"\n\
             content_epoch = {TEST_CONTENT_EPOCH}\n\n\
             [[scenario]]\nid = \"{id}\"\nworld = \"{world}\"\n"
        )
    }

    fn no_base(_: &str) -> Option<String> {
        None
    }

    /// A host that serves no parsed entity templates at all.
    ///
    /// The reference checks that consult a `TemplateLoader` (doctrine anchors)
    /// read it as "unknown template", which is what these packs' worlds mean:
    /// they declare no entities. Injecting it — rather than letting the module
    /// reach for the host loader — is what keeps these tests independent of the
    /// filesystem and the wasm thread-locals.
    struct NoTemplates;

    impl TemplateLoader for NoTemplates {
        fn load_template(&self, _path: &str) -> Option<crate::entity_config::EntityConfig> {
            None
        }

        /// NOT authoritative (issue #973). A loader that serves nothing knows
        /// nothing; claiming authority here would have it reject every hull a
        /// pack carries as `unresolvable-template`, which is precisely the
        /// blindness that check is gated to avoid. See the note on
        /// [`validate_mod_pack`].
        fn absence_is_final(&self) -> bool {
            false
        }
    }

    // ── store ZIP reader ─────────────────────────────────────────────────────

    #[test]
    fn store_zip_round_trips() {
        let zip = create_store_zip(&[
            ("scenarios.toml", "manifest"),
            ("assets/worlds/x.toml", "body"),
        ]);
        let files = read_store_zip(&zip).expect("round-trips");
        assert_eq!(
            files.get("scenarios.toml").map(String::as_str),
            Some("manifest")
        );
        assert_eq!(
            files.get("assets/worlds/x.toml").map(String::as_str),
            Some("body")
        );
    }

    #[test]
    fn store_zip_rejects_non_store_method() {
        let mut zip = create_store_zip(&[("scenarios.toml", "manifest")]);
        // Method field is at offset +8 in the first local header.
        zip[8] = 8; // deflate
        assert!(read_store_zip(&zip).is_err());
    }

    #[test]
    fn store_zip_rejects_crc_mismatch() {
        let mut zip = create_store_zip(&[("scenarios.toml", "manifest")]);
        // Corrupt the CRC at offset +14.
        zip[14] ^= 0xff;
        assert!(read_store_zip(&zip).is_err());
    }

    #[test]
    fn store_zip_ignores_bytes_after_central_dir() {
        // The reader stops at the first non-local-header signature, so the
        // central directory + EOCD the writer appends are never mis-read.
        let zip = create_store_zip(&[("scenarios.toml", "manifest")]);
        let files = read_store_zip(&zip).unwrap();
        assert_eq!(files.len(), 1);
    }

    // ── path whitelist ───────────────────────────────────────────────────────

    #[test]
    fn whitelist_accepts_supported_paths_and_manifest() {
        assert!(is_allowed_content_path("scenarios.toml"));
        assert!(is_allowed_content_path("assets/worlds/x.toml"));
        assert!(is_allowed_content_path("assets/entities/e.toml"));
        assert!(is_allowed_content_path("assets/factions/f.toml"));
        assert!(is_allowed_content_path("assets/models/m.toml"));
    }

    #[test]
    fn whitelist_rejects_traversal_backslash_and_nesting() {
        assert!(!is_allowed_content_path("assets/worlds/../secret.toml"));
        assert!(!is_allowed_content_path("assets\\worlds\\x.toml"));
        assert!(!is_allowed_content_path("assets/worlds/sub/x.toml"));
        assert!(!is_allowed_content_path("assets/other/x.toml"));
        assert!(!is_allowed_content_path("assets/worlds/x.json"));
        assert!(!is_allowed_content_path("assets/worlds/.toml"));
    }

    // ── atomic validation ────────────────────────────────────────────────────

    #[test]
    fn valid_pack_is_accepted() {
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("modx", "assets/worlds/modx.toml"),
            ),
            ("assets/worlds/modx.toml", &simple_world("world.modx.title")),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(
            result.is_accepted(),
            "unexpected findings: {:?}",
            result.findings
        );
        // Overlay carries the world but NOT the manifest itself.
        assert!(result.files.contains_key("assets/worlds/modx.toml"));
        assert!(!result.files.contains_key("scenarios.toml"));
        assert_eq!(
            result.manifest_toml,
            manifest_for("modx", "assets/worlds/modx.toml")
        );
    }

    #[test]
    fn corrupt_archive_rejects_whole_pack() {
        // A ZIP that starts with a valid local-file-header signature but is
        // truncated mid-record fails to read → invalid-archive, applying
        // nothing (AC1).
        let mut zip = create_store_zip(&[("scenarios.toml", "manifest")]);
        zip.truncate(20); // cut off inside the first local header
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result.files.is_empty());
        assert_eq!(result.findings[0].category, "invalid-archive");
    }

    #[test]
    fn non_archive_bytes_reject_whole_pack() {
        // Bytes with no local-file-header signature parse as an empty archive,
        // so the required manifest is absent — still an atomic rejection with
        // nothing applied.
        let result =
            validate_mod_pack(b"not a zip at all", &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result.files.is_empty());
        assert!(result
            .findings
            .iter()
            .any(|f| f.category == "missing-manifest"));
    }

    #[test]
    fn missing_manifest_rejects_whole_pack() {
        let zip = create_store_zip(&[("assets/worlds/x.toml", &simple_world("t"))]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result
            .findings
            .iter()
            .any(|f| f.category == "missing-manifest"));
        assert!(result.files.is_empty());
    }

    #[test]
    fn disallowed_path_rejects_whole_pack() {
        let zip = create_store_zip(&[
            ("scenarios.toml", &manifest_for("m", "assets/worlds/m.toml")),
            ("assets/worlds/m.toml", &simple_world("t")),
            ("assets/secret/keys.toml", "danger = true"),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result
            .findings
            .iter()
            .any(|f| f.category == "disallowed-path"));
    }

    #[test]
    fn unresolved_manifest_world_rejects_whole_pack() {
        // Manifest names a world neither in the pack nor in base content.
        let zip = create_store_zip(&[(
            "scenarios.toml",
            &manifest_for("ghost", "assets/worlds/ghost.toml"),
        )]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result
            .findings
            .iter()
            .any(|f| f.category == "missing-scenario-world"));
    }

    #[test]
    fn unparseable_content_rejects_whole_pack() {
        let zip = create_store_zip(&[
            ("scenarios.toml", &manifest_for("m", "assets/worlds/m.toml")),
            ("assets/worlds/m.toml", &simple_world("t")),
            ("assets/entities/broken.toml", "not valid ["),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result
            .findings
            .iter()
            .any(|f| f.category == "unparseable-content"));
    }

    #[test]
    fn unresolved_composition_reference_rejects_whole_pack() {
        // A world whose objective transition references an undeclared id is a
        // definite composition error (validate.rs), blocking the whole pack.
        let bad_world = r#"
[global]
title = "world.bad.title"

[[trigger]]
condition = "on_destroyed"
entity = "raider"

  [[trigger.action]]
  type = "complete_objective"
  id = "obj-ghost"
"#;
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("bad", "assets/worlds/bad.toml"),
            ),
            ("assets/worlds/bad.toml", bad_world),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(result
            .findings
            .iter()
            .any(|f| f.category == "unresolved-objective-reference"));
    }

    #[test]
    fn manifest_world_resolves_against_base_content() {
        // The pack ships no world, but the manifest names a base world the host
        // has already fetched — resolve_base supplies it, so the pack is valid.
        let zip = create_store_zip(&[(
            "scenarios.toml",
            &manifest_for("base_ref", "assets/worlds/default.toml"),
        )]);
        let base = |p: &str| {
            if p == "assets/worlds/default.toml" {
                Some(simple_world("world.default.title"))
            } else {
                None
            }
        };
        let result = validate_mod_pack(&zip, &base_identity(), base, &NoTemplates);
        assert!(result.is_accepted(), "findings: {:?}", result.findings);
    }

    // ── Composition resolves against the PACK's own files (issue #906) ───────

    /// A world spawning one entity template, for the composition tests below.
    fn world_spawning(template_path: &str, name: &str) -> String {
        format!(
            "[global]\ntitle = \"world.pack.title\"\n\n\
             [[entity]]\ntemplate_path = \"{template_path}\"\nname = \"{name}\"\n"
        )
    }

    /// A pack that carries BOTH a composed hull and the fragment it includes
    /// validates.
    ///
    /// This is the case a host-defaulted fragment source gets wrong: it looks in
    /// the session overlay (not installed — validation is atomic and comes
    /// first), then the host's raw templates, then disk, and finds a fragment
    /// that exists only inside the archive in none of them. The pack would be
    /// rejected for an `include-missing` that is not true.
    ///
    /// The base resolver deliberately knows the HULL: the pack overrides a
    /// shipped hull with a composed one and supplies the fragment itself. That
    /// detail is what gives the test teeth — a composition check that cannot
    /// see the root at all reports nothing by design (blindness is not a
    /// finding), so a base that knew neither file would let a wrong fragment
    /// source pass unnoticed.
    #[test]
    fn pack_carrying_a_hull_and_its_fragment_validates() {
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("modc", "assets/worlds/modc.toml"),
            ),
            (
                "assets/worlds/modc.toml",
                &world_spawning("assets/entities/pack_hull.toml", "pack_one"),
            ),
            (
                "assets/entities/pack_hull.toml",
                "includes = [\"pack_core.toml\"]\nname = \"Pack Hull\"\n",
            ),
            ("assets/entities/pack_core.toml", "class = \"escort\"\n"),
        ]);
        let base = |p: &str| {
            if p == "assets/entities/pack_hull.toml" {
                // The shipped hull the pack replaces. It declares the same
                // include, and the base resolver canNOT serve that fragment —
                // so if composition resolves against base instead of the pack,
                // it reports `include-missing` and the pack is wrongly rejected.
                Some("includes = [\"pack_core.toml\"]\nname = \"Shipped Hull\"\n".to_string())
            } else {
                None
            }
        };
        let result = validate_mod_pack(&zip, &base_identity(), base, &NoTemplates);
        assert!(
            result.is_accepted(),
            "a pack's hull must compose against the fragment the pack itself \
             carries: {:?}",
            result.findings
        );
    }

    /// The same pack MINUS the fragment is rejected — the check is real, not
    /// merely permissive.
    #[test]
    fn pack_carrying_a_hull_without_its_fragment_is_rejected() {
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("modd", "assets/worlds/modd.toml"),
            ),
            (
                "assets/worlds/modd.toml",
                &world_spawning("assets/entities/pack_hull.toml", "pack_one"),
            ),
            (
                "assets/entities/pack_hull.toml",
                "includes = [\"pack_core.toml\"]\nname = \"Pack Hull\"\n",
            ),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted(), "findings: {:?}", result.findings);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == "include-missing"),
            "expected an include-missing finding: {:?}",
            result.findings
        );
    }

    /// A pack hull may include a SHIPPED fragment: what the pack does not carry
    /// falls through to the injected base resolver, exactly as a manifest root
    /// world does.
    #[test]
    fn pack_hull_may_include_a_base_fragment() {
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("mode", "assets/worlds/mode.toml"),
            ),
            (
                "assets/worlds/mode.toml",
                &world_spawning("assets/entities/pack_hull.toml", "pack_one"),
            ),
            (
                "assets/entities/pack_hull.toml",
                "includes = [\"base_core.toml\"]\nname = \"Pack Hull\"\n",
            ),
        ]);
        let base = |p: &str| {
            if p == "assets/entities/base_core.toml" {
                Some("class = \"escort\"\n".to_string())
            } else {
                None
            }
        };
        let result = validate_mod_pack(&zip, &base_identity(), base, &NoTemplates);
        assert!(result.is_accepted(), "findings: {:?}", result.findings);
    }

    // ── Authoritative loaders (issue #973 review, F3) ────────────────────────

    /// A loader that serves nothing AND claims authority over absence.
    ///
    /// Exactly what a future NATIVE caller gets from
    /// [`crate::entity_loader::WasmTemplateLoader`] with an empty cache and no
    /// pack file on disk — the same type the documented wasm caller passes, so
    /// it is the obvious thing to write and answers `true` on native. Every
    /// other fixture in this module answers `false`, which is why the dangerous
    /// arm went unexercised while the constraint lived in a doc comment.
    struct AuthoritativeNoTemplates;

    impl TemplateLoader for AuthoritativeNoTemplates {
        fn load_template(&self, _path: &str) -> Option<crate::entity_config::EntityConfig> {
            None
        }

        fn absence_is_final(&self) -> bool {
            true
        }
    }

    /// A pack's own hulls are served in front of the caller's loader, so an
    /// authoritative loader that can see none of them still accepts the pack.
    ///
    /// Before F3 this rejected every valid pack with one bogus
    /// `unresolvable-template` per hull.
    #[test]
    fn a_pack_carrying_its_own_hull_is_accepted_by_an_authoritative_loader() {
        assert!(AuthoritativeNoTemplates.absence_is_final(), "precondition");
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("modf", "assets/worlds/modf.toml"),
            ),
            (
                "assets/worlds/modf.toml",
                &world_spawning("assets/entities/pack_hull.toml", "pack_one"),
            ),
            ("assets/entities/pack_hull.toml", "name = \"Pack Hull\"\n"),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &AuthoritativeNoTemplates);
        assert!(
            result.is_accepted(),
            "a hull the pack itself carries must not read as absent: {:?}",
            result.findings
        );
    }

    /// …and the wrapping does not blanket-suppress the check: a pack naming a
    /// hull that exists neither in the archive nor anywhere the loader can see
    /// is still rejected, on a host authoritative enough to say so.
    #[test]
    fn a_pack_naming_a_hull_it_does_not_carry_is_rejected_by_an_authoritative_loader() {
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                &manifest_for("modg", "assets/worlds/modg.toml"),
            ),
            (
                "assets/worlds/modg.toml",
                &world_spawning("assets/entities/absent_hull.toml", "pack_one"),
            ),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &AuthoritativeNoTemplates);
        assert!(!result.is_accepted(), "findings: {:?}", result.findings);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == "unresolvable-template"),
            "expected an unresolvable-template finding: {:?}",
            result.findings
        );
    }

    // ── Pack identity + compatibility gate (issue #986) ──────────────────────

    fn has_category(result: &ValidatedModPack, category: &str) -> bool {
        result.findings.iter().any(|f| f.category == category)
    }

    /// A pack whose manifest carries no `[pack]` table is rejected.
    #[test]
    fn missing_pack_header_rejects_whole_pack() {
        // The bare pre-#986 manifest shape — no [pack] table.
        let zip = create_store_zip(&[
            (
                "scenarios.toml",
                "[[scenario]]\nid = \"m\"\nworld = \"assets/worlds/m.toml\"\n",
            ),
            ("assets/worlds/m.toml", &simple_world("world.m.title")),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "missing-pack-header"));
        assert!(result.files.is_empty());
    }

    /// A `format` above the host's supported max is rejected BEFORE any content
    /// validation runs — the manifest here references a world that is NOT in the
    /// pack, which would be a `missing-scenario-world` if content validation ran,
    /// but the format gate short-circuits so only that one finding appears.
    #[test]
    fn unsupported_pack_format_rejects_before_content_validation() {
        let manifest = format!(
            "[pack]\nformat = {}\nid = \"future\"\nversion = \"9.0.0\"\nname = \"Future Pack\"\n\n\
             [pack.requires]\ncontent_id = \"{TEST_CONTENT_ID}\"\ncontent_epoch = {TEST_CONTENT_EPOCH}\n\n\
             [[scenario]]\nid = \"ghost\"\nworld = \"assets/worlds/ghost.toml\"\n",
            SUPPORTED_PACK_FORMAT + 1,
        );
        let zip = create_store_zip(&[("scenarios.toml", &manifest)]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "unsupported-pack-format"));
        // No wall of content errors: the missing world is never reached.
        assert!(
            !has_category(&result, "missing-scenario-world"),
            "content validation must not run for an unsupported format: {:?}",
            result.findings,
        );
        assert_eq!(
            result.findings.len(),
            1,
            "only the format finding: {:?}",
            result.findings
        );
    }

    /// An empty / whitespace `[pack] id` is rejected.
    #[test]
    fn invalid_pack_id_rejects_whole_pack() {
        let manifest = format!(
            "[pack]\nformat = 1\nid = \"   \"\nversion = \"1.0.0\"\nname = \"Nameless\"\n\n\
             [pack.requires]\ncontent_id = \"{TEST_CONTENT_ID}\"\ncontent_epoch = {TEST_CONTENT_EPOCH}\n\n\
             [[scenario]]\nid = \"s\"\nworld = \"assets/worlds/s.toml\"\n",
        );
        let zip = create_store_zip(&[
            ("scenarios.toml", &manifest),
            ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "invalid-pack-id"));
    }

    /// A `requires.content_id` that does not match the injected base content is
    /// a `pack-content-mismatch`.
    #[test]
    fn pack_content_id_mismatch_rejects_whole_pack() {
        let manifest = "[pack]\nformat = 1\nid = \"x\"\nversion = \"1.0.0\"\nname = \"X\"\n\n\
             [pack.requires]\ncontent_id = \"some-other-content\"\ncontent_epoch = 1\n\n\
             [[scenario]]\nid = \"s\"\nworld = \"assets/worlds/s.toml\"\n";
        let zip = create_store_zip(&[
            ("scenarios.toml", manifest),
            ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "pack-content-mismatch"));
    }

    /// A `requires.content_epoch` that does not match the injected base content
    /// is a `pack-content-mismatch`, even when the id matches.
    #[test]
    fn pack_content_epoch_mismatch_rejects_whole_pack() {
        let manifest = format!(
            "[pack]\nformat = 1\nid = \"x\"\nversion = \"1.0.0\"\nname = \"X\"\n\n\
             [pack.requires]\ncontent_id = \"{TEST_CONTENT_ID}\"\ncontent_epoch = {}\n\n\
             [[scenario]]\nid = \"s\"\nworld = \"assets/worlds/s.toml\"\n",
            TEST_CONTENT_EPOCH + 1,
        );
        let zip = create_store_zip(&[
            ("scenarios.toml", &manifest),
            ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ]);
        let result = validate_mod_pack(&zip, &base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "pack-content-mismatch"));
    }

    // ── Committed fixtures, validated through the real archive bytes ─────────
    //
    // The SAME .zip bytes are read on the JS side through `readStoreZip`
    // (editor/tests/mod-pack-export.test.js), so the two languages are proven to
    // agree on one archive. Regenerate the fixtures with
    // `node scripts/build-mod-pack-fixtures.mjs` (gated by `--check`).

    /// The base identity the committed fixtures declare `[pack.requires]`
    /// against — kept in step with scripts/build-mod-pack-fixtures.mjs.
    fn fixture_base_identity() -> ContentIdentity {
        ContentIdentity {
            id: "phoenix-base".to_string(),
            epoch: 1,
        }
    }

    #[test]
    fn fixture_valid_v1_is_accepted() {
        let bytes = include_bytes!("../../tests/fixtures/mod-packs/valid-v1.zip");
        let result = validate_mod_pack(bytes, &fixture_base_identity(), no_base, &NoTemplates);
        assert!(
            result.is_accepted(),
            "valid-v1.zip must be accepted: {:?}",
            result.findings
        );
    }

    #[test]
    fn fixture_format_too_new_is_rejected() {
        let bytes = include_bytes!("../../tests/fixtures/mod-packs/format-too-new.zip");
        let result = validate_mod_pack(bytes, &fixture_base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "unsupported-pack-format"));
    }

    #[test]
    fn fixture_content_epoch_mismatch_is_rejected() {
        let bytes = include_bytes!("../../tests/fixtures/mod-packs/content-epoch-mismatch.zip");
        let result = validate_mod_pack(bytes, &fixture_base_identity(), no_base, &NoTemplates);
        assert!(!result.is_accepted());
        assert!(has_category(&result, "pack-content-mismatch"));
    }
}
