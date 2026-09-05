//! What the viewscreen's mod-pack shelf is shown, and what choosing one does
//! (issue #1366, PRD #1355).
//!
//! # This is an ADAPTER, and deliberately holds no rule of its own
//!
//! Three things already exist, and none of them is re-decided here:
//!
//! * **what is on the shelf** is [`crate::native_host::mod_packs`], which takes
//!   a directory listing and never touches a filesystem;
//! * **whether a pack is any good** is
//!   [`crate::world::mod_pack::validate_mod_pack`], the same atomic validation a
//!   browser upload goes through — same archive reader, same manifest gate, same
//!   composition checks, same all-or-nothing acceptance;
//! * **what an accepted pack does to this session** is
//!   [`crate::entities::config_cache::push_mod_pack`], the same ordered overlay
//!   stack, with the same precedence and the same
//!   [`overlay_conflicts`](crate::entities::config_cache::overlay_conflicts)
//!   report over it.
//!
//! What is left — and all that is here — is the shape the surface renders, the
//! seams the pure validator needs filled in on a native host, and the
//! translation of a finding into something a viewscreen can draw.
//!
//! # The native seams are not the browser's, and that is the point of them
//!
//! `bridge::wasm_add_mod_pack` fills `validate_mod_pack`'s two injected seams
//! from the browser's own preload caches: `cached_base_world_source` for base
//! world text and `WasmTemplateLoader` for parsed templates. Off the browser the
//! first of those is a `None` stub by construction (`config_cache`'s native
//! half), because a native host does not fetch content over HTTP — it reads it.
//! So [`install_pack`] takes a `resolve_base` closure and a
//! [`TemplateLoader`](crate::entities::loader::TemplateLoader) as arguments, the
//! caller passes a filesystem read rooted at `--content-dir` and
//! [`FsTemplateLoader`](crate::entities::loader::FsTemplateLoader), and this
//! module stays as free of a host default as the validator it calls. That is the
//! same seam discipline `world::mod_pack`'s own header sets out, one layer up.
//!
//! # Findings are carried as prose, and that is not a rule-11 exemption
//!
//! A `WorldFinding`'s message names an authored path, a missing reference or a
//! malformed manifest key: it is a sentence *about the operator's own file*,
//! generated from that file, and there is no string-table entry that could hold
//! it. The browser host shows exactly these messages in `#mod-pack-findings`.
//! What IS in the table is everything around them — the panel's heading, its
//! empty state, the severity labels, the conflict line — so the surface says
//! nothing in English that a translation could have carried.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::entities::config_cache::{active_packs, overlay_conflicts, push_mod_pack, ActivePack};
use crate::entities::loader::TemplateLoader;
use crate::native_host::mod_packs::{self, ShelfPack};
use crate::world::manifest::{parse_content_identity, parse_pack_manifest};
use crate::world::mod_pack::validate_mod_pack;
use crate::world::validate::Severity;

/// One pack the shelf offers, as the surface sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferedPack {
    /// The archive's file name, which is also the id a press sends back.
    pub file: String,
    /// What the row shows.
    pub label: String,
}

impl From<&ShelfPack> for OfferedPack {
    fn from(pack: &ShelfPack) -> Self {
        Self {
            file: pack.file.clone(),
            label: pack.label.clone(),
        }
    }
}

/// One pack already in this session's overlay stack, oldest → newest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPack {
    pub id: String,
    pub name: String,
    pub version: String,
}

impl From<&ActivePack> for InstalledPack {
    fn from(pack: &ActivePack) -> Self {
        Self {
            id: pack.id.clone(),
            name: pack.name.clone(),
            version: pack.version.clone(),
        }
    }
}

/// One thing wrong (or worth knowing) about the last pack an operator chose.
///
/// The same five fields `wasm_add_mod_pack` reflects into a JS object, so the
/// two hosts report a refusal in the same shape and a reader of one can read the
/// other.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFinding {
    /// `"error"` or `"warning"`. An error is what blocked the install.
    pub severity: String,
    /// The validator's machine category, e.g. `missing-manifest`.
    pub category: String,
    /// The sentence about the operator's own file. See the module note.
    pub message: String,
    /// The authored path the finding is about.
    pub file: String,
}

impl PackFinding {
    /// A finding this adapter raised itself, before the validator was reached.
    ///
    /// Two things can go wrong on a native host that cannot go wrong in a
    /// browser — the named archive is not on the shelf, and the archive is on
    /// the shelf but cannot be read — and both have to look to the operator
    /// exactly like the validator's own refusals, because from where they are
    /// standing they are the same event: "I chose that one and it did not go
    /// in."
    pub fn error(category: &str, file: &str, message: String) -> Self {
        Self {
            severity: "error".to_string(),
            category: category.to_string(),
            message,
            file: file.to_string(),
        }
    }

