import {
  workshopLayoutModel,
} from './workshop-layout-model.js';

const NARROW_WIDTH = 880;
let nextLayoutInstance = 0;

export function mountDockLayout({ root, surface, panels, labels, initial, onChange, onVisible,
  available = () => true, retain = false, mayReset = () => true, mayClose = () => true,
  onOpen = () => {}, onDiscard = () => {},
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
  /** Picking on a document covers the surface with a gesture, and an in-surface
   * floating panel sits on top of exactly that. Hiding the floats for the
   * duration is a presentation state, not a layout change: nothing is closed,
   * moved or persisted, and turning it off puts back the same frames in the
   * same z-order the stacking rule already derives. */
  let picking = false;
  // The panel the gesture is FOR stays: it names the control capturing the map
  // and previews what committing would place, and a hidden subtree is out of
  // the accessibility tree as well as off the screen. Hiding it would leave the
  // preview with no channel at all.
  let pickingFor = null;
  /** A panel another owner has put away keeps its PLACEMENT and loses its
   * frame, tab and switcher button. Availability is read, never written: one
   * writer owns the panel's own visibility (on the Live desk that is the role
   * preset) and the dock derives from it, so the two can never race. */
  const readAvailable = panel => {
    try { return available(panel) !== false; } catch { return true; }
  };
  /** Availability as ONE render sees it.
   *
   * A render decides what it will draw from availability, then `paint` clears
   * the canvas and asks again while it builds — and clearing the canvas takes
   * every old frame out of the document, panel contents included. An owner that
   * answers by looking its node up in the document (the Live desk's handoff
   * panel resolves `#gm-workshop-source` by id, and that node lives INSIDE the
   * panel) then answers "no" mid-paint: the frame the signature promised is
   * never built, `painted` records that it was, and the panel is parked with
   * a switcher button pointing at nothing. So a render reads availability once,
   * before it touches the tree, and signature, paint and park all see that
   * one answer. Outside a render — `reveal` deciding whether to proceed, a
   * sync asking whether anything changed — the live answer is the right one. */
  let observed = null;
  const usable = panel => observed ? observed.has(panel) : readAvailable(panel);
  const observeAvailability = () => new Set(panelIds.filter(readAvailable));
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

  const settle = (value, bounds) => (model.settle || model.normalize)(value, bounds);
  const emit = (next, focusPanel = next.selected) => {
    state = settle(next, narrow ? undefined : canvasBounds()); projectedPanel = null;
    render(); onChange?.(state);
    const restoreFocus = () => {
      const panelTab = canvas.querySelector(`[role="tab"][data-layout-panel="${focusPanel}"]`)
        || canvas.querySelector(`[data-panel="${focusPanel}"] .workshop-panel-tab`);
      const switcherButton = switcher.querySelector(`[data-layout-panel="${focusPanel}"]`);
      (panelTab || switcherButton)?.focus();
    };
    // The newly rendered control already exists. Restore focus now so a
    // throttled animation frame cannot strand keyboard docking on <body>, then
    // repeat after layout in case the browser's resize observer repaints it.
    restoreFocus();
    win.requestAnimationFrame?.(restoreFocus);
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
        header.append(makeButton(labels.close, () => {
          // Closing a panel that holds unsent work is the operator losing it,
          // so whoever owns that work gets asked first.
          if (mayClose(panel) !== true) return;
          onDiscard(panel);
          emit(model.close(state, panel), panel);
        }, {
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
    // A column may squeeze to nothing — min-width: 0 handles what is inside —
    // but a row is at least its content: a wrapped tab list must not take its
    // rows out of the frame below it, and the canvas scrolls what will not fit.
    const floor = node.axis === 'vertical' ? 'min-content' : '0';
    const tracks = rendered.map(([, size]) => `minmax(${floor}, ${size}fr)`).join(' ');
    split.style.gridTemplateColumns = node.axis === 'horizontal' ? tracks : '';
    split.style.gridTemplateRows = node.axis === 'vertical' ? tracks : '';
    split.append(...rendered.map(([child]) => child)); return split;
  }
  /** What a paint would BUILD, as opposed to what it would merely set.
   *
   * Choosing a tab, revealing a panel and moving a float all leave the frames
   * exactly where they were and change only attributes — and rebuilding anyway
   * would reparent every panel node. That is not free: moving a node between
   * parents re-creates an iframe's document, which on this surface means the
   * authentic Station console reloading, and its pending commands being
   * dropped, every time the operator clicks an unrelated tab. So a render whose
   * structure is unchanged restyles instead of rebuilding. */
  function shapeOf(node) {
    if (!node) return null;
    if (node.type === 'tabs') {
      const shown = availableTabs(node);
      return shown.length ? { tabs: shown } : null;
    }
    const children = node.children.map(shapeOf).filter(Boolean);
    return children.length ? (children.length === 1 ? children[0] : { axis: node.axis, children }) : null;
  }
  const signature = () => JSON.stringify(narrow
    ? { narrow: true, panel: narrowProjection() }
    : { root: shapeOf(state.root), floats: state.floats.map(entry => entry.panel) });
  let painted = null;
  const render = (...args) => {
    observed = observeAvailability();
    try {
      const next = signature();
      if (painted === next) restyle();
      else { paint(...args); painted = next; }
      park();
    } finally { observed = null; }
    reportVisible();
  };
  /** Bring an unchanged arrangement up to date without touching the tree. */
  function restyle() {
    const chosen = narrow ? narrowProjection() : state.selected;
    for (const button of switcher.querySelectorAll('[data-layout-control="switcher"]')) {
      button.setAttribute('aria-pressed', String(button.dataset.layoutPanel === chosen));
    }
    for (const stack of canvas.querySelectorAll('.workshop-tab-stack')) {
      const list = stack.querySelector(':scope > .workshop-tab-list');
      const frames = [...stack.querySelectorAll(':scope > [data-panel]')];
      const tabs = frames.map(node => node.dataset.panel);
      const active = tabs.includes(state.selected) ? state.selected
        : tabs.find(panel => !frames[tabs.indexOf(panel)].hidden) || tabs[0];
      for (const node of frames) node.hidden = node.dataset.panel !== active;
      for (const tab of list?.querySelectorAll('[role="tab"]') || []) {
        tab.setAttribute('aria-selected', String(tab.dataset.layoutPanel === active));
        tab.tabIndex = tab.dataset.layoutPanel === active ? 0 : -1;
      }
    }
    for (const node of canvas.querySelectorAll('[data-panel] > .workshop-panel-header > .workshop-panel-tab')) {
      node.setAttribute('aria-pressed', String(node.closest('[data-panel]').dataset.panel === chosen));
    }
    for (const entry of state.floats) {
      const node = canvas.querySelector(`[data-panel="${entry.panel}"].is-floating`);
      if (!node) continue;
      node.style.left = `${entry.x}px`; node.style.top = `${entry.y}px`;
      node.style.width = `${entry.width}px`; node.style.height = `${entry.height}px`;
      node.hidden = picking && entry.panel !== pickingFor;
    }
    updateFloatStacking();
  }
  function paint() {
    switcher.replaceChildren(...panelIds.filter(usable).map(panel => makeButton(labels.panels[panel], () => {
      if (narrow) {
        const opened = openInState(panel);
        projectedPanel = panel; render();
        if (opened) onChange?.(state);
        switcher.querySelector(`[data-layout-panel="${panel}"]`)?.focus();
      } else if (!state.closed.includes(panel)) {
        emit(model.select(state, panel), panel);
      } else {
        // A temporary panel is a draft the operator is opening, so it opens
        // where a draft belongs: floating over the arrangement, not as a tab in
        // somebody else's group.
        emit(model.isTemporary?.(panel) ? model.float(state, panel, {}, canvasBounds())
          : model.reopen(state, panel), panel);
      }
    }, { 'aria-pressed': String((narrow ? projectedPanel || state.selected : state.selected) === panel), 'data-layout-panel': panel, 'data-layout-control': 'switcher' })),
    makeButton(labels.reset, () => {
      // Resetting the arrangement would take an open draft away with it, so the
      // drafts get their say first.
      if (mayReset() !== true) return;
      onDiscard(null);
      emit(model.defaultLayout());
    }, { class: 'workshop-layout-reset', 'data-layout-control': 'reset' }));
    canvas.replaceChildren(); canvas.classList.toggle('is-narrow', narrow);
    if (narrow) {
      const selected = narrowProjection();
      if (selected) canvas.append(frame(selected, false, true));
      return;
    }
    const tree = state.root && renderNode(state.root);
    if (tree) canvas.append(tree);
    for (const entry of state.floats.filter(entry => usable(entry.panel))) {
      const node = frame(entry.panel, true); node.style.left = `${entry.x}px`; node.style.top = `${entry.y}px`;
      node.style.width = `${entry.width}px`; node.style.height = `${entry.height}px`;
      node.hidden = picking && entry.panel !== pickingFor; canvas.append(node);
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
  /** Bring a closed panel back into the arrangement.
   *
   * The narrow projection shows one panel at a time, but it must still OPEN the
   * one it shows: a panel left `closed` while the operator is filling it in is
   * one nothing else knows is on screen — a draft would be wiped by the next
   * open and taken by the next reset without anybody being asked. */
  function openInState(panel) {
    // Only a DRAFT. The narrow projection is deliberately not a layout change
    // for an ordinary panel — it shows one of the arrangement's panels at a
    // time and leaves the retained desktop tree alone — but a draft has no
    // place in that arrangement to be shown FROM, so opening one is real.
    if (!model.isTemporary?.(panel) || !state.closed.includes(panel)) return false;
    state = settle(model.float(state, panel, {}, canvasBounds()), canvasBounds());
    return true;
  }

  /** An operator standing on a panel its owner just put away lands on one that
   * is still there rather than on an empty projection. */
  function narrowProjection() {
    const preferred = projectedPanel || state.selected;
    return usable(preferred) ? preferred
      : orderedPanels(state.root).find(usable) || state.floats.map(entry => entry.panel).find(usable) || null;
  }
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
    const repaired = nextNarrow ? state : settle(state, canvasBounds());
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
  function reveal(panel, { focus = null, notify = true, reopen = true } = {}) {
    if (!panelIds.includes(panel) || !usable(panel)) return false;
    // Closing a panel is a decision; a caller may bring one FORWARD without
    // undoing it. `reopen: false` says so.
    if (!reopen && state.closed.includes(panel)) return false;
    if (narrow) {
      const opened = reopen && openInState(panel);
      projectedPanel = panel;
      render();
      if (opened && notify) onChange?.(state);
    } else {
      // A temporary panel is a draft: opening one opens it floating, exactly as
      // the switcher does, rather than docking it into somebody else's group.
      const opened = !state.closed.includes(panel) ? model.select(state, panel)
        : model.isTemporary?.(panel) ? model.float(state, panel, {}, canvasBounds())
          : model.reopen(state, panel);
      state = settle(opened, canvasBounds());
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
   * Nothing in the arrangement changes, so this neither persists nor notifies.
   *
   * What is already DRAWN is the comparison, not a memo of its own. A record
   * of "the availability I last synced" drifts, because `paint` reads
   * availability too: an ordinary arrangement change landing in a window where
   * a panel's owner had it hidden repaints without that panel and leaves the
   * memo still claiming it is shown. The panel then never returns — the next
   * sync compares reality against the memo, finds them equal, and returns
   * early while the surface holds no frame for it. `signature()` is what
   * `render` paints FROM and `painted` is what it last painted, so comparing
   * those two cannot disagree with what the operator can see.
   */
  function syncAvailability() {
    if (signature() === painted) return false;
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
  /** Hide every in-surface floating panel while a gesture owns the surface, and
   * put them all back when it ends. Docked panels are untouched. */
  function setPicking(value, forPanel = null) {
    const next = value === true;
    if (picking === next && pickingFor === (next ? forPanel : null)) return false;
    picking = next;
    pickingFor = next ? forPanel : null;
    canvas.classList.toggle('is-picking', picking);
    for (const node of canvas.querySelectorAll('.workshop-dock-panel.is-floating')) {
      node.hidden = picking && node.dataset.panel !== pickingFor;
    }
    // Opening or resetting a panel mid-gesture would put a frame on screen the
    // operator cannot see, and persist it. The arrangement is not up for
    // rearranging while a gesture owns the surface.
    for (const control of switcher.querySelectorAll('button')) control.disabled = picking;
    updateFloatStacking();
    reportVisible();
    return true;
  }
  return { model, setPicking, isPicking: () => picking,
    state: () => cloneState(state),
    // `set` takes an arrangement from a CALLER — a stored profile, a reset —
    // so it is guarded like any other stored state rather than merely settled.
    set: next => emit(model.normalize(next, narrow ? undefined : canvasBounds())),
    reset: () => { if (mayReset() !== true) return; onDiscard(null); emit(model.defaultLayout()); },
    reveal, syncAvailability, dispose() {
      painted = null;
    observer?.disconnect(); win.removeEventListener?.('resize', resize); doc.removeEventListener('keydown', keydown);
    parked.remove();
    surface.classList.remove('workshop-dock-root');
  } };
}

const cloneState = state => JSON.parse(JSON.stringify(state));

export const mountWorkshopLayout = options => mountDockLayout(options);
