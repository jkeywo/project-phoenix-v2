/**
 * entity-file-list-view.js
 *
 * Left pane of Entity Mode. Renders one row per entity TOML path, with a
 * dirty-dot indicator (`modeShell.isDirty('Entity', path)`) and click →
 * `onSelect(path)`.
 */

export function renderEntityFileListView(host, { paths, activePath, modeShell, onSelect }) {
  if (!host) return;
  host.innerHTML = '';

  const root = document.createElement('div');
  root.className = 'entity-file-list';
  host.appendChild(root);

  if (!paths || paths.length === 0) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'No entity files. Pick a project root.';
    root.appendChild(p);
    return;
  }

  for (const path of paths) {
    const row = document.createElement('div');
    row.className = 'entity-file-list-row';
    row.dataset.path = path;
    if (path === activePath) row.classList.add('entity-file-list-row-active');

    if (modeShell && modeShell.isDirty('Entity', path)) {
      const dot = document.createElement('span');
      dot.className = 'dirty-dot';
      dot.textContent = '●';
      row.appendChild(dot);
    }

    const label = document.createElement('span');
    label.className = 'entity-file-list-label';
    label.textContent = path.replace(/^assets\/entities\//, '');
    row.appendChild(label);

    row.addEventListener('click', () => onSelect(path));
    root.appendChild(row);
  }
}
