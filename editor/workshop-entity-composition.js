import { parse, stringify } from 'smol-toml';
import { COMPONENT_SCHEMA, ENTITY_CONFIG_SECTIONS } from './component-schema.js';
import { getRawSectionDefaults } from './component-templates.js';
import { canonicalIncludePath, materialiseOverride, parseFieldPath, resolveTemplate } from './entity-includes.js';
import { WorkshopDocument } from './workshop-document.js';

const ENTITY_PREFIX = 'assets/entities/';
const entityPath = value => typeof value === 'string' && value.startsWith(ENTITY_PREFIX)
  && value.endsWith('.toml') && !value.includes('\\') && !value.split('/').includes('..');
const newlineOf = source => source.includes('\r\n') ? '\r\n' : '\n';
const quote = value => JSON.stringify(value);

function relativeIncludePath(declaring, target) {
  const from = declaring.split('/').slice(0, -1), to = target.split('/');
  let shared = 0;
  while (shared < from.length && shared < to.length && from[shared] === to[shared]) shared += 1;
  const relative = [...from.slice(shared).map(() => '..'), ...to.slice(shared)].join('/');
  if (!relative || canonicalIncludePath(declaring, relative) !== target) throw new Error('include-path-unavailable');
  return relative;
}

function parsed(source, path) {
  try { return parse(source); }
  catch (error) { throw new Error(`${path}: ${error.message}`); }
}

function valueEnd(source, start) {
  let index = start, quoteMark = null, escaped = false, comment = false, square = 0, curly = 0;
  while (index < source.length) {
    const character = source[index];
    if (comment) {
      if (character === '\r' || character === '\n') comment = false;
    } else if (quoteMark) {
      if (escaped) escaped = false;
      else if (character === '\\' && quoteMark === '"') escaped = true;
      else if (character === quoteMark) quoteMark = null;
    } else if (character === '"' || character === "'") quoteMark = character;
    else if (character === '#') {
      if (!square && !curly) break;
      comment = true;
    } else if (character === '[') square += 1;
    else if (character === ']') square -= 1;
    else if (character === '{') curly += 1;
    else if (character === '}') curly -= 1;
    else if ((character === '\r' || character === '\n') && !square && !curly) break;
    index += 1;
  }
  while (index > start && /\s/.test(source[index - 1])) index -= 1;
  return index;
}

