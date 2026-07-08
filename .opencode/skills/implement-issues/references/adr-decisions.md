# Web Component Console Architecture — ADR Decisions

(From grill-me session on 2026-07-08, for issues #643-#669)

## Core Pattern

- **File location:** `gui/components/ph-{name}.js`
- **Template:** Inline in constructor via `innerHTML` on `<template>` element (no lazy module variable)
- **Shadow DOM:** `this.attachShadow({ mode: 'open' })` in constructor
- **State API:** `set state(val)` → stores in `this.#state`, schedules rAF-debounced `#render()`
- **Private members:** `#state` private field, `#render()` private method
- **Registration:** `customElements.define('ph-{name}', Ph{Name})` (no `typeof` guard)
- **sendAction:** `window.sendAction` global set by `initConsole()`; component reads `this.sendAction ??= window.sendAction` in `connectedCallback`; guard with `if (this.sendAction)`
- **Layout:** Component owns internal layout via Shadow DOM `@media`. Console HTML assigns position/size only.

## Existing Components (#645)

- `ph-damage-bar` and `ph-damage-detail` used `.data` + `observedAttributes` — migrate to `.state` + `#render` pattern when moving to `gui/components/`
- `ph-sensor-panel` and `ph-shield-panel` already use `.state` — migrate to `#render` private method and inline template

## Testing Pattern

- Construct element via `document.createElement('ph-{name}')`
- Set `.state = { ... }`
- Assert DOM output (shadowRoot contents)
- For action-dispatching components: mock `element.sendAction` and assert it was called
