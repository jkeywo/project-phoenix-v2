import {
  workshopLayoutModel,
} from './workshop-layout-model.js';

const NARROW_WIDTH = 880;
let nextLayoutInstance = 0;

export function mountDockLayout({ root, surface, panels, labels, initial, onChange, onVisible,
  available = () => true, retain = false,
  model = workshopLayoutModel, viewportNarrow = false, doc = root.ownerDocument, win = doc.defaultView }) {
  surface.classList.add('workshop-dock-root');
  const panelIds = model.panels;
  const layoutId = `workshop-layout-${nextLayoutInstance += 1}`;
  const switcher = doc.createElement('div'); switcher.className = 'workshop-panel-switcher';
  switcher.setAttribute('role', 'toolbar'); switcher.setAttribute('aria-label', labels.switcher);
  const canvas = doc.createElement('div'); canvas.className = 'workshop-dock-canvas';
  // A panel with no frame this paint still has to stay in the document when its
  // content is owned elsewhere: that owner resolves it by id — sometimes only
  // after the dock has mounted — and for a panel its own owner put away it is
  // the only thing that can bring it back. Parking keeps `getElementById`
  // working without giving the panel a frame, a tab or a switcher button.
  // `retain` extends that to closed and narrow-projected-away panels; a surface
  // whose panels are held by reference (Workshop) leaves it off and a closed
  // panel is detached as usual.
  const parked = doc.createElement('div'); parked.className = 'workshop-dock-parked'; parked.hidden = true;
  surface.replaceChildren(switcher, canvas, parked);
  const canvasBounds = () => ({
    width: canvas.clientWidth || surface.clientWidth || win.innerWidth,
    height: canvas.clientHeight || surface.clientHeight || win.innerHeight,
  });
  const narrowWidth = () => viewportNarrow ? win.innerWidth : (surface.clientWidth || win.innerWidth);
  let narrow = narrowWidth() <= NARROW_WIDTH;
  let state = model.normalize(initial, narrow ? undefined : canvasBounds());
  let projectedPanel = null;
  let drag = null;
  /** A panel another owner has put away keeps its PLACEMENT and loses its
   * frame, tab and switcher button. Availability is read, never written: one
   * writer owns the panel's own visibility (on the Live desk that is the role
   * preset) and the dock derives from it, so the two can never race. */
  const usable = panel => {
    try { return available(panel) !== false; } catch { return true; }
  };
  const availableTabs = node => node.tabs.filter(usable);
  const activeTab = node => {
    const tabs = availableTabs(node);
    return tabs.includes(node.active) ? node.active : tabs[0] || null;
  };

  const orderedPanels = node => {
    if (!node) return [];
    if (node.type === 'tabs') return [...node.tabs];
    return node.children.flatMap(orderedPanels);
  };

  const emit = (next, focusPanel = next.selected) => {
    state = model.normalize(next, narrow ? undefined : canvasBounds()); projectedPanel = null;
    render(); onChange?.(state);
    win.requestAnimationFrame?.(() => {
      const panelTab = canvas.querySelector(`[role="tab"][data-layout-panel="${focusPanel}"]`)
        || canvas.querySelector(`[data-panel="${focusPanel}"] .workshop-panel-tab`);
      const switcherButton = switcher.querySelector(`[data-layout-panel="${focusPanel}"]`);
      (panelTab || switcherButton)?.focus();
    });
  };
  const updateFloatStacking = () => {
    const active = doc.activeElement;
    state.floats.forEach((entry, index) => {
      const node = canvas.querySelector(`[data-panel="${entry.panel}"].is-floating`);
      if (node) node.style.zIndex = String(10 + index
        + (entry.panel === state.selected ? state.floats.length : 0)
        + (node.contains(active) ? state.floats.length * 2 : 0));
    });
  };
  const makeButton = (text, action, attrs = {}) => {
    const button = doc.createElement('button'); button.type = 'button'; button.textContent = text;
    for (const [key, value] of Object.entries(attrs)) button.setAttribute(key, value);
    button.addEventListener('click', action); return button;
  };
  const pointerTarget = event => doc.elementFromPoint?.(event.clientX, event.clientY)
    ?.closest?.('.workshop-dock-target');
  const showPointerTarget = event => {
    const target = pointerTarget(event);
    canvas.querySelectorAll('.workshop-dock-target.is-pointer-target')
      .forEach(node => node.classList.remove('is-pointer-target'));
    target?.classList.add('is-pointer-target');
    return target;
  };
  const beginPointer = (event, panel, floating, node) => {
    if (event.pointerType === 'mouse' && event.button !== 0) return;
    drag = { panel, x: event.clientX, y: event.clientY, floating, moved: false, node };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    canvas.classList.add('is-dragging');
  };
  const movePointer = event => {
    if (!drag) return;
    showPointerTarget(event);
    if (!drag.floating) return;
    const entry = state.floats.find(value => value.panel === drag.panel);
    if (!entry) return;
    const x = entry.x + event.clientX - drag.x, y = entry.y + event.clientY - drag.y;
    drag.x = event.clientX; drag.y = event.clientY; drag.moved = true;
    state = model.moveFloat(state, drag.panel, x, y, canvasBounds());
    const moved = state.floats.find(value => value.panel === drag.panel);
    drag.node.style.left = `${moved.x}px`; drag.node.style.top = `${moved.y}px`;
  };
  const endPointer = (event, cancelled = false) => {
    if (!drag) return;
    const gesture = drag;
    const target = cancelled ? null : pointerTarget(event);
    drag = null; canvas.classList.remove('is-dragging');
    canvas.querySelectorAll('.workshop-dock-target.is-pointer-target')
      .forEach(node => node.classList.remove('is-pointer-target'));
    const targetPanel = target?.closest('[data-panel]')?.dataset.panel;
    const placement = target?.dataset.placement;
    if (targetPanel && placement && targetPanel !== gesture.panel) {
      emit(model.dock(state, gesture.panel, targetPanel, placement), gesture.panel);
    } else if (gesture.moved) {
      onChange?.(state);
    }
  };
  const attachPointerDocking = (tab, panel, floating = false, node = null) => {
    tab.addEventListener('pointerdown', event => beginPointer(event, panel, floating, node));
    tab.addEventListener('pointermove', movePointer);
    tab.addEventListener('pointerup', endPointer);
    tab.addEventListener('pointercancel', event => endPointer(event, true));
  };
  function frame(panel, floating = false, projection = false, tabId = null) {
    const node = doc.createElement('section'); node.className = `workshop-dock-panel${floating ? ' is-floating' : ''}`;
    node.dataset.panel = panel;
    // Document panels are the surfaces a context is arranged around; tools sit beside them.
    const kind = model.kind?.(panel);
    if (kind) node.dataset.panelKind = kind;
    if (tabId) {
      node.id = `${layoutId}-panel-${panel}`;
      node.setAttribute('role', 'tabpanel');
      node.setAttribute('aria-labelledby', tabId);
    }
    const header = doc.createElement('header'); header.className = 'workshop-panel-header';
    const tab = makeButton(labels.panels[panel], () => {
      if (!projection) emit(model.select(state, panel), panel);
    }, {
      class: 'workshop-panel-tab', 'aria-pressed': String((projection ? projectedPanel : state.selected) === panel),
      'data-layout-control': 'tab',
    });
    if (!projection) attachPointerDocking(tab, panel, floating, node);
    header.append(tab);
    if (!projection) {
      header.append(makeButton(labels.float, () => emit(model.float(state, panel, {}, canvasBounds()), panel), {
        'aria-label': `${labels.float}: ${labels.panels[panel]}`, 'data-layout-control': 'float',
      }));
      // A pinned panel offers no way to close it, because there is none.
      if (!model.isPinned?.(panel)) {
        header.append(makeButton(labels.close, () => emit(model.close(state, panel), panel), {
          'aria-label': `${labels.close}: ${labels.panels[panel]}`, 'data-layout-control': 'close',
        }));
      }
    }
    const targets = doc.createElement('div'); targets.className = 'workshop-dock-targets';
    if (!projection) {
      for (const [placement, symbol] of [['left', '<'], ['top', '^'], ['tab', '+'], ['bottom', 'v'], ['right', '>']]) {
        const target = makeButton(symbol, event => event.preventDefault(), {
          class: `workshop-dock-target is-${placement}`, 'data-placement': placement,
          'aria-label': `${labels.dock[placement]}: ${labels.panels[panel]}`,
        });
        targets.append(target);
      }
    }
    node.append(header, targets, panels[panel]);
    if (floating) {
      node.addEventListener('focusin', updateFloatStacking);
      node.addEventListener('focusout', () => win.requestAnimationFrame?.(updateFloatStacking));
    }
    return node;
  }
  function renderNode(node) {
    if (node.type === 'tabs') {
      const shown = availableTabs(node);
      if (!shown.length) return null;
      const active = activeTab(node);
      const stack = doc.createElement('div'); stack.className = 'workshop-tab-stack';
      if (shown.length > 1) {
        const tabs = doc.createElement('div'); tabs.className = 'workshop-tab-list'; tabs.setAttribute('role', 'tablist');
        if (labels.tabs) tabs.setAttribute('aria-label', labels.tabs);
        for (const panel of shown) {
          const tabId = `${layoutId}-tab-${panel}`;
          const tab = makeButton(labels.panels[panel], () => emit(model.select(state, panel), panel), {
            id: tabId,
            role: 'tab', 'aria-selected': String(active === panel),
            'aria-controls': `${layoutId}-panel-${panel}`,
            tabindex: active === panel ? '0' : '-1',
            'data-layout-panel': panel, 'data-layout-control': 'stack-tab',
          });
          tab.addEventListener('keydown', event => {
            if (event.ctrlKey || event.shiftKey || event.altKey || event.metaKey) return;
            const current = shown.indexOf(panel);
            const index = event.key === 'Home' ? 0 : event.key === 'End' ? shown.length - 1
              : event.key === 'ArrowLeft' ? (current - 1 + shown.length) % shown.length
                : event.key === 'ArrowRight' ? (current + 1) % shown.length : -1;
            if (index < 0) return;
            event.preventDefault(); emit(model.select(state, shown[index]), shown[index]);
          });
          attachPointerDocking(tab, panel);
          tabs.append(tab);
        }
        stack.append(tabs);
      }
      for (const panel of shown) {
        const tabId = shown.length > 1 ? `${layoutId}-tab-${panel}` : null;
        const child = frame(panel, false, false, tabId);
        child.hidden = panel !== active; stack.append(child);
      }
      return stack;
    }
    const rendered = node.children.map((child, index) => [renderNode(child), node.sizes[index]])
      .filter(([child]) => child);
    if (!rendered.length) return null;
    if (rendered.length === 1) return rendered[0][0];
    const split = doc.createElement('div'); split.className = `workshop-split is-${node.axis}`;
    split.style.setProperty('--workshop-sizes', rendered.map(([, size]) => size).join('fr '));
    const tracks = rendered.map(([, size]) => `minmax(0, ${size}fr)`).join(' ');
    split.style.gridTemplateColumns = node.axis === 'horizontal' ? tracks : '';
    split.style.gridTemplateRows = node.axis === 'vertical' ? tracks : '';
    split.append(...rendered.map(([child]) => child)); return split;
  }
  const render = (...args) => { paint(...args); park(); reportVisible(); };
  function paint() {
    switcher.replaceChildren(...panelIds.filter(usable).map(panel => makeButton(labels.panels[panel], () => {
      if (narrow) {
        projectedPanel = panel; render();
        switcher.querySelector(`[data-layout-panel="${panel}"]`)?.focus();
      } else {
        const next = state.closed.includes(panel) ? model.reopen(state, panel) : model.select(state, panel);
        emit(next, panel);
      }
    }, { 'aria-pressed': String((narrow ? projectedPanel || state.selected : state.selected) === panel), 'data-layout-panel': panel, 'data-layout-control': 'switcher' })),
    makeButton(labels.reset, () => emit(model.defaultLayout()), { class: 'workshop-layout-reset', 'data-layout-control': 'reset' }));
    canvas.replaceChildren(); canvas.classList.toggle('is-narrow', narrow);
    if (narrow) {
      const preferred = projectedPanel || state.selected;
      // An operator standing on a panel its owner just put away lands on one
      // that is still there rather than on an empty projection.
      const selected = usable(preferred) ? preferred
        : orderedPanels(state.root).find(usable) || state.floats.map(entry => entry.panel).find(usable);
      if (selected) canvas.append(frame(selected, false, true));
      return;
    }
    const tree = state.root && renderNode(state.root);
    if (tree) canvas.append(tree);
    for (const entry of state.floats.filter(entry => usable(entry.panel))) {
      const node = frame(entry.panel, true); node.style.left = `${entry.x}px`; node.style.top = `${entry.y}px`;
      node.style.width = `${entry.width}px`; node.style.height = `${entry.height}px`; canvas.append(node);
    }
    updateFloatStacking();
  }
  function park() {
    const homeless = panelIds.filter(panel => panels[panel]
      && (!usable(panel) || (retain && !canvas.querySelector(`[data-panel="${panel}"]`))));
    parked.replaceChildren(...homeless.map(panel => panels[panel]));
  }
  /** Panels with a frame the operator can actually see. A closed panel, an
   * inactive tab and a panel the narrow projection left out are all absent, so a
   * panel holding an expensive live resource can release it. */
  function reportVisible() {
    onVisible?.(new Set([...canvas.querySelectorAll('[data-panel]')]
      .filter(node => !node.hidden).map(node => node.dataset.panel)));
  }
  function keydown(event) {
    if (event.defaultPrevented || event.isComposing || !(event.ctrlKey && event.shiftKey)) return;
    if (!event.target?.closest?.('.workshop-panel-header, .workshop-panel-switcher, .workshop-tab-list')) return;
    const panelNode = event.target?.closest?.('[data-panel], [data-layout-panel]');
    const panel = panelNode?.dataset.panel || panelNode?.dataset.layoutPanel;
    const direction = ['ArrowLeft', 'ArrowUp'].includes(event.code) ? -1
      : ['ArrowRight', 'ArrowDown'].includes(event.code) ? 1 : 0;
    const order = [...orderedPanels(state.root), ...state.floats.map(entry => entry.panel)].filter(usable);
    const index = order.indexOf(panel);
    if (!direction || index < 0 || order.length < 2 || narrow) return;
    const target = order[(index + direction + order.length) % order.length];
    const placement = event.altKey ? 'tab' : event.code === 'ArrowLeft' ? 'left'
      : event.code === 'ArrowRight' ? 'right' : event.code === 'ArrowUp' ? 'top' : 'bottom';
    event.preventDefault(); emit(model.dock(state, panel, target, placement), panel);
  }
  function resize() {
    const nextNarrow = narrowWidth() <= NARROW_WIDTH;
    const active = doc.activeElement;
    const focus = active && (canvas.contains(active) || switcher.contains(active)) ? {
      element: active,
      panel: active.closest?.('[data-panel]')?.dataset.panel || active.dataset?.layoutPanel,
      control: active.dataset?.layoutControl,
    } : null;
    const repaired = nextNarrow ? state : model.normalize(state, canvasBounds());
    const changed = JSON.stringify(repaired) !== JSON.stringify(state);
    if (nextNarrow === narrow && !changed) return;
    narrow = nextNarrow;
    state = repaired;
    projectedPanel = narrow && focus?.panel ? focus.panel : null;
    render();
    const replacement = focus?.element?.isConnected ? focus.element
      : focus?.panel && focus?.control
        ? surface.querySelector(`[data-layout-panel="${focus.panel}"][data-layout-control="${focus.control}"], [data-panel="${focus.panel}"] [data-layout-control="${focus.control}"]`)
        : null;
    (replacement || (focus?.panel ? switcher.querySelector(`[data-layout-panel="${focus.panel}"]`) : null))?.focus();
    if (changed) onChange?.(state);
  }
  const observer = typeof win.ResizeObserver === 'function' ? new win.ResizeObserver(resize) : null;
  observer?.observe(surface); win.addEventListener?.('resize', resize); doc.addEventListener('keydown', keydown);
  render();
  function reveal(panel, { focus = null, notify = true } = {}) {
    if (!panelIds.includes(panel) || !usable(panel)) return false;
    if (narrow) {
      projectedPanel = panel;
      render();
    } else {
      state = model.normalize(state.closed.includes(panel) ? model.reopen(state, panel) : model.select(state, panel), canvasBounds());
      projectedPanel = null;
      render();
      if (notify) onChange?.(state);
    }
    win.requestAnimationFrame?.(() => {
      const panelNode = canvas.querySelector(`[data-panel="${panel}"]`);
      (focus ? panelNode?.querySelector(focus) : panelNode?.querySelector('.workshop-panel-tab')
        || switcher.querySelector(`[data-layout-panel="${panel}"]`))?.focus?.();
    });
    return true;
  }
  /** Re-read availability after its owner changed a panel's own visibility.
   * Nothing in the arrangement changes, so this neither persists nor notifies. */
  let lastAvailable = panelIds.filter(usable).join('\u0000');
  function syncAvailability() {
    const current = panelIds.filter(usable).join('\u0000');
    if (current === lastAvailable) return false;
    lastAvailable = current;
    const active = doc.activeElement;
    // A tab button and a switcher button name their panel directly and live
    // OUTSIDE the frame, so the frame lookup alone would miss the commonest
    // case: the operator standing on the tab of the panel that just went away.
    const focused = active?.dataset?.layoutPanel
      || active?.closest?.('[data-panel]')?.dataset.panel || null;
    const stranded = !!focused && (canvas.contains(active) || switcher.contains(active))
      && !usable(focused);
    render();
    if (stranded) win.requestAnimationFrame?.(() => {
      (canvas.querySelector('[role="tab"]') || canvas.querySelector('.workshop-panel-tab')
        || switcher.querySelector('[data-layout-panel]'))?.focus?.();
    });
    return true;
  }
  return { state: () => cloneState(state), set: next => emit(next), reset: () => emit(model.defaultLayout()),
    reveal, syncAvailability, dispose() {
    observer?.disconnect(); win.removeEventListener?.('resize', resize); doc.removeEventListener('keydown', keydown);
    parked.remove();
    surface.classList.remove('workshop-dock-root');
  } };
}

const cloneState = state => JSON.parse(JSON.stringify(state));

export const mountWorkshopLayout = options => mountDockLayout(options);
