---
title: Localisation
type: concept
tags: [localisation, strings, client, display-text]
sources: [assets/strings/strings.csv, gui/csv.js, gui/string-catalogue.js, gui/strings.js, gui/strings-boot.js, gui/locale-preference.js, gui/locale-edit-context.js, gui/surface-language.js, gui/gm-language.js, gui/gm-workspace.js, gui/gm-entity-tree.js, gui/gm-direct-effect-panel.js, gui/gm-activity-feed.js, gui/gm-journal-panel.js, gui/game-over-view.js, gui/workshop-authoring.js, gui/workshop-layout-model.js, gui/workshop-boot.js, gui/workshop-scripts-panel.js, gui/workshop-test-panel.js, gui/rendezvous-transport.js, gui/workshop-localisation-panel.js, gui/snapshot-status.js, editor/script-editor-view.js, editor/workshop-localisation.js, src/core/messages.rs, src/lobby/handler.rs, src/world/mod_pack.rs, src/server/bridge.rs, server.html, scripts/check-strings.mjs, scripts/extract-strings.mjs, scripts/generate-german.py, scripts/german-domain-draft.json, scripts/german-dynasty-draft.json, docs/strings-authoring-guide.md, docs/acceptance/1538-localised-host-journey.md, docs/acceptance/1554-german-catalogue-v1.md]
updated: 2026-09-27
---

# Localisation

All display text lives in `assets/strings/strings.csv` (`id,context,en` plus
German value, exact English source and provenance columns). The
server is localisation-blind: TOML holds string ids, Rust passes them through
the wire untouched, and the client resolves them once at the message boundary
(`localiseTree()` in `gui/strings.js`, applied in `gui/connection-manager.js`).
Client-side chrome resolves through `t(id, params)` and `data-i18n` attributes,
loaded at boot by `gui/strings-boot.js`.
The browser host's one-shot save and restore status bridge is a separate
presentation edge: Rust sends an outcome kind and typed parameters, and
`gui/snapshot-status.js` resolves its String Id at the host page. External
storage and version error details remain parameters within translated frames.

Ordinary mods can carry a partial `assets/strings/strings.csv`. Welcome projects
those catalogues in load order and `gui/string-catalogue.js` composes them over
the shipped table per id/locale, with later packs winning. Missing, blank,
invalid, or stale translations render effective English. The retained report
names conflicts, winners, sources and freshness for author tooling; it never
adds diagnostic markers to player text. `<locale>_source` records the exact
English value translated and `<locale>_provenance` records its origin.
The phone's browser/private choice is installed in the shell and every Console
iframe realm before component construction, then repeated on iframe reload and
reconnect. A live choice updates mounted realms in place; raw semantic
Objective/Comms snapshots retained beside resolved delivery are rendered again
in the new locale without replaying game messages. Console edits, focus and
scroll are restored around that presentation repaint. The browser GM and
Workshop each keep a separate private locale. The GM repaints retained semantic
activity, intervention feedback, journal and entity tree from retained semantic
state. Its numeric display parameters stay numbers until catalogue formatting,
and authored String Id names resolve at the tree and inspector without changing
the selected entity or a literal operator name. The end report resolves its
semantic narrative at presentation time while preserving literal lines and
ship boundaries. Workshop repaints its mounted editor without
replacing the draft and exposes a temporary preview choice for translation
tooling. Workshop consumes the retained report in its Localisation panel,
searches effective String Ids and context, and edits translation values and
provenance in the ordinary undo/save/export document flow. The editor checks
placeholder names through the composed runtime report; changing a value records
its effective English source, while an unchanged stale value needs an explicit
source-metadata refresh. A pack without a String Table can start a locale here.
Workshop's Script editor, Test controls and diagnostics repaint from String IDs
without remounting source inputs. Technical source paths, compiler details and
unknown authored prose remain literal inside translated status frames. The
strict string scan includes the Workshop page and mounted Script editor view,
and checks thrown String IDs in the Script and Objective snippet helpers.
Runtime findings retain semantic severity through a locale repaint while file
paths and compiler detail remain literal. Unknown provider errors receive a
translated refusal frame without treating their message as a String ID.

A text id may be joined on the wire by a sibling field named `<field>_params`
(`ObjectiveSnapshot::text_params`, `CommsMessage::body_params`). `localiseTree`
finds it by name and resolves `t(id, params)`, so a figure the server computed
lands inside the sentence instead of only on a panel beside it. A script authors
it as an optional `params` / `text_params` key; an empty table is not sent at
all, so payloads that name a figure-free string are unchanged.
`TEXT_PARAMS_SUFFIX` in `src/core/messages.rs` is the contract.
Numeric presentation parameters format with `Intl.NumberFormat` in the private
locale; typed ISO date/time values use `Intl.DateTimeFormat`. Counted interface
text uses `.one`/`.other` String Id families through `tPlural()` and
`Intl.PluralRules`. Catalogue composition reports missing or invalid plural
forms while the renderer falls back to effective English.

English text wrapped in `[square brackets]` is agent-drafted placeholder copy;
a human removes the brackets (and edits freely) to approve a line. Re-running
`scripts/extract-strings.mjs` merges by id, never overwrites approved rows, and
preserves every locale and metadata column while appending new English rows.
`scripts/generate-german.py` creates source-fresh offline machine German drafts
without adding a game runtime translation dependency. `machine` provenance
identifies those drafts for later editorial review.

Names that Rust matches as identifiers — `[[station]] name`,
`[[station.rating]] name`, faction `name` — stay English in TOML;
`scripts/strings-rules.mjs` is the shared authority on which keys are display
text. `scripts/check-strings.mjs` enforces table integrity in CI.

Full authoring workflow: [`docs/strings-authoring-guide.md`](../../docs/strings-authoring-guide.md).
