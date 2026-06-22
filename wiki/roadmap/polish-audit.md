---
title: Polish Audit
type: roadmap
tags: [polish, quality-of-life, presentation, audio, juice, roadmap]
sources: [server.html, client.html, gui/, src/core/messages.rs, src/server/viewscreen_border.rs, src/server/renderer.rs, docs/scenarios/before_the_fire.md, assets/sounds/]
updated: 2026-06-22
---

# Polish Audit

## Summary

Project Phoenix has a strong simulation and UI shell, but the moment-to-moment feedback layer is still thin. The game already exposes rich event messages (`DamageTaken`, `PhaserFired`, `TorpedoLaunched`, `ObjectiveSummary`, `CommsState`, `GameOver`, `ConsoleHullUpdate`, `ShieldStatus`, `PowerState`) and already has basic viewscreen and phone feedback. The missing polish is a coherent presentation pass: event-specific audio, better game-over and objective ceremonies, clearer crew guidance, richer status transitions, and tactile/visual feedback for each console.

## Existing polish foundation

- `assets/sounds/` currently contains four files: `background_hum.ogg`, `red_alert_siren.ogg`, `ship_engine.wav`, `ui_click.ogg`.
- `server.html` plays the background hum and engine loop after `GameStarted`; engine volume follows `SimSnapshot.engine_thrust`; the red-alert siren plays on the rising edge of `red_alert`.
- `client.html` plays `ui_click.ogg` for most deliberate outbound messages and vibrates the phone for hull damage on `DamageTaken`.
- The viewscreen has red-alert border/vignette swaps, HUD status, shield flash, hull camera shake, asteroid-destroyed ripple effects, loading overlays, scenario selection, and fullscreen controls.
- The client has HTML console iframes for all nine consoles, a phone bezel with red-alert asset swaps, tab bar, complexity UI, help modal, orientation handling, and asset-loading overlay.

## Highest-impact missing features

### 1. Audio event language

Current audio is ambient rather than expressive. Add a small sound design system that can play one-shot sounds on both host and phones, with per-event routing.

Needed files:

- `assets/sounds/ui_click.ogg` exists; add variants: `ui_back.ogg`, `ui_confirm.ogg`, `ui_error.ogg`, `ui_toggle_on.ogg`, `ui_toggle_off.ogg`, `ui_tab_switch.ogg`.
- Lobby: `lobby_join.ogg`, `lobby_leave.ogg`, `station_assigned.ogg`, `station_released.ogg`, `engage_ready.ogg`, `game_start_stinger.ogg`.
- Bridge ambience: keep `background_hum.ogg`; add `bridge_red_alert_loop.ogg` separate from the siren, `bridge_low_hull_loop.ogg`, `bridge_power_low_loop.ogg`, `bridge_static_burst_01.ogg`.
- Helm: `engine_thrust_rise.ogg`, `engine_thrust_fall.ogg`, `boost_start.ogg`, `boost_loop.ogg`, `boost_empty.ogg`, `impulse_charge_start.ogg`, `impulse_charge_loop.ogg`, `impulse_engage.ogg`, `impulse_cancel.ogg`, `nav_waypoint_set.ogg`, `nav_waypoint_clear.ogg`.
- Tactical: `phaser_fire_01.ogg`, `phaser_fire_02.ogg`, `phaser_cooldown_ready.ogg`, `target_lock_acquire.ogg`, `target_lock_lost.ogg`, `torpedo_load_start.ogg`, `torpedo_loaded.ogg`, `torpedo_launch.ogg`, `torpedo_detonate.ogg`, `shield_hit_light.ogg`, `shield_hit_heavy.ogg`, `enemy_hull_hit.ogg`, `enemy_destroyed.ogg`.
- Repair/Power/Shields: `repair_dispatch.ogg`, `repair_complete.ogg`, `repair_failed.ogg`, `console_damage_light.ogg`, `console_damage_critical.ogg`, `power_increase.ogg`, `power_decrease.ogg`, `battery_lockout.ogg`, `battery_recovered.ogg`, `shield_focus_set.ogg`, `shield_facing_down.ogg`, `shield_facing_restored.ogg`.
- Sensors/Navigation/Comms/Objectives: `scan_ping.ogg`, `new_contact.ogg`, `objective_added.ogg`, `objective_complete.ogg`, `objective_failed.ogg`, `comms_incoming.ogg`, `comms_urgent.ogg`, `comms_response_sent.ogg`, `comms_out_of_range.ogg`.
- End states: `ship_destroyed.ogg`, `mission_success.ogg`, `mission_failed.ogg`, `scenario_end_stinger.ogg`.

Implementation shape:

- Add a tiny JS `gui/audio-bus.js` for client-side one-shots and settings. Server page can use a sibling inline module or shared copied module.
- Gate autoplay by first user gesture, then queue/mute gracefully until audio is unlocked.
- Route host-only cinematic sounds on `server.html`; route console-local sounds in `client.html` and/or individual iframes.
- Add persisted `localStorage` settings: master volume, music/ambience, UI, alerts, effects, mute.

### 2. Game-over and scenario-end ceremony

`GameOver { reason }` is wired into state, but the client currently just moves into `GameOver` phase; no strong end screen is visible in the shell. Before the Fire also notes that ship destruction does not end gracefully narratively.

Needed:

