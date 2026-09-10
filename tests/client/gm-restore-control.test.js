// @vitest-environment jsdom
//
// The single-simulation-peer live-restore control (issue #1446), mounted on the
// REAL `server.html` markup with the REAL String Table, driven by the exact
// `gm_health` payload `src/gm_health.rs` publishes and the exact catalogue-row
// shape `bridge::save_slot_js` publishes.
//
// The DECISIONS under test live in Rust and are pinned there
// (`src/gm_restore.rs`, `tests/gm_restore.rs`). What is tested here is the
// contract PRD #1418 puts on the surface: that every phase is a SENTENCE, that
// a control a GM cannot press says why, that the confirmation preview names the
// consequences, and that a refusal reaches the operator instead of vanishing.
import { beforeEach, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createGmCheckpointPanel } from '../../gui/gm-checkpoint-panel.js';
import {
  createGmRestoreControl,
  parseGmRestoreState,
  restorePhaseLabelId,
} from '../../gui/gm-restore-control.js';
import { GM_CONFIRMATION_CATEGORIES, GM_ACTION_CONFIRMATION_METADATA } from '../../gui/gm-confirmation.js';
import { applyToDom, buildTable, has, setTable, t } from '../../gui/strings.js';

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
    capture_tick: '1200',
    metadata: 'present',
    preflight: { eligible: true, blocks: [] },
    ...overrides,
  };
}

/** One `gm_health` payload exactly as `GmHealthProjection` serialises. */
function health(restore, { paused = true } = {}) {
  return {
    tick: 1400,
    paused,
    peers: [{ id: 'gm:gm-1', operators: ['gm-1'], state: 'live', local: true }],
    stations: [],
    operators: [],
    alerts: [],
    ...(restore ? { restore } : {}),
  };
}

let control;
let checkpoint;
let submitted;
let resumed;
let confirmations;

async function mount({
  rows = [row()],
  operator = { id: 'gm-1', name: 'Rowan' },
  accept = true,
  submitRestore = () => true,
  submitResume = () => true,
} = {}) {
  submitted = [];
  resumed = [];
  confirmations = [];
  checkpoint = createGmCheckpointPanel({
    doc: document,
    t,
    api: { list: vi.fn(() => rows), create: vi.fn(() => '') },
    canCapture: () => true,
    onSelect: () => control?.refresh(),
  });
  await checkpoint.ready;
  control = createGmRestoreControl({
    doc: document,
    t,
    getCandidate: () => checkpoint.state().selected,
    getOperator: () => operator,
    submitRestore: (request) => {
      submitted.push(request);
      return submitRestore(request);
    },
    submitResume: (correlation) => {
      resumed.push(correlation);
      return submitResume(correlation);
    },
    confirmAction: (request) => {
      confirmations.push(request);
      return accept ? request.accept() : (request.onCancel?.(), false);
    },
    correlation: () => 'corr-1',
    schedule: () => 1,
    cancelSchedule: () => {},
  });
  return control;
}

beforeEach(() => {
  setTable(realStrings);
  const parsed = new DOMParser().parseFromString(pageSource, 'text/html');
  document.body.replaceChildren(parsed.getElementById('gm-checkpoint'));
  applyToDom(document);
});

const region = () => document.getElementById('gm-restore');
const summary = () => document.getElementById('gm-restore-summary');
const status = () => document.getElementById('gm-restore-status');
const applyButton = () => document.getElementById('gm-restore-apply');
const resumeButton = () => document.getElementById('gm-restore-resume');
const select = (slotId) => document.querySelector(`[data-checkpoint-slot-id="${slotId}"]`).click();

// ── The bounded contract is spelled out in copy, not only in a disabled button ──

