# Localisation

All display text lives in `assets/strings/strings.csv` (`id,context,en`). The
server is localisation-blind: TOML holds string ids, Rust passes them through
the wire untouched, and the client resolves them once at the message boundary
(`localiseTree()` in `gui/strings.js`, applied in `gui/connection-manager.js`).
Client-side chrome resolves through `t(id, params)` and `data-i18n` attributes,
loaded at boot by `gui/strings-boot.js`.

English text wrapped in `[square brackets]` is agent-drafted placeholder copy;
a human removes the brackets (and edits freely) to approve a line. Re-running
`scripts/extract-strings.mjs` merges by id and never overwrites approved rows.

Names that Rust matches as identifiers — `[[station]] name`,
`[[station.rating]] name`, faction `name` — stay English in TOML;
`scripts/strings-rules.mjs` is the shared authority on which keys are display
text. `scripts/check-strings.mjs` enforces table integrity in CI.

Full authoring workflow: [`docs/strings-authoring-guide.md`](../../docs/strings-authoring-guide.md).
