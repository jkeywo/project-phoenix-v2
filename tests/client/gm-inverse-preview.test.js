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
  GM_INVERSE_SUPPORTED,
  GM_INVERSE_UNKNOWN,
  GM_INVERSE_EXPIRED,
  gmAffectedFieldText,
  normaliseGmSpawnExposure,
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

/** A placement crews have had for half a second: the window is open. */
const OPEN = Object.freeze({ exposed_ms: 500, limit_ms: 2000, latched: false });
/** The same placement once the cutoff has closed. */
const SHUT = Object.freeze({ exposed_ms: 2000, limit_ms: 2000, latched: true });
const PLACED = Object.freeze({
  'spawned-entity': { name: 'gm_courier_7', before: false, after: true },
});

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
    // A family whose answer depends on this entry's live exposure is asked
    // with one, so the table's own answer is what is under test here.
    const availability = gmInverseAvailability(kind, OPEN);
    expect([GM_INVERSE_SUPPORTED, GM_INVERSE_PLANNED, GM_INVERSE_OUT_OF_SCOPE])
      .toContain(availability.status);
    expect(availability.supported).toBe(availability.status === GM_INVERSE_SUPPORTED);
    expect(t(availability.reasonId)).not.toMatch(/^⟨/);
  }
});

it('claims undo support for exactly the families the reducer can reverse', () => {
  // Issue #1442 builds the NPC-doctrine and faction-relation inverses, #1443
  // the placement one. Despawn remains PLANNED and nothing else may claim
  // support.
  expect(gmInverseSupportedKinds().sort())
    .toEqual(['faction-relation', 'npc-doctrine', 'world-spawn']);
  for (const planned of ['world-despawn']) {
    expect(gmInverseAvailability(planned).status).toBe(GM_INVERSE_PLANNED);
  }
  expect(gmInverseAvailability('direct-effect').status).toBe(GM_INVERSE_OUT_OF_SCOPE);
  expect(gmInverseAvailability('comms').status).toBe(GM_INVERSE_OUT_OF_SCOPE);
  // An undo of an undo is a redo, and is refused as a matter of vocabulary.
  expect(gmInverseAvailability('action-undo').status).toBe(GM_INVERSE_OUT_OF_SCOPE);
});

it('describes a recorded affected field in words on both sides', () => {
  const preview = createGmInversePreview({ doc: document, t });
  preview.render(host, {
    actionKind: 'npc-doctrine',
    affected: { 'npc-doctrine': { entity: 'courier-1', before: null, after: 'north' } },
  });
  const values = [...host.querySelectorAll('dd')].map((node) => node.textContent);
  // Subject, before, after — the `None` side says the entity's own authored
  // doctrine rather than reading as "nothing" or "unknown".
  expect(values[0]).toContain('courier-1');
  expect(values[1]).toBe(t('server.gm.inverse.doctrine_authored'));
  expect(values[2]).toBe(t('server.gm.inverse.doctrine_value', { doctrine: 'north' }));
  const eligibility = host.querySelector('.gm-inverse-eligibility');
  expect(eligibility.dataset.supported).toBe('true');
  expect(eligibility.textContent).toContain(t('server.gm.inverse.available'));
  // Even a supported inverse states what cannot be undone.
  expect(host.textContent).toContain(t('server.gm.inverse.witnessed_note'));
});

