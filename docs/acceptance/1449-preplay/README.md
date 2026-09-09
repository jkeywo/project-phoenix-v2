# Current pre-play retint evidence (#1356 / #1449 A2)

Captured 8 September 2026 in Chromium at 1440×1000, using the same served
bundle and fixture transport for both arms. `dist/gui/` matched all 273 source
files byte-for-byte at `349dea20`. The after arm replaced only the served
document's `<style>` block with this change's source; the before arm retained
the built block. The real vendored QR encoder ran (the smoke fixture's QR stub
was removed). Fonts were ready and screenshots disabled animations so finite
transitions reached their endpoints. Join codes vary per test session.

These are **current** before/after images of #1449's approved inline retint,
not recovered historical screenshots from #1356/#1357. The owner approved
visual changes and reuse of existing tokens; no token was invented. Sub-floor
type sizes now use the existing minimum. The owner also accepted #1357's
landed fire-colour contrast correction (`#e64a34`).

| Surface | Before | After |
| --- | --- | --- |
| Landing | [Before](before-landing.png) | [After](after-landing.png) |
| Scenario picker | [Before](before-picker.png) | [After](after-picker.png) |
| Crew lobby and join panel | [Before](before-lobby.png) | [After](after-lobby.png) |

The capture run selected two tests (before/after), both passed, exit 0, 8.2s.
The picker and lobby after images were visually inspected. This is a rendered
HTML/CSS check; it does not attest to 3D fidelity, all GM/debug panels, or
hardware accessibility.

`npx vitest run tests/client/design-tokens.test.js tests/client/host-landing-render.test.js tests/client/host-landing.test.js`
passed **524 tests in three files**, exit 0. Inline style now has **zero colour
and zero type-size literals**. Outside it, JavaScript/HTML attributes retain
**23 colour and nine size literal syntaxes**, enumerated in `KNOWN_LITERALS`.
Those are outside the requested inline stylesheet criterion.

The broader existing browser selection passed 13, failed one landing first-paint
case waiting for catalogue entries, and skipped the pre-existing loading test.
The landing case then passed alone with tracing (one test, exit 0). The original
timeout is unexplained, so the broad run is not recorded as green. The loading
test deliberately skips under browser automation, which bypasses Loading; its
skip is not loading-progress evidence.

Later current-flow evidence supersedes the need to rely on that failed broad
attempt: port 3159 passed 14 landing/lobby-related cases, exit 0, 58.2 seconds.
The skipped loading spec was replaced by `loading-progress.render.spec.js`,
which drives actual GLB preload with SwiftShader and checks visible intermediate
progress and client delivery. Its final port-3163 run passed 1 case, exit 0,
16.6 seconds (19.2 total), using bundle `project-phoenix-5f8fb494bdf5d61a`.
The earlier run remains a recorded failed attempt; it is not relabelled green.
