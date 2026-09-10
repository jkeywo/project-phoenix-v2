// @vitest-environment jsdom
//
// Named GM checkpoints (issue #1445), mounted on the REAL `server.html` markup
// with the REAL String Table, driven by the exact catalogue-row shape
// `bridge::save_slot_js` publishes — including the `preflight` object whose
// verdict `src/gm_checkpoint.rs` decides (pinned from the Rust side in
// `src/gm_checkpoint.rs`'s own tests and `tests/gm_checkpoint.rs`).
import { beforeEach, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  confirmedCheckpoint,
  createGmCheckpointPanel,
  normalizeCheckpointRow,
} from '../../gui/gm-checkpoint-panel.js';
import {
  candidateBlockText,
  normalizeCandidatePreflight,
} from '../../gui/gm-checkpoint-preflight.js';
import { captureAvailableForPhase } from '../../gui/save-slots.js';
import { applyToDom, buildTable, setTable, t } from '../../gui/strings.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');
const realStrings = buildTable(read('assets/strings/strings.csv'));
const pageSource = read('server.html');

const WORLD = 'assets/worlds/duel.toml';

/** One catalogue row exactly as the wasm boundary assembles it. */
function row(overrides = {}) {
  return {
    slot_id: 'slot-a',
    kind: 'manual',
    display_name: 'Before the ambush',
    scenario: WORLD,
    selected_ship: 'assets/entities/alliance_cruiser.toml',
    capture_tick: '1200',
    compatible: true,
    startable: true,
    refusal_kind: null,
    refusal: null,
    metadata: 'present',
    metadata_error: null,
    preflight: { eligible: true, blocks: [] },
    ...overrides,
  };
}

const INCOMPATIBLE = row({
  slot_id: 'slot-b',
  display_name: 'Another world',
  scenario: 'assets/worlds/combat_test.toml',
  capture_tick: '900',
  preflight: {
    eligible: false,
    blocks: [
      { kind: 'scenario-differs', candidate: 'assets/worlds/combat_test.toml', live: WORLD },
      { kind: 'missing-ship', slot: 9, stations: ['helm', 'tactical'] },
    ],
  },
});

let api;
let panel;
let listed;
let created;

function mount({ rows = [], capture = true, now = () => new Date('2026-09-10T14:05:00Z') } = {}) {
  listed = rows;
  created = [];
  api = {
    list: vi.fn(() => listed),
    create: vi.fn((name) => {
      created.push(name);
      return 'slot-new';
    }),
  };
  panel = createGmCheckpointPanel({
    doc: document,
    t,
    api,
    canCapture: () => capture,
    now,
  });
  return panel.ready;
}

beforeEach(() => {
  setTable(realStrings);
  const parsed = new DOMParser().parseFromString(pageSource, 'text/html');
  document.body.replaceChildren(parsed.getElementById('gm-checkpoint'));
  // The page's own localisation pass, so the static copy under test is the copy
  // a real GM reads rather than one this test typed in.
  applyToDom(document);
});

const rowsIn = () => [...document.querySelectorAll('.gm-checkpoint-row')];
const status = () => document.getElementById('gm-checkpoint-status');
const nameField = () => document.getElementById('gm-checkpoint-name');
const bookmarkButton = () => document.getElementById('gm-checkpoint-bookmark');

// ── Named capture ──────────────────────────────────────────────────────────

it('shows a capture tick and time only after the checkpoint really reaches the catalogue', async () => {
  await mount();
  nameField().value = 'Before the ambush';
  bookmarkButton().click();

  // Requested, not captured: no tick, no time, no past tense.
  expect(created).toEqual(['Before the ambush']);
  expect(status().textContent)
    .toBe(t('server.gm.checkpoint.pending', { name: 'Before the ambush' }));
  expect(status().dataset.tone).toBe('pending');
  expect(status().textContent).not.toMatch(/1200/);

  // The Store reports success AND the row comes back carrying its own tick.
  listed = [row({ slot_id: 'slot-new', display_name: 'Before the ambush' })];
  await expect(panel.reportOutcome(true, 'saved at tick 1200')).resolves.toBe(true);
  expect(status().dataset.tone).toBe('ok');
  expect(status().textContent).toBe(t('server.gm.checkpoint.confirmed', {
    name: 'Before the ambush',
    tick: '1200',
    time: new Date('2026-09-10T14:05:00Z').toLocaleTimeString(),
  }));
  // ...and it is the row now selected, so the GM can inspect what they made.
  expect(panel.state().selected.slotId).toBe('slot-new');
  // The name field is cleared, so the next bookmark is not silently a retype.
  expect(nameField().value).toBe('');
});

