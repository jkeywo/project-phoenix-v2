//! Pack archive decoding, integrity and admitted content paths.
use std::collections::BTreeMap;

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
pub use vellum_digest::crc32_ieee as crc32;

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
