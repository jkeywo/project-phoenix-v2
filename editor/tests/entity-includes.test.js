import { describe, it, expect } from 'vitest';
import { parse as tomlParse } from 'smol-toml';
import {
  resolveTemplate,
  canonicalTemplatePath,
  canonicalIncludePath,
  fieldOrigin,
  isFieldInherited,
  sectionOrigin,
  provenanceSources,
  materialiseOverride,
  mergeComposeFragments,
  stripRemovals,
} from '../entity-includes.js';
import { EntityModeShell } from '../entity-mode.js';

// Issue #910 — the editor's JS twin of the Rust composable-template resolver
// (`src/entities/include_resolve.rs` + the ComposeFragments merge in
// `src/entities/entity_override.rs`). These cases mirror the Rust unit tests so
// the two resolvers cannot drift.

/** Resolve `root` against an in-memory { path: text } fixture; expect success. */
function resolve(root, pairs) {
  const result = resolveTemplate(root, pairs, tomlParse);
  expect(result.ok, `fixture must resolve: ${result.error?.message}`).toBe(true);
  return result.resolved;
}

function idsAt(resolved, key) {
  const arr = resolved.value[key];
  return Array.isArray(arr) ? arr.map((e) => e && e.id) : [];
}

describe('ordered precedence', () => {
  it('the includer wins over its fragment; a fragment-only field survives', () => {
    const r = resolve('e/hull.toml', {
      'e/base.toml': 'class = "escort"\n[hull]\nhull_integrity = 100.0\n',
      'e/hull.toml': 'includes = ["base.toml"]\n[hull]\nhull_integrity = 500.0\n',
    });
    expect(r.value.hull.hull_integrity).toBe(500);
    expect(r.value.class).toBe('escort');
  });

  it('a later include wins over an earlier one; unmentioned fields survive', () => {
    const r = resolve('e/hull.toml', {
      'e/a.toml': 'class = "a"\nhull_id = "from-a"\n',
      'e/b.toml': 'class = "b"\n',
      'e/hull.toml': 'includes = ["a.toml", "b.toml"]\n',
    });
    expect(r.value.class).toBe('b');
    expect(r.value.hull_id).toBe('from-a');
  });

  it('merge order is depth-first, declared order, declarer last', () => {
    const r = resolve('e/hull.toml', {
      'e/deep.toml': 'class = "deep"\nhull_id = "deep"\n',
      'e/mid.toml': 'includes = ["deep.toml"]\nclass = "mid"\n',
      'e/hull.toml': 'includes = ["mid.toml"]\n',
    });
    expect(provenanceSources(r.provenance)).toEqual(['e/deep.toml', 'e/mid.toml', 'e/hull.toml']);
    expect(r.value.class).toBe('mid');
    expect(r.value.hull_id).toBe('deep');
    expect(r.isComposed).toBe(true);
  });

  it('a diamond include is merged twice, not rejected', () => {
    const r = resolve('e/hull.toml', {
      'e/base.toml': 'class = "base"\n',
      'e/a.toml': 'includes = ["base.toml"]\nhull_id = "a"\n',
      'e/b.toml': 'includes = ["base.toml"]\npower_rating = 2\n',
      'e/hull.toml': 'includes = ["a.toml", "b.toml"]\n',
    });
    expect(r.value.class).toBe('base');
    expect(r.value.hull_id).toBe('a');
    expect(r.value.power_rating).toBe(2);
  });
});

describe('relative include paths', () => {
  it('resolve relative to the DECLARING template, not the root', () => {
    const r = resolve('assets/entities/hull.toml', {
      'assets/entities/frag/a.toml': 'class = "a"\n',
      'assets/entities/shared/b.toml': 'class = "b"\nhull_id = "b"\n',
      'assets/entities/hull.toml': 'includes = ["frag/a.toml", "./shared/b.toml"]\n',
    });
    expect(r.value.hull_id).toBe('b');
    expect(provenanceSources(r.provenance)).toEqual([
      'assets/entities/frag/a.toml',
      'assets/entities/shared/b.toml',
      'assets/entities/hull.toml',
    ]);
  });

  it('a nested fragment resolves its own includes relative to itself', () => {
    const r = resolve('assets/entities/hull.toml', {
      'assets/shared/core.toml': 'class = "core"\n',
      'assets/entities/frag/mid.toml': 'includes = ["../../shared/core.toml"]\nhull_id = "mid"\n',
      'assets/entities/hull.toml': 'includes = ["frag/mid.toml"]\n',
    });
    expect(r.value.class).toBe('core');
    expect(provenanceSources(r.provenance)[0]).toBe('assets/shared/core.toml');
  });

  it('canonicalIncludePath collapses dot segments and normalises backslashes', () => {
    expect(canonicalIncludePath('a/b/hull.toml', './frag/../frag/x.toml')).toBe('a/b/frag/x.toml');
    expect(canonicalIncludePath('a/b/hull.toml', '..\\shared\\x.toml')).toBe('a/shared/x.toml');
  });

  it('canonicalIncludePath rejects absolute references', () => {
    expect(canonicalIncludePath('a/hull.toml', '/etc/passwd.toml')).toBeNull();
    expect(canonicalIncludePath('a/hull.toml', 'C:\\x\\y.toml')).toBeNull();
    expect(canonicalIncludePath('a/hull.toml', '   ')).toBeNull();
  });

  it('canonicalTemplatePath normalises backslashes and dot segments', () => {
    expect(canonicalTemplatePath('a\\b\\.\\c.toml')).toBe('a/b/c.toml');
  });
});

