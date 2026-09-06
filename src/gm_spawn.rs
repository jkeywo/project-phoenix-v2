//! GM palette spawn: the authored placement vocabulary, its apply-tick
//! validation and its page-local projection (issue #1305, PRD #930 milestone
//! M2).
//!
//! # What a GM may place, and what it may not
//!
//! Everything a Game Master can spawn is a `[[gm_palette]]` row the scenario
//! or mod pack authored ([`crate::world::config::GmPaletteEntry`]): a stable
//! id, a String Table label, one `template_path`, and a CLOSED list of
//! authored variants — the "allowed overrides" half of
//! `gm-t2-directed-world-actions`. The typed action carries the palette id,
//! at most one variant id, and a resolved world position and heading. It has
//! no field an asset path, a component, a faction or a free-text override
//! could arrive in, so "arbitrary loaded asset paths are never accepted" is a
//! property of the wire shape rather than a rule somebody has to enforce.
//!
//! # Two-stage application, for [`crate::gm_event`]'s reason
//!
//! [`crate::gm_action::apply_due_actions`] runs in `PreUpdate` (a Pause must
//! be resumable while `FixedUpdate` is starved), and it holds none of the
//! parameters a spawn needs. So the reducer REVALIDATES everything that
//! decides the result — the palette entry, the variant, the placement — at the
//! agreed apply tick and then ARMS a [`PendingGmSpawn`] on
//! [`WorldContentRuntime`](crate::world::server::WorldContentRuntime). The
//! ordinary `tick_trigger_pipeline` drains that queue into the ordinary
//! `TriggerAction::SpawnEntity` dispatch every scenario author already uses,
//! so a GM spawn and a scripted spawn are the same spawn: one template loader,
//! one override merge, one `WorldIdMint` draw, one `EntitySpawnOrigin`.
//!
//! # Determinism
//!
//! The palette is a `Vec` resolved by linear scan — never a map, whose
//! iteration order must not reach a result. The pending queue is a `Vec` in
//! canonical grant order, so every peer mints the same ids in the same order at
//! the same tick. The spawned entity's scenario name is derived from the
//! canonical sequence rather than from a counter, so a peer that restored from
//! a snapshot names it identically. Nothing here reads a wall clock, a local
//! ship, or `is_local`.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::world::config::{GmPaletteEntry, TriggerAction};

/// The absolute bound on a placed coordinate, in MILLIMETRES.
///
/// Not a gameplay tunable: it is the "is this a coordinate at all" gate the
/// typed action applies before anything downstream sees it, sized far outside
/// any authored world (5,000 km) so it can never clip a legitimate placement.
pub const MAX_GM_SPAWN_COORD_MM: i64 = 5_000_000_000;

/// The absolute bound on a placed heading, in MILLIDEGREES.
pub const MAX_GM_SPAWN_HEADING_MDEG: i32 = 360_000;

/// Millimetres per metre, and millidegrees per degree — the one place the two
/// fixed-point scales are written down.
const FIXED_POINT_SCALE: f64 = 1000.0;

/// Convert one fixed-point placement to the world-space transform it means.
///
/// The conversion happens HERE, at the apply boundary, and never on the wire:
/// a placement crosses the mesh, the journal, the snapshot and the digest as
/// INTEGERS. That is deliberate. A `GmAction` is folded into the deterministic
/// digest through postcard and compared for equality by the journal's own
/// idempotency checks, and a float has neither a total equality nor a single
/// spelling — `NaN != NaN`, `0.0 == -0.0` with different bits. Fixed point
/// gives the action an exact identity and makes an out-of-range placement a
/// bounds check rather than a `is_finite()` dance.
pub fn placement_metres(position_mm: [i64; 3]) -> [f32; 3] {
    position_mm.map(|axis| ((axis as f64) / FIXED_POINT_SCALE) as f32)
}

/// Convert one fixed-point heading to degrees in the simulation's own bearing
/// convention (`atan2(dx, -dz)`: 0 faces -Z, 90 faces +X).
pub fn heading_degrees(heading_mdeg: i32) -> f32 {
    ((heading_mdeg as f64) / FIXED_POINT_SCALE) as f32
}

