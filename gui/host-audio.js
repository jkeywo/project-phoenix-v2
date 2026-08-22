/**
 * gui/host-audio.js — the host page's data-driven audio engine (issue #1228,
 * extracted from server.html's formerly-inline "Data-driven audio" block).
 *
 * Rust owns the config (ship `[audio]` + world `[audio.red_alert]`), the
 * forcefield envelope, and the blaster's listener-relative geometry. This
 * module only plays sounds. Nothing here hardcodes a filename or a volume —
 * see `tests/smoke/audio.spec.js`, which asserts the whole ship/world TOML →
 * host-channel → `<audio>`-graph chain end to end.
 *
 * Looping beds and the siren use `<audio>` elements (constructed via the
 * injected `AudioCtor`, never `document.querySelectorAll` — they are
 * deliberately detached elements that never enter the DOM). The blaster uses
 * Web Audio instead: one-shots overlap, and `<audio>` cannot pan.
 *
 * ## Master volume (issue #939)
 *
 * A SCALE FACTOR over the authored volumes, never a replacement for them.
 * Every per-sound level is designed data — ship `[audio]`, world
 * `[audio.red_alert]`, the blaster spec, the engine's idle/full-thrust
 * coefficients — and the host's slider must not overwrite the mix the
 * designer authored, only turn the whole thing up or down. So each channel
 * keeps its authored level in `_authoredVol` and what reaches the element is
 * always `authored × master`. 1 is the identity (unattenuated), which is why
 * it is the default rather than a tunable.
 *
 * ## The `applyHudAudio` seam
 *
 * Rust pushes `ViewscreenHudState` JSON over the "hud" host channel; that
 * message also carries the two audio state flags (`red_alert`,
 * `phaser_firing`) and `engine_thrust`. server.html's `__updateHud` still
 * owns the non-audio half of that payload (the vignette CSS class, the
 * status-strip text) but hands the whole parsed object to `applyHudAudio`
 * for the siren/music/engine/phaser side, rather than reaching into this
 * module's `_els`/`_audioCfg`/`setChannelVolume` internals directly.
 *
 * ## Node-import-safety
 *
 * Nothing here touches a free-standing `window`/`document`/`Audio`/
 * `localStorage` beyond what the caller passes in via `createHostAudio`'s
 * `{ doc, storage, AudioCtor }` — so it imports cleanly under vitest with
 * `@vitest-environment jsdom` and a stubbed `Audio` constructor (jsdom's own
 * `HTMLMediaElement.play()` is "Not implemented"). See
 * tests/client/host-audio.test.js.
 *
 * ## Classic-script boundary
 *
 * server.html's audio call sites (`startGameAudio`, `startMenuMusic`,
 * `stopMenuMusic` inside `__updateLobby`'s phase handling; `applyHudAudio`
 * inside `__updateHud`) live in a CLASSIC script and cannot `import` this
 * module directly. That script bridges to the module island through the
 * same queue/latch shape the #1224 host-channel and #1225/#1227 islands
 * use — see the "Host audio bridge (issue #1228)" block in server.html.
 * The three host-channel audio handlers (`audio_config`/`audio_cue`/
 * `audio_level`) delegate straight to this module's instance rather than to
 * inline functions — see the host-audio island near the #1225 dispatcher.
 */

/**
 * Build the host page's audio engine.
 *
 * @param {{
 *   doc?: Document,
 *   storage?: Storage|null,
 *   AudioCtor?: typeof Audio,
 * }} opts
 * @returns {{
 *   audioConfig: (json: string) => void,
 *   audioCue: (json: string) => void,
 *   audioLevel: (level: number) => void,
 *   applyHudAudio: (s: object) => void,
 *   startGameAudio: () => void,
 *   startMenuMusic: () => void,
 *   stopMenuMusic: () => void,
 *   getMasterVolume: () => number,
 *   setMasterVolume: (v: number) => void,
 *   debug: () => object,
 * }}
 */
