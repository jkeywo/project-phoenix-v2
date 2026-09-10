// @vitest-environment jsdom
//
// The saved GM action history (issue #1441), mounted on the REAL `server.html`
// markup with the REAL String Table, driven by the exact `gm_session` payload
// shape `gm_action::projection` publishes (see tests/gm_journal.rs, which pins
// that wire shape from the Rust side).
import { beforeEach, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  createGmJournalPanel,
  filterGmJournalEntries,
  gmJournalEntryKey,
  parseGmJournalProjection,
} from '../../gui/gm-journal-panel.js';
import { buildTable, setTable, t } from '../../gui/strings.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');
const realStrings = buildTable(read('assets/strings/strings.csv'));
const pageSource = read('server.html');

const NAMES = { 'gm-alex': 'Alex', 'gm-sam': 'Sam' };

/** One row exactly as Rust serialises it: absent fields, not null ones. */
function entry(overrides = {}) {
  return {
    operator_id: 'gm-alex',
    correlation: 'act-1',
    action_kind: 'session-pause',
    tick: 10,
    sequence: 1,
    outcome: 'applied',
    ...overrides,
  };
}

function payload(entries, extra = {}) {
  return {
    paused: false,
    results: [],
    journal: { capacity: 4096, total: entries.length, entries, ...extra },
  };
}

const HISTORY = [
  entry(),
  entry({ correlation: 'act-2', tick: 10, sequence: 2, outcome: 'no-op' }),
  entry({
    operator_id: 'gm-sam',
    correlation: 'act-3',
    action_kind: 'world-despawn',
    target: 'courier',
    tick: 12,
    sequence: 3,
    outcome: 'refused',
    reason: 'unknown-entity',
  }),
];

let panel;
beforeEach(() => {
  setTable(realStrings);
  const parsed = new DOMParser().parseFromString(pageSource, 'text/html');
  document.body.replaceChildren(parsed.getElementById('gm-journal'));
  panel = createGmJournalPanel({
    doc: document,
    t,
    getOperatorName: (id) => NAMES[id] || id,
  });
});

const rows = () => [...document.querySelectorAll('.gm-journal-row')];
const rowText = (index) => [...rows()[index].querySelectorAll('span')].map((s) => s.textContent);

it('renders the real applied, no-op and refused facts with public attribution and order', () => {
  expect(panel.update(payload(HISTORY))).toBe(true);
  expect(rows()).toHaveLength(3);
  expect(rowText(0)).toEqual([
    t('server.gm.journal.order', { tick: '10', sequence: '1' }),
    'Alex',
    t('server.gm.journal.kind.session_pause'),
    t('server.gm.journal.outcome.applied'),
  ]);
  expect(rowText(1)[3]).toBe(t('server.gm.journal.outcome.no_op'));
  expect(rowText(2)).toEqual([
    t('server.gm.journal.order', { tick: '12', sequence: '3' }),
    'Sam',
    t('server.gm.journal.action_target', {
      action: t('server.gm.journal.kind.world_despawn'),
      target: 'courier',
    }),
    t('server.gm.journal.outcome.refused'),
  ]);
  // Outcome is words on the row, not colour alone.
  expect(rows().map((row) => row.dataset.outcome)).toEqual(['applied', 'no-op', 'refused']);
  expect(document.getElementById('gm-journal-status').textContent)
    .toBe(t('server.gm.journal.status', { shown: '3', total: '3', capacity: '4096' }));
  expect(document.getElementById('gm-journal-empty').hidden).toBe(true);
});

it('states up front which families this build can undo', () => {
  expect(document.getElementById('gm-journal-inverse-support').textContent)
    .toBe(t('server.gm.journal.inverse_support_some', {
      kinds: [
        t('server.gm.journal.kind.npc_doctrine'),
        t('server.gm.journal.kind.faction_relation'),
        t('server.gm.journal.kind.world_spawn'),
        t('server.gm.journal.kind.world_despawn'),
      ].join(', '),
    }));
});

it('reports the honest total when the durable journal is longer than the published window', () => {
  panel.update(payload(HISTORY, { total: 900 }));
  expect(document.getElementById('gm-journal-status').textContent)
    .toBe(t('server.gm.journal.status', { shown: '3', total: '900', capacity: '4096' }));
});

