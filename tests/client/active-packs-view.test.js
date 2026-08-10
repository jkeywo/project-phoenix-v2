import { describe, it, expect } from 'vitest';
import { activePacksView, scenarioOriginBadge } from '../../gui/lobby-state.js';

// Pure view helpers for the "players see the active mods the host applied"
// surface (issue #990): the lobby "Mods active" list rows and the per-scenario
// mod-origin badge. DOM-free so client.html's rendering logic is unit-tested
// without a browser.

describe('activePacksView (Mods active list rows)', () => {
  it('is empty for no packs — the lobby renders no empty banner', () => {
    expect(activePacksView([])).toEqual([]);
    expect(activePacksView(null)).toEqual([]);
    expect(activePacksView(undefined)).toEqual([]);
  });

  it('maps each pack to a name + version row', () => {
    const rows = activePacksView([
      { id: 'aurora-skirmish', name: 'Aurora Skirmish', version: '1.0.0' },
      { id: 'nebula-run', name: 'Nebula Run', version: '2.1' },
    ]);
    expect(rows).toEqual([
      { name: 'Aurora Skirmish', version: '1.0.0' },
      { name: 'Nebula Run', version: '2.1' },
    ]);
  });

  it('falls back to the pack id when it has no display name', () => {
    expect(activePacksView([{ id: 'aurora-skirmish', version: '1.0.0' }]))
      .toEqual([{ name: 'aurora-skirmish', version: '1.0.0' }]);
  });

  it('defaults a missing version to the empty string', () => {
    expect(activePacksView([{ id: 'p', name: 'Pack' }]))
      .toEqual([{ name: 'Pack', version: '' }]);
  });

  it('drops a pack with neither name nor id', () => {
    expect(activePacksView([{ version: '9' }, { id: 'keep', name: 'Keep' }]))
      .toEqual([{ name: 'Keep', version: '' }]);
  });
});

describe('scenarioOriginBadge (per-scenario mod badge)', () => {
  const PACKS = [{ id: 'aurora-skirmish', name: 'Aurora Skirmish', version: '1.0.0' }];

  it('gives a base scenario no badge (source "base")', () => {
    expect(scenarioOriginBadge({ id: 'default', source: 'base' }, PACKS)).toBeNull();
  });

  it('gives a scenario with no source no badge', () => {
    expect(scenarioOriginBadge({ id: 'default' }, PACKS)).toBeNull();
    expect(scenarioOriginBadge({ id: 'default', source: '' }, PACKS)).toBeNull();
  });

  it('badges a mod scenario with the applied pack display name', () => {
    expect(scenarioOriginBadge({ id: 'aurora', source: 'aurora-skirmish' }, PACKS))
      .toEqual({ name: 'Aurora Skirmish' });
  });

  it('falls back to the raw source id when the pack is not in the active list', () => {
    expect(scenarioOriginBadge({ id: 'x', source: 'ghost-pack' }, PACKS))
      .toEqual({ name: 'ghost-pack' });
    expect(scenarioOriginBadge({ id: 'x', source: 'ghost-pack' }, []))
      .toEqual({ name: 'ghost-pack' });
  });

  it('accepts the internal `origin` field name as a fallback', () => {
    // The Rust catalog entry field is `origin`; the wire field is `source`.
    // Accept either so a caller reading the raw catalog also gets a badge.
    expect(scenarioOriginBadge({ id: 'a', origin: 'aurora-skirmish' }, PACKS))
      .toEqual({ name: 'Aurora Skirmish' });
  });
});
