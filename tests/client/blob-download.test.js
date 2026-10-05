// @vitest-environment jsdom
import { it, expect, vi } from 'vitest';
import { downloadBlob } from '../../gui/blob-download.js';

it.each([false, true])('releases the anchor and URL even when click fails: %s', fails => {
  const pending = [];
  const blob = new Blob([Uint8Array.of(0, 255, 17)], { type: 'application/zip' });
  const view = {
    URL: { createObjectURL: vi.fn(() => 'blob:download'), revokeObjectURL: vi.fn() },
    setTimeout: callback => pending.push(callback),
  };
  const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function () {
    expect(this.download).toBe('pack.zip');
    expect(this.isConnected).toBe(true);
    if (fails) throw Error('click failed');
  });
  try {
    const start = () => downloadBlob(document, view, 'pack.zip', blob);
    if (fails) expect(start).toThrow('click failed');
    else start();
    expect(view.URL.createObjectURL).toHaveBeenCalledWith(blob);
    expect(document.querySelector('a[download="pack.zip"]')).toBeNull();
    expect(view.URL.revokeObjectURL).not.toHaveBeenCalled();
    expect(pending).toHaveLength(1);
    pending[0]();
    expect(view.URL.revokeObjectURL).toHaveBeenCalledWith('blob:download');
  } finally { click.mockRestore(); }
});
