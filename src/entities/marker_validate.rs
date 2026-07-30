//! Model-marker contract validation (issue #758).
//!
//! Authored entity TOMLs name *rig markers* (`marker = "phasers_fore"`,
//! `markers = ["engine_port", …]`) that must exist in the model rig sidecar
//! selected by the entity's `[mesh] model` + `variant`
//! ([`crate::model_rig::sidecar_path`]). Until this module existed every
//! resolution path was a silent `Option` fallback: a misspelled marker simply
//! attached the beam/exhaust/camera to the ship centre and nobody found out.
//!
//! This module is the **contract**: given an [`EntityConfig`] and the
//! [`ModelRig`] it selects, it reports source-located findings instead of
//! falling back. It is pure — no Bevy systems, no filesystem — so callers
//! (headless startup gate, tests, the editor's Rust-side twin) supply the rig.
//!
//! # Finding classes
//!
//! * **missing** (`missing-marker`) — a referenced name is not declared by the
//!   selected rig. Error.
//! * **missing rig** (`unresolved-model-rig`) — the entity names markers but
//!   selects no model at all (or the sidecar could not be resolved), so no
//!   reference can ever resolve. Error.
//! * **duplicate** (`duplicate-marker`) — the sidecar declares the same
//!   `[markers.<name>]` header twice. The `toml` crate rejects a redefined
//!   table outright and `HashMap` would dedupe it anyway, so this is detected
//!   by scanning the raw sidecar text; that turns an opaque parse error into a
//!   located authoring finding. Error.
//! * **incompatible** (`incompatible-marker`) — the reference resolves, but to
//!   a marker reserved for a different role. The one reserved namespace is the
//!   `camera_` prefix ([`CAMERA_MARKER_PREFIX`]): the captain's camera-view
//!   list is built by filtering rig marker names by that prefix
//!   (`console::captain::server`), so `camera_*` markers are viewpoints and
//!   nothing else. A weapon or effect pointing at one is an authoring mistake,
//!   as is a camera view pointing at a marker outside the namespace. Error.
//! * **missing default camera** (`missing-camera-marker`) — a hull a player can
//!   fly ([`is_player_flyable`]: a `[captain_console]` on a design not tagged
//!   `npc` — see that function for why the section alone stopped meaning it)
//!   whose rig declares no
//!   [`crate::core::messages::CameraView`] default marker. The viewscreen
//!   would silently snap to the ship's origin. Warning, because the hull is
//!   still playable.
//!
//! # Where the check actually runs — and where it does not
//!
//! This module is pure; it only reports when something calls it. Today that is:
//!
//! * **Native / headless startup** — `headless::app::build_headless_app` gates
//!   the whole run on it, before `App::new()` and therefore before anything
//!   spawns (`phoenix-headless` only).
//! * **The editor** — the JS twin `editor/marker-validate.js`, wired into
//!   `SaveFlow`, refuses to write an entity or rig sidecar with an unresolved
//!   reference. This is the authoring-time surface an author actually meets.
//! * **CI** — `every_shipped_entity_marker_reference_resolves` walks every
//!   shipped template, so a bad reference cannot land in the repo.
//!
//! **Gap: the WASM host.** The shipped game runs in the browser, where rig
//! sidecars arrive *asynchronously* from JS
//! ([`crate::config_cache::wasm_push_sidecar_toml`]) some frames after the
//! template is parsed. There is no point in that boot at which every sidecar
//! is known, so nothing validates before `glb_visual` attaches. A hand-edited
//! entity TOML shipped past the editor and past CI would therefore still fall
//! back silently in the browser. Closing this needs a WASM-side gate that runs
//! once the sidecar fetches settle — tracked as an exception on the
//! `model-marker-contract` entity in
//! `pasm/spec/architecture/ship-entity-configuration.yaml`.
//!
//! # Deliberately not validated: `[[system]] marker`
//!
//! [`crate::ship::config::SystemInstanceConfig::marker`] is *declared but
//! unread* — no runtime path resolves it against a rig. It is excluded from
//! this validator on purpose: validating a field nothing consumes would invent
//! a contract rather than check one. When a consumer lands, add the refs to
//! [`collect_marker_refs`] and the check follows for free.

