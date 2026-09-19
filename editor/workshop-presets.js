/** GM role presets, panel assignments, quick actions and typed mission widgets
 * over the Workshop's source owner (issue #1477). No filesystem, live ECS, TOML
 * serializer or separate history: the runtime reads the selected world into a
 * preset catalog, this module turns the Presets, Panels, Quick actions, Contacts
 * and Widgets forms into the structural edits the runtime applies, and every
 * change lands in the draft as one exact-source edit — refused by the runtime,
 * with the source untouched, when it would introduce a reserved or duplicate id,
 * an unknown widget type, a key on a type that does not own it, or a reference
 * the world does not have. The readings, snapshots and stale checks are the
 * definitions module's: the candidate is the same set of text members either way.
 *
 * The vocabularies are SPLIT, and this file is the browser's half (contract D2).
 * Widget types, widget action ids, attention bands and categories and the
 * world's own entity names come from the runtime through the catalog's
 * `choices`. The panel ids this build DRAWS and the quick-action ids are the
 * browser's, and they are imported from gui/gm-role-presets.js rather than
 * restated here — there is one such vocabulary and it belongs to the desk that
 * draws it. An authored id this build does not draw is NOT an error (Rust keeps
 * the vocabulary open on purpose): it is shown as authored-but-not-drawn and
 * carries a WARNING finding raised here, beside the runtime's own.
 *
 * Nothing authored through these forms is authority (criterion 4). A widget's
 * `actions` list repeats a shipped GM control BY DOM ID, so the ids offered are
 * the intersection of the runtime's list with the buttons this build actually
 * has: an id no control answers to would be a card with a dead button, and it
 * would be the only way anything authored here could name a route the desk does
 * not already own. */
import { tomlBasicString } from './workshop-definitions.js';
import { GM_ALL_ROLE_PRESET_ID, GM_ROLE_PRESET_PANEL_IDS, GM_ROLE_PRESET_PANEL_FOLLOWERS,
  GM_ROLE_PRESET_QUICK_ACTION_IDS, GM_WIDGET_ACTION_IDS, GM_WIDGET_TYPES,
  parseGmRolePresets } from '../gui/gm-role-presets.js';

/** A preset is authored in a world member, the same document family #1475's
 * `extra_worlds` edits. */
export const WORLD_PATH = /^assets\/worlds\/[^/]+\.toml$/;
/** The one id a preset may never be authored as: `parse_world` refuses it
 * because the built-in All preset already owns it. */
export const RESERVED_PRESET_ID = GM_ALL_ROLE_PRESET_ID;
export const DRAWN_PANEL_IDS = GM_ROLE_PRESET_PANEL_IDS;
export const PANEL_FOLLOWERS = GM_ROLE_PRESET_PANEL_FOLLOWERS;
export const DRAWN_QUICK_ACTION_IDS = GM_ROLE_PRESET_QUICK_ACTION_IDS;
export const DRAWN_WIDGET_ACTION_IDS = GM_WIDGET_ACTION_IDS;

/** A refusal the panel can show: a string id, plus the offending value. */
function refusal(id, detail = '') {
  const error = new Error(id);
  error.detail = detail;
  return error;
}

const trimmed = value => String(value ?? '').trim();
const stringArray = values => `[${values.map(tomlBasicString).join(', ')}]`;

/** The edits that turn `before` into `after` inside an array of strings:
 * removals first, by descending index so each one still names the element it
 * meant, then appends. An empty reading is written as a whole array through
 * `put`, because the catalog cannot say whether an empty list is an absent key
 * (`panels`, `quick_actions`, `contacts` and `actions` all default) and `insert`
 * needs the array. The same shape #1475's composition planner uses.
 *
 * The comparison counts OCCURRENCES rather than membership: `panels`,
 * `quick_actions` and `contacts` are open string vocabularies with no duplicate
 * rule in Rust, so a hand-authored `contacts = ["Kestrel", "Kestrel"]` is
 * reachable, and treating the array as a set would make the Remove button beside
 * one of those rows plan nothing at all. The SURPLUS occurrences go from the
 * tail, so the rows the form kept keep their own lines. */
