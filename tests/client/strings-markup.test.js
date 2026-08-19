import { describe, it, expect } from 'vitest';
import {
  TEXT_BEARING_ATTRS,
  blankInterpolations,
  isDisplayAttrValue,
  isHtmlTemplate,
  isMarkupText,
  lineOf,
  markupFindings,
  scanTemplates,
  stripInterpolations,
  untranslatedMarkup,
} from '../../scripts/strings-markup.mjs';

/** Just the reported strings, for terser assertions. */
const texts = (findings) => findings.map((f) => f.text);

/** Just the template bodies, for terser assertions. */
const bodies = (src) => scanTemplates(src).templates.map((l) => l.text);

/** A backtick, so a test's fixture can hold one without fighting the parser. */
const BT = '`';

describe('stripInterpolations', () => {
  it('removes an interpolation and keeps the literal either side', () => {
    expect(stripInterpolations('a ${x} b')).toBe('a  b');
  });

  it('counts braces so an object literal does not close it early', () => {
    expect(stripInterpolations("<i>${t('a.b', { n: 1 })}</i>")).toBe('<i></i>');
  });

  it('leaves markup with no interpolation alone', () => {
    expect(stripInterpolations('<span>Station</span>')).toBe('<span>Station</span>');
  });
});

describe('isMarkupText', () => {
  // Loose on purpose: a text node cannot hold a class name or a wire token, so
  // any word between `>` and `<` is display text. This is what the textContent
  // rule's isDisplayText could NOT do — it requires a space or all-caps, so
  // `Station` and `Core` were invisible to it.
  it('accepts a bare capitalised word', () => {
    expect(isMarkupText('Station')).toBe(true);
    expect(isMarkupText('Core')).toBe(true);
  });

  it('accepts a two-letter unit', () => {
    expect(isMarkupText('AU')).toBe(true);
  });

  it('rejects glyph-only runs, which read the same in every locale', () => {
    expect(isMarkupText('▲')).toBe(false);
    expect(isMarkupText('—')).toBe(false);
    expect(isMarkupText(' · ')).toBe(false);
    expect(isMarkupText('50%')).toBe(false);
    expect(isMarkupText('°')).toBe(false);
  });

  it('rejects a node that is nothing but a resolved lookup', () => {
    expect(isMarkupText("${t('component.blasters.title')}")).toBe(false);
  });

  it('rejects an entity, which is punctuation rather than a word', () => {
    // `&amp;` would otherwise report every ampersand in the client as the
    // untranslated English word "amp".
    expect(isMarkupText('&amp;')).toBe(false);
    expect(isMarkupText('&#8592;')).toBe(false);
  });

  it('still sees the English beside an entity', () => {
    expect(isMarkupText('&#8592; Back')).toBe(true);
  });
});

describe('isDisplayAttrValue', () => {
  // The false-positive surface this rule exists to manage: attributes carry
  // machine tokens as well as display text, and getting it wrong in either
  // direction fails. The line is capitalisation-and-spacing.
  it('accepts a capitalised word', () => {
    expect(isDisplayAttrValue('Core')).toBe(true);
  });

  it('accepts a phrase', () => {
    expect(isDisplayAttrValue('Station systems')).toBe(true);
    expect(isDisplayAttrValue('Launch with fully AI-controlled crew')).toBe(true);
    expect(isDisplayAttrValue('Your name')).toBe(true);
  });

  it('rejects the machine tokens the issue named as the risk', () => {
    expect(isDisplayAttrValue('close')).toBe(false);
    expect(isDisplayAttrValue('nav-back')).toBe(false);
    expect(isDisplayAttrValue('camera_fore')).toBe(false);
    expect(isDisplayAttrValue('console.repair.title')).toBe(false);
    expect(isDisplayAttrValue('assets/logo.png')).toBe(false);
  });

  it('rejects a value with no word in it', () => {
    expect(isDisplayAttrValue('100%')).toBe(false);
    expect(isDisplayAttrValue('')).toBe(false);
  });

  it('rejects a value that is entirely a resolved lookup', () => {
    expect(isDisplayAttrValue("${t('component.station_damage.bar_title')}")).toBe(false);
  });

  it('accepts a COMPOSED value whose literal half is lowercase', () => {
    // `${x} systems` is prose whatever case it is in — a composed value cannot
    // be a single machine token, so the token test is skipped for it.
    expect(isDisplayAttrValue('${name} systems')).toBe(true);
  });

  it('misses a lowercase single word — the deliberate blind spot', () => {
    // Pinned so the next author knows this corner is unenforced by design, and
    // does not "fix" it by widening the rule until every DOM hook is a warning.
    expect(isDisplayAttrValue('core')).toBe(false);
  });
});