it('names the ordered faction pair rather than implying a mutual relationship', () => {
  const described = gmAffectedFieldText(
    { 'faction-hostility': { faction: 'Alliance', enemy: 'Harrow', before: false, after: true } },
    t,
  );
  expect(described.subject).toBe(
    t('server.gm.inverse.subject_relation', { faction: 'Alliance', enemy: 'Harrow' }),
  );
  expect(described.before).toBe(t('server.gm.inverse.hostility_neutral'));
  expect(described.after).toBe(t('server.gm.inverse.hostility_hostile'));
  // An unrecognised future field is not guessed at.
  expect(gmAffectedFieldText({ 'something-new': { value: 1 } }, t)).toBeNull();
  expect(gmAffectedFieldText(undefined, t)).toBeNull();
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

// ── The placement inverse and its two-second window (issue #1443) ────────────

it('names the placement and says plainly whether it is in the world', () => {
  const described = gmAffectedFieldText(PLACED, t);
  expect(described.subject).toBe(
    t('server.gm.inverse.subject_placement', { name: 'gm_courier_7' }),
  );
  expect(described.before).toBe(t('server.gm.inverse.presence_absent'));
  expect(described.after).toBe(t('server.gm.inverse.presence_present'));
});

it('offers the placement inverse while the window is open, and says how much is left', () => {
  const preview = createGmInversePreview({ doc: document, t });
  const availability = preview.render(host, {
    actionKind: 'world-spawn',
    affected: PLACED,
    exposure: OPEN,
  });
  expect(availability.supported).toBe(true);
  const exposure = host.querySelector('.gm-inverse-exposure');
  expect(exposure.dataset.latched).toBe('false');
  // The count in words and numbers, before it runs out rather than after.
  expect(exposure.textContent)
    .toBe(t('server.gm.inverse.exposure_remaining', { elapsed: '0.5', limit: '2.0' }));
  expect(host.querySelector('.gm-inverse-eligibility').textContent)
    .toContain(t('server.gm.inverse.available'));
});

it('withdraws the offer once crews have had the placement for two seconds', () => {
  const preview = createGmInversePreview({ doc: document, t });
  const availability = preview.render(host, {
    actionKind: 'world-spawn',
    affected: PLACED,
    exposure: SHUT,
  });
  expect(availability.supported).toBe(false);
  expect(availability.status).toBe(GM_INVERSE_EXPIRED);
  const eligibility = host.querySelector('.gm-inverse-eligibility');
  expect(eligibility.dataset.supported).toBe('false');
  expect(eligibility.textContent).toContain(t('server.gm.inverse.unavailable.sensor_exposure'));
  expect(host.querySelector('.gm-inverse-exposure').dataset.latched).toBe('true');
  expect(host.querySelector('.gm-inverse-exposure').textContent)
    .toBe(t('server.gm.inverse.exposure_elapsed', { limit: '2.0' }));
  // The family is still reversible in general; this one was seen. The two
  // sentences must not be confused.
  expect(eligibility.textContent).not.toContain(t('server.gm.inverse.unavailable.planned'));
});

it('offers nothing for a placement whose exposure this session is not reporting', () => {
  const preview = createGmInversePreview({ doc: document, t });
  for (const exposure of [undefined, null, {}, { exposed_ms: 1, limit_ms: 2 },
    { exposed_ms: -1, limit_ms: 2000, latched: false },
    { exposed_ms: 1, limit_ms: 2000, latched: 'no' }]) {
    const availability = preview.render(host, {
      actionKind: 'world-spawn',
      affected: PLACED,
      exposure,
    });
    expect(availability.supported).toBe(false);
    expect(availability.status).toBe(GM_INVERSE_UNKNOWN);
    expect(host.querySelector('.gm-inverse-eligibility').textContent)
      .toContain(t('server.gm.inverse.unavailable.exposure_unknown'));
    // No half-believed clock either.
    expect(host.querySelector('.gm-inverse-exposure')).toBeNull();
  }
});

it('parses an exposure status strictly rather than half-believing one', () => {
  expect(normaliseGmSpawnExposure(OPEN)).toEqual({ ...OPEN });
  expect(normaliseGmSpawnExposure({ ...OPEN, exposed_ms: 1.5 })).toBeNull();
  expect(normaliseGmSpawnExposure('2000')).toBeNull();
  expect(normaliseGmSpawnExposure(null)).toBeNull();
});

it('never lets an exposure clock reach a family that has no window', () => {
  const preview = createGmInversePreview({ doc: document, t });
  // A doctrine change is reversible for as long as nothing else moves it; a
  // stray exposure payload must not shorten that or draw a clock.
  const availability = preview.render(host, {
    actionKind: 'npc-doctrine',
    affected: { 'npc-doctrine': { entity: 'courier-1', before: null, after: 'north' } },
    exposure: SHUT,
  });
  expect(availability.supported).toBe(true);
  expect(host.querySelector('.gm-inverse-eligibility').textContent)
    .toContain(t('server.gm.inverse.available'));
});
