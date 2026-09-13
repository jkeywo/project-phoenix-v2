// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mountWorkshopAuthoring } from '../../gui/workshop-authoring.js';
import { readStoreZip, createStoreZip } from '../../editor/mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { OPERATOR_PROFILE_KEY, createOperatorProfileSnapshot } from '../../gui/operator-profile.js';
import { t } from '../../gui/strings.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createNativeWorkshopProvider } from '../../editor/workshop-provider.js';

let mounted;
let download;
let runtime;
const byId = id => document.getElementById(`workshop-${id}`);
const input = () => document.querySelector('input[type=file]');
async function importBytes(bytes = workshopPack()) {
  await mounted.ready;
  byId('import').click();
  Object.defineProperty(input(), 'files', { configurable: true, value: [{ arrayBuffer: async () => bytes }] });
  input().dispatchEvent(new Event('change'));
  await vi.waitFor(() => expect(byId('import').disabled).toBe(false));
}
function edit(text) { byId('source').value = text; byId('source').dispatchEvent(new Event('input')); }
function select(path) { byId('files').value = path; byId('files').dispatchEvent(new Event('change')); }
async function evaluated(id) {
  byId(id).click();
  await vi.waitFor(() => expect(byId('check').disabled).toBe(false));
}

beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  localStorage.clear();
  download = vi.fn();
  runtime = { validate: vi.fn(async bytes => {
    const checked = new WorkshopDocument(bytes).check();
    return { accepted: checked.ok, findings: (checked.errors || []).map(message => ({ severity: 'error', message, file: WORKSHOP_WORLD })) };
  }) };
  mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download, runtime });
});
afterEach(() => { mounted.dispose(); vi.restoreAllMocks(); });

