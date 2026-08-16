import { stringifyToml } from './toml-utils.js';
import { getColorForEntity } from './layers.js';
import { entityCache, loadEntityConfig, preloadEntityList } from './entity-cache.js';

const templates = {
  ship: {
    name: 'Ship',
    scaffold: { tags: ['ship'], collider: { shape: 'Ball', radius: 8.0 }, hull: { hull_integrity: 100 } }
  },
  station: {
    name: 'Station',
    // `half_height` is not optional on a Cylinder — the Rust loader rejects one
    // without it — so a scaffold that omitted it produced a station TOML the
    // game refuses to load. It had been doing so since before `Cylinder` was a
    // shape at all.
    scaffold: { tags: ['station'], collider: { shape: 'Cylinder', radius: 15.0, half_height: 6.0 }, hull: { hull_integrity: 200 } }
  },
  asteroid: {
    name: 'Asteroid',
    scaffold: { tags: ['asteroid'], collider: { shape: 'Ball', radius: 5.0 } }
  },
  region: {
    name: 'Region',
    scaffold: { tags: ['region'], shape: { type: 'sphere', radius: 100.0 } }
  },
  custom: {
    name: 'Custom',
    scaffold: { tags: [] }
  }
};

export class EntityEditor {
  constructor(canvasManager, layerManager, onEntitySaved) {
    this.canvasManager = canvasManager;
    this.layerManager = layerManager;
    this.onEntitySaved = onEntitySaved;
    this.selectedTemplate = null;
    this.entities = [];
  }

  init() {
    this.modal = document.getElementById('newEntityModal');
    this.form = document.getElementById('newEntityForm');

    this.templateBtns = document.querySelectorAll('.template-btn');
    this.templateBtns.forEach(btn => {
      btn.addEventListener('click', () => {
        this.selectTemplate(btn.dataset.template);
        this.templateBtns.forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
      });
    });

    document.getElementById('cancelNewEntity').addEventListener('click', () => {
      this.closeModal();
    });

    this.form.addEventListener('submit', (e) => {
      e.preventDefault();
      this.saveEntity();
    });

    document.getElementById('entityName').addEventListener('input', () => this.updateRawToml());
    document.getElementById('entityTags').addEventListener('input', () => this.updateRawToml());
    document.getElementById('colliderShape').addEventListener('change', () => this.updateRawToml());
    document.getElementById('colliderRadius').addEventListener('input', () => this.updateRawToml());
    document.getElementById('hullIntegrity').addEventListener('input', () => this.updateRawToml());
    document.getElementById('regionShape').addEventListener('change', () => {
      const isTorus = document.getElementById('regionShape').value === 'torus';
      document.getElementById('regionRadiusField').style.display = isTorus ? 'none' : '';
      document.getElementById('regionTorusFields').style.display = isTorus ? '' : 'none';
      this.updateRawToml();
    });
    document.getElementById('regionRadius').addEventListener('input', () => this.updateRawToml());
    document.getElementById('regionInnerRadius').addEventListener('input', () => this.updateRawToml());
    document.getElementById('regionOuterRadius').addEventListener('input', () => this.updateRawToml());
    document.getElementById('rawToml').addEventListener('input', (e) => {
      try {
        const obj = window.tomlParse(e.target.value);
        if (obj.tags) document.getElementById('entityTags').value = obj.tags.join(', ');
      } catch { }
    });
  }

  loadEntitiesPalette() {
    const container = document.getElementById('entitiesList');
    container.innerHTML = '';

    const known = preloadEntityList();
    for (const ent of known) {
      this.addEntityToPalette(container, ent.name, ent.path, ent.tags);
    }

    for (const [path, config] of entityCache) {
      const name = path.split('/').pop().replace('.toml', '');
      const tags = config.tags || [];
      if (!known.find(e => e.path === path)) {
        this.addEntityToPalette(container, name, path, tags);
      }
    }
  }

  addEntityToPalette(container, name, path, tags) {
    const color = getColorForEntity(tags);

    const el = document.createElement('div');
    el.className = 'entity-palette-item';
    el.innerHTML = `
      <span class="color-dot" style="background: ${color}"></span>
      <span>${name}</span>
    `;
    el.title = tags.join(', ');

    el.addEventListener('click', () => {
      this.canvasManager.startPlaceMode(name, path);
      document.querySelectorAll('.entity-palette-item').forEach(e => e.classList.remove('selected'));
      el.classList.add('selected');
    });

    container.appendChild(el);
  }

