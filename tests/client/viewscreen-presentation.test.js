// @vitest-environment jsdom
//
// Issue #1427 — the Viewscreen settings menu saves and previews THIS display's
// presentation settings.
//
// The claims this file has to make true are the acceptance criteria's, in the
// order an operator meets them:
//
//   1. the menu on both viewscreen runtimes actually offers the controls;
//   2. what they write stays on this ENDPOINT — a distinct store, never the
//      operator profile, never a scenario save;
//   3. it is still there after a restart, and after a different scenario;
//   4. a private console profile and this record cannot disturb each other;
//   5. per-setting reset and Reset all are each scoped to what they name;
//   6. the menu and its overlays are usable at 200% and dismiss on Escape.
//
// Everything below drives the REAL modules against a real jsdom document, and
// the surface-level assertions read `server.html`, `gui/native-settings.css` and
// the native lobby's own link script off disk rather than a hand-written
// stand-in — a rule that is only in a file nobody imports is not a rule.

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { t } from '../../gui/strings.js';
import {
  VIEWSCREEN_PRESENTATION_KEY,
  VIEWSCREEN_EFFECTS,
  emptyViewscreenPresentation,
  normalizeViewscreenPresentation,
  presentationWithEffect,
  presentationWithDefaults,
  isDefaultViewscreenPresentation,
  loadViewscreenPresentation,
  saveViewscreenPresentation,
  browserViewscreenStore,
  readInjectedViewscreenPresentation,
  viewscreenPresentationRecordFields,
  resolveViewscreenEffects,
  viewscreenPresentationStatus,
  createViewscreenPresentation,
} from '../../gui/viewscreen-presentation.js';
import { VIEWSCREEN_PRESENTATION_CONTROLS } from '../../gui/viewscreen-presentation-panel.js';
import {
  TEXT_SCALE_MAX,
  TEXT_SCALE_MIN,
  TEXT_SCALE_STEP,
} from '../../gui/accessibility-profile.js';
import { OPERATOR_PROFILE_KEY } from '../../gui/operator-profile.js';
import {
  visibleTabs,
  visibleViewscreenTabs,
  resolveViewscreenActiveTab,
} from '../../gui/settings-tabs.js';
import { mountServerSettings } from '../../gui/server-settings.js';
import { mountNativeSettings, nativeSettingsView } from '../../gui/native-settings.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const read = (rel) => fs.readFileSync(path.join(HERE, '../../', rel), 'utf-8');
const SERVER_HTML = read('server.html');
const NATIVE_CSS = read('gui/native-settings.css');
const NATIVE_LINK_JS = read('src/native_host/host_lobby/host_lobby_link.js');
const RECORD_RS = read('src/native_host/host_lobby/scenario.rs');
const TOKENS_CSS = read('gui/tokens.css');

/** A localStorage-shaped store that also lets a test look at the raw bytes. */
function fakeStorage(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => { map.set(key, String(value)); },
    removeItem: (key) => { map.delete(key); },
    keys: () => [...map.keys()].sort(),
    raw: (key) => (map.has(key) ? map.get(key) : null),
  };
}

/**
 * A document with no browsing context, for the cases that only need a root to
 * write onto. The cog tests below use the REAL jsdom document instead: focus,
 * `activeElement` and dispatched keyboard events only behave like a browser's
 * inside a browsing context, and Escape-dismissal is one of the claims.
 */
function freshDoc() {
  return document.implementation.createHTMLDocument('');
}

/**
 * jsdom has no `matchMedia` and no injected host defaults, so its window IS the
 * silent machine every "follows the system" case below needs: nothing asked for
 * reduced motion, nothing asked for contrast, nothing named a text size.
 */
function silentWindow(doc) {
  return doc.defaultView || window;
}

/** Put the real document back the way each cog test expects to find it. */
function resetRealDocument() {
  document.body.innerHTML = '';
  document.documentElement.style.removeProperty('--a11y-text-scale');
  document.documentElement.removeAttribute('data-contrast');
  document.documentElement.removeAttribute('data-reduced-motion');
}

