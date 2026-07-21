---
title: Shields Intent
type: concept
tags: [shields, ai, damage, focus, pasm]
sources: [src/ship/shields.rs, src/console_ai/core.rs, 59bf07c8]
updated: 2026-07-14
---

# Shields Intent

AI-controlled Shields should focus the arc taking concentrated recent incoming
damage. It uses an authored timing window and concentration threshold, then
falls back to focusing a disproportionately weak arc; otherwise it clears
focus. Human Shields retains exclusive focus control whenever the system is
human-controlled.

Commit `59bf07c8` introduced this implementation for player and NPC ships. The
current damage-history comparison is defective: it stores damage deltas but
later treats the latest delta as the previous arc HP. The intended policy is
therefore recorded as partially implemented until that state representation is
corrected. The separate open Sensors-to-Shields `ThreatBearing` proposal is not
part of the accepted focus policy.

## Sources

- `src/console_ai/server.rs:192` (`ai_shield_focus` — the shield-focus AI decide-and-emit system)
- `src/ship/shields.rs:285` (`handle_shields_messages` — applies the admitted `SetShieldFocus` from human and AI alike)
- `src/console_ai/core.rs:471` (`tick_shield_focus_ai` — the pure focus-policy decision)
