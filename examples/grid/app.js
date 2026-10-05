import { createGridLifecycle, createGridPeers } from './host-lifecycle.js';
import { createRendezvousHost, createRendezvousJoiner } from '../../packages/transport/src/rendezvous-transport.js';
import { installSessionToken } from '../../packages/session/src/session-token.js';
const data = await (await fetch('./join-codes.json')).json();
const token = installSessionToken(window, { tabKey: 'grid-tab', sharedKey: 'grid-session', registryKey: 'grid-tabs' });
const $ = (id) => document.getElementById(id);
const status = (text) => { $('status').textContent = text; };
let transport, grid;
const lifecycle = createGridLifecycle();
const ctx = document.querySelector('canvas').getContext('2d');
function paint(state, digest = '') {
  ctx.clearRect(0, 0, 400, 400);
  for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) {
    ctx.fillStyle = (x + y) % 2 ? '#354859' : '#273744'; ctx.fillRect(x * 50, y * 50, 50, 50);
  }
  ctx.fillStyle = '#ffc857'; ctx.fillRect(state.x * 50 + 8, state.y * 50 + 8, 34, 34);
  $('digest').textContent = `Tick ${state.tick}  ${digest}`;
}
function receive(message) {
  if (message.type === 'State') { paint(message.data.state, message.data.digest); for (const button of document.querySelectorAll('[data-dx]')) button.disabled = false; }
  if (message.type === 'Recovery') {
    const saved = JSON.parse(message.data.checkpoint);
    paint(saved.state); status(`Checkpoint received at tick ${saved.state.tick}`);
  }
}
function stop() {
  lifecycle.stop(); transport = null; grid = null;
  $('digest').textContent = '';
  for (const button of document.querySelectorAll('[data-dx]')) button.disabled = true;
}
$('disconnect').onclick = () => { stop(); status('Disconnected'); };
$('join').onclick = () => {
  stop();
  const operation = lifecycle.begin();
  transport = createRendezvousJoiner({ base: $('service').value, data, code: $('code').value, stamp: 'grid/1',
    onAccepted: ({ send }) => send(JSON.stringify({ type: 'Identify', data: { token, name: 'Grid player' } })),
    onData: message => { if (operation.current()) receive(message); },
    onStatus: text => { if (operation.current()) status(text); }, onError: text => { if (operation.current()) status(text); } });
  const ownedTransport = transport; operation.own(() => ownedTransport.close());
};
$('host').onclick = async () => {
  stop();
  const operation = lifecycle.begin();
  try {
    const module = await import('./pkg/phoenix_grid.js');
    if (!operation.current()) return;
    await module.default();
    if (!operation.current()) return;
    const ownedGrid = new module.GridHost(); operation.own(() => ownedGrid.free());
    const saved = localStorage.getItem('grid-checkpoint'); if (saved) ownedGrid.restore(saved);
    const peers = createGridPeers(ownedGrid); operation.own(() => peers.close());
    const ownedTransport = createRendezvousHost({ base: $('service').value, data,
      checkStamp: stamp => JSON.parse(module.GridHost.check_stamp(stamp || '')),
      onCode: code => { if (operation.current()) { $('code').value = code.suffix; status(`Hosting ${code.suffix}`); } },
      onError: text => { if (operation.current()) status(text); },
      onConnection: peer => { if (operation.current()) peers.attach(peer); else peer.close(); },
    });
    operation.own(() => ownedTransport.close());
    grid = ownedGrid; transport = ownedTransport;
    const ownedTimer = setInterval(() => {
      if (!operation.current()) return;
      ownedGrid.tick(); const raw = ownedGrid.state(); receive(JSON.parse(raw));
      peers.publish(raw);
      localStorage.setItem('grid-checkpoint', ownedGrid.checkpoint());
    }, 100);
    operation.own(() => clearInterval(ownedTimer));
  } catch (error) {
    if (operation.current()) { stop(); status(`Cannot host: ${error.message || error}. Build the grid WASM first.`); }
  }
};
for (const button of document.querySelectorAll('[data-dx]')) button.onclick = () => {
  const move = { dx: Number(button.dataset.dx), dy: Number(button.dataset.dy) };
  if (grid) grid.move_piece(move.dx, move.dy); else transport?.send('Move', move);
};
$('recover').onclick = () => { if (grid) status(`Local checkpoint at tick ${JSON.parse(grid.checkpoint()).state.tick}`); else transport?.send('Recover'); };
paint({x:0,y:0,tick:0});
for (const button of document.querySelectorAll('[data-dx]')) button.disabled = true;
