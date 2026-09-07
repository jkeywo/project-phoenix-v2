# Execution record

User approved implementation on 7 September 2026. Integration worktree: codex/review-followup-plan. Code, documentation, profiling evidence and required validation are all part of this batch; the conditional experiments retain the stopping criteria in the approved plan.

## Current work

R1, R2a, R3, A1, A2, A3, A4, A5, G0 and P1 are integrated. R2b and P2/P3 remain in isolated child worktrees. G0/P1 and the later architecture slices entered as validation-pending checkpoints so one combined Rust build could cover them. No completed slice is claimed until tests and independent review pass. Cargo builds and actual profiling are coordinated serially. The integration branch was rebased onto main 76d36d24, including delivered #1404 commit 718d9a90; the profiling WIP was restored without conflicts.
| Slice | Tracker | Status | Integrated commit / evidence |
|---|---|---|---|
| R1 | [#1406](https://github.com/jkeywo/project-phoenix-v2/issues/1406) | Reviewed, tested and integrated; browser smoke pending | 20de458e |
| R2a | [#1410](https://github.com/jkeywo/project-phoenix-v2/issues/1410) | Reviewed, tested and integrated | b7d158ab |
| R2b | [#1411](https://github.com/jkeywo/project-phoenix-v2/issues/1411) | Shared WASM owner and bootstrap integration implemented; JS/smoke preparation underway | — |
| R3 | [#1407](https://github.com/jkeywo/project-phoenix-v2/issues/1407) | Reviewed, tested and integrated; browser smoke pending | 47a60ec1 |
| G0 | [#1409](https://github.com/jkeywo/project-phoenix-v2/issues/1409) | Harness implementation underway | — |
| P5 | [#1416](https://github.com/jkeywo/project-phoenix-v2/issues/1416) | Named-system and supported GPU harness in progress | — |
| P6 | [#1417](https://github.com/jkeywo/project-phoenix-v2/issues/1417) | Named-system/spike harness in progress | — |
| A1 | [#1408](https://github.com/jkeywo/project-phoenix-v2/issues/1408) | Reviewed, tested and integrated | 53663196 |
| A2 | [#1412](https://github.com/jkeywo/project-phoenix-v2/issues/1412) | Reviewed, tested and integrated | 23e77dc9 |
| A3 | [#1413](https://github.com/jkeywo/project-phoenix-v2/issues/1413) | Integrated; Power tests pass, Control next-action digest mismatch under diagnosis | 9e2fa97f |
| A4 | [#1414](https://github.com/jkeywo/project-phoenix-v2/issues/1414) | Reviewed, integrated after R2a and three registered flow tests pass | a1c8de08 |
| A5 | [#1415](https://github.com/jkeywo/project-phoenix-v2/issues/1415) | Integrated; behavior/mint tests pass, corrected registration-inventory fixture awaits rerun | 3b750738 |
| P1/P2/P3/P4 | [#1405](https://github.com/jkeywo/project-phoenix-v2/issues/1405) | P1 integrated; P2 reviewed checkpoint, P3 implementation underway, P4 conditional | 50618b63; P2 child 7c1f8dc4 |
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

## Combined validation checkpoint

The combined host/headless/perf build passed. Frozen test binaries report: native frame observer 3/3; registered lobby-result flows 3/3; boot parity 9/9; Power owner 2/2; Control owner 4/4; panes 169/169; attribution example 2/2; native pane integration 12/12. Power continuation passed its real resumed-simulation test. Control continuation's folded state matched immediately after restore at the asserted control frontier, but its first resumed full digest differed; cumulative digest and snapshot diagnostics are being added before deciding whether production or the synthetic fixture is responsible.

Native lobby integration passed 25/26. Both new materialization behavior/mint tests passed; the registration-inventory test inspected an already-initialized graph, whose systems Bevy moves into the executable schedule. Its fixture now inspects the actual production registration before first use and awaits rerun.

The profiling reducers pass 22 JS tests. Review added a shared frame/surface origin, exact interval reduction, and proof of a loaded, uploaded, continuously live HUD. A static HUD remains valid without repeated measured applications or uploads. The separate named renderer example explicitly records no surface attribution. The newly added Rust profiling serialization now goes through core::codec; that change and the inventory correction are in the next targeted compile. No fresh profiling artifact or measured speedup is claimed.