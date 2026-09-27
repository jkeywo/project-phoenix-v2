# First-party German catalogue — machine draft v1

English source base: integrated main `7e826047` (through the named T5 journeys).
The catalogue has **4,655** first-party String IDs. Every row has a nonblank
German value, `de_source` equal to its exact English cell, and nonblank
`de_provenance`. Generated values are marked `machine`; no human editorial
approval is claimed. The 3,917 bracketed English values remain bracketed and
retain their own approval status. Translation is offline authoring work: the
game has no translation service or model dependency.

The draft used Argos Translate 1.11.0 with the English-to-German model package
version 1.3. Source-bound AI corrections in `scripts/german-domain-draft.json`
and `scripts/german-dynasty-draft.json` keep proper transport and ship names
intact and repair ambiguous scenario and onboarding terminology. The generator protects interpolation parameters, URLs and issue
references; a source change invalidates both reuse and any matching correction.

## Automated evidence

- `uv run python scripts/test_generate_german.py` — 6/6.
- `npx vitest run tests/client/german-catalogue.test.js tests/client/strings.test.js tests/client/string-catalogue.test.js tests/client/extract-strings.test.js editor/tests/mod-pack-export.test.js` — 138/138.
- `node scripts/check-strings.mjs --strict` — 4,655 strings, 0 errors, 0 warnings.
- `uv run pasm validate`, `uv run pasm scan`, `uv run pasm traceability` —
  Status/Validation OK on the updated architecture model.
- Wiki lint — 65 pages, 1,316 existing sources, 20 Rust line references and
  63 index links; 0 broken references.
- `tests/client/german-catalogue.test.js` asserts complete row coverage,
  source freshness, composed placeholder/plural validity, English fallback for
  stale and third-party entries, extraction merge preservation, and exact
  translation-pack ZIP export/reopen.
- A UTF-8 structural scan found 0 empty German cells, 0 stale sources,
  0 missing provenance cells, 0 unreplaced mask tokens and 0 replacement
  characters.
- Comparing parsed rows against integrated English source `7e826047` found
  0 changed IDs, contexts or English values.

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