it('refuses to paint a successful bookmark when the write failed', async () => {
  await mount();
  nameField().value = 'Doomed';
  bookmarkButton().click();
  listed = [];
  await expect(panel.reportOutcome(false, 'the save could not be written: quota exceeded'))
    .resolves.toBe(false);
  expect(status().dataset.tone).toBe('failed');
  expect(status().textContent).toBe(t('server.gm.checkpoint.failed', {
    detail: 'the save could not be written: quota exceeded',
  }));
  expect(status().textContent).not.toMatch(/tick/i);
  expect(panel.state().selected).toBe(null);
});

it('treats a success that left nothing in the catalogue as a failure', async () => {
  await mount();
  nameField().value = 'Phantom';
  bookmarkButton().click();
  // Storage said yes; the catalogue disagrees. A checkpoint a GM cannot select
  // afterwards is not a checkpoint.
  listed = [row({ slot_id: 'someone-else' })];
  await expect(panel.reportOutcome(true, 'saved at tick 1200')).resolves.toBe(false);
  expect(status().textContent).toBe(t('server.gm.checkpoint.failed', {
    detail: t('server.gm.checkpoint.unconfirmed'),
  }));
});

it('treats a row with no readable capture tick as unconfirmed', async () => {
  await mount();
  nameField().value = 'Damaged';
  bookmarkButton().click();
  listed = [row({ slot_id: 'slot-new', capture_tick: null, scenario: null })];
  await expect(panel.reportOutcome(true, 'saved')).resolves.toBe(false);
  expect(confirmedCheckpoint(listed, 'slot-new')).toBe(null);
});

it('reports a dropped fixed-tick boundary rather than leaving a bookmark pending forever', async () => {
  await mount();
  nameField().value = 'Interrupted';
  bookmarkButton().click();
  expect(panel.state().pending).toBe(true);
  panel.setPhase('GameOver');
  expect(panel.state().pending).toBe(false);
  expect(status().textContent).toBe(t('server.gm.checkpoint.failed', {
    detail: t('server.gm.checkpoint.capture_dropped'),
  }));
});

it('will not request a nameless checkpoint and says why', async () => {
  await mount();
  nameField().value = '   ';
  bookmarkButton().click();
  expect(api.create).not.toHaveBeenCalled();
  expect(status().textContent).toBe(t('server.gm.checkpoint.name_required'));
});

it('disables capture outside the only phase that admits one, with a readable hint', async () => {
  await mount({ capture: false });
  expect(bookmarkButton().disabled).toBe(true);
  expect(nameField().disabled).toBe(true);
  const hint = document.getElementById('gm-checkpoint-hint');
  expect(hint.hidden).toBe(false);
  expect(hint.textContent).toBe(t('server.gm.checkpoint.unavailable'));
  panel.setPhase('InProgress');
  expect(bookmarkButton().disabled).toBe(false);
  expect(hint.hidden).toBe(true);
});

it('admits a capture in exactly the phases the save catalogue admits one in', async () => {
  // A bookmark IS the catalogue's manual capture, so the two surfaces must not
  // answer "may I capture now?" differently: a Bookmark control offered in a
  // phase the catalogue refuses would ask for a capture that can never land.
  // Driven through the real panel rather than by comparing two predicates.
  await mount({ capture: false });
  for (const phase of ['Lobby', 'Loading', 'InProgress', 'Paused', 'GameOver', '']) {
    panel.setPhase(phase);
    expect({ phase, disabled: bookmarkButton().disabled })
      .toEqual({ phase, disabled: !captureAvailableForPhase(phase) });
  }
});

