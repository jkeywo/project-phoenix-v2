/**
 * entity-component-card-view.js
 *
 * Per-card DOM renderer for one TOML section in Entity Mode.
 *
 * Header: section label, collapse toggle, "raw TOML" toggle, delete (✕).
 * Body:
 *   - When `card.showRaw` is true: <textarea> bound to `getRawToml()` and
 *     parsed back on every input (a parse failure shows an inline error
 *     without writing back).
 *   - Otherwise: schema-driven inputs (string/number/boolean/array<…>),
 *     with the following special-cases:
 *       * `dropdownSource === 'factions'`  → <select> from getFactionDropdownOptions()
 *       * `dropdownSource === 'complexity'` → <select> from getComplexityPaths()
 *       * `section === 'behaviour'`        → delegates to entity-behaviour-view
 *       * `section === 'stations'`          → delegates to entity-stations-view
 *   - When `card.schema` is null: degrades to raw-TOML-only mode.
 *
 * Edits flow out via `onEdit(sectionKey, newData)`; the host is responsible
 * for snapshotting + calling shell.setSection.
 *
 * Pure DOM module: no globals beyond `document`.
 */

import { parseEntityToml, stringifyEntityToml } from './entity-toml.js';
import { renderEntityBehaviourView } from './entity-behaviour-view.js';
import { renderEntityStationsView } from './entity-stations-view.js';

/**
 * @param {HTMLElement} host
 * @param {import('./entity-mode.js').ComponentCard} card
 * @param {object} deps
 * @param {() => Array<{uuid:string,name:string}>} deps.getFactionOptions
 * @param {() => string[]} deps.getComplexityPaths
 * @param {(section:string, newData:any)=>void} deps.onEdit
 * @param {(section:string)=>void} [deps.onDelete]
 */
export function renderEntityComponentCard(host, card, deps) {
  if (!host) return;
  host.innerHTML = '';

  const root = document.createElement('div');
  root.className = 'entity-card';
  root.dataset.section = card.section;
  host.appendChild(root);

  // Header
  root.appendChild(renderHeader(card, root, deps));

  // Body
  const body = document.createElement('div');
  body.className = 'entity-card-body';
  if (card.collapsed) body.classList.add('hidden');
  root.appendChild(body);

  // Special-cased sections delegate entirely.
  if (card.section === 'behaviour' && !card.showRaw) {
    renderEntityBehaviourView(body, card.data, {
      onEdit: (newData) => deps.onEdit(card.section, newData),
    });
    return;
  }
  if (card.section === 'stations' && !card.showRaw) {
    renderEntityStationsView(body, card.data, {
      onEdit: (newData) => deps.onEdit(card.section, newData),
    });
    return;
  }

  // Raw OR no schema → textarea.
  if (card.showRaw || !card.schema) {
    renderRawTextarea(body, card, deps);
    return;
  }

  renderSchemaFields(body, card, deps);
}

// ── Header ─────────────────────────────────────────────────────────────

function renderHeader(card, root, deps) {
  const header = document.createElement('div');
  header.className = 'entity-card-header';

  const label = document.createElement('span');
  label.className = 'entity-card-label';
  label.textContent = card.schema?.label || card.section;
  header.appendChild(label);

  const collapseBtn = makeIconButton(card.collapsed ? '▸' : '▾', () => {
    card.toggle();
    // Re-render entirely (host owns the re-render flow on edits, but
    // toggle is a local UI concern; the easiest path is to re-render
    // this single card in-place).
    rerender(root, card, deps);
  }, 'collapse');
  header.appendChild(collapseBtn);

  const rawBtn = makeIconButton(card.showRaw ? 'fields' : 'raw', () => {
    card.toggleRaw();
    rerender(root, card, deps);
  }, 'raw-toggle');
  header.appendChild(rawBtn);

  if (typeof deps.onDelete === 'function') {
    const delBtn = makeIconButton('✕', () => deps.onDelete(card.section), 'delete');
    header.appendChild(delBtn);
  }

  return header;
}

function makeIconButton(text, fn, kind) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = `entity-card-btn entity-card-btn-${kind}`;
  b.textContent = text;
  b.addEventListener('click', fn);
  return b;
}

function rerender(root, card, deps) {
  const host = root.parentElement;
  if (!host) return;
  renderEntityComponentCard(host, card, deps);
}

// ── Raw textarea ───────────────────────────────────────────────────────

