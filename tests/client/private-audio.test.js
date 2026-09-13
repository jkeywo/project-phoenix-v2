import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createPrivateAudio, privateFeedbackReceiver } from '../../gui/private-audio.js';
import { createPrivateRequestFeedback } from '../../gui/private-request-feedback.js';
import { createGmConfirmationProfile } from '../../gui/gm-confirmation.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { ACCESSIBILITY_PROFILE_KEY } from '../../gui/accessibility-profile.js';
import { normalizePrivateAudio, LEGACY_PRIVATE_MASTER_KEY } from '../../gui/private-audio-preferences.js';
import { createDefaultOperatorProfile, loadOperatorProfile, OPERATOR_PROFILE_KEY,
  serializeOperatorProfile, prepareOperatorProfileImport } from '../../gui/operator-profile.js';
import { FakeAudioContext, audioFetch, memoryStorage, settleAudio } from './audio-fixtures.js';

const manifest = JSON.parse(readFileSync(new URL('../../assets/audio/private-feedback.json', import.meta.url), 'utf8'));
const transition = (state, correlation = 'one', actionId = 'captain.red-alert') =>
  ({ state, correlation, actionId, lifecycleTransition: true });
async function fixture(options = {}) {
  const context = new FakeAudioContext({ state: 'running', ...options.context });
  let profile = normalizePrivateAudio(options.preferences), clock = 1000;
  const audio = createPrivateAudio({ manifest, contextFactory: () => context,
    fetchAudio: audioFetch(), read: () => profile, now: () => clock,
    save: next => { profile = next; return { status: 'saved' }; }, ...options });
  await audio.ready; await settleAudio();
  return { audio, context, advance: () => { clock += 101; }, profile: () => profile };
}
function accepted(audio, correlation = 'one', metadata) {
  audio.action(transition('Pressed', correlation), metadata);
  audio.action(transition('Pending', correlation), metadata);
}

