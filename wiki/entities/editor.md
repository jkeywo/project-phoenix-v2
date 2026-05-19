---
title: Editor
type: entity
tags: [editor, tooling, scenario, entities, definitions]
sources: [PRD-350, editor/app-v2.js, editor/project-root.js, editor/world-toml.js, editor/entity-toml.js, editor/mode-shell.js]
updated: 2026-05-19
---

The Scenario Editor is an in-browser TOML editing tool for editing scenario worlds, entity templates, and definition files directly on disk using the File System Access API. It provides a mode switcher (Scenario / Entity / Definitions) that preserves per-mode open file state, and includes FSA-based read/write with IndexedDB root handle persistence. The editor is built as a progressive rewrite (v2) alongside the existing v1 Konva-based editor, with deep ESM modules for testability.
