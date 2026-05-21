/**
 * definitions-file-list-view.js
 *
 * Shared left-pane file list used by both Definitions Mode sections
 * (factions + complexity presets). One row per path, with a dirty-dot
 * indicator from `modeShell.isDirty(mode, path)` and click → `onSelect(path)`.
 *
 * `mode` is always `'Definitions'` — the section label distinction
 * (faction vs complexity) is encoded in the path prefix.
 */

export function renderDefinitionsFileListView(host, { paths, activePath, modeShell, mode = 'Definitions', onSelect }) {
  if (!host) return;
  host.innerHTML = '';

  const root = host.ownerDocument
    ? host.ownerDocument.createElement('div')
    : document.createElement('div');
  root.className = 'definitions-file-list';
  host.appendChild(root);

  if (!paths || paths.length === 0) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'No files.';
    root.appendChild(p);
    return;
  }

  for (const path of paths) {
    const row = document.createElement('div');
    row.className = 'definitions-file-list-row';
    row.dataset.path = path;
    if (path === activePath) row.classList.add('definitions-file-list-row-active');

    if (modeShell && modeShell.isDirty(mode, path)) {
      const dot = document.createElement('span');
      dot.className = 'dirty-dot';
      dot.textContent = '\u25CF';
      row.appendChild(dot);
    }

    const label = document.createElement('span');
    label.className = 'definitions-file-list-label';
    // Strip the assets/factions/ or assets/complexity/ prefix for display.
    label.textContent = path.replace(/^assets\/(factions|complexity)\//, '');
    row.appendChild(label);

    row.addEventListener('click', () => onSelect && onSelect(path));
    root.appendChild(row);
  }
}
