---
title: Shields Intent
type: concept
tags: [shields, ai, damage, focus, pasm]
sources: [src/ship/shields.rs, src/console_ai/core.rs, 3c5eca9e, 122e9f4d]
updated: 2026-08-13
---

# Shields Intent

AI-controlled Shields should focus the arc taking concentrated recent incoming
damage. It uses an authored timing window and concentration threshold, then
falls back to focusing a disproportionately weak arc; otherwise it clears
focus. Human Shields retains exclusive focus control whenever the system is
human-controlled.

Commit `59bf07c8` introduced this for player and NPC ships, but its
damage-history comparison was defective: it stored damage deltas and later
treated the latest delta as the previous arc HP. Issue #747
(`3c5eca9e`) corrected it: `tick_shield_focus_ai` now scores concentration over
timestamped per-arc damage records within an authored `damage_window_secs`
window (floored at `min_damage_window_secs`), rather than comparing deltas.
Issue #783 (`122e9f4d`) then folded shields into the authored channel/verb
policy shape alongside the other AI hosts — arcs stay a fixed 4-set of
in-ship indices rather than a variable candidate set, which is why Shields did
not move to the #785-style `TargetSelector` used by Navigation/Repair. The
policy is implemented and authored, not partially implemented. The separate
open Sensors-to-Shields `ThreatBearing` proposal is not part of the accepted
focus policy.

## Sources

- `src/console_ai/server.rs:213` (`ai_shield_focus` — the shield-focus AI decide-and-emit system)
- `src/ship/shields.rs:331` (`handle_shields_messages` — applies the admitted `SetShieldFocus` from human and AI alike)
- `src/console_ai/core.rs:422` (`tick_shield_focus_ai` — the pure focus-policy decision)