describe('the endpoint record', () => {
  it('starts out following the machine it is running on', () => {
    const record = emptyViewscreenPresentation();
    expect(record).toEqual({ textScale: 'default', contrast: 'default' });
    expect(isDefaultViewscreenPresentation(record)).toBe(true);
    expect(VIEWSCREEN_EFFECTS).toEqual(['textScale', 'contrast']);
  });

  it('coerces anything untrusted into a usable record rather than throwing', () => {
    // Storage JSON, a hand-edited file and a host injection all arrive here.
    for (const bad of [null, 7, 'nonsense', [], { presentation: 3 }]) {
      expect(normalizeViewscreenPresentation(bad)).toEqual(emptyViewscreenPresentation());
    }
    // A value outside the supported range is CLAMPED, not dropped: a record
    // asking for 400% wants the largest size this build claims to reflow at.
    expect(normalizeViewscreenPresentation({ textScale: 4 }).textScale).toBe(TEXT_SCALE_MAX);
    expect(normalizeViewscreenPresentation({ textScale: Number.NaN }).textScale).toBe('default');
    // An unknown tri-state reads as "follow the machine", never as "off".
    expect(normalizeViewscreenPresentation({ contrast: 'maybe' }).contrast).toBe('default');
  });

  it('changes one effect at a time and reports "nothing changed" by identity', () => {
    const record = emptyViewscreenPresentation();
    const scaled = presentationWithEffect(record, 'textScale', 1.5);
    expect(scaled.textScale).toBe(1.5);
    expect(scaled.contrast).toBe('default');
    // The same value again is the SAME object, so a caller can skip a persist
    // and a re-apply — the contract the private profile's writer has.
    expect(presentationWithEffect(scaled, 'textScale', 1.5)).toBe(scaled);
    // An effect this record does not carry is a no-op, not a wiped record.
    expect(presentationWithEffect(scaled, 'reducedMotion', 'on')).toBe(scaled);
  });
});

describe('endpoint storage', () => {
  it('survives a restart of the page that wrote it', () => {
    const storage = fakeStorage();
    saveViewscreenPresentation(storage, { textScale: 1.75, contrast: 'on' });
    // A fresh read with no memory of the writer is the next launch.
    expect(loadViewscreenPresentation(storage)).toEqual({ textScale: 1.75, contrast: 'on' });
    expect(storage.keys()).toEqual([VIEWSCREEN_PRESENTATION_KEY]);
  });

  it('writes ONE key, and not the private operator profile', () => {
    // Both records live on one origin — a laptop that is a viewscreen in one tab
    // and a console in another — so the isolation is the key, not the machine.
    const storage = fakeStorage({ [OPERATOR_PROFILE_KEY]: '{"kind":"x","version":1}' });
    saveViewscreenPresentation(storage, { textScale: 2, contrast: 'off' });
    expect(storage.raw(OPERATOR_PROFILE_KEY)).toBe('{"kind":"x","version":1}');
    expect(storage.keys()).toEqual([OPERATOR_PROFILE_KEY, VIEWSCREEN_PRESENTATION_KEY].sort());
    expect(VIEWSCREEN_PRESENTATION_KEY).not.toBe(OPERATOR_PROFILE_KEY);
  });

  it('is not disturbed by a private console profile being rewritten', () => {
    const storage = fakeStorage();
    saveViewscreenPresentation(storage, { textScale: 1.5, contrast: 'on' });
    const before = storage.raw(VIEWSCREEN_PRESENTATION_KEY);
    // Whatever the console writes under its own key, byte for byte.
    storage.setItem(OPERATOR_PROFILE_KEY, JSON.stringify({
      kind: 'project-phoenix/operator-profile',
      version: 1,
      accessibility: { presentation: { textScale: 1, contrast: 'off' } },
    }));
    expect(storage.raw(VIEWSCREEN_PRESENTATION_KEY)).toBe(before);
    expect(loadViewscreenPresentation(storage)).toEqual({ textScale: 1.5, contrast: 'on' });
  });

  it('survives every other key on the endpoint being cleared', () => {
    // "Independently of scenario saves": nothing about a save, a session or a
    // scenario is in this record, so removing all of them leaves it whole.
    const storage = fakeStorage({
      'phoenix-save-slots': '[]',
      'session-token': 'abc',
      'phoenix-scenario': 'combat_test',
    });
    saveViewscreenPresentation(storage, { textScale: 2, contrast: 'on' });
    for (const key of ['phoenix-save-slots', 'session-token', 'phoenix-scenario']) {
      storage.removeItem(key);
    }
    expect(loadViewscreenPresentation(storage)).toEqual({ textScale: 2, contrast: 'on' });
  });

  it('forgets rather than fails when storage refuses', () => {
    const refusing = {
      getItem: () => { throw new Error('blocked'); },
      setItem: () => { throw new Error('blocked'); },
    };
    expect(() => saveViewscreenPresentation(refusing, { textScale: 2 })).not.toThrow();
    expect(loadViewscreenPresentation(refusing)).toEqual(emptyViewscreenPresentation());
    // Corrupt bytes are the same answer: the display comes up at its default.
    expect(loadViewscreenPresentation(fakeStorage({
      [VIEWSCREEN_PRESENTATION_KEY]: '{not json',
    }))).toEqual(emptyViewscreenPresentation());
  });
});