    fn from_world_finding(finding: &crate::world::validate::WorldFinding) -> Self {
        Self {
            severity: match finding.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }
            .to_string(),
            category: finding.category.to_string(),
            message: finding.message.clone(),
            file: finding.source.file.clone(),
        }
    }
}

/// One authored path more than one active pack carries (issue #987's report,
/// shown here rather than only logged).
///
/// Named on the surface because "which pack won" is invisible otherwise: two
/// packs that both replace `assets/entities/alliance_destroyer.toml` produce one
/// hull, and an operator who cannot see which one is flying has no way to work
/// out why their change did nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackConflict {
    pub path: String,
    /// The pack id that owns the path — the latest-loaded one carrying it.
    pub winner: String,
    /// The shadowed pack ids, in load order.
    pub losers: Vec<String>,
}

/// The mod-pack shelf's whole state, as one snapshot the surface renders.
///
/// Deserialised straight into `landingViewModel`'s `packs` input by
/// `host_lobby_link.js`. Like [`super::landing::LandingPanelPayload`] it carries
/// no opinion about which entry is open and no opinion about which row an
/// operator has highlighted — the first is `nextOpenEntry`'s over the page's own
/// memory, and the second is the page's memory too, for the same reason.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModPackPanelPayload {
    /// The directory being scanned, as the operator wrote it at the prompt.
    ///
    /// Shown, because a shelf with nothing on it is otherwise indistinguishable
    /// from a host that was never given a folder — and the answer to the first
    /// is "put a pack in there" while the answer to the second is "restart with
    /// `--mod-pack-dir`".
    pub dir: String,
    /// Why the scan produced nothing, when it failed rather than found nothing.
    ///
    /// A missing or unreadable folder is an operator mistake at the prompt, and
    /// an empty shelf that does not say so reads as "there are no packs".
    pub scan_error: Option<String>,
    /// Every archive on the shelf, in the order they are offered.
    pub offered: Vec<OfferedPack>,
    /// Every pack installed so far this session, oldest → newest.
    pub installed: Vec<InstalledPack>,
    /// The file of the pack the last install attempt was for, or `None` before
    /// the first attempt.
    pub attempted: Option<String>,
    /// Whether that attempt was accepted.
    pub accepted: bool,
    /// What the attempt had to say. Empty on a clean accept.
    pub findings: Vec<PackFinding>,
    /// Every authored path two active packs both carry, with the winner named.
    pub conflicts: Vec<PackConflict>,
}

impl ModPackPanelPayload {
    /// Encode for the bridge.
    ///
    /// Infallible in practice — strings, bools and lists of both — and an encode
    /// that somehow failed answers with an EMPTY shelf rather than a stale one:
    /// a row an operator can press that does not name a file this host is
    /// holding would install whatever the last payload happened to say.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"dir":"","offered":[]}"#.into())
    }
}

/// Every pack currently in the overlay stack, as the surface shows them.
pub fn installed_packs() -> Vec<InstalledPack> {
    active_packs().iter().map(InstalledPack::from).collect()
}

/// Every authored path two active packs both carry, as the surface shows them.
pub fn active_conflicts() -> Vec<PackConflict> {
    overlay_conflicts(&active_packs())
        .into_iter()
        .map(|c| PackConflict {
            path: c.path,
            winner: c.winner,
            losers: c.losers,
        })
        .collect()
}

/// What one install attempt did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallOutcome {
    /// Whether the pack went onto the overlay stack.
    pub accepted: bool,
    /// Everything the validator (or this adapter) had to say.
    pub findings: Vec<PackFinding>,
}

