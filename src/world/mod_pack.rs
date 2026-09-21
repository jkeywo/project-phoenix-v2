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

use std::collections::{BTreeMap, BTreeSet};

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

/// Partial String Table carried by a translation-only ordinary mod.
pub const STRING_CATALOGUE_PATH: &str = "assets/strings/strings.csv";

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
    if path.contains("..")
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return false;
    }
    if is_pack_asset_path(path)
        || path == MANIFEST_PATH
        || path == crate::sound_cues::PATH
        || path == STRING_CATALOGUE_PATH
    {
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

/// Formats consumed by the current model/image/audio loaders. Binary members
/// may use subdirectories; every component is a portable content name.
pub fn is_pack_asset_path(path: &str) -> bool {
    if !path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with(['.', ' '])
            && !part.contains(['\\', ':'])
            && !part.chars().any(char::is_control)
    }) {
        return false;
    }
    let Some((_, extension)) = path.rsplit_once('.') else {
        return false;
    };
    (path.starts_with("assets/models/")
        && matches!(extension, "glb" | "bin" | "png" | "jpg" | "jpeg" | "ktx2"))
        || ((path.starts_with("assets/textures/") || path.starts_with("assets/planets/"))
            && matches!(extension, "png" | "jpg" | "jpeg" | "ktx2" | "ptex"))
        || (path.starts_with("assets/sounds/") && matches!(extension, "wav" | "ogg" | "mp3"))
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
/// Local headers and the central directory must describe exactly the same
/// members. Trailing bytes and incomplete envelopes are refused.
pub fn read_store_zip(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    read_store_zip_bytes(bytes)?
        .into_iter()
        .map(|(path, bytes)| {
            String::from_utf8(bytes)
                .map(|text| (path.clone(), text))
                .map_err(|_| format!("file {path:?} is not valid UTF-8"))
        })
        .collect()
}

