import { getSpawnName, getSpawnPosition, getEntityPath, getRelativeInfo, setSpawnPosition, getAllAnchors, getSpawnRotation, setSpawnRotation, getSpawnScale, setSpawnScale } from './toml-utils.js';
import { getRelativeToCandidates, getSpawnReference, matchesSpawnReference } from './toml-utils.js';
import { renderOverridePanel } from './override-view.js';

export class PropertiesPanel {
  constructor(canvasManager, layerManager, undoController) {
    if (!undoController || typeof undoController.snapshotForUndo !== 'function') {
      throw new Error('PropertiesPanel: undoController with snapshotForUndo is required');
    }
    this.canvasManager = canvasManager;
    this.layerManager = layerManager;
    this.undoController = undoController;
    this.container = document.getElementById('propertiesPanelContent');
    this.currentSpawn = null;
    this.currentLayer = null;
  }

  /**
   * Render the properties pane for the given selection.
   *
   * Accepts either the new discriminated-union form (`render(selection)`)
   * or the legacy `(spawn, layer)` signature used by Slice 1-3 callers.
   */
  render(selectionOrSpawn, layer) {
    const selection = this._coerceSelection(selectionOrSpawn, layer);

    if (!selection || selection.type === null) {
      this.currentSpawn = null;
      this.currentLayer = null;
      this.container.innerHTML = '<p class="placeholder">Select a spawn to edit properties</p>';
      return;
    }

    // selection.type === 'spawn' — existing V1 form + override panel.
    // There is no 'trigger' / 'comms' selection to route: the World Content
    // tree stopped listing them with the declarative scenario front-end
    // (issue #985), and scenario logic is edited in the Script editor.
    this._renderSpawn(selection.spawn, selection.layer);
  }

  _coerceSelection(selectionOrSpawn, layer) {
    if (selectionOrSpawn && typeof selectionOrSpawn === 'object' && 'type' in selectionOrSpawn) {
      return selectionOrSpawn;
    }
    if (!selectionOrSpawn || !layer) {
      return { type: null };
    }
    return { type: 'spawn', spawn: selectionOrSpawn, layer };
  }

