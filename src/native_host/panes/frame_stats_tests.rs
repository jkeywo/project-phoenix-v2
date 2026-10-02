use super::*;

#[test]
fn experiments_parse_by_name_forgiving_case_and_whitespace() {
    assert_eq!(
        PaneExperiments::parse("").unwrap(),
        PaneExperiments::default()
    );
    assert!(PaneExperiments::parse(" , ").unwrap().is_empty());
    let both = PaneExperiments::parse(" raf33, NOVSYNC ").unwrap();
    assert!(both.novsync && both.raf33);
    assert_eq!(both.active(), vec!["novsync", "raf33"]);
    assert_eq!(both.to_string(), "novsync,raf33");
    assert_eq!(PaneExperiments::default().to_string(), "none");
    let all = PaneExperiments::parse(&PaneExperiments::NAMES.join(",")).unwrap();
    assert_eq!(all.active(), PaneExperiments::NAMES.to_vec());
}

#[test]
fn an_unknown_or_retired_experiment_is_refused_and_names_the_known_ones() {
    for retired in ["untracked", "noforce", "novsinc"] {
        let err = PaneExperiments::parse(&format!("raf33,{retired}")).unwrap_err();
        assert!(err.contains(retired), "{err}");
        for name in PaneExperiments::NAMES {
            assert!(err.contains(name), "{err} should name {name}");
        }
    }
}

#[test]
fn each_experiment_maps_to_exactly_its_knob() {
    let none = PaneExperiments::default();
    assert_eq!(none.station_present_mode(), PresentMode::default());
    assert_eq!(none.document_options(), PaneDocumentOptions::default());

    let novsync = PaneExperiments::parse("novsync").unwrap();
    assert_eq!(novsync.station_present_mode(), PresentMode::AutoNoVsync);
    assert_eq!(novsync.document_options(), PaneDocumentOptions::default());

    let raf33 = PaneExperiments::parse("raf33").unwrap();
    assert_eq!(raf33.station_present_mode(), PresentMode::default());
    assert_eq!(raf33.document_options().frame_ms, Some(EXPERIMENT_FRAME_MS));

    let scale2 = PaneExperiments::parse("scale2").unwrap();
    assert_eq!(scale2.station_present_mode(), PresentMode::default());
    assert_eq!(scale2.document_options(), PaneDocumentOptions::default());
    use super::super::{pane_thread::PaneKind, render_geometry::PaneRenderScale};
    assert_eq!(
        scale2.render_scale(PaneKind::Console),
        PaneRenderScale::Half
    );
    for kind in [PaneKind::Lobby, PaneKind::Hud] {
        assert_eq!(scale2.render_scale(kind), PaneRenderScale::Native);
    }
    for experiments in [none, novsync, raf33] {
        assert_eq!(
            experiments.render_scale(PaneKind::Console),
            PaneRenderScale::Native
        );
    }
}

#[test]
fn an_empty_window_has_no_report() {
    let window = FrameStatsWindow::default();
    assert!(window.report(Duration::from_secs(1)).is_none());
}

fn fabricated() -> FrameStatsWindow {
    let mut window = FrameStatsWindow::default();
    for _ in 0..2 {
        window.push_frame(100.0);
        window.push_thread(PaneThreadSample {
            update_ms: 1.0,
            pump_ms: 2.0,
            render_ms: 3.0,
            copy_ms: 4.0,
            publish_ms: 5.0,
            panes: 4,
            copied: 4,
            forced: 2,
            pixels: 1_000_000,
            iteration_ms: 16.0,
            starved: 0,
        });
        window.push_pane(PaneFrameSample {
            drain_ms: 1.0,
            upload_ms: 0.5,
            frames: 4,
            stale: 0,
            uploads: 3,
            uploads_full: 1,
            upload_bytes: 2_000_000,
            deferred: 1,
            lost: 2,
        });
        window.push_fixed(FixedLoopSample { ms: 12.0, ticks: 6 });
        window.push_modified(4);
    }
    window
}

#[test]
fn a_window_reduces_to_per_frame_rates_and_a_residual() {
    let report = fabricated().report(Duration::from_secs(1)).unwrap();
    assert_eq!(report.frames, 2);
    assert_eq!(report.frame.mean, 100.0);
    assert_eq!(report.fps, 10.0);
    assert_eq!(report.panes, 4.0);
    assert_eq!(report.update.mean, 1.0);
    assert_eq!(report.publish.p95, 5.0);
    assert_eq!(report.pane_total_ms, 1.5);
    assert_eq!(report.copied, 4.0);
    assert_eq!(report.forced, 2.0);
    assert_eq!(report.megapixels, 1.0);
    assert_eq!(report.uploads, 3.0);
    assert_eq!(report.uploads_full, 1.0);
    assert_eq!(report.upload_mb, 2.0);
    assert_eq!(report.deferred, 1.0);
    assert_eq!(report.lost, 2.0);
    assert_eq!(report.modified, 4.0);
    assert_eq!(report.ticks, 6.0);
    assert_eq!(report.fixed_ms, 12.0);
    assert_eq!(report.ms_per_tick, Some(2.0));
    assert_eq!(report.residual_ms, 100.0 - 1.5 - 12.0);
}

#[test]
fn the_line_carries_every_number_an_operator_reads_it_for() {
    let line = fabricated()
        .report(Duration::from_secs(1))
        .unwrap()
        .to_string();
    assert!(!line.contains('\n'), "one report is one log line");
    for needle in [
        "2 frames",
        "frame 100.0 ms mean",
        "(10.0 fps)",
        "panes 4.0",
        "update 1.0/1.0",
        "16.0 ms/iteration",
        "copied 4.0/iteration",
        "forced 2.0",
        "1.00 Mpx/iteration",
        "uploads 3.0/frame",
        "(1.0 full)",
        "2.00 MB/frame",
        "deferred 1.0",
        "lost 2.0",
        "image assets changed 4.0/frame",
        "fixed 6.0 ticks/frame",
        "(2.00 ms/tick)",
        "residual 86.5 ms/frame",
    ] {
        assert!(line.contains(needle), "{line:?} should contain {needle:?}");
    }
}

#[test]
fn multiple_thread_samples_do_not_change_the_main_frame_residual() {
    let mut window = fabricated();
    window.push_thread(PaneThreadSample {
        iteration_ms: 90.0,
        copied: 4,
        forced: 2,
        pixels: 1_000_000,
        starved: 2,
        ..Default::default()
    });
    let report = window.report(Duration::from_secs(1)).unwrap();
    assert_eq!(report.iterations_per_s, 3.0);
    assert_eq!(report.iteration.mean, (16.0 + 16.0 + 90.0) / 3.0);
    assert_eq!(report.copied, 4.0);
    assert_eq!(report.lost, 3.0);
    assert_eq!(report.residual_ms, 86.5);
    assert!(report.to_string().contains("3.0 iterations/s"));
}

#[test]
fn a_window_with_no_ticks_reports_no_per_tick_cost() {
    let mut window = FrameStatsWindow::default();
    window.push_frame(16.0);
    window.push_fixed(FixedLoopSample { ms: 0.5, ticks: 0 });
    let report = window.report(Duration::from_millis(16)).unwrap();
    assert_eq!(report.ms_per_tick, None);
    assert!(!report.to_string().contains("ms/tick"));
}

#[test]
fn clearing_forgets_the_frames() {
    let mut window = fabricated();
    assert_eq!(window.frames(), 2);
    window.clear();
    assert_eq!(window.frames(), 0);
    assert!(window.report(Duration::from_secs(1)).is_none());
}