function arrayEdits(path, before, after) {
  const surplus = new Map();
  for (const value of before) surplus.set(value, (surplus.get(value) ?? 0) + 1);
  const added = [];
  for (const value of after) {
    const have = surplus.get(value) ?? 0;
    if (have > 0) surplus.set(value, have - 1);
    else added.push(value);
  }
  if (!before.length) return added.length ? [{ op: 'put', path, value_source: stringArray(added) }] : [];
  const edits = [];
  for (let index = before.length - 1; index >= 0; index -= 1) {
    const value = before[index];
    const extra = surplus.get(value) ?? 0;
    if (extra <= 0) continue;
    surplus.set(value, extra - 1);
    edits.push({ op: 'remove', path: [...path, index] });
  }
  const kept = before.length - edits.length;
  added.forEach((value, offset) => edits.push({ op: 'insert', path, index: kept + offset,
    value_source: tomlBasicString(value) }));
  return edits;
}

/** The optional keys a widget may carry beyond its id, type and label. */
export const WIDGET_KEYS = Object.freeze(['band', 'category', 'ship', 'actions', 'text']);
/** A value of each key that the browser's own normaliser accepts, used only to
 * ask it which keys a type keeps. */
const PROBE_VALUES = Object.freeze({ band: 'probe-band', category: 'probe-category', ship: 'Probe Ship',
  actions: [GM_WIDGET_ACTION_IDS[0]], text: 'probe.text' });

/** Ask gui/gm-role-presets.js's own parser what a widget of this type keeps.
 * The two types whose normalisation DROPS a widget without their key carry it,
 * so the probe survives to be read. */
function probeWidget(kind, key) {
  const widget = { id: 'probe', type: kind, label: 'probe' };
  if (kind === 'actions') widget.actions = PROBE_VALUES.actions;
  if (kind === 'note') widget.text = PROBE_VALUES.text;
  widget[key] = PROBE_VALUES[key];
  const [preset] = parseGmRolePresets([{ id: 'probe', label: 'probe', widget: [widget] }]);
  return preset?.widgets?.[0] ?? null;
}

/** Which keys each widget type OWNS, derived by asking the browser's own
 * normaliser rather than restating its rule a second time. The runtime enforces
 * the same ownership at world load (`GmRolePresetWidget::validate` refuses a key
 * that belongs to another type) and `src/world/config_tests.rs` reads this
 * file's vocabulary to keep the two in step, so there is one rule with two
 * readers and no third copy. A type the browser does not know owns nothing,
 * which is what makes `widget-unknown-type` the only finding such a row can
 * raise rather than a form full of controls for a card nobody draws. */
export const WIDGET_KEY_OWNERS = Object.freeze(Object.fromEntries(GM_WIDGET_TYPES.map(kind => [kind,
  Object.freeze(WIDGET_KEYS.filter(key => probeWidget(kind, key)?.[key] !== undefined))])));

/** Whether a widget of this type owns this key. Every per-widget control is
 * built through this, so a key can never be OFFERED on a type that does not own
 * it (contract D). */
export const widgetOwns = (kind, key) => (WIDGET_KEY_OWNERS[trimmed(kind)] || []).includes(key);

/** The widget types the panel offers: the runtime's own list (contract D2),
 * narrowed to the types this build can draw a card for. */
export function widgetTypeChoices(catalog) {
  return (catalog?.choices?.widget_types || []).filter(kind => GM_WIDGET_TYPES.includes(kind));
}

/** The action ids an `actions` widget may repeat: the runtime's list narrowed to
 * the buttons this build actually draws (criterion 4 — see the module header). */
export function widgetActionChoices(catalog) {
  return (catalog?.choices?.widget_actions || []).filter(id => DRAWN_WIDGET_ACTION_IDS.includes(id));
}

