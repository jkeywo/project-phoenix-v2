# Browser pre-init content coverage (#1248)

Acceptance map for the #1449 continuation of #1316. The existing
`browser_resume_versions` script-source-set fix is retained. Discovery uses
`lift_world_scripts`, `OverlayScriptResolver`, `script_source_ledger_digest`,
and the existing composed-template resolver rather than introducing a second
content identity.

| Criterion | Evidence owner |
| --- | --- |
| Root and static child siblings before compatibility | `world_preload::tests`, real `preinit-content.spec.js` local/import cases |
| Literal spawn hulls and transitive includes | Shared config preload; browser sibling-only hull includes a separate fragment |
| Missing source/template/include refuses before App | Browser missing-path cases; terminal Rust error state; JS timeout test |
| Changed sibling and composed hull refuse old saves | Browser local-slot and import cases for both edits |
| Completion order preserves identity | Pure discovery digest comparison; browser unchanged cases reverse independent script response order |
| Overlay authority | Production overlay resolver; existing #1316 overlay tests; root source uses resident overlay before HTTP |
| Computed post-freeze paths unchanged | Existing literal scanner and ledger freeze remain the owners |

Independent review identified promotion of a resident include fragment to an
entity root as an order-sensitive case. Discovery now runs that resident source
through `wasm_load_config`, so the existing resolution/ingestion loop runs even
when no further HTTP response will arrive.

Validation on the implementation working tree above base `f84cce55`:

- `npx vitest run tests/client/host-content-fetch.test.js tests/client/design-tokens.test.js`:
  **319 passed**, two files, exit 0. Five delivery tests cover body completion,
  empty scripts, required 404/500, timeout, and optional sidecar absence.
- `cargo test --lib --features headless -- world_preload::tests browser_preinit every_shipped_ladder_declares_the_tier_rig_its_files_actually_have`:
  **9 passed, 0 failed, 0 ignored**, exit 0. Eight cover discovery and the
  retained #1316 seam; the ninth belongs to #1239's LOD repair.
- `trunk build`, with `NO_COLOR=true`, produced the real development WASM
  bundle `project-phoenix-c5a6d04188e75e54`; `node scripts/build-client.mjs`
  rebuilt the phone page. Trunk required access to its existing wasm-bindgen
  cache. Existing target-feature/core_plugins warnings remained.
- `preinit-content.spec.js` in Chromium: **all eleven named cases passed**
  across two targeted runs on that same bundle. Four local-slot cases passed
  on port 3155; seven import/missing cases passed on port 3156 (46.1 seconds,
  exit 0). Both accepted-content cases reverse independent script delivery
  timing; both promotion cases deliver the fragment before/after discovery.
  No failed or ignored case is counted as a pass.

Initial test-authoring failures were corrected: the real content refusal says
“authored data has changed”, and the first-paint catalogue can appear before
the import change handler's WASM-ready guard opens. Tests now assert the actual
dimension-specific refusal and wait for catalogue population. The first two
runs were interrupted after those findings; they are not green suite receipts.
No application assertion or compatibility check was weakened.

Independent read-only review found no remaining implementation issue after the
resident-fragment fix. These are focused results; final integration and wider
boot/reconnect smoke remain separate.