it('shows the refusal reason and the technical/witnessed split on the selected entry', () => {
  panel.update(payload(HISTORY));
  rows()[2].click();
  const detail = document.getElementById('gm-journal-detail');
  expect(detail.hidden).toBe(false);
  expect(document.getElementById('gm-journal-detail-target').textContent)
    .toBe(t('server.gm.journal.detail_target', { target: 'courier' }));
  expect(document.getElementById('gm-journal-detail-outcome').textContent)
    .toBe(t('server.gm.journal.detail_outcome_reason', {
      outcome: t('server.gm.journal.outcome.refused'),
      reason: t('server.gm.session.reason.unknown_entity'),
    }));
  const inverse = document.getElementById('gm-journal-inverse');
  expect(inverse.textContent).toContain(t('server.gm.inverse.technical_refused'));
  expect(inverse.textContent).toContain(t('server.gm.inverse.witnessed_note'));
  // The family is reversible (issue #1444), but THIS entry was refused and
  // recorded no pair, so no control is offered over it.
  expect(panel.inverseAvailability().supported).toBe(true);
  expect(document.getElementById('gm-journal-undo').hidden).toBe(true);
  expect(inverse.querySelector('button')).toBeNull();
});

it('says an action names no target rather than leaving the line blank', () => {
  panel.update(payload(HISTORY));
  rows()[0].click();
  expect(document.getElementById('gm-journal-detail-target').textContent)
    .toBe(t('server.gm.journal.detail_no_target'));
  expect(document.getElementById('gm-journal-inverse').textContent)
    .toContain(t('server.gm.inverse.state_uncaptured'));
});

it('keeps the selection, the filters and keyboard focus across a republish', () => {
  panel.update(payload(HISTORY));
  document.getElementById('gm-journal-operator-filter').value = 'gm-alex';
  document.getElementById('gm-journal-operator-filter')
    .dispatchEvent(new Event('change', { bubbles: true }));
  rows()[1].click();
  rows()[1].focus();
  const focused = document.activeElement.dataset.key;

  const later = [...HISTORY, entry({ correlation: 'act-4', tick: 20, sequence: 4 })];
  panel.update(payload(later));

  expect(document.getElementById('gm-journal-operator-filter').value).toBe('gm-alex');
  expect(rows().map((row) => row.dataset.correlation)).toEqual(['act-1', 'act-2', 'act-4']);
  expect(panel.state().selected.correlation).toBe('act-2');
  expect(document.activeElement.dataset.key).toBe(focused);
  expect(rows()[1].getAttribute('aria-pressed')).toBe('true');
});

it('keeps a selected entry that the filter hides, and drops one the journal no longer holds', () => {
  panel.update(payload(HISTORY));
  rows()[2].click();
  expect(panel.state().selected.correlation).toBe('act-3');

  document.getElementById('gm-journal-outcome-filter').value = 'applied';
  document.getElementById('gm-journal-outcome-filter')
    .dispatchEvent(new Event('change', { bubbles: true }));
  expect(rows()).toHaveLength(1);
  expect(panel.state().selected.correlation).toBe('act-3');

  // A restore rewinds the journal past that entry: it stops being selectable.
  panel.update(payload(HISTORY.slice(0, 2)));
  expect(panel.state().selected).toBeNull();
  expect(document.getElementById('gm-journal-detail').hidden).toBe(true);
});

it('discards entries that are no longer in the published journal after a restore', () => {
  const saved = HISTORY.slice(0, 2);
  panel.update(payload(saved));
  panel.update(payload([...saved, entry({ correlation: 'act-3', tick: 30, sequence: 3 })]));
  expect(rows().map((row) => row.dataset.correlation)).toEqual(['act-1', 'act-2', 'act-3']);
  // The restored payload carries exactly the save's history; the panel keeps no
  // copy of the discarded entry and shows no abandoned branch.
  panel.update(payload(saved));
  expect(rows().map((row) => row.dataset.correlation)).toEqual(['act-1', 'act-2']);
  expect(panel.state().entries).toHaveLength(2);
  expect(document.body.textContent).not.toContain('act-3');
});

it('clears both filters together and restores the whole history', () => {
  panel.update(payload(HISTORY));
  document.getElementById('gm-journal-outcome-filter').value = 'refused';
  document.getElementById('gm-journal-outcome-filter')
    .dispatchEvent(new Event('change', { bubbles: true }));
  expect(rows()).toHaveLength(1);
  document.getElementById('gm-journal-clear-filters').click();
  expect(rows()).toHaveLength(3);
  expect(document.getElementById('gm-journal-outcome-filter').value).toBe('all');
});