/** Entities the selected preset could still name as a contact: the world's own
 * entity names, minus the ones the form already lists. */
export function contactChoices(catalog, listed = []) {
  return (catalog?.choices?.entities || []).filter(name => !listed.includes(name));
}

/** The world members the selector offers: the draft's own world members, then
 * every world the runtime named that is not one of them. A dependency world is
 * listed so its presets can be READ — it is authored where it is defined, so the
 * panel shows it read-only with its origin. */
export function worldChoices(paths, catalog = null) {
  const entries = (paths || []).filter(path => WORLD_PATH.test(path)).sort((a, b) => a.localeCompare(b))
    .map(path => ({ path, origin: 'draft' }));
  const seen = new Set(entries.map(entry => entry.path));
  const extra = [...(catalog?.worlds || [])];
  if (catalog?.path) extra.push({ path: catalog.path, origin: catalog.origin ?? null });
  for (const entry of extra) {
    if (!entry?.path || seen.has(entry.path)) continue;
    seen.add(entry.path);
    entries.push({ path: entry.path, origin: entry.origin ?? null });
  }
  return entries;
}

const spanValue = span => (span && typeof span === 'object' ? String(span.value ?? '') : '');

/** One widget as its row holds it. `present` records which optional keys the
 * DOCUMENT carries, which is what tells a `put` from a `set` — a value read as
 * empty is not the same thing as an absent key. */
function widgetForm(widget) {
  const form = { index: widget?.index ?? null, id: widget?.id ?? '', id_line: widget?.id_line ?? null,
    kind: widget?.kind ?? '', kind_line: widget?.kind_line ?? null,
    label: widget?.label ?? '', label_line: widget?.label_line ?? null,
    band: spanValue(widget?.band), category: spanValue(widget?.category), ship: spanValue(widget?.ship),
    actions: (widget?.actions || []).map(entry => entry.value), text: spanValue(widget?.text),
    unknown_keys: [...(widget?.unknown_keys || [])], present: {} };
  for (const key of WIDGET_KEYS) {
    form.present[key] = key === 'actions' ? (widget?.actions || []).length > 0 : Boolean(widget?.[key]);
  }
  return form;
}

/** The whole form for one world's presets: every preset as the values its
 * controls hold, in document order. `index` is the entry's slot in the
 * `[[gm_role_preset]]` array; a preset created through the runtime's own
 * skeleton has landed in the source before it is ever shown here, so every row
 * has one. */
export function presetsForm(catalog) {
  return { presets: (catalog?.presets || []).map(preset => ({
    index: preset.index, id: preset.id ?? '', id_line: preset.id_line ?? null,
    label: preset.label ?? '', label_line: preset.label_line ?? null,
    panels: (preset.panels || []).map(entry => entry.value),
    quick_actions: (preset.quick_actions || []).map(entry => entry.value),
    contacts: (preset.contacts || []).map(entry => entry.value),
    widgets: (preset.widgets || []).map(widgetForm),
    unknown_keys: [...(preset.unknown_keys || [])], removed: false })) };
}

/** A widget row the panel adds: it has no slot until an edit lands, so Apply
 * appends it as a `[[gm_role_preset.widget]]` table. */
export function newWidget(id, kind) {
  const widget = { index: null, id: trimmed(id), id_line: null, kind: trimmed(kind), kind_line: null,
    label: trimmed(id), label_line: null, band: '', category: '', ship: '', actions: [], text: '',
    unknown_keys: [], present: {} };
  for (const key of WIDGET_KEYS) widget.present[key] = false;
  return widget;
}

/** Whether a preset's whole content can be carried to another slot. A move is
 * `set` edits on the swapped slots (contract D3, the shape #1475's roots use),
 * and those edits can carry every scalar and list a preset holds — but NOT its
 * `[[gm_role_preset.widget]]` tables, which are their own tables in the
 * document and which the edit vocabulary cannot move, nor an unknown key whose
 * VALUE the catalog does not carry. Swapping only what can be carried would
 * hand one preset another's widgets, so such a preset is not movable here and
 * the row says so: reordering it is a source edit. */