  selectTemplate(template) {
    this.selectedTemplate = template;
    const tmpl = templates[template];

    document.getElementById('entityName').value = '';
    document.getElementById('entityTags').value = '';
    document.getElementById('colliderShape').value = 'Ball';
    document.getElementById('colliderRadius').value = '8';
    document.getElementById('hullIntegrity').value = '100';
    document.getElementById('regionShape').value = 'sphere';
    document.getElementById('regionRadius').value = '100';
    document.getElementById('effectCommsJam').checked = false;
    document.getElementById('effectSensorBlind').checked = false;
    document.getElementById('effectDamageZone').checked = false;
    document.getElementById('effectSlowZone').checked = false;

    document.querySelector('.physics-fields').classList.add('hidden');
    document.querySelector('.region-fields').classList.add('hidden');

    if (template === 'ship' || template === 'station' || template === 'asteroid') {
      document.querySelector('.physics-fields').classList.remove('hidden');
    } else if (template === 'region') {
      document.querySelector('.region-fields').classList.remove('hidden');
    }

    this.updateRawToml();
  }

  updateRawToml() {
    if (!this.selectedTemplate) return;

    const tagsStr = document.getElementById('entityTags').value;
    const tags = tagsStr.split(',').map(t => t.trim()).filter(t => t);

    let obj = { tags };

    if (this.selectedTemplate === 'ship' || this.selectedTemplate === 'station') {
      obj = {
        tags,
        collider: {
          shape: document.getElementById('colliderShape').value,
          radius: parseFloat(document.getElementById('colliderRadius').value)
        },
        hull: {
          hull_integrity: parseInt(document.getElementById('hullIntegrity').value)
        }
      };
    } else if (this.selectedTemplate === 'asteroid') {
      obj = {
        tags,
        collider: {
          shape: document.getElementById('colliderShape').value,
          radius: parseFloat(document.getElementById('colliderRadius').value)
        }
      };
    } else if (this.selectedTemplate === 'region') {
      const effects = {};
      if (document.getElementById('effectCommsJam').checked) effects.comms_jam = {};
      if (document.getElementById('effectSensorBlind').checked) effects.sensor_blind = {};
      if (document.getElementById('effectDamageZone').checked) effects.damage_zone = {};
      if (document.getElementById('effectSlowZone').checked) effects.slow_zone = {};

      const shapeType = document.getElementById('regionShape').value;
      let shape;
      if (shapeType === 'torus') {
        shape = {
          type: 'torus',
          inner_radius: parseFloat(document.getElementById('regionInnerRadius').value),
          outer_radius: parseFloat(document.getElementById('regionOuterRadius').value)
        };
      } else {
        shape = {
          type: shapeType,
          radius: parseFloat(document.getElementById('regionRadius').value)
        };
      }

      obj = { tags, shape };
      if (Object.keys(effects).length > 0) {
        obj.effects = effects;
      }
    }

    const raw = stringifyToml(obj);
    document.getElementById('rawToml').value = raw;
  }

  async saveEntity() {
    const rawToml = document.getElementById('rawToml').value;
    const name = document.getElementById('entityName').value || 'untitled';

    try {
      const fileHandle = await window.showSaveFilePicker({
        suggestedName: `${name}.toml`,
        types: [{ description: 'TOML File', accept: { 'application/toml': ['.toml'] } }]
      });

      const writable = await fileHandle.createWritable();
      await writable.write(rawToml);
      await writable.close();

      const path = `assets/entities/${fileHandle.name}`;
      entityCache.set(path, window.tomlParse(rawToml));
      this.onEntitySaved({ name, path });
      this.closeModal();
    } catch (err) {
      if (err.name !== 'AbortError') {
        console.error('Failed to save entity:', err);
      }
    }
  }

  openModal() {
    this.modal.classList.remove('hidden');
    this.selectTemplate('ship');
  }

  closeModal() {
    this.modal.classList.add('hidden');
  }
}