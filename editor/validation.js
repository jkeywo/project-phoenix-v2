import { validateWorldToml } from './world-toml.js';
import { validateEntityToml, validateEntitySections } from './entity-toml.js';
import { validateStations } from './stations-validate.js';
import { validateWorldReferences } from './world-references.js';
import { validateWorldReferencesIndexed } from './world-references-indexed.js';
import { validateEntityMarkers } from './marker-validate.js';
import { validateBlasterBanks } from './blaster-validate.js';
import { validateTorpedoTubes } from './torpedo-validate.js';

/**
 * Pure admission primitive (issue #757). Mirrors the Rust atomic-activation
 * gate (`src/world/validate.rs`: `WorldFinding::is_error` / `has_error`).
 *
 * A validation record blocks a save/export iff its `severity === 'error'`.
 * `'warning'` (and any unrecognised severity) is non-blocking — it stays
 * visible but never refuses the write.
 *
 * @param {Array<{ severity?: string }>} results  Findings from `validateFile`.
 * @returns {boolean} true when at least one finding is a definite error.
 */
export function hasBlockingErrors(results) {
  if (!Array.isArray(results)) return false;
  return results.some((r) => r && r.severity === 'error');
}

/**
 * Split validation findings into blocking errors and non-blocking warnings.
 * The reusable admission primitive `SaveFlow` (and, later, the mod-pack
 * exporter, issue #759) consume this at the save/export chokepoint.
 *
 * @param {Array<{ severity?: string }>} results  Findings from `validateFile`.
 * @returns {{ errors: Array, warnings: Array }}
 */
export function partitionFindings(results) {
  const errors = [];
  const warnings = [];
  if (Array.isArray(results)) {
    for (const r of results) {
      if (r && r.severity === 'error') errors.push(r);
      else if (r) warnings.push(r);
    }
  }
  return { errors, warnings };
}

function isEntityFile(filePath, parsed) {
  if (filePath && filePath.includes('assets/entities/')) return true;
  if (parsed && 'tags' in parsed) return true;
  return false;
}

function isWorldFile(filePath, parsed) {
  if (filePath && filePath.includes('assets/worlds/')) return true;
  if (parsed && 'global' in parsed) return true;
  return false;
}

function findStationIndex(stationsConfig, count, stationName) {
  const defs = stationsConfig[count];
  if (!Array.isArray(defs)) return -1;
  return defs.findIndex((d) => d && d.name === stationName);
}

function translateStationError(stationsConfig, err) {
  const { count, station, message, type } = err;
  const idx = count != null && station ? findStationIndex(stationsConfig, count, station) : -1;
  const indexPart = idx >= 0 ? idx : 0;

  switch (type) {
    case 'duplicate-name':
      return { path: `stations.${count}.${indexPart}.name`, severity: 'error', message };
    case 'empty-consoles':
      return { path: `stations.${count}.${indexPart}.consoles`, severity: 'error', message };
    case 'unknown-console':
      return { path: `stations.${count}.${indexPart}.consoles`, severity: 'warning', message };
    case 'dangling-next':
      return { path: `stations.${count}.${indexPart}.next`, severity: 'error', message };
    case 'dangling-previous':
      return { path: `stations.${count}.${indexPart}.previous`, severity: 'error', message };
    case 'missing-next':
      return { path: `stations.${count}.${indexPart}.next`, severity: 'warning', message };
    case 'count-out-of-range':
      return { path: `stations.${count}`, severity: 'error', message };
    case 'parse-error':
      return { path: count != null ? `stations.${count}` : 'stations', severity: 'error', message };
    default:
      return { path: 'stations', severity: 'error', message };
  }
}

function validateBehaviourBlock(behaviour) {
  const results = [];
  const states = behaviour.state;
  const transitions = behaviour.transition;
  const initialState = behaviour.initial_state;
  const doctrine = behaviour.doctrine;

  // Doctrine-based AI (issue #572) — validate doctrine entries.
  if (Array.isArray(doctrine) && doctrine.length > 0) {
    for (let i = 0; i < doctrine.length; i++) {
      const d = doctrine[i];
      if (!d.id) {
        results.push({ path: `behaviour.doctrine[${i}]`, severity: 'error', message: 'Doctrine entry must have an id' });
      }
      if (!d.directive_kind) {
        results.push({ path: `behaviour.doctrine[${i}]`, severity: 'error', message: 'Doctrine entry must have a directive_kind' });
      }
      if (d.base_priority == null || typeof d.base_priority !== 'number') {
        results.push({ path: `behaviour.doctrine[${i}]`, severity: 'error', message: 'Doctrine entry must have a numeric base_priority' });
      }
      if ((d.directive_kind === 'Patrol' || d.directive_kind === 'patrol') && (!Array.isArray(d.directive_anchors) || d.directive_anchors.length === 0)) {
        results.push({ path: `behaviour.doctrine[${i}]`, severity: 'error', message: 'Patrol doctrine must have directive_anchors' });
      }
    }
    return results;
  }

  const stateNames = Array.isArray(states) ? states.map((s) => s.name).filter(Boolean) : [];
  const hasStates = Array.isArray(states) && states.length > 0;

  if (!hasStates) {
    results.push({ path: 'behaviour.state', severity: 'error', message: 'Must have at least one state' });
  }

  if (hasStates && !initialState) {
    results.push({ path: 'behaviour.initial_state', severity: 'error', message: 'initial_state is required when states are present' });
  }

  if (Array.isArray(states)) {
    const seen = new Set();
    for (const s of states) {
      if (s.name && seen.has(s.name)) {
        results.push({ path: 'behaviour.state', severity: 'error', message: `Duplicate state name: "${s.name}"` });
      }
      if (s.name) seen.add(s.name);
    }
  }

  if (initialState && hasStates && !stateNames.includes(initialState)) {
    results.push({ path: 'behaviour.initial_state', severity: 'warning', message: `initial_state "${initialState}" does not match any state name` });
  }

  if (Array.isArray(transitions)) {
    for (let i = 0; i < transitions.length; i++) {
      const t = transitions[i];
      if (Array.isArray(t.from)) {
        for (const f of t.from) {
          if (f && !stateNames.includes(f)) {
            results.push({ path: 'behaviour.transition', severity: 'warning', message: `Transition ${i}: from "${f}" is not a valid state name` });
          }
        }
      }
      if (t.to && !stateNames.includes(t.to)) {
        results.push({ path: 'behaviour.transition', severity: 'warning', message: `Transition ${i}: to "${t.to}" is not a valid state name` });
      }
    }
  }

  return results;
}

