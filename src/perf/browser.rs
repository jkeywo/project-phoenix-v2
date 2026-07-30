//! Browser-host measurement (issue #868).
//!
//! Boot and preload are timed at the bridge's own entry points; frame time is
//! sampled once per `App::update`, which under `requestAnimationFrame` is once
//! per presented frame. `performance.now()` is the clock, because
//! `std::time::Instant` panics on this target.
//!
//! # What a CI capture does and does not mean
//!
//! `wasm_init` detects WebDriver and skips the render, audio, glTF and gizmo
//! plugins, because a headless CI runner has no GPU for wgpu to initialise.
//! That is load-bearing for reading these numbers: under automation
//! `browser.frame` measures the ECS schedule per animation frame and **not**
//! rendering. The runtime in the capture's provenance says which world it came
//! from — `wasm-automation` or `wasm-browser` — so the two never silently
//! compare against each other.
//!
//! The state is a thread-local rather than a Bevy resource for the same reason
//! the headless sampler is owned by its caller: the boot timer has to start
//! before an `App` exists, and measurement must not become something a system
//! can read.

use std::cell::RefCell;

use vellum_perf::{Recorder, Unit};
use wasm_bindgen::prelude::*;

/// Time from `wasm_init` entry to the app being handed to the frame loop.
pub const BOOT_METRIC: &str = "browser.boot";
/// Time spent in the pre-app asset/config preload.
pub const PRELOAD_METRIC: &str = "browser.preload";
/// Wall time of one `App::update`, sampled per animation frame.
pub const FRAME_METRIC: &str = "browser.frame";

/// Runtime recorded when the page is driven by WebDriver, where the render
/// stack is deliberately absent.
pub const RUNTIME_AUTOMATION: &str = "wasm-automation";
/// Runtime recorded for an ordinary browser session, render stack and all.
pub const RUNTIME_BROWSER: &str = "wasm-browser";

thread_local! {
    static PERF: RefCell<BrowserPerf> = RefCell::new(BrowserPerf::default());
}

#[derive(Default)]
struct BrowserPerf {
    recorder: Recorder,
    boot_started: Option<f64>,
    preload_started: Option<f64>,
    preload_done: bool,
    frame_started: Option<f64>,
    automation: bool,
}

/// `performance.now()` in milliseconds, or `None` when there is no window —
/// which is not a case worth panicking over inside a measurement path.
fn now() -> Option<f64> {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
}

/// Called at the top of `wasm_init`, before any plugin is added.
pub fn boot_begin(automation: bool) {
    PERF.with(|perf| {
        let mut perf = perf.borrow_mut();
        perf.automation = automation;
        perf.boot_started = now();
    });
}

/// Called once the app is built and about to be handed to `App::run`.
pub fn boot_end() {
    PERF.with(|perf| {
        let mut perf = perf.borrow_mut();
        let (Some(started), Some(ended)) = (perf.boot_started, now()) else {
            return;
        };
        perf.recorder
            .sample(BOOT_METRIC, Unit::Millis, ended - started);
        perf.boot_started = None;
    });
}

/// Start the preload clock on the first config the page loads.
///
/// Idempotent because JS drives the preload one config at a time and there is
/// no single entry point to hang a start on — the first call is the start.
pub fn preload_begin_once() {
    PERF.with(|perf| {
        let mut perf = perf.borrow_mut();
        if perf.preload_started.is_none() && !perf.preload_done {
            perf.preload_started = now();
        }
    });
}

/// Stop it the first time the page observes the preload complete. Later calls
/// do nothing: the page polls this, and re-sampling would turn one preload
/// into a series measuring how often JS asked.
pub fn preload_end_once() {
    PERF.with(|perf| {
        let mut perf = perf.borrow_mut();
        if perf.preload_done {
            return;
        }
        let (Some(started), Some(ended)) = (perf.preload_started, now()) else {
            return;
        };
        perf.recorder
            .sample(PRELOAD_METRIC, Unit::Millis, ended - started);
        perf.preload_started = None;
        perf.preload_done = true;
    });
}

/// Called at the start of each animation frame's update.
pub fn frame_begin() {
    PERF.with(|perf| perf.borrow_mut().frame_started = now());
}

/// Called at the end of each animation frame's update.
pub fn frame_end() {
    PERF.with(|perf| {
        let mut perf = perf.borrow_mut();
        let (Some(started), Some(ended)) = (perf.frame_started, now()) else {
            return;
        };
        perf.recorder
            .sample(FRAME_METRIC, Unit::Millis, ended - started);
        perf.frame_started = None;
    });
}

/// The runtime this page is measuring.
pub fn runtime(automation: bool) -> &'static str {
    if automation {
        RUNTIME_AUTOMATION
    } else {
        RUNTIME_BROWSER
    }
}

/// The capture so far, as JSON, for a harness to pull out of the page.
///
/// Non-destructive: the page keeps sampling afterwards, so a test may take a
/// reading at several points in a session. Returns an empty string when
/// nothing has been sampled, which a harness should treat as "not ready yet"
/// rather than as a zeroed measurement.
#[wasm_bindgen]
pub fn wasm_perf_capture(scenario: &str) -> String {
    PERF.with(|perf| {
        let perf = perf.borrow();
        if perf.recorder.is_empty() {
            return String::new();
        }
        let profile = crate::perf::profile(runtime(perf.automation));
        perf.recorder.clone().finish(scenario, profile).to_json()
    })
}
