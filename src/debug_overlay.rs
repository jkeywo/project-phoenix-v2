use bevy::prelude::*;
use std::collections::VecDeque;

use crate::entity_spawner::RegionShapeSection;
use crate::modifiers::ShipModifiers;
use crate::region_shape::RegionShape;

/// Resource indicating whether debug region wireframes are enabled.
#[derive(Resource)]
pub struct DebugRegionsEnabled(pub bool);

/// Resource indicating whether the modifier debug overlay (F3) is enabled.
#[derive(Resource, Default)]
pub struct DebugOverlayEnabled(pub bool);

/// Resource indicating whether the simulation is debug-paused (F9).
#[derive(Resource, Default)]
pub struct DebugPaused(pub bool);

/// Resource indicating whether the damage debug overlay (F8) is enabled.
#[derive(Resource, Default)]
pub struct DebugDamageEnabled(pub bool);

/// Resource indicating whether the entity behavior debug overlay (F5) is enabled.
#[derive(Resource, Default)]
pub struct DebugEntitiesEnabled(pub bool);

/// Resource indicating whether the entity inspector overlay (F6) is enabled.
#[derive(Resource, Default)]
pub struct DebugEntityInspectorEnabled(pub bool);

/// Maximum number of damage log entries retained.
pub const DAMAGE_LOG_CAPACITY: usize = 10;

/// A single damage event recorded for the F8 overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageLogEntry {
    /// Human-readable description of the damage source (e.g. asteroid uuid,
    /// region uuid, weapon name).
    pub source: String,
    /// Shield arc label hit, or `None` when shields were bypassed / absent.
    pub shield_arc: Option<String>,
    /// Total damage amount before shield absorption (hull + shield combined).
    pub amount: f32,
}

/// Ring-buffer of the most recent damage events.
///
/// Always retains up to `DAMAGE_LOG_CAPACITY` entries, newest at the front.
/// Populated by damage application sites; read by the F8 overlay system.
#[derive(Resource, Default)]
pub struct DamageLog {
    pub entries: VecDeque<DamageLogEntry>,
}

impl DamageLog {
    /// Push a new entry to the front, evicting the oldest when at capacity.
    pub fn push(&mut self, entry: DamageLogEntry) {
        self.entries.push_front(entry);
        while self.entries.len() > DAMAGE_LOG_CAPACITY {
            self.entries.pop_back();
        }
    }

    /// Format the log as a multi-line string for display.
    pub fn format(&self) -> String {
        if self.entries.is_empty() {
            return "(no damage)".to_string();
        }
        let mut out = String::from("DAMAGE LOG (newest first)\n");
        for (i, e) in self.entries.iter().enumerate() {
            let arc = e.shield_arc.as_deref().unwrap_or("—");
            out.push_str(&format!(
                "{:>2}. {:<24} arc={:<10} dmg={:.1}\n",
                i + 1,
                e.source,
                arc,
                e.amount
            ));
        }
        out
    }
}

/// Server-only plugin that draws region shape wireframes when enabled.
///
/// The `enabled` field is typically set from the `?debug_regions=1` URL parameter
/// on WASM (via `bridge.rs`), or directly in tests.
pub struct DebugOverlayPlugin {
    pub enabled: bool,
}

impl Plugin for DebugOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DebugRegionsEnabled(self.enabled));
        app.init_resource::<DebugOverlayEnabled>();
        app.init_resource::<DebugPaused>();
        app.init_resource::<DebugDamageEnabled>();
        app.init_resource::<DamageLog>();
        app.init_resource::<DebugEntitiesEnabled>();
        app.init_resource::<DebugEntityInspectorEnabled>();
        if should_install_region_wireframes() {
            app.add_systems(
                Update,
                draw_region_wireframes.run_if(|r: Res<DebugRegionsEnabled>| r.0),
            );
        }
        app.add_systems(
            PostUpdate,
            write_debug_state.run_if(|r: Res<DebugOverlayEnabled>| r.0),
        );
        app.add_systems(
            PostUpdate,
            write_damage_log.run_if(|r: Res<DebugDamageEnabled>| r.0),
        );
        app.add_systems(
            PostUpdate,
            write_entity_debug_state.run_if(|r: Res<DebugEntitiesEnabled>| r.0),
        );
        app.add_systems(
            PostUpdate,
            update_entity_inspector.run_if(|r: Res<DebugEntityInspectorEnabled>| r.0),
        );
    }
}

