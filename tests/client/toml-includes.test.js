// Issue #949 (review follow-up) — reading a top-level key through an entity
// template's `includes` closure.
//
// check-strings.mjs derives `component.ship_picker.class.<class>` for every
// hull a world offers. `class` is inheritable, so reading only a hull's OWN
// top-level key left a composed hull at zero errors while ph-ship-picker.js
// badged it with the raw token. These cases mirror the merge-order asserts in
// src/entities/include_resolve.rs — if Rust's precedence ever changes, both
// sides should fail together rather than the checker drifting silently.

import { describe, it, expect } from 'vitest';
import {
  topLevelString,
  topLevelIncludes,
  resolveInclude,
  resolveThroughIncludes,
} from '../../scripts/toml-includes.mjs';

/** A fake tree: `{ path: source }` → the injected reader. */
const reader = (files) => async (file) =>
  Object.prototype.hasOwnProperty.call(files, file) ? files[file] : null;

const classOf = (files, start) => resolveThroughIncludes(start, 'class', reader(files));

describe('topLevelString', () => {
  it('reads a top-level key with its line number', () => {
    expect(topLevelString('name = "n"\nclass = "destroyer"\n', 'class'))
      .toEqual({ value: 'destroyer', lineNo: 2 });
  });

  it('stops at the first table header, so a nested key is not top level', () => {
    // Appending `class = "x"` to the end of a file with tables puts it INSIDE
    // the last table — TOML's rule, and the reason this must not be greedy.
    expect(topLevelString('name = "n"\n[mesh]\nclass = "x"\n', 'class')).toBeNull();
  });

  it('ignores a commented-out assignment', () => {
    expect(topLevelString('# class = "ghost"\nclass = "real"\n', 'class'))
      .toEqual({ value: 'real', lineNo: 2 });
  });

  it('keeps a # that is inside the quoted value', () => {
    expect(topLevelString('hull_id = "NCC#1"\n', 'hull_id').value).toBe('NCC#1');
  });

  it('returns null when the key is absent', () => {
    expect(topLevelString('name = "n"\n', 'class')).toBeNull();
  });
});

describe('topLevelIncludes', () => {
  it('reads the one-line form', () => {
    expect(topLevelIncludes('includes = ["a.toml"]\nclass = "c"\n')).toEqual(['a.toml']);
  });

  it('reads the multi-line form the shipped hulls use', () => {
    const src = [
      'class = "destroyer"',
      'includes = [',
      '  "fragments/ai/fleet_baseline.toml",',
      '  "fragments/ai/captain_alliance.toml",',
      ']',
      '',
      '[mesh]',
      'model = "x.glb"',
    ].join('\n');
    expect(topLevelIncludes(src)).toEqual([
      'fragments/ai/fleet_baseline.toml',
      'fragments/ai/captain_alliance.toml',
    ]);
  });

  it('reads an empty array as no includes', () => {
    expect(topLevelIncludes('includes = []\n')).toEqual([]);
  });

  it('ignores an `includes` nested under a table', () => {
    expect(topLevelIncludes('[behaviour]\nincludes = ["a.toml"]\n')).toEqual([]);
  });

  it('ignores a trailing comment on the opening line', () => {
    expect(topLevelIncludes('includes = [ # the shared AI\n  "a.toml",\n]\n')).toEqual(['a.toml']);
  });
});

describe('resolveInclude', () => {
  it('resolves relative to the declaring template, not the root hull', () => {
    expect(resolveInclude('assets/entities/hull.toml', 'frag/a.toml'))
      .toBe('assets/entities/frag/a.toml');
  });

  it('collapses `..` against the declaring fragment’s directory', () => {
    expect(resolveInclude('assets/entities/frag/mid.toml', '../../shared/core.toml'))
      .toBe('assets/shared/core.toml');
  });

  it('normalises backslashes so a Windows path joins the same way', () => {
    expect(resolveInclude('C:\\repo\\assets\\entities\\hull.toml', './frag/a.toml'))
      .toBe('C:/repo/assets/entities/hull.toml'.replace('hull.toml', 'frag/a.toml'));
  });
});

describe('resolveThroughIncludes', () => {
  it('takes the template’s own value when it authors one', async () => {
    const files = {
      'e/base.toml': 'class = "base"\n',
      'e/hull.toml': 'includes = ["base.toml"]\nclass = "self"\n',
    };
    expect(await classOf(files, 'e/hull.toml')).toMatchObject({
      value: 'self',
      file: 'e/hull.toml',
    });
  });

  it('inherits from an include when it authors none', async () => {
    const files = {
      'e/base.toml': 'name = "n"\nclass = "escort"\n',
      'e/hull.toml': 'includes = ["base.toml"]\nhull_id = "FIXTURE"\n',
    };
    expect(await classOf(files, 'e/hull.toml')).toEqual({
      value: 'escort',
      file: 'e/base.toml',
      lineNo: 2,
    });
  });

  it('lets the LAST include win — includes merge in declared order', async () => {
    const files = {
      'e/a.toml': 'class = "a"\n',
      'e/b.toml': 'class = "b"\n',
      'e/hull.toml': 'includes = ["a.toml", "b.toml"]\n',
    };
    expect((await classOf(files, 'e/hull.toml')).value).toBe('b');
  });

  it('falls back to an earlier include when the later one has none', async () => {
    const files = {
      'e/a.toml': 'class = "a"\n',
      'e/b.toml': 'hull_id = "b"\n',
      'e/hull.toml': 'includes = ["a.toml", "b.toml"]\n',
    };
    expect((await classOf(files, 'e/hull.toml')).value).toBe('a');
  });

  it('resolves depth-first through a nested fragment', async () => {
    const files = {
      'assets/shared/core.toml': 'class = "core"\n',
      'assets/entities/frag/mid.toml': 'includes = ["../../shared/core.toml"]\nhull_id = "mid"\n',
      'assets/entities/hull.toml': 'includes = ["frag/mid.toml"]\n',
    };
    expect(await classOf(files, 'assets/entities/hull.toml')).toMatchObject({
      value: 'core',
      file: 'assets/shared/core.toml',
    });
  });

  it('merges a diamond twice rather than rejecting it', async () => {
    const files = {
      'e/base.toml': 'class = "base"\n',
      'e/a.toml': 'includes = ["base.toml"]\nhull_id = "a"\n',
      'e/b.toml': 'includes = ["base.toml"]\npower_rating = 2\n',
      'e/hull.toml': 'includes = ["a.toml", "b.toml"]\n',
    };
    expect((await classOf(files, 'e/hull.toml')).value).toBe('base');
  });

  it('terminates on a cycle instead of recursing forever', async () => {
    const files = {
      'e/a.toml': 'includes = ["b.toml"]\n',
      'e/b.toml': 'includes = ["a.toml"]\n',
    };
    expect(await classOf(files, 'e/a.toml')).toBeNull();
  });

  it('says nothing about a dangling include — that is world::validate’s finding', async () => {
    const files = { 'e/hull.toml': 'includes = ["missing.toml"]\n' };
    expect(await classOf(files, 'e/hull.toml')).toBeNull();
  });

  it('returns null for a template that does not exist at all', async () => {
    expect(await classOf({}, 'e/gone.toml')).toBeNull();
  });
});
