import { getSpawns, getAnchors, getSpawnPosition, getSpawnName, getEntityPath, getRelativeInfo, setSpawnPosition, matchesSpawnReference } from './toml-utils.js';
import { getColorForEntity } from './layers.js';
import { loadEntityConfig, getEntityConfig } from './entity-cache.js';
import { resolveEntityAppearance, drawEntityShape, colourToHex, RADAR_SHAPE_FALLBACK } from './canvas-world.js';
import { getRegionRenderSpec } from './canvas-region.js';
import { getAnchorRenderSpecs } from './canvas-anchor.js';
import { analyzeAnchorRename } from './anchor-rename.js';
import { canDeleteAnchor } from './anchor-delete.js';

/**
 * Build a THROWAWAY render-only view of `spawn` with its template/fragment
 * appearance fields merged in (issue #993).
 *
 * The persisted spawn is the layer's saved data model: a save serialises it
 * straight to the world/scenario TOML, and the override panel writes
 * `spawn.override` into the same object. So template- and fragment-derived
 * fields must NOT be baked onto it — doing so freezes a stale copy of the
 * template into the file, and editing the template later leaves every
 * already-saved spawn stale, defeating `template_path` as the single source of
 * truth. This function instead returns a fresh object the renderer reads and
 * then discards; `spawn` itself is never mutated.
 *
 * `entConfig` is the RESOLVED document from `getEntityConfig` (issue #910), so
 * the merge already carries fragment-inherited fields. Template fields are
 * shared by reference — safe, because nothing mutates them on the throwaway
 * view. The field set and the `!field` "only when the spawn lacks it" guards
 * match the historic in-place merge exactly, so existing worlds render
 * identically; only the persistence side effect is removed.
 *
 * @param {object} spawn          The persisted spawn (never mutated).
 * @param {object|null} entConfig Resolved template config, or null.
 * @returns {object} throwaway view: spawn-authored fields + template appearance.
 */
export function buildSpawnRenderView(spawn, entConfig) {
  const view = { ...spawn };
  if (!entConfig) return view;
  if (!view.tags && entConfig.tags) view.tags = entConfig.tags;
  if (!view.radar_appearance && entConfig.radar_appearance) view.radar_appearance = entConfig.radar_appearance;
  if (!view.collider && entConfig.collider) view.collider = entConfig.collider;
  if (!view.shape && entConfig.shape) view.shape = entConfig.shape;
  // Synthesize a torus shape from an asteroid_field block. Checked AFTER the
  // `entConfig.shape` merge above so an explicit template shape wins.
  if (!view.shape && entConfig.asteroid_field) {
    view.shape = {
      type: 'torus',
      inner_radius: entConfig.asteroid_field.inner_radius ?? 100,
      outer_radius: entConfig.asteroid_field.outer_radius ?? 200,
    };
  }
  // Region-entity fields needed by the canvas-region renderer.
  if (!view.effects && entConfig.effects) view.effects = entConfig.effects;
  if (!view.colour && entConfig.colour) view.colour = entConfig.colour;
  return view;
}

export class CanvasManager {
  constructor(layerManager, onSpawnSelect, onSpawnUpdate, onSpawnCreate, onSpawnDrag, undoController) {
    if (!undoController || typeof undoController.snapshotForUndo !== 'function') {
      throw new Error('CanvasManager: undoController with snapshotForUndo is required');
    }
    this.layerManager = layerManager;
    this.onSpawnSelect = onSpawnSelect;
    this.onSpawnUpdate = onSpawnUpdate;
    this.onSpawnCreate = onSpawnCreate;
    this.onSpawnDrag = onSpawnDrag;
    this.undoController = undoController;

    this.stage = null;
    this.baseLayer = null;
    this.spawnGroups = new Map();
    this.arrowShapes = new Map();
    this.selectedSpawn = null;
    this.placeMode = null;
    this.placeEntityPath = null;

    this.scale = 1;
    this.offsetX = 0;
    this.offsetY = 0;
  }

