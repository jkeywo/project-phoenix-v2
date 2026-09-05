// @vitest-environment jsdom
//
// gui/native-settings.js — the native host's settings cog (issue #1367).
//
// Two claims, and the PRD states both in one line: settings are not rebuilt,
// and the native surface gains the existing one. So the tests below assert
// what a reader would otherwise have to take on trust —
//
//   * the shell, the tab strip and the section primitives come from
//     `gui/settings-overlay-kit.js` and the tab LIST from
//     `gui/settings-tabs.js`, which is checked by driving the real module
//     against a real document and reading back the classes and ids the kit
//     produces, not by asserting an import exists;
//
//   * a tab this surface can put nothing on is not offered, which is the
//     "a control exists exactly when something behind it answers it" rule
//     applied one level up — a blank Audio page would be the settings menu
//     claiming a setting this host does not have;
//
//   * every action leaves through the injected hook, so the module can be
//     mounted on a surface that answers its verbs some other way.

import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  mountNativeSettings,
  nativeSettingsView,
  NATIVE_SETTINGS_BUTTON_ID,
  NATIVE_SETTINGS_OVERLAY_ID,
  NATIVE_SETTINGS_CONTROLS,
} from '../../gui/native-settings.js';
import { TABS, visibleTabs } from '../../gui/settings-tabs.js';

import { readStripped, colourLiterals, fontSizeLiterals, GUI } from './css-scan.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CSS = fs.readFileSync(path.join(HERE, '../../gui/native-settings.css'), 'utf-8');
/** The same sheet with its comments removed, as the enforcing suite reads it. */
const CSS_STRIPPED = readStripped(path.join(GUI, 'native-settings.css'));
const LINK_JS = fs.readFileSync(
  path.join(HERE, '../../src/native_host/host_lobby/host_lobby_link.js'),
  'utf-8',
);

/** The string resolver the surface injects; here it just echoes the id. */
const t = (id) => id;

function freshDoc() {
  return document.implementation.createHTMLDocument('');
}

// ── The pure decision ──────────────────────────────────────────────────────

describe('nativeSettingsView', () => {
  it('offers only the shared tabs this surface can put a control on', () => {
    const vm = nativeSettingsView({});
    expect(vm.tabs.map((tab) => tab.id)).toEqual(['gameplay']);
    // …and that is a SUBSET of the shared list rather than a list of its own:
    // a tab this surface offers must be one the fleet already has.
    const shared = visibleTabs(false).map((tab) => tab.id);
    for (const tab of vm.tabs) expect(shared).toContain(tab.id);
    // The label is the shared table's, not one written here.
    const gameplay = TABS.find((tab) => tab.id === 'gameplay');
    expect(vm.tabs[0].labelId).toBe(gameplay.labelId);
  });

  it('drops a tab whose only control was gated away by the demo build', () => {
    // Debug is the one gated tab, so a table that only put a control there
    // must lose the tab entirely in a demo build rather than show it empty.
    const debugOnly = [{ id: 'x', tab: 'debug', sectionId: 's', labelId: 'l', action: 'a' }];
    expect(nativeSettingsView({ controls: debugOnly, demo: false }).tabs.map((v) => v.id))
      .toEqual(['debug']);
    expect(nativeSettingsView({ controls: debugOnly, demo: true }).tabs).toEqual([]);
    expect(nativeSettingsView({ controls: debugOnly, demo: true }).activeTab).toBeNull();
  });

  it('falls back to the first offered tab when the remembered one is gone', () => {
    // The caller's memory can outlive the table — a build flag flips, or a
    // slice removes the last control from a tab. A blank body would read as
    // "the settings menu is broken", which is exactly what `resolveActiveTab`
    // exists to prevent on the two surfaces that already had one.
    expect(nativeSettingsView({ activeTab: 'audio' }).activeTab).toBe('gameplay');
    expect(nativeSettingsView({ activeTab: 'gameplay' }).activeTab).toBe('gameplay');
  });

  it('groups the active tab’s controls into sections in table order', () => {
    const vm = nativeSettingsView({});
    expect(vm.sections.map((s) => s.headingId))
      .toEqual(['settings.qr_code', 'settings.display']);
    expect(vm.sections[1].hintId).toBe('settings.display_hint');
    expect(vm.sections[0].hintId).toBeNull();
    expect(vm.sections.flatMap((s) => s.controls.map((c) => c.action)))
      .toEqual(['toggle_qr', 'toggle_fullscreen']);
  });

  it('shows nothing at all rather than a section from another tab', () => {
    const other = [{ id: 'x', tab: 'audio', sectionId: 's', labelId: 'l', action: 'a' }];
    const vm = nativeSettingsView({ controls: other, activeTab: 'gameplay' });
    expect(vm.activeTab).toBe('audio');
    expect(vm.sections).toHaveLength(1);
  });
});

// ── The mount ──────────────────────────────────────────────────────────────

