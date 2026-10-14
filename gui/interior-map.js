/** Render the permitted own-ship layout and physical team projection. */
export function renderInteriorMap(host, state, translate, dispatch, seal, viewChange) {
  const layout = state?.interior;
  host.hidden = !layout;
  if (!layout) { host.replaceChildren(); return; }
  const doc = host.ownerDocument;
  const signature = JSON.stringify(layout);
  if (host.dataset.layout !== signature) {
    host.dataset.layout = signature;
    host.replaceChildren();
    const label = doc.createElement('label');
    label.textContent = translate('interior.map.deck');
    const select = doc.createElement('select');
    select.setAttribute('aria-label', translate('interior.map.deck'));
    for (const deck of layout.deck || []) {
      const option = doc.createElement('option'); option.value = deck.id;
      option.textContent = translate(deck.label); select.append(option);
    }
    label.append(select); host.append(label, doc.createElement('div'));
    select.addEventListener('change', () => {
      const view={...(host.mapState.console_view || {}),interior_deck:select.value};
      const selected={...host.mapState,console_view:view};
      renderInteriorMap(host, selected, translate, host.mapDispatch, host.mapSeal,host.mapViewChange);
      host.mapViewChange?.(view);
    });
  }
  host.mapState = state; host.mapDispatch = dispatch; host.mapSeal = seal; host.mapViewChange=viewChange;
  const select=host.querySelector('select');
  if ((layout.deck || []).some(deck=>deck.id===state.console_view?.interior_deck)) select.value=state.console_view.interior_deck;
  const focused=host.getRootNode().activeElement || doc.activeElement;
  const focusKey=host.contains(focused) ? focused.dataset.mapFocus : null;
  const deck = host.querySelector('select')?.value;
  const body = host.lastElementChild; body.replaceChildren();
  const rooms = (layout.compartment || []).filter(room => room.deck === deck);
  if (!rooms.length) return;
  const minX = Math.min(...rooms.map(room => room.x));
  const minZ = Math.min(...rooms.map(room => room.z));
  const maxX = Math.max(...rooms.map(room => room.x + room.width));
  const maxZ = Math.max(...rooms.map(room => room.z + room.depth));
  const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', `${minX - 1} ${minZ - 1} ${maxX - minX + 2} ${maxZ - minZ + 2}`);
  svg.setAttribute('role', 'img'); svg.setAttribute('aria-label', translate('interior.map.layout'));
  svg.style.cssText = 'width:100%;max-height:18rem;display:block';
  for (const room of rooms) {
    const rect = doc.createElementNS(svg.namespaceURI, 'rect');
    for (const [key, value] of Object.entries({x:room.x, y:room.z, width:room.width, height:room.depth, fill:'var(--bg-card, #122)', stroke:'var(--cyan, #5cc)', 'stroke-width':0.15})) rect.setAttribute(key, String(value));
    const title = doc.createElementNS(svg.namespaceURI, 'title'); title.textContent = translate(room.label); rect.append(title); svg.append(rect);
    const text = doc.createElementNS(svg.namespaceURI, 'text');
    text.setAttribute('x', String(room.x + 0.3)); text.setAttribute('y', String(room.z + 1));
    text.setAttribute('font-size', '0.7'); text.setAttribute('fill', 'currentColor'); text.textContent = translate(room.label); svg.append(text);
  }
  body.append(svg);
  for (const boundary of state.boundaries || []) {
    const row = doc.createElement('p');
    const status = doc.createElement('span');
    status.textContent = `${boundary.system}: ${translate(`interior.boundary.${boundary.phase}`)} `;
    row.append(status);
    if (boundary.controllable) {
      const button = doc.createElement('button'); button.type = 'button';
      button.dataset.mapFocus=`gate:${boundary.system}`;
      const requested=boundary.seal_requested || boundary.actuator_sealed;
      button.textContent = translate(requested ? 'interior.boundary.release' : 'interior.boundary.seal');
      button.addEventListener('click', () => host.mapSeal?.(boundary.system, !requested)); row.append(button);
    }
    body.append(row);
  }
  const allowed = new Set(state.damageable_systems || []);
  for (const room of rooms) {
    const section = doc.createElement('section');
    const name = doc.createElement('strong'); name.textContent = translate(room.label); section.append(name);
    const physical=(state.system_hull || []).find(row=>row.physical?.compartment===room.id)?.physical;
    if (physical) {
      const status=doc.createElement('p');
      status.textContent=translate('damage.status.structure',{room:translate(room.label),current:Math.round(physical.structural_hp),max:Math.round(physical.structural_max_hp)});
      section.append(status);
    }
    const repair=doc.createElement('button');repair.type='button';
    repair.textContent=translate('damage.repair.structure');repair.dataset.mapFocus=`room:${room.id}`;
    repair.disabled=!!state.auto || !Number.isInteger(state.selectedTeam);
    repair.addEventListener('click',()=>{if(!repair.disabled)host.mapDispatch?.({kind:'compartment',compartment:room.id});});section.append(repair);
    for (const [system, compartment] of Object.entries(layout.placements || {})) {
      if (compartment !== room.id) continue;
      const button = doc.createElement('button'); button.type = 'button'; button.textContent = system;
      button.dataset.mapFocus=`system:${system}`;
      button.disabled = !!state.auto || !Number.isInteger(state.selectedTeam) || !allowed.has(system);
      button.addEventListener('click', () => { if (!button.disabled) host.mapDispatch?.(system); }); section.append(button);
      const detail=(state.system_hull || []).find(row=>row.system_id===system)?.physical;
      for (const component of detail?.components || []) {
        const status=doc.createElement('span');status.textContent=`${component.id} · ${translate('damage.status.'+component.condition.kind)}`;section.append(status);
        if (component.condition.kind==='healthy') continue;
        const work=doc.createElement('button');work.type='button';work.textContent=translate('damage.repair.component',{component:component.id});
        work.dataset.mapFocus=`component:${system}:${component.id}`;
        work.disabled=button.disabled;
        work.addEventListener('click',()=>{if(!work.disabled)host.mapDispatch?.({kind:'component',system,component:component.id});});section.append(work);
      }
    }
    const present = (state.navigation || []).flatMap((team, index) => team.location?.kind === 'compartment' && team.location.compartment === room.id ? [index + 1] : []);
    if (present.length) { const p = doc.createElement('p'); p.textContent = translate('interior.map.teams', {teams: present.join(', ')}); section.append(p); }
    body.append(section);
  }
  for (const [index, team] of (state.navigation || []).entries()) {
    const p = doc.createElement('p');
    const location = team.location || {};
    const place = location.kind === 'crossing' ? `${location.from} → ${location.to}` : location.compartment || location.target || '';
    p.textContent = translate(team.blocked ? 'interior.map.blocked' : 'interior.map.location', {n:index + 1, place}); body.append(p);
  }
  if (focusKey) [...host.querySelectorAll('[data-map-focus]')].find(element=>element.dataset.mapFocus===focusKey)?.focus({preventScroll:true});
}