/// Decode the same archive envelope while retaining exact model, texture and
/// sound bytes. Source validators decide which members must be UTF-8. Duplicate
/// member names are refused, so no reader can disagree about the winning file.
pub fn read_store_zip_bytes(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let mut locals = BTreeMap::new();
    let mut pos = 0usize;

    while pos + 4 <= bytes.len() && read_u32_le(bytes, pos) == Some(LOCAL_FILE_HEADER_SIG) {
        let flags = read_u16_le(bytes, pos + 6).ok_or("truncated local file header")?;
        let method = read_u16_le(bytes, pos + 8).ok_or("truncated local file header")?;
        let crc = read_u32_le(bytes, pos + 14).ok_or("truncated local file header")?;
        let comp_size = read_u32_le(bytes, pos + 18).ok_or("truncated local file header")? as usize;
        let size = read_u32_le(bytes, pos + 22).ok_or("truncated local file header")? as usize;
        let name_len = read_u16_le(bytes, pos + 26).ok_or("truncated local file header")? as usize;
        let extra_len = read_u16_le(bytes, pos + 28).ok_or("truncated local file header")? as usize;
        let name_start = pos + 30;
        let data_start = name_start
            .checked_add(name_len)
            .and_then(|value| value.checked_add(extra_len))
            .ok_or("ZIP member offset overflows")?;
        let data_end = data_start
            .checked_add(comp_size)
            .ok_or("ZIP member size overflows")?;

        if flags & !0x0800 != 0 {
            return Err("encrypted or streaming ZIP members are unsupported".into());
        }
        if method != 0 {
            return Err(format!("unsupported compression method {method}"));
        }
        if comp_size != size {
            return Err("stored ZIP member sizes disagree".into());
        }

        let name_bytes = bytes
            .get(name_start..name_start + name_len)
            .ok_or("truncated file name")?;
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| "file name is not valid UTF-8".to_string())?
            .to_string();

        let data = bytes
            .get(data_start..data_end)
            .ok_or("truncated file data")?;
        if crc32(data) != crc {
            return Err(format!("CRC mismatch for {name:?}"));
        }
        if files.insert(name.clone(), data.to_vec()).is_some() {
            return Err(format!("duplicate ZIP member {name:?}"));
        }
        locals.insert(name, (pos, flags, crc, size));
        pos = data_end;
    }

    let central_start = pos;
    let mut central_names = std::collections::BTreeSet::new();
    while read_u32_le(bytes, pos) == Some(0x0201_4b50) {
        if bytes.get(pos..pos + 46).is_none() {
            return Err("truncated central directory".into());
        }
        let flags = read_u16_le(bytes, pos + 8).ok_or("truncated central flags")?;
        let method = read_u16_le(bytes, pos + 10).ok_or("truncated central method")?;
        let crc = read_u32_le(bytes, pos + 16).ok_or("truncated central CRC")?;
        let compressed = read_u32_le(bytes, pos + 20).ok_or("truncated central size")? as usize;
        let size = read_u32_le(bytes, pos + 24).ok_or("truncated central size")? as usize;
        let name_len = read_u16_le(bytes, pos + 28).ok_or("truncated central name")? as usize;
        let extra_len = read_u16_le(bytes, pos + 30).ok_or("truncated central extra")? as usize;
        let comment_len = read_u16_le(bytes, pos + 32).ok_or("truncated central comment")? as usize;
        let disk = read_u16_le(bytes, pos + 34).ok_or("truncated central disk")?;
        let offset = read_u32_le(bytes, pos + 42).ok_or("truncated central offset")? as usize;
        let name_start = pos + 46;
        let next = name_start
            .checked_add(name_len)
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .filter(|end| *end <= bytes.len())
            .ok_or("truncated central entry")?;
        let name = std::str::from_utf8(&bytes[name_start..name_start + name_len])
            .map_err(|_| "central file name is not valid UTF-8")?;
        if method != 0
            || compressed != size
            || disk != 0
            || locals.get(name) != Some(&(offset, flags, crc, size))
            || !central_names.insert(name)
        {
            return Err(format!("central directory disagrees with member {name:?}"));
        }
        pos = next;
    }
    if read_u32_le(bytes, pos) != Some(0x0605_4b50) {
        return Err("missing ZIP end record".into());
    }
    let disk = read_u16_le(bytes, pos + 4).ok_or("truncated ZIP end record")?;
    let central_disk = read_u16_le(bytes, pos + 6).ok_or("truncated ZIP end record")?;
    let disk_count = read_u16_le(bytes, pos + 8).ok_or("truncated ZIP end record")? as usize;
    let count = read_u16_le(bytes, pos + 10).ok_or("truncated ZIP end record")? as usize;
    let central_size = read_u32_le(bytes, pos + 12).ok_or("truncated ZIP end record")? as usize;
    let offset = read_u32_le(bytes, pos + 16).ok_or("truncated ZIP end record")? as usize;
    let comment = read_u16_le(bytes, pos + 20).ok_or("truncated ZIP end record")? as usize;
    if disk != 0
        || central_disk != 0
        || disk_count != count
        || count != locals.len()
        || count != central_names.len()
        || offset != central_start
        || central_size != pos - central_start
        || pos.checked_add(22).and_then(|end| end.checked_add(comment)) != Some(bytes.len())
    {
        return Err("ZIP end record disagrees with its members".into());
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
    pub assets: BTreeMap<String, std::sync::Arc<[u8]>>,
    pub source_archive: Option<std::sync::Arc<[u8]>>,
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

/// A decoded member (including an unchanged dependency) failed validation.
/// Attribute it to the file the author must repair, not the pack manifest.
fn member_error(category: &'static str, path: &str, message: String) -> WorldFinding {
    let mut finding = archive_error(category, path, message);
    finding.source.file = path.to_owned();
    finding
}

fn member_warning(category: &'static str, path: &str, message: String) -> WorldFinding {
    let mut finding = archive_finding(Severity::Warning, category, path, message);
    finding.source.file = path.to_owned();
    finding
}

fn parse_catalogue_csv(source: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = source.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' && field.is_empty() {
            quoted = true;
        } else if ch == ',' {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' {
            row.push(std::mem::take(&mut field));
            if !(row.len() == 1 && row[0].is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else if ch != '\r' {
            field.push(ch);
        }
    }
    if quoted {
        return Err("unterminated quoted field".into());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn catalogue_locale(name: &str) -> bool {
    if name == "id" {
        return false;
    }
    let mut parts = name.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    let language_ok =
        (2..=3).contains(&language.len()) && language.chars().all(|ch| ch.is_ascii_lowercase());
    let region_ok = match parts.next() {
        None => true,
        Some(region) => region.len() == 2 && region.chars().all(|ch| ch.is_ascii_uppercase()),
    };
    language_ok && region_ok && parts.next().is_none()
}

fn catalogue_placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        let value = &rest[..close];
        if !value.is_empty()
            && value
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            found.push(value);
        }
        rest = &rest[close + 1..];
    }
    found.sort_unstable();
    found
}

