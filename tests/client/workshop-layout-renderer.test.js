// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  closeWorkshopPanel, defaultWorkshopLayout, dockWorkshopPanel, selectWorkshopPanel, WORKSHOP_PANELS,
} from '../../gui/workshop-layout-model.js';
import { mountWorkshopLayout } from '../../gui/workshop-layout-renderer.js';
import { createDockLayoutModel } from '../../gui/dock-layout-model.js';

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
  let mounted, changes, panels;
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
    // A tab strip is a set of destinations, and one group can hold six of them
    // in a quarter-width column (the Live inspector's, since issue #1512). A
    // strip that overflowed sideways would put a panel behind a horizontal
    // scroll nobody looks for, so it wraps and the panel body takes the room.
    expect(css).toMatch(/\.workshop-tab-list\s*\{[^}]*flex-wrap:\s*wrap/);
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

  it('brings back a panel an ordinary repaint dropped while its owner had it away', () => {
    // The drift this pins: `paint` reads availability too, so an arrangement
    // change landing WHILE a panel is unavailable repaints without it. Nothing
    // told `syncAvailability` that happened, so a sync that compared against its
    // own memo of the last sync found "no change" and left the operator with a
    // switcher button and no panel — issue #1505's handoff at session start.
    const hidden = new Set();
    ({ mounted } = mount(defaultWorkshopLayout(), { available: panel => !hidden.has(panel) }));
    expect(document.querySelector('[data-panel="findings"]')).not.toBeNull();

    // Its owner puts it away, and an UNRELATED arrangement change repaints
    // before any sync runs.
    hidden.add('findings');
    mounted.set(closeWorkshopPanel(mounted.state(), 'recovery'));
    expect(document.querySelector('[data-panel="findings"]')).toBeNull();

    // Its owner brings it back. The sync must repaint, because what is drawn
    // and what is available no longer agree.
    hidden.delete('findings');
    expect(mounted.syncAvailability()).toBe(true);
    expect(document.querySelector('[data-panel="findings"]')).not.toBeNull();
    // And it is still reachable, not merely present.
    expect(document.querySelector('[data-layout-panel="findings"][data-layout-control="switcher"]'))
      .not.toBeNull();
    expect(mounted.syncAvailability()).toBe(false);
  });

  it('draws what its signature promised even when an owner answers by looking its node up', () => {
    // The Live desk's handoff panel answers availability by resolving
    // `#gm-workshop-source` by id — and that node lives INSIDE the panel. A
    // paint clears the canvas before it builds, which takes every old frame
    // out of the document, so an owner that answers that way says "no"
    // mid-paint: the frame the signature promised is never built and the
    // panel is parked behind a switcher button pointing at nothing.
    const marker = document.createElement('span'); marker.id = 'findings-marker';
    let panels;
    ({ mounted, panels } = mount(defaultWorkshopLayout(), {
      available: panel => panel !== 'findings' || !!document.getElementById('findings-marker') }));
    expect(document.querySelector('[data-panel="findings"]')).toBeNull();
    panels.findings.append(marker);
    expect(mounted.syncAvailability()).toBe(true);
    expect(document.querySelector('[data-panel="findings"]')).not.toBeNull();

    // Any ordinary repaint.
    mounted.set(closeWorkshopPanel(mounted.state(), 'recovery'));
    expect(document.querySelector('[data-panel="findings"]')).not.toBeNull();
    expect(document.querySelector('.workshop-dock-parked').contains(panels.findings)).toBe(false);
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

  it('does not rebuild the tree when only the shown tab changes', () => {
    // Reparenting a panel node re-creates an iframe's document, so a render
    // whose structure is unchanged must restyle rather than rebuild.
    ({ mounted, panels } = mount());
    const sourceFrame = document.querySelector('[data-panel="source"]');
    const findingsFrame = document.querySelector('[data-panel="findings"]');
    expect(findingsFrame.hidden).toBe(true);
    document.querySelector('[role="tab"][data-layout-panel="findings"]').click();
    expect(document.querySelector('[data-panel="findings"]')).toBe(findingsFrame);
    expect(document.querySelector('[data-panel="source"]')).toBe(sourceFrame);
    expect(panels.findings.parentElement).toBe(findingsFrame);
    expect(findingsFrame.hidden).toBe(false);
    expect(sourceFrame.hidden).toBe(true);
    expect(document.querySelector('[role="tab"][data-layout-panel="findings"]').getAttribute('aria-selected'))
      .toBe('true');
    expect(document.querySelector('[role="tab"][data-layout-panel="source"]').tabIndex).toBe(-1);
    // Revealing one is the same: it changes which tab is shown, nothing else.
    mounted.reveal('source');
    expect(document.querySelector('[data-panel="source"]')).toBe(sourceFrame);
    expect(sourceFrame.hidden).toBe(false);
    // A move IS structural, so that one rebuilds.
    document.querySelector('[data-panel="findings"] [data-layout-control="float"]').click();
    expect(document.querySelector('[data-panel="findings"]')).not.toBe(findingsFrame);
    expect(panels.findings.closest('[data-panel]').classList.contains('is-floating')).toBe(true);
  });

  it('brings a panel forward without undoing a close when asked not to reopen', () => {
    ({ mounted } = mount());
    document.querySelector('[data-panel="findings"] [data-layout-control="close"]').click();
    expect(mounted.state().closed).toContain('findings');
    expect(mounted.reveal('findings', { reopen: false })).toBe(false);
    expect(mounted.state().closed).toContain('findings');
    expect(mounted.reveal('findings')).toBe(true);
    expect(mounted.state().closed).not.toContain('findings');
  });

  it('opens a temporary panel as a floating draft, from the switcher or a reveal', () => {
    // A draft belongs over the arrangement, not as a tab in somebody else's
    // group — and it is not restored, so opening one must not be undone by the
    // very rule that refuses to restore it.
    const model = createDockLayoutModel({
      version: 1, panels: ['files', 'draft'], temporary: ['draft'],
      defaultLayout: () => ({ version: 1, root: { type: 'tabs', tabs: ['files'], active: 'files' },
        floats: [], closed: ['draft'], selected: 'files' }),
    });
    document.body.innerHTML = '<main id="root"><div id="surface"></div></main>';
    const panels = Object.fromEntries(['files', 'draft'].map(panel => {
      const node = document.createElement('div'); node.id = `panel-${panel}`; return [panel, node];
    }));
    mounted = mountWorkshopLayout({
      root: document.getElementById('root'), surface: document.getElementById('surface'),
      panels, model,
      labels: { ...labels, panels: { files: 'files', draft: 'draft' } },
      initial: model.defaultLayout(),
    });
    expect(document.querySelector('[data-panel="draft"]')).toBeNull();
    document.querySelector('[data-layout-panel="draft"][data-layout-control="switcher"]').click();
    expect(document.querySelector('[data-panel="draft"]').classList.contains('is-floating')).toBe(true);
    expect(mounted.state().floats.map(entry => entry.panel)).toEqual(['draft']);

    document.querySelector('[data-panel="draft"] [data-layout-control="close"]').click();
    expect(mounted.state().closed).toContain('draft');
    expect(mounted.reveal('draft')).toBe(true);
    expect(document.querySelector('[data-panel="draft"]').classList.contains('is-floating')).toBe(true);
    // Restoring it is what discards it, which is the whole point of temporary.
    expect(model.normalize(mounted.state()).closed).toContain('draft');
  });

  it('puts the floating panels away while a gesture owns the surface', () => {
    ({ mounted } = mount());
    document.querySelector('[data-panel="findings"] [data-layout-control="float"]').click();
    document.querySelector('[data-panel="feedback"] [data-layout-control="float"]').click();
    const floats = () => [...document.querySelectorAll('.workshop-dock-panel.is-floating')];
    expect(floats()).toHaveLength(2);
    const before = floats().map(node => node.dataset.panel);
    const stacking = floats().map(node => node.style.zIndex);

    // The panel the gesture is FOR stays: it names the capturing control and
    // previews what committing would place, and a hidden subtree is out of the
    // accessibility tree as well as off the screen.
    expect(mounted.setPicking(true, 'feedback')).toBe(true);
    expect(mounted.isPicking()).toBe(true);
    expect(document.querySelector('[data-panel="feedback"].is-floating').hidden).toBe(false);
    expect(document.querySelector('[data-panel="findings"].is-floating').hidden).toBe(true);
    // The arrangement is not up for rearranging while a gesture owns it: a
    // panel opened now would be framed where the operator cannot see it.
    expect([...document.querySelectorAll('.workshop-panel-switcher button')]
      .every(button => button.disabled)).toBe(true);
    // A docked panel is untouched, and nothing was closed, moved or persisted.
    expect(document.querySelector('[data-panel="source"]').hidden).toBe(false);
    expect(mounted.state().floats.map(entry => entry.panel)).toEqual(before);

    expect(mounted.setPicking(false)).toBe(true);
    expect(floats().every(node => node.hidden)).toBe(false);
    expect(floats().map(node => node.dataset.panel)).toEqual(before);
    expect(floats().map(node => node.style.zIndex)).toEqual(stacking);
    expect([...document.querySelectorAll('.workshop-panel-switcher button')]
      .some(button => button.disabled)).toBe(false);
    expect(mounted.setPicking(false)).toBe(false);
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
