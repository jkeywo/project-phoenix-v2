//! Simulation integration facade; implementation ownership is split below.
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rhai::AST;

use crate::core::messages::{GamePhase, ServerMessage};
use crate::effect_queue::EffectQueue;
use crate::lobby::{Target, WorldResource};
use crate::objectives::ObjectiveManager;
use crate::server_app::SimOutbox;
#[cfg(test)]
use crate::world::content::TriggerAction;
#[cfg(test)]
use crate::world::content::TriggerState;
use crate::world::content::WorldEvent;
use crate::world::delayed::{partition_delayed_actions, DelayedAction};
use crate::world::dispatch::{
    dispatch_action, ActionCmd, DispatchContext, DispatchResult, LayerView,
    WORLD_MODIFIER_SOURCE_ID,
};
use crate::world::layers::{evaluate_layer_load, LayerLoadOutcome, LayerValidationContext};
use crate::world::load::{load, LoadError, LoadPolicy, LoadRequest, MemoryReader};
use crate::world::script::effects::BufferedEffect;
use crate::world::script::engine::{RuntimeHost, ScriptTrigger};
use crate::world::script::schedule::{CallEffects, PendingCallbacks, SchedClock, TickBudget};

#[path = "../script_invocation.rs"]
mod script_invocation;
pub(crate) use script_invocation::DialogueInvocationError;

use crate::entities::spawner::BehaviourSection;
use crate::entities::spawner::EntityUuid;

#[cfg(test)]
#[path = "../server_tests.rs"]
pub(crate) mod tests;

pub use phoenix_sim_contracts::identity::EntityOriginLayer;
mod state;
pub use state::entity_flag_chain;
pub use state::layer_path_chain;
pub use state::layered_flag_chain;
pub use state::layered_flag_chain_with_paths;
pub(crate) use state::project_layer_views;
pub use state::BridgeWorldSource;
pub use state::MissionClock;
pub use state::ObjectiveInstanceManagerRes;
pub use state::ObjectiveManagerRes;
pub use state::PendingScenarioLoad;
pub use state::PendingWorldLayerChanges;
pub use state::PreCompiledScripts;
pub use state::RawWorldSource;
pub(crate) use state::ScriptActivationGate;
pub use state::ScriptRuntimeParams;
pub use state::WorldContentRuntime;
pub use state::WorldEventBuffer;
pub use state::WorldLayerChange;
pub use state::WorldLayerMap;
pub use state::WorldRuntime;
pub use state::WorldScriptRuntime;
use state::MAX_CHAIN_PASSES;
use state::MAX_TOML_RETRIES;

mod registration;
pub use registration::WorldPlugin;

mod triggers;
pub(crate) use triggers::collect_world_events;
use triggers::handle_region_entered_event;
use triggers::handle_region_exited_event;
pub(crate) use triggers::tick_delayed_actions;
pub(crate) use triggers::tick_script_callbacks;
pub(crate) use triggers::tick_trigger_pipeline;

mod activation;
pub(crate) use activation::anchor_mission_clock;
pub(crate) use activation::arm_mission_clock;
pub(crate) use activation::arm_mission_deadlines;
pub(crate) use activation::arm_mission_workforces;
pub use activation::compile_world_scripts;
pub use activation::freeze_host_preloaded_content;
pub(crate) use activation::init_world_runtime;
pub(crate) use activation::insert_raw_world_source_resource;
pub use activation::insert_world_config_resource;
pub(crate) use activation::load_extra_worlds;
pub(crate) use activation::merge_layer_scripts;
#[cfg(test)]
pub(crate) use activation::merge_script_triggers;
pub use activation::seed_ship_power_counter;
pub use activation::world_activation_blocked;

mod spawning;
pub use spawning::spawn_immediate_entities_internal;
pub(crate) use spawning::spawn_world_entities;

mod publication;
pub(crate) use publication::broadcast_objective_summary;

mod effects;
pub(crate) use effects::apply_dispatch_result;
pub(crate) use effects::apply_script_call;
pub(crate) use effects::build_uuid_to_faction;
pub(crate) use effects::revalidate_ai_targets_after_faction_change;
pub use effects::AiEventReaders;
pub(crate) use effects::EffectQueues;
pub use effects::FactionDispatchParams;
pub(crate) use effects::ScriptCallContext;
pub(crate) use effects::ScriptEventTarget;
pub use effects::ShipModifiersParams;
pub use effects::WorldLayerParams;

mod layers;
use layers::apply_pending_scenario_loads;
use layers::apply_world_layer_changes;
#[cfg(test)]
use layers::remove_layer_triggers;