describe('resolution against the machine, and the status it reports', () => {
  it('lets an explicit choice win over the system in both directions', () => {
    // The system asks for higher contrast…
    const os = { contrast: true, textScale: 1.5 };
    expect(resolveViewscreenEffects(emptyViewscreenPresentation(), os))
      .toEqual({ textScale: 1.5, contrast: true });
    // …and the operator at the screen overrules it, both ways.
    expect(resolveViewscreenEffects({ contrast: 'off', textScale: 1 }, os))
      .toEqual({ textScale: 1, contrast: false });
    expect(resolveViewscreenEffects({ contrast: 'on' }, { contrast: false }).contrast).toBe(true);
  });

  it('says whether a live value is the operator’s, the machine’s or the default', () => {
    const chosen = viewscreenPresentationStatus({ textScale: 2 }, { textScale: 1.5 });
    expect(chosen.textScale).toEqual({ value: 2, source: 'explicit', available: true });
    const followed = viewscreenPresentationStatus(emptyViewscreenPresentation(), { textScale: 1.5 });
    expect(followed.textScale.source).toBe('system');
    const silent = viewscreenPresentationStatus(emptyViewscreenPresentation(), {});
    expect(silent.textScale).toEqual({ value: 1, source: 'default', available: true });
    // A read that FAILED is distinct from a machine that said nothing.
    const unread = viewscreenPresentationStatus(emptyViewscreenPresentation(), {}, ['contrast']);
    expect(unread.contrast.available).toBe(false);
    // …and it never invalidates a value the operator chose here.
    const overridden = viewscreenPresentationStatus({ contrast: 'on' }, {}, ['contrast']);
    expect(overridden.contrast.available).toBe(true);
  });
});