describe('array semantics (ComposeFragments)', () => {
  it('tags UNION between fragments (a shared tag is not duplicated)', () => {
    const r = resolve('e/hull.toml', {
      'e/base.toml': 'tags = ["ship", "npc"]\n',
      'e/hull.toml': 'includes = ["base.toml"]\ntags = ["npc", "scenery"]\n',
    });
    expect(r.value.tags).toEqual(['ship', 'npc', 'scenery']);
  });

  it('an authored empty tags array CLEARS them', () => {
    const r = resolve('e/hull.toml', {
      'e/base.toml': 'tags = ["ship", "npc"]\n',
      'e/hull.toml': 'includes = ["base.toml"]\ntags = []\n',
    });
    expect(r.value.tags).toEqual([]);
  });

  it('doctrine merges by id; unmentioned entries and keys survive', () => {
    const r = resolve('e/hull.toml', {
      'e/base.toml':
        '[behaviour]\n[[behaviour.doctrine]]\nid = "destroy-hostiles"\ndirective_kind = "Destroy"\nbase_priority = 40.0\n[[behaviour.doctrine]]\nid = "hold-station"\nbase_priority = 10.0\n',
      'e/hull.toml':
        'includes = ["base.toml"]\n[[behaviour.doctrine]]\nid = "destroy-hostiles"\nbase_priority = 90.0\n',
    });
    const doctrine = r.value.behaviour.doctrine;
    expect(doctrine).toHaveLength(2);
    expect(doctrine[0].base_priority).toBe(90);
    expect(doctrine[0].directive_kind).toBe('Destroy');
  });

  it('a hull EXTENDS a fragment system suite instead of replacing it', () => {
    const r = resolve('e/hull.toml', {
      'e/systems.toml':
        '[[system]]\nid = "helm-thrust"\nkind = "helm_thrust"\n[[system]]\nid = "power-reactor"\nkind = "power_reactor"\n',
      'e/hull.toml':
        'includes = ["systems.toml"]\n[[system]]\nid = "phaser-dorsal"\nkind = "phaser_bank"\n',
    });
    expect(idsAt(r, 'system')).toEqual(['helm-thrust', 'power-reactor', 'phaser-dorsal']);
  });

  it('a composed chain specialises and REMOVES inherited entries; no _remove leaks', () => {
    const r = resolve('e/hull.toml', {
      'e/library.toml':
        '[[system]]\nid = "helm-thrust"\nkind = "helm_thrust"\nai_only = true\n[[system]]\nid = "power-reactor"\nkind = "power_reactor"\n[[system]]\nid = "legacy-probe"\nkind = "sensor_probe"\n',
      'e/class.toml':
        'includes = ["library.toml"]\n[[system]]\nid = "legacy-probe"\n_remove = true\n[[system]]\nid = "phaser-dorsal"\nkind = "phaser_bank"\n',
      'e/hull.toml':
        'includes = ["class.toml"]\n[[system]]\nid = "helm-thrust"\nai_only = false\n',
    });
    expect(idsAt(r, 'system')).toEqual(['helm-thrust', 'power-reactor', 'phaser-dorsal']);
    expect(r.value.system[0].ai_only).toBe(false);
    expect(r.value.system[0].kind).toBe('helm_thrust');
    expect(JSON.stringify(r.value)).not.toContain('_remove');
  });

  it('nested [[station.rating]] reconciles by name inside [[station]] by id', () => {
    const r = resolve('e/hull.toml', {
      'e/frag.toml':
        '[[station]]\nid = "bridge"\n[[station.rating]]\nname = "helm"\nlevel = 1\n[[station.rating]]\nname = "tactical"\nlevel = 1\n[[station]]\nid = "engineering"\n',
      'e/hull.toml':
        'includes = ["frag.toml"]\n[[station]]\nid = "bridge"\n[[station.rating]]\nname = "tactical"\nlevel = 3\n',
    });
    expect(idsAt(r, 'station')).toEqual(['bridge', 'engineering']);
    const ratings = r.value.station[0].rating;
    expect(ratings).toHaveLength(2);
    expect(ratings[0].level).toBe(1);
    expect(ratings[1].level).toBe(3);
  });

  it('keyless arrays (AI rules) replace wholesale between fragments', () => {
    const r = resolve('e/hull.toml', {
      'e/base.toml': '[[captain_console.ai.rule]]\nchannel = "a"\npriority = 1\n',
      'e/hull.toml':
        'includes = ["base.toml"]\n[[captain_console.ai.rule]]\nchannel = "b"\npriority = 2\n',
    });
    const rules = r.value.captain_console.ai.rule;
    expect(rules).toHaveLength(1);
    expect(rules[0].channel).toBe('b');
  });
});

