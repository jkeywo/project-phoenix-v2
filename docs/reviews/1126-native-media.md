# Native media adapter completion (#1126)

Working tree over `18013e6b`, 8-9 September 2026. This extends the committed
output diagnostic with microphone metering and Windows camera preview.

`--setup` asynchronously enumerates Windows cameras and enumerates CPAL audio
inputs/outputs. Profiles retain the existing validation/sharing/persistence
model. Only successfully enumerated classes are resolved; unavailable classes
remain explicit. Camera keys include the OS interface ID. Audio endpoints with
ambiguous or missing names are refused instead of rebound by ordinal/default.

The explicit setup actions require a profile and a named surface:
`--preview-camera`, `--meter-microphone`, `--test-output`. One action per process.
All selected inputs/outputs are preflighted before activation. Sharing warnings
are printed before each action. Missing devices never substitute another device.
The diagnostic is separate from an active Station or mission.

Camera initialization and polling remain on the actual native UI thread, with
asynchronous consent/start/stop and bounded deadlines. Only the newest 320x240
requested frame is retained (640x480 hard allocation limit), at most four copies
per second. Bevy audio is disabled for the preview. Microphone samples become a
peak in the callback and are discarded; empty/stalled callbacks cannot pass.
Neither adds recording or network traffic. Host-only dependencies stay outside
ordinary non-host builds.

Validation so far:

- `cargo check --features host`: passed.
- `cargo test --lib --features host -- native_host::media_ native_host::bridge_media delivery::args::tests`: 83 passed, zero failed/ignored.
- `cargo build --features host --bin phoenix-host`: passed.
- Actual `--setup`: exit 0, two cameras, three microphones, three outputs,
  one 1920x1080 monitor. No capture was activated. Local log:
  `target/issue-1449/media-setup.log`.
- A deliberately absent microphone refused with exit 1 and no substitution.
- The first actual missing-camera invocation found a Bevy integration error:
  WinitWindows is thread-local in Bevy 0.18, not a World Resource. The owner-thread
  registry access was corrected and a native CLI regression was added in
  `tests/native_media_setup.rs`; its result is recorded below after execution.
- Independent source review corrected async cleanup, failure preservation,
  no-frame handling and sharing warnings. No remaining source blocker reported.

A small partial-enumeration report regression was added after the 83-test run;
the final suite must include it. The actual camera image, microphone level,
audible output, permission changes, hot unplug and multi-device contention are
NOT RUN human hardware checks. `docs/acceptance/1126-media.md` now describes the
implemented commands and retains those checks. #1126 remains open for Part B.

After the Bevy registry fix, `cargo test --features host --test native_media_setup
-- --ignored --nocapture` passed **1/1**, zero failed/ignored, exit 0, 1.69 seconds.
The actual camera window now refuses the absent key cleanly; microphone refusal
passes in the same case. The test activates no capture hardware.
