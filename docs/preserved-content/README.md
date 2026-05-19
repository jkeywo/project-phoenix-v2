# Preserved World Content

These TOML files were originally authored under `assets/maps/` and `assets/scenarios/`, then briefly moved to `assets/worlds/` during the world-merge work. They are **not loadable by the current engine** — they rely on schema that has since been removed or never landed:

- `LoadScenario` / `UnloadScenario` trigger actions (scenario chaining, removed in the world-merge refactor)
- `set_flag` / `flag_is_set` conditions
- `force_ai_state`
- `on_attacked_by`
- `comms.response.actions`

They are preserved here as authored design content until either:
(a) the engine grows the missing features, or
(b) someone migrates them to the supported schema (which would lose the mutual-path-exclusivity design that depends on scenario chaining).

Per PRD #341 these files were moved out of `assets/` entirely — preserved authored content does not belong in the loadable-asset tree.

## Files

| File | Origin | Notes |
|------|--------|-------|
| `axiom_system.toml` | Original star-system map for the "Before the Fire" scenario; defines `[[star]]`, `[[planet]]`, `[[asteroid_field]]` plus patrol anchors. |
| `before_the_fire.toml` | Main scenario file. |
| `btf_path_a.toml`, `btf_path_b.toml`, `btf_path_c.toml` | Three mutually-exclusive narrative paths chained via `LoadScenario`. |
| `btf_aphelion_protocol.toml` | Climax scenario. |
| `btf_sidequest_courier.toml`, `btf_sidequest_rescue.toml` | Side quests. |

See also `docs/scenarios/before_the_fire.md` for the design notes.
