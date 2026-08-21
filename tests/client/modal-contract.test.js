/**
 * tests/client/modal-contract.test.js — the modal-contract structural floor
 * (issue #1174, PRD #1168).
 *
 * A sibling of control-floors.test.js, and it exists for the same reason: not to
 * check the two modals that ship today, but to make the NEXT modal declare the
 * contract or fail the build. A confirmation dialog added six months from now
 * that sets `aria-modal` and forgets to trap focus does not crash — it just
 * quietly strands a keyboard operator inside a layer they cannot leave or
 * dismiss. This test is what turns that silent regression into a red bar.
 *
 * ── How "a modal" is decided ────────────────────────────────────────────────
 *
 * Not from a hand-kept list of files — a list is a thing to forget to add to.
 * From what the author already wrote: a surface that sets `aria-modal="true"`
 * or `role="dialog"` IS a modal by its own declaration, whether or not anyone
 * remembered to wire the trap. Every such file is then asked for the contract.
 *
 * ── What "declaring the contract" means ─────────────────────────────────────
 *
 * The contract is one shared helper, gui/focus-trap.js, exactly as roving
 * tabindex (issue #1170) is one shared helper. A modal declares the contract by
 * adopting it: importing `createFocusTrap`, building a trap over its overlay,
 * and driving it from open/close (`activate` / `release`). A modal that sets
 * aria-modal but wires none of that is the failure this floor catches.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import {
  REPO_ROOT, GUI, readStripped, consoleDocuments, componentFiles, rel,
} from './css-scan.js';

/** Every client JavaScript surface: the shell modules and the components. */
function jsFiles() {
  const top = fs.readdirSync(GUI)
    .filter((f) => f.endsWith('.js'))
    .map((f) => path.join(GUI, f));
  return [...top, ...componentFiles()].sort();
}

/**
 * Does this source DECLARE a modal — set `aria-modal="true"` or a dialog role,
 * in either the `setAttribute('aria-modal', 'true')` spelling the JS modals use
 * or an inline attribute? Comments are already stripped by `readStripped`, so a
 * doc-comment that merely names `role="dialog"` (focus-trap.js's own prose) does
 * not count.
 */
export function declaresModal(source) {
  return (
    /aria-modal["']\s*,\s*["']true["']/.test(source)
    || /role["']\s*,\s*["'](?:dialog|alertdialog)["']/.test(source)
    || /aria-modal\s*=\s*["']true["']/.test(source)
    || /role\s*=\s*["'](?:dialog|alertdialog)["']/.test(source)
  );
}

/**
 * Does this source ADOPT the shared focus-trap contract — import the helper,
 * build a trap, and drive both ends of it (open activates, close releases)?
 */
export function declaresModalContract(source) {
  const importsHelper = /from\s+["'][^"']*\/focus-trap\.js["']/.test(source);
  const buildsTrap = /createFocusTrap\s*\(/.test(source);
  const activates = /\.activate\s*\(/.test(source);
  const releases = /\.release\s*\(/.test(source);
  return importsHelper && buildsTrap && activates && releases;
}

const MODAL_FILES = jsFiles().filter((f) => declaresModal(readStripped(f)));

describe('every modal declares the focus contract', () => {
  it('finds the modals to check', () => {
    // A silent zero would let the whole floor pass while asserting nothing.
    // The two cogs' `aria-modal`/`role="dialog"` declaration and their
    // `createFocusTrap` wiring both moved onto the shared shell (issue
    // #1238's dedup of #939/#940) — `gui/settings-panel.js` and
    // `gui/server-settings.js` each call `mountOverlayShell` rather than
    // declaring the contract inline, so the file this floor actually finds
    // and checks is the shell they share.
    const names = MODAL_FILES.map(rel);
    expect(names).toContain('gui/settings-overlay-kit.js');
  });

  for (const file of MODAL_FILES) {
    it(`${rel(file)} traps, escapes and restores via the shared helper`, () => {
      expect(declaresModalContract(readStripped(file))).toBe(true);
    });
  }

  it('a new modal that forgets the contract fails this test', () => {
    // The floor's own teeth, made explicit: a surface that declares itself a
    // modal but wires no trap is caught. If this ever passes, the check above
    // has been hollowed out.
    const undeclared = `
      const overlay = doc.createElement('div');
      overlay.setAttribute('role', 'dialog');
      overlay.setAttribute('aria-modal', 'true');
      doc.body.appendChild(overlay);
    `;
    expect(declaresModal(undeclared)).toBe(true);
    expect(declaresModalContract(undeclared)).toBe(false);
  });

  it('a modal that adopts the helper passes', () => {
    const declared = `
      import { createFocusTrap } from './focus-trap.js';
      overlay.setAttribute('aria-modal', 'true');
      const trap = createFocusTrap(overlay, { doc, onEscape: () => close() });
      function open() { trap.activate(); }
      function close() { trap.release(); }
    `;
    expect(declaresModalContract(declared)).toBe(true);
  });
});

describe('the shared helper actually implements the contract', () => {
  const helper = readStripped(path.join(GUI, 'focus-trap.js'));

  it('traps Tab and Shift+Tab within the modal', () => {
    // The wrap is keyed on Tab and the shift modifier.
    expect(helper).toMatch(/['"]Tab['"]/);
    expect(helper).toMatch(/shiftKey/);
  });

  it('closes on Escape', () => {
    expect(helper).toMatch(/['"]Escape['"]/);
    expect(helper).toMatch(/onEscape/);
  });

  it('moves focus in on activate and restores it on release', () => {
    expect(helper).toMatch(/function activate/);
    expect(helper).toMatch(/function release/);
    // The opener is remembered on activate and handed focus back on release.
    expect(helper).toMatch(/opener\s*=\s*activeElementOf/);
    expect(helper).toMatch(/callFocus\(back\)/);
  });

  it('makes the background inert/aria-hidden to match the trap', () => {
    expect(helper).toMatch(/setAttribute\(['"]inert['"]/);
    expect(helper).toMatch(/setAttribute\(['"]aria-hidden['"],\s*['"]true['"]\)/);
  });
});

describe('no modal is authored inline in HTML, dodging the JS contract', () => {
  // The modals are built in JavaScript so the trap can be wired to them; an
  // inline `aria-modal`/`role="dialog"` in a page or console document would be
  // a modal no JS file owns, which this JS-scanning floor could not reach. Hold
  // the door: the markup surfaces carry no modal of their own.
  const htmlSurfaces = [
    path.join(REPO_ROOT, 'client.html'),
    path.join(REPO_ROOT, 'server.html'),
    ...consoleDocuments(),
  ];

  for (const file of htmlSurfaces) {
    it(`${rel(file)} authors no inline modal`, () => {
      expect(declaresModal(readStripped(file))).toBe(false);
    });
  }
});