function renderRawTextarea(body, card, deps) {
  const wrap = document.createElement('div');
  wrap.className = 'entity-card-raw-wrap';
  body.appendChild(wrap);

  const ta = document.createElement('textarea');
  ta.className = 'entity-card-raw-textarea';
  ta.rows = 8;
  ta.value = card.getRawToml();
  wrap.appendChild(ta);

  const err = document.createElement('div');
  err.className = 'entity-card-raw-error';
  wrap.appendChild(err);

  ta.addEventListener('input', () => {
    try {
      const parsed = parseEntityToml(ta.value);
      if (!parsed || typeof parsed !== 'object') throw new Error('Section TOML must be an object.');
      const sectionData = parsed[card.section];
      if (sectionData === undefined) {
        err.textContent = `Expected a [${card.section}] section.`;
        return;
      }
      err.textContent = '';
      deps.onEdit(card.section, sectionData);
    } catch (e) {
      err.textContent = `Parse error: ${e.message}`;
    }
  });
}

// ── Schema-driven inputs ───────────────────────────────────────────────

function renderSchemaFields(body, card, deps) {
  const data = card.data;
  // Array-of-tables top-level section (e.g. light → [[light]]).
  if (card.schema?.arrayOfTables) {
    renderArrayOfTablesSection(body, card, deps);
    return;
  }
  // Top-level scalar (e.g. `faction = "uuid"`): the section name IS the key.
  if (data === null || typeof data !== 'object' || Array.isArray(data)) {
    renderScalarSection(body, card, deps);
    return;
  }

  for (const field of card.schema.fields) {
    const row = renderField(card, field, deps);
    if (row) body.appendChild(row);
  }
}

function renderArrayOfTablesSection(body, card, deps) {
  const entries = Array.isArray(card.data) ? card.data : [];
  const entryFields = card.schema.entryFields ?? [];
  const entryDefaults = card.schema.entryDefaults ?? {};

  entries.forEach((entry, idx) => {
    const entryWrap = document.createElement('div');
    entryWrap.className = 'entity-card-array-entry';

    const entryHeader = document.createElement('div');
    entryHeader.className = 'entity-card-array-entry-header';
    const tag = document.createElement('span');
    tag.textContent = `[[${card.section}]] #${idx + 1}`;
    entryHeader.appendChild(tag);
    const rmBtn = makeIconButton('✕', () => {
      const next = entries.slice();
      next.splice(idx, 1);
      deps.onEdit(card.section, next);
    }, 'delete');
    entryHeader.appendChild(rmBtn);
    entryWrap.appendChild(entryHeader);

    for (const field of entryFields) {
      const row = document.createElement('div');
      row.className = 'entity-card-field';
      const label = document.createElement('label');
      label.textContent = field.key;
      row.appendChild(label);
      const value = entry?.[field.key];
      const input = makeInputForField(field, value, deps, (newValue) => {
        const nextEntry = { ...entry };
        if (newValue === undefined || newValue === null || newValue === '') {
          delete nextEntry[field.key];
        } else {
          nextEntry[field.key] = newValue;
        }
        const next = entries.slice();
        next[idx] = nextEntry;
        deps.onEdit(card.section, next);
      });
      row.appendChild(input);
      entryWrap.appendChild(row);
    }
    body.appendChild(entryWrap);
  });

  const addBtn = document.createElement('button');
  addBtn.type = 'button';
  addBtn.className = 'entity-card-btn entity-card-array-add';
  addBtn.textContent = `+ entry`;
  addBtn.addEventListener('click', () => {
    const next = entries.slice();
    next.push(cloneEntryDefaults(entryDefaults));
    deps.onEdit(card.section, next);
  });
  body.appendChild(addBtn);
}

function cloneEntryDefaults(value) {
  if (value === null || typeof value !== 'object') return value;
  if (typeof structuredClone === 'function') return structuredClone(value);
  return JSON.parse(JSON.stringify(value));
}

function renderScalarSection(body, card, deps) {
  // Treat as a single-field section where the field key matches the section.
  const field = card.schema.fields[0];
  if (!field) return;
  const row = document.createElement('div');
  row.className = 'entity-card-field';

  const label = document.createElement('label');
  label.textContent = field.key;
  row.appendChild(label);

  const input = makeInputForField(field, card.data, deps, (newValue) => {
    deps.onEdit(card.section, newValue);
  });
  row.appendChild(input);
  body.appendChild(row);
}

