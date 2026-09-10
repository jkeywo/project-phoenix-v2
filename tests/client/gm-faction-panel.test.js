// @vitest-environment jsdom
//
// The GM faction-relation control (issue #1442), mounted on the REAL
// `server.html` markup with the REAL String Table and driven by the exact
// `gm_session` payload shape `gm_action::projection` publishes (see
// tests/gm_undo.rs, which pins that wire shape from the Rust side).
//
// The point of these tests is what the control REFUSES to do: it can name only
// authored factions, it states the direction of an asymmetric relation in
// words, and it decides nothing — the canonical journal does.
import { beforeEach, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  createGmFactionPanel,
  parseGmFactionRoster,
  GM_FACTION_CONFIRMATION,
} from '../../gui/gm-faction-panel.js';
import { GM_ACTION_CONFIRMATION_METADATA } from '../../gui/gm-confirmation.js';
import { buildTable, setTable, t } from '../../gui/strings.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');
const realStrings = buildTable(read('assets/strings/strings.csv'));
const pageSource = read('server.html');

const FACTIONS = [
  { name: 'Alliance', label: 'faction.alliance.display_name', enemies: ['Pirate'] },
  { name: 'Harrow', label: 'faction.harrow.display_name', enemies: [] },
  { name: 'Pirate', enemies: ['Alliance'] },
];

function payload(factions = FACTIONS, entries = []) {
  return {
    paused: false,
    results: [],
    factions,
    journal: { capacity: 4096, total: entries.length, entries },
  };
}

const el = (suffix) => document.getElementById(`gm-faction-${suffix}`);

let sent;
let panel;
function mount({ operator = { id: 'gm-alex' }, confirmAction, submit } = {}) {
  const parsed = new DOMParser().parseFromString(pageSource, 'text/html');
  document.body.replaceChildren(parsed.getElementById('gm-faction-panel'));
  sent = [];
  panel = createGmFactionPanel({
    doc: document,
    t,
    getOperator: () => operator,
    submit: submit || ((request) => { sent.push(request); return true; }),
    confirmAction: confirmAction || ((request) => request.accept()),
    correlation: () => 'undo-corr-1',
    schedule: () => 1,
    cancelSchedule: () => {},
  });
  return panel;
}

beforeEach(() => {
  setTable(realStrings);
});

it('offers only the factions the running world actually loaded', () => {
  mount();
  expect(panel.update(payload())).toBe(true);
  const names = (id) => [...el(id).options].map((option) => option.value);
  expect(names('source')).toEqual(['Alliance', 'Harrow', 'Pirate']);
  expect(names('enemy')).toEqual(['Alliance', 'Harrow', 'Pirate']);
  // A faction with an authored crew-facing label is shown by it; one without
  // keeps its reference name rather than being hidden or renamed here.
  expect([...el('source').options].map((option) => option.textContent))
    .toEqual([t('faction.alliance.display_name'), t('faction.harrow.display_name'), 'Pirate']);
  // There is no free-text entry anywhere on this control.
  expect(document.querySelectorAll('#gm-faction-panel input')).toHaveLength(0);
});

it('states the direction of an asymmetric relation in words, not colour alone', () => {
  mount();
  panel.update(payload());
  el('source').value = 'Alliance';
  el('enemy').value = 'Pirate';
  el('enemy').dispatchEvent(new Event('change'));
  expect(el('relation').textContent).toBe(t('server.gm.faction.is_hostile', {
    faction: t('faction.alliance.display_name'),
    enemy: 'Pirate',
  }));
  expect(el('relation').dataset.hostile).toBe('true');
  expect(el('apply').textContent).toBe(t('server.gm.faction.make_neutral'));

  // The other direction is a different fact, and the control says so.
  el('source').value = 'Harrow';
  el('source').dispatchEvent(new Event('change'));
  expect(el('relation').textContent).toBe(t('server.gm.faction.is_neutral', {
    faction: t('faction.harrow.display_name'),
    enemy: 'Pirate',
  }));
  expect(el('apply').textContent).toBe(t('server.gm.faction.make_hostile'));
});