describe('the live controller', () => {
  let doc;
  let win;
  let storage;
  let presentation;

  beforeEach(() => {
    doc = freshDoc();
    win = window;
    storage = fakeStorage();
    presentation = createViewscreenPresentation({
      doc, win, store: browserViewscreenStore(storage),
    });
  });

  it('previews on this document’s root the moment a value changes', () => {
    presentation.set('textScale', 2);
    expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('2');
    presentation.set('contrast', 'on');
    expect(doc.documentElement.getAttribute('data-contrast')).toBe('more');
    presentation.set('contrast', 'off');
    expect(doc.documentElement.getAttribute('data-contrast')).toBe('standard');
  });

  it('does not stamp motion, which this endpoint does not own yet', () => {
    // The viewscreen's shake and flash follow `prefers-reduced-motion` today
    // (issue #1428 owns the controls). Stamping the attribute here would
    // out-specify that query for a setting nothing on this tab offers.
    presentation.set('contrast', 'on');
    expect(doc.documentElement.hasAttribute('data-reduced-motion')).toBe(false);
  });

  it('persists every change, so the next launch reads it back', () => {
    presentation.set('textScale', 1.5);
    presentation.set('contrast', 'on');
    const nextLaunch = createViewscreenPresentation({
      doc: freshDoc(), win, store: browserViewscreenStore(storage),
    });
    expect(nextLaunch.record()).toEqual({ textScale: 1.5, contrast: 'on' });
    // …and applies it without anyone opening the menu.
    const secondDoc = freshDoc();
    createViewscreenPresentation({
      doc: secondDoc, win, store: browserViewscreenStore(storage),
    }).apply();
    expect(secondDoc.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('1.5');
  });

  it('resets one setting without touching the other, and all of them together', () => {
    presentation.set('textScale', 1.5);
    presentation.set('contrast', 'on');

    presentation.reset('contrast');
    expect(presentation.record()).toEqual({ textScale: 1.5, contrast: 'default' });
    expect(loadViewscreenPresentation(storage).textScale).toBe(1.5);

    presentation.resetAll();
    expect(presentation.record()).toEqual(emptyViewscreenPresentation());
    expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('1');
  });

  it('keeps a Reset all inside this record and nowhere near the endpoint’s other data', () => {
    const shared = fakeStorage({
      [OPERATOR_PROFILE_KEY]: '{"bindings":"mine"}',
      'phoenix-save-slots': '["slot-1"]',
    });
    const scoped = createViewscreenPresentation({
      doc, win, store: browserViewscreenStore(shared),
    });
    scoped.set('textScale', 2);
    scoped.resetAll();
    expect(shared.raw(OPERATOR_PROFILE_KEY)).toBe('{"bindings":"mine"}');
    expect(shared.raw('phoenix-save-slots')).toBe('["slot-1"]');
    // Reset-all is `presentationWithDefaults`, which cannot NAME another record.
    expect(presentationWithDefaults({ textScale: 2, contrast: 'on' }))
      .toEqual(emptyViewscreenPresentation());
  });

  it('keeps the preview when the store cannot be written', () => {
    const refusing = { load: () => ({}), save: () => { throw new Error('read-only'); } };
    const live = createViewscreenPresentation({ doc, win, store: refusing });
    expect(() => live.set('textScale', 2)).not.toThrow();
    expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('2');
  });
});

describe('the browser viewscreen’s cog', () => {
  // The REAL jsdom document: focus, `activeElement` and dispatched keyboard
  // events need a browsing context, and Escape-dismissal is one of the claims.
  const doc = document;
  let storage;
  let panel;

  beforeEach(() => {
    resetRealDocument();
    storage = fakeStorage();
    panel = mountServerSettings({
      doc,
      bindings: { __getMasterVolume: () => 1 },
      isDemo: () => false,
      autoRefresh: false,
      startGamepad: false,
      presentation: createViewscreenPresentation({
        doc, win: window, store: browserViewscreenStore(storage),
      }),
    });
  });

  afterEach(() => {
    panel.destroy();
    resetRealDocument();
  });

  const control = (id) => doc.querySelector(`[data-control="${id}"]`);

  it('offers the Display tab beside the shared operational tabs', () => {
    panel.open();
    panel.selectTab('presentation');
    const tabs = [...doc.querySelectorAll('.server-settings-tab')].map(
      (el) => el.getAttribute('data-tab'),
    );
    expect(tabs).toContain('presentation');
    // The shared list is still the shared list — this is an addition, not a
    // second tab table, and Display is not the tab a demo build lands on.
    for (const shared of visibleTabs(false)) expect(tabs).toContain(shared.id);
    expect(resolveViewscreenActiveTab(null, true)).toBe(visibleTabs(true)[0].id);
    expect(doc.querySelector('.server-settings-tab.active').getAttribute('data-tab'))
      .toBe('presentation');
  });

  it('builds the slider at the shared contract’s bounds, never its own', () => {
    panel.open();
    panel.selectTab('presentation');
    const slider = control(VIEWSCREEN_PRESENTATION_CONTROLS.textScale);
    expect(slider.min).toBe(String(TEXT_SCALE_MIN));
    expect(slider.max).toBe(String(TEXT_SCALE_MAX));
    expect(slider.step).toBe(String(TEXT_SCALE_STEP));
    expect(slider.getAttribute('aria-label')).toBe(t('settings.viewscreen.text_scale'));
  });

  it('applies and saves a text size while the slider is still moving', () => {
    panel.open();
    panel.selectTab('presentation');
    const slider = control(VIEWSCREEN_PRESENTATION_CONTROLS.textScale);
    slider.value = '2';
    // `input`, not `change`: the menu has to grow under the finger.
    slider.dispatchEvent(new Event('input'));

    expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('2');
    expect(loadViewscreenPresentation(storage).textScale).toBe(2);
    // The panel was NOT rebuilt, so the drag survives: the same node is still
    // in the document and still focusable.
    expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.textScale)).toBe(slider);
    // …and the readout and status line moved with it.
    expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.textScaleStatus).textContent)
      .toContain(t('settings.viewscreen.text_scale_value', { value: '200' }));
    expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.textScaleStatus).textContent)
      .toContain(t('settings.viewscreen.source_explicit'));
  });

  it('presses one contrast option at a time and leaves focus on it', () => {
    panel.open();
    panel.selectTab('presentation');
    const more = control(VIEWSCREEN_PRESENTATION_CONTROLS.contrast('on'));
    const standard = control(VIEWSCREEN_PRESENTATION_CONTROLS.contrast('off'));
    const system = control(VIEWSCREEN_PRESENTATION_CONTROLS.contrast('default'));

    more.focus();
    more.click();
    expect(doc.documentElement.getAttribute('data-contrast')).toBe('more');
    expect(more.getAttribute('aria-pressed')).toBe('true');
    expect(standard.getAttribute('aria-pressed')).toBe('false');
    expect(system.getAttribute('aria-pressed')).toBe('false');
    // A press must not throw the panel away under the operator's finger.
    expect(doc.activeElement).toBe(more);

    standard.click();
    expect(doc.documentElement.getAttribute('data-contrast')).toBe('standard');
    expect(more.getAttribute('aria-pressed')).toBe('false');
  });

  it('resets one setting from its own button and both from Reset all', () => {
    panel.open();
    panel.selectTab('presentation');
    control(VIEWSCREEN_PRESENTATION_CONTROLS.contrast('on')).click();
    const slider = control(VIEWSCREEN_PRESENTATION_CONTROLS.textScale);
    slider.value = '1.5';
    slider.dispatchEvent(new Event('input'));

    control(VIEWSCREEN_PRESENTATION_CONTROLS.contrastReset).click();
    expect(loadViewscreenPresentation(storage)).toEqual({ textScale: 1.5, contrast: 'default' });

    control(VIEWSCREEN_PRESENTATION_CONTROLS.resetAll).click();
    expect(loadViewscreenPresentation(storage)).toEqual(emptyViewscreenPresentation());
    // Two reset buttons, named differently, so neither reads as the other —
    // and the scope hint says in words what this one leaves alone.
    expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.resetAll).textContent)
      .toBe(t('settings.viewscreen.reset_all'));
    expect(doc.querySelector('.server-settings-body').textContent)
      .toContain(t('settings.viewscreen.reset_all_scope_hint'));
  });

  it('applies the saved settings at mount, before anyone opens the menu', () => {
    const bootDoc = freshDoc();
    const saved = fakeStorage();
    saveViewscreenPresentation(saved, { textScale: 1.5, contrast: 'on' });
    const booted = mountServerSettings({
      doc: bootDoc,
      bindings: {},
      isDemo: () => false,
      autoRefresh: false,
      startGamepad: false,
      presentation: createViewscreenPresentation({
        doc: bootDoc, win: window, store: browserViewscreenStore(saved),
      }),
    });
    booted.destroy();
    expect(bootDoc.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('1.5');
    expect(bootDoc.documentElement.getAttribute('data-contrast')).toBe('more');
  });

  it('stays operable and dismissible at 200% text', () => {
    // Opened by PRESSING the cog, not by calling `open()`: the focus trap
    // remembers the element that opened it, and "focus returns to the cog"
    // is only a real claim about the route an operator actually takes.
    const cog = doc.getElementById('server-settings-btn');
    cog.focus();
    cog.click();
    panel.selectTab('presentation');
    const slider = control(VIEWSCREEN_PRESENTATION_CONTROLS.textScale);
    slider.value = String(TEXT_SCALE_MAX);
    slider.dispatchEvent(new Event('input'));

    // Every control the tab offers is still in the document and still enabled
    // at the largest supported size — nothing was dropped to make room.
    for (const id of [
      VIEWSCREEN_PRESENTATION_CONTROLS.textScale,
      VIEWSCREEN_PRESENTATION_CONTROLS.textScaleReset,
      VIEWSCREEN_PRESENTATION_CONTROLS.contrast('default'),
      VIEWSCREEN_PRESENTATION_CONTROLS.contrast('on'),
      VIEWSCREEN_PRESENTATION_CONTROLS.contrast('off'),
      VIEWSCREEN_PRESENTATION_CONTROLS.contrastReset,
      VIEWSCREEN_PRESENTATION_CONTROLS.resetAll,
    ]) {
      const el = control(id);
      expect(el, id).toBeTruthy();
      expect(el.disabled).toBeFalsy();
    }

    // Keyboard dismissal, with the setting kept: Escape closes the modal and
    // hands focus back to the cog (the kit's #1174 contract).
    expect(panel.isOpen()).toBe(true);
    doc.querySelector('#server-settings-overlay').dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
    );
    expect(panel.isOpen()).toBe(false);
    expect(doc.activeElement).toBe(cog);
    expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale'))
      .toBe(String(TEXT_SCALE_MAX));
    expect(loadViewscreenPresentation(storage).textScale).toBe(TEXT_SCALE_MAX);
  });
});

