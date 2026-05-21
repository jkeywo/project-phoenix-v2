/**
 * comms-view.js
 *
 * Renders the comms-template editor inside the right-hand properties pane.
 *
 * Structure (mirrors `trigger-view.js`):
 *   - Header:
 *       * `from` text input
 *       * `trigger` <select> (on_attacked | on_destroyed | on_hailed —
 *         on_timer is intentionally absent because RawCommsEntry in
 *         `src/world/config.rs` does not model after_secs for comms; see
 *         the pre-flight finding documented in Slice 4b).
 *       * Entity picker (via shared `renderEntitySelect`)
 *   - Body: <textarea> for the node body
 *   - Responses list: each `.response-card` with text input + action-card
 *     list (`renderActionCard`) + "Add Action" composer + "Add Follow-Up
 *     Node" button. If `follow_up` exists, render an inner mini-editor
 *     (recursive walk via `nodePath`).
 *
 * All mutations honour the snapshot-BEFORE-mutate contract via
 * `snapshotForUndo(layer)`. After each mutation the editor calls
 * `editorCommsToWorld` to write back into `layer.toml.comms`, marks
 * `layer.isDirty = true`, calls `canvasManager.renderAll()`, and
 * re-renders the panel.
 */

import { ACTION_SCHEMA } from './action-schema.js';
import { renderActionCard } from './action-card-view.js';
import { snapshotForUndo } from './undo-controller.js';
import { renderEntitySelect } from './entity-select-view.js';
import { worldCommsToEditor, editorCommsToWorld } from './comms-adapter.js';
import { buildDefaultAction } from './trigger-view.js';
import { CommsEditor } from './comms-editor.js';
import { validateFile } from './validation.js';
import { applyValidationResults } from './validation-badge.js';

// `on_timer` is omitted here — comms templates cannot use it (see
// pre-flight finding in the Slice 4b commit log).
const COMMS_TRIGGER_KINDS = ['on_attacked', 'on_destroyed', 'on_hailed'];

/**
 * Render the comms template editor.
 *
 * @param {HTMLElement} host
 * @param {object} selection  { type:'comms', commsIndex, layer }
 * @param {object} deps
 * @param {Array<{path,worldState}>} deps.allLayers
 * @param {object} deps.canvasManager
 * @param {object} [deps.layerManager]
 */
export function renderCommsPanel(host, selection, deps) {
  const layer = selection.layer;
  const idx = selection.commsIndex;

  if (!layer || !layer.toml) {
    host.innerHTML = '<p class="placeholder">Comms template no longer exists</p>';
    return;
  }
  if (!Array.isArray(layer.toml.comms)) {
    host.innerHTML = '<p class="placeholder">No comms templates in this layer</p>';
    return;
  }
  const tomlTemplate = layer.toml.comms[idx];
  if (!tomlTemplate) {
    host.innerHTML = '<p class="placeholder">Comms template no longer exists</p>';
    return;
  }

  // Load the editor model from the live TOML each render so we always see
  // the freshest data after a re-render or undo.
  const editor = new CommsEditor();
  editor.load(worldCommsToEditor(layer.toml.comms));

  const rerender = () => renderCommsPanel(host, selection, deps);

  // Apply current editor templates back to layer.toml.comms.
  const writeback = () => {
    layer.toml.comms = editorCommsToWorld(editor.getTemplates());
    layer.isDirty = true;
    if (typeof deps.canvasManager?.renderAll === 'function') {
      deps.canvasManager.renderAll();
    }
  };

  // Convenience: snapshot → editor mutator → writeback → rerender.
  const mutate = (fn) => {
    snapshotForUndo(layer);
    fn();
    writeback();
    rerender();
  };

  host.innerHTML = '';

  const panel = document.createElement('div');
  panel.className = 'comms-panel';

  // ── Header ────────────────────────────────────────────────────────────
  const h4 = document.createElement('h4');
  h4.textContent = 'Comms Template';
  panel.appendChild(h4);

  const headerWrap = document.createElement('div');
  headerWrap.className = 'comms-trigger-header';
  panel.appendChild(headerWrap);

  // From.
  headerWrap.appendChild(makeFromRow(editor, idx, mutate));

  // Trigger kind.
  headerWrap.appendChild(makeTriggerKindRow(editor, idx, mutate));

  // Entity (always shown for the three supported triggers).
  headerWrap.appendChild(makeEntityRow(editor, idx, deps, mutate));

  // ── Body ──────────────────────────────────────────────────────────────
  const bodyH4 = document.createElement('h4');
  bodyH4.textContent = 'Prompt';
  panel.appendChild(bodyH4);

  panel.appendChild(makeBodyRow(editor, idx, [], mutate));

  // ── Responses (recursive walk via nodePath) ───────────────────────────
  const responsesH4 = document.createElement('h4');
  responsesH4.textContent = 'Responses';
  panel.appendChild(responsesH4);

  panel.appendChild(renderResponsesForNode(editor, idx, [], deps, mutate));

  // Slice 7: decorate fields whose validation path has a record.
  try {
    const results = validateFile(layer.filename, layer.toml);
    applyValidationResults(panel, results);
  } catch (err) {
    console.warn('[comms-view] validation badge pass failed:', err?.message || err);
  }

  host.appendChild(panel);
}

