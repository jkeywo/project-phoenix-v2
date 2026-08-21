/**
 * tests/client/interaction-scan.js — reading the client's shadow-DOM
 * components as text to find the interactive surface (PRD #1168, issue #1175).
 *
 * Not a test file (no `.test.js`, so vitest does not collect it). It is the
 * shared scanner behind interaction-floors.test.js, the sibling of css-scan.js:
 * where css-scan answers "what is a style declaration", this answers "what is
 * an interactive control, and does it meet the #1170 keyboard conventions".
 *
 * ── Why source, not a mounted DOM ──────────────────────────────────────────
 *
 * The house pattern (control-floors.test.js) is a STRUCTURAL test over source
 * with a documented allow-list, deliberately not an external audit tool. This
 * scanner keeps to it. It cannot compute a live accessible name the way a
 * browser would — a `${t(...)}` is a string id, not resolved text — so it does
 * not try to. It classifies from what the author wrote: a `<button>` is
 * focusable whether or not anyone said so; a control that gets text (its own,
 * a child's, or a runtime fill by id) is named; a glyph with no `aria-label`
 * is not; a `pointerdown` on a bare `<div>` with no `tabindex` is a control the
 * keyboard cannot reach. That is the same discrimination interactiveRules() in
 * control-floors makes for touch, one layer up.
 *
 * The cost of a source scan is a coarse edge: a component that mixes a real
 * <button> with a delegated `e.target.closest('.pip')` click on a <div> reads
 * as "has a focusable control" and its div-pips escape this floor. The family
 * sweeps (#1176–#1178) carry the fine-grained per-control conversion with
 * mounted jsdom and keyboard smokes; this floor is the structural minimum that
 * stops the egregious regression — a surface with NOTHING focusable, a glyph
 * with no name, a composite with no role — and pins the debt in an allow-list.
 *
 * The scanning primitives (comment stripping, the console-surface enumerators)
 * are shared with css-scan.js rather than re-derived here.
 */
import { readStripped, componentFiles, consoleDocuments, rel } from './css-scan.js';

const NATIVE_CONTROL = /^(button|select|textarea|input|a)$/;

