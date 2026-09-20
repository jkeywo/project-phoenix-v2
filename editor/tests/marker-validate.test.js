import { describe, it, expect } from 'vitest';
import {
  CAMERA_MARKER_PREFIX,
  CATEGORY_DUPLICATE,
  CATEGORY_INCOMPATIBLE,
  CATEGORY_MISSING,
  CATEGORY_MISSING_CAMERA,
  CATEGORY_NO_RIG,
  RigIndex,
  collectMarkerRefs,
  isPlayerFlyable,
  isValidMarkerName,
  roleAcceptsMarker,
  sidecarPathFor,
  validateCameraView,
  validateEntityMarkers,
  validateRigMarkerNames,
  validateRigSidecarToml,
} from '../marker-validate.js';
import { validateFile } from '../validation.js';

const RIG = {
  markers: {
    phasers_fore: {},
    blaster_fore: {},
    torpedo_port: {},
    engine_port: {},
    camera_fore: {},
  },
};

function entity(extra) {
  return {
    tags: ['ship'],
    mesh: { model: 'assets/models/fixture.glb', shape: 'cuboid', colour: [1, 1, 1] },
    ...extra,
  };
}

describe('sidecarPathFor', () => {
  it('mirrors the engine default and named variants', () => {
    expect(sidecarPathFor('assets/models/x.glb')).toBe('assets/models/x.model.toml');
    expect(sidecarPathFor('assets/models/x.glb', 'model')).toBe('assets/models/x.model.toml');
    expect(sidecarPathFor('assets/models/x.glb', 'large')).toBe('assets/models/x.large.toml');
    expect(sidecarPathFor('assets/models/X.GLB')).toBe('assets/models/X.model.toml');
    expect(sidecarPathFor(null)).toBe(null);
  });
});

describe('collectMarkerRefs', () => {
  it('collects weapon and effect references with indexed paths', () => {
    const refs = collectMarkerRefs(
      entity({
        weapons_console: {
          phaser_banks: [{ id: 'fore', marker: 'phasers_fore' }],
          blaster_banks: [{ id: 'nose', marker: 'blaster_fore' }],
        },
        torpedoes: { tubes: [{ id: 'port', marker: 'torpedo_port' }] },
        helm_console: { engine_pfx: { markers: ['engine_port'] } },
      }),
    );
    expect(refs.map((r) => r.path)).toEqual([
      'weapons_console.phaser_banks[0].marker',
      'weapons_console.blaster_banks[0].marker',
      'torpedoes.tubes[0].marker',
      'helm_console.engine_pfx.markers[0]',
    ]);
    expect(refs.map((r) => r.role)).toEqual(['weapon', 'weapon', 'weapon', 'effect']);
  });

  it('ignores [[system]] marker — declared but unread in the engine', () => {
    const refs = collectMarkerRefs(entity({ system: [{ id: 'shields', marker: 'ship' }] }));
    expect(refs).toEqual([]);
  });

  it('emits one reference per authored barrel marker (issue #765)', () => {
    const refs = collectMarkerRefs(
      entity({
        weapons_console: {
          blaster_banks: [
            { id: 'twin', marker: 'blaster_fore', barrels: ['blaster_fore_port', 'blaster_fore_starboard'] },
          ],
        },
      }),
    );
    expect(refs.map((r) => r.path)).toEqual([
      'weapons_console.blaster_banks[0].marker',
      'weapons_console.blaster_banks[0].barrels[0]',
      'weapons_console.blaster_banks[0].barrels[1]',
    ]);
    expect(refs.map((r) => r.name)).toEqual([
      'blaster_fore',
      'blaster_fore_port',
      'blaster_fore_starboard',
    ]);
  });

  it('emits one reference per authored torpedo barrel marker (issue #766)', () => {
    const refs = collectMarkerRefs(
      entity({
        torpedoes: {
          tubes: [
            { id: 'centre', marker: 'torpedo_centre', barrels: ['torpedo_centre_port', 'torpedo_centre_starboard'] },
          ],
        },
      }),
    );
    expect(refs.map((r) => r.path)).toEqual([
      'torpedoes.tubes[0].marker',
      'torpedoes.tubes[0].barrels[0]',
      'torpedoes.tubes[0].barrels[1]',
    ]);
    expect(refs.map((r) => r.name)).toEqual([
      'torpedo_centre',
      'torpedo_centre_port',
      'torpedo_centre_starboard',
    ]);
  });
});

