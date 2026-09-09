# Native output diagnostic (#1126 / #1449 C1, first vertical path)

The host-only CPAL 0.15.3 adapter enumerates real outputs and retains their
handles for a named-surface test. `--setup` now lists outputs; an empty successful
scan resolves assigned outputs as missing. Camera and microphone support is
reported separately as unimplemented, not falsely inferred from output discovery.

```powershell
phoenix-host --setup
phoenix-host --setup --profile bridge.toml --test-output comms
```

The second command requires an explicit profile and surface. The existing media
law checks kinds, duplicates and sharing consent. Every assigned output is
preflighted before playback; an absent, unnamed or ambiguous endpoint is refused
without substitution. CPAL 0.15.3 has no portable public persistent endpoint ID,
so ambiguous names must be resolved in OS settings rather than rebound by an
enumeration ordinal. No default endpoint is guessed.

Each requested output plays a quiet one-second 440 Hz tone with fade and silence
tail. Streams remain on their calling setup thread and are dropped on completion,
error or timeout. The completion wait is four seconds; this does **not** place a
deadline on driver open/play/drop calls. There is no recording, network call,
microphone access or background stream. This diagnostic exits before a host
listener or simulation starts.

Independent read-only review passed. Review refinements added all-output
preflight coverage and made the success report explicitly output-only.

Validation on the final working tree over `28181846` (with unrelated loading
smoke commit `6f60c7ad` subsequently present):

- `cargo check --features host --bin phoenix-host`: exit 0.
- `cargo test --lib --features host -- native_host::media_output
  native_host::bridge_media delivery::args::tests`: **77 passed, 0 failed,
  0 ignored**, exit 0. Includes explicit CLI requirements, missing/ambiguous
  multi-output preflight, quiet finite tone and empty-scan reporting.
- `cargo build --features host --bin phoenix-host`: exit 0.
- `target/debug/phoenix-host.exe --setup`: exit 0, three real outputs found on
  this Windows machine, one 1920x1080 monitor and successful OS default reads.
- A temporary profile assigned the uniquely named detected headset to `comms`.
  `--setup --profile ... --test-output comms`: exit 0, 1.40 seconds; the adapter
  reported test completion and stream closure.
- A profile naming an absent endpoint: exit 1, with the named missing-output
  refusal and no substitution. This is **not** a physical unplug test.

**Still incomplete:** microphone metering, camera preview, interactive media
assignment/testing on native surfaces, runtime disconnect/reassignment handling,
and Part B's camera + two microphones + two outputs rig pass. Audible routing
was not independently judged. This first output diagnostic does not close #1126
or establish actual remote-media playback. Final integration/push remains.

Subsequent implementation `7fec5813` added the real microphone meter and camera
preview adapters; their current scope and hardware remainder are recorded in
`docs/acceptance/1126-media.md`. The paragraph above is the first output-path
checkpoint, not the final implementation status. The integrated Clippy matrix,
host build and ordinary native suite have since passed; push remains pending.
