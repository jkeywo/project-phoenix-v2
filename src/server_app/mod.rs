use crate::simmath;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::core::messages::{
    DeliveryClass, EntitySnapshot, GamePhase, ServerMessage, ShieldFacingStatus, StationId,
};
use crate::lobby::{LobbyOutbox, OutboundMessage, Sessions, Target, WorldResource};
use crate::weapons::shield::ShieldSystem;

use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::ship::damage::{apply_damage_with_shields, apply_hull_damage, collision_damage};
use crate::weapons::shield::attacker_bearing_relative;
use bevy_rapier3d::prelude::ReadRapierContext;
// Re-export ShipPhysics so `crate::server_app::ShipPhysics` and
// `crate::server_app::ShipPhysics` both resolve.
pub use crate::ship::state::ShipPhysics as ShipPhysicsComponent;

use crate::core::messages::ModifierSlot;
use crate::entities::spawner::{
    AsteroidFieldSection, BehaviourSection, ColliderSection, EntityId, EntityName,
    EntityTagsSection, EntityUuid, FactionComponent, MeshSection, RadarAppearanceSection,
    RegionShapeSection,
};
use crate::modifiers::ShipModifiers;
use crate::ship::impulse::ImpulseState;
use crate::world::server::ObjectiveManagerRes;
use std::collections::{BTreeMap, HashMap};

// â"€â"€ Beam constants â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
pub use crate::console::weapons::{
    weapons_update_broadcaster, ActiveBeam, AsteroidDestroyedVfx, CurrentPhaserMode,
    LastShipAttacker, LastWeaponsUpdate, PhaserCooldown, PhaserRenderConfig,
    TacticalRadarSelection, TorpedoSystemResource,
};

pub use crate::console::repair::server::{repair_state_broadcaster, ShipRepairTeams};

pub use crate::ship::power::{
    power_state_broadcaster, PowerConfigResource, PowerMultiplierResource, ShipPowerSystem,
};

// -- Render half (issue #1195) --
// The render systems live in `crate::server_app_render` but are still
// registered from this module under `SimPluginOptions::render`, in the same
// schedule and order. Bring the system names the registration references into
// scope, and re-export the cache the registration initialises (and the model
// viewer also resolves through `crate::server_app::ProceduralMeshCache`).
pub(crate) use crate::server_app_render::ProceduralMeshCache;
use crate::server_app_render::{face_player_lights, render_spawned_entities, update_mesh_lod};
// Only the model viewer names `crate::server_app::procedural_mesh_material`
// (it builds a ladder's procedural far level through the same factory), so the
// re-export is gated to that feature to stay used under the default build.
#[cfg(feature = "viewer")]
pub(crate) use crate::server_app_render::procedural_mesh_material;

// The command-admission seam lives in its own module (issue #736) so that
// dependants can name it with an explicit `use crate::command_admission::…;`.
// Re-exported here so the existing `crate::server_app::Admission*` call sites
// keep resolving unchanged.
pub use crate::command_admission::{
    admit_system_commands, is_command_authorized, station_for_system, AdmissionPlugin, AdmissionSet,
};

// ── Simulation app assembly (issue #1199) ────────────────────────────────────
//
// The simulation half of `server_app` is split along four seams into the sibling
// modules below (the render half already lives in `crate::server_app_render`,
// #1195). Every item each module defines is re-exported here, so nothing outside
// the crate changes its `crate::server_app::X` import paths — this parent stays a
// thin directory of the seam modules plus the (still-inline, per #1192) tests.
//
//   * `components`   — the ECS vocabulary: components, resources, SystemParam
//                      bundles the simulation defines.
//   * `registration` — `SimPluginOptions`, plugin ordering, and the
//                      `add_simulation_plugins[_with]` assembly.
//   * `broadcast`    — the sim-state snapshot builders.
//   * `broadcast_publish` — the broadcaster factories + publish/HUD systems
//                      downstream of `broadcast`'s snapshots (issue #1241:
//                      split out once the combined file ran 2% over the
//                      ~1,500-line ceiling; neither half calls into the
//                      other's functions).
//   * `world_setup`  — world setup and the game-start spawn systems.
//   * `collision`    — the collision handler (its wide parameter list gathered
//                      into named SystemParam bundles).
//
// Collision handling is a sibling module of `world_setup` rather than folded into
// it: the two together exceed the ~1,500-line ceiling (`spawn_game_start_entities`
// alone is ~1k lines), and collision is the seam #1199 singled out for the
// SystemParam-bundling work.
mod broadcast;
mod broadcast_publish;
mod collision;
mod components;
mod registration;
mod world_setup;

pub use broadcast::*;
pub use broadcast_publish::*;
pub(crate) use collision::*;
pub use components::*;
pub use registration::*;
pub(crate) use world_setup::*;

// â"€â"€ Tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
