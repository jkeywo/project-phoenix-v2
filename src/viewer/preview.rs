//! The Workshop's disposable model preview: the ordinary viewer, bound to one
//! captured draft.
//!
//! This is the SAME `ViewerPlugin` the standalone viewer runs — the same
//! subject dispatch, the same LOD ladder, the same lighting, the same measured
//! statistics — so a previewed model is the model the game draws rather than a
//! Workshop-only approximation. What differs is where its bytes come from: the
//! standalone viewer fetches them from a dev server, and a preview may read
//! nothing but the draft it was handed. `register_snapshot` binds this App to
//! exactly those bytes, and its reader answers `NotFound` for anything else, so
//! an uncaptured project file is not a slower path — it is absent.
#[cfg(target_arch = "wasm32")]
use super::{ViewerArgs, ViewerPlugin};
#[cfg(target_arch = "wasm32")]
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Boot one preview over captured bytes. The frame is disposable and boots
/// once; a second call is a caller error, not a reload.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_workshop_preview_init(selection: String, source: JsValue) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let fail = |message: String| JsValue::from_str(&message);
    let selection = crate::core::codec::decode_workshop_preview_selection(selection.as_bytes())
        .map_err(|e| fail(format!("Invalid preview selection: {e}")))?;
    let subject = selection
        .subject()
        .ok_or_else(|| fail("A preview shows exactly one model or one entity".to_string()))?
        .to_string();
    let captured = crate::workshop::captured_source::capture(source, "Workshop preview", "preview")
        .map_err(fail)?;
    // The subject itself must be IN the draft. Without this the App would boot
    // and the asset server would report a missing file frames later, which
    // reads as an empty picture rather than as the refusal it is.
    if !captured.assets.contains_key(&subject) {
        return Err(fail(format!(
            "The preview subject is not in the captured draft: {subject}"
        )));
    }
    // An entity subject is read as SOURCE by the same include/compose path the
    // game uses, so it has to have been captured as text rather than as opaque
    // bytes. A GLB is the other way round and is read through the asset server.
    if let Some(entity) = selection.entity.as_deref() {
        if !captured.text.contains_key(entity) {
            return Err(fail(format!(
                "The preview entity was not captured as source: {entity}"
            )));
        }
    }
    let args = ViewerArgs {
        model: selection.model,
        variant: selection.variant.filter(|v| !v.is_empty()),
        entity: selection.entity,
        gizmos: selection.gizmos,
    };
    let mut app = App::new();
    // Before `DefaultPlugins`, which is what adds `AssetPlugin`: an asset
    // source registered after it is registered against nothing.
    crate::entities::pack_assets::register_snapshot(&mut app, std::sync::Arc::new(captured.assets));
    app.add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
        primary_window: Some(bevy::window::Window {
            canvas: Some("#canvas".into()),
            fit_canvas_to_parent: true,
            ..default()
        }),
        ..default()
    }))
    .add_plugins(ViewerPlugin { args })
    .run();
    Ok(())
}