describe('TEXT_BEARING_ATTRS', () => {
  it('is an explicit allowlist, not a denylist', () => {
    expect([...TEXT_BEARING_ATTRS].sort()).toEqual([
      'alt', 'aria-label', 'data-screen-label', 'label', 'placeholder', 'title',
    ]);
  });

  it('leaves `value` out — it is a wire token as often as it is display text', () => {
    expect(TEXT_BEARING_ATTRS.has('value')).toBe(false);
  });
});

describe('scanTemplates', () => {
  it('finds a template and reports where its body starts', () => {
    const src = 'tpl.innerHTML = `<b>Hi</b>`;';
    const [lit] = scanTemplates(src).templates;
    expect(lit.text).toBe('<b>Hi</b>');
    expect(src.slice(lit.start, lit.start + lit.text.length)).toBe(lit.text);
  });

  it('keeps `${…}` verbatim so offsets still line up with the source', () => {
    expect(bodies("`<b>${t('a.b')}</b>`")).toEqual(["<b>${t('a.b')}</b>"]);
  });

  it('finds a template nested inside an interpolation', () => {
    // blankInterpolations erases the span this sits in, so without the
    // recursion a row-rendering `.map()` would be entirely unscanned.
    const src = '`<ul>${rows.map((r) => `<li>Standing by</li>`).join(\'\')}</ul>`';
    expect(bodies(src)).toContain('<li>Standing by</li>');
  });

  it('ignores a backtick inside a line comment', () => {
    expect(bodies('// see `<b>Station</b>`\nconst x = 1;')).toEqual([]);
  });

  it('ignores a backtick inside a block comment', () => {
    expect(bodies('/* renders `<b>Station</b>` */')).toEqual([]);
  });

  it('ignores a backtick inside an ordinary string', () => {
    expect(bodies('const s = "a ` b";')).toEqual([]);
  });

  it('does not let an apostrophe in prose swallow the rest of the file', () => {
    // A stray quote is nearly always this scanner misreading an apostrophe.
    // It must stop at the line end, or a template further down goes unscanned
    // and the gate reports a clean bill of health it has not earned.
    const src = "const n = a / b; // the element's own\nconst t = `<b>Station</b>`;";
    expect(bodies(src)).toEqual(['<b>Station</b>']);
    expect(scanTemplates(src).problems).toEqual([]);
  });
});

describe('scanTemplates — regex literals', () => {
  // The scanner had no notion of a regex, so a `/…/` holding an odd quote or a
  // `//` was lexed as a string or a comment and the REST OF THE LINE vanished —
  // template literal included. Zero findings, gate green, hardcoded English on
  // screen: the exact failure this module exists to eliminate, shipped inside
  // the fix for it. Both shapes below returned `[]` before regex handling.
  it('sees past a regex containing a quote', () => {
    const src = "el.innerHTML = s.replace(/'/g, '') + " + BT + '<b>Station</b>' + BT + ';';
    expect(bodies(src)).toEqual(['<b>Station</b>']);
    expect(texts(untranslatedMarkup(src, false))).toEqual(['Station']);
  });

  it('sees past a regex containing a `//`', () => {
    const src = "const u = s.replace(/\\/\\//g, '/') + " + BT + '<b>Station</b>' + BT + ';';
    expect(bodies(src)).toEqual(['<b>Station</b>']);
    expect(texts(untranslatedMarkup(src, false))).toEqual(['Station']);
  });

  it('sees past a character class holding a slash and a quote', () => {
    const src = "const p = /[/']+/g; const t = " + BT + '<b>Station</b>' + BT + ';';
    expect(bodies(src)).toEqual(['<b>Station</b>']);
  });

  it('still reads a division as a division', () => {
    // The other error direction: lexing `a / b … / c` as a regex would swallow
    // everything between the two slashes, including a template.
    const src = 'const half = width / 2; const t = ' + BT + '<b>Station</b>' + BT + ';';
    expect(bodies(src)).toEqual(['<b>Station</b>']);
    expect(scanTemplates(src).problems).toEqual([]);
  });

  it('reads a regex after a keyword, not a division', () => {
    const src = "if (x) return /'/.test(s); const t = " + BT + '<b>Station</b>' + BT + ';';
    expect(bodies(src)).toEqual(['<b>Station</b>']);
  });
});