describe('the native viewscreen’s cog', () => {
  const t9n = (id) => id;

  it('offers the Display tab when the surface has a store behind it', () => {
    const doc = freshDoc();
    const presentation = createViewscreenPresentation({
      doc, win: silentWindow(doc), store: browserViewscreenStore(fakeStorage()),
    });
    mountNativeSettings(doc, { run: () => {}, presentation }, { t: t9n }).open();
    const tabs = [...doc.querySelectorAll('.native-settings-tab')].map(
      (el) => el.getAttribute('data-tab'),
    );
    expect(tabs).toEqual(['gameplay', 'presentation']);
  });

  it('drops the tab entirely on a mount with nothing behind it', () => {
    // The surface's own rule, one level up: a control exists exactly when
    // something answers it. A slider that forgot every press would be worse
    // than no slider.
    const doc = freshDoc();
    mountNativeSettings(doc, { run: () => {} }, { t: t9n }).open();
    const tabs = [...doc.querySelectorAll('.native-settings-tab')].map(
      (el) => el.getAttribute('data-tab'),
    );
    expect(tabs).toEqual(['gameplay']);
    expect(doc.querySelector(`[data-control="${VIEWSCREEN_PRESENTATION_CONTROLS.textScale}"]`))
      .toBeNull();
  });

  it('builds the same controls the browser cog builds', () => {
    const doc = freshDoc();
    const storage = fakeStorage();
    const presentation = createViewscreenPresentation({
      doc, win: silentWindow(doc), store: browserViewscreenStore(storage),
    });
    const cog = mountNativeSettings(doc, { run: () => {}, presentation }, { t: t9n });
    cog.open();
    cog.selectTab('presentation');
    const control = (id) => doc.querySelector(`[data-control="${id}"]`);

    control(VIEWSCREEN_PRESENTATION_CONTROLS.contrast('on')).click();
    expect(doc.documentElement.getAttribute('data-contrast')).toBe('more');
    expect(loadViewscreenPresentation(storage).contrast).toBe('on');

    control(VIEWSCREEN_PRESENTATION_CONTROLS.resetAll).click();
    expect(loadViewscreenPresentation(storage)).toEqual(emptyViewscreenPresentation());
  });

  it('shows the shared panel’s own headings rather than a duplicate one', () => {
    // The row carries no `sectionId`: the panel writes its own headings, and a
    // heading from the table too would be the same words twice.
    const vm = nativeSettingsView({ activeTab: 'presentation' });
    const presentationSection = vm.sections.find((s) => s.kind === 'presentation');
    expect(presentationSection).toBeTruthy();
    expect(presentationSection.headingId).toBeNull();
    // The ordinary button sections are untouched by the new `kind`.
    const gameplay = nativeSettingsView({ activeTab: 'gameplay' });
    expect(gameplay.sections.map((s) => s.kind)).toEqual(['action', 'action']);
    expect(gameplay.sections.map((s) => s.headingId))
      .toEqual(['settings.qr_code', 'settings.display']);
  });
});

