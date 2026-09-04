import { describe, it, expect, beforeEach } from 'vitest';
import { setupDefinitionsMode } from './slice-6-helpers.js';

/**
 * Slice-6 integration tests for Definitions Mode.
 *
 *   AC1: Click Definitions tab → two-section layout.
 *   AC2: Open faction file → UUID readonly + name input + enemy multi-select.
 *   AC3: Enemy multi-select shows faction NAMES (not UUIDs).
 *   AC4: Open complexity file → presets + hidden_elements + delegated + AI.
 *   AC5: Save / undo round-trip.
 */

describe('Slice 6: Definitions Mode', () => {
  let ctx;
  beforeEach(async () => {
    ctx = await setupDefinitionsMode();
  });

  it('mounts a two-section layout with both file lists populated (AC1)', () => {
    const sections = ctx.host.querySelectorAll('.definitions-section');
    expect(sections.length).toBe(2);
    expect(sections[0].classList.contains('definitions-section-faction')).toBe(true);
    expect(sections[1].classList.contains('definitions-section-complexity')).toBe(true);

    const factionRows = sections[0].querySelectorAll('.definitions-file-list-row');
    const complexityRows = sections[1].querySelectorAll('.definitions-file-list-row');
    expect(factionRows.length).toBe(4);
    // Complexity presets were retired in #534; the section renders empty
    // until a complexity TOML file is loaded.
    expect(complexityRows.length).toBe(0);
  });

  it('clicking a faction file populates the form with UUID + name + enemies (AC2)', async () => {
    await ctx.view._internal.loadFactionFile('assets/factions/alliance.toml');

    const formRoot = ctx.host.querySelector('.faction-form');
    expect(formRoot).toBeTruthy();

    const uuid = ctx.host.querySelector('.def-uuid-readonly');
    expect(uuid.textContent).toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');

    const nameInput = ctx.host.querySelector('.def-name-input');
    expect(nameInput.value).toBe('Alliance');

    const select = ctx.host.querySelector('.def-multi-select');
    expect(select).toBeTruthy();
  });

  it('enemy multi-select option labels are faction NAMES, not UUIDs (AC3)', async () => {
    await ctx.view._internal.loadFactionFile('assets/factions/alliance.toml');

    const select = ctx.host.querySelector('.def-multi-select');
    const options = select.querySelectorAll('OPTION');
    const texts = options.map((o) => o.textContent).sort();

    // The other three factions (Alliance is excluded as the active file).
    expect(texts).toEqual(['Harrow', 'Pirate', 'Requiem']);

    // The UUID lives in `.value`, never in the visible text.
    for (const opt of options) {
      expect(opt.value).toMatch(/^[a-f0-9]{8}-/);
      expect(opt.textContent).not.toBe(opt.value);
    }
  });

  it('mutating a faction name stages content + marks dirty + saves via TOML stringifier; fireFactionSaved fires (AC5)', async () => {
    await ctx.view._internal.loadFactionFile('assets/factions/alliance.toml');

    const factionFired = [];
    ctx.invalidationBus.onFactionSaved((p) => factionFired.push(p));

    ctx.view._internal.handleFactionNameChange('United Alliance');

    const path = 'assets/factions/alliance.toml';
    expect(ctx.modeShell.isDirty('Definitions', path)).toBe(true);
    const stash = ctx.saveFlow._contentCache.Definitions[path];
    expect(stash.kind).toBe('faction');
    expect(stash.data.name).toBe('United Alliance');

    const undoEntries = ctx.modeShell.getUndoHistory('Definitions', path);
    expect(undoEntries.length).toBe(1);
    expect(undoEntries[0].data.name).toBe('Alliance');

    ctx.modeShell.setActiveFile('Definitions', path);
    const result = await ctx.saveFlow.saveActive();
    expect(result.ok).toBe(true);
    expect(ctx.writeFileCalls.length).toBe(1);
    expect(ctx.writeFileCalls[0].path).toBe(path);
    expect(ctx.writeFileCalls[0].content).toContain('United Alliance');
    expect(ctx.modeShell.isDirty('Definitions', path)).toBe(false);
    expect(factionFired).toEqual([path]);
  });

  it('undo restores prior faction name', async () => {
    await ctx.view._internal.loadFactionFile('assets/factions/alliance.toml');
    ctx.view._internal.handleFactionNameChange('United Alliance');

    const restore = ctx.getRestoreCb();
    expect(restore).toBeDefined();
    restore(ctx.modeShell, 'assets/factions/alliance.toml', 'undo');

    expect(ctx.view.factionEditor.getFormState().name).toBe('Alliance');
  });
});
