/**
 * script-editor.test.js — pure logic for the Rhai script editor (#983).
 */
import { describe, it, expect } from 'vitest';
import {
  tokenizeRhai,
  completionContext,
  matchCompletions,
  extractScriptUnits,
  siblingScriptPath,
  inlineBlockBaseLine,
} from '../script-editor.js';

const HOST_FNS = [
  { name: 'on_destroyed', receiver: '', category: 'trigger', signature: 'on_destroyed(entity, handler)', summary: 'x' },
  { name: 'on_timer', receiver: '', category: 'trigger', signature: 'on_timer(after_secs, handler)', summary: 'x' },
  { name: 'on', receiver: '', category: 'register', signature: 'on(event, handler)', summary: 'x' },
  { name: 'complete_objective', receiver: 'effects', category: 'effect', signature: 'effects.complete_objective(id)', summary: 'x' },
  { name: 'fail_objective', receiver: 'effects', category: 'effect', signature: 'effects.fail_objective(id)', summary: 'x' },
  { name: 'increment', receiver: 'flags', category: 'flag', signature: 'flags.increment(name, by)', summary: 'x' },
  { name: 'in_seconds', receiver: 'schedule', category: 'schedule', signature: 'schedule.in_seconds(secs)', summary: 'x' },
  { name: 'complete_objective', receiver: 'delay', category: 'delay', signature: 'delay.complete_objective(id)', summary: 'x' },
];

describe('tokenizeRhai', () => {
  it('reconstructs the source exactly from token values', () => {
    const src = 'fn on_x(ctx) {\n  ctx.effects.complete_objective("obj"); // done\n}';
    const tokens = tokenizeRhai(src);
    expect(tokens.map((t) => t.value).join('')).toBe(src);
  });

  it('classifies keywords, strings, comments and numbers', () => {
    const tokens = tokenizeRhai('let x = 5; // note\n"hi"');
    const byType = (ty) => tokens.filter((t) => t.type === ty).map((t) => t.value);
    expect(byType('keyword')).toContain('let');
    expect(byType('number')).toContain('5');
    expect(byType('comment')).toEqual(['// note']);
    expect(byType('string')).toEqual(['"hi"']);
  });

  it('tags known host-fn identifiers as hostfn', () => {
    const known = new Set(['on_destroyed', 'complete_objective']);
    const tokens = tokenizeRhai('on_destroyed("r", "h"); ordinary_fn();', known);
    const hostfns = tokens.filter((t) => t.type === 'hostfn').map((t) => t.value);
    expect(hostfns).toContain('on_destroyed');
    expect(hostfns).not.toContain('ordinary_fn');
  });

  it('handles an unterminated string without looping forever', () => {
    const tokens = tokenizeRhai('let x = "oops');
    expect(tokens.map((t) => t.value).join('')).toBe('let x = "oops');
  });
});

describe('completionContext', () => {
  it('reports a top-level prefix with empty receiver', () => {
    expect(completionContext('on_de')).toEqual({ prefix: 'on_de', receiver: '' });
  });
  it('reports the ctx namespace after ctx.', () => {
    expect(completionContext('  ctx.')).toEqual({ prefix: '', receiver: 'ctx' });
  });
  it('reports a member receiver after ctx.effects.', () => {
    expect(completionContext('ctx.effects.comp')).toEqual({ prefix: 'comp', receiver: 'effects' });
  });
  it('reports the delay builder after a call result', () => {
    expect(completionContext('ctx.schedule.in_seconds(5).comp')).toEqual({
      prefix: 'comp',
      receiver: 'delay',
    });
  });
});

describe('matchCompletions', () => {
  it('offers top-level trigger builders for the empty receiver', () => {
    const out = matchCompletions(HOST_FNS, { prefix: 'on_', receiver: '' });
    const names = out.map((h) => h.name);
    expect(names).toContain('on_destroyed');
    expect(names).toContain('on_timer');
    // effects are not top-level.
    expect(names).not.toContain('complete_objective');
  });

  it('offers effects methods for the effects receiver, filtered by prefix', () => {
    const out = matchCompletions(HOST_FNS, { prefix: 'comp', receiver: 'effects' });
    expect(out.map((h) => h.name)).toEqual(['complete_objective']);
  });

  it('offers the ctx namespaces after ctx.', () => {
    const out = matchCompletions(HOST_FNS, { prefix: '', receiver: 'ctx' });
    expect(out.map((h) => h.name).sort()).toEqual(['effects', 'flags', 'schedule']);
  });

  it('offers the delay verbs after a delay builder', () => {
    const out = matchCompletions(HOST_FNS, { prefix: '', receiver: 'delay' });
    expect(out.map((h) => h.name)).toEqual(['complete_objective']);
  });

  it('returns nothing for an unknown member context', () => {
    expect(matchCompletions(HOST_FNS, { prefix: 'x', receiver: 'member' })).toEqual([]);
  });
});

describe('siblingScriptPath', () => {
  it('resolves relative to the world file directory', () => {
    expect(siblingScriptPath('assets/worlds/combat_test.toml', 'combat.rhai'))
      .toBe('assets/worlds/combat.rhai');
  });
  it('normalises backslashes', () => {
    expect(siblingScriptPath('assets\\worlds\\w.toml', 'sub\\a.rhai'))
      .toBe('assets/worlds/sub/a.rhai');
  });
});

describe('extractScriptUnits', () => {
  it('returns [] for a world with no script key', () => {
    expect(extractScriptUnits({ global: {} }, 'w.toml')).toEqual([]);
  });

  it('lifts a sibling file reference', () => {
    const units = extractScriptUnits({ script: 'combat.rhai' }, 'assets/worlds/w.toml');
    expect(units).toHaveLength(1);
    expect(units[0]).toMatchObject({
      kind: 'sibling',
      label: 'combat.rhai',
      path: 'assets/worlds/combat.rhai',
    });
  });

  it('lifts inline [script.*] blocks sorted by key, carrying their source', () => {
    const units = extractScriptUnits(
      { script: { on_zulu: 'fn z(ctx){}', on_alpha: 'fn a(ctx){}' } },
      'w.toml',
    );
    expect(units.map((u) => u.key)).toEqual(['on_alpha', 'on_zulu']);
    expect(units[0]).toMatchObject({ kind: 'inline', label: '[script.on_alpha]', source: 'fn a(ctx){}' });
  });
});

describe('inlineBlockBaseLine', () => {
  it('returns 0 when the raw text is unavailable', () => {
    expect(inlineBlockBaseLine('', 'setup')).toBe(0);
  });

  it('maps a triple-quoted block to the line after its assignment', () => {
    const raw = [
      'name = "w"',        // line 0
      '',                  // line 1
      '[script]',          // line 2
      'setup = """',       // line 3  → content starts line 4
      'on_timer(5, "h");', // line 4
      '"""',
    ].join('\n');
    expect(inlineBlockBaseLine(raw, 'setup')).toBe(4);
  });

  it('maps a single-line block to its own assignment line', () => {
    const raw = ['[script]', 'setup = "fn s(ctx){}"'].join('\n');
    expect(inlineBlockBaseLine(raw, 'setup')).toBe(1);
  });
});
