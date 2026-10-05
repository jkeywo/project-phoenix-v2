use crate::core::messages::{CameraView, ViewMode};
use serde::{Deserialize, Serialize};
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
