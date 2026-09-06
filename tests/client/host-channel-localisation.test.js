// Issue #949 — the host channel is a localisation boundary.
//
// Rust pushes authored DATA over the host channels, and authored data holds
// string ids: a world's `[global] title` on the lobby channel, a `game_over`
// trigger's `message` on the hud channel. A phone crosses localiseTree() once
// in gui/connection-manager.js and renders those resolved; the host page had no
// equivalent boundary, so `#lobby-title` read `world.combat_test.global.title`
// verbatim — the symptom #949 names.
//
// Issue #1225 lifted the dispatcher, the handler table and the localisation
// boundary out of server.html's inline classic script into gui/host-channel.js,
// so this test imports the real module directly rather than scraping
// server.html's source text and re-evaluating it.

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { createHostChannel, localiseHostPayload } from '../../gui/host-channel.js';
import { t, has, localiseTree } from '../../gui/strings.js';

/**
 * The real dispatcher, wired to stub handlers that record what they were
 * handed.
 */
function mountDispatcher({ strings = { t, has, localiseTree } } = {}) {
  const seen = [];
  const record = (channel) => (payload) => seen.push([channel, payload]);
  const handlers = {
    hud: record('hud'),
    lobby: record('lobby'),
    chatter: record('chatter'),
    audio_config: record('audio_config'),
    audio_cue: record('audio_cue'),
    audio_level: record('audio_level'),
    shake: record('shake'),
    gm_entity: record('gm_entity'),
    gm_activity: record('gm_activity'),
    gm_spawn: record('gm_spawn'),
    gm_session: record('gm_session'),
  };
  const dispatch = createHostChannel({ handlers, strings });
  return { dispatch, seen };
}

/** What the handler for `channel` was handed, JSON-decoded. */
function payloadFor(seen, channel) {
  const hit = seen.find(([name]) => name === channel);
  if (!hit) throw new Error(`nothing dispatched on ${channel}`);
  return JSON.parse(hit[1]);
}