- Host overlay: title, reason, crew survival/ship status, mission outcome, major completed/failed objectives, elapsed mission time.
- Phone overlay: station-specific final status, "Awaiting host reset", reconnect-safe final reason.
- Distinguish `ShipDestroyed` from scenario-authored `game_over` outcomes.
- Add authored final comms/outcome blocks for Before the Fire: Fight, Evacuate, Shield Containment, Requiem Override, Ship Lost.
- Add end-state audio files listed above and a host screen fade/desaturation.

### 3. Objective and comms presentation

Objectives are mechanically present, and Comms is rich, but important state changes need ceremony.

Needed:

- Host-side objective toast rail: added, completed, failed, mandatory objective highlighted.
- Phone toasts for relevant consoles: Captain always; Comms for dialogue-driven objectives; Navigation/Sensors when target-bearing objectives appear.
- Audible differentiation: `objective_added.ogg`, `objective_complete.ogg`, `objective_failed.ogg`, `comms_incoming.ogg`, `comms_urgent.ogg`.
- Comms urgency escalation: urgent message should pulse the tab, optionally vibrate, and surface a small unread count in the tab bar.
- "Show On Screen" should create a host viewscreen transmission card with sender portrait/icon placeholder, speaker, subject, body, and response state, not just a hidden data transition.

### 4. Console health and damage feedback

Per-console hull damage exists, but players need stronger cues about "my station is dying".

Needed:

- Phone-local damage overlay driven by `ConsoleHullUpdate`: cracked glass/noise layer, red flash, warning chirp, vibration pattern per severity.
- Console disable/critical state language: when a console is at 0 HP, show a persistent "offline / repair required" overlay and suppress misleading ready buttons.
- Repair loop celebration: team dispatched, en route, repairing, complete, failed/cancelled, with sound and small animation.
- Host damage attribution: when a console is damaged, the viewscreen HUD can briefly call out `HELM DAMAGE`, `TACTICAL DAMAGE`, etc.

### 5. Weapons and impact juice

Combat has mechanics, beam rendering, torpedoes, shields, and damage events. The presentation should make every fire/hit/kill legible.

Needed:

- Phaser fire audio and short phone recoil animation on Tactical.
- Phaser hit spark/shield flare on target, separate from player shield flash.
- Torpedo launch trail, glow, host camera-relative streak, detonation bloom, and `torpedo_detonate.ogg`.
- Target lock state change effects: acquire, lost, invalid target, target destroyed.
- Enemy hull/shield bars on Tactical/Sensors should pulse on damage and settle, rather than only changing value.
- Kill confirmation: host ripple plus audio plus Tactical confirmation chip.

### 6. Lobby and onboarding quality-of-life

The lobby is functional and responsive, but a new crew needs better guidance.

Needed:

- Host "join flow" steps: scan QR, enter name, choose station, captain engages. Keep it short and state-driven, not a manual.
- Phone pre-game station preview: show what consoles a station contains, complexity presets, and what the player will do.
- Captain-only engage gating explanation: why Engage is disabled, e.g. "Need Helm station filled" or "Waiting for asset load".
- Connection quality/status: reconnecting, stale state, host lost, seat reserved.
- Scenario intro briefing screen before Engage or immediately after load: title, premise, starting orders, suggested crew size.

### 7. Accessibility and phone ergonomics

Needed:

- Audio mute/volume controls on host and phone.
- Optional reduced motion flag: disables screen shake, vignette pulse, radar sweep intensity, and heavy phone vibration.
- Optional high-contrast mode for radar blips and warning colours.
- Larger touch targets audit across every console in short landscape and small portrait.
- Haptic settings: off/light/full. Current vibration only fires on hull damage; extend to warning classes.
- Wake lock request on phone during game, with fallback message when unavailable.

### 8. Viewscreen cinematic pass

The host view is the shared spectacle. It already has frame, HUD, red alert, camera shake, shield flash, fog/region effects, torpedo and ripple rendering. It still needs more event direction.

Needed:

- Camera micro-motions: slight impulse surge, torpedo launch kick, heavy shield impact shudder, red-alert transition shake.
- Context banners: `NEW CONTACT`, `OBJECTIVE UPDATED`, `INCOMING TRANSMISSION`, `SHIELDS DOWN`, `HULL CRITICAL`.
- Diegetic reticle overlays for current view target, navigation waypoint, and hostile lock.
- Scenario title card and act transition card for Before the Fire branches.
- Better non-combat space ambience: distant station lights, nebula volume treatment, region entry colour shift, star/planet scale cues.

## Suggested implementation order

1. Audio bus and settings, then wire the existing four sounds through it.
2. Add event one-shots for lobby, red alert, damage, objectives, comms, weapons, repair, power, game over.
3. Add host and phone Game Over overlays.
4. Add objective/comms toasts and tab unread/urgent pulses.
5. Add per-console damage overlays and repair completion feedback.
6. Add combat hit/kill polish: phaser hit, torpedo detonation, target lock changes.
7. Add scenario intro/outcome screens for Before the Fire.

## Open questions

- Should all console-local sounds come from `client.html`, or should individual console iframes own their own sound triggers?
- Should the host play all cinematic sounds even when no player has the relevant console, or only sounds visible on the viewscreen?
- Should scenario authors be able to trigger custom audio cues from TOML, or is the initial pass strictly message-driven?
- Should end states become typed outcomes instead of a single human-readable `GameOver.reason` string?
