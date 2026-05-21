/**
 * comms-adapter.js
 *
 * Pure adapter between the live TOML shape of `[[comms]]` blocks (see
 * `assets/worlds/default.toml`) and the shape consumed by `CommsEditor`
 * / `comms-view.js`.
 *
 * Live TOML shape (per template):
 *   { from, trigger, entity?, message, response: [
 *       { text, action: [...], follow_up?: { message, response: [...] } }
 *   ] }
 *
 * Editor shape (per template):
 *   { from,
 *     trigger: { kind, entity },   // on_attacked | on_destroyed | on_hailed
 *     node:    { body, responses: [
 *       { text, actions: [...], follow_up?: { body, responses: [...] } }
 *     ] }
 *   }
 *
 * Notes on `on_timer` (audit risk #7): `RawCommsEntry` in
 * `src/world/config.rs` does NOT include an `after_secs` field, and the
 * parser passes `None` for after_secs when parsing comms templates. So
 * `on_timer` is NOT supported at the comms-template level. The adapter
 * does NOT model `after_secs` for comms; the view layer restricts the
 * trigger `<select>` to the three supported variants.
 *
 * Round-trip contract: `editorCommsToWorld(worldCommsToEditor(x)) ≈ x`
 * for any well-formed live comms array.
 */

function deepClone(value) {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) return value.map(deepClone);
  const out = {};
  for (const k of Object.keys(value)) {
    out[k] = deepClone(value[k]);
  }
  return out;
}

function mapResponseToEditor(resp) {
  const out = {
    text: resp.text ?? '',
    actions: Array.isArray(resp.action) ? deepClone(resp.action) : [],
  };
  if (resp.follow_up && typeof resp.follow_up === 'object') {
    out.follow_up = mapNodeToEditor(resp.follow_up);
  }
  return out;
}

function mapNodeToEditor(node) {
  return {
    body: node.message ?? '',
    responses: Array.isArray(node.response)
      ? node.response.map(mapResponseToEditor)
      : [],
  };
}

/**
 * Convert a live comms array (TOML shape) to the editor's normalized tree.
 *
 * @param {Array<object>|undefined|null} commsArray
 * @returns {Array<object>} editor templates
 */
export function worldCommsToEditor(commsArray) {
  if (!Array.isArray(commsArray)) return [];
  return commsArray.map((tpl) => {
    const trigger = { kind: tpl.trigger ?? '' };
    if (tpl.entity !== undefined && tpl.entity !== null) {
      trigger.entity = tpl.entity;
    }
    return {
      from: tpl.from ?? '',
      trigger,
      node: {
        body: tpl.message ?? '',
        responses: Array.isArray(tpl.response)
          ? tpl.response.map(mapResponseToEditor)
          : [],
      },
    };
  });
}

function mapResponseToWorld(resp) {
  const out = { text: resp.text ?? '' };
  if (Array.isArray(resp.actions) && resp.actions.length > 0) {
    out.action = deepClone(resp.actions);
  }
  if (resp.follow_up && typeof resp.follow_up === 'object') {
    out.follow_up = mapNodeToWorld(resp.follow_up);
  }
  return out;
}

function mapNodeToWorld(node) {
  const out = { message: node.body ?? '' };
  if (Array.isArray(node.responses) && node.responses.length > 0) {
    out.response = node.responses.map(mapResponseToWorld);
  }
  return out;
}

/**
 * Convert editor templates back to the live TOML comms array.
 *
 * @param {Array<object>} editorTemplates
 * @returns {Array<object>} live comms array
 */
export function editorCommsToWorld(editorTemplates) {
  if (!Array.isArray(editorTemplates)) return [];
  return editorTemplates.map((tpl) => {
    const out = {
      from: tpl.from ?? '',
      trigger: tpl.trigger?.kind ?? '',
      message: tpl.node?.body ?? '',
    };
    if (tpl.trigger?.entity !== undefined && tpl.trigger.entity !== null) {
      out.entity = tpl.trigger.entity;
    }
    if (Array.isArray(tpl.node?.responses) && tpl.node.responses.length > 0) {
      out.response = tpl.node.responses.map(mapResponseToWorld);
    }
    return out;
  });
}
