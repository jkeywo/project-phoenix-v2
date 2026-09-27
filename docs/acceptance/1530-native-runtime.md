# Native fleet runtime runner (#1530)

Build the native host with `cargo build --features ultralight --bin phoenix-host`
and prepare a matching host bundle plus `node scripts/build-client.mjs`.
The executable must contain Ultralight; staged SDK DLLs beside a host-only
binary do not establish that. Coordinate local Cargo commands with other tasks.

Start the loopback service in a separate terminal:

```powershell
node scripts/rendezvous-dev-server.mjs --port 18888
```

Run the native matrix from the source checkout, using a new output directory:

```powershell
node scripts/fleet-native-matrix.mjs --binary target/debug/phoenix-host.exe --bundle dist --source . --out .phoenix/native-matrix-run --rendezvous http://127.0.0.1:18888 --origin http://localhost:8080 --seconds 180 --workload true
```

The runner starts four real native ship simulations and two separate native GM
simulations. Each ship gets three real Ultralight Station documents (Captain,
Helm, Engineering). They claim their own Stations, ready, and send correlated
commands through the ordinary participant adapter: red alert, discrete Helm
boost and Engineering power allocation. Continuous Helm axes deliberately lack
correlated terminal feedback and are not used as receipt probes. GM documents use the private
native GM adapter for Ready, Force Start when needed, and faction hostility
actions. A private manifest exposes the unchanged six-peer probe world so each
GM can select it through the normal scenario arbiter.

The observer lives only in each run's copied GUI modules and client document.
It records actual transport callbacks, terminal Station feedback, attributed GM
activity and outgoing simulation digests. It does not replace fleet factories,
sockets, admission, lockstep, rendering or authoritative state. Each process has
its own SDK working directory, explicit fleet identity store and save directory; only authored assets
are shared through a junction.

A workload pass requires all six peers admitted, all five member links observed
on relay, twelve ready and launched Station documents with at least two distinct
Applied command receipts each, at least two Applied actions from each separate
GM, six distinct acknowledged simulation slots with four ship and two stationless
GM peers, and at least two common digest ticks agreeing across all six peers
after those action receipts. Missing,
refused, duplicate or contradictory evidence fails the gate. `--workload false`
only exercises admission and cannot report a workload pass.

A native owner currently advertises a local GM even when no GM monitor is
assigned. That operator remains unready. The runner allows six seconds for an
automatic start, then asks a separate admitted GM to use the normal Force Start
control; `forceRequested` records this explicitly. Do not describe such a run as
an automatic all-ready launch or as having only two GM operators.

## Artifacts and limits

Every peer directory retains `manifest.json`, `events.ndjson`, `events.json`,
`stdout.log`, `stderr.log`, `source.patch`, the instrumented modules and private
runtime state. The matrix directory contains `matrix.json`, including failure
stage, rosters, observed routes, action counts and common digest samples.
Reconnect credentials and arbitrary wire payloads are excluded from observations.
The binary and input GUI hashes, native engine user agent, OS/CPU/RAM and content
stamp are recorded; the native renderer's log identifies GPU and driver.

The executable's source-build provenance is deliberately marked unverified by
the runner. Preserve the successful build command, exact source revision/diff,
feature list and dependency record with the artifacts before using the run as
acceptance evidence. If a shared Cargo target changed worktrees, refresh the
library's modification time and verify that the dependency record names the
intended source, as required by AGENTS.md.

This is a bounded loopback smoke runner. It does not establish internet/mobile
behavior, controlled impairment, sustained performance or the one-hour mixed
run. All-native direct WebRTC is unavailable: native fleet links advertise
WebSocket relay. Browser/mixed route and impairment evidence belongs in the
[full matrix ledger](1530-six-peer-matrix.md).

For a smaller diagnostic, `scripts/fleet-native-probe.mjs` accepts the same input
options. Omit `--fleet-code` for an owner, provide it for a ship member, or add
`--role gm --fleet-code CODE` to exercise the retained GM landing. Its result is
always bootstrap-only, even if membership succeeds.

## Runtime defects covered by the runner

A lobby with an explicit rendezvous now waits for world selection or an accepted
Join as Peer request before publishing its fleet role. It no longer creates an
owner handle that prevents that later GM join. The ordinary `--world` owner and
`--fleet-code` ship paths keep their roles.

