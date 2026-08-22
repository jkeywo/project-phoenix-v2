use super::*;
use crate::world::load::MemoryTemplateLoader;

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

/// A host that serves no parsed entity templates at all, and is NOT
/// authoritative about that absence (issue #973). A loader that serves
/// nothing knows nothing; claiming authority here would have it reject
/// every hull a pack carries as `unresolvable-template`, which is
/// precisely the blindness that check is gated to avoid. See the note on
/// [`validate_mod_pack`].
///
/// The reference checks that consult a `TemplateLoader` (doctrine anchors)
/// read it as "unknown template", which is what these packs' worlds mean:
/// they declare no entities. Injecting it — rather than letting the module
/// reach for the host loader — is what keeps these tests independent of the
/// filesystem and the wasm thread-locals.
fn no_templates() -> MemoryTemplateLoader {
    MemoryTemplateLoader::blind()
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

// ── .rhai whitelist, asserted against the SHIPPED script layout (issue #988)

/// The whitelist admits a `.rhai` at the exact path `world::script::load`
/// resolves a `script = "..."` declaration to — enumerated from the loader,
/// not written as a literal, so there is one place that defines where a
/// script lives (AC1).
#[test]
fn whitelist_accepts_the_loaders_resolved_sibling_script_path() {
    use crate::world::script::load::{lift_world_scripts, ScriptResolver};

    // A resolver that serves any sibling, so lift reports the REAL resolved
    // path the loader would read.
    struct AnyScript;
    impl ScriptResolver for AnyScript {
        fn read(&self, _path: &str) -> Option<String> {
            Some("fn on_x(ctx) { }".to_string())
        }
    }

    let world: toml::Value = toml::from_str(r#"script = "combat.rhai""#).unwrap();
    let (sources, findings) =
        lift_world_scripts("assets/worlds/combat_test.toml", &world, &AnyScript);
    assert!(findings.is_empty(), "{:?}", findings);
    assert_eq!(sources.len(), 1);
    assert!(
        sources[0].path.ends_with(".rhai"),
        "the loader resolves a sibling .rhai: {:?}",
        sources[0].path,
    );
    assert!(
        is_allowed_content_path(&sources[0].path),
        "the shipped sibling-script path {:?} must be an allowed pack path",
        sources[0].path,
    );
}

#[test]
fn whitelist_accepts_rhai_only_directly_under_worlds() {
    assert!(is_allowed_content_path("assets/worlds/combat.rhai"));
    // Wrong directory, nested, or empty stem — all rejected.
    assert!(!is_allowed_content_path("assets/entities/x.rhai"));
    assert!(!is_allowed_content_path("assets/worlds/sub/x.rhai"));
    assert!(!is_allowed_content_path("assets/worlds/.rhai"));
    assert!(!is_allowed_content_path("combat.rhai"));
    assert!(!is_allowed_content_path("assets/worlds/../x.rhai"));
}

// ── Pack scripts compile under the sandbox (issue #988) ──────────────────

/// A world declaring a sibling script, for the compile tests below. The
/// `script` key is TOP-LEVEL (a sibling of `[global]`), matching what
/// `world::script::load::lift_world_scripts` reads (`world_toml.get("script")`).
fn world_with_script(title: &str, rel: &str) -> String {
    format!("script = \"{rel}\"\n\n[global]\ntitle = \"{title}\"\n")
}

#[test]
fn a_pack_carrying_a_valid_script_is_accepted() {
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        (
            "assets/worlds/s.toml",
            &world_with_script("world.s.title", "s.rhai"),
        ),
        (
            "assets/worlds/s.rhai",
            "fn on_alarm(ctx) { let n = 2 + 2; n }\n",
        ),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(result.is_accepted(), "findings: {:?}", result.findings);
    // The script rides the overlay alongside the world.
    assert!(result.files.contains_key("assets/worlds/s.rhai"));
}

#[test]
fn an_unparseable_script_rejects_the_whole_pack() {
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ("assets/worlds/broken.rhai", "fn oops(ctx) { let x = ; }\n"),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "unparseable-script"),
        "findings: {:?}",
        result.findings
    );
}

#[test]
fn a_script_reaching_the_wall_clock_is_a_denied_capability() {
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ("assets/worlds/clock.rhai", "let now = timestamp();\n"),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "denied-script-capability"),
        "findings: {:?}",
        result.findings
    );
}

