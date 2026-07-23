/**
 * world-references-indexed.js — Mirror of `world-references.js` that
 * emits validation records with FULL INDEXED PATHS so the badge layer
 * can decorate the specific field that is broken.
 *
 * `world-references.js` emits non-indexed `context` strings (e.g.
 * `"trigger.action entity"`) which are correct for the human-readable
 * message but useless as a `data-validation-path` key — multiple
 * triggers / actions would all collide on the same path.
 *
 * This module walks the same shape (`worldObj.trigger[*]` and
 * `worldObj.comms[*].response[*].action[*]`) and emits paths like:
 *   trigger[3].entity
 *   trigger[3].action[1].entity
 *   trigger[3].action[2].target_entity
 *   comms[0].entity
 *   comms[0].response[1].action[0].entity
 *
 * `comms.from` mirrors `world-references.js:54` — it is free-form
 * (display string) and NOT validated.
 *
 * Anchor refs on `[[entity]] anchor = "..."` remain unvalidated here
 * (the Rust side surfaces missing anchors at world-parse time).
 */

/**
 * @param {object} worldObj  Parsed world TOML.
 * @returns {Array<{ path: string, severity: 'warning', message: string }>}
 */
export function validateWorldReferencesIndexed(worldObj) {
  const results = [];
  if (!worldObj || typeof worldObj !== 'object') return results;

  const knownEntities = new Set();
  if (Array.isArray(worldObj.entity)) {
    for (const ent of worldObj.entity) {
      if (ent && typeof ent.name === 'string') knownEntities.add(ent.name);
    }
  }

  const emit = (path, name) => {
    // Non-blocking warning — see world-references.js (issue #757): dangling
    // entity cross-references mirror the Rust validator's `Severity::Warning`
    // for bare unresolved references and must not refuse a save.
    results.push({
      path,
      severity: 'warning',
      message: `Reference to unknown entity "${name}" (${path})`,
    });
  };

  const checkRef = (name, path) => {
    if (!name || typeof name !== 'string') return;
    if (knownEntities.has(name)) return;
    emit(path, name);
  };

  // ── Triggers ──────────────────────────────────────────────────────────
  if (Array.isArray(worldObj.trigger)) {
    worldObj.trigger.forEach((trigger, i) => {
      if (trigger && typeof trigger === 'object') {
        if (trigger.entity) checkRef(trigger.entity, `trigger[${i}].entity`);
        if (Array.isArray(trigger.action)) {
          trigger.action.forEach((action, j) => {
            checkActionRefs(action, `trigger[${i}].action[${j}]`, checkRef);
          });
        }
      }
    });
  }

  // ── Comms ─────────────────────────────────────────────────────────────
  if (Array.isArray(worldObj.comms)) {
    worldObj.comms.forEach((block, i) => {
      if (!block || typeof block !== 'object') return;
      if (block.entity) checkRef(block.entity, `comms[${i}].entity`);
      // `comms[i].from` is free-form — DO NOT validate.
      if (Array.isArray(block.response)) {
        block.response.forEach((resp, r) => {
          if (!resp || !Array.isArray(resp.action)) return;
          resp.action.forEach((action, j) => {
            checkActionRefs(action, `comms[${i}].response[${r}].action[${j}]`, checkRef);
          });
        });
      }
    });
  }

  return results;
}

function checkActionRefs(action, basePath, checkRef) {
  if (!action || typeof action !== 'object') return;
  if (action.entity) checkRef(action.entity, `${basePath}.entity`);
  if (action.target_entity) checkRef(action.target_entity, `${basePath}.target_entity`);
}