describe('validateEntityMarkers — representative systems, success and failure', () => {
  it('phaser bank resolves, then fails when misspelled', () => {
    const ok = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'phasers_fore' }] } });
    expect(validateEntityMarkers(ok, RIG)).toEqual([]);

    const bad = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'phasers_front' }] } });
    const findings = validateEntityMarkers(bad, RIG);
    expect(findings).toHaveLength(1);
    expect(findings[0].category).toBe(CATEGORY_MISSING);
    expect(findings[0].severity).toBe('error');
    expect(findings[0].path).toBe('weapons_console.phaser_banks[0].marker');
    expect(findings[0].message).toContain('Phaser bank "fore"');
  });

  it('blaster bank resolves, then fails when misspelled', () => {
    const ok = entity({ weapons_console: { blaster_banks: [{ id: 'fore', marker: 'blaster_fore' }] } });
    expect(validateEntityMarkers(ok, RIG)).toEqual([]);

    const bad = entity({ weapons_console: { blaster_banks: [{ id: 'fore', marker: 'blaster_nose' }] } });
    expect(validateEntityMarkers(bad, RIG)[0].category).toBe(CATEGORY_MISSING);
  });

  it('torpedo tube resolves, then fails when misspelled', () => {
    const ok = entity({ torpedoes: { tubes: [{ id: 'port', marker: 'torpedo_port' }] } });
    expect(validateEntityMarkers(ok, RIG)).toEqual([]);

    const bad = entity({ torpedoes: { tubes: [{ id: 'port', marker: 'torpdo_port' }] } });
    const findings = validateEntityMarkers(bad, RIG);
    expect(findings[0].category).toBe(CATEGORY_MISSING);
    expect(findings[0].path).toBe('torpedoes.tubes[0].marker');
  });

  it('engine PFX markers resolve, then fail when misspelled', () => {
    const ok = entity({ helm_console: { engine_pfx: { markers: ['engine_port'] } } });
    expect(validateEntityMarkers(ok, RIG)).toEqual([]);

    const bad = entity({ helm_console: { engine_pfx: { markers: ['engine_starbord'] } } });
    const findings = validateEntityMarkers(bad, RIG);
    expect(findings[0].category).toBe(CATEGORY_MISSING);
    expect(findings[0].path).toBe('helm_console.engine_pfx.markers[0]');
  });

  it('camera view resolves, and fails as missing or incompatible', () => {
    expect(validateCameraView('camera_fore', RIG)).toEqual([]);
    expect(validateCameraView('camera_aft', RIG)[0].category).toBe(CATEGORY_MISSING);
    expect(validateCameraView('engine_port', RIG)[0].category).toBe(CATEGORY_INCOMPATIBLE);
  });

  it('a weapon pointing at a camera marker is incompatible, not missing', () => {
    const bad = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'camera_fore' }] } });
    const findings = validateEntityMarkers(bad, RIG);
    expect(findings).toHaveLength(1);
    expect(findings[0].category).toBe(CATEGORY_INCOMPATIBLE);
    expect(findings[0].message).toContain(CAMERA_MARKER_PREFIX);
  });

  it('references with no resolvable rig are errors', () => {
    const bad = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'phasers_fore' }] } });
    const findings = validateEntityMarkers(bad, null);
    expect(findings).toHaveLength(1);
    expect(findings[0].category).toBe(CATEGORY_NO_RIG);
  });

  it('an entity with no marker references is clean even without a rig', () => {
    expect(validateEntityMarkers(entity({}), null)).toEqual([]);
  });

  it('warns when a playable hull rig lacks the default camera marker', () => {
    const hull = entity({ captain_console: {} });
    const findings = validateEntityMarkers(hull, { markers: { engine_port: {} } });
    expect(findings).toHaveLength(1);
    expect(findings[0].category).toBe(CATEGORY_MISSING_CAMERA);
    expect(findings[0].severity).toBe('warning');

    expect(validateEntityMarkers(hull, RIG)).toEqual([]);
  });

  it('does not demand a bridge viewpoint from an NPC hull', () => {
    // Every AI-bearing hull authors `[captain_console.ai]` since #885b, so the
    // section is on NPC designs too and no longer means "a player flies this".
    // The `npc` tag is what still says it — and dropping the tag brings the
    // warning straight back, so the check has not been weakened for the hulls
    // it exists for.
    const npc = entity({ tags: ['ship', 'npc', 'enemy'], captain_console: { ai: { idle: true } } });
    expect(validateEntityMarkers(npc, { markers: { engine_port: {} } })).toEqual([]);
    expect(isPlayerFlyable(npc)).toBe(false);

    const player = { ...npc, tags: ['ship'] };
    expect(isPlayerFlyable(player)).toBe(true);
    expect(validateEntityMarkers(player, { markers: { engine_port: {} } })[0].category).toBe(
      CATEGORY_MISSING_CAMERA,
    );
  });
});