it('moves between rows with the keyboard without changing the selection', () => {
  panel.update(payload(HISTORY));
  rows()[0].focus();
  rows()[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
  expect(document.activeElement.dataset.correlation).toBe('act-2');
  document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));
  expect(document.activeElement.dataset.correlation).toBe('act-3');
  expect(panel.state().selected).toBeNull();
});

it('never interrupts: a republish opens no dialog and moves no panel', () => {
  const alerted = vi.spyOn(window, 'alert').mockImplementation(() => {});
  const confirmed = vi.spyOn(window, 'confirm').mockImplementation(() => true);
  panel.update(payload(HISTORY));
  panel.update(payload([...HISTORY, entry({ correlation: 'act-9', tick: 44, sequence: 4 })]));
  expect(alerted).not.toHaveBeenCalled();
  expect(confirmed).not.toHaveBeenCalled();
  expect(document.getElementById('gm-journal-detail').hidden).toBe(true);
  alerted.mockRestore();
  confirmed.mockRestore();
});

it('ignores a payload that carries no journal instead of blanking the history', () => {
  panel.update(payload(HISTORY));
  // The native host's bootstrap push, and any pre-#1441 payload.
  expect(panel.update({ paused: true })).toBe(false);
  expect(panel.update(JSON.stringify({ paused: true }))).toBe(false);
  expect(rows()).toHaveLength(3);
});

it('rejects a malformed journal rather than showing a half-parsed history', () => {
  panel.update(payload(HISTORY));
  for (const broken of [
    payload([entry({ outcome: 'pending' })]),
    payload([entry({ operator_id: '' })]),
    payload([entry({ tick: -1 })]),
    payload([entry({ sequence: 1.5 })]),
    payload([entry({ correlation: 7 })]),
    payload(HISTORY, { capacity: 'lots' }),
    payload(HISTORY, { total: 1 }),
  ]) {
    expect(panel.update(broken)).toBe(false);
  }
  expect(rows()).toHaveLength(3);
});

it('keeps an unrecognised future action family visible with its wire identity', () => {
  panel.update(payload([entry({ action_kind: 'future-family' })]));
  expect(rowText(0)[2]).toBe(t('server.gm.journal.kind_unknown', { kind: 'future-family' }));
  rows()[0].click();
  expect(document.getElementById('gm-journal-inverse').textContent)
    .toContain(t('server.gm.inverse.unavailable.unknown'));
});

// A refused live restore (issue #1446) leaves a PERMANENT journal row — the
// multi-peer refusal and the concurrent-request refusal both do — so this
// build's own most consequential family must be named here rather than fall
// through the forward-compatibility hatch above and read as unrecognised.
it('names a refused live restore as the restore family, not an unknown one', () => {
  panel.update(payload([entry({
    action_kind: 'live-restore',
    outcome: 'refused',
    reason: 'multiple-simulation-peers',
  })]));
  expect(rowText(0)[2]).toBe(t('server.gm.journal.kind.live_restore'));
  expect(rowText(0)[2]).not.toBe(t('server.gm.journal.kind_unknown', { kind: 'live-restore' }));
  rows()[0].click();
  expect(document.getElementById('gm-journal-detail-outcome').textContent)
    .toBe(t('server.gm.journal.detail_outcome_reason', {
      outcome: t('server.gm.journal.outcome.refused'),
      reason: t('server.gm.session.reason.multiple_simulation_peers'),
    }));
  expect(document.getElementById('gm-journal-inverse').textContent)
    .toContain(t('server.gm.inverse.unavailable.live_restore'));
});

it('parses a JSON string payload and keys rows by their canonical identity', () => {
  const parsed = parseGmJournalProjection(JSON.stringify(payload(HISTORY)));
  expect(parsed.entries).toHaveLength(3);
  expect(parsed.entries[2].reason).toBe('unknown-entity');
  expect(gmJournalEntryKey(parsed.entries[0]))
    .not.toBe(gmJournalEntryKey(parsed.entries[1]));
  expect(filterGmJournalEntries(parsed.entries, { operator: 'gm-sam' })).toHaveLength(1);
  expect(filterGmJournalEntries(parsed.entries, { outcome: 'no-op' })).toHaveLength(1);
  expect(filterGmJournalEntries(parsed.entries, { operator: 'gm-sam', outcome: 'applied' }))
    .toHaveLength(0);
});