describe('the native endpoint’s host-side store', () => {
  it('seeds the page from the host and sends the whole record back', () => {
    // The two halves of the seam, asserted where they meet: the injected shape
    // is the host's (a number and a BOOLEAN, or null), the record's is the
    // page's tri-state, and neither side is free to invent a third.
    expect(readInjectedViewscreenPresentation({
      PhoenixViewscreenPresentation: { textScale: 1.5, contrast: true },
    })).toEqual({ textScale: 1.5, contrast: 'on' });
    expect(readInjectedViewscreenPresentation({
      PhoenixViewscreenPresentation: { textScale: null, contrast: false },
    })).toEqual({ textScale: 'default', contrast: 'off' });
    // Nothing injected — an older host, or a machine with no settings
    // directory — is "follow this machine", not a crash.
    expect(readInjectedViewscreenPresentation({})).toBeNull();
    expect(readInjectedViewscreenPresentation(null)).toBeNull();

    expect(viewscreenPresentationRecordFields({ textScale: 1.5, contrast: 'on' }))
      .toEqual({ text_scale_percent: 150, contrast: true });
    expect(viewscreenPresentationRecordFields(emptyViewscreenPresentation()))
      .toEqual({ text_scale_percent: null, contrast: null });
  });

  it('speaks the record the Rust side actually decodes', () => {
    // A cross-language contract with no compiler behind it: the page writes the
    // tag and both field names by hand. Pinned against the enum rather than
    // discovered on a viewscreen that quietly stopped remembering.
    expect(RECORD_RS).toContain('SetPresentation {');
    expect(RECORD_RS).toContain('text_scale_percent: Option<u32>');
    expect(RECORD_RS).toContain('contrast: Option<bool>');
    expect(NATIVE_LINK_JS).toContain("kind: 'set_presentation'");
    // …and the page applies it locally rather than waiting for the host, which
    // is what makes the preview live on a surface whose store is a file.
    expect(NATIVE_LINK_JS).toContain('presentation.apply()');
    expect(NATIVE_LINK_JS).toContain('readInjectedViewscreenPresentation');
  });
});

