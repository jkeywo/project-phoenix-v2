# Issue 1488 — Workshop shell retirement evidence

The independent World Editor and Model Viewer applications are retired. This
record separates automated parity from the human observations deferred with
issue #1487.

## Automated parity

- `editor.html` and `viewer.html` contain only accessible migration copy and a
  redirect module. They cannot boot the removed editor application or a second
  WASM renderer.
- Legacy file/model/entity/variant/lighting/gizmo selections are parsed by
  `editor/workshop-launch.js`, bounded to authored asset paths and passed into
  the same Workshop boot path in browser and native builds.
- `start-editor.bat`, `start-viewer.bat` and `npm run dev:viewer` launch native
  Workshop against the current project. npm accepts only validated exact
  selectors. The Windows launchers expand no arguments; an optional bounded
  selection travels in `PHOENIX_WORKSHOP_OPEN` and is read directly by Node.
- Workshop's existing focused suites cover exact-source editing, grouped
  history, recovery, save/export, entity and world composition, Rhai, structured
  model/rig authoring and captured preview. The migration suite additionally
  exercises old bookmarks and built redirect dependency closure.
- The `viewer` Cargo feature remains a shared renderer boundary. Focused tests
  and the server-off build keep `ViewerPlugin`, LOD selection, celestial/GLB
  rendering and render statistics usable without restoring an independent UI.

## Human observations still required by #1487

No automated result is recorded as a human observation. A person with the
native Workshop and representative GPU/display/accessibility setup still needs
to observe:

- GLB, rig, entity, star and planet appearance under the retained lighting,
  camera, gizmo and LOD controls;
- LOD/remesh and billboard tool progress, cancellation and adoption with the
  configured external tools;
- keyboard focus, 200% scaling and forced-colour/non-colour presentation;
- recovery and save behavior against a real chosen project root; and
- the five Live Inspector workflows and stale/refused feedback called out by
  the M6 golden workflow.