  init() {
    const container = document.getElementById('canvas');

    this.stage = new Konva.Stage({
      container: 'canvas',
      width: container.clientWidth,
      height: container.clientHeight
    });

    this.baseLayer = new Konva.Layer();
    this.stage.add(this.baseLayer);

    this.stage.draggable(true);
    this.stage.on('dragmove', () => {
      this.offsetX = this.stage.x();
      this.offsetY = this.stage.y();
    });

    this.stage.on('wheel', (e) => {
      e.evt.preventDefault();
      const scaleBy = 1.1;
      const oldScale = this.scale;
      const pointer = this.stage.getPointerPosition();

      if (e.evt.deltaY < 0) {
        this.scale *= scaleBy;
      } else {
        this.scale /= scaleBy;
      }

      this.scale = Math.max(0.1, Math.min(5, this.scale));

      const mousePointTo = {
        x: (pointer.x - this.stage.x()) / oldScale,
        y: (pointer.y - this.stage.y()) / oldScale
      };

      this.stage.scale({ x: this.scale, y: this.scale });

      this.stage.x(pointer.x - mousePointTo.x * this.scale);
      this.stage.y(pointer.y - mousePointTo.y * this.scale);

      document.getElementById('zoomLevel').textContent = Math.round(this.scale * 100) + '%';
    });

    this.stage.on('click', (e) => {
      if (e.target === this.stage || e.target.getParent()?.className === 'Layer') {
        if (this.placeMode) {
          this.handleCanvasClick(e);
        } else {
          this.deselectSpawn();
        }
      }
    });

    window.addEventListener('resize', () => {
      const container = document.getElementById('canvas');
      this.stage.width(container.clientWidth);
      this.stage.height(container.clientHeight);
    });
  }

  worldToCanvas(x, z) {
    return {
      x: x * this.scale,
      y: -z * this.scale
    };
  }

  canvasToWorld(x, y) {
    return {
      x: x / this.scale,
      z: -y / this.scale
    };
  }

  async renderAll() {
    this.baseLayer.destroyChildren();
    this.spawnGroups.clear();
    this.arrowShapes.clear();

    const layers = this.layerManager.getLayers();
    const allAnchors = [];

    // First pass: aggregate anchors from all layers (used by spawn positioning).
    for (const layer of layers) {
      if (!layer.visible) continue;
      allAnchors.push(...getAnchors(layer));
    }

    // Adapter for cross-layer pure modules (anchor-rename, anchor-delete).
    const v2Layers = layers.map(l => ({ path: l.filename, worldState: l.toml }));

    for (const layer of layers) {
      if (!layer.visible) continue;

      const layerGroup = new Konva.Group();
      this.baseLayer.add(layerGroup);

      // Spawns first — region overlays + entity markers beneath anchors.
      const spawns = getSpawns(layer);
      for (const spawn of spawns) {
        this.renderSpawn(spawn, layer, layerGroup, allAnchors);
      }

      // Anchors LAST — drawn on top of entities for visibility + clickability.
      const anchorSpecs = getAnchorRenderSpecs(layer.toml?.anchors || {});
      for (const spec of anchorSpecs) {
        this.renderAnchor(spec, layer, layerGroup, v2Layers);
      }
    }

    this.updateArrows();
    this.baseLayer.batchDraw();
  }

  renderAnchor(spec, layer, container, v2Layers) {
    const pos = this.worldToCanvas(spec.x, spec.z);
    const group = new Konva.Group({
      x: pos.x,
      y: pos.y,
      draggable: true,
      name: 'anchorGroup',
    });

    const size = spec.size;
    // Cross-hair: horizontal + vertical lines, length spec.size*2.
    const hLine = new Konva.Line({
      points: [-size, 0, size, 0],
      stroke: '#ffff66',
      strokeWidth: 1.5,
    });
    const vLine = new Konva.Line({
      points: [0, -size, 0, size],
      stroke: '#ffff66',
      strokeWidth: 1.5,
    });

    const label = new Konva.Text({
      x: -40,
      y: size + 4,
      text: spec.name,
      fontSize: 10,
      fill: '#ffff66',
      width: 80,
      align: 'center',
    });

    group.add(hLine);
    group.add(vLine);
    group.add(label);

    // Drag: snapshot ONCE on dragstart (avoids the per-pixel snapshot anti-pattern
    // used by spawn drag). dragmove mutates the anchor; dragend re-renders to
    // refresh entity positions resolved via this anchor.
    group.on('dragstart', () => {
      this.undoController.snapshotForUndo(layer);
    });
    group.on('dragmove', () => {
      const newX = group.x() / this.scale;
      const newZ = -group.y() / this.scale;
      const existing = layer.toml?.anchors?.[spec.name];
      const preservedY = Array.isArray(existing) && existing.length >= 2 ? existing[1] : 0.0;
      if (!layer.toml.anchors) layer.toml.anchors = {};
      layer.toml.anchors[spec.name] = [newX, preservedY, newZ];
      layer.isDirty = true;
    });
    group.on('dragend', () => {
      this.renderAll();
    });

    // Right-click menu (commits C5/C6).
    group.on('contextmenu', (e) => {
      e.evt.preventDefault();
      this.showAnchorMenu(e.evt.clientX, e.evt.clientY, spec.name, layer);
    });

    container.add(group);
  }