fn validate_string_catalogue(source: &str) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let rows = match parse_catalogue_csv(source) {
        Ok(rows) if !rows.is_empty() => rows,
        Ok(_) => {
            return vec![member_error(
                "string-catalogue-schema",
                STRING_CATALOGUE_PATH,
                "String Table is empty".into(),
            )];
        }
        Err(error) => {
            return vec![member_error(
                "malformed-string-catalogue",
                STRING_CATALOGUE_PATH,
                format!("String Table CSV is malformed: {error}"),
            )];
        }
    };
    let header = &rows[0];
    let id_col = header.iter().position(|name| name == "id");
    if id_col.is_none() {
        findings.push(member_error(
            "string-catalogue-schema",
            STRING_CATALOGUE_PATH,
            "String Table is missing required 'id' column".into(),
        ));
    }
    let locales: Vec<_> = header
        .iter()
        .filter(|name| catalogue_locale(name))
        .collect();
    if locales.is_empty() {
        findings.push(member_error(
            "string-catalogue-schema",
            STRING_CATALOGUE_PATH,
            "String Table has no locale columns".into(),
        ));
    }
    let mut columns = BTreeSet::new();
    if header.iter().any(|name| !columns.insert(name)) {
        findings.push(member_error(
            "string-catalogue-schema",
            STRING_CATALOGUE_PATH,
            "String Table has duplicate column names".into(),
        ));
    }
    for name in header {
        if let Some(locale) = name
            .strip_suffix("_source")
            .or_else(|| name.strip_suffix("_provenance"))
        {
            if !locales.iter().any(|candidate| candidate.as_str() == locale) {
                findings.push(member_error(
                    "string-catalogue-schema",
                    STRING_CATALOGUE_PATH,
                    format!("{name} has no matching {locale} locale column"),
                ));
            }
        }
    }
    let mut ids = BTreeSet::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        let id = id_col
            .and_then(|col| row.get(col))
            .map_or("", String::as_str)
            .trim();
        if row.len() != header.len() {
            findings.push(member_error(
                "malformed-string-catalogue",
                STRING_CATALOGUE_PATH,
                format!(
                    "row {} ({id}) has {} fields; header has {}",
                    index + 1,
                    row.len(),
                    header.len()
                ),
            ));
            continue;
        }
        if id.is_empty() || !ids.insert(id) {
            findings.push(member_error(
                "string-catalogue-schema",
                STRING_CATALOGUE_PATH,
                if id.is_empty() {
                    format!("row {} has a blank id", index + 1)
                } else {
                    format!("String Table has duplicate id {id:?}")
                },
            ));
        }
        for locale in locales.iter().filter(|locale| locale.as_str() != "en") {
            let value = &row[header
                .iter()
                .position(|name| name == locale.as_str())
                .unwrap()];
            if value.trim().is_empty() {
                findings.push(member_warning(
                    "translation-fallback",
                    STRING_CATALOGUE_PATH,
                    format!("{id}: {locale} is blank; players use effective English"),
                ));
                continue;
            }
            let source_name = format!("{locale}_source");
            let source = header
                .iter()
                .position(|name| name == &source_name)
                .map(|col| row[col].as_str())
                .unwrap_or("");
            if source.is_empty() {
                findings.push(member_warning(
                    "translation-fallback",
                    STRING_CATALOGUE_PATH,
                    format!(
                        "{id}: {source_name} is missing or blank; players use effective English"
                    ),
                ));
                continue;
            }
            let english = header
                .iter()
                .position(|name| name == "en")
                .map(|col| row[col].as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or(source);
            if catalogue_placeholders(english) != catalogue_placeholders(value) {
                findings.push(member_warning("invalid-translation-placeholders", STRING_CATALOGUE_PATH,
                    format!("{id}: {locale} placeholders do not match its English source; players use effective English")));
            }
        }
    }
    findings
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
    validate_mod_pack_with_assets(
        zip_bytes,
        base_content,
        resolve_base,
        template_loader,
        active,
        &|_| None,
        &[],
    )
}