/// Returns `true` when running under Playwright/WebDriver automation (WASM only).
///
/// On native and non-server builds this always returns `false`, so non-WASM
/// tests and native simulation apps keep full gizmo rendering without any
/// special setup.
///
/// Uses `navigator.webdriver` (set by Playwright / Selenium) to detect
/// automation. When the property is absent or the detection fails, the safe
/// default (`false` — not automation) is returned. Callers in automation
/// mode should skip any functionality that depends on renderer resources
/// (e.g. `Gizmos`) which are not available under `MinimalPlugins`.
#[cfg(target_arch = "wasm32")]
pub fn is_playwright_automation() -> bool {
    web_sys::window()
        .and_then(|w| {
            let nav = w.navigator();
            js_sys::Reflect::get(&nav, &"webdriver".into())
                .ok()
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false)
}

/// Native / non-server fallback — no automation possible.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_playwright_automation() -> bool {
    false
}

fn should_install_region_wireframes() -> bool {
    !is_playwright_automation()
}

/// Reads `ShipModifiers` from the LocalShip entity and writes the formatted
/// debug text to the WASM thread-local `DEBUG_STATE_STRING`.
///
/// Only runs when `DebugOverlayEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn write_debug_state(modifiers_q: Query<&ShipModifiers, With<crate::server_app::LocalShip>>) {
    if let Some(modifiers) = modifiers_q.iter().next() {
        let text = modifiers.format_debug();
        crate::bridge::set_debug_state_string(text);
    }
}

/// Native / test stub — does nothing (no thread-locals available outside WASM).
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn write_debug_state(_modifiers_q: Query<&ShipModifiers, With<crate::server_app::LocalShip>>) {}

/// Reads the `DamageLog` resource and writes the formatted text to the WASM
/// thread-local `DAMAGE_LOG_STRING` for the F8 overlay.
///
/// Only runs when `DebugDamageEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn write_damage_log(log: Res<DamageLog>) {
    let text = log.format();
    crate::bridge::set_damage_log_string(text);
}

/// Native / test stub — does nothing.
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn write_damage_log(_log: Res<DamageLog>) {}

/// Reads all entities with `BehaviourSection` (i.e. AI-driven NPCs) and writes a
/// formatted table (name, position, current state) to the WASM thread-local for F5.
///
/// Only runs when `DebugEntitiesEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn write_entity_debug_state(
    entities: Query<(
        &crate::entities::spawner::BehaviourSection,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::weapons_plugin::TacticalRadarSelection>,
    )>,
) {
    let count = entities.iter().count();
    let mut out = format!("ENTITY BEHAVIOR ({} entities)\n", count);
    for (i, (_ai, transform, name, memory)) in entities.iter().enumerate() {
        let label = name.map(|n| n.0.as_str()).unwrap_or("<unnamed>");
        let p = transform.translation;
        // The ship's authoritative Tactical lock (issue #702). Was
        // `ShipAiMemory.target`, a private mirror that could disagree with what
        // the ship was actually shooting — so the overlay could report a target
        // the ship had not selected.
        let target_str = memory
            .and_then(|t| t.0.clone())
            .unwrap_or_else(|| "none".to_string());
        out.push_str(&format!(
            "{:>2}. {:<20} pos=({:>7.1},{:>7.1},{:>7.1})  target={}\n",
            i + 1,
            label,
            p.x,
            p.y,
            p.z,
            target_str
        ));
    }
    crate::bridge::set_entity_debug_string(out);
}

/// Native / test stub — does nothing.
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn write_entity_debug_state(
    _entities: Query<(
        &crate::entities::spawner::BehaviourSection,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::weapons_plugin::TacticalRadarSelection>,
    )>,
) {
}