  showAnchorMenu(clientX, clientY, anchorName, ownerLayer) {
    // Tear down any prior menu.
    this.dismissAnchorMenu();

    const menu = document.createElement('div');
    menu.style.cssText = [
      'position:fixed',
      `left:${clientX}px`,
      `top:${clientY}px`,
      'z-index:9999',
      'background:#2a2a2a',
      'border:1px solid #555',
      'border-radius:4px',
      'padding:4px 0',
      'box-shadow:0 4px 8px rgba(0,0,0,0.6)',
      'font-family:sans-serif',
      'font-size:12px',
      'color:#eee',
      'min-width:120px',
    ].join(';');

    const makeItem = (label, onClick) => {
      const item = document.createElement('div');
      item.textContent = label;
      item.style.cssText = 'padding:6px 12px;cursor:pointer';
      item.addEventListener('mouseenter', () => { item.style.background = '#444'; });
      item.addEventListener('mouseleave', () => { item.style.background = 'transparent'; });
      item.addEventListener('click', () => {
        this.dismissAnchorMenu();
        onClick();
      });
      return item;
    };

    menu.appendChild(makeItem('Rename', () => this.renameAnchor(anchorName, ownerLayer)));
    menu.appendChild(makeItem('Delete', () => this.deleteAnchor(anchorName, ownerLayer)));

    document.body.appendChild(menu);
    this._anchorMenu = menu;

    // Dismiss on next outside click.
    const dismissOnClick = (ev) => {
      if (!menu.contains(ev.target)) {
        this.dismissAnchorMenu();
      }
    };
    setTimeout(() => document.addEventListener('mousedown', dismissOnClick, { once: true }), 0);
    this._anchorMenuDismiss = () => document.removeEventListener('mousedown', dismissOnClick);
  }

  dismissAnchorMenu() {
    if (this._anchorMenu && this._anchorMenu.parentNode) {
      this._anchorMenu.parentNode.removeChild(this._anchorMenu);
    }
    if (this._anchorMenuDismiss) {
      this._anchorMenuDismiss();
      this._anchorMenuDismiss = null;
    }
    this._anchorMenu = null;
  }

  buildV2Layers() {
    return this.layerManager.getLayers().map(l => ({ path: l.filename, worldState: l.toml }));
  }

  renameAnchor(currentName, ownerLayer) {
    const newName = window.prompt('Rename anchor', currentName);
    if (newName == null || newName === '' || newName === currentName) return;

    const v2Layers = this.buildV2Layers();
    const result = analyzeAnchorRename(currentName, newName, v2Layers);
    if (!result.allowed) {
      window.alert(result.error || 'Rename blocked.');
      return;
    }
    if (result.crossLayerReferences.length > 0) {
      const proceed = window.confirm(
        `Anchor "${currentName}" is referenced in ${result.crossLayerReferences.length} other layer(s). These will also be rewritten. Proceed?`
      );
      if (!proceed) return;
    }

    const layersByPath = new Map(this.layerManager.getLayers().map(l => [l.filename, l]));

    // Snapshot + rewrite owner layers (anchor key + in-layer references).
    for (const pair of result.rewritePairs) {
      const ownerL = layersByPath.get(pair.layerPath);
      if (!ownerL || !ownerL.toml) continue;
      this.undoController.snapshotForUndo(ownerL);
      const anchors = ownerL.toml.anchors;
      if (anchors && anchors[currentName] != null) {
        anchors[newName] = anchors[currentName];
        delete anchors[currentName];
      }
      this.rewriteAnchorRefsInLayer(ownerL.toml, currentName, newName);
      ownerL.isDirty = true;
    }

    // Snapshot + rewrite cross-layer references (no anchor key change).
    const crossPaths = new Set(result.crossLayerReferences.map(r => r.layerPath));
    for (const crossPath of crossPaths) {
      const crossL = layersByPath.get(crossPath);
      if (!crossL || !crossL.toml) continue;
      this.undoController.snapshotForUndo(crossL);
      this.rewriteAnchorRefsInLayer(crossL.toml, currentName, newName);
      crossL.isDirty = true;
    }

    this.renderAll();
  }