Native fleet updates use an ordered queue capped at 256 entries. Those updates
carry one-shot join/launch requests and drained simulation frames, so replacing
them with the next paint snapshot lost a boot-time ship join before Ultralight
loaded. Deferred updates precede newer ones; overflow is a terminal fleet fault.
Presentation-only lobby snapshots continue to keep the latest value.

A selected native fleet engages managed start policy before the first local
lobby readiness handler, while the retained page may still be loading. Crew
readiness cannot start independent worlds before frozen-roster adoption and the
shared grant. A default member-capable configuration without a join request
retains ordinary LAN-only crew launch.

Health updates no longer resend unchanged ship, crew, GM-ready or validation
announcements. Each changed control value still publishes, while every mesh
frame and Force Start edge is preserved. After member Welcome the cached control
snapshot publishes once even if Rust has no newer update; frames, Force Start
and roster acknowledgements are excluded from that replay. The pre-fix reliable bridge overflow
message did not distinguish its incoming-wire and surface-record queues.

An admitted native GM's own public identity is available to Ready before roster
freeze. The private adapter emits it only after its authenticated owner's
Welcome; Rust checks the selected GM join and admitted status. A pending or
refused join cannot seed presence. Frozen membership replaces that provisional
identity, and new joins or faults clear it. Reconnect capability contents remain
in private storage and are excluded from the observer.

On Windows, the standard Known Folders API ignores process `APPDATA` overrides.
The runner therefore sets `PHOENIX_FLEET_IDENTITY_DIR` to each peer's absolute
private `fleet-identities` path and records that path. The ordinary user store
remains the default outside the runner; a relative or empty override disables
reconnect storage rather than falling back to shared user state. This separation
is necessary for two independent native GM identities on one Windows account.

Native and browser fleet handshakes use the same Rust
`DeliveryStamp::to_field()` compact stamp. The native configured field is
validated with `delivery::check_host_stamp`, then the embedded owner compares
that canonical field exactly. Two empty identities cannot match; missing or
JSON-form stamps fail rather than weakening the browser/native boundary.

## Measured native run — 2026-09-27

The actual Windows/Ultralight six-process workload passed in **107.192 seconds**.
The portable [result and artifact hashes](1530-native-runtime-result.json) record
four ship peers, two separate stationless GM peers, twelve real Station documents
with **50 correlated Applied receipts each**, and at least two attributed Applied
GM actions per separate GM. All five member links reported WebSocket relay.
Accepted roster generations identified six distinct simulation slots; every
peer reported the same post-action digest at tick 300 (`b3da68e4cf499292`) and
600 (`113057e2b1642152`). No terminal runtime or observer error occurred before
intentional teardown, and all six process exits were observed.

The run used native GM Force Start. The ship-owner's combined GM role remained
in the roster, so this proves four ship peers plus two stationless GM peers,
not exactly two GM operator rows or automatic all-ready launch. The rig was
Windows 11, Intel Core Ultra 9 275HX, 64 GiB RAM, RTX 5090 Laptop GPU, Vulkan,
NVIDIA driver 617.14, and Ultralight 1.4.0. This is the bounded local smoke result;
the internet, impairment and endurance limits above remain open.

Private raw evidence is retained under
`.phoenix/native-six-workload-welcome-replay-1530/` in the native runtime worktree.
The binary SHA-256 is
`9df9fde30250e1a511fd724a7924f65a590506ec29f8d10dac06cf4812820822`.
The successful build receipt is `.phoenix/native-build-isolated-stamp-1530.json`;
`.phoenix/native-bundle-welcome-1530.json` records the later JS-only admission
replay. These receipts preserve baseline `178d2274` plus the dirty source diff,
feature list and native dependency record. The private bundle also includes the
reviewed stale-target relay fix from integrated main `2bd24830`; its exact hash
is recorded. The checked-in summary hashes the receipts, each peer's raw events,
manifest and source patch; private reconnect capabilities are excluded.

Earlier failing runs remain in the same ignored artifact directory, including
`native-six-workload-after-lifecycle-1530` (repeated control announcements and
reliable bridge overflow), `native-six-workload-serialized-gm-1530` (shared GM
identity store), and `native-six-workload-isolated-stamp-1530` (ship-ready snapshot
sent before admission). The last was deliberately stopped after that launch
blocker was identified; its `operator-stop.json` records the reason and exact
owned process. None of those failures is counted as workload acceptance.
