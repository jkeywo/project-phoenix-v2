# Physical acceptance kit — #1323, T2 input and feedback

**Prepared; not run.** This kit does not close #1323 or claim a physical result.
A human records the observations and explicitly accepts or rejects the platform
exit after running it. A passing automated suite is supporting evidence, not a
substitute for two real controllers, visible focus, readable feedback or a
physical disconnect. Use a disposable mission and copies of profiles/saves.

## 0. Candidate, people and equipment

Copy the report template in §9 before starting. Record the exact integrated
commit, clean/dirty state, bundle/build identification, and native binary version
if used. Do not run this against the documentation branch merely because it
contains this file; require the final integrated T2 candidate.
The integrator supplies one built candidate and its validation evidence; this
kit does not request duplicate builds or a second concurrent Cargo process.

- [ ] The candidate includes #1277–#1288, #865, #1315, #1321/#1322 and #1316,
      or each missing prerequisite is recorded as **Blocked**, not passed.
      In particular #1316 must retain human Station ratings at fleet boot, and
      #1315 must supply the shared GM confirmation Settings and captured intent.
- [ ] Record links to the actual automated reports, their tested revisions and
      remaining failures. Do not infer that the final candidate passed because
      an earlier isolated worktree did.
- [ ] Two physical standard-mapping gamepads, labelled **A** and **B** on the
      devices, with manufacturer/model, connection type, firmware if known,
      and the browser's displayed device names. Include identical-model pads
      if that is the intended deployment; device-name equality is worth testing.
- [ ] Two independently stored private operator profiles, **Operator A** and
      **Operator B**, on separate browser profiles or devices. Two ordinary tabs
      sharing storage do not prove independent profile persistence. Record the
      browser/profile/window owning each surface and which pads it can see.
- [ ] A keyboard, pointer, operator and observer; both participant surfaces
      visible together. Keep both pads connected throughout the main ownership
      pass. Record the focused window when OS focus limits background input.
- [ ] A ship host, a second ship host, and two equal GM peers in a disposable
      fleet. Join with the displayed fleet code and ordinary crew joins/Station
      claims. Use the prepared two-ship Combat Test variant described below;
      ordinary `assets/worlds/combat_test.toml` has only one GameStart ship slot.
      Record the generated world/hash, seed, hulls and selected Station ratings.
      Use another shipped world/hull when its authored equipment or dialogue is
      required below. Enter play through normal readiness. No console commands
      granting seats, fake gamepads or direct simulation mutation count as
      physical evidence.

