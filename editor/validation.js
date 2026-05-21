import { validateWorldToml } from './world-toml.js';
import { validateEntityToml, validateEntitySections } from './entity-toml.js';
import { validateStations } from './stations-validate.js';
import { validateTriggerActions } from './action-schema.js';
import { validateWorldReferences } from './world-references.js';

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

export function validateFile(filePath, parsedContent) {
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
  }

  if (isWorldFile(filePath, parsedContent)) {
    const worldResult = validateWorldToml(parsedContent);
    for (const msg of worldResult.errors) {
      let path = '';
      if (msg.includes('[global]')) path = 'global';
      else if (msg.includes('[anchors]')) path = 'anchors';
      results.push({ path, severity: 'error', message: msg });
    }

    // Validate every `[[trigger.action]]` and `[[comms.response.action]]`
    // against the action schema (mirrors TriggerAction in src/world/config.rs).
    if (Array.isArray(parsedContent.trigger)) {
      for (let i = 0; i < parsedContent.trigger.length; i++) {
        const trigger = parsedContent.trigger[i];
        if (Array.isArray(trigger?.action)) {
          const r = validateTriggerActions(trigger.action);
          for (const msg of r.errors) {
            results.push({
              path: `trigger[${i}].action`,
              severity: 'error',
              message: msg,
            });
          }
        }
      }
    }
    if (Array.isArray(parsedContent.comms)) {
      for (let i = 0; i < parsedContent.comms.length; i++) {
        const block = parsedContent.comms[i];
        if (Array.isArray(block?.response)) {
          for (let r = 0; r < block.response.length; r++) {
            const resp = block.response[r];
            if (Array.isArray(resp?.action)) {
              const result = validateTriggerActions(resp.action);
              for (const msg of result.errors) {
                results.push({
                  path: `comms[${i}].response[${r}].action`,
                  severity: 'error',
                  message: msg,
                });
              }
            }
          }
        }
      }
    }

    // Resolve every entity reference in triggers and comms against the
    // set of `[[entity]] name = "..."` declared in this world.
    const refResults = validateWorldReferences(parsedContent);
    results.push(...refResults);
  }

  return results;
}
