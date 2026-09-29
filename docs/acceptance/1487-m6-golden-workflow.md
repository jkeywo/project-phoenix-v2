# M6 golden workflow — #1487

Status: **not run by a human**. This kit prepares the exit for #1469; automated
tests and an agent operating a browser do not fill its observation cells.
Run the browser and native legs on the same commit after #1475 and #1500 land.
The coordinated M5 recovery exit [#1448](1448-m5-recovery.md) remains required.

## Record the run

Record commit, operator, date, OS, browser/version, native binary build flags,
display size, text scale and input devices. Save screenshots, exported ZIP and
before/after source diffs beside the run record. Each row receives **passed**,
**failed**, or **unavailable**, with an observation and evidence path. An
unavailable device is an explicit coverage gap. Link every failure to its owning
implementation issue before signing off; this checklist does not absorb defects.

## Prepare disposable workspaces

1. Build the browser host and client with `trunk build` and
   `node scripts/build-client.mjs`. Serve `dist/` using
   `node scripts/serve-workshop.mjs --no-open`, then open its reported Workshop URL.
2. From the repository root, generate the existing minimal test pack:

   ```powershell
   node --input-type=module -e "import fs from 'node:fs'; import { workshopPack } from './tests/fixtures/workshop-pack.js'; fs.writeFileSync('workshop-acceptance.zip', workshopPack());"
   ```

   Move the generated ZIP to the run's evidence directory. The browser operator
   imports only that ZIP; grant no project-directory access. Dependency origins
   must remain read-only.
3. For native, use a disposable **copy** of a project or extracted mod, outside
   the working checkout. Use an Ultralight-enabled `phoenix-host` and the same
   built `dist/` bundle:

   ```powershell
   .\target\release\phoenix-host.exe --client-dir dist --workshop-project C:\Temp\phoenix-m6-project
   ```

   Substitute the actual copied root. The mod variant is `--workshop-mod <root>`
   with `--content-dir <base-content-root>`. Record the selected root and take
   hashes of a sibling directory to verify it stays untouched. Do not save this
   acceptance run into the production project.

## Author and preserve exact source

Run each row in browser and native. Keep a comment and an unknown field near
each edited value as sentinels. Inspect Changes and Source after Apply, Undo,
Redo, recovery and final persistence. Only the intended source bytes may change.

| Step | Action and expected observation | Browser | Native |
|---|---|---|---|
| Project lifecycle | Import/open the selected workspace; inspect immutable base and retained-pack origins. Refuse an invalid replacement without losing this draft. | Not run | Not run |
| Scenario | In `assets/worlds/workshop.toml`, add an available ship and game-start player entity using `assets/entities/alliance_cruiser.toml`. Edit a global scalar and an anchor through the forms. Verify Source and grouped Undo. | Not run | Not run |
| Composition | Create a child world, add/remove it in Composition, and edit a script load/unload dependency through the structured controls. Change a manifest root. Missing paths and a cycle must refuse without a history entry. | Not run | Not run |
| Entity/Ship | Add a local entity, include a fragment, inspect field ownership, deliberately materialize one inherited field, and add/remove a supported component. An invalid include must leave the draft untouched. | Not run | Not run |
| Rhai | Create an inline script, edit a sibling script, introduce a compile error and follow its source diagnostic; repair it. Use a named boolean Flag or counter for the breakpoint leg below. | Not run | Not run |
| Definitions | Create/edit a faction and complexity definition; provoke and repair one reference error with a source location. | Not run | Not run |
| Roles/widgets | Add a role preset, reorder panel assignments and quick actions, add/reorder/remove a typed widget. Invalid widget references must refuse. Preview the unsaved preset; it filters presentation without granting authority. | Not run | Not run |
| Models | Add a supported model plus sidecar; edit rig, marker, variant and LOD structure. Inspect model, entity, star and planet previews. Preserve untouched model bytes. Record unsupported capabilities explicitly. | Not run | Not run |
| Native tooling | Generate LOD/remesh output and billboard captures into the selected workspace; inspect changes and Undo. Browser must describe unavailable native tooling accurately. | Not run | Not run |
| Recovery | Leave several source families dirty, close/reopen, explicitly restore the draft, inspect changes and undo across families. Declining restore must not silently adopt it. | Not run | Not run |
| Persistence | Validate the exact draft, export/reimport the browser ZIP or save/reopen the native workspace. Compare sources and the offered scenario catalogue. Verify the native sibling directory remains untouched. | Not run | Not run |

For the initial scenario, this source fragment can be added after `[anchors]`:

```toml
[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
spawn_on = "game_start"
```

## Disposable Test

1. Change the world title without saving. Select its world, hull and fixed seed
   in Test. Start and confirm the unsaved selection runs with Stations on
   Backfill. Authoring must be held against accidental concurrent editing.
2. Pause, record the tick, Step once and check exactly one tick advanced. Resume
   at each offered rate. Return to Authoring, edit, then restart: a fresh run
   must use the new draft, never silently mutate the old run.
3. Switch between omniscient GM and each available simulated player ship's
   authentic Viewscreen. Check the view changes while tick continuity remains.
   Verify the image is inside the docked Test document on both runtimes.
4. In the draft script, arrange a named Flag/counter change. Set its scenario-state
   breakpoint before launch. Confirm the completed tick is held, condition and
   source are readable, and nearby host-call/Flag/callback traces are available.
   Step and resume without an immediate repeated stop on the same unchanged value.
5. Select and preview the draft role/widgets. Restart and confirm that local
   preview choices do not persist as simulation state.
6. Stop, close the document and restart Workshop. No former run, stale launch
   values or still-running native child may be restored from layout storage.

Record browser and native observations separately for all six steps.

## Connected Live

Use a disposable multiplayer mission with two equal GM operators. Exercise world/
scenario, entities/AI, hull/Stations/Systems, Regions and presentation/audio
inspection. Record baseline, current value, winning source and mutability. Select
an object that disappears: its final reading must be marked stale/gone with
actions disabled. Follow related objects through Back/Forward.

Have both GMs read one NPC doctrine revision. Apply a change from the first;
submit the stale second proposal. Record refusal, first operator attribution and
canonical result across both peers. Exercise the existing typed action instead
of attempting arbitrary source writes. Run the linked M5 recovery workflow and
record its result here. Private audio and operator preferences must remain private.

## Accessibility and completion

Repeat the major actions keyboard-only with visible focus, at 200% text and
forced colours. Check status without relying on colour and structured readings
alongside canvases. Record any OS preference that cannot reach native Ultralight
as unavailable; do not substitute a browser observation for native coverage.

Use [the dock acceptance kit](1514-docked-workspace.md) for the complete panel
inventory and layout lifecycle. Existing editor/viewer entry points are already
redirects: verify supported deep links arrive at Workshop, and record that their
retirement preceded this human exit rather than claiming this run authorized it.

Final disposition: **pending human browser/native operation and #1448 evidence**.
