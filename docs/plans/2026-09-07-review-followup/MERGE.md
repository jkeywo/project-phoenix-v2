# Main integration and publication — 8 September 2026

The user requested a rebase and merge to main. This task's complete reviewed work is rebased onto local main `07e2eedb3120cf063398961b1f38ec18751e0fd6`. T2's unfinished GM and scheduler changes are excluded; its separate integration branch remains untouched. Original review and evidence branches are preserved.

## Scope

The 36 commits from `76d36d24..8b16496f` replay as `07e2eedb..6c81c895`. Independent review and `git range-diff` confirm equivalent patches except one browser-test selector: main already correctly scoped the host QR overlay, so that equivalent selector was retained. All 72 files changed by main since the original base retain their changes.

Six separately reviewed follow-up commits were then replayed:

| Original | Rebased | Change |
| --- | --- | --- |
| `7691ddb1` | `09e6a096` | Forward Escape to the focused native pane |
| `c621fca2` | `9dc006f5` | Recognize unnamed native Escape events |
| `80bd07de` | `97352cf5` | Reconcile the selected hull's Station roster |
| `a8375a0b` | `d07fb27c` | Clean up obsolete Station consoles |
| `c37f0843` | `3e1859e0` | Pin the reviewed Vellum animation refresh |
| `7f4ab1d3` | `e29da1a0` | Reconnect crew during GameOver |

These replays required documentation conflict resolution only. Roster production/tests and reconnect fixtures match their validated originals. The shared lobby-result adapter remains in place, with Identify available in every phase after disconnect handling and before countdown evaluation. Station actions retain their existing gates.

## Validation

The runtime candidate was clean `e29da1a088e3a7fee80330273d38c54b751c720d` throughout these checks. This final documentation change does not modify compiled inputs.

| Check | Result |
| --- | --- |
| Native roster unit selection | 5 passed, 0 failed/ignored |
| Native lobby integration suite | 34 passed, 0 failed/ignored |
| Native pane integration suite | 12 passed, 0 failed/ignored |
| Focused JavaScript batch | 184 passed across 11 files, 0 failed/skipped |
| `cargo fmt -- --check` | Exit 0 |
| PASM validate / scan / traceability | All exit 0 |
| Wiki structural and source-reference lint | 63 Markdown files, 61 indexed pages, 2,695 references; all 20 numbered references resolve |

Both native commands used `--locked --offline --jobs 6 --features host,headless,perf` and six test threads, sequentially against the shared target directory. The actual project compilation named this worktree. The [roster receipt](C:/Coding/project-phoenix-v2/.worktrees/review-main/.phoenix/merge-validation/roster.json), [integration receipt](C:/Coding/project-phoenix-v2/.worktrees/review-main/.phoenix/merge-validation/native-integration.json), [JavaScript receipt](C:/Coding/project-phoenix-v2/.worktrees/review-main/.phoenix/merge-validation/js/receipt.json), and [static report](C:/Coding/project-phoenix-v2/.worktrees/review-main/.phoenix/merge-validation/static/REPORT.md) preserve commands, source revisions and results. Native fixture logs include mesh-load warnings; these checks establish application/transport behavior, not renderer acceptance.

PASM used the existing installed `pasm.exe` from Vellum `bde9b37`; local Git inspection confirms its PASM source is unchanged in pinned `606b0c06`. No dependency install was needed. Four raw wiki path candidates are documented examples or explicit external-repository references. Two pre-existing prose defects were corrected: the stale snapshot-format number and the claim that all triggers are single-shot.

Independent reviews passed for the rebase, roster/reconnect ports and dependency pin. Historical SDK, browser and profiling evidence retains its original source/artifact scope in [FOLLOWUPS.md](FOLLOWUPS.md); it is not relabelled as fresh testing of this complete assembly. The renderer crash and pre-start AFK findings remain unresolved.

## Final publication checks

The user subsequently authorized the push and publication of the Vellum dependency. The complete JavaScript suite passed 6,827 tests across 287 files on clean main `2a11dfce`, with no failures or skips. Debug-surface drift, strict strings, LOD drift and billboard-capture checks also passed. The [non-Rust receipt](C:/Coding/project-phoenix-v2/.phoenix/publish-validation/non-rust/receipt.json) records the exact commands and results.

The initial workspace Clippy run found three test-only lint errors. Commit `d8e3bdde858ec3b87dd84478a36e76d60c07b878` removes two redundant dereferences and applies the repository's test-fixture UUID exemption to a temporary-directory name. Independent review confirmed no production or assertion change. Formatting passed again; the original failing log is preserved separately.

| Final Rust check | Result |
| --- | --- |
| Workspace/all-target Clippy; `server,viewer,debug,headless,perf,host,capture`; warnings denied | Exit 0 |
| Complete workspace tests; `headless`, six test threads | 7,631 passed, 0 failed, 80 configured ignores |
| Library and `phoenix-host` Clippy; `host,ultralight`; warnings denied | Exit 0 |

These commands ran sequentially with `--locked --offline --jobs 6` on clean `d8e3bdde`, from 07:17:28 to 07:36:29 UTC, using the shared target. All source guards passed. The [Rust receipts](C:/Coding/project-phoenix-v2/.phoenix/publish-validation/rust-after-fixture-lints) preserve commands, environment, exit codes and full logs. This report changes documentation only. The unchanged JavaScript, PASM and wiki inputs retain their completed evidence above.

The earlier `c2fde76b` matrix supplies supplemental viewer, demo, minimal-feature and host/capture-link evidence at its recorded source. It is not a fresh nine-configuration run. Historical WASM artifacts predate shared repair publication, model-marker caching and GameOver reconnect changes; fresh CI WASM/build-smoke coverage remains pending. These checks do not repeat profiling or establish new hardware acceptance.

## Publication boundary

The reviewed [Vellum commit `606b0c06`](https://github.com/jkeywo/vellum/commit/606b0c06a6e6419a5b2a8c4f5bfb255146f87814) is published at `codex/review-vellum-refresh`, with that remote ref verified at the exact pinned SHA. GitHub now serves the commit, PASM metadata and both composite actions. The earlier HTTP 422 described the local-merge checkpoint; the dependency is now fetchable upstream.

The change adds only `refresh_display(0)` before rendering and its real-SDK regression. All seven Cargo dependencies, UV/PASM and eleven workflow action references consistently pin that revision; no local path override is committed. Existing upstream validation remains recorded in the preserved Vellum handoff. No new Vellum CI result is claimed: its branch push did not trigger a workflow.

Phoenix main's local publication gate is complete. Remote push and CI outcomes must be verified against the published head; CI remains pending until observed. T2 retains responsibility for its separate unfinished GM/scheduler batch, which is excluded from this assembly and must reconcile with the new main before its own later integration.
