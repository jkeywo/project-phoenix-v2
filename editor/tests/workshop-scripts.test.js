import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applyWorkshopScript, replaceInlineScript, workshopScriptUnits } from '../workshop-scripts.js';

const WORLD = 'assets/worlds/root.toml';
function draft(world = '[global]\ntitle="Root"\n[script]\nsetup = """\nfn start(ctx) { ctx.flags.increment("x", 1); }\n"""\n') {
  return new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="p"\nname="p"\nversion="1"\n[pack.requires]\ncontent_id="base"\ncontent_epoch=1\n[[scenario]]\nid="root"\nworld="assets/worlds/root.toml"\n' },
    { path: WORLD, text: world },
  ]));
}

describe('Workshop Rhai exact-source authoring', () => {
  it('replaces only a multiline inline payload and preserves surrounding comments and CRLF', () => {
    const before = '# keep\r\n[script]\r\nsetup = """\r\nfn old(ctx) {}\r\n""" # tail\r\n[global]\r\ntitle="Keep"\r\n';
    const after = replaceInlineScript(before, 'setup', 'fn next(ctx) { ctx.effects.game_over("done"); }');
    expect(after).toContain('# keep\r\n[script]\r\nsetup = """\r\n');
    expect(after).toContain('""" # tail\r\n[global]\r\ntitle="Keep"');
    expect(parse(after).script.setup).toBe('fn next(ctx) { ctx.effects.game_over("done"); }');
  });

  it('discovers inline and sibling units with exact diagnostic coordinates', () => {
    const inline = draft();
    expect(workshopScriptUnits(inline, WORLD)[0]).toMatchObject({
      kind: 'inline', documentPath: WORLD, lineOffset: 4,
    });
    const sibling = draft('script="root.rhai"\n[global]\ntitle="Root"\n');
    sibling.put('assets/worlds/root.rhai', 'fn start(ctx) {}');
    expect(workshopScriptUnits(sibling, WORLD)[0]).toMatchObject({
      kind: 'sibling', documentPath: 'assets/worlds/root.rhai', lineOffset: 0,
    });
    const commentedHeader = draft('[global]\ntitle="Root"\n[script] # retained\nsetup = """\nfn start(ctx) {}\n"""\n');
    expect(workshopScriptUnits(commentedHeader, WORLD)[0].lineOffset).toBe(4);
  });

  it('validates an immutable candidate before one shared undo entry', async () => {
    const value = draft();
    const unit = workshopScriptUnits(value, WORLD)[0];
    const original = value.read(WORLD), revision = value.sourceRevision;
    const runtime = { validate: vi.fn(async (_archive, candidate) => {
      expect(candidate.read(WORLD)).toContain('fn changed');
      expect(value.read(WORLD)).toBe(original);
      return { accepted: true, findings: [] };
    }) };
    expect(await applyWorkshopScript({ draft: value, runtime, unit, source: 'fn changed(ctx) {}',
      current: () => value.sourceRevision === revision })).toBe(true);
    expect(value.read(WORLD)).toContain('fn changed');
    value.undo();
    expect(value.read(WORLD)).toBe(original);
    expect(value.canUndo()).toBe(false);
  });

  it('keeps the draft byte-identical when ordinary validation refuses', async () => {
    const value = draft(), unit = workshopScriptUnits(value, WORLD)[0], original = value.read(WORLD);
    const runtime = { validate: vi.fn(async () => ({ accepted: false, findings: [
      { severity: 'error', file: WORLD, line: 5, message: 'Unknown function' },
    ] })) };
    await expect(applyWorkshopScript({ draft: value, runtime, unit, source: 'unknown();', current: () => true }))
      .rejects.toMatchObject({ report: { accepted: false } });
    expect(value.read(WORLD)).toBe(original);
    expect(value.canUndo()).toBe(false);
  });

  it('replaces one sibling member through the same candidate and history path', async () => {
    const value = draft('script="root.rhai"\n[global]\ntitle="Root"\n');
    value.put('assets/worlds/root.rhai', '// retained until edited\nfn start(ctx) {}\n');
    // Treat the member addition as imported source for this focused operation.
    value.markExported();
    const unit = workshopScriptUnits(value, WORLD)[0], revision = value.sourceRevision;
    expect(await applyWorkshopScript({ draft: value, runtime: { validate: async () => ({ accepted: true, findings: [] }) },
      unit, source: '// deliberate replacement\nfn start(ctx) {}\n', current: () => value.sourceRevision === revision })).toBe(true);
    expect(value.read(unit.documentPath)).toContain('deliberate replacement');
    value.undo();
    expect(value.read(unit.documentPath)).toContain('retained until edited');
  });
});
