//! Deliberate GM-local audition through the existing private output owner.
//! The current preview may await decoding, but close/replacement/generation
//! changes retire its request before a prepared result can reach any output.
use super::*;
use crate::sound_cues::SoundDefinition;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: u64,
    at_ms: u64,
    definition: SoundDefinition,
    asset: Option<crate::sound_cues::Asset>,
}
#[derive(Clone)]
pub(super) struct Pending {
    at: Instant,
    definition: SoundDefinition,
    pcm: Option<Result<Arc<Pcm>, String>>,
}
pub(super) fn stop(entry: &mut Entry) {
    entry.preview = None;
    entry.status.preview = "idle";
    for mixer in &entry.mixers {
        mixer.lock().unwrap().stop("audition");
    }
}
pub(super) fn mute(entry: &mut Entry) {
    if entry.preview.as_ref().is_some_and(|preview| {
        entry
            .mix
            .gain(&preview.definition.category, preview.definition.volume)
            == 0.0
    }) {
        stop(entry);
    }
}
pub(super) fn request(entry: &mut Entry, request: Request) {
    stop(entry);
    entry.status.preview_id = request.id;
    if !entry.status.audition {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if now < u128::from(request.at_ms)
        || now - u128::from(request.at_ms) > FRESH.as_millis()
        || crate::sound_cues::validate_definition(&request.definition, request.asset).is_err()
    {
        entry.status.preview = "failed";
        entry.status.detail = "settings.audio.audition_invalid".into();
        return;
    }
    if entry.status.status != "playing"
        || entry
            .mix
            .gain(&request.definition.category, request.definition.volume)
            == 0.0
    {
        return;
    }
    entry.status.preview = "loading";
    entry.preview = Some(Pending {
        at: Instant::now(),
        definition: request.definition,
        pcm: None,
    });
}
#[cfg(test)]
pub(super) fn prepare(hub: &PrivateAudio) {
    prepare_with_revision(hub, None);
}
pub(super) fn prepare_with_revision(hub: &PrivateAudio, source: Option<fn() -> u64>) {
    let requests: Vec<_> = {
        let state = hub.0.lock().unwrap();
        state
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                entry
                    .preview
                    .as_ref()
                    .filter(|preview| preview.pcm.is_none())
                    .map(|preview| {
                        (
                            *key,
                            entry.status.generation,
                            state.asset_revision,
                            preview.clone(),
                        )
                    })
            })
            .collect()
    };
    // Disk/decode work is never under endpoint or callback locks. The current
    // request remains with its endpoint and is checked again at commit.
    for (key, generation, revision, preview) in requests {
        let result = decoder::read(&preview.definition.file).and_then(|pcm| {
            let Some(bearing) = preview
                .definition
                .equivalent
                .as_ref()
                .and_then(|v| v.bearing)
            else {
                return Ok(pcm);
            };
            let pitch = preview
                .definition
                .equivalent
                .as_ref()
                .and_then(|v| v.elevation)
                .unwrap_or(0.0)
                .to_radians();
            let angle = bearing.to_radians();
            let position = [
                crate::simmath::sin(angle) * crate::simmath::cos(pitch),
                crate::simmath::sin(pitch),
                -crate::simmath::cos(angle) * crate::simmath::cos(pitch),
            ];
            let bounded = Pcm {
                rate: pcm.rate,
                channels: pcm.channels,
                samples: pcm
                    .samples
                    .iter()
                    .take(pcm.rate as usize * pcm.channels * 2)
                    .copied()
                    .collect(),
            };
            super::super::hrtf::render(
                &bounded,
                position,
                Some(preview.at + Duration::from_secs(5)),
            )
            .ok_or_else(|| "audition-preparation-expired".into())
        });
        let mut state = hub.0.lock().unwrap();
        if let Some(read) = source {
            refresh_assets(&mut state, read());
        }
        if state.asset_revision != revision {
            continue;
        }
        if let Some(entry) = state
            .entries
            .get_mut(&key)
            .filter(|entry| entry.status.generation == generation)
        {
            if let Some(current) = entry.preview.as_mut().filter(|current| {
                current.at == preview.at && current.definition == preview.definition
            }) {
                current.pcm = Some(result);
            }
        }
    }
}
pub(super) fn commit(entry: &mut Entry) {
    if let Some(mut preview) = entry.preview.take() {
        if entry.status.status != "playing" || preview.at.elapsed() > Duration::from_secs(5) {
            entry.status.preview = "failed";
            return;
        }
        let category = match preview.definition.category.as_str() {
            "music" => "music",
            "ambience" => "ambience",
            "effects" => "effects",
            "alerts" => "alerts",
            "interface" => "interface",
            _ => return,
        };
        match preview.pcm.take() {
            Some(Ok(pcm)) => {
                for mixer in &entry.mixers {
                    mixer.lock().unwrap().cue(
                        "audition",
                        pcm.clone(),
                        category,
                        preview.definition.volume,
                        Some(2.0),
                    );
                }
                entry.status.preview = "playing";
            }
            Some(Err(_)) => {
                entry.status.preview = "failed";
                entry.status.detail = "settings.audio.asset_failed".into();
            }
            None => {
                entry.preview = Some(preview);
            }
        }
    }
    if entry.status.preview == "playing"
        && !entry
            .mixers
            .iter()
            .any(|mixer| mixer.lock().unwrap().active("audition"))
    {
        entry.status.preview = "idle";
    }
}
