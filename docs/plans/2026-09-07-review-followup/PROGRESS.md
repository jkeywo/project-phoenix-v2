# Execution record

User approved implementation on 7 September 2026. Integration worktree: codex/review-followup-plan. Code, documentation, profiling evidence and required validation are all part of this batch; the conditional experiments retain the stopping criteria in the approved plan.

## Current work

R1, R3, A1 and A2 are integrated. Reviewed P1, A3 and A5 have validation-pending checkpoints ready for integration; R2a's final integration rerun is in progress. G0 and named CPU/GPU profiling are being implemented on the integration branch. No completed slice is claimed until its tests and independent review pass. Cargo builds and actual profiling are coordinated serially. The remaining reviewed Rust slices will be tested together after integration to avoid repeated crate rebuilds and shared-cache contention; checkpoint commits are not completion claims. On the user's request the integration branch was rebased onto main 76d36d24, including the delivered #1404 commit 718d9a90; profiling WIP restored without conflicts.

| Slice | Tracker | Status | Integrated commit / evidence |
|---|---|---|---|
| R1 | [#1406](https://github.com/jkeywo/project-phoenix-v2/issues/1406) | Reviewed, tested and integrated; browser smoke pending | 20de458e |
| R2a | [#1410](https://github.com/jkeywo/project-phoenix-v2/issues/1410) | Implementation underway | — |
| R2b | [#1411](https://github.com/jkeywo/project-phoenix-v2/issues/1411) | Queued after dependencies | — |
| R3 | [#1407](https://github.com/jkeywo/project-phoenix-v2/issues/1407) | Reviewed, tested and integrated; browser smoke pending | 47a60ec1 |
| G0 | [#1409](https://github.com/jkeywo/project-phoenix-v2/issues/1409) | Harness implementation underway | — |
| P5 | [#1416](https://github.com/jkeywo/project-phoenix-v2/issues/1416) | Named-system and supported GPU harness in progress | — |
| P6 | [#1417](https://github.com/jkeywo/project-phoenix-v2/issues/1417) | Named-system/spike harness in progress | — |
| A1 | [#1408](https://github.com/jkeywo/project-phoenix-v2/issues/1408) | Reviewed, tested and integrated | 53663196 |
| A2 | [#1412](https://github.com/jkeywo/project-phoenix-v2/issues/1412) | Reviewed, tested and integrated | 23e77dc9 |
| A3 | [#1413](https://github.com/jkeywo/project-phoenix-v2/issues/1413) | Power and conditional Control migration implemented and reviewed; Rust tests queued | — |
| A4 | [#1414](https://github.com/jkeywo/project-phoenix-v2/issues/1414) | Complete result adapter implemented and reviewed; tests queued, integration follows R2 | — |
| A5 | [#1415](https://github.com/jkeywo/project-phoenix-v2/issues/1415) | Shared materialization implemented and reviewed; Rust tests queued | — |
| P1/P2/P3/P4 | [#1405](https://github.com/jkeywo/project-phoenix-v2/issues/1405) | P1 implementation underway against delivered #1404; P4 conditional | — |
| Pane thread | [#1404](https://github.com/jkeywo/project-phoenix-v2/issues/1404) | Integrated through rebase; follow-up acceptance still to be recorded | 718d9a90 |

A3b follows the reviewed Power pilot: both capture and restore field conversion move out of snapshot orchestration, while the old payload paths and field layout remain compatible. Performance acceptance requiring real hardware is recorded separately from automated correctness, and no unrun leg is considered complete.

The user selected separate monitors for the current rig: 1080p Viewscreen and Helm, with Tactical on the laptop panel. The current desktop query reports that panel as 1920×1200 at 125% scale (the GPU inventory reports its 3840×2400 native mode); actual host window dimensions must corroborate the capture. This replaces the original review's now-disconnected third 1080p monitor. Preserve the changed workload in conclusions.

## Branch baseline

- Planning commit: 38a97585 (was 9c1b333d before rebase).
- Main base: 76d36d24, containing #1404 718d9a90 and smoke dependency fix 4853f00c. The duplicate ec8db067 cherry-pick was dropped by rebase.
- Child implementation branches: codex/review-restore, codex/review-catalogue, codex/review-script-effects.

## Validation ledger

G0 analysis: 14 Vitest tests passed; PowerShell parsers and JavaScript syntax checks passed. Two earlier native observer tests passed; the added real workload observer test and attribution example tests await the serial Cargo slot. Independent review corrected PDB filenames, immutable profile launch, continuous console/workload evidence and observation-only contention accounting. Comparative binaries build in a private target directory after a stale shared-worktree library was observed during A2 validation.

A1 source review found no production issues: 2 new-call tests, 135 World tests, 172 Comms tests and 16 engine tests passed; PASM validate/scan/traceability passed. A2 passed 212 targeted tests plus the PASM triplet and independent review. R1 source review required a real GameOver state transition in its regression, now corrected. R3 review moved real native pack installation out of the lib-test process. A3, A4 and A5 source/test reviews pass; their Rust execution remains queued. P1 surface capture and reducer reviews pass, with 6 JS tests and the PASM triplet green; SDK compilation and Rust tests remain queued. Targeted checks are recorded with each slice; full repository gates run once after integration before any push. Any changes after a gate invalidate the affected checks.