/// Reads all non-asteroid entities plus the player ship resources and writes a
/// formatted entity inspector block to the WASM thread-local for F6.
///
/// Displays: name, tags, position, distance from player, faction name, hull HP,
/// shield arcs (player ship only), comms hailability, and AI state.
///
/// Only runs when `DebugEntityInspectorEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn update_entity_inspector(
    entities: Query<
        (
            &Transform,
            &crate::entities::spawner::EntityName,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::comms::component::CommsRange>,
            Option<&crate::weapons_plugin::TacticalRadarSelection>,
            &crate::entities::spawner::EntityTagsSection,
        ),
        bevy::ecs::query::Without<crate::server_app::Asteroid>,
    >,
    ship_physics_q: Query<&crate::ship_state::ShipPhysics, With<crate::server_app::LocalShip>>,
    player_hull_q: Query<
        &crate::entity_spawner::EntitySystemHull,
        With<crate::server_app::LocalShip>,
    >,
    ship_shields_q: Query<&crate::server_app::ShipShields, With<crate::server_app::LocalShip>>,
    faction_registry: Res<crate::entities::config_cache::FactionRegistryResource>,
) {
    let Ok(ship_shields) = ship_shields_q.single() else {
        return;
    };
    let ship_phys = ship_physics_q.single().ok().copied().unwrap_or_default();
    let player_x = ship_phys.x;
    let player_z = ship_phys.z;

    let mut out = String::from("ENTITY INSPECTOR\n");
    out.push_str("────────────────────────────────────────────────────────────\n");

    // ── Player ship ────────────────────────────────────────────────────────
    out.push_str(&format!(
        "[Player Ship]  pos=({:>8.1}, {:>8.1})\n",
        player_x, player_z
    ));

    // Per-system hull from the LocalShip's EntitySystemHull component.
    let hull_entries: Vec<(crate::messages::SystemId, f32, f32)> = player_hull_q
        .single()
        .map(|h| {
            h.0.entries()
                .map(|(sid, cur, max)| (sid.clone(), cur, max))
                .collect()
        })
        .unwrap_or_default();
    if hull_entries.is_empty() {
        out.push_str("  hull: n/a\n");
    } else {
        out.push_str("  hull:");
        for (sid, cur, max) in &hull_entries {
            out.push_str(&format!("  {} {}/{}", sid.0, *cur as i32, *max as i32));
        }
        out.push('\n');
    }

    // Per-arc shields
    let facings = &ship_shields.0.facings;
    if facings.is_empty() {
        out.push_str("  shields: n/a\n");
    } else {
        out.push_str("  shields:");
        for f in facings {
            let pct = if f.max_hp > 0 {
                (f.hp as f32 / f.max_hp as f32 * 100.0) as i32
            } else {
                0
            };
            let status = if f.offline_remaining > 0.0 {
                " [OFFLINE]"
            } else {
                ""
            };
            let focus = if f.is_focused { "*" } else { "" };
            out.push_str(&format!(
                "  {}{} {}/{} ({}%){}",
                focus, f.label, f.hp, f.max_hp, pct, status
            ));
        }
        out.push('\n');
    }

    out.push_str("────────────────────────────────────────────────────────────\n");

    // ── World entities ─────────────────────────────────────────────────────
    let mut sorted: Vec<_> = entities.iter().collect();
    // Sort by distance from player for readability
    sorted.sort_by(|a, b| {
        let da = {
            let p = a.0.translation;
            let dx = p.x - player_x;
            let dz = p.z - player_z;
            dx * dx + dz * dz
        };
        let db = {
            let p = b.0.translation;
            let dx = p.x - player_x;
            let dz = p.z - player_z;
            dx * dx + dz * dz
        };
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });

    for (i, (transform, name, hull, faction_comp, comms_range, ai, tags)) in
        sorted.iter().enumerate()
    {
        let p = transform.translation;
        let dx = p.x - player_x;
        let dz = p.z - player_z;
        let dist = (dx * dx + dz * dz).sqrt();

        let tag_list = tags.0.join(", ");
        out.push_str(&format!("{:>2}. {}  [{}]\n", i + 1, name.0, tag_list));
        out.push_str(&format!(
            "    pos=({:>8.1}, {:>8.1})  dist={:>7.1}u\n",
            p.x, p.z, dist
        ));

        // Faction
        if let Some(fc) = faction_comp {
            let faction_name = faction_registry
                .0
                .get(&fc.0)
                .map(|f| f.name.as_str())
                .unwrap_or("<unknown>");
            out.push_str(&format!("    faction: {}\n", faction_name));
        }

        // Hull
        if let Some(h) = hull {
            let cur = h.0.total_current();
            let max = h.0.total_max();
            let pct = if max > 0.0 {
                (cur / max * 100.0) as i32
            } else {
                0
            };
            out.push_str(&format!(
                "    hull: {}/{} ({}%)\n",
                cur as i32, max as i32, pct
            ));
        }

        // Comms
        if let Some(range) = comms_range {
            let in_range = dist <= range.0;
            if in_range {
                out.push_str("    comms: hailable (in range)\n");
            } else {
                out.push_str(&format!("    comms: hailable (range {:.0}u)\n", range.0));
            }
        }

        // AI state
        if let Some(target) = ai {
            out.push_str(&format!(
                "    ai: target={}\n",
                target.0.clone().unwrap_or_else(|| "none".to_string())
            ));
        }
    }

    out.push_str("────────────────────────────────────────────────────────────\n");
    crate::bridge::set_entity_inspector_string(out);
}

