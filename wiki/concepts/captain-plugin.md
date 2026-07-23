# CaptainPlugin

`CaptainPlugin` is a Bevy plugin that handles captain-specific input: Red Alert state setting and camera view selection.

## Scope

First extracted console plugin in the simulation-split series ([#227](https://github.com/jkeywo/project-phoenix-v2/issues/227)). Validated the per-plugin extraction pattern on the smallest console.

**Systems:**
- `handle_set_red_alert` — assigns the ship's `ShipRedAlert` from `SetRedAlert { active }` from the `CaptainChair` holder (lobby ignored, non-captain ignored); assigning (not inverting) makes retries and stale-UI commands idempotent
- `handle_set_view` — sets `ShipState.view_mode` on `SetView` from the authorized console per view variant (camera views → captain, radar → helm, science/sensors radar → sensors, chart views → navigation, comms → comms)

**Tests:**
- `set_red_alert_during_lobby_is_processed_when_no_simset_gate`
- `non_captain_set_red_alert_is_ignored`
- `captain_set_red_alert_works`
- `captain_set_red_alert_false_turns_off`
- `captain_set_red_alert_true_twice_is_idempotent`
- `captain_stale_set_red_alert_false_when_already_off_is_noop`
- `set_view_during_lobby_is_ignored`
- `non_captain_set_view_is_ignored`
- `captain_set_view_changes_direction`

## File

| File | Contents |
|------|----------|
| `src/captain_plugin.rs` | `CaptainPlugin` struct, `Plugin` impl, two handler systems, unit tests |

## Registration

`CaptainPlugin` was extracted from the former `src/simulation.rs` and is now registered by `add_simulation_plugins()` in `src/server_app.rs` and the test `test_app` in `src/server_app.rs::tests`. The module is declared in `src/lib.rs`.

## Related

- [#227 — Architecture: Split simulation.rs into per-table plugins](https://github.com/jkeywo/project-phoenix-v2/issues/227)
- [#233 — Simulation split #1: Extract CaptainPlugin](https://github.com/jkeywo/project-phoenix-v2/issues/233)
- [Captain Console](../entities/captain-console.md)
- [View Modes](../concepts/view-modes.md)
