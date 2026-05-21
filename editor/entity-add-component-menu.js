/**
 * entity-add-component-menu.js
 *
 * Two-tier picker for the "+ Add Component" button in Entity Mode.
 *
 *   Top tier: combo templates (Ship, Station, Region, NPC, Asteroid,
 *             Asteroid Field, Star, Planet) + a single "Raw section ▸"
 *             entry that expands a flat submenu.
 *   Bottom tier: one row per `ENTITY_CONFIG_SECTIONS` key.
 *
 * Selecting either tier returns the choice via the `onSelect` callback:
 *   { kind: 'combo', name: 'Ship' }
 *   { kind: 'raw',   sectionKey: 'hull' }
 *
 * Caller is responsible for hiding/removing the menu on selection.
 */

import { getPickerModel } from './component-templates.js';

/**
 * Render the picker into `host`. Returns the root <div> element so the
 * caller can position/remove it.
 *
 * @param {HTMLElement} host
 * @param {(choice:{kind:'combo'|'raw',name?:string,sectionKey?:string}) => void} onSelect
 * @returns {HTMLElement}
 */
export function renderAddComponentMenu(host, onSelect) {
  if (!host) return null;
  host.innerHTML = '';

  const root = document.createElement('div');
  root.className = 'entity-add-menu';
  host.appendChild(root);

  const model = getPickerModel();

  const state = { mode: 'top' }; // 'top' | 'raw'

  const renderTop = () => {
    root.innerHTML = '';
    for (const combo of model.combos) {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = 'entity-add-menu-item entity-add-menu-combo';
      item.dataset.combo = combo.name;
      item.textContent = combo.label;
      item.addEventListener('click', () => onSelect({ kind: 'combo', name: combo.name }));
      root.appendChild(item);
    }

    const rawBtn = document.createElement('button');
    rawBtn.type = 'button';
    rawBtn.className = 'entity-add-menu-item entity-add-menu-raw-toggle';
    rawBtn.textContent = 'Raw section ▸';
    rawBtn.addEventListener('click', () => {
      state.mode = 'raw';
      renderRaw();
    });
    root.appendChild(rawBtn);
  };

  const renderRaw = () => {
    root.innerHTML = '';
    const back = document.createElement('button');
    back.type = 'button';
    back.className = 'entity-add-menu-item entity-add-menu-back';
    back.textContent = '◂ Back';
    back.addEventListener('click', () => {
      state.mode = 'top';
      renderTop();
    });
    root.appendChild(back);

    const sub = document.createElement('div');
    sub.className = 'entity-add-menu-submenu';
    for (const r of model.rawSections) {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = 'entity-add-menu-item entity-add-menu-raw-section';
      item.dataset.section = r.key;
      item.textContent = r.label;
      item.addEventListener('click', () => onSelect({ kind: 'raw', sectionKey: r.key }));
      sub.appendChild(item);
    }
    root.appendChild(sub);
  };

  renderTop();
  return root;
}