describe('provenance distinguishes inherited from hull-authored', () => {
  it('records who authored each leaf and through which chain', () => {
    const r = resolve('e/hull.toml', {
      'e/systems.toml':
        'class = "escort"\n[[system]]\nid = "helm-thrust"\nkind = "helm_thrust"\n',
      'e/hull.toml':
        'includes = ["systems.toml"]\n[hull]\nhull_integrity = 500.0\n[[system]]\nid = "phaser-dorsal"\nkind = "phaser_bank"\n',
    });

    // A hull-authored field.
    expect(fieldOrigin(r.provenance, 'hull.hull_integrity').source).toBe('e/hull.toml');
    expect(isFieldInherited(r.provenance, 'hull.hull_integrity', 'e/hull.toml')).toBe(false);

    // A fragment-authored field, addressed by its array identity key.
    const helmKind = fieldOrigin(r.provenance, 'system[id=helm-thrust].kind');
    expect(helmKind.source).toBe('e/systems.toml');
    // The chain runs from the root template DOWN to the authoring source.
    expect(helmKind.chain).toEqual(['e/hull.toml', 'e/systems.toml']);
    expect(isFieldInherited(r.provenance, 'system[id=helm-thrust].kind', 'e/hull.toml')).toBe(true);

    // A system the hull itself added is hull-authored.
    expect(isFieldInherited(r.provenance, 'system[id=phaser-dorsal].kind', 'e/hull.toml')).toBe(
      false,
    );

    // Section-level classification.
    expect(sectionOrigin(r.provenance, 'hull', 'e/hull.toml')).toBe('authored');
    expect(sectionOrigin(r.provenance, 'class', 'e/hull.toml')).toBe('inherited');
    expect(sectionOrigin(r.provenance, 'system', 'e/hull.toml')).toBe('mixed');
  });
});

describe('resolution failures name the declaring file, not silent omissions', () => {
  it('a missing fragment is an include-missing error naming the hull', () => {
    const result = resolveTemplate('e/hull.toml', { 'e/hull.toml': 'includes = ["missing.toml"]\n' }, tomlParse);
    expect(result.ok).toBe(false);
    expect(result.error.category).toBe('include-missing');
    expect(result.error.file).toBe('e/hull.toml');
    expect(result.error.chain).toEqual(['e/hull.toml', 'e/missing.toml']);
  });

  it('a cycle is an include-cycle error with the full chain', () => {
    const result = resolveTemplate(
      'e/a.toml',
      { 'e/a.toml': 'includes = ["b.toml"]\n', 'e/b.toml': 'includes = ["a.toml"]\n' },
      tomlParse,
    );
    expect(result.ok).toBe(false);
    expect(result.error.category).toBe('include-cycle');
    expect(result.error.chain).toEqual(['e/a.toml', 'e/b.toml', 'e/a.toml']);
    // The cycle is reported against the DECLARING file — `e/b.toml` is the
    // template whose `includes` closes the loop back to `e/a.toml`. Asserting
    // it guards against a misnaming mutation that would otherwise pass green.
    expect(result.error.file).toBe('e/b.toml');
  });

  it('a non-array includes declaration is include-malformed', () => {
    const result = resolveTemplate('e/hull.toml', { 'e/hull.toml': 'includes = "base.toml"\n' }, tomlParse);
    expect(result.ok).toBe(false);
    expect(result.error.category).toBe('include-malformed');
    expect(result.error.file).toBe('e/hull.toml');
  });

  it('a non-string includes entry is include-malformed', () => {
    const result = resolveTemplate('e/hull.toml', { 'e/hull.toml': 'includes = [123]\n' }, tomlParse);
    expect(result.ok).toBe(false);
    expect(result.error.category).toBe('include-malformed');
  });

  it('an absolute include reference is include-malformed', () => {
    const result = resolveTemplate('e/hull.toml', { 'e/hull.toml': 'includes = ["/etc/x.toml"]\n' }, tomlParse);
    expect(result.ok).toBe(false);
    expect(result.error.category).toBe('include-malformed');
    expect(result.error.file).toBe('e/hull.toml');
  });

  it('an unparseable fragment is include-parse naming the fragment', () => {
    const result = resolveTemplate(
      'e/hull.toml',
      { 'e/hull.toml': 'includes = ["bad.toml"]\n', 'e/bad.toml': 'not = valid [ toml' },
      tomlParse,
    );
    expect(result.ok).toBe(false);
    expect(result.error.category).toBe('include-parse');
    expect(result.error.file).toBe('e/bad.toml');
  });
});