use std::collections::HashSet;

use crate::entity_config::EntityConfig;
use crate::model_rig::ModelRig;
use crate::world::validate::{line_of, Severity, SourceLocation};

/// Reserved marker-name prefix for camera viewpoints. The captain console
/// builds its camera-view list by filtering rig marker names by this prefix,
/// so the namespace belongs to cameras alone.
pub const CAMERA_MARKER_PREFIX: &str = "camera_";

// Finding category slugs (kebab-case, mirroring `world::validate`).
pub const CATEGORY_MISSING: &str = "missing-marker";
pub const CATEGORY_DUPLICATE: &str = "duplicate-marker";
pub const CATEGORY_INCOMPATIBLE: &str = "incompatible-marker";
pub const CATEGORY_NO_RIG: &str = "unresolved-model-rig";
pub const CATEGORY_MISSING_CAMERA: &str = "missing-camera-marker";

/// What a marker reference is *for*. Roles decide reserved-namespace
/// compatibility, not resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerRole {
    /// Weapon mount: phaser bank, blaster bank, torpedo tube.
    Weapon,
    /// Visual effect origin: engine exhaust PFX.
    Effect,
    /// Camera viewpoint (`CameraView.marker_name`).
    Camera,
}

impl MarkerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkerRole::Weapon => "weapon",
            MarkerRole::Effect => "effect",
            MarkerRole::Camera => "camera",
        }
    }

    /// Whether a marker *name* is compatible with this role. Only the
    /// `camera_` namespace is reserved: cameras must sit inside it, everything
    /// else must stay out.
    pub fn accepts(self, marker_name: &str) -> bool {
        let is_camera_marker = marker_name.starts_with(CAMERA_MARKER_PREFIX);
        match self {
            MarkerRole::Camera => is_camera_marker,
            MarkerRole::Weapon | MarkerRole::Effect => !is_camera_marker,
        }
    }
}

/// One authored marker reference found in an entity config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerRef {
    pub role: MarkerRole,
    /// Human-readable owner, e.g. `phaser bank 'fore'`.
    pub owner: String,
    /// The authored marker name.
    pub name: String,
}

impl MarkerRef {
    fn new(role: MarkerRole, owner: impl Into<String>, name: impl Into<String>) -> Self {
        MarkerRef {
            role,
            owner: owner.into(),
            name: name.into(),
        }
    }
}

/// A source-located marker finding. Same shape as
/// [`crate::world::validate::WorldFinding`] and reusing its
/// [`Severity`]/[`SourceLocation`] types, so callers can log and gate both
/// kinds identically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerFinding {
    pub severity: Severity,
    /// Short kebab-case category slug.
    pub category: &'static str,
    pub message: String,
    pub source: SourceLocation,
}

impl MarkerFinding {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    fn new(
        severity: Severity,
        category: &'static str,
        file: &str,
        source_text: &str,
        reference: &str,
        message: String,
    ) -> Self {
        MarkerFinding {
            severity,
            category,
            message,
            source: SourceLocation {
                file: file.to_string(),
                line: line_of(source_text, reference),
                reference: reference.to_string(),
            },
        }
    }

    /// Render as `[category] message (file:line)` for logs and errors.
    pub fn describe(&self) -> String {
        let loc = match self.source.line {
            Some(l) => format!("{}:{}", self.source.file, l),
            None => self.source.file.clone(),
        };
        format!("[{}] {} ({loc})", self.category, self.message)
    }
}

/// True when any finding is an error — the gate predicate.
pub fn has_error(findings: &[MarkerFinding]) -> bool {
    findings.iter().any(MarkerFinding::is_error)
}

