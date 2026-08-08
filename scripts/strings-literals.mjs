/**
 * scripts/strings-literals.mjs — finding un-localised display text in client JS.
 *
 * The check-strings gate reports a `.textContent = 'some English'` that never
 * became a string id. That rule used to anchor on a quote IMMEDIATELY after the
 * `=`, which made it blind to every shape a real assignment takes beyond the
 * simplest one: a ternary (`s.red_alert ? 'WEAPONS HOT' : 'CLEAR'`), a fallback
 * (`s.scenario_title || 'Unknown Scenario'`) and template literals
 * (`` `${sp} (waiting)` ``) all passed silently, so "the client sweep is
 * complete" was a claim the gate could not actually back.
 *
 * The rule here scans the whole right-hand side instead, which means it must
 * also decide which of the literals it finds are *display text* — hence the two
 * predicates below rather than one regex.
 */

/**
 * Every literal text run inside a JS expression: `'…'`, `"…"`, and the literal
 * segments of a `` `…${x}…` `` template.
 *
 * A template's `${…}` is code, not text, so it ENDS the current run and starts
 * a fresh one after the closing brace — otherwise the interpolation's own
 * source (and any machine token inside it) would read as prose, and a
 * pluralising `` `slot${n === 1 ? '' : 's'}` `` would be scored as one long
 * string rather than as the English either side of it.
 *
 * Each run carries the expression text on either side of its delimiters so a
 * caller can tell display text from a token being compared. A template's
 * segments all share the whole literal's surroundings.
 *
 * @param {string} expr
 * @returns {{ text: string, before: string, after: string }[]}
 */
export function literalRuns(expr) {
  const runs = [];
  for (let i = 0; i < expr.length; i += 1) {
    const quote = expr[i];
    if (quote !== "'" && quote !== '"' && quote !== '`') continue;
    const opened = runs.length;
    let run = '';
    let j = i + 1;
    for (; j < expr.length; j += 1) {
      const c = expr[j];
      if (c === '\\') { j += 1; run += expr[j] ?? ''; continue; }
      if (c === quote) break;
      if (quote === '`' && c === '$' && expr[j + 1] === '{') {
        runs.push({ text: run });
        run = '';
        // Skip the interpolation, counting braces so a nested object literal
        // or template inside it does not close it early.
        let depth = 1;
        for (j += 2; j < expr.length && depth > 0; j += 1) {
          if (expr[j] === '{') depth += 1;
          else if (expr[j] === '}') depth -= 1;
        }
        j -= 1;
        continue;
      }
      run += c;
    }
    runs.push({ text: run });
    const before = expr.slice(0, i);
    const after = expr.slice(j + 1);
    for (let k = opened; k < runs.length; k += 1) Object.assign(runs[k], { before, after });
    i = j;
  }
  return runs;
}

/**
 * Whether a run reads as prose rather than as a css value, an element id or a
 * single token used as a key: it must hold a word (three consecutive letters)
 * AND either contain a space or be all-caps display text.
 *
 * A multi-word css class LIST ("station-card empty per-slot") passes this, so
 * the caller must only ever hand it right-hand sides of `.textContent` — where
 * a class list cannot appear. Pointing the same predicate at `.className`
 * would report every one of them.
 *
 * @param {string} text
 * @returns {boolean}
 */
export function isDisplayText(text) {
  return /[A-Za-z]{3}/.test(text) && (/\s/.test(text) || /^[A-Z][A-Z ]{2,}$/.test(text));
}

/**
 * Whether a run is an operand of an equality test — a machine token being
 * COMPARED, never text being shown:
 *
 *     (s.grid_status || 'GRID NOMINAL') === 'GRID OFFLINE' ? t(…) : t(…)
 *
 * displays neither literal. Closing parens may sit between the literal and the
 * operator, so they are allowed through.
 *
 * @param {{ before: string, after: string }} run
 * @returns {boolean}
 */
export function isTokenTest({ before, after }) {
  return /(?:===|!==|==|!=)\s*$/.test(before) || /^\s*\)*\s*(?:===|!==|==|!=)/.test(after);
}

/**
 * A `.textContent = …` assignment, capturing the whole right-hand side.
 *
 * `[^;]*` stops at the FIRST `;`, which is only the statement terminator when
 * the right-hand side itself contains none. It spans newlines, so a
 * right-hand side wrapped onto the next line is still seen — but a `;`
 * nested inside the right-hand side (a callback body, say) truncates the
 * capture before the real end of statement, and everything after that
 * nested `;` — including a later ternary arm — is invisible to the scan.
 * Live at gui/battleship/sensors.html:104, where
 * `shields.some(function(f) { return f.online === false; }) ? t(…) : t(…)`
 * is cut at the `return`'s `;`, so neither ternary arm is seen; benign today
 * because both arms are already `t()` calls, but it is exactly the ternary
 * shape this scan exists to catch. A `;` inside the string literal itself is
 * worse: the capture stops mid-quote and the resulting unterminated run
 * silently fails `isDisplayText` (no trailing all-caps run, no space before
 * the cut), so `el.textContent = 'Warning; check the grid';` is reported as
 * `[]` — no warning, no error. A depth-tracking scanner is the real fix;
 * tracked as the scanner gap on issue #976 alongside the gate's other blind
 * spots (component templates, attributes).
 *
 * Stateful (`g`) — take a fresh copy per scan rather than sharing one.
 */
export const TEXT_ASSIGN = () => /\.textContent\s*=\s*([^;]*)/g;

/**
 * Every un-localised display literal assigned to `.textContent` in `src`.
 *
 * @param {string} src
 * @returns {string[]}
 */
export function untranslatedTextContent(src) {
  const found = [];
  for (const m of src.matchAll(TEXT_ASSIGN())) {
    for (const run of literalRuns(m[1])) {
      if (isDisplayText(run.text) && !isTokenTest(run)) found.push(run.text);
    }
  }
  return found;
}
