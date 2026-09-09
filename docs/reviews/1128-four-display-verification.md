# Four-display native verification — #1127 / #1128

9 September 2026, Windows desktop. Application source `cd030d16`; the
window-test fixture correction is `bb41f528`. These results replace the earlier
single-monitor skip for the automated hardware checks. They do not complete the
human accessibility walkthrough.

## Hardware

The repository's `scripts/profile-hardware.ps1` recorded:

| Display | Physical pixels | Desktop origin | OS scale | Role in test |
| --- | --- | --- | --- | --- |
| DISPLAY3 | 1920 × 1080 | 0, 0 | 100% | Primary / viewscreen |
| DISPLAY1 | 1920 × 1200 | −1920, 0 | 125% | Station |
| DISPLAY5 | 1920 × 1080 | −3840, 0 | 100% | Connected, unassigned |
| DISPLAY2 | 1920 × 1080 | −5760, 0 | 100% | Connected, unassigned |

Available adapters: Intel Graphics (32.0.101.8628) and NVIDIA GeForce RTX 5090
Laptop GPU (32.0.16.1074). Windows power scheme: High performance. This inventory
is not a claim that either particular adapter rendered the test; its warn-level
log does not name the selected adapter. Raw provenance is retained locally at
`target/issue-1449/four-display-hardware.json`.

## Results

`cargo test --features host --test native_bridge_accessibility -- --ignored --nocapture`
passed **1 test, zero failed/ignored**, 1.16 seconds. Winit reported four monitors.
Two Station panes on `DISPLAY1@1920x1200` each had a 768 × 960 logical content box.
Both preserve the model's console headroom from 1× to 1.5× text scale; the focus
order was Ada, Grace. The real OS query reported text scale 1, contrast off and
reduced motion off. This verifies current OS reading, not changing or adopting
preferences visibly in an embedded console.

`cargo test --features host --test native_bridge_displays -- --ignored --nocapture`
initially failed twice: the adapter had already adopted its unconfigured default
before the fixture published its dynamically discovered authored profile. The
retained timeout diagnostic showed only a 1280 × 720 Windowed primary window.
The fixture now runs before `BridgeDisplaySet`, matching production's boot-time
profile availability. Timeout and geometry assertions were unchanged; no
production display code changed.

The corrected run passed **1 test, zero failed/ignored**, 1.12 seconds: viewscreen
`DISPLAY3@1920x1080` was borderless fullscreen at 1920 × 1080, and Station
`DISPLAY1@1920x1200` was borderless fullscreen at 1920 × 1200 with one pane covering
it. The other two monitors were intentionally unassigned by this fixture.
The native windows closed themselves when verification completed. Full Clippy
with the required feature matrix and formatting passed after the fixture edit.

Logs: `target/issue-1449/four-display-accessibility.log`,
`four-display-windows.log`, `four-display-windows-diagnostic.log`,
`four-display-windows-ordered.log`, and `four-display-clippy.log`.

## Still human / out of scope

Visible text reflow, focus cues, contrast and motion adoption, real keyboard,
mouse or touch traversal, legibility at bridge distance, re-tile blink and
physical unplug/replug have not been accepted by a person. The fixture checks
one viewscreen/Station pair, not every four-monitor arrangement. The #1422
shared 200% tracer dependency remains; these results do not claim completed T3
support or close #1127/#1128.