export const presetMovable = preset => !(preset?.widgets || []).length && !(preset?.unknown_keys || []).length;

/** Whether a widget can be carried to another slot: the same rule one level
 * down. Every typed key is rewritten by a `set`, so only an unknown key — whose
 * value the catalog does not carry — stops a move. */
export const widgetMovable = widget => !(widget?.unknown_keys || []).length;

/** Swap the live preset at `position` with its nearest live neighbour in
 * `direction` (-1 up, +1 down), in the form only. Returns the position it moved
 * to, or null when it is already at that end or either preset cannot be
 * carried. */
export function movePreset(form, position, direction) {
  const list = form?.presets || [];
  const live = list.map((entry, at) => (entry.removed ? null : at)).filter(at => at != null);
  const rank = live.indexOf(position);
  if (rank < 0) return null;
  const target = live[rank + direction];
  if (target == null || !presetMovable(list[position]) || !presetMovable(list[target])) return null;
  [list[position], list[target]] = [list[target], list[position]];
  return target;
}

/** Swap two widgets of one preset in the form only, the same way. */
export function moveWidget(widgets, position, direction) {
  const list = widgets || [];
  const target = position + direction;
  if (position < 0 || position >= list.length || target < 0 || target >= list.length) return null;
  if (!widgetMovable(list[position]) || !widgetMovable(list[target])) return null;
  [list[position], list[target]] = [list[target], list[position]];
  return target;
}

/** The rule each preset and widget breaks, as a multiset keyed by the rule and
 * what it names — the shape #1476's `introduced` compares, for the same reason:
 * a violation the document ALREADY carried must not refuse an edit that does not
 * add one, so a hand-broken world can be repaired one edit at a time.
 *
 * Each key is the key RUST uses for the same rule (`presets.rs`'s `Issue::key`),
 * not a fuller address: the runtime keys an empty id or label by the KEY NAME and
 * an unknown type by the type, so a rename must not move the key. Keying a
 * widget's empty label by `<preset>/<widget>` instead made the browser refuse
 * renaming any preset or widget that already carried one — a pre-existing
 * violation read as new, which is the exact mistake the introduced-only rule
 * below exists to stop, and a refusal `presets::compose` would not have made.
 *
 * `widget-key-on-wrong-type` is deliberately NOT among them: the planner writes a
 * key only when the widget's type owns it and removes the old type's keys when the
 * type changes, so no edit this module plans can introduce that violation. A key
 * the document already carries on the wrong type is the runtime's finding to
 * report at its line, and refusing an unrelated edit over it would be the exact
 * mistake the introduced-only rule exists to stop. */
function presetViolations(presets) {
  const counts = new Map();
  const add = (rule, key, detail) => {
    const mapKey = JSON.stringify([rule, key]);
    if (!counts.has(mapKey)) counts.set(mapKey, []);
    counts.get(mapKey).push([rule, detail]);
  };
  const ids = new Set();
  (presets || []).forEach((preset, position) => {
    const id = trimmed(preset.id);
    const where = preset.index == null ? `#${position}` : `#${preset.index}`;
    if (!id) add('empty', 'id', where);
    else if (id === RESERVED_PRESET_ID) add('reserved', id, id);
    else if (ids.has(id)) add('duplicate', id, id);
    else ids.add(id);
    if (!trimmed(preset.label)) add('label', 'label', id || where);
    const seen = new Set();
    (preset.widgets || []).forEach((widget, at) => {
      const widgetId = trimmed(widget.id);
      const scope = `${id || where}/${widgetId || `#${at}`}`;
      if (!widgetId) add('widget_empty', 'id', scope);
      else if (seen.has(widgetId)) add('widget_duplicate', widgetId, widgetId);
      else seen.add(widgetId);
      if (!trimmed(widget.label)) add('widget_label', 'label', scope);
      const kind = trimmed(widget.kind);
      if (!WIDGET_KEY_OWNERS[kind]) add('type', kind, kind);
    });
  });
  return counts;
}

