# Localisation

All display text lives in `assets/strings/strings.csv` (`id,context,en`). The
server is localisation-blind: TOML holds string ids, Rust passes them through
the wire untouched, and the client resolves them once at the message boundary
(`localiseTree()` in `gui/strings.js`, applied in `gui/connection-manager.js`).
Client-side chrome resolves through `t(id, params)` and `data-i18n` attributes,
loaded at boot by `gui/strings-boot.js`.

A text id may be joined on the wire by a sibling field named `<field>_params`
(`ObjectiveSnapshot::text_params`, `CommsMessage::body_params`). `localiseTree`
finds it by name and resolves `t(id, params)`, so a figure the server computed
lands inside the sentence instead of only on a panel beside it. A script authors
it as an optional `params` / `text_params` key; an empty table is not sent at
all, so payloads that name a figure-free string are unchanged.
`TEXT_PARAMS_SUFFIX` in `src/core/messages.rs` is the contract.

English text wrapped in `[square brackets]` is agent-drafted placeholder copy;
a human removes the brackets (and edits freely) to approve a line. Re-running
`scripts/extract-strings.mjs` merges by id and never overwrites approved rows.

Names that Rust matches as identifiers — `[[station]] name`,
`[[station.rating]] name`, faction `name` — stay English in TOML;
`scripts/strings-rules.mjs` is the shared authority on which keys are display
text. `scripts/check-strings.mjs` enforces table integrity in CI.

Full authoring workflow: [`docs/strings-authoring-guide.md`](../../docs/strings-authoring-guide.md).