// ── Candidate preflight ────────────────────────────────────────────────────

it('renders the shared verdict and every readable reason Rust reported', async () => {
  await mount({ rows: [row(), INCOMPATIBLE] });
  expect(rowsIn()).toHaveLength(2);
  expect(rowsIn().map((button) => button.dataset.eligible)).toEqual(['true', 'false']);
  // Words, not colour alone.
  expect(rowsIn()[0].querySelector('.gm-checkpoint-verdict').textContent)
    .toBe(t('server.gm.checkpoint.eligible'));
  expect(rowsIn()[1].querySelector('.gm-checkpoint-verdict').textContent)
    .toBe(t('server.gm.checkpoint.ineligible'));
  expect(document.getElementById('gm-checkpoint-summary').textContent)
    .toBe(t('server.gm.checkpoint.summary', { eligible: '1', total: '2' }));

  panel.select('slot-b');
  const reasons = [...document.querySelectorAll('#gm-checkpoint-detail-preflight li')]
    .map((item) => item.textContent);
  expect(reasons).toEqual([
    t('server.gm.checkpoint.block.scenario_differs', {
      candidate: 'assets/worlds/combat_test.toml',
      live: WORLD,
    }),
    t('server.gm.checkpoint.block.missing_ship', { slot: '9', stations: 'helm, tactical' }),
  ]);
  expect(document.getElementById('gm-checkpoint-detail-preflight').dataset.eligible).toBe('false');
});

it('names the hull on both sides when a live slot is flying something else', () => {
  const block = normalizeCandidatePreflight({
    eligible: false,
    blocks: [{
      kind: 'hull-differs',
      slot: 2,
      candidate: 'assets/entities/alliance_cruiser.toml',
      live: 'assets/entities/alliance_destroyer.toml',
      stations: ['helm'],
    }],
  }).blocks[0];
  expect(candidateBlockText(block, t)).toBe(t('server.gm.checkpoint.block.hull_differs', {
    slot: '2',
    candidate: 'assets/entities/alliance_cruiser.toml',
    live: 'assets/entities/alliance_destroyer.toml',
    stations: 'helm',
  }));
});

it('spells out a roster row that names no hull rather than printing nothing', () => {
  const block = normalizeCandidatePreflight({
    eligible: false,
    blocks: [{ kind: 'hull-differs', slot: 0, candidate: null, live: 'a.toml' }],
  }).blocks[0];
  expect(candidateBlockText(block, t))
    .toBe(t('server.gm.checkpoint.block.hull_differs_uncrewed', {
      slot: '0',
      candidate: t('server.gm.checkpoint.hull_unspecified'),
      live: 'a.toml',
    }));
});

it('says a hull could not be determined rather than implying it matched', () => {
  const crewed = normalizeCandidatePreflight({
    eligible: false,
    blocks: [{ kind: 'hull-unknown', slot: 1, stations: ['helm'] }],
  });
  expect(crewed.eligible).toBe(false);
  expect(candidateBlockText(crewed.blocks[0], t))
    .toBe(t('server.gm.checkpoint.block.hull_unknown', { slot: '1', stations: 'helm' }));

  const uncrewed = normalizeCandidatePreflight({
    eligible: false,
    blocks: [{ kind: 'hull-unknown', slot: 0 }],
  });
  expect(candidateBlockText(uncrewed.blocks[0], t))
    .toBe(t('server.gm.checkpoint.block.hull_unknown_uncrewed', { slot: '0', stations: '' }));
});

it('keeps a refusal this build does not know instead of reading it as eligible', () => {
  const answer = normalizeCandidatePreflight({
    eligible: true,
    blocks: [{ kind: 'some-future-reason' }],
  });
  // A verdict that disagrees with its own reasons is not a green light.
  expect(answer.eligible).toBe(false);
  expect(candidateBlockText(answer.blocks[0], t))
    .toBe(t('server.gm.checkpoint.block.unknown', { kind: 'some-future-reason' }));
});

it('carries no verdict at all for a catalogue with no live session', () => {
  expect(normalizeCandidatePreflight(undefined)).toBe(null);
  expect(normalizeCheckpointRow(row({ preflight: null })).preflight).toBe(null);
});