describe('mountNativeSettings', () => {
  it('find-or-creates the kit’s cog and modal, closed and named', () => {
    const doc = freshDoc();
    const panel = mountNativeSettings(doc, {}, { t });

    const btn = doc.getElementById(NATIVE_SETTINGS_BUTTON_ID);
    const overlay = doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID);
    expect(btn).toBeTruthy();
    expect(overlay).toBeTruthy();
    // The kit's own contract, not this module's: a modal dialog, hidden until
    // it is opened, with the cog reporting its state.
    expect(overlay.getAttribute('role')).toBe('dialog');
    expect(overlay.getAttribute('aria-modal')).toBe('true');
    expect(overlay.hidden).toBe(true);
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(panel.isOpen()).toBe(false);
    // The cog wears the landing's icon-button chrome plus its own placement
    // hook, so there is one chrome vocabulary on this surface rather than two.
    expect(btn.className.split(/\s+/)).toContain('landing-icon-btn');
    expect(btn.className.split(/\s+/)).toContain('native-settings-btn');
  });

  it('opens on a click and builds the tab strip and the offered controls', () => {
    const doc = freshDoc();
    mountNativeSettings(doc, {}, { t });
    doc.getElementById(NATIVE_SETTINGS_BUTTON_ID).click();

    const overlay = doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID);
    expect(overlay.hidden).toBe(false);
    expect([...overlay.querySelectorAll('[data-tab]')].map((el) => el.getAttribute('data-tab')))
      .toEqual(['gameplay']);
    expect([...overlay.querySelectorAll('[data-control]')].map((el) => el.getAttribute('data-control')))
      .toEqual(NATIVE_SETTINGS_CONTROLS.map((row) => row.id));
    // The sections are the kit's primitives, so the two other cogs' CSS shape
    // and this one's are the same shape under different class names.
    expect(overlay.querySelectorAll('.native-settings-section')).toHaveLength(2);
    expect(overlay.querySelectorAll('.native-settings-heading')).toHaveLength(2);
    expect(overlay.querySelectorAll('.native-settings-hint')).toHaveLength(1);
  });

  it('sends the pressed row’s verb through the hook and nowhere else', () => {
    const doc = freshDoc();
    const ran = [];
    mountNativeSettings(doc, { run: (action) => ran.push(action) }, { t });
    doc.getElementById(NATIVE_SETTINGS_BUTTON_ID).click();

    const overlay = doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID);
    overlay.querySelector('[data-control="qr"]').click();
    overlay.querySelector('[data-control="fullscreen"]').click();
    expect(ran).toEqual(['toggle_qr', 'toggle_fullscreen']);
  });

  it('renders and does nothing when no hook is supplied', () => {
    // The same contract every hook in this fleet has: absent, the control is
    // drawn and the press is inert, rather than the module reaching for a
    // default nobody asked for.
    const doc = freshDoc();
    mountNativeSettings(doc, undefined, { t });
    doc.getElementById(NATIVE_SETTINGS_BUTTON_ID).click();
    expect(() => {
      doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID)
        .querySelector('[data-control="qr"]').click();
    }).not.toThrow();
  });

  it('rebuilds on every open, so a stale tab body cannot flash', () => {
    const doc = freshDoc();
    const panel = mountNativeSettings(doc, {}, { t });
    panel.open();
    const first = doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID).innerHTML;
    panel.close();
    panel.open();
    const overlay = doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID);
    expect(overlay.querySelectorAll('.native-settings-popup')).toHaveLength(1);
    expect(overlay.innerHTML).toBe(first);
  });

  it('closes on a backdrop click, which is the kit’s behaviour and not this module’s', () => {
    const doc = freshDoc();
    const panel = mountNativeSettings(doc, {}, { t });
    panel.open();
    doc.getElementById(NATIVE_SETTINGS_OVERLAY_ID).click();
    expect(panel.isOpen()).toBe(false);
  });
});

// ── The sheet and the wiring ───────────────────────────────────────────────

describe('the native settings chrome', () => {
  it('is painted from the token vocabulary and never from a literal', () => {
    // tests/client/design-tokens.test.js enforces this over the whole sheet;
    // this is the same claim stated where a reader of this module will see it.
    expect(colourLiterals(CSS_STRIPPED)).toEqual([]);
    expect(fontSizeLiterals(CSS_STRIPPED)).toEqual([]);
  });

  it('stacks above everything the native lobby document already lifts', () => {
    // `GROUND_CSS` puts the join panel at 210 and its QR toggle at 211, over a
    // landing at 205. A settings modal underneath any of them would be a panel
    // an operator can open and not reach.
    const zOf = (selector) => {
      const rule = CSS.match(new RegExp(`\\${selector}\\s*\\{[^}]*\\}`));
      return Number(rule[0].match(/z-index:\s*(\d+)/)[1]);
    };
    expect(zOf('.native-settings-overlay')).toBeGreaterThan(211);
    // The cog above its own scrim, as the host page stacks the same pair, so
    // the control that closes the panel is never underneath it.
    expect(zOf('.native-settings-btn')).toBeGreaterThan(zOf('.native-settings-overlay'));
  });

  it('is mounted by the viewscreen surface, with its local verb named once', () => {
    // The one place this module is wired. `toggle_qr` is answered in the page
    // because the QR panel is that document's own DOM; anything else is a
    // record the host answers, which is the default rather than a case.
    expect(LINK_JS).toContain('mountNativeSettings(document');
    expect(LINK_JS).toContain('toggle_qr:');
    expect(LINK_JS).toContain("send({ kind: 'toggle_fullscreen' })");
    // And it is the shared kit that is reused, not the host page's cog: that
    // one is wired to `wasm_*` bindings this process publishes to no document.
    expect(LINK_JS).not.toContain('server-settings');
  });
});
