//! Server-side viewscreen radar plugin.
//!
//! Replaces the legacy gizmos-based `draw_radar_overlay` with the same
//! `GuiRadarPlugin` + `GenericRadarWidget` + `RadarAppearance` pipeline used
//! by all client phone consoles.  Configuration is sourced from
//! `assets/entities/player_ship.toml` via the `config_cache` — the same path
//! `lobby/server.rs` uses — so the viewscreen and the phones always reflect the
//! same TOML values.
//!
//! Four radar containers are spawned at startup (hidden by default):
//!
//! | View mode            | Range source                            | Orientation  |
//! |----------------------|-----------------------------------------|--------------|
//! | `Radar`              | `[helm_console.radar]`                  | ShipRelative |
//! | `ScienceRadar` / `SensorsRadar` | `[sensors_console.long_range_radar]` | WorldFixed |
//! | `SystemChart`        | `[navigation_console.system_chart]`     | WorldFixed   |
//! | `NavigationChart`    | `[navigation_console.system_chart]`     | WorldFixed + WorldCentredRadar + AutoScale |
//!
//! `sync_server_radar_bridge` mirrors `bridge_sim_to_radar` in `gui/radar.rs`:
//! each frame it picks the active widget (matching `ship.view_mode`) and
//! reconciles `WorldResource.0.entities` into ECS blips under that widget's
//! `RadarBlipMap`. Inactive widgets keep their stale blip set but are
//! invisible, so the cost is bounded to one widget per frame.

use bevy::prelude::*;
use std::collections::HashSet;

use crate::gui::radar::GuiRadarPlugin;
use crate::gui::{
    bridge_sim_to_radar, AutoScaleRadar, ConsoleRadar, GenericRadar, OrientationMode, RadarBlipMap,
    RadarCenterPose, RadarClipMode, RadarFilter, WorldCentredRadar,
};
use crate::lobby::WorldResource;
use crate::messages::{GamePhase, ViewMode};
use crate::ship_state::ShipPhysics;

// ── Marker component ──────────────────────────────────────────────────────────

/// Identifies which view mode a radar container corresponds to.
/// All four containers carry this component; `toggle_viewscreen_radar_widgets`
/// compares it against `ShipState.view_mode` to show/hide the right one.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum RadarContainerMode {
    Helm,
    Science,
    SystemChart,
    Nav,
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct ServerViewscreenRadarPlugin;

