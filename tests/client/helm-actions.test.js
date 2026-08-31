import { describe, expect, it, vi } from 'vitest';
import {
  clientSettingsSemanticActions,
  createClientSemanticActionRegistry,
} from '../../gui/client-semantic-actions.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import {
  HELM_ACTION_CONTEXT,
  HELM_STEERING_ACTION,
  HELM_STEERING_ACTION_ID,
  registerHelmActions,
} from '../../gui/stations/helm-actions.js';

describe('Helm semantic steering action', () => {
  it('authors one undirected standard axis with range, cadence, and tuning defaults', () => {
    expect(HELM_STEERING_ACTION).toMatchObject({
      id: HELM_STEERING_ACTION_ID,
      contexts: [HELM_ACTION_CONTEXT],
      continuous: { min: -1, max: 1, neutral: 0, cadenceMs: 100 },
      tuning: { deadzone: 0.1, inverted: false },
      bindings: [
        { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
        null,
      ],
    });
  });

  it('adapts validated values to only the narrow steering console action', () => {
    const sendAction = vi.fn();
    const registry = registerHelmActions(createSemanticActionRegistry(), {
      getState: () => ({ helm_auto: false }),
      sendAction,
    });
    expect(registry.activate(HELM_STEERING_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      source: 'gamepad',
      value: -0.35,
    })).toMatchObject({ claimed: true, handled: true });
    expect(sendAction).toHaveBeenCalledOnce();
    expect(sendAction).toHaveBeenCalledWith('set_helm_steering', { value: -0.35 });
  });

  it('does not locally operate Helm while authoritative control source is Backfill', () => {
    const sendAction = vi.fn();
    const registry = registerHelmActions(createSemanticActionRegistry(), {
      getState: () => ({ helm_auto: true }),
      sendAction,
    });
    expect(registry.activate(HELM_STEERING_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      value: 0.8,
    })).toMatchObject({ claimed: true, handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('publishes play actions while retaining hidden editor bindings in the parent catalogue', () => {
    const registry = createClientSemanticActionRegistry();
    const ids = registry.list().map((action) => action.id);
    expect(ids).toEqual([
      'captain.red-alert',
      'captain.weapons-hold',
      'captain.view',
      'captain.objective-priority',
      HELM_STEERING_ACTION_ID,
      'tactical.target-selection',
      'tactical.phaser-mode',
      'tactical.phaser-fire',
      'tactical.blaster-charge',
      'tactical.blaster-fire',
      'tactical.blaster-cancel',
      'tactical.torpedo-volley-down',
      'tactical.torpedo-volley-up',
      'tactical.torpedo-fire',
      'comms.hail',
      'comms.select-message',
      'comms.respond',
      'comms.clear',
      'comms.show-on-screen',
      'sensors.target-selection',
      'sensors.scan',
      'sensors.viewscreen',
      'sensors.cancel-impulse',
      'science.shield-focus',
      'editor.mod.import',
      'editor.mod.validate',
      'editor.mod.export',
    ]);
    expect(clientSettingsSemanticActions(registry).map((action) => action.id)).toEqual([
      'captain.red-alert',
      'captain.weapons-hold',
      'captain.view',
      'captain.objective-priority',
      HELM_STEERING_ACTION_ID,
      'tactical.target-selection',
      'tactical.phaser-mode',
      'tactical.phaser-fire',
      'tactical.blaster-charge',
      'tactical.blaster-fire',
      'tactical.blaster-cancel',
      'tactical.torpedo-volley-down',
      'tactical.torpedo-volley-up',
      'tactical.torpedo-fire',
      'comms.hail',
      'comms.select-message',
      'comms.respond',
      'comms.clear',
      'comms.show-on-screen',
      'sensors.target-selection',
      'sensors.scan',
      'sensors.viewscreen',
      'sensors.cancel-impulse',
      'science.shield-focus',
    ]);
  });
});
