import { expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { definitionsSnapshot } from '../../editor/workshop-definitions.js';
import { createWorkshopEditSession } from '../../gui/workshop-edit-session.js';

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
