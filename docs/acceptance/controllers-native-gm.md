# Controllers, assigned consoles and a local GM

Run this after the automated Rust and JavaScript checks. Record the commit,
host build flags, bundle build, Windows version, controller models and monitor
arrangement with the results. Unticked items are unverified, not implied passes.

Use a fresh bundle and a native build with `host,ultralight`. Launch
`run-native.bat lobby`. Three extended-desktop monitors allow a viewscreen,
station consoles and a dedicated GM screen at the same time. Use two controllers
for exclusive-assignment checks; include identical models if available.

## Controllers

- [ ] Plug a controller in **before launching the host**. Open a Helm screen
      later. Its settings list the controller without unplugging it or pressing
      a controller button. Select it and verify actual steering and thrust.
- [ ] Assign a second controller to another native console. Each drives only
      its own console, regardless of keyboard focus. The first console's device
      is visibly unavailable in the second console's selector.
- [ ] Change a binding, tuning and the hide-touch-controls preference. Move the
      console to another monitor, then quit and relaunch the host with the same
      hull. Confirm all preferences and an unambiguous device selection return.
- [ ] While a console reloads after a move or a changed screen split, its
      controller remains unavailable to other consoles. Turning the console Off
      releases that controller for another console to select.
- [ ] Change controller enumeration order across restarts. Verify a different
      device never inherits control from a saved slot number. Where identical
      controllers cannot be distinguished, verify selection is required.
- [ ] With the default hide option enabled, Helm's virtual joystick and lateral
      thrust control disappear only where their controller bindings are usable.
      Disable the option and verify the controls return without disconnecting.
- [ ] Unplug the selected controller while providing input. Verify thrust and
      holds neutralize, touch controls return, and another connected controller
      cannot silently take over. Reconnect and verify the neutral-input gate.
- [ ] Help lists the selected console's current controller bindings. Remap a
      binding and switch console tabs; the guide updates. Unplug and verify
      controller-only guidance disappears. Repeat the browser checks in a
      browser console, including refresh with a controller already available.

## Lobby and station ownership

- [ ] On an ordinary phone/browser, claim a station. Its guide replaces the
      picker. Change station follows release confirmation and restores selection.
- [ ] Open a station screen from the host lobby. It shows its assigned guide
      and permits normal console tabs, but offers no release or station switch.
- [ ] A second phone cannot claim that reserved station, including during a
      native console reload or unplug. Backfill operates while it is disconnected.
- [ ] Unplug its monitor. The host can still move the assigned console or turn
      it Off. Move preserves identity and settings; Off releases the station.
- [ ] A connected phone's held station cannot be silently displaced by opening
      a native screen. Release it, then verify the assignment succeeds.

## Native GM

- [ ] Before launch, use the GM row to select a free dedicated monitor. The
      viewscreen and station rows cannot assign a surface to that monitor.
- [ ] Verify the full shared GM workspace: map and inspector, activity feed,
      mission events and objectives, communications, entity placement/effects,
      station puppeting, knowledge comparison and session controls.
- [ ] Fly the crew's ship and manipulate it through a typed GM action. Both
      screens reflect the same running simulation. The activity/result shows GM
      attribution; a phone has no privileged GM controls or projection stream.
- [ ] The GM must ready through the existing readiness flow. Exercise Force
      Start and its existing validation/refusal behavior.
- [ ] After launch, Off is disabled and another GM cannot be enabled. Moving
      the existing GM screen to a free monitor still works.
- [ ] Unplug the GM monitor during play. The simulation pauses once. Move or
      restore its screen; it retains its operator identity and remains paused
      until the GM explicitly chooses Resume. Crew inputs cannot bypass this.
- [ ] Return to the lobby, change the arrangement, quit and relaunch. The GM
      placement restores with the saved bridge layout; missing hardware is
      explained and can be reassigned before launch.
- [ ] Before launch, disconnect every monitor except the GM's. The viewscreen
      does not overlap it. Use Return to host lobby on the GM screen; the GM
      role turns Off and the lobby becomes available on the remaining monitor.

An actual embedded-view crash needs a reproducible runtime fault or diagnostic
harness. Record that case separately from unplug testing; automated recovery
tests do not establish that a real SDK crash was observed.

## Automated validation — 8 September 2026

Implementation checks passed on the integrated code through `04c254be`.
Acceptance corrections through `7b75c8bc` were checked separately below.

| Check | Result |
| --- | --- |
| Workspace Rust suite, `--features headless` | 8,018 passed; 93 explicitly ignored |
| Full Vitest suite | 7,216 passed |
| Native GM queue framing regression | 4 passed, including overflow and multiline payloads |
| Chromium controller, lobby and GM regression | 34 passed on `7b75c8bc` |
| Actual Ultralight GM workspace and host lobby | Both passed after the GM framing correction |
| Actual Ultralight participant panes | Passed on `7b75c8bc`, including correlated console input, storage isolation and recovery |
| Formatting, required Clippy configuration, strings and generated-data drift checks | Passed |
| PASM validate, scan and traceability | Passed |
| Wiki lint | 64 pages and 1,357 references checked; no broken or unindexed entries |

The native suite used `PHOENIX_AMBIGUITY_BASE_REF=a57bcca4dac4569ab0d1f1ee788f2f31149b67c8`.
The browser and embedded checks used the matching Trunk/client bundle built from
`04c254be`; subsequent changes corrected the native GM queue and test fixtures.
The final native check used `host,ultralight`. After returning from the shared
build directory's other worktree, `src/lib.rs` was timestamp-refreshed without
changing its SHA-256. Cargo reported a fresh library build from this checkout,
and its dependency record included the expected native pane and GM sources.

The final browser command was run from `tests/smoke` with an owned server port:

```text
npx playwright test semantic-action-helm.spec.js lobby.spec.js gm-page.spec.js gm-confirmation.spec.js --project=chromium --reporter=line
```

Physical controller and multi-monitor acceptance remains unverified. The
checkboxes above remain open for that hardware pass.
