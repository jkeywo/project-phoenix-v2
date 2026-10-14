# Simulation composition integration

The five structural stages from `codex/simulation-composition` are integrated
into local main, preserving gameplay redesign through increment 5. No gameplay
or content tuning is included. This receipt replaces the old branch's evidence.

Work began at `f14895ca87379b0a4823054f90ba1c32ee62b5b4`. During implementation,
main advanced to documentation commit `e69d655dd2550968b205561883388cc5ce3f4c83`.
The integration was rebased onto it before main was fast-forwarded. All final
gates below tested runtime revision `62ba40b3644dfbacf4450d17f2844ec9dd44629a`;
the subsequent receipt update changes documentation only. Nothing was pushed.
The source branch remains at `d0a4d7b176a84a001316af0a9ae8939ac763988d`.

## Integrated stages

| Stage | Commit | Ownership |
|---|---|---|
| Offline preparation | `d38503bd2bb81e60220a5b9ed5f3bf848e15477f` | `phoenix-content-preparation`: loading, validation and Workshop authoring |
| World execution | `e49752b426ec30897b030231001a3d11c6c16853` | Explicit state, registration, activation, spawn, trigger, effect, layer and publication modules |
| Snapshot continuity | `e2f817be4a3bd18ce4b8d4ce507ac9b96cc4221e` | Schema, capture, artifacts, preflight, materialisation, restoration and application |
| GM organisation | `f84c3d176a8fc26f13307233daabc29e0c2efb92` | Actions, operations, observation and continuity |
| Host delivery | `62ba40b3644dfbacf4450d17f2844ec9dd44629a` | `phoenix-delivery`: HTTP and version codecs over injected content sources |

Snapshot format **41**, simulation rules **0.13** and protocol **6** are unchanged.
Compatibility facades preserve callers. Console authority and knowledge boundaries,
System admission, AI continuation, observation provenance and canonical repair-job
ownership retain main's implementation. Live content adapters stay in Simulation;
CLI handling, template preload and global-cache access stay in Host. Simulation
does not depend on Delivery, and Preparation does not depend on Simulation.

## Physical Rust lines

Each cell is **other source / dedicated test-file lines**, including blank lines
and comments. Inline tests count with their source file. Host means root `src/`.
These are physical ownership counts, not a claim that gameplay code was deleted.

| Revision | Simulation | Preparation | Delivery | Host |
|---|---:|---:|---:|---:|
| Pre-integration `e69d655d` | 145,642 / 199,091 | 0 / 0 | 0 / 0 | 56,573 / 38,688 |
| Preparation `d38503bd` | 134,747 / 187,637 | 10,970 / 11,459 | 0 / 0 | 56,573 / 38,688 |
| World `e49752b4` | 134,851 / 187,639 | 10,970 / 11,459 | 0 / 0 | 56,573 / 38,688 |
| Snapshot `e2f817be` | 135,047 / 187,663 | 10,970 / 11,459 | 0 / 0 | 56,573 / 38,688 |
| GM `f84c3d17` | 135,279 / 187,757 | 10,970 / 11,459 | 0 / 0 | 56,573 / 38,688 |
| Delivery `62ba40b3` | 132,367 / 185,265 | 10,970 / 11,459 | 1,915 / 1,011 | 57,668 / 40,267 |

## Validation on 14 October 2026

[Machine-readable receipt](simulation-composition-validation.json) records exact
commands, revisions, log hashes and baseline comparisons. Large logs and raw
captures remain under ignored `.phoenix/composition-integration/`.

- Owner tests: Preparation **300**, Delivery **56**, independent Simulation with
  `--no-default-features` **4,256** passed. The intermediate World/snapshot run
  passed 4,385 tests, including one subsequently removed duplicate relocated test.
- Final ordinary AGENTS gates all passed once on integrated main: formatting,
  required Clippy features with denied warnings, workspace/headless tests
  (**9,112 passed, 102 ignored**), debug surfaces, live layout, Vitest
  (**8,732 passed in 438 files**), strict strings, both LOD checks, and PASM
  validate/scan/traceability. PASM's informational warnings remain informational.
  `PHOENIX_AMBIGUITY_BASE_REF` was the verified pre-integration main `e69d655d`,
  whose runtime sources match initial main. No ambiguity golden was regenerated.
- Layer checks, the independent dependency-tree check and graph freshness passed.
  Viewer configuration passed **31** tests; demo configuration passed **11**.
  Host and capture binaries built successfully.
- Fresh server and Workshop-preview WASM development bundles built successfully;
  the client bundle and preview copy completed. **63 Playwright tests** passed
  across Chromium and render projects: Workshop authoring/composition/preview,
  Console/Dock editing and ownership, repair, packs, saves, snapshot portability
  and channels, and Viewscreens including texture recovery.
- Workspace regressions include runtime World loading and deterministic identities,
  schedule census, Console authority/knowledge publication, AI and observation
  continuation, fleet recovery and GM restore, queued repair redirects and invalid
  job-state refusal. Workshop coverage includes inherited definitions, AI trees,
  repair workflows, shipped Courier content and bounded schema replies.
- Source review retained all **48 World functions**, **114 snapshot functions**
  and **56 snapshot schema types**. Function differences were module paths,
  visibility/import placement and formatting; schema fields were unchanged.
- Wiki schema lint passed: source files and local links resolve, index coverage
  matches pages, and moved ownership references point to the new paths.

## Increment-5 comparison

The reference is `e858616a`, whose runtime/content files match pre-integration main.
Both Combat Test and Falling Skyway used the six-seat Alliance Cruiser, seed 42,
60 simulated seconds, and a fresh `measure` executable. Both repeated ordinary
reports match increment 5 **byte for byte**. Replay checkpoints pass, and all six
performance-capture continuation digests match the reference.

Performance-enabled report differences were inspected field by field: only
`console_latency.actions[*].admit_to_broadcast.{p50_ms,p75_ms,max_ms}` changed.
These are measured wall-clock service times; all other report fields match.
No expected report, authored value or balance threshold was changed.

| World | Increment-5 tick p50 ms | Integrated tick p50 ms | Increment-5 tick p95 ms | Integrated tick p95 ms |
|---|---:|---:|---:|---:|
| Combat Test | 1.420–1.451 | 1.412–1.453 | 2.008–2.042 | 1.946–1.985 |
| Falling Skyway | 1.375–1.401 | 1.361–1.372 | 1.820–2.012 | 1.805–1.898 |

Ranges cover three same-machine captures excluding the first 300 harness updates.
No competing Phoenix build, browser suite or simulation ran during capture.
These diagnostic timings are not release-performance or device acceptance.

The unchanged Cruiser matrix produced **20/20 victories**, passed **all 16
thresholds**, and yielded a merged result byte-identical to increment 5.

## Preservation and remaining acceptance

The initial 35-file edit inventory and exact backups remain in
`.phoenix/composition-integration/original-edits.json` and `original-files/`.
Another actor committed those drafts in `e69d655d` while this work ran; that
commit was preserved. Thirty-three files retain their original bytes. The updated
GDD current-baseline file matches that intervening commit exactly; the remaining
wiki file differs only by its relocated snapshot source link. There is no GDD
diff between `e69d655d` and this integration. This task did not stage those drafts.

Physical phones, native multi-monitor displays, real networks and human workload
remain pending where recorded in the gameplay redesign acceptance ledger.
Automated refactor validation does not close those exits.
