# Docked Workshop and GM acceptance — #1514

Status: **not run by a human**. Run alongside [#1487](1487-m6-golden-workflow.md)
on one recorded revision after its implementation prerequisites pass. This is a
verification gate: send a failed behavior back to its owning implementation issue.

## Setup and evidence

Use one browser and one native Ultralight workspace, plus a disposable connected
GM session. Record operator, commit, OS, runtime version, display dimensions,
text scale and input devices. Use `raw/GM Workshop Shell.html` as the hierarchy
and interaction reference; judge Phoenix styling and accessibility rather than
pixel identity. Store screenshots/video and a completed copy of these tables.

## Inventory

Record every panel offered by the running build, including panels initially
closed or hidden by a role. Check that each panel remains available in the
appropriate context after reset. The runtime registries are the inventory source:
`gui/workshop-layout-model.js`, the Test layout registry, and the Live GM dock
registry. Include later-added panels instead of restricting the run to this list.

| Context | Required coverage | Browser | Native |
|---|---|---|---|
| Authoring documents | Exact Source, Rhai, captured model/entity/celestial preview | Not run | Not run |
| Authoring tools | Files, Changes, inspector, Add, Recovery, Findings, Feedback, Dependencies, Settings, Models, Sound, Definitions, Composition, Entity/Ship/Spatial forms, Presets and Localisation | Not run | Not run |
| Test documents | Disposable Viewscreen, including GM and ship view selection | Not run | Not run |
| Test tools | Launch/clock controls, traces and breakpoint result, role/widget preview | Not run | Not run |
| Live documents | Map and authentic Station console panels | Not run | Not run |
| Live tools | Every offered operational panel; five Live Inspector domains, action forms, roles/widgets, private audio/settings and correlated feedback | Not run | Not run |

## Layout operation

For each context on each runtime, perform and record:

1. Tab between documents and tools. Split horizontally and vertically. Float,
   move, resize and dock a panel. Close it, reopen it from the panel selector and
   reset the layout. The original contents and pending edits must survive moves.
2. Repeat without a pointer. Focus a tab/header and use its controls;
   `Ctrl+Shift+Arrow` docks toward the adjacent panel and adding `Alt` tabs it.
   Move splitter and floating-size handles with their keyboard controls. Focus
   must stay visible and usable after moves, close/reopen and reset.
3. Give Authoring, Test and Live visibly different layouts. Switch contexts,
   reload and relaunch. Each restores only its own placement. Launch values,
   selected runtime instances, action form payloads and old simulations must
   not reappear from profile layout data.
4. In a disposable profile, replace a layout with an obsolete or malformed
   record, then reload. Verify safe defaults/migration, no duplicate or missing
   mandatory documents, and no revived runtime state. Preserve the record as
   evidence and restore/reset that disposable profile afterward.
5. Narrow the window, then repeat at 200% text. Fixed menu/toolbar bars and one
   selected panel must remain usable. Switch panels and widen again; the desktop
   arrangement and form content must survive without clipped controls.

| Context | Pointer | Keyboard/focus | Independent restore | Invalid data | Narrow/200% |
|---|---|---|---|---|---|
| Browser Authoring | Not run | Not run | Not run | Not run | Not run |
| Browser Test | Not run | Not run | Not run | Not run | Not run |
| Browser Live | Not run | Not run | Not run | Not run | Not run |
| Native Authoring | Not run | Not run | Not run | Not run | Not run |
| Native Test | Not run | Not run | Not run | Not run | Not run |
| Native Live | Not run | Not run | Not run | Not run | Not run |

## Preserve workflows while moving panels

- Leave a source edit and a validation diagnostic visible; move both panels,
  follow the diagnostic, undo, recover and save/export. Repeat while auditioning
  media and while a captured preview is loading. No move may replace the draft.
- Pause Test, dock/float its Viewscreen and controls, step once and verify one
  tick. Stop and reload: placement remains, the simulation does not. Native Test
  must appear in its document, not solely in a separate child window.
- In Live, move a selected projection and authentic Station console. Verify
  equal-GM information, role filtering, private audio and typed action feedback
  survive. Close a console and confirm its retained panel cannot send commands.
- Open Spawn and each other complex action. Check floating default, docking,
  Keep open, dirty-discard confirmation, success/refusal and safe field reset.
  Refusal must keep the useful current values and readable reason; a later
  action must not inherit stale object authority.
- Start directed and default spatial placement with floating panels present.
  Check the appropriate panels hide, then restore on completion and Escape with
  focus returned to a usable originating control. Repeat with keyboard input.
- Check forced colours and non-colour status distinctions. Record structured
  readings for each canvas and unavailable hardware separately.

## Sign-off

Attach evidence and link every defect to the owning prerequisite of #1514.
Record **passed / failed / unavailable** for each inventory and behavior row,
with actual observations. Keep #1514, #1487 and parent #1469 open while human
coverage or prerequisites remain unresolved. Automated smoke results are
supporting evidence and never substitute for an operator's recorded run.
