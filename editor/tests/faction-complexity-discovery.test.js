import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

import { discoverFactionsAndComplexity } from '../faction-complexity-discovery.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

function makeDeps({ factions, complexity, factionContent }) {
  return {
    listDirectory: async (rel) => {
      if (rel === 'assets/factions') {
        if (factions === '__throw__') throw new Error('missing dir');
        return factions ?? [];
      }
      if (rel === 'assets/complexity') {
        if (complexity === '__throw__') throw new Error('missing dir');
        return complexity ?? [];
      }
      return [];
    },
    readFile: async (path) => {
      if (factionContent && factionContent[path] !== undefined) {
        return factionContent[path];
      }
      throw new Error(`no file: ${path}`);
    },
  };
}

describe('discoverFactionsAndComplexity', () => {
  it('returns empty results when both directories are missing', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({ factions: '__throw__', complexity: '__throw__' }),
    );
    expect(result.factionMap.size).toBe(0);
    expect(result.complexityPaths).toEqual([]);
  });

  it('returns empty results when both directories are empty', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({ factions: [], complexity: [] }),
    );
    expect(result.factionMap.size).toBe(0);
    expect(result.complexityPaths).toEqual([]);
  });

  it('skips non-.toml files in factions', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({
        factions: [
          { name: 'README.md', kind: 'file' },
          { name: 'fed.toml', kind: 'file' },
        ],
        factionContent: {
          'assets/factions/fed.toml': 'uuid = "00000000-0000-4000-8000-000000000001"\nname = "Fed"\n',
        },
        complexity: [],
      }),
    );
    expect(result.factionMap.size).toBe(1);
    expect(result.factionMap.get('00000000-0000-4000-8000-000000000001')).toBe('Fed');
  });

  it('skips directory entries when scanning factions', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({
        factions: [
          { name: 'subdir', kind: 'directory' },
          { name: 'good.toml', kind: 'file' },
        ],
        factionContent: {
          'assets/factions/good.toml': 'uuid = "00000000-0000-4000-8000-000000000002"\nname = "Good"\n',
        },
        complexity: [],
      }),
    );
    expect(result.factionMap.size).toBe(1);
  });

  it('skips malformed faction TOML', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({
        factions: [
          { name: 'bad.toml', kind: 'file' },
          { name: 'ok.toml', kind: 'file' },
        ],
        factionContent: {
          'assets/factions/bad.toml': 'this is not = valid ===',
          'assets/factions/ok.toml': 'uuid = "00000000-0000-4000-8000-000000000003"\nname = "Ok"\n',
        },
        complexity: [],
      }),
    );
    expect(result.factionMap.size).toBe(1);
    expect(result.factionMap.get('00000000-0000-4000-8000-000000000003')).toBe('Ok');
  });

  it('skips non-.toml files in complexity', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({
        factions: [],
        complexity: [
          { name: 'README.md', kind: 'file' },
          { name: 'tactical.toml', kind: 'file' },
          { name: 'power.toml', kind: 'file' },
        ],
      }),
    );
    expect(result.complexityPaths).toEqual([
      'assets/complexity/power.toml',
      'assets/complexity/tactical.toml',
    ]);
  });

  it('handles only-factions-missing gracefully', async () => {
    const result = await discoverFactionsAndComplexity(
      makeDeps({
        factions: '__throw__',
        complexity: [{ name: 'tactical.toml', kind: 'file' }],
      }),
    );
    expect(result.factionMap.size).toBe(0);
    expect(result.complexityPaths).toEqual(['assets/complexity/tactical.toml']);
  });

  it('loads all real shipped faction and complexity files', async () => {
    const factionEntries = readdirSync(resolve(projectRoot, 'assets/factions'));
    const complexityEntries = readdirSync(resolve(projectRoot, 'assets/complexity'));
    const factionContent = {};
    for (const f of factionEntries) {
      if (!f.endsWith('.toml')) continue;
      factionContent[`assets/factions/${f}`] = readFileSync(
        resolve(projectRoot, 'assets/factions', f), 'utf-8',
      );
    }
    const result = await discoverFactionsAndComplexity({
      listDirectory: async (rel) => {
        if (rel === 'assets/factions') return factionEntries.map((n) => ({ name: n, kind: 'file' }));
        if (rel === 'assets/complexity') return complexityEntries.map((n) => ({ name: n, kind: 'file' }));
        return [];
      },
      readFile: async (path) => factionContent[path],
    });
    expect(result.factionMap.size).toBeGreaterThan(0);
    expect(result.factionMap.get('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa')).toBe('Federation');
    expect(result.complexityPaths.length).toBe(
      complexityEntries.filter((f) => f.endsWith('.toml')).length,
    );
    for (const p of result.complexityPaths) {
      expect(p.startsWith('assets/complexity/')).toBe(true);
    }
  });
});
