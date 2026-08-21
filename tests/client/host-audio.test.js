// @vitest-environment jsdom
//
// tests/client/host-audio.test.js — issue #1228: server.html's "Data-driven
// audio" block (clampVol/applyMaster/setChannelVolume/mkLoop/
// initBlasterAudio/startGameAudio, the master-volume localStorage key,
// window.__audioDebug, window.__setMasterVolume) moved to gui/host-audio.js.
// This suite exercises the module directly rather than scraping server.html
// as text — same rationale as host-channel-localisation.test.js for
// gui/host-channel.js (issue #1225) and page-chrome.test.js for
// gui/page-chrome.js (issue #1227).
//
// jsdom's own HTMLMediaElement.play() is "Not implemented" (it logs a jsdom
// error rather than resolving), so every test here supplies its own stubbed
// `Audio` constructor via createHostAudio's `AudioCtor` — never jsdom's real
// one. That is also why this suite doesn't need `@vitest-environment jsdom`
// for the audio elements themselves; it's used anyway because `doc` (for
// the blaster's AudioContext lookup) and a couple of fixtures read more
// naturally against a real Document.
//
// The real end-to-end guard for this module is tests/smoke/audio.spec.js
// (Playwright, CI-only) — driving `window.__updateHud` ~8 times and reading
// `window.__audioDebug()`. This suite mirrors that spec's HUD-driven
// scenarios in miniature so the contract it depends on
// (`applyHudAudio`/`debug()`/`audioLevel` clamping) is covered locally too.

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { createHostAudio } from '../../gui/host-audio.js';

/** A minimal HTMLMediaElement-like stub: tracks play/pause state without
 * jsdom's "Not implemented" media stack. */
function FakeAudio(src) {
  this.src = src;
  this.loop = false;
  this.preload = '';
  this.volume = 1;
  this.paused = true;
  this.currentTime = 0;
  this.playCalls = 0;
}
FakeAudio.prototype.play = function () {
  this.paused = false;
  this.playCalls++;
  return Promise.resolve();
};
FakeAudio.prototype.pause = function () {
  this.paused = true;
};

/** A `Storage`-like fake so master-volume persistence can be asserted
 * without touching jsdom's real localStorage. */
function fakeStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => { map.set(k, String(v)); },
    removeItem: (k) => { map.delete(k); },
    _map: map,
  };
}

const MASTER_VOLUME_KEY = 'phoenix-server-master-volume';

/** Drain the microtask queue enough turns for fetch().then(arrayBuffer)
 * .then(decodeAudioData).then(assign) to settle in tests below. */
async function flushMicrotasks(turns = 8) {
  for (let i = 0; i < turns; i++) await Promise.resolve();
}

/** Ship-shaped audio config matching assets/entities/alliance_battleship.toml's
 * [audio.*] tables, so the fixture reads like the real wire payload the
 * "audio_config" host channel pushes. */
function shipAudioConfig(overrides = {}) {
  return {
    ambient: { file: 'assets/sounds/Ambient.mp3', volume: 0.25 },
    engine: { file: 'assets/sounds/Engine.mp3', idle_volume: 0.1, volume_at_full_thrust: 0.15 },
    phaser_loop: { file: 'assets/sounds/PhaserLoop.mp3', volume: 0.5 },
    blaster: {
      file: 'assets/sounds/Blaster.mp3',
      volume: 0.9,
      ref_distance: 30.0,
      max_distance: 800.0,
      rolloff_factor: 1.2,
      distance_model: 'inverse',
      panning_model: 'equalpower',
    },
    forcefield: { file: 'assets/sounds/ForcefieldHit.mp3' },
    red_alert: { siren_file: 'assets/sounds/Siren.mp3', siren_volume: 0.6, music_file: 'assets/sounds/RedAlert.mp3', music_volume: 0.4 },
    ...overrides,
  };
}

function hud(over) {
  return {
    heading: 0,
    hull_pct: 100,
    condition: 'NOMINAL',
    red_alert: false,
    engine_thrust: 0,
    phaser_firing: false,
    ...over,
  };
}

