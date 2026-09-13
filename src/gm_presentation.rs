//! Typed, per-ship presentation shared by GM actions and scenario dispatch.
//! Preferences and playback devices are deliberately outside this state.
use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::messages::{CameraView, ViewMode};
pub mod sound;
#[cfg(test)]
mod sound_tests;

/// Externally tagged: unlike the client ViewMode DTO, this survives postcard
/// journal/snapshot encoding without a deserialize_any requirement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationView {
    Camera(String),
    Radar,
    SensorsRadar,
    NavigationChart,
    Cinematic,
}

impl PresentationView {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "radar" => Self::Radar,
            "sensors_radar" => Self::SensorsRadar,
            "navigation_chart" => Self::NavigationChart,
            "cinematic" => Self::Cinematic,
            name if name.starts_with("camera_")
                && name.len() <= 128
                && !name.chars().any(char::is_control) =>
            {
                Self::Camera(name.into())
            }
            _ => return None,
        })
    }
    pub fn view_mode(&self) -> ViewMode {
        match self {
            Self::Camera(marker) => ViewMode::Camera(CameraView::new(marker)),
            Self::Radar => ViewMode::Radar,
            Self::SensorsRadar => ViewMode::SensorsRadar,
            Self::NavigationChart => ViewMode::NavigationChart,
            Self::Cinematic => ViewMode::Cinematic,
        }
    }
}

/// View and card durations are authored explicitly in simulation ticks. Pausing the
/// mission holds the cue; reconnect/restore shows only its still-live state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationCue {
    ForceView {
        view: PresentationView,
        duration_ticks: u32,
    },
    ReleaseView,
    TitleCard {
        title: String,
        subtitle: String,
        duration_ticks: u32,
    },
    IncomingComms {
        message: String,
        duration_ticks: u32,
    },
    ClearCard,
    Sound {
        id: String,
        source: Option<String>,
    },
}