describe('private local audio through the production output graph', () => {
  it('uses only private assets and multiplies authored level by Interface and Master', async () => {
    const { audio, context } = await fixture();
    audio.setBus('master', { level: 0.4 }); audio.setBus('interface', { level: 0.5 });
    expect(audio.state().categories).toEqual(['alerts', 'interface']);
    expect(audio.state().buses).toEqual(['master', 'alerts', 'interface']);
    expect(audio.state().ready).not.toContain('music');
    audio.action(transition('Pressed'));
    expect(context.sample()).toBe(0);
    audio.action(transition('Pending'));
    expect(context.sample()).toBeCloseTo(0.5 * manifest.sounds.clicks.volume * 0.4 * 0.5);
    audio.action(transition('Applied')); // opt-in only
    expect(audio.state().active.map(v => v.id)).toEqual(['clicks']);
    audio.setBus('master', { muted: true }); expect(context.sample()).toBe(0);
    expect(audio.actionable()).toBe(false);
    audio.dispose();
  });
  it('consumes duplicates, restored presentation, unowned results and continuous updates silently', async () => {
    const { audio, context, advance } = await fixture();
    audio.action(transition('Refused', 'foreign'));
    accepted(audio, 'held', { hold: true }); audio.action(transition('Refused', 'held'));
    accepted(audio, 'axis', { continuous: true }); audio.action(transition('TimedOut', 'axis'));
    audio.action({ ...transition('Pending'), presentationRestored: true });
    expect(context.sample()).toBe(0);
    accepted(audio); audio.action(transition('Refused'));
    const count = context.sources.length;
    advance(); accepted(audio); audio.action(transition('Refused'));
    audio.action({ ...transition('Applied'), lifecycleTransition: false });
    expect(context.sources).toHaveLength(count);
    audio.dispose();
  });
  it('accepts only the live source window and drops late results from replaced iframes', async () => {
    const { audio, context, advance } = await fixture();
    const old = {}, current = {}, other = {}; let source = old;
    const receive = privateFeedbackReceiver({ getAudio: () => audio, currentSource: () => source });
    receive(other, transition('Pressed')); receive(other, transition('Pending'));
    expect(context.sample()).toBe(0);
    receive(old, transition('Pressed')); receive(old, transition('Pending'));
    context.advance(11); source = current; advance();
    receive(old, transition('Refused'));
    receive(current, transition('Refused', 'unowned'));
    expect(context.sample()).toBe(0);
    receive(current, transition('Pressed', 'new')); receive(current, transition('Pending', 'new'));
    expect(context.sample()).toBeGreaterThan(0); audio.dispose();
  });
  it('coalesces bursts and supports opted-in Pending and Applied without doubling the click', async () => {
    const { audio, advance } = await fixture();
    audio.setCue('pending', true); audio.setCue('applied', true);
    accepted(audio); accepted(audio, 'two'); accepted(audio, 'three');
    expect(audio.state().active.map(v => v.id)).toEqual(['pending']);
    audio.action(transition('Applied')); audio.action(transition('Applied', 'two'));
    expect(audio.state().active.map(v => v.id)).toEqual(['pending', 'applied']);
    advance(); accepted(audio, 'four');
    expect(audio.state().active.filter(v => v.id === 'pending')).toHaveLength(2);
    audio.dispose();
  });
  it('does not replay blocked or hidden transitions after unlock, return or test', async () => {
    const { audio, context, advance } = await fixture({ context: { state: 'suspended', refuseResume: true } });
    accepted(audio); audio.action(transition('Refused'));
    expect(context.sample()).toBe(0); expect(await audio.enable()).toBe(false);
    context.refuseResume = false; expect(await audio.enable()).toBe(true);
    expect(context.sample()).toBe(0);
    advance(); accepted(audio, 'two'); audio.setActive(false);
    expect(context.sample()).toBe(0); audio.setActive(true);
    audio.action(transition('TimedOut', 'two')); expect(context.sample()).toBe(0);
    expect(await audio.testOutput()).toBe(true);
    audio.setBus('interface', { muted: true }); expect(context.sample()).toBe(0);
    context.advance(2); expect(audio.state().active).toHaveLength(0);
    expect(await audio.testOutput()).toBe(false); audio.dispose();
  });
  it('keeps native output unavailable without its explicit private adapter even with browser-shaped audio APIs', async () => {
    const contextFactory = vi.fn(() => new FakeAudioContext());
    const audio = createPrivateAudio({ manifest, root: { PhoenixOperatorCapabilities: { surface: 'native-pane' } }, contextFactory });
    await audio.ready; accepted(audio);
    expect(contextFactory).not.toHaveBeenCalled(); expect(audio.state().status).toBe('unavailable');
    expect(await audio.testOutput()).toBe(false); audio.dispose();
  });
  it('retains in-memory mix after failed persistence and discards a failed provider without throwing into sends', async () => {
    const { audio, context } = await fixture({ save: () => { throw new Error('storage unavailable'); } });
    expect(() => audio.setBus('master', { level: 0 })).not.toThrow(); accepted(audio);
    expect(context.sample()).toBe(0); expect(audio.state().persistence).toBe('unavailable'); audio.dispose();
    const failed = createPrivateAudio({ manifest, providerFactory: () => { throw new Error('device'); } });
    await failed.ready; expect(() => accepted(failed)).not.toThrow(); expect(await failed.testOutput()).toBe(false); failed.dispose();
  });
});

