import { describe, it, expect, beforeEach } from 'vitest';
import { setupEntityMode } from './slice-5-helpers.js';

/**
 * Slice 5 integration test: behaviour-view validation banner.
 *
 * Open pirate_raider.toml and set behaviour.initial_state to a name that
 * isn't in the state list. After re-render, the inline
 * `.entity-behaviour-error` banner should be present and reference the
 * bad name.
 */

describe('Slice 5: behaviour validation surfaces inline', () => {
  let ctx;
  beforeEach(async () => {
    ctx = await setupEntityMode();
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');
  });

  it('shows an inline validation banner when initial_state references a missing state', () => {
    // Sanity: no banner before mutation — pirate_raider.toml validates cleanly.
    const before = ctx.host.querySelectorAll('div')
      .filter((d) => d.classList.contains('entity-behaviour-error'));
    expect(before.length).toBe(0);

    // Patch behaviour.initial_state to a name that's not in the state list.
    const behaviour = ctx.view.shell.getCard('behaviour').data;
    const patched = { ...behaviour, initial_state: 'does_not_exist' };
    ctx.view._internal.handleCardEdit('behaviour', patched);

    const banner = ctx.host.querySelectorAll('div')
      .find((d) => d.classList.contains('entity-behaviour-error'));
    expect(banner).toBeDefined();
    expect(banner.textContent).toMatch(/initial_state/i);
    expect(banner.textContent).toContain('does_not_exist');
  });
});
