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
  isValidMarkerName,
  roleAcceptsMarker,
  sidecarPathFor,
  validateCameraView,
  validateEntityMarkers,
  validateRigMarkerNames,
  validateRigSidecarToml,
} from '../marker-validate.js';
import { validateFile } from '../validation.js';
import { SaveFlow } from '../save-flow.js';
import { InvalidationBus } from '../invalidation-bus.js';
import { parseRigToml, validateRigSidecarText, wireRigIndexToSaves } from '../models-rig.js';

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

describe('SaveFlow admission gate', () => {
  function makeShell() {
    const dirty = {};
    return {
      getCurrentMode: () => 'Entity',
      getActiveFile: () => 'assets/entities/fixture.toml',
      getModes: () => ['Entity'],
      getOpenFiles: () => ['assets/entities/fixture.toml'],
      isDirty: (m, p) => !!dirty[`${m}:${p}`],
      markDirty: (m, p, v) => { dirty[`${m}:${p}`] = v; },
      clearUndoHistory: () => {},
    };
  }

  function makeFlow(writes) {
    const flow = new SaveFlow(
      makeShell(),
      { entity: () => 'entity-toml', models: (s) => s },
      async (p, c) => { writes.push([p, c]); },
    );
    flow.setRigIndex(new RigIndex().set('assets/models/fixture.model.toml', RIG));
    return flow;
  }

  it('blocks an entity save whose marker does not resolve — nothing is written', async () => {
    const writes = [];
    const flow = makeFlow(writes);
    flow.setContent(
      'Entity',
      'assets/entities/fixture.toml',
      entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'phasers_front' }] } }),
    );
    const result = await flow.saveActive();
    expect(result.ok).toBe(false);
    expect(result.errors.join(' ')).toContain('phasers_front');
    expect(writes).toEqual([]);
  });

  it('allows the save once the marker resolves', async () => {
    const writes = [];
    const flow = makeFlow(writes);
    flow.setContent(
      'Entity',
      'assets/entities/fixture.toml',
      entity({ weapons_console: { phaser_banks: [{ id: 'fore', marker: 'phasers_fore' }] } }),
    );
    const result = await flow.saveActive();
    expect(result.errors).toEqual([]);
    expect(result.ok).toBe(true);
    expect(writes).toHaveLength(1);
  });

  it('blocks a Models-mode save whose sidecar redeclares a marker', async () => {
    const writes = [];
    const shell = makeShell();
    shell.getCurrentMode = () => 'Models';
    shell.getActiveFile = () => 'assets/models/fixture.model.toml';
    const flow = new SaveFlow(
      shell,
      { entity: () => '', models: (s) => s },
      async (p, c) => { writes.push([p, c]); },
    );
    flow.setContent(
      'Models',
      'assets/models/fixture.model.toml',
      '[markers.a]\nposition = [0,0,0]\n\n[markers.a]\nposition = [1,0,0]\n',
    );
    const result = await flow.saveActive();
    expect(result.ok).toBe(false);
    expect(writes).toEqual([]);
  });

  it('allows a well-formed Models-mode save', async () => {
    const writes = [];
    const shell = makeShell();
    shell.getCurrentMode = () => 'Models';
    shell.getActiveFile = () => 'assets/models/fixture.model.toml';
    const flow = new SaveFlow(
      shell,
      { entity: () => '', models: (s) => s },
      async (p, c) => { writes.push([p, c]); },
    );
    flow.setContent('Models', 'assets/models/fixture.model.toml', '[markers.a]\nposition = [0,0,0]\n');
    const result = await flow.saveActive();
    expect(result.ok).toBe(true);
    expect(writes).toHaveLength(1);
  });
});