describe('portable private preferences and migration', () => {
  it.each(['0', '0.12', '0.25old'])('migrates legacy Master %s without raising the old effective level', value => {
    const storage = memoryStorage({ [LEGACY_PRIVATE_MASTER_KEY]: value });
    const profile = loadOperatorProfile(storage).profile;
    expect(profile.audio.mix.master.level).toBe(parseFloat(value));
    expect(JSON.parse(storage.getItem(OPERATOR_PROFILE_KEY)).audio).toEqual(profile.audio);
    expect(storage.getItem(LEGACY_PRIVATE_MASTER_KEY)).toBeNull();
  });
  it('preserves current audio over a stale legacy key and preserves that key if migration cannot be saved', () => {
    const current = createDefaultOperatorProfile(); current.audio.mix.master.level = 0.07;
    const storage = memoryStorage({ [OPERATOR_PROFILE_KEY]: serializeOperatorProfile(current), [LEGACY_PRIVATE_MASTER_KEY]: '0.8' });
    expect(loadOperatorProfile(storage, { registry: createClientSemanticActionRegistry() }).profile.audio.mix.master.level).toBe(0.07);
    const failed = memoryStorage({ [LEGACY_PRIVATE_MASTER_KEY]: '0' }); failed.setItem = () => { throw new Error('quota'); };
    expect(loadOperatorProfile(failed).profile.audio.mix.master.level).toBe(0);
    expect(failed.getItem(LEGACY_PRIVATE_MASTER_KEY)).toBe('0');
  });
  it('migrates Accessibility and quiet Master in one durable write', () => {
    const storage = memoryStorage({ [LEGACY_PRIVATE_MASTER_KEY]: '0.04',
      [ACCESSIBILITY_PROFILE_KEY]: JSON.stringify({ presentation: { textScale: 1.5 }, assistance: {} }) });
    const write = storage.setItem; let writes = 0;
    storage.setItem = (key, value) => { if (++writes > 1) throw new Error('quota'); write(key, value); };
    const options = { registry: createClientSemanticActionRegistry() };
    expect(loadOperatorProfile(storage, options).profile.audio.mix.master.level).toBe(0.04);
    expect(writes).toBe(1);
    expect(loadOperatorProfile(storage, options).profile.audio.mix.master.level).toBe(0.04);
  });
  it('imports/exports only private preferences, retaining audio across GM confirmation changes', () => {
    const storage = memoryStorage(); const owner = createGmConfirmationProfile({ storage });
    const audio = normalizePrivateAudio({ mix: { master: { level: 0.3, muted: true } }, cues: { applied: true } });
    audio.hardware_output = 'private-headphones'; audio.occurrences = ['secret']; audio.mix.music = { level: 1 };
    owner.setAudio(audio); owner.setMode('event.fire', 'immediate');
    const exported = owner.exportProfile();
    expect(exported).not.toContain('headphones'); expect(exported).not.toContain('secret');
    const prepared = prepareOperatorProfileImport(exported, { registry: createClientSemanticActionRegistry() });
    expect(prepared.profile.audio.mix.master).toEqual({ level: 0.3, muted: true });
    expect(prepared.profile.audio.mix.music).toEqual({ level: 1, muted: false });
    expect(prepared.profile.audio.cues.applied).toBe(true);
    expect(owner.importProfile('{broken').status).toBe('rejected');
    expect(owner.audio().mix.master.level).toBe(0.3);
  });
});

describe('correlated typed GM request feedback', () => {
  it('settles only this operator and correlation, expires once, and resets without replay', async () => {
    const { audio, advance } = await fixture(); const timers = new Map(); let seq = 0;
    const requests = createPrivateRequestFeedback({ audio, getOperator: () => ({ id: 'gm-one' }),
      schedule: callback => { timers.set(++seq, callback); return seq; }, cancelSchedule: id => timers.delete(id) });
    const send = vi.fn(() => true);
    requests.submit('gm.presentation', { correlation: 'present' }, send);
    expect(send).toHaveBeenCalledOnce();
    requests.settle([{ operator_id: 'gm-other', correlation: 'present', outcome: 'refused' }]);
    expect(audio.state().active.map(v => v.id)).toEqual(['clicks']);
    requests.settle([{ operator_id: 'gm-one', correlation: 'present', outcome: 'refused' }]);
    expect(audio.state().active.map(v => v.id)).toEqual(['clicks', 'refused']); expect(timers.size).toBe(0);
    advance(); requests.submit('gm.contact', { correlation: 'contact' }, send);
    for (const callback of [...timers.values()]) callback();
    expect(audio.state().active.map(v => v.id)).toContain('timedOut');
    advance(); requests.submit('gm.restore', { correlation: 'restore' }, send); requests.reset();
    expect(timers.size).toBe(0); audio.dispose();
  });
});
