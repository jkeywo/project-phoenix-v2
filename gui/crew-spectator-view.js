/** Camera-only controls. Never consumes the selected ship's console data. */
export function renderCrewSpectator(root, state, { t, send }) {
  if (!root) return;
  const signature = JSON.stringify([state, ...['title', 'following', 'empty', 'help', 'choose']
    .map(key => t(`client.crew_spectator.${key}`))]);
  if (root.dataset.projection === signature) return;
  root.dataset.projection = signature;
  const focused = root.ownerDocument.activeElement?.dataset?.shipUuid;
  const document = root.ownerDocument;
  const heading = document.createElement('h2');
  heading.textContent = t('client.crew_spectator.title');
  const status = document.createElement('p');
  status.setAttribute('role', 'status');
  const selected = (state.ships || []).find(ship => ship.uuid === state.target);
  status.textContent = selected
    ? `${t('client.crew_spectator.following')} ${selected.name}`
    : t('client.crew_spectator.empty');
  const help = document.createElement('p');
  help.textContent = t('client.crew_spectator.help');
  const list = document.createElement('div');
  list.setAttribute('role', 'group');
  list.setAttribute('aria-label', t('client.crew_spectator.choose'));
  for (const ship of state.ships || []) {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = ship.name;
    button.dataset.shipUuid = ship.uuid;
    button.setAttribute('aria-pressed', String(ship.uuid === state.target));
    button.disabled = !state.can_select;
    button.addEventListener('click', () => send('SelectCrewSpectatorTarget', { uuid: ship.uuid }));
    list.append(button);
  }
  root.replaceChildren(heading, status, help, list);
  if (focused) {
    const next = [...list.children].find(button => button.dataset.shipUuid === focused)
      || [...list.children].find(button => button.dataset.shipUuid === state.target);
    if (next) next.focus();
    else { root.tabIndex = -1; root.focus(); }
  }
}

if (typeof window !== 'undefined') window.renderCrewSpectator = renderCrewSpectator;
