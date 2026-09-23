/** Sole owner of the session strip's visibility and enabled state. */
export function createGmSessionWidget({ doc, t, getOperator }) {
  const actions = doc.getElementById('gm-session-actions');
  const pause = doc.getElementById('gm-session-pause');
  const resume = doc.getElementById('gm-session-resume');
  for (const button of [pause, resume]) if (button) button.dataset.sessionOwner = 'widget';
  const ready = doc.createElement('button');
  ready.id = 'gm-header-ready'; ready.type = 'button';
  actions?.prepend(ready);
  actions?.classList.add('gm-state-widget');
  let phase = null, paused = null;
  const render = () => {
    const operator = getOperator();
    const connected = !!operator && operator.connected !== false;
    ready.hidden = phase !== 'Lobby';
    ready.textContent = t(operator?.ready ? 'server.gm.start.unready' : 'server.gm.start.ready');
    ready.disabled = !connected || phase !== 'Lobby';
    for (const [button, visible] of [[pause, phase === 'InProgress' && paused !== true],
      [resume, phase === 'InProgress' && paused === true]]) {
      if (!button) continue;
      button.hidden = !visible;
      button.disabled = !connected || paused == null || !visible;
      button.setAttribute('aria-disabled', String(button.disabled));
    }
  };
  ready.addEventListener('click', () => {
    if (!ready.disabled) doc.getElementById('gm-ready-btn')?.click();
  });
  render();
  return { render, update(value = {}) {
    // These are independent absolute channels. A phase-only publication must
    // not erase a pause value already received for this world. reset() is the
    // world-generation boundary that makes previously known state unknown.
    if (typeof value.phase === 'string') phase = value.phase;
    if (typeof value.paused === 'boolean') paused = value.paused;
    render();
  }, reset() { phase = null; paused = null; render(); }, dispose() { ready.remove(); } };
}
