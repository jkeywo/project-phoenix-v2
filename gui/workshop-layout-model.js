export const WORKSHOP_LAYOUT_VERSION = 1;
export const WORKSHOP_PANELS = Object.freeze(['files', 'source', 'inspector']);

const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });

export function defaultWorkshopLayout() {
  return {
    version: WORKSHOP_LAYOUT_VERSION,
    root: {
      type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
      children: [group(['files']), group(['source']), group(['inspector'])],
    },
    floats: [], closed: [], selected: 'source',
  };
}

const clone = value => JSON.parse(JSON.stringify(value));
const isPanel = value => WORKSHOP_PANELS.includes(value);
const MAX_LAYOUT_DEPTH = WORKSHOP_PANELS.length;
const INVALID_NODE = Symbol('invalid workshop layout node');

function normalizeGroup(value, seen) {
  if (!value || value.type !== 'tabs' || !Array.isArray(value.tabs)) return null;
  const tabs = value.tabs.filter(panel => isPanel(panel) && !seen.has(panel) && seen.add(panel));
  if (!tabs.length) return null;
  return group(tabs, tabs.includes(value.active) ? value.active : tabs[0]);
}

function normalizeNode(value, seen, depth = 0) {
  if (depth > MAX_LAYOUT_DEPTH) return INVALID_NODE;
  const direct = normalizeGroup(value, seen);
  if (direct) return direct;
  if (!value || value.type !== 'split' || !['horizontal', 'vertical'].includes(value.axis)
      || !Array.isArray(value.children)) return null;
  const children = [];
  for (const child of value.children.slice(0, WORKSHOP_PANELS.length)) {
    const normalized = normalizeNode(child, seen, depth + 1);
    if (normalized === INVALID_NODE) return INVALID_NODE;
    if (normalized) children.push(normalized);
  }
  if (!children.length) return null;
  if (children.length === 1) return children[0];
  const supplied = Array.isArray(value.sizes) ? value.sizes : [];
  const sizes = children.map((_, index) => Number.isFinite(supplied[index]) && supplied[index] > 0
    ? supplied[index] : 1);
  return { type: 'split', axis: value.axis, sizes, children };
}

function clampFloat(entry, bounds) {
  const number = (candidate, fallback) => Number.isFinite(candidate) ? candidate : fallback;
  const boundedWidth = Number.isFinite(bounds?.width) && bounds.width > 0;
  const boundedHeight = Number.isFinite(bounds?.height) && bounds.height > 0;
  const width = boundedWidth
    ? Math.min(Math.max(240, number(entry.width, 420)), bounds.width)
    : Math.max(240, number(entry.width, 420));
  const height = boundedHeight
    ? Math.min(Math.max(180, number(entry.height, 360)), bounds.height)
    : Math.max(180, number(entry.height, 360));
  return {
    panel: entry.panel,
    x: Math.min(Math.max(0, number(entry.x, 12)), boundedWidth ? bounds.width - width : Infinity),
    y: Math.min(Math.max(0, number(entry.y, 12)), boundedHeight ? bounds.height - height : Infinity),
    width,
    height,
  };
}

export function normalizeWorkshopLayout(value, bounds) {
  if (!value || value.version !== WORKSHOP_LAYOUT_VERSION) return defaultWorkshopLayout();
  const seen = new Set();
  const root = normalizeNode(value.root, seen);
  if (root === INVALID_NODE) return defaultWorkshopLayout();
  const floats = Array.isArray(value.floats) ? value.floats
    .map(entry => {
      if (!entry || !isPanel(entry.panel) || seen.has(entry.panel)) return null;
      seen.add(entry.panel);
      return clampFloat(entry, bounds);
    }).filter(Boolean).slice(0, WORKSHOP_PANELS.length) : [];
  if (value.root !== null && !root && !floats.length) return defaultWorkshopLayout();
  const closed = Array.isArray(value.closed)
    ? value.closed.filter(panel => isPanel(panel) && !seen.has(panel) && seen.add(panel)) : [];
  for (const panel of WORKSHOP_PANELS) if (!seen.has(panel)) closed.push(panel);
  const selected = isPanel(value.selected) && !closed.includes(value.selected)
    ? value.selected : firstVisible(root, floats) || WORKSHOP_PANELS[0];
  return { version: WORKSHOP_LAYOUT_VERSION, root, floats, closed, selected };
}

function firstVisible(node, floats = []) {
  if (node?.type === 'tabs') return node.active;
  if (node?.type === 'split') return node.children.map(child => firstVisible(child)).find(Boolean);
  return floats[0]?.panel || null;
}