  // An anchor referenced from a `[script]` body is not tracked (issue #985) —
  // a scripted `spawn_entity` names its anchor inside a Rhai map this pass
  // cannot read, so only `[[entity]]` references are rewritten.
  rewriteAnchorRefsInLayer(toml, oldName, newName) {
    if (!toml || typeof toml !== 'object') return;
    if (Array.isArray(toml.entity)) {
      for (const ent of toml.entity) {
        if (ent && ent.transform && ent.transform.anchor === oldName) {
          ent.transform.anchor = newName;
        }
      }
    }
  }

  deleteAnchor(anchorName, ownerLayer) {
    const v2Layers = this.buildV2Layers();
    const result = canDeleteAnchor(anchorName, v2Layers, ownerLayer?.filename);
    if (!result.canDelete) {
      const list = result.blockers
        .map(b => `- ${b.type} "${b.entityName ?? '(unnamed)'}" in ${b.layerPath}`)
        .join('\n');
      window.alert(`Cannot delete anchor "${anchorName}". It is referenced by:\n${list}`);
      return;
    }
    if (!window.confirm(`Delete anchor "${anchorName}"?`)) return;

    if (!ownerLayer || !ownerLayer.toml || !ownerLayer.toml.anchors) return;
    this.undoController.snapshotForUndo(ownerLayer);
    delete ownerLayer.toml.anchors[anchorName];
    ownerLayer.isDirty = true;
    this.renderAll();
  }

