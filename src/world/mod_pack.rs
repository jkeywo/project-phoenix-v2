// Host mod-pack upload validation (issue #760).
//
// Pure Rust module — no Bevy, no wasm_bindgen. Consumes the store-only ZIP the
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
//   * `world::validate::{validate_composition, has_error}` for every
//     manifest-listed world's authored references.
//
// Acceptance is gated on `has_error` (definite errors block; warnings are
// non-blocking, consistent with #757/#759). The Bevy/wasm adapter that turns a
// browser upload into a call here — and populates the session overlay on
// success — lives in `server::bridge` + `entities::config_cache`, keeping this
// module a pure, natively-testable core.

use std::collections::BTreeMap;

use crate::world::config::parse_world;
use crate::world::manifest::{parse_manifest, validate_manifest};
use crate::world::validate::{
    has_error, validate_composition, Severity, SourceLocation, WorldFinding, WorldSource,
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

/// Validate an uploaded mod-pack ZIP atomically (issue #760, AC1).
///
/// `zip_bytes` is the raw uploaded archive. `resolve_base_world` resolves a
/// world path against BASE content (returning `None` when the host has not
/// fetched it), so a manifest root world may resolve either inside the pack or
/// against shipped content. Composition references are validated per
/// manifest-listed world against the pack + base worlds.
///
/// The returned [`ValidatedModPack`] carries error findings on any failure and
/// the overlay files + manifest on success; the caller applies the overlay only
/// when [`ValidatedModPack::is_accepted`] holds.
pub fn validate_mod_pack(
    zip_bytes: &[u8],
    resolve_base_world: impl Fn(&str) -> Option<String>,
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

    // 2. Path whitelist — any file outside the supported authored paths (or a
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

    // 3. Require the manifest.
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

    // 4. Parse every non-manifest TOML (worlds are re-parsed by the validators
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

    // 5. Parse + validate the manifest, resolving worlds against pack THEN base.
    let manifest = match parse_manifest(&manifest_toml) {
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

    let resolve = |path: &str| {
        files
            .get(path)
            .cloned()
            .or_else(|| resolve_base_world(path))
    };
    // Bind a shared reference so the same resolver serves both the manifest
    // validation here and the per-world composition checks below (`&F: Fn` is
    // Copy, so this passes by value without moving the closure).
    let resolve = &resolve;
    findings.extend(validate_manifest(&manifest, &manifest_toml, resolve));

    // 6. Composition references for every manifest-listed root world that
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
        findings.extend(validate_composition(&root_src, &child_srcs));
    }

    // 7. On success, hand back the supported authored files (excluding the
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

    fn manifest_for(id: &str, world: &str) -> String {
        format!("[[scenario]]\nid = \"{id}\"\nworld = \"{world}\"\n")
    }

    fn no_base(_: &str) -> Option<String> {
        None
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
        let result = validate_mod_pack(&zip, no_base);
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
        let result = validate_mod_pack(&zip, no_base);
        assert!(!result.is_accepted());
        assert!(result.files.is_empty());
        assert_eq!(result.findings[0].category, "invalid-archive");
    }

    #[test]
    fn non_archive_bytes_reject_whole_pack() {
        // Bytes with no local-file-header signature parse as an empty archive,
        // so the required manifest is absent — still an atomic rejection with
        // nothing applied.
        let result = validate_mod_pack(b"not a zip at all", no_base);
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
        let result = validate_mod_pack(&zip, no_base);
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
        let result = validate_mod_pack(&zip, no_base);
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
        let result = validate_mod_pack(&zip, no_base);
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
        let result = validate_mod_pack(&zip, no_base);
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
        let result = validate_mod_pack(&zip, no_base);
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
        let result = validate_mod_pack(&zip, base);
        assert!(result.is_accepted(), "findings: {:?}", result.findings);
    }
}