/// Collect every marker reference an entity config authors, in a stable
/// order (phaser banks, blaster banks, torpedo tubes, engine PFX).
///
/// `[[system]] marker` is deliberately excluded — see the module docs.
pub fn collect_marker_refs(config: &EntityConfig) -> Vec<MarkerRef> {
    let mut refs = Vec::new();

    if let Some(weapons) = config.weapons_console.as_ref() {
        for bank in &weapons.phaser_banks {
            if let Some(name) = bank.marker.as_deref() {
                refs.push(MarkerRef::new(
                    MarkerRole::Weapon,
                    format!("phaser bank '{}'", bank.id),
                    name,
                ));
            }
        }
        for bank in &weapons.blaster_banks {
            if let Some(name) = bank.marker.as_deref() {
                refs.push(MarkerRef::new(
                    MarkerRole::Weapon,
                    format!("blaster bank '{}'", bank.id),
                    name,
                ));
            }
            // Each authored barrel marker (issue #765) is its own reference, so
            // a missing or incompatible barrel marker is rejected exactly like
            // the single `marker`.
            for (i, name) in bank.barrels.iter().enumerate() {
                refs.push(MarkerRef::new(
                    MarkerRole::Weapon,
                    format!("blaster bank '{}' barrel {i}", bank.id),
                    name,
                ));
            }
        }
    }

    if let Some(torpedoes) = config.torpedoes.as_ref() {
        for tube in &torpedoes.tubes {
            if let Some(name) = tube.marker.as_deref() {
                refs.push(MarkerRef::new(
                    MarkerRole::Weapon,
                    format!("torpedo tube '{}'", tube.id),
                    name,
                ));
            }
            // Each authored barrel marker (issue #766) is its own reference, so
            // a missing or incompatible barrel marker is rejected exactly like
            // the single `marker`.
            for (i, name) in tube.barrels.iter().enumerate() {
                refs.push(MarkerRef::new(
                    MarkerRole::Weapon,
                    format!("torpedo tube '{}' barrel {i}", tube.id),
                    name,
                ));
            }
        }
    }

    if let Some(helm) = config.helm_console.as_ref() {
        if let Some(pfx) = helm.engine_pfx.as_ref() {
            for name in &pfx.markers {
                refs.push(MarkerRef::new(
                    MarkerRole::Effect,
                    "engine exhaust PFX",
                    name.as_str(),
                ));
            }
        }
    }

    refs
}

/// Scan a raw rig sidecar for a `[markers.<name>]` table declared more than
/// once. Runs on the *text* because both the `toml` crate (hard parse error)
/// and the parsed `HashMap` (silent dedupe) destroy the evidence.
pub fn duplicate_marker_findings(sidecar_path: &str, sidecar_toml: &str) -> Vec<MarkerFinding> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut findings = Vec::new();
    for (idx, line) in sidecar_toml.lines().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("[markers.") else {
            continue;
        };
        let Some(name) = rest.strip_suffix(']') else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        if name.is_empty() {
            continue;
        }
        if !seen.insert(name.to_string()) {
            findings.push(MarkerFinding {
                severity: Severity::Error,
                category: CATEGORY_DUPLICATE,
                message: format!("model rig declares marker '{name}' more than once"),
                source: SourceLocation {
                    file: sidecar_path.to_string(),
                    line: Some(idx + 1),
                    reference: name.to_string(),
                },
            });
        }
    }
    findings
}

/// Validate one camera view name against a rig.
///
/// Used both by the entity pass (the default [`CameraView`] a player hull
/// falls back to) and by callers that resolve an explicit view.
///
/// [`CameraView`]: crate::core::messages::CameraView
pub fn validate_camera_view(
    file: &str,
    source_text: &str,
    rig: &ModelRig,
    marker_name: &str,
) -> Vec<MarkerFinding> {
    check_ref(
        file,
        source_text,
        Some(rig),
        &MarkerRef::new(MarkerRole::Camera, "camera view", marker_name),
    )
    .into_iter()
    .collect()
}

