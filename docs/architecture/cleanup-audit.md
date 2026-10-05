# Small unconditional audit cleanups

Selected against `f01f732d`, after the 35 larger improvements. The original `report.md` is unchanged. Duplicate audit observations count once. Only local cleanup with current consumers or a confirmed unused implementation is included; conditional proposals and broader ownership/test relocations are excluded.

All 46 selected cleanups are implemented. Integrated verification is complete.

| # | Audit references | Cleanup |
|---|---|---|
| 1 | 1.1, 4.2 | Remove inherited root dependency declarations that no longer have root consumers |
| 2 | 1.2 | Keep boot-path test markers out of production ECS registration |
| 3 | 4.1 | Delete the second implementation of the checkpoint preflight wire shape |
| 4 | 5.1 | Remove the unused WebSocket implementation dependency from the simulation host feature |
| 5 | 6.1 | Reuse the existing whole-fixed-step-debt helper across simulation pause and restore paths |
| 6 | 8.1 | Consolidate the six Live Inspector descriptor defaults; retain the domain-specific field constructors. |
| 7 | 9.2 | Remove target selection around the identical faction-resource dereference implementation |
| 8 | 10.2 | Phaser and torpedo signed-angle differences are identical, but nearby angle helpers are not interchangeable |
| 9 | 12.1 | Use the existing struct defaults as the deserialization defaults for three fully optional configuration blocks |
| 10 | 14.2 | Computer-message duration conversion can now reuse Contracts' existing seconds-to-ticks implementation. |
| 11 | 18.1, 20.2 | Share the vacated-Station rating reset already implemented independently by three handlers |
| 12 | 20.1 | Replace repeated empty lobby-result literals with a single derived default |
| 13 | 20.3 | Derive SessionManager's empty initialization instead of maintaining it twice |
| 14 | 21.1 | Low-priority deletion opportunity: logging wraps empty collection constructors without adding policy |
| 15 | 22.1, 23.2, 24.1 | The two authored cadence-ratio checks repeat the same numeric rule |
| 16 | 24.2 | Remove the private allocating flag-chain resolver and resolve directly in the borrowed-chain reader. |
| 17 | 24.3 | Use one private comparison implementation for integer counters and floating-point facts. |
| 18 | 25.2 | Native readback carries an enable/disable synchronization mechanism with no disabling operation |
| 19 | 28.2 | Replace the manually assembled ordinary billboard quad with Bevy's existing rectangle primitive |
| 20 | 29.1, 30.1 | Hand-written `ModifierSource` equality/hash machinery duplicates derivable semantics |
| 21 | 30.2 | Model already supplies `parse_tags`, but callers rebuild exactly the same conversion pipeline |
| 22 | 32.1 | Inline the nine private star-default constants into the existing Default implementation |
| 23 | 32.2 | Let BaseTransform's existing Default implementation own sparse rig defaults |
| 24 | 32.3 | Remove the private power-floor forwarding function, retaining its canonical wire default |
| 25 | 34.1 | Sound catalogue path validation repeats the archive asset predicate, with a real additional restriction that must remain |
| 26 | 36.2 | Share the identical error/warning finding assembly without changing the caller-facing helpers |
| 27 | 38.1 | Low-priority cleanup: two public watermark accessors independently implement exactly the same lookup |
| 28 | 40.1, 37.2 | Delete the UTF-8 splitter fallback for a chunk size the crate explicitly forbids |
| 29 | 40.2 | Replace the incomplete-fleet bookkeeping with the function's existing `Option` exit |
| 30 | 40.3, 37.1 | Simplify the receiver's completion code around its already-enforced private invariant |
| 31 | 41.1, 42.1, 44.1 | The handshake boundary decodes an already-decoded payload again |
| 32 | 44.3 | A struct-level Serde default can replace five identical field annotations on `JoinCode` |
| 33 | 46.1, 48.1 | The captured-pointer path duplicates the ordinary hit path's coordinate conversion |
| 34 | 52.2 | Use the standard iterator reduction to remove manual empty-accumulator handling from numerical history reducers |
| 35 | 53.1 | Remove the former combination-picker catalogue while retaining the raw defaults Workshop actually consumes |
| 36 | 53.2 | Delete the unused validation-badge subsystem instead of carrying a second field-feedback renderer |
| 37 | 54.2 | Workshop ZIP export repeats the save/profile browser-download mechanism |
| 38 | 54.3 | Workshop recovery and source handoff can share IndexedDB connection opening, but not their transactions |
| 39 | 57.1 | Relay pairs expose a mutable limit-negotiation phase that no production path uses |
| 40 | 58.1 | Small concrete opportunity: share delivery-class vocabulary instead of maintaining three equivalent JavaScript declarations |
| 41 | 60.1 | Remove the relay hub's unused writability query and redundant boolean overflow query |
| 42 | 60.2 | Delete the registry's unread creation timestamp |
| 43 | 61.1 | Low priority: generic JSON deep cloning is unnecessary for continuation rows |
| 44 | 61.2 | Low priority: the owner transaction exposes an unused hold callback and roster accessor |
| 45 | 64.1 | Reuse the existing continuation-envelope encoder for chunk envelopes — a small genuine deletion |
| 46 | 65.2, 68.1 | Small confirmed deletion: the Durable Object retains environment configuration it never uses |

