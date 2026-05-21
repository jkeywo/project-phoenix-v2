/**
 * complexity-form-view.js
 *
 * Centre pane of the Definitions Mode → Complexity section. Renders, for
 * one open complexity TOML, all of its presets and the per-preset blocks:
 *
 *   Preset tabs  — click switches `activePresetIndex`.
 *   hidden_elements multi-select — options = knownUiElements ∪ authored
 *                                   hidden_elements (dedup) so unknown
 *                                   authored values stay visible.
 *   delegated    — table of rows { consoleKey: dropdown, controlsCsv }.
 *   ai           — one collapsible block per behaviorKey with typed inputs
 *                  derived from `typeof value`.
 *
 * Callbacks (all optional; missing ones are no-ops):
 *   callbacks.onSwitchPreset(presetIndex)
 *   callbacks.onSetHiddenElements(presetIndex, string[])
 *   callbacks.onSetDelegated(presetIndex, consoleKey, controls: string[])
 *   callbacks.onRemoveDelegated(presetIndex, consoleKey)
 *   callbacks.onSetAiParam(presetIndex, behaviorKey, paramKey, value)
 *   callbacks.onRemoveAiBlock(presetIndex, behaviorKey)
 *   callbacks.onAddDelegated(presetIndex)
 *   callbacks.onAddAiBlock(presetIndex)
 */

export const KNOWN_CONSOLE_KEYS = [
  'Tactical',
  'Helm',
  'Repair',
  'Power',
  'Sensors',
  'Shields',
  'Navigation',
  'Comms',
  'CaptainChair',
];