  renderSpawn(spawn, layer, container, allAnchors) {
    const name = getSpawnName(spawn);
    const pos = getSpawnPosition(spawn, allAnchors);
    const relative = getRelativeInfo(spawn);

    // Resolve template/fragment appearance onto a THROWAWAY view (issue #993).
    //
    // `getEntityConfig` returns the RESOLVED document (issue #910), whose fields
    // must drive what we DRAW but must never be baked onto `spawn`. The spawn is
    // the layer's persisted data model — a save serialises it straight to the
    // world/scenario TOML, and the override panel writes `spawn.override` into
    // the same object — so mutating it here would freeze a stale copy of the
    // template/fragment fields into the file. `buildSpawnRenderView` merges
    // those fields onto a fresh object read only by the drawing code below and
    // then discarded; `spawn` keeps only its own authored data + `template_path`
    // + `override`. Everything downstream reads `view`, not `spawn`.
    const entConfig = spawn.template_path
      ? getEntityConfig(spawn.template_path)
      : null;
    const view = buildSpawnRenderView(spawn, entConfig);

    const canvasPos = this.worldToCanvas(pos.x, pos.z);

    // Resolve appearance: use radar_appearance when present, X fallback otherwise
    const appearance = resolveEntityAppearance(view, allAnchors);
    const isSelected = this.selectedSpawn?.spawn === spawn;

    // Determine canvas radius for label placement
    const displayRadius = appearance.hasFallback
      ? 10
      : Math.max(4, Math.min(60, appearance.radius * 0.15 + 4));

    const group = new Konva.Group({
      x: canvasPos.x,
      y: canvasPos.y,
      draggable: true,
      name: 'spawnGroup'
    });

    // Draw region area overlay FIRST (behind the entity marker)
    if (view.asteroid_field) {
      const af = view.asteroid_field;
      const s = this.scale;
      const ring = new Konva.Ring({
        innerRadius: (af.inner_radius ?? 0) * s,
        outerRadius: (af.outer_radius ?? 0) * s,
        fill: '#85521e26',
        stroke: '#85521e',
        strokeWidth: 1 / s,
      });
      group.add(ring);
    }

    if (view.shape) {
      // Build a region entity input for the pure renderer. The spawn group is
      // already positioned at (canvasPos.x, canvasPos.y), so we want the spec
      // centred on the group origin — pass position=[0,0,0] and let the
      // shapes draw at local (0,0). `cx`/`cz` from the spec are not used here.
      const regionEntity = {
        shape: view.shape,
        colour: view.colour,
        effects: view.effects,
        position: [0, 0, 0],
      };
      const spec = getRegionRenderSpec(regionEntity);
      const hexColour = colourToHex(spec.colour);
      const alphaHex = Math.round(spec.fillAlpha * 255).toString(16).padStart(2, '0');
      const fillColour = hexColour + alphaHex;
      const s = this.scale;

      if (spec.shape === 'circle') {
        const fillCircle = new Konva.Circle({
          radius: (spec.radius ?? 0) * s,
          fill: fillColour,
          stroke: hexColour,
          strokeWidth: 1 / s
        });
        group.add(fillCircle);
      } else if (spec.shape === 'torus') {
        const ring = new Konva.Ring({
          innerRadius: (spec.inner_radius ?? 0) * s,
          outerRadius: (spec.outer_radius ?? 0) * s,
          fill: fillColour,
          stroke: hexColour,
          strokeWidth: 1 / s
        });
        group.add(ring);
      } else if (spec.shape === 'rect') {
        const hx = (spec.half_x ?? 0) * s;
        const hz = (spec.half_z ?? 0) * s;
        const rect = new Konva.Rect({
          x: -hx,
          y: -hz,
          width: hx * 2,
          height: hz * 2,
          fill: fillColour,
          stroke: hexColour,
          strokeWidth: 1 / s
        });
        group.add(rect);
      }

      // Effect-icon cluster: horizontal row centred on the region origin.
      if (Array.isArray(spec.effects) && spec.effects.length > 0) {
        const iconFontSize = 14;
        const iconGap = 4;
        const iconWidth = iconFontSize + iconGap;
        const totalWidth = spec.effects.length * iconWidth - iconGap;
        const startX = -totalWidth / 2;
        spec.effects.forEach((key, i) => {
          const glyph = spec.effectIcons[key] || '?';
          const txt = new Konva.Text({
            x: startX + i * iconWidth,
            y: -iconFontSize / 2,
            text: glyph,
            fontSize: iconFontSize,
            fill: '#ffffff',
          });
          group.add(txt);
        });
      }
    }

    // Draw the entity marker on top of the region shape
    drawEntityShape(group, Konva, appearance, isSelected);

    const label = new Konva.Text({
      x: -40,
      y: displayRadius + 4,
      text: name.length > 20 ? name.substring(0, 18) + '...' : name,
      fontSize: 10,
      fill: '#ffffff',
      width: 80,
      align: 'center'
    });

    group.add(label);

    group.on('mousedown', () => {
      this.selectSpawn(spawn, layer);
    });

    group.on('dragmove', () => {
      const newX = group.x() / this.scale;
      const newZ = -group.y() / this.scale;
      const oldPos = getSpawnPosition(spawn, allAnchors);

      if (this.placeMode) {
        setSpawnPosition(spawn, newX, newZ, 'absolute');
      } else {
        this.undoController.snapshotForUndo(layer);
        if (relative) {
          setSpawnPosition(spawn, newX, newZ, 'relative', relative.parent, { x: newX - oldPos.x, z: newZ - oldPos.z });
        } else {
          setSpawnPosition(spawn, newX, newZ, 'absolute');
        }
        layer.isDirty = true;

        if (this.selectedSpawn?.spawn === spawn) {
          this.onSpawnDrag(spawn, layer);
        }
      }
    });

    group.on('dragend', () => {
      if (!this.placeMode) {
        if (this.selectedSpawn?.spawn === spawn) {
          this.onSpawnUpdate(spawn, layer);
        }
      }
    });

    container.add(group);
    this.spawnGroups.set(spawn, group);
  }