/** The fields a new `[[gm_role_preset.widget]]` table is appended with: its id,
 * its type under the TOML key the runtime reads (`type`), its label, and only
 * the keys its TYPE owns. */
function widgetFields(widget) {
  const kind = trimmed(widget.kind);
  const fields = [['id', tomlBasicString(trimmed(widget.id))], ['type', tomlBasicString(kind)],
    ['label', tomlBasicString(trimmed(widget.label))]];
  for (const key of WIDGET_KEYS) {
    if (!widgetOwns(kind, key)) continue;
    if (key === 'actions') {
      if (widget.actions?.length) fields.push(['actions', stringArray(widget.actions)]);
      continue;
    }
    if (trimmed(widget[key])) fields.push([key, tomlBasicString(trimmed(widget[key]))]);
  }
  return fields;
}

/** An id, a type or a label the reading shows as EMPTY is written with `put`,
 * which inserts the key or replaces it, and a non-empty one with `set`, which
 * keeps the key's decor and its type. The line cannot make that choice: the
 * catalog gives an absent key the entry's own header line, so a `set` would be
 * planned for a key that is not there and the exact-source owner refuses a `set`
 * it cannot locate. A widget or preset whose `label` key is missing is exactly
 * the hand-broken world these forms exist to repair — `widget-empty-label`
 * refuses the world's load and `preset-empty-label` blocks save and export — so
 * the repair must be reachable from the form. */
function scalarEdit(base, key, value, had) {
  if (value === (had ?? '')) return [];
  return [{ op: (had ?? '') === '' ? 'put' : 'set', path: [...base, key],
    value_source: tomlBasicString(value) }];
}

function widgetSlotEdits(base, original, widget) {
  const edits = [];
  const kind = trimmed(widget.kind);
  const kindChanged = kind !== original.kind;
  const scalar = (key, value, had) => edits.push(...scalarEdit(base, key, value, had));
  scalar('id', trimmed(widget.id), original.id);
  scalar('type', kind, original.kind);
  scalar('label', trimmed(widget.label), original.label);
  for (const key of WIDGET_KEYS) {
    if (!widgetOwns(kind, key)) {
      // A key this type does not own is left exactly as the document holds it:
      // it is a `widget-key-on-wrong-type` finding the runtime reports at its
      // line, and removing it unasked would be an edit nobody pressed for. The
      // one exception is a type this edit CHANGED, where the old type's keys
      // would otherwise become a violation this edit introduced.
      if (kindChanged && original.present[key]) edits.push({ op: 'remove', path: [...base, key] });
      continue;
    }
    if (key === 'actions') {
      edits.push(...arrayEdits([...base, 'actions'], original.actions, widget.actions || []));
      continue;
    }
    const value = trimmed(widget[key]);
    if (value === original[key]) continue;
    if (!value) edits.push({ op: 'remove', path: [...base, key] });
    else edits.push({ op: original.present[key] ? 'set' : 'put', path: [...base, key], value_source: tomlBasicString(value) });
  }
  return edits;
}

/** The edits that take one preset's widgets from the reading to the form. The
 * kept widgets fill the surviving slots in form order — so a reorder rewrites
 * the values whose slot changed — then removals follow by descending index, and
 * new widgets are appended last, so no edit shifts the slot a later one names. */
