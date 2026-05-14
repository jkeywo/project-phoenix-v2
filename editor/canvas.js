import { getSpawns, getAnchors, getSpawnPosition, getSpawnName, getEntityPath, getRelativeInfo, setSpawnPosition } from './toml-utils.js';
import { getColorForEntity } from './layers.js';
import { loadEntityConfig, getEntityConfig } from './entity-cache.js';

export class CanvasManager {
  constructor(layerManager, onSpawnSelect, onSpawnUpdate, onSpawnCreate, onSpawnDrag) {
    this.layerManager = layerManager;
    this.onSpawnSelect = onSpawnSelect;
    this.onSpawnUpdate = onSpawnUpdate;
    this.onSpawnCreate = onSpawnCreate;
    this.onSpawnDrag = onSpawnDrag;

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

    for (const layer of layers) {
      if (!layer.visible) continue;

      const layerGroup = new Konva.Group();
      this.baseLayer.add(layerGroup);

      const anchors = getAnchors(layer);
      allAnchors.push(...anchors);

      for (const anchor of anchors) {
        const pos = this.worldToCanvas(anchor.position[0], anchor.position[2]);
        const diamond = new Konva.Shape({
          x: pos.x,
          y: pos.y,
          stroke: '#ffffff',
          strokeWidth: 2,
          sceneFunc: (ctx, shape) => {
            const size = 8;
            ctx.beginPath();
            ctx.moveTo(size, 0);
            ctx.lineTo(0, size);
            ctx.lineTo(-size, 0);
            ctx.lineTo(0, -size);
            ctx.closePath();
            ctx.stroke();
          }
        });

        const label = new Konva.Text({
          x: -30,
          y: 12,
          text: anchor.name,
          fontSize: 10,
          fill: '#ffffff',
          width: 60,
          align: 'center'
        });

        const group = new Konva.Group({ draggable: false });
        group.add(diamond);
        group.add(label);
        layerGroup.add(group);
      }

      const spawns = getSpawns(layer);
      for (const spawn of spawns) {
        const entConfig = spawn.entity_path || spawn.template_path
          ? getEntityConfig(spawn.entity_path || spawn.template_path)
          : null;

        if (entConfig) {
          if (!spawn.tags && entConfig.tags) spawn.tags = entConfig.tags;
          if (!spawn.collider && entConfig.collider) spawn.collider = entConfig.collider;
          if (!spawn.shape && entConfig.shape) spawn.shape = entConfig.shape;
        }

        this.renderSpawn(spawn, layer, layerGroup, allAnchors);
      }
    }

    this.updateArrows();
    this.baseLayer.batchDraw();
  }

  renderSpawn(spawn, layer, container, allAnchors) {
    const name = getSpawnName(spawn);
    const pos = getSpawnPosition(spawn, allAnchors);
    const relative = getRelativeInfo(spawn);
    const tags = spawn.tags || [];
    const color = getColorForEntity(tags);

    const canvasPos = this.worldToCanvas(pos.x, pos.z);

    let radius = 12;
    if (spawn.collider && spawn.collider.radius) {
      radius = Math.max(8, Math.min(30, spawn.collider.radius));
    } else if (spawn.shape && spawn.shape.radius) {
      radius = Math.max(10, Math.min(50, spawn.shape.radius));
    }

    const group = new Konva.Group({
      x: canvasPos.x,
      y: canvasPos.y,
      draggable: true,
      name: 'spawnGroup'
    });

    const circle = new Konva.Circle({
      radius: radius,
      fill: color,
      stroke: '#ffffff',
      strokeWidth: 2
    });

    group.add(circle);

    if (spawn.shape && spawn.shape.type === 'sphere' && spawn.shape.radius) {
      const fillCircle = new Konva.Circle({
        radius: spawn.shape.radius,
        fill: color + '33',
        stroke: color,
        strokeWidth: 1
      });
      group.add(fillCircle);
    }

    const label = new Konva.Text({
      x: -40,
      y: radius + 4,
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

        const parent = spawns.find(s => getSpawnName(s) === relative.parent);
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

    const arr = activeLayer.kind === 'scenario' ? 'spawn' : 'entity';
    spawn[activeLayer.kind === 'scenario' ? 'entity_path' : 'template_path'] = this.placeEntityPath;

    if (!activeLayer.toml[arr]) {
      activeLayer.toml[arr] = [];
    }
    activeLayer.toml[arr].push(spawn);
    activeLayer.isDirty = true;

    this.onSpawnCreate(spawn, activeLayer);
    this.selectSpawn(spawn, activeLayer);

    this.placeMode = null;
    this.placeEntityPath = null;
    this.renderAll();
  }

  selectSpawn(spawn, layer) {
    this.selectedSpawn = { spawn, layer };

    for (const [s, group] of this.spawnGroups) {
      const circle = group.find('Circle')[0];
      if (s === spawn) {
        circle.stroke('#00ff00');
        circle.strokeWidth(4);
      } else {
        circle.stroke('#ffffff');
        circle.strokeWidth(2);
      }
    }

    this.onSpawnSelect(spawn, layer);
  }

  deselectSpawn() {
    this.selectedSpawn = null;

    for (const [spawn, group] of this.spawnGroups) {
      const circle = group.find('Circle')[0];
      if (circle) {
        circle.stroke('#ffffff');
        circle.strokeWidth(2);
      }
    }

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