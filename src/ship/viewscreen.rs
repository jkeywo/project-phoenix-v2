use crate::messages::{CameraView, SystemId, ViewMode};

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
    active: Option<ActiveView>,
    sequence: u64,
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
            active: None,
            sequence: 0,
        }
    }

    pub fn resolved(&self) -> ViewscreenResolution {
        if let Some(active) = &self.active {
            ViewscreenResolution {
                owner: active.requester.clone(),
                mode: active.mode.clone(),
            }
        } else {
            self.captain_resolution()
        }
    }

    pub fn request_channel_2(&mut self, request: ViewscreenRequest) -> ViewscreenResolution {
        self.sequence += 1;
        match request.mode {
            ViewMode::Camera(view) => {
                self.captain_view = view;
                self.active = None;
            }
            mode => {
                let request_priority = view_priority(&request.requester, &mode);
                let should_clear = self.active.as_ref().is_some_and(|active| {
                    active.requester == request.requester && active.mode == mode
                });
                if should_clear {
                    self.active = None;
                } else {
                    let should_replace = self.active.as_ref().is_none_or(|active| {
                        let active_priority = view_priority(&active.requester, &active.mode);
                        request_priority > active_priority
                            || (request_priority == active_priority
                                && self.sequence > active.sequence)
                    });
                    if should_replace {
                        self.active = Some(ActiveView {
                            requester: request.requester,
                            mode,
                            sequence: self.sequence,
                        });
                    }
                }
            }
        }
        self.resolved()
    }

    pub fn show_channel_2(&mut self, request: ViewscreenRequest) -> ViewscreenResolution {
        self.sequence += 1;
        match request.mode {
            ViewMode::Camera(view) => {
                self.captain_view = view;
                self.active = None;
            }
            mode => {
                self.active = Some(ActiveView {
                    requester: request.requester,
                    mode,
                    sequence: self.sequence,
                });
            }
        }
        self.resolved()
    }

    pub fn restore_captain_view(&mut self) -> ViewscreenResolution {
        self.active = None;
        self.captain_resolution()
    }

    pub fn captain_view(&self) -> CameraView {
        self.captain_view.clone()
    }

    fn captain_resolution(&self) -> ViewscreenResolution {
        ViewscreenResolution {
            owner: crate::system_registry::captain_system_id(),
            mode: ViewMode::Camera(self.captain_view.clone()),
        }
    }
}

pub fn source_system_for_view_mode(mode: &ViewMode) -> SystemId {
    match mode {
        ViewMode::Camera(_) => crate::system_registry::captain_system_id(),
        ViewMode::Radar => crate::system_registry::helm_system_id(),
        ViewMode::ScienceRadar | ViewMode::SensorsRadar => {
            crate::system_registry::sensors_system_id()
        }
        ViewMode::SystemChart | ViewMode::NavigationChart => {
            crate::system_registry::navigation_system_id()
        }
        ViewMode::Comms => crate::system_registry::comms_system_id(),
    }
}

fn view_priority(source: &SystemId, mode: &ViewMode) -> u8 {
    if source == &crate::system_registry::captain_system_id() {
        return 100;
    }
    match mode {
        ViewMode::Comms => 90,
        ViewMode::NavigationChart => 70,
        ViewMode::ScienceRadar | ViewMode::SensorsRadar | ViewMode::SystemChart => 60,
        ViewMode::Radar => 50,
        ViewMode::Camera(_) => 100,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_captain_camera() {
        let arbiter = ViewscreenArbiter::new();

        assert_eq!(
            arbiter.resolved(),
            ViewscreenResolution {
                owner: crate::system_registry::captain_system_id(),
                mode: ViewMode::Camera(CameraView::default()),
            }
        );
    }

    #[test]
    fn channel_2_radar_request_toggles_back_to_captain_camera() {
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::captain_system_id(),
            mode: ViewMode::Camera(CameraView::new("camera_aft")),
        });

        let first = arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::helm_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(first.mode, ViewMode::Radar);

        let second = arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::helm_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(second.mode, ViewMode::Camera(CameraView::new("camera_aft")));
    }

    #[test]
    fn higher_priority_comms_overrides_helm_radar() {
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::helm_system_id(),
            mode: ViewMode::Radar,
        });

        let resolved = arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        assert_eq!(resolved.owner, crate::system_registry::comms_system_id());
        assert_eq!(resolved.mode, ViewMode::Comms);
    }

    #[test]
    fn lower_priority_request_does_not_steal_active_view() {
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        let resolved = arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::helm_system_id(),
            mode: ViewMode::Radar,
        });

        assert_eq!(resolved.owner, crate::system_registry::comms_system_id());
        assert_eq!(resolved.mode, ViewMode::Comms);
    }

    #[test]
    fn captain_camera_request_resolves_competing_overlay() {
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        let resolved = arbiter.request_channel_2(ViewscreenRequest {
            requester: crate::system_registry::captain_system_id(),
            mode: ViewMode::Camera(CameraView::new("camera_port")),
        });

        assert_eq!(resolved.owner, crate::system_registry::captain_system_id());
        assert_eq!(resolved.mode, ViewMode::Camera(CameraView::new("camera_port")));
    }
}