/// Native / test stub — does nothing.
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn update_entity_inspector(
    _entities: Query<
        (
            &Transform,
            &crate::entities::spawner::EntityName,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::comms::component::CommsRange>,
            Option<&crate::weapons_plugin::TacticalRadarSelection>,
            &crate::entities::spawner::EntityTagsSection,
        ),
        bevy::ecs::query::Without<crate::server_app::Asteroid>,
    >,
    _ship_shields_q: Query<&crate::server_app::ShipShields, With<crate::server_app::LocalShip>>,
    _faction_registry: Res<crate::entities::config_cache::FactionRegistryResource>,
) {
}

/// Draws wireframe outlines for every region entity with a shape component.
fn draw_region_wireframes(regions: Query<(&Transform, &RegionShapeSection)>, mut gizmos: Gizmos) {
    for (transform, shape) in regions.iter() {
        let origin = transform.translation - Vec3::Y * 10.0;
        match &shape.0 {
            RegionShape::Sphere { radius } => {
                draw_sphere_wireframe(&mut gizmos, origin, *radius);
            }
            RegionShape::Box { half_extents, .. } => {
                draw_box_wireframe(&mut gizmos, origin, *half_extents);
            }
            RegionShape::Torus {
                inner_radius,
                outer_radius,
            } => {
                draw_torus_wireframe(&mut gizmos, origin, *inner_radius, *outer_radius);
            }
        }
    }
}