function renderField(card, field, deps) {
  // Skip optional fields that are missing AND have no default value present?
  // Render them anyway so the user can fill in. The faction dropdown is
  // optional and we always want to show it.
  const value = card.data?.[field.key];

  const row = document.createElement('div');
  row.className = 'entity-card-field';

  const label = document.createElement('label');
  label.textContent = field.key;
  row.appendChild(label);

  const input = makeInputForField(field, value, deps, (newValue) => {
    const next = { ...card.data };
    if (newValue === undefined || newValue === null || newValue === '') {
      delete next[field.key];
    } else {
      next[field.key] = newValue;
    }
    deps.onEdit(card.section, next);
  });
  row.appendChild(input);

  return row;
}

function makeInputForField(field, value, deps, onChange) {
  // Faction dropdown.
  if (field.dropdownSource === 'factions') {
    const sel = document.createElement('select');
    sel.className = 'entity-card-input entity-card-input-faction';
    const blank = document.createElement('option');
    blank.value = '';
    blank.textContent = '(none)';
    sel.appendChild(blank);
    for (const { uuid, name } of deps.getFactionOptions()) {
      const o = document.createElement('option');
      o.value = uuid;
      o.textContent = `${name} (${uuid})`;
      sel.appendChild(o);
    }
    sel.value = value != null ? String(value) : '';
    sel.addEventListener('change', (e) => onChange(e.target.value || null));
    return sel;
  }

  // Complexity dropdown.
  if (field.dropdownSource === 'complexity') {
    const sel = document.createElement('select');
    sel.className = 'entity-card-input entity-card-input-complexity';
    const blank = document.createElement('option');
    blank.value = '';
    blank.textContent = '(none)';
    sel.appendChild(blank);
    for (const p of deps.getComplexityPaths()) {
      const o = document.createElement('option');
      o.value = p;
      o.textContent = p;
      sel.appendChild(o);
    }
    sel.value = value != null ? String(value) : '';
    sel.addEventListener('change', (e) => onChange(e.target.value || null));
    return sel;
  }

  // Enum.
  if (Array.isArray(field.enum)) {
    const sel = document.createElement('select');
    sel.className = 'entity-card-input';
    for (const opt of field.enum) {
      const o = document.createElement('option');
      o.value = opt;
      o.textContent = opt;
      sel.appendChild(o);
    }
    sel.value = value != null ? String(value) : field.enum[0];
    sel.addEventListener('change', (e) => onChange(e.target.value));
    return sel;
  }

  // Boolean.
  if (field.type === 'boolean') {
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.className = 'entity-card-input';
    cb.checked = !!value;
    cb.addEventListener('change', (e) => onChange(!!e.target.checked));
    return cb;
  }

  // Number.
  if (field.type === 'number') {
    const inp = document.createElement('input');
    inp.type = 'number';
    inp.step = 'any';
    inp.className = 'entity-card-input';
    inp.value = value != null ? String(value) : '';
    inp.addEventListener('input', (e) => {
      const s = e.target.value;
      if (s === '') return onChange(undefined);
      const n = Number(s);
      if (Number.isFinite(n)) onChange(n);
    });
    return inp;
  }

  // Array.
  if (field.type === 'array') {
    const ta = document.createElement('textarea');
    ta.className = 'entity-card-input entity-card-input-array';
    ta.rows = 2;
    ta.value = Array.isArray(value) ? value.join(', ') : '';
    ta.addEventListener('input', (e) => {
      const raw = e.target.value;
      if (raw.trim() === '') return onChange([]);
      const parts = raw.split(',').map((p) => p.trim()).filter((p) => p.length > 0);
      if (field.items === 'number') {
        const nums = parts.map(Number);
        if (nums.some((n) => !Number.isFinite(n))) return; // skip until valid
        onChange(nums);
      } else {
        onChange(parts);
      }
    });
    return ta;
  }

  // Object → JSON-ish read-only display (advanced edits go through raw mode).
  if (field.type === 'object') {
    const ta = document.createElement('textarea');
    ta.className = 'entity-card-input entity-card-input-object';
    ta.rows = 3;
    try {
      ta.value = value !== undefined ? stringifyEntityToml({ [field.key]: value }) : '';
    } catch {
      ta.value = '';
    }
    ta.disabled = true;
    return ta;
  }

  // String (default).
  const inp = document.createElement('input');
  inp.type = 'text';
  inp.className = 'entity-card-input';
  inp.value = value != null ? String(value) : '';
  inp.addEventListener('input', (e) => onChange(e.target.value));
  return inp;
}
