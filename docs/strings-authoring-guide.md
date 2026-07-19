# String table authoring guide

All user-facing text lives in `assets/strings/strings.csv`. This is the one
sanctioned exception to AGENTS.md rule 11 ("tunable values belong in TOML"):
text needs a translator-facing format with a context column, which TOML's
per-file layout does not give us.

## The file

```csv
id,context,en
entity.alliance_cruiser.name,"assets/entities/alliance_cruiser.toml → name (top level)","[Alliance Cruiser]"
console.sensors.contacts,"Sensors console — contact count. {n} is the number of radar blips","[{n} CONTACTS]"
```

| Column | Purpose |
| --- | --- |
| `id` | Stable key that code and TOML refer to. Never reuse an id for different text. |
| `context` | Where it appears and what any `{placeholder}` means. This is what a translator reads — a bare file path is the minimum, a sentence is better. |
| `en` | The English text. |

Add a locale by appending a column (`fr`, `de`, …). No code changes are needed;
`buildTable(csv, 'fr')` reads it and falls back to `en` for blank cells.

## Square brackets mean "not reviewed yet"

Text an agent wrote is wrapped in `[square brackets]`. It renders bracketed in
game, so anything still in brackets on screen is visibly a first draft.

When a human approves a line, they **remove the brackets** and edit the text as
they see fit. Nothing else changes. Re-running the extractor will not undo this:
it merges by id and never overwrites an existing row.

An unbracketed string on screen means someone signed it off.

## Adding a string

**In a console or component** — add a row to the CSV, then reference it:

```html
<h2 data-i18n="console.sensors.scan_summary"></h2>
<button data-i18n-attr="title:console.sensors.cancel_tip">…</button>
```

```js
import { t } from '../strings.js';
el.textContent = t('console.sensors.contacts', { n: blips.length });
```

Import `./strings-boot.js` once from the page's entry point so the table is
loaded before anything renders.

**In TOML** — write prose as normal, then run:

```
node scripts/extract-strings.mjs
```

It pulls the new text into the CSV and replaces it in the TOML with the
generated id. The script is idempotent: already-migrated values are skipped, so
re-running is safe and only picks up what is new.

## Placeholders and plurals

`t()` substitutes `{name}` tokens. Prefer a placeholder over string
concatenation — `[HEADING {deg}]` rather than `'HEADING ' + deg` — because word
order differs between languages.

There is no plural rule. Use two ids (`.one` and `.other`) and pick between them
at the call site. This is deliberate: a real plural system is only worth it once
a language with non-binary plurals is actually on the roadmap.

## Which TOML keys get localised

`display_name`, `label`, `description`, `message`, `text`, `title`, `from`,
`speaker` — always.

`name` is the awkward one, because it is sometimes display text and sometimes a
lookup key, with nothing in the syntax to tell them apart. Getting it wrong
fails *silently*: the lookup stops matching and whatever it gated quietly stops
working. `scripts/strings-rules.mjs` is the single source of truth, shared by
the extractor and the CI checker. Today:

- **Localised**: world `[[entity]] name`, and the top-level `name` of an entity
  template.
- **Left as identifiers**: `[[station]] name`, `[[station.rating]]  name`,
  faction `name`, and the wave/range-band names in `combat_test.toml`. These are
  matched by Rust (`get_station`, `rating_for_station`, `ai::faction`). They all
  carry a real `id`/`uuid` beside the name, so the client localises them by
  deriving an id from that instead.

If you add a new lookup-by-name in Rust, update `strings-rules.mjs` in the same
change.

## How resolution works

The server is localisation-blind. TOML holds string ids, Rust passes them
through the wire untouched, and the client resolves them in one place —
`localiseTree()` in `gui/strings.js`, applied at the two `onData` ingress points
in `gui/connection-manager.js`. Only strings present in the table are
substituted, so uuids, system ids and tokens pass through unchanged.

This means no Rust code needs to know about text, and no console needs to know
which of its fields are localisable.

Rust tests therefore assert on **ids**, not English — the id is Rust's contract.

## CI

`node scripts/check-strings.mjs` runs in the `editor-test` job and fails on:

- duplicate or blank ids
- a `t()` / `data-i18n` id with no CSV row
- a localisable TOML key still holding prose

It also warns about hardcoded text still in the client. Those are warnings
rather than errors while the client migration is in progress; once `gui/` is
fully migrated, switch the job to `--strict` so they fail the build.
