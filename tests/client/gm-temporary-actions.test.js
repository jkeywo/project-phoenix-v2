// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { createTemporaryActions } from '../../gui/gm-temporary-actions.js';
import { defaultLiveLayout, liveLayoutModel } from '../../gui/live-layout-model.js';

/** A stand-in for the mounted dock: the controller only reads state and asks it
 * to change, which is the whole of the seam. */
function harness({ confirmDiscard = () => true } = {}) {
  let state = defaultLiveLayout();
  const revealed = [];
  const layout = {
    model: liveLayoutModel,
    state: () => state,
    set: next => { state = liveLayoutModel.normalize(next); },
    reveal: (panel, options = {}) => {
      if (options.reopen === false && state.closed.includes(panel)) return false;
      revealed.push(panel);
      state = state.closed.includes(panel)
        ? liveLayoutModel.float(state, panel, {}) : liveLayoutModel.select(state, panel);
      return true;
    },
  };
  const actions = createTemporaryActions({ layout, confirmDiscard });
  const draft = { dirty: false, keepOpen: false, resets: [], focused: 0 };
  actions.register('spawn', {
    isDirty: () => draft.dirty,
    reset: options => { draft.resets.push(options); draft.dirty = false; },
    keepOpen: () => draft.keepOpen,
    focus: () => { draft.focused += 1; },
  });
  return { actions, draft, revealed, current: () => state };
}

describe('complex GM action lifecycle', () => {
  it('opens one floating draft per action and focuses it when invoked again', () => {
    const { actions, draft, revealed, current } = harness();
    expect(current().closed).toContain('spawn');

    expect(actions.open('spawn')).toBe(true);
    expect(current().floats.map(entry => entry.panel)).toEqual(['spawn']);
    // A fresh open starts from the defaults: a draft is not a memory.
    expect(draft.resets).toEqual([{ keepReusable: false }]);
    expect(draft.focused).toBe(1);

    draft.dirty = true;
    expect(actions.open('spawn')).toBe(true);
    // The same one draft, focused — not a second copy, and not cleared.
    expect(current().floats.map(entry => entry.panel)).toEqual(['spawn']);
    expect(draft.resets).toHaveLength(1);
    expect(draft.focused).toBe(2);
    expect(revealed).toEqual(['spawn', 'spawn']);
  });

  it('closes an undocked unchecked draft only on authoritative success', () => {
    const { actions, draft, current } = harness();
    actions.open('spawn');
    expect(actions.succeeded('spawn')).toBe(true);
    expect(current().closed).toContain('spawn');
    // A repeat keeps WHAT and clears WHERE.
    expect(draft.resets.at(-1)).toEqual({ keepReusable: true });
  });

  it('keeps a checked draft open, and keeps a docked one whatever the checkbox says', () => {
    const { actions, draft, current } = harness();
    actions.open('spawn');
    draft.keepOpen = true;
    expect(actions.succeeded('spawn')).toBe(false);
    expect(current().closed).not.toContain('spawn');
    expect(draft.focused).toBe(2);

    // Docking implies Keep open: the operator put it somewhere on purpose.
    draft.keepOpen = false;
    const docked = liveLayoutModel.dock(current(), 'spawn', 'roster', 'tab');
    expect(docked.floats.some(entry => entry.panel === 'spawn')).toBe(false);
    const { actions: dockedActions, draft: dockedDraft } = harness();
    dockedActions.open('spawn');
    dockedActions.close('spawn');
    expect(dockedDraft.resets.at(-1)).toEqual({ keepReusable: false });
  });

  it('asks before losing typed work, and leaves the draft alone when refused', () => {
    const refusals = [];
    const { actions, draft, current } = harness({
      confirmDiscard: panel => { refusals.push(panel); return false; },
    });
    actions.open('spawn');
    draft.dirty = true;

    expect(actions.close('spawn')).toBe(false);
    expect(refusals).toEqual(['spawn']);
    expect(current().closed).not.toContain('spawn');
    // A layout reset would take it away too, so it gets the same say.
    expect(actions.mayReset()).toBe(false);

    draft.dirty = false;
    expect(actions.close('spawn')).toBe(true);
    expect(current().closed).toContain('spawn');
    expect(actions.mayReset()).toBe(true);
  });

  it('takes a draft off the screen without asking at a run boundary', () => {
    const confirmDiscard = vi.fn(() => false);
    const { actions, draft, current } = harness({ confirmDiscard });
    actions.open('spawn');
    draft.dirty = true;
    // The world the draft was for has gone: there is nothing left to confirm.
    expect(actions.closeSilently('spawn')).toBe(true);
    expect(current().closed).toContain('spawn');
    expect(confirmDiscard).not.toHaveBeenCalled();
  });

  it('forgets a draft only after the caller says the discard happened', () => {
    const { actions, draft } = harness();
    actions.open('spawn');
    draft.dirty = true;
    expect(actions.mayDiscard('spawn')).toBe(true);
    // Agreeing is not forgetting: the caller does its thing, then says so.
    expect(draft.resets).toHaveLength(1);
    actions.discard('spawn');
    expect(draft.resets.at(-1)).toEqual({ keepReusable: false });
    expect(draft.dirty).toBe(false);
  });

  it('never asks about a clean draft', () => {
    const confirmDiscard = vi.fn(() => true);
    const { actions } = harness({ confirmDiscard });
    actions.open('spawn');
    expect(actions.close('spawn')).toBe(true);
    expect(confirmDiscard).not.toHaveBeenCalled();
  });
});
