import { describe, expect, it, vi } from 'vitest';
import {
  MOD_ACTION_CONTEXT,
  MOD_IMPORT_ACTION,
  MOD_IMPORT_ACTION_ID,
  MOD_T2_SCOPE,
  createModActionRegistry,
  installModActionKeyboard,
} from '../mod-actions.js';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
} from '../../gui/action-feedback.js';

describe('MOD semantic import action', () => {
  it('has stable editor scope and exactly two portable binding slots', () => {
    expect(MOD_IMPORT_ACTION).toMatchObject({
      id: MOD_IMPORT_ACTION_ID,
      contexts: [MOD_ACTION_CONTEXT],
      feedback: 'local',
    });
    expect(MOD_IMPORT_ACTION.bindings).toHaveLength(2);
    expect(MOD_IMPORT_ACTION.bindings[0]).toMatchObject({
      type: 'keyboard',
      code: 'KeyI',
    });
    expect(MOD_IMPORT_ACTION.bindings[1]).toBeNull();
  });

  it('runs chooser work through the shared local feedback lifecycle', () => {
    const transitions = [];
    const lifecycle = new ActionFeedbackLifecycle({
      correlation: () => 'editor-import-1',
      now: () => 42,
      onTransition: (value) => transitions.push(value),
    });
    let activation;
    const openImport = vi.fn((value) => { activation = value; return true; });
    const registry = createModActionRegistry({ actionFeedback: lifecycle, openImport });

    expect(registry.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: true, correlation: 'editor-import-1' });
    activation.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);

    expect(openImport).toHaveBeenCalledOnce();
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.APPLIED,
    ]);
  });

  it('keeps M6 inspectors and project tooling outside the T2 adapter', () => {
    expect(MOD_T2_SCOPE).toEqual({
      import: true,
      inspectors: false,
      projectTooling: false,
    });
  });

  it('dispatches keyboard input only while the MOD context is active', () => {
    const target = new EventTarget();
    const modeShell = { getCurrentMode: vi.fn(() => 'World') };
    const modActions = { dispatchKeyboardEvent: vi.fn() };
    const uninstall = installModActionKeyboard({ target, modeShell, modActions });

    target.dispatchEvent(new Event('keydown'));
    expect(modActions.dispatchKeyboardEvent).not.toHaveBeenCalled();

    modeShell.getCurrentMode.mockReturnValue('MOD');
    target.dispatchEvent(new Event('keydown'));
    expect(modActions.dispatchKeyboardEvent).toHaveBeenCalledOnce();

    uninstall();
    target.dispatchEvent(new Event('keydown'));
    expect(modActions.dispatchKeyboardEvent).toHaveBeenCalledOnce();
  });
});
