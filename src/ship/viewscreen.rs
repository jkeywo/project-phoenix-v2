use crate::core::messages::{CameraView, SystemId, ViewMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewscreenRequest {
    pub requester: SystemId,
    pub mode: ViewMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewscreenResolution {
    pub owner: SystemId,
    pub mode: ViewMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveView {
    requester: SystemId,
    mode: ViewMode,
    sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewscreenArbiter {
    captain_view: CameraView,
    cinematic: bool,
    active: Option<ActiveView>,
    sequence: u64,
    forced: Option<ViewMode>,
}

impl Default for ViewscreenArbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewscreenArbiter {
    pub fn new() -> Self {
        Self {
            captain_view: CameraView::default(),
            cinematic: false,
            active: None,
            sequence: 0,
            forced: None,
        }
    }

    pub fn resolved(&self) -> ViewscreenResolution {
        if let Some(mode) = &self.forced {
            return ViewscreenResolution {
                owner: source_system_for_view_mode(mode),
                mode: mode.clone(),
            };
        }
        self.unforced_resolution()
    }

    /// Latest ordinary crew choice beneath any temporary presentation cue.
    pub fn unforced_resolution(&self) -> ViewscreenResolution {
        if let Some(active) = &self.active {
            ViewscreenResolution {
                owner: active.requester.clone(),
                mode: active.mode.clone(),
            }
        } else if self.cinematic {
            ViewscreenResolution {
                owner: crate::ship::system_registry::captain_system_id(),
                mode: ViewMode::Cinematic,
            }
        } else {
            self.captain_resolution()
        }
    }

    /// Apply a channel-2 viewscreen request under the
    /// latest-valid-command-wins policy (issue #769).
    ///
    /// Every request bumps the monotonic `sequence` — the authoritative
    /// recency ordering carried by each active view. A valid overlay request
    /// ALWAYS replaces the currently active view regardless of the requesting
    /// system: there is no source ranking. The single exception is the
    /// toggle-off / captain-camera-return semantics, which are orthogonal to
    /// source arbitration and preserved here:
    ///   * `Camera` reclaims the shared screen for the captain camera.
    ///   * `Cinematic` reclaims the screen for cinematic presentation.
    ///   * repeating the *exact* active overlay (same requester + mode)
    ///     dismisses it, returning to the captain camera.
    ///
    /// Both `SetView` (captain console) and `ShowOnScreen` (comms console)
    /// route here through `ShipViewMode`, so they obey the identical policy.
    pub fn apply_channel_2(&mut self, request: ViewscreenRequest) -> ViewscreenResolution {
        self.sequence += 1;
        match request.mode {
            ViewMode::Camera(view) => {
                self.cinematic = false;
                self.captain_view = view;
                self.active = None;
            }
            ViewMode::Cinematic => {
                self.cinematic = true;
                self.active = None;
            }
            mode => {
                let should_clear = self.active.as_ref().is_some_and(|active| {
                    active.requester == request.requester && active.mode == mode
                });
                if should_clear {
                    // Toggle-off: the active overlay's owner re-requested the
                    // same mode → dismiss back to the captain camera.
                    self.active = None;
                } else {
                    // Latest valid request wins, ordered by `sequence`.
                    self.active = Some(ActiveView {
                        requester: request.requester,
                        mode,
                        sequence: self.sequence,
                    });
                }
            }
        }
        self.resolved()
    }

    pub fn forced_view(&self) -> Option<&ViewMode> {
        self.forced.as_ref()
    }

    pub fn force(&mut self, mode: Option<ViewMode>) -> ViewscreenResolution {
        self.forced = mode;
        self.resolved()
    }

    pub fn restore_captain_view(&mut self) -> ViewscreenResolution {
        self.cinematic = false;
        self.active = None;
        self.resolved()
    }

    pub fn captain_view(&self) -> CameraView {
        self.captain_view.clone()
    }

    fn captain_resolution(&self) -> ViewscreenResolution {
        ViewscreenResolution {
            owner: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Camera(self.captain_view.clone()),
        }
    }
}

pub fn source_system_for_view_mode(mode: &ViewMode) -> SystemId {
    match mode {
        ViewMode::Camera(_) => crate::ship::system_registry::captain_system_id(),
        ViewMode::Radar => crate::ship::system_registry::helm_radar_system_id(),
        ViewMode::ScienceRadar | ViewMode::SensorsRadar => {
            crate::ship::system_registry::sensors_system_id()
        }
        ViewMode::SystemChart | ViewMode::NavigationChart => {
            crate::ship::system_registry::navigation_system_id()
        }
        ViewMode::Comms => crate::ship::system_registry::comms_system_id(),
        ViewMode::Cinematic => crate::ship::system_registry::captain_system_id(),
    }
}

#[cfg(test)]
#[path = "viewscreen_tests.rs"]
mod tests;
