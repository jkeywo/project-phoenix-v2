# Native surface attribution (#1405, P1)

P1 adds observation to the delivered pane worker. It does not change HUD
application frequency, hidden DOM application, SDK cadence, staging depth,
epoch admission or GPU upload policy. Establish an instrumented baseline before
the separate HUD-revision and hidden-paint changes.

Use the reviewed native profiling harness and its frozen executable, SDK,
content bundle, scenario, seed and display profile. Record actual window sizes,
station claims, asset readiness and competing processes. Begin with quiet 1080p
Combat Test and Falling Skyway runs: renderer only, chrome only, one Station
console, and two Station consoles. Do not count a pane parked in its join lobby
as a Station console. The two-console matrix does not close #1404's separate
three-Station hardware acceptance. No concurrent build or asset generation is a
valid comparative capture.

## Enable and finish

The native host's ordinary `app::run` wrapper installs capture when
`PHOENIX_SURFACE_CAPTURE` names a new output file. The parent directory must
already exist. An existing file is refused. With `PHOENIX_FRAME_CAPTURE` set,
surface capture defaults to its companion: `frames.json` becomes
`frames.surfaces.json`. The explicit surface path wins. It does not require
`--frame-stats` or a particular log level. Remove both capture variables for an
uninstrumented control. No new surface timing clocks are read in that control;
the existing `--frame-stats` clocks remain independently opt-in.

The recorder has no duration controller of its own: use the harness's bounded
run and graceful close. The owner lives outside Bevy's App, so winit destroying
the World cannot discard the output handle. Finish closes the recorder without
joining or waiting for the pane worker. A forced process termination may leave
the reserved file empty; that is incomplete evidence, never a zero-work result.

Compare control / instrumented / control under the same conditions to disclose
observer overhead. Keep the warm-up and measurement windows in the receipt;
initial loading is not steady-state throughput. A renderer-only example that
deliberately bypasses `native_host::run` has no Ultralight surface capture.

## Reduce a capture

`node scripts/profile-surfaces.mjs frames.surfaces.json 40 30 summary.json`
summarizes a 40-second warm-up and 30-second observation window. The optional
output file must be new; without it, JSON goes to stdout. An incomparable run
exits 2, and malformed schema/count/lifetime data is refused. The exported
`analyzeSurfaceCapture(artifact, { warmupSeconds, measureSeconds })` is the
same reducer used by integration callers.

The window is half-open `[start, end)`. Main passes and worker iterations that
straddle warm-up are counted as excluded boundary samples. Phase totals divide
by main frames or complete worker iterations respectively. Surface rows include
every identity field; HUD repeat detection carries successful revisions across
visibility changes in the same pane epoch, including warm-up history. There is
no per-view raster estimate.

Each row separates window uploads of older frames from outcomes of frames
produced inside the window. The latter can resolve inside the window, afterward,
or remain unfinished when recording closes. `comparable` establishes complete
raw detail and sufficient recording duration, not that every frame has a known
terminal outcome. Unfinished cohort counts remain unknown outcomes. Expected
Station claims, asset readiness, clock alignment and process/build provenance
are the profiling harness's separate validity checks.

## Read schema 1

`events` holds raw events with integer `at_ns` offsets from the recorder's
monotonic origin. `started_unix_ms` anchors that origin to wall time at
millisecond precision. Clock origins of independently installed collectors are
not interchangeable. Concurrent producers may acquire the recorder lock in a
different order from their timestamps; sort by `at_ns` for a timeline.

Each surface event carries pane `id`, texture `epoch`, `kind` (console, lobby or
hud), physical `width` and `height`, `device_scale` and compositor `visible`.
Visibility records Phoenix's decision, not an OS occlusion query. Frame events
carry a unique `frame` sequence and retain the identity captured when the
pixels were produced, even after resize, close, or a new texture at that pane.

| Event | Meaning and reduction |
| --- | --- |
| `main_frame` / `main_pass` | Main-frame denominator and event drain / upload queueing nanoseconds. Queueing does not include render-world texture writes. |
| `iteration` | Worker denominator and global SDK update/render, aggregate pump/copy, actual sink publication, and whole-iteration wall durations. Concurrent worker time is never subtracted from the main frame. |
| `hud_slot` / `push` | Source revision replacement versus actual per-view applications/failures. A repeated revision is visible as repeated successful applications. Bridge-pump duration also includes load checks, encoding, queue handling and inbound draining; it is not isolated JavaScript execution time. |
| `copy` | `copied`, `clean`, `failed`, or `buffer_starved`; only the first means pixels were produced. Dirty pixels are known only for an unforced successful copy; forced or failed copies record null because the wrapper exposes no pre-force dirty bounds. |
| `produced` | Copied pixels, full-copy causes and last successfully applied HUD revision. Causes can coexist: initial, resize, reveal, bridge/HUD push, failed-copy retry or buffer retry. |
| `drained` / `extracted` | Age at acceptance into the main loop and movement into the render world. Neither means the frame reached a texture. |
| `deferred` / `promoted_full` | Attempt count including inherited patience, and full upload inherited when superseding a full deferral. A deferral is not a terminal loss. |
| `uploaded` | Actual rectangle pixels and bytes (pixels × 4), age, full flag, and host `write_texture` enqueue time. This is not GPU execution time. |
| `discarded` | Exactly one terminal outcome per disposed frame, with closed/stale epoch, superseded deferral, exhausted retry, refused layout, no renderer, or generic buffer teardown reason. |

SDK update and render are global operations. Per-surface push/copy observations
and controlled single-surface experiments identify contributors; dividing the
global render duration among visible surfaces does not measure their raster
cost. Worker `total_ns` minus its disjoint phase durations is unattributed
overhead; individual per-view pump observations are already inside aggregate
pump time and must not be added to it again.

## Loss, boundaries and validity

`totals` contains exact event counts through closure; a missing key means zero.
`deferred_attempts` counts attempts, `copy_decisions` includes starvation, and
`push_batches` counts observed batches, not messages. Sum raw push `applied` /
`failed` fields for those message counts. Sum pixel fields by event type rather
than using the surface's full area for every event.

Memory is bounded to 262,144 raw events. After that, exact event totals continue
but `omitted_events` increases. Reject truncated captures for per-surface,
pixel or duration comparisons: the lost detail cannot be recovered from totals.
Shorten or narrow the next run. Do not silently raise the cap.

At closure, `produced = uploaded + discarded + in_flight_at_close` across the
whole recording. `in_flight_at_close` discloses buffers still held elsewhere;
finish cannot invent their eventual outcome. Later worker events and buffer
drops are excluded from the closed artifact. A successful process exit alone
does not prove complete frame lifetimes or a sufficient measurement window.
Report boundary frames separately and never classify them as uploads, loss or
zero-age samples. For a cropped window, distinguish frames produced inside it
from uploads of frames produced earlier.

A `worker_failed` lifecycle event rejects a run as comparative pane evidence,
even if the host later exits successfully: the simulation deliberately survives
a failed pane worker. The final failed main drain is still counted; its recovery
teardown remains unattributed main-frame work.

Keep raw JSON and provenance beside any summary. Before/after acceptance needs
repeatable changes beyond control variation and the relevant visual/input pass:
first paint, HUD updates and alerts, resize/recovery, F9 hide/reveal, phase
transitions, and Station input. The P1 unit seams establish attribution and
ownership behavior; they do not establish a speedup or close hardware checks.
