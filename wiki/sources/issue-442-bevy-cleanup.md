---
title: Issue #442 — Bevy Cleanup (lobby + tab bar + bezel)
type: source
tags: [issue, client, cleanup, bevy, html, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/442
status: shipped
updated: 2026-06-08
---

# Issue #442 — Bevy Cleanup (lobby + tab bar + bezel)

Fourth and final vertical slice of the [HTML/JS Client GUI Shell PRD #438](./prd-438-html-client-gui-shell.md). Deletes the Bevy lobby UI tree, the embedded tab bar widget, and the phone-bezel frame that issues #439 / #440 / #441 already superseded. Three files shrink from ~2700 to ~900 lines; the Bevy client app now spawns only a UI camera plus the nine per-console panel roots, and every chrome surface (bezel, lobby, tab bar, game-over) is owned by `client.html`.

## Status

Shipped 2026-06-08. Closes PRD #438.

## Problem

After issues #439–#441 landed, the Bevy client app still carried the full pre-HTML chrome inside three files:

- `src/client/console_shell.rs` (712 lines) — `EmbeddedTabBar` widget, `ConsoleShellPlugin`, tab-button rendering, 44 px bezel inset reserved for the Bevy phone bezel.
- `src/client/phone_border/framing.rs` (597 lines) — bezel corner / edge spawning, red-alert texture swap, alert banner, panel reparenting into the bezel safe zone.
- `src/client/app.rs` (1525 lines) — 21 lobby-UI marker components, `setup_lobby_ui` (~300 lines), station-row spawning, complexity segmented control, engage-button press handler, game-over screen, orientation-change rebuild systems.

All three were dead in production: the HTML shell rendered its own bezel (z-index 15), its own tab bar (z-index 16), its own lobby section (`#lobby-ui`), and its own game-over banner. The Bevy code still compiled, still spawned entities, and still ran the orientation watcher — wasting frames and confusing every new contributor with two parallel implementations.

## Solution

Surgical neuter, not file delete. Keep the surfaces the nine per-console panel plugins still call (`PhoneAssets`, `DeviceOrientation`, `is_landscape`, `ConsoleShell::spawn`) and delete only the bezel/tab-bar/lobby code on top.

### `src/client/console_shell.rs` — rewritten 712 → 195 lines

Deletions:

- `EmbeddedTabBar` component.
- `EmbeddedTabButton` component.
- `TabBarSnapshot` resource.
- `rebuild_embedded_tab_bars` system.
- `handle_embedded_tab_press` system.
- `tab_button_visuals` system.
- `ConsoleShellPlugin` (registered the three systems above).
- The 44 px bezel inset on the shell root (`top`/`left`/`right`/`bottom` offsets) — HTML CSS now provides the safe-zone padding.
- The `tab_bar` field on `ConsoleShellEntities`.

Preserved:

- `ConsoleShell::spawn(commands, panel_bg, is_landscape, help_panel, fill_primary, fill_secondary, _phone_assets)` — exact pre-#442 signature. The trailing `_phone_assets` param is now unused (prefixed `_`) but kept so every per-console panel plugin compiles without edits.
- `ConsoleShellEntities { root, primary, secondary }` — drops only the `tab_bar` field.
- Top-left "?" help button + `spawn_help_overlay_root` modal at window root.

Spawn tree is now: `root` (fills window, `PositionType::Absolute`, zero offsets, `ImageNode(panel_bg)`, `ZIndex(1)`) → `primary` + `secondary` (flex slots) + absolutely-positioned help button. The help overlay still spawns at window root via `spawn_help_overlay_root` so it can render over the HTML bezel + tab bar.

### `src/client/phone_border/framing.rs` — rewritten 597 → 258 lines

Deletions:

- `spawn_bezel_on_startup` system (spawned the 9-slice border).
- `refresh_alert_banner` system.
- `swap_phone_border_textures` system (red-alert texture swap).
- `reparent_panels_into_bezel` system.
- `update_red_alert_intensity` system.
- `AlertBannerText` marker component.
- `pulse_intensity` / `sine_pulse` / `approach` helpers + their unit tests.
- The `BorderAssets` loader call inside `load_phone_assets` (the asset map is no longer populated on the client; the server's `ViewscreenBorderPlugin` has its own `ViewscreenBorderAssets` for the desktop viewscreen, untouched).

Preserved:

- `PhoneAssets` resource — 32+ texture and font handles consumed by every panel plugin's spawn system. Name and field set unchanged.
- `RadarIconHandles` struct + the radar-icon asset paths nested inside `PhoneAssets`.
- `load_phone_assets` Startup system (minus the `BorderAssets` section).
- `populate_radar_icon_lookup` Update system — still fills `RadarIconLookup` from `PhoneAssets.radar_icons`.
- `DeviceOrientation` resource + `is_landscape(Option<&DeviceOrientation>)` helper used by every panel for portrait-vs-landscape layout decisions.
- `detect_orientation` PreUpdate system — change-detected window-aspect watcher.
- `PhoneBorderPlugin` — name kept for `add_client_plugins` compatibility. Now only inits `DeviceOrientation`, startup-loads `PhoneAssets`, runs `detect_orientation` in PreUpdate, and runs `populate_radar_icon_lookup` in Update. A rename to `PhoneAssetsPlugin` was considered and deferred to avoid churning the registration site.

### `src/client/app.rs` — rewritten 1525 → 504 lines

Deletions (21 lobby-UI marker components):

- `LobbyRoot`, `GameOverScreen`, `LandscapeMode`, `ConsoleListRoot`, `EngageButton`, `StationButton`, `ScenarioIntroBlock`, `CrewHeader`, `CrewCountCurrent`, `CrewCountMax`, `ReadyPill`, `ReadyPillText`, `FooterStatus`, `ReleaseStationButton`, `ComplexitySegControl`, `ComplexityOptionButton`, `StationDetailPanel`, `StationDetailTitle`, `StationDetailConsoles`, `GameOverReasonText`, `ScenarioIntroTitle`, `ScenarioIntroBody`.

Deletions (systems):

- `setup_lobby_ui` (~300 lines spawning the lobby tree).
- `detect_initial_orientation`, `detect_orientation_change` (LandscapeMode driver).
- `toggle_lobby_visibility_on_phase`, `toggle_game_over_visibility`.
- `rebuild_lobby_ui_on_change` (LobbyView change-detected rebuild).
- `refresh_engage_button`, `spawn_station_row`, `spawn_station_row_inner`, `spawn_detail_column`, `refresh_station_detail`, `refresh_crew_header`, `update_scenario_intro`, `refresh_footer_status`.
- `handle_station_button_press`, `handle_engage_button_press`, `handle_release_station_button_press`, `handle_complexity_option_press`.
- `COL_*` colour constants used only by the deleted spawn helpers.

Deletions (registration):

- `ConsoleShellPlugin` dropped from `add_client_plugins`.

Imports trimmed: `engage_message`, `message_for_station_slot_click`, `StationSlot`, `GamePhase` no longer referenced.

Additions:

- `setup_ui_camera` Startup system that spawns `(Camera2d, IsDefaultUiCamera)`. The camera spawn was previously nested inside the deleted `setup_lobby_ui`; moving it to a dedicated system keeps every panel renderable in headless and lobby phases without depending on the lobby tree.

### Tests

No new unit tests. Coverage was already in place:

- `tests/client/{phone-bezel,phase-toggle,tab-bar,content-switcher}.test.js` (85 Vitest tests) cover the HTML modules that replaced the deleted code.
- `cargo test` — 2040 lib tests pass after the cleanup (no regression).
- `npm.cmd run test:editor` — 1067/1067 vitest pass.
- Smoke tests (`tests/smoke/`) don't touch the bezel, tab bar, or lobby chrome and were unaffected.

## Schema additions

None. The deletions don't touch any wire types, components consumed across the crate, or the `ConsoleShell::spawn` signature. The only newly-added Bevy entity is the UI camera (was previously spawned by `setup_lobby_ui`).

## Key decisions

- **Surgical neuter, not file delete.** Each of the three files exports a surface the rest of the client crate depends on: `ConsoleShell::spawn` (called by all nine panel plugins), `PhoneAssets` (consumed by every panel's spawn system to look up 32+ texture handles), `DeviceOrientation` + `is_landscape` (consumed by every panel for portrait/landscape decisions), `ClientAppPlugin` + `OutboundClientMessage` + `add_client_plugins` (the crate's main entry point). Deleting the files would have triggered a cascade of edits across the nine panel plugins. Keeping the files and gutting only the bezel / tab-bar / lobby code limits the blast radius to the three rewritten files.
- **`ConsoleShell::spawn` keeps the `_phone_assets` param.** Prefixed `_` to silence the unused-variable warning. Every panel still passes its `PhoneAssets` reference; touching that contract would have forced edits in nine `*_panel.rs` plugins for no real benefit.
- **`PhoneBorderPlugin` name kept.** The crate-level `add_client_plugins` registers it under that name. Renaming to `PhoneAssetsPlugin` would have been more honest (it loads assets, doesn't spawn a border anymore) but would have forced a churning grep-rename pass across the crate. Deferred indefinitely; the doc comment on the plugin explains the new reality.
- **`Camera2d` spawn moved to its own system.** The camera was previously created inside `setup_lobby_ui`. Moving it to `setup_ui_camera` keeps it independent of any lobby logic — the camera should exist regardless of game phase.
- **`BorderAssets` retained in `src/gui/border.rs`.** The struct is now orphaned on the client (no system populates or reads it), but it's a small, self-contained type that may be useful again later. Removing it is a separate concern from this slice.
- **Doc comment in `src/gui/border.rs:271` left for opportunistic cleanup.** Mentions `update_red_alert_intensity`, which no longer exists. Cosmetic; flagged for a future lint pass.
- **`Setup_ui_camera` doesn't gate on game phase.** The lobby is now HTML; the Bevy camera renders nothing during lobby (panels spawn only when the player owns a console and the phase is InProgress). Eager camera creation is harmless and avoids any "first phase render is blank" race.

## Out of scope

- Renaming `PhoneBorderPlugin` → `PhoneAssetsPlugin`. Mentioned above.
- Migrating the remaining Bevy-rendered panels (Helm, Sensors, Shields, Navigation, Power, Comms) to HTML — explicitly out of scope per PRD #438.

## Follow-up cleanup pass (post-merge, 2026-06-08)

Three reviewer items from the `review-plan` pass against PRD #438 were addressed in a follow-up commit on the same day, before pushing the series to `origin/main`:

- **Deleted `src/gui/border.rs` entirely.** Every export (`BorderAssets`, `BorderConfig`, `CornerSlot`, `EdgeSlot`, `GuiBorder`, `BorderContentArea`, `GuiBorderWidget`, `update_border_textures`, `GuiBorderPlugin`) and the five inline unit tests were orphaned after the `framing.rs` rewrite. `src/gui/mod.rs` dropped `pub mod border;`, the re-export block, and the `GuiBorderPlugin` registration in `GuiPlugin::build`. Doc comments in `src/gui/vignette.rs:1-25` and `src/ship_view.rs:121-161` were updated to drop stale `BorderContentArea` references. The server's `ViewscreenBorderPlugin` is unaffected — it uses its own `ViewscreenBorderAssets` and is independent.
- **Extracted `nextActiveConsole` to `gui/active-console.js`** with 9 Vitest tests in `tests/client/active-console.test.js`. The inline `setActiveConsole(name)` in `client.html:659` now delegates the `null`/`undefined`/`""` → `""` sentinel mapping to the module, locking the `src/client/bridge.rs:160` contract (the `""` = "follow player's primary console" sentinel) by test instead of inspection. The inline function keeps a fallback inline branch so the first paint never blanks out if the module hasn't loaded yet.
- **Replaced inline `CONSOLE_LABEL`/`CONSOLE_INITIAL` duplication in `client.html:699-720`** with a `consoleLabel(name)` helper that reads `window.CONSOLE_LABEL` from `gui/tab-bar.js` (the single frozen source). Three call sites (`client.html:749, 837, 883`) now route through the helper. `CONSOLE_INITIAL` was dropped entirely from the inline scope — it was declared but never referenced inline; only `gui/tab-bar.js` consumes it via its own module-local copy. The helper falls back to the raw console name when the module hasn't loaded yet, matching the race-safety pattern of `sectionVisibility` / `consoleSections` / `tabBarLayout`.

Module 8 (`gui/captain-{landscape,portrait}.html` reference files) was marked "Optional design QA" in the PRD body and is deferred — no GitHub issue, no follow-up. See the parent PRD's Out of Scope section.

After the cleanup pass: `cargo test` 2035/2035 pass (was 2040 pre-cleanup; the 5 inline tests in the deleted `src/gui/border.rs` are gone), `npm.cmd run test:editor` 1076/1076 pass (1067 pre-cleanup + 9 new), `cargo check --features client --no-default-features` and default `cargo check` both clean (13 pre-existing dead-code warnings in `src/server/viewscreen_border.rs` are not introduced by this slice and remain).

## Cross-references

- Parent: [PRD #438 — HTML/JS Client GUI Shell](./prd-438-html-client-gui-shell.md) — now shipped.
- Siblings:
  - [Issue #439 — HTML Phone Bezel Frame](./issue-439-html-phone-bezel.md) — shipped.
  - [Issue #440 — Lobby Integration + Phase Toggle](./issue-440-html-lobby-phase-toggle.md) — shipped.
  - [Issue #441 — Tab Bar + Content Switching](./issue-441-html-tab-bar-content-switching.md) — shipped.
- Files rewritten: `src/client/console_shell.rs`, `src/client/phone_border/framing.rs`, `src/client/app.rs`.
- Surfaces retained for downstream callers: `ConsoleShell::spawn`, `ConsoleShellEntities { root, primary, secondary }`, `PhoneAssets`, `RadarIconHandles`, `DeviceOrientation`, `is_landscape`, `PhoneBorderPlugin`, `ClientAppPlugin`, `OutboundClientMessage`, `InboundServerMessage`, `add_client_plugins`.
- HTML modules that replaced the deleted Bevy code: `gui/phone-bezel.js` (#439), `gui/phase-toggle.js` (#440), `gui/tab-bar.js` + `gui/content-switcher.js` (#441).
- Predecessor (the Rust implementation now superseded): [PRD #187 — Phone Console HUD](./prd-187-phone-console-hud.md).
- Post-merge cleanup pass: also deleted `src/gui/border.rs` and created `gui/active-console.js` + `tests/client/active-console.test.js`. Edited `src/gui/mod.rs`, `src/gui/vignette.rs`, `src/ship_view.rs`, and `client.html` (active-console module + `consoleLabel(name)` helper).