fn check_ref(
    file: &str,
    source_text: &str,
    rig: Option<&ModelRig>,
    r: &MarkerRef,
) -> Option<MarkerFinding> {
    let Some(rig) = rig else {
        return Some(MarkerFinding::new(
            Severity::Error,
            CATEGORY_NO_RIG,
            file,
            source_text,
            &r.name,
            format!(
                "{} references marker '{}' but no model rig could be resolved for this entity",
                r.owner, r.name
            ),
        ));
    };
    if rig.marker(&r.name).is_none() {
        let mut known: Vec<&str> = rig.markers.keys().map(|s| s.as_str()).collect();
        known.sort_unstable();
        return Some(MarkerFinding::new(
            Severity::Error,
            CATEGORY_MISSING,
            file,
            source_text,
            &r.name,
            format!(
                "{} references marker '{}' which the model rig does not declare (declared: [{}])",
                r.owner,
                r.name,
                known.join(", ")
            ),
        ));
    }
    if !r.role.accepts(&r.name) {
        return Some(MarkerFinding::new(
            Severity::Error,
            CATEGORY_INCOMPATIBLE,
            file,
            source_text,
            &r.name,
            format!(
                "{} ({} role) references marker '{}': the '{}' prefix is reserved for camera viewpoints",
                r.owner,
                r.role.as_str(),
                r.name,
                CAMERA_MARKER_PREFIX
            ),
        ));
    }
    None
}

/// The content tag that declares a hull to be an NPC-only design — one no
/// player ever occupies the bridge of.
pub const NPC_HULL_TAG: &str = "npc";

/// Whether a hull is one a player can fly, and therefore one whose rig owes the
/// viewscreen a default camera viewpoint.
///
/// `[captain_console]` used to answer this on its own, and did so only because
/// the player hulls were the only ones that declared the section. #885b ended
/// that: every AI-bearing hull now authors `[captain_console.ai]`, which cannot
/// be written without bringing `[captain_console]` into existence, so the
/// section is on all ten shipped hulls and distinguishes nothing.
///
/// The hull's own `tags` are what still say it: the NPC designs all declare
/// [`NPC_HULL_TAG`] and the player hulls declare none. The check therefore stays
/// exactly as strict where it matters — an Alliance hull whose rig loses
/// `camera_fore` still fails — and stops demanding a bridge viewpoint from a
/// design no bridge crew ever boards.
pub fn is_player_flyable(config: &EntityConfig) -> bool {
    config.captain_console.is_some() && !config.tags.iter().any(|t| t == NPC_HULL_TAG)
}

