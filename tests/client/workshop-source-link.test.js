// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountWorkshopSourceLink } from '../../gui/workshop-source-link.js';

let mounted;
afterEach(() => { mounted?.dispose(); mounted = null; document.body.replaceChildren(); });
function setup(capability) {
  document.body.innerHTML = '<section id="gm-console"><div id="gm-desk-brief"></div></section>';
  const getStorage = vi.fn(() => { throw new DOMException('Storage denied', 'SecurityError'); });
  const win = {
    location: { assign: vi.fn() },
    addEventListener: (...args) => window.addEventListener(...args),
    removeEventListener: (...args) => window.removeEventListener(...args),
    __hostLocalGm: () => ({ id: 'gm-a' }),
    __hostWorkshopDisconnect: vi.fn(() => true),
    ...capability,
  };
  Object.defineProperty(win, 'indexedDB', { get: getStorage });
  mounted = mountWorkshopSourceLink({ root: document.getElementById('gm-console'), win, t: id => id });
  return { win, getStorage };
}
describe('optional Workshop source control', () => {
  it('mounts without storage access when the native surface has no source-transfer capability', () => {
    const { getStorage } = setup();
    expect(document.getElementById('gm-workshop-source').hidden).toBe(true);
    expect(getStorage).not.toHaveBeenCalled();
  });
  it('contains a storage-denial error inside the requested handoff and preserves the GM connection', async () => {
    const { win, getStorage } = setup({ __hostWorkshopPacks: () => [{ id: 'pack', name: 'Source pack' }],
      __hostWorkshopSource: async () => ({ selectedId: 'pack' }) });
    expect(getStorage).not.toHaveBeenCalled();
    document.getElementById('gm-workshop-open').click();
    await vi.waitFor(() => expect(document.querySelector('#gm-workshop-source [role="status"]').textContent).toBe('workshop.source_failed'));
    expect(getStorage).toHaveBeenCalledOnce();
    expect(win.__hostWorkshopDisconnect).not.toHaveBeenCalled();
    expect(win.location.assign).not.toHaveBeenCalled();
    expect(document.getElementById('gm-workshop-open').disabled).toBe(false);
  });
});
