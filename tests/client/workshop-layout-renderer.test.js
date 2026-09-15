// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  defaultWorkshopLayout, dockWorkshopPanel, selectWorkshopPanel,
} from '../../gui/workshop-layout-model.js';
import { mountWorkshopLayout } from '../../gui/workshop-layout-renderer.js';

const labels = {
  switcher: 'Panels', reset: 'Reset', float: 'Float', close: 'Close',
  dock: { left: 'Dock left', right: 'Dock right', top: 'Dock above', bottom: 'Dock below', tab: 'Dock as tab' },
  panels: { files: 'Files', source: 'Source', inspector: 'Inspector', add: 'Add files', recovery: 'Recovery' },
};

function relation(node, first, second) {
  if (node.type === 'tabs' && node.tabs.includes(first) && node.tabs.includes(second)) return 'tab';
  if (node.type !== 'split') return null;
  const firstIndex = node.children.findIndex(child => JSON.stringify(child).includes(`"${first}"`));
  const secondIndex = node.children.findIndex(child => JSON.stringify(child).includes(`"${second}"`));
  if (firstIndex >= 0 && secondIndex >= 0 && firstIndex !== secondIndex) {
    if (node.axis === 'horizontal') return firstIndex < secondIndex ? 'left' : 'right';
    return firstIndex < secondIndex ? 'top' : 'bottom';
  }
  return node.children.map(child => relation(child, first, second)).find(Boolean) || null;
}

function mount(initial = defaultWorkshopLayout()) {
  document.body.innerHTML = '<main id="root"><div id="surface"></div></main>';
  const root = document.getElementById('root');
  const surface = document.getElementById('surface');
  const panels = Object.fromEntries(['files', 'source', 'inspector', 'add', 'recovery'].map(panel => {
    const node = document.createElement('div'); node.textContent = panel; return [panel, node];
  }));
  const changes = [];
  const mounted = mountWorkshopLayout({ root, surface, panels, labels, initial, onChange: state => changes.push(state) });
  return { mounted, surface, changes };
}

function pointer(type, x, y, pointerType = 'touch') {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0 });
  Object.defineProperty(event, 'pointerId', { value: 1 });
  Object.defineProperty(event, 'pointerType', { value: pointerType });
  return event;
}

function pointerDock(panel, target) {
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: vi.fn(() => target) });
  const tab = document.querySelector(`[data-panel="${panel}"] .workshop-panel-tab`);
  tab.dispatchEvent(pointer('pointerdown', 10, 10));
  tab.dispatchEvent(pointer('pointermove', 30, 30));
  tab.dispatchEvent(pointer('pointerup', 30, 30));
}