fn draw_sphere_wireframe(gizmos: &mut Gizmos, origin: Vec3, radius: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    gizmos.circle(
        Isometry3d::new(origin, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), radius, color);
    gizmos.circle(
        Isometry3d::new(origin, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
}

fn draw_box_wireframe(gizmos: &mut Gizmos, origin: Vec3, half_extents: [f32; 3]) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    let [hx, hy, hz] = half_extents;
    let corners = [
        Vec3::new(-hx, -hy, -hz),
        Vec3::new(hx, -hy, -hz),
        Vec3::new(hx, -hy, hz),
        Vec3::new(-hx, -hy, hz),
        Vec3::new(-hx, hy, -hz),
        Vec3::new(hx, hy, -hz),
        Vec3::new(hx, hy, hz),
        Vec3::new(-hx, hy, hz),
    ]
    .map(|c| origin + c);
    let edges: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (i, j) in edges {
        gizmos.line(corners[i], corners[j], color);
    }
}

fn draw_torus_wireframe(gizmos: &mut Gizmos, origin: Vec3, inner_radius: f32, outer_radius: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    // Draw two horizontal circles representing the inner and outer edges of the torus
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), inner_radius, color);
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), outer_radius, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_regions_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(!enabled.0, "default should be disabled");
    }

    #[test]
    fn debug_regions_enabled_when_flag_set() {
        let plugin = DebugOverlayPlugin { enabled: true };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(enabled.0, "should be enabled when flag is set");
    }

    /// Toggling the resource from false → true should flip DebugRegionsEnabled.
    #[test]
    fn toggle_debug_regions_false_to_true() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        // Simulate what drain_debug_toggles does: flip the resource.
        app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = true;
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(enabled.0, "resource should be true after toggle");
    }

    /// Toggling the resource from true → false should flip DebugRegionsEnabled.
    #[test]
    fn toggle_debug_regions_true_to_false() {
        let plugin = DebugOverlayPlugin { enabled: true };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = false;
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(!enabled.0, "resource should be false after toggle");
    }

    // ── DebugOverlayEnabled tests ─────────────────────────────────────────

    #[test]
    fn debug_overlay_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugOverlayEnabled>();
        assert!(!enabled.0, "overlay should be disabled by default");
    }

    #[test]
    fn toggle_debug_overlay_false_to_true() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = true;
        let enabled = app.world().resource::<DebugOverlayEnabled>();
        assert!(enabled.0, "overlay should be enabled after toggle");
    }

    #[test]
    fn toggle_debug_overlay_true_to_false() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = true;
        app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = false;
        let enabled = app.world().resource::<DebugOverlayEnabled>();
        assert!(!enabled.0, "overlay should be disabled after second toggle");
    }

    // ── DamageLog tests ───────────────────────────────────────────────────

    fn entry(source: &str, arc: Option<&str>, amount: f32) -> DamageLogEntry {
        DamageLogEntry {
            source: source.to_string(),
            shield_arc: arc.map(|s| s.to_string()),
            amount,
        }
    }

    #[test]
    fn damage_log_starts_empty() {
        let log = DamageLog::default();
        assert!(log.entries.is_empty());
        assert_eq!(log.format(), "(no damage)");
    }

    #[test]
    fn damage_log_pushes_newest_to_front() {
        let mut log = DamageLog::default();
        log.push(entry("a", Some("Fore"), 1.0));
        log.push(entry("b", Some("Port"), 2.0));
        assert_eq!(log.entries.len(), 2);
        assert_eq!(log.entries[0].source, "b");
        assert_eq!(log.entries[1].source, "a");
    }

    #[test]
    fn damage_log_caps_at_capacity() {
        let mut log = DamageLog::default();
        for i in 0..(DAMAGE_LOG_CAPACITY + 5) {
            log.push(entry(&format!("s{}", i), None, i as f32));
        }
        assert_eq!(log.entries.len(), DAMAGE_LOG_CAPACITY);
        // Newest at front
        assert_eq!(
            log.entries[0].source,
            format!("s{}", DAMAGE_LOG_CAPACITY + 4)
        );
        // Oldest retained is the one DAMAGE_LOG_CAPACITY back from newest
        assert_eq!(log.entries[DAMAGE_LOG_CAPACITY - 1].source, "s5");
    }

    #[test]
    fn damage_log_format_includes_source_arc_and_amount() {
        let mut log = DamageLog::default();
        log.push(entry("asteroid-42", Some("Fore"), 12.5));
        log.push(entry("region-zone", None, 3.0));
        let text = log.format();
        assert!(text.contains("region-zone"));
        assert!(text.contains("asteroid-42"));
        assert!(text.contains("Fore"));
        assert!(text.contains("12.5"));
        assert!(text.contains("3.0"));
        // None arc renders as em-dash placeholder
        assert!(text.contains("—"));
    }

    #[test]
    fn debug_damage_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugDamageEnabled>();
        assert!(!enabled.0, "damage overlay should be disabled by default");
    }
}
