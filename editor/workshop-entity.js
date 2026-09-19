/** Entity template and fragment composition over the Workshop's source owner
 * (issue #1476). No filesystem, live ECS, TOML serializer or separate history:
 * the runtime resolves the template, says who authored each field and which
 * components it supports, this module turns the Includes, Components and Fields
 * forms into the structural edits the runtime applies, and every change lands
 * in the draft as one exact-source edit — refused by the runtime, with the
 * source untouched, when it would introduce a missing, cyclic, self or
 * disallowed include, an unsupported component, or a template that no longer
 * parses. Nothing here decides what a runtime type means: a component's default
 * text and an inherited field's value are the runtime's own serialisations,
 * carried through, never composed here (contract D2 and D5). The readings,
 * snapshots and stale checks are the definitions module's: the candidate is the
 * same set of text members either way. */
import { tomlBasicString } from './workshop-definitions.js';

export const ENTITY_DIRECTORY = 'assets/entities/';
/** An include may only name an entity template: the resolver reads nothing else
 * and the pack archive gate carries nothing else under that directory. */
export const ENTITY_PATH = /^assets\/entities\/(?:[^/]+\/)*[^/]+\.toml$/;

/** A refusal the panel can show: a string id, plus the offending value. */
function refusal(id, detail = '') {
  const error = new Error(id);
  error.detail = detail;
  return error;
}

const trimmed = value => String(value ?? '').trim();
const stringArray = values => `[${values.map(tomlBasicString).join(', ')}]`;

/** The text an `includes` entry carries for a chosen member: its path relative
 * to the DECLARING template's own directory, which is how the resolver reads it
 * (`canonical_include_path` joins the entry onto that directory). Computed from
 * the two paths rather than canonicalising arbitrary text, so no copy of the
 * resolver's rules lives here. */
export function authoredInclude(declaring, target) {
  const directory = String(declaring ?? '').slice(0, String(declaring ?? '').lastIndexOf('/') + 1);
  const path = String(target ?? '');
  if (directory && path.startsWith(directory)) return path.slice(directory.length);
  const from = directory.split('/').filter(Boolean);
  const to = path.split('/');
  let shared = 0;
  while (shared < from.length && shared < to.length - 1 && from[shared] === to[shared]) shared += 1;
  return [...Array(from.length - shared).fill('..'), ...to.slice(shared)].join('/');
}

/** The Includes form: the entries the template declares, in merge order, each
 * carrying the text the document holds, the member it resolves to and where
 * that member is defined. The authored text is what an edit writes; the
 * canonical path is what self and duplicate are judged on, because two
 * different spellings can name one member. */
export function includesForm(composition) {
  return { includes: (composition?.includes || []).map(entry => ({ authored: entry.authored ?? '',
    canonical: entry.canonical ?? null, origin: entry.origin ?? null, line: entry.line ?? null })) };
}

/** An include the panel adds for a chosen fragment: the runtime already said
 * this choice neither is the template itself nor closes a cycle. It has no line
 * until an edit lands, so the findings a line carries are simply none yet. */
export function newInclude(path, choice) {
  return { authored: authoredInclude(path, choice?.path ?? choice), canonical: choice?.path ?? choice,
    origin: choice?.origin ?? null, line: null };
}

/** Swap the include at `position` with its neighbour in `direction` (-1 up,
 * +1 down), in the form only. Returns the position it moved to, or null when it
 * is already at that end. Merge order is the array's order, so a move is a
 * `set` on both slots rather than a remove and an append: the entry's comments
 * and blank lines stay with their positions. */
export function moveInclude(form, position, direction) {
  const list = form?.includes || [];
  const target = position + direction;
  if (position < 0 || position >= list.length || target < 0 || target >= list.length) return null;
  [list[position], list[target]] = [list[target], list[position]];
  return target;
}

/** Fragments this template could still include: what the runtime offers (every
 * entity template but itself and anything that would cycle) minus what the
 * form already lists. */
export function includeChoices(composition, listed = (composition?.includes || []).map(entry => entry.canonical)) {
  return (composition?.fragment_choices || []).filter(entry => !listed.includes(entry.path));
}

/** The rule each entry of an include list breaks, as a multiset keyed by the
 * rule and the member it names — the shape entity.rs's `introduced` compares,
 * for the same reason: a violation the document ALREADY carried must not refuse
 * an edit that does not add one. */
