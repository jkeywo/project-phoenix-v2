# First-party German catalogue — machine draft v1

English source base: integrated main `da812fb0` (27 September 2026).
The catalogue has **4,671** first-party String IDs. Every row has a nonblank
German value, `de_source` equal to its exact English cell, and nonblank
`de_provenance`. Generated values are marked `machine`; no human editorial
approval is claimed. The 3,933 bracketed English values remain bracketed and
retain their own approval status. Translation is offline authoring work: the
game has no translation service or model dependency.

The draft used Argos Translate 1.11.0 with the English-to-German model package
version 1.3. Source-bound AI corrections in `scripts/german-domain-draft.json`
and `scripts/german-dynasty-draft.json` keep proper transport and ship names
intact and repair ambiguous scenario and onboarding terminology. The generator protects interpolation parameters, URLs and issue
references; a source change invalidates both reuse and any matching correction.

Since the original `7e826047` draft, the integrated source added 16 IDs:
two Workshop Script/Objective diagnostics, eight server fleet refusals, five
client join refusals, and one Objective progress label. A parsed-row comparison
found no removed IDs or changes to any earlier ID's context or English value.
The 16 added rows have source-matched German values and `machine` provenance.

## Automated evidence

- `.venv/Scripts/python.exe scripts/test_generate_german.py` — 6/6.
- `npx vitest run tests/client/german-catalogue.test.js tests/client/strings.test.js tests/client/string-catalogue.test.js tests/client/extract-strings.test.js editor/tests/mod-pack-export.test.js` — 138/138.
- `npx vitest run tests/client/t5-journeys.test.js tests/client/host-journey-localisation.test.js tests/client/gm-direct-effect-panel.test.js tests/client/gm-journal-panel.test.js tests/client/game-over-view.test.js editor/tests/script-editor-view.test.js` — 139/139 representative journey, GM, report and Workshop outcomes.
- `node scripts/check-strings.mjs --strict` — 4,671 strings, 0 errors, 0 warnings.
- `uv run pasm validate`, `uv run pasm scan`, `uv run pasm traceability` —
  Status/Validation OK on the original draft's architecture model at `7e826047`.
  These PASM commands were not repeated for this catalogue closeout.
- Wiki lint at the original `7e826047` draft — 65 pages, 1,316 existing
  sources, 20 Rust line references and 63 index links; 0 broken references.
  This lint was not repeated for the catalogue closeout.
- `tests/client/german-catalogue.test.js` asserts complete row coverage,
  source freshness, composed placeholder/plural validity, English fallback for
  stale and third-party entries, extraction merge preservation, and exact
  translation-pack ZIP export/reopen.
- A UTF-8 structural scan at `da812fb0` found 0 duplicate IDs, 0 empty German
  cells, 0 stale sources, 0 missing provenance cells, 0 unreplaced mask tokens
  and 0 replacement characters. All 4,671 provenance cells are `machine`.
- Comparing parsed rows against `7e826047` found 16 added IDs and 0 removed
  IDs or changed contexts or English values on existing IDs.

## Bounded human acceptance

The next review should assess language quality, especially domain terms and
scenario narrative; structural checks cannot establish fluency. Representative
surfaces are Console Settings/Comms, GM intervention and journal, Workshop
Script/Test/Localisation, Dynasty onboarding, the Alliance convoy's recovery
and pursuit report, and Cruiser Elimination's team and outcome copy. Switch
each mounted surface to German, inspect a long accented label and one
parameterized count, then return to English. Confirm that source drafts,
focus and authority are unchanged. A changed English line or an English-only
mod entry must still show the effective English fallback until its German
translation is refreshed.