  _renderSpawn(spawn, layer) {
    this.currentSpawn = spawn;
    this.currentLayer = layer;

    const name = getSpawnName(spawn);
    const entityPath = getEntityPath(spawn);
    const allAnchors = getAllAnchors(this.layerManager.getLayers());
    const pos = getSpawnPosition(spawn, allAnchors);
    const relative = getRelativeInfo(spawn);
    const parentCandidates = getRelativeToCandidates(layer, spawn);
    const parentOptions = this._buildParentOptions(parentCandidates, relative);
    // A `relative` transform is an offset FROM something. With no eligible
    // parent in this layer and no authored reference worth keeping there is no
    // base to offer, and picking the mode anyway would write `relative_to =
    // null` — which smol-toml drops on the way out, leaving an `offset` with
    // nothing to offset from. `validate_relative_to` never sees a key to
    // complain about, so the entity just silently lands on the origin. Offer
    // the mode only where it can be satisfied.
    const canBeRelative = parentCandidates.length > 0 || Boolean(relative);

    let positionMode = 'absolute';
    if (relative) positionMode = 'relative';
    else if (spawn.transform && spawn.transform.anchor) positionMode = 'anchor';

    const shapeHtml = this._buildShapeHtml(spawn);
    const spawnToml = this._spawnToToml(spawn);
    const rot = getSpawnRotation(spawn);
    const scl = getSpawnScale(spawn);

    this.container.innerHTML = `
      <div class="property-group">
        <label>Name</label>
        <input type="text" id="propName" value="${name}">
      </div>

      <div class="property-group">
        <label>Entity</label>
        <input type="text" id="propEntity" value="${entityPath}" readonly>
        <button id="changeEntityBtn">Change…</button>
      </div>

      <div class="property-group">
        <label>Position Mode</label>
        <div class="radio-group">
          <label><input type="radio" name="posMode" value="absolute" ${positionMode === 'absolute' ? 'checked' : ''}> Absolute</label>
          <label><input type="radio" name="posMode" value="anchor" ${positionMode === 'anchor' ? 'checked' : ''}> Anchor</label>
          <label><input type="radio" name="posMode" value="relative" ${positionMode === 'relative' ? 'checked' : ''}${canBeRelative ? '' : ' disabled'}> Relative To</label>
        </div>
      </div>

      <div class="property-group${positionMode !== 'absolute' ? ' hidden' : ''}" id="absolutePos">
        <div class="input-row">
          <div>
            <label>X</label>
            <input type="number" id="propX" step="0.1" value="${pos.x.toFixed(2)}">
          </div>
          <div>
            <label>Z</label>
            <input type="number" id="propZ" step="0.1" value="${pos.z.toFixed(2)}">
          </div>
        </div>
      </div>

      <div class="property-group${positionMode !== 'anchor' ? ' hidden' : ''}" id="anchorPos">
        <label>Anchor</label>
        <select id="propAnchor">
          ${allAnchors.map(a => `<option value="${a.name}" ${spawn.transform?.anchor === a.name ? 'selected' : ''}>${a.name}</option>`).join('')}
        </select>
      </div>

      <div class="property-group${positionMode !== 'relative' ? ' hidden' : ''}" id="relativePos">
        <label>Parent</label>
        <select id="propParent">
          ${parentOptions}
        </select>
        <div class="input-row" style="margin-top: 8px;">
          <div>
            <label>Offset X</label>
            <input type="number" id="propOffsetX" step="0.1" value="${relative?.offset.x.toFixed(2) ?? '0'}">
          </div>
          <div>
            <label>Offset Z</label>
            <input type="number" id="propOffsetZ" step="0.1" value="${relative?.offset.z.toFixed(2) ?? '0'}">
          </div>
        </div>
      </div>

      ${shapeHtml}

      <div class="property-group">
        <label>Rotation (radians, XYZ)</label>
        <div class="input-row">
          <div>
            <label>X</label>
            <input type="number" id="propRotX" step="0.01" value="${rot[0]}">
          </div>
          <div>
            <label>Y</label>
            <input type="number" id="propRotY" step="0.01" value="${rot[1]}">
          </div>
          <div>
            <label>Z</label>
            <input type="number" id="propRotZ" step="0.01" value="${rot[2]}">
          </div>
        </div>
      </div>

      <div class="property-group">
        <label>Scale (XYZ)</label>
        <div class="input-row">
          <div>
            <label>X</label>
            <input type="number" id="propScaleX" step="0.1" value="${scl[0]}">
          </div>
          <div>
            <label>Y</label>
            <input type="number" id="propScaleY" step="0.1" value="${scl[1]}">
          </div>
          <div>
            <label>Z</label>
            <input type="number" id="propScaleZ" step="0.1" value="${scl[2]}">
          </div>
        </div>
      </div>

      <button id="deleteSpawnBtn" style="margin-top: 8px;">Delete Spawn</button>

      <details class="spawn-toml-details">
        <summary>World Entry (TOML)</summary>
        <textarea id="spawnToml" rows="10" spellcheck="false">${spawnToml}</textarea>
        <div class="spawn-toml-actions">
          <button id="applySpawnToml">Apply</button>
          <span id="spawnTomlError" class="toml-error"></span>
        </div>
      </details>

      <div id="overridePanelHost"></div>
    `;

    this._attachListeners(spawn, layer, allAnchors, pos, relative);

    // Slice 3: resolved-template + override summary card below the V1 form.
    renderOverridePanel(
      document.getElementById('overridePanelHost'),
      spawn,
      layer,
      { canvasManager: this.canvasManager, undoController: this.undoController },
    );
  }

