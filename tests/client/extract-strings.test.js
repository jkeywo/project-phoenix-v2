/**
 * scripts/extract-strings.mjs — the rewrite half, driven over fabricated TOML.
 *
 * No file is read and none is written: `processFile` takes source text and
 * returns rewritten source plus the rows it wants, so what is pinned here is the
 * decision, not the plumbing around it.
 */
import { describe, it, expect } from 'vitest';
import { createRowSink, processFile } from '../../scripts/extract-strings.mjs';

/** The two passes main() runs, over an in-memory set of world files. */
function extractWorlds(files) {
  const sink = createRowSink();

  // Pass 1 — harvest entity names so references can follow a rename.
  const nameMap = new Map();
  for (const [file, src] of files) {
    const { names } = processFile(file, src, 'world', null, true, sink);
    for (const [name, id] of names) if (!nameMap.has(name)) nameMap.set(name, id);
  }

  // Pass 2 — rewrite definitions and references together.
  const rewritten = new Map();
  for (const [file, src] of files) {
    rewritten.set(file, processFile(file, src, 'world', nameMap, false, sink).src);
  }

  return { rows: sink.rows, rewritten, nameMap };
}

describe('createRowSink', () => {
  it('suffixes a genuine id collision rather than dropping a row', () => {
    // Two array entries with no discriminator land on the same minted id. Both
    // are real strings and both must survive.
    const sink = createRowSink();
    expect(sink.addRow('world.axiom.hail.text', 'ctx', 'First')).toBe('world.axiom.hail.text');
    expect(sink.addRow('world.axiom.hail.text', 'ctx', 'Second')).toBe('world.axiom.hail.text_2');
    expect(sink.rows.map((r) => r.en)).toEqual(['[First]', '[Second]']);
  });

  it('adds nothing the second time an already-chosen id is offered', () => {
    const sink = createRowSink();
    expect(sink.addRowOnce('world.entity.earth.name', 'ctx', 'Earth')).toBe('world.entity.earth.name');
    expect(sink.addRowOnce('world.entity.earth.name', 'other ctx', 'Earth')).toBe('world.entity.earth.name');
    expect(sink.rows).toHaveLength(1);
  });
});

describe('world entity names', () => {
  const DEFAULT_WORLD = `
[[entity]]
name = "Earth"
template_path = "assets/entities/planet_earth.toml"
`;
  const PATROL_WORLD = `
[[entity]]
name = "Earth"
template_path = "assets/entities/planet_earth.toml"

[[trigger]]
entity = "Earth"
`;

  it('gives one entity name one id and one row across every world that names it', () => {
    // The orphan-row bug: the id is chosen by the collect pass and the TOML is
    // rewritten with it, so a suffixed second row would be referenced by
    // nothing — and the CSV merge only ever appends, so it would stay forever.
    const { rows, rewritten } = extractWorlds([
      ['assets/worlds/default.toml', DEFAULT_WORLD],
      ['assets/worlds/patrol.toml', PATROL_WORLD],
    ]);

    expect(rows.map((r) => r.id)).toEqual(['world.entity.earth.name']);
    expect(rows.map((r) => r.id).filter((id) => /_\d+$/.test(id))).toEqual([]);
    for (const src of rewritten.values()) {
      expect(src).toContain('name = "world.entity.earth.name"');
    }
  });

  it('still rewrites a reference in a second world file to the shared id', () => {
    const { rewritten } = extractWorlds([
      ['assets/worlds/default.toml', DEFAULT_WORLD],
      ['assets/worlds/patrol.toml', PATROL_WORLD],
    ]);
    expect(rewritten.get('assets/worlds/patrol.toml')).toContain('entity = "world.entity.earth.name"');
  });

  it('still gives two different names two ids', () => {
    const { rows } = extractWorlds([
      ['assets/worlds/default.toml', DEFAULT_WORLD],
      ['assets/worlds/patrol.toml', '\n[[entity]]\nname = "Raider Alpha"\n'],
    ]);
    expect(rows.map((r) => r.id).sort())
      .toEqual(['world.entity.earth.name', 'world.entity.raider_alpha.name']);
  });

  it('leaves an already-migrated name alone on a re-run', () => {
    const migrated = '\n[[entity]]\nname = "world.entity.earth.name"\n';
    const { rows, rewritten } = extractWorlds([['assets/worlds/default.toml', migrated]]);
    expect(rows).toEqual([]);
    expect(rewritten.get('assets/worlds/default.toml')).toBe(migrated);
  });
});
