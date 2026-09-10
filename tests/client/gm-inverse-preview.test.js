// @vitest-environment jsdom
//
// The shared undo-explanation component (issue #1441). Its whole job is
// honesty, so these tests assert what it REFUSES to claim as hard as what it
// renders.
import { beforeEach, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  createGmInversePreview,
  gmInverseAvailability,
  gmInverseSupportedKinds,
  GM_INVERSE_SUPPORT,
  GM_INVERSE_PLANNED,
  GM_INVERSE_OUT_OF_SCOPE,
} from '../../gui/gm-inverse-preview.js';
import { buildTable, setTable, t } from '../../gui/strings.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');
const realStrings = buildTable(read('assets/strings/strings.csv'));

/** Every `GmActionKind`, read off the Rust enum rather than restated here. */
function rustActionKinds() {
  const source = read('src/gm_action.rs');
  const body = source.slice(source.indexOf('pub enum GmActionKind {'));
  const variants = body.slice(0, body.indexOf('\n}')).match(/^\s{4}([A-Z]\w+),$/gm) || [];
  return variants
    .map((line) => line.trim().replace(/,$/, ''))
    .map((variant) => variant.replace(/(?<!^)([A-Z])/g, '-$1').toLowerCase());
}

let host;
beforeEach(() => {
  setTable(realStrings);
  document.body.innerHTML = '<div id="host"></div>';
  host = document.getElementById('host');
});

it('answers for every action family the canonical journal can record', () => {
  const kinds = rustActionKinds();
  expect(kinds.length).toBeGreaterThan(10);
  expect(Object.keys(GM_INVERSE_SUPPORT).sort()).toEqual(kinds.sort());
  for (const kind of kinds) {
    const availability = gmInverseAvailability(kind);
    expect(availability.supported).toBe(false);
    expect([GM_INVERSE_PLANNED, GM_INVERSE_OUT_OF_SCOPE]).toContain(availability.status);
    expect(t(availability.reasonId)).not.toMatch(/^⟨/);
  }
});

it('claims no undo support at all in this build, and says which families are merely planned', () => {
  expect(gmInverseSupportedKinds()).toEqual([]);
  for (const planned of ['world-spawn', 'world-despawn', 'npc-doctrine']) {
    expect(gmInverseAvailability(planned).status).toBe(GM_INVERSE_PLANNED);
  }
  expect(gmInverseAvailability('direct-effect').status).toBe(GM_INVERSE_OUT_OF_SCOPE);
  expect(gmInverseAvailability('comms').status).toBe(GM_INVERSE_OUT_OF_SCOPE);
});

it('renders unavailable eligibility as readable words, not only a data attribute', () => {
  const preview = createGmInversePreview({ doc: document, t });
  preview.render(host, { actionKind: 'world-despawn', target: 'courier' });
  const eligibility = host.querySelector('.gm-inverse-eligibility');
  expect(eligibility.dataset.supported).toBe('false');
  expect(eligibility.dataset.status).toBe(GM_INVERSE_PLANNED);
  expect(eligibility.textContent).toContain(t('server.gm.inverse.unavailable'));
  expect(eligibility.textContent).toContain(t('server.gm.inverse.unavailable.planned'));
  // Nothing that could be mistaken for an offer to undo.
  expect(host.querySelector('button')).toBeNull();
  expect(host.textContent).not.toContain(t('server.gm.inverse.available'));
});

it('says state was not captured rather than showing an empty before/after', () => {
  const preview = createGmInversePreview({ doc: document, t });
  preview.render(host, { actionKind: 'session-pause' });
  const values = [...host.querySelectorAll('dd')].map((node) => node.textContent);
  expect(values[0]).toBe(t('server.gm.inverse.state_uncaptured'));
  expect(values[1]).toBe(t('server.gm.inverse.state_uncaptured'));
  expect(values.some((value) => value.trim() === '')).toBe(false);
});

it('never lets an uncaptured before/after row deny the inverse a planned family is due', () => {
  const preview = createGmInversePreview({ doc: document, t });
  const planned = Object.entries(GM_INVERSE_SUPPORT)
    .filter(([, entry]) => entry.status === GM_INVERSE_PLANNED)
    .map(([kind]) => kind);
  expect(planned.length).toBeGreaterThan(0);
  for (const kind of planned) {
    preview.render(host, { actionKind: kind });
    const [before, after] = [...host.querySelectorAll('dd')].map((node) => node.textContent);
    // An uncaptured row reports only what this surface recorded. It must not
    // rule an inverse out, because the eligibility row for these same families
    // says the opposite: planned, not impossible.
    for (const text of [before, after]) {
      expect(text).not.toMatch(/inverse|undo|reverse|restore/i);
    }
    expect(host.querySelector('.gm-inverse-eligibility').textContent)
      .toContain(t('server.gm.inverse.unavailable.planned'));
  }
});

it('does not claim a family records no state just because this caller passed none', () => {
  const preview = createGmInversePreview({ doc: document, t });
  // `direct-effect` does record its own before/after (the applied/discarded
  // milli-HP delta and lethality, on `LoggedGmAction::effect`); the journal
  // projection simply does not carry it to this surface. Saying the family
  // records nothing would be false.
  preview.render(host, { actionKind: 'direct-effect' });
  const [before, after] = [...host.querySelectorAll('dd')].map((node) => node.textContent);
  for (const text of [before, after]) {
    expect(text).not.toMatch(/records no|never records|cannot be captured/i);
  }
});

it('carries captured before/after through when a caller has them', () => {
  const preview = createGmInversePreview({ doc: document, t });
  preview.render(host, { actionKind: 'npc-doctrine', before: 'patrol', after: 'north' });
  const values = [...host.querySelectorAll('dd')].map((node) => node.textContent);
  expect(values[0]).toBe('patrol');
  expect(values[1]).toBe('north');
});

it('always distinguishes the technical change from what crews already witnessed', () => {
  const preview = createGmInversePreview({ doc: document, t });
  // Even when the caller supplies no witnessed text, and even when it tries to
  // suppress it with an empty string, the standing sentence survives.
  for (const witnessed of [undefined, '']) {
    preview.render(host, { actionKind: 'direct-effect', technical: 'Hull fell by 20.', witnessed });
    const terms = [...host.querySelectorAll('dt')].map((node) => node.textContent);
    const values = [...host.querySelectorAll('dd')].map((node) => node.textContent);
    expect(terms).toContain(t('server.gm.inverse.technical'));
    expect(terms).toContain(t('server.gm.inverse.witnessed'));
    expect(values).toContain('Hull fell by 20.');
    expect(values).toContain(t('server.gm.inverse.witnessed_note'));
  }
});

it('treats an unrecognised future action family as unsupported rather than throwing', () => {
  const preview = createGmInversePreview({ doc: document, t });
  const availability = preview.render(host, { actionKind: 'future-family' });
  expect(availability.supported).toBe(false);
  expect(availability.status).toBe('unknown');
  expect(host.querySelector('.gm-inverse-eligibility').textContent)
    .toContain(t('server.gm.inverse.unavailable.unknown'));
});

it('clears back to nothing so a deselected entry leaves no stale explanation', () => {
  const preview = createGmInversePreview({ doc: document, t });
  preview.render(host, { actionKind: 'comms' });
  preview.clear(host);
  expect(host.childElementCount).toBe(0);
  expect(preview.state()).toBeNull();
});