#[test]
fn a_script_using_eval_is_a_denied_capability() {
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ("assets/worlds/evil.rhai", "let x = eval(\"1 + 1\");\n"),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "denied-script-capability"),
        "findings: {:?}",
        result.findings
    );
}

#[test]
fn a_script_importing_a_module_is_a_denied_capability() {
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        ("assets/worlds/s.toml", &simple_world("world.s.title")),
        ("assets/worlds/reach.rhai", "import \"secret\" as s;\n"),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "denied-script-capability"),
        "findings: {:?}",
        result.findings
    );
}

#[test]
fn an_inline_script_block_in_a_pack_world_is_compiled_by_the_same_gate() {
    // A denied capability inside an inline [script.*] block rejects the pack
    // exactly as a sibling file does (AC3).
    let world = "[global]\ntitle = \"world.s.title\"\n\n\
                 [script]\nsetup = \"let t = timestamp();\"\n";
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        ("assets/worlds/s.toml", world),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "denied-script-capability"),
        "findings: {:?}",
        result.findings
    );
}

#[test]
fn a_valid_inline_script_block_is_accepted() {
    let world = "[global]\ntitle = \"world.s.title\"\n\n\
                 [script]\nsetup = \"fn helper(ctx) { 1 + 1 }\"\n";
    let zip = create_store_zip(&[
        ("scenarios.toml", &manifest_for("s", "assets/worlds/s.toml")),
        ("assets/worlds/s.toml", world),
    ]);
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(result.is_accepted(), "findings: {:?}", result.findings);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(result.files.is_empty());
    assert_eq!(result.findings[0].category, "invalid-archive");
}

#[test]
fn non_archive_bytes_reject_whole_pack() {
    // Bytes with no local-file-header signature parse as an empty archive,
    // so the required manifest is absent — still an atomic rejection with
    // nothing applied.
    let result = validate_mod_pack(
        b"not a zip at all",
        &base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    assert!(result
        .findings
        .iter()
        .any(|f| f.category == "unparseable-content"));
}

/// A pack shipping a world that still authors the retired declarative
/// front-end is refused WHOLE, not partially loaded.
///
/// This used to author a `complete_objective` naming an undeclared id and
/// assert the `unresolved-objective-reference` finding blocked the pack.
/// Issue #985 deleted the `[[trigger]]` parser, and `parse_world` now
/// refuses such a world by name — which is the stronger claim, and the one a
/// pack author is actually going to hit while converting.
#[test]
fn a_pack_authoring_the_retired_declarative_front_end_is_rejected_whole() {
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
    assert!(!result.is_accepted());
    let finding = result
        .findings
        .iter()
        .find(|f| f.category == "unparseable-scenario-world")
        .unwrap_or_else(|| panic!("expected a parse refusal: {:?}", result.findings));
    assert!(
        finding.message.contains("[[trigger]]"),
        "the refusal must name the retired block: {}",
        finding.message
    );
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
    let result = validate_mod_pack(&zip, &base_identity(), base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), base, &no_templates(), &[]);
    assert!(result.is_accepted(), "findings: {:?}", result.findings);
}

// ── Authoritative loaders (issue #973 review, F3) ────────────────────────

/// A loader that serves nothing AND claims authority over absence.
///
/// Exactly what a future NATIVE caller gets from
/// [`crate::entities::loader::WasmTemplateLoader`] with an empty cache and no
/// pack file on disk — the same type the documented wasm caller passes, so
/// it is the obvious thing to write and answers `true` on native. Every
/// other fixture in this module answers `false`, which is why the dangerous
/// arm went unexercised while the constraint lived in a doc comment.
fn authoritative_no_templates() -> MemoryTemplateLoader {
    MemoryTemplateLoader::authoritative_empty()
}

/// A pack's own hulls are served in front of the caller's loader, so an
/// authoritative loader that can see none of them still accepts the pack.
///
/// Before F3 this rejected every valid pack with one bogus
/// `unresolvable-template` per hull.
#[test]
fn a_pack_carrying_its_own_hull_is_accepted_by_an_authoritative_loader() {
    assert!(
        authoritative_no_templates().absence_is_final(),
        "precondition"
    );
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
    let result = validate_mod_pack(
        &zip,
        &base_identity(),
        no_base,
        &authoritative_no_templates(),
        &[],
    );
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
    let result = validate_mod_pack(
        &zip,
        &base_identity(),
        no_base,
        &authoritative_no_templates(),
        &[],
    );
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &[]);
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
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(
        result.is_accepted(),
        "valid-v1.zip must be accepted: {:?}",
        result.findings
    );
}

