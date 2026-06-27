import { describe, it, expect, beforeEach } from 'vitest';
import { setupEntityMode } from './slice-5-helpers.js';

/**
 * Slice 5 integration test: behaviour-view validation banner (doctrine-based AI).
 *
 * Open pirate_raider.toml (doctrine format) and remove the directive_kind
 * from the first doctrine entry. After re-render, the inline
 * `.entity-behaviour-error` banner should be present and reference the
 * missing field.
 */

describe('Slice 5: behaviour validation surfaces inline', () => {
  let ctx;
  beforeEach(async () => {
    ctx = await setupEntityMode();
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');
  });

  it('shows an inline validation banner when a doctrine entry is missing directive_kind', () => {
    // Sanity: no banner before mutation — pirate_raider.toml validates cleanly.
    const before = ctx.host.querySelectorAll('div')
      .filter((d) => d.classList.contains('entity-behaviour-error'));
    expect(before.length).toBe(0);

    // Patch the first doctrine entry to remove directive_kind.
    const behaviour = ctx.view.shell.getCard('behaviour').data;
    const patched = {
      ...behaviour,
      doctrine: [{ ...behaviour.doctrine[0], directive_kind: undefined }],
    };
    ctx.view._internal.handleCardEdit('behaviour', patched);

    const banner = ctx.host.querySelectorAll('div')
      .find((d) => d.classList.contains('entity-behaviour-error'));
    expect(banner).toBeDefined();
    expect(banner.textContent).toMatch(/directive_kind/i);
  });
});
