/** Faction and console-complexity authoring over the Workshop's source owner
 * (issue #1474). No filesystem, live ECS, TOML serializer or separate history:
 * the runtime reads the draft into a catalog, this module turns a form's state
 * into the structural edits the runtime applies, and every change lands in the
 * draft as one exact-source edit. The only value text composed here is what a
 * form can produce — a quoted string, a whole number, an array of quoted
 * strings or an inline table of those — and the runtime parses each one back
 * as exactly one TOML value before it touches the document. */

export const FACTION_DIRECTORY = 'assets/factions/';
const TEXT_MEMBER = /\.(toml|rhai)$/;
/** The catalog names the refusal reason by the contract's short key; the
 * runtime type spells the TOML key in full, and an edit must name the key the
 * document actually holds. */
const COMPLIANCE_TOML_KEYS = Object.freeze({ refusal: 'refusal_reason' });
const tomlKey = key => COMPLIANCE_TOML_KEYS[key] || key;
const BARE_KEY = /^[A-Za-z0-9_-]+$/;

/** The compliance keys and their kinds come from the catalog's defaults — the
 * runtime's `ComplianceDisposition::default()` — not from a list typed here: a
 * key whose default is a number is whole seconds, one whose default is an
 * order response is a response, and any other present key (the refusal reason)
 * is text. */
export const complianceKeys = defaults => Object.keys(defaults || {});
export const complianceIsSeconds = (key, defaults) => typeof defaults?.[key] === 'number';
export const complianceIsResponse = (key, defaults, responses) => typeof defaults?.[key] === 'string'
  && (responses || []).includes(defaults[key]);

/** The draft's toml/rhai members as `{ path: text }` — what the catalog reads. */
export function textMembers(draft) {
  const files = {};
  for (const path of draft?.paths() || []) {
    if (!TEXT_MEMBER.test(path)) continue;
    const text = draft.read(path);
    if (typeof text === 'string') files[path] = text;
  }
  return files;
}

/** What a reading was taken from: the draft's identity and every text member's
 * exact source, so a later edit anywhere in the draft can be detected. */
export function definitionsSnapshot(draft) {
  const files = textMembers(draft);
  return { draft, files, paths: Object.keys(files).join('\n') };
}

/** Whether a reading still describes the draft. Every text member counts, not
 * only the factions and hulls: a finding about an entity's faction reference or
 * a world trigger's faction name is part of the reading too. Members are
 * compared as strings rather than hashed, because an untouched member keeps the
 * same string object and the comparison is then one reference check per
 * member — cheap enough for the refresh the surface runs on every keystroke. */
export function snapshotIsCurrent(snapshot, draft) {
  if (!snapshot || !draft || snapshot.draft !== draft) return false;
  const current = textMembers(draft);
  if (Object.keys(current).join('\n') !== snapshot.paths) return false;
  return Object.entries(current).every(([path, text]) => snapshot.files[path] === text);
}

/** A faction's file stem from its reference name: lowercase, with every run of
 * characters outside `[a-z0-9_-]` folded to one hyphen. */
export function factionSlug(name) {
  return String(name ?? '').toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '');
}

export function factionSlugPath(name) {
  const slug = factionSlug(name);
  if (!slug) throw new Error('workshop.definitions.invalid_name');
  return `${FACTION_DIRECTORY}${slug}.toml`;
}

/** Factions this one could still list as an enemy: not itself, and none it
 * already lists (the form's current list, which may differ from the reading's). */
export function enemyChoices(choices, definition, listed = (definition?.enemies || []).map(entry => entry.uuid)) {
  return (choices?.factions || []).filter(entry => entry.uuid !== definition?.uuid && !listed.includes(entry.uuid));
}

