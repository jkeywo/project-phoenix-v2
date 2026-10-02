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