Prepare the same reviewed two-ship variant used by [#1320](1320-gm-live-event.md)
from the actual integrated Combat Test, after its GM authoring and the #1320
preparation increment have been adopted. Before building the event bundle:

```sh
node scripts/prepare-gm-live-event.mjs
node scripts/prepare-gm-live-event.mjs --check
```

Retain the generator's source, slot and output hashes. It writes
`assets/worlds/prepared/gm_live_event_two_ship.toml` and
`assets/scenarios.gm-live-event.toml`, preserving the ordinary world and base
catalogue. These outputs are excluded from Git; regenerate after authoring
changes. The normal Trunk/client build copies the prepared assets. Serve the
candidate and open both ship hosts at
`/?manifest=assets/scenarios.gm-live-event.toml`, selecting the offered scenario
and Alliance Cruiser hulls. Verify the served world and manifest against the
recorded hashes. A second joined host alone does not supply a second Fleet hull.

**Native topology precheck: pending.** Before the human event, require an actual
PASS from the existing #1320 precheck on the candidate's relevant source/content.
The integrator supplies its guarded execution receipt and determines whether
existing evidence still applies; this kit does not request a duplicate build.
The exact test selection is:

```sh
cargo test --features headless --test gm_live_event_precheck prepared_event_has_two_authored_fleet_hulls_and_equal_peer_digests -- --ignored --exact --nocapture
```

Execution stays within the shared compiler reservation. Retain the true exit 0,
**one passed / zero failed / zero ignored**, and the `GM_LIVE_EVENT_PRECHECK`
JSON emitted after the assertions. It checks two distinct Fleet hulls at their
authored positions, different LocalShip projections, and equal advancing ticks
and digests through the ordinary two-peer headless/mesh path. A queued command,
zero-match result or successful compilation is not a PASS. Record the tested
revision and content hashes; an earlier isolated result does not by itself
validate the final candidate. Missing or failed precheck evidence leaves this
setup **Blocked**. Its Backfill crews do not replace the ordinary human joins,
Station claims, physical devices or observations required by this kit.

Labels below are descriptive; current unapproved UI copy may have draft square
brackets. Record the exact visible label and action when they differ. If a
control is absent on a hull, change to a suitable shipped hull/world and record
it; an absent control is not a successful test.

## 1. Explicit ownership before flight

1. Open **Settings → Controls** for Operator A. In its gamepad selector choose
   physical A explicitly. Repeat for Operator B, choosing physical B. Identify
   each by operating it while the other lies untouched; do not trust connection
   order alone. Record the selected labels and status on both profiles.
2. With both devices visible to the relevant runtime, press the same button on
   A and B separately. Only the profile that selected that pad may dispatch its
   action. Where windows require focus, focus each in turn and repeat using
   **both** pads against that window. Background inactivity alone does not prove
   ownership. Exercise simultaneous input with both live surfaces where the
   runtime supports it; record that capability and any OS restriction.
3. Reload/reopen each profile, keeping both controllers connected. Verify its
   choice is retained or explicitly requires selection; it must never silently
   adopt the other pad. Keyboard operation must remain available.
4. If the selector reports unsupported mapping or no usable Gamepad API, record
   **Blocked** with runtime details. Do not emulate the missing hardware route.

Evidence: screenshots of both selectors, A/B physical labels and a short video
showing which action responds to each pad. Record any accidental dual dispatch
or transfer as a defect even if the resulting action is harmless.

## 2. Remap the two slots and exercise conflicts

1. On a live Captain console, choose the visible **Red Alert** action in
   Controls. Record its two slots. Put a keyboard key with a modifier in slot 1
   and a button from the selected gamepad in slot 2. Return to the console and
   exercise each once, observing the same action/feedback and authoritative state.
2. Bind another action in an overlapping context to that same keyboard chord.
   The conflict dialog must identify the existing action/slot. Choose **Cancel**:
   both prior mappings survive. Repeat and choose **Replace**: the conflicting
   assignment clears and only the replacement fires. Repeat with a pad binding.
3. Choose actions whose displayed contexts do not overlap; reuse a binding and
   switch the actual console context. Only the active context acts. If the
   catalogue has no suitable pair, record the pair needed and leave this blocked.
4. Attempt a reserved browser/OS chord in the remapper. Record the actual chord
   and refusal or interception. A browser shortcut taking focus is not evidence
   that the application accepted it. Restore one action, then the complete
   Controls defaults using the displayed reset controls; confirm the scope of
   each reset before activating it.
5. While a text field or remap capture has focus, type the chosen gameplay key.
   It must not issue a gameplay command. Escape cancels capture/dialog and visible
   focus returns to an appropriate control. Repeat using keyboard-only navigation.

## 3. Analogue and physical loss/recovery

Use Helm with an authored continuous control and a safe open-space heading.
Choose a continuous axis binding, then a discrete axis-direction binding on a
separate suitable action. Do not replace these with two synthetic button events.

1. Move the stick through centre, partial travel and full travel. Observe
   proportional thrust/steering, return to neutral, then adjust the exposed
   deadzone and inversion controls. Small drift inside the deadzone must not
   move the control; inversion changes the direction deliberately. Record values.
2. Cross the discrete direction threshold, hold, return below it and cross again.
   Record the activation count and any repeated dispatch; compare with the
   action's declared hold/repeat behavior rather than assuming all actions repeat.
3. With A applying a non-neutral continuous input, physically disconnect A.
   The owner must report loss and stop its live input; B must not take over.
   Confirm the commanded control returns neutral rather than requiring the ship
   to stop instantaneously (inertia may remain). Operate B on its own surface
   and operate A's surface with the keyboard while A is absent.
4. Reconnect A while its stick/button is held. It must wait for neutral input
   before resuming. Release to centre, then deliberately actuate it. Record
   whether explicit re-selection is needed; silent substitution of B is failure.
   Repeat with the ports/connection order swapped and with B disconnected.
5. Switch console/context and focus while holding a control. Release it after
   switching. The old control must receive its neutral/release and not latch on;
   an unrelated newly focused control must not inherit the held value.

## 4. Migrated-family coverage matrix

For **every row**, record world/hull, Station, visible action, keyboard slot,
selected pad/slot, actual outcome and evidence. Perform at least one action with
keyboard and one with each owned pad, assigning the profile its ordinary Station
as necessary. Use the visible Settings catalogue rather than assuming defaults.
A disabled or unavailable action does not replace a positive exercise.

| Family / shipped adapter | Concrete exercise and prerequisite |
| --- | --- |
| Captain | Toggle Red Alert, change the viewscreen; adjust a visible Objective priority when authored. Confirm feedback and resulting state. |
| Helm | Partial thrust/steering and neutral; exercise a discrete impulse or boost control only where fitted. Apply §3. |
| Comms | Hail an existing in-range contact, select its message, choose an authored reply and show it on screen; clear eligible history. Use a shipped dialogue world and record its path/contact. Selection is local; reply/hail are authoritative. |
| Tactical | Cycle/select a real target; use a fitted weapon's ordinary fire/load/selection control in Combat Test. Record weapon System and actual readiness. |
| Sensors | Select a real contact, start a scan and observe completion/refusal; use its viewscreen control where exposed. |
| Science / shield focus | On a hull exposing this family, change the selected shield facing/focus and verify the authoritative result. |
| Navigation | Navigate the chart with keyboard, select a contact, place/clear a waypoint. Exercise a civilian order only where an authored eligible recipient exists. |
| Power | Increase/decrease an authored group's allocation and observe resulting values. |
| Internal Repair | Dispatch/recall a real team and prioritize a named damaged System; use ordinary combat damage as the prerequisite. |
| Engineering equipment | Engage/release Tractor, start/stop Umbilical and dispatch/recall external repair on a fitted hull with a valid nearby target. Record each, not just one toggle. |
| Composite consoles | On Courier Captain or another shipped composite, move between its Captain/Comms/Power/Repair controls. Verify the same binding acts only in the active subcontext and returns held axes to neutral. |
| Client chrome | Open/close Settings and navigate its tabs with bindings and focus; do not dispatch underlying Station actions. |
| Host / GM | Show/hide the host join code; exercise GM pause/resume and a real Station interface action after explicit takeover. Repeat with the second equal GM and record separate attribution. |
| Existing editor/mod workflow | In the shipped editor, import a disposable supported file, edit a field, validate and export it using the exposed semantic controls; retain validation output. Do not invent a new Workshop surface. |

Source coverage: [Station adapters](../../gui/stations/),
[client actions](../../gui/client-semantic-actions.js),
[host actions](../../gui/host-actions.js),
[GM actions](../../gui/gm-session-actions.js),
[existing editor actions](../../gui/editor-mod-actions.js).

## 5. Feedback and shared confirmation

Keep an observation row for **Pressed, Pending, Applied, Refused and Timed Out**.
Capture slow-motion video if Pending is too brief to read; do not claim it was
visible merely because a terminal state appeared. A local selection can complete
Applied immediately; it should not invent a network Pending step.

1. Use a valid command from §4 to observe authoritative Applied. While the first
   GM holds a confirmation open, have the second GM remove or invalidate that
   exact target through normal controls. Accept the original captured request:
   observe ordinary Refused/status/activity, with the original operator and target.
2. In a disposable run, interrupt the issuing surface's real connection immediately
   after an authoritative command enters Pending. Record the interruption method,
   timing and resulting state. Keep it interrupted beyond the visible timeout.
   If it produces Refused/disconnected instead of Timed Out, record that result;
   leave Timed Out unverified until a real pending-without-terminal case is
   captured. Do not edit feedback state or inject a fake acknowledgement. Rejoin,
   reconcile the actual state and record any late result; timeout alone never
   proves the simulation did not apply a command.
3. On GM A, set one destructive category to **Confirm + Preview** in Settings;
   leave GM B's corresponding private policy unchanged. Use directed damage on a
   disposable target. Record the captured target/scope and preview, cancel once
   (no command/attributed result), then accept once (one ordinary result).
4. The confirmation is modal and traps focus; do not bypass it to operate the
   same GM's map. Have the second GM move or invalidate the captured target
   through normal controls. Record the first GM's unchanged captured description
   and target UUID; after acceptance, match the ordinary result to that UUID.
   For removal/invalidation, require the ordinary Refused result from §5.1.
   Exercise Immediate,
   Confirm and Confirm + Preview; record a policy change affecting only its owner.
5. While a held Station input is active, open a different GM confirmation and
   release the held control. The release must still reach its ordinary consumer;
   accepting the other dialog must not restart the held input.
6. Record textual/icon/status feedback without relying on color, accessible status
   announcement where available, and optional vibration on supported hardware.
   Unsupported vibration must be reported as unsupported, not passed or failed
   because new sound/vibration production was assumed.

## 6. Private profile transfer

Export A and B through **Controls → Operator Profile → Export Profile**. Preserve
original files before importing. On a separate browser profile, import A's file,
then export again: compare bindings, gamepad preference/tuning, accessibility,
feedback choices and GM confirmations. Check inactive console-family bindings
survive. Return to B and verify its settings remain unchanged.

Repeat the export/import through the intended native surface if it exposes the
profile controls; record native host/embedded runtime versions. Missing native
capability remains an explicit blocked leg, not an inferred browser success.
A transferred preferred pad may need explicit selection on the new machine;
it must not silently operate the other physical controller.

Inspect the exported JSON: no player identity/token, Station ownership, session
state or saves. Import a **copy** with an unsupported version, then a malformed
file. Observe refusal and unchanged settings; never overwrite the originals.
Record migrations/normalization notices rather than claiming byte equality when
the documented importer changes supported legacy fields.

## 7. Peer-local saves

Use two simulation peers (ship/GM), not a phone console without simulation.
Open each peer's save controls/catalogue; record baseline rows and storage origin.
On a browser viewscreen the in-session save controls are in the settings cog's
Gameplay tab rather than on the viewscreen itself; the pre-session catalogue is
still the landing's **Load Game** route, and a GM desk still carries the same
panel in its roster.
Create a uniquely named manual save on A at the next deterministic capture tick;
observe local completion and confirm B acquired no matching manual row. Rename,
export and delete only that disposable A row with the ordinary confirmation.
B's catalogue and the live mission must remain unaffected. Record autosave rows
and their capture ticks separately from the manual-save operation.

End the disposable mission through normal controls. From a compatible retained
local save, use the catalogue's **Start** to begin a new session; verify the
restored scenario state. Do not attempt to restore over the live fleet: live
restore is outside this T2 contract. Record an incompatible save's normal refusal
if such a real older artifact is available; otherwise leave that case unverified.
Profile import/export must neither move nor delete either peer's save catalogue.

## 8. Accessibility and supported surfaces

Repeat key steps with keyboard only: visible focus through Settings, conflict
prompts, confirmation accept/cancel, save controls and the console. Check focus
returns on close and cannot disappear behind the modal. Increase browser zoom
and text size, test reduced motion/high contrast where the runtime supports them,
and distinguish all states without color. Record viewport/zoom and screenshots of
any clipped control. On a large landscape tablet, repeat representative touch
selection and confirm/cancel if that is a deployment target; phone-portrait GM
layout is outside T2. Screen-reader announcements and touch each need actual
hardware/software evidence; absence is recorded explicitly. For native input and
OS preference limitations consult [#1124](1124-input.md) and
[#1128](1128-accessibility.md); do not copy their historical verdicts into this run.

## 9. Retained human report

Copy this template into the run's evidence folder. Leave the original kit intact.

```text
#1323 physical acceptance report — NOT RUN / IN PROGRESS / COMPLETE
Date/time/timezone:
Operator and observer:
Candidate commit / dirty diff attachment:
Host bundle identity/hash / native binary identity/hash:
OS/build; browser/version; native/embedded runtime/version (or not tested):
Machines, displays, resolution/scaling; network topology and origin URLs:
Pad A model/firmware/connection/displayed ID:
Pad B model/firmware/connection/displayed ID:
Profile A storage/browser owner -> selected physical pad:
Profile B storage/browser owner -> selected physical pad:
World/hull/seed; Station ratings; two GM identifiers:
Prerequisite #1316/#1315 and automated evidence links + exact revisions:
Prepared world/manifest source and output hashes; served-asset check:
#1320 native precheck revision, true exit/test count and JSON evidence (pending until run):

One row per numbered step AND per family matrix row:
Case | surface/profile/pad | action/contexts/bindings | setup | expected |
observed | Pass/Fail/Blocked/Not run | timestamp/evidence | defect/follow-up

Feedback observations: Pressed / Pending / Applied / Refused / Timed Out
Accessibility observations and unsupported capabilities:
Profile files (redacted copies if needed) and before/after comparisons:
Local save names/capture ticks/artifact paths and second-peer comparison:
Defects: reproduction, expected/observed, severity, evidence, owner/follow-up
Remaining blocked/not-run cases and required equipment/candidate changes:

HUMAN T2 PLATFORM VERDICT: ACCEPT / REJECT / DEFER
Reason and any explicitly accepted limitations:
Decision-maker name/signature and date:
#1323 issue report/link (only after authorized human publication):
```

An **Accept** requires evidence for all mandatory physical ownership, migrated
families, feedback outcomes, confirmations, profile transfer and save cases.
Missing mandatory evidence means **Defer**, or **Reject** for a demonstrated
failure; it is never an implicit pass. Preserve defects and explicit human
judgment together. This document's preparation and review do not supply that
judgment or authorize closing/posting to the issue.

Design/source anchors: [T2 platform contract](../../pasm/spec/design/t2-platform-input-feedback.yaml),
[gamepad ownership](../../gui/gamepad-input.js),
[remapper](../../gui/semantic-controls-remapper.js),
[private profile](../../gui/operator-profile.js),
[GM confirmations](../../gui/gm-confirmation.js),
[save catalogue](../../gui/save-slots.js).