it('resets to an empty history at a run boundary', () => {
  panel.update(payload(HISTORY));
  rows()[0].click();
  panel.reset();
  expect(rows()).toHaveLength(0);
  expect(panel.state().selected).toBeNull();
  expect(document.getElementById('gm-journal-empty').hidden).toBe(false);
});


// ── The typed inverse control (issue #1442) ─────────────────────────────────

/** One real applied doctrine row, with the affected pair the reducer records. */
const DOCTRINE = entry({
  correlation: 'act-9',
  action_kind: 'npc-doctrine',
  target: 'courier-1',
  tick: 20,
  sequence: 9,
  outcome: 'applied',
  affected: { 'npc-doctrine': { entity: 'courier-1', before: null, after: 'north' } },
});

function undoPanel({ operator = { id: 'gm-sam' }, submitUndo = () => true, confirmAction } = {}) {
  const parsed = new DOMParser().parseFromString(pageSource, 'text/html');
  document.body.replaceChildren(parsed.getElementById('gm-journal'));
  return createGmJournalPanel({
    doc: document,
    t,
    getOperatorName: (id) => NAMES[id] || id,
    getOperator: () => operator,
    submitUndo,
    confirmAction: confirmAction || ((request) => request.accept()),
    schedule: () => 1,
    cancelSchedule: () => {},
  });
}

const undoButton = () => document.getElementById('gm-journal-undo');

it('offers Undo only for an applied entry whose family and recorded pair support one', () => {
  const panel2 = undoPanel();
  panel2.update(payload([
    DOCTRINE,
    // Applied, but a family with no typed inverse.
    entry({ correlation: 'act-10', action_kind: 'direct-effect', sequence: 10 }),
    // Right family, but nothing changed, so no pair was recorded.
    entry({ correlation: 'act-11', action_kind: 'npc-doctrine', sequence: 11, outcome: 'no-op' }),
    // Right family and applied, but another GM already reversed it.
    entry({
      correlation: 'act-12',
      action_kind: 'npc-doctrine',
      sequence: 12,
      affected: { 'npc-doctrine': { entity: 'courier-2', before: 'east', after: 'north' } },
      inverted: true,
    }),
  ]));
  const offered = [];
  for (const row of rows()) {
    row.click();
    offered.push([row.dataset.correlation, !undoButton().hidden]);
  }
  expect(offered).toEqual([
    ['act-9', true],
    ['act-10', false],
    ['act-11', false],
    ['act-12', false],
  ]);
  panel2.destroy();
});

it('submits the recorded pair verbatim with the original public identity', () => {
  const sent = [];
  const panel2 = undoPanel({ submitUndo: (request) => { sent.push(request); return true; } });
  panel2.update(payload([DOCTRINE]));
  rows()[0].click();
  undoButton().click();
  expect(sent).toHaveLength(1);
  const request = sent[0];
  expect(request.action).toBe('undo_gm_action');
  expect(request.operator_id).toBe('gm-sam');
  expect(request.original).toBe('act-9');
  expect(request.original_operator).toBe('gm-alex');
  expect(request.original_sequence).toBe(9);
  // Byte-for-byte what the projection published: the reducer compares this
  // against the canonical fact to refuse a stale reading.
  expect(request.expected).toEqual(DOCTRINE.affected);
  expect(typeof request.correlation).toBe('string');
  expect(document.getElementById('gm-journal-undo-feedback').textContent)
    .toBe(t('server.gm.journal.undo_pending'));
  panel2.destroy();
});

it('reports the canonical answer from the journal itself, not a second feed', () => {
  let sent = null;
  const panel2 = undoPanel({ submitUndo: (request) => { sent = request; return true; } });
  panel2.update(payload([DOCTRINE]));
  rows()[0].click();
  undoButton().click();
  // The refusal arrives as an ordinary row of the ONE journal.
  panel2.update(payload([
    { ...DOCTRINE },
    entry({
      operator_id: 'gm-sam',
      correlation: sent.correlation,
      action_kind: 'action-undo',
      sequence: 10,
      outcome: 'refused',
      reason: 'affected-state-changed',
    }),
  ]));
  expect(document.getElementById('gm-journal-undo-feedback').textContent)
    .toBe(t('server.gm.journal.undo_refused'));
  expect(panel2.pending()).toBeNull();
  // And the reason is readable on that entry, in words.
  rows()[1].click();
  expect(document.getElementById('gm-journal-detail-outcome').textContent)
    .toContain(t('server.gm.session.reason.affected_state_changed'));
  panel2.destroy();
});

