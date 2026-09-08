// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { createGamepadInputRuntime, matchPreferredGamepad, usableGamepadControls } from '../../gui/gamepad-input.js';
import { helmTouchVisibility, applyHelmTouchVisibility } from '../../gui/gamepad-presentation.js';
import { HELM_ACTIONS } from '../../gui/stations/helm-actions.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { createDefaultOperatorProfile, prepareOperatorProfileImport, serializeOperatorProfile } from '../../gui/operator-profile.js';

const pad = (index, id = 'controller A', extra = {}) => ({
  index, id, mapping: 'standard', axes: [0, 0, 0, 0],
  buttons: Array.from({ length: 16 }, () => ({ pressed: false, value: 0 })), ...extra,
});
const preference = { id: 'controller A', mapping: 'standard' };

describe('device preferences', () => {
  it('matches a controller after its slot changes and refuses ambiguous identical devices', () => {
    expect(matchPreferredGamepad([pad(0, 'other'), null, pad(2)], preference))
      .toEqual({ status: 'matched', index: 2 });
    expect(matchPreferredGamepad([pad(0), pad(1)], preference)).toEqual({ status: 'ambiguous' });
    expect(matchPreferredGamepad([pad(0, 'other')], preference)).toEqual({ status: 'disconnected' });
    expect(matchPreferredGamepad([pad(0, undefined, { available: false })], preference))
      .toEqual({ status: 'assigned' });
  });
  it('discovers a preconnected idle device from a snapshot arriving after console startup', () => {
    let snapshot = [];
    const requestSelection = vi.fn();
    const runtime = createGamepadInputRuntime({ getGamepads: () => snapshot, requestSelection });
    runtime.restoreDevice(preference);
    snapshot = [pad(0)];
    expect(runtime.poll()).toMatchObject({ connected: true, selectedIndex: 0 });
    expect(requestSelection).toHaveBeenLastCalledWith(0);
    // A newly opened/recreated console starts from the retained snapshot too.
    const recreated = createGamepadInputRuntime({ getGamepads: () => snapshot });
    expect(recreated.restoreDevice(preference)).toMatchObject({ connected: true, selectedIndex: 0 });
  });
  it('waits for native host ownership before publishing any controller input', () => {
    let snapshot = [pad(0, undefined, { nativeOwned: false })];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({ getGamepads: () => snapshot,
      getContext: () => 'helm', getActions: () => HELM_ACTIONS, activate });
    runtime.restoreDevice(preference);
    expect(runtime.poll()).toMatchObject({ connected: false, status: 'pending' });
    snapshot[0].axes[0] = 1;
    runtime.poll();
    expect(activate).not.toHaveBeenCalled();
    snapshot[0].nativeOwned = true;
    expect(runtime.poll().status).toBe('neutral');
    snapshot[0].axes[0] = 0;
    runtime.poll();
    snapshot[0].axes[0] = 1;
    runtime.poll();
    expect(activate).toHaveBeenCalledWith('helm.steering', expect.objectContaining({ value: 1 }));
  });
  it('retains the device through disconnect and never transfers to another device', () => {
    let snapshot = [pad(0)];
    const runtime = createGamepadInputRuntime({ getGamepads: () => snapshot });
    runtime.restoreDevice(preference);
    snapshot = [null, pad(1, 'other')];
    expect(runtime.poll()).toMatchObject({ connected: false, preferredDevice: preference });
    snapshot = [pad(0, 'other'), pad(1)];
    expect(runtime.poll()).toMatchObject({ connected: true, selectedIndex: 1 });
  });
  it('round trips the device and hide preference in existing operator JSON and defaults old profiles on', () => {
    const registry = createClientSemanticActionRegistry();
    const profile = createDefaultOperatorProfile(registry);
    profile.gamepad.preferredDevice = preference;
    profile.gamepad.hideTouchControls = false;
    const restored = prepareOperatorProfileImport(serializeOperatorProfile(profile), { registry });
    expect(restored.profile.gamepad).toMatchObject({ preferredDevice: preference, hideTouchControls: false });
    delete profile.gamepad.preferredDevice;
    delete profile.gamepad.hideTouchControls;
    expect(prepareOperatorProfileImport(JSON.stringify(profile), { registry }).profile.gamepad)
      .toMatchObject({ preferredDevice: null, hideTouchControls: true });
  });
});

describe('Helm touch presentation', () => {
  it('finds the shipped courier component IDs and preserves authored inline display rules', () => {
    const doc = document.implementation.createHTMLDocument();
    doc.body.innerHTML = '<ph-helm-joystick id="helm" style="display:flex"></ph-helm-joystick>'
      + '<ph-lateral-thrust-joystick id="lateral"></ph-lateral-thrust-joystick>';
    applyHelmTouchVisibility(doc, { joystick: false, lateral: false });
    expect(doc.getElementById('helm').style.display).toBe('none');
    expect(doc.getElementById('lateral').style.display).toBe('none');
    applyHelmTouchVisibility(doc, { joystick: true, lateral: true });
    expect(doc.getElementById('helm').style.display).toBe('flex');
    expect(doc.getElementById('lateral').style.display).toBe('');
  });
  it('keeps touch controls for missing physical inputs and nonstandard mappings', () => {
    const controls = usableGamepadControls(pad(0, undefined, { axes: [0], buttons: [] }));
    expect(helmTouchVisibility(HELM_ACTIONS, { connected: true, controls }))
      .toEqual({ joystick: true, lateral: true });
    expect(usableGamepadControls(pad(0, undefined, { mapping: '' }))).toEqual([]);
  });
  it('hides the joystick only with both its axes bound and treats lateral thrust independently', () => {
    expect(helmTouchVisibility(HELM_ACTIONS, { connected: true, controls: usableGamepadControls(pad(0)) })).toEqual({ joystick: false, lateral: false });
    const noSteering = HELM_ACTIONS.filter((action) => action.id !== 'helm.steering');
    expect(helmTouchVisibility(noSteering, { connected: true, controls: usableGamepadControls(pad(0)) })).toEqual({ joystick: true, lateral: false });
    const noLateral = HELM_ACTIONS.filter((action) => action.id !== 'helm.lateral-thrust');
    expect(helmTouchVisibility(noLateral, { connected: true, controls: usableGamepadControls(pad(0)) })).toEqual({ joystick: false, lateral: true });
  });
  it('restores controls on disconnect, an explicit preference, or unusable remaps', () => {
    expect(helmTouchVisibility(HELM_ACTIONS, { connected: false })).toEqual({ joystick: true, lateral: true });
    expect(helmTouchVisibility(HELM_ACTIONS, { connected: true, controls: usableGamepadControls(pad(0)) }, false)).toEqual({ joystick: true, lateral: true });
    expect(helmTouchVisibility(HELM_ACTIONS.map((action) => ({ ...action, bindings: [null, null] })),
      { connected: true, controls: usableGamepadControls(pad(0)) })).toEqual({ joystick: true, lateral: true });
  });
});
