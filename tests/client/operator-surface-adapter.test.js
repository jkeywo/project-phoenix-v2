import { describe, expect, it } from 'vitest';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import {
  createDefaultOperatorProfile,
  prepareOperatorProfileImport,
  serializeOperatorProfile,
} from '../../gui/operator-profile.js';
import {
  OPERATOR_SURFACE_KIND,
  applyOperatorProfileToSurface,
  detectOperatorCapabilities,
} from '../../gui/operator-surface-adapter.js';

const BROWSER_CAPABILITIES = Object.freeze({
  surface: OPERATOR_SURFACE_KIND.BROWSER,
  keyboard: true,
  gamepad: true,
  vibration: true,
  semanticCues: true,
  accessibility: true,
});

const NATIVE_CAPABILITIES = Object.freeze({
  surface: OPERATOR_SURFACE_KIND.NATIVE_PANE,
  keyboard: true,
  gamepad: false,
  vibration: false,
  semanticCues: true,
  accessibility: true,
});

function portableProfile() {
  const registry = createClientSemanticActionRegistry();
  const profile = createDefaultOperatorProfile(registry);
  profile.accessibility.presentation = {
    textScale: 1.3,
    contrast: 'on',
    reducedMotion: 'off',
  };
  profile.bindings['captain.red-alert'][0] = {
    type: 'keyboard', code: 'KeyY',
    ctrlKey: false, shiftKey: true, altKey: false, metaKey: false,
  };
  profile.gamepad.preferredSlot = 2;
  profile.gamepad.tuning['helm.steering'] = { deadzone: 0.24, inverted: true };
  profile.feedback = { vibration: true, semanticCues: false };
  return serializeOperatorProfile(profile);
}

describe('operator profile surface adapter', () => {
  it('applies one imported schema to browser and native without losing portable choices', () => {
    const json = portableProfile();
    const browserRegistry = createClientSemanticActionRegistry();
    const nativeRegistry = createClientSemanticActionRegistry();
    const browserPrepared = prepareOperatorProfileImport(json, { registry: browserRegistry });
    const nativePrepared = prepareOperatorProfileImport(json, { registry: nativeRegistry });

    const browser = applyOperatorProfileToSurface(
      browserPrepared.profile,
      browserRegistry,
      { capabilities: BROWSER_CAPABILITIES },
    );
    const native = applyOperatorProfileToSurface(
      nativePrepared.profile,
      nativeRegistry,
      { capabilities: NATIVE_CAPABILITIES },
    );

    expect(browser.status).toBe('applied');
    expect(native.status).toBe('applied');
    expect(nativeRegistry.bindingProfile()).toEqual(browserRegistry.bindingProfile());
    expect(nativeRegistry.tuningProfile()).toEqual(browserRegistry.tuningProfile());
    expect(native.active.accessibility).toEqual(browser.active.accessibility);
    expect(native.active.feedback).toEqual({ vibration: false, semanticCues: false });
    expect(browser.active.feedback).toEqual({ vibration: true, semanticCues: false });
    expect(native.active.preferredGamepadSlot).toBeNull();
    expect(browser.active.preferredGamepadSlot).toBe(2);
    expect(native.unavailable).toEqual(['gamepad', 'vibration']);

    // Capability filtering is an active projection only. Re-exporting either
    // retained profile produces the same versioned JSON, including device and
    // vibration choices the native pane could not use.
    expect(serializeOperatorProfile(native.portableProfile)).toBe(json);
    expect(serializeOperatorProfile(browser.portableProfile)).toBe(json);
  });

  it('treats the native declaration as authoritative over browser-shaped stubs', () => {
    const root = {
      navigator: { getGamepads() {}, vibrate() {} },
      PhoenixOperatorCapabilities: {
        surface: 'native-pane', keyboard: true, gamepad: false,
        vibration: false, semanticCues: true, accessibility: true,
      },
    };
    expect(detectOperatorCapabilities(root)).toEqual(NATIVE_CAPABILITIES);
  });

  it('detects browser gamepad and vibration APIs without a native declaration', () => {
    const capable = detectOperatorCapabilities({
      navigator: { getGamepads() {}, vibrate() {} },
    });
    const limited = detectOperatorCapabilities({ navigator: {} });
    expect(capable).toMatchObject({
      surface: 'browser', gamepad: true, vibration: true,
    });
    expect(limited).toMatchObject({
      surface: 'browser', gamepad: false, vibration: false,
    });
  });
});