it('routes the press through the configurable confirmation and sends nothing when cancelled', () => {
  const requests = [];
  let sent = 0;
  const panel2 = undoPanel({
    submitUndo: () => { sent += 1; return true; },
    confirmAction: (request) => { requests.push(request); return true; },
  });
  panel2.update(payload([DOCTRINE]));
  rows()[0].click();
  undoButton().click();
  expect(sent).toBe(0);
  expect(requests[0].category).toBe('action.undo');
  expect(requests[0].defaultMode).toBe('confirm-preview');
  // The preview a `confirm-preview` policy renders states the thing a restored
  // value cannot do.
  expect(requests[0].preview()).toContain(t('server.gm.inverse.witnessed_note'));
  requests[0].onCancel();
  expect(requests[0].accept()).toBe(false);
  expect(sent).toBe(0);
  panel2.destroy();
});

it('shows the same consequence sentences with no dialog at all', () => {
  // `immediate` is a legitimate private policy. The detail region still carries
  // the whole explanation, so configuring the step away never hides it
  // (PRD #1418 story 30).
  const panel2 = undoPanel({ confirmAction: (request) => request.accept() });
  panel2.update(payload([DOCTRINE]));
  rows()[0].click();
  const detail = document.getElementById('gm-journal-detail').textContent;
  expect(detail).toContain(t('server.gm.inverse.witnessed_note'));
  expect(detail).toContain(t('server.gm.inverse.doctrine_authored'));
  expect(detail).toContain(t('server.gm.inverse.doctrine_value', { doctrine: 'north' }));
  expect(detail).toContain(t('server.gm.inverse.available'));
  panel2.destroy();
});

it('keeps the selected entry, its Undo control and focus across a republish', () => {
  const panel2 = undoPanel();
  panel2.update(payload([DOCTRINE, entry({ correlation: 'act-13', sequence: 13 })]));
  rows()[0].click();
  rows()[0].focus();
  expect(document.activeElement.dataset.correlation).toBe('act-9');
  panel2.update(payload([
    DOCTRINE,
    entry({ correlation: 'act-13', sequence: 13 }),
    entry({ correlation: 'act-14', sequence: 14 }),
  ]));
  expect(panel2.state().selected.correlation).toBe('act-9');
  expect(document.activeElement.dataset.correlation).toBe('act-9');
  expect(undoButton().hidden).toBe(false);
  panel2.destroy();
});

it('reads an unrecognised future affected field without claiming to describe it', () => {
  const panel2 = undoPanel();
  expect(panel2.update(payload([
    entry({
      correlation: 'act-15',
      action_kind: 'npc-doctrine',
      sequence: 15,
      affected: { 'something-new': { value: 1 } },
    }),
  ]))).toBe(true);
  rows()[0].click();
  const values = [...document.querySelectorAll('#gm-journal-inverse dd')]
    .map((node) => node.textContent);
  expect(values[0]).toBe(t('server.gm.inverse.state_uncaptured'));
  panel2.destroy();
});

it('rejects a malformed undo reference rather than rendering half a history', () => {
  const panel2 = undoPanel();
  expect(panel2.update(payload([
    entry({ correlation: 'act-16', undo_of: { operator_id: 'gm-alex' } }),
  ]))).toBe(false);
  panel2.destroy();
});

// ── The placement inverse and its two-second window (issue #1443) ────────────

/** One real applied placement row, with the pair and the clock Rust publishes. */
function placement(overrides = {}, exposure = { exposed_ms: 400, limit_ms: 2000, latched: false }) {
  return entry({
    correlation: 'act-20',
    action_kind: 'world-spawn',
    target: 'courier',
    tick: 40,
    sequence: 20,
    outcome: 'applied',
    affected: { 'spawned-entity': { name: 'gm_courier_20', before: false, after: true } },
    ...(exposure == null ? {} : { spawn_exposure: exposure }),
    ...overrides,
  });
}

