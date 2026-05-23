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
//! | `SystemChart`        | `[sensors_console.long_range_radar]`    | WorldFixed   |
//! | `NavigationChart`    | `[navigation_console.system_chart]`     | WorldFixed + WorldCentredRadar + AutoScale |
//!
//! `sync_server_radar_bridge` mirrors the entity bridge pattern from
//! `console/helm/client.rs`: it reads `WorldResource.0.entities` each frame and
//! reconciles `OnRadar + RadarAppearance + Transform + GlobalTransform` ECS
//! entities, using `tags_to_radar_layer`, `layer_to_icon`,
//! `default_layer_colour`, and `region_shape_from_snapshot` from `gui/radar.rs`
//! — the same functions the client consoles use.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::entity_tags::EntityTag;
use crate::gui::{
    AutoScaleRadar, GenericRadar, OnRadar, OrientationMode, RadarAppearance, RadarCenter,
    RadarClipMode, RadarFilter, RadarIcon, RadarLayer, WorldCentredRadar, default_layer_colour,
    layer_to_icon, region_shape_from_snapshot, tags_to_radar_layer,
};
use crate::gui::radar::GuiRadarPlugin;
use crate::lobby::WorldResource;
use crate::messages::{GamePhase, ViewMode};
use crate::ship_state::ShipState;

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

// ── Resource ──────────────────────────────────────────────────────────────────

/// Tracks the Bevy entities that represent radar blips on the server viewscreen.
///
/// - `center`: the single `RadarCenter + OnRadar(PlayerShip)` entity for the
///   player ship.
/// - `blips`: UUID → Bevy entity for every world entity currently on the radar.
#[derive(Resource, Default)]
pub struct ServerRadarEntityMap {
    center: Option<Entity>,
    blips: HashMap<String, Entity>,
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