impl PresentationCue {
    pub fn valid(&self) -> bool {
        let text = |s: &str| s.len() <= 4096 && !s.chars().any(|c| c.is_control() && c != '\n');
        let id = |s: &str| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control);
        match self {
            Self::ForceView {
                view,
                duration_ticks,
            } => {
                *duration_ticks > 0
                    && match view {
                        PresentationView::Camera(marker) => id(marker),
                        _ => true,
                    }
            }
            Self::TitleCard {
                title,
                subtitle,
                duration_ticks,
            } => *duration_ticks > 0 && !title.trim().is_empty() && text(title) && text(subtitle),
            Self::IncomingComms {
                message,
                duration_ticks,
            } => *duration_ticks > 0 && id(message),
            Self::ReleaseView | Self::ClearCard => true,
            Self::Sound { id: cue, source } => {
                !cue.is_empty()
                    && cue.len() <= 512
                    && cue.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
                    && !cue.starts_with('-')
                    && source.as_deref().is_none_or(id)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedView {
    pub view: PresentationView,
    pub until_tick: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationCard {
    Title { title: String, subtitle: String },
    Incoming { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedCard {
    pub card: PresentationCard,
    pub until_tick: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipPresentation {
    pub forced_view: Option<TimedView>,
    pub card: Option<TimedCard>,
}

pub type PresentationState = BTreeMap<String, ShipPresentation>;

/// Current forced mode directly from canonical state, including its original
/// deadline. Fixed-tick presentation producers use this before the frame-side
/// ShipViewMode mirror runs; they must not expose a previous mode's information.
pub fn active_forced_view(state: Option<&ShipPresentation>, tick: u64) -> Option<ViewMode> {
    state?
        .forced_view
        .as_ref()
        .filter(|view| view.until_tick > tick)
        .map(|view| view.view.view_mode())
}

pub fn resolved_view_mode(
    state: Option<&ShipPresentation>,
    tick: u64,
    view: &crate::ship::state::ShipViewMode,
) -> ViewMode {
    active_forced_view(state, tick).unwrap_or_else(|| view.viewscreen.unforced_resolution().mode)
}

/// Same validation and state transition for both entry points. The caller has
/// resolved and checked the receiving player ship; this owner checks message
/// audience at the application boundary, never at UI submission time alone.
fn apply(
    state: &mut PresentationState,
    ship: &str,
    cue: &PresentationCue,
    tick: u64,
    inbox: Option<&crate::comms::server::CommsInboxRes>,
    cameras: &[String],
) -> Result<bool, crate::gm_action::GmActionRefusalReason> {
    use crate::gm_action::GmActionRefusalReason as Refusal;
    if !cue.valid() {
        return Err(Refusal::InvalidAction);
    }
    if matches!(cue, PresentationCue::ForceView { view: PresentationView::Camera(marker), .. } if !cameras.contains(marker))
    {
        return Err(Refusal::InvalidAction);
    }
    if let PresentationCue::IncomingComms { message, .. } = cue {
        if !inbox.is_some_and(|inbox| {
            inbox
                .0
                .messages()
                .iter()
                .any(|m| m.id == *message && !m.is_orphaned && m.is_for_ship(Some(ship)))
        }) {
            return Err(Refusal::InvalidAction);
        }
    }
    let before = state.get(ship).cloned().unwrap_or_default();
    let mut next = before.clone();
    match cue {
        PresentationCue::ForceView {
            view,
            duration_ticks,
        } => {
            next.forced_view = Some(TimedView {
                view: view.clone(),
                until_tick: tick.saturating_add(u64::from(*duration_ticks)),
            })
        }
        PresentationCue::ReleaseView => next.forced_view = None,
        PresentationCue::TitleCard {
            title,
            subtitle,
            duration_ticks,
        } => {
            next.card = Some(TimedCard {
                card: PresentationCard::Title {
                    title: title.clone(),
                    subtitle: subtitle.clone(),
                },
                until_tick: tick.saturating_add(u64::from(*duration_ticks)),
            })
        }
        PresentationCue::IncomingComms {
            message,
            duration_ticks,
        } => {
            next.card = Some(TimedCard {
                card: PresentationCard::Incoming {
                    message: message.clone(),
                },
                until_tick: tick.saturating_add(u64::from(*duration_ticks)),
            })
        }
        PresentationCue::ClearCard => next.card = None,
        // Sound is an occurrence. No persistent view/card state is invented.
        PresentationCue::Sound { .. } => return Ok(true),
    }
    let changed = next != before;
    if next == ShipPresentation::default() {
        state.remove(ship);
    } else {
        state.insert(ship.into(), next);
    }
    Ok(changed)
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct PresentationControl<'w, 's> {
    catalog: Option<Res<'w, sound::LiveSoundCatalog>>,
    entities: Query<'w, 's, &'static crate::entities::spawner::EntityUuid>,
    commands: Commands<'w, 's>,
    pub inbox: Option<Res<'w, crate::comms::server::CommsInboxRes>>,
    markers: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static crate::entities::model_rig::ModelMarkers,
        ),
    >,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationMessageChoice {
    pub message: String,
    pub sender: String,
    pub ship: Option<String>,
}

impl PresentationControl<'_, '_> {
    pub fn cameras(&self) -> BTreeMap<String, Vec<String>> {
        self.markers
            .iter()
            .map(|(ship, markers)| {
                let mut cameras: Vec<_> = markers
                    .marker_names()
                    .filter(|name| name.starts_with("camera_"))
                    .map(str::to_owned)
                    .collect();
                cameras.sort();
                (ship.0.clone(), cameras)
            })
            .collect()
    }
    pub fn apply(
        &mut self,
        state: &mut PresentationState,
        ship: &str,
        cue: &PresentationCue,
        tick: u64,
    ) -> Result<bool, crate::gm_action::GmActionRefusalReason> {
        if !cue.valid() {
            return Err(crate::gm_action::GmActionRefusalReason::InvalidAction);
        }
        if let PresentationCue::Sound { id, source } = cue {
            let definition = self
                .catalog
                .as_deref()
                .and_then(|catalog| catalog.resolve(id))
                .ok_or(crate::gm_action::GmActionRefusalReason::InvalidAction)?;
            if source
                .as_ref()
                .is_some_and(|source| !self.entities.iter().any(|entity| entity.0 == *source))
            {
                return Err(crate::gm_action::GmActionRefusalReason::UnknownEntity);
            }
            let request = sound::LiveSoundRequest {
                ship: ship.into(),
                source: source.clone(),
                definition,
            };
            self.commands.queue(move |world: &mut World| {
                if let Some(mut requests) =
                    world.get_resource_mut::<Messages<sound::LiveSoundRequest>>()
                {
                    requests.write(request);
                }
            });
            return Ok(true);
        }
        let cameras = self.cameras();
        apply(
            state,
            ship,
            cue,
            tick,
            self.inbox.as_deref(),
            cameras.get(ship).map_or(&[], Vec::as_slice),
        )
    }
    pub fn message_choices(&self) -> Vec<PresentationMessageChoice> {
        self.inbox
            .as_deref()
            .map(|inbox| {
                inbox
                    .0
                    .messages()
                    .into_iter()
                    .filter(|m| !m.is_orphaned)
                    .map(|m| PresentationMessageChoice {
                        message: m.id,
                        sender: m.sender_name,
                        ship: m.recipient_ship.map(|s| s.0),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn sound_choices(&self) -> Vec<String> {
        self.catalog
            .as_deref()
            .map_or_else(Vec::new, sound::LiveSoundCatalog::choices)
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct PresentationHud<'w, 's> {
    content: Option<Res<'w, crate::world::server::WorldContentRuntime>>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    local: Query<
        'w,
        's,
        &'static crate::entities::spawner::EntityUuid,
        With<crate::server_app::LocalShip>,
    >,
    inbox: Option<Res<'w, crate::comms::server::CommsInboxRes>>,
}

impl PresentationHud<'_, '_> {
    pub fn card(&self) -> Option<PresentationCardWire> {
        let ship = &self.local.single().ok()?.0;
        card_wire(
            self.content.as_deref()?.presentation.get(ship),
            self.tick.as_deref().map_or(0, |t| t.0),
            ship,
            self.inbox.as_deref(),
        )
    }
}

/// Declarative and Rhai actions resolve authored names in world::dispatch,
/// then arrive here. No extra command protocol or privileged display adapter.
pub fn apply_scenario_command(
    In((ship, cue)): In<(String, PresentationCue)>,
    mut content: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    ships: Query<
        &crate::entities::spawner::EntityUuid,
        (
            With<crate::lockstep::FleetSlotOf>,
            With<crate::server_app::Ship>,
        ),
    >,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut control: PresentationControl,
) {
    let Some(content) = content.as_deref_mut() else {
        return;
    };
    if !ships.iter().any(|id| id.0 == ship) {
        return;
    }
    if let Err(reason) = control.apply(
        &mut content.presentation,
        &ship,
        &cue,
        tick.as_deref().map_or(0, |tick| tick.0),
    ) {
        warn!(?reason, ship, "scenario presentation refused");
    }
}

/// Fixed-step cleanup is canonical. Merely reading or rendering a cue never
/// changes its lifetime or manufactures another occurrence.
pub fn prune(
    mut content: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    ships: Query<&crate::entities::spawner::EntityUuid, With<crate::lockstep::FleetSlotOf>>,
) {
    let Some(content) = content.as_deref_mut() else {
        return;
    };
    let tick = tick.as_deref().map_or(0, |tick| tick.0);
    content.presentation.retain(|ship, state| {
        if !ships.iter().any(|id| id.0 == *ship) {
            return false;
        }
        if state
            .forced_view
            .as_ref()
            .is_some_and(|v| v.until_tick <= tick)
        {
            state.forced_view = None;
        }
        if state.card.as_ref().is_some_and(|c| c.until_tick <= tick) {
            state.card = None;
        }
        *state != ShipPresentation::default()
    });
}

/// Frame-driven so GM controls also work while the simulation is paused.
pub fn sync_views(
    content: Option<Res<crate::world::server::WorldContentRuntime>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    phase: Option<Res<State<crate::core::messages::GamePhase>>>,
    mut ships: Query<(
        &crate::entities::spawner::EntityUuid,
        &mut crate::ship::state::ShipViewMode,
    )>,
) {
    let tick = tick.as_deref().map_or(0, |t| t.0);
    let active = phase
        .as_deref()
        .is_some_and(|p| *p.get() == crate::core::messages::GamePhase::InProgress);
    for (ship, mut view) in &mut ships {
        let forced = active
            .then(|| active_forced_view(content.as_deref()?.presentation.get(&ship.0), tick))
            .flatten();
        if view.viewscreen.forced_view() != forced.as_ref() {
            view.force_view_mode(forced);
        }
    }
}

/// A live card, with only the receiving ship's allowed message text. There is
/// intentionally no history or replay command in this projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationCardWire {
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body_params: BTreeMap<String, String>,
    #[serde(default)]
    pub literal_body: bool,
    #[serde(default)]
    pub literal_title: bool,
}

pub fn card_wire(
    state: Option<&ShipPresentation>,
    tick: u64,
    ship: &str,
    inbox: Option<&crate::comms::server::CommsInboxRes>,
) -> Option<PresentationCardWire> {
    let card = state?.card.as_ref().filter(|c| c.until_tick > tick)?;
    match &card.card {
        PresentationCard::Title { title, subtitle } => Some(PresentationCardWire {
            kind: "title".into(),
            title: title.clone(),
            body: subtitle.clone(),
            body_params: BTreeMap::new(),
            literal_body: true,
            literal_title: true,
        }),
        PresentationCard::Incoming { message } => {
            let inbox = inbox?;
            let msg = inbox
                .0
                .messages()
                .into_iter()
                .find(|m| m.id == *message && !m.is_orphaned && m.is_for_ship(Some(ship)))?;
            Some(PresentationCardWire {
                kind: "incoming".into(),
                title: msg.sender_name.clone(),
                body: msg.body.clone(),
                body_params: msg.body_params.clone(),
                literal_body: msg.literal_body,
                literal_title: false,
            })
        }
    }
}
