// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import {
  captureAvailableForPhase,
  clearPendingResume,
  mountSaveSlots,
  normalizeSaveSlot,
  saveSlotsModel,
  startPreparationResult,
} from '../../gui/save-slots.js';

const READY = {
  slot_id: 'manual-1111',
  kind: 'manual',
  display_name: 'Before the jump',
  scenario: 'assets/worlds/combat_test.toml',
  selected_ship: 'assets/entities/alliance_destroyer.toml',
  capture_tick: '1800',
  compatible: true,
  refusal_kind: null,
  refusal: null,
  metadata: 'present',
  metadata_error: null,
};

const MOVED = {
  slot_id: 'manual-2222',
  kind: 'manual',
  display_name: 'Old rules',
  scenario: 'assets/worlds/default.toml',
  capture_tick: '2400',
  compatible: false,
  refusal_kind: 'rules',
  refusal: 'rules moved',
  metadata: 'present',
  metadata_error: null,
};

const CONTENT_PENDING = {
  ...READY,
  slot_id: 'manual-pending',
  display_name: 'Needs its world',
  compatible: false,
  startable: true,
  refusal_kind: 'content-pending',
  refusal: null,
};

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function fixture(initialRows = [READY, MOVED], overrides = {}) {
  document.body.innerHTML = '<aside id="save-slots"></aside>';
  let rows = initialRows.map((row) => ({ ...row }));
  const calls = [];
  const api = {
    list: () => rows,
    create: (name) => {
      calls.push(['create', name]);
      return 'manual-created';
    },
    rename: (id, name) => {
      calls.push(['rename', id, name]);
      const row = rows.find((entry) => entry.slot_id === id);
      if (row) row.display_name = name;
      return '';
    },
    export: (id) => {
      calls.push(['export', id]);
      return '';
    },
    takeExport: () => '(stored: true)',
    exportName: () => 'phoenix-save.ron',
    delete: (id, confirmed) => {
      calls.push(['delete', id, confirmed]);
      rows = rows.filter((entry) => entry.slot_id !== id);
      return '';
    },
    ...overrides.api,
  };
  const downloads = [];
  const starts = [];
  const droppedCaptures = [];
  const controller = mountSaveSlots({
    root: document.getElementById('save-slots'),
    doc: document,
    api,
    canCapture: () => overrides.canCapture !== false,
    download: (_doc, name, text) => {
      downloads.push([name, text]);
      return overrides.downloadResult !== false;
    },
    onStart: overrides.onStart || ((row) => {
      starts.push(row.slotId);
    }),
    onNewSession: () => calls.push(['new-session']),
    onPendingCaptureDropped: (message) => droppedCaptures.push(message),
    mode: overrides.mode,
    idPrefix: overrides.idPrefix,
    headerAction: overrides.headerAction,
  });
  return { controller, calls, downloads, droppedCaptures, starts, rows: () => rows };
}

function select(slotId) {
  document.querySelector(`[data-slot-id="${slotId}"]`).click();
}

describe('the pure catalogue model', () => {
  it('admits capture only during authoritative active play', () => {
    expect(captureAvailableForPhase('InProgress')).toBe(true);
    expect(captureAvailableForPhase('GameOver')).toBe(false);
    expect(captureAvailableForPhase('Lobby')).toBe(false);
  });

  it('normalises bridge fields and preserves Rust catalogue order', () => {
    const raw = { ...READY, seed: '99' };
    expect(normalizeSaveSlot(raw)).toMatchObject({
      slotId: 'manual-1111',
      displayName: 'Before the jump',
      scenario: 'assets/worlds/combat_test.toml',
      selectedShip: 'assets/entities/alliance_destroyer.toml',
      captureTick: '1800',
      canStart: true,
      canRename: true,
    });
    expect(saveSlotsModel([MOVED, READY]).slots.map((row) => row.slotId)).toEqual([
      'manual-2222',
      'manual-1111',
    ]);
  });

  it('names the incompatible dimension and corrupt metadata independently', () => {
    const rows = saveSlotsModel([
      MOVED,
      { ...READY, slot_id: 'format', compatible: false, refusal_kind: 'format' },
      { ...READY, slot_id: 'content', compatible: false, refusal_kind: 'content' },
      { ...READY, slot_id: 'corrupt', metadata: 'corrupt' },
    ]).slots;
    expect(rows.map((row) => row.compatibilityLabelId)).toEqual([
      'server.save_slots.incompatible_rules',
      'server.save_slots.incompatible_format',
      'server.save_slots.incompatible_content',
      'server.save_slots.compatible',
    ]);
    expect(rows[3].metadataLabelId).toBe('server.save_slots.metadata_corrupt');
  });
});