describe('Workshop layout renderer', () => {
  let mounted, changes;
  beforeEach(() => { window.requestAnimationFrame = callback => callback(); });
  afterEach(() => { mounted?.dispose(); vi.restoreAllMocks(); });

  it.each(['left', 'right', 'top', 'bottom', 'tab'])('offers touch pointer %s docking', placement => {
    ({ mounted } = mount());
    const target = document.querySelector(`[data-panel="source"] [data-placement="${placement}"]`);
    pointerDock('files', target);
    expect(relation(mounted.state().root, 'files', 'source')).toBe(placement);
  });

  it.each([
    ['ArrowLeft', 'left'], ['ArrowRight', 'right'], ['ArrowUp', 'top'], ['ArrowDown', 'bottom'],
  ])('docks dynamically ordered panels with %s', (code, placement) => {
    const initial = dockWorkshopPanel(defaultWorkshopLayout(), 'inspector', 'files', 'tab');
    ({ mounted } = mount(initial));
    const inspector = document.querySelector('[data-panel="inspector"] .workshop-panel-tab');
    inspector.dispatchEvent(new KeyboardEvent('keydown', {
      code, ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true,
    }));
    const target = ['ArrowLeft', 'ArrowUp'].includes(code) ? 'files' : 'source';
    expect(relation(mounted.state().root, 'inspector', target)).toBe(placement);
  });

  it('tabs against the next panel in current spatial order', () => {
    const initial = dockWorkshopPanel(defaultWorkshopLayout(), 'inspector', 'files', 'tab');
    ({ mounted } = mount(initial));
    const inspector = document.querySelector('[data-panel="inspector"] .workshop-panel-tab');
    inspector.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'ArrowRight', ctrlKey: true, shiftKey: true, altKey: true, bubbles: true, cancelable: true,
    }));
    expect(relation(mounted.state().root, 'inspector', 'source')).toBe('tab');
  });

  it.each([
    ['ArrowRight', 'inspector'], ['ArrowLeft', 'inspector'], ['Home', 'source'], ['End', 'inspector'],
  ])('uses roving tab focus and activates tabs with %s', (key, expected) => {
    const initial = selectWorkshopPanel(
      dockWorkshopPanel(defaultWorkshopLayout(), 'inspector', 'source', 'tab'), 'source',
    );
    ({ mounted } = mount(initial));
    const source = document.querySelector('[role="tab"][data-layout-panel="source"]');
    expect(source.tabIndex).toBe(0);
    expect(document.querySelector('[role="tab"][data-layout-panel="inspector"]').tabIndex).toBe(-1);
    source.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
    expect(mounted.state().selected).toBe(expected);
    expect(document.activeElement).toBe(document.querySelector(
      `[role="tab"][data-layout-panel="${expected}"]`,
    ));
  });

  it('associates every tab with its labelled tabpanel', () => {
    const initial = dockWorkshopPanel(defaultWorkshopLayout(), 'inspector', 'source', 'tab');
    ({ mounted } = mount(initial));

    for (const tab of document.querySelectorAll('[role="tab"]')) {
      expect(tab.id).not.toBe('');
      const panel = document.getElementById(tab.getAttribute('aria-controls'));
      expect(panel?.getAttribute('role')).toBe('tabpanel');
      expect(panel?.getAttribute('aria-labelledby')).toBe(tab.id);
    }
  });

  it('keeps deterministic float order and raises the selected or focused float without persisting z-index', () => {
    const initial = {
      version: 2, root: null, closed: [], selected: 'source',
      floats: [
        { panel: 'files', x: 12, y: 12, width: 300, height: 200 },
        { panel: 'source', x: 40, y: 40, width: 300, height: 200 },
        { panel: 'inspector', x: 68, y: 68, width: 300, height: 200 },
      ],
    };
    ({ mounted } = mount(initial));
    const floating = panel => document.querySelector(`[data-panel="${panel}"].is-floating`);
    expect(floating('source').style.zIndex).toBe('14');
    expect(floating('files').style.zIndex).toBe('10');
    floating('files').querySelector('[data-layout-control="close"]').focus();
    expect(floating('files').style.zIndex).toBe('16');
    expect(mounted.state().floats.every(entry => !('zIndex' in entry))).toBe(true);
  });

  it('keeps the outer canvas floor while nested split tracks can shrink', () => {
    ({ mounted } = mount());
    const files = document.querySelector('[data-panel="files"] .workshop-panel-tab');
    files.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'ArrowDown', ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true,
    }));
    const nested = document.querySelector('.workshop-split .workshop-split.is-vertical');
    expect(nested.style.gridTemplateRows).toMatch(/^minmax\(0, .+fr\) minmax\(0, .+fr\)$/);

    const css = readFileSync('gui/workshop.css', 'utf8');
    expect(css).toMatch(/\.workshop-dock-canvas\s*\{[^}]*min-height:\s*32rem/);
    expect(css).toMatch(/\.workshop-split\s*\{[^}]*min-height:\s*0/);
    expect(css).not.toMatch(/\.workshop-split\s*\{[^}]*min-height:\s*32rem/);
  });

  it('returns focus to the switcher after closing the final panel', () => {
    ({ mounted } = mount());
    for (const panel of ['files', 'source', 'inspector', 'add', 'recovery']) {
      document.querySelector(`[data-panel="${panel}"] [data-layout-control="close"]`).click();
    }
    expect(document.querySelector('.workshop-dock-panel')).toBeNull();
    expect(document.activeElement).toBe(document.querySelector(
      '[data-layout-panel="recovery"][data-layout-control="switcher"]',
    ));
  });

  it('keeps narrow rendering as a projection and restores desktop float geometry', () => {
    const initial = {
      ...defaultWorkshopLayout(),
      root: defaultWorkshopLayout().root.children[1],
      floats: [{ panel: 'files', x: 2000, y: 2000, width: 400, height: 300 }],
      closed: ['inspector'],
    };
    ({ mounted, changes } = mount(initial));
    expect(mounted.state().floats[0]).toMatchObject({ x: 624, y: 468, width: 400, height: 300 });
    const desktop = mounted.state();

    const floatButton = document.querySelector('[data-panel="files"] [data-layout-control="float"]');
    floatButton.focus();
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 600 });
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 500 });
    window.dispatchEvent(new Event('resize'));
    expect(mounted.state()).toEqual(desktop);
    expect(document.querySelectorAll('.workshop-dock-panel')).toHaveLength(1);
    expect(changes).toHaveLength(0);
    expect(document.activeElement).toBe(document.querySelector(
      '[data-layout-panel="files"][data-layout-control="switcher"]',
    ));

    document.querySelector('[data-layout-panel="inspector"][data-layout-control="switcher"]').click();
    expect(document.querySelector('[data-panel="inspector"]')).not.toBeNull();
    expect(mounted.state()).toEqual(desktop);
    expect(changes).toHaveLength(0);

    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1024 });
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 768 });
    window.dispatchEvent(new Event('resize'));
    expect(mounted.state()).toEqual(desktop);
    expect(document.querySelector('[data-panel="files"].is-floating').style.left).toBe('624px');
    expect(document.querySelector('[data-panel="inspector"]')).toBeNull();
  });
});
