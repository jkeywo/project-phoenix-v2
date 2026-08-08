// @vitest-environment jsdom
//
// Issue #949 — the host channel is a localisation boundary.
//
// Rust pushes authored DATA over the host channels, and authored data holds
// string ids: a world's `[global] title` on the lobby channel, a `game_over`
// trigger's `message` on the hud channel. A phone crosses localiseTree() once
// in gui/connection-manager.js and renders those resolved; the host page had no
// equivalent boundary, so `#lobby-title` read `world.combat_test.global.title`
// verbatim — the symptom #949 names.
//
// The dispatcher lives inline in server.html (classic script, closed over host
// state), so there is no module to import. This test lifts the REAL source of
// localiseHostPayload, the handler table and __hostChannel out of the file and
// runs them against stub handlers, so it fails if the boundary is removed or
// routed around rather than re-testing a copy of the logic.

import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { t, localiseTree, has } from '../../gui/strings.js';

const SERVER_HTML = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../server.html',
);
const SRC = fs.readFileSync(SERVER_HTML, 'utf-8');

/** The source of one brace-balanced declaration, starting at `marker`. */
function declaration(marker) {
  const start = SRC.indexOf(marker);
  if (start === -1) throw new Error(`server.html no longer contains: ${marker}`);
  let depth = 0;
  for (let i = SRC.indexOf('{', start); i < SRC.length; i += 1) {
    if (SRC[i] === '{') depth += 1;
    else if (SRC[i] === '}') {
      depth -= 1;
      if (depth === 0) return SRC.slice(start, i + 1);
    }
  }
  throw new Error(`unbalanced braces after: ${marker}`);
}

/**
 * The real dispatcher, evaluated against a `window` whose channel handlers
 * record what they were handed.
 */
function mountDispatcher({ strings = { t, has, localiseTree } } = {}) {
  const seen = [];
  const record = (channel) => (payload) => seen.push([channel, payload]);
  const win = {
    phStrings: strings,
    __updateHud: record('hud'),
    __updateLobby: record('lobby'),
    __updateChatter: record('chatter'),
    __audioConfig: record('audio_config'),
    __audioCue: record('audio_cue'),
    __audioLevel: record('audio_level'),
    __applyShake: (x, y) => seen.push(['shake', [x, y]]),
  };
  const build = new Function(
    'window',
    'console',
    [
      declaration('function localiseHostPayload(payload)'),
      declaration('var _hostChannelHandlers = {') + ';',
      declaration('window.__hostChannel = function(name, payload)') + ';',
      'return { localiseHostPayload: localiseHostPayload };',
    ].join('\n'),
  );
  const warnings = [];
  const helpers = build(win, { warn: (...a) => warnings.push(a) });
  return { win, seen, warnings, ...helpers };
}

/** What the handler for `channel` was handed, JSON-decoded. */
function payloadFor(seen, channel) {
  const hit = seen.find(([name]) => name === channel);
  if (!hit) throw new Error(`nothing dispatched on ${channel}`);
  return JSON.parse(hit[1]);
}

describe('host channel localisation boundary', () => {
  let d;
  beforeEach(() => { d = mountDispatcher(); });

  it('resolves the lobby scenario title and body — the reported symptom', () => {
    // Exactly what src/world/server.rs puts on the wire for combat_test.toml.
    d.win.__hostChannel('lobby', JSON.stringify({
      phase: 'Lobby',
      scenario_title: 'world.combat_test.global.title',
      scenario_body: 'world.combat_test.global.description',
      stations: [],
    }));
    const p = payloadFor(d.seen, 'lobby');
    expect(p.scenario_title).toBe(t('world.combat_test.global.title'));
    expect(p.scenario_body).toBe(t('world.combat_test.global.description'));
    expect(p.scenario_title).not.toContain('world.combat_test');
  });

  it('resolves the hud game-over message', () => {
    // A `[[trigger.action]] type = "game_over"` message id, which reaches the
    // overlay through GameOverReason untouched.
    d.win.__hostChannel('hud', JSON.stringify({
      heading: 7,
      hull_pct: 64,
      game_over_message: 'world.combat_test.trigger.action.18.message',
    }));
    const p = payloadFor(d.seen, 'hud');
    expect(p.game_over_message).toBe(t('world.combat_test.trigger.action.18.message'));
  });

  it('resolves an id nested inside an array of station payloads', () => {
    // Station names are English in the shipped hulls today, but the payload is
    // a tree and the boundary must reach all of it — not just the two fields
    // that happened to be reported.
    d.win.__hostChannel('lobby', JSON.stringify({
      phase: 'Lobby',
      stations: [{ name: 'entity.alliance_destroyer.name', rank: 'Cpt.' }],
    }));
    const p = payloadFor(d.seen, 'lobby');
    expect(p.stations[0].name).toBe(t('entity.alliance_destroyer.name'));
    expect(p.stations[0].rank).toBe('Cpt.');
  });

  it('leaves machine tokens, player names and authored prose alone', () => {
    d.win.__hostChannel('lobby', JSON.stringify({
      phase: 'InProgress',
      crew_count: 2,
      all_ready: true,
      spectators: ['Jo'],
      stations: [{ name: 'Captain', preset_names: ['Std', 'Simplified'] }],
      scenario_title: 'A Mod Pack Wrote This Literally',
    }));
    expect(payloadFor(d.seen, 'lobby')).toEqual({
      phase: 'InProgress',
      crew_count: 2,
      all_ready: true,
      spectators: ['Jo'],
      stations: [{ name: 'Captain', preset_names: ['Std', 'Simplified'] }],
      scenario_title: 'A Mod Pack Wrote This Literally',
    });
  });

  it('passes text a phone already resolved through untouched', () => {
    // No CSV `en` value is itself an id, so crossing the rule twice is a no-op.
    const resolved = t('world.combat_test.global.title');
    d.win.__hostChannel('lobby', JSON.stringify({ scenario_title: resolved }));
    expect(payloadFor(d.seen, 'lobby').scenario_title).toBe(resolved);
  });

  it('routes every channel through the boundary, not just the lobby', () => {
    for (const channel of ['hud', 'chatter', 'audio_config', 'audio_cue']) {
      const fresh = mountDispatcher();
      fresh.win.__hostChannel(channel, JSON.stringify({
        text: 'entity.alliance_destroyer.name',
      }));
      expect(payloadFor(fresh.seen, channel).text)
        .toBe(t('entity.alliance_destroyer.name'));
    }
  });

  it('carries the numeric taps through unchanged', () => {
    d.win.__hostChannel('audio_level', 0.42);
    d.win.__hostChannel('shake', [3, -4]);
    expect(d.seen).toEqual([['audio_level', 0.42], ['shake', [3, -4]]]);
  });

  it('hands a non-JSON payload to the handler as Rust sent it', () => {
    d.win.__hostChannel('hud', 'not json {');
    expect(d.seen).toEqual([['hud', 'not json {']]);
  });

  it('passes through before the strings module has evaluated', () => {
    // phStrings is published by a module script; a push that beats it must not
    // throw, and t() on this page degrades the same way.
    const early = mountDispatcher({ strings: undefined });
    early.win.__hostChannel('lobby', JSON.stringify({ scenario_title: 'world.x.y' }));
    expect(payloadFor(early.seen, 'lobby').scenario_title).toBe('world.x.y');
  });

  it('warns rather than throwing on an unknown channel', () => {
    d.win.__hostChannel('not_a_channel', '{}');
    expect(d.seen).toEqual([]);
    expect(d.warnings.length).toBe(1);
  });
});
