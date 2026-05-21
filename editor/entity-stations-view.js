/**
 * entity-stations-view.js
 *
 * DOM renderer for the [stations] component card.
 *
 * Wraps `StationsEditor`. Per-player-count tab strip; the active tab shows
 * an editable list of station rows (description / consoles / rank /
 * short_code / next / previous). `name` is read-only. Inline validation
 * banner at the top of each tab and per-station error chip next to the
 * offending field.
 *
 * `onEdit(stationsData)` is called with `editor.toStations()` after every
 * mutation.
 */

import { StationsEditor } from './stations-editor.js';

let _activeCount = null; // module-level (single editor active at a time)

export function renderEntityStationsView(host, stations, { onEdit }) {
  if (!host) return;
  host.innerHTML = '';

  const editor = new StationsEditor();
  editor.load(stations || {});

  const counts = editor.getCounts();
  if (_activeCount == null || !counts.includes(_activeCount)) {
    _activeCount = counts[0] ?? null;
  }

  const rerender = () => renderEntityStationsView(host, editor.toStations(), { onEdit });

  const mutate = (fn) => {
    fn(editor);
    onEdit(editor.toStations());
    rerender();
  };

  const root = document.createElement('div');
  root.className = 'entity-stations';
  host.appendChild(root);

  // ── Player count range ────────────────────────────────────────────────
  const rangeRow = document.createElement('div');
  rangeRow.className = 'entity-stations-range';

  const minLbl = document.createElement('label');
  minLbl.textContent = 'min_players';
  rangeRow.appendChild(minLbl);
  const minInp = document.createElement('input');
  minInp.type = 'number';
  minInp.value = String(editor.getMinPlayers());
  minInp.addEventListener('input', (e) => {
    const n = Number(e.target.value);
    if (Number.isInteger(n) && n >= 1) {
      mutate(() => { editor._minPlayers = n; });
    }
  });
  rangeRow.appendChild(minInp);

  const maxLbl = document.createElement('label');
  maxLbl.textContent = 'max_players';
  rangeRow.appendChild(maxLbl);
  const maxInp = document.createElement('input');
  maxInp.type = 'number';
  maxInp.value = String(editor.getMaxPlayers());
  maxInp.addEventListener('input', (e) => {
    const n = Number(e.target.value);
    if (Number.isInteger(n) && n >= editor.getMinPlayers()) {
      mutate(() => { editor._maxPlayers = n; });
    }
  });
  rangeRow.appendChild(maxInp);

  root.appendChild(rangeRow);

  // ── Tab strip ─────────────────────────────────────────────────────────
  const tabs = document.createElement('div');
  tabs.className = 'entity-stations-tabs';
  for (const c of counts) {
    const tab = document.createElement('button');
    tab.type = 'button';
    tab.className = 'entity-stations-tab';
    if (c === _activeCount) tab.classList.add('entity-stations-tab-active');
    tab.textContent = String(c);
    tab.addEventListener('click', () => {
      _activeCount = c;
      rerender();
    });
    tabs.appendChild(tab);
  }
  root.appendChild(tabs);

  // ── Active tab content ────────────────────────────────────────────────
  const tabBody = document.createElement('div');
  tabBody.className = 'entity-stations-tab-body';
  root.appendChild(tabBody);

  if (_activeCount == null) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'No player counts configured.';
    tabBody.appendChild(p);
    return;
  }

  // Validation. Filter to errors that mention this count.
  const validation = editor.validate();
  const tabErrors = (validation.errors || []).filter((err) => {
    if (err.count == null) return false;
    return Number(err.count) === _activeCount;
  });
  if (tabErrors.length > 0) {
    const errList = document.createElement('div');
    errList.className = 'entity-stations-error-list';
    for (const err of tabErrors) {
      const chip = document.createElement('div');
      chip.className = 'entity-stations-error';
      chip.dataset.kind = err.type || '';
      chip.textContent = err.message || `${err.type}: ${err.station || ''}`;
      errList.appendChild(chip);
    }
    tabBody.appendChild(errList);
  }

  // Station rows.
  const stationsList = editor.getStations(_activeCount);
  for (const s of stationsList) {
    tabBody.appendChild(renderStationRow(s, _activeCount, editor, tabErrors, mutate));
  }

  // + Add Station.
  const addBtn = document.createElement('button');
  addBtn.type = 'button';
  addBtn.className = 'entity-stations-add';
  addBtn.textContent = '+ Add Station';
  addBtn.addEventListener('click', () => {
    mutate((ed) => {
      const baseName = `station_${ed.getStations(_activeCount).length + 1}`;
      ed.addStation(_activeCount, baseName, [], '', '', '');
    });
  });
  tabBody.appendChild(addBtn);
}