function includeViolations(entries, path) {
  const counts = new Map();
  const seen = new Set();
  for (const entry of entries) {
    const authored = trimmed(entry?.authored);
    const canonical = entry?.canonical || authored;
    const rule = !authored ? ['empty', 'includes']
      : !ENTITY_PATH.test(canonical) ? ['disallowed', authored]
        : canonical === path ? ['self', authored]
          : seen.has(canonical) ? ['duplicate', authored]
            : null;
    if (!rule) { seen.add(canonical); continue; }
    const key = JSON.stringify([rule[0], canonical]);
    if (!counts.has(key)) counts.set(key, []);
    counts.get(key).push(rule);
  }
  return counts;
}

/** The exact-source edits that take `includes` from the reading to the form,
 * empty when nothing changed. A position whose text changed is a `set`, so a
 * reorder rewrites values in place; entries past the new end are removed by
 * descending index; new entries are inserted at the end. An empty reading is
 * written as a whole array through `put`, because the catalog cannot say
 * whether an empty list is an absent key (`includes` defaults) and `insert`
 * needs the array. Self, duplicate and non-entity paths are refused here as
 * string ids; a missing fragment or a cycle over the graph of the draft and its
 * dependencies is the runtime's to refuse.
 *
 * Only a violation this edit ADDS is refused, exactly as the runtime's own
 * `introduced` rule does: a draft whose `includes` was hand-edited to hold a
 * path outside `assets/entities/`, a self-reference or a duplicate must still
 * let its OTHER entries be reordered or removed, one edit at a time. */
export function planIncludeEdits({ composition, form }) {
  const wanted = (form?.includes || []).map(entry => ({ ...entry, authored: trimmed(entry.authored) }));
  const carried = includeViolations(composition?.includes || [], composition?.path);
  for (const [key, list] of includeViolations(wanted, composition?.path)) {
    const already = carried.get(key)?.length ?? 0;
    if (already >= list.length) continue;
    // The first one this edit adds beyond what the document already carried.
    const [rule, detail] = list[already];
    throw refusal(`workshop.entity.refused.${rule}`, detail);
  }
  const before = (composition?.includes || []).map(entry => entry.authored ?? '');
  const after = wanted.map(entry => entry.authored);
  if (before.join('\n') === after.join('\n')) return [];
  if (!before.length) return [{ op: 'put', path: ['includes'], value_source: stringArray(after) }];
  const edits = [];
  for (let index = 0; index < Math.min(before.length, after.length); index += 1) {
    if (before[index] !== after[index]) {
      edits.push({ op: 'set', path: ['includes', index], value_source: tomlBasicString(after[index]) });
    }
  }
  for (let index = before.length - 1; index >= after.length; index -= 1) edits.push({ op: 'remove', path: ['includes', index] });
  for (let index = before.length; index < after.length; index += 1) {
    edits.push({ op: 'insert', path: ['includes'], index, value_source: tomlBasicString(after[index]) });
  }
  return edits;
}

/** The runtime's own default text for a component, or null when it has none.
 * The catalog says both things: `skeleton` is the availability flag and
 * `skeleton_source` is the default serialised as ONE inline TOML value, which
 * is what an exact-source `put` needs. Either field is read as the text, so a
 * build that carries only the flag offers no Add and the panel says so rather
 * than composing a default of its own — deciding what a component's default IS
 * belongs to the runtime type (contract D2). */
export function componentSkeleton(component) {
  for (const candidate of [component?.skeleton, component?.skeleton_source]) {
    if (typeof candidate === 'string' && candidate.trim()) return candidate;
  }
  return null;
}

/** Whether the Components form may offer Add for this key: absent from the
 * effective template, and the runtime has a default to write. */
export const componentAddable = component => !component?.local && !component?.inherited_from
  && componentSkeleton(component) !== null;

/** The exact-source edit that adds a component from the runtime's own default.
 * One `put` of one value, so the rest of the document is untouched. */
export function planComponentAdd({ composition, key }) {
  const component = (composition?.components || []).find(entry => entry.key === key);
  if (!component) throw refusal('workshop.entity.refused.unsupported', key);
  if (component.local) throw refusal('workshop.entity.refused.local', key);
  const skeleton = componentSkeleton(component);
  if (!skeleton) throw refusal('workshop.entity.refused.no_skeleton', key);
  return [{ op: 'put', path: [key], value_source: skeleton }];
}

/** The exact-source edit that removes a LOCAL component table. An inherited
 * component has no local text to remove and no tombstone the merge understands
 * for a whole table, so it is refused with the sentence that says what to do
 * instead: materialise it, then edit it (contract B). */
export function planComponentRemove({ composition, key }) {
  const component = (composition?.components || []).find(entry => entry.key === key);
  if (!component) throw refusal('workshop.entity.refused.unsupported', key);
  if (!component.local) {
    if (component.inherited_from) throw refusal('workshop.entity.refused.inherited', component.inherited_from);
    throw new Error('workshop.inspector_stale');
  }
  return [{ op: 'remove', path: [key] }];
}

