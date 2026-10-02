import { expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { definitionsSnapshot } from '../../editor/workshop-definitions.js';
import { createWorkshopEditSession, runWorkshopMutation, refreshWorkshopReading, retainUnappliedForms, restoreUnappliedForms } from '../../gui/workshop-edit-session.js';

const path = 'assets/worlds/test.toml';
const source = '# retained\n[global]\nseed = 1\n';
const replacement = source.replace('1', '2');
const document = () => new WorkshopDocument(createStoreZip([{ path, text: source }]), { kind: 'project' });
function setup(extra = {}) {
  let draft = document(), disposed = false, busy = false;
  const events = [];
  const showError = vi.fn();
  const session = createWorkshopEditSession({
    draft: () => draft, disposed: () => disposed,
    setBusy: value => { busy = value; }, changed: () => events.push('changed'),
    showChanged: () => events.push('success'), showError,
    reload: async () => { expect(busy).toBe(true); events.push('reload'); },
    refresh: () => { expect(busy).toBe(false); events.push('refresh'); },
    restoreFocus: (...ids) => { expect(busy).toBe(false); events.push(ids); },
    ...extra,
  });
  return { ...session, draft, read: definitionsSnapshot(draft), events, showError,
    dispose: () => { disposed = true; }, replace: () => { draft = document(); } };
}

it('lands one undoable edit, refreshes, then restores focus after releasing controls', async () => {
  const s = setup();
  await s.guarded(() => s.land(s.read, path, replacement, ['apply', 'refresh']));
  expect(s.draft.read(path)).toBe(replacement);
  expect(s.events).toEqual(['changed', 'success', 'reload', 'refresh', ['apply', 'refresh']]);
  expect(s.draft.undo()).toBe(path);
  expect(s.draft.read(path)).toBe(source);
  expect(s.draft.undo()).toBeNull();
});

it.each(['edit', 'replace', 'dispose'])('does not land an asynchronous answer after %s', async change => {
  const s = setup();
  let answer;
  const pending = new Promise(resolve => { answer = resolve; });
  const operation = s.guarded(async () => s.land(s.read, path, await pending));
  if (change === 'edit') s.draft.edit(path, '# newer source');
  if (change === 'replace') s.replace();
  if (change === 'dispose') s.dispose();
  answer(replacement);
  await operation;
  expect(s.draft.read(path)).toBe(change === 'edit' ? '# newer source' : source);
  expect(s.events).not.toContain('changed');
  if (change !== 'dispose') expect(s.showError.mock.calls[0][0].message).toBe('workshop.inspector_stale');
  else { expect(s.showError).not.toHaveBeenCalled(); expect(s.events).toEqual([]); }
});

it('retains a landed edit and focus when rereading fails', async () => {
  const s = setup({ reload: async () => { throw new Error('unavailable'); } });
  await s.guarded(() => s.land(s.read, path, replacement, ['apply']));
  expect(s.draft.read(path)).toBe(replacement);
  expect(s.showError).not.toHaveBeenCalled();
  expect(s.events.at(-1)).toEqual(['apply']);
});

it.each(['runtime', 'invalid-answer'])('releases controls without changing source on %s refusal', async mode => {
  const s = setup();
  await s.guarded(() => {
    if (mode === 'runtime') throw new Error('operation-specific refusal');
    return s.land(s.read, path, null);
  });
  expect(s.draft.read(path)).toBe(source);
  expect(s.showError).toHaveBeenCalledOnce();
  expect(s.events).toEqual(['refresh']);
});

it('captures edits made during a read after validation and before rebuilding forms', async () => {
  const draft = document(), events = [];
  let answer, state = { value: 1 };
  const pending = new Promise(resolve => { answer = resolve; });
  await Promise.resolve();
  const operation = refreshWorkshopReading({
    draft: () => draft, disposed: () => false, read: () => pending,
    validate: result => { expect(result).toBe('typed reading'); events.push('validate'); },
    forms: () => { events.push('capture'); return [{ state, baseline: { value: 1 },
      identity: [path, 'station'], source,
      current: () => ({ identity: [path, 'station'], source }),
      restore: saved => { state = saved; events.push('restore'); } }]; },
    install: () => { state = { value: 1 }; events.push('install'); },
    announce: () => events.push('announce'),
  });
  state = { value: 2 };
  answer('typed reading');
  await operation;
  expect(state).toEqual({ value: 2 });
  expect(events).toEqual(['validate', 'capture', 'install', 'restore', 'announce']);
});

it.each(['source', 'selection', 'clean', 'absent'])('does not restore an ineligible %s form', kind => {
  const restore = vi.fn();
  const form = { state: kind === 'absent' ? null : { value: kind === 'clean' ? 1 : 2 }, baseline: { value: 1 },
    identity: [path, 'station'], source,
    current: () => ({ identity: [path, kind === 'selection' ? 'other' : 'station'],
      source: kind === 'source' ? replacement : source }), restore };
  restoreUnappliedForms(retainUnappliedForms([form]));
  expect(restore).not.toHaveBeenCalled();
});

it.each(['edit', 'replace', 'dispose', 'malformed'])('prevents refresh painting after %s', async kind => {
  let draft = document(), disposed = false, answer;
  const pending = new Promise(resolve => { answer = resolve; });
  const install = vi.fn(), forms = vi.fn(() => []), announce = vi.fn();
  const operation = refreshWorkshopReading({
    draft: () => draft, disposed: () => disposed, read: () => pending,
    validate: result => { if (result === null) throw new Error('workshop.inspector_refused'); },
    forms, install, announce,
  });
  if (kind === 'edit') draft.edit(path, replacement);
  if (kind === 'replace') draft = document();
  if (kind === 'dispose') disposed = true;
  answer(kind === 'malformed' ? null : {});
  if (kind === 'malformed') await expect(operation).rejects.toThrow('workshop.inspector_refused');
  else await operation;
  expect(forms).not.toHaveBeenCalled(); expect(install).not.toHaveBeenCalled(); expect(announce).not.toHaveBeenCalled();
});


it('acquires mutation busy state synchronously and releases before refresh', async () => {
  let resolve;
  const pending = new Promise(done => { resolve = done; });
  const order = [];
  const run = runWorkshopMutation({ setBusy: value => order.push(value), start: () => order.push('start'),
    invoke: () => { order.push('invoke'); return pending; }, current: () => true,
    success: value => order.push(value), error: error => { throw error; }, release: () => order.push('refresh') });
  expect(order).toEqual([true, 'start', 'invoke']);
  resolve('success'); await run;
  expect(order).toEqual([true, 'start', 'invoke', 'success', false, 'refresh']);
});

it('retains adapter phase policies, including unguarded Models and Scripts success', async () => {
  let current = true, resolve;
  const events = [], pending = new Promise(done => { resolve = done; });
  const run = runWorkshopMutation({ setBusy: value => events.push(value), current: () => current,
    successCurrent: () => true, invoke: () => pending, success: () => events.push('success'),
    error: () => events.push('error'), release: () => events.push('refresh') });
  current = false; resolve(); await run;
  expect(events).toEqual([true, 'success']);
  await runWorkshopMutation({ setBusy: value => events.push(value), current: () => false,
    invoke: () => { throw new Error('refused'); }, success: () => events.push('guarded success'),
    error: () => events.push('error'), release: () => events.push('refresh') });
  expect(events).toEqual([true, 'success', true]);
});

it('delegates synchronous invocation and success errors before normal release', async () => {
  const failure = new Error('refused'), order = [];
  await runWorkshopMutation({ setBusy: value => order.push(value), current: () => true,
    invoke: () => 4, success: () => { throw failure; },
    error: error => order.push(error), release: () => order.push('refresh') });
  expect(order).toEqual([true, failure, false, 'refresh']);
});
