# Acceptance kit — #1448, the M5 live-event recovery run

**Status: preparation only. No box below is checked, and no human session
has been run or accepted by this document.**

Issue #1448 is PRD #1420's own closing instruction (story 17): *"As a GM, I
want to complete a real live-event recovery acceptance with truthful
failures and preserved seating."* It is the human exit gate for GM console
M5 — Operations and Safety — carrying PRD #1418's usability contract into
that acceptance rather than treating it as a later polish pass. Six T3
issues (#1441–#1447, this branch) each shipped its own feature and its own
exhaustive automated integration coverage. **This document does not
re-derive or duplicate any of it.** It is the single script two real Game
Masters run, once, to exercise the whole M5 workflow together — because the
question #1448 actually asks is whether a live recovery decision is
understandable and trustworthy to a person under time pressure, not whether
each mechanism taken alone has a passing test.

Parent PRD: #1420, story 17; inherited PRD #1418 stories 28–33. Blocked by
#1442, #1443, #1444, #1447, #1430 — all landed on this integration branch
(see §1).

---

## 1. What is already integrated on this branch

Every row below is a landed commit on `claude/t3-prd-sub-issues-65d146`.
This kit assumes their behaviour rather than re-testing it; each issue's own
automated suite remains the authority for *its* feature, and is cited below
so a failure found during this run can be filed against the right owner.

| Issue | Commit | What it added | Its own automated coverage |
| --- | --- | --- | --- |
| #1441 | `cb14b54c` | Canonical `GmActionLog` → `GmJournalProjection` on the existing `gm_session` channel; `gui/gm-journal-panel.js` read-only journal with operator/outcome filters and a detail region | `tests/gm_journal.rs`, `tests/client/gm-journal-panel.test.js`, `tests/smoke/gm-journal-200.spec.js` (200%-scale) |
| #1442 | `b6e3e412` | `GmAction::UndoGmAction` as an ordinary typed action; per-field affected-state tracking (`GmAffectedField::{NpcDoctrine, FactionHostility}`); `UnknownGmAction`/`InverseUnsupported`/`InverseFactsMismatch`/`AffectedStateChanged`/`AlreadyInverted` refusals | `tests/gm_undo.rs` (18 tests), `tests/smoke/gm-undo-200.spec.js` (200%-scale) |
| #1443 | `137bd381` | `GmAffectedField::SpawnedEntity`; `src/gm_exposure.rs` two-cumulative-second sensor-range latch; `SensorExposureElapsed` refusal | `tests/gm_spawn_undo.rs` (22 tests) |
| #1444 | `a554f8ee` | `src/gm_despawn_undo.rs` capture-at-removal via `snapshot::capture_entity_state`/`apply_entity_state`; `RestoreIdentityOccupied`/`RestoreReferenceConflict` refusals; `GmJournalEntry::capture_lost` | `tests/gm_despawn_undo.rs` (12 tests) |
| #1445 | `521d6e09` | `gui/gm-checkpoint-panel.js` named manual capture on the ordinary save path; `gm_checkpoint::confirmed_checkpoint` read-back rule; `CandidateBlock` compatibility preflight shared with the restore control | `tests/gm_checkpoint.rs` (7 tests), `tests/smoke/gm-checkpoint-200.spec.js` (200%-scale) |
| #1446 | `72a0e91f` | `GmAction::RequestLiveRestore` on the canonical journal; single-simulation-peer restore: recovery-checkpoint-first, #1118 load/digest/rollback, #1119 fence, explicit-resume hold; `gui/gm-restore-control.js` | `tests/gm_restore.rs` (18 tests), `tests/smoke/gm-restore-200.spec.js` (200%-scale, shared with #1447) |
| #1447 | `7c9f66ad` | Extends #1446 across every simulation peer: ten-real-second readiness countdown, canonical nonresponder disconnect, all-peers digest agreement, `MeshFrame::GmRestore` (protocol rev 12→13) | `tests/lockstep_gm_restore.rs` (14 tests), `tests/smoke/gm-restore-200.spec.js` (200%-scale, shared with #1446) |
| #1430 | `b0f104ee` | GM directing/performing desk (T2, PRD #930 M1–M3 mission log/Objective list/Knowledge Compare panels only) carried into 200%/forced-colours/dense-list coverage, plus the GM Station-puppet mirroring the GM's own resolved presentation — does **not** touch the journal, checkpoint or restore panels covered by #1441/#1442/#1445/#1446/#1447 above, which are new in T3 | `docs/acceptance/1432-platform-comfort.md` Task 4, `tests/client/gm-directing-performing-dense-content.test.js` |

The confirmation-policy contract those panels sit behind is unchanged by
this issue: `gui/gm-confirmation.js` registers `action.undo` and
`world.restore` at default `confirm-preview`, independently choosable per
operator down to `confirm` or `immediate` (`GM_CONFIRMATION_CATEGORIES`,
`gui/gm-confirmation.js:52,59`) — the same private per-GM setting §5 uses to
exercise "different T2 confirmation policies."

---

## 2. Cheap automated prechecks — run before booking the two GMs

None of these need generated fixtures, an `--ignored` flag, or a build; they
are the existing per-feature suites, and running them fresh on the exact
revision under test is cheap insurance against booking two real people's
time against a regression the ordinary gate would have caught. Record the
revision and the pass counts in §6 before scheduling the human session.

```sh
cargo test --features headless --test gm_undo
cargo test --features headless --test gm_spawn_undo
cargo test --features headless --test gm_despawn_undo
cargo test --features headless --test gm_checkpoint
cargo test --features headless --test gm_restore
cargo test --features headless --test lockstep_gm_restore
cargo test --features headless --test gm_journal
npx vitest run tests/client/gm-journal-panel.test.js tests/client/gm-checkpoint-panel.test.js tests/client/gm-restore-control.test.js
```

Distributed restore (§5.6–5.7) needs two real simulation peers — two
separate ship hosts joined into one Fleet, not two GM tabs on one host. The
ordinary `assets/worlds/combat_test.toml` boots only one `GameStart` ship
slot, exactly as #1320's kit found. Reuse that kit's own preparation
tooling rather than authoring a second one:

```sh
node scripts/prepare-gm-live-event.mjs
node scripts/prepare-gm-live-event.mjs --check
cargo test --features headless --test gm_live_event_precheck \
  prepared_event_has_two_authored_fleet_hulls_and_equal_peer_digests \
  -- --ignored --exact --nocapture
```

Record a true exit 0 and the printed `GM_LIVE_EVENT_PRECHECK` line as done
in `docs/acceptance/1320-gm-live-event.md` §1. It proves two ordinary
headless peers boot the generated two-ship world and fold to the same
digest — the exact precondition §5.6/§5.7 need before a human wastes time
on a restore neither peer can agree on for reasons unrelated to M5. It does
not exercise any M5 mechanism itself.

If any command above fails, do not proceed to §4: file the failure against
the owning issue from §1 and re-run this section after a fix lands.

---

## 3. Prepare the event

Nominate GM A and GM B — two separate humans, not two tabs run by one
person; the whole point of #1442's "both operators recorded" and #1447's
peer-readiness room is that a real second person is making a real decision
under the same time pressure. Nominate an evidence recorder (may be either
GM, but record who).

| Required input | Record before starting |
| --- | --- |
| Integrated build | Full commit, build/content stamp, date, owner |
| §2 precheck results | Exact commands, pass counts, `GM_LIVE_EVENT_PRECHECK` line |
| Scenario (local restore, §5.2–5.5) | `assets/worlds/combat_test.toml`, seed, hull |
| Scenario (distributed restore, §5.6–5.7) | Generated `assets/worlds/prepared/gm_live_event_two_ship.toml`, two Alliance Cruiser hulls, source/generated hashes from §2 |
| Confirmation policies | GM A's and GM B's own `action.undo` and `world.restore` settings — deliberately different (see below) |
| Evidence capture | Recorder, destination, read-only journal/health export method |

Set GM A's confirmation policy for `action.undo` and `world.restore` to
the shipped default `confirm-preview`. Set GM B's to `immediate` for both.
This is deliberate, not an oversight: PRD #1420's Implementation Decisions
say the consequence preview must stay inspectable "even when confirmation
is skipped," and #1418 story 30 makes that a named acceptance criterion.
Running the two GMs on different policies is how this kit actually tests
that sentence instead of assuming it.

Each GM opens their own **Settings → Controls** and confirms the change
took only their own profile (GM A still sees GM B's undo/restore land
without a dialog interrupting *A's* screen, and vice versa).

---

## 4. Read the journal before touching anything

1. With the scenario in progress, have each GM apply one ordinary
   directing action of their own (e.g. GM A sets an NPC doctrine, GM B
   fires an authored event). Both open `gui/gm-journal-panel.js` and locate
   both rows. Confirm each row names the acting operator, the action
   family, its target, apply tick and canonical sequence, and its terminal
   Applied/No-op/Refused outcome — record what each GM could read without
   being told out of band.
2. Confirm neither row exposes a transport identity (host slot, peer
   number, connection token) — the projection deliberately excludes it
   (#1441). Record whether either GM found that absence confusing or
   correct.
3. Filter by operator, then by outcome, then clear filters. Confirm
   selection and keyboard focus survive a live republish while a row is
   open (#1430's dense-list/stable-reading contract) — have the other GM
   apply a fresh action while the first GM's detail region is open, and
   confirm it does not move under them.

Evidence: two attributed rows, both GMs' description of what the panel told
them, and the filter/republish observation.

---

## 5. Exercises

Rotate who acts and who reads the consequence between each numbered item so
both GMs exercise both roles by the end of the run.

### 5.1 Cross-GM undo and the affected-field conflict

1. GM A sets an NPC's doctrine. GM B — a **different** operator — opens
   that row in the journal and presses **Undo this action**
   (`server.gm.journal.undo`). Because GM B's policy is `immediate`, the
   inverse applies without a dialog; GM A, still on `confirm-preview`, would
   see a dialog naming "This would change {before} back to {after}"
   (`server.gm.journal.undo_change`) — have GM A undo a *different* row to
   confirm they actually see that text rather than taking it on trust.
   Confirm the journal now shows two rows for that decision: the original,
   marked reversed, and the inverse, attributed to GM B with `undo_of`
   pointing at the original (#1442 — "both operators recorded"). Record
   both operator ids.
2. Repeat with a faction-hostility change (the other affected-field
   family). Confirm an *unrelated* relation change on the same faction in
   between does not block the undo (`unrelated_relation_changes_are_allowed_while_the_affected_pair_is_guarded`,
   `tests/gm_undo.rs`) — have the acting GM make that unrelated change
   themselves so the crew sees it happen.
3. **Affected-field conflict.** GM A sets a doctrine. Before GM B's undo
   lands, have GM A (or the scenario) change that *same* doctrine again.
   GM B's undo must be refused with a named, human-readable reason
   (`AffectedStateChanged` → the journal's refusal sentence), not a generic
   failure. Record the exact sentence GM B saw and whether it was
   understandable without reading this kit.
4. Have both GMs attempt to undo the *same* original at effectively the
   same moment (agree a spoken cue). Confirm exactly one inverse is
   recorded and the second GM sees a truthful `AlreadyInverted` refusal —
   never a duplicate reversal, never a silent no-op presented as success.

Evidence: journal screenshots for each row pair, the two confirmation-policy
screens side by side, the exact refusal sentences, both operators' ids.

### 5.2 Exact sensor cutoff on a spawn undo

1. GM A places an authored removable NPC at a point outside every player
   ship's current sensor range. Before it enters range, GM A undoes the
   placement — confirm it is taken back cleanly with no cutoff in play.
2. GM A places a second one already inside a player ship's sensor range (or
   flies a ship to it) and leaves it there. Have the crew watch a real
   clock or the scenario's own tick display. At a little under two
   cumulative real/simulation seconds of exposure, confirm the undo still
   applies. Immediately after the cutoff (two cumulative seconds — an
   overlap between two player hulls still counts once, per
   `overlapping_player_hulls_count_one_second_per_second`), attempt the
   same undo and confirm it is now **permanently** refused with
   `SensorExposureElapsed`, not merely "currently in range" — move the ship
   back out of range and try again to confirm the refusal does not lift.
3. Have the acting GM pause the session mid-exposure and confirm the paused
   time does not count towards the two seconds
   (`a_paused_world_does_not_spend_the_window`) — resume and watch the
   remaining budget elapse from where it left off, not from zero.
4. Record whether a real GM, watching only the product surface (no test
   file open), could tell *why* a placement they placed seconds ago could
   no longer be taken back. This is the human-comprehension half of the
   criterion; the timing itself is exhaustively covered by
   `tests/gm_spawn_undo.rs` and is not this kit's job to re-verify.

Evidence: the two undo attempts (before/after cutoff), the pause
observation, and each GM's own account of whether the refusal was legible.

### 5.3 Despawn restore and consequence comprehension

1. GM B removes an authored removable entity the crew can see leave.
   Confirm the crew's own screen reflects the removal.
2. GM A undoes that despawn from the journal. Because GM A's policy is
   `confirm-preview`, a dialog must appear that names the concrete target
   and distinguishes *technical* reversibility from what the crew already
   witnessed — record its exact wording and whether the crew was told, or
   could tell, that their own memory of "it disappeared" does not un-happen
   even though the entity is technically back. This is #1418 story 29's
   named requirement, not a nice-to-have.
3. Confirm the restored entity keeps its original identity and any
   references pointing at it (e.g. a contact override, an objective
   target) rather than becoming a fresh, unrelated spawn. Have the crew
   check the specific contact/objective the entity was tied to before
   removal, not just that "an entity is back."
4. Remove an entity that has no runtime spawn recipe (an authored
   `[[entity]]` block a fresh boot would recreate from the world file — see
   `a_removal_no_capture_can_rebuild_refuses_its_inverse_and_says_so_on_the_journal`).
   Confirm the journal tells the GM **before** they press Undo that this
   row cannot be rebuilt (`GmJournalEntry::capture_lost`), rather than
   offering a control that then fails.
5. Have both GMs race to undo one despawn (spoken cue). Confirm one
   restore, one `AlreadyInverted`-style refusal, and that the loser's
   screen does not show a duplicate entity.

Evidence: crew-visible before/after screenshots, the confirmation dialog's
exact text, the reference-preservation check, the capture-lost precondition
message, and the race outcome.

### 5.4 Named checkpoint and compatibility refusals

1. GM A bookmarks a named checkpoint mid-session
   (`server.gm.checkpoint.bookmark`). Confirm the confirmed row shows the
   capture tick and local time (`server.gm.checkpoint.confirmed`), and that
   GM B's own checkpoint panel does **not** show GM A's row — the catalogue
   is per-browser private (`server.gm.checkpoint.intro`;
   `one_peers_checkpoints_never_appear_in_another_peers_catalogue`).
2. Attempt a bookmark while the session phase does not permit a capture
   (e.g. before start or after ending). Confirm the panel names the actual
   unavailable phase (`server.gm.checkpoint.unavailable`) rather than
   silently doing nothing.
3. **Compatibility refusals.** With a checkpoint captured on the current
   roster, change the crew's seating — add or move a player to a different
   Station, or (if the fixture allows) swap a hull — then look at that same
   checkpoint's candidate row. Confirm it now reads ineligible with a
   specific reason (`server.gm.checkpoint.block.*` — missing ship, hull
   differs, or hull unknown, naming the actual slot and Stations) rather
   than a bare "no". Separately, if practical, load a different scenario
   file's checkpoint to confirm `scenario_differs` names both the candidate
   and live world.
4. Undo the seating change and confirm the same checkpoint becomes eligible
   again — compatibility is evaluated live, not cached from capture time.

Evidence: both GMs' checkpoint panels side by side, the unavailable-phase
message, the exact ineligibility reasons before and after reverting the
seating change.

### 5.5 Local (single-simulation-peer) restore

Run this on the ordinary `combat_test.toml` session with both GMs present
but one simulation peer.

1. GM A previews the checkpoint from §5.4 in the restore control
   (`server.gm.restore.candidate` names it and its tick). Because GM A's
   policy is `confirm-preview`, confirm the dialog reads
   `server.gm.restore.confirm_consequences` verbatim: everything after that
   tick is discarded, current ship/Station assignments are kept, crews
   remember what they saw, a recovery checkpoint is taken first, and the
   session stays paused until an explicit resume. Have GM A read that
   sentence aloud before confirming — this is the one decision PRD #1420
   says a crew cannot un-see, and the point of the exercise is whether a
   real person actually absorbs it before pressing the button.
2. Confirm the phase line moves through
   `capturing-recovery → loading → restored` (or the readiness/agreement
   phases are skipped outright on one peer) and that the world visibly
   holds — no automatic resume. Confirm current seating (who is on which
   Station) is unchanged even though the restored world is from before
   some of them joined that Station, per
   `a_live_restore_rewinds_the_world_keeps_the_live_seating_and_stays_paused`.
3. **Log integrity.** Open the journal. Confirm it now shows exactly the
   checkpoint's own saved rows plus nothing invented, and that any action
   applied *after* the checkpoint but *before* the restore no longer
   appears — there is no "abandoned branch" view anywhere in the product.
   Cross-check against `the_restored_journal_is_the_candidates_own_log_with_later_entries_discarded`.
4. Have GM B (not the one who asked) attempt a session Resume while the
   restore is still `Loading`. Confirm it is refused by name (not silently
   ignored) and does not resume a half-loaded world.
5. GM B explicitly presses **Resume session**
   (`server.gm.restore.resume`). Confirm the attributed result names GM B,
   and that a fresh GM action taken immediately after appends to the
   restored log rather than the discarded one.

Evidence: the read-aloud consequence text, phase-transition screenshots,
the pre/post-restore journal contents, the resume-during-load refusal, and
the post-restore append.

### 5.6 Distributed restore: readiness, timeout, exclusion, reconnect

Switch to the generated two-ship world from §2/§3, with GM A and GM B each
present and a real crew on **both** ships (§2's precheck already proved the
two peers boot and agree before any human joins).

1. GM A bookmarks a checkpoint (private to A's own browser, per §5.4) and
   requests a restore. Confirm the health banner
   (`server.gm.health.reason.live_restore_readiness`) and the restore
   control both show a **countdown in whole seconds** and **name the
   waiting peer** — not a bare spinner. Both crews should be able to see
   from their own screens that a restore is in flight (the banner is
   unfilterable per `pasm/spec/design/gm-console-t3.yaml`).
2. Let Ship B's crew answer readiness promptly. Confirm the restore
   proceeds to `Loading` on both peers within the ten-second window and
   both settle on `Restored` with matching digests (compare each ship's
   own displayed state, not just the GM desk's).
3. **Timeout and exclusion.** Repeat with a fresh checkpoint, but this time
   have Ship B's host go genuinely unresponsive (close the tab/process,
   not just look away) before answering. Confirm the countdown runs to
   zero, Ship B is canonically disconnected
   (`server.gm.restore.excluded`), and Ship A/GM proceed to `Restored`
   without Ship B — record the actual wall-clock time from request to
   exclusion (should be close to ten real seconds, not instant and not
   indefinite).
4. **Reconnect.** Reopen Ship B's host against the same build. Confirm it
   rejoins through the ordinary #1117/#1118 recovery transport onto the
   *restored* world (not the abandoned pre-restore one), that its digest
   then matches the rest of the fleet, and that it comes back **paused**,
   not auto-resumed onto a session it never agreed to.
5. Confirm phone consoles on either ship never see a readiness prompt and
   are never counted among the waiting peers (`Phone consoles do not
   vote`, PRD #1420 Implementation Decisions) — have a phone player try to
   act during the wait and confirm nothing about their screen implies they
   are being waited on.

Evidence: countdown screenshots at request, mid-wait and at exclusion; the
excluded/reconnected ship's own before/after digest; the phone console's
unaffected screen.

### 5.7 Failed load, mismatch and rollback

1. Force a candidate that has stopped fitting at execution (e.g. change a
   Station's crewed hull between the preview and the actual request, if the
   fixture allows) and confirm `server.gm.restore.failed.ineligible`
   appears and **nothing was loaded anywhere** — both ships stay on the
   pre-request world, held.
2. If practical, force a digest mismatch or a peer-load failure (see
   `tests/lockstep_gm_restore.rs`'s
   `a_peer_that_cannot_take_part_rolls_the_whole_fleet_back` and
   `a_peer_that_ends_up_with_a_different_world_rolls_the_whole_fleet_back`
   for the exact conditions a scripted rehearsal can reproduce without a
   real fault). Confirm the failure sentence names the actual peer count
   (`server.gm.restore.failed.peer_load` / `.peer_digest`) rather than a
   generic "something went wrong," that **every** peer rolls back to its
   own recovery checkpoint (not just the initiator's), and that the session
   remains held rather than silently resuming the old world.
3. Confirm the rolled-back journal is unchanged from before the attempt —
   the failed restore leaves no partial or phantom entry.
4. Record, for each forced failure, whether the GM reading only the product
   screen could tell (a) that it failed, (b) roughly why, and (c) that the
   world is safe to keep playing on. A truthful failure that a real person
   cannot act on does not satisfy PRD #1420 story 17.

Evidence: the failure sentence for each forced condition, the unchanged
pre-attempt journal, and both GMs' account of whether the failure was
actionable.

### 5.8 Enlarged text and forced colours across the recovery panels

Unlike the T2 directing/performing desk (mission log, Objective list,
Knowledge Compare), which #1430 already carried into 200%/forced-colours/
dense-list coverage (§1), the journal, checkpoint and restore-control panels
are new in this tranche and have only automated Playwright coverage
(`tests/smoke/gm-journal-200.spec.js`, `gm-undo-200.spec.js`,
`gm-checkpoint-200.spec.js`, `gm-restore-200.spec.js`) — none of which has
been run against a live human yet. Mirroring
`docs/acceptance/1432-platform-comfort.md` Task 4 step 2 for these three
panels specifically:

1. At some point while the journal is open during §4/§5.1 (both the
   attributed-row read and the undo dialog), have each GM switch their own
   text scale to 200% (`server.html` → Settings → Display) and, separately,
   enable forced/high-contrast colours. Confirm the journal's row identity,
   operator id, outcome badge and the undo confirmation's "This would change
   {before} back to {after}" sentence (`server.gm.journal.undo_change`)
   remain fully readable — no truncation, no overlap with adjacent rows —
   and that the Applied/No-op/Refused outcome is distinguishable under
   forced colours by more than colour alone.
2. At some point during §5.4, have the GM viewing the checkpoint panel do
   the same: confirm the confirmed-checkpoint row, the phase-unavailable
   message and the compatibility-refusal reasons
   (`server.gm.checkpoint.block.*`) stay legible and attached to the correct
   candidate row at 200% and under forced colours.
3. At some point during §5.5 or §5.6, have the GM previewing or confirming a
   restore do the same: confirm `server.gm.restore.confirm_consequences`
   (the full discard/seating/witnessed/paused sentence), the readiness
   countdown and waiting-peer name, and any `server.gm.restore.failed.*`
   sentence from §5.7 stay complete and legible at 200% and under forced
   colours, and that reading position/focus/target identity do not shift
   when the scale changes mid-read.
4. Record, per panel, whether either GM found anything clipped, overlapping,
   or dependent on colour alone — this is the one live 200%/forced-colours
   pass over these three panels anywhere in the batch; #1430's own coverage
   (§1) stops at the T2 mission/Objective/Knowledge-Compare panels and does
   not reach here.

Evidence: a screenshot of each of the three panels at 200% (and, where
practical, forced colours) with the specific text named above visible and
intact, plus both GMs' account of legibility.

---

## 6. Preserve evidence and decide

Use the ordinary save/export UI and the journal panel's own read-only
export (or the same read-only collector `docs/acceptance/1320-gm-live-event.md`
describes) to retain the journal contents, health-banner sequence and
per-peer digests at each numbered step above. A screenshot from a different
tick than the one being compared is not evidence of agreement — capture
digests at the same boundary as `docs/acceptance/1320-gm-live-event.md` §8
already requires.

The coordinator marks **Accept** only when all three required exercise
groups (§5.1–5.4 field/undo mechanics, §5.5–5.7 restore mechanics including
the distributed path, and §5.8's 200%/forced-colours pass across the
journal, checkpoint and restore-control panels) are complete, every AC below
has cited evidence, and any blocking failure has been resolved with its
owning issue from §1 —
**a software fix or an automated PASS does not change a human Reject
without the relevant human rerun.** Missing mandatory exercise or evidence
keeps the record Pending; #1448 stays open either way.

```markdown
# #1448 M5 recovery record

Status: Pending / Accept / Reject
Decision owner and date:
Session start/end and timezone:
Integrated commit/build stamp:
§2 precheck results (commands, pass counts, GM_LIVE_EVENT_PRECHECK line):

## People and devices
| Role | Participant | Public operator id | Device/OS/browser | Confirmation policy (undo/restore) |
| --- | --- | --- | --- | --- |
| GM A | | | | |
| GM B | | | | |
| Ship A crew | | | | |
| Ship B crew | | | | |
| Evidence recorder | | | | |

## Exercises
| Step | Actual action/observation | Result | Artifact/tick | Defect or follow-up |
| --- | --- | --- | --- | --- |
| 5.1 Cross-GM undo + affected-field conflict | | Pending | | |
| 5.2 Exact sensor cutoff | | Pending | | |
| 5.3 Despawn restore + consequence comprehension | | Pending | | |
| 5.4 Named checkpoint + compatibility refusals + seat preservation | | Pending | | |
| 5.5 Local restore success + log integrity + paused-until-resume | | Pending | | |
| 5.6 Distributed restore: readiness/timeout/exclusion/reconnect | | Pending | | |
| 5.7 Failed load/mismatch/rollback | | Pending | | |
| 5.8 Recovery panels at 200%/forced-colours (journal, checkpoint, restore-control) | | Pending | | |

## Decision against #1448
AC1 — cross-GM undo, affected-state conflict, exact sensor cutoff, despawn restore, consequence comprehension under different confirmation policies:
AC2 — named checkpoint, compatibility refusal, seat preservation, local/distributed success, readiness timeout/exclusion/reconnect, failed load/mismatch/rollback:
AC3 — log is saved log plus new actions only, no abandoned branch, held until explicit resume on every path:
AC4 — revision/devices/operators/scenario/outcomes recorded, blocking failures resolved with owning feature:
AC5 (§5.8; inherited PRD #1418 Required-HITL bullet via PRD #1420, not covered by #1430/#1432) — journal, checkpoint and restore-control panels stay legible and stable at 200% text scale and under forced colours:
Accept/Reject rationale, or missing work keeping Pending:
Defect dispositions and required human reruns:
```

---

## 7. Source pointers for the coordinator

- [Canonical GM action journal](../../src/gm_action.rs) (the `UndoGmAction` reducer lives here; there is no separate `gm_undo.rs`) and its [read-only projection](../../src/gm_journal.rs)
- [Spawn exposure cutoff](../../src/gm_exposure.rs), [despawn capture/restore](../../src/gm_despawn_undo.rs)
- [Named checkpoint panel](../../gui/gm-checkpoint-panel.js), [shared compatibility preflight](../../gui/gm-checkpoint-preflight.js), [live restore control](../../gui/gm-restore-control.js), [journal panel](../../gui/gm-journal-panel.js)
- [Confirmation policy registry](../../gui/gm-confirmation.js) — `action.undo`/`world.restore` categories and per-operator override
- [Single-peer restore driver](../../src/gm_restore.rs) and its [multi-peer extension](../../src/lockstep/mod.rs)'s `MeshFrame::GmRestore`
- [Public health banner](../../gui/gm-health-banner.js) and [its Rust source](../../src/gm_health.rs)
- [#1320's two-ship preparation tooling](../../scripts/prepare-gm-live-event.mjs) and [its ignored boot precheck](../../tests/gm_live_event_precheck.rs), reused unchanged by §2/§5.6
- [`pasm/spec/design/gm-console-t3.yaml`](../../pasm/spec/design/gm-console-t3.yaml) — the M5 design entries (`gm-t3-single-peer-live-restore`, `gm-t3-multi-peer-live-restore`) this kit's §5.5–5.7 exercise
- [`pasm/spec/roadmap/gm-console-milestones.yaml`](../../pasm/spec/roadmap/gm-console-milestones.yaml) — M5 `gm-milestone-operations-safety` scope boundary
- The four 200%-scale Playwright specs §5.8 mirrors for a live human pass: [`tests/smoke/gm-journal-200.spec.js`](../../tests/smoke/gm-journal-200.spec.js), [`gm-undo-200.spec.js`](../../tests/smoke/gm-undo-200.spec.js), [`gm-checkpoint-200.spec.js`](../../tests/smoke/gm-checkpoint-200.spec.js), [`gm-restore-200.spec.js`](../../tests/smoke/gm-restore-200.spec.js)

This kit does not itself supply integrated runtime evidence or human
acceptance; both remain the responsibility of the operator who runs it.
