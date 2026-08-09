/**
 * scripts/strings-rust.mjs — Find player-visible English composed in Rust.
 *
 * AGENTS.md rule 11 forbids hardcoded player-visible English. The string-table
 * gate (check-strings.mjs) enforces it across the client, but nothing scanned
 * `src/` until issue #975: a `format!` / `.to_string()` that builds display
 * text and crosses a host channel was invisible here, so it shipped as English
 * over the wire while the CSV stayed green. This is the rule that keeps that
 * class closed on the modules it was closed on.
 *
 * It is deliberately NARROW and sound rather than broad. check-strings.mjs runs
 * it over a short allowlist of modules known to compose wire-visible text, not
 * all of `src/` — a blanket `format!` scan would drown in log lines, error
 * strings and machine tokens, and a gate that cries wolf gets a blanket
 * suppression. Within a scanned file it:
 *
 *   - reads only the PRODUCTION region (everything before the first
 *     `#[cfg(test)]`), because a test fixture that hand-writes an English label
 *     is not shipped to a player;
 *   - skips comments and the insides of char literals / raw strings;
 *   - flags a string literal only when it reads as PROSE — after stripping
 *     `{…}` format placeholders it still contains a letter AND either an
 *     uppercase letter or a space. That is the same capitalisation/spacing
 *     signal the markup scanner (scripts/strings-markup.mjs) draws its line at,
 *     so an id (`server.hud_alert`) and a machine token (`tactical`) pass while
 *     `"Designating target: {label}"` and `"ALERT"` do not.
 *
 * Unit-tested in tests/client/strings-rust.test.js against both the shapes it
 * must catch and the tokens it must not.
 */

/** Everything before the first `#[cfg(test)]` — the shipped, non-test code. */
export function productionRegion(src) {
  const idx = src.indexOf('#[cfg(test)]');
  return idx === -1 ? src : src.slice(0, idx);
}

/**
 * Does this literal read as player-visible English rather than an id or a
 * machine token? Placeholders are stripped first so `"{label} Offline"` is
 * judged on `" Offline"` (space + letter → prose) and `"station.{}.name"` on
 * `"station..name"` (lowercase, dotted → id).
 * @param {string} content the raw text between the quotes
 */
export function readsAsProse(content) {
  const residue = content.replace(/\{[^}]*\}/g, '');
  if (!/[A-Za-z]/.test(residue)) return false; // punctuation / format glue only
  return /[A-Z]/.test(residue) || /\s/.test(residue);
}

/**
 * Extract every string-literal body from Rust source, skipping comments, char
 * literals and lifetimes. Handles plain `"…"` (with escapes) and raw strings
 * (`r"…"`, `r#"…"#`, and their `b`-prefixed forms). Line numbers are 1-based
 * and point at the line the literal opens on.
 * @param {string} src
 * @returns {{ line: number, text: string }[]}
 */
export function stringLiterals(src) {
  const out = [];
  const n = src.length;
  let i = 0;
  let line = 1;

  while (i < n) {
    const c = src[i];

    if (c === '\n') { line += 1; i += 1; continue; }

    // Line comment.
    if (c === '/' && src[i + 1] === '/') {
      i += 2;
      while (i < n && src[i] !== '\n') i += 1;
      continue;
    }

    // Block comment (Rust allows nesting).
    if (c === '/' && src[i + 1] === '*') {
      i += 2;
      let depth = 1;
      while (i < n && depth > 0) {
        if (src[i] === '\n') line += 1;
        else if (src[i] === '/' && src[i + 1] === '*') { depth += 1; i += 1; }
        else if (src[i] === '*' && src[i + 1] === '/') { depth -= 1; i += 1; }
        i += 1;
      }
      continue;
    }

    // Raw string: optional `b`, then `r`, then #* then `"`.
    if (c === 'r' || (c === 'b' && src[i + 1] === 'r')) {
      let j = c === 'b' ? i + 1 : i; // at the `r`
      j += 1; // past `r`
      let hashes = 0;
      while (src[j] === '#') { hashes += 1; j += 1; }
      if (src[j] === '"') {
        j += 1;
        const startLine = line;
        const closer = `"${'#'.repeat(hashes)}`;
        let content = '';
        while (j < n) {
          if (src.startsWith(closer, j)) { j += closer.length; break; }
          if (src[j] === '\n') line += 1;
          content += src[j];
          j += 1;
        }
        out.push({ line: startLine, text: content });
        i = j;
        continue;
      }
      // Not a raw string after all (e.g. an identifier starting with `r`).
    }

    // Char literal or lifetime. Neither can hold a `"` that opens a string, so
    // stepping past them keeps the string scanner honest (`'"'` must not be
    // read as the start of a string).
    if (c === "'") {
      if (src[i + 1] === '\\') {
        let j = i + 2;
        if (src[j] === 'u' && src[j + 1] === '{') { while (j < n && src[j] !== '}') j += 1; j += 1; }
        else j += 1;
        if (src[j] === "'") j += 1;
        i = j;
        continue;
      }
      if (src[i + 2] === "'") { i += 3; continue; } // 'x'
      i += 1; // lifetime `'a`
      continue;
    }

    // Plain string literal.
    if (c === '"') {
      i += 1;
      const startLine = line;
      let content = '';
      while (i < n) {
        const d = src[i];
        if (d === '\\') { content += d + (src[i + 1] ?? ''); i += 2; continue; }
        if (d === '"') { i += 1; break; }
        if (d === '\n') line += 1;
        content += d;
        i += 1;
      }
      out.push({ line: startLine, text: content });
      continue;
    }

    i += 1;
  }

  return out;
}

/**
 * The rule: prose-reading string literals in the production region of a Rust
 * source. An empty result means the module composes no player-visible English.
 * @param {string} src
 * @returns {{ line: number, text: string }[]}
 */
export function proseLiterals(src) {
  return stringLiterals(productionRegion(src)).filter(({ text }) => readsAsProse(text));
}
