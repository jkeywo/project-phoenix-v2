// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { parse } from 'smol-toml';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { mountWorkshopScripts } from '../../gui/workshop-scripts-panel.js';
import { buildTable, setTable } from '../../gui/strings.js';

const WORLD = 'assets/worlds/root.toml';
const makeDraft = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="p"\nname="p"\nversion="1"\n[pack.requires]\ncontent_id="base"\ncontent_epoch=1\n[[scenario]]\nid="root"\nworld="assets/worlds/root.toml"\n' },
  { path: WORLD, text: '[global]\ntitle="Root"\n[script]\nsetup="""\nfn start(ctx) {}\n"""\n' },
]));
const makeScriptlessDraft = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="p"\nname="p"\nversion="1"\n[pack.requires]\ncontent_id="base"\ncontent_epoch=1\n[[scenario]]\nid="root"\nworld="assets/worlds/root.toml"\n' },
  { path: WORLD, text: '[global]\ntitle="Root"\n' },
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

  it('inserts a scoped instance and addressed completion for runtime checked save', async () => {
    const value = makeDraft(), changed = vi.fn();
    const runtime = { scriptHostFunctions: async () => [], scriptDiagnostics: async () => [],
      validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const panel = mountWorkshopScripts({ root: document.getElementById('root'), runtime, draft: () => value,
      busy: () => false, setBusy: vi.fn(), changed });
    document.querySelector('.script-list-row').click();
    await vi.waitFor(() => expect(document.querySelector('.script-editor-input')).not.toBeNull());
    document.getElementById('workshop-objective-id').value = 'escort';
    document.getElementById('workshop-objective-instance-id').value = 'lead';
    document.getElementById('workshop-objective-text').value = 'objective.escort';
    document.getElementById('workshop-objective-slots').value = 'lead';
    document.getElementById('workshop-objective-delay').value = '30';
    document.getElementById('workshop-objective-insert').click();
    const buffer = document.querySelector('.script-editor-input').value;
    expect(buffer).toContain('recipient_ship_slots: ["lead"]');
    expect(buffer).toContain('complete_objective("escort", "lead")');
    expect(value.read(WORLD)).not.toContain('recipient_ship_slots');
    document.querySelector('.script-editor-save').click();
    await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
    expect(runtime.validate).toHaveBeenCalledOnce();
    expect(parse(value.read(WORLD)).script.setup).toContain('recipient_ship_slots: ["lead"]');
    expect(parse(WorkshopDocument.restore(value.snapshot()).read(WORLD)).script.setup)
      .toContain('complete_objective("escort", "lead")');
    expect(parse(new WorkshopDocument(value.archive()).read(WORLD)).script.setup)
      .toContain('recipient_ship_slots: ["lead"]');
    value.undo();
    expect(value.read(WORLD)).not.toContain('recipient_ship_slots');
    panel.dispose();
  });

  it('inserts an addressed modifier using the same recipient fields', async () => {
    const value = makeDraft();
    const panel = mountWorkshopScripts({ root: document.getElementById('root'),
      runtime: { scriptHostFunctions: async () => [], scriptDiagnostics: async () => [],
        validate: async () => ({ accepted: true, findings: [] }) }, draft: () => value,
      busy: () => false, setBusy: vi.fn() });
    document.querySelector('.script-list-row').click();
    await vi.waitFor(() => expect(document.querySelector('.script-editor-input')).not.toBeNull());
    document.getElementById('workshop-objective-id').value = 'escort';
    document.getElementById('workshop-objective-instance-id').value = 'pair';
    document.getElementById('workshop-objective-slots').value = 'lead';
    document.getElementById('workshop-action-members').checked = true;
    document.getElementById('workshop-action-slot').value = 'MaxSpeed';
    document.getElementById('workshop-action-tag').value = 'escort_boost';
    document.getElementById('workshop-action-bonus').value = '1.5';
    document.getElementById('workshop-action-insert').click();
    const source = document.querySelector('.script-editor-input').value;
    expect(source).toContain('ctx.effects.addressed(#{');
    expect(source).toContain('recipient_objective_instances: [#{ objective_id: "escort", instance_id: "pair" }]');
    panel.dispose();
  });

  it('keeps an invalid selector out of history and reports its source location', async () => {
    const value = makeDraft();
    const runtime = { scriptHostFunctions: async () => [], scriptDiagnostics: async () => [],
      validate: vi.fn(async () => ({ accepted: false, findings: [{ file: WORLD, line: 9,
        message: 'unknown ship slot "ghost"' }] })) };
    const panel = mountWorkshopScripts({ root: document.getElementById('root'), runtime, draft: () => value,
      busy: () => false, setBusy: vi.fn() });
    document.querySelector('.script-list-row').click();
    await vi.waitFor(() => expect(document.querySelector('.script-editor-input')).not.toBeNull());
    document.getElementById('workshop-objective-id').value = 'escort';
    document.getElementById('workshop-objective-instance-id').value = 'lead';
    document.getElementById('workshop-objective-text').value = 'objective.escort';
    document.getElementById('workshop-objective-slots').value = 'ghost';
    document.getElementById('workshop-objective-insert').click();
    document.querySelector('.script-editor-save').click();
    await vi.waitFor(() => expect(document.getElementById('workshop-script-status').textContent)
      .toContain(`${WORLD}:9: unknown ship slot "ghost"`));
    expect(document.getElementById('workshop-script-status').getAttribute('role')).toBe('alert');
    expect(value.read(WORLD)).not.toContain('ghost');
    panel.dispose();
  });

  it('can start a script in a world with none, then author an Objective', async () => {
    const value = makeScriptlessDraft(), changed = vi.fn();
    const runtime = { scriptHostFunctions: async () => [], scriptDiagnostics: async () => [],
      validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const panel = mountWorkshopScripts({ root: document.getElementById('root'), runtime, draft: () => value,
      busy: () => false, setBusy: vi.fn(), changed });
    expect(document.querySelectorAll('.script-list-row')).toHaveLength(0);
    document.getElementById('workshop-script-create').click();
    await vi.waitFor(() => expect(document.querySelector('.script-editor-input')).not.toBeNull());
    expect(changed).toHaveBeenCalledWith(WORLD);
    expect(parse(value.read(WORLD)).script.setup).toContain('Add scenario callbacks');
    value.undo();
    expect(parse(value.read(WORLD)).script).toBeUndefined();
    panel.dispose();
  });

  it('presents translated labels and focuses actionable validation errors', async () => {
    setTable(buildTable('id,context,en,de\nworkshop.objective.title,heading,Objective instance,Zielinstanz\nworkshop.objective.id,label,Objective ID,Zielkennung\nworkshop.objective.action_title,heading,Ship-addressed action,Schiffsbezogene Aktion\nworkshop.scripts.validation_refused,error,Validation refused,Prüfung fehlgeschlagen\n', 'de'));
    const value = makeDraft();
    const runtime = { scriptHostFunctions: async () => [], scriptDiagnostics: async () => [],
      validate: async () => ({ accepted: false, findings: [{ file: WORLD, line: 8,
        message: 'unknown instance escort/ghost' }] }) };
    const panel = mountWorkshopScripts({ root: document.getElementById('root'), runtime, draft: () => value,
      busy: () => false, setBusy: vi.fn() });
    expect(document.querySelector('#workshop-objective-instance legend').textContent).toBe('Zielinstanz');
    expect(document.querySelector('#workshop-addressed-action legend').textContent).toBe('Schiffsbezogene Aktion');
    expect(document.querySelector('label[for="workshop-objective-id"]').textContent).toBe('Zielkennung');
    document.querySelector('.script-list-row').click();
    await vi.waitFor(() => expect(document.querySelector('.script-editor-input')).not.toBeNull());
    document.querySelector('.script-editor-input').value = 'fn changed(ctx) {}';
    document.querySelector('.script-editor-save').click();
    await vi.waitFor(() => expect(document.getElementById('workshop-script-status').textContent)
      .toContain(`${WORLD}:8: unknown instance escort/ghost`));
    const status = document.getElementById('workshop-script-status');
    expect(status.textContent).toContain('Prüfung fehlgeschlagen');
    expect(document.activeElement).toBe(status);
    panel.dispose();
    setTable(new Map());
  });

  it('frames literal validation detail in German through a language repaint', async () => {
    setTable(buildTable('id,context,en,de\nworkshop.scripts.operation_refused,error,Script operation failed,Schreibvorgang fehlgeschlagen\n', 'de'));
    const value = makeDraft();
    const panel = mountWorkshopScripts({ root: document.getElementById('root'), draft: () => value,
      runtime: { scriptHostFunctions: async () => [], scriptDiagnostics: async () => [],
        validate: async () => { throw new Error('literal compiler detail'); } },
      busy: () => false, setBusy: vi.fn(), changed: vi.fn() });
    document.querySelector('.script-list-row').click();
    await vi.waitFor(() => expect(document.querySelector('.script-editor-input')).not.toBeNull());
    document.querySelector('.script-editor-input').value = 'fn changed(ctx) {}';
    document.querySelector('.script-editor-save').click();
    await vi.waitFor(() => expect(document.getElementById('workshop-script-status').textContent)
      .toContain('literal compiler detail'));
    expect(document.getElementById('workshop-script-status').textContent).toContain('Schreibvorgang fehlgeschlagen');
    expect(document.getElementById('workshop-script-status').getAttribute('role')).toBe('alert');
    expect(document.activeElement).toBe(document.getElementById('workshop-script-status'));
    panel.refreshLanguage();
    expect(document.getElementById('workshop-script-status').textContent).toContain('literal compiler detail');
    expect(document.getElementById('workshop-script-status').textContent).toContain('Schreibvorgang fehlgeschlagen');
    expect(value.read(WORLD)).toContain('fn start');
    panel.dispose();
    setTable(new Map());
  });
});
