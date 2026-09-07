// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  GM_CONFIRMATION_CATEGORIES, GM_ACTION_CONFIRMATION_METADATA, createGmConfirmationController,
  createGmConfirmationProfile, gmConfirmationMetadata,
} from '../../gui/gm-confirmation.js';
import { OPERATOR_PROFILE_KEY } from '../../gui/operator-profile.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import { renderGmConfirmationSettings } from '../../gui/gm-confirmation-settings.js';

function storage() {
  const values = new Map();
  return { getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value) };
}
afterEach(() => { document.body.replaceChildren(); });

describe('private GM confirmation policies', () => {
  it('rejects missing semantic metadata and covers every actual canonical action', () => {
    const definition = { id: 'gm.new-action', contexts: ['gm'],
      labelId: 'test.label', accessibilityLabelId: 'test.accessibility' };
    const registry = createSemanticActionRegistry();
    expect(() => registry.register(definition, () => true)).toThrow(/metadata/);
    expect(() => registry.register({ ...definition, ...gmConfirmationMetadata('effect.damage') }, () => true)).not.toThrow();
    const source = readFileSync('src/gm_action.rs', 'utf8');
    const body = source.split('pub enum GmAction {')[1].split(/\r?\n}\r?\n/)[0];
    const variants = [...body.matchAll(/^ {4}(\w+) \{/gm)].map((match) => match[1]);
    expect(variants.length).toBeGreaterThan(0);
    expect(Object.keys(GM_ACTION_CONFIRMATION_METADATA).sort()).toEqual(variants.sort());
  });

  it('keeps existing console remaps when a GM edits and exports confirmation choices', () => {
    const disk = storage();
    const client = createClientSemanticActionRegistry();
    const action = client.list().find((entry) => !entry.continuous);
    const original = createGmConfirmationProfile({ storage: disk, registry: client });
    client.setBinding(action.id, 0, null);
    client.setBinding(action.id, 1, null);
    original.setMode('effect.damage', 'immediate');
    const gm = createGmConfirmationProfile({ storage: disk });
    gm.setMode('effect.lethal', 'confirm');
    expect(JSON.parse(gm.exportProfile()).bindings[action.id]).toEqual([null, null]);
  });

  it('renders the live matrix in Settings and persists a control change', () => {
    const disk = storage();
    const profile = createGmConfirmationProfile({ storage: disk });
    renderGmConfirmationSettings({ doc: document, target: document.body, profile, t: (id) => id });
    const selector = document.querySelector('[data-gm-confirmation-category="effect.lethal"]');
    expect(selector.value).toBe('confirm-preview');
    selector.value = 'immediate';
    selector.dispatchEvent(new Event('change'));
    expect(createGmConfirmationProfile({ storage: disk }).mode('effect.lethal')).toBe('immediate');
    expect(document.querySelectorAll('[data-gm-confirmation-category]')).toHaveLength(GM_CONFIRMATION_CATEGORIES.length);
  });

  it('exports and imports the matrix through the actual Settings file controls', async () => {
    const first = createGmConfirmationProfile({ storage: storage() });
    first.setMode('effect.damage', 'immediate');
    const download = vi.fn(() => true);
    renderGmConfirmationSettings({ doc: document, target: document.body, profile: first,
      t: (id) => id, download });
    document.querySelector('[data-gm-profile-export]').click();
    const exported = download.mock.calls[0][2];
    document.body.replaceChildren();
    const second = createGmConfirmationProfile({ storage: storage() });
    renderGmConfirmationSettings({ doc: document, target: document.body, profile: second,
      t: (id) => id, read: async () => exported });
    const input = document.querySelector('[data-gm-profile-import]');
    Object.defineProperty(input, 'files', { value: [new File([exported], 'profile.json')] });
    input.dispatchEvent(new Event('change'));
    await vi.waitFor(() => expect(document.querySelector('[data-gm-confirmation-category="effect.damage"]').value).toBe('immediate'));
    expect(second.mode('effect.damage')).toBe('immediate');
  });
  it('offers all three modes for each category and round-trips private choices', () => {
    const disk = storage();
    const a = createGmConfirmationProfile({ storage: disk });
    for (const entry of GM_CONFIRMATION_CATEGORIES) {
      expect(a.mode(entry.id)).toBe(entry.defaultMode);
      for (const mode of ['immediate', 'confirm', 'confirm-preview']) {
        expect(a.setMode(entry.id, mode).status).toBe('saved');
        expect(a.mode(entry.id)).toBe(mode);
      }
    }
    const exported = a.exportProfile();
    const b = createGmConfirmationProfile({ storage: storage() });
    expect(b.importProfile(exported).status).not.toBe('rejected');
    expect(b.exportProfile()).toBe(exported);
    expect(createGmConfirmationProfile({ storage: disk }).mode('effect.damage')).toBe('confirm-preview');
    expect(Object.keys(JSON.parse(exported))).toEqual([
      'kind', 'version', 'accessibility', 'bindings', 'gamepad', 'feedback', 'gmConfirmations',
    ]);
  });

  it('keeps two live operators independent and rejects missing category metadata', () => {
    const a = createGmConfirmationProfile({ storage: storage() });
    const b = createGmConfirmationProfile({ storage: storage() });
    a.setMode('effect.damage', 'immediate');
    expect(a.mode('effect.damage')).toBe('immediate');
    expect(b.mode('effect.damage')).toBe('confirm');
    expect(() => gmConfirmationMetadata('unknown.action')).toThrow(/Unregistered/);
    expect(() => a.setMode('unknown.action', 'immediate')).toThrow(/Unregistered/);
    expect(a.setMode('effect.damage', 'unknown').status).toBe('rejected');
  });

  it('does not apply rejected imports or claim a failed save changed the policy', () => {
    const disk = storage();
    const profile = createGmConfirmationProfile({ storage: disk });
    profile.setMode('effect.damage', 'immediate');
    const original = disk.getItem(OPERATOR_PROFILE_KEY);
    expect(profile.importProfile('{bad').status).toBe('rejected');
    expect(disk.getItem(OPERATOR_PROFILE_KEY)).toBe(original);
    disk.setItem = () => { throw new Error('full'); };
    expect(profile.setMode('effect.damage', 'confirm').status).toBe('rejected');
    expect(profile.mode('effect.damage')).toBe('immediate');
  });
});

