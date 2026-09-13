// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mountWorkshopAuthoring } from '../../gui/workshop-authoring.js';
import { readStoreZip, createStoreZip } from '../../editor/mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { OPERATOR_PROFILE_KEY, createOperatorProfileSnapshot } from '../../gui/operator-profile.js';
import { t } from '../../gui/strings.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';

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
