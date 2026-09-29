import { describe, expect, it } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { literalWorldReferences, scriptReferenceForms, applyScriptReference } from '../workshop-script-references.js';

const WORLD = 'assets/worlds/root.toml', OLD = 'assets/worlds/old.toml', NEXT = 'assets/worlds/next.toml';
const body = `fn start(ctx) { ctx.effects.load_world("${OLD}"); ctx.effects.unload_world("${OLD}"); }`;
function draft(source, sibling) {
  const files = [{ path: 'scenarios.toml', text: '[pack]\nid="test"\n' }, { path: WORLD, text: source }];
  if (sibling) files.push({ path: 'assets/worlds/root.rhai', text: sibling });
  return new WorkshopDocument(createStoreZip(files));
}
const runtime = { validate: async () => ({ accepted: true, findings: [] }) };

describe('structured composition script references', () => {
  it('ignores nested comments, strings, computed arguments and longer identifiers', () => {
    const source = `/* outer /* end */ load_world("bad"); */ // load_world("bad")\n
      let prose = "load_world(\\"bad\\")"; reload_world("bad");
      ctx.effects.load_world("prefix" + x); ctx.effects.load_world(path);
      ctx.effects.load_world /* allowed gap */ ( "${OLD}" ); ${body}`;
    expect(literalWorldReferences(source).map(ref => [ref.kind, ref.path])).toEqual([
      ['load', OLD], ['load', OLD], ['unload', OLD],
    ]);
    expect(literalWorldReferences('`template ${load_world("bad")}`')).toEqual([]);
  });

  it.each([
    ['literal multiline', `# keep\r\n[script]\r\nsetup = '''\r\n${body}\r\n''' # tail\r\n`],
    ['basic single-line', `[script]\nsetup = ${JSON.stringify(body)} # keep\n`],
    ['basic multiline escapes', `[script]\nsetup = """\n// preserve \\u0061\n${body.replaceAll('"', '\\"')}\n"""\n`],
    ['dotted field', `script.setup = ${JSON.stringify(body)}\n`],
  ])('patches one literal in %s without rewriting unrelated bytes', async (_name, original) => {
    const value = draft(original), [reference] = scriptReferenceForms(value, WORLD);
    expect(reference).toBeDefined();
    const expected = original.replace(OLD, NEXT);
    const result = await applyScriptReference({ draft: value, runtime, reference, path: NEXT });
    expect(result.applied).toBe(true);
    expect(value.read(WORLD)).toBe(expected);
    expect(parse(value.read(WORLD)).script.setup).toContain(`load_world("${NEXT}")`);
    value.undo(); expect(value.read(WORLD)).toBe(original);
  });

  it('edits a sibling unload and validates before the exact single-member mutation', async () => {
    const value = draft('script="root.rhai"\n', body), reference = scriptReferenceForms(value, WORLD)[1];
    let checked = false;
    await applyScriptReference({ draft: value, reference, path: NEXT, runtime: { validate: async (_archive, candidate) => {
      checked = true;
      expect(candidate.read(reference.unit.documentPath)).toContain(`unload_world("${NEXT}")`);
      expect(value.read(reference.unit.documentPath)).toBe(body);
      return { accepted: true };
    } } });
    expect(checked).toBe(true);
    expect(value.read(WORLD)).toBe('script="root.rhai"\n');
    expect(value.read(reference.unit.documentPath)).toBe(body.replace(`unload_world("${OLD}")`, `unload_world("${NEXT}")`));
  });

  it('never offers an immutable sibling or ambiguously located inline body', () => {
    expect(scriptReferenceForms(draft('script="base.rhai"\n'), WORLD)).toEqual([]);
    expect(scriptReferenceForms(draft(`[script]\nsetup=${JSON.stringify(body)}\nother=${JSON.stringify(body)}\n`), WORLD)).toEqual([]);
  });

  it('refuses stale readings, disallowed targets and runtime refusals without history changes', async () => {
    const original = `[script]\nsetup=${JSON.stringify(body)}\n`;
    const value = draft(original), [reference] = scriptReferenceForms(value, WORLD);
    await expect(applyScriptReference({ draft: value, runtime, reference, path: '../escape.toml' })).rejects.toThrow('disallowed');
    await expect(applyScriptReference({ draft: value, reference, path: NEXT,
      runtime: { validate: async () => ({ accepted: false, findings: [{ message: 'Missing target' }] }) } })).rejects.toThrow('validation_refused');
    expect(value.read(WORLD)).toBe(original); expect(value.canUndo()).toBe(false);
    value.edit(WORLD, original + '# concurrent\n');
    await expect(applyScriptReference({ draft: value, runtime, reference, path: NEXT })).rejects.toThrow('stale');
  });

  it('refuses a runtime answer arriving after another edit', async () => {
    const value = draft('script="root.rhai"\n', body), [reference] = scriptReferenceForms(value, WORLD);
    await expect(applyScriptReference({ draft: value, reference, path: NEXT, runtime: { validate: async () => {
      value.edit(WORLD, 'script="root.rhai"\n# changed\n'); return { accepted: true };
    } } })).rejects.toThrow('stale');
    expect(value.read(reference.unit.documentPath)).toBe(body);
  });
});

it('does not mistake backticks in comments or ordinary strings for templates', () => {
  const source = '// Use `load_world` for composition.\n/* `example` */\n'
    + 'let prose = "A `quoted` word";\n' + body;
  expect(literalWorldReferences(source).map(ref => ref.path)).toEqual([OLD, OLD]);
  expect(literalWorldReferences(body + '\nlet value = `real template`;')).toEqual([]);
});
