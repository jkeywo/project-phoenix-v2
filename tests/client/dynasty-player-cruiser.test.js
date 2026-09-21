// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

describe('Dynasty player-cruiser console delivery', () => {
  beforeEach(() => {
    document.head.innerHTML = '';
    vi.resetModules();
  });

  it('loads the authored Dynasty theme inside the Station iframe realm', async () => {
    await import('../../gui/dynasty-cruiser/console.js');
    const theme = document.querySelector('link[data-dynasty-cruiser-theme]');
    expect(theme).not.toBeNull();
    expect(theme.getAttribute('rel')).toBe('stylesheet');
    expect(theme.getAttribute('href')).toBe('../themes/dynasty-cruiser.css');
  });
});