// ── Privacy and scope ──────────────────────────────────────────────────────

it('reads only the catalogue it was handed, and offers no way to reach another peer', async () => {
  await mount({ rows: [row()] });
  await panel.refresh();
  // Every read went through the one injected local API; the panel has no
  // operator/peer parameter and exposes no control that could take one.
  expect(api.list).toHaveBeenCalledTimes(2);
  expect(api.list.mock.calls.every((call) => call.length === 0)).toBe(true);
  expect(Object.keys(panel)).not.toContain('listForOperator');
  // ...and nothing here restores, renames, exports or deletes: the panel says
  // in the page that it only reports compatibility.
  expect(document.querySelectorAll('#gm-checkpoint button')).toHaveLength(
    1 + rowsIn().length,
  );
  expect(document.getElementById('gm-checkpoint-restore-note').textContent)
    .toBe(t('server.gm.checkpoint.restore_note'));
});

it('states in the panel that the catalogue is private to this browser', async () => {
  await mount();
  expect(document.getElementById('gm-checkpoint-intro').textContent)
    .toBe(t('server.gm.checkpoint.intro'));
  expect(t('server.gm.checkpoint.intro')).toMatch(/only/i);
});

it('reports a catalogue read failure instead of silently showing an empty list', async () => {
  await mount();
  api.list = vi.fn(() => { throw new Error('storage is unavailable'); });
  await expect(panel.refresh()).resolves.toBe(false);
  expect(status().textContent)
    .toBe(t('server.gm.checkpoint.list_failed', { detail: 'storage is unavailable' }));
  expect(document.getElementById('gm-checkpoint-empty').hidden).toBe(false);
});

// ── #1418 usability contract ───────────────────────────────────────────────

it('keeps the selection and the keyboard focus across a refresh', async () => {
  await mount({ rows: [row(), INCOMPATIBLE] });
  panel.select('slot-b');
  rowsIn()[1].focus();
  expect(document.activeElement.dataset.checkpointSlotId).toBe('slot-b');
  await panel.refresh();
  expect(document.activeElement.dataset.checkpointSlotId).toBe('slot-b');
  expect(panel.state().selected.slotId).toBe('slot-b');
  expect(rowsIn()[1].getAttribute('aria-pressed')).toBe('true');
});

it('preserves a half-typed checkpoint name across a refresh', async () => {
  await mount({ rows: [row()] });
  nameField().value = 'Half typed';
  await panel.refresh();
  expect(nameField().value).toBe('Half typed');
});

it('moves between candidates with the arrow, Home and End keys', async () => {
  await mount({ rows: [row(), INCOMPATIBLE, row({ slot_id: 'slot-c' })] });
  rowsIn()[0].focus();
  const press = (key) => rowsIn()
    .find((button) => button === document.activeElement)
    .dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  press('ArrowDown');
  expect(document.activeElement.dataset.checkpointSlotId).toBe('slot-b');
  press('End');
  expect(document.activeElement.dataset.checkpointSlotId).toBe('slot-c');
  press('Home');
  expect(document.activeElement.dataset.checkpointSlotId).toBe('slot-a');
  press('ArrowUp');
  expect(document.activeElement.dataset.checkpointSlotId).toBe('slot-a');
});

it('opens no dialog and moves no panel while a bookmark is settled', async () => {
  await mount({ rows: [row()] });
  const before = document.getElementById('gm-checkpoint').outerHTML.length;
  nameField().value = 'Quiet';
  bookmarkButton().click();
  listed = [row({ slot_id: 'slot-new', display_name: 'Quiet' })];
  await panel.reportOutcome(true, 'saved at tick 1200');
  expect(document.querySelectorAll('[role="dialog"], [role="alertdialog"]')).toHaveLength(0);
  expect(document.getElementById('gm-checkpoint').hidden).toBe(false);
  expect(before).toBeGreaterThan(0);
  // The status is a persistent, readable line, not a transient alert.
  expect(status().getAttribute('role')).toBe('status');
  expect(status().getAttribute('aria-live')).toBe('polite');
});