/** One provenance key without the quotes provenance wrapped it in. */
const unquoteKey = part => (part.length > 1 && part.startsWith('"') && part.endsWith('"')
  ? part.slice(1, -1) : part);

/** The document segments a provenance address names, or null when it addresses
 * a keyed array entry (`system[id=helm-thrust].ai_only`). Those arrays are
 * reconciled by key rather than by position, so the local index an edit would
 * need is not in the address — authoring them is #1481's, and the panel shows
 * such a field read-only instead of guessing.
 *
 * The split tracks QUOTING, exactly as entity.rs's `split_address` does:
 * `include_resolve`'s `join_field` quotes any key carrying a dot, a bracket, an
 * equals or a space, so `"odd.key".child` is two segments and not three. Split
 * naively, such a row named a key nothing authors and every Apply on it was
 * refused with the catch-all sentence. */
export function fieldSegments(address) {
  const parts = [];
  let current = '';
  let quoted = false;
  for (const character of String(address ?? '')) {
    if (character === '"') { quoted = !quoted; current += character; continue; }
    if (quoted) { current += character; continue; }
    if (character === '.') { parts.push(current); current = ''; continue; }
    if (character === '[' || character === ']') return null;
    current += character;
  }
  if (quoted) return null;
  parts.push(current);
  const segments = parts.map(unquoteKey);
  // A quote that is not a whole key's own wrapper is not something provenance
  // emits, and naming such a key would plan an edit the document cannot locate.
  return segments.some(segment => segment === '' || segment.includes('"')) ? null : segments;
}

/** The TOML types the exact-source `set` route can replace. It refuses an array
 * and a table outright — contract B says `set` on local SCALARS — so a row
 * holding one must not offer a text box whose every Apply is refused. Writing a
 * list or a sub-table is a source edit, not a field edit. */
const STRUCTURED_KINDS = Object.freeze(['array', 'table']);
export const fieldIsScalar = field => !STRUCTURED_KINDS.includes(String(field?.kind ?? ''));

/** Whether the Fields form may offer Materialise for this row: the value is
 * inherited, and the runtime said the local document can name its address at all
 * (`FieldView::materialisable`) — a field inside a keyed array entry this
 * template does not author is refused by the runtime, so the row says why
 * instead of holding a button whose every press fails. A catalog that carries no
 * such flag offers the control as before rather than hiding every one of them. */
export const fieldIsMaterialisable = field => Boolean(field) && !field?.local
  && field?.materialisable !== false;

/** Whether the Fields form may let this row be typed into: the value is the
 * local document's own, its address is one an edit can name, and it is a scalar
 * the `set` route can replace. */
export const fieldIsEditable = field => Boolean(field?.local) && fieldSegments(field?.address) !== null
  && fieldIsScalar(field);

/** The component a field belongs to: the address's first segment, whether the
 * rest is dotted or keyed. */
export function fieldComponent(address) {
  const text = String(address ?? '');
  const cuts = [text.indexOf('.'), text.indexOf('[')].filter(index => index >= 0);
  return cuts.length ? text.slice(0, Math.min(...cuts)) : text;
}

/** The fields grouped by the component that owns them, in the catalog's own
 * order — provenance is a BTreeMap, so this is deterministic. */
export function fieldGroups(composition) {
  const groups = new Map();
  for (const field of composition?.fields || []) {
    const key = fieldComponent(field.address);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(field);
  }
  return [...groups.entries()].map(([key, fields]) => ({ key, fields }));
}

/** The Fields form: every field's value as the row holds it, keyed by address.
 * An inherited row carries the runtime's serialisation of the effective value
 * and is never written back; a local row carries the exact span text. */
export function fieldsForm(composition) {
  const values = {};
  for (const field of composition?.fields || []) values[field.address] = field.value_source ?? '';
  return { values };
}

/** The exact-source edits that take the LOCAL scalars from the reading to the
 * form, empty when nothing changed. An inherited field is never written: until
 * it is materialised it has no local text to set (criterion 2). */
export function planFieldEdits({ composition, form }) {
  const edits = [];
  for (const field of composition?.fields || []) {
    const wanted = form?.values?.[field.address];
    if (wanted == null || wanted === (field.value_source ?? '')) continue;
    if (!field.local) throw refusal('workshop.entity.refused.inherited', field.source ?? field.address);
    const path = fieldSegments(field.address);
    if (!path) throw refusal('workshop.entity.refused.keyed', field.address);
    if (!fieldIsScalar(field)) throw refusal('workshop.entity.refused.structured', field.address);
    if (!trimmed(wanted)) throw refusal('workshop.entity.refused.empty', field.address);
    edits.push({ op: 'set', path, value_source: wanted });
  }
  return edits;
}