it('offers Restore only for a selected, eligible candidate, and says why otherwise', async () => {
  await mount({
    rows: [
      row(),
      row({
        slot_id: 'slot-b',
        display_name: 'Another world',
        preflight: {
          eligible: false,
          blocks: [{ kind: 'scenario-differs', candidate: 'assets/worlds/combat_test.toml', live: WORLD }],
        },
      }),
    ],
  });
  // Nothing selected: the control asks for a selection rather than sitting
  // blank with a dead button.
  expect(applyButton().disabled).toBe(true);
  expect(summary().textContent).toBe(t('server.gm.restore.no_candidate'));

  select('slot-b');
  expect(applyButton().disabled).toBe(true);
  // The refusal is in WORDS, and in the same vocabulary the picker used.
  expect(summary().textContent).toContain('Another world');
  expect(summary().textContent).toContain('assets/worlds/combat_test.toml');

  select('slot-a');
  expect(applyButton().disabled).toBe(false);
  expect(summary().textContent).toContain('Before the ambush');
  expect(summary().textContent).toContain('1200');
});

it('names an engine-named candidate by its authored sentence, in summary and preview', async () => {
  // A restore leaves its own recovery checkpoint in the catalogue, and
  // `src/gm_restore.rs` names that row with the String Table id
  // `RECOVERY_CHECKPOINT_NAME`. It is an ordinary candidate for the NEXT
  // restore, so neither the summary nor the confirmation preview — the two
  // places a GM reads which session they are about to load — may show the id.
  const id = 'server.gm.restore.recovery_name';
  await mount({ rows: [row({ slot_id: 'slot-recovery', display_name: id })] });
  const authored = t(id);
  expect(authored).not.toBe(id);

  select('slot-recovery');
  expect(summary().textContent).toContain(authored);
  expect(summary().textContent).not.toContain(id);

  applyButton().click();
  expect(confirmations[0].description).toContain(authored);
  expect(confirmations[0].description).not.toContain(id);
  // The name is presentation only: the submitted action still names the slot.
  expect(submitted[0].candidate).toBe('slot-recovery');
});

it('names the concrete consequences in the confirmation preview', async () => {
  await mount();
  select('slot-a');
  applyButton().click();

  expect(confirmations).toHaveLength(1);
  const request = confirmations[0];
  expect(request.category).toBe('world.restore');
  expect(request.description).toContain('Before the ambush');
  expect(request.description).toContain('1200');
  const preview = request.preview();
  // The technical change AND what the crew already witnessed (PRD #1420 story
  // 6): a restored value cannot un-see what a room saw.
  expect(preview).toContain('discarded');
  expect(preview).toContain('Station');
  expect(preview).toContain('remember');
  expect(submitted).toEqual([{
    action: 'request_live_restore',
    operator_id: 'gm-1',
    candidate: 'slot-a',
    correlation: 'corr-1',
  }]);
});

it('submits nothing when the confirmation is cancelled', async () => {
  await mount({ accept: false });
  select('slot-a');
  applyButton().click();
  expect(submitted).toEqual([]);
});

it('submits the candidate the GM confirmed, not whatever is selected afterwards', async () => {
  const pending = [];
  await mount({
    rows: [row(), row({ slot_id: 'slot-b', display_name: 'Second' })],
    accept: false,
  });
  // Capture the accept callback, move the selection, then accept.
  control.destroy();
  control = createGmRestoreControl({
    doc: document,
    t,
    getCandidate: () => checkpoint.state().selected,
    getOperator: () => ({ id: 'gm-1' }),
    submitRestore: (request) => { pending.push(request); return true; },
    confirmAction: (request) => { confirmations.push(request); return true; },
    correlation: () => 'corr-1',
    schedule: () => 1,
    cancelSchedule: () => {},
  });
  select('slot-a');
  applyButton().click();
  select('slot-b');
  confirmations[0].accept();
  expect(pending[0].candidate).toBe('slot-a');
});

// ── Every phase is a sentence ───────────────────────────────────────────────

it('reads every restore phase back as words, from the shared health projection', async () => {
  await mount();
  for (const [phase, fragment] of [
    ['accepted', 'asked for a restore'],
    ['capturing-recovery', 'recovery checkpoint'],
    ['loading', 'Loading'],
    ['restored', 'Restored and held'],
    ['rolled-back', 'did not happen'],
    ['failed', 'could not be put back'],
  ]) {
    expect(control.update(health({
      phase, operator: 'Rowan', working: ['accepted', 'capturing-recovery', 'loading'].includes(phase),
      restored_tick: 1200,
    }))).toBe(true);
    expect(region().dataset.phase).toBe(phase);
    expect(status().hidden).toBe(false);
    expect(status().textContent).toContain(fragment);
    // Every phase id this build can report really exists in the String Table.
    expect(has(restorePhaseLabelId(phase))).toBe(true);
  }
});