function removeFromNode(node, panel) {
  if (!node) return null;
  if (node.type === 'tabs') {
    const tabs = node.tabs.filter(id => id !== panel);
    return tabs.length ? group(tabs, tabs.includes(node.active) ? node.active : tabs[0]) : null;
  }
  const survivors = node.children.map((child, index) => ({
    child: removeFromNode(child, panel),
    size: node.sizes[index],
  })).filter(entry => entry.child);
  const children = survivors.map(entry => entry.child);
  if (!children.length) return null;
  if (children.length === 1) return children[0];
  const previousTotal = node.sizes.reduce((sum, size) => sum + size, 0);
  const survivingTotal = survivors.reduce((sum, entry) => sum + entry.size, 0);
  const sizes = survivors.map(entry => entry.size * previousTotal / survivingTotal);
  return { ...node, sizes, children };
}

function updateNode(node, target, update) {
  if (!node) return { node, found: false };
  if (node.type === 'tabs' && node.tabs.includes(target)) return { node: update(node), found: true };
  if (node.type !== 'split') return { node, found: false };
  for (let index = 0; index < node.children.length; index += 1) {
    const result = updateNode(node.children[index], target, update);
    if (result.found) {
      const children = [...node.children]; children[index] = result.node;
      return { node: { ...node, children }, found: true };
    }
  }
  return { node, found: false };
}

function detached(state, panel) {
  const next = clone(state);
  next.root = removeFromNode(next.root, panel);
  next.floats = next.floats.filter(entry => entry.panel !== panel);
  next.closed = next.closed.filter(id => id !== panel);
  return next;
}

export function selectWorkshopPanel(state, panel) {
  if (!isPanel(panel) || state.closed.includes(panel)) return state;
  const next = clone(state); next.selected = panel;
  const result = updateNode(next.root, panel, node => ({ ...node, active: panel }));
  next.root = result.node;
  return next;
}

export function dockWorkshopPanel(state, panel, target, placement = 'tab') {
  if (!isPanel(panel) || !isPanel(target) || panel === target) return state;
  let next = detached(state, panel);
  const axis = ['left', 'right'].includes(placement) ? 'horizontal' : 'vertical';
  const before = ['left', 'top'].includes(placement);
  const floatingTarget = next.floats.find(entry => entry.panel === target);
  if (floatingTarget) {
    next.floats = next.floats.filter(entry => entry.panel !== target);
    const joined = placement === 'tab' ? group([target, panel], panel)
      : { type: 'split', axis, sizes: [1, 1], children: before ? [group([panel]), group([target])] : [group([target]), group([panel])] };
    next.root = next.root ? { type: 'split', axis: 'horizontal', sizes: [3, 1], children: [next.root, joined] } : joined;
    next.selected = panel;
    return normalizeWorkshopLayout(next);
  }
  const result = updateNode(next.root, target, node => placement === 'tab'
    ? group([...node.tabs, panel], panel)
    : { type: 'split', axis, sizes: [1, 1], children: before ? [group([panel]), node] : [node, group([panel])] });
  if (!result.found) return state;
  next.root = result.node; next.selected = panel;
  return normalizeWorkshopLayout(next);
}

export function floatWorkshopPanel(state, panel, rect = {}, bounds) {
  if (!isPanel(panel)) return state;
  const next = detached(state, panel);
  const offset = 24 + next.floats.length * 28;
  next.floats.push({ panel, x: rect.x ?? offset, y: rect.y ?? offset,
    width: rect.width ?? 420, height: rect.height ?? 360 });
  next.selected = panel;
  return normalizeWorkshopLayout(next, bounds);
}

export function moveWorkshopFloat(state, panel, x, y, bounds) {
  const next = clone(state);
  const entry = next.floats.find(value => value.panel === panel);
  if (!entry || !Number.isFinite(x) || !Number.isFinite(y)) return state;
  entry.x = x; entry.y = y; next.selected = panel;
  return normalizeWorkshopLayout(next, bounds);
}

export function closeWorkshopPanel(state, panel) {
  if (!isPanel(panel)) return state;
  const next = detached(state, panel);
  next.closed.push(panel);
  next.selected = firstVisible(next.root, next.floats) || panel;
  return normalizeWorkshopLayout(next);
}

export function reopenWorkshopPanel(state, panel) {
  if (!isPanel(panel) || !state.closed.includes(panel)) return state;
  const target = firstVisible(state.root);
  if (!target) {
    const next = detached(state, panel); next.root = group([panel]); next.selected = panel;
    return normalizeWorkshopLayout(next);
  }
  return dockWorkshopPanel(state, panel, target, 'tab');
}