/** The templates the selector offers: the draft's own entity members, then
 * every template the runtime named that is not one of them. A dependency
 * template is listed so its composition can be READ — it is authored where it
 * is defined, so the panel shows it read-only with its origin. */
export function templateChoices(paths, composition = null) {
  const entries = (paths || []).filter(path => ENTITY_PATH.test(path)).sort((a, b) => a.localeCompare(b))
    .map(path => ({ path, origin: 'draft' }));
  const seen = new Set(entries.map(entry => entry.path));
  const extra = [...(composition?.fragment_choices || [])];
  if (composition?.path) extra.push({ path: composition.path, origin: composition.origin ?? null });
  for (const entry of extra) {
    if (!entry?.path || seen.has(entry.path)) continue;
    seen.add(entry.path);
    entries.push({ path: entry.path, origin: entry.origin ?? null });
  }
  return entries;
}

/** The runtime refuses an entity edit with `<rule>: <detail>` — the rule is the
 * finding category the same violation reports as (entity.rs `refusal`), the
 * detail names the offending value. The panel shows a localised sentence per
 * CATEGORY with that message as the detail, so the rule is read off the prefix;
 * the word patterns beneath are the fallback for a message with no such prefix
 * (a document-level refusal, a runtime the panel has not been rebuilt against),
 * ordered from the most specific: a cycle message also names the include it
 * closes on, a self-include names the template twice, and serde's own
 * "unknown field" is an unsupported component before it is a missing anything. */
const REFUSAL_RULES = Object.freeze({
  'include-missing': 'missing',
  'include-cycle': 'cycle',
  'include-self': 'self',
  'include-disallowed': 'disallowed',
  'include-duplicate': 'duplicate',
  'component-unsupported': 'unsupported',
  'component-inherited': 'inherited',
  'entity-invalid': 'invalid',
  'entity-unresolvable': 'missing',
  'materialise-local': 'local',
  // NOT `missing`, which is the include-missing sentence: this rule is raised
  // for an address the resolved template does not carry, one that cannot be
  // written as TOML, and one that would need more than one new local table —
  // none of them a fragment that is absent from the draft.
  'materialise-unknown-address': 'address',
  'materialise-keyed-entry': 'keyed',
});
const REFUSAL_CATEGORIES = Object.freeze([
  ['cycle', /cycl/i],
  ['self', /\bitself\b|self[- ]refer|includes? itself/i],
  ['inherited', /inherit/i],
  ['unsupported', /unknown field|unsupported|not a (supported )?component|expected one of/i],
  ['local', /already (local|in the local|authored locally)/i],
  ['keyed', /reconcile[sd]? by key|keyed array|without the key/i],
  ['duplicate', /duplicat|twice|already (lists|names|includes|included)/i],
  ['invalid', /does not parse|no longer parses|failed to parse|invalid/i],
  ['missing', /missing|not found|not present|absent|does not exist|no such|unresolv|not in (the )?(candidate|draft|dependenc)/i],
  ['disallowed', /disallow|not allowed|not permitted|forbidden|outside|must (be|name) (an? )?(path )?(under )?assets\/entities/i],
]);

export function refusalMessage(error) {
  if (typeof error === 'string') return error;
  return String(error?.message ?? error ?? '');
}

/** A refusal of the DOCUMENT rather than of an entity rule: the member is not
 * in the draft at all, so the panel's reading has parted from it. It names no
 * rule the entity strings describe and must not borrow one's words — "missing"
 * would otherwise read as a missing fragment — so it is the shared stale
 * sentence, which is also the one thing an author can act on. */
const DOCUMENT_RULES = Object.freeze(['unknown-document']);

export function refusalStringId(message, fallback = 'workshop.entity.refused.other') {
  const text = refusalMessage(message);
  const prefix = /^([a-z-]+):\s/.exec(text)?.[1];
  if (DOCUMENT_RULES.includes(prefix)) return 'workshop.inspector_stale';
  // The exact-source owner's own refusal of a stale `expected_source`, which
  // carries no rule prefix and is the same thing an author must do about it.
  if (/document changed/i.test(text)) return 'workshop.inspector_stale';
  const rule = REFUSAL_RULES[prefix];
  if (rule) return `workshop.entity.refused.${rule}`;
  const found = REFUSAL_CATEGORIES.find(([, pattern]) => pattern.test(text));
  // The default fallback CARRIES the runtime's sentence: an unmapped refusal is
  // the one case where its own words are all the author has, and the shared
  // `workshop.inspector_refused` has no {detail} to put them in (#1475).
  return found ? `workshop.entity.refused.${found[0]}` : fallback;
}
