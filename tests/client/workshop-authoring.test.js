// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mountWorkshopAuthoring } from '../../gui/workshop-authoring.js';
import { readStoreZip, createStoreZip } from '../../editor/mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { OPERATOR_PROFILE_KEY, createOperatorProfileSnapshot } from '../../gui/operator-profile.js';
import { t } from '../../gui/strings.js';

let mounted;
let download;
const byId = id => document.getElementById(`workshop-${id}`);
const input = () => document.querySelector('input[type=file]');
async function importBytes(bytes = workshopPack()) {
  byId('import').click();
  Object.defineProperty(input(), 'files', { configurable: true, value: [{ arrayBuffer: async () => bytes }] });
  input().dispatchEvent(new Event('change'));
  await vi.waitFor(() => expect(byId('import').disabled).toBe(false));
}
function edit(text) { byId('source').value = text; byId('source').dispatchEvent(new Event('input')); }
function select(path) { byId('files').value = path; byId('files').dispatchEvent(new Event('change')); }

beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  localStorage.clear();
  download = vi.fn();
  mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download });
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
    byId('check').click();
    expect(byId('dirty').textContent).toBe(t('workshop.dirty'));
    expect(document.querySelector('.workshop-findings').textContent).toContain('host still validates');
    byId('export').click();
    const files = readStoreZip(download.mock.calls[0][0]);
    expect(files[WORKSHOP_WORLD]).toBe(`${WORKSHOP_WORLD_TEXT}# Changed\r\n`);
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
    expect(document.querySelector('[data-action-id="editor.mod.export"]').dataset.state).toBe('Applied');
  });

  it('refuses invalid exports with focused findings and keeps the draft', async () => {
    await importBytes();
    select(WORKSHOP_WORLD);
    edit('[global\n');
    byId('export').click();
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
    byId('export').click();
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
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download });
    expect(document.documentElement.style.getPropertyValue('--a11y-text-scale')).toBe('2');
    expect(document.documentElement.getAttribute('data-contrast')).toBe('more');
    await importBytes();
    byId('check').focus();
    byId('check').dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyQ', bubbles: true, cancelable: true }));
    expect(document.querySelector('[data-action-id="editor.mod.validate"]').dataset.state).toBe('Applied');
  });

  it('honours a saved Ctrl+Z action binding before conventional undo outside source fields', async () => {
    mounted.dispose();
    localStorage.setItem(OPERATOR_PROFILE_KEY, JSON.stringify(createOperatorProfileSnapshot({
      bindings: { 'editor.mod.validate': [{ type: 'keyboard', code: 'KeyZ', ctrlKey: true, shiftKey: false, altKey: false, metaKey: false }, null] },
    })));
    mounted = mountWorkshopAuthoring({ root: document.getElementById('root'), download });
    await importBytes();
    select(WORKSHOP_WORLD);
    edit(`${WORKSHOP_WORLD_TEXT}# Must remain\n`);
    byId('check').focus();
    byId('check').dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyZ', ctrlKey: true, bubbles: true, cancelable: true }));
    expect(document.querySelector('[data-action-id="editor.mod.validate"]').dataset.state).toBe('Applied');
    expect(byId('source').value).toContain('# Must remain');
    byId('source').dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyZ', ctrlKey: true, bubbles: true, cancelable: true }));
    expect(byId('dirty').textContent).toBe(t('workshop.saved'));
  });
});