impl Plugin for ServerViewscreenRadarPlugin {
    fn build(&self, app: &mut App) {
        // Guard against double-registration in case GuiPlugin (which wraps
        // GuiRadarPlugin) has already been added elsewhere.
        if !app.is_plugin_added::<GuiRadarPlugin>() {
            app.add_plugins(GuiRadarPlugin);
        }

        app.add_systems(Startup, spawn_viewscreen_radar_widgets)
            .add_systems(
                Update,
                (
                    sync_server_radar_bridge.run_if(in_state(GamePhase::InProgress)),
                    toggle_viewscreen_radar_widgets,
                ),
            );
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a `RadarFilter` from a TOML `shows` tag list.
fn radar_filter_from_shows(shows: &[crate::entity_tags::EntityTag]) -> RadarFilter {
    let set: HashSet<String> = shows.iter().map(|t| t.as_str().to_string()).collect();
    RadarFilter(set)
}

/// Map a `ViewMode` to the `ConsoleRadar` variant of the widget that should
/// receive blip updates while that mode is active. Returns `None` for view
/// modes that don't drive a viewscreen radar.
fn view_mode_to_console_radar(mode: &ViewMode) -> Option<ConsoleRadar> {
    match mode {
        ViewMode::Radar => Some(ConsoleRadar::ViewscreenHelm),
        ViewMode::ScienceRadar | ViewMode::SensorsRadar => Some(ConsoleRadar::ViewscreenScience),
        ViewMode::SystemChart => Some(ConsoleRadar::ViewscreenSystemChart),
        ViewMode::NavigationChart => Some(ConsoleRadar::ViewscreenNav),
        _ => None,
    }
}

/// Spawn a full-screen absolute container node that holds a radar widget as
/// its only child.  The widget is constrained to 80 % of viewport height,
/// centred inside the container (which fills the full viewport).
///
/// The container starts hidden; `toggle_viewscreen_radar_widgets` shows/hides
/// it based on the current view mode.
fn spawn_radar_container(commands: &mut Commands, mode: RadarContainerMode, widget_entity: Entity) {
    let container = commands
        .spawn((
            mode,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            ZIndex(5),
        ))
        .id();
    commands.entity(widget_entity).insert(Node {
        width: Val::Vh(80.0),
        aspect_ratio: Some(1.0),
        position_type: PositionType::Relative,
        ..default()
    });
    commands.entity(container).add_child(widget_entity);
}

// ── Startup system: radar widgets ─────────────────────────────────────────────
//
// Radar icon PNGs are no longer eagerly preloaded into a fixed whitelist —
// `sync_radar_blip_nodes` (gui/radar.rs) loads each icon name lazily into
// `RadarIconLookup` the first time it's seen, by naming convention
// (`icon_asset_path`). No fixed icon set to maintain here.

/// Description of one viewscreen radar widget that can be table-driven.
struct ViewscreenRadarSpec {
    container_mode: RadarContainerMode,
    console_radar: ConsoleRadar,
    range: f32,
    orientation: OrientationMode,
    filter: RadarFilter,
    world_centred: bool,
    auto_scale: Option<AutoScaleRadar>,
}

fn spawn_viewscreen_radar_widgets(mut commands: Commands) {
    let config_cache = crate::config_cache::get_config_cache();
    let ship_config = config_cache.get("assets/entities/player_ship.toml");

    let helm_radar = ship_config
        .and_then(|c| c.helm_console.as_ref())
        .and_then(|hc| hc.radar.as_ref())
        .cloned()
        .unwrap_or_default();
    let science_radar = ship_config
        .and_then(|c| c.sensors_console.as_ref())
        .map(|sc| sc.long_range_radar.clone())
        .unwrap_or_default();
    let nav_chart = ship_config
        .and_then(|c| c.navigation_console.as_ref())
        .map(|nc| nc.system_chart.clone())
        .unwrap_or_default();

    let specs = [
        ViewscreenRadarSpec {
            container_mode: RadarContainerMode::Helm,
            console_radar: ConsoleRadar::ViewscreenHelm,
            range: helm_radar.range,
            orientation: OrientationMode::ShipRelative,
            filter: radar_filter_from_shows(&helm_radar.shows),
            world_centred: false,
            auto_scale: None,
        },
        ViewscreenRadarSpec {
            container_mode: RadarContainerMode::Science,
            console_radar: ConsoleRadar::ViewscreenScience,
            range: science_radar.range,
            orientation: OrientationMode::WorldFixed,
            filter: radar_filter_from_shows(&science_radar.shows),
            world_centred: false,
            auto_scale: None,
        },
        // System chart: same TOML config as the navigation console's chart
        // (range from `[navigation_console.system_chart]`, not science).
        ViewscreenRadarSpec {
            container_mode: RadarContainerMode::SystemChart,
            console_radar: ConsoleRadar::ViewscreenSystemChart,
            range: nav_chart.range,
            orientation: OrientationMode::WorldFixed,
            filter: radar_filter_from_shows(&nav_chart.shows),
            world_centred: false,
            auto_scale: None,
        },
        // Navigation chart: world-centred (star at origin), auto-scaling.
        ViewscreenRadarSpec {
            container_mode: RadarContainerMode::Nav,
            console_radar: ConsoleRadar::ViewscreenNav,
            range: nav_chart.range,
            orientation: OrientationMode::WorldFixed,
            filter: radar_filter_from_shows(&nav_chart.shows),
            world_centred: true,
            auto_scale: Some(AutoScaleRadar {
                margin: 1.1,
                min_range: 50.0,
            }),
        },
    ];

    for spec in specs {
        let widget = GenericRadar::spawn(
            &mut commands,
            spec.range,
            spec.orientation,
            spec.filter,
            None,
            None,
            RadarClipMode::Circle,
            1.0,
            1.0,
        );
        commands
            .entity(widget)
            .insert((spec.console_radar, RadarBlipMap::default()));
        if spec.world_centred {
            commands.entity(widget).insert(WorldCentredRadar);
        }
        if let Some(auto) = spec.auto_scale {
            commands.entity(widget).insert(auto);
        }
        spawn_radar_container(&mut commands, spec.container_mode, widget);
    }
}

// ── Update: bridge WorldResource → radar entities ─────────────────────────────

/// Reconciles `WorldResource.0.entities` into ECS radar blip entities under
/// the widget whose `ConsoleRadar` matches the current `ship.view_mode`.
///
/// Per-frame the bridge runs for at most one widget (the active one). The
/// other viewscreen widgets retain their stale blip sets while hidden, which
/// keeps the work bounded.
fn sync_server_radar_bridge(
    mut commands: Commands,
    world: Option<Res<WorldResource>>,
    view_mode_q: Query<&crate::ship_state::ShipViewMode, With<crate::simulation::LocalShip>>,
    physics_q: Query<&ShipPhysics, With<crate::simulation::LocalShip>>,
    mut widgets: Query<(Entity, &ConsoleRadar, &mut RadarBlipMap)>,
) {
    let Some(world) = world else { return };
    let view_mode = view_mode_q
        .single()
        .map(|vm| vm.view_mode.clone())
        .unwrap_or(crate::messages::ViewMode::Camera(
            crate::messages::ViewDirection::Fore,
        ));
    let Some(active) = view_mode_to_console_radar(&view_mode) else {
        return;
    };
    let Some((widget, _, mut map)) = widgets.iter_mut().find(|(_, c, _)| **c == active) else {
        return;
    };
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    bridge_sim_to_radar(
        &mut commands,
        widget,
        &mut map,
        RadarCenterPose {
            x: physics.x,
            z: physics.z,
            yaw: physics.yaw,
        },
        &world.0.entities,
    );
}

// ── Update: toggle radar container visibility ─────────────────────────────────

fn toggle_viewscreen_radar_widgets(
    view_mode_q: Option<
        Query<&crate::ship_state::ShipViewMode, With<crate::simulation::LocalShip>>,
    >,
    state: Res<State<GamePhase>>,
    mut containers: Query<(&RadarContainerMode, &mut Visibility)>,
) {
    let view_mode = view_mode_q
        .and_then(|q| q.single().ok().map(|vm| vm.view_mode.clone()))
        .unwrap_or(crate::messages::ViewMode::Camera(
            crate::messages::ViewDirection::Fore,
        ));
    if !state.is_changed() {
        return;
    }

    let in_game = state.get() == &GamePhase::InProgress;

    for (mode, mut vis) in containers.iter_mut() {
        *vis = if in_game {
            match (mode, &view_mode) {
                (RadarContainerMode::Helm, ViewMode::Radar) => Visibility::Visible,
                (RadarContainerMode::Science, ViewMode::ScienceRadar | ViewMode::SensorsRadar) => {
                    Visibility::Visible
                }
                (RadarContainerMode::SystemChart, ViewMode::SystemChart) => Visibility::Visible,
                (RadarContainerMode::Nav, ViewMode::NavigationChart) => Visibility::Visible,
                _ => Visibility::Hidden,
            }
        } else {
            Visibility::Hidden
        };
    }
}