// ── Header rows ─────────────────────────────────────────────────────────

function makeFromRow(editor, idx, mutate) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const label = document.createElement('label');
  label.textContent = 'From:';
  group.appendChild(label);

  const input = document.createElement('input');
  input.type = 'text';
  input.id = 'commsFrom';
  const tpl = editor.getTemplates()[idx];
  input.value = tpl?.from ?? '';
  input.addEventListener('input', (e) => {
    mutate(() => editor.setTemplateField(idx, 'from', e.target.value));
  });
  group.appendChild(input);
  return group;
}

function makeTriggerKindRow(editor, idx, mutate) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const label = document.createElement('label');
  label.textContent = 'Trigger:';
  group.appendChild(label);

  const select = document.createElement('select');
  select.id = 'commsTriggerKind';
  for (const k of COMMS_TRIGGER_KINDS) {
    const opt = document.createElement('option');
    opt.value = k;
    opt.textContent = k;
    select.appendChild(opt);
  }

  const tpl = editor.getTemplates()[idx];
  const current = tpl?.trigger?.kind ?? '';
  if (current && !COMMS_TRIGGER_KINDS.includes(current)) {
    const opt = document.createElement('option');
    opt.value = current;
    opt.textContent = `${current} (unknown)`;
    select.appendChild(opt);
  }
  select.value = current || COMMS_TRIGGER_KINDS[0];
  select.addEventListener('change', (e) => {
    mutate(() => editor.setTemplateField(idx, 'trigger.kind', e.target.value));
  });
  group.appendChild(select);
  return group;
}

function makeEntityRow(editor, idx, deps, mutate) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const label = document.createElement('label');
  label.textContent = 'Entity:';
  group.appendChild(label);

  const tpl = editor.getTemplates()[idx];
  const value = tpl?.trigger?.entity ?? '';

  renderEntitySelect(
    group,
    value,
    deps.allLayers || [],
    (newValue) => {
      mutate(() => editor.setTemplateField(idx, 'trigger.entity', newValue || ''));
    },
  );
  return group;
}

// ── Body row ────────────────────────────────────────────────────────────

function makeBodyRow(editor, templateIdx, nodePath, mutate) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const ta = document.createElement('textarea');
  ta.className = 'comms-body-input';
  ta.rows = 3;
  const node = editor.getNode(templateIdx, nodePath);
  ta.value = node?.body ?? '';
  ta.addEventListener('input', (e) => {
    mutate(() => editor.setNodeBody(templateIdx, nodePath, e.target.value));
  });
  group.appendChild(ta);
  return group;
}

// ── Responses (recursive) ───────────────────────────────────────────────

function renderResponsesForNode(editor, templateIdx, nodePath, deps, mutate) {
  const wrap = document.createElement('div');
  wrap.className = 'response-list';

  const node = editor.getNode(templateIdx, nodePath) || { responses: [] };
  const responses = node.responses || [];

  responses.forEach((resp, respIdx) => {
    wrap.appendChild(
      renderResponseCard(editor, templateIdx, nodePath, respIdx, resp, deps, mutate),
    );
  });

  // + Add Response button.
  const addBtn = document.createElement('button');
  addBtn.type = 'button';
  addBtn.className = 'btn-add-response';
  addBtn.textContent = '+ Add Response';
  addBtn.addEventListener('click', () => {
    mutate(() => editor.addResponse(templateIdx, nodePath));
  });
  wrap.appendChild(addBtn);

  return wrap;
}