describe('scanTemplates — failing loud', () => {
  // A scanner that cannot lex a file must say so. Silence is indistinguishable
  // from a clean bill of health, and "it stopped looking" reporting the same
  // green as "it looked and found nothing" is what issue #976 was filed about.
  it('reports an unterminated string rather than returning quietly', () => {
    const src = 'const bad = "oops\nconst t = ' + BT + '<b>Station</b>' + BT + ';';
    const { problems } = scanTemplates(src);
    expect(problems).toHaveLength(1);
    expect(problems[0].reason).toMatch(/unterminated/);
    expect(lineOf(src, problems[0].index)).toBe(1);
  });

  it('reports an unterminated template literal', () => {
    const { problems } = scanTemplates('tpl.innerHTML = ' + BT + '<b>Station</b>;');
    expect(problems.map((p) => p.reason)).toContain('unterminated template literal');
  });

  it('reports an unterminated interpolation instead of consuming the file', () => {
    const src = 'tpl.innerHTML = ' + BT + '<b>${ x </b>' + BT + ';';
    expect(scanTemplates(src).problems.map((p) => p.reason))
      .toContain('unterminated ${…} interpolation');
  });

  it('surfaces the problem as a finding, so --strict cannot pass over it', () => {
    const src = 'const bad = "oops\nconst t = ' + BT + '<b>Station</b>' + BT + ';';
    const found = untranslatedMarkup(src, false);
    expect(found.map((f) => f.kind)).toContain('unscannable');
  });

  it('reports the same mis-lex once, not once per nesting level', () => {
    // The interpolation is lexed twice — by its enclosing template, and by the
    // recursion that looks for templates inside it — so the raw problem arrives
    // twice at the same offset.
    const src = 'const t = ' + BT + '${rows.map(() => "oops\n)}' + BT + ';';
    expect(untranslatedMarkup(src, false).filter((f) => f.kind === 'unscannable'))
      .toHaveLength(1);
  });
});

describe('blankInterpolations', () => {
  it('erases the JS inside `${…}` without moving a character', () => {
    const s = "<b>${t('a.b')}</b>";
    const blanked = blankInterpolations(s);
    expect(blanked).toBe('<b>${        }</b>');
    expect(blanked).toHaveLength(s.length);
  });

  it('keeps newlines, so a finding still lands on its real line', () => {
    const s = '<b>${a\nb}</b>';
    expect(blankInterpolations(s)).toBe('<b>${ \n }</b>');
  });

  it('keeps the `${` marker, which is how a composed attribute is recognised', () => {
    expect(isDisplayAttrValue(blankInterpolations('${name} systems'))).toBe(true);
  });
});

describe('isHtmlTemplate', () => {
  it('accepts markup', () => {
    expect(isHtmlTemplate('<div class="a">x</div>')).toBe(true);
    expect(isHtmlTemplate('<br/>')).toBe(true);
  });

  it('rejects a plain message template', () => {
    expect(isHtmlTemplate('range is ${a} < ${b}')).toBe(false);
  });
});

describe('markupFindings — text nodes', () => {
  it('reports a bare English text node', () => {
    expect(texts(markupFindings('<span class="bar-label">Station</span>')))
      .toEqual(['Station']);
  });

  it('says nothing about a node that is a resolved lookup', () => {
    expect(markupFindings("<span>${t('component.blasters.title')}</span>")).toEqual([]);
  });

  it('exempts an element carrying data-i18n — the literal is its fallback', () => {
    expect(markupFindings('<span data-i18n="console.repair.title">REPAIR</span>')).toEqual([]);
  });

  it('exempts a DESCENDANT of a data-i18n element too', () => {
    // applyToDom sets the tagged element's whole textContent, so everything
    // under it is replaced, not just its first text node.
    expect(markupFindings('<div data-i18n="a.b">Some <b>bold</b> text</div>')).toEqual([]);
  });

  it('does NOT exempt a sibling once the tagged element has closed', () => {
    expect(texts(markupFindings('<div><span data-i18n="a.b">Nav</span>Comms</div>')))
      .toEqual(['Comms']);
  });

  it('keeps the stack straight across a void element', () => {
    // <input> never closes; if it were pushed, the following </label> would pop
    // the wrong entry and the data-i18n exemption would drift down the page.
    expect(texts(markupFindings('<label><input type="text"/></label>Name')))
      .toEqual(['Name']);
  });

  it('skips a <style> body, which is wall-to-wall lowercase words', () => {
    expect(markupFindings('<style>.bar { display: inline-flex; }</style>')).toEqual([]);
  });

  it('skips a <script> body', () => {
    expect(markupFindings('<script>var greeting = "Standing by";</script>')).toEqual([]);
  });

  it('skips a comment', () => {
    expect(markupFindings('<!-- Redirect stale bookmarks. --><b>${x}</b>')).toEqual([]);
  });
});