  updateArrows() {
    for (const key of this.arrowShapes.keys()) {
      this.arrowShapes.get(key).destroy();
    }
    this.arrowShapes.clear();

    const allAnchors = [];
    for (const layer of this.layerManager.getLayers()) {
      if (!layer.visible) continue;
      allAnchors.push(...getAnchors(layer));
    }

    for (const layer of this.layerManager.getLayers()) {
      if (!layer.visible) continue;
      const spawns = getSpawns(layer);

      for (const spawn of spawns) {
        const relative = getRelativeInfo(spawn);
        if (!relative) continue;

        const childGroup = this.spawnGroups.get(spawn);
        if (!childGroup) continue;

        // Match the runtime's lookup, not the display label (issue #969): a
        // `relative_to` resolves against `name` OR `id`, so a parent that
        // carries both and is referenced by its `id` would never be found by
        // `getSpawnName` (which prefers `name`) and its arrow would be missing.
        // The sidebar's parent picker asks the same question of the same data,
        // so the two share one predicate rather than each spelling it out —
        // they disagreed once already, and the canvas drew an arrow to a parent
        // the sidebar was labelling `(unresolved)`.
        const parent = spawns.find(s => matchesSpawnReference(s, relative.parent));
        if (!parent) continue;

        const parentGroup = this.spawnGroups.get(parent);
        if (!parentGroup) continue;

        const arrow = new Konva.Arrow({
          points: [0, 0, childGroup.x() - parentGroup.x(), childGroup.y() - parentGroup.y()],
          stroke: '#ff9800',
          strokeWidth: 2,
          dash: [5, 5],
          pointerLength: 8,
          pointerWidth: 8
        });

        parentGroup.add(arrow);
        this.arrowShapes.set(`${getSpawnName(parent)}->${getSpawnName(spawn)}`, arrow);
      }
    }
  }

  handleCanvasClick(e) {
    if (!this.placeMode || !this.placeEntityPath) return;

    const pointer = this.stage.getPointerPosition();
    const worldPos = this.canvasToWorld(
      pointer.x - this.offsetX,
      pointer.y - this.offsetY
    );

    const activeLayer = this.layerManager.getActiveLayer();
    if (!activeLayer) return;

    const spawns = getSpawns(activeLayer);
    const name = this.placeMode + '_' + (spawns.length + 1);

    const spawn = {
      name,
      tags: [this.placeMode]
    };

    setSpawnPosition(spawn, worldPos.x, worldPos.z, 'absolute');

    spawn.template_path = this.placeEntityPath;

    this.undoController.snapshotForUndo(activeLayer);
    if (!activeLayer.toml.entity) {
      activeLayer.toml.entity = [];
    }
    activeLayer.toml.entity.push(spawn);
    activeLayer.isDirty = true;

    this.onSpawnCreate(spawn, activeLayer);
    this.selectSpawn(spawn, activeLayer);

    this.placeMode = null;
    this.placeEntityPath = null;
    this.renderAll();
  }

  selectSpawn(spawn, layer) {
    this.selectedSpawn = { spawn, layer };
    this.onSpawnSelect(spawn, layer);
  }

  /**
   * Find a spawn by `entity.name` across every open layer and select it.
   * First-match-wins; layers are scanned in `getLayers()` order.
   * No-op (returns false) if no spawn carries that name.
   *
   * Used by the World Content panel to highlight a referenced entity on
   * the canvas (Slice 3 PRD #350 AC #2).
   *
   * @param {string} name
   * @returns {boolean}  true if a spawn was found and selected.
   */
  selectByEntityName(name) {
    if (!name) return false;
    for (const layer of this.layerManager.getLayers()) {
      const list = layer.toml?.entity;
      if (!Array.isArray(list)) continue;
      const match = list.find(s => s && s.name === name);
      if (match) {
        // Activate the owning layer so subsequent panels (properties,
        // world-content active-layer filtering) line up with the selection.
        this.layerManager.setActiveLayer(layer);
        this.selectSpawn(match, layer);
        this.renderAll();
        return true;
      }
    }
    return false;
  }

  deselectSpawn() {
    this.selectedSpawn = null;
    this.onSpawnSelect(null, null);
  }

  getSelectedSpawn() {
    return this.selectedSpawn;
  }

  startPlaceMode(mode, entityPath) {
    this.placeMode = mode;
    this.placeEntityPath = entityPath;
  }

  cancelPlaceMode() {
    this.placeMode = null;
    this.placeEntityPath = null;
  }

  isPlacing() {
    return this.placeMode !== null;
  }
}