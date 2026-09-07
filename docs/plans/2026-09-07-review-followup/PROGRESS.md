# Execution record

User approved implementation on 7 September 2026. Integration worktree: codex/review-followup-plan. Code, documentation, profiling evidence and required validation are all part of this batch; the conditional experiments retain the stopping criteria in the approved plan.

## Current work

R1, R2a, R2b, R3, A1, A2, A3, A4, A5, G0 and P1 are integrated, through ac700aa0. P2 and P3 are independently reviewed checkpoints, held outside integration until the profiling baseline is frozen. G0/P1 and the later architecture slices entered as validation-pending checkpoints so one combined Rust build could cover them; targeted results are recorded below. Browser, SDK, hardware and final repository validation remain separate pending work. Cargo builds and actual profiling are coordinated serially. The integration branch was rebased onto main 76d36d24, including delivered #1404 commit 718d9a90; the profiling WIP was restored without conflicts.

| Slice | Tracker | Status | Integrated commit / evidence |
|---|---|---|---|
| R1 | [#1406](https://github.com/jkeywo/project-phoenix-v2/issues/1406) | Reviewed, tested and integrated; browser smoke pending | 20de458e |
| R2a | [#1410](https://github.com/jkeywo/project-phoenix-v2/issues/1410) | Reviewed, tested and integrated | b7d158ab |
| R2b | [#1411](https://github.com/jkeywo/project-phoenix-v2/issues/1411) | Reviewed and integrated; 192 targeted JS checks pass; Rust/WASM and browser smoke validation pending | ac700aa0 |
| R3 | [#1407](https://github.com/jkeywo/project-phoenix-v2/issues/1407) | Reviewed, tested and integrated; browser smoke pending | 47a60ec1 |
| G0 | [#1409](https://github.com/jkeywo/project-phoenix-v2/issues/1409) | Harness integrated and reviewed; observer/reducer tests pass; baseline capture pending | 21185b7e, 8d610f17 |
| P5 | [#1416](https://github.com/jkeywo/project-phoenix-v2/issues/1416) | Named-system and supported GPU harness compiles; example tests pass; controlled captures pending | Integrated G0 harness |
| P6 | [#1417](https://github.com/jkeywo/project-phoenix-v2/issues/1417) | Named-system/spike harness compiles; example tests pass; attribution and explanation pending | Integrated G0 harness |
| A1 | [#1408](https://github.com/jkeywo/project-phoenix-v2/issues/1408) | Reviewed, tested and integrated | 53663196 |
| A2 | [#1412](https://github.com/jkeywo/project-phoenix-v2/issues/1412) | Reviewed, tested and integrated | 23e77dc9 |
| A3 | [#1413](https://github.com/jkeywo/project-phoenix-v2/issues/1413) | Reviewed and integrated; owner and real continuation tests pass | 9e2fa97f plus reviewed fixture correction; final Control rerun 1/1, both cases × 12 steps |
| A4 | [#1414](https://github.com/jkeywo/project-phoenix-v2/issues/1414) | Reviewed, integrated after R2a and three registered flow tests pass | a1c8de08 |
| A5 | [#1415](https://github.com/jkeywo/project-phoenix-v2/issues/1415) | Reviewed and integrated; behavior/mint and corrected registration-inventory tests pass | 3b750738, fixture 8d610f17; native lobby 25 passing tests plus corrected inventory 1/1 |
| P1/P2/P3/P4 | [#1405](https://github.com/jkeywo/project-phoenix-v2/issues/1405) | P1 integrated; P2/P3 reviewed checkpoints held for baseline freeze; SDK/hardware validation pending; P4 conditional | P1 50618b63; P2 child 7c1f8dc4; P3 child 3c6341f4 |
| Pane thread | [#1404](https://github.com/jkeywo/project-phoenix-v2/issues/1404) | Integrated through rebase; follow-up acceptance still to be recorded | 718d9a90 |

A3b follows the reviewed Power pilot: both capture and restore field conversion move out of snapshot orchestration, while the old payload paths and field layout remain compatible. Performance acceptance requiring real hardware is recorded separately from automated correctness, and no unrun leg is considered complete.

The user selected separate monitors for the current rig: 1080p Viewscreen and Helm, with Tactical on the laptop panel. The current desktop query reports that panel as 1920×1200 at 125% scale (the GPU inventory reports its 3840×2400 native mode); actual host window dimensions must corroborate the capture. This replaces the original review's now-disconnected third 1080p monitor. Preserve the changed workload in conclusions.

## Branch baseline

- Planning commit: 38a97585 (was 9c1b333d before rebase).
- Main base: 76d36d24, containing #1404 718d9a90 and smoke dependency fix 4853f00c. The duplicate ec8db067 cherry-pick was dropped by rebase.
- R2b integration checkpoint: ac700aa0. The subsequent validated correction includes the A3 fixture, the headless logging borrow and removal of an obsolete A5 helper comment.
- P2/P3 remain pinned on their child refs, separate from the P1 baseline: codex/review-hud-revisions and codex/review-hidden-paint.

## Validation ledger

G0 analysis: 22 focused JS tests passed; PowerShell parsers and JavaScript syntax checks passed. The native observer's three tests and the attribution example's two tests pass. Independent review corrected PDB filenames, immutable profile launch, continuous console/workload evidence and observation-only contention accounting. Comparative binaries build in a private target directory after a stale shared-worktree library was observed during A2 validation.

A1 source review found no production issues: 2 new-call tests, 135 World tests, 172 Comms tests and 16 engine tests passed; PASM validate/scan/traceability passed. A2 passed 212 targeted tests plus the PASM triplet and independent review. R1 source review required a real GameOver state transition in its regression, now corrected. R3 review moved real native pack installation out of the lib-test process. R2b's final review confirms shared ownership before ECS startup, immutable exact handles, stale-incarnation filtering at queue flush and codec-boundary serialization; 192 targeted JS checks pass, while native/WASM transcript and live browser reconnect execution remain pending.

A3, A4 and A5 source/test reviews and targeted Rust checks pass, as detailed below. P1 surface capture/reducer reviews and PASM triplet pass; the non-SDK pane suite passes 169 tests. P2 and P3 have independent source/test reviews, focused JS checks, formatting/wiki checks and PASM validation/scan/traceability green. P3's reducer addition passes 7/7, including hidden retention and reveal accounting. These child checkpoints still require integrated Rust/SDK tests and real-page/hardware acceptance after baseline freeze. No measured speedup is claimed. Full repository gates run once after integration before any push; later changes invalidate the affected checks.

## Combined validation checkpoint

The combined host/headless/perf build passed. Frozen test binaries report: native frame observer 3/3; registered lobby-result flows 3/3; boot parity 9/9; Power owner 2/2; Control owner 4/4; panes 169/169; attribution example 2/2; native pane integration 12/12. Power continuation passed its real resumed-simulation test. These are targeted results, not the final repository gate.

Control continuation's initial failure was diagnosed against the unchanged duel continuation, which still passes all 120 frames. Immediate restored full digests matched; the first resumed mismatch began in the entity fold. Per-ship diagnostics showed the restored NPC retained active boost while the live fixture had deactivated it. The fixture had called the complete `restore_into` operation merely to seed axes and targets, manufacturing a changed Boost command on the live ship; Bevy deliberately skips that command on a freshly inserted high-fidelity bundle. The reviewed test-only correction writes real axis/LastHelmInput/Tactical/Science components directly and retains the completed-tick drive intents and attacker/policy/recovery memory. Complete frontier equality, immediate full digest equality and all 12 subsequent frontier/full-digest comparisons remain. The final rerun passes both nondefault and cleared cases (1/1 test); no production conversion or digest coverage was changed. Evidence: [Control rerun](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/control-fixture-test.log), [120-frame control](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/validation-diagnostic/bounded-duel-control.log), [diagnostic values](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/validation-entity-diagnostic/snapshot_resume.log).

Native lobby integration initially passed 25/26, including both new materialization behavior/mint tests. The registration-inventory fixture inspected an already-initialized graph, whose systems Bevy moves into the executable schedule. It now inspects the actual production registration before first use; that corrected test passes 1/1. This is the original 25 passing cases plus the corrected inventory rerun, not a claimed second full-suite run. Evidence: [inventory rerun](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/validation-diagnostic/native_host_lobby.log).

The profiling reducers pass 22 JS tests. Review added a shared frame/surface origin, exact interval reduction, and proof of a loaded, uploaded, continuously live HUD. A static HUD remains valid without repeated measured applications or uploads. The separate named renderer example explicitly records no surface attribution. Rust profiling serialization goes through core::codec; read-only review found field names, values, nullability and nested payloads preserved. A later diagnostic build exposed a temporary-borrow error in the headless logging macro; the working-tree correction compiles with the final Control test rerun. R2b validation and release/SDK artifact preparation remain separate pending work. No fresh profiling artifact or measured speedup is claimed.