/// Whether a resolved placement is a placement at all.
///
/// Checked at BOTH ends for the reason every GM action's shape is: at the
/// browser ingress so a malformed request is refused as an invalid action, and
/// again at the apply tick so a grant that somehow crossed the mesh with a
/// broken payload cannot mutate the world.
pub fn placement_is_valid(position_mm: [i64; 3], heading_mdeg: i32) -> bool {
    position_mm
        .iter()
        .all(|axis| axis.abs() <= MAX_GM_SPAWN_COORD_MM)
        && heading_mdeg.abs() <= MAX_GM_SPAWN_HEADING_MDEG
}

/// Resolve one palette id against the live authored list.
///
/// A linear scan over authored order: the palette is small, and a map lookup
/// here would put a hash iteration order inside a deterministic reducer.
pub fn palette_entry<'a>(entries: &'a [GmPaletteEntry], id: &str) -> Option<&'a GmPaletteEntry> {
    entries.iter().find(|entry| entry.id == id)
}

/// One GM spawn that has crossed its canonical apply boundary and is waiting
/// for the ordinary trigger pipeline to perform it.
///
/// Genuinely cross-tick, exactly like
/// [`pending_gm_event_fires`](crate::world::server::WorldContentRuntime::pending_gm_event_fires):
/// the arm is written in `PreUpdate` at the grant's exact apply tick, while the
/// pipeline that consumes it runs in `FixedUpdate` — which a paused session or
/// a frame that runs no fixed step does not reach. So it is captured in the
/// snapshot and folded into the digest.
///
/// Everything the drain needs except the template itself is resolved HERE, at
/// the apply tick: the template is looked up from the palette at drain time so
/// one authored path has one owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGmSpawn {
    /// The authored `[[gm_palette]]` id, revalidated at the apply tick.
    pub palette: String,
    /// The chosen authored variant id, or `None` for the bare template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// The deterministic scenario name the spawned entity takes, derived from
    /// the palette entry's stem and the grant's canonical sequence.
    pub name: String,
    /// Resolved world coordinates in millimetres — never pixels, never floats
    /// (see [`placement_metres`]).
    pub position_mm: [i64; 3],
    /// Resolved heading in millidegrees, in the simulation's own bearing
    /// convention (`atan2(dx, -dz)`: 0 faces -Z, 90 faces +X), which is what
    /// `ShipPhysics::yaw` measures.
    pub heading_mdeg: i32,
}

impl PendingGmSpawn {
    /// The scenario name one canonical grant's spawn takes.
    ///
    /// Derived from the canonical sequence rather than from a live counter: a
    /// peer that restored mid-run has no counter to agree about, and the
    /// sequence is already globally unique and identical on every peer.
    pub fn derive_name(entry: &GmPaletteEntry, sequence: u64) -> String {
        format!("{}_{sequence}", entry.name_stem())
    }
}

/// Build the ordinary scenario spawn action one armed GM placement means.
///
/// The GM path deliberately produces a `TriggerAction::SpawnEntity` rather than
/// its own spawn: `dispatch_spawn_entity` owns template loading, the override
/// merge, the uuid draw, name/group registration and the contingency gate, and
/// a second implementation of any of those is a second set of answers.
///
/// `rotation` is the Transform Euler the sim's own physics writes —
/// `Quat::from_euler(YXZ, -yaw, 0, roll)` — so the authored rotation and the
/// hull's own pose agree the moment it exists rather than one tick later.
pub fn spawn_action(pending: &PendingGmSpawn, entry: &GmPaletteEntry) -> TriggerAction {
    let overrides = pending
        .variant
        .as_deref()
        .and_then(|id| entry.variant(id))
        .and_then(|variant| variant.overrides.clone());
    TriggerAction::SpawnEntity {
        template_path: entry.template_path.clone(),
        name: pending.name.clone(),
        anchor: None,
        position: Some(placement_metres(pending.position_mm)),
        rotation: Some([
            0.0,
            -heading_degrees(pending.heading_mdeg).to_radians(),
            0.0,
        ]),
        scale: None,
        groups: entry.groups.clone(),
        overrides,
    }
}

