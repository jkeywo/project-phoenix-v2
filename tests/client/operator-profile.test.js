import { describe, it, expect } from 'vitest';
import {
  ACCESSIBILITY_PROFILE_KEY,
  ASSIST_REQUEST,
  emptyAccessibilityProfile,
} from '../../gui/accessibility-profile.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import {
  FEEDBACK_PREFERENCE_DEFAULTS,
  OPERATOR_PROFILE_KEY,
  OPERATOR_PROFILE_KIND,
  OPERATOR_PROFILE_VERSION,
  applyOperatorProfile,
  createDefaultOperatorProfile,
  createOperatorProfileSnapshot,
  loadOperatorProfile,
  prepareOperatorProfileImport,
  saveOperatorProfile,
  serializeOperatorProfile,
} from '../../gui/operator-profile.js';

function fakeStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => map.set(key, String(value)),
    map,
  };
}

function currentProfile(registry = createClientSemanticActionRegistry()) {
  registry.setBinding('captain.red-alert', 0, { code: 'KeyY', shiftKey: true });
  registry.setTuning('helm.steering', { deadzone: 0.25, inverted: true });
  return createOperatorProfileSnapshot({
    accessibility: {
      presentation: { textScale: 1.35, contrast: 'on', reducedMotion: 'off' },
      assistance: { 'helm.course-keeping': ASSIST_REQUEST },
    },
    bindings: registry.bindingProfile(),
    preferredGamepadSlot: 3,
    tuning: registry.tuningProfile(),
    feedback: { vibration: false, semanticCues: true },
    gmConfirmations: {
      'event.fire': 'confirm',
      'entity.safe-despawn': 'confirm-preview',
    },
  });
}

describe('private operator profile schema', () => {
  it('round-trips every delivered private setting through current JSON', () => {
    const sourceRegistry = createClientSemanticActionRegistry();
    const profile = currentProfile(sourceRegistry);
    const json = serializeOperatorProfile(profile);
    const targetRegistry = createClientSemanticActionRegistry();
    const prepared = prepareOperatorProfileImport(json, { registry: targetRegistry });

    expect(prepared.status).toBe('imported');
    expect(prepared.diagnostics).toEqual([]);
    expect(prepared.profile).toMatchObject({
      kind: OPERATOR_PROFILE_KIND,
      version: OPERATOR_PROFILE_VERSION,
      accessibility: profile.accessibility,
      gamepad: {
        preferredSlot: 3,
        tuning: { 'helm.steering': { deadzone: 0.25, inverted: true } },
      },
      feedback: { vibration: false, semanticCues: true },
      gmConfirmations: {
        'event.fire': 'confirm',
        'entity.safe-despawn': 'confirm-preview',
      },
    });
    expect(prepared.profile.bindings['captain.red-alert']).toEqual([
      {
        type: 'keyboard', code: 'KeyY', ctrlKey: false, shiftKey: true,
        altKey: false, metaKey: false,
      },
      { type: 'gamepad', input: 'button', control: 'face-bottom' },
    ]);
    expect(applyOperatorProfile(prepared.profile, targetRegistry).status).toBe('applied');
    expect(targetRegistry.bindingProfile()).toEqual(prepared.profile.bindings);
    expect(targetRegistry.tuningProfile()).toEqual(prepared.profile.gamepad.tuning);
  });

  it('has authored action defaults, exactly two slots, and sparse future settings', () => {
    const profile = createDefaultOperatorProfile(createClientSemanticActionRegistry());
    expect(profile.feedback).toEqual(FEEDBACK_PREFERENCE_DEFAULTS);
    expect(profile.gmConfirmations).toEqual({});
    expect(profile.gamepad.preferredSlot).toBeNull();
    for (const slots of Object.values(profile.bindings)) expect(slots).toHaveLength(2);
  });

  it('migrates the preceding Accessibility-only shape', () => {
    const registry = createClientSemanticActionRegistry();
    const legacy = {
      presentation: { textScale: 1.4, contrast: 'on', reducedMotion: 'default' },
      assistance: { 'sensors.contact-triage': ASSIST_REQUEST },
    };
    const result = prepareOperatorProfileImport(JSON.stringify(legacy), { registry });
    expect(result.status).toBe('migrated');
    expect(result.profile.accessibility).toEqual(legacy);
    expect(result.profile.feedback).toEqual(FEEDBACK_PREFERENCE_DEFAULTS);
    expect(result.profile.gamepad.preferredSlot).toBeNull();
    expect(result.profile.bindings).toEqual(registry.bindingProfile());
  });

  it('normalizes bounded preferences and reports every unsupported class', () => {
    const registry = createClientSemanticActionRegistry();
    const profile = createDefaultOperatorProfile(registry);
    profile.gamepad.preferredSlot = 999;
    profile.feedback = { vibration: 'yes', semanticCues: false };
    profile.gmConfirmations = {
      '__proto__': 'confirm',
      'event.fire': 'sometimes',
      'event.skip': 'confirm',
    };
    profile.identity = { token: 'secret' };
    profile.station = 'captain';
    profile.saves = [{ secret: true }];

    const result = prepareOperatorProfileImport(JSON.stringify(profile), { registry });
    expect(result.status).toBe('imported');
    expect(result.profile.gamepad.preferredSlot).toBeNull();
    expect(result.profile.feedback).toEqual({ vibration: true, semanticCues: false });
    expect(result.profile.gmConfirmations).toEqual({ 'event.skip': 'confirm' });
    expect(result.diagnostics.map((entry) => entry.code)).toEqual(expect.arrayContaining([
      'private-or-unsupported-fields-ignored',
      'gamepad-preference-normalized',
      'feedback-normalized',
      'gm-confirmation-ignored',
    ]));
  });

  it('never coerces malformed preferred gamepad slots into device ownership', () => {
    const registry = createClientSemanticActionRegistry();
    for (const malformed of [false, true, '0', '3', 1.5, -1, 999]) {
      const profile = createDefaultOperatorProfile(registry);
      profile.gamepad.preferredSlot = malformed;
      const result = prepareOperatorProfileImport(JSON.stringify(profile), { registry });
      expect(result.status).toBe('imported');
      expect(result.profile.gamepad.preferredSlot).toBeNull();
      expect(result.diagnostics).toContainEqual(expect.objectContaining({
        code: 'gamepad-preference-normalized',
      }));
    }

    const valid = createDefaultOperatorProfile(registry);
    valid.gamepad.preferredSlot = 0;
    const result = prepareOperatorProfileImport(JSON.stringify(valid), { registry });
    expect(result.profile.gamepad.preferredSlot).toBe(0);
    expect(result.diagnostics).not.toContainEqual(expect.objectContaining({
      code: 'gamepad-preference-normalized',
    }));
  });
});