#[test]
fn fixture_format_too_new_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/format-too-new.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert!(has_category(&result, "unsupported-pack-format"));
}

#[test]
fn fixture_content_epoch_mismatch_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/content-epoch-mismatch.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert!(has_category(&result, "pack-content-mismatch"));
}

/// A committed pack carrying a `.rhai` that uses only allowed capabilities is
/// accepted, script and all (issue #988). The SAME bytes round-trip through
/// `readStoreZip` on the JS side.
#[test]
fn fixture_script_valid_is_accepted() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/script-valid.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(
        result.is_accepted(),
        "script-valid.zip must be accepted: {:?}",
        result.findings
    );
}

/// A committed pack whose `.rhai` reaches for a denied capability is rejected
/// atomically with a `denied-script-capability` finding (issue #988).
#[test]
fn fixture_script_denied_capability_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/script-denied-capability.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert!(has_category(&result, "denied-script-capability"));
}

// ── #991 corpus completion, validated through the REAL archive bytes ──────
//
// The five fixtures the earlier slices left unbuilt. Each is asserted here
// AND in the browser smoke spec (tests/smoke/mod-pack.spec.js) over the SAME
// committed .zip, so the Rust validator and the real host page cannot drift.

/// A committed pack carrying a file OUTSIDE the authored whitelist is
/// rejected `disallowed-path`, with nothing applied (atomic).
#[test]
fn fixture_disallowed_path_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/disallowed-path.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "disallowed-path"),
        "findings: {:?}",
        result.findings
    );
    // Atomicity is the caller's `is_accepted` gate (bridge::wasm_add_mod_pack),
    // not a cleared `files` — the whitelist violation blocks acceptance, which
    // is what stops anything being applied.
}

/// The deliberately CRC-corrupted committed archive fails to read, so the
/// whole pack is rejected `invalid-archive` (its intact manifest is never
/// reached). Byte-for-byte the archive the browser spec uploads.
#[test]
fn fixture_corrupt_crc_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/corrupt-crc.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert_eq!(
        result.findings[0].category, "invalid-archive",
        "findings: {:?}",
        result.findings
    );
    assert!(result.files.is_empty());
}

/// A committed pack whose world is valid TOML but violates the world schema
/// (an `[[entity]]` with no `template_path`) is rejected
/// `unparseable-scenario-world`.
#[test]
fn fixture_schema_invalid_world_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/schema-invalid-world.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "unparseable-scenario-world"),
        "findings: {:?}",
        result.findings
    );
}

/// A committed pack whose manifest names a world neither carried nor in base
/// content is rejected `missing-scenario-world`.
#[test]
fn fixture_unresolved_manifest_world_is_rejected() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/unresolved-manifest-world.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(!result.is_accepted());
    assert!(
        has_category(&result, "missing-scenario-world"),
        "findings: {:?}",
        result.findings
    );
}

/// The editor exporter's OWN output validates on the host — the round trip
/// #991 closes end to end (editor #759/#989 → host validator #760/#986). The
/// archive is produced by `exportModPack` in the fixture generator, not
/// hand-authored, so this proves the real editor bytes are accepted.
#[test]
fn fixture_editor_round_trip_is_accepted() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/editor-round-trip.zip");
    let result = validate_mod_pack(
        bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(
        result.is_accepted(),
        "editor-round-trip.zip must be accepted: {:?}",
        result.findings
    );
    assert!(result.files.contains_key("assets/worlds/editor_arena.toml"));
}

// ── Multi-pack stack: precedence, conflicts, provenance (issue #987) ─────

/// Build a minimal already-active pack from a set of `(path, text)` files.
fn active_pack(id: &str, files: &[(&str, &str)]) -> ActivePack {
    let mut map = std::collections::HashMap::new();
    for (p, t) in files {
        map.insert((*p).to_string(), (*t).to_string());
    }
    ActivePack {
        id: id.to_string(),
        name: format!("Pack {id}"),
        version: "1.0.0".to_string(),
        files: map,
        manifest_toml: String::new(),
    }
}

