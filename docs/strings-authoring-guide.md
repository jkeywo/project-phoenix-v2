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
| `en` | The English text. **Wrap any value containing a comma, a double quote or a newline in double quotes** (`"…, …"`), doubling embedded quotes as `""`. An unquoted comma splits the row and truncates the text — see CI below. |

It is standard RFC 4180 CSV, so quoted values may run over several lines; the
comms prose in the file already does.

Add a locale by appending a column (`fr`, `de`, …). No code changes are needed;
`buildTable(csv, 'fr')` reads the column directly — it falls back to `en` only
when the column itself is missing, not for blank cells within it. A
present-but-blank cell resolves to `''`, and the console renders nothing for
that row. Fill the new column for **every** row so CI's field-count check
passes — CI checks that each row has as many fields as the header, so a
half-width column fails the build one row at a time — but a blank cell only
satisfies CI, it gives players nothing to read: carry the English text into
the new column for rows not yet translated, and swap it for the real
translation later.

`scripts/extract-strings.mjs` rewrites the whole file as `id,context,en`; it
does not know about locale columns and drops them. Run it, if at all,
**before** adding a locale column — running it afterwards silently deletes
that column, and the field-count gate stays green, because the header it just
rewrote is 3 wide and every row it just wrote is 3 wide.

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
- a row whose field count does not match the header — usually an unquoted comma
  inside a value, which splits the row and silently truncates player-visible text
  (issue #966); also catches a stray trailing comma or a short row
- a `t()` / `data-i18n` id with no CSV row
- a localisable TOML key still holding prose

CI runs it with `--strict`, so hardcoded display text in the client fails the
build. The only sanctioned English-in-place is developer/operator tooling (the
host debug panel, crash guidance pointing at DevTools) — allowlisted in
`DEV_FACING` inside the checker.

## Tests

Test assertions resolve through the table too, so they survive copy edits:

- **vitest** — `tests/client/setup-strings.js` loads the real CSV before every
  suite; assert with `t('some.id')` from `gui/strings.js`.
- **Playwright smoke** — import `ts()` from `tests/smoke/strings.ts` (its own
  small loader; the smoke package is CommonJS). `:has-text("Captain")`
  selectors survive bracketing because Playwright matches substrings.

## Known gaps

A few strings are still composed in Rust and reach the screen as plain
English (repair-request comms text, game-over reasons, AI sender labels, the
HUD condition token). They need structured id+params payloads rather than a
CSV row — tracked as follow-up work, see the wire-boundary notes in
`gui/strings.js`.