it('offers Undo for a placement only while its window is open', () => {
  const panel2 = undoPanel();
  panel2.update(payload([
    placement(),
    // The same placement two seconds later: the offer is withdrawn for good.
    placement(
      { correlation: 'act-21', sequence: 21 },
      { exposed_ms: 2000, limit_ms: 2000, latched: true },
    ),
    // A session that reports no clock at all cannot honestly offer the control.
    placement({ correlation: 'act-22', sequence: 22 }, null),
  ]));
  const offered = [];
  for (const row of rows()) {
    row.click();
    offered.push([row.dataset.correlation, !undoButton().hidden]);
  }
  expect(offered).toEqual([['act-20', true], ['act-21', false], ['act-22', false]]);
  panel2.destroy();
});

it('shows the exposure clock and the reason on the selected placement', () => {
  const panel2 = undoPanel();
  panel2.update(payload([placement()]));
  rows()[0].click();
  const inverse = document.getElementById('gm-journal-inverse');
  expect(inverse.textContent)
    .toContain(t('server.gm.inverse.exposure_remaining', { elapsed: '0.4', limit: '2.0' }));
  expect(inverse.textContent).toContain(t('server.gm.inverse.subject_placement', {
    name: 'gm_courier_20',
  }));
  // Consequence information is present with no dialog opened at all, which is
  // what a GM whose confirmation policy is `immediate` has to be able to read.
  expect(inverse.textContent).toContain(t('server.gm.inverse.witnessed_note'));

  panel2.update(payload([
    placement({}, { exposed_ms: 2000, limit_ms: 2000, latched: true }),
  ]));
  const shut = document.getElementById('gm-journal-inverse');
  expect(shut.querySelector('.gm-inverse-eligibility').dataset.status).toBe('expired');
  // Words, not colour alone.
  expect(shut.textContent).toContain(t('server.gm.inverse.unavailable.sensor_exposure'));
  panel2.destroy();
});

it('submits the placement pair verbatim and never echoes the clock back', () => {
  let sent = null;
  const panel2 = undoPanel({ submitUndo: (request) => { sent = request; return true; } });
  panel2.update(payload([placement()]));
  rows()[0].click();
  undoButton().click();
  expect(sent.action).toBe('undo_gm_action');
  expect(sent.original).toBe('act-20');
  expect(sent.original_operator).toBe('gm-alex');
  expect(sent.original_sequence).toBe(20);
  expect(sent.expected).toEqual(placement().affected);
  // Exposure is a fact about the world, which the reducer reads for itself.
  expect(Object.keys(sent)).not.toContain('spawn_exposure');
  expect(Object.keys(sent)).not.toContain('exposure');
  panel2.destroy();
});

it('reports the canonical exposure refusal in words when the window shut first', () => {
  const panel2 = undoPanel({ submitUndo: () => true });
  panel2.update(payload([placement()]));
  rows()[0].click();
  undoButton().click();
  // The answer arrives as an ordinary journal row, as every GM answer does.
  panel2.update(payload([
    placement({ inverted: false }),
    entry({
      operator_id: 'gm-sam',
      correlation: panel2.pending() ? panel2.pending().correlation : 'x',
      action_kind: 'action-undo',
      sequence: 21,
      tick: 41,
      outcome: 'refused',
      reason: 'sensor-exposure-elapsed',
    }),
  ]));
  expect(document.getElementById('gm-journal-undo-feedback').textContent)
    .toBe(t('server.gm.journal.undo_refused'));
  panel2.destroy();
});

it('rejects a malformed exposure status rather than rendering half a row', () => {
  const panel2 = undoPanel();
  expect(panel2.update(payload([
    placement({}, { exposed_ms: 400, limit_ms: 2000 }),
  ]))).toBe(false);
  expect(parseGmJournalProjection(payload([placement({}, { exposed_ms: '400' })]))).toBeUndefined();
  panel2.destroy();
});

// ── The #1418 usability contract this feature ships with ────────────────────