function renderStationRow(s, count, editor, tabErrors, mutate) {
  const row = document.createElement('div');
  row.className = 'entity-stations-row';
  row.dataset.station = s.name;

  appendField(row, 'name', s.name, true, null);
  appendField(row, 'description', s.description, false, (v) => {
    mutate((ed) => ed.updateStation(count, s.name, { description: v }));
  });
  appendField(row, 'consoles', (s.consoles || []).join(', '), false, (v) => {
    const list = v.split(',').map((p) => p.trim()).filter(Boolean);
    mutate((ed) => ed.updateStation(count, s.name, { consoles: list }));
  });
  appendField(row, 'rank', s.rank, false, (v) => {
    mutate((ed) => ed.updateStation(count, s.name, { rank: v }));
  });
  appendField(row, 'short_code', s.short_code, false, (v) => {
    mutate((ed) => ed.updateStation(count, s.name, { short_code: v }));
  });

  // next dropdown.
  const nextSel = makeStationDropdown(
    'next',
    s.next || '',
    editor.getNextOptions(count),
    (v) => mutate((ed) => ed.updateStation(count, s.name, { next: v })),
  );
  row.appendChild(nextSel.wrap);
  const nextErr = tabErrors.find((e) => e.station === s.name && e.type && e.type.includes('next'));
  if (nextErr) attachInlineError(nextSel.wrap, nextErr);

  // previous dropdown.
  const prevSel = makeStationDropdown(
    'previous',
    s.previous || '',
    editor.getPreviousOptions(count),
    (v) => mutate((ed) => ed.updateStation(count, s.name, { previous: v })),
  );
  row.appendChild(prevSel.wrap);
  const prevErr = tabErrors.find((e) => e.station === s.name && e.type && e.type.includes('previous'));
  if (prevErr) attachInlineError(prevSel.wrap, prevErr);

  // remove.
  const removeBtn = document.createElement('button');
  removeBtn.type = 'button';
  removeBtn.className = 'entity-stations-row-remove';
  removeBtn.textContent = '✕';
  removeBtn.addEventListener('click', () => {
    mutate((ed) => ed.removeStation(count, s.name));
  });
  row.appendChild(removeBtn);

  return row;
}

function appendField(row, key, value, disabled, onChange) {
  const wrap = document.createElement('div');
  wrap.className = `entity-stations-field entity-stations-field-${key}`;
  const lbl = document.createElement('label');
  lbl.textContent = key;
  wrap.appendChild(lbl);
  const inp = document.createElement('input');
  inp.type = 'text';
  inp.value = value != null ? String(value) : '';
  if (disabled) inp.disabled = true;
  if (!disabled && onChange) {
    inp.addEventListener('input', (e) => onChange(e.target.value));
  }
  wrap.appendChild(inp);
  row.appendChild(wrap);
}

function makeStationDropdown(key, value, options, onChange) {
  const wrap = document.createElement('div');
  wrap.className = `entity-stations-field entity-stations-field-${key}`;
  const lbl = document.createElement('label');
  lbl.textContent = key;
  wrap.appendChild(lbl);

  const sel = document.createElement('select');
  sel.className = `entity-stations-${key}`;
  const blank = document.createElement('option');
  blank.value = '';
  blank.textContent = '(none)';
  sel.appendChild(blank);
  for (const name of options) {
    const o = document.createElement('option');
    o.value = name;
    o.textContent = name;
    sel.appendChild(o);
  }
  // If current value isn't in options, add it as a "dangling" sentinel so
  // the user can see what's set even when it doesn't resolve.
  if (value && !options.includes(value)) {
    const o = document.createElement('option');
    o.value = value;
    o.textContent = `${value} (dangling)`;
    sel.appendChild(o);
  }
  sel.value = value || '';
  sel.addEventListener('change', (e) => onChange(e.target.value));
  wrap.appendChild(sel);
  return { wrap, sel };
}

function attachInlineError(wrap, err) {
  // Slice 7: dangling-next / dangling-previous / missing-* are
  // recoverable references (the user can still save, the runtime falls
  // back), so we surface them as yellow warning badges. Hard structural
  // errors (duplicate-name, empty-consoles, etc.) stay red.
  const type = err?.type || '';
  const severity = /dangling|missing/i.test(type) ? 'warning' : 'error';

  const chip = document.createElement('span');
  // Keep the legacy class (existing CSS + Slice 5 tests depend on it)
  // and ADDITIVELY stamp the validation-badge classes so the new
  // colour layer takes effect.
  chip.className = `entity-stations-inline-error validation-badge validation-badge-${severity}`;
  chip.dataset.kind = type;
  chip.title = err?.message || type || 'error';
  chip.textContent = type || 'error';
  wrap.appendChild(chip);
}

/** Test hook: reset active tab between renderings. */
export function _resetActiveCountForTest() {
  _activeCount = null;
}
