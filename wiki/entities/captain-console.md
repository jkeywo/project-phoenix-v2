---
title: Captain Console
type: entity
tags: [console, captain, red-alert, view-mode, authority]
sources: [src/console/captain/server.rs, src/core/messages.rs, gui/components/ph-red-alert.js, gui/components/ph-camera-select.js]
updated: 2026-07-14
---

# Captain Console

The Captain station operates the ship's Red Alert system and the local viewscreen mode. Its controls use the normal admitted `ControlSystem` path; station ownership and system control source determine whether a human input is accepted or an AI operates it.

`ToggleRedAlert` changes the addressed ship's `ShipRedAlert`. `SetView` selects a camera, Radar, Navigation Chart, or Cinematic mode for the local ship's viewscreen. The phone UI composes reusable Red Alert and camera-select components and renders the authoritative state.

The Captain console does not own the lobby start transition: the game starts collectively when connected crew are ready. Other station capabilities are defined by the loaded ship configuration; see [Station](./station.md), [Console](./console.md), and [System](./system.md).