describe('the two viewscreen documents scale their own chrome', () => {
  it('names one token and both documents multiply it by the text scale', () => {
    expect(TOKENS_CSS).toContain('--root-size-viewscreen:');
    const rule = 'html { font-size: calc(var(--root-size-viewscreen) * var(--a11y-text-scale, 1)); }';
    expect(SERVER_HTML).toContain(rule);
    // The native lobby's ground CSS carries the identical rule — checked in
    // Rust (`this_surface_scales_its_own_chrome_with_the_saved_text_size`),
    // because that string lives in `document.rs` rather than in a sheet.
  });

  it('wraps the tab strip so five tabs do not squeeze at 200%', () => {
    const tabs = SERVER_HTML.slice(SERVER_HTML.indexOf('.server-settings-tabs {'));
    expect(tabs.slice(0, tabs.indexOf('}'))).toContain('flex-wrap: wrap');
    expect(NATIVE_CSS.slice(NATIVE_CSS.indexOf('.native-settings-tabs {')))
      .toContain('flex-wrap: wrap');
  });

  it('bounds and scrolls each settings popup instead of shrinking its text', () => {
    // PRD #1418: panels may wrap, stack and scroll; they may not silently
    // shrink text to preserve the original composition. Both popups are capped
    // against the viewport and scroll inside that cap, which is what makes a
    // 200% Display tab reachable rather than clipped.
    for (const [name, sheet, selector] of [
      ['server.html', SERVER_HTML, '.server-settings-popup {'],
      ['native-settings.css', NATIVE_CSS, '.native-settings-popup {'],
    ]) {
      const at = sheet.indexOf(selector);
      expect(at, name).toBeGreaterThan(-1);
      const body = sheet.slice(at, sheet.indexOf('}', at));
      expect(body, name).toContain('max-height');
      expect(body, name).toContain('overflow-y: auto');
    }
  });

  it('puts every settings-menu control on a rung that follows the text scale', () => {
    // `--text-min` is the ABSOLUTE FLOOR of the ramp (gui/tokens.css) — a bare
    // `11px`, not a `max(px, rem)` — so a declaration that reaches for it opts
    // out of the root rule above entirely. On the browser cog that had pinned
    // the tab labels, the contrast options, both per-setting Reset buttons,
    // Reset all and the percentage readout an operator is reading at 11px while
    // the headings around them grew to 20.8px: PRD #1418 story 4's exact
    // failure, "help and recovery become the least readable parts", landing on
    // the recovery controls. `--text-xs` is `max(--text-min, 0.65rem)`, which is
    // the same 11px at the unscaled 16px root and grows from there.
    const styles = SERVER_HTML.slice(SERVER_HTML.indexOf('<style'))
      .replace(/\/\*[\s\S]*?\*\//g, '');
    const pinned = styles
      .split('}')
      .filter((chunk) => chunk.includes('{'))
      .map((chunk) => {
        const brace = chunk.indexOf('{');
        // Everything after the previous rule's close brace up to this one's
        // open brace is the selector list; keep its last line so a rule sitting
        // under an unrelated one is not mistaken for part of it.
        const selector = chunk.slice(0, brace).trim().split('\n').pop().trim();
        return { selector, body: chunk.slice(brace + 1) };
      })
      .filter(({ selector, body }) =>
        /server-settings|settings-binding|host-action-feedback/.test(selector)
        && /font-size:\s*var\(--text-min\)/.test(body))
      .map(({ selector }) => selector);
    expect(pinned).toEqual([]);
  });

  it('marks the chosen contrast option by more than colour on both surfaces', () => {
    // A selection shown only in colour is a selection that disappears the
    // moment the operator turns contrast up, which is the control they are
    // standing at. Border plus background, and `aria-pressed` besides.
    const active = NATIVE_CSS.slice(NATIVE_CSS.indexOf('.native-settings-control.active {'));
    const body = active.slice(0, active.indexOf('}'));
    expect(body).toContain('border-color');
    expect(body).toContain('background');
    expect(SERVER_HTML).toContain('.server-settings-control.active {');
  });
});

describe('the tab list', () => {
  it('adds Display to the viewscreen list without touching the shared one', () => {
    const shared = visibleTabs(false).map((tab) => tab.id);
    const viewscreen = visibleViewscreenTabs(false).map((tab) => tab.id);
    expect(shared).not.toContain('presentation');
    expect(viewscreen).toEqual(['audio', 'gameplay', 'controls', 'presentation', 'debug']);
    // A demo build hides Debug and keeps Display: a shared screen in a demo is
    // still a screen someone reads from the back of the room.
    expect(visibleViewscreenTabs(true).map((tab) => tab.id))
      .toEqual(['audio', 'gameplay', 'controls', 'presentation']);
    // …and it is never the tab a cold menu lands on.
    expect(resolveViewscreenActiveTab(null, false)).toBe('audio');
    expect(resolveViewscreenActiveTab('presentation', false)).toBe('presentation');
  });
});
