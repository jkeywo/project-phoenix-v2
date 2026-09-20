// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { mountWorkshopScripts } from '../../gui/workshop-scripts-panel.js';

const WORLD = 'assets/worlds/root.toml';
const makeDraft = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="p"\nname="p"\nversion="1"\n[pack.requires]\ncontent_id="base"\ncontent_epoch=1\n[[scenario]]\nid="root"\nworld="assets/worlds/root.toml"\n' },
  { path: WORLD, text: '[global]\ntitle="Root"\n[script]\nsetup="""\nfn start(ctx) {}\n"""\n' },
]));

describe('Workshop Rhai panel', () => {
  beforeEach(() => { document.body.innerHTML = '<main id="root"></main>'; });

  it('uses the runtime registry and diagnostics, then applies through shared validation/history', async () => {
    const value = makeDraft(), changed = vi.fn();
    const runtime = {
      scriptHostFunctions: vi.fn(async () => [{ name: 'on_timer', receiver: '', category: 'register', signature: 'on_timer(after_secs, handler)', summary: 'Timer' }]),
      scriptDiagnostics: vi.fn(async () => [{ severity: 'error', message: 'Expected expression', line: 5, column: 2 }]),
      validate: vi.fn(async () => ({ accepted: true, findings: [] })),
    };
    const panel = mountWorkshopScripts({ root: document.getElementById('root'), runtime, draft: () => value,
      busy: () => false, setBusy: vi.fn(), changed });
    document.querySelector('.script-list-row').click();
    await vi.waitFor(() => expect(runtime.scriptHostFunctions).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(document.querySelector('.script-diagnostic-msg')?.textContent).toContain('Expected'));
    const input = document.querySelector('.script-editor-input');
    expect(input.getAttribute('aria-label')).toContain(WORLD);
    input.value = 'fn changed(ctx) { on_timer(1, "done"); }';
    document.querySelector('.script-editor-save').click();
    await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
    expect(value.read(WORLD)).toContain('fn changed');
    value.undo();
    expect(value.read(WORLD)).toContain('fn start');
    panel.dispose();
  });

  it('publishes the complete Rhai panel editor dependency closure in the standalone build', () => {
    const build = readFileSync('scripts/build-workshop.mjs', 'utf8');
    const block = build.match(/const editorModules = \[([\s\S]*?)\];/)?.[1] || '';
    const published = new Set([...block.matchAll(/'([^']+)'/g)].map(match => match[1]));
    const panel = readFileSync('gui/workshop-scripts-panel.js', 'utf8');
    const pending = [...panel.matchAll(/from ['"]\.\.\/editor\/([^'"]+)\.js['"]/g)]
      .map(match => `editor/${match[1]}.js`);
    const seen = new Set();
    while (pending.length) {
      const file = pending.pop();
      if (seen.has(file)) continue;
      seen.add(file);
      expect(published, `${file} must be copied by build-workshop`).toContain(path.basename(file, '.js'));
      const source = readFileSync(file, 'utf8');
      for (const match of source.matchAll(/(?:from\s+|import\()\s*['"](\.[^'"]+\.js)['"]/g)) {
        const dependency = path.posix.normalize(path.posix.join(path.posix.dirname(file), match[1]));
        if (dependency.startsWith('editor/')) pending.push(dependency);
      }
    }
  });
});