describe('rig sidecar checks', () => {
  it('flags a duplicate [markers.<name>] table', () => {
    const text = [
      '[markers.engine_port]',
      'position = [0, 0, 0]',
      '',
      '[markers.engine_port]',
      'position = [1, 0, 0]',
    ].join('\n');
    const findings = validateRigSidecarToml(text);
    expect(findings).toHaveLength(1);
    expect(findings[0].category).toBe(CATEGORY_DUPLICATE);
    expect(findings[0].severity).toBe('error');
    expect(findings[0].message).toContain('Line 4');
  });

  it('accepts distinct marker tables', () => {
    expect(validateRigSidecarToml('[markers.a]\n[markers.b]\n')).toEqual([]);
  });

  it('rejects marker names that are not valid rig keys', () => {
    expect(isValidMarkerName('engine_port')).toBe(true);
    expect(isValidMarkerName('engine port')).toBe(false);
    expect(isValidMarkerName('engine.port')).toBe(false);
    expect(isValidMarkerName('')).toBe(false);
    const findings = validateRigMarkerNames({ markers: { 'engine port': {} } });
    expect(findings).toHaveLength(1);
    expect(findings[0].severity).toBe('error');
  });
});

describe('role namespace rules', () => {
  it('reserves the camera_ prefix for cameras', () => {
    expect(roleAcceptsMarker('camera', 'camera_fore')).toBe(true);
    expect(roleAcceptsMarker('camera', 'phasers_fore')).toBe(false);
    expect(roleAcceptsMarker('weapon', 'phasers_fore')).toBe(true);
    expect(roleAcceptsMarker('weapon', 'camera_fore')).toBe(false);
    expect(roleAcceptsMarker('effect', 'camera_aft')).toBe(false);
  });
});

describe('validateFile integration', () => {
  const path = 'assets/entities/fixture.toml';

  function indexed() {
    return new RigIndex().set('assets/models/fixture.model.toml', RIG);
  }

  it('skips marker checks with no rig index (back-compat two-arg call)', () => {
    const bad = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'nope' }] } });
    expect(validateFile(path, bad).some((r) => r.category === CATEGORY_MISSING)).toBe(false);
  });

  it('reports an unresolved marker when a rig index is supplied', () => {
    const bad = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'nope' }] } });
    const results = validateFile(path, bad, { rigIndex: indexed() });
    const marker = results.filter((r) => r.category === CATEGORY_MISSING);
    expect(marker).toHaveLength(1);
    expect(marker[0].severity).toBe('error');
  });

  it('stays clean when the marker resolves', () => {
    const ok = entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'phasers_fore' }] } });
    const results = validateFile(path, ok, { rigIndex: indexed() });
    expect(results.filter((r) => r.category)).toEqual([]);
  });

  it('skips entities whose sidecar is not indexed rather than failing them', () => {
    const other = {
      tags: [],
      mesh: { model: 'assets/models/unknown.glb' },
      weapons_console: { phaser_banks: [{ id: 'fore', marker: 'whatever' }] },
    };
    expect(validateFile(path, other, { rigIndex: indexed() })
      .some((r) => r.category === CATEGORY_MISSING)).toBe(false);
  });
});
