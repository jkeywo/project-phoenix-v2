/**
 * models-mode-view.js
 *
 * Integration/mount for Models Mode. Discovers `assets/models/*.glb`, pairs
 * each with its sidecar variants, and renders a three-pane shell:
 *
 *   Left   — model list, and (per selected model) its variant list +
 *            "Save as new variant" entry.
 *   Centre — base transform form + marker list (add/rename/delete/select) +
 *            gizmo-mode toggle + Save button.
 *   Right  — the Three.js canvas (createRigScene) + live extents readout.
 *
 * All rig math goes through models-rig.js; the 3D view is models-rig-view.js.
 * This file is the glue: file IO, DOM, dirty-marking via modeShell.
 *
 * Dirty/guard: per Q11, Models uses LOCAL Save buttons. We participate in
 * the shared unsaved indicator + tab-switch guard ONLY via
 * `modeShell.markDirty('Models', path, ...)`. Save writes TOML directly via
 * `io.writeFile` — it does NOT route through SaveFlow. See the Save-All
 * safety note in the implementation report.
 */
import {
  defaultRig,
  parseRigToml,
  buildRigToml,
  computeExtents,
  addMarker as rigAddMarker,
  updateMarker as rigUpdateMarker,
  removeMarker as rigRemoveMarker,
  renameMarker as rigRenameMarker,
  buildSidecarName,
  groupModelFiles,
  validateVariantName,
  DEFAULT_VARIANT,
  FORWARD,
} from './models-rig.js';

const MODELS_DIR = 'assets/models';
const MODE = 'Models';

/**
 * @param {object} opts
 * @param {HTMLElement} opts.host
 * @param {import('./mode-shell.js').ModeShell} opts.modeShell
 * @param {{ readFile, writeFile, listDirectory, readBinaryFile }} opts.io
 * @param {import('./save-flow.js').SaveFlow} [opts.saveFlow]
 *   Optional. When supplied, Models caches its serialized TOML string via
 *   `setContent('Models', path, toml)` so a global Save All writes the file
 *   verbatim (through the registered passthrough 'models' stringifier)
 *   instead of mis-routing or no-op'ing. Models' own Save buttons still
 *   write directly via `io.writeFile`.
 * @param {object} [opts.deps]  test seam: { createRigScene }
 */