it('appends the failure sentence to the phase sentence', async () => {
  await mount();
  control.update(health({
    phase: 'rolled-back', operator: 'Rowan', working: false,
    failure: 'server.gm.restore.failed.recovery_capture',
  }));
  expect(status().textContent).toContain(t('server.gm.restore.failed.recovery_capture'));
  expect(status().dataset.tone).toBe('failed');
});

it('leaves the last honest picture on screen when a payload is not a projection', async () => {
  await mount();
  control.update(health({ phase: 'restored', operator: 'Rowan', working: false, restored_tick: 7 }));
  const before = status().textContent;
  expect(control.update('not json')).toBe(false);
  expect(control.update({ peers: 'nonsense' })).toBe(false);
  expect(status().textContent).toBe(before);
});

// ── Working, resuming and refusing ──────────────────────────────────────────

it('offers Resume only while a reported restore still holds the session', async () => {
  await mount();
  select('slot-a');
  // Nothing running: nothing to resume.
  expect(resumeButton().disabled).toBe(true);

  control.update(health({ phase: 'loading', operator: 'Rowan', working: true }));
  expect(resumeButton().disabled).toBe(true);
  expect(applyButton().disabled).toBe(true);

  control.update(health({ phase: 'restored', operator: 'Rowan', working: false, restored_tick: 1200 }));
  expect(resumeButton().disabled).toBe(false);
  resumeButton().click();
  expect(resumed).toEqual(['corr-1']);

  // A resumed session is running again, so the hold — and the offer — is gone.
  control.update(health(null, { paused: false }));
  expect(resumeButton().disabled).toBe(true);
});

it('reports a canonical refusal off the one saved journal', async () => {
  await mount();
  select('slot-a');
  applyButton().click();
  expect(status().textContent).toBe(t('server.gm.restore.requesting'));

  expect(control.settleJournal([{
    operator_id: 'gm-1',
    correlation: 'corr-1',
    outcome: 'refused',
    reason: 'multiple-simulation-peers',
  }])).toBe(true);
  expect(status().dataset.tone).toBe('failed');
  expect(status().textContent).toContain(t('server.gm.session.reason.multiple_simulation_peers'));
});

it('reports a local submission failure rather than showing a pending restore', async () => {
  await mount({ submitRestore: () => false });
  select('slot-a');
  applyButton().click();
  expect(status().dataset.tone).toBe('failed');
  expect(status().textContent).toBe(t('server.gm.restore.local_refusal'));
});

// ── Shape guards ────────────────────────────────────────────────────────────

it('treats an absent restore field as idle and an unknown phase as idle', async () => {
  expect(parseGmRestoreState(health(null))).toMatchObject({ phase: 'idle', paused: true });
  expect(parseGmRestoreState(health({ phase: 'teleported', operator: '', working: true })))
    .toMatchObject({ phase: 'idle', working: false });
  expect(parseGmRestoreState('not a projection')).toBe(null);
});

it('never reports a settled phase as still working, whatever the payload claims', async () => {
  // A host that said "restored" and "working" at once would disable the very
  // Resume the GM needs to end the hold.
  expect(parseGmRestoreState(health({ phase: 'restored', operator: 'a', working: true })).working)
    .toBe(false);
});

it('registers its confirmation category centrally with the preview default', () => {
  const category = GM_CONFIRMATION_CATEGORIES.find((entry) => entry.id === 'world.restore');
  expect(category?.defaultMode).toBe('confirm-preview');
  expect(GM_ACTION_CONFIRMATION_METADATA.RequestLiveRestore)
    .toEqual([{ confirmationCategory: 'world.restore', confirmationDefault: 'confirm-preview' }]);
});