export function renderComplexityFormView(host, opts) {
  if (!host) return;
  host.innerHTML = '';

  const {
    presets,
    knownUiElements = [],
    activePresetIndex = 0,
    callbacks = {},
  } = opts || {};

  if (!Array.isArray(presets) || presets.length === 0) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'Select a complexity file from the list.';
    host.appendChild(p);
    return;
  }

  const idx = Math.max(0, Math.min(activePresetIndex, presets.length - 1));
  const active = presets[idx];

  // ── Preset tabs ───────────────────────────────────────────────────────
  const tabs = document.createElement('div');
  tabs.className = 'def-preset-tabs';
  host.appendChild(tabs);
  presets.forEach((preset, i) => {
    const btn = document.createElement('button');
    btn.className = 'def-preset-tab';
    if (i === idx) btn.classList.add('def-preset-tab-active');
    btn.dataset.presetIndex = String(i);
    btn.textContent = preset.name || `(preset ${i})`;
    btn.addEventListener('click', () => {
      if (callbacks.onSwitchPreset) callbacks.onSwitchPreset(i);
    });
    tabs.appendChild(btn);
  });

  // ── hidden_elements multi-select ──────────────────────────────────────
  const hiddenSection = document.createElement('section');
  hiddenSection.className = 'def-form-section def-hidden-section';
  host.appendChild(hiddenSection);

  const hiddenLabel = document.createElement('label');
  hiddenLabel.textContent = 'hidden_elements';
  hiddenSection.appendChild(hiddenLabel);

  const hiddenSelect = document.createElement('select');
  hiddenSelect.multiple = true;
  hiddenSelect.className = 'def-multi-select def-hidden-elements-select';

  // Dedup: known UI elements ∪ authored values. Order: known first, then
  // any extras the file has but our vocab doesn't.
  const knownSet = new Set(knownUiElements);
  const authored = Array.isArray(active.hidden_elements) ? active.hidden_elements : [];
  const merged = [...knownUiElements];
  for (const v of authored) {
    if (!knownSet.has(v)) merged.push(v);
  }
  const authoredSet = new Set(authored);
  for (const elName of merged) {
    const opt = document.createElement('option');
    opt.value = elName;
    opt.textContent = elName;
    opt.selected = authoredSet.has(elName);
    hiddenSelect.appendChild(opt);
  }
  hiddenSelect.addEventListener('change', () => {
    const values = [];
    for (const child of hiddenSelect.children) {
      if (child && child.selected) values.push(child.value);
    }
    if (callbacks.onSetHiddenElements) callbacks.onSetHiddenElements(idx, values);
  });
  hiddenSection.appendChild(hiddenSelect);

  // ── Delegated table ───────────────────────────────────────────────────
  const delegatedSection = document.createElement('section');
  delegatedSection.className = 'def-form-section def-delegated-section';
  host.appendChild(delegatedSection);

  const delegatedHeader = document.createElement('div');
  delegatedHeader.className = 'def-form-section-header';
  delegatedHeader.textContent = 'delegated';
  delegatedSection.appendChild(delegatedHeader);

  const table = document.createElement('div');
  table.className = 'def-delegated-table';
  delegatedSection.appendChild(table);

  const delegatedEntries = active.delegated && typeof active.delegated === 'object'
    ? Object.entries(active.delegated)
    : [];
  for (const [consoleKey, val] of delegatedEntries) {
    const row = document.createElement('div');
    row.className = 'def-delegated-row';
    row.dataset.consoleKey = consoleKey;
    table.appendChild(row);

    // Console-key dropdown (with free-text fallback if value is unknown).
    const keySelect = document.createElement('select');
    keySelect.className = 'def-delegated-console';
    const keyOptions = KNOWN_CONSOLE_KEYS.includes(consoleKey)
      ? KNOWN_CONSOLE_KEYS
      : [...KNOWN_CONSOLE_KEYS, consoleKey];
    for (const k of keyOptions) {
      const opt = document.createElement('option');
      opt.value = k;
      opt.textContent = k;
      opt.selected = k === consoleKey;
      keySelect.appendChild(opt);
    }
    keySelect.addEventListener('change', (e) => {
      const newKey = e.target.value;
      if (newKey === consoleKey) return;
      // Rename: remove the old key, add the new one with the same controls.
      const controls = (val && Array.isArray(val.controls)) ? [...val.controls] : [];
      if (callbacks.onRemoveDelegated) callbacks.onRemoveDelegated(idx, consoleKey);
      if (callbacks.onSetDelegated) callbacks.onSetDelegated(idx, newKey, controls);
    });
    row.appendChild(keySelect);

    // Controls — CSV input
    const controlsInput = document.createElement('input');
    controlsInput.type = 'text';
    controlsInput.className = 'def-delegated-controls';
    controlsInput.value = Array.isArray(val?.controls) ? val.controls.join(', ') : '';
    controlsInput.placeholder = 'comma-separated control names';
    controlsInput.addEventListener('input', (e) => {
      const csv = e.target.value;
      const list = csv.split(',').map((s) => s.trim()).filter(Boolean);
      if (callbacks.onSetDelegated) callbacks.onSetDelegated(idx, consoleKey, list);
    });
    row.appendChild(controlsInput);

    const removeBtn = document.createElement('button');
    removeBtn.className = 'def-delegated-remove';
    removeBtn.textContent = 'Remove';
    removeBtn.addEventListener('click', () => {
      if (callbacks.onRemoveDelegated) callbacks.onRemoveDelegated(idx, consoleKey);
    });
    row.appendChild(removeBtn);
  }

  const addDelegated = document.createElement('button');
  addDelegated.className = 'def-delegated-add';
  addDelegated.textContent = '+ Add delegated';
  addDelegated.addEventListener('click', () => {
    if (callbacks.onAddDelegated) callbacks.onAddDelegated(idx);
  });
  delegatedSection.appendChild(addDelegated);

  // ── AI tuning blocks ──────────────────────────────────────────────────
  const aiSection = document.createElement('section');
  aiSection.className = 'def-form-section def-ai-section';
  host.appendChild(aiSection);

  const aiHeader = document.createElement('div');
  aiHeader.className = 'def-form-section-header';
  aiHeader.textContent = 'ai';
  aiSection.appendChild(aiHeader);

  const aiEntries = active.ai && typeof active.ai === 'object'
    ? Object.entries(active.ai)
    : [];
  for (const [behaviorKey, params] of aiEntries) {
    const block = document.createElement('div');
    block.className = 'def-ai-block';
    block.dataset.behaviorKey = behaviorKey;
    aiSection.appendChild(block);

    const blockHeader = document.createElement('div');
    blockHeader.className = 'def-ai-block-header';
    blockHeader.textContent = behaviorKey;
    block.appendChild(blockHeader);

    const paramEntries = params && typeof params === 'object'
      ? Object.entries(params)
      : [];
    for (const [paramKey, value] of paramEntries) {
      const paramRow = document.createElement('div');
      paramRow.className = 'def-ai-param-row';
      paramRow.dataset.paramKey = paramKey;
      block.appendChild(paramRow);

      const paramLabel = document.createElement('label');
      paramLabel.textContent = paramKey;
      paramRow.appendChild(paramLabel);

      const input = document.createElement('input');
      const t = typeof value;
      if (t === 'number') {
        input.type = 'number';
        input.step = '0.1';
        input.value = String(value);
        input.addEventListener('input', (e) => {
          const n = Number(e.target.value);
          if (callbacks.onSetAiParam) {
            callbacks.onSetAiParam(idx, behaviorKey, paramKey, Number.isFinite(n) ? n : 0);
          }
        });
      } else if (t === 'boolean') {
        input.type = 'checkbox';
        input.checked = !!value;
        input.addEventListener('change', (e) => {
          if (callbacks.onSetAiParam) {
            callbacks.onSetAiParam(idx, behaviorKey, paramKey, !!e.target.checked);
          }
        });
      } else {
        input.type = 'text';
        input.value = value == null ? '' : String(value);
        input.addEventListener('input', (e) => {
          if (callbacks.onSetAiParam) {
            callbacks.onSetAiParam(idx, behaviorKey, paramKey, e.target.value);
          }
        });
      }
      input.className = 'def-ai-param-input';
      paramRow.appendChild(input);
    }

    const removeBlock = document.createElement('button');
    removeBlock.className = 'def-ai-remove';
    removeBlock.textContent = 'Remove block';
    removeBlock.addEventListener('click', () => {
      if (callbacks.onRemoveAiBlock) callbacks.onRemoveAiBlock(idx, behaviorKey);
    });
    block.appendChild(removeBlock);
  }

  const addAi = document.createElement('button');
  addAi.className = 'def-ai-add';
  addAi.textContent = '+ Add AI block';
  addAi.addEventListener('click', () => {
    if (callbacks.onAddAiBlock) callbacks.onAddAiBlock(idx);
  });
  aiSection.appendChild(addAi);
}