/// Validate every marker reference in one entity config against the rig its
/// `[mesh]` selects.
///
/// * `file` / `entity_toml` — the entity's path and raw TOML, for source
///   locations.
/// * `rig` — the parsed sidecar, or `None` when the entity selects no model or
///   the sidecar could not be resolved.
pub fn validate_entity_markers(
    file: &str,
    entity_toml: &str,
    config: &EntityConfig,
    rig: Option<&ModelRig>,
) -> Vec<MarkerFinding> {
    let mut findings = Vec::new();
    for r in collect_marker_refs(config) {
        if let Some(f) = check_ref(file, entity_toml, rig, &r) {
            findings.push(f);
        }
    }

    // A hull a player can fly needs the default camera viewpoint, or the
    // viewscreen silently snaps to the ship's origin.
    if is_player_flyable(config) {
        if let Some(rig) = rig {
            let default_view = crate::core::messages::CameraView::default().marker_name;
            if rig.marker(&default_view).is_none() {
                findings.push(MarkerFinding {
                    severity: Severity::Warning,
                    category: CATEGORY_MISSING_CAMERA,
                    message: format!(
                        "hull has a captain console but its model rig declares no '{default_view}' \
                         marker; the viewscreen falls back to the ship's origin"
                    ),
                    source: SourceLocation {
                        file: file.to_string(),
                        line: None,
                        reference: default_view,
                    },
                });
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIG: &str = r##"
[markers.phasers_fore]
position = [0.0, 0.0, -6.0]
direction = [0.0, 0.0, -1.0]

[markers.torpedo_port]
position = [-1.0, 0.0, -4.0]
direction = [0.0, 0.0, -1.0]

[markers.blaster_fore]
position = [0.0, 0.5, -5.0]
direction = [0.0, 0.0, -1.0]

[markers.engine_port]
position = [-1.0, 0.0, 5.0]
direction = [0.0, 0.0, 1.0]

[markers.camera_fore]
position = [0.0, 1.0, -3.0]
direction = [0.0, 0.0, -1.0]
"##;

    fn rig() -> ModelRig {
        ModelRig::from_toml(RIG).expect("fixture rig parses")
    }

    fn entity(body: &str) -> (String, EntityConfig) {
        let toml = format!(
            r##"
name = "Fixture"

[mesh]
model = "assets/models/fixture.glb"
shape = "cuboid"
colour = [1.0, 1.0, 1.0]
{body}
"##
        );
        let cfg = EntityConfig::from_toml(&toml).expect("fixture entity parses");
        (toml, cfg)
    }

    const PHASER_OK: &str = r##"
[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 45.0
marker = "phasers_fore"
"##;

    #[test]
    fn phaser_bank_marker_resolves() {
        let (toml, cfg) = entity(PHASER_OK);
        let findings = validate_entity_markers("fixture.toml", &toml, &cfg, Some(&rig()));
        assert!(findings.is_empty(), "expected clean, got {findings:?}");
    }

    #[test]
    fn phaser_bank_missing_marker_is_located_error() {
        let (toml, cfg) = entity(&PHASER_OK.replace("phasers_fore", "phasers_front"));
        let findings = validate_entity_markers("fixture.toml", &toml, &cfg, Some(&rig()));
        assert_eq!(findings.len(), 1, "{findings:?}");
        let f = &findings[0];
        assert!(f.is_error());
        assert_eq!(f.category, CATEGORY_MISSING);
        assert_eq!(f.source.file, "fixture.toml");
        assert_eq!(f.source.reference, "phasers_front");
        assert!(f.source.line.is_some(), "finding must be source-located");
        assert!(f.message.contains("phaser bank 'fore'"), "{}", f.message);
    }

    #[test]
    fn blaster_bank_marker_success_and_failure() {
        let body = r##"
[[weapons_console.blaster_banks]]
id = "fore"
facing_deg = 0.0
marker = "blaster_fore"
"##;
        let (toml, cfg) = entity(body);
        assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

        let (toml, cfg) = entity(&body.replace("blaster_fore", "blaster_nose"));
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_MISSING);
        assert!(
            findings[0].message.contains("blaster bank 'fore'"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn blaster_barrel_markers_validated_per_barrel() {
        // Two authored barrels + a pattern; one barrel marker is misspelled.
        let body = r##"
[[weapons_console.blaster_banks]]
id = "twin"
facing_deg = 0.0
barrels = [ "blaster_fore", "blaster_nose" ]
[[weapons_console.blaster_banks.pattern]]
barrels = [ 0 ]
offset_secs = 0.0
[[weapons_console.blaster_banks.pattern]]
barrels = [ 1 ]
offset_secs = 0.2
"##;
        let (toml, cfg) = entity(body);
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
        // `blaster_fore` resolves; `blaster_nose` does not.
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, CATEGORY_MISSING);
        assert!(
            findings[0].message.contains("blaster bank 'twin' barrel 1"),
            "{}",
            findings[0].message
        );

        // Both barrels valid → clean.
        let (toml, cfg) = entity(&body.replace("blaster_nose", "blaster_fore"));
        assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());
    }

    #[test]
    fn torpedo_tube_marker_success_and_failure() {
        let body = r##"
[[torpedoes.tubes]]
id = "port"
facing_deg = 0.0
fire_arc_deg = 90.0
marker = "torpedo_port"
"##;
        let (toml, cfg) = entity(body);
        assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

        let (toml, cfg) = entity(&body.replace("torpedo_port", "torpdo_port"));
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_MISSING);
        assert!(
            findings[0].message.contains("torpedo tube 'port'"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn torpedo_barrel_markers_validated_per_barrel() {
        // Two authored barrels + a pattern; one barrel marker is misspelled.
        let body = r##"
[[torpedoes.tubes]]
id = "twin"
facing_deg = 0.0
fire_arc_deg = 90.0
barrels = [ "torpedo_port", "torpedo_nose" ]
[[torpedoes.tubes.pattern]]
barrels = [ 0 ]
offset_secs = 0.0
[[torpedoes.tubes.pattern]]
barrels = [ 1 ]
offset_secs = 0.2
"##;
        let (toml, cfg) = entity(body);
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
        // `torpedo_port` resolves; `torpedo_nose` does not.
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, CATEGORY_MISSING);
        assert!(
            findings[0].message.contains("torpedo tube 'twin' barrel 1"),
            "{}",
            findings[0].message
        );

        // Both barrels valid → clean.
        let (toml, cfg) = entity(&body.replace("torpedo_nose", "torpedo_port"));
        assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());
    }

    #[test]
    fn engine_pfx_markers_success_and_failure() {
        let body = r##"
[helm_console.engine_pfx]
markers = [ "engine_port" ]
"##;
        let (toml, cfg) = entity(body);
        assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

        let (toml, cfg) = entity(&body.replace("engine_port", "engine_starbord"));
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_MISSING);
        assert!(
            findings[0].message.contains("engine exhaust PFX"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn camera_view_success_and_failure() {
        let rig = rig();
        assert!(validate_camera_view("f.toml", "", &rig, "camera_fore").is_empty());

        // Missing camera marker.
        let findings = validate_camera_view("f.toml", "", &rig, "camera_aft");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_MISSING);

        // Present, but outside the reserved camera namespace → incompatible.
        let findings = validate_camera_view("f.toml", "", &rig, "engine_port");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_INCOMPATIBLE);
        assert!(findings[0].is_error());
    }

    #[test]
    fn weapon_referencing_camera_marker_is_incompatible() {
        let (toml, cfg) = entity(&PHASER_OK.replace("phasers_fore", "camera_fore"));
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, CATEGORY_INCOMPATIBLE);
        assert_eq!(findings[0].source.reference, "camera_fore");
    }

    #[test]
    fn markers_without_a_rig_are_errors() {
        let (toml, cfg) = entity(PHASER_OK);
        let findings = validate_entity_markers("f.toml", &toml, &cfg, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_NO_RIG);
        assert!(has_error(&findings));
    }

    /// The default-camera warning fires for a hull a player can fly, and only
    /// for one.
    ///
    /// `[captain_console]` answered "can a player fly this?" on its own until
    /// #885b stage 5c, when every AI-bearing hull — NPC designs included —
    /// authored `[captain_console.ai]` and brought the section into existence.
    /// The `npc` tag carries the distinction now, and both halves are asserted
    /// here so re-pointing the gate cannot have quietly switched the check off:
    /// the same rig, the same console, tag present ⇒ silent, tag absent ⇒ warned.
    #[test]
    fn the_default_camera_warning_follows_the_npc_tag_not_the_console() {
        let bare_rig = ModelRig::from_toml(
            "[markers.engine_port]\nposition = [0.0, 0.0, 0.0]\ndirection = [0.0, 0.0, 1.0]\n",
        )
        .expect("fixture rig parses");

        let (toml, cfg) = entity("\n[captain_console]\n");
        assert!(cfg.tags.is_empty(), "precondition: not an NPC design");
        assert!(is_player_flyable(&cfg));
        let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&bare_rig));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, CATEGORY_MISSING_CAMERA);
        assert!(!findings[0].is_error(), "the hull is still playable");
        // …and satisfied by a rig that declares the default viewpoint.
        assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

        // The same file, one tag different.
        let (npc_toml, mut npc_cfg) = entity("\n[captain_console]\n");
        npc_cfg.tags = vec!["ship".to_string(), NPC_HULL_TAG.to_string()];
        assert!(npc_cfg.captain_console.is_some(), "precondition");
        assert!(!is_player_flyable(&npc_cfg));
        assert!(
            validate_entity_markers("f.toml", &npc_toml, &npc_cfg, Some(&bare_rig)).is_empty(),
            "an NPC design's rig owes the viewscreen nothing: no bridge crew ever \
             boards it, and since #885b its `[captain_console]` exists only to hold \
             the Red Alert policy it is now required to declare"
        );
    }

    #[test]
    fn entity_without_marker_refs_and_without_rig_is_clean() {
        let (toml, cfg) = entity("");
        assert!(validate_entity_markers("f.toml", &toml, &cfg, None).is_empty());
    }

    #[test]
    fn duplicate_marker_declaration_is_located() {
        let sidecar = r##"
[markers.engine_port]
position = [0.0, 0.0, 0.0]
direction = [0.0, 0.0, 1.0]

[markers.engine_port]
position = [1.0, 0.0, 0.0]
direction = [0.0, 0.0, 1.0]
"##;
        let findings = duplicate_marker_findings("rig.model.toml", sidecar);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, CATEGORY_DUPLICATE);
        assert_eq!(findings[0].source.reference, "engine_port");
        assert_eq!(findings[0].source.line, Some(6));
        assert!(has_error(&findings));
    }

    #[test]
    fn distinct_marker_declarations_are_clean() {
        assert!(duplicate_marker_findings("rig.model.toml", RIG).is_empty());
    }

    #[test]
    fn role_namespace_rules() {
        assert!(MarkerRole::Camera.accepts("camera_fore"));
        assert!(!MarkerRole::Camera.accepts("phasers_fore"));
        assert!(MarkerRole::Weapon.accepts("phasers_fore"));
        assert!(!MarkerRole::Weapon.accepts("camera_fore"));
        assert!(MarkerRole::Effect.accepts("engine_port"));
        assert!(!MarkerRole::Effect.accepts("camera_aft"));
    }

    #[test]
    fn collect_marker_refs_skips_system_marker() {
        // `[[system]] marker` is declared-but-unread; it must not produce refs.
        let toml = r##"
name = "Fixture"

[mesh]
model = "assets/models/fixture.glb"
shape = "cuboid"
colour = [1.0, 1.0, 1.0]

[[station]]
id = "Helm"
name = "Helm"
description = "Fixture station"
rank = "Cmdr."

[[system]]
id = "shields"
kind = "shields"
station = "Helm"
marker = "not_a_rig_marker"
"##;
        let cfg = EntityConfig::from_toml(toml).expect("parses");
        assert!(collect_marker_refs(&cfg).is_empty());
        assert!(validate_entity_markers("f.toml", toml, &cfg, Some(&rig())).is_empty());
    }
}

