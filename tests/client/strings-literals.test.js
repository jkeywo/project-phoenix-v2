import { describe, it, expect } from 'vitest';
import {
  isDisplayText,
  isTokenTest,
  literalRuns,
  untranslatedTextContent,
} from '../../scripts/strings-literals.mjs';

describe('literalRuns', () => {
  it('reads a single-quoted run', () => {
    expect(literalRuns("'WEAPONS HOT'").map((r) => r.text)).toEqual(['WEAPONS HOT']);
  });

  it('reads both arms of a ternary', () => {
    expect(literalRuns("s.red_alert ? 'WEAPONS HOT' : 'CLEAR'").map((r) => r.text))
      .toEqual(['WEAPONS HOT', 'CLEAR']);
  });

  it('reads a double-quoted run', () => {
    expect(literalRuns('"Unknown Scenario"').map((r) => r.text)).toEqual(['Unknown Scenario']);
  });

  it('does not end a run on an escaped quote', () => {
    expect(literalRuns("'don\\'t fire'").map((r) => r.text)).toEqual(["don't fire"]);
  });

  it('splits a template on its interpolations', () => {
    expect(literalRuns('`${sp} (waiting)`').map((r) => r.text)).toEqual(['', ' (waiting)']);
  });

  it('skips a nested ternary inside an interpolation rather than reading it as prose', () => {
    // The pluralising shape: the `''`/`'s'` are code, and the English either
    // side of them has to survive as two separate runs.
    const expr = '`↻ ${n} station slot${n === 1 ? \'\' : \'s\'} reserved (max ${MAX})`';
    expect(literalRuns(expr).map((r) => r.text))
      .toEqual(['↻ ', ' station slot', ' reserved (max ', ')']);
  });

  it('counts braces so an object literal in an interpolation does not close it early', () => {
    expect(literalRuns('`a ${t(id, { n: 1 })} b`').map((r) => r.text)).toEqual(['a ', ' b']);
  });

  it('carries the surrounding expression on every run', () => {
    const [run] = literalRuns("x === 'GRID OFFLINE' ? y : z");
    expect(run.before).toBe('x === ');
    expect(run.after).toBe(' ? y : z');
  });
});

describe('isDisplayText', () => {
  it('accepts prose with a space', () => {
    expect(isDisplayText('Unknown Scenario')).toBe(true);
  });

  it('accepts an all-caps caption with no space', () => {
    expect(isDisplayText('CLEAR')).toBe(true);
  });

  it('rejects a css class name', () => {
    expect(isDisplayText('station-card')).toBe(false);
    expect(isDisplayText('complexity-pill')).toBe(false);
  });

  it('cannot tell a multi-word class LIST from prose — which is why only textContent is scanned', () => {
    // Honest limit of the heuristic, pinned so nobody widens the rule past
    // `.textContent` (to `.className`, say) expecting it to hold up.
    expect(isDisplayText('station-card empty per-slot')).toBe(true);
  });

  it('rejects an element id and a machine token', () => {
    expect(isDisplayText('lobby-title')).toBe(false);
    expect(isDisplayText('camera_fore')).toBe(false);
  });

  it('rejects a string id', () => {
    expect(isDisplayText('console.common.no_target')).toBe(false);
  });

  it('rejects a css value and a url', () => {
    expect(isDisplayText('var(--ink-faint)')).toBe(false);
    expect(isDisplayText('https://example.test/a')).toBe(false);
  });

  it('rejects punctuation and glyphs with no word in them', () => {
    expect(isDisplayText(' · ')).toBe(false);
    expect(isDisplayText('↻ ')).toBe(false);
  });
});

describe('isTokenTest', () => {
  it('flags a literal on the right of an equality', () => {
    expect(isTokenTest({ before: 'x === ', after: ' ? a : b' })).toBe(true);
  });

  it('flags a literal on the left of an equality through closing parens', () => {
    expect(isTokenTest({ before: '(s.grid || ', after: ') === "GRID OFFLINE"' })).toBe(true);
  });

  it('leaves a `||` fallback alone — that one IS displayed', () => {
    expect(isTokenTest({ before: 's.scenario_title || ', after: '' })).toBe(false);
  });
});