  /**
   * `<option>` markup for the Relative To parent picker.
   *
   * Only spawns a `relative_to` can actually resolve against are offered — see
   * `getRelativeToCandidates`. Before issue #969 the picker listed every spawn
   * in every open layer under `getSpawnName`, so three of its entries could
   * never resolve: the literal `'unnamed'`, anything in another layer, and any
   * spawn already positioned by `relative_to`. Each of those used to cost one
   * misplaced entity; now an unresolvable `relative_to` blocks the whole world,
   * so the picker must not be able to author one.
   *
   * The `value` a NEW selection writes is `getSpawnReference` — one identifier,
   * `name` first; the label stays `getSpawnName`, which is what the tree and
   * canvas show. Recognising an EXISTING reference is the other direction and
   * goes through `matchesSpawnReference`, which accepts `id` or `name` exactly
   * as the runtime table does. Comparing against `getSpawnReference` instead
   * marked every shipped landmark unresolved: they all carry both identifiers
   * and are all referenced by the `id` that `name` beats.
   *
   * A `relative_to` already on the spawn that no candidate matches is preserved
   * as its own option rather than dropped — silently re-pointing an authored
   * reference at whatever happened to sort first is the failure mode this whole
   * issue is about.
   */
  _buildParentOptions(candidates, relative) {
    const authored = relative?.parent;
    const options = candidates.map(s => {
      const reference = getSpawnReference(s);
      const selected = matchesSpawnReference(s, authored) ? ' selected' : '';
      return `<option value="${reference}"${selected}>${getSpawnName(s)}</option>`;
    });

    if (authored && !candidates.some(s => matchesSpawnReference(s, authored))) {
      options.unshift(
        `<option value="${authored}" selected>${authored} (unresolved)</option>`
      );
    }
    if (options.length === 0) {
      return '<option value="" disabled selected>no eligible parent in this layer</option>';
    }
    return options.join('');
  }

  _buildShapeHtml(spawn) {
    if (!spawn.shape) return '';
    const { type } = spawn.shape;
    if (type === 'sphere') {
      return `
        <div class="property-group">
          <label>Radius</label>
          <input type="number" id="shapeRadius" step="1" value="${spawn.shape.radius ?? 100}">
        </div>`;
    }
    if (type === 'torus') {
      return `
        <div class="property-group">
          <label>Belt Radii</label>
          <div class="input-row">
            <div>
              <label>Inner</label>
              <input type="number" id="shapeInnerRadius" step="1" value="${spawn.shape.inner_radius ?? 50}">
            </div>
            <div>
              <label>Outer</label>
              <input type="number" id="shapeOuterRadius" step="1" value="${spawn.shape.outer_radius ?? 150}">
            </div>
          </div>
        </div>`;
    }
    return '';
  }

  _spawnToToml(spawn) {
    const clean = {};
    for (const [k, v] of Object.entries(spawn)) {
      if (!k.startsWith('_')) clean[k] = v;
    }
    try {
      return window.tomlStringify(clean);
    } catch {
      return JSON.stringify(clean, null, 2);
    }
  }