describe('host channel localisation boundary', () => {
  let d;
  let warnSpy;

  beforeEach(() => {
    d = mountDispatcher();
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
  });

  it('resolves the lobby scenario title and body — the reported symptom', () => {
    // Exactly what src/world/server.rs puts on the wire for combat_test.toml.
    d.dispatch('lobby', JSON.stringify({
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
    d.dispatch('hud', JSON.stringify({
      heading: 7,
      hull_pct: 64,
      game_over_message: 'world.combat_test.trigger.action.18.message',
    }));
    const p = payloadFor(d.seen, 'hud');
    expect(p.game_over_message).toBe(t('world.combat_test.trigger.action.18.message'));
    // A missing id resolves to itself, which would let the assertion above
    // pass vacuously — so also require actual resolution happened.
    expect(p.game_over_message).not.toBe('world.combat_test.trigger.action.18.message');
  });

  it('resolves an id nested inside an array of station payloads', () => {
    // Station names are English in the shipped hulls today, but the payload is
    // a tree and the boundary must reach all of it — not just the two fields
    // that happened to be reported.
    d.dispatch('lobby', JSON.stringify({
      phase: 'Lobby',
      stations: [{ name: 'entity.alliance_destroyer.name', rank: 'Cpt.' }],
    }));
    const p = payloadFor(d.seen, 'lobby');
    expect(p.stations[0].name).toBe(t('entity.alliance_destroyer.name'));
    expect(p.stations[0].rank).toBe('Cpt.');
  });

  it('leaves machine tokens, player names and authored prose alone', () => {
    d.dispatch('lobby', JSON.stringify({
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
    d.dispatch('lobby', JSON.stringify({ scenario_title: resolved }));
    expect(payloadFor(d.seen, 'lobby').scenario_title).toBe(resolved);
  });

  it('routes every channel through the boundary, not just the lobby', () => {
    for (const channel of [
      'hud', 'chatter', 'audio_config', 'audio_cue', 'gm_session',
    ]) {
      const fresh = mountDispatcher();
      fresh.dispatch(channel, JSON.stringify({
        text: 'entity.alliance_destroyer.name',
      }));
      expect(payloadFor(fresh.seen, channel).text)
        .toBe(t('entity.alliance_destroyer.name'));
    }
  });

  it('delivers the strict gm_entity DTO raw before projection parsing', () => {
    const localiseSpy = vi.fn(localiseTree);
    const raw = mountDispatcher({ strings: { t, has, localiseTree: localiseSpy } });
    const dto = {
      entities: [{
        entity_id: '00000000-0000-4000-8000-000000000001',
        name: 'entity.alliance_destroyer.display_name',
        kind: 'structure',
        position: [12, 0, -8],
        faction: {
          entity_id: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
          name: 'faction.alliance.display_name',
        },
        status: { hull_percent: null, condition_percent: null, destroyed: false },
        current_target: {
          entity_id: '00000000-0000-4000-8000-000000000002',
          name: 'entity.alliance_destroyer.display_name',
        },
        geometry: null,
        // An opaque authored visual token is allowed to look like a String
        // Table key. It must not be rewritten before the strict DTO adapter.
        radar: {
          icon: 'entity.alliance_destroyer.name',
          colour: [0.9, 0.7, 0.2],
          size: 5,
          region_colour: null,
        },
      }],
    };

    expect(has(dto.entities[0].name)).toBe(true);
    expect(has(dto.entities[0].faction.name)).toBe(true);
    expect(has(dto.entities[0].radar.icon)).toBe(true);
    raw.dispatch('gm_entity', JSON.stringify(dto));

    expect(payloadFor(raw.seen, 'gm_entity')).toEqual(dto);
    expect(localiseSpy).not.toHaveBeenCalled();
    expect(t(dto.entities[0].name)).not.toBe(dto.entities[0].name);
  });

  it('delivers the gm_spawn palette DTO raw, ids and labels untouched', () => {
    // The spawn projection (issue #1305) is the sharpest case for the raw-DTO
    // rule: every authored `[[gm_palette]]` row REQUIRES a String Table label
    // id, and the sibling `id` is exactly what the typed SpawnPaletteEntity
    // action carries on the wire. Localising here would resolve the label a
    // first time (so the panel's own t() would then miss and render
    // `⟨[Support Tender]⟩`) and would rewrite any id that happened to equal a
    // table key.
    const localiseSpy = vi.fn(localiseTree);
    const raw = mountDispatcher({ strings: { t, has, localiseTree: localiseSpy } });
    const dto = {
      palette: [{
        id: 'world.smoke_gm_palette.tender.label',
        label: 'world.smoke_gm_palette.tender.label',
        variants: [{
          id: 'world.smoke_gm_palette.tender.escort.label',
          label: 'world.smoke_gm_palette.tender.escort.label',
        }],
      }],
      results: [{
        operator_id: 'gm-1',
        correlation: 'spawn-1',
        action_kind: 'WorldSpawn',
        requested_active: true,
        outcome: 'Applied',
        tick: 12,
        target: 'world.smoke_gm_palette.tender.label',
      }],
    };

    expect(has(dto.palette[0].label)).toBe(true);
    expect(has(dto.palette[0].variants[0].label)).toBe(true);
    const wire = JSON.stringify(dto);
    raw.dispatch('gm_spawn', wire);

    expect(raw.seen.find(([name]) => name === 'gm_spawn')[1]).toBe(wire);
    expect(payloadFor(raw.seen, 'gm_spawn')).toEqual(dto);
    expect(localiseSpy).not.toHaveBeenCalled();
    // The panel — not the dispatcher — is what turns the id into English.
    expect(t(dto.palette[0].label)).not.toBe(dto.palette[0].label);
  });

  it('delivers gm_activity raw without mutating identities, weapons, or systems', () => {
    const localiseSpy = vi.fn(localiseTree);
    const raw = mountDispatcher({ strings: { t, has, localiseTree: localiseSpy } });
    const dto = {
      capacity: 4,
      entries: [{
        tick: 9,
        category: 'damage',
        ships: [{
          entity_id: 'entity.alliance_destroyer.name',
          name: 'entity.alliance_destroyer.display_name',
        }],
        links: [{
          role: 'victim',
          entity: {
            entity_id: 'entity.alliance_destroyer.name',
            name: 'entity.alliance_destroyer.display_name',
          },
        }],
        detail: {
          type: 'damage',
          data: {
            victim_kind: 'ship',
            weapon: 'entity.alliance_destroyer.name',
            amount: 2,
            shield_absorbed: 0,
            hull_damage: 2,
            system_hit: 'entity.alliance_destroyer.display_name',
          },
        },
      }],
    };
    raw.dispatch('gm_activity', JSON.stringify(dto));
    expect(payloadFor(raw.seen, 'gm_activity')).toEqual(dto);
    expect(localiseSpy).not.toHaveBeenCalled();
  });

  it('carries the numeric taps through unchanged', () => {
    d.dispatch('audio_level', 0.42);
    d.dispatch('shake', [3, -4]);
    expect(d.seen).toEqual([['audio_level', 0.42], ['shake', [3, -4]]]);
  });

  it('hands a non-JSON payload to the handler as Rust sent it', () => {
    d.dispatch('hud', 'not json {');
    expect(d.seen).toEqual([['hud', 'not json {']]);
  });

  it('passes through before the strings module has evaluated', () => {
    // phStrings is published by a module script; a push that beats it must not
    // throw, and t() on this page degrades the same way.
    const early = mountDispatcher({ strings: undefined });
    early.dispatch('lobby', JSON.stringify({ scenario_title: 'world.x.y' }));
    expect(payloadFor(early.seen, 'lobby').scenario_title).toBe('world.x.y');
  });

  it('warns rather than throwing on an unknown channel', () => {
    d.dispatch('not_a_channel', '{}');
    expect(d.seen).toEqual([]);
    expect(warnSpy).toHaveBeenCalledTimes(1);
  });
});

describe('localiseHostPayload', () => {
  const strings = { t, has, localiseTree };

  it('resolves a JSON-string payload and re-encodes it', () => {
    const payload = JSON.stringify({ scenario_title: 'world.combat_test.global.title' });
    expect(localiseHostPayload(payload, strings)).toBe(
      JSON.stringify({ scenario_title: t('world.combat_test.global.title') }),
    );
  });

  it('resolves ids inside a non-string (already-decoded) payload — the numeric taps', () => {
    // audio_level / shake never carry a JSON-string payload; localiseTree runs
    // directly, and neither a bare number nor an [x, y] array holds a string
    // id, so both pass through unchanged.
    expect(localiseHostPayload(0.42, strings)).toBe(0.42);
    expect(localiseHostPayload([3, -4], strings)).toEqual([3, -4]);
  });

  it('returns a non-JSON string payload untouched rather than throwing', () => {
    expect(localiseHostPayload('not json {', strings)).toBe('not json {');
  });

  it('returns the payload unchanged when strings has no localiseTree', () => {
    const payload = JSON.stringify({ scenario_title: 'world.x.y' });
    expect(localiseHostPayload(payload, {})).toBe(payload);
    expect(localiseHostPayload(payload, null)).toBe(payload);
    expect(localiseHostPayload(payload, undefined)).toBe(payload);
  });
});