describe('markupFindings — attributes', () => {
  it('reports an allowlisted attribute holding display text', () => {
    const found = markupFindings('<ph-station-damage label="Core"></ph-station-damage>');
    expect(found).toHaveLength(1);
    expect(found[0]).toMatchObject({ kind: 'attr', attr: 'label', text: 'Core' });
  });

  it('reports a title', () => {
    expect(texts(markupFindings('<button title="Station systems"></button>')))
      .toEqual(['Station systems']);
  });

  it('exempts an attribute that data-i18n-attr already resolves', () => {
    expect(markupFindings(
      '<button title="Retry connection now" data-i18n-attr="title:client.retry_now_tip"></button>',
    )).toEqual([]);
  });

  it('exempts one attribute of a pair without exempting the other', () => {
    expect(texts(markupFindings(
      '<a title="Toggle QR code" aria-label="Show Code" data-i18n-attr="aria-label:a.b"></a>',
    ))).toEqual(['Toggle QR code']);
  });

  it('says nothing about an attribute outside the allowlist', () => {
    expect(markupFindings('<div class="Station Card" data-mode="Fore"></div>')).toEqual([]);
  });

  it('reads an attribute quoted with single quotes', () => {
    expect(texts(markupFindings("<b title='Toggle fullscreen'></b>")))
      .toEqual(['Toggle fullscreen']);
  });

  it('is not confused by a `>` inside a quoted value', () => {
    // A naive `indexOf('>')` would end the tag mid-attribute, leaving `Max">`
    // as a text node and losing the tag's real attributes.
    expect(texts(markupFindings('<b title="Range > Max">Station</b>')))
      .toEqual(['Range > Max', 'Station']);
  });

  it('points at the attribute VALUE, not the start of the tag', () => {
    const html = '<x id="a" label="Core"></x>';
    const [found] = markupFindings(html);
    expect(html.slice(found.index, found.index + 4)).toBe('Core');
  });
});

describe('untranslatedMarkup — the three shapes issue #976 reported', () => {
  // Each of these is a failing case against the code as it shipped: the gate
  // scanned `.textContent = …` only, so all three rendered hardcoded English
  // with `check-strings --strict` green.

  it('sees English in a shadow-DOM template string', () => {
    const src = [
      "import { t } from '../strings.js';",
      'const tpl = document.createElement(\'template\');',
      'tpl.innerHTML = `',
      '  <style>.bar-label { font-size: 0.62rem; }</style>',
      '  <span class="bar-label" id="bar-label">Station</span>',
      '  <div class="popup-title">Station Systems</div>',
      '`;',
    ].join('\n');
    expect(texts(untranslatedMarkup(src, false))).toEqual(['Station', 'Station Systems']);
  });

  it('sees English in an allowlisted attribute of an .html file', () => {
    const src = '<div>\n  <ph-station-damage id="core-damage" label="Core"></ph-station-damage>\n</div>';
    const found = untranslatedMarkup(src, true);
    expect(found).toHaveLength(1);
    expect(found[0]).toMatchObject({ kind: 'attr', attr: 'label', text: 'Core' });
    expect(lineOf(src, found[0].index)).toBe(2);
  });

  it('sees English in a template string inside an .html file\'s inline <script>', () => {
    const src = [
      '<body>',
      '  <script type="module">',
      "    grid.innerHTML = `<div class=\"empty\">No ships available</div>`;",
      '  </script>',
      '</body>',
    ].join('\n');
    expect(texts(untranslatedMarkup(src, true))).toEqual(['No ships available']);
  });

  it('none of the three were visible to the textContent rule', () => {
    // Regression guard for the widening itself. If someone narrows the scan
    // back to `.textContent`, this is the test that says why they must not.
    const TEXT_ASSIGN = /\.textContent\s*=/g;
    const src = 'tpl.innerHTML = `<span label="Core">Station</span>`;';
    expect(src.match(TEXT_ASSIGN)).toBeNull();
    expect(texts(untranslatedMarkup(src, false)).sort()).toEqual(['Core', 'Station']);
  });

  it('stays quiet on a fully localised component template', () => {
    const src = [
      'tpl.innerHTML = `',
      '  <style>.bar { display: inline-flex; }</style>',
      "  <button class=\"bar\" title=\"${t('component.station_damage.bar_title', { name: label })}\">",
      '    <span class="bar-label">${label}</span>',
      '    <span class="pct">—</span>',
      '    <span class="caret">▲</span>',
      '  </button>',
      '`;',
    ].join('\n');
    expect(untranslatedMarkup(src, false)).toEqual([]);
  });
});