describe('the mounted catalogue', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  it('renders display name, scenario, tick, and compatibility', async () => {
    const { controller } = fixture();
    await controller.ready;
    const row = document.querySelector('[data-slot-id="manual-1111"]');
    expect(row.textContent).toContain('Before the jump');
    expect(row.textContent).toContain('assets/worlds/combat_test.toml');
    expect(row.textContent).toContain('1800');
    expect(row.textContent).toContain(t('server.save_slots.compatible'));
    expect(document.querySelector('[data-slot-id="manual-2222"]').textContent)
      .toContain(t('server.save_slots.incompatible_rules'));
  });

  it('renders the reserved storage key through its localized display label', async () => {
    const autosave = {
      ...READY,
      slot_id: 'autosave',
      kind: 'autosave',
      display_name: 'autosave',
    };
    const { controller } = fixture([autosave]);
    await controller.ready;

    const name = document.querySelector('[data-slot-id="autosave"] .save-slot-name');
    expect(name.textContent).toBe(t('server.save_slots.autosave'));
    expect(name.textContent).not.toBe('autosave');
  });

  // The recovery checkpoint a GM live restore (#1446) takes before it replaces
  // the world is an ordinary manual row in this list, and `src/gm_restore.rs`
  // names it with the String Table id `RECOVERY_CHECKPOINT_NAME` because that
  // crate has no locale. Every place this catalogue prints a name resolves it,
  // or a player opening Load reads `server.gm.restore.recovery_name` verbatim.
  it('renders an engine-named row through its authored sentence, never the id', async () => {
    const recovery = {
      ...READY,
      slot_id: 'manual-recovery',
      display_name: 'server.gm.restore.recovery_name',
    };
    const { controller } = fixture([recovery]);
    await controller.ready;
    const authored = t('server.gm.restore.recovery_name');

    const name = document.querySelector('[data-slot-id="manual-recovery"] .save-slot-name');
    expect(name.textContent).toBe(authored);
    expect(name.textContent).not.toContain('server.gm.restore.recovery_name');

    select('manual-recovery');
    expect(document.getElementById('save-slot-rename').value).toBe(authored);

    document.querySelector('[data-save-action="delete"]').click();
    expect(document.getElementById('save-slots-confirm-text').textContent)
      .toBe(t('server.save_slots.delete_confirm', { name: authored }));
  });

  it('cannot start an incompatible or unreadable row but starts the exact compatible row', async () => {
    const { controller, starts } = fixture([
      MOVED,
      { ...MOVED, slot_id: 'broken', refusal_kind: 'unreadable', scenario: null },
      READY,
    ]);
    await controller.ready;

    select('manual-2222');
    expect(document.querySelector('[data-save-action="start"]').disabled).toBe(true);
    document.querySelector('[data-save-action="start"]').click();
    expect(starts).toEqual([]);

    select('broken');
    expect(document.querySelector('[data-save-action="start"]').disabled).toBe(true);
    expect(document.querySelector('[data-save-action="delete"]').disabled).toBe(false);

    select('manual-1111');
    const start = document.querySelector('[data-save-action="start"]');
    expect(start.disabled).toBe(false);
    start.click();
    expect(starts).toEqual(['manual-1111']);
  });

  it('surfaces an async fail-closed identity handoff instead of staying pending', async () => {
    const { controller } = fixture([READY], { onStart: async () => false });
    await controller.ready;
    select('manual-1111');

    document.querySelector('[data-save-action="start"]').click();
    await flush();

    const status = document.querySelector('[role="status"]');
    expect(status.textContent).toBe(t('server.save_slots.action_failed', {
      detail: t('server.save_slots.local_failure'),
    }));
    expect(status.classList.contains('failed')).toBe(true);
  });

  it('keeps saved-session Start single-flight until navigation or refusal', async () => {
    let finishStart;
    const onStart = vi.fn(() => new Promise((resolve) => { finishStart = resolve; }));
    const { controller } = fixture([READY], { onStart });
    await controller.ready;
    select('manual-1111');
    const start = document.querySelector('[data-save-action="start"]');

    start.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    start.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(onStart).toHaveBeenCalledTimes(1);
    expect(start.disabled).toBe(true);
    expect(document.querySelector('.save-slots-actions').getAttribute('aria-busy')).toBe('true');

    finishStart(false);
    await flush();
    expect(start.disabled).toBe(false);
    expect(document.querySelector('.save-slots-actions').getAttribute('aria-busy')).toBe('false');
    expect(document.querySelector('[role="status"]').classList.contains('failed')).toBe(true);
  });

  it('allows deferred content to start and preserves a post-load content refusal', async () => {
    const { controller, starts } = fixture([CONTENT_PENDING]);
    await controller.ready;
    select('manual-pending');

    const row = document.querySelector('[data-slot-id="manual-pending"]');
    expect(row.textContent).toContain(t('server.save_slots.content_pending'));
    const start = document.querySelector('[data-save-action="start"]');
    expect(start.disabled).toBe(false);
    start.click();
    expect(starts).toEqual(['manual-pending']);

    // After that click the host loads this row's scenario and runs the full
    // Rust Versions::check. A true content move must remain a refusal for the
    // visible status rather than being mistaken for the earlier deferral.
    expect(startPreparationResult('scenario content moved')).toEqual({
      ok: false,
      message: 'scenario content moved',
    });
    expect(startPreparationResult('')).toEqual({ ok: true, message: '' });
  });

  it('routes export through the exact selected slot and exposes a local status', async () => {
    const { controller, calls, downloads } = fixture();
    await controller.ready;
    select('manual-2222');
    document.querySelector('[data-save-action="export"]').click();

    expect(calls).toContainEqual(['export', 'manual-2222']);
    expect(downloads).toEqual([['phoenix-save.ron', '(stored: true)']]);
    const status = document.querySelector('[role="status"]');
    expect(status.getAttribute('aria-live')).toBe('polite');
    expect(status.textContent).toBe(t('server.save_slots.exported'));
  });

  it('creates a named manual save as pending until the fixed-tick outcome arrives', async () => {
    const { controller, calls } = fixture();
    await controller.ready;
    const input = document.getElementById('save-slot-name');
    input.value = 'Crossing point';
    document.querySelector('[data-save-action="create"]').click();

    expect(calls).toContainEqual(['create', 'Crossing point']);
    expect(document.querySelector('[role="status"]').textContent).toBe(t('server.save_slots.pending'));
    expect(document.querySelector('.save-slots-create').getAttribute('aria-busy')).toBe('true');
    await controller.reportOutcome(false, 'browser save request queue capacity reached');
    expect(document.querySelector('[role="status"]').textContent)
      .toContain('browser save request queue capacity reached');
    expect(document.querySelector('[role="status"]').classList.contains('failed')).toBe(true);
    expect(document.querySelector('[role="status"]').hidden).toBe(false);
    expect(document.querySelector('.save-slots-create').getAttribute('aria-busy')).toBe('false');
    expect(document.querySelector('[data-save-action="create"]').disabled).toBe(false);
  });

  it('renames only the selected manual slot', async () => {
    const { controller, calls } = fixture();
    await controller.ready;
    select('manual-1111');
    const input = document.getElementById('save-slot-rename');
    input.value = 'After the jump';
    document.querySelector('[data-save-action="rename"]').click();
    await flush();
    expect(calls).toContainEqual(['rename', 'manual-1111', 'After the jump']);
    expect(document.querySelector('[data-slot-id="manual-1111"]').textContent).toContain('After the jump');
  });

  it('cancels deletion without calling the backend and returns focus to Delete', async () => {
    const { controller, calls } = fixture();
    await controller.ready;
    select('manual-1111');
    const trigger = document.querySelector('[data-save-action="delete"]');
    trigger.focus();
    trigger.click();

    expect(document.activeElement.dataset.saveAction).toBe('cancel-delete');
    document.querySelector('[data-save-action="cancel-delete"]').click();
    expect(calls.filter(([kind]) => kind === 'delete')).toEqual([]);
    expect(document.activeElement.dataset.saveAction).toBe('delete');
  });

  it('traps delete confirmation focus and Escape closes back to Delete', async () => {
    const { controller, calls } = fixture();
    await controller.ready;
    select('manual-1111');
    const trigger = document.querySelector('[data-save-action="delete"]');
    trigger.focus();
    trigger.click();

    const dialog = document.querySelector('[role="alertdialog"]');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement.dataset.saveAction).toBe('cancel-delete');
    const escape = new KeyboardEvent('keydown', {
      key: 'Escape',
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(escape);

    expect(escape.defaultPrevented).toBe(true);
    expect(dialog.hidden).toBe(true);
    expect(calls.filter(([kind]) => kind === 'delete')).toEqual([]);
    expect(document.activeElement).toBe(trigger);
  });

  it('makes the whole page inert for nested delete confirmation and restores it', async () => {
    const { controller } = fixture();
    await controller.ready;
    const outside = document.createElement('button');
    outside.setAttribute('aria-hidden', 'false');
    document.body.appendChild(outside);
    select('manual-1111');
    document.querySelector('[data-save-action="delete"]').click();

    const actions = document.querySelector('.save-slots-actions');
    expect(actions.hasAttribute('inert')).toBe(true);
    expect(actions.getAttribute('aria-hidden')).toBe('true');
    expect(outside.hasAttribute('inert')).toBe(true);
    expect(outside.getAttribute('aria-hidden')).toBe('true');

    document.querySelector('[data-save-action="cancel-delete"]').click();
    expect(actions.hasAttribute('inert')).toBe(false);
    expect(actions.hasAttribute('aria-hidden')).toBe(false);
    expect(outside.hasAttribute('inert')).toBe(false);
    expect(outside.getAttribute('aria-hidden')).toBe('false');
  });

  it('deletes only after explicit confirmation and moves focus to the catalogue heading', async () => {
    const { controller, calls } = fixture();
    await controller.ready;
    select('manual-2222');
    document.querySelector('[data-save-action="delete"]').click();
    document.querySelector('[data-save-action="confirm-delete"]').click();
    await flush();

    expect(calls).toContainEqual(['delete', 'manual-2222', true]);
    expect(document.querySelector('[data-slot-id="manual-2222"]')).toBeNull();
    expect(document.activeElement.id).toBe('save-slots-heading');
    expect(document.querySelector('[role="status"]').textContent).toBe(t('server.save_slots.deleted'));
  });

  it('keeps a corrupt-metadata row operable and reports local catalogue failure visibly', async () => {
    const corrupt = { ...READY, slot_id: 'corrupt', metadata: 'corrupt' };
    let mounted = fixture([corrupt]);
    await mounted.controller.ready;
    expect(document.querySelector('[data-slot-id="corrupt"]').textContent)
      .toContain(t('server.save_slots.metadata_corrupt'));
    select('corrupt');
    expect(document.querySelector('[data-save-action="start"]').disabled).toBe(false);

    mounted = fixture([], { api: { list: () => { throw new Error('local storage denied'); } } });
    await mounted.controller.ready;
    const status = document.querySelector('[role="status"]');
    expect(status.textContent).toContain('local storage denied');
    expect(status.classList.contains('failed')).toBe(true);
    expect(status.hidden).toBe(false);
  });

  it('keeps New Session available while the pre-session catalogue omits capture', async () => {
    const { controller, calls } = fixture([READY], { mode: 'catalogue', canCapture: false });
    await controller.ready;
    expect(document.querySelector('[data-save-action="create"]')).toBeNull();
    const fresh = document.querySelector('[data-save-action="new-session"]');
    expect(fresh.disabled).toBe(false);
    fresh.click();
    expect(calls).toContainEqual(['new-session']);
  });

  it('rerenders the persistent named-save control when a capturable phase arrives', async () => {
    const { controller, calls } = fixture([], {
      mode: 'manual',
      idPrefix: 'in-session',
      canCapture: false,
    });
    await controller.ready;

    expect(document.querySelector('[data-save-action="new-session"]')).toBeNull();
    const input = document.getElementById('in-session-save-slot-name');
    const create = document.querySelector('[data-save-action="create"]');
    expect(input.disabled).toBe(true);
    expect(create.disabled).toBe(true);

    controller.setPhase('InProgress');
    expect(input.disabled).toBe(false);
    expect(create.disabled).toBe(false);
    input.value = 'At the boundary';
    create.click();
    expect(calls).toContainEqual(['create', 'At the boundary']);
  });

  it.each(['GameOver', 'Lobby'])(
    'resolves an admitted capture when InProgress transitions to %s',
    async (nextPhase) => {
      const { controller, droppedCaptures } = fixture([], {
        mode: 'manual',
        idPrefix: 'in-session',
        canCapture: false,
      });
      await controller.ready;

      controller.setPhase('InProgress');
      const input = document.getElementById('in-session-save-slot-name');
      const create = document.querySelector('[data-save-action="create"]');
      input.value = 'Boundary save';
      create.click();
      expect(document.querySelector('.save-slots-create').getAttribute('aria-busy')).toBe('true');

      const refusal = controller.setPhase(nextPhase);

      expect(refusal).toBe(t('server.save_slots.capture_dropped'));
      expect(droppedCaptures).toEqual([t('server.save_slots.capture_dropped')]);
      expect(document.querySelector('.save-slots-create').getAttribute('aria-busy')).toBe('false');
      expect(create.disabled).toBe(true);
      const status = document.querySelector('[role="status"]');
      expect(status.textContent).toContain(t('server.save_slots.capture_dropped'));
      expect(status.classList.contains('failed')).toBe(true);
      expect(status.hidden).toBe(false);
    },
  );

  it('New Session consumes every resume boot field without dropping unrelated state', () => {
    const replaceState = vi.fn();
    const cleaned = clearPendingResume({
      location: {
        href: 'https://bridge.example/server.html?resume=autosave&scenario=assets%2Fworlds%2Fdefault.toml&ship=assets%2Fentities%2Falliance_destroyer.toml&log=info#crew',
      },
      history: { replaceState },
    });
    const url = new URL(cleaned);

    expect(url.searchParams.has('resume')).toBe(false);
    expect(url.searchParams.has('scenario')).toBe(false);
    expect(url.searchParams.has('ship')).toBe(false);
    expect(url.searchParams.get('log')).toBe('info');
    expect(url.hash).toBe('#crew');
    expect(replaceState).toHaveBeenCalledWith(null, '', cleaned);
  });
});