describe('materialise-override (the deliberate edit-an-inherited-field decision)', () => {
  it('writes a table field onto the authored hull, preserving includes', () => {
    const authored = { includes: ['base.toml'], hull: {} };
    const next = materialiseOverride(authored, 'hull.hull_integrity', 500);
    expect(next.hull.hull_integrity).toBe(500);
    expect(next.includes).toEqual(['base.toml']);
    // Input is not mutated (pure).
    expect(authored.hull.hull_integrity).toBeUndefined();
  });

  it('writes an array-keyed field, creating the addressed entry', () => {
    const authored = { includes: ['systems.toml'] };
    const next = materialiseOverride(authored, 'system[id=helm-thrust].ai_only', false);
    expect(next.system).toEqual([{ id: 'helm-thrust', ai_only: false }]);
    expect(next.includes).toEqual(['systems.toml']);
  });

  it('mergeComposeFragments and stripRemovals are exposed for reuse', () => {
    const merged = mergeComposeFragments({ tags: ['a'] }, { tags: ['b'] });
    expect(merged.tags).toEqual(['a', 'b']);
    expect(stripRemovals({ system: [{ id: 'x', _remove: true }, { id: 'y' }] })).toEqual({
      system: [{ id: 'y' }],
    });
  });
});

describe('EntityModeShell composition awareness', () => {
  const authoredToml = 'includes = ["systems.toml"]\n[hull]\nhull_integrity = 500.0\n';
  const resolved = {
    hull: { hull_integrity: 500 },
    tags: ['ship'],
    system: [{ id: 'helm-thrust', kind: 'helm_thrust' }],
  };
  // Provenance as produced by the resolver: hull authored the hull block,
  // the fragment authored the system + tags.
  function provenanceFor() {
    const r = resolveTemplate(
      'assets/entities/hull.toml',
      {
        'assets/entities/systems.toml':
          'tags = ["ship"]\n[[system]]\nid = "helm-thrust"\nkind = "helm_thrust"\n',
        'assets/entities/hull.toml': authoredToml,
      },
      tomlParse,
    );
    return r.resolved;
  }

  it('preview reads the resolved document; provenance marks inherited sections', () => {
    const shell = new EntityModeShell();
    const res = provenanceFor();
    const ok = shell.openFile('assets/entities/hull.toml', authoredToml, {
      resolved: res.value,
      provenance: res.provenance,
    });
    expect(ok.ok).toBe(true);
    expect(shell.isComposed()).toBe(true);

    // The preview sees the fragment's tags, not the empty authored set.
    const preview = shell.getPreviewPane();
    expect(preview.textOverlay.tags).toEqual(['ship']);
    expect(preview.textOverlay.hullTotal).toBe(500);

    // Inherited vs authored is provenance-driven.
    expect(shell.getSectionOrigin('hull')).toBe('authored');
    expect(shell.getSectionOrigin('system')).toBe('inherited');
    const inherited = shell.getInheritedSections().map((s) => s.section).sort();
    expect(inherited).toContain('system');
    expect(inherited).toContain('tags');
  });

  it('materialiseSection copies an inherited section onto the authored hull', () => {
    const shell = new EntityModeShell();
    const res = provenanceFor();
    shell.openFile('assets/entities/hull.toml', authoredToml, {
      resolved: res.value,
      provenance: res.provenance,
    });
    // Before: the hull does not author `system`.
    expect(shell.getParsedEntity().system).toBeUndefined();

    const out = shell.materialiseSection('system');
    expect(out.ok).toBe(true);
    // After: the hull now authors `system`, and `includes` is preserved.
    expect(shell.getParsedEntity().system).toEqual(res.value.system);
    expect(shell.getParsedEntity().includes).toEqual(['systems.toml']);
    // A card now exists for the materialised section.
    expect(shell.getCard('system')).not.toBeNull();
  });
});