  _attachListeners(spawn, layer, allAnchors, pos, relative) {
    document.getElementById('propName').addEventListener('input', (e) => {
      this.undoController.snapshotForUndo(layer);
      spawn.name = e.target.value;
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('changeEntityBtn').addEventListener('click', () => {
      console.log('Change entity clicked');
    });

    document.querySelectorAll('input[name="posMode"]').forEach(radio => {
      radio.addEventListener('change', (e) => {
        const mode = e.target.value;
        const parentSelect = document.getElementById('propParent');
        // Belt and braces with the disabled radio: never author a `relative`
        // transform with no base. When the layer offers no eligible parent the
        // picker's only entry is the empty sentinel, and `relative_to = null`
        // serialises to an `offset` alone — no base, nothing for
        // `validate_relative_to` to reject, entity silently at the origin.
        // Bail before the undo snapshot so a refused switch costs nothing.
        if (mode === 'relative' && !parentSelect?.value) return;
        document.getElementById('absolutePos').classList.toggle('hidden', mode !== 'absolute');
        document.getElementById('anchorPos').classList.toggle('hidden', mode !== 'anchor');
        document.getElementById('relativePos').classList.toggle('hidden', mode !== 'relative');

        this.undoController.snapshotForUndo(layer);
        if (mode === 'absolute') {
          const x = parseFloat(document.getElementById('propX')?.value || '0');
          const z = parseFloat(document.getElementById('propZ')?.value || '0');
          setSpawnPosition(spawn, x, z, 'absolute');
        } else if (mode === 'anchor') {
          const anchorSelect = document.getElementById('propAnchor');
          setSpawnPosition(spawn, 0, 0, 'anchor', anchorSelect?.value || null, null);
        } else if (mode === 'relative') {
          const offsetX = parseFloat(document.getElementById('propOffsetX')?.value || '0');
          const offsetZ = parseFloat(document.getElementById('propOffsetZ')?.value || '0');
          const currentPos = getSpawnPosition(spawn, allAnchors);
          // Non-empty: the guard above returned early otherwise.
          setSpawnPosition(spawn, currentPos.x, currentPos.z, 'relative', parentSelect.value, { x: offsetX, z: offsetZ });
        }
        layer.isDirty = true;
        this.canvasManager.renderAll();
      });
    });

    document.getElementById('propX')?.addEventListener('input', (e) => {
      const x = parseFloat(e.target.value) || 0;
      const z = parseFloat(document.getElementById('propZ')?.value || '0');
      this.undoController.snapshotForUndo(layer);
      setSpawnPosition(spawn, x, z, 'absolute');
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('propZ')?.addEventListener('input', (e) => {
      const x = parseFloat(document.getElementById('propX')?.value || '0');
      const z = parseFloat(e.target.value) || 0;
      this.undoController.snapshotForUndo(layer);
      setSpawnPosition(spawn, x, z, 'absolute');
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('propAnchor')?.addEventListener('change', (e) => {
      this.undoController.snapshotForUndo(layer);
      setSpawnPosition(spawn, 0, 0, 'anchor', e.target.value, null);
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('propParent')?.addEventListener('change', () => {
      const parent = document.getElementById('propParent').value;
      const offsetX = parseFloat(document.getElementById('propOffsetX')?.value || '0');
      const offsetZ = parseFloat(document.getElementById('propOffsetZ')?.value || '0');
      this.undoController.snapshotForUndo(layer);
      setSpawnPosition(spawn, pos.x, pos.z, 'relative', parent, { x: offsetX, z: offsetZ });
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('propOffsetX')?.addEventListener('input', (e) => {
      const cur = getRelativeInfo(spawn);
      const offsetX = parseFloat(e.target.value) || 0;
      const offsetZ = parseFloat(document.getElementById('propOffsetZ')?.value || '0');
      this.undoController.snapshotForUndo(layer);
      setSpawnPosition(spawn, 0, 0, 'relative', cur?.parent, { x: offsetX, z: offsetZ });
      layer.isDirty = true;
      this.canvasManager.updateArrows();
    });

    document.getElementById('propOffsetZ')?.addEventListener('input', (e) => {
      const cur = getRelativeInfo(spawn);
      const offsetX = parseFloat(document.getElementById('propOffsetX')?.value || '0');
      const offsetZ = parseFloat(e.target.value) || 0;
      this.undoController.snapshotForUndo(layer);
      setSpawnPosition(spawn, 0, 0, 'relative', cur?.parent, { x: offsetX, z: offsetZ });
      layer.isDirty = true;
      this.canvasManager.updateArrows();
    });

    // Shape size fields
    document.getElementById('shapeRadius')?.addEventListener('input', (e) => {
      this.undoController.snapshotForUndo(layer);
      if (!spawn.shape) spawn.shape = { type: 'sphere' };
      spawn.shape.radius = parseFloat(e.target.value) || 100;
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('shapeInnerRadius')?.addEventListener('input', (e) => {
      this.undoController.snapshotForUndo(layer);
      if (!spawn.shape) spawn.shape = { type: 'torus' };
      spawn.shape.inner_radius = parseFloat(e.target.value) || 50;
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('shapeOuterRadius')?.addEventListener('input', (e) => {
      this.undoController.snapshotForUndo(layer);
      if (!spawn.shape) spawn.shape = { type: 'torus' };
      spawn.shape.outer_radius = parseFloat(e.target.value) || 150;
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    const readRot = () => [
      parseFloat(document.getElementById('propRotX')?.value || '0') || 0,
      parseFloat(document.getElementById('propRotY')?.value || '0') || 0,
      parseFloat(document.getElementById('propRotZ')?.value || '0') || 0,
    ];
    const readScale = () => [
      parseFloat(document.getElementById('propScaleX')?.value || '1') || 0,
      parseFloat(document.getElementById('propScaleY')?.value || '1') || 0,
      parseFloat(document.getElementById('propScaleZ')?.value || '1') || 0,
    ];
    for (const id of ['propRotX', 'propRotY', 'propRotZ']) {
      document.getElementById(id)?.addEventListener('input', () => {
        this.undoController.snapshotForUndo(layer);
        setSpawnRotation(spawn, readRot());
        layer.isDirty = true;
        this.canvasManager.renderAll();
      });
    }
    for (const id of ['propScaleX', 'propScaleY', 'propScaleZ']) {
      document.getElementById(id)?.addEventListener('input', () => {
        this.undoController.snapshotForUndo(layer);
        setSpawnScale(spawn, readScale());
        layer.isDirty = true;
        this.canvasManager.renderAll();
      });
    }

    document.getElementById('deleteSpawnBtn').addEventListener('click', () => {
      const idx = layer.toml.entity?.indexOf(spawn);
      if (idx !== -1) {
        this.undoController.snapshotForUndo(layer);
        layer.toml.entity.splice(idx, 1);
        layer.isDirty = true;
        this.canvasManager.spawnGroups.delete(spawn);
        this.canvasManager.deselectSpawn();
        this.canvasManager.renderAll();
      }
    });

    document.getElementById('applySpawnToml').addEventListener('click', () => {
      const errorEl = document.getElementById('spawnTomlError');
      try {
        const parsed = window.tomlParse(document.getElementById('spawnToml').value);
        this.undoController.snapshotForUndo(layer);
        // Update spawn in-place
        for (const key of Object.keys(spawn)) {
          if (!key.startsWith('_')) delete spawn[key];
        }
        Object.assign(spawn, parsed);
        layer.isDirty = true;
        errorEl.textContent = '';
        this.canvasManager.renderAll();
        this.render(spawn, layer);
      } catch (err) {
        errorEl.textContent = 'Parse error: ' + err.message;
      }
    });
  }

  updatePositionFields(layerManager) {
    if (!this.currentSpawn || !this.currentLayer) return;

    const allAnchors = layerManager ? getAllAnchors(layerManager.getLayers()) : [];
    const pos = getSpawnPosition(this.currentSpawn, allAnchors);
    const relative = getRelativeInfo(this.currentSpawn);

    const xInput = document.getElementById('propX');
    const zInput = document.getElementById('propZ');
    const offsetXInput = document.getElementById('propOffsetX');
    const offsetZInput = document.getElementById('propOffsetZ');

    if (xInput) xInput.value = pos.x.toFixed(2);
    if (zInput) zInput.value = pos.z.toFixed(2);
    if (offsetXInput) offsetXInput.value = relative?.offset.x.toFixed(2) ?? '0';
    if (offsetZInput) offsetZInput.value = relative?.offset.z.toFixed(2) ?? '0';
  }
}