export function mountModelsMode({ host, modeShell, saveFlow = null, io, deps = {} }) {
  if (!host) return null;

  const createScene = deps.createRigScene || null; // lazy-imported below
  const readFile = io?.readFile;
  const writeFile = io?.writeFile;
  const listDirectory = io?.listDirectory;
  const readBinaryFile = io?.readBinaryFile;
  const onRootChanged = io?.onRootChanged;

  // ── State ─────────────────────────────────────────────────────────────
  let models = []; // [{ stem, glb, variants: [] }]
  let activeStem = null;
  let activeVariant = null; // variant name string
  let rig = defaultRig();
  let scene = null; // rig-view controller
  let loadedStem = null; // which glb is currently in the scene
  let gizmoMode = 'translate';
  let selectedMarker = null;
  let resizeObserver = null; // observes canvasHost -> scene.resize()
  let rootChangeSub = null; // onRootChanged subscription handle

  // ── DOM skeleton ──────────────────────────────────────────────────────
  host.innerHTML = '';
  const wrap = el('div', 'models-three-pane');
  host.appendChild(wrap);

  const leftPane = el('div', 'models-pane models-pane-left');
  // Centre column = viewport (canvas); right column = transform + markers
  const viewportPane = el('div', 'models-pane models-pane-viewport');
  const controlPane = el('div', 'models-pane models-pane-controls');
  wrap.append(leftPane, viewportPane, controlPane);

  // Keep legacy variable names so existing code below compiles unchanged.
  // renderCenter() writes the transform/marker controls into controlPane.
  const centerPane = controlPane;
  const rightPane = viewportPane; // unused after this point except extents

  const canvasHost = el('div', 'models-rig-canvas');
  const extentsDisplay = el('div', 'models-extents-display');
  viewportPane.append(canvasHost, extentsDisplay);

  // ── Path helpers ──────────────────────────────────────────────────────
  const sidecarPath = (stem, variant) => `${MODELS_DIR}/${buildSidecarName(stem, variant)}`;
  const glbPath = (glb) => `${MODELS_DIR}/${glb}`;

  function activeModel() {
    return models.find((m) => m.stem === activeStem) || null;
  }
  function activePath() {
    if (!activeStem || !activeVariant) return null;
    return sidecarPath(activeStem, activeVariant);
  }

  // ── Rendering ─────────────────────────────────────────────────────────
  function renderLeft() {
    leftPane.innerHTML = '';
    leftPane.appendChild(sectionTitle('MODELS'));
    if (models.length === 0) {
      leftPane.appendChild(placeholder('No .glb files in assets/models.'));
      return;
    }
    const list = el('div', 'models-file-list');
    for (const m of models) {
      const row = el('div', 'models-file-row');
      if (m.stem === activeStem) row.classList.add('models-file-row-active');
      row.textContent = m.stem;
      row.addEventListener('click', () => selectModel(m.stem));
      list.appendChild(row);
    }
    leftPane.appendChild(list);

    const model = activeModel();
    if (!model) return;

    leftPane.appendChild(sectionTitle('VARIANTS'));
    const vlist = el('div', 'models-variant-list');
    const variants = model.variants.length ? model.variants : [DEFAULT_VARIANT];
    for (const v of variants) {
      const row = el('div', 'models-variant-row');
      if (v === activeVariant) row.classList.add('models-variant-row-active');
      const label = el('span', 'models-variant-label');
      label.textContent = v;
      row.appendChild(label);
      if (modeShell.isDirty(MODE, sidecarPath(model.stem, v))) {
        row.appendChild(dirtyDot());
      }
      row.addEventListener('click', () => selectVariant(v));
      vlist.appendChild(row);
    }
    leftPane.appendChild(vlist);

    // Save-as-new-variant entry.
    const newVarWrap = el('div', 'models-new-variant');
    const input = el('input', 'models-new-variant-input');
    input.type = 'text';
    input.placeholder = 'new variant name';
    const btn = el('button', 'models-btn');
    btn.textContent = 'Save as new variant';
    btn.addEventListener('click', () => {
      const name = input.value.trim();
      if (!name) return;
      saveAsVariant(name);
    });
    newVarWrap.append(input, btn);
    leftPane.appendChild(newVarWrap);
  }

  function renderCenter() {
    centerPane.innerHTML = '';
    if (!activeStem) {
      centerPane.appendChild(placeholder('Select a model from the left.'));
      return;
    }

    // Base transform form.
    centerPane.appendChild(sectionTitle('BASE TRANSFORM'));
    const baseForm = el('div', 'models-base-form');
    baseForm.appendChild(vec3Row('Offset', rig.base.offset, (v) => {
      rig.base.offset = v; onBaseEdit();
    }, 0.1));
    baseForm.appendChild(vec3Row('Rotation (rad)', rig.base.rotation, (v) => {
      rig.base.rotation = v; onBaseEdit();
    }, 0.01));
    baseForm.appendChild(vec3Row('Scale', rig.base.scale, (v) => {
      rig.base.scale = v; onBaseEdit();
    }, 0.1));
    centerPane.appendChild(baseForm);

    // Marker list.
    centerPane.appendChild(sectionTitle('MARKERS'));
    centerPane.appendChild(renderGizmoToggle());
    const mlist = el('div', 'models-marker-list');
    const names = Object.keys(rig.markers);
    if (names.length === 0) {
      mlist.appendChild(placeholder('No markers yet.'));
    }
    for (const name of names) {
      mlist.appendChild(renderMarkerRow(name));
    }
    centerPane.appendChild(mlist);

    // Add-marker entry.
    const addWrap = el('div', 'models-add-marker');
    const addInput = el('input', 'models-add-marker-input');
    addInput.type = 'text';
    addInput.placeholder = 'marker name';
    const addBtn = el('button', 'models-btn');
    addBtn.textContent = '+ Add marker';
    addBtn.addEventListener('click', () => {
      const name = addInput.value.trim();
      if (!name) return;
      handleAddMarker(name);
    });
    addWrap.append(addInput, addBtn);
    centerPane.appendChild(addWrap);

    // Save button.
    const saveWrap = el('div', 'models-save-row');
    const saveBtn = el('button', 'models-btn models-btn-primary');
    const path = activePath();
    saveBtn.textContent = `Save ${activeVariant}`;
    if (path && modeShell.isDirty(MODE, path)) saveBtn.classList.add('models-btn-dirty');
    saveBtn.addEventListener('click', () => saveCurrent());
    saveWrap.appendChild(saveBtn);
    centerPane.appendChild(saveWrap);
  }

  function renderGizmoToggle() {
    const wrapEl = el('div', 'models-gizmo-toggle');
    for (const mode of ['translate', 'rotate']) {
      const b = el('button', 'models-gizmo-btn');
      b.textContent = mode === 'translate' ? 'Move' : 'Rotate';
      if (gizmoMode === mode) b.classList.add('active');
      b.addEventListener('click', () => {
        gizmoMode = mode;
        scene?.setGizmoMode(mode);
        renderCenter();
      });
      wrapEl.appendChild(b);
    }
    return wrapEl;
  }

  function renderMarkerRow(name) {
    const wrap = el('div', 'models-marker-wrap');

    const row = el('div', 'models-marker-row');
    if (name === selectedMarker) row.classList.add('models-marker-row-active');

    const label = el('span', 'models-marker-label');
    label.textContent = name;
    label.addEventListener('click', () => selectMarker(name));
    row.appendChild(label);

    const renameBtn = el('button', 'models-marker-btn');
    renameBtn.textContent = 'Rename';
    renameBtn.addEventListener('click', () => {
      const next = (typeof prompt === 'function') ? prompt('Rename marker', name) : null;
      if (next && next.trim() && next.trim() !== name) handleRenameMarker(name, next.trim());
    });
    row.appendChild(renameBtn);

    const delBtn = el('button', 'models-marker-btn models-marker-btn-delete');
    delBtn.textContent = 'Delete';
    delBtn.addEventListener('click', () => handleRemoveMarker(name));
    row.appendChild(delBtn);

    wrap.appendChild(row);

    // Numerical editors shown only for the selected marker.
    if (name === selectedMarker) {
      const m = rig.markers[name];
      const fields = el('div', 'models-marker-fields');

      fields.appendChild(vec3Row('Position', m.position, (v) => {
        m.position = v;
        rigUpdateMarker(rig, name, { position: v, direction: m.direction });
        scene?.addMarker(name, { position: v, direction: m.direction });
        scene?.select(name);
        markCurrentDirty();
      }, 0.01));

      fields.appendChild(vec3Row('Direction', m.direction, (v) => {
        m.direction = v;
        rigUpdateMarker(rig, name, { position: m.position, direction: v });
        scene?.addMarker(name, { position: m.position, direction: v });
        scene?.select(name);
        markCurrentDirty();
      }, 0.01));

      wrap.appendChild(fields);
    }

    return wrap;
  }

  function renderExtents() {
    const ext = scene ? scene.getExtents() : rig.extents;
    extentsDisplay.innerHTML = '';
    const fmt = (n) => (Number.isFinite(n) ? n.toFixed(2) : '—');
    const rowsData = [
      ['min', ext.min],
      ['max', ext.max],
      ['size', ext.size],
    ];
    for (const [label, v] of rowsData) {
      const r = el('div', 'models-extents-row');
      r.textContent = `${label}: [${fmt(v[0])}, ${fmt(v[1])}, ${fmt(v[2])}]`;
      extentsDisplay.appendChild(r);
    }
  }

  function renderAll() {
    renderLeft();
    renderCenter();
    renderExtents();
  }

  // ── Edit pipeline ─────────────────────────────────────────────────────
  function markCurrentDirty() {
    const path = activePath();
    if (!path) return;
    modeShell.markDirty(MODE, path, true);
    // Cache the serialized TOML so a global Save All can write it verbatim
    // (via the 'models' passthrough stringifier). Best-effort: Models' own
    // Save buttons don't depend on this.
    if (saveFlow && typeof saveFlow.setContent === 'function') {
      if (scene) rig.extents = computeExtents(scene.getExtents());
      saveFlow.setContent(MODE, path, buildRigToml(rig));
    }
    refreshUnsavedIndicator();
  }

  // Drive the shared cross-mode unsaved indicator. Scenario Mode owns the
  // same element and ORs in modeShell.hasAnyDirty(); Models edits don't pass
  // through Scenario Mode, so refresh it here too. Derived purely from
  // modeShell so we never clobber another mode's dirty state.
  function refreshUnsavedIndicator() {
    if (typeof document === 'undefined') return;
    const indicator = document.getElementById('unsavedIndicator');
    if (!indicator) return;
    indicator.textContent = modeShell.hasAnyDirty?.() ? '● Unsaved changes' : '';
  }

  function onBaseEdit() {
    if (scene) {
      const ext = scene.setBase(rig.base);
      rig.extents = computeExtents(ext);
    }
    markCurrentDirty();
    renderExtents();
    renderLeft();
    renderCenter();
  }

  function handleAddMarker(name) {
    if (rig.markers[name]) return;
    rigAddMarker(rig, name, { position: [0, 0, 0], direction: [...FORWARD] });
    scene?.addMarker(name, rig.markers[name]);
    markCurrentDirty();
    selectMarker(name);
    renderAll();
  }

  function handleRemoveMarker(name) {
    rigRemoveMarker(rig, name);
    scene?.removeMarker(name);
    if (selectedMarker === name) selectedMarker = null;
    markCurrentDirty();
    renderAll();
  }

  function handleRenameMarker(from, to) {
    try {
      rigRenameMarker(rig, from, to);
    } catch (err) {
      console.warn('[models-mode] rename failed:', err?.message || err);
      return;
    }
    // Rebuild the marker visual under the new name.
    scene?.removeMarker(from);
    scene?.addMarker(to, rig.markers[to]);
    if (selectedMarker === from) selectedMarker = to;
    markCurrentDirty();
    renderAll();
    if (selectedMarker) scene?.select(selectedMarker);
  }

  function selectMarker(name) {
    selectedMarker = name;
    scene?.select(name);
    scene?.setGizmoMode(gizmoMode);
    renderCenter();
  }

  // Gizmo → rig sync (from the 3D view).
  function onMarkerMoved(name, { position, direction }) {
    rigUpdateMarker(rig, name, { position, direction });
    markCurrentDirty();
    // Refresh dirty indicator + numerical fields for the moved marker.
    renderLeft();
    if (name === selectedMarker) renderCenter();
  }

  // ── Selection / loading ───────────────────────────────────────────────
  async function selectModel(stem) {
    activeStem = stem;
    const model = activeModel();
    const variants = model?.variants?.length ? model.variants : [DEFAULT_VARIANT];
    await selectVariant(variants[0]);
  }

  async function selectVariant(variant) {
    activeVariant = variant;
    selectedMarker = null;
    const model = activeModel();
    if (!model) return;

    // Load rig sidecar (or default if none exists yet).
    rig = defaultRig();
    const path = sidecarPath(model.stem, variant);
    if (model.variants.includes(variant)) {
      try {
        const text = await readFile(path);
        rig = parseRigToml(text);
      } catch (err) {
        console.warn(`[models-mode] failed to read ${path}:`, err?.message || err);
      }
    }
    modeShell.setActiveFile(MODE, path);

    await ensureScene();
    await loadGlbIfNeeded(model);
    if (scene) {
      // Drop the previous variant's marker visuals first; addMarker only
      // replaces same-named markers, so without this they'd ghost.
      scene.clearMarkers?.();
      const ext = scene.setBase(rig.base);
      rig.extents = computeExtents(ext);
      // (Re)create marker visuals in the post-base-rig frame.
      for (const [name, m] of Object.entries(rig.markers)) {
        scene.addMarker(name, m);
      }
      // Frame AFTER the base transform is applied so the camera fits the
      // rigged model, not the raw GLB (loadModel framed pre-setBase).
      scene.frame?.();
    }
    renderAll();
  }

  async function ensureScene() {
    if (scene) return;
    let factory = createScene;
    if (!factory) {
      const mod = await import('./models-rig-view.js');
      factory = mod.createRigScene;
    }
    try {
      scene = factory(canvasHost, deps.sceneDeps || {});
      scene.onChange(onMarkerMoved);
      attachResizeObserver();
    } catch (err) {
      console.warn('[models-mode] 3D scene unavailable:', err?.message || err);
      scene = null;
      canvasHost.innerHTML = '<p class="models-canvas-error">3D preview unavailable.</p>';
    }
  }

  function attachResizeObserver() {
    if (resizeObserver || typeof ResizeObserver !== 'function') return;
    resizeObserver = new ResizeObserver(() => {
      scene?.resize?.();
    });
    resizeObserver.observe(canvasHost);
  }

  /**
   * Tear down the live scene + observers. Called when a fresh scene must
   * replace the current one (e.g. project-root change) and on teardown().
   */
  function disposeScene() {
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (scene) {
      try { scene.dispose(); } catch (err) {
        console.warn('[models-mode] scene dispose failed:', err?.message || err);
      }
      scene = null;
    }
    loadedStem = null;
  }

  /** Full teardown: dispose the scene and drop the root-change subscription. */
  function teardown() {
    disposeScene();
    if (rootChangeSub && typeof rootChangeSub.unsubscribe === 'function') {
      rootChangeSub.unsubscribe();
      rootChangeSub = null;
    }
  }

  async function loadGlbIfNeeded(model) {
    if (!scene) return;
    if (loadedStem === model.stem) return;
    if (typeof readBinaryFile !== 'function') return;
    try {
      const buf = await readBinaryFile(glbPath(model.glb));
      await scene.loadModel(buf);
      loadedStem = model.stem;
    } catch (err) {
      console.warn(`[models-mode] failed to load ${model.glb}:`, err?.message || err);
    }
  }

  // ── Save ──────────────────────────────────────────────────────────────
  async function saveCurrent() {
    const model = activeModel();
    if (!model || !activeVariant) return;
    await writeRig(model.stem, activeVariant);
  }

  async function saveAsVariant(name) {
    const model = activeModel();
    if (!model) return;

    // Guard reserved + duplicate names (pure logic in models-rig.js).
    const verdict = validateVariantName(name, model.variants);
    if (!verdict.ok) {
      if (verdict.reason === 'reserved') {
        if (typeof alert === 'function') {
          alert(`"${DEFAULT_VARIANT}" is reserved — use the Save button to write the default sidecar.`);
        } else {
          console.warn(`[models-mode] "${DEFAULT_VARIANT}" is reserved; use plain Save.`);
        }
      }
      // 'empty' is silently ignored (matches the left-pane button guard).
      return;
    }
    if (verdict.requiresConfirm) {
      const ok = (typeof confirm === 'function')
        ? confirm(`Variant "${verdict.variant}" already exists. Overwrite it?`)
        : true;
      if (!ok) return;
    }
    name = verdict.variant;

    await writeRig(model.stem, name);
    // Register the new variant and switch to it.
    if (!model.variants.includes(name)) {
      model.variants = [...model.variants, name].sort((a, b) => {
        if (a === DEFAULT_VARIANT) return -1;
        if (b === DEFAULT_VARIANT) return 1;
        return a.localeCompare(b);
      });
    }
    activeVariant = name;
    modeShell.setActiveFile(MODE, sidecarPath(model.stem, name));
    refreshOpenFiles();
    renderAll();
  }

  async function writeRig(stem, variant) {
    // Refresh cached extents from the live scene before serializing.
    if (scene) rig.extents = computeExtents(scene.getExtents());
    const text = buildRigToml(rig);
    const path = sidecarPath(stem, variant);
    try {
      await writeFile(path, text);
    } catch (err) {
      console.warn(`[models-mode] save failed for ${path}:`, err?.message || err);
      return;
    }
    modeShell.markDirty(MODE, path, false);
    refreshUnsavedIndicator();
    refreshOpenFiles();
    renderLeft();
    renderCenter();
  }

  // ── Discovery ─────────────────────────────────────────────────────────
  function refreshOpenFiles() {
    // Register every known sidecar path (existing + default) as an open file
    // so modeShell's dirty tracking + guard see them.
    const paths = [];
    for (const m of models) {
      const variants = m.variants.length ? m.variants : [DEFAULT_VARIANT];
      for (const v of variants) paths.push(sidecarPath(m.stem, v));
    }
    modeShell.setOpenFiles(MODE, paths);
  }

  async function discover() {
    let entries = [];
    try {
      entries = await listDirectory(MODELS_DIR);
    } catch (err) {
      console.warn('[models-mode] listDirectory failed:', err?.message || err);
      entries = [];
    }
    models = groupModelFiles(entries);
    refreshOpenFiles();
    renderAll();
  }

  // ── Project-root change: dispose the old scene + re-discover ──────────
  if (typeof onRootChanged === 'function') {
    rootChangeSub = onRootChanged(() => {
      // The previous root's GLB/sidecars are gone; rebuild from scratch.
      activeStem = null;
      activeVariant = null;
      selectedMarker = null;
      rig = defaultRig();
      disposeScene();
      discover().catch((err) => {
        console.warn('[models-mode] re-discover after root change failed:', err?.message || err);
      });
    });
  }

  // ── Bootstrap ─────────────────────────────────────────────────────────
  (async () => {
    await discover();
  })();

  return {
    render: renderAll,
    teardown,
    _internal: {
      discover,
      selectModel,
      selectVariant,
      saveCurrent,
      saveAsVariant,
      handleAddMarker,
      handleRemoveMarker,
      handleRenameMarker,
      disposeScene,
      teardown,
      getRig: () => rig,
      getModels: () => models,
      getScene: () => scene,
    },
  };
}

// ── tiny DOM helpers ──────────────────────────────────────────────────

function el(tag, className) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

function sectionTitle(text) {
  const h = el('div', 'models-section-title');
  h.textContent = text;
  return h;
}

function placeholder(text) {
  const p = el('p', 'placeholder');
  p.textContent = text;
  return p;
}

function dirtyDot() {
  return el('span', 'dirty-dot');
}

function vec3Row(label, value, onChange, step) {
  const row = el('div', 'models-form-row');
  const lab = el('span', 'models-form-label');
  lab.textContent = label;
  row.appendChild(lab);
  const inputs = [];
  for (let i = 0; i < 3; i++) {
    const input = el('input', 'models-form-input');
    input.type = 'number';
    input.step = String(step);
    input.value = String(value[i]);
    input.addEventListener('change', () => {
      const v = inputs.map((inp) => {
        const n = Number(inp.value);
        return Number.isFinite(n) ? n : 0;
      });
      onChange(v);
    });
    inputs.push(input);
    row.appendChild(input);
  }
  return row;
}