/// Validate one archive and, when it passes, push it onto the session overlay
/// stack.
///
/// The native twin of `bridge::wasm_add_mod_pack`, and deliberately the same
/// shape: validate atomically, report every finding, and install only when
/// nothing is an error. It is **not** a second implementation of any of that —
/// every judgement below is [`validate_mod_pack`]'s, and the only lines here are
/// the ones that fill its injected seams and turn its answer into something a
/// viewscreen can draw.
///
/// `base_manifest_toml` is the base scenario manifest this host serves; its
/// `[content]` identity is INJECTED rather than defaulted, exactly as the
/// browser injects it, so a host whose manifest declares no `[content]` block
/// yields an identity no real pack can match and an upload is refused rather
/// than silently accepted against unknown content.
pub fn install_pack(
    zip_bytes: &[u8],
    base_manifest_toml: &str,
    resolve_base: impl Fn(&str) -> Option<String>,
    template_loader: &dyn TemplateLoader,
) -> InstallOutcome {
    let base_content = parse_content_identity(base_manifest_toml).unwrap_or_default();
    let active = active_packs();
    let result = validate_mod_pack(
        zip_bytes,
        &base_content,
        resolve_base,
        template_loader,
        &active,
    );
    let findings = result
        .findings
        .iter()
        .map(PackFinding::from_world_finding)
        .collect();
    if !result.is_accepted() {
        return InstallOutcome {
            accepted: false,
            findings,
        };
    }
    // Atomic, and the stack is NOT cleared first: installing B after A keeps A
    // (issue #987), with B merely shadowing it for the paths they share. That
    // shadowing is what `active_conflicts` above reports.
    let (id, name, version) = parse_pack_manifest(&result.manifest_toml)
        .ok()
        .and_then(|pm| pm.pack)
        .map(|p| (p.id, p.name, p.version))
        .unwrap_or_default();
    push_mod_pack(ActivePack {
        id,
        name,
        version,
        files: result.files.into_iter().collect(),
        manifest_toml: result.manifest_toml,
    });
    InstallOutcome {
        accepted: true,
        findings,
    }
}

/// The shelf one host owns, and the one press it has not answered yet.
///
/// Installed only by a host given `--mod-pack-dir` (issue #1366), which is what
/// keeps the landing's Load-mod-pack row inert everywhere else: the row's stage
/// opens exactly when the surface can answer it, and the surface can answer it
/// exactly when this resource exists.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModPackShelfResource {
    /// The directory to scan, as the operator wrote it.
    pub dir: std::path::PathBuf,
    /// The content root base paths resolve against (`--content-dir`).
    pub content_dir: String,
    /// The base scenario manifest, relative to [`Self::content_dir`].
    pub manifest_rel: String,
    /// What the last scan found.
    pub shelf: Vec<ShelfPack>,
    /// Why the last scan found nothing, when it failed rather than found none.
    pub scan_error: Option<String>,
    /// The file of the last pack an operator chose.
    pub attempted: Option<String>,
    /// Whether that choice was accepted.
    pub accepted: bool,
    /// What that choice had to say.
    pub findings: Vec<PackFinding>,
    /// A press the drain has taken off the surface's queue and this frame's
    /// installer has not answered yet.
    ///
    /// A latch rather than a direct call, for the reason
    /// `PendingForceStart` is one: `drain_surface_records` is a DISPATCHER over
    /// one queue, and an arm of it that read a directory, opened an archive and
    /// rebuilt the scenario catalogue would make every other record wait behind
    /// a disk read.
    pub pending: Option<String>,
}

impl ModPackShelfResource {
    /// A shelf at `dir`, scanned once.
    pub fn new(
        dir: impl Into<std::path::PathBuf>,
        content_dir: impl Into<String>,
        manifest_rel: impl Into<String>,
    ) -> Self {
        let mut shelf = Self {
            dir: dir.into(),
            content_dir: content_dir.into(),
            manifest_rel: manifest_rel.into(),
            ..Default::default()
        };
        shelf.rescan();
        shelf
    }

    /// Read the directory again and rebuild the shelf from what is there.
    ///
    /// Called at startup and after every attempt, so the list an operator is
    /// looking at is the folder as it is now rather than as it was when the host
    /// booted — a bridge machine's mod folder is exactly the sort of thing
    /// somebody drops a file into while the host is already up.
    pub fn rescan(&mut self) {
        match mod_packs::read_shelf_listing(&self.dir) {
            Ok(listing) => {
                self.shelf = mod_packs::shelf_from_listing(&listing);
                self.scan_error = None;
            }
            Err(e) => {
                self.shelf.clear();
                self.scan_error = Some(e);
            }
        }
    }