/// One authored variant as the GM spawn panel sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmPaletteVariantOption {
    pub id: String,
    /// String Table id for the operator-facing label.
    pub label: String,
}

/// One placeable palette row as the GM spawn panel sees it.
///
/// Deliberately NOT the authored struct: `template_path` and the override
/// documents stay on the simulation side. The browser composes a spawn from an
/// id and a variant id, so shipping it the path would be shipping it a
/// vocabulary it must never use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmPaletteOption {
    pub id: String,
    pub label: String,
    pub variants: Vec<GmPaletteVariantOption>,
}

/// Absolute GM spawn-panel projection: the placeable palette, plus the bounded
/// attributed results of the world-spawn action family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GmSpawnProjection {
    pub palette: Vec<GmPaletteOption>,
    pub results: Vec<crate::gm_action::LoggedGmAction>,
}

/// The placeable palette in authored order.
pub fn palette_options(entries: &[GmPaletteEntry]) -> Vec<GmPaletteOption> {
    entries
        .iter()
        .map(|entry| GmPaletteOption {
            id: entry.id.clone(),
            label: entry.label.clone(),
            variants: entry
                .variants
                .iter()
                .map(|variant| GmPaletteVariantOption {
                    id: variant.id.clone(),
                    label: variant.label.clone(),
                })
                .collect(),
        })
        .collect()
}

/// Page-local last-published spawn projection. Presentation only: the
/// authoritative facts are the authored palette and the GM action journal.
#[derive(Resource, Clone, Debug, Default)]
pub struct LastGmSpawnProjection(Option<GmSpawnProjection>);

/// Build the absolute projection from live authoritative state.
pub fn projection(
    entries: &[GmPaletteEntry],
    log: &crate::gm_action::GmActionLog,
    refusals: &crate::gm_action::LocalGmActionRefusals,
) -> GmSpawnProjection {
    GmSpawnProjection {
        palette: palette_options(entries),
        results: crate::gm_action::projected_results(
            crate::gm_action::GmActionKind::WorldSpawn,
            log,
            refusals,
        ),
    }
}