it('gives every candidate row a spoken name carrying its verdict', async () => {
  await mount({ rows: [INCOMPATIBLE] });
  expect(rowsIn()[0].getAttribute('aria-label')).toBe(t('server.gm.checkpoint.select', {
    name: 'Another world',
    verdict: t('server.gm.checkpoint.ineligible'),
  }));
});

it('never blanks an unreadable row, it says what it does not know', async () => {
  await mount({ rows: [row({ scenario: null, capture_tick: null, preflight: null })] });
  expect(rowsIn()[0].querySelector('.gm-checkpoint-record').textContent)
    .toBe(t('server.gm.checkpoint.row_detail', {
      scenario: t('server.gm.checkpoint.unknown_scenario'),
      tick: t('server.gm.checkpoint.unknown_tick'),
    }));
  expect(rowsIn()[0].dataset.eligible).toBe('unknown');
});

it('says an unchecked row is unchecked rather than refused', async () => {
  // The landing catalogue publishes no preflight: there is no live session for
  // a row to be a candidate for. That is not the same claim as "this save
  // cannot hold the current assignments", and a row must not make it.
  await mount({ rows: [row(), row({ slot_id: 'slot-c', display_name: 'Unchecked', preflight: null })] });
  const unchecked = rowsIn()[1];
  expect(unchecked.dataset.eligible).toBe('unknown');
  expect(unchecked.querySelector('.gm-checkpoint-verdict').textContent)
    .toBe(t('server.gm.checkpoint.verdict_unknown'));
  expect(unchecked.getAttribute('aria-label')).toBe(t('server.gm.checkpoint.select', {
    name: 'Unchecked',
    verdict: t('server.gm.checkpoint.verdict_unknown'),
  }));

  // ...and it is not counted as a shortfall against the rows that WERE checked.
  expect(document.getElementById('gm-checkpoint-summary').textContent).toBe(
    `${t('server.gm.checkpoint.summary', { eligible: '1', total: '1' })} `
    + t('server.gm.checkpoint.summary_unchecked', { unchecked: '1' }));

  // The detail region says the same thing instead of falling blank.
  panel.select('slot-c');
  const detail = document.getElementById('gm-checkpoint-detail-preflight');
  expect(detail.dataset.eligible).toBe('unknown');
  expect(detail.querySelector('.gm-checkpoint-verdict').textContent)
    .toBe(t('server.gm.checkpoint.verdict_unknown'));
  expect(detail.querySelectorAll('li')).toHaveLength(0);
});

it('keeps the very row element an operator is reaching for across a refresh', async () => {
  // The catalogue is re-read continuously on a live console. If each read
  // rebuilt the list, the node under a finger would be replaced between the
  // press and the release and the click would land on nothing — which is what
  // #1418's stable target means in practice, and what a real browser catches.
  await mount({ rows: [row(), INCOMPATIBLE] });
  const before = rowsIn();
  await panel.refresh();
  expect(rowsIn()[0]).toBe(before[0]);
  expect(rowsIn()[1]).toBe(before[1]);
  expect(before[0].isConnected).toBe(true);

  // A row still repaints in place when its verdict changes...
  listed = [row({ preflight: { eligible: false, blocks: [{ kind: 'rules-moved' }] } }), INCOMPATIBLE];
  await panel.refresh();
  expect(rowsIn()[0]).toBe(before[0]);
  expect(before[0].dataset.eligible).toBe('false');

  // ...and a row that leaves the catalogue leaves the list with it.
  listed = [INCOMPATIBLE];
  await panel.refresh();
  expect(rowsIn()).toHaveLength(1);
  expect(rowsIn()[0]).toBe(before[1]);
  expect(before[0].isConnected).toBe(false);
});

it('keys its rows on an attribute the save catalogue does not also use', async () => {
  // server.html carries BOTH lists at once, over the same slot ids. Sharing
  // `data-slot-id` made every unscoped selector for a save row ambiguous.
  await mount({ rows: [row()] });
  expect(rowsIn()[0].dataset.checkpointSlotId).toBe('slot-a');
  expect(rowsIn()[0].hasAttribute('data-slot-id')).toBe(false);
});