it('keeps the placement window readable and operable at 200% text', () => {
  // The scale the presentation profile sets. jsdom lays out nothing, so what
  // is under test here is the BEHAVIOUR the contract asks for at that scale:
  // status in words, no pop-up, and a control that stays where the operator
  // put their hands while the clock republishes underneath them.
  document.documentElement.style.setProperty('--a11y-text-scale', '2');
  const dialogs = [];
  const panel2 = undoPanel({
    confirmAction: (request) => { dialogs.push(request); return request.accept(); },
  });
  panel2.update(payload([placement(), entry({ correlation: 'act-30', sequence: 30 })]));
  rows()[0].click();
  rows()[0].focus();
  expect(document.activeElement.dataset.correlation).toBe('act-20');

  // The clock advances. Routine attention: no dialog, no panel switch, and
  // nothing moves under the operator's hands (PRD #1418 stories 23/26/31).
  for (const exposed of [600, 900, 1400, 1900]) {
    panel2.update(payload([
      placement({}, { exposed_ms: exposed, limit_ms: 2000, latched: false }),
      entry({ correlation: 'act-30', sequence: 30 }),
    ]));
    expect(document.activeElement.dataset.correlation).toBe('act-20');
    expect(panel2.state().selected.correlation).toBe('act-20');
    expect(undoButton().hidden).toBe(false);
  }
  expect(dialogs).toHaveLength(0);
  expect(document.querySelector('dialog[open]')).toBeNull();

  // Status is carried by sentences, not only by `data-*` or colour: strip
  // every data attribute and the meaning survives.
  const inverse = document.getElementById('gm-journal-inverse');
  for (const node of inverse.querySelectorAll('*')) {
    for (const name of [...node.getAttributeNames()]) {
      if (name.startsWith('data-')) node.removeAttribute(name);
    }
  }
  expect(inverse.textContent)
    .toContain(t('server.gm.inverse.exposure_remaining', { elapsed: '1.9', limit: '2.0' }));
  expect(inverse.textContent).toContain(t('server.gm.inverse.available'));

  // The window shuts. The control goes, the sentence explains why, the
  // selection and focus still belong to the operator, and still no dialog.
  panel2.update(payload([
    placement({}, { exposed_ms: 2000, limit_ms: 2000, latched: true }),
    entry({ correlation: 'act-30', sequence: 30 }),
  ]));
  expect(document.activeElement.dataset.correlation).toBe('act-20');
  expect(panel2.state().selected.correlation).toBe('act-20');
  expect(undoButton().hidden).toBe(true);
  expect(document.getElementById('gm-journal-inverse').textContent)
    .toContain(t('server.gm.inverse.unavailable.sensor_exposure'));
  expect(dialogs).toHaveLength(0);
  document.documentElement.style.removeProperty('--a11y-text-scale');
  panel2.destroy();
});