    /// This shelf as the surface renders it.
    pub fn payload(&self) -> ModPackPanelPayload {
        ModPackPanelPayload {
            dir: self.dir.display().to_string(),
            scan_error: self.scan_error.clone(),
            offered: self.shelf.iter().map(OfferedPack::from).collect(),
            installed: installed_packs(),
            attempted: self.attempted.clone(),
            accepted: self.accepted,
            findings: self.findings.clone(),
            conflicts: active_conflicts(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_carries_the_shelf_the_landing_draws() {
        let mut shelf = ModPackShelfResource {
            dir: "mods".into(),
            content_dir: ".".into(),
            manifest_rel: "assets/scenarios.toml".into(),
            ..Default::default()
        };
        shelf.shelf =
            mod_packs::shelf_from_listing(&[mod_packs::ShelfListingEntry::file("thin-margin.zip")]);
        let json = shelf.payload().to_json();
        assert!(json.contains(r#""file":"thin-margin.zip""#));
        assert!(json.contains(r#""label":"thin-margin""#));
        assert!(json.contains(r#""dir":"mods""#));
        // …and nothing about which row is highlighted or which entry is open.
        // Both are the page's own memory, for the reason `landing` states.
        assert!(!json.contains("chosen"));
        assert!(!json.contains("open_entry"));
    }

    #[test]
    fn a_failed_scan_is_reported_rather_than_shown_as_an_empty_folder() {
        // "There are no packs in this folder" and "there is no such folder" ask
        // the operator to do two different things, so they must not be the same
        // screen. The scan is against a path this test knows cannot exist, which
        // is the one thing about it that needs no filesystem to arrange.
        let shelf = ModPackShelfResource::new(
            "no-such-directory-for-issue-1366",
            ".",
            "assets/scenarios.toml",
        );
        assert!(shelf.shelf.is_empty());
        let error = shelf
            .scan_error
            .clone()
            .expect("an unreadable folder says so");
        assert!(
            error.contains("no-such-directory-for-issue-1366"),
            "the message must name the folder the operator wrote: {error}"
        );
        assert!(shelf.payload().scan_error.is_some());
    }

    #[test]
    fn an_adapter_raised_refusal_looks_exactly_like_a_validators() {
        // From where the operator is standing, "that archive is not on the
        // shelf any more" and "that archive is missing its manifest" are the
        // same event: they chose one and it did not go in. So they are one
        // shape, and the panel needs no second way to draw one.
        let finding = PackFinding::error("unknown-pack", "gone.zip", "it went away".into());
        assert_eq!(finding.severity, "error");
        assert_eq!(finding.category, "unknown-pack");
        assert_eq!(finding.file, "gone.zip");
    }

    #[test]
    fn a_pack_that_is_not_an_archive_at_all_is_refused_and_installs_nothing() {
        // The whole atomic-acceptance claim, exercised through this adapter
        // rather than restated: the validation is `validate_mod_pack`'s and the
        // only thing checked here is that a refusal reaches the surface as
        // findings and leaves the overlay stack alone.
        let _overlay = crate::entities::config_cache::overlay_test_guard();
        let before = active_packs().len();
        let outcome = install_pack(
            b"this is not a zip file",
            "[content]\nid = \"phoenix-base\"\nepoch = 1\n",
            |_| None,
            &crate::entities::loader::WasmTemplateLoader,
        );
        assert!(!outcome.accepted);
        assert!(
            outcome.findings.iter().any(|f| f.severity == "error"),
            "a refusal has to say what is wrong: {:?}",
            outcome.findings
        );
        assert_eq!(active_packs().len(), before, "nothing partial is installed");
    }

    #[test]
    fn conflicts_name_the_winner_so_it_is_clear_which_pack_is_flying() {
        // Read straight off `config_cache::overlay_conflicts`, which is issue
        // #987's own report — this only reshapes it for the wire. Driven through
        // the real stack so the two cannot drift — and off the browser that stack
        // is process-global, so the guard is what keeps this test's two packs its
        // own.
        let _overlay = crate::entities::config_cache::overlay_test_guard();
        let path = "assets/entities/__i1366_conflict.toml";
        let pack = |id: &str, body: &str| ActivePack {
            id: id.to_string(),
            name: id.to_string(),
            version: "1".to_string(),
            files: [(path.to_string(), body.to_string())].into_iter().collect(),
            manifest_toml: String::new(),
        };
        push_mod_pack(pack("i1366-a", "A"));
        push_mod_pack(pack("i1366-b", "B"));
        let conflicts = active_conflicts();
        let found = conflicts
            .iter()
            .find(|c| c.path == path)
            .expect("both packs carry the path");
        assert_eq!(found.winner, "i1366-b", "the latest loaded wins");
        assert_eq!(found.losers, vec!["i1366-a".to_string()]);
        // And the installed list is what the panel shows beside it.
        let ids: Vec<String> = installed_packs().into_iter().map(|p| p.id).collect();
        assert!(ids.contains(&"i1366-a".to_string()));
        assert!(ids.contains(&"i1366-b".to_string()));
    }
}