export function createHostAudio({
  doc = (typeof document !== 'undefined' ? document : undefined),
  storage = null,
  AudioCtor = (typeof Audio !== 'undefined' ? Audio : undefined),
} = {}) {
  let _audioCfg = null;      // payload from the "audio_config" host channel
  let _audioUnlocked = false; // user gesture / game start has happened
  let _audioStarted = false;
  const _els = {};            // key -> HTMLAudioElement
  let _actx = null;           // AudioContext (blaster only)
  let _blasterBuf = null;     // decoded AudioBuffer
  let _prevRedAlert = false;
  let _prevPhaser = false;
  let _menuMusic = null;

  // HTMLMediaElement.volume throws IndexSizeError outside 0..1.
  function clampVol(v) {
    const n = Number(v);
    if (!isFinite(n)) return 0;
    return Math.min(1, Math.max(0, n));
  }

  const MASTER_VOLUME_KEY = 'phoenix-server-master-volume';
  const MASTER_VOLUME_DEFAULT = 1;
  // The menu bed predates the audio config and has no TOML entry to read;
  // named here so it composes with master like every other channel.
  const MENU_MUSIC_VOLUME = 0.5;
  const _authoredVol = {};   // channel key -> authored (pre-master) volume
  let _masterVolume = MASTER_VOLUME_DEFAULT;
  try {
    const stored = storage ? storage.getItem(MASTER_VOLUME_KEY) : null;
    if (stored !== null && isFinite(Number(stored))) _masterVolume = clampVol(stored);
  } catch (_) { /* private mode / storage disabled — stay at the default */ }

  function applyMaster(authored) {
    return clampVol(clampVol(authored) * _masterVolume);
  }

  /** Set an element's live volume from an authored level, remembering it. */
  function setChannelVolume(key, authored) {
    _authoredVol[key] = clampVol(authored);
    if (_els[key]) _els[key].volume = applyMaster(authored);
  }

  function getMasterVolume() {
    return _masterVolume;
  }

  // Live: re-derives every channel from its authored level, so dragging the
  // slider is audible immediately rather than at the next config push.
  function setMasterVolume(v) {
    _masterVolume = clampVol(v);
    try { if (storage) storage.setItem(MASTER_VOLUME_KEY, String(_masterVolume)); }
    catch (_) { /* not worth failing the drag over */ }
    Object.keys(_els).forEach(function(k) {
      if (k in _authoredVol) _els[k].volume = applyMaster(_authoredVol[k]);
    });
    if (_menuMusic) _menuMusic.volume = applyMaster(MENU_MUSIC_VOLUME);
  }

  function startMenuMusic() {
    if (_menuMusic) return;
    _menuMusic = new AudioCtor('assets/sounds/exploration.mp3');
    _menuMusic.loop = true;
    _menuMusic.volume = applyMaster(MENU_MUSIC_VOLUME);
    _menuMusic.play().catch(function() {});
  }

  function stopMenuMusic() {
    if (_menuMusic) {
      _menuMusic.pause();
      _menuMusic.currentTime = 0;
      _menuMusic = null;
    }
  }

  function mkLoop(key, spec, volume) {
    if (!spec || !spec.file) return;
    const el = new AudioCtor(spec.file);
    el.loop = true;
    el.preload = 'auto';
    _els[key] = el;
    setChannelVolume(key, volume);
  }

  function audioConfig(json) {
    let c;
    try { c = JSON.parse(json); }
    catch (e) { console.warn('[Phoenix] bad audio config', e); return; }
    _audioCfg = c;

    mkLoop('ambient', c.ambient, c.ambient && c.ambient.volume);
    // Engine starts at idle and is ridden by thrust in applyHudAudio.
    mkLoop('engine', c.engine, c.engine && c.engine.idle_volume);
    // Phaser and forcefield start silent: the phaser is gated on the firing
    // flag, and the forcefield's level is pushed from Rust each frame.
    mkLoop('phaser', c.phaser_loop, 0);
    mkLoop('forcefield', c.forcefield, 0);

    if (c.red_alert && c.red_alert.music_file) {
      const m = new AudioCtor(c.red_alert.music_file);
      m.loop = true;
      m.preload = 'auto';
      _els.music = m;
      setChannelVolume('music', c.red_alert.music_volume);
    }
    if (c.red_alert && c.red_alert.siren_file) {
      const s = new AudioCtor(c.red_alert.siren_file);
      s.preload = 'auto';
      _els.siren = s;
      setChannelVolume('siren', c.red_alert.siren_volume);
    }

    initBlasterAudio(c.blaster);
    maybeStartAudio();
  }

  function initBlasterAudio(spec) {
    if (!spec || !spec.file) return;
    try {
      const view = doc && doc.defaultView;
      const Ctor = view && (view.AudioContext || view.webkitAudioContext);
      if (!Ctor) return;
      _actx = _actx || new Ctor();
    } catch (e) {
      console.warn('[Phoenix] no AudioContext; blaster audio disabled', e);
      return;
    }
    // Must not reject: a bubbling rejection fails the smoke test's
    // pageerror assertion. Headless Chromium may refuse to decode at all,
    // and "no blaster SFX" is an acceptable degradation.
    fetch(spec.file)
      .then(function(r) { return r.arrayBuffer(); })
      .then(function(b) { return _actx.decodeAudioData(b); })
      .then(function(buf) { _blasterBuf = buf; })
      .catch(function(e) { console.warn('[Phoenix] blaster decode failed', e); });
  }

  // Called on the Loading → InProgress edge, which is also the autoplay
  // unlock. The config push and this edge can land in the same frame, and
  // although a single flush (`flush_host_channels`, issue #818) now walks
  // the host channels in a fixed order, cross-channel ordering remains an
  // implementation detail JS must not rely on — whichever arrives second
  // starts the audio.
  function startGameAudio() {
    _audioUnlocked = true;
    maybeStartAudio();
  }

  function maybeStartAudio() {
    if (_audioStarted || !_audioUnlocked || !_audioCfg) return;
    _audioStarted = true;
    ['ambient', 'engine', 'phaser', 'forcefield'].forEach(function(k) {
      if (_els[k]) _els[k].play().catch(function() {});
    });
    if (_actx && _actx.state === 'suspended') {
      _actx.resume().catch(function() {});
    }
  }

  // One-shot positional cue. Coordinates are already listener-relative, so
  // the Web Audio listener stays at the origin facing -Z.
  function audioCue(json) {
    let c;
    try { c = JSON.parse(json); } catch (e) { return; }
    if (c.kind !== 'blaster' || !_blasterBuf || !_actx) return;
    const spec = _audioCfg && _audioCfg.blaster;
    if (!spec) return;
    try {
      const src = _actx.createBufferSource();
      src.buffer = _blasterBuf;
      const pan = _actx.createPanner();
      pan.panningModel  = spec.panning_model;
      pan.distanceModel = spec.distance_model;
      pan.refDistance   = spec.ref_distance;
      pan.maxDistance   = spec.max_distance;
      pan.rolloffFactor = spec.rolloff_factor;
      if (pan.positionX) {
        pan.positionX.value = c.x;
        pan.positionY.value = c.y;
        pan.positionZ.value = c.z;
      } else {
        pan.setPosition(c.x, c.y, c.z); // older Safari
      }
      const g = _actx.createGain();
      // Web Audio, so this one is scaled at the cue rather than carried on
      // an element — same authored × master composition.
      g.gain.value = applyMaster(spec.volume);
      src.connect(pan).connect(g).connect(_actx.destination);
      src.start();
    } catch (e) {
      console.warn('[Phoenix] blaster cue failed', e);
    }
  }

  // Forcefield SFX level, computed and clamped in Rust. Rust's number is the
  // authored level; master scales it like every other channel.
  function audioLevel(level) {
    if (_els.forcefield) setChannelVolume('forcefield', level);
  }

  // The HUD-driven half of the audio module: red-alert siren/music, engine
  // volume from thrust, and the phaser loop's firing edge. `s` is the
  // already-parsed ViewscreenHudState the "hud" host channel pushed — the
  // caller (server.html's __updateHud) owns the JSON.parse and the non-audio
  // half of that payload (vignette CSS, status strip).
  function applyHudAudio(s) {
    // ── Red alert: siren one-shot on the edge, music loops underneath ───────
    if (s.red_alert && !_prevRedAlert && _els.siren) {
      _els.siren.currentTime = 0;
      _els.siren.play().catch(() => {});
    }
    if (_els.music) {
      if (s.red_alert && _els.music.paused) {
        _els.music.play().catch(() => {});
      } else if (!s.red_alert && !_els.music.paused) {
        _els.music.pause();
        _els.music.currentTime = 0;
      }
    }
    _prevRedAlert = !!s.red_alert;
    // ── Engine volume from thrust (coefficients from the ship's TOML) ───────
    if (_els.engine && _audioCfg && _audioCfg.engine && typeof s.engine_thrust === 'number') {
      const e = _audioCfg.engine;
      // setChannelVolume, not a direct assignment: the ride is the authored
      // level, and master scales whatever it lands on (issue #939).
      setChannelVolume('engine', e.idle_volume + s.engine_thrust * e.volume_at_full_thrust);
    }
    // ── Phaser loop follows the change-detected firing flag ────────────────
    if (_els.phaser && _audioCfg && _audioCfg.phaser_loop) {
      if (s.phaser_firing && !_prevPhaser) {
        setChannelVolume('phaser', _audioCfg.phaser_loop.volume);
        _els.phaser.currentTime = 0;
        _els.phaser.play().catch(() => {});
      } else if (!s.phaser_firing && _prevPhaser) {
        _els.phaser.pause();
      }
    }
    _prevPhaser = !!s.phaser_firing;
  }

  // Smoke-test hook — lets Playwright assert on audio state without needing
  // to hear anything.
  //
  // The elements are deliberately exposed here rather than looked up via
  // document.querySelectorAll('audio'): `new Audio()` builds *detached*
  // elements that never enter the DOM, so a querySelectorAll sweep finds
  // nothing and any assertion over it passes vacuously.
  function debug() {
    const vols = {};
    Object.keys(_els).forEach(function(k) { vols[k] = _els[k].volume; });
    const paused = {};
    Object.keys(_els).forEach(function(k) { paused[k] = _els[k].paused; });
    return {
      cfg: _audioCfg,
      els: Object.keys(_els),
      started: _audioStarted,
      volumes: vols,
      // Authored (pre-master) levels and the scale factor over them, so a
      // test can assert the composition rather than just the product.
      authoredVolumes: Object.assign({}, _authoredVol),
      master: _masterVolume,
      paused: paused,
      musicPlaying: !!(_els.music && !_els.music.paused),
      phaserPlaying: !!(_els.phaser && !_els.phaser.paused),
      blasterReady: !!_blasterBuf,
    };
  }

  return {
    audioConfig,
    audioCue,
    audioLevel,
    applyHudAudio,
    startGameAudio,
    startMenuMusic,
    stopMenuMusic,
    getMasterVolume,
    setMasterVolume,
    debug,
  };
}