/**
 * @param {string} filePath
 * @param {object} parsedContent
 * @param {{ rigIndex?: import('./marker-validate.js').RigIndex }} [context]
 *   Optional cross-file context. When a `rigIndex` is supplied, entity marker
 *   references are resolved against the model rig the entity's `[mesh]`
 *   selects (issue #758). Without it — older callers, tests that only care
 *   about single-file checks — marker validation is skipped rather than
 *   reporting every marker as missing.
 */
export function validateFile(filePath, parsedContent, context = null) {
  if (!parsedContent || typeof parsedContent !== 'object' || Array.isArray(parsedContent)) {
    return [{ path: '', severity: 'error', message: 'Root value must be an object' }];
  }

  const results = [];

  if (isEntityFile(filePath, parsedContent)) {
    const entityResult = validateEntityToml(parsedContent);
    for (const msg of entityResult.errors) {
      const path = msg.includes('tag') ? 'tags' : '';
      results.push({ path, severity: 'error', message: msg });
    }

    const sectionsResult = validateEntitySections(parsedContent);
    for (const msg of sectionsResult.errors) {
      const path = msg.includes('shape') ? 'shape' : '';
      results.push({ path, severity: 'error', message: msg });
    }

    if (parsedContent.stations) {
      const stationsResult = validateStations(parsedContent.stations);
      for (const err of stationsResult.errors) {
        results.push(translateStationError(parsedContent.stations, err));
      }
    }

    if (parsedContent.behaviour) {
      const behaviourResults = validateBehaviourBlock(parsedContent.behaviour);
      results.push(...behaviourResults);
    }

    // Blaster barrel-pattern schema (issue #765): barrel-index references,
    // per-step barrel lists, and offsets must be valid, mirroring
    // `validate_blaster_banks` in src/entities/config.rs. Errors block save.
    if (parsedContent.weapons_console?.blaster_banks) {
      results.push(...validateBlasterBanks(parsedContent.weapons_console.blaster_banks));
    }

    // Torpedo barrel-pattern schema (issue #766): same barrel-index / per-step /
    // offset checks, mirroring `validate_torpedo_tubes` in
    // src/entities/config.rs. Errors block save via the #757 admission gate.
    if (parsedContent.torpedoes?.tubes) {
      results.push(...validateTorpedoTubes(parsedContent.torpedoes.tubes));
    }

    // Model-marker contract (issue #758). Cross-file: the entity's `[mesh]`
    // picks a rig sidecar, and every authored `marker` / `markers` entry must
    // name a marker that sidecar declares. Only runs when the caller supplied
    // a rig index — see the `context` parameter.
    const rigIndex = context?.rigIndex;
    if (rigIndex && typeof rigIndex.forEntity === 'function') {
      const rig = rigIndex.forEntity(parsedContent);
      if (rig !== undefined) {
        results.push(...validateEntityMarkers(parsedContent, rig));
      }
    }
  }

  if (isWorldFile(filePath, parsedContent)) {
    const worldResult = validateWorldToml(parsedContent);
    for (const msg of worldResult.errors) {
      let path = '';
      if (msg.includes('[global]')) path = 'global';
      else if (msg.includes('[anchors]')) path = 'anchors';
      results.push({ path, severity: 'error', message: msg });
    }

    // Per-action schema validation of `[[trigger.action]]` /
    // `[[comms.response.action]]` was removed with the card-based scenario
    // editor (#983): scenario logic is authored as Rhai now, whose vocabulary
    // is validated by the WASM diagnostics pass + the Rust cross-reference gate.
    // The cross-reference checks below (entity references) still run over any
    // declarative-TOML world so it keeps opening in a degraded but coherent view.

    // Resolve every entity reference in triggers and comms against the
    // set of `[[entity]] name = "..."` declared in this world.
    const refResults = validateWorldReferences(parsedContent);
    results.push(...refResults);

    // Slice 7: emit the same checks again with FULL INDEXED PATHS so
    // the badge layer can decorate the specific input field that is
    // broken (`world-references.js` keeps its human-readable context
    // strings for the messages list).
    const indexedRefResults = validateWorldReferencesIndexed(parsedContent);
    results.push(...indexedRefResults);
  }

  return results;
}