/** A TOML basic string carrying a form's text exactly. */
export function tomlBasicString(text) {
  const escaped = String(text ?? '').replace(/[\\"\x00-\x1f\x7f]/g, char => {
    switch (char) {
      case '\\': return '\\\\';
      case '"': return '\\"';
      case '\n': return '\\n';
      case '\r': return '\\r';
      case '\t': return '\\t';
      case '\b': return '\\b';
      case '\f': return '\\f';
      default: return `\\u${char.charCodeAt(0).toString(16).toUpperCase().padStart(4, '0')}`;
    }
  });
  return `"${escaped}"`;
}

/** The text a TOML string value carries, or the trimmed source itself when it
 * is not a string (a number, a bare word). Basic and literal single-line strings
 * are the shapes the runtime writes and the authored content uses. */
export function decodeTomlString(source) {
  if (typeof source !== 'string') return '';
  const text = source.trim();
  if (text.length >= 2 && text.startsWith("'") && text.endsWith("'")) return text.slice(1, -1);
  if (text.length >= 2 && text.startsWith('"') && text.endsWith('"')) {
    return text.slice(1, -1).replace(/\\(u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8}|.)/g, (_match, code) => {
      switch (code[0]) {
        case 'n': return '\n';
        case 'r': return '\r';
        case 't': return '\t';
        case 'b': return '\b';
        case 'f': return '\f';
        case 'u': case 'U': return String.fromCodePoint(Number.parseInt(code.slice(1), 16));
        default: return code;
      }
    });
  }
  return text;
}

function integerSource(value) {
  const text = String(value ?? '').trim();
  if (!/^[+-]?\d+$/.test(text)) throw new Error('workshop.definitions.invalid_number');
  return String(Number.parseInt(text, 10));
}

function inlineTable(pairs) {
  const key = name => (BARE_KEY.test(name) ? name : tomlBasicString(name));
  return pairs.length ? `{ ${pairs.map(([name, value]) => `${key(name)} = ${value}`).join(', ')} }` : '{}';
}

const stringArray = values => `[${values.map(tomlBasicString).join(', ')}]`;

/** The edits that turn `before` into `after` inside an array value: removals
 * first, by descending index so each one still names the element it meant,
 * then appends. With `createWhenEmpty` an empty reading is written as a whole
 * array through `put`, because the catalog cannot say whether an empty list is
 * an absent key (the faction type defaults it) and `insert` needs the array. */
function arrayEdits(path, before, after, { createWhenEmpty = false } = {}) {
  const kept = before.filter(value => after.includes(value));
  const added = after.filter(value => !before.includes(value));
  if (createWhenEmpty && !before.length) return added.length ? [{ op: 'put', path, value_source: stringArray(added) }] : [];
  const edits = [];
  for (let index = before.length - 1; index >= 0; index -= 1) {
    if (!after.includes(before[index])) edits.push({ op: 'remove', path: [...path, index] });
  }
  added.forEach((value, offset) => edits.push({ op: 'insert', path, index: kept.length + offset, value_source: tomlBasicString(value) }));
  return edits;
}

/** The compliance fieldset's state: every key the runtime type defaults — a
 * present key from its exact source, an absent one from the runtime's default
 * — plus any other present key (the refusal reason) as text. */
export function complianceForm(present, defaults = {}) {
  const form = {};
  for (const key of complianceKeys(defaults)) {
    form[key] = present?.[key] ? decodeTomlString(present[key].source) : String(defaults[key] ?? '');
  }
  for (const [key, entry] of Object.entries(present || {})) if (!(key in form)) form[key] = decodeTomlString(entry?.source);
  return form;
}

export function factionForm(definition, defaults = {}) {
  return { name: definition?.name ?? '', display_name: definition?.display_name ?? '',
    enemies: (definition?.enemies || []).map(entry => entry.uuid),
    compliance: definition?.compliance ? complianceForm(definition.compliance, defaults) : null };
}

const complianceValueSource = (key, value, defaults) => (complianceIsSeconds(key, defaults)
  ? integerSource(value) : tomlBasicString(String(value ?? '').trim()));
