//! Authored live sound uses the existing presentation action. The catalog is
//! immutable content; occurrences are frame-local presentation, never a saved
//! field or a source for replay/review.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::sound_cues::{Catalog, SoundDefinition};

pub const PATH: &str = "assets/audio/sound-cues.toml";
pub const BUNDLED: &str = include_str!("../../assets/audio/sound-cues.toml");

/// Captured once through the world's own reader, before its content freeze.
#[derive(Resource, Clone)]
pub struct LiveSoundCatalog(pub Catalog);
impl Default for LiveSoundCatalog {
    fn default() -> Self {
        Self(crate::sound_cues::bundled())
    }
}
impl LiveSoundCatalog {
    /// Preload owns fetch completion. This read cannot start a late fetch and
    /// silently freeze stock content while an authored catalog is still in flight.
    #[cfg(target_arch = "wasm32")]
    pub fn preloaded() -> Result<(Self, String), String> {
        let source = crate::entities::config_cache::resolved_world_source(PATH)
            .ok_or_else(|| "authored-sound-catalog-not-preloaded".to_owned())?;
        Self::capture(Some(source))
    }
    pub fn capture(source: Option<String>) -> Result<(Self, String), String> {
        let source = source.unwrap_or_else(|| BUNDLED.into());
        let catalog: Catalog = toml::from_str(&source).map_err(|error| error.to_string())?;
        catalog.validate_all().map_err(str::to_owned)?;
        Ok((Self(catalog), source))
    }
    pub fn resolve(&self, id: &str) -> Option<SoundDefinition> {
        self.0
            .cues
            .iter()
            .find(|cue| cue.id == id && cue.audience == "viewscreen")
            .cloned()
    }
    pub fn choices(&self) -> Vec<String> {
        self.0
            .cues
            .iter()
            .filter(|cue| cue.audience == "viewscreen")
            .map(|cue| cue.id.clone())
            .collect()
    }
    pub fn room(&self) -> Vec<SoundDefinition> {
        self.0
            .cues
            .iter()
            .filter(|cue| cue.audience == "viewscreen")
            .cloned()
            .collect()
    }
}

/// Only the accepted reducer/scenario route creates this ephemeral request.
#[derive(Message, Clone)]
pub struct LiveSoundRequest {
    pub ship: String,
    pub source: Option<String>,
    pub definition: SoundDefinition,
}

/// Ordinary room channel envelope. IDs only suppress duplicate delivery during
/// this page/process lifetime; there is no occurrence list or restore cursor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSoundCue {
    pub kind: String,
    pub generation: u64,
    pub occurrence: u64,
    pub definition: SoundDefinition,
}

pub fn publish(
    mut requests: MessageReader<LiveSoundRequest>,
    mut serial: Local<u64>,
    lifecycle: Res<crate::server::audio_lifecycle::RoomAudioLifecycle>,
    content: Option<Res<crate::world::server::WorldContentRuntime>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    local: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &crate::ship::state::ShipViewMode,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut output: MessageWriter<crate::console_bridge::AudioCueEvent>,
) {
    for request in requests.read() {
        if !lifecycle.state.running || lifecycle.state.suspended {
            continue;
        }
        let Ok((ship, view)) = local.single() else {
            continue;
        };
        if ship.0 != request.ship {
            continue;
        }
        if let Some(source) = &request.source {
            let Some(content) = content.as_deref() else {
                continue;
            };
            let effective = super::resolved_view_mode(
                content.presentation.get(&ship.0),
                tick.as_deref().map_or(0, |tick| tick.0),
                view,
            );
            if crate::gm_information::suppresses_spatial_cue(
                &content.contact_information,
                &content.contact_overrides,
                &ship.0,
                source,
                &effective,
            ) {
                continue;
            }
        }
        *serial = serial.wrapping_add(1);
        let cue = LiveSoundCue {
            kind: "authored".into(),
            generation: lifecycle.state.generation,
            occurrence: *serial,
            definition: request.definition.clone(),
        };
        if let Ok(json) = crate::core::codec::encode_live_sound_cue(&cue) {
            output.write(crate::console_bridge::AudioCueEvent { json });
        }
    }
}
