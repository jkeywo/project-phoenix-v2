/**
 * definitions-mode-view.js
 *
 * Top-level coordinator for the Definitions Mode two-section shell.
 *
 *   Top section    — Factions: left file list (assets/factions/*.toml)
 *                              + centre form (faction-form-view).
 *   Bottom section — Complexity presets: left file list
 *                              (assets/complexity/*.toml) + centre form
 *                              (complexity-form-view).
 *
 * Both sections share one `'Definitions'` ModeShell key; the kind
 * distinction is encoded in the path prefix (`assets/factions/` vs
 * `assets/complexity/`) and round-trips through the SaveFlow content
 * cache as `{ kind: 'faction' | 'complexity', data }`.
 *
 * Wires:
 *   - file-list click → io.readFile → editor.openFile → re-render section
 *   - form edit → snapshot-before-mutate → editor mutator →
 *       saveFlow.setContent('Definitions', path, {kind, data}) →
 *       modeShell.markDirty → re-render section
 *   - registerRestore('Definitions', …) for Ctrl+Z restore
 *   - InvalidationBus.fireFactionSaved fires from save-flow itself when
 *     a faction file saves; Entity Mode subscribes to refresh its dropdown.
 */

import { FactionEditor } from './faction-editor.js';
import { ComplexityEditor } from './complexity-editor.js';
import { renderDefinitionsFileListView } from './definitions-file-list-view.js';
import { renderFactionFormView } from './faction-form-view.js';
import { renderComplexityFormView } from './complexity-form-view.js';
import { readFile, listDirectory, getProjectRoot } from './project-root.js';

const FACTIONS_DIR = 'assets/factions';
const COMPLEXITY_DIR = 'assets/complexity';