const complianceSourceEquals = (key, value, source, defaults) => (complianceIsSeconds(key, defaults)
  ? Number(String(value).trim()) === Number(decodeTomlString(source)) : String(value).trim() === decodeTomlString(source));
function complianceDefaultEquals(key, value, defaults) {
  const fallback = defaults?.[key];
  if (fallback == null) return String(value ?? '').trim() === '';
  return complianceIsSeconds(key, defaults) ? Number(String(value).trim()) === Number(fallback) : String(value).trim() === String(fallback);
}

/** The exact-source edits that take a faction from its reading to the form,
 * empty when nothing changed. Refusals are string ids the panel can show. */
export function planFactionEdits({ definition, form, defaults = {} }) {
  const edits = [];
  const name = String(form.name ?? '').trim();
  if (!name) throw new Error('workshop.definitions.invalid_name');
  if (name !== definition.name) {
    edits.push({ op: definition.name == null ? 'put' : 'set', path: ['name'], value_source: tomlBasicString(name) });
  }
  const display = String(form.display_name ?? '').trim();
  if (display !== (definition.display_name ?? '')) {
    if (!display) edits.push({ op: 'remove', path: ['display_name'] });
    else edits.push({ op: definition.display_name == null ? 'put' : 'set', path: ['display_name'], value_source: tomlBasicString(display) });
  }
  edits.push(...arrayEdits(['enemies'], (definition.enemies || []).map(entry => entry.uuid), form.enemies || [],
    { createWhenEmpty: true }));
  const present = definition.compliance;
  if (!form.compliance) {
    if (present) edits.push({ op: 'remove', path: ['compliance'] });
  } else if (!present) {
    // Materialised as one inline table so the new key carries every value the
    // form shows, defaults included: a table holding only the changed keys would
    // read back through the same defaults, but the author asked for them in
    // the source where the next reader can see and tune them.
    const pairs = Object.entries(form.compliance).filter(([, value]) => String(value ?? '').trim() !== '')
      .map(([key, value]) => [tomlKey(key), complianceValueSource(key, value, defaults)]);
    edits.push({ op: 'put', path: ['compliance'], value_source: inlineTable(pairs) });
  } else {
    for (const [key, value] of Object.entries(form.compliance)) {
      const entry = present[key];
      const blank = String(value ?? '').trim() === '';
      if (entry) {
        if (blank) edits.push({ op: 'remove', path: ['compliance', tomlKey(key)] });
        else if (!complianceSourceEquals(key, value, entry.source, defaults)) {
          edits.push({ op: 'set', path: ['compliance', tomlKey(key)], value_source: complianceValueSource(key, value, defaults) });
        }
      } else if (!blank && !complianceDefaultEquals(key, value, defaults)) {
        edits.push({ op: 'put', path: ['compliance', tomlKey(key)], value_source: complianceValueSource(key, value, defaults) });
      }
    }
  }
  return edits;
}

/** A station's complexity form: its visiting rating and every rung as the
 * ids it automates and the AI rules it tunes. A rung added in the form has no
 * index yet and becomes an `append_table` on Apply. */
export function ratingForm(station) {
  return { visiting_rating: station?.visiting_rating ? decodeTomlString(station.visiting_rating.source) : '',
    ratings: (station?.ratings || []).map(rating => ({ index: rating.index, name: rating.name ?? '',
      automated_systems: (rating.automated_systems || []).map(entry => entry.id),
      ai_rules: (rating.ai_tuning || []).map(entry => entry.rule), removed: false })) };
}

export function newRung(name) {
  return { index: null, name, automated_systems: [], ai_rules: [], removed: false };
}

/** The rung names the form would write, for the visiting-rating choices and
 * the duplicate check a runtime rung name must pass. */
export function rungNames(form) {
  return (form?.ratings || []).filter(rung => !rung.removed).map(rung => String(rung.name ?? '').trim());
}

const rulesTable = rules => inlineTable(rules.map(rule => [rule, '{}']));