/// A candidate whose `[pack] id` is already active is rejected — the overlay
/// stack keys packs by id, so a collision could never be addressed.
#[test]
fn a_candidate_whose_pack_id_is_already_active_is_rejected() {
    // `manifest_for` always sets `[pack] id = "test-pack"`.
    let zip = create_store_zip(&[
        (
            "scenarios.toml",
            &manifest_for("modx", "assets/worlds/modx.toml"),
        ),
        ("assets/worlds/modx.toml", &simple_world("world.modx.title")),
    ]);
    let active = [active_pack("test-pack", &[])];
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &active);
    assert!(!result.is_accepted());
    assert!(has_category(&result, "duplicate-pack-id"));
}

/// An authored path the candidate shares with an EARLIER active pack is a
/// non-blocking WARNING (winner = candidate, loser = active pack) — the pack
/// is still accepted.
#[test]
fn an_overlapping_path_is_a_non_blocking_warning_naming_winner_and_loser() {
    let zip = create_store_zip(&[
        (
            "scenarios.toml",
            &manifest_for("modx", "assets/worlds/modx.toml"),
        ),
        ("assets/worlds/modx.toml", &simple_world("world.modx.title")),
    ]);
    // A different-id active pack that already carries the same world path.
    let active = [active_pack(
        "earlier",
        &[(
            "assets/worlds/modx.toml",
            "[global]\ntitle = \"world.other.title\"\n",
        )],
    )];
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &active);
    assert!(
        result.is_accepted(),
        "an overlap is a warning, not a rejection: {:?}",
        result.findings
    );
    assert!(
        result
            .findings
            .iter()
            .any(|f| f.category == "overlapping-pack-path" && f.severity == Severity::Warning),
        "expected an overlapping-pack-path warning: {:?}",
        result.findings
    );
}

/// A pack whose hull includes a fragment carried by an EARLIER ACTIVE PACK
/// validates — composition resolves candidate → active stack → base (issue
/// #987). Mirrors `pack_hull_may_include_a_base_fragment`, but the fragment
/// lives in the active stack rather than in base content.
#[test]
fn pack_hull_may_include_a_fragment_from_an_earlier_active_pack() {
    let zip = create_store_zip(&[
        (
            "scenarios.toml",
            &manifest_for("modh", "assets/worlds/modh.toml"),
        ),
        (
            "assets/worlds/modh.toml",
            &world_spawning("assets/entities/pack_hull.toml", "pack_one"),
        ),
        (
            "assets/entities/pack_hull.toml",
            "includes = [\"layer_core.toml\"]\nname = \"Pack Hull\"\n",
        ),
    ]);
    // The earlier active pack supplies the fragment the candidate's hull
    // includes; the candidate does NOT carry it and base knows nothing.
    let active = [active_pack(
        "base-layer",
        &[("assets/entities/layer_core.toml", "class = \"escort\"\n")],
    )];
    let result = validate_mod_pack(&zip, &base_identity(), no_base, &no_templates(), &active);
    assert!(
        result.is_accepted(),
        "a pack hull must compose against a fragment an earlier active pack \
         carries: {:?}",
        result.findings
    );
}

/// The committed overlapping-pair fixtures, validated through the REAL
/// archive bytes: both carry `assets/worlds/shared_arena.toml`, so loading
/// the second while the first is active warns (winner + loser) yet accepts.
#[test]
fn fixture_overlapping_pair_warns_but_accepts() {
    let a_bytes = include_bytes!("../../tests/fixtures/mod-packs/overlap-a.zip");
    let b_bytes = include_bytes!("../../tests/fixtures/mod-packs/overlap-b.zip");

    // A validates on an empty stack.
    let ra = validate_mod_pack(
        a_bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[],
    );
    assert!(
        ra.is_accepted(),
        "overlap-a.zip must accept: {:?}",
        ra.findings
    );
    let active_a = active_pack(
        "overlap-a",
        &ra.files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect::<Vec<_>>(),
    );

    // B validates with A active → accepted, with an overlapping-path warning.
    let rb = validate_mod_pack(
        b_bytes,
        &fixture_base_identity(),
        no_base,
        &no_templates(),
        &[active_a],
    );
    assert!(
        rb.is_accepted(),
        "overlap-b.zip must accept over an active overlap-a (overlap warns, \
         does not block): {:?}",
        rb.findings
    );
    assert!(
        rb.findings
            .iter()
            .any(|f| f.category == "overlapping-pack-path"),
        "expected an overlapping-pack-path warning: {:?}",
        rb.findings
    );
}
