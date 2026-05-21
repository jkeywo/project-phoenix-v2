/**
 * faction-complexity-discovery.js
 *
 * Pure async discovery helpers for Entity Mode. Walks
 *   assets/factions/*.toml  → uuid→name map
 *   assets/complexity/*.toml → sorted path list
 *
 * No DOM, no Bevy, no globals — accepts `listDirectory` and `readFile`
 * as injected deps so tests can pass mocks.
 *
 * Missing directories degrade to empty results rather than throwing.
 */
import { buildFactionMap, buildComplexityPaths } from './entity-toml.js';

/**
 * Discover factions and complexity TOML paths under the project root.
 *
 * @param {object} deps
 * @param {(rel: string) => Promise<Array<{name:string,kind:string}>>} deps.listDirectory
 * @param {(rel: string) => Promise<string>} deps.readFile
 * @returns {Promise<{ factionMap: Map<string,string>, complexityPaths: string[] }>}
 */
export async function discoverFactionsAndComplexity({ listDirectory, readFile }) {
  const factionFiles = await readFactionFiles({ listDirectory, readFile });
  const factionMap = buildFactionMap(factionFiles);

  const complexityFilenames = await listComplexityFilenames({ listDirectory });
  const complexityPaths = buildComplexityPaths(complexityFilenames);

  return { factionMap, complexityPaths };
}

async function readFactionFiles({ listDirectory, readFile }) {
  let entries;
  try {
    entries = await listDirectory('assets/factions');
  } catch {
    return [];
  }
  if (!Array.isArray(entries)) return [];

  const out = [];
  for (const e of entries) {
    if (!e || e.kind !== 'file') continue;
    if (typeof e.name !== 'string' || !e.name.endsWith('.toml')) continue;
    try {
      const content = await readFile(`assets/factions/${e.name}`);
      out.push({ name: e.name, content });
    } catch {
      // skip unreadable files
    }
  }
  return out;
}

async function listComplexityFilenames({ listDirectory }) {
  let entries;
  try {
    entries = await listDirectory('assets/complexity');
  } catch {
    return [];
  }
  if (!Array.isArray(entries)) return [];
  return entries
    .filter((e) => e && e.kind === 'file' && typeof e.name === 'string' && e.name.endsWith('.toml'))
    .map((e) => e.name);
}