describe('untranslatedTextContent', () => {
  it('finds the plain assignment the anchored rule already caught', () => {
    expect(untranslatedTextContent("el.textContent = 'Standing by';"))
      .toEqual(['Standing by']);
  });

  // ── The shapes the old anchored rule was blind to (it required a quote
  // IMMEDIATELY after the `=`, so none of these ever reached a warning). ──

  it('finds both arms of a ternary', () => {
    expect(untranslatedTextContent("v.textContent = s.red_alert ? 'WEAPONS HOT' : 'CLEAR';"))
      .toEqual(['WEAPONS HOT', 'CLEAR']);
  });

  it('finds a `||` fallback', () => {
    expect(untranslatedTextContent("el.textContent = s.scenario_title || 'Unknown Scenario';"))
      .toEqual(['Unknown Scenario']);
  });

  it('finds the English inside a template literal', () => {
    expect(untranslatedTextContent('p.textContent = `${sp} (waiting)`;'))
      .toEqual([' (waiting)']);
  });

  it('these three really were invisible to the anchored rule', () => {
    // Regression guard for the widening itself: if someone re-anchors the
    // pattern, this is the test that says why they must not.
    const ANCHORED = /\.textContent\s*=\s*(['"])([^'"]*[A-Za-z]{3}[^'"]*)\1/g;
    const src = [
      "v.textContent = s.red_alert ? 'WEAPONS HOT' : 'CLEAR';",
      "el.textContent = s.scenario_title || 'Unknown Scenario';",
      'p.textContent = `${sp} (waiting)`;',
    ].join('\n');
    expect([...src.matchAll(ANCHORED)]).toHaveLength(0);
    expect(untranslatedTextContent(src)).toHaveLength(4);
  });

  it('reads a right-hand side wrapped onto the next line', () => {
    expect(untranslatedTextContent("el.textContent =\n  'Waiting for the crew';"))
      .toEqual(['Waiting for the crew']);
  });

  it('stops at the statement terminator so the next statement cannot leak in', () => {
    expect(untranslatedTextContent("el.textContent = t('a.b'); el.className = 'crew dot';"))
      .toEqual([]);
  });

  it('ignores a machine token being compared rather than shown', () => {
    // gui/battleship/shields.html: both literals are wire tokens; the displayed
    // value is the t() call.
    const src = "g.textContent = (s.grid_status || 'GRID NOMINAL') === 'GRID OFFLINE'"
      + " ? t('component.shield_panel.grid_offline') : t('component.shield_panel.grid_nominal');";
    expect(untranslatedTextContent(src)).toEqual([]);
  });

  it('ignores a localised assignment', () => {
    expect(untranslatedTextContent("el.textContent = t('server.reserved');")).toEqual([]);
  });

  it('ignores an assignment that is not a literal at all', () => {
    expect(untranslatedTextContent('el.textContent = crewN + "/" + maxP;')).toEqual([]);
  });

  // ── Known limitation, tracked on #976: `[^;]*` stops at the FIRST `;` in
  // the right-hand side, which is only the statement terminator when the
  // right-hand side contains none. A nested `;` truncates the scan early.
  // Pinned here so nobody over-trusts the rule, and so a future
  // depth-tracking scanner has a red test to turn green. ──

  it('is blind past a `;` nested inside the right-hand side (gui/battleship/sensors.html:104 shape)', () => {
    // The live case: a callback body's `return …;` ends the capture before
    // the ternary that follows it, so neither arm is seen. Benign here
    // because both arms are already t() calls — there is no English to miss
    // — but the scan cannot tell that; it simply never looks.
    const src = "document.getElementById('shields-tag').textContent = "
      + "shields.some(function(f) { return f.online === false; }) "
      + "? t('console.shield.degraded') : t('console.shield.online');";
    expect(untranslatedTextContent(src)).toEqual([]);
  });

  it('is blind to a `;` inside the string literal itself, and fails silently rather than erroring', () => {
    // Worse than the callback case: the capture stops mid-quote, and the
    // unterminated run doesn't even look like display text to
    // isDisplayText, so this reports zero literals instead of one.
    expect(untranslatedTextContent("el.textContent = 'Warning; check the grid';"))
      .toEqual([]);
  });
});
