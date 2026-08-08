/**
 * scripts/strings-csv.mjs — CSV mechanics shared by the strings tooling.
 *
 * `gui/strings.js` owns the *parser*: the client reads assets/strings/strings.csv
 * at runtime, so there must be exactly one definition of what a row is, and every
 * script here reads through its `parseCsv`. This module owns the two things only
 * the tooling needs — writing a field back out safely, and mapping a parsed row
 * back to the physical line it came from.
 */

/**
 * Quote a field if it would otherwise change the shape of the row.
 *
 * A value carrying a comma, a quote or a newline must be quoted or it splits
 * into extra fields on the next read — the truncation bug of issue #966. Every
 * writer of strings.csv goes through here so no generator can reintroduce it.
 *
 * @param {string} s
 * @returns {string}
 */
export function csvField(s) {
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/**
 * Physical (1-based) line number for each row `parseCsv` returned, or `null`
 * where it cannot be proven.
 *
 * A parsed row index is NOT a line number. strings.csv holds multi-line quoted
 * comms prose that swallows 35 physical lines, so by the end of the file the
 * index runs 34 behind: report a bad row by its index and you name an innocent
 * neighbour, which is worse than naming nothing at all.
 *
 * The mapping falls out of the parse rather than needing a second state machine:
 * every physical line is either a row terminator or a newline inside a quoted
 * field, so each row starts where its predecessors' newlines leave off. The one
 * thing `parseCsv` consumes without emitting is a physically blank line (a row of
 * one empty field, which it drops) — stepped over here.
 *
 * Ids carry no comma, quote or newline, so a row always begins a physical line
 * with its own first field. That makes each derived number *checkable*: anything
 * that does not line up yields `null` and the caller falls back to a row number
 * instead of guessing.
 *
 * @param {string} text raw CSV, exactly as handed to `parseCsv`
 * @param {string[][]} rows the rows `parseCsv` returned for that text
 * @returns {(number|null)[]} one entry per row
 */
export function rowLineNumbers(text, rows) {
  // `parseCsv` strips a leading BOM; strip it here too or row 0 fails its check.
  const physical = text.replace(/^\uFEFF/, '').split('\n');
  const at = (n) => (physical[n - 1] ?? '').replace(/\r$/, '');
  const out = [];
  let line = 1;

  for (const row of rows) {
    while (line <= physical.length && at(line) === '') line += 1;
    const head = row[0] ?? '';
    out.push(at(line) === head || at(line).startsWith(`${head},`) ? line : null);
    line += 1 + row.reduce((n, f) => n + (f.split('\n').length - 1), 0);
  }

  return out;
}
