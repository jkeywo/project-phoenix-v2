---
title: Issue #549 — E3 Per-kind helm AI plugin
type: source
tags: [ai, helm, operate-helm-ai, prd-520, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/549
status: shipped
updated: 2026-06-25
---

# Issue #549 — E3 Per-kind helm AI plugin

PRD #520 slice E3. Adds `operate_helm_ai` Bevy system and `last_helm_intent` field to `AiControllerComponent`.

## Changes

- `AiControllerComponent` gains `last_helm_intent: Option<(f32, f32)>`
- `tick_ai_controllers` sets `last_helm_intent` from the `AiInput::Helm` output
- `operate_helm_ai` runs after `AiTickLabel`; reads `last_helm_intent` and writes `LastHelmInput`
- Deleted `HelmAiController` constant Resource (was `thrust: 0.5, steering: 0.0`)

## Files

- `src/ship_plugin.rs`, `src/ai/server.rs`

## Cross-references

- [PRD #520](./prd-520-ai-ship-unification.md) — parent
- [AI Ship Unification](../concepts/ai-ship-unification.md)