describe('confirmation before the ordinary action lifecycle', () => {
  it.each(['immediate', 'confirm', 'confirm-preview'])('%s invokes only the accepted intent', (mode) => {
    const profile = createGmConfirmationProfile({ storage: storage() });
    profile.setMode('effect.damage', mode);
    const controller = createGmConfirmationController({ doc: document, profile });
    const accept = vi.fn(() => true);
    controller.request({ category: 'effect.damage', description: 'Damage Courier',
      preview: () => 'Courier survives', accept });
    if (mode === 'immediate') {
      expect(accept).toHaveBeenCalledOnce();
      expect(controller.isOpen()).toBe(false);
    } else {
      expect(accept).not.toHaveBeenCalled();
      expect(document.querySelector('[data-confirmation-preview]').hidden).toBe(mode !== 'confirm-preview');
      document.querySelector('[data-confirmation-accept]').click();
      expect(accept).toHaveBeenCalledOnce();
      document.querySelector('[data-confirmation-accept]').click();
      expect(accept).toHaveBeenCalledOnce();
    }
    controller.destroy();
  });

  it('cancels without sending, restores focus and updates an advisory stale preview', () => {
    const opener = document.createElement('button');
    document.body.append(opener);
    opener.focus();
    const profile = createGmConfirmationProfile({ storage: storage() });
    const controller = createGmConfirmationController({ doc: document, profile });
    const accept = vi.fn();
    let consequence = 'Would destroy Courier';
    const intent = { category: 'effect.lethal', description: 'Damage Courier',
      preview: () => consequence, accept };
    controller.request(intent);
    consequence = 'Courier now survives';
    controller.refresh();
    expect(document.querySelector('[data-confirmation-preview]').textContent).toBe(consequence);
    document.querySelector('[data-confirmation-cancel]').click();
    expect(accept).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(opener);
    controller.request(intent);
    expect(controller.request({ ...intent, accept: vi.fn() })).toBe(false);
    document.querySelector('[data-confirmation-accept]').click();
    expect(accept).toHaveBeenCalledOnce();
    controller.destroy();
  });

  it('keeps only the latest continuous intent while confirmation is open', () => {
    const profile = createGmConfirmationProfile({ storage: storage() });
    profile.setMode('station.command', 'confirm');
    const controller = createGmConfirmationController({ doc: document, profile });
    const send = vi.fn();
    const cancelled = vi.fn();
    controller.request({ category: 'station.command', key: 'ship/helm/thrust',
      description: 'thrust', accept: () => send(1), onCancel: cancelled });
    controller.request({ category: 'station.command', key: 'ship/helm/thrust',
      description: 'neutral', accept: () => send(0) });
    expect(cancelled).toHaveBeenCalledOnce();
    expect(send).not.toHaveBeenCalled();
    document.querySelector('[data-confirmation-accept]').click();
    expect(send).toHaveBeenCalledExactlyOnceWith(0);
    controller.destroy();
  });
});