function widgetEdits(base, before, after) {
  const kept = (after || []).filter(widget => widget.index != null);
  const removed = (before || []).map(widget => widget.index)
    .filter(index => !kept.some(widget => widget.index === index));
  const surviving = (before || []).map(widget => widget.index)
    .filter(index => !removed.includes(index)).sort((a, b) => a - b);
  if (kept.length !== surviving.length) throw new Error('workshop.inspector_stale');
  const edits = [];
  kept.forEach((widget, rank) => {
    const slot = surviving[rank];
    const original = (before || []).find(entry => entry.index === slot);
    if (!original) throw new Error('workshop.inspector_stale');
    if (widget.index !== slot && (!widgetMovable(original) || !widgetMovable(widget))) {
      throw refusal('workshop.presets.refused.reorder', trimmed(widget.id) || original.id);
    }
    edits.push(...widgetSlotEdits([...base, slot], original, widget));
  });
  for (const index of [...removed].sort((a, b) => b - a)) edits.push({ op: 'remove', path: [...base, index] });
  for (const widget of (after || []).filter(entry => entry.index == null)) {
    edits.push({ op: 'append_table', path: base, fields: widgetFields(widget) });
  }
  return edits;
}

function presetSlotEdits(base, original, entry, moved) {
  if (moved && (!presetMovable(original) || !presetMovable(entry))) {
    throw refusal('workshop.presets.refused.reorder', trimmed(entry.id) || original.id);
  }
  const edits = [];
  const scalar = (key, value, had) => edits.push(...scalarEdit(base, key, value, had));
  scalar('id', trimmed(entry.id), original.id);
  scalar('label', trimmed(entry.label), original.label);
  edits.push(...arrayEdits([...base, 'panels'], original.panels, entry.panels || []));
  edits.push(...arrayEdits([...base, 'quick_actions'], original.quick_actions, entry.quick_actions || []));
  edits.push(...arrayEdits([...base, 'contacts'], original.contacts, entry.contacts || []));
  edits.push(...widgetEdits([...base, 'widget'], original.widgets, entry.widgets));
  return edits;
}

/** The exact-source edits that take one world's presets from the reading to the
 * form, empty when nothing changed. The kept presets fill the surviving slots in
 * form order — so a reorder is `set`s on the slots whose content moved — then
 * whole-preset removals follow by descending index. Refusals the form can see
 * itself (a reserved or duplicate id, an empty id or label, a duplicate or
 * unsupported widget, a key on a type that does not own it) are string ids;
 * everything the world owns (an unknown band, category, ship, action or contact)
 * is the runtime's to refuse against the world it is reading. */
export function planPresetEdits({ catalog, form }) {
  const presets = catalog?.presets || [];
  const reading = presetsForm(catalog).presets;
  const live = (form?.presets || []).filter(entry => !entry.removed);
  const carried = presetViolations(reading);
  for (const [key, list] of presetViolations(live)) {
    const already = carried.get(key)?.length ?? 0;
    if (already >= list.length) continue;
    // The first one this edit adds beyond what the document already carried.
    const [rule, detail] = list[already];
    throw refusal(`workshop.presets.refused.${rule}`, detail);
  }
  const removed = (form?.presets || []).filter(entry => entry.removed && entry.index != null)
    .map(entry => entry.index);
  const surviving = presets.map(entry => entry.index).filter(index => !removed.includes(index)).sort((a, b) => a - b);
  const kept = live.filter(entry => entry.index != null);
  // A preset the reading no longer has is a stale form, not an edit at that slot.
  if (kept.length !== live.length || kept.length !== surviving.length
    || kept.some(entry => !presets.some(original => original.index === entry.index))) {
    throw new Error('workshop.inspector_stale');
  }
  const edits = [];
  kept.forEach((entry, rank) => {
    const slot = surviving[rank];
    const original = reading.find(candidate => candidate.index === slot);
    if (!original) throw new Error('workshop.inspector_stale');
    edits.push(...presetSlotEdits(['gm_role_preset', slot], original, entry, entry.index !== slot));
  });
  for (const index of [...removed].sort((a, b) => b - a)) edits.push({ op: 'remove', path: ['gm_role_preset', index] });
  return edits;
}