describe('rig index freshness across a session (issue #758)', () => {
  const ENTITY_PATH = 'assets/entities/fixture.toml';
  const SIDECAR_PATH = 'assets/models/fixture.model.toml';

  function makeShell(mode) {
    const dirty = {};
    let active = { Entity: ENTITY_PATH, Models: SIDECAR_PATH };
    let current = mode;
    return {
      setMode: (m) => { current = m; },
      getCurrentMode: () => current,
      getActiveFile: (m) => active[m ?? current],
      getModes: () => ['Entity', 'Models'],
      getOpenFiles: (m) => [active[m]],
      isDirty: (m, p) => !!dirty[`${m}:${p}`],
      markDirty: (m, p, v) => { dirty[`${m}:${p}`] = v; },
      clearUndoHistory: () => {},
    };
  }

  // The rig on disk when the session starts: no `torpedo_dorsal` yet.
  const INITIAL_SIDECAR = [
    '[markers.phasers_fore]',
    'position = [0, 0, -1]',
    '',
    '[markers.camera_fore]',
    'position = [0, 0, -2]',
    '',
  ].join('\n');

  function session() {
    const writes = [];
    const shell = makeShell('Models');
    const bus = new InvalidationBus();
    const rigIndex = new RigIndex().set(SIDECAR_PATH, parseRigToml(INITIAL_SIDECAR));
    wireRigIndexToSaves(rigIndex, bus);
    const flow = new SaveFlow(
      shell,
      { entity: () => 'entity-toml', models: (s) => s },
      async (p, c) => { writes.push([p, c]); },
      bus,
    );
    flow.setRigIndex(rigIndex);
    return { writes, shell, bus, rigIndex, flow };
  }

  const withDorsalTube = entity({
    torpedoes: { tubes: [{ id: 'dorsal', marker: 'torpedo_dorsal' }] },
  });

  it('refuses the entity save while the rig has no such marker', async () => {
    const { shell, flow, writes } = session();
    shell.setMode('Entity');
    flow.setContent('Entity', ENTITY_PATH, withDorsalTube);

    const result = await flow.saveActive();
    expect(result.ok).toBe(false);
    expect(result.errors.join(' ')).toContain('torpedo_dorsal');
    expect(writes).toEqual([]);
  });

  it('accepts the entity save after the marker is added to the rig IN THE SAME SESSION', async () => {
    const { shell, flow, writes } = session();

    // 1. Author adds `torpedo_dorsal` in Models Mode and saves the sidecar.
    const updatedSidecar = `${INITIAL_SIDECAR}\n[markers.torpedo_dorsal]\nposition = [0, 1, 0]\n`;
    flow.setContent('Models', SIDECAR_PATH, updatedSidecar);
    const rigResult = await flow.saveActive();
    expect(rigResult.ok).toBe(true);

    // 2. Switches to Entity Mode and points a tube at the new marker.
    //    No reload happens in between — the index must already know.
    shell.setMode('Entity');
    flow.setContent('Entity', ENTITY_PATH, withDorsalTube);

    const result = await flow.saveActive();
    expect(result.errors).toEqual([]);
    expect(result.ok).toBe(true);
    expect(writes.map(([p]) => p)).toEqual([SIDECAR_PATH, ENTITY_PATH]);
  });

  it('re-seeds the index from a Models write fired directly on the bus', () => {
    const { bus, rigIndex } = session();
    bus.fireModelSaved(SIDECAR_PATH, '[markers.late_addition]\nposition = [0, 0, 0]\n');
    expect(Object.keys(rigIndex.get(SIDECAR_PATH).markers)).toEqual(['late_addition']);
  });

  it('drops the entry rather than keeping a stale rig when the written text will not parse', () => {
    const { bus, rigIndex } = session();
    bus.fireModelSaved(SIDECAR_PATH, 'this is not = = toml');
    expect(rigIndex.has(SIDECAR_PATH)).toBe(false);
    // "unknown" means marker checks are SKIPPED, not that everything fails.
    const results = validateFile(ENTITY_PATH, withDorsalTube, { rigIndex });
    expect(results.some((r) => r.category === CATEGORY_MISSING)).toBe(false);
  });
});

describe('both Models write paths apply the same rule set (issue #758)', () => {
  // `models-mode-view.js` writeRig and the SaveFlow 'Models' branch both call
  // validateRigSidecarText, so a sidecar one refuses cannot go through the
  // other. An unusable marker NAME is the case only the view used to catch.
  const UNUSABLE_NAME = '[markers."engine port"]\nposition = [0, 0, 0]\n';

  it('rejects an unusable marker name through the shared validator', () => {
    const findings = validateRigSidecarText(UNUSABLE_NAME);
    expect(findings.some((f) => f.severity === 'error' && f.message.includes('engine port')))
      .toBe(true);
  });

  it('blocks a SaveFlow Models save on an unusable marker name', async () => {
    const writes = [];
    const shell = {
      getCurrentMode: () => 'Models',
      getActiveFile: () => 'assets/models/fixture.model.toml',
      getModes: () => ['Models'],
      getOpenFiles: () => ['assets/models/fixture.model.toml'],
      isDirty: () => false,
      markDirty: () => {},
      clearUndoHistory: () => {},
    };
    const flow = new SaveFlow(
      shell,
      { entity: () => '', models: (s) => s },
      async (p, c) => { writes.push([p, c]); },
    );
    flow.setContent('Models', 'assets/models/fixture.model.toml', UNUSABLE_NAME);
    const result = await flow.saveActive();
    expect(result.ok).toBe(false);
    expect(writes).toEqual([]);
  });
});