describe('Workshop Authoring browser surface', () => {
  it('tests unsaved mod source with a read-only base hull and keeps Authoring and Test exclusive', async () => {
    mounted.dispose();
    vi.spyOn(window, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ version: 1, assets: [], cues: [] }) });
    const files = readStoreZip(workshopPack());
    const baseHull = 'assets/entities/base-hull.toml';
    let run = null;
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'mod', revision: 'initial', files };
      if (value.op === 'recovery-load') return { status: 'recovery', recovery: null };
      if (value.op === 'test-catalog') return { status: 'test-catalog', catalog: { worlds: [WORKSHOP_WORLD], ships: [baseHull] } };
      if (value.op === 'test-start') {
        run = { running: true, paused: false, tick: 0, multiplier: 1, selection: value.selection };
        return { status: 'test', run };
      }
      if (value.op === 'test-control') {
        if (value.control.command === 'pause') run = { ...run, paused: true };
        if (value.control.command === 'step') run = { ...run, tick: run.tick + 1 };
        return { status: 'test', run };
      }
      if (value.op === 'test-status') return { status: 'test', run };
      if (value.op === 'test-stop') { run = null; return { status: 'test', run }; }
      return { status: 'done' };
    });
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), provider: createNativeWorkshopProvider({ request }) });
    await mounted.ready;
    await vi.waitFor(() => expect(byId('test-start').disabled).toBe(false));
    expect([...byId('files').options].some(option => option.value === baseHull)).toBe(false);
    expect(byId('test-ship').value).toBe(baseHull);
    select(WORKSHOP_WORLD); edit(`${WORKSHOP_WORLD_TEXT}# unsaved first\n`);
    await vi.waitFor(() => expect(byId('test-start').disabled).toBe(false));
    byId('test-start').click();
    await vi.waitFor(() => expect(document.querySelector('.workshop-layout').hidden).toBe(true));
    expect(document.querySelector('.workshop-toolbar').hidden).toBe(true);
    expect(document.querySelector('.sound-audition').hidden).toBe(true);
    expect(byId('test-world').disabled).toBe(true);
    const first = request.mock.calls.find(([value]) => value.op === 'test-start')[0];
    expect(first.files[WORKSHOP_WORLD]).toContain('# unsaved first');
    expect(first.selection).toMatchObject({ ship: baseHull, seed: 1 });
    edit('must not enter the running draft'); // Even synthetic input is held.
    byId('test-authoring').click();
    await vi.waitFor(() => expect(document.querySelector('.workshop-layout').hidden).toBe(false));
    expect(document.querySelector('.sound-audition').hidden).toBe(false);
    expect(byId('source').value).toContain('# unsaved first');
    expect(request.mock.calls.filter(([value]) => value.op === 'test-control').map(([value]) => value.control)).toEqual([
      { command: 'pause' }, { command: 'visibility', visible: false },
    ]);
    edit(`${WORKSHOP_WORLD_TEXT}# unsaved second\n`);
    expect(byId('test-status').textContent).toContain(t('workshop.test_stale'));
    await vi.waitFor(() => expect(byId('test-start').disabled).toBe(false));
    byId('test-start').click();
    await vi.waitFor(() => expect(document.querySelector('.workshop-layout').hidden).toBe(true));
    const starts = request.mock.calls.filter(([value]) => value.op === 'test-start');
    expect(starts).toHaveLength(2);
    expect(starts[1][0].files[WORKSHOP_WORLD]).toContain('# unsaved second');
    expect(byId('test-status').textContent).not.toContain(t('workshop.test_stale'));
    byId('test-pause').click();
    await vi.waitFor(() => expect(byId('test-step').disabled).toBe(false));
    byId('test-step').click();
    await vi.waitFor(() => expect(run.tick).toBe(1));
    run = { ...run, running: false, error: 'Model load failed' };
    await vi.waitFor(() => expect(document.querySelector('.workshop-layout').hidden).toBe(false));
    expect(byId('test-status').textContent).toContain('Model load failed');
    expect(byId('source').value).toContain('# unsaved second');
    expect(request.mock.calls.some(([value]) => value.op === 'save-sources')).toBe(false);
  });

  it('retains an unavailable Test hull after source edits until an explicit replacement is selected', async () => {
    mounted.dispose();
    const files = readStoreZip(workshopPack());
    const oldHull = 'assets/entities/old.toml', newHull = 'assets/entities/new.toml';
    let ships = [oldHull];
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'mod', revision: 'initial', files };
      if (value.op === 'recovery-load') return { status: 'recovery', recovery: null };
      if (value.op === 'test-catalog') return { status: 'test-catalog', catalog: { worlds: [WORKSHOP_WORLD], ships } };
      if (value.op === 'test-stop') return { status: 'test', run: null };
      return { status: 'done' };
    });
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), provider: createNativeWorkshopProvider({ request }) });
    await mounted.ready;
    await vi.waitFor(() => expect(byId('test-start').disabled).toBe(false));
    ships = [newHull];
    select(WORKSHOP_WORLD); edit(`${WORKSHOP_WORLD_TEXT}# hull removed\n`);
    await vi.waitFor(() => expect([...byId('test-ship').options].map(option => option.value)).toContain(newHull));
    expect(byId('test-ship').value).toBe(oldHull);
    expect(byId('test-ship').selectedOptions[0].disabled).toBe(true);
    expect(byId('test-start').disabled).toBe(true);
    byId('test-start').click();
    expect(request.mock.calls.some(([value]) => value.op === 'test-start')).toBe(false);
    byId('test-ship').value = newHull; byId('test-ship').dispatchEvent(new Event('change'));
    expect(byId('test-start').disabled).toBe(false);
    byId('test-seed').value = ''; byId('test-seed').dispatchEvent(new Event('change'));
    expect(byId('test-start').disabled).toBe(true);
  });

  it.each(['assets/models/test.glb', 'assets/models/nested/buffer.bin'])('creates one pack and imports %s through chronological undo without editing dependency source', async path => {
    mounted.dispose();
    const dependencies = { base_files: { 'assets/scenarios.toml': '[content]\nid="phoenix-base"\nepoch=1\n' }, packs: [] };
    runtime.dependencies = async () => dependencies;
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download, runtime });
    await mounted.ready;
    byId('new').click();
    await vi.waitFor(() => expect(byId('files').options.length).toBe(2));
    byId('add-path').value = path;
    const assetInput = document.querySelectorAll('input[type=file]')[1];
    Object.defineProperty(assetInput, 'files', { configurable: true, value: [{ arrayBuffer: async () => new Uint8Array([0, 255, 13, 10]) }] });
    assetInput.dispatchEvent(new Event('change'));
    await vi.waitFor(() => expect(byId('files').value).toBe(path));
    expect(byId('source').disabled).toBe(true);
    expect(byId('source').value).toContain('4');
    byId('undo').click();
    expect([...byId('files').options].map(option => option.value)).not.toContain(path);
    byId('redo').click();
    expect(byId('files').value).toBe(path);
    byId('dependencies-load').click();
    await vi.waitFor(() => expect(byId('dependency-source').value).toContain('phoenix-base'));
    expect(byId('dependency-source').readOnly).toBe(true);
  });

  it('loads the native project into the same controls and retains the draft after a refused save', async () => {
    mounted.dispose();
    const files = { 'assets/scenarios.toml': Array.from(new TextEncoder().encode('[content]\nid="base"\nepoch=1\n')),
      [WORKSHOP_WORLD]: Array.from(new TextEncoder().encode(WORKSHOP_WORLD_TEXT)) };
    let saved = false;
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'project', revision: 'initial', files };
      if (value.op === 'recovery-load') return { status: 'recovery', recovery: null };
      if (value.op === 'save-sources') return saved ? { status: 'saved', revision: 'next' } : { status: 'refused', message: 'External edit', report: null };
      return { status: 'done' };
    });
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), provider: createNativeWorkshopProvider({ request }) });
    await mounted.ready;
    expect(byId('import').hidden).toBe(true);
    expect(byId('save').hidden).toBe(false);
    select(WORKSHOP_WORLD); edit(`${WORKSHOP_WORLD_TEXT}# native change\n`);
    byId('save').click();
    await vi.waitFor(() => expect(byId('save').disabled).toBe(false));
    expect(byId('source').value).toContain('# native change');
    expect(byId('dirty').textContent).toBe(t('workshop.native_dirty'));
    expect(document.querySelector('.workshop-findings').textContent).toContain('External edit');
    saved = true;
    byId('save').click();
    await vi.waitFor(() => expect(byId('dirty').textContent).toBe(t('workshop.native_clean')));
    const save = request.mock.calls.filter(([value]) => value.op === 'save-sources').at(-1)[0];
    expect(save.files[WORKSHOP_WORLD]).toContain('# native change');
  });
  it('displays and recovers native asset references without asking the document for huge byte arrays', async () => {
    mounted.dispose();
    const path = 'assets/models/large.glb';
    const reference = { asset: '0000000000000001-400000000', length: 400000000 };
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'project', revision: 'initial', files: {
        [WORKSHOP_WORLD]: Array.from(new TextEncoder().encode(WORKSHOP_WORLD_TEXT)), [path]: reference,
      } };
      if (value.op === 'recovery-load') return { status: 'recovery', recovery: null };
      return { status: 'done' };
    });
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), provider: createNativeWorkshopProvider({ request }) });
    await mounted.ready;
    select(path);
    expect(byId('source').disabled).toBe(true);
    expect(byId('source').value).toContain('400000000');
    select(WORKSHOP_WORLD); edit(`${WORKSHOP_WORLD_TEXT}# native draft\n`);
    await vi.waitFor(() => expect(request.mock.calls.some(([value]) => value.op === 'recovery-save')).toBe(true));
    const record = JSON.parse(request.mock.calls.filter(([value]) => value.op === 'recovery-save').at(-1)[0].record);
    expect(record.draft.version).toBe(3);
    expect(record.draft.sourceFiles.find(([file]) => file === path)[1]).toEqual(reference);
    expect(JSON.stringify(record).length).toBeLessThan(20000);
    expect(request.mock.calls.some(([value]) => value.op === 'asset-read')).toBe(false);
  });
  it('imports and edits with chronological cross-document undo and checked export', async () => {
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Changed\n`);
    select('scenarios.toml');
    edit(byId('source').value.replace('Workshop test', 'Edited pack'));
    byId('undo').click();
    expect(byId('files').value).toBe('scenarios.toml');
    byId('undo').click();
    expect(byId('files').value).toBe(WORKSHOP_WORLD);
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
    byId('redo').click();
    await evaluated('check');
    expect(byId('dirty').textContent).toBe(t('workshop.dirty'));
    expect(document.querySelector('.workshop-findings').textContent).toContain(t('workshop.runtime_checked'));
    await evaluated('export');
    const files = readStoreZip(download.mock.calls[0][0]);
    expect(files[WORKSHOP_WORLD]).toBe(`${WORKSHOP_WORLD_TEXT}# Changed\r\n`);
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
    expect(document.querySelector('[data-action-id="editor.mod.export"]').dataset.state).toBe('Applied');
  });

  it('refuses invalid exports with focused findings and keeps the draft', async () => {
    await importBytes();
    select(WORKSHOP_WORLD);
    edit('[global\n');
    await evaluated('export');
    expect(download).not.toHaveBeenCalled();
    expect(document.activeElement.className).toBe('workshop-findings');
    expect(byId('source').value).toBe('[global\n');
    expect(document.querySelector('[data-action-id="editor.mod.export"]').dataset.state).toBe('Refused');
    expect(byId('dirty').textContent).toBe(t('workshop.dirty'));
  });

  it('does not discard the current draft or its undo history after unreadable import', async () => {
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Keep\n`);
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    await importBytes(new Uint8Array([1, 2, 3]));
    expect(byId('source').value).toContain('# Keep');
    expect(byId('undo').disabled).toBe(false);
    expect(document.querySelector('[data-action-id="editor.mod.import"]').dataset.state).toBe('Refused');
    byId('undo').click();
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
  });

  it('keeps dirty state when a validated ZIP cannot be downloaded', async () => {
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Keep\n`);
    download.mockImplementation(() => { throw new Error('download unavailable'); });
    await evaluated('export');
    expect(byId('dirty').textContent).toBe(t('workshop.dirty'));
    expect(document.querySelector('[data-action-id="editor.mod.export"]').dataset.state).toBe('Refused');
  });

  it('localises a missing-manifest refusal and preserves the previously imported pack', async () => {
    await importBytes();
    await importBytes(createStoreZip([{ path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
    expect(document.querySelector('.workshop-findings').textContent).toContain(t('workshop.missing_manifest'));
    expect(byId('files').querySelectorAll('option')).toHaveLength(2);
    expect(document.querySelector('[data-action-id="editor.mod.import"]').dataset.state).toBe('Refused');
  });

  it('uses the shared profile text scale and remapped structural-check action', async () => {
    mounted.dispose();
    localStorage.setItem(OPERATOR_PROFILE_KEY, JSON.stringify(createOperatorProfileSnapshot({
      accessibility: { presentation: { textScale: 2, contrast: 'on' } },
      bindings: { 'editor.mod.validate': [{ type: 'keyboard', code: 'KeyQ', ctrlKey: false, shiftKey: false, altKey: false, metaKey: false }, null] },
    })));
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download, runtime });
    expect(document.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('2');
    expect(document.documentElement.getAttribute('data-contrast')).toBe('more');
    await importBytes();
    byId('check').focus();
    byId('check').dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyQ', bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(byId('check').disabled).toBe(false));
    expect(document.querySelector('[data-action-id="editor.mod.validate"]').dataset.state).toBe('Applied');
  });

  it('honours a saved Ctrl+Z action binding before conventional undo outside source fields', async () => {
    mounted.dispose();
    localStorage.setItem(OPERATOR_PROFILE_KEY, JSON.stringify(createOperatorProfileSnapshot({
      bindings: { 'editor.mod.validate': [{ type: 'keyboard', code: 'KeyZ', ctrlKey: true, shiftKey: false, altKey: false, metaKey: false }, null] },
    })));
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download, runtime });
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Must remain\n`);
    byId('check').focus();
    byId('check').dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyZ', ctrlKey: true, bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(byId('check').disabled).toBe(false));
    expect(document.querySelector('[data-action-id="editor.mod.validate"]').dataset.state).toBe('Applied');
    expect(byId('source').value).toContain('# Must remain');
    byId('source').dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyZ', ctrlKey: true, bubbles: true, cancelable: true }));
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
  });

  it('fails closed when runtime validation is unavailable and keeps source/history', async () => {
    runtime.validate.mockRejectedValue(new Error('No WASM module'));
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Keep after failed validation\n`);
    await evaluated('export');
    expect(download).not.toHaveBeenCalled();
    expect(document.querySelector('.workshop-findings').textContent).toContain(t('workshop.runtime_unavailable'));
    expect(byId('dirty').textContent).toBe(t('workshop.dirty'));
    byId('undo').click();
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
  });

  it('navigates a runtime finding to the exact unsaved document line', async () => {
    runtime.validate.mockResolvedValue({ accepted: false, findings: [
      { severity: 'error', category: 'script-parse-error', file: WORKSHOP_WORLD, line: 2, message: 'Invalid runtime source' },
    ] });
    await importBytes();
    await evaluated('check');
    document.querySelector('.workshop-findings button').click();
    expect(byId('files').value).toBe(WORKSHOP_WORLD);
    expect(document.activeElement).toBe(byId('source'));
    expect(byId('source').value.slice(byId('source').selectionStart, byId('source').selectionEnd)).toBe('[global]');
  });

  it('locks the candidate until async validation finishes and exports those exact bytes', async () => {
    let finish;
    runtime.validate.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    await importBytes();
    byId('export').click();
    expect(byId('source').disabled).toBe(true);
    expect(byId('import').disabled).toBe(true);
    expect(download).not.toHaveBeenCalled();
    finish({ accepted: true, findings: [] });
    await vi.waitFor(() => expect(download).toHaveBeenCalledOnce());
    expect(download.mock.calls[0][0]).toEqual(runtime.validate.mock.calls[0][0]);
  });

  it('applies a runtime field patch to source and the shared chronological history', async () => {
    runtime.inspect = vi.fn(async () => [{ path: ['global', 'title'], kind: 'string',
      source: "'Before'", line: 2, runtime_owned: true, default_source: null }]);
    runtime.patch = vi.fn(async (source, change) => source.replace("'Before'", change.value_source));
    await importBytes();
    select(WORKSHOP_WORLD);
    edit("# Preserve comment\n[global]\ntitle = 'Before' # Keep\n");
    await evaluated('inspect');
    expect(byId('field-info').textContent).toContain(t('workshop.field_runtime', { type: 'string', line: '2' }));
    byId('field-value').value = "'After'";
    const historyKey = new KeyboardEvent('keydown', { code: 'KeyZ', ctrlKey: true, bubbles: true, cancelable: true });
    byId('field-value').dispatchEvent(historyKey);
    expect(historyKey.defaultPrevented).toBe(false); // unapplied field input owns its ordinary text undo
    expect(byId('source').value).toContain("title = 'Before'");
    await evaluated('apply-field');
    expect(runtime.patch.mock.calls[0][1]).toMatchObject({ path: ['global', 'title'], value_source: "'After'" });
    expect(byId('source').value).toBe("# Preserve comment\n[global]\ntitle = 'After' # Keep\n");
    expect(byId('apply-field').disabled).toBe(true);
    byId('undo').click();
    expect(byId('source').value).toContain("title = 'Before' # Keep");
  });

  it('invalidates inspected fields after source changes and preserves source on refused patches', async () => {
    runtime.inspect = vi.fn(async () => [{ path: ['global', 'title'], kind: 'string',
      source: "'Before'", line: 2, runtime_owned: false }]);
    runtime.patch = vi.fn(async () => { throw 'wrong field type'; }); // wasm-bindgen string JsValue
    await importBytes();
    select(WORKSHOP_WORLD);
    await evaluated('inspect');
    const before = byId('source').value;
    await evaluated('apply-field');
    expect(byId('source').value).toBe(before);
    expect(document.querySelector('.workshop-findings').textContent).toContain('wrong field type');
    edit(`${before}# changed\n`);
    expect(byId('field').options).toHaveLength(0);
    expect(byId('apply-field').disabled).toBe(true);
  });

  it('offers recovery before replacement and restores source, selected file and history', async () => {
    mounted.dispose();
    const draft = new WorkshopDocument(workshopPack());
    draft.edit(WORKSHOP_WORLD, '[invalid\n');
    const recovery = { load: vi.fn(async () => ({ version: 1, selected: WORKSHOP_WORLD, draft: draft.snapshot() })),
      save: vi.fn(async () => {}), clear: vi.fn(async () => {}) };
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download, runtime, recovery });
    await mounted.ready;
    expect(byId('import').disabled).toBe(true);
    expect(recovery.save).not.toHaveBeenCalled();
    byId('restore').click();
    expect(byId('files').value).toBe(WORKSHOP_WORLD);
    expect(byId('source').value).toBe('[invalid\n');
    expect(byId('dirty').textContent).toBe(t('workshop.dirty'));
    byId('undo').click();
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
    await vi.waitFor(() => expect(recovery.save).toHaveBeenCalledOnce());
    expect(recovery.save.mock.calls[0][0].draft.history.redo).toHaveLength(1);
  });

  it('requires explicit discard for corrupt recovery and keeps open source after storage failure', async () => {
    mounted.dispose();
    const recovery = { load: vi.fn(async () => ({ version: 99 })),
      save: vi.fn(async () => { throw new Error('quota'); }), clear: vi.fn(async () => {}) };
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download, runtime, recovery });
    await mounted.ready;
    expect(byId('restore').hidden).toBe(true);
    expect(byId('import').disabled).toBe(true);
    byId('discard').click();
    await vi.waitFor(() => expect(byId('import').disabled).toBe(false));
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Still here\n`);
    await vi.waitFor(() => expect(byId('recovery-status').textContent).toBe(t('workshop.recovery_failed')));
    expect(byId('source').value).toContain('# Still here');
    expect(byId('undo').disabled).toBe(false);
  });
});