export function mountDefinitionsMode({ host, modeShell, saveFlow, registerRestore, invalidationBus, io } = {}) {
  if (!host) return null;

  const ioDeps = {
    readFile: io?.readFile || readFile,
    listDirectory: io?.listDirectory || listDirectory,
    getProjectRoot: io?.getProjectRoot || getProjectRoot,
  };

  const factionEditor = new FactionEditor();
  const complexityEditor = new ComplexityEditor();

  // Complexity centre state (active preset index per file). Faction
  // editor is single-screen so no equivalent index needed.
  let activePresetIndex = 0;

  // Cache the raw text for each file so re-opening after edits stays
  // consistent with what the editor parsed.
  const factionRawCache = new Map();
  const complexityRawCache = new Map();

  // ── Skeleton ──────────────────────────────────────────────────────────
  host.innerHTML = '';
  const wrap = document.createElement('div');
  wrap.className = 'definitions-two-section';
  host.appendChild(wrap);

  // Faction section
  const factionSection = document.createElement('section');
  factionSection.className = 'definitions-section definitions-section-faction';
  wrap.appendChild(factionSection);

  const factionHeader = document.createElement('div');
  factionHeader.className = 'definitions-section-header';
  factionHeader.textContent = 'Factions';
  factionSection.appendChild(factionHeader);

  const factionBody = document.createElement('div');
  factionBody.className = 'definitions-section-body';
  factionSection.appendChild(factionBody);

  const factionLeft = document.createElement('div');
  factionLeft.className = 'def-pane def-pane-left';
  factionBody.appendChild(factionLeft);

  const factionCenter = document.createElement('div');
  factionCenter.className = 'def-pane def-pane-center';
  factionBody.appendChild(factionCenter);

  // Complexity section
  const complexitySection = document.createElement('section');
  complexitySection.className = 'definitions-section definitions-section-complexity';
  wrap.appendChild(complexitySection);

  const complexityHeader = document.createElement('div');
  complexityHeader.className = 'definitions-section-header';
  complexityHeader.textContent = 'Complexity Presets';
  complexitySection.appendChild(complexityHeader);

  const complexityBody = document.createElement('div');
  complexityBody.className = 'definitions-section-body';
  complexitySection.appendChild(complexityBody);

  const complexityLeft = document.createElement('div');
  complexityLeft.className = 'def-pane def-pane-left';
  complexityBody.appendChild(complexityLeft);

  const complexityCenter = document.createElement('div');
  complexityCenter.className = 'def-pane def-pane-center';
  complexityBody.appendChild(complexityCenter);

  // ── Render helpers ────────────────────────────────────────────────────

  function renderFactionLeft() {
    renderDefinitionsFileListView(factionLeft, {
      paths: factionEditor.getFileList(),
      activePath: factionEditor.getActiveFile(),
      modeShell,
      mode: 'Definitions',
      onSelect: (p) => loadFactionFile(p),
    });
  }

  function renderFactionCenter() {
    const formState = factionEditor.getFormState();
    renderFactionFormView(factionCenter, {
      formState,
      enemyOptions: factionEditor.getEnemyOptions(),
      onNameChange: (name) => handleFactionNameChange(name),
      onEnemiesChange: (uuids) => handleFactionEnemiesChange(uuids),
    });
  }

  function renderComplexityLeft() {
    renderDefinitionsFileListView(complexityLeft, {
      paths: complexityEditor.getFileList(),
      activePath: complexityEditor.getActiveFile(),
      modeShell,
      mode: 'Definitions',
      onSelect: (p) => loadComplexityFile(p),
    });
  }

  function renderComplexityCenter() {
    const presets = complexityEditor.getPresets();
    renderComplexityFormView(complexityCenter, {
      presets: presets || [],
      knownUiElements: complexityEditor.getKnownUiElements(),
      activePresetIndex,
      callbacks: {
        onSwitchPreset: (i) => {
          activePresetIndex = i;
          renderComplexityCenter();
        },
        onSetHiddenElements: (i, list) => handleComplexityMutation(() => {
          complexityEditor.setHiddenElements(i, list);
        }),
        onSetDelegated: (i, key, controls) => handleComplexityMutation(() => {
          complexityEditor.setDelegated(i, key, controls);
        }),
        onRemoveDelegated: (i, key) => handleComplexityMutation(() => {
          complexityEditor.removeDelegated(i, key);
        }),
        onSetAiParam: (i, b, k, v) => handleComplexityMutation(() => {
          complexityEditor.setAiParam(i, b, k, v);
        }),
        onRemoveAiBlock: (i, b) => handleComplexityMutation(() => {
          complexityEditor.removeAiBlock(i, b);
        }),
        onAddDelegated: (i) => handleComplexityMutation(() => {
          // Pick the first known console not yet present in the table.
          const existing = new Set(Object.keys(complexityEditor.getPreset(i)?.delegated || {}));
          const fallback = ['Tactical', 'Helm', 'Repair', 'Power', 'Sensors',
            'Shields', 'Navigation', 'Comms', 'CaptainChair']
            .find((k) => !existing.has(k)) || 'Tactical';
          complexityEditor.setDelegated(i, fallback, []);
        }),
        onAddAiBlock: (i) => handleComplexityMutation(() => {
          // Generate a stable placeholder behavior key.
          let n = 1;
          const existing = new Set(Object.keys(complexityEditor.getPreset(i)?.ai || {}));
          while (existing.has(`new_behavior_${n}`)) n += 1;
          complexityEditor.setAiBlock(i, `new_behavior_${n}`, {});
        }),
      },
    });
  }

  function renderAll() {
    renderFactionLeft();
    renderFactionCenter();
    renderComplexityLeft();
    renderComplexityCenter();
  }

  // ── Mutation helpers ──────────────────────────────────────────────────

  function snapshotFaction(path) {
    const formState = factionEditor.getFormState();
    if (!formState) return;
    modeShell.pushUndoEntry('Definitions', path, structuredClone({
      kind: 'faction',
      data: formState,
    }));
  }

  function commitFaction(path) {
    const data = factionEditor.getFormState();
    saveFlow.setContent('Definitions', path, { kind: 'faction', data });
    modeShell.markDirty('Definitions', path, true);
    // Re-render both panes (left for dirty-dot, centre for current values).
    renderFactionLeft();
    renderFactionCenter();
  }

  function handleFactionNameChange(name) {
    const path = factionEditor.getActiveFile();
    if (!path) return;
    snapshotFaction(path);
    factionEditor.setName(name);
    commitFaction(path);
  }

  function handleFactionEnemiesChange(uuids) {
    const path = factionEditor.getActiveFile();
    if (!path) return;
    snapshotFaction(path);
    factionEditor.setEnemies(uuids);
    commitFaction(path);
  }

  function snapshotComplexity(path) {
    const presets = complexityEditor.getPresets();
    if (!presets) return;
    modeShell.pushUndoEntry('Definitions', path, structuredClone({
      kind: 'complexity',
      data: presets,
    }));
  }

  function handleComplexityMutation(mutator) {
    const path = complexityEditor.getActiveFile();
    if (!path) return;
    snapshotComplexity(path);
    mutator();
    const data = complexityEditor.getPresets();
    saveFlow.setContent('Definitions', path, { kind: 'complexity', data });
    modeShell.markDirty('Definitions', path, true);
    renderComplexityLeft();
    renderComplexityCenter();
  }

  // ── File load ────────────────────────────────────────────────────────

  async function loadFactionFile(path) {
    let text = factionRawCache.get(path);
    if (text === undefined) {
      try {
        text = await ioDeps.readFile(path);
        factionRawCache.set(path, text);
      } catch {
        return;
      }
    }
    if (!factionEditor.openFile(path)) {
      // Editor's loadAll may not include this path yet (e.g. new file).
      // Append a single-file load on the fly.
      factionEditor.loadAll([
        ...factionEditor.getFileList().map((p) => ({
          path: p,
          content: factionRawCache.get(p),
        })),
        { path, content: text },
      ]);
      factionEditor.openFile(path);
    }
    modeShell.setActiveFile('Definitions', path);
    const formState = factionEditor.getFormState();
    if (formState) {
      saveFlow.setContent('Definitions', path, { kind: 'faction', data: formState });
    }
    renderFactionLeft();
    renderFactionCenter();
  }

  async function loadComplexityFile(path) {
    let text = complexityRawCache.get(path);
    if (text === undefined) {
      try {
        text = await ioDeps.readFile(path);
        complexityRawCache.set(path, text);
      } catch {
        return;
      }
    }
    if (!complexityEditor.openFile(path)) {
      complexityEditor.loadAll([
        ...complexityEditor.getFileList().map((p) => ({
          path: p,
          content: complexityRawCache.get(p),
        })),
        { path, content: text },
      ]);
      complexityEditor.openFile(path);
    }
    activePresetIndex = 0;
    modeShell.setActiveFile('Definitions', path);
    const presets = complexityEditor.getPresets();
    if (presets) {
      saveFlow.setContent('Definitions', path, { kind: 'complexity', data: presets });
    }
    renderComplexityLeft();
    renderComplexityCenter();
  }

  // ── File-list bootstrap ──────────────────────────────────────────────

  async function refreshFactionList() {
    let entries = [];
    try {
      entries = await ioDeps.listDirectory(FACTIONS_DIR);
    } catch {
      entries = [];
    }
    const paths = (entries || [])
      .filter((e) => e && e.kind === 'file' && typeof e.name === 'string' && e.name.endsWith('.toml'))
      .map((e) => `${FACTIONS_DIR}/${e.name}`)
      .sort();

    const loaded = [];
    for (const p of paths) {
      try {
        const content = await ioDeps.readFile(p);
        factionRawCache.set(p, content);
        loaded.push({ path: p, content });
      } catch {
        // Skip files that fail to read.
      }
    }
    factionEditor.loadAll(loaded);

    // Reflect into ModeShell's open-files registry so SaveAll picks these up.
    const currentOpen = modeShell.getOpenFiles('Definitions') || [];
    const filtered = currentOpen.filter((p) => !p.startsWith(`${FACTIONS_DIR}/`));
    modeShell.setOpenFiles('Definitions', [...filtered, ...paths]);
    renderFactionLeft();
  }

  async function refreshComplexityList() {
    let entries = [];
    try {
      entries = await ioDeps.listDirectory(COMPLEXITY_DIR);
    } catch {
      entries = [];
    }
    const paths = (entries || [])
      .filter((e) => e && e.kind === 'file' && typeof e.name === 'string' && e.name.endsWith('.toml'))
      .map((e) => `${COMPLEXITY_DIR}/${e.name}`)
      .sort();

    const loaded = [];
    for (const p of paths) {
      try {
        const content = await ioDeps.readFile(p);
        complexityRawCache.set(p, content);
        loaded.push({ path: p, content });
      } catch {
        // Skip
      }
    }
    complexityEditor.loadAll(loaded);

    const currentOpen = modeShell.getOpenFiles('Definitions') || [];
    const filtered = currentOpen.filter((p) => !p.startsWith(`${COMPLEXITY_DIR}/`));
    modeShell.setOpenFiles('Definitions', [...filtered, ...paths]);
    renderComplexityLeft();
  }

  // ── Undo restore registration ────────────────────────────────────────

  if (typeof registerRestore === 'function') {
    registerRestore('Definitions', (ms, path, direction) => {
      // Determine which editor owns the path.
      const isFaction = typeof path === 'string' && path.startsWith(`${FACTIONS_DIR}/`);
      const isComplexity = typeof path === 'string' && path.startsWith(`${COMPLEXITY_DIR}/`);
      if (!isFaction && !isComplexity) return;

      // Capture current state in the wrapped { kind, data } shape so the
      // opposite stack can replay it later.
      let current;
      if (isFaction) {
        const fs = factionEditor.getFormState();
        if (!fs) return;
        current = structuredClone({ kind: 'faction', data: fs });
      } else {
        const ps = complexityEditor.getPresets();
        if (!ps) return;
        current = structuredClone({ kind: 'complexity', data: ps });
      }

      const snap = direction === 'undo'
        ? ms.swapUndoActive('Definitions', path, current)
        : ms.swapRedoActive('Definitions', path, current);
      if (!snap) return;

      if (isFaction && snap.kind === 'faction') {
        factionEditor.setName(snap.data.name);
        factionEditor.setEnemies(snap.data.enemies || []);
        saveFlow.setContent('Definitions', path, { kind: 'faction', data: snap.data });
        renderFactionLeft();
        renderFactionCenter();
      } else if (isComplexity && snap.kind === 'complexity') {
        // Replay every preset block. ComplexityEditor doesn't expose a
        // wholesale setter, so reach in via the per-preset mutators.
        const presets = snap.data || [];
        for (let i = 0; i < presets.length; i += 1) {
          complexityEditor.setHiddenElements(i, presets[i].hidden_elements || []);
          // Reset the delegated map: remove all existing keys first, then
          // re-apply the snapshot's.
          const existing = Object.keys(complexityEditor.getPreset(i)?.delegated || {});
          for (const k of existing) complexityEditor.removeDelegated(i, k);
          for (const [k, v] of Object.entries(presets[i].delegated || {})) {
            complexityEditor.setDelegated(i, k, v.controls || []);
          }
          const aiExisting = Object.keys(complexityEditor.getPreset(i)?.ai || {});
          for (const k of aiExisting) complexityEditor.removeAiBlock(i, k);
          for (const [k, v] of Object.entries(presets[i].ai || {})) {
            complexityEditor.setAiBlock(i, k, v);
          }
        }
        saveFlow.setContent('Definitions', path, { kind: 'complexity', data: presets });
        renderComplexityLeft();
        renderComplexityCenter();
      }
    });
  }

  // ── Bootstrap ────────────────────────────────────────────────────────

  let bootstrapPromise = null;
  function bootstrap() {
    if (bootstrapPromise) return bootstrapPromise;
    bootstrapPromise = (async () => {
      try {
        const root = await ioDeps.getProjectRoot();
        if (!root) {
          factionLeft.innerHTML = '<p class="placeholder">Pick a project root to load definition files.</p>';
          complexityLeft.innerHTML = '<p class="placeholder">Pick a project root to load definition files.</p>';
          return;
        }
      } catch {
        // best-effort; carry on
      }
      await refreshFactionList();
      await refreshComplexityList();
    })();
    return bootstrapPromise;
  }

  // Fire-and-forget. Tests await `_internal.bootstrap` directly to dedupe.
  bootstrap();

  return {
    factionEditor,
    complexityEditor,
    render: renderAll,
    _internal: {
      bootstrap,
      loadFactionFile,
      loadComplexityFile,
      refreshFactionList,
      refreshComplexityList,
      handleFactionNameChange,
      handleFactionEnemiesChange,
      handleComplexityMutation,
    },
  };
}