describe('safe and atomic import', () => {
  it('rejects corrupt JSON, non-records, and unknown versions explicitly', () => {
    const registry = createClientSemanticActionRegistry();
    expect(prepareOperatorProfileImport('{oops', { registry }))
      .toMatchObject({ status: 'rejected', code: 'profile-json' });
    expect(prepareOperatorProfileImport('[]', { registry }))
      .toMatchObject({ status: 'rejected', code: 'profile-shape' });
    expect(prepareOperatorProfileImport(JSON.stringify({
      kind: OPERATOR_PROFILE_KIND, version: 77,
    }), { registry })).toMatchObject({ status: 'rejected', code: 'profile-version' });
  });

  it('rejects a reserved binding without changing any live setting', () => {
    const registry = createClientSemanticActionRegistry();
    registry.setBinding('captain.red-alert', 0, { code: 'KeyY' });
    registry.setTuning('helm.steering', { deadzone: 0.2, inverted: true });
    const beforeBindings = registry.bindingProfile();
    const beforeTuning = registry.tuningProfile();
    const profile = createDefaultOperatorProfile(registry);
    profile.bindings['captain.red-alert'][0] = {
      type: 'keyboard', code: 'KeyR', ctrlKey: true,
      shiftKey: false, altKey: false, metaKey: false,
    };

    const result = prepareOperatorProfileImport(JSON.stringify(profile), { registry });
    expect(result).toMatchObject({ status: 'rejected', code: 'binding-reserved' });
    expect(registry.bindingProfile()).toEqual(beforeBindings);
    expect(registry.tuningProfile()).toEqual(beforeTuning);
  });

  it('rejects overlapping conflicts and malformed slot counts atomically', () => {
    for (const mutate of [
      (profile) => { profile.bindings['captain.red-alert'][0] = { code: 'KeyH' }; },
      (profile) => { profile.bindings['captain.red-alert'] = [{ code: 'KeyY' }]; },
    ]) {
      const registry = createClientSemanticActionRegistry();
      const before = registry.bindingProfile();
      const profile = createDefaultOperatorProfile(registry);
      mutate(profile);
      const result = prepareOperatorProfileImport(JSON.stringify(profile), { registry });
      expect(result.status).toBe('rejected');
      expect(result.code).toMatch(/binding-(conflict|slot-count)/);
      expect(registry.bindingProfile()).toEqual(before);
    }
  });

  it('preserves imported remaps over defaults for actions added later', () => {
    const registry = createClientSemanticActionRegistry();
    const profile = createDefaultOperatorProfile(registry);
    profile.bindings['captain.red-alert'][0] = { code: 'KeyH' };
    delete profile.bindings['captain.weapons-hold'];

    const result = prepareOperatorProfileImport(JSON.stringify(profile), { registry });
    expect(result.status).toBe('imported');
    expect(result.profile.bindings['captain.red-alert'][0]).toMatchObject({
      type: 'keyboard', code: 'KeyH',
    });
    expect(result.profile.bindings['captain.weapons-hold']).toEqual([null, null]);
    expect(result.diagnostics).toContainEqual({
      code: 'new-action-defaults-cleared', count: 1,
    });
    expect(applyOperatorProfile(result.profile, registry).status).toBe('applied');
    expect(registry.action('captain.red-alert').bindings[0].code).toBe('KeyH');
    expect(registry.action('captain.weapons-hold').bindings).toEqual([null, null]);
  });

  it('ignores unknown and prototype-named action entries without pollution', () => {
    const registry = createClientSemanticActionRegistry();
    const profile = JSON.parse(serializeOperatorProfile(createDefaultOperatorProfile(registry)));
    Object.defineProperty(profile.bindings, '__proto__', {
      enumerable: true,
      value: [{ code: 'KeyZ' }, null],
    });
    profile.bindings['future.action'] = [{ code: 'KeyU' }, null];
    const result = prepareOperatorProfileImport(JSON.stringify(profile), { registry });
    expect(result.status).toBe('imported');
    expect(result.diagnostics).toContainEqual(expect.objectContaining({ code: 'future-actions-ignored' }));
    expect(result.profile.bindings).not.toHaveProperty('__proto__');
    expect(result.profile.bindings).not.toHaveProperty('future.action');
    expect({}.polluted).toBeUndefined();
  });
});