describe('untranslatedMarkup — nested templates', () => {
  // `readTemplate` keeps each `${…}` verbatim so offsets survive, which left
  // live JS in what markupFindings then read as markup: the nested `<li>`
  // parsed as a real tag, and the code between its closing backtick and the
  // outer `}` parsed as a TEXT NODE. Under --strict that is CI red on fully
  // localised code, pointing at no display text — and any real English inside
  // the nested template was reported twice on top.

  it('says nothing about a fully localised `.map()` row builder', () => {
    const src = "grid.innerHTML = `<ul>${rows.map((r) => `<li>${r.name}</li>`).join('')}</ul>`;";
    expect(untranslatedMarkup(src, false)).toEqual([]);
  });

  it('does not read `).join(\'\')}` as untranslated display text', () => {
    const src = "grid.innerHTML = `<ul>${rows.map((r) => `<li>${r.name}</li>`).join('')}</ul>`;";
    expect(texts(untranslatedMarkup(src, false))).not.toContain("`).join('')}");
  });

  it('reports English inside a nested template exactly once', () => {
    const src = "grid.innerHTML = `<ul>${rows.map(() => `<li>Standing by</li>`).join('')}</ul>`;";
    expect(texts(untranslatedMarkup(src, false))).toEqual(['Standing by']);
  });

  it('points a nested finding at its own position, not the outer template', () => {
    const src = [
      'grid.innerHTML = `<ul>',
      "  ${rows.map(() => `<li>Standing by</li>`).join('')}",
      '</ul>`;',
    ].join('\n');
    const [found] = untranslatedMarkup(src, false);
    expect(lineOf(src, found.index)).toBe(2);
  });

  it('is the shape the module docstring advertises as supported', () => {
    // Documented at `scanTemplates`. The old tests only exercised it against
    // the template extractor, which was never where it broke.
    const src = "el.innerHTML = `<ul>${xs.map((x) => `<li>${x}</li>`).join('')}</ul>`;";
    expect(bodies(src)).toContain('<li>${x}</li>');
    expect(untranslatedMarkup(src, false)).toEqual([]);
  });

  it('does not let an interpolated regex swallow the following template', () => {
    // F3's shape: `skipInterpolation` has no line guard, so a quote inside a
    // regex ran `depth` past the real `}` and consumed several lines, emitting
    // raw JS source as a "hardcoded markup text" warning.
    const src = 'const h = ' + BT + '<b>${x.replace(/\'/g, \'\')}</b>' + BT + ';\n'
      + 'const g = ' + BT + '<i>Standing by</i>' + BT + ';';
    expect(texts(untranslatedMarkup(src, false))).toEqual(['Standing by']);
    expect(lineOf(src, untranslatedMarkup(src, false)[0].index)).toBe(2);
  });
});

describe('untranslatedMarkup — data-i18n in a JS-built template', () => {
  // `applyToDom` is only ever called on `document`, at boot (client.html,
  // server.html, gui/console-core.js). Nothing calls it on a shadowRoot or
  // re-runs it after markup is built, so `data-i18n` in a component template
  // resolves NOTHING — while still buying the subtree an exemption from this
  // scan. That is hardcoded English shipping green, so it is reported.
  it('reports the tag instead of falling silent over its subtree', () => {
    const src = 'tpl.innerHTML = `<span data-i18n="console.repair.title">REPAIR</span>`;';
    const found = untranslatedMarkup(src, false);
    expect(found).toHaveLength(1);
    expect(found[0]).toMatchObject({ kind: 'inert-i18n', attr: 'data-i18n' });
  });

  it('reports an inert data-i18n-attr too', () => {
    const src = 'tpl.innerHTML = `<b title="Close" data-i18n-attr="title:a.b"></b>`;';
    expect(untranslatedMarkup(src, false).map((f) => f.kind)).toContain('inert-i18n');
  });

  it('leaves the exemption alone in an .html file, where applyToDom does run', () => {
    expect(untranslatedMarkup('<span data-i18n="console.repair.title">REPAIR</span>', true))
      .toEqual([]);
  });
});

describe('lineOf', () => {
  it('is 1-based and counts newlines before the index', () => {
    const src = 'a\nb\nc';
    expect(lineOf(src, 0)).toBe(1);
    expect(lineOf(src, 2)).toBe(2);
    expect(lineOf(src, 4)).toBe(3);
  });
});