/// Shipped-asset conformance: the model-marker contract holds for everything
/// in `assets/`. These read the real files (native only, like the real-asset
/// linkage test in `model_rig`) so an authoring typo fails `cargo test`
/// instead of silently attaching a beam to the ship's centre.
#[cfg(test)]
mod shipped_assets {
    use super::*;
    use crate::model_rig::sidecar_path;

    /// Resolve an entity's rig sidecar from disk. `None` when the entity
    /// selects no model; `Some(default)` when the sidecar file is absent
    /// (mirrors `glb_visual::resolve_sidecar_rig` on native).
    fn rig_for(config: &EntityConfig) -> Option<ModelRig> {
        let mesh = config.mesh.as_ref()?;
        let model = mesh.model.as_deref()?;
        let path = sidecar_path(model, mesh.variant.as_deref());
        let toml = std::fs::read_to_string(&path).unwrap_or_default();
        Some(ModelRig::from_toml(&toml).unwrap_or_else(|e| panic!("{path} must parse: {e}")))
    }

    #[test]
    fn every_shipped_entity_marker_reference_resolves() {
        let mut checked_refs = 0usize;
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities must exist");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file = path.to_string_lossy().replace('\\', "/");
            let toml = std::fs::read_to_string(&path).expect("entity readable");
            let cfg = EntityConfig::from_toml(&toml)
                .unwrap_or_else(|e| panic!("{file} must parse: {e:?}"));
            checked_refs += collect_marker_refs(&cfg).len();
            let rig = rig_for(&cfg);
            for f in validate_entity_markers(&file, &toml, &cfg, rig.as_ref()) {
                problems.push(f.describe());
            }
        }
        assert!(
            checked_refs > 0,
            "shipped entities should author marker references"
        );
        assert!(
            problems.is_empty(),
            "shipped entity marker references must all resolve:\n{}",
            problems.join("\n")
        );
    }

    #[test]
    fn no_shipped_rig_declares_a_duplicate_marker() {
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/models").expect("assets/models must exist");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file = path.to_string_lossy().replace('\\', "/");
            let toml = std::fs::read_to_string(&path).expect("sidecar readable");
            for f in duplicate_marker_findings(&file, &toml) {
                problems.push(f.describe());
            }
        }
        assert!(
            problems.is_empty(),
            "shipped model rigs must not redeclare markers:\n{}",
            problems.join("\n")
        );
    }
}
