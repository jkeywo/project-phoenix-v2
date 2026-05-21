import { describe, it, expect, beforeEach } from 'vitest';
import { setupEntityMode, fireClick } from './slice-5-helpers.js';

/**
 * Slice 5 integration test: stations-view tab + dangling-next error.
 *
 * Open player_ship.toml, click the count-4 tab, set a station's `next`
 * to a name not present at count 5 → the inline `dangling-next` chip
 * renders next to that station's `next` dropdown.
 */

describe('Slice 5: stations tab + dangling-next inline error', () => {
  let ctx;
  beforeEach(async () => {
    const { _resetActiveCountForTest } = await import('../entity-stations-view.js');
    _resetActiveCountForTest();
    ctx = await setupEntityMode();
    await ctx.view._internal.loadEntity('assets/entities/player_ship.toml');
  });

  it('clicking count tab 4 makes it the active tab', () => {
    const tabs = ctx.host.querySelectorAll('button')
      .filter((b) => b.classList.contains('entity-stations-tab'));
    expect(tabs.length).toBeGreaterThan(0);

    const tab4 = tabs.find((b) => b.textContent === '4');
    expect(tab4).toBeDefined();
    fireClick(tab4);

    const tabsAfter = ctx.host.querySelectorAll('button')
      .filter((b) => b.classList.contains('entity-stations-tab'));
    const active = tabsAfter.find((b) => b.classList.contains('entity-stations-tab-active'));
    expect(active).toBeDefined();
    expect(active.textContent).toBe('4');
  });

  it('setting a station next to a dangling name surfaces inline dangling-next chip', () => {
    // Activate count-4 tab.
    const tabs = ctx.host.querySelectorAll('button')
      .filter((b) => b.classList.contains('entity-stations-tab'));
    const tab4 = tabs.find((b) => b.textContent === '4');
    fireClick(tab4);

    // Mutate count-4 station 'Helm' so its `next` points at a non-existent
    // station at count 5. Edit through the data path so we can plant a
    // value that the dropdown wouldn't normally allow.
    const stations = structuredClone(ctx.view.shell.getCard('stations').data);
    const helm4 = stations['4'].find((s) => s.name === 'Helm');
    expect(helm4).toBeDefined();
    helm4.next = 'NoSuchStation';
    ctx.view._internal.handleCardEdit('stations', stations);

    // After re-render, the count-4 tab must still be active so the
    // validation block + per-row chip render here.
    const activeAfter = ctx.host.querySelectorAll('button')
      .filter((b) => b.classList.contains('entity-stations-tab-active'));
    expect(activeAfter.length).toBe(1);
    expect(activeAfter[0].textContent).toBe('4');

    // The error list at the top of the tab body should mention dangling-next.
    const chips = ctx.host.querySelectorAll('div')
      .filter((d) => d.classList.contains('entity-stations-error'));
    expect(chips.length).toBeGreaterThan(0);
    expect(chips.some((c) => c.dataset.kind === 'dangling-next')).toBe(true);
    expect(chips.some((c) => /dangling next/i.test(c.textContent))).toBe(true);

    // The inline-error chip is attached next to the Helm row's `next` field.
    const inlineChips = ctx.host.querySelectorAll('span')
      .filter((s) => s.classList.contains('entity-stations-inline-error'));
    expect(inlineChips.length).toBeGreaterThan(0);
    expect(inlineChips.some((c) => c.dataset.kind === 'dangling-next')).toBe(true);
  });
});
