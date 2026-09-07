# Native Settings above the HUD

The observed defect was at the native compositor boundary: the lobby's CSS
placed Settings above its own content, but the entire texture remained below
the separately rendered HUD border. The cog was clickable at its covered
location and its open popup was partly obscured.

The HUD now yields its draw layer while the lobby is composited. Hiding chrome
restores the HUD's existing layer above scene overlays. Lobby/console input
priority, page geometry, HUD visibility and surface update/copy policy remain
unchanged. This also clears the lobby's other controls without moving them to
a resolution-dependent inset or changing browser/phone CSS.

The SDK-independent test
`native_host::panes::hud::tests::revealed_lobby_clears_the_hud_without_covering_tiled_consoles`
checks the reveal/hide ordering relative to the lobby, tiled consoles and
native scene overlays. The existing `tests/client/native-settings.test.js`
checks the real Settings shell and its ordinary controls. Neither test proves
final native pixels.

## Isolated source validation

Validated the owned change against base `6e77d705153fc145e103663103e0cd46ed549abe`:

- `node C:/Coding/project-phoenix-v2/node_modules/vitest/vitest.mjs run tests/client/native-settings.test.js`: 14/14 passed. This checks the existing unchanged Settings shell, not Bevy composition.
- `rustfmt --edition 2021 --check src/native_host/panes/hud.rs src/native_host/panes/ultralight.rs` and `git diff --check`: passed.
- PASM `validate`, `scan` and `traceability`: each exited 0, validation OK. The existing `.venv/Scripts/pasm.exe` was used after sandbox Python/cache failures; its installed Vellum revision matches the project's `bde9b3724a4f4536cc601852cee3bf26bb6e3d46` pin. No dependency or compiler setup was performed.
- The changed wiki page's ten local Markdown links, index entry and newly named source targets resolve. Independent read-only source review passed.

The new Rust regression and an SDK-enabled compile have **not** run in this
worktree. They remain queued for the integrator along with the physical pass.

## Physical acceptance still required

On a newly built native executable with the ordinary host bundle:

1. In Lobby, inspect and open the Settings cog, then close it by backdrop and
   keyboard. Confirm the scenario, QR and Station controls remain usable.
2. During a mission, reveal chrome with F9. The full cog and open popup must
   remain legible without the HUD border covering them. Operate the ordinary
   fullscreen control and verify the popup after resizing.
3. Hide and reveal chrome with F9 while Settings remains open. The same popup
   should return without reloading. With chrome hidden, the normal HUD and
   its updates remain visible above the scene.
4. Confirm Station UI input still reaches its console. On a legacy tiled-pane
   configuration, consoles must still cover the lobby and take input first.
5. Check a genuine ending: its HUD overlay remains visible while chrome is
   hidden; revealing chrome puts the interactive lobby above that overlay.

No actual view crash, performance improvement or physical pass is implied by
source inspection. Build/SDK and physical checks were deliberately deferred
to the integrator's coordinated compiler and computer-use slots.

## Escape forwarding follow-up

Physical acceptance on source `734683cdf2577789dffe8f97ade15ebc758bbee5`
found that backdrop dismissal worked while Escape did not. The shared Settings
shell already listens for Escape; the native adapter discarded that logical key.
The follow-up forwards it through the existing focused-pane input stream and
maps it to the SDK's Escape key. The shared dialog and browser/phone code are
unchanged.

The SDK-independent `native_host::panes::keyboard::tests` regressions cover
Escape to the focused pane, no delivery without focus or on release, reserved
F9/Ctrl+Tab, and ordinary text/editing/Tab forwarding. The SDK-gated
`native_host::panes::ultralight::keyboard_tests::escape_maps_to_the_sdk_dismissal_key`
checks the final key mapping without creating a runtime. These new Rust tests,
an SDK compile and physical acceptance are queued for the integrator; no passing
runtime result is implied. The unchanged 14 Settings JavaScript checks recorded
above remain applicable to the shared shell.

Local source validation for this follow-up passed: `rustfmt --check` on the
three owned Rust implementation files, `git diff --check`, and PASM `validate`,
`scan`, and `traceability` through the same installed interpreter recorded
above (each exit 0). The wiki's 52 source paths, 12 local links and index entry
resolve. These static checks do not replace the queued Rust/SDK tests.
Independent read-only source review passed with no actionable findings.

On the corrected native build, open Settings with the mouse and close it with
Escape in Lobby and after F9 reveal during a mission/ending. Reopen it and check
Tab plus Enter/Space and backdrop dismissal; Ctrl+Tab must still move pane focus
and F9 must still toggle chrome. Check Escape while another Station has focus
to confirm it is delivered to that page, not broadcast to every dialog.
