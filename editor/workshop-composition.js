/** World composition and scenario entry points over the Workshop's source
 * owner (issue #1475). No filesystem, live ECS, TOML serializer or separate
 * history: the runtime reads the draft and its read-only dependencies into a
 * composition catalog, this module turns the Roots and Worlds forms into the
 * structural edits the runtime applies, and every change lands in the draft as
 * one exact-source edit — refused by the runtime, with the source untouched,
 * when it would introduce a missing, cyclic, duplicate or disallowed reference.
 * The readings, snapshots and stale checks are the definitions module's: the
 * candidate is the same set of text members either way. */
import { tomlBasicString, factionSlug } from './workshop-definitions.js';

export const WORLD_DIRECTORY = 'assets/worlds/';
/** A root or extra world may only name a world file: the loader reads nothing
 * else, and the pack archive gate carries nothing else under that directory. */
export const WORLD_PATH = /^assets\/worlds\/[^/]+\.toml$/;

/** A refusal the panel can show: a string id, plus the offending value. */
function refusal(id, detail = '') {
  const error = new Error(id);
  error.detail = detail;
  return error;
}

const trimmed = value => String(value ?? '').trim();
const stringArray = values => `[${values.map(tomlBasicString).join(', ')}]`;

function topLevelValueSpan(source, key) {
  let offset = 0;
  for (const line of source.match(/.*(?:\r\n|\n|$)/g) || []) {
    if (/^\s*\[/.test(line)) return null;
    const match = new RegExp(`^\\s*${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*=\\s*`).exec(line);
    if (!match) { offset += line.length; continue; }
    const start = offset + match[0].length;
    let index = start, quote = null, escaped = false, comment = false, square = 0, curly = 0;
    while (index < source.length) {
      const character = source[index];
      if (comment) { if (character === '\r' || character === '\n') comment = false; }
      else if (quote) {
        if (escaped) escaped = false;
        else if (character === '\\' && quote === '"') escaped = true;
        else if (character === quote) quote = null;
      } else if (character === '"' || character === "'") quote = character;
      else if (character === '#') { if (square === 0 && curly === 0) break; comment = true; }
      else if (character === '[') square += 1;
      else if (character === ']') square -= 1;
      else if (character === '{') curly += 1;
      else if (character === '}') curly -= 1;
      else if ((character === '\r' || character === '\n') && square === 0 && curly === 0) break;
      index += 1;
    }
    while (index > start && /\s/.test(source[index - 1])) index -= 1;
    return { start, end: index };
  }
  return null;
}

/** Exact-source compatibility used by the spatial authoring transaction. */
export function setExtraWorlds(source, values) {
  if (!Array.isArray(values) || values.some(value => !WORLD_PATH.test(value)) || new Set(values).size !== values.length) {
    throw new Error('invalid-composition-reference');
  }
  const replacement = stringArray(values);
  const span = topLevelValueSpan(source, 'extra_worlds');
  if (span) return source.slice(0, span.start) + replacement + source.slice(span.end);
  const newline = source.includes('\r\n') ? '\r\n' : '\n';
  const prefix = `extra_worlds = ${replacement}${newline}`;
  return source.startsWith('\uFEFF') ? `\uFEFF${prefix}${source.slice(1)}` : prefix + source;
}

/** The edits that turn `before` into `after` inside an array of strings:
 * removals first, by descending index so each one still names the element it
 * meant, then appends. An empty reading is written as a whole array through
 * `put`, because the catalog cannot say whether an empty list is an absent key
 * (`ships` and `extra_worlds` both default) and `insert` needs the array. The
 * same shape the definitions planner uses for enemies; kept here rather than
 * shared because that module owns no composition and this one owns no
 * definitions. */
function arrayEdits(path, before, after) {
  const kept = before.filter(value => after.includes(value));
  const added = after.filter(value => !before.includes(value));
  if (!before.length) return added.length ? [{ op: 'put', path, value_source: stringArray(added) }] : [];
  const edits = [];
  for (let index = before.length - 1; index >= 0; index -= 1) {
    if (!after.includes(before[index])) edits.push({ op: 'remove', path: [...path, index] });
  }
  added.forEach((value, offset) => edits.push({ op: 'insert', path, index: kept.length + offset, value_source: tomlBasicString(value) }));
  return edits;
}

/** The world file a new world of this title is created at: the title's slug
 * under assets/worlds/, the only directory a root or extra world may name. */
export function worldSlugPath(title) {
  const slug = factionSlug(title);
  if (!slug) throw new Error('workshop.composition.invalid_title');
  return `${WORLD_DIRECTORY}${slug}.toml`;
}

/** What a world is called on screen: its `[global] title` when it has one,
 * else the file stem — a world without a title is still a world. */
export function worldTitle(world) {
  const title = trimmed(world?.title);
  if (title) return title;
  const path = String(world?.path ?? '');
  return path.slice(path.lastIndexOf('/') + 1).replace(/\.toml$/, '');
}

/** The Roots form: every manifest scenario as the values its controls hold,
 * in manifest order. `index` is the entry's slot in the `[[scenario]]` array;
 * a root added in the form has none yet and becomes an `append_table`. */
export function rootsForm(manifest) {
  return (manifest?.scenarios || []).map(entry => ({ index: entry.index, id: entry.id ?? '', world: entry.world ?? '',
    label: entry.label ?? '', ships: (entry.ships || []).map(ship => ship.path), removed: false }));
}

export function newRoot(id, world) {
  return { index: null, id: trimmed(id), world: trimmed(world), label: '', ships: [], removed: false };
}

/** Swap the live root at `position` with its nearest live neighbour in
 * `direction` (-1 up, +1 down), in the form only. Returns the position it moved
 * to, or null when it is already at that end. The plan expresses the move as
 * `set` edits on both slots' scalar fields, never as a remove-and-append: the
 * manifest's comments and blank lines stay with their positions, and an entry
 * cannot be inserted mid-array through the edit vocabulary anyway. */
export function moveRoot(form, position, direction) {
  const live = form.map((entry, at) => (entry.removed ? null : at)).filter(at => at != null);
  const rank = live.indexOf(position);
  if (rank < 0) return null;
  const target = live[rank + direction];
  if (target == null) return null;
  [form[position], form[target]] = [form[target], form[position]];
  return target;
}

/** The ships a root may offer: what its world declares in `[[available_ships]]`,
 * read from the catalog's world entry, or from the reading's own scenario when
 * the world is the one it was read with. */
export function offeredShips(catalog, world, scenario = null) {
  const entry = (catalog?.worlds || []).find(candidate => candidate.path === world);
  if (entry) return (entry.available_ships || []).map(ship => ship.template_path);
  return scenario && scenario.world === world ? [...(scenario.offered_ships || [])] : [];
}

function slotEdits(base, original, entry) {
  const edits = [];
  const scalar = (key, value, had) => {
    // An absent key and an empty field are the same thing to the form.
    if (value === (had ?? '')) return;
    if (!value) edits.push({ op: 'remove', path: [...base, key] });
    else edits.push({ op: had == null ? 'put' : 'set', path: [...base, key], value_source: tomlBasicString(value) });
  };
  scalar('id', trimmed(entry.id), original.id ?? null);
  scalar('world', trimmed(entry.world), original.world ?? null);
  scalar('label', trimmed(entry.label), original.label ?? null);
  edits.push(...arrayEdits([...base, 'ships'], (original.ships || []).map(ship => ship.path), entry.ships || []));
  return edits;
}

/** The exact-source edits that take the manifest's roots from the reading to
 * the form, empty when nothing changed. The kept roots fill the surviving
 * slots in form order — so a reorder is `set`s on the slots whose content
 * moved — then whole-root removals follow by descending index, and new roots
 * are appended last, so no edit shifts the slot a later one names. Refusals
 * the form can see itself (an empty id or world, a path outside
 * assets/worlds/, two roots with one id) are string ids; everything cross-file
 * (a missing world, a ship the world does not offer) is the runtime's to
 * refuse. */
export function planScenarioEdits({ manifest, form }) {
  const originals = manifest?.scenarios || [];
  const live = (form || []).filter(entry => !entry.removed);
  const ids = new Set();
  for (const entry of live) {
    const id = trimmed(entry.id), world = trimmed(entry.world);
    if (!id) throw refusal('workshop.composition.refused.empty', 'id');
    if (!world) throw refusal('workshop.composition.refused.empty', 'world');
    if (!WORLD_PATH.test(world)) throw refusal('workshop.composition.refused.disallowed', world);
    if (ids.has(id)) throw refusal('workshop.composition.refused.duplicate', id);
    ids.add(id);
  }
  const removed = (form || []).filter(entry => entry.removed && entry.index != null).map(entry => entry.index);
  const surviving = originals.map(entry => entry.index).filter(index => !removed.includes(index)).sort((a, b) => a - b);
  const kept = live.filter(entry => entry.index != null);
  // A root the reading no longer has is a stale form, not an edit at that slot.
  if (kept.length !== surviving.length || kept.some(entry => !originals.some(original => original.index === entry.index))) {
    throw new Error('workshop.inspector_stale');
  }
  const edits = [];
  kept.forEach((entry, rank) => {
    const slot = surviving[rank];
    edits.push(...slotEdits(['scenario', slot], originals.find(original => original.index === slot), entry));
  });
  for (const index of [...removed].sort((a, b) => b - a)) edits.push({ op: 'remove', path: ['scenario', index] });
  for (const entry of live.filter(candidate => candidate.index == null)) {
    const fields = [['id', tomlBasicString(trimmed(entry.id))], ['world', tomlBasicString(trimmed(entry.world))]];
    if (trimmed(entry.label)) fields.push(['label', tomlBasicString(trimmed(entry.label))]);
    if (entry.ships?.length) fields.push(['ships', stringArray(entry.ships)]);
    edits.push({ op: 'append_table', path: ['scenario'], fields });
  }
  return edits;
}

/** The Worlds form for one world: the extra worlds it declares. */
export function worldForm(world) {
  return { extra_worlds: (world?.extra_worlds || []).map(entry => entry.path) };
}

/** Worlds this one could still list as an extra world: not itself, and none
 * it already lists (the form's current list, which may differ from the
 * reading's). */
export function extraWorldChoices(choices, world, listed = (world?.extra_worlds || []).map(entry => entry.path)) {
  return (choices?.worlds || []).filter(entry => entry.path !== world?.path && !listed.includes(entry.path));
}

/** The exact-source edits that take a world's `extra_worlds` from the reading
 * to the form, empty when nothing changed. Self, duplicate and non-world paths
 * are refused here as string ids; a missing child or a cycle over the graph of
 * the draft and its dependencies is the runtime's to refuse. */
export function planExtraWorldEdits({ world, form }) {
  const wanted = (form?.extra_worlds || []).map(trimmed);
  const seen = new Set();
  for (const path of wanted) {
    if (!WORLD_PATH.test(path)) throw refusal('workshop.composition.refused.disallowed', path);
    if (path === world?.path) throw refusal('workshop.composition.refused.self', path);
    if (seen.has(path)) throw refusal('workshop.composition.refused.duplicate', path);
    seen.add(path);
  }
  return arrayEdits(['extra_worlds'], (world?.extra_worlds || []).map(entry => entry.path), wanted);
}

/** The runtime refuses a compose request with `<rule>: <detail>` — the rule
 * is the finding category the same violation reports as (composition.rs
 * `refusal`), the detail names the offending value. The panel shows a
 * localised sentence per CATEGORY with that message as the detail, so the
 * rule is read off the prefix; the word patterns beneath are the fallback for
 * a message with no such prefix (a document-level refusal, a runtime the
 * panel has not been rebuilt against), ordered from the most specific: a
 * cycle message also names the world it closes on, a self-reference names the
 * world twice, and a missing world is "not in" the candidate before it is
 * "not a path". Anything unrecognised is the generic refusal. */
const REFUSAL_RULES = Object.freeze({
  'duplicate-scenario-id': 'duplicate',
  'extra-worlds-duplicate': 'duplicate',
  'invalid-manifest-entry': 'empty',
  'scenario-world-disallowed': 'disallowed',
  'extra-worlds-disallowed': 'disallowed',
  'missing-scenario-world': 'missing',
  'extra-worlds-missing': 'missing',
  'unknown-scenario-ship': 'ship',
  'manifest-header-removed': 'header',
  'extra-worlds-self': 'self',
  'extra-worlds-cycle': 'cycle',
});
const REFUSAL_CATEGORIES = Object.freeze([
  ['cycle', /cycl/i],
  ['self', /\bitself\b|self[- ]refer/i],
  ['duplicate', /duplicat|twice|already (lists|names|declares)/i],
  ['ship', /\boffer/i],
  ['header', /\[pack\]|\[content\]|header/i],
  ['empty', /\bempty\b|\bblank\b/i],
  ['missing', /missing|not found|not present|absent|does not exist|no such|unknown|unresolv|not in (the )?(candidate|draft|dependenc)|neither the/i],
  ['disallowed', /disallow|not allowed|not permitted|forbidden|outside|not an? [^.]*assets\/worlds|must (be|name) (an? )?(path )?(under )?assets\/worlds/i],
]);

export function refusalMessage(error) {
  if (typeof error === 'string') return error;
  return String(error?.message ?? error ?? '');
}

/** A refusal of the DOCUMENT rather than of a composition rule: the member is
 * not in the draft at all, so the panel's reading has parted from it. It names
 * no rule the composition strings describe and must not borrow one's words —
 * "unknown" would otherwise read as a missing world — so it is the shared stale
 * sentence, which is also the one thing an author can act on. */
const DOCUMENT_RULES = Object.freeze(['unknown-document']);

export function refusalStringId(message, fallback = 'workshop.composition.refused.other') {
  const text = refusalMessage(message);
  const prefix = /^([a-z-]+):\s/.exec(text)?.[1];
  // A document-level refusal says the panel's reading has parted from the
  // draft, which is what the shared stale sentence already tells an author to
  // do about it. It is not a composition rule, so it never borrows one's words.
  if (DOCUMENT_RULES.includes(prefix)) return 'workshop.inspector_stale';
  // The exact-source owner's own refusal of a stale `expected_source`, which
  // carries no rule prefix and is the same thing an author must do about it.
  if (/document changed/i.test(text)) return 'workshop.inspector_stale';
  const rule = REFUSAL_RULES[prefix];
  if (rule) return `workshop.composition.refused.${rule}`;
  const found = REFUSAL_CATEGORIES.find(([, pattern]) => pattern.test(text));
  // The default fallback CARRIES the runtime's sentence: an unmapped refusal is
  // the one case where its own words are all the author has, and the shared
  // `workshop.inspector_refused` has no {detail} to put them in.
  return found ? `workshop.composition.refused.${found[0]}` : fallback;
}