describe('createHostAudio', () => {
  let doc;

  beforeEach(() => {
    // A Document with no real AudioContext — initBlasterAudio should
    // degrade gracefully (blaster tests below stub defaultView themselves).
    doc = document.implementation.createHTMLDocument('host-audio test');
  });

  it('starts with no config and an idle debug snapshot', () => {
    const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    const dbg = audio.debug();
    expect(dbg.cfg).toBeNull();
    expect(dbg.els).toEqual([]);
    expect(dbg.started).toBe(false);
    expect(dbg.master).toBe(1);
    expect(dbg.musicPlaying).toBe(false);
    expect(dbg.phaserPlaying).toBe(false);
    expect(dbg.blasterReady).toBe(false);
  });

  it('audioConfig builds one element per configured channel with authored volumes', () => {
    const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    audio.audioConfig(JSON.stringify(shipAudioConfig()));
    const dbg = audio.debug();

    expect(dbg.els).toEqual(
      expect.arrayContaining(['ambient', 'engine', 'phaser', 'forcefield', 'music', 'siren']),
    );
    // Engine starts at idle, not authored ambient/etc — ridden by thrust later.
    expect(dbg.volumes.ambient).toBeCloseTo(0.25, 5);
    expect(dbg.volumes.engine).toBeCloseTo(0.1, 5);
    // Phaser/forcefield start silent regardless of any authored volume.
    expect(dbg.volumes.phaser).toBe(0);
    expect(dbg.volumes.forcefield).toBe(0);
    expect(dbg.volumes.siren).toBeCloseTo(0.6, 5);
    expect(dbg.volumes.music).toBeCloseTo(0.4, 5);
    // The forcefield envelope stays server-side: JS gets the file only.
    expect(dbg.cfg.forcefield).toEqual({ file: 'assets/sounds/ForcefieldHit.mp3' });
  });

  it('omitting a sub-block silences that channel (no element constructed)', () => {
    const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    const cfg = shipAudioConfig();
    delete cfg.forcefield;
    audio.audioConfig(JSON.stringify(cfg));
    expect(audio.debug().els).not.toContain('forcefield');
  });

  it('bad JSON is reported and does not throw or mutate state', () => {
    const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(() => audio.audioConfig('{not json')).not.toThrow();
    expect(audio.debug().cfg).toBeNull();
    warn.mockRestore();
  });

  it('starts playback once both audioConfig and startGameAudio have landed, in either order', () => {
    // Order A: config then unlock.
    const a = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    a.audioConfig(JSON.stringify(shipAudioConfig()));
    expect(a.debug().started).toBe(false);
    a.startGameAudio();
    expect(a.debug().started).toBe(true);
    expect(a.debug().paused.ambient).toBe(false);
    expect(a.debug().paused.engine).toBe(false);
    expect(a.debug().paused.phaser).toBe(false);
    expect(a.debug().paused.forcefield).toBe(false);
    // music/siren are NOT auto-started — only red alert starts them.
    expect(a.debug().paused.music).toBe(true);

    // Order B: unlock then config (the reverse race named in the module doc).
    const b = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    b.startGameAudio();
    expect(b.debug().started).toBe(false);
    b.audioConfig(JSON.stringify(shipAudioConfig()));
    expect(b.debug().started).toBe(true);
  });

  it('is idempotent: a second startGameAudio/audioConfig does not replay playback', () => {
    const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
    audio.audioConfig(JSON.stringify(shipAudioConfig()));
    audio.startGameAudio();
    const ambientEl = audio.debug(); // started already true
    audio.startGameAudio();
    expect(audio.debug().started).toBe(true);
    expect(ambientEl.started).toBe(true);
  });

  describe('master volume', () => {
    it('defaults to 1 with no stored value', () => {
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
      expect(audio.getMasterVolume()).toBe(1);
    });

    it('reads a stored value on construction', () => {
      const storage = fakeStorage({ [MASTER_VOLUME_KEY]: '0.5' });
      const audio = createHostAudio({ doc, storage, AudioCtor: FakeAudio });
      expect(audio.getMasterVolume()).toBe(0.5);
    });

    it('ignores a non-numeric stored value and stays at the default', () => {
      const storage = fakeStorage({ [MASTER_VOLUME_KEY]: 'garbage' });
      const audio = createHostAudio({ doc, storage, AudioCtor: FakeAudio });
      expect(audio.getMasterVolume()).toBe(1);
    });

    it('tolerates a storage that throws (private mode)', () => {
      const storage = {
        getItem: () => { throw new Error('denied'); },
        setItem: () => { throw new Error('denied'); },
      };
      expect(() => createHostAudio({ doc, storage, AudioCtor: FakeAudio })).not.toThrow();
    });

    it('setMasterVolume clamps, persists, and rescales every live channel + menu music', () => {
      const storage = fakeStorage();
      const audio = createHostAudio({ doc, storage, AudioCtor: FakeAudio });
      audio.audioConfig(JSON.stringify(shipAudioConfig()));
      audio.startMenuMusic();

      audio.setMasterVolume(0.5);
      expect(audio.getMasterVolume()).toBe(0.5);
      expect(storage.getItem(MASTER_VOLUME_KEY)).toBe('0.5');
      // authored ambient 0.25 * master 0.5
      expect(audio.debug().volumes.ambient).toBeCloseTo(0.125, 5);
      // menu music: authored 0.5 * master 0.5
      expect(audio.debug()).toBeTruthy();

      // Out-of-range input is clamped rather than thrown.
      audio.setMasterVolume(5);
      expect(audio.getMasterVolume()).toBe(1);
      audio.setMasterVolume(-3);
      expect(audio.getMasterVolume()).toBe(0);
      expect(audio.debug().volumes.ambient).toBe(0);
    });
  });

  describe('startMenuMusic / stopMenuMusic', () => {
    it('starts a looping bed at the menu volume scaled by master', () => {
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
      audio.startMenuMusic();
      // No public element inspection for menu music (it's not in _els / debug()
      // by design — only channel-table sounds are), so assert indirectly via
      // idempotency and stop below.
      const before = audio.debug();
      audio.startMenuMusic(); // second call is a no-op while already playing
      expect(audio.debug()).toEqual(before);
    });

    it('stop then start constructs a fresh element', () => {
      let constructed = 0;
      function CountingAudio(src) {
        constructed++;
        FakeAudio.call(this, src);
      }
      CountingAudio.prototype = FakeAudio.prototype;
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: CountingAudio });
      audio.startMenuMusic();
      expect(constructed).toBe(1);
      audio.stopMenuMusic();
      audio.startMenuMusic();
      expect(constructed).toBe(2);
    });
  });

  describe('audioLevel (forcefield)', () => {
    it('clamps out-of-range levels into 0..1', () => {
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
      audio.audioConfig(JSON.stringify(shipAudioConfig()));
      audio.audioLevel(5);
      expect(audio.debug().volumes.forcefield).toBe(1);
      audio.audioLevel(-2);
      expect(audio.debug().volumes.forcefield).toBe(0);
      audio.audioLevel(0.4);
      expect(audio.debug().volumes.forcefield).toBeCloseTo(0.4, 5);
    });

    it('is a no-op before audioConfig has built the forcefield element', () => {
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
      expect(() => audio.audioLevel(0.5)).not.toThrow();
    });
  });

  describe('applyHudAudio', () => {
    /** Config + unlocked engine ready to be driven by applyHudAudio, mirroring
     * tests/smoke/audio.spec.js's HUD-driven section. */
    function readyAudio() {
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
      audio.audioConfig(JSON.stringify(shipAudioConfig()));
      audio.startGameAudio();
      return audio;
    }

    it('red alert on: siren one-shots and music starts looping', () => {
      const audio = readyAudio();
      audio.applyHudAudio(hud({ red_alert: true, condition: 'ALERT' }));
      const dbg = audio.debug();
      expect(dbg.musicPlaying).toBe(true);
      expect(dbg.paused.siren).toBe(false);
    });

    it('red alert off: music stops', () => {
      const audio = readyAudio();
      audio.applyHudAudio(hud({ red_alert: true }));
      audio.applyHudAudio(hud({ red_alert: false }));
      expect(audio.debug().musicPlaying).toBe(false);
    });

    it('the siren only re-fires on the false→true edge, not while alert stays on', () => {
      const audio = readyAudio();
      audio.applyHudAudio(hud({ red_alert: true }));
      const sirenPlaysAfterEdge = audio.debug().paused.siren;
      // Manually pause the siren (as if the one-shot finished) and drive the
      // same red_alert:true state again — must NOT replay.
      audio.applyHudAudio(hud({ red_alert: true }));
      expect(audio.debug().paused.siren).toBe(sirenPlaysAfterEdge);
    });

    it('phaser firing: loop starts at authored volume, then stops on release', () => {
      const audio = readyAudio();
      audio.applyHudAudio(hud({ phaser_firing: true }));
      let dbg = audio.debug();
      expect(dbg.phaserPlaying).toBe(true);
      expect(dbg.volumes.phaser).toBeCloseTo(0.5, 5);

      audio.applyHudAudio(hud({ phaser_firing: false }));
      dbg = audio.debug();
      expect(dbg.phaserPlaying).toBe(false);
    });

    it('engine volume tracks thrust via idle_volume + thrust * volume_at_full_thrust', () => {
      const audio = readyAudio();
      audio.applyHudAudio(hud({ engine_thrust: 1.0 }));
      expect(audio.debug().volumes.engine).toBeCloseTo(0.1 + 1.0 * 0.15, 5);

      audio.applyHudAudio(hud({ engine_thrust: 0.5 }));
      expect(audio.debug().volumes.engine).toBeCloseTo(0.1 + 0.5 * 0.15, 5);
    });

    it('is a no-op on every field when no config has been pushed yet', () => {
      const audio = createHostAudio({ doc, storage: fakeStorage(), AudioCtor: FakeAudio });
      expect(() => audio.applyHudAudio(hud({ red_alert: true, phaser_firing: true }))).not.toThrow();
      expect(audio.debug().musicPlaying).toBe(false);
      expect(audio.debug().phaserPlaying).toBe(false);
    });
  });

  describe('blaster (Web Audio one-shot)', () => {
    /** A fake AudioContext + node graph sufficient for audioCue's calls. */
    function fakeAudioContextCtor() {
      const gainNodes = [];
      const pannerNodes = [];
      const sources = [];
      function FakeGain() {
        this.gain = { value: 0 };
        this.connect = vi.fn(() => this);
        gainNodes.push(this);
      }
      function FakePanner() {
        this.positionX = { value: 0 };
        this.positionY = { value: 0 };
        this.positionZ = { value: 0 };
        this.connect = vi.fn(() => this);
        pannerNodes.push(this);
      }
      function FakeSource() {
        this.connect = vi.fn(() => this);
        this.start = vi.fn();
        sources.push(this);
      }
      function FakeAudioContext() {
        this.state = 'running';
        this.destination = {};
        this.createBufferSource = () => new FakeSource();
        this.createPanner = () => new FakePanner();
        this.createGain = () => new FakeGain();
        this.decodeAudioData = (buf) => Promise.resolve({ __decoded: buf });
        this.resume = vi.fn(() => Promise.resolve());
      }
      FakeAudioContext.__nodes = { gainNodes, pannerNodes, sources };
      return FakeAudioContext;
    }

    function docWithAudioContext(Ctor) {
      return { defaultView: { AudioContext: Ctor } };
    }

    beforeEach(() => {
      vi.stubGlobal('fetch', vi.fn(() =>
        Promise.resolve({ arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)) }),
      ));
    });

    it('gracefully no-ops with no AudioContext available (headless-safe degradation)', async () => {
      const audio = createHostAudio({
        doc: { defaultView: {} },
        storage: fakeStorage(),
        AudioCtor: FakeAudio,
      });
      audio.audioConfig(JSON.stringify(shipAudioConfig()));
      // Flush the microtask queue the (never-started) decode chain would use.
      await flushMicrotasks();
      expect(audio.debug().blasterReady).toBe(false);
      expect(() => audio.audioCue(JSON.stringify({ kind: 'blaster', x: 0, y: 0, z: -1 }))).not.toThrow();
    });

    it('decodes the blaster buffer from the configured file and marks blasterReady', async () => {
      const Ctor = fakeAudioContextCtor();
      const audio = createHostAudio({
        doc: docWithAudioContext(Ctor),
        storage: fakeStorage(),
        AudioCtor: FakeAudio,
      });
      audio.audioConfig(JSON.stringify(shipAudioConfig()));
      // Let the fetch → arrayBuffer → decodeAudioData chain resolve.
      await flushMicrotasks();
      expect(audio.debug().blasterReady).toBe(true);
      expect(global.fetch).toHaveBeenCalledWith('assets/sounds/Blaster.mp3');
    });

    it('audioCue builds a source→panner→gain chain scaled by master volume and ignores non-blaster kinds', async () => {
      const Ctor = fakeAudioContextCtor();
      const audio = createHostAudio({
        doc: docWithAudioContext(Ctor),
        storage: fakeStorage(),
        AudioCtor: FakeAudio,
      });
      audio.audioConfig(JSON.stringify(shipAudioConfig()));
      await flushMicrotasks();
      audio.setMasterVolume(0.5);

      audio.audioCue(JSON.stringify({ kind: 'not-blaster', x: 1, y: 2, z: 3 }));
      expect(Ctor.__nodes.sources.length).toBe(0);

      audio.audioCue(JSON.stringify({ kind: 'blaster', x: 1, y: 2, z: 3 }));
      expect(Ctor.__nodes.sources.length).toBe(1);
      expect(Ctor.__nodes.sources[0].start).toHaveBeenCalled();
      // authored blaster volume 0.9 * master 0.5
      expect(Ctor.__nodes.gainNodes[0].gain.value).toBeCloseTo(0.45, 5);
      expect(Ctor.__nodes.pannerNodes[0].positionX.value).toBe(1);
    });
  });
});
