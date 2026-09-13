import { describe, expect, it } from 'vitest';
import { createHostAudio } from '../../gui/host-audio.js';
import { createBrowserAudioProvider } from '../../gui/browser-audio-provider.js';
import { defaultAudioMix } from '../../gui/audio-mix.js';
import { AUDIO_CONFIG, FakeAudioContext, audioFetch, memoryStorage, settleAudio } from './audio-fixtures.js';

function host(options = {}) {
  const context = new FakeAudioContext(options.context);
  const equivalents = [];
  const audio = createHostAudio({ storage: memoryStorage(), contextFactory: () => context,
    fetchAudio: audioFetch(), onEquivalent: cue => equivalents.push(cue), ...options });
  return { audio, context, equivalents };
}
async function mission(audio, cfg = AUDIO_CONFIG) {
  audio.audioConfig(JSON.stringify(cfg));
  audio.startGameAudio();
  await settleAudio();
}
const hud = (extra = {}) => ({ red_alert: false, engine_thrust: 0, phaser_firing: false, ...extra });

describe('Viewscreen mix reaching actual provider output', () => {
  it.each(['info', 'advisory', 'warning', 'critical'])('plays the authored %s computer tone through Alerts', async severity => {
    const { audio, context } = host();
    await mission(audio, { computer_message: AUDIO_CONFIG.computer_message });
    audio.audioCue(JSON.stringify({ kind: 'computer_message', severity }));
    expect(context.sample()).toBeCloseTo(0.5 * AUDIO_CONFIG.computer_message[severity].volume);
    audio.setBus('alerts', { muted: true });
    expect(context.sample()).toBe(0);
    audio.dispose();
  });

  it('interruption discards active one-shots and tests, resuming only current loops', async () => {
    const { audio, context } = host();
    await mission(audio);
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 0, y: 0, z: -10 }));
    await audio.testOutput();
    expect(audio.state().active.some(voice => voice.test)).toBe(true);
    context.state = 'suspended'; context.onstatechange();
    expect(audio.state().active).toHaveLength(0);
    expect(audio.state().test).toBe('idle');
    await audio.enable();
    expect(audio.state().active.map(voice => voice.id)).toEqual(expect.arrayContaining(['ambient', 'engine']));
    expect(audio.state().active.some(voice => voice.id === 'blaster' || voice.test)).toBe(false);
    audio.dispose();
  });

  it('preserves authored levels and changes running and future sounds through category and Master gains', async () => {
    const { audio, context } = host();
    await mission(audio);
    audio.applyHudAudio(hud({ engine_thrust: 1, phaser_firing: true }));
    audio.audioLevel(0.2);
    expect(context.sample()).toBeCloseTo(0.5 * (0.25 + 0.25 + 0.5 + 0.2));
    audio.setBus('ambience', { level: 0.4 });
    audio.setMasterVolume(0.5);
    expect(context.sample()).toBeCloseTo(0.5 * 0.5 * ((0.25 + 0.25) * 0.4 + 0.5 + 0.2));
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 30, y: 5, z: -100 }));
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: -30, y: 0, z: -100 }));
    audio.audioCue(JSON.stringify({ kind: 'computer_message', severity: 'critical' }));
    audio.applyHudAudio(hud({ red_alert: true }));
    expect(audio.state().active.filter(voice => voice.id === 'blaster')).toHaveLength(2);
    expect(audio.state().active.map(voice => voice.id)).toContain('siren');
    expect(audio.state().active.map(voice => voice.id)).toContain('computer_critical');
    const beforeMute = context.sample();
    audio.setBus('master', { muted: true });
    expect(context.sample()).toBe(0);
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 1, y: 0, z: 0 }));
    expect(audio.state().active.filter(voice => voice.id === 'blaster')).toHaveLength(0);
    audio.setBus('master', { muted: false });
    expect(context.sample()).toBeGreaterThan(0);
    expect(context.sample()).toBeLessThan(beforeMute);
    expect(audio.state().active.some(voice => voice.id === 'siren' || voice.id === 'computer_critical')).toBe(false);
    expect(audio.getMasterVolume()).toBe(0.5);
    audio.setBus('effects', { muted: true });
    expect(context.sample()).toBeLessThan(beforeMute);
    audio.setMasterVolume(0);
    expect(context.sample()).toBe(0);
    audio.dispose();
  });

  it('lobby music and a deliberate bounded test use the same real Music bus and clean up', async () => {
    const { audio, context } = host();
    await settleAudio();
    expect(await audio.testOutput()).toBe(true);
    expect(audio.state().test).toBe('playing');
    expect(context.sample()).toBeCloseTo(0.25);
    audio.setBus('music', { level: 0.4 });
    expect(context.sample()).toBeCloseTo(0.1);
    audio.setBus('music', { muted: true });
    expect(context.sample()).toBe(0);
    context.advance(2);
    expect(audio.state().active).toHaveLength(0);
    expect(audio.state().test).toBe('idle');
    expect(await audio.testOutput()).toBe(false);
    audio.setBus('music', { muted: false });
    audio.startMenuMusic();
    await settleAudio();
    expect(context.sample()).toBeCloseTo(0.1);
    audio.stopMenuMusic();
    expect(context.sample()).toBe(0);
    audio.dispose();
  });

  it('drops locked cues; explicit enable resumes only current loops and failure does not escape input dispatch', async () => {
    const { audio, context, equivalents } = host({ context: { refuseResume: true } });
    await mission(audio);
    audio.applyHudAudio(hud());
    audio.applyHudAudio(hud({ red_alert: true }));
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 2, y: 0, z: -1 }));
    expect(audio.state().status).toBe('blocked');
    expect(context.sample()).toBe(0);
    expect(equivalents).toContainEqual({ kind: 'blaster', x: 2, y: 0, z: -1 });
    context.refuseResume = false;
    await audio.enable();
    expect(audio.state().active.map(voice => voice.id)).toEqual(expect.arrayContaining(['ambient', 'engine', 'music']));
    expect(audio.state().active.map(voice => voice.id)).not.toContain('blaster');
    expect(audio.state().active.map(voice => voice.id)).not.toContain('siren');
    audio.dispose();
    const unavailable = createHostAudio({ contextFactory: () => null, storage: memoryStorage() });
    expect(() => { unavailable.audioConfig('{}'); unavailable.startGameAudio(); unavailable.audioCue('{bad'); }).not.toThrow();
    expect(unavailable.state().status).toBe('unavailable');
    expect(await unavailable.testOutput()).toBe(false);
  });

  it('a device start refusal leaves no phantom voice and a deliberate retry can play', async () => {
    const { audio, context } = host({ context: { rejectStart: true } });
    await settleAudio();
    expect(await audio.testOutput()).toBe(false);
    expect(audio.state().status).toBe('failed');
    expect(audio.state().active).toHaveLength(0);
    expect(context.sources.every(source => source.targets.length === 0)).toBe(true);
    context.rejectStart = false;
    expect(await audio.testOutput()).toBe(true);
    expect(context.sample()).toBeGreaterThan(0);
    audio.dispose();
  });

  it('failed decoding is visible and retry does not recover a missed shot', async () => {
    const { audio, context } = host({ context: { rejectDecode: true } });
    await mission(audio);
    expect(audio.state().status).toBe('failed');
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 0, y: 0, z: -10 }));
    context.rejectDecode = false;
    await audio.enable();
    await settleAudio();
    expect(audio.state().status).toBe('playing');
    expect(audio.state().active.map(voice => voice.id)).not.toContain('blaster');
    audio.dispose();
  });

  it('new config removes old voices and async decode cannot revive a removed sound', async () => {
    let complete;
    const context = new FakeAudioContext();
    const provider = createBrowserAudioProvider({ contextFactory: () => context,
      fetchAudio: () => new Promise(resolve => { complete = resolve; }) });
    provider.register('old', { file: 'old.mp3', volume: 1, category: 'music', loop: true });
    provider.loop('old', true);
    await provider.enable();
    provider.remove('old');
    complete(await audioFetch()());
    await settleAudio();
    expect(context.sample()).toBe(0);
    expect(provider.snapshot().active).toHaveLength(0);
    provider.dispose();
    const { audio, context: device } = host();
    await mission(audio);
    audio.audioConfig(JSON.stringify({ ambient: { file: 'other.mp3', volume: 0.8 } }));
    await settleAudio();
    expect(device.sample()).toBeCloseTo(0.4);
    expect(audio.debug().els).toEqual(['ambient']);
    audio.dispose();
  });

  it('lobby/restore reset ends old playback, silently seeds current Red Alert and retains preferences', async () => {
    const { audio, context, equivalents } = host();
    await mission(audio);
    audio.applyHudAudio(hud());
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    audio.setMasterVolume(0.3);
    audio.resetSession();
    expect(context.sample()).toBe(0);
    expect(equivalents.at(-1)).toEqual({ kind: 'clear' });
    audio.audioCue(JSON.stringify({ kind: 'computer_message', severity: 'critical' }));
    audio.startGameAudio();
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    expect(audio.state().active.map(voice => voice.id)).not.toContain('siren');
    expect(audio.state().active.map(voice => voice.id)).not.toContain('computer_critical');
    expect(audio.getMasterVolume()).toBe(0.3);
    audio.audioConfig(JSON.stringify(AUDIO_CONFIG));
    expect(audio.state().active.filter(voice => voice.id === 'ambient')).toHaveLength(1);
    audio.dispose();
  });

  it('runtime lifecycle boundaries hold playback during restore, then seed current state without old cue replay', async () => {
    const { audio, context, equivalents } = host();
    await mission(audio);
    audio.audioLifecycle(JSON.stringify({ generation: 1, running: true, suspended: false }));
    audio.applyHudAudio(hud());
    audio.audioCue(JSON.stringify({ kind: 'computer_message', severity: 'advisory' }));
    expect(audio.state().active.some(voice => voice.id === 'computer_advisory')).toBe(true);
    audio.audioLifecycle(JSON.stringify({ generation: 2, running: true, suspended: true }));
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    audio.audioLevel(0.9);
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 1, y: 0, z: -2 }));
    expect(context.sample()).toBe(0);
    expect(equivalents.some(cue => cue.kind === 'blaster')).toBe(false);
    audio.audioLifecycle(JSON.stringify({ generation: 3, running: true, suspended: false }));
    audio.audioConfig(JSON.stringify(AUDIO_CONFIG));
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    expect(audio.state().active.map(voice => voice.id)).toContain('music');
    expect(audio.state().active.some(voice => voice.id === 'siren' || voice.id === 'computer_advisory')).toBe(false);
    audio.audioLifecycle(JSON.stringify({ generation: 4, running: false, suspended: false }));
    audio.startMenuMusic(); await settleAudio();
    expect(audio.state().categories).toEqual(['music']);
    expect(audio.state().active.map(voice => voice.id)).toEqual(['menu']);
    audio.dispose();
  });

  it('lobby and loading boundaries retain the current menu bed, held only while restore is held', async () => {
    const { audio, context } = host();
    audio.startMenuMusic();
    await settleAudio();
    audio.audioLifecycle(JSON.stringify({ generation: 1, running: false, suspended: false }));
    expect(audio.state().active.map(voice => voice.id)).toContain('menu');
    audio.audioLifecycle(JSON.stringify({ generation: 2, running: false, suspended: true }));
    expect(context.sample()).toBe(0);
    audio.audioLifecycle(JSON.stringify({ generation: 3, running: false, suspended: false }));
    expect(audio.state().active.map(voice => voice.id)).toContain('menu');
    audio.audioLifecycle(JSON.stringify({ generation: 4, running: true, suspended: false }));
    expect(audio.state().active.map(voice => voice.id)).not.toContain('menu');
    audio.dispose();
  });

  it('a cached page return resumes current loops without a missed one-shot or siren', async () => {
    const { audio, context } = host();
    await mission(audio);
    audio.applyHudAudio(hud());
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: 1, y: 0, z: 0 }));
    audio.setPageActive(false);
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    audio.audioCue(JSON.stringify({ kind: 'computer_message', severity: 'critical' }));
    expect(context.sample()).toBe(0);
    audio.setPageActive(true);
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    expect(audio.state().active.map(voice => voice.id)).toEqual(expect.arrayContaining(['music', 'phaser']));
    expect(audio.state().active.some(voice => ['blaster', 'computer_critical', 'siren'].includes(voice.id))).toBe(false);
    audio.dispose();
  });

  it('master mute preserves live informative equivalents and adds none for music or engine beds', async () => {
    const { audio, equivalents, context } = host();
    await mission(audio);
    audio.setBus('master', { muted: true });
    audio.applyHudAudio(hud({ phaser_firing: true, engine_thrust: 1 }));
    audio.audioLevel(0.05);
    audio.audioLevel(0.8);
    audio.audioCue(JSON.stringify({ kind: 'blaster', x: -2, y: 4, z: -9, hidden_name: 'Secret ship' }));
    expect(context.sample()).toBe(0);
    expect(equivalents).toEqual([
      { kind: 'beam', active: true }, { kind: 'impact' }, { kind: 'blaster', x: -2, y: 4, z: -9 },
    ]);
    audio.dispose();
  });

  it('private GM surface never starts shared audio even if it owns a simulation', async () => {
    const { audio, context } = host({ isRoom: () => false });
    await mission(audio);
    audio.startMenuMusic();
    audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }));
    expect(await audio.testOutput()).toBe(false);
    expect(context.sample()).toBe(0);
    expect(audio.state().categories).toEqual([]);
    audio.dispose();
  });
});

it('all defined categories including future Interface use the same signal law', async () => {
  const context = new FakeAudioContext();
  const provider = createBrowserAudioProvider({ contextFactory: () => context, fetchAudio: audioFetch() });
  provider.register('click', { file: 'click.ogg', category: 'interface', volume: 0.6 });
  await settleAudio(); await provider.enable();
  const mix = defaultAudioMix(); mix.interface.level = 0.25; mix.master.level = 0.8;
  provider.setMix(mix); provider.cue('click');
  expect(context.sample()).toBeCloseTo(0.5 * 0.6 * 0.25 * 0.8);
  mix.interface.muted = true; provider.setMix(mix);
  expect(context.sample()).toBe(0);
  provider.dispose();
});