        app.init_resource::<ServerRadarEntityMap>()
            .add_systems(Startup, spawn_viewscreen_radar_widgets)
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

/// Map a single `EntityTag` to its corresponding `RadarLayer`.
fn entity_tag_to_radar_layer(tag: EntityTag) -> Option<RadarLayer> {
    match tag {
        EntityTag::Ship => Some(RadarLayer::Ship),
        EntityTag::Asteroid => Some(RadarLayer::Asteroid),
        EntityTag::AsteroidField => Some(RadarLayer::AsteroidField),
        EntityTag::Station => Some(RadarLayer::Station),
        EntityTag::Planet => Some(RadarLayer::Planet),
        EntityTag::Star => Some(RadarLayer::Star),
        EntityTag::Region => Some(RadarLayer::Region),
    }
}

/// Build a `RadarFilter` from a TOML `shows` list.
///
/// `PlayerShip` is always included so the ship blip is visible on every widget.
fn radar_filter_from_shows(shows: &[EntityTag]) -> RadarFilter {
    let mut set = HashSet::new();
    set.insert(RadarLayer::PlayerShip);
    for &tag in shows {
        if let Some(layer) = entity_tag_to_radar_layer(tag) {
            set.insert(layer);
        }
    }
    RadarFilter(set)
}

/// Spawn a full-screen absolute container node that holds a radar widget as
/// its only child.  The container starts hidden; `toggle_viewscreen_radar_widgets`
/// shows/hides it based on the current view mode.
fn spawn_radar_container(
    commands: &mut Commands,
    mode: RadarContainerMode,
    widget_entity: Entity,
) {
    commands
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
        .add_child(widget_entity);
}

// ── Startup system ────────────────────────────────────────────────────────────

fn spawn_viewscreen_radar_widgets(mut commands: Commands) {
    // Read ship config from the thread-local config cache (populated by JS
    // before wasm_init on the server, same pattern as lobby/server.rs).
    let config_cache = crate::config_cache::get_config_cache();
    let ship_config = config_cache.get("assets/entities/player_ship.toml");

    // ── Helm radar ─────────────────────────────────────────────────────────
    let helm_radar = ship_config
        .and_then(|c| c.helm_console.as_ref())
        .and_then(|hc| hc.radar.as_ref())
        .cloned()
        .unwrap_or_default();

    let helm_widget = GenericRadar::spawn(
        &mut commands,
        helm_radar.range,
        OrientationMode::ShipRelative,
        radar_filter_from_shows(&helm_radar.shows),
        None,
        None,
        RadarClipMode::Circle,
        1.0,
        1.0,
    );
    spawn_radar_container(&mut commands, RadarContainerMode::Helm, helm_widget);

    // ── Science / Sensors radar ────────────────────────────────────────────
    let science_radar = ship_config
        .and_then(|c| c.sensors_console.as_ref())
        .map(|sc| sc.long_range_radar.clone())
        .unwrap_or_default();

    let science_widget = GenericRadar::spawn(
        &mut commands,
        science_radar.range,
        OrientationMode::WorldFixed,
        radar_filter_from_shows(&science_radar.shows),
        None,
        None,
        RadarClipMode::Circle,
        1.0,
        1.0,
    );
    spawn_radar_container(&mut commands, RadarContainerMode::Science, science_widget);

    // ── System chart ────────────────────────────────────────────────────────
    // Same range as the sensors long-range radar; filter restricted to large
    // fixed bodies (star, planet, asteroid fields, regions) + player ship.
    let mut system_chart_set = HashSet::new();
    system_chart_set.insert(RadarLayer::PlayerShip);
    system_chart_set.insert(RadarLayer::Star);
    system_chart_set.insert(RadarLayer::Planet);
    system_chart_set.insert(RadarLayer::AsteroidField);
    system_chart_set.insert(RadarLayer::Region);

    let system_chart_widget = GenericRadar::spawn(
        &mut commands,
        science_radar.range,
        OrientationMode::WorldFixed,
        RadarFilter(system_chart_set),
        None,
        None,
        RadarClipMode::Circle,
        1.0,
        1.0,
    );
    spawn_radar_container(
        &mut commands,
        RadarContainerMode::SystemChart,
        system_chart_widget,
    );

    // ── Navigation chart ────────────────────────────────────────────────────
    // World-centred (star at origin), auto-scaling so all chart entities fit,
    // sourced from [navigation_console.system_chart] in the TOML.
    let nav_chart = ship_config
        .and_then(|c| c.navigation_console.as_ref())
        .map(|nc| nc.system_chart.clone())
        .unwrap_or_default();

    let nav_widget = GenericRadar::spawn(
        &mut commands,
        nav_chart.range,
        OrientationMode::WorldFixed,
        radar_filter_from_shows(&nav_chart.shows),
        None,
        None,
        RadarClipMode::Circle,
        1.0,
        1.0,
    );
    commands
        .entity(nav_widget)
        .insert((WorldCentredRadar, AutoScaleRadar { margin: 1.1, min_range: 50.0 }));
    spawn_radar_container(&mut commands, RadarContainerMode::Nav, nav_widget);
}

// ── Update: bridge WorldResource → radar entities ─────────────────────────────

/// Reconciles `WorldResource.0.entities` into ECS radar blip entities.
///
/// Mirrors `bridge_client_sim_to_radar_entities` in `console/helm/client.rs`:
/// same `tags_to_radar_layer`, `layer_to_icon`, `default_layer_colour`, and
/// `region_shape_from_snapshot` calls, same spawn/update/despawn logic.
fn sync_server_radar_bridge(
    mut commands: Commands,
    world: Option<Res<WorldResource>>,
    ship: Res<ShipState>,
    mut entity_map: ResMut<ServerRadarEntityMap>,
) {
    // ── Player-ship RadarCenter entity ────────────────────────────────────
    let ship_appearance = RadarAppearance {
        icon: RadarIcon::Ship,
        world_size: 6.0,
        color: Color::srgb(0.95, 0.95, 1.0),
        region_colour: None,
        region_shape: None,
    };
    let ship_transform = Transform::from_xyz(ship.x, 0.0, ship.z)
        .with_rotation(Quat::from_rotation_y(ship.yaw));
    let ship_global = GlobalTransform::from(ship_transform);

    match entity_map.center {
        Some(e) => {
            commands.entity(e).insert((
                RadarCenter { world_x: ship.x, world_z: ship.z, yaw: ship.yaw },
                OnRadar(RadarLayer::PlayerShip),
                ship_appearance,
                ship_transform,
                ship_global,
            ));
        }
        None => {
            let e = commands
                .spawn((
                    RadarCenter { world_x: ship.x, world_z: ship.z, yaw: ship.yaw },
                    OnRadar(RadarLayer::PlayerShip),
                    ship_appearance,
                    ship_transform,
                    ship_global,
                ))
                .id();
            entity_map.center = Some(e);
        }
    }

    // ── World entity blips ────────────────────────────────────────────────
    let Some(world) = world else { return };

    let mut seen = HashSet::<String>::new();

    for snapshot in &world.0.entities {
        let uuid = &snapshot.uuid;
        if !seen.insert(uuid.clone()) {
            continue; // deduplicate
        }

        let Some(layer) = tags_to_radar_layer(&snapshot.tags) else {
            continue; // entity type not recognised for radar
        };

        let entity_yaw = snapshot.yaw.unwrap_or(0.0);
        let colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));