/** `ai_tuning` presence is what the runtime reads (`has_ai_rule` checks the
 * key), so a rule is an empty table under it, put or removed one key at a
 * time. Never the whole table: the runtime materialises a missing `ai_tuning`
 * as an inline table on the first key, while a rung whose last rule was
 * removed keeps its `[station.rating.ai_tuning]` header as an EMPTY STANDARD
 * table, which the catalog reads as no rules and a table-valued put would be
 * refused over. A per-key put enters both shapes. */
function ruleEdits(path, before, after) {
  const removed = before.filter(rule => !after.includes(rule));
  const added = after.filter(rule => !before.includes(rule));
  return [...removed.map(rule => ({ op: 'remove', path: [...path, 'ai_tuning', rule] })),
    ...added.map(rule => ({ op: 'put', path: [...path, 'ai_tuning', rule], value_source: '{}' }))];
}

/** The exact-source edits that take one station's complexity from its reading
 * to the form, empty when nothing changed. Scalar and list edits on existing
 * rungs come first under their read indices, whole-rung removals follow by
 * descending index, and new rungs are appended last, so no edit shifts the
 * index a later one names. */
export function planRatingEdits({ station, stationIndex = station?.index, form }) {
  if (!Number.isInteger(stationIndex)) throw new Error('workshop.inspector_refused');
  const base = ['station', stationIndex];
  const live = (form?.ratings || []).filter(rung => !rung.removed);
  const names = new Set();
  for (const rung of live) {
    const name = String(rung.name ?? '').trim();
    if (!name) throw new Error('workshop.definitions.invalid_name');
    if (names.has(name)) throw new Error('workshop.definitions.rung_exists');
    names.add(name);
  }
  const edits = [];
  const current = station.visiting_rating ? decodeTomlString(station.visiting_rating.source) : '';
  const wanted = String(form?.visiting_rating ?? '').trim();
  if (wanted !== current) {
    if (!wanted) edits.push({ op: 'remove', path: [...base, 'visiting_rating'] });
    else edits.push({ op: station.visiting_rating ? 'set' : 'put', path: [...base, 'visiting_rating'], value_source: tomlBasicString(wanted) });
  }
  for (const rung of live) {
    if (rung.index == null) continue;
    const original = (station.ratings || []).find(entry => entry.index === rung.index);
    if (!original) throw new Error('workshop.inspector_stale');
    const path = [...base, 'rating', rung.index];
    const name = String(rung.name).trim();
    if (name !== original.name) edits.push({ op: 'set', path: [...path, 'name'], value_source: tomlBasicString(name) });
    edits.push(...arrayEdits([...path, 'automated_systems'], (original.automated_systems || []).map(entry => entry.id),
      rung.automated_systems || []));
    edits.push(...ruleEdits(path, (original.ai_tuning || []).map(entry => entry.rule), rung.ai_rules || []));
  }
  const removed = (form?.ratings || []).filter(rung => rung.removed && rung.index != null).sort((a, b) => b.index - a.index);
  for (const rung of removed) edits.push({ op: 'remove', path: [...base, 'rating', rung.index] });
  for (const rung of live.filter(entry => entry.index == null)) {
    const fields = [['name', tomlBasicString(String(rung.name).trim())], ['automated_systems', stringArray(rung.automated_systems || [])]];
    if (rung.ai_rules?.length) fields.push(['ai_tuning', rulesTable(rung.ai_rules)]);
    edits.push({ op: 'append_table', path: [...base, 'rating'], fields });
  }
  return edits;
}

/** The catalog findings that point at one line of one member. */
export function findingsAt(catalog, file, line) {
  return (catalog?.findings || []).filter(finding => finding.file === file && finding.line === line);
}

/** Draft members first, so an author's own definitions lead the selector and
 * the read-only base and pack entries follow in their catalog order. */
export function draftFirst(entries) {
  return [...(entries || [])].sort((a, b) => Number(b.origin === 'draft') - Number(a.origin === 'draft'));
}