/** The text a new preset's `[[gm_role_preset]]` block is appended to the world
 * with. The block is the RUNTIME's own serialisation — this only joins it to the
 * document, and it joins it with the document's OWN line ending: a world written
 * with CRLF must not gain a lone LF, which is the same rule the exact-source
 * owner restores per line. A top-level table header at the end of the file is
 * the one textual append that cannot capture what precedes it. */
export function appendPresetBlock(source, block) {
  const text = String(source ?? '');
  const ending = /\r\n/.test(text) ? '\r\n' : '\n';
  const body = String(block ?? '').replace(/\r\n/g, '\n').replace(/\n/g, ending);
  const separator = !text || text.endsWith(ending) ? '' : ending;
  return `${text}${separator}${body}`;
}

/** Findings the BROWSER owns (contract C and D2). `panels` and `quick_actions`
 * are open string vocabularies in Rust on purpose — a preset may already name a
 * panel this build does not draw yet — so a name this build does not draw is a
 * WARNING raised here, in the same list as the runtime's own findings, and never
 * a refusal. Rust is left free of the browser's vocabulary. A widget's `actions`
 * are the same buttons the quick-action facet narrows, so an authored action id
 * this build does not draw is reported the same way.
 *
 * A finding carries a string id and its params rather than a sentence: the panel
 * localises it, exactly as it localises a severity word. Deterministic, sorted
 * and deduped, so two readings of one world give one list. */
export function presetFindings(catalog) {
  const records = [...(catalog?.findings || [])];
  const file = catalog?.path ?? '';
  const warn = (category, line, string, params) => records.push({ severity: 'warning', category, file,
    line: line ?? null, string, params });
  for (const preset of catalog?.presets || []) {
    for (const entry of preset.panels || []) {
      if (DRAWN_PANEL_IDS.includes(entry.value)) continue;
      const leader = PANEL_FOLLOWERS[entry.value];
      if (leader) warn('preset-panel-not-drawn', entry.line, 'workshop.presets.panel_follows', { value: entry.value, leader });
      else warn('preset-panel-not-drawn', entry.line, 'workshop.presets.panel_not_drawn', { value: entry.value });
    }
    for (const entry of preset.quick_actions || []) {
      if (DRAWN_QUICK_ACTION_IDS.includes(entry.value)) continue;
      warn('preset-quick-action-not-drawn', entry.line, 'workshop.presets.quick_action_not_drawn', { value: entry.value });
    }
    for (const widget of preset.widgets || []) {
      for (const entry of widget.actions || []) {
        if (DRAWN_WIDGET_ACTION_IDS.includes(entry.value)) continue;
        warn('preset-quick-action-not-drawn', entry.line, 'workshop.presets.quick_action_not_drawn', { value: entry.value });
      }
    }
  }
  const seen = new Set();
  const unique = [];
  for (const record of records) {
    const key = JSON.stringify([record.file ?? '', record.line ?? 0, record.severity ?? '', record.category ?? '',
      record.message ?? '', record.string ?? '', record.params ?? null]);
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(record);
  }
  return unique.sort((a, b) => String(a.file ?? '').localeCompare(String(b.file ?? ''))
    || (a.line ?? 0) - (b.line ?? 0)
    || String(a.severity ?? '').localeCompare(String(b.severity ?? ''))
    || String(a.category ?? '').localeCompare(String(b.category ?? ''))
    || String(a.message ?? a.string ?? '').localeCompare(String(b.message ?? b.string ?? '')));
}

/** The findings that point at one line of one member, over the merged list. */
export function findingsOn(findings, file, line) {
  return (findings || []).filter(finding => finding.file === file && finding.line === line);
}

/** The runtime refuses a preset edit with `<rule>: <detail>` — the rule is the
 * finding category the same violation reports as (presets.rs `refusal`), the
 * detail is the runtime's own sentence, which for a widget rule is
 * `GmRolePresetWidget::validate`'s own words. The panel shows a localised
 * sentence per CATEGORY with that message as the detail, so the rule is read off
 * the prefix; the word patterns beneath are the fallback for a message with no
 * such prefix (a document-level refusal, a runtime the panel has not been
 * rebuilt against), ordered from the most specific. */