        let appearance = if layer == RadarLayer::Region || layer == RadarLayer::AsteroidField {
            // Region / field: rendered as a shape (torus, sphere, box).
            let region_colour = colour.unwrap_or_else(|| default_layer_colour(layer));
            let region_shape = region_shape_from_snapshot(snapshot);
            let world_size = snapshot
                .radar_world_size
                .or_else(|| Some(snapshot.radius_or_zero()))
                .filter(|&s| s > 0.0)
                .unwrap_or(4.0);
            RadarAppearance {
                icon: layer_to_icon(layer),
                world_size,
                color: region_colour,
                region_colour: Some(region_colour),
                region_shape,
            }
        } else {
            // Point entity: rendered as an icon blip.
            let world_size = snapshot
                .radar_world_size
                .or_else(|| Some(snapshot.radius_or_zero()))
                .filter(|&s| s > 0.0)
                .unwrap_or(4.0);
            RadarAppearance {
                icon: layer_to_icon(layer),
                world_size,
                color: colour.unwrap_or_else(|| default_layer_colour(layer)),
                region_colour: None,
                region_shape: None,
            }
        };

        let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
            .with_rotation(Quat::from_rotation_y(entity_yaw));

        if let Some(&existing) = entity_map.blips.get(uuid) {
            commands
                .entity(existing)
                .insert((OnRadar(layer), appearance, t, GlobalTransform::from(t)));
        } else {
            let blip = commands
                .spawn((OnRadar(layer), appearance, t, GlobalTransform::from(t)))
                .id();
            entity_map.blips.insert(uuid.clone(), blip);
        }
    }

    // Despawn blips for entities that have left the world.
    entity_map.blips.retain(|uuid, &mut entity| {
        if seen.contains(uuid) {
            true
        } else {
            commands.entity(entity).despawn();
            false
        }
    });
}

// ── Update: toggle radar container visibility ─────────────────────────────────

fn toggle_viewscreen_radar_widgets(
    ship: Option<Res<ShipState>>,
    state: Res<State<GamePhase>>,
    mut containers: Query<(&RadarContainerMode, &mut Visibility)>,
) {
    let Some(ship) = ship else { return };
    if !ship.is_changed() && !state.is_changed() {
        return;
    }

    let in_game = state.get() == &GamePhase::InProgress;

    for (mode, mut vis) in containers.iter_mut() {
        *vis = if in_game {
            match (mode, &ship.view_mode) {
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