function renderResponseCard(editor, templateIdx, nodePath, respIdx, resp, deps, mutate) {
  const card = document.createElement('div');
  card.className = 'response-card';

  // ── Header: text + remove ─────────────────────────────────────────────
  const header = document.createElement('div');
  header.className = 'response-card-header';

  const textInput = document.createElement('input');
  textInput.type = 'text';
  textInput.className = 'response-text-input';
  textInput.placeholder = 'Player response text…';
  textInput.value = resp.text ?? '';
  textInput.addEventListener('input', (e) => {
    mutate(() => editor.setResponseText(templateIdx, nodePath, respIdx, e.target.value));
  });
  header.appendChild(textInput);

  const removeBtn = document.createElement('button');
  removeBtn.type = 'button';
  removeBtn.className = 'btn-icon';
  removeBtn.dataset.action = 'remove';
  removeBtn.textContent = '✕';
  removeBtn.addEventListener('click', () => {
    mutate(() => editor.removeResponse(templateIdx, nodePath, respIdx));
  });
  header.appendChild(removeBtn);

  card.appendChild(header);

  // ── Actions list ──────────────────────────────────────────────────────
  const actionsLabel = document.createElement('div');
  actionsLabel.className = 'response-section-label';
  actionsLabel.textContent = 'Actions';
  card.appendChild(actionsLabel);

  const actionsHost = document.createElement('div');
  actionsHost.className = 'response-actions';
  card.appendChild(actionsHost);

  const actions = Array.isArray(resp.actions) ? resp.actions : [];
  const layer = nearestLayer(deps);
  // Slice 7: only the top-level response array maps 1:1 to the
  // indexed validator paths (`comms[i].response[r].action[j]`).
  // Follow-up sub-trees use a different schema (see comms-adapter.js)
  // and are not yet covered by validateWorldReferencesIndexed; leave
  // basePath undefined so no badges are mis-attached there.
  const isTopLevel = Array.isArray(nodePath) && nodePath.length === 0;
  const actionBasePath = isTopLevel
    ? `comms[${templateIdx}].response[${respIdx}].action`
    : undefined;
  actions.forEach((action, actionIdx) => {
    renderActionCard(actionsHost, action, {
      worldState: layer?.toml || {},
      allLayers: deps.allLayers || [],
      basePath: actionBasePath,
      actionIndex: actionBasePath ? actionIdx : undefined,
      onChange: (updated) => {
        mutate(() => {
          editor.removeResponseAction(templateIdx, nodePath, respIdx, actionIdx);
          // re-insert at same position to preserve order.
          const node = editor._resolveNode(templateIdx, nodePath);
          if (node?.responses?.[respIdx]) {
            node.responses[respIdx].actions.splice(actionIdx, 0, { ...updated });
          }
        });
      },
      onMove: (direction) => {
        mutate(() => {
          const node = editor._resolveNode(templateIdx, nodePath);
          if (!node?.responses?.[respIdx]?.actions) return;
          const arr = node.responses[respIdx].actions;
          const j = direction === 'up' ? actionIdx - 1 : actionIdx + 1;
          if (j < 0 || j >= arr.length) return;
          [arr[actionIdx], arr[j]] = [arr[j], arr[actionIdx]];
        });
      },
      onRemove: () => {
        mutate(() => editor.removeResponseAction(templateIdx, nodePath, respIdx, actionIdx));
      },
      openFilePicker: deps.openFilePicker,
    });
  });

  // ── + Add Action composer ─────────────────────────────────────────────
  const addActionRow = document.createElement('div');
  addActionRow.className = 'action-add-row';

  const select = document.createElement('select');
  select.className = 'newCommsActionType';
  for (const key of Object.keys(ACTION_SCHEMA)) {
    const opt = document.createElement('option');
    opt.value = key;
    opt.textContent = ACTION_SCHEMA[key].label;
    select.appendChild(opt);
  }
  addActionRow.appendChild(select);

  const addBtn = document.createElement('button');
  addBtn.type = 'button';
  addBtn.className = 'btn-add-comms-action';
  addBtn.textContent = '+ Add Action';
  addBtn.addEventListener('click', () => {
    const type = select.value;
    const action = buildDefaultAction(type);
    if (!action) return;
    mutate(() => editor.addResponseAction(templateIdx, nodePath, respIdx, action));
  });
  addActionRow.appendChild(addBtn);
  card.appendChild(addActionRow);

  // ── Follow-up ─────────────────────────────────────────────────────────
  if (!resp.follow_up) {
    const addFollowBtn = document.createElement('button');
    addFollowBtn.type = 'button';
    addFollowBtn.className = 'btn-add-follow-up';
    addFollowBtn.textContent = '+ Add Follow-Up Node';
    addFollowBtn.addEventListener('click', () => {
      mutate(() => editor.addFollowUp(templateIdx, nodePath, respIdx));
    });
    card.appendChild(addFollowBtn);
  } else {
    const followWrap = document.createElement('div');
    followWrap.className = 'follow-up-node';

    const followHeader = document.createElement('div');
    followHeader.className = 'follow-up-header';
    const followLabel = document.createElement('span');
    followLabel.textContent = '↳ Follow-Up Node';
    followHeader.appendChild(followLabel);
    const removeFollowBtn = document.createElement('button');
    removeFollowBtn.type = 'button';
    removeFollowBtn.className = 'btn-icon';
    removeFollowBtn.textContent = '✕';
    removeFollowBtn.addEventListener('click', () => {
      mutate(() => editor.removeFollowUp(templateIdx, nodePath, respIdx));
    });
    followHeader.appendChild(removeFollowBtn);
    followWrap.appendChild(followHeader);

    const childPath = [...nodePath, respIdx];

    followWrap.appendChild(makeBodyRow(editor, templateIdx, childPath, mutate));

    const innerResponsesLabel = document.createElement('div');
    innerResponsesLabel.className = 'response-section-label';
    innerResponsesLabel.textContent = 'Responses';
    followWrap.appendChild(innerResponsesLabel);

    followWrap.appendChild(
      renderResponsesForNode(editor, templateIdx, childPath, deps, mutate),
    );

    card.appendChild(followWrap);
  }

  return card;
}

// ── Helpers ─────────────────────────────────────────────────────────────

function nearestLayer(deps) {
  // The trigger panel uses the active layer's `toml` as the `worldState`
  // for picker context. Comms editor: use the selection's layer (passed
  // through deps when sidebar.js forwards layerManager).
  if (deps.layerManager?.getActiveLayer) {
    return deps.layerManager.getActiveLayer();
  }
  return null;
}