it('gives the placement Undo control a reachable name and the shipped hit floor', () => {
  const css = read('gui/gm-workspace.css');
  // The control is sized in tokens rather than pixels, so it grows with the
  // text rather than clipping it at 200% (PRD #1418 stories 1/2).
  expect(css).toMatch(/#gm-journal-undo[\s\S]{0,200}min-height: var\(--control-hit-min\)/);
  expect(css).toMatch(/#gm-journal-undo[\s\S]{0,200}white-space: normal/);
  // The exposure row reflows with the rest of the description list, and its
  // shut state is marked by more than a hue.
  expect(css).toContain('.gm-inverse-exposure[data-latched="true"]');

  const panel2 = undoPanel();
  panel2.update(payload([placement()]));
  rows()[0].click();
  expect(undoButton().getAttribute('aria-label')).toBe(t('server.gm.journal.undo_entry', {
    action: t('server.gm.journal.action_target', {
      action: t('server.gm.journal.kind.world_spawn'),
      target: 'courier',
    }),
    operator: 'Alex',
    order: t('server.gm.journal.order', { tick: '40', sequence: '20' }),
  }));
  panel2.destroy();
});

// ── Reversing an allowed removal (issue #1444) ───────────────────────────────

/** An applied GM removal, exactly as Rust serialises it. */
const REMOVAL = entry({
  correlation: 'act-30',
  action_kind: 'world-despawn',
  target: 'raider-7',
  tick: 30,
  sequence: 30,
  outcome: 'applied',
  affected: { 'entity-presence': { entity: 'raider-7', before: true, after: false } },
});

it('describes a removal as a presence pair and offers its Undo', () => {
  const panel2 = undoPanel();
  expect(panel2.update(payload([REMOVAL]))).toBe(true);
  rows()[0].click();
  const values = [...document.querySelectorAll('#gm-journal-inverse dd')]
    .map((node) => node.textContent);
  expect(values[0]).toBe(t('server.gm.inverse.subject_presence', { entity: 'raider-7' }));
  expect(values[1]).toBe(t('server.gm.inverse.presence_present'));
  expect(values[2]).toBe(t('server.gm.inverse.presence_absent'));
  expect(undoButton().hidden).toBe(false);
  panel2.destroy();
});

it('names the released links a restore does not re-make, beside the standing witness note', () => {
  const panel2 = undoPanel();
  panel2.update(payload([REMOVAL]));
  rows()[0].click();
  const witnessed = [...document.querySelectorAll('#gm-journal-inverse dd')][4].textContent;
  // Both halves: crews remember, AND the concrete links that were released.
  expect(witnessed).toContain(t('server.gm.inverse.witnessed_note'));
  expect(witnessed).toContain(t('server.gm.inverse.witnessed_removal'));
  panel2.destroy();
});

it('hides the Undo control when the run no longer holds the capture a rebuild needs', () => {
  const panel2 = undoPanel();
  panel2.update(payload([{ ...REMOVAL, capture_lost: true }]));
  rows()[0].click();
  expect(undoButton().hidden).toBe(true);
  // The family is still described as reversible; this ENTRY is the exception,
  // and nothing on screen offers a control the reducer would refuse.
  expect(panel2.inverseAvailability().supported).toBe(true);
  panel2.destroy();
});

it('echoes the recorded presence pair back verbatim when a GM asks to undo a removal', () => {
  const submitted = [];
  const panel2 = undoPanel({ submitUndo: (request) => { submitted.push(request); return true; } });
  panel2.update(payload([REMOVAL]));
  rows()[0].click();
  undoButton().click();
  expect(submitted).toHaveLength(1);
  expect(submitted[0].action).toBe('undo_gm_action');
  expect(submitted[0].original).toBe('act-30');
  expect(submitted[0].original_operator).toBe('gm-alex');
  expect(submitted[0].original_sequence).toBe(30);
  expect(submitted[0].expected).toEqual(REMOVAL.affected);
  panel2.destroy();
});

it('states the witnessed consequences in the confirmation preview under every policy', () => {
  const previews = [];
  const panel2 = undoPanel({
    confirmAction: (request) => { previews.push(request.preview()); request.accept(); },
  });
  panel2.update(payload([REMOVAL]));
  rows()[0].click();
  undoButton().click();
  expect(previews).toHaveLength(1);
  // `confirm-preview`, `confirm` and `immediate` all route through the same
  // controller; what the sentence SAYS is what this pins.
  expect(previews[0]).toContain(t('server.gm.inverse.witnessed_note'));
  panel2.destroy();
});

it('rejects a malformed capture_lost rather than rendering a row it cannot trust', () => {
  const panel2 = undoPanel();
  expect(panel2.update(payload([{ ...REMOVAL, capture_lost: 'yes' }]))).toBe(false);
  panel2.destroy();
});

it('keeps the removal Undo readable and operable at 200% text', () => {
  const panel2 = undoPanel();
  document.documentElement.style.setProperty('--a11y-text-scale', '2');
  panel2.update(payload([REMOVAL]));
  rows()[0].click();
  // Status is words, never colour alone, and the accessible name names the
  // action, the operator and the order rather than saying only "Undo".
  expect(rows()[0].dataset.outcome).toBe('applied');
  expect(undoButton().hidden).toBe(false);
  expect(undoButton().getAttribute('aria-label')).toBe(t('server.gm.journal.undo_entry', {
    action: t('server.gm.journal.action_target', {
      action: t('server.gm.journal.kind.world_despawn'),
      target: 'raider-7',
    }),
    operator: 'Alex',
    order: t('server.gm.journal.order', { tick: '30', sequence: '30' }),
  }));
  // Keyboard reaches the control, and the selection it acts on is stable while
  // the projection republishes underneath it.
  rows()[0].focus();
  expect(document.activeElement.dataset.correlation).toBe('act-30');
  panel2.update(payload([REMOVAL, entry({ correlation: 'act-31', sequence: 31 })]));
  expect(panel2.state().selected.correlation).toBe('act-30');
  expect(document.activeElement.dataset.correlation).toBe('act-30');
  expect(undoButton().hidden).toBe(false);
  // Routine attention never opens a pop-up: the feedback line is a live region.
  const feedback = document.getElementById('gm-journal-undo-feedback');
  expect(feedback.getAttribute('role')).toBe('status');
  expect(feedback.getAttribute('aria-live')).toBe('polite');
  document.documentElement.style.removeProperty('--a11y-text-scale');
  panel2.destroy();
});
