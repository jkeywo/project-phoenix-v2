/// One `[[scenario]]` entry in the base scenario manifest.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct ScenarioEntry {
    /// Stable scenario id, unique within the manifest.
    pub id: String,
    /// Path to the selectable root world TOML (`assets/worlds/*.toml`).
    pub world: String,
    /// Optional display label override. When absent, the catalog falls back to
    /// the referenced world's `[global] title`.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional playable-hull curation (issue #917): `template_path` values
    /// this entry restricts the world's `[[available_ships]]` to. Empty (the
    /// default) means "every ship the world offers" — pre-#917 behaviour, and
    /// what every mod-pack manifest exported by #759 still produces. A
    /// non-empty list NEVER edits the referenced world TOML; it filters the
    /// catalog built from it, in the world's own authored order.
    #[serde(default)]
    pub ships: Vec<String>,
}

/// The parsed base scenario manifest: the ordered list of selectable roots.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Manifest {
    #[serde(default, rename = "scenario")]
    pub scenarios: Vec<ScenarioEntry>,
}

/// Parse the base scenario manifest TOML.
///
/// Returns an `Err` with a human-readable message on TOML syntax errors. Empty
/// or missing `[[scenario]]` tables parse into an empty manifest — semantic
/// problems (no entries, bad references, duplicates) are reported by
/// [`validate_manifest`] as source-located findings, not parse errors.
pub fn parse_manifest(toml_str: &str) -> Result<Manifest, String> {
    toml::from_str(toml_str).map_err(|e| e.to_string())
}

// ── Mod-pack identity + content compatibility (issue #986) ────────────────────

/// The highest `[pack] format` version this host understands. A pack declaring
/// a format above this is rejected outright, before any of its content is
/// validated (see `world::mod_pack::validate_mod_pack`), so a future format can
/// never bury its one real incompatibility under a wall of content findings.
///
/// This is a code constant on purpose — it is a versioning boundary, not a
/// gameplay value a designer would tune, so it is exempt from the "no hardcoded
/// gameplay values" rule (AGENTS.md).
pub const SUPPORTED_PACK_FORMAT: u32 = 1;

/// The host's declared content identity — the `[content]` block of the base
/// `assets/scenarios.toml`. It is the host side of the mod-pack compatibility
/// contract: a pack's `[pack.requires]` names the `id`/`epoch` it was authored
/// against, and the upload is rejected unless both match.
///
/// This type is *injected* into `validate_mod_pack` rather than read from a host
/// default there, the same discipline the `resolve_base` seam follows: the pure
/// validator keeps no host dependency of its own, and the wasm bridge reads this
/// from `config_cache::get_scenario_manifest_toml()` and hands it in.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ContentIdentity {
    /// Stable identifier for the shipped content set (e.g. `"phoenix-base"`).
    pub id: String,
    /// Monotonic content revision. A pack authored against an older epoch is
    /// rejected, so incompatible content can never be silently merged.
    pub epoch: i64,
}

/// Read the `[content]` identity block from a base scenario manifest, or `None`
/// when the manifest declares none.
///
/// Deliberately does NOT touch [`parse_manifest`] or the [`Manifest`] struct —
/// the base manifest still parses through the unchanged reader with no knowledge
/// of `[content]`; this is a separate, additive read used only by the mod-pack
/// upload seam.
pub fn parse_content_identity(manifest_toml: &str) -> Option<ContentIdentity> {
    #[derive(serde::Deserialize)]
    struct Doc {
        content: Option<ContentIdentity>,
    }
    toml::from_str::<Doc>(manifest_toml)
        .ok()
        .and_then(|d| d.content)
}

/// The `[pack.requires]` compatibility clause: the base content identity a pack
/// was authored against. Both fields are optional at the type level so a pack
/// that omits them parses (and is then rejected as a mismatch), rather than
/// failing to parse with a confusing TOML error.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct PackRequires {
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub content_epoch: Option<i64>,
}

/// The required top-level `[pack]` identity table a mod pack's `scenarios.toml`
/// carries (issue #986). `format` is mandatory — it is the version discriminator
/// the compatibility gate reads first; the display/identity fields default to
/// empty so an otherwise-parseable pack surfaces a semantic finding
/// (`invalid-pack-id`) rather than a parse error.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct PackMeta {
    /// Manifest format version. A pack whose `format` exceeds
    /// [`SUPPORTED_PACK_FORMAT`] is rejected before any content validation.
    pub format: u32,
    /// Stable pack id.
    #[serde(default)]
    pub id: String,
    /// Human-facing version string (e.g. `"1.0.0"`).
    #[serde(default)]
    pub version: String,
    /// Display name shown in the host status when the pack is accepted.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Base content this pack requires to be compatible.
    #[serde(default)]
    pub requires: PackRequires,
}

/// A parsed mod-pack manifest: its `[pack]` identity header (absent when the
/// required table is missing — reported as `missing-pack-header` downstream)
/// plus the `[[scenario]]` [`Manifest`] the base reader already understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    pub pack: Option<PackMeta>,
    pub manifest: Manifest,
}

/// Parse a mod-pack `scenarios.toml`: the `[pack]` identity header on top of the
/// shared `[[scenario]]` schema.
///
/// Wraps [`parse_manifest`] for the scenario list (so a pack manifest and the
/// base manifest validate on exactly the same surface) and additionally reads
/// the top-level `[pack]` table. The base `assets/scenarios.toml`, which has no
/// `[pack]` block, still parses here with `pack: None`.
pub fn parse_pack_manifest(toml_str: &str) -> Result<PackManifest, String> {
    #[derive(serde::Deserialize)]
    struct PackDoc {
        pack: Option<PackMeta>,
    }
    let manifest = parse_manifest(toml_str)?;
    let doc: PackDoc = toml::from_str(toml_str).map_err(|e| e.to_string())?;
    Ok(PackManifest {
        pack: doc.pack,
        manifest,
    })
}