/// The same admission with an explicit immutable base-asset snapshot. A
/// dependency resolves candidate first, then accepted packs, then this base.
pub fn validate_mod_pack_with_assets(
    zip_bytes: &[u8],
    base_content: &ContentIdentity,
    resolve_base: impl Fn(&str) -> Option<String>,
    template_loader: &dyn TemplateLoader,
    active: &[ActivePack],
    resolve_base_asset: &crate::world::pack_asset_validation::AssetResolver<'_>,
    base_asset_paths: &[String],
) -> ValidatedModPack {
    // 1. Parse the store ZIP — a malformed / non-store / CRC-mismatched archive
    //    rejects the whole pack.
    let members = match read_store_zip_bytes(zip_bytes) {
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
    let mut deferred_content_findings = Vec::new();
    let mut files = BTreeMap::new();
    let mut assets: BTreeMap<String, std::sync::Arc<[u8]>> = BTreeMap::new();
    for (path, bytes) in members {
        if is_pack_asset_path(&path) {
            if bytes.is_empty() {
                deferred_content_findings.push(archive_error(
                    "empty-asset",
                    &path,
                    "Pack asset is empty".into(),
                ));
            }
            assets.insert(path, std::sync::Arc::from(bytes));
        } else {
            match String::from_utf8(bytes) {
                Ok(text) => {
                    files.insert(path, text);
                }
                Err(_) => deferred_content_findings.push(archive_error(
                    "invalid-content-encoding",
                    &path,
                    "Authored source is not valid UTF-8".into(),
                )),
            }
        }
    }
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
    for path in files.keys().chain(assets.keys()) {
        if path == MANIFEST_PATH {
            continue;
        }
        for active_pack in active {
            if active_pack.files.contains_key(path) || active_pack.assets.contains_key(path) {
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

    // Content decoding follows the supported-format gate. Do not spend work
    // interpreting, or report misleading asset errors for, a future format.
    findings.append(&mut deferred_content_findings);
    let resolve_asset = |path: &str| {
        assets
            .get(path)
            .cloned()
            .or_else(|| {
                active
                    .iter()
                    .rev()
                    .find_map(|pack| pack.assets.get(path).cloned())
            })
            .or_else(|| resolve_base_asset(path))
    };
    let descriptor_sources = crate::world::pack_asset_validation::descriptor_sources(
        assets
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_ref())),
    );
    for (path, bytes) in &assets {
        if let Err(error) = crate::world::pack_asset_validation::validate_member(
            path,
            bytes,
            &resolve_asset,
            &descriptor_sources,
        ) {
            findings.push(member_error("invalid-runtime-asset", path, error));
        }
    }

    // External buffers can replace bytes under an unchanged base/active GLB.
    // Validate those consumers too, before any new stack revision is visible.
    let changed_buffers: std::collections::BTreeSet<_> = assets
        .keys()
        .filter(|path| path.ends_with(".bin"))
        .collect();
    if !changed_buffers.is_empty() {
        let descriptors: std::collections::BTreeSet<_> = base_asset_paths
            .iter()
            .chain(active.iter().flat_map(|pack| pack.assets.keys()))
            .filter(|path| path.ends_with(".glb") && !assets.contains_key(*path))
            .collect();
        for path in descriptors {
            let Some(bytes) = resolve_asset(path) else {
                findings.push(member_error(
                    "unavailable-asset-consumer",
                    path,
                    "Cannot validate a buffer replacement without its immutable model dependency"
                        .into(),
                ));
                continue;
            };
            let required = match crate::world::pack_asset_validation::required_assets(path, &bytes)
            {
                Ok(required) => required,
                Err(error) => {
                    findings.push(member_error("invalid-runtime-asset", path, error));
                    continue;
                }
            };
            if required.iter().any(|path| changed_buffers.contains(path)) {
                if let Err(error) =
                    crate::world::pack_asset_validation::validate(path, &bytes, &resolve_asset)
                {
                    findings.push(member_error("invalid-runtime-asset", path, error));
                }
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
        if path == MANIFEST_PATH || path == STRING_CATALOGUE_PATH || path.ends_with(".rhai") {
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

    // String Tables are ordinary mod members but not TOML. Surface their own
    // schema/malformed errors and non-blocking fallback/placeholder findings
    // through the same upload report as every other authored member.
    if let Some(source) = files.get(STRING_CATALOGUE_PATH) {
        findings.extend(validate_string_catalogue(source));
    }

    // 5a. Compile every script the pack carries under the SAME deny-by-default
    //     vellum sandbox M1's loader uses (issue #988): a `.rhai` that fails to
    //     compile, or reaches for a denied capability, rejects the whole pack
    //     atomically. Reconciles #856's "packs contain no executable code" — the
    //     sandbox profile, not the extension, is the trust boundary.
    findings.extend(validate_pack_scripts(&files));
    if let Some(source) = files.get(crate::sound_cues::PATH) {
        if let Err(error) =
            crate::world::pack_asset_validation::validate_sound_catalog(source, &resolve_asset)
        {
            findings.push(member_error(
                "invalid-sound-cues",
                crate::sound_cues::PATH,
                error,
            ));
        }
    }

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
    let translation_only = manifest.scenarios.is_empty()
        && files.len() == 2
        && files.contains_key(STRING_CATALOGUE_PATH);
    if !translation_only {
        findings.extend(validate_manifest(&manifest, &manifest_toml, resolve));
    }

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
        assets,
        source_archive: Some(std::sync::Arc::from(zip_bytes)),
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
pub(crate) fn validate_pack_scripts(files: &BTreeMap<String, String>) -> Vec<WorldFinding> {
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