describe('the panel header (issue #1363)', () => {
  // Load Game shows this catalogue in the landing's middle column, and the
  // portable-save importer moved into this panel's header with it — it is an
  // action ON the catalogue, not a separate block of the boot panel it happened
  // to sit beside. What that costs the module is one row and one option, and
  // both are asserted here rather than in the page that supplies the node.

  /** The importer as server.html builds it: a button and a hidden file input. */
  function importer() {
    const host = document.createElement('div');
    host.id = 'snapshot-import';
    const input = document.createElement('input');
    input.id = 'snapshot-import-file';
    input.type = 'file';
    const button = document.createElement('button');
    button.id = 'snapshot-import-btn';
    button.type = 'button';
    host.append(input, button);
    return { host, button };
  }

  it('draws a header row holding the heading, in every mode', () => {
    for (const mode of ['combined', 'catalogue', 'manual']) {
      fixture([READY], { mode });
      const head = document.querySelector('.save-slots-head');
      expect(head, mode).not.toBe(null);
      const heading = document.querySelector('.save-slots-heading');
      expect(heading.parentElement).toBe(head);
      // The heading still NAMES the panel: it is what `aria-labelledby` points
      // at, and re-parenting it must not break that.
      expect(document.getElementById('save-slots').getAttribute('aria-labelledby'))
        .toBe(heading.id);
    }
  });

  it('moves the live header action into the row, listeners and children intact', () => {
    const { host, button } = importer();
    let presses = 0;
    button.addEventListener('click', () => { presses += 1; });
    document.body.appendChild(host);
    fixture([READY], { mode: 'catalogue', headerAction: host });

    const slot = document.querySelector('.save-slots-head-actions');
    expect(host.parentElement).toBe(slot);
    // The SAME node, not a rebuild of it: the page-lifetime handler and the
    // hidden <input type=file> are what make the control work at all.
    expect(document.getElementById('snapshot-import')).toBe(host);
    expect(document.getElementById('snapshot-import-file')).not.toBe(null);
    button.click();
    expect(presses).toBe(1);
  });

  it('leaves the slot empty when no action was handed over', () => {
    // A demo build removes the importer outright, and a surface that carries no
    // file chooser never had one. Neither is a hole where a control should be.
    fixture([READY], { mode: 'catalogue' });
    const slot = document.querySelector('.save-slots-head-actions');
    expect(slot).not.toBe(null);
    expect(slot.children).toHaveLength(0);
  });

  it('keeps the header above the rows it acts on', async () => {
    const { host } = importer();
    document.body.appendChild(host);
    fixture([READY, MOVED], { mode: 'catalogue', headerAction: host });
    await flush();
    const root = document.getElementById('save-slots');
    const kids = [...root.children];
    expect(kids.indexOf(document.querySelector('.save-slots-head')))
      .toBeLessThan(kids.indexOf(document.querySelector('.save-slots-list')));
    // ...and the list it sits above is still the one the catalogue lists into,
    // refusal and all — nothing about moving the importer moved the rows.
    expect([...document.querySelectorAll('.save-slot-select')]).toHaveLength(2);
    expect(document.querySelector('.save-slot-compatibility.incompatible').textContent)
      .toBe(t('server.save_slots.incompatible_rules'));
  });
});