## Excluded work

- Explicit conditional or compatibility-dependent proposals: 8.2, 9.1, 12.2, 13.1, 13.2, 16.1, 17.1, 18.2, 28.3, 33.1/36.1, 44.2, 45.1, 48.2, 49.1, 50.2/52.3, 52.1, 56.1, 56.2, 57.2, 66.1/68.2, 70.2/72.1.
- Larger ownership or test organization changes: 7.1, 11.2, 15.2, 19.2, 23.2 (global validation ownership beyond the shared ratio), 26.3, 31.3, 35.2, 43.2, 47.2, 50.1, 51.2.
- Policy decisions: 26.2 (numeric JSON presentation), 54.1 (retained row ordering).
- Root physics/math feature declarations from 4.2 remain; 1.1 covers only the five confirmed redundant implementation dependencies.

## Verification

All checks below passed against the implemented batch:

- `cargo fmt --all -- --check` and workspace clippy across all targets with `server,viewer,debug,headless,perf,host,capture`, denying warnings.
- Full native rerun: **8,991 passed**, zero failed, across 141 suites. `PHOENIX_AMBIGUITY_BASE_REF` was the verified pre-change commit `f01f732d`.
- Independent Simulation: **4,608 passed** with `cargo test -p phoenix-simulation --lib --no-default-features`.
- Presentation: **31 viewer tests passed** with `cargo test -p phoenix-presentation --lib --features viewer viewer::`.
- JavaScript: **8,579 tests passed** across 425 files. Tests solely exercising the retired combination-picker catalogue were removed; Workshop's raw-default coverage remains.
- Layer dependency tree and graph freshness; generated debug surfaces, Live layout, LODs and billboard captures; strict strings.
- PASM validate, scan and traceability.
- Wiki lint: **67 pages and 1,831 references**, no broken or unindexed pages.
- Fresh Trunk WASM host build, pure-JavaScript client build and standalone Workshop build, with the new storage helper included in the packaged distribution.
- **15 Chromium smoke checks passed** on a fresh local port against that bundle: host startup, fetched-layer sibling execution/cache, checkpoint preflight, save portability/refusals, Workshop recovery/export and atomic single-use source handoff.
- **Two scene-rendering smoke checks passed** using software WebGL: the shipped combat scene, and web motes/shadows/flare independent of native settings.

Production Rust and JavaScript source has a net reduction of 1,037 lines, including comments and blank lines, excluding tests and documentation. This includes the new shared helpers. The source diff has no whitespace errors.

Read-only review found and resolved a missing Workshop bundle entry for the new IndexedDB helper, a test-only `PI` import, and two stale comments. The native cross-language delivery-class guard now checks the canonical JavaScript declaration and each preserved public export rather than requiring duplicated literals.

The original `report.md` SHA-256 remains `CE315E3C517B3055779CAE2B7F412A63284F155C18440D1C7FC5F51EF27A2FD5`. Unrelated GDD edits are outside this cleanup batch.