function topLevelSpan(source, key) {
  let offset = 0;
  for (const line of source.match(/.*(?:\r\n|\n|$)/g) || []) {
    if (/^\uFEFF?[ \t]*\[/.test(line)) return null;
    const match = new RegExp(`^\\uFEFF?[ \\t]*${key}[ \\t]*=[ \\t]*`).exec(line);
    if (match) {
      const start = offset + match[0].length;
      return { lineStart: offset + (line.startsWith('\uFEFF') ? 1 : 0), start, end: valueEnd(source, start) };
    }
    offset += line.length;
  }
  return null;
}

function setIncludes(source, includes) {
  if (!Array.isArray(includes) || includes.some(value => typeof value !== 'string')
    || new Set(includes).size !== includes.length) throw new Error('invalid-includes');
  const replacement = `[${includes.map(quote).join(', ')}]`;
  const span = topLevelSpan(source, 'includes');
  if (span) return source.slice(0, span.start) + replacement + source.slice(span.end);
  const newline = newlineOf(source), prefix = `includes = ${replacement}${newline}`;
  return source.startsWith('\uFEFF') ? `\uFEFF${prefix}${source.slice(1)}` : prefix + source;
}

function header(source, section) {
  const escaped = section.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const expression = new RegExp(`^[ \\t]*\\[${escaped}\\][ \\t]*(?:#.*)?(?:\\r?\\n|$)`, 'gm');
  const match = expression.exec(source);
  if (!match) return null;
  const anyHeader = /^[ \t]*\[{1,2}[^\r\n]+?\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm;
  anyHeader.lastIndex = match.index + match[0].length;
  return { start: match.index, body: match.index + match[0].length,
    end: anyHeader.exec(source)?.index ?? source.length };
}

function insertTopLevelDotted(source, path, value) {
  const newline = newlineOf(source);
  const firstHeader = /^[ \t]*\[{1,2}[^\r\n]+?\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/m.exec(source);
  const at = firstHeader?.index ?? source.length;
  const before = source.slice(0, at), gap = !before.length || before.endsWith(newline) ? '' : newline;
  return before + gap + `${path} = ${tomlValue(value)}${newline}` + source.slice(at);
}

function arrayHeader(source, section, index) {
  const escaped = section.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const matches = [...source.matchAll(new RegExp(`^[ \\t]*\\[\\[${escaped}\\]\\][ \\t]*(?:#.*)?(?:\\r?\\n|$)`, 'gm'))];
  const match = matches[index];
  if (!match) return null;
  const anyHeader = /^[ \t]*\[{1,2}[^\r\n]+?\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm;
  anyHeader.lastIndex = match.index + match[0].length;
  return { start: match.index, body: match.index + match[0].length,
    end: anyHeader.exec(source)?.index ?? source.length };
}

function sectionSpans(source, section) {
  const escaped = section.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const headers = [...source.matchAll(new RegExp(`^[ \\t]*\\[{1,2}${escaped}(?:\\.[^\\]\\r\\n]+)?\\]{1,2}[ \\t]*(?:#.*)?(?:\\r?\\n|$)`, 'gm'))];
  if (!headers.length) return [];
  const all = [...source.matchAll(/^[ \t]*\[{1,2}[^\r\n]+?\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)];
  return headers.map(match => ({ start: match.index,
    end: all.find(candidate => candidate.index > match.index)?.index ?? source.length }));
}

function removeComponent(source, section) {
  const document = parsed(source, 'entity');
  if (!Object.prototype.hasOwnProperty.call(document, section)) throw new Error('component-not-local');
  const top = topLevelSpan(source, section);
  if (top) {
    let end = source.indexOf('\n', top.end);
    end = end < 0 ? source.length : end + 1;
    return source.slice(0, top.lineStart) + source.slice(end);
  }
  const spans = sectionSpans(source, section);
  if (!spans.length) throw new Error('component-source-unavailable');
  return spans.slice().reverse().reduce((result, span) => result.slice(0, span.start) + result.slice(span.end), source);
}

function defaultValue(field) {
  if ('default' in field) return structuredClone(field.default);
  if (field.enum?.length) return field.enum[0];
  if (field.optional) return undefined;
  if (field.type === 'number') return 0;
  if (field.type === 'boolean') return false;
  if (field.type === 'array' || field.type === 'array-of-tables') return [];
  if (field.type === 'subobject') return Object.fromEntries((field.subfields || []).map(child =>
    [child.key, defaultValue(child)]).filter(([, value]) => value !== undefined));
  return '';
}

function componentDefault(section) {
  const schema = COMPONENT_SCHEMA[section];
  if (!schema) throw new Error('unsupported-component');
  if (schema.arrayOfTables) {
    if (!schema.entryDefaults || !Object.keys(schema.entryDefaults).length) throw new Error('component-needs-input');
    return [structuredClone(schema.entryDefaults)];
  }
  const raw = getRawSectionDefaults(section);
  if (raw !== undefined && raw !== null && (typeof raw !== 'object' || Array.isArray(raw) || Object.keys(raw).length)) return raw;
  if (schema.fields.length === 1 && schema.fields[0].key === section) {
    const value = defaultValue(schema.fields[0]);
    if (value === undefined) throw new Error('component-needs-input');
    return value;
  }
  return Object.fromEntries(schema.fields.map(field => [field.key, defaultValue(field)])
    .filter(([, value]) => value !== undefined));
}

export const ADDABLE_ENTITY_COMPONENTS = Object.freeze(ENTITY_CONFIG_SECTIONS.filter(section => {
  try { return stringify({ [section]: componentDefault(section) }).trim().length > 0; }
  catch { return false; }
}));

function tomlValue(value) {
  if (typeof value === 'string') return quote(value);
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  if (typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return `[${value.map(tomlValue).join(', ')}]`;
  if (value && typeof value === 'object') return `{ ${Object.entries(value).map(([key, child]) =>
    `${/^[A-Za-z0-9_-]+$/.test(key) ? key : quote(key)} = ${tomlValue(child)}`).join(', ')} }`;
  throw new Error('unsupported-field-value');
}

function appendComponent(source, section, value = componentDefault(section)) {
  const document = parsed(source, 'entity');
  if (Object.prototype.hasOwnProperty.call(document, section)) throw new Error('duplicate-component');
  const fragment = stringify({ [section]: value }).trim();
  const newline = newlineOf(source);
  const normalized = fragment.replace(/\n/g, newline);
  const gap = !source.length || source.endsWith(`${newline}${newline}`) ? '' : source.endsWith(newline) ? newline : `${newline}${newline}`;
  return source + gap + normalized + newline;
}

function ownerFor(provenance, fieldPath, fallback) {
  const owners = new Set();
  for (const [path, step] of provenance.fields.entries()) {
    if (path === fieldPath || path.startsWith(`${fieldPath}.`) || path.startsWith(`${fieldPath}[`)) owners.add(step.source);
  }
  return owners.size === 1 ? [...owners][0] : owners.size ? 'mixed' : fallback;
}

function valueAtPath(value, path) {
  let cursor = value;
  for (const segment of parseFieldPath(path)) {
    cursor = cursor?.[segment.name];
    if (segment.selector) {
      if (!Array.isArray(cursor)) return undefined;
      cursor = segment.selector.key !== undefined
        ? cursor.find(entry => entry && String(entry[segment.selector.key]) === segment.selector.val)
        : cursor[segment.selector.index];
    }
  }
  return cursor;
}

function fieldsFor(resolved) {
  return [...resolved.provenance.fields.entries()]
    .filter(([path]) => path && path !== 'includes' && !path.startsWith('includes['))
    .map(([path, step]) => ({ path, section: parseFieldPath(path)[0]?.name,
      value: valueAtPath(resolved.value, path), owner: step.source }));
}

export function entityInventory(draft, dependencies = { base_files: {}, packs: [] }) {
  const files = new Map();
  const add = (source, origin, editable = false) => Object.entries(source || {}).forEach(([path, text]) => {
    if (entityPath(path) && typeof text === 'string') files.set(path, { path, text, origin, editable });
  });
  add(dependencies.base_files, 'base');
  for (const pack of dependencies.packs || []) add(pack.files, pack.id);
  add(Object.fromEntries(draft.paths().filter(path => entityPath(path) && !draft.isBinary(path)).map(path => [path, draft.read(path)])), 'draft', true);
  return [...files.values()].sort((a, b) => a.path.localeCompare(b.path));
}

export function inspectEntityComposition(draft, dependencies, path) {
  const inventory = entityInventory(draft, dependencies);
  const byPath = Object.fromEntries(inventory.map(row => [row.path, row.text]));
  if (!inventory.some(row => row.path === path && row.editable)) throw new Error('entity-not-editable');
  const result = resolveTemplate(path, byPath, parse);
  if (!result.ok) throw result.error;
  const own = parsed(byPath[path], path);
  const includes = Array.isArray(own.includes) ? [...own.includes] : [];
  return { path, includes, inventory, resolved: result.resolved,
    components: Object.keys(result.resolved.value).filter(key => key !== 'includes').map(section => ({
      section, owner: ownerFor(result.resolved.provenance, section, path), local: Object.prototype.hasOwnProperty.call(own, section),
    })), fields: fieldsFor(result.resolved) };
}

function materializeField(source, row) {
  const document = parsed(source, 'entity');
  const segments = parseFieldPath(row.path);
  if (!segments.length || row.owner === undefined) throw new Error('field-source-ambiguous');
  const first = segments[0], section = first.name;
  if (document[section] === undefined) {
    const override = materialiseOverride({}, row.path, row.value);
    return appendComponent(source, section, override[section]);
  }
  let local = document[section], table;
  if (first.selector) {
    if (!Array.isArray(local)) throw new Error('component-source-unavailable');
    const index = first.selector.key !== undefined
      ? local.findIndex(entry => entry && String(entry[first.selector.key]) === first.selector.val)
      : first.selector.index;
    if (index < 0 || index >= local.length) {
      const override = materialiseOverride({}, row.path, row.value);
      const fragment = stringify(override).trim().replace(/\n/g, newlineOf(source));
      const newline = newlineOf(source), gap = source.endsWith(newline) ? newline : `${newline}${newline}`;
      return source + gap + fragment + newline;
    }
    local = local[index]; table = arrayHeader(source, section, index);
  } else table = header(source, section);
  if (segments.length === 1 || !local || typeof local !== 'object') throw new Error('component-source-unavailable');
  let remaining = segments.slice(1);
  let nestedIndex = remaining.findIndex(segment => segment.selector);
  if (!first.selector) {
    const deepestPlain = nestedIndex >= 0 ? nestedIndex : remaining.length - 1;
    for (let depth = deepestPlain; depth > 0; depth -= 1) {
      const nestedTable = header(source, [section, ...remaining.slice(0, depth).map(segment => segment.name)].join('.'));
      if (!nestedTable) continue;
      for (const segment of remaining.slice(0, depth)) local = local?.[segment.name];
      table = nestedTable; remaining = remaining.slice(depth); break;
    }
    nestedIndex = remaining.findIndex(segment => segment.selector);
    if (!table && nestedIndex < 0) {
      const path = segments.map(segment => /^[A-Za-z0-9_-]+$/.test(segment.name) ? segment.name : quote(segment.name)).join('.');
      return insertTopLevelDotted(source, path, row.value);
    }
  }
  if (nestedIndex >= 0) {
    if (nestedIndex !== 0 || remaining.slice(1).some(segment => segment.selector)) {
      throw new Error('nested-array-materialization-unavailable');
    }
    const nested = remaining[0], entries = local[nested.name];
    if (entries !== undefined && !Array.isArray(entries)) throw new Error('component-source-unavailable');
    const localIndex = nested.selector.key !== undefined
      ? (entries || []).findIndex(entry => entry && String(entry[nested.selector.key]) === nested.selector.val)
      : nested.selector.index;
    const headerName = `${section}.${nested.name}`;
    if (localIndex < 0 || localIndex >= (entries || []).length) {
      const override = materialiseOverride({}, row.path, row.value);
      const serialized = stringify(override).trim().replace(/\n/g, newlineOf(source));
      const marker = `[[${headerName}]]`;
      const start = serialized.indexOf(marker);
      if (start < 0) throw new Error('field-source-unavailable');
      const newline = newlineOf(source), at = table?.end ?? source.length;
      const before = source.slice(0, at), gap = before.endsWith(newline) ? newline : `${newline}${newline}`;
      return before + gap + serialized.slice(start) + newline + source.slice(at);
    }
    let globalIndex = localIndex;
    if (first.selector && Array.isArray(document[section])) {
      const topIndex = document[section].indexOf(local);
      globalIndex += document[section].slice(0, topIndex)
        .reduce((count, entry) => count + (Array.isArray(entry?.[nested.name]) ? entry[nested.name].length : 0), 0);
    }
    table = arrayHeader(source, headerName, globalIndex);
    local = entries[localIndex]; remaining = remaining.slice(1);
    if (!table || !remaining.length) throw new Error('component-source-unavailable');
  }
  const firstLocal = local[remaining[0].name];
  if (firstLocal !== undefined && remaining.length > 1) {
    const key = remaining[0].name;
    const block = source.slice(table.body, table.end);
    const match = new RegExp(`^[ \\t]*${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[ \\t]*=[ \\t]*`, 'm').exec(block);
    if (!match || !firstLocal || typeof firstLocal !== 'object' || Array.isArray(firstLocal)) throw new Error('field-source-unavailable');
    const start = table.body + match.index + match[0].length;
    const after = materialiseOverride({ value: firstLocal }, `value.${remaining.slice(1).map(segment => segment.name).join('.')}`, row.value).value;
    return source.slice(0, start) + tomlValue(after) + source.slice(valueEnd(source, start));
  }
  const key = remaining.map(segment => /^[A-Za-z0-9_-]+$/.test(segment.name) ? segment.name : quote(segment.name)).join('.');
  const fragment = `${key} = ${tomlValue(row.value)}`;
  const newline = newlineOf(source);
  return source.slice(0, table.end) + fragment.replace(/\n/g, newline) + newline + source.slice(table.end);
}

export function prepareEntityOperation(draft, dependencies, operation) {
  const inspection = inspectEntityComposition(draft, dependencies, operation.path);
  const source = draft.read(operation.path), own = parsed(source, operation.path);
  let after;
  if (operation.type === 'include-add') {
    if (!entityPath(operation.include) || !inspection.inventory.some(row => row.path === operation.include)) throw new Error('include-missing');
    const reference = relativeIncludePath(operation.path, operation.include);
    if (inspection.includes.some(value => canonicalIncludePath(operation.path, value) === operation.include)) {
      throw new Error('duplicate-include');
    }
    after = setIncludes(source, [...inspection.includes, reference]);
  } else if (operation.type === 'include-remove') {
    if (!inspection.includes.includes(operation.include)) throw new Error('include-not-local');
    after = setIncludes(source, inspection.includes.filter(value => value !== operation.include));
  } else if (operation.type === 'component-add') {
    if (!ADDABLE_ENTITY_COMPONENTS.includes(operation.section)) throw new Error('unsupported-component');
    if (inspection.components.some(component => component.section === operation.section)) throw new Error('duplicate-component');
    after = appendComponent(source, operation.section);
  } else if (operation.type === 'component-remove') {
    if (!ENTITY_CONFIG_SECTIONS.includes(operation.section)) throw new Error('unsupported-component');
    after = removeComponent(source, operation.section);
  } else if (operation.type === 'field-materialize') {
    const row = inspection.fields.find(field => field.path === operation.field);
    if (!row || row.owner === operation.path) throw new Error('field-not-inherited');
    after = materializeField(source, row);
  } else throw new Error('invalid-entity-operation');
  return { inspection, changes: [{ path: operation.path, before: source, after }], own };
}

export async function applyEntityOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const revision = draft.sourceRevision;
  const effective = dependencies || await runtime.dependencies();
  const prepared = prepareEntityOperation(draft, effective, operation);
  const candidate = provider?.restoreDocument ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  candidate.apply(prepared.changes);
  inspectEntityComposition(candidate, effective, operation.path);
  const report = await runtime.validate(candidate.kind === 'mod' && !provider?.save ? candidate.archive() : null, candidate);
  if (!report?.accepted) { const error = new Error('runtime-validation-refused'); error.report = report; throw error; }
  if (!current() || draft.sourceRevision !== revision || prepared.changes.some(change => draft.read(change.path) !== change.before)) {
    throw new Error('stale-entity-operation');
  }
  draft.apply(prepared.changes);
  return { changes: prepared.changes, report, inspection: inspectEntityComposition(draft, effective, operation.path) };
}
