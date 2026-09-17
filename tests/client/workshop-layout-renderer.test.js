// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  closeWorkshopPanel, defaultWorkshopLayout, dockWorkshopPanel, selectWorkshopPanel, WORKSHOP_PANELS,
} from '../../gui/workshop-layout-model.js';
import { mountWorkshopLayout } from '../../gui/workshop-layout-renderer.js';

const labels = {
  switcher: 'Panels', reset: 'Reset', float: 'Float', close: 'Close',
  dock: { left: 'Dock left', right: 'Dock right', top: 'Dock above', bottom: 'Dock below', tab: 'Dock as tab' },
  panels: Object.fromEntries(WORKSHOP_PANELS.map(panel => [panel, panel])),
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

function mount(initial = defaultWorkshopLayout(), options = {}) {
  document.body.innerHTML = '<main id="root"><div id="surface"></div></main>';
  const root = document.getElementById('root');
  const surface = document.getElementById('surface');
  const panels = Object.fromEntries(WORKSHOP_PANELS.map(panel => {
    const node = document.createElement('div'); node.textContent = panel; return [panel, node];
  }));
  const changes = [];
  const mounted = mountWorkshopLayout({ root, surface, panels, labels, initial,
    onChange: state => changes.push(state), ...options });
  return { mounted, surface, changes, panels };
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
    ['ArrowRight', 'findings'], ['ArrowLeft', 'inspector'], ['Home', 'source'], ['End', 'inspector'],
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
      version: 3, root: null, closed: ['findings', 'feedback', 'dependencies', 'settings'], selected: 'source',
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

    const css = readFileSync('gui/dock-layout.css', 'utf8');
    expect(css).toMatch(/\.workshop-dock-root \.workshop-dock-canvas\s*\{[^}]*min-height:\s*32rem/);
    expect(css).toMatch(/\.workshop-dock-root \.workshop-split\s*\{[^}]*min-height:\s*0/);
    expect(css).not.toMatch(/\.workshop-split\s*\{[^}]*min-height:\s*32rem/);
  });

  it('marks document panels so a renderer can arrange a context around them', () => {
    ({ mounted } = mount());
    expect(document.querySelector('[data-panel="source"]').dataset.panelKind).toBe('document');
    expect(document.querySelector('[data-panel="model-preview"]').dataset.panelKind).toBe('document');
    document.querySelector('[data-panel="model-preview"] [data-layout-control="float"]').click();
    expect(document.querySelector('[data-panel="model-preview"].is-floating').dataset.panelKind).toBe('document');
    expect(document.querySelector('[data-panel="files"]').dataset.panelKind).toBe('tool');
    expect(document.querySelector('[data-panel="models"]').dataset.panelKind).toBe('tool');
  });

  it('drops the frame, tab and button of an unavailable panel without changing its placement', () => {
    const hidden = new Set(['findings']);
    ({ mounted } = mount(defaultWorkshopLayout(), { available: panel => !hidden.has(panel) }));
    expect(document.querySelector('[data-panel="findings"]')).toBeNull();
    expect(document.querySelector('[role="tab"][data-layout-panel="findings"]')).toBeNull();
    expect(document.querySelector('[data-layout-panel="findings"][data-layout-control="switcher"]')).toBeNull();
    // Unavailability is read, never written: the arrangement still holds it.
    expect(JSON.stringify(mounted.state())).toContain('"findings"');
    expect(mounted.state().closed).not.toContain('findings');
    // And the node is parked rather than detached, so its owner can find it.
    expect(document.querySelector('.workshop-dock-parked').children.length).toBe(1);
    expect(mounted.reveal('findings')).toBe(false);
    hidden.delete('findings');
    expect(mounted.syncAvailability()).toBe(true);
    expect(document.querySelector('[data-panel="findings"]')).not.toBeNull();
    expect(mounted.syncAvailability()).toBe(false);
  });

  it('moves focus off a tab whose panel its owner just put away', async () => {
    const hidden = new Set();
    ({ mounted } = mount(defaultWorkshopLayout(), { available: panel => !hidden.has(panel) }));
    const tab = document.querySelector('[role="tab"][data-layout-panel="findings"]');
    tab.focus();
    expect(document.activeElement).toBe(tab);
    hidden.add('findings');
    mounted.syncAvailability();
    // The tab is gone; focus must not be left on the body.
    expect(document.querySelector('[role="tab"][data-layout-panel="findings"]')).toBeNull();
    await vi.waitFor(() => expect(document.activeElement).not.toBe(document.body));
  });

  it('keeps every registered panel node in the document when the surface asks it to', () => {
    const initial = closeWorkshopPanel(defaultWorkshopLayout(), 'recovery');
    const { mounted: retained, panels } = mount(initial, { retain: true });
    mounted = retained;
    expect(document.querySelector('[data-panel="recovery"]')).toBeNull();
    expect(panels.recovery.isConnected).toBe(true);
    expect(panels.recovery.closest('.workshop-dock-parked')).not.toBeNull();
    // Reopening moves it out of the parking container into its own frame.
    document.querySelector('[data-layout-panel="recovery"][data-layout-control="switcher"]').click();
    expect(panels.recovery.closest('[data-panel]').dataset.panel).toBe('recovery');
  });

  it('returns focus to the switcher after closing the final panel', () => {
    ({ mounted } = mount());
    for (const panel of WORKSHOP_PANELS) {
      document.querySelector(`[data-panel="${panel}"] [data-layout-control="close"]`).click();
    }
    expect(document.querySelector('.workshop-dock-panel')).toBeNull();
    expect(document.activeElement).toBe(document.querySelector(
      `[data-layout-panel="${WORKSHOP_PANELS.at(-1)}"][data-layout-control="switcher"]`,
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

  it('reveals and focuses a closed panel without mutating retained layout in narrow mode', () => {
    let initial = defaultWorkshopLayout();
    initial = { ...initial, closed: [...initial.closed, 'findings'] };
    initial.root.children[1].tabs = initial.root.children[1].tabs.filter(panel => panel !== 'findings');
    const focus = document.createElement('button');
    focus.className = 'finding-target';
    ({ mounted, changes } = mount(initial));
    document.querySelector('[data-panel="source"]').append(focus);
    expect(mounted.reveal('findings')).toBe(true);
    expect(mounted.state().closed).not.toContain('findings');
    expect(changes).toHaveLength(1);

    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 600 });
    window.dispatchEvent(new Event('resize'));
    const retained = mounted.state();
    const findings = document.querySelector('[data-panel="findings"]');
    const target = document.createElement('button'); target.className = 'finding-target';
    findings.lastElementChild.append(target);
    expect(mounted.reveal('findings', { focus: '.finding-target' })).toBe(true);
    expect(document.activeElement).toBe(target);
    expect(mounted.state()).toEqual(retained);
    expect(changes).toHaveLength(1);
  });
});
