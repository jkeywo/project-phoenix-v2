/** Keyed presentation tree. Selection never grants station authority. */
export function createGmEntityTree({ root, doc, t, onSelect }) {
  const search = doc.createElement('input'); search.type = 'search';
  search.placeholder = t('server.gm.tree.search'); search.setAttribute('aria-label', search.placeholder);
  const tree = doc.createElement('div'); tree.setAttribute('role', 'tree');
  tree.setAttribute('aria-label', t('server.gm.tree.title'));
  root.replaceChildren(search, tree);
  const nodes = new Map(), expanded = new Set();
  let selected = null, focused = null, rows = [], signature = '';
  function select(row) {
    selected = row.id; paint(); onSelect(row);
  }
  function paint() {
    const query = search.value.trim().toLocaleLowerCase();
    const visible = new Set();
    const byId = new Map(rows.map(row => [row.id, row]));
    if (!byId.has(selected)) selected = null;
    if (!byId.has(focused)) focused = selected || rows[0]?.id;
    for (const row of rows) {
      if (!query || row.label.toLocaleLowerCase().includes(query)) {
        let parent = row;
        while (parent) { visible.add(parent.id); parent = byId.get(parent.parent); }
      }
    }
    const retained = new Set();
    const branches = new Set(rows.map(row => row.parent));
    const previousSibling = new Map();
    for (const row of rows) {
      retained.add(row.id);
      let item = nodes.get(row.id);
      if (!item) {
        const el = doc.createElement('div'); el.className = 'gm-tree-node';
        el.setAttribute('role', 'treeitem'); el.tabIndex = -1;
        const line = doc.createElement('div'); line.className = 'gm-tree-line';
        const toggle = doc.createElement('button'); toggle.type = 'button'; toggle.tabIndex = -1;
        const label = doc.createElement('button'); label.type = 'button'; label.tabIndex = -1;
        const group = doc.createElement('div'); group.setAttribute('role', 'group');
        line.append(toggle, label); el.append(line, group);
        item = { el, toggle, label, group, row }; nodes.set(row.id, item);
        toggle.addEventListener('click', () => {
          expanded.has(row.id) ? expanded.delete(row.id) : expanded.add(row.id); paint();
        });
        label.addEventListener('click', () => { select(item.row); el.focus(); });
        el.addEventListener('focus', () => {
          focused = item.row.id;
          for (const node of nodes.values()) node.el.tabIndex = node.row.id === focused ? 0 : -1;
        });
        el.addEventListener('keydown', event => {
          if (event.target !== el) return;
          const items = [...tree.querySelectorAll('[role="treeitem"]')].filter(node => !node.closest('[hidden]'));
          const index = items.indexOf(el);
          if (event.key === 'ArrowDown') items[index + 1]?.focus();
          else if (event.key === 'ArrowUp') items[index - 1]?.focus();
          else if (event.key === 'ArrowRight') {
            if (expanded.has(row.id)) item.group.firstElementChild?.focus();
            else { expanded.add(row.id); paint(); }
          }
          else if (event.key === 'ArrowLeft') {
            if (expanded.delete(row.id)) paint(); else nodes.get(item.row.parent)?.el.focus();
          } else if (event.key === 'Home') items[0]?.focus();
          else if (event.key === 'End') items.at(-1)?.focus();
          else if (event.key === 'Enter' || event.key === ' ') select(item.row);
          else return;
          event.preventDefault(); event.stopPropagation();
        });
      }
      item.row = row;
      if (item.label.textContent !== row.label) item.label.textContent = row.label;
      const branch = branches.has(row.id);
      const open = !!query || expanded.has(row.id);
      item.toggle.hidden = !branch; item.toggle.textContent = open ? '−' : '+';
      item.toggle.setAttribute('aria-label', t(open ? 'server.gm.tree.collapse' : 'server.gm.tree.expand', { name: row.label }));
      item.group.hidden = !open;
      if (branch) item.el.setAttribute('aria-expanded', String(open)); else item.el.removeAttribute('aria-expanded');
      item.el.hidden = !visible.has(row.id);
      item.el.setAttribute('aria-selected', String(row.id === selected));
      item.el.tabIndex = row.id === focused ? 0 : -1;
      const parent = nodes.get(row.parent)?.group || tree;
      const before = previousSibling.get(parent)?.nextElementSibling || parent.firstElementChild;
      if (before !== item.el) parent.insertBefore(item.el, before);
      previousSibling.set(parent, item.el);
    }
    for (const [id, item] of nodes) if (!retained.has(id)) { item.el.remove(); nodes.delete(id); expanded.delete(id); }
  }
  search.addEventListener('input', paint);
  return { update({ entities = [], worlds = {}, membership = {}, stations = [], slots = [] }) {
    const next = [];
    const worldIds = new Set([...Object.keys(worlds), ...entities.map(entity => membership[entity.entity_id] || 'unassigned')]);
    if (slots.length) worldIds.add('root');
    for (const world of [...worldIds].sort()) {
      const id = `world:${world}`;
      next.push({ id, kind: 'world', world, label: worlds[world]?.label || (world === 'unassigned' ? t('server.gm.tree.unassigned') : world) });
      const members = entities.filter(entity => (membership[entity.entity_id] || 'unassigned') === world);
      const factions = new Map(members.map(entity => [entity.faction?.entity_id || 'none', entity.faction?.name || t('server.gm.tree.no_faction')]));
      for (const [faction, label] of [...factions].sort((a, b) => a[1].localeCompare(b[1]))) {
        const factionId = `${id}:faction:${faction}`;
        next.push({ id: factionId, parent: id, kind: 'faction', world, faction, label });
        for (const entity of members.filter(entity => (entity.faction?.entity_id || 'none') === faction).sort((a, b) => a.name.localeCompare(b.name))) {
          const entityId = `entity:${entity.entity_id}`;
          next.push({ id: entityId, parent: factionId, kind: 'entity', entity, label: entity.name || entity.entity_id });
          for (const station of stations.find(ship => ship.ship_id === entity.entity_id)?.stations || []) {
            next.push({ id: `${entityId}:station:${station.station_id}`, parent: entityId, kind: 'station', entity, station,
              label: `${station.name} · ${station.rating}` });
          }
        }
      }
      if (world === 'root') for (const slot of slots.filter(slot => slot.state === 'empty')) {
        next.push({ id: `slot:${slot.slot_id || slot.id}`, parent: id, kind: 'slot', slot,
          label: slot.label || slot.slot_id || slot.id });
      }
    }
    if (!rows.length) next.filter(row => row.kind === 'world').forEach(row => expanded.add(row.id));
    rows = next;
    const nextSignature = JSON.stringify(rows.map(row => [row.id, row.parent, row.label, row.slot?.can_backfill]));
    if (signature === nextSignature) {
      for (const row of rows) if (nodes.has(row.id)) nodes.get(row.id).row = row;
      return;
    }
    signature = nextSignature; paint();
  }, selectEntity(id) {
    const next = id ? `entity:${id}` : null;
    if (next === selected) return;
    selected = next;
    let row = rows.find(row => row.id === selected);
    while (row?.parent) { expanded.add(row.parent); row = rows.find(parent => parent.id === row.parent); }
    paint();
  } };
}
