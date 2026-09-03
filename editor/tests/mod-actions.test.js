import { describe, expect, it, vi } from 'vitest';
import {
  MOD_ACTION_CONTEXT,
  MOD_ACTIONS,
  MOD_EXPORT_ACTION,
  MOD_EXPORT_ACTION_ID,
  MOD_IMPORT_ACTION,
  MOD_IMPORT_ACTION_ID,
  MOD_T2_SCOPE,
  MOD_VALIDATE_ACTION,
  MOD_VALIDATE_ACTION_ID,
  createModActionRegistry,
  installModActionKeyboard,
} from '../mod-actions.js';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
} from '../../gui/action-feedback.js';

describe('MOD semantic import action', () => {
  it('has stable editor scope and exactly two portable binding slots per tracer action', () => {
    expect(MOD_ACTIONS).toEqual([
      MOD_IMPORT_ACTION,
      MOD_VALIDATE_ACTION,
      MOD_EXPORT_ACTION,
    ]);
    expect(MOD_ACTIONS.map((action) => [action.id, action.bindings[0]?.code])).toEqual([
      [MOD_IMPORT_ACTION_ID, 'KeyI'],
      [MOD_VALIDATE_ACTION_ID, 'KeyV'],
      [MOD_EXPORT_ACTION_ID, 'KeyE'],
    ]);
    for (const action of MOD_ACTIONS) {
      expect(action).toMatchObject({ contexts: [MOD_ACTION_CONTEXT], feedback: 'local' });
      expect(action.bindings).toHaveLength(2);
      expect(action.bindings[1]).toBeNull();
    }
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

  it('keeps all requested M6 surfaces outside the T2 adapter', () => {
    expect(MOD_T2_SCOPE).toEqual({
      import: true,
      memberSourceEdit: true,
      validate: true,
      export: true,
      inspectors: false,
      projectTooling: false,
      modelTooling: false,
      workshopRedesign: false,
    });
  });

  it.each([
    [MOD_VALIDATE_ACTION_ID, 'validatePack'],
    [MOD_EXPORT_ACTION_ID, 'exportPack'],
  ])('runs %s through the shared local feedback lifecycle', (actionId, adapterName) => {
    const transitions = [];
    const lifecycle = new ActionFeedbackLifecycle({
      correlation: () => `${actionId}-1`,
      now: () => 43,
      onTransition: (value) => transitions.push(value),
    });
    let activation;
    const adapter = vi.fn((value) => { activation = value; return true; });
    const registry = createModActionRegistry({
      actionFeedback: lifecycle,
      [adapterName]: adapter,
    });

    expect(registry.activate(actionId, { context: MOD_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: true, correlation: `${actionId}-1` });
    activation.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.APPLIED,
    ]);
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

  it('leaves plain-letter shortcuts typeable in member and metadata editors', () => {
    let onKeydown;
    const target = {
      addEventListener: (_type, handler) => { onKeydown = handler; },
      removeEventListener: vi.fn(),
    };
    const modeShell = { getCurrentMode: () => 'MOD' };
    const modActions = { dispatchKeyboardEvent: vi.fn() };
    installModActionKeyboard({ target, modeShell, modActions });

    onKeydown({ target: { tagName: 'TEXTAREA' }, code: 'KeyE' });
    onKeydown({ target: { tagName: 'INPUT' }, code: 'KeyV' });
    onKeydown({ target: { tagName: 'DIV', isContentEditable: true }, code: 'KeyI' });
    expect(modActions.dispatchKeyboardEvent).not.toHaveBeenCalled();

    onKeydown({ target: { tagName: 'BUTTON' }, code: 'KeyE' });
    expect(modActions.dispatchKeyboardEvent).toHaveBeenCalledOnce();
  });
});