/** Does this text carry an accessible name — literal text or a string id? */
function textNames(text) {
  const bare = text.replace(/<[^>]*>/g, ' ');   // drop tags: attribute words are not the name
  return /t\(/.test(bare)            // t('id') — a catalogue string
    || /\$\{/.test(bare)             // ${expr} — interpolated text
    || /[A-Za-z]/.test(bare);        // literal words (FIRE, Intel, Back)
}

/** Is the only content a glyph/symbol — the case that NEEDS an aria-label? */
function isGlyphOnly(text) {
  const bare = text.replace(/<[^>]*>/g, ' ').replace(/['"`;]/g, ' ').trim();
  if (bare === '') return false;             // empty is "unnamed", handled separately
  return !/t\(|\$\{|[A-Za-z]/.test(bare);    // has content, none of it text
}

/**
 * The ids JS wires a name onto at runtime: `getElementById('x')` /
 * `querySelector('#x')`. A markup control carrying such an id resolves its
 * name (text or aria-label) from script after mount, not from static markup —
 * the hidden `<button id="action">` the operations and scan consoles fill.
 */
export function runtimeWiredIds(source) {
  const ids = new Set();
  const re = /(?:getElementById\(\s*['"]([\w-]+)['"]\)|querySelector\(\s*['"]#([\w-]+)['"]\))/g;
  let m;
  while ((m = re.exec(source)) !== null) ids.add(m[1] || m[2]);
  return ids;
}

/**
 * Controls built imperatively: `x = document.createElement('button')`.
 *
 * A control is NAMED when, anywhere in the source, its variable receives text
 * (`x.textContent =`, `x.innerHTML = '…FIRE…'`), an `aria-label`, or fills a
 * descendant it owns (`x.querySelector('.name').textContent`, `x.children[0]`,
 * `x.append(...)`) — a button's name is its descendant text. It is GLYPH-ONLY
 * when its only text is a symbol (`−`) and it carries no aria-label. It is
 * removed from the tab order when it sets `tabindex="-1"`.
 */
export function createdControls(source) {
  const out = [];
  const re = /(?:(?:const|let|var)\s+)?([A-Za-z_$][\w$]*)\s*=\s*document\.createElement\(\s*['"]([a-z]+)['"]\s*\)/g;
  const seen = new Set();
  let m;
  while ((m = re.exec(source)) !== null) {
    const [, name, tag] = m;
    if (!NATIVE_CONTROL.test(tag)) continue;
    if (seen.has(name)) continue;   // the same var reassigned in another branch is one control
    seen.add(name);
    const v = name.replace(/[$]/g, '\\$');
    const aria = new RegExp(`\\b${v}\\.(?:setAttribute\\(\\s*['"]aria-label(?:ledby)?['"]|ariaLabel\\s*=)`).test(source);
    let text = '';
    const textRe = new RegExp(`\\b${v}\\.(?:innerHTML|textContent|innerText)\\s*=\\s*([^\\n;]*)`, 'g');
    let tm;
    while ((tm = textRe.exec(source)) !== null) text += ` ${tm[1]}`;
    const ariaInHtml = /aria-label/.test(text);
    const descendant = new RegExp(`\\b${v}\\.(?:children\\[|querySelector|firstChild|lastChild|firstElementChild|lastElementChild|append\\(|appendChild\\()`).test(source);
    const removed = new RegExp(`\\b${v}\\.(?:setAttribute\\(\\s*['"]tabindex['"]\\s*,\\s*['"]?-1|tabIndex\\s*=\\s*-1)`).test(source);
    const named = aria || ariaInHtml || descendant || textNames(text);
    out.push({
      origin: 'created', tag, focusable: true, removedFromTabOrder: removed,
      named, glyphOnly: !named && isGlyphOnly(text),
    });
  }
  return out;
}

/** Controls written as markup in a template literal or an HTML document. */
export function markupControls(source, runtimeIds) {
  const out = [];
  const paired = /<(button|select|textarea|a)\b([^>]*)>([\s\S]*?)<\/\1>/gi;
  let m;
  while ((m = paired.exec(source)) !== null) {
    const [, rawTag, attrs, inner] = m;
    const tag = rawTag.toLowerCase();
    if (tag === 'a' && !/href/i.test(attrs)) continue;   // an <a> with no href is not a control
    const id = (attrs.match(/\bid\s*=\s*['"]([\w-]+)['"]/) || [])[1];
    const runtimeNamed = !!(id && runtimeIds.has(id));
    const aria = /aria-label/i.test(attrs);
    const named = aria || runtimeNamed || textNames(inner);
    out.push({
      origin: 'markup', tag,
      focusable: true, removedFromTabOrder: /tabindex\s*=\s*['"]?-1/i.test(attrs),
      named, glyphOnly: !named && !runtimeNamed && isGlyphOnly(inner),
    });
  }
  const voidRe = /<input\b([^>]*)>/gi;
  while ((m = voidRe.exec(source)) !== null) {
    const attrs = m[1];
    if (/type\s*=\s*['"]?hidden/i.test(attrs)) continue;
    const id = (attrs.match(/\bid\s*=\s*['"]([\w-]+)['"]/) || [])[1];
    out.push({
      origin: 'markup', tag: 'input', focusable: true,
      removedFromTabOrder: /tabindex\s*=\s*['"]?-1/i.test(attrs),
      // A bare input carries no text, so its name must be an aria-label or a
      // runtime fill; there is nothing to be glyph-only.
      named: /aria-label|aria-labelledby/i.test(attrs) || !!(id && runtimeIds.has(id)),
      glyphOnly: false,
    });
  }
  return out;
}

/** How the component treats its own host element and whether it adopts focus. */
export function hostFacts(source) {
  return {
    definesElement: /customElements\.define/.test(source),
    adoptsFocusFamily: /phAdoptConsoleStyles\s*\(/.test(source),
    extendsComponent: (source.match(/class\s+\w+\s+extends\s+([A-Za-z_$][\w$]*)/) || [])[1] || null,
    hostRole: /this\.setAttribute\(\s*['"]role['"]/.test(source),
    hostNamed: /this\.setAttribute\(\s*['"]aria-label(?:ledby)?['"]/.test(source),
    hostTabindex: /this\.(?:setAttribute\(\s*['"]tabindex['"]|tabIndex\s*=)/.test(source),
    roving: /installRovingTabindex|rovingKeyTarget|syncRovingTabindex/.test(source),
    // A drag/tap widget with no native control: it makes a bare element
    // interactive, so it needs a role and a tab stop of its own.
    pointerWidget: /addEventListener\(\s*['"](?:pointerdown|mousedown|touchstart)['"]/.test(source),
    clickWidget: /addEventListener\(\s*['"](?:click|pointerup)['"]/.test(source),
  };
}

/**
 * Evaluate one surface (a component's source, or a console document) against
 * the four interaction floors. Returns the structural facts plus, for each
 * floor, the list of violations found — an empty list is a pass.
 *
 * @param {string} source  raw file text (comments are stripped here)
 * @param {{isDocument?: boolean}} [opts]
 */
export function evaluateSurface(source, { isDocument = false } = {}) {
  const src = stripFor(source);
  const runtimeIds = runtimeWiredIds(src);
  const controls = isDocument
    ? markupControls(src, runtimeIds)                          // a document has no createElement controls of its own
    : [...createdControls(src), ...markupControls(src, runtimeIds)];
  const host = hostFacts(src);

  // A host is a COMPOSITE — a thing that manages focus and therefore owes a
  // role and a name — when it roves its children, takes a tabindex, or already
  // claims a role. Documents are plain pages, never composites.
  const compositeHost = !isDocument && (host.roving || host.hostRole || host.hostTabindex);
  // A custom WIDGET: interaction wired onto bare elements, with no native
  // control and no host focus target — the joystick well, the shield arcs.
  const customWidget = !isDocument && (host.pointerWidget || host.clickWidget);

  const focusTargets = controls.filter((c) => c.focusable && !c.removedFromTabOrder).length
    + (host.hostTabindex ? 1 : 0);
  const hasSurface = controls.length > 0 || compositeHost || customWidget;

  const focusability = [];
  const reachability = [];
  const naming = [];

  if (hasSurface) {
    for (const c of controls) {
      if (c.removedFromTabOrder && !host.roving) {
        reachability.push(`a <${c.tag}> sets tabindex="-1" with no roving to bring it back into reach`);
      }
      if (!c.named) {
        naming.push(c.glyphOnly
          ? `a glyph-only <${c.tag}> carries no aria-label`
          : `a <${c.tag}> exposes no accessible name`);
      }
    }
    if (focusTargets === 0) {
      // An interactive surface a keyboard can never land on.
      focusability.push('the surface is interactive but exposes nothing focusable');
      reachability.push('the surface has no keyboard-reachable focus target');
    }
    if (compositeHost) {
      if (!host.hostRole) naming.push('a composite host declares no role');
      if (!host.hostNamed) naming.push('a composite host declares no accessible name');
    } else if (customWidget && focusTargets === 0) {
      naming.push('a custom interactive widget declares no role or accessible name');
    }
  }

  return {
    hasSurface, controls, host, compositeHost, customWidget, focusTargets,
    floors: { focusability, reachability, naming },
    conformant: focusability.length + reachability.length + naming.length === 0,
  };
}

/**
 * Whether a component adopts the shared control family — directly, or by
 * extending another `ph-*` component that does. Adoption is what puts the
 * #1170 focus ring on the component's controls, so a component that hand-rolls
 * its chrome instead is the regression this floor exists to catch.
 */
export function focusFamilyAdoption(source) {
  const src = stripFor(source);
  const host = hostFacts(src);
  return {
    definesElement: host.definesElement,
    adopts: host.adoptsFocusFamily,
    extendsComponent: host.extendsComponent,
    // Anything a ph-* component extends is another ph-* component (they all
    // extend HTMLElement or a sibling); extending a sibling inherits its adopt.
    ok: host.adoptsFocusFamily || (!!host.extendsComponent && host.extendsComponent !== 'HTMLElement'),
  };
}

// stripComments is not exported by css-scan; readStripped reads+strips a file,
// but the scanner also runs against inline fixture STRINGS in its own tests. So
// keep a local stripper that matches css-scan's, applied wherever a raw string
// enters. (readStripped stays the path for real files, so fixtures and files
// go through byte-identical stripping.)
function stripFor(source) {
  const blank = (m) => m.replace(/[^\n]/g, ' ');
  return source
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/^[ \t]*\/\/.*$/gm, blank)
    .replace(/[ \t]\/\/[^\n'"`]*$/gm, blank);
}

export { readStripped, componentFiles, consoleDocuments, rel };
