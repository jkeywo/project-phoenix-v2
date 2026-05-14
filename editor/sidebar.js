import { getSpawnName, getSpawnPosition, getEntityPath, getRelativeInfo, setSpawnPosition, getAnchors, getAllAnchors } from './toml-utils.js';
import { getSpawnsFromAllLayers } from './toml-utils.js';

export class PropertiesPanel {
  constructor(canvasManager, layerManager) {
    this.canvasManager = canvasManager;
    this.layerManager = layerManager;
    this.container = document.getElementById('propertiesPanel');
    this.currentSpawn = null;
    this.currentLayer = null;
  }

  render(spawn, layer) {
    this.currentSpawn = spawn;
    this.currentLayer = layer;

    if (!spawn || !layer) {
      this.container.innerHTML = '<p class="placeholder">Select a spawn to edit properties</p>';
      return;
    }

    const name = getSpawnName(spawn);
    const entityPath = getEntityPath(spawn);
    const allAnchors = getAllAnchors(this.layerManager.getLayers());
    const pos = getSpawnPosition(spawn, allAnchors);
    const relative = getRelativeInfo(spawn);
    const layerSpawns = getSpawnsFromAllLayers(this.layerManager.getLayers());

    let positionMode = 'absolute';
    if (relative) {
      positionMode = 'relative';
    } else if (spawn.anchor) {
      positionMode = 'anchor';
    }

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
          <label><input type="radio" name="posMode" value="relative" ${positionMode === 'relative' ? 'checked' : ''}> Relative To</label>
        </div>
      </div>

      <div class="property-group" id="absolutePos">
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

      <div class="property-group hidden" id="anchorPos">
        <label>Anchor</label>
        <select id="propAnchor">
          ${allAnchors.map(a => `<option value="${a.name}" ${spawn.anchor === a.name ? 'selected' : ''}>${a.name}</option>`).join('')}
        </select>
      </div>

      <div class="property-group hidden" id="relativePos">
        <label>Parent</label>
        <select id="propParent">
          ${layerSpawns.map(s => `<option value="${getSpawnName(s)}" ${relative?.parent === getSpawnName(s) ? 'selected' : ''}>${getSpawnName(s)}</option>`).join('')}
        </select>
        <div class="input-row" style="margin-top: 8px;">
          <div>
            <label>Offset X</label>
            <input type="number" id="propOffsetX" step="0.1" value="${relative?.offset.x.toFixed(2) || '0'}">
          </div>
          <div>
            <label>Offset Z</label>
            <input type="number" id="propOffsetZ" step="0.1" value="${relative?.offset.z.toFixed(2) || '0'}">
          </div>
        </div>
      </div>

      <button id="deleteSpawnBtn">Delete Spawn</button>
    `;

    document.getElementById('propName').addEventListener('input', (e) => {
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
        document.getElementById('absolutePos').classList.toggle('hidden', mode !== 'absolute');
        document.getElementById('anchorPos').classList.toggle('hidden', mode !== 'anchor');
        document.getElementById('relativePos').classList.toggle('hidden', mode !== 'relative');

        if (mode === 'absolute') {
          const x = parseFloat(document.getElementById('propX')?.value || '0');
          const z = parseFloat(document.getElementById('propZ')?.value || '0');
          setSpawnPosition(spawn, x, z, 'absolute');
        } else if (mode === 'anchor') {
          const anchorSelect = document.getElementById('propAnchor');
          setSpawnPosition(spawn, 0, 0, 'anchor', anchorSelect?.value || null, null);
        } else if (mode === 'relative') {
          const parentSelect = document.getElementById('propParent');
          const offsetX = parseFloat(document.getElementById('propOffsetX')?.value || '0');
          const offsetZ = parseFloat(document.getElementById('propOffsetZ')?.value || '0');
          const currentPos = getSpawnPosition(spawn, allAnchors);
          setSpawnPosition(spawn, currentPos.x, currentPos.z, 'relative', parentSelect?.value || null, { x: offsetX, z: offsetZ });
        }
        layer.isDirty = true;
        this.canvasManager.renderAll();
      });
    });

    document.getElementById('propX').addEventListener('input', (e) => {
      const x = parseFloat(e.target.value) || 0;
      const z = parseFloat(document.getElementById('propZ')?.value || '0');
      setSpawnPosition(spawn, x, z, 'absolute');
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    document.getElementById('propZ').addEventListener('input', (e) => {
      const x = parseFloat(document.getElementById('propX')?.value || '0');
      const z = parseFloat(e.target.value) || 0;
      setSpawnPosition(spawn, x, z, 'absolute');
      layer.isDirty = true;
      this.canvasManager.renderAll();
    });

    if (document.getElementById('propAnchor')) {
      document.getElementById('propAnchor').addEventListener('change', (e) => {
        setSpawnPosition(spawn, 0, 0, 'anchor', e.target.value, null);
        layer.isDirty = true;
        this.canvasManager.renderAll();
      });
    }

    if (document.getElementById('propParent')) {
      document.getElementById('propParent').addEventListener('change', () => {
        const parent = document.getElementById('propParent').value;
        const offsetX = parseFloat(document.getElementById('propOffsetX')?.value || '0');
        const offsetZ = parseFloat(document.getElementById('propOffsetZ')?.value || '0');
        setSpawnPosition(spawn, pos.x, pos.z, 'relative', parent, { x: offsetX, z: offsetZ });
        layer.isDirty = true;
        this.canvasManager.renderAll();
      });
    }

    if (document.getElementById('propOffsetX')) {
      document.getElementById('propOffsetX').addEventListener('input', (e) => {
        const currentRelative = getRelativeInfo(spawn);
        const offsetX = parseFloat(e.target.value) || 0;
        const offsetZ = parseFloat(document.getElementById('propOffsetZ')?.value || '0');
        setSpawnPosition(spawn, 0, 0, 'relative', currentRelative?.parent, { x: offsetX, z: offsetZ });
        layer.isDirty = true;
        this.canvasManager.updateArrows();
      });
    }

    if (document.getElementById('propOffsetZ')) {
      document.getElementById('propOffsetZ').addEventListener('input', (e) => {
        const currentRelative = getRelativeInfo(spawn);
        const offsetX = parseFloat(document.getElementById('propOffsetX')?.value || '0');
        const offsetZ = parseFloat(e.target.value) || 0;
        setSpawnPosition(spawn, 0, 0, 'relative', currentRelative?.parent, { x: offsetX, z: offsetZ });
        layer.isDirty = true;
        this.canvasManager.updateArrows();
      });
    }

    document.getElementById('deleteSpawnBtn').addEventListener('click', () => {
      const arr = layer.kind === 'scenario' ? 'spawn' : 'entity';
      const idx = layer.toml[arr]?.indexOf(spawn);
      if (idx !== -1) {
        layer.toml[arr].splice(idx, 1);
        layer.isDirty = true;
        this.canvasManager.spawnGroups.delete(spawn);
        this.canvasManager.deselectSpawn();
        this.canvasManager.renderAll();
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
    if (offsetXInput) offsetXInput.value = relative?.offset.x.toFixed(2) || '0';
    if (offsetZInput) offsetZInput.value = relative?.offset.z.toFixed(2) || '0';
  }
}