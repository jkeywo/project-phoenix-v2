/**
 * entity-component-stack-view.js
 *
 * Centre pane of Entity Mode. Stacks one card per `ComponentCard` plus a
 * "+ Add Component" button that opens the two-tier picker.
 */

import { renderEntityComponentCard } from './entity-component-card-view.js';
import { renderAddComponentMenu } from './entity-add-component-menu.js';

export function renderEntityComponentStackView(host, { cards, deps, onAddChoice }) {
  if (!host) return;
  host.innerHTML = '';

  const root = document.createElement('div');
  root.className = 'entity-component-stack';
  host.appendChild(root);

  if (!cards || cards.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'placeholder';
    empty.textContent = 'No components — add one below.';
    root.appendChild(empty);
  }

  for (const card of cards || []) {
    const cardHost = document.createElement('div');
    cardHost.className = 'entity-component-stack-card-host';
    root.appendChild(cardHost);
    renderEntityComponentCard(cardHost, card, deps);
  }

  // Add-component composer.
  const addWrap = document.createElement('div');
  addWrap.className = 'entity-add-component';
  root.appendChild(addWrap);

  const addBtn = document.createElement('button');
  addBtn.type = 'button';
  addBtn.className = 'entity-add-component-btn';
  addBtn.textContent = '+ Add Component';
  addWrap.appendChild(addBtn);

  const menuHost = document.createElement('div');
  menuHost.className = 'entity-add-component-menu-host';
  menuHost.style.display = 'none';
  addWrap.appendChild(menuHost);

  addBtn.addEventListener('click', () => {
    const shown = menuHost.style.display !== 'none';
    if (shown) {
      menuHost.style.display = 'none';
      menuHost.innerHTML = '';
      return;
    }
    menuHost.style.display = '';
    renderAddComponentMenu(menuHost, (choice) => {
      menuHost.style.display = 'none';
      menuHost.innerHTML = '';
      onAddChoice(choice);
    });
  });
}