it('submits an absolute hostility for the chosen ordered pair', () => {
  mount();
  panel.update(payload());
  el('source').value = 'Alliance';
  el('enemy').value = 'Harrow';
  el('enemy').dispatchEvent(new Event('change'));
  el('apply').click();
  expect(sent).toEqual([{
    action: 'set_faction_hostility',
    operator_id: 'gm-alex',
    faction: 'Alliance',
    enemy: 'Harrow',
    hostile: true,
    correlation: 'undo-corr-1',
  }]);
  expect(el('feedback').textContent).toBe(t('server.gm.faction.pending'));
});

it('refuses a pair that names one faction twice without sending anything', () => {
  mount();
  panel.update(payload());
  el('source').value = 'Alliance';
  el('enemy').value = 'Alliance';
  el('enemy').dispatchEvent(new Event('change'));
  expect(el('apply').disabled).toBe(true);
  expect(panel.apply()).toBe(false);
  expect(sent).toEqual([]);
});

it('reports the canonical answer from the one saved journal', () => {
  mount();
  panel.update(payload());
  el('enemy').value = 'Harrow';
  el('enemy').dispatchEvent(new Event('change'));
  el('apply').click();
  panel.update(payload(FACTIONS, [{
    operator_id: 'gm-alex',
    correlation: 'undo-corr-1',
    action_kind: 'faction-relation',
    tick: 12,
    sequence: 1,
    outcome: 'refused',
    reason: 'unknown-faction',
  }]));
  expect(el('feedback').textContent).toBe(t('server.gm.faction.refused', {
    reason: t('server.gm.session.reason.unknown_faction'),
  }));
  expect(panel.state().pending).toBeNull();
});

it('routes the press through the registered confirmation category', () => {
  const requests = [];
  mount({ confirmAction: (request) => { requests.push(request); return true; } });
  panel.update(payload());
  el('enemy').value = 'Harrow';
  el('enemy').dispatchEvent(new Event('change'));
  el('apply').click();
  expect(sent).toEqual([]);
  expect(requests[0].category).toBe(GM_FACTION_CONFIRMATION.category);
  // The category is centrally registered against the typed action, so a build
  // that added the action without registering it would fail here.
  expect(GM_ACTION_CONFIRMATION_METADATA.SetFactionHostility.map((row) => row.confirmationCategory))
    .toEqual(['faction.relation']);
  expect(GM_ACTION_CONFIRMATION_METADATA.UndoGmAction.map((row) => row.confirmationCategory))
    .toEqual(['action.undo']);
  requests[0].onCancel();
  expect(requests[0].accept()).toBe(false);
  expect(sent).toEqual([]);
});

it('keeps the operator selection across a republish and survives a partial payload', () => {
  mount();
  panel.update(payload());
  el('source').value = 'Pirate';
  el('enemy').value = 'Harrow';
  el('enemy').dispatchEvent(new Event('change'));
  panel.update(payload(FACTIONS));
  expect(panel.state().faction).toBe('Pirate');
  expect(panel.state().enemy).toBe('Harrow');
  // A bootstrap payload with no faction roster leaves the last good one up.
  expect(panel.update({ paused: true })).toBe(false);
  expect(panel.state().factions).toHaveLength(3);
});

it('rejects a malformed roster rather than rendering half of one', () => {
  expect(parseGmFactionRoster(payload())).toHaveLength(3);
  expect(parseGmFactionRoster({ factions: [{ name: 'A', enemies: [1] }] })).toBeUndefined();
  expect(parseGmFactionRoster({ factions: [{ enemies: [] }] })).toBeUndefined();
  // Two rows sharing a name would make the two selects ambiguous.
  expect(parseGmFactionRoster({
    factions: [{ name: 'A', enemies: [] }, { name: 'A', enemies: [] }],
  })).toBeUndefined();
  expect(parseGmFactionRoster({ paused: true })).toBeUndefined();
});

it('says plainly when a world has no faction pair to change', () => {
  mount();
  panel.update(payload([{ name: 'Alliance', enemies: [] }]));
  expect(el('empty').hidden).toBe(false);
  expect(el('empty').textContent).toBe(t('server.gm.faction.empty'));
  expect(el('apply').disabled).toBe(true);
});