const REFUSAL_RULES = Object.freeze({
  'preset-reserved-id': 'reserved',
  'preset-duplicate-id': 'duplicate',
  'preset-empty-id': 'empty',
  'preset-empty-label': 'label',
  'widget-duplicate-id': 'widget_duplicate',
  'widget-empty-id': 'widget_empty',
  'widget-empty-label': 'widget_label',
  'widget-unknown-type': 'type',
  'widget-key-on-wrong-type': 'key',
  'widget-unknown-band': 'band',
  'widget-unknown-category': 'category',
  'widget-unknown-ship': 'ship',
  'widget-unknown-action': 'action',
  'preset-unknown-contact': 'contact',
});
/** Rules the runtime refuses that this panel has no sentence of its own for, and
 * deliberately does not borrow one: each is a whole-widget rule about a facet the
 * form already keeps inside its type (an `actions` card with no button, a `note`
 * with no text or a text that is not a strings.csv id, one button named twice),
 * so an author only reaches them by editing the source by hand. Naming them here
 * rather than letting the word patterns below guess keeps `empty` from claiming an
 * empty BUTTON ROW and `duplicate` from claiming a repeated ACTION: both would
 * name the wrong thing. They take the catch-all row, which carries the runtime's
 * own sentence. */
const DETAIL_ONLY_RULES = Object.freeze(['widget-empty-actions', 'widget-duplicate-action',
  'widget-empty-text', 'widget-invalid-text']);
const REFUSAL_CATEGORIES = Object.freeze([
  ['reserved', /\breserved\b/i],
  ['widget_duplicate', /duplicat[^.]*widget|widget[^.]*duplicat/i],
  ['duplicate', /duplicat|twice|already (lists|names|declares)/i],
  ['key', /belongs to|not a key of|only on|wrong type/i],
  ['type', /unknown (widget )?type|not a (widget )?type/i],
  ['band', /\bband\b/i],
  ['category', /\bcategor/i],
  ['ship', /\bship\b/i],
  ['action', /\baction\b/i],
  ['contact', /\bcontact\b/i],
  ['label', /\blabel\b/i],
  ['empty', /\bempty\b|\bblank\b/i],
]);

export function refusalMessage(error) {
  if (typeof error === 'string') return error;
  return String(error?.message ?? error ?? '');
}

/** A refusal of the DOCUMENT rather than of a preset rule: the member is not in
 * the draft at all, so the panel's reading has parted from it. It names no rule
 * the preset strings describe and must not borrow one's words, so it is the
 * shared stale sentence, which is also the one thing an author can act on. */
const DOCUMENT_RULES = Object.freeze(['unknown-document']);

export function refusalStringId(message, fallback = 'workshop.presets.refused.other') {
  const text = refusalMessage(message);
  const prefix = /^([a-z-]+):\s/.exec(text)?.[1];
  if (DOCUMENT_RULES.includes(prefix)) return 'workshop.inspector_stale';
  if (DETAIL_ONLY_RULES.includes(prefix)) return fallback;
  // The exact-source owner's own refusal of a stale `expected_source`, which
  // carries no rule prefix and is the same thing an author must do about it.
  if (/document changed/i.test(text)) return 'workshop.inspector_stale';
  const rule = REFUSAL_RULES[prefix];
  if (rule) return `workshop.presets.refused.${rule}`;
  const found = REFUSAL_CATEGORIES.find(([, pattern]) => pattern.test(text));
  // The default fallback CARRIES the runtime's sentence: an unmapped refusal is
  // the one case where its own words are all the author has, and the shared
  // `workshop.inspector_refused` has no {detail} to put them in (#1475).
  return found ? `workshop.presets.refused.${found[0]}` : fallback;
}
