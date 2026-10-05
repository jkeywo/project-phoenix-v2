//! Host projections between simulation contracts and local presentation resources.
use bevy::prelude::*;

pub fn apply_test_visibility(
    visibility: Option<Res<crate::presentation_contracts::TestWindowVisibility>>,
    mut windows: Query<&mut Window>,
) {
    if let Some(visible) = visibility.and_then(|v| v.0) {
        for mut window in &mut windows {
            window.visible = visible;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn mirror_debug_readbacks(
    station_activity: Option<Res<crate::debug::station_activity::StationActivityCapture>>,
    ai_state: Option<Res<crate::debug::ai_state::AiDoctrineCapture>>,
    scenario: Option<Res<crate::debug::scenario::ScenarioStateCapture>>,
    console_latency: Option<Res<crate::debug::console_latency::ConsoleLatencyCapture>>,
    modifiers: Option<Res<crate::debug::modifiers::ModifierDebugCapture>>,
    damage: Option<Res<crate::debug::damage::DamageDebugCapture>>,
    entities: Option<Res<crate::debug::entities::EntityBehaviorCapture>>,
    inspector: Option<Res<crate::debug::inspector::EntityInspectorCapture>>,
    flags: Option<Res<crate::debug_overlay::LastReportedDebugState>>,
) {
    if let Some(capture) = station_activity.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_station_activity_string(json.clone());
        }
    }
    if let Some(capture) = ai_state.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_ai_doctrine_string(json.clone());
        }
    }
    if let Some(capture) = scenario.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_scenario_state_string(json.clone());
        }
    }
    if let Some(capture) = console_latency.filter(|c| c.is_changed()) {
        crate::server::bridge::set_console_latency_string(capture.0.clone().unwrap_or_default());
    }
    if let Some(capture) = modifiers.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_debug_state_string(json.clone());
        }
    }
    if let Some(capture) = damage.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_damage_log_string(json.clone());
        }
    }
    if let Some(capture) = entities.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_entity_debug_string(json.clone());
        }
    }
    if let Some(capture) = inspector.filter(|c| c.is_changed()) {
        if let Some(json) = &capture.0 {
            crate::server::bridge::set_entity_inspector_string(json.clone());
        }
    }
    if let Some(flags) = flags.filter(|c| c.is_changed()) {
        if let Some((flags, _, _)) = &flags.0 {
            crate::server::bridge::set_debug_flags_string(
                crate::core::codec::encode_debug_surfaces(flags),
            );
        }
    }
}

pub fn sample_endpoint_input(
    #[cfg(not(target_arch = "wasm32"))] mut native_preference: Local<Option<bool>>,
    mut input: Option<ResMut<crate::server::viewscreen_border::ViewscreenEndpointInput>>,
) {
    let Some(ref mut input) = input else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    let reduced = crate::server::bridge::reduced_motion_requested();
    #[cfg(not(target_arch = "wasm32"))]
    let reduced =
        *native_preference.get_or_insert_with(crate::server::bridge::native_reduced_motion);
    let (shake, flash, decorative) = crate::server::bridge::published_effect_intensities();
    let next = crate::server::viewscreen_border::ViewscreenEndpointInput {
        reduced_motion: reduced,
        shake,
        flash,
        decorative,
    };
    if **input != next {
        **input = next;
    }
}
