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
        }
    }

    pub fn resolved(&self) -> ViewscreenResolution {
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

    pub fn restore_captain_view(&mut self) -> ViewscreenResolution {
        self.cinematic = false;
        self.active = None;
        self.captain_resolution()
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
mod tests {
    use super::*;

    #[test]
    fn defaults_to_captain_camera() {
        let arbiter = ViewscreenArbiter::new();

        assert_eq!(
            arbiter.resolved(),
            ViewscreenResolution {
                owner: crate::ship::system_registry::captain_system_id(),
                mode: ViewMode::Camera(CameraView::default()),
            }
        );
    }

    #[test]
    fn channel_2_radar_request_toggles_back_to_captain_camera() {
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Camera(CameraView::new("camera_aft")),
        });

        let first = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(first.mode, ViewMode::Radar);

        let second = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(second.mode, ViewMode::Camera(CameraView::new("camera_aft")));
    }

    #[test]
    fn latest_valid_request_wins_regardless_of_source() {
        // Latest-wins policy (issue #769): under the old fixed-priority ranking
        // Comms (90) outranked Helm radar (50). Recency alone now decides, so a
        // helm-radar request landing AFTER comms takes the screen.
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        let resolved = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });

        assert_eq!(
            resolved.owner,
            crate::ship::system_registry::helm_radar_system_id()
        );
        assert_eq!(resolved.mode, ViewMode::Radar);
    }

    #[test]
    fn newer_comms_request_wins_over_earlier_radar() {
        // The mirror case: comms lands last and therefore wins. Under the old
        // policy this "passed" only because comms outranked radar; now it holds
        // purely because it is the most recent valid request.
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });

        let resolved = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        assert_eq!(
            resolved.owner,
            crate::ship::system_registry::comms_system_id()
        );
        assert_eq!(resolved.mode, ViewMode::Comms);
    }

    #[test]
    fn competing_systems_last_admitted_wins() {
        // AC4 (competing systems): helm-radar → comms → navigation, applied in
        // that order across three DIFFERENT source systems. The last valid
        // request wins with no regard to which console it came from.
        let mut arbiter = ViewscreenArbiter::new();
        let after_radar = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(after_radar.mode, ViewMode::Radar);

        let after_comms = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });
        assert_eq!(after_comms.mode, ViewMode::Comms);

        let after_nav = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::navigation_system_id(),
            mode: ViewMode::NavigationChart,
        });
        assert_eq!(
            after_nav.owner,
            crate::ship::system_registry::navigation_system_id()
        );
        assert_eq!(after_nav.mode, ViewMode::NavigationChart);
    }

    #[test]
    fn sequence_is_the_authoritative_ordering_token() {
        // AC1 / AC4 (command ordering): each valid request carries a strictly
        // increasing `sequence`. Two requests applied back-to-back (the
        // deterministic same-tick ordering source) resolve to whichever was
        // applied LAST — i.e. the higher sequence.
        let mut arbiter = ViewscreenArbiter::new();

        let first = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        let first_seq = arbiter.active.as_ref().map(|a| a.sequence);
        assert_eq!(first.mode, ViewMode::Radar);

        let second = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::sensors_system_id(),
            mode: ViewMode::SensorsRadar,
        });
        let second_seq = arbiter.active.as_ref().map(|a| a.sequence);
        assert_eq!(second.mode, ViewMode::SensorsRadar);

        // Strictly increasing: the later request outranks the earlier one.
        assert!(second_seq > first_seq);
    }

    #[test]
    fn reconnect_persisted_sequence_prevents_stale_clobber() {
        // AC4 (reconnect): `ViewscreenArbiter` lives on the per-entity
        // `ShipViewMode` component, which is NOT re-initialised on reconnect,
        // so its monotonic `sequence` persists. We model a reconnecting comms
        // console that made an early request, a newer helm-radar request from
        // another console, then the comms console re-issuing after reconnect.
        let mut arbiter = ViewscreenArbiter::new();

        // Comms's original (pre-reconnect) request.
        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        // Another console posts a NEWER request while comms is away.
        let after_radar = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        // The reconnecting console cannot clobber the newer view just by
        // reconnecting — the persisted sequence keeps radar on screen.
        assert_eq!(after_radar.mode, ViewMode::Radar);
        assert_eq!(
            arbiter.resolved().owner,
            crate::ship::system_registry::helm_radar_system_id()
        );

        // After reconnect the comms console issues a genuinely NEWER request,
        // which now correctly wins under latest-wins.
        let after_reconnect = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });
        assert_eq!(
            after_reconnect.owner,
            crate::ship::system_registry::comms_system_id()
        );
        assert_eq!(after_reconnect.mode, ViewMode::Comms);
    }

    #[test]
    fn cinematic_mode_resolved_and_survives_overlay() {
        let mut arbiter = ViewscreenArbiter::new();

        // Activate cinematic.
        let resolved = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Cinematic,
        });
        assert_eq!(resolved.mode, ViewMode::Cinematic);

        // Overlay on top of cinematic.
        let overlay = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(overlay.mode, ViewMode::Radar);

        // Dismiss overlay → back to Cinematic.
        let dismiss = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::helm_radar_system_id(),
            mode: ViewMode::Radar,
        });
        assert_eq!(dismiss.mode, ViewMode::Cinematic);
    }

    #[test]
    fn camera_view_clears_cinematic() {
        let mut arbiter = ViewscreenArbiter::new();

        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Cinematic,
        });
        assert_eq!(arbiter.resolved().mode, ViewMode::Cinematic);

        // Switch to a camera view → cinematic cleared.
        let cam = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Camera(CameraView::new("camera_fore")),
        });
        assert_eq!(cam.mode, ViewMode::Camera(CameraView::new("camera_fore")));
    }

    #[test]
    fn restore_captain_view_clears_cinematic() {
        let mut arbiter = ViewscreenArbiter::new();

        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Cinematic,
        });

        let restored = arbiter.restore_captain_view();
        assert_eq!(restored.mode, ViewMode::Camera(CameraView::default()));
    }

    #[test]
    fn captain_camera_request_resolves_competing_overlay() {
        let mut arbiter = ViewscreenArbiter::new();
        arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::comms_system_id(),
            mode: ViewMode::Comms,
        });

        let resolved = arbiter.apply_channel_2(ViewscreenRequest {
            requester: crate::ship::system_registry::captain_system_id(),
            mode: ViewMode::Camera(CameraView::new("camera_port")),
        });

        assert_eq!(
            resolved.owner,
            crate::ship::system_registry::captain_system_id()
        );
        assert_eq!(
            resolved.mode,
            ViewMode::Camera(CameraView::new("camera_port"))
        );
    }
}