describe('private persistence and export boundary', () => {
  it('migrates legacy storage once and writes the current versioned key', () => {
    const registry = createClientSemanticActionRegistry();
    const legacy = {
      presentation: { textScale: 1.25, contrast: 'default', reducedMotion: 'on' },
      assistance: {},
    };
    const storage = fakeStorage({
      [ACCESSIBILITY_PROFILE_KEY]: JSON.stringify(legacy),
    });
    const loaded = loadOperatorProfile(storage, { registry });
    expect(loaded.status).toBe('migrated');
    expect(loaded.profile.accessibility).toEqual(legacy);
    expect(storage.map.has(OPERATOR_PROFILE_KEY)).toBe(true);
  });

  it('never falls back to legacy data behind a corrupt current record', () => {
    const registry = createClientSemanticActionRegistry();
    const storage = fakeStorage({
      [OPERATOR_PROFILE_KEY]: '{broken',
      [ACCESSIBILITY_PROFILE_KEY]: JSON.stringify({
        presentation: { textScale: 1.5, contrast: 'on', reducedMotion: 'on' },
      }),
    });
    const loaded = loadOperatorProfile(storage, { registry });
    expect(loaded).toMatchObject({ status: 'rejected', code: 'profile-json' });
    expect(loaded.profile.accessibility).toEqual(emptyAccessibilityProfile());
  });

  it('round-trips storage and reports read/write denial', () => {
    const registry = createClientSemanticActionRegistry();
    const profile = currentProfile(registry);
    const storage = fakeStorage();
    expect(saveOperatorProfile(storage, profile).status).toBe('saved');
    expect(loadOperatorProfile(storage, { registry }).profile).toEqual(profile);
    expect(saveOperatorProfile({ setItem() { throw new Error('quota'); } }, profile))
      .toMatchObject({ status: 'rejected', code: 'profile-storage-write' });
    expect(loadOperatorProfile({ getItem() { throw new Error('denied'); } }, { registry }))
      .toMatchObject({ status: 'rejected', code: 'profile-storage-read' });
  });

  it('exports only the approved schema, never identity, Station, session, or saves', () => {
    const registry = createClientSemanticActionRegistry();
    const profile = currentProfile(registry);
    profile.token = 'secret';
    profile.player = { name: 'Ada' };
    profile.station = 'captain';
    profile.session = { peer: 'x' };
    profile.saveCatalogue = ['slot'];
    profile.hardwareId = 'vendor-device';
    profile.connectionGeneration = 4;
    profile.pendingFeedback = { correlation: 'secret-pending' };
    const exported = JSON.parse(serializeOperatorProfile(profile));
    expect(Object.keys(exported).sort()).toEqual([
      'accessibility', 'bindings', 'feedback', 'gamepad', 'gmConfirmations',
      'kind', 'version',
    ]);
    expect(JSON.stringify(exported)).not.toMatch(
      /secret|"player"|"station"|"session"|saveCatalogue|hardware|generation|pendingFeedback/i,
    );
  });
});