/// Push an absolute page-local projection whenever the palette or the bounded
/// result feed changes.
///
/// Frame-driven for [`crate::gm_event::publish_mission_projection`]'s reason: a
/// paused session still has to report the result of a placement that was
/// refused at admission.
pub fn publish_spawn_projection(
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    log: Res<crate::gm_action::GmActionLog>,
    refusals: Res<crate::gm_action::LocalGmActionRefusals>,
    mut last: ResMut<LastGmSpawnProjection>,
    mut writer: MessageWriter<crate::console_bridge::GmSpawnChanged>,
) {
    let empty: Vec<GmPaletteEntry> = Vec::new();
    let entries = match runtime.as_deref() {
        Some(runtime) => runtime.gm_palette.as_slice(),
        None => empty.as_slice(),
    };
    let next = projection(entries, &log, &refusals);
    if last.0.as_ref() == Some(&next) {
        return;
    }
    last.0 = Some(next.clone());
    writer.write(crate::console_bridge::GmSpawnChanged { payload: next });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::GmPaletteVariant;

    fn entry(id: &str) -> GmPaletteEntry {
        GmPaletteEntry {
            id: id.to_string(),
            label: format!("world.test.gm_palette.{id}.label"),
            template_path: format!("assets/entities/{id}.toml"),
            name_prefix: None,
            groups: vec!["hostiles".to_string()],
            variants: Vec::new(),
        }
    }

    #[test]
    fn a_placement_outside_the_coordinate_bound_is_not_a_placement() {
        assert!(placement_is_valid([10_000, 0, -20_000], 45_000));
        assert!(!placement_is_valid([MAX_GM_SPAWN_COORD_MM + 1, 0, 0], 0));
        assert!(!placement_is_valid(
            [0, 0, 0],
            MAX_GM_SPAWN_HEADING_MDEG + 1
        ));
        assert!(!placement_is_valid(
            [0, 0, 0],
            -MAX_GM_SPAWN_HEADING_MDEG - 1
        ));
    }

    #[test]
    fn fixed_point_placement_converts_to_world_space_exactly() {
        assert_eq!(
            placement_metres([120_500, 0, -40_250]),
            [120.5, 0.0, -40.25]
        );
        assert_eq!(heading_degrees(90_000), 90.0);
        assert_eq!(heading_degrees(-1_500), -1.5);
    }

    #[test]
    fn a_palette_id_resolves_by_authored_order_and_nothing_else() {
        let entries = vec![entry("raider"), entry("tender")];
        assert_eq!(
            palette_entry(&entries, "tender").map(|e| e.id.as_str()),
            Some("tender")
        );
        assert!(palette_entry(&entries, "assets/entities/raider.toml").is_none());
        assert!(palette_entry(&entries, "").is_none());
    }

    #[test]
    fn the_spawn_action_is_the_ordinary_scenario_one() {
        let mut raider = entry("raider");
        raider.name_prefix = Some("gm_raider".to_string());
        raider.variants.push(GmPaletteVariant {
            id: "blood_eagle".to_string(),
            label: "world.test.gm_palette.raider.blood_eagle.label".to_string(),
            overrides: Some(toml::Value::Table(toml::map::Map::new())),
        });
        let pending = PendingGmSpawn {
            palette: "raider".to_string(),
            variant: Some("blood_eagle".to_string()),
            name: PendingGmSpawn::derive_name(&raider, 7),
            position_mm: [120_000, 0, -40_000],
            heading_mdeg: 90_000,
        };
        assert_eq!(pending.name, "gm_raider_7");
        let TriggerAction::SpawnEntity {
            template_path,
            name,
            anchor,
            position,
            rotation,
            groups,
            overrides,
            ..
        } = spawn_action(&pending, &raider)
        else {
            panic!("a GM placement is an ordinary scenario spawn");
        };
        assert_eq!(template_path, "assets/entities/raider.toml");
        assert_eq!(name, "gm_raider_7");
        assert_eq!(anchor, None, "a GM placement carries resolved coordinates");
        assert_eq!(position, Some([120.0, 0.0, -40.0]));
        let rotation = rotation.expect("a placed heading is an authored rotation");
        assert!((rotation[1] + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert_eq!(groups, vec!["hostiles".to_string()]);
        assert!(
            overrides.is_some(),
            "the chosen variant's overrides ride along"
        );
    }

    #[test]
    fn an_unknown_variant_id_contributes_no_overrides() {
        let raider = entry("raider");
        let pending = PendingGmSpawn {
            palette: "raider".to_string(),
            variant: Some("not-authored".to_string()),
            name: "raider_1".to_string(),
            position_mm: [0, 0, 0],
            heading_mdeg: 0,
        };
        let TriggerAction::SpawnEntity { overrides, .. } = spawn_action(&pending, &raider) else {
            panic!("spawn action");
        };
        assert!(overrides.is_none());
    }

    #[test]
    fn the_projected_palette_never_carries_a_template_path() {
        let mut raider = entry("raider");
        raider.variants.push(GmPaletteVariant {
            id: "blood_eagle".to_string(),
            label: "world.test.gm_palette.raider.blood_eagle.label".to_string(),
            overrides: None,
        });
        let options = palette_options(std::slice::from_ref(&raider));
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].id, "raider");
        assert_eq!(options[0].label, "world.test.gm_palette.raider.label");
        assert_eq!(options[0].variants.len(), 1);
        assert_eq!(options[0].variants[0].id, "blood_eagle");
        // Exhaustive destructuring rather than a string search: this fails to
        // COMPILE if a future field (a template path, an override document)
        // joins the projected row, which is the guarantee worth having.
        let GmPaletteOption {
            id: _,
            label: _,
            variants,
        } = &options[0];
        let GmPaletteVariantOption { id: _, label: _ } = &variants[0];
    }
}
