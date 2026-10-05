import { createRendezvousHost, createRendezvousJoiner } from '../../packages/transport/src/rendezvous-transport.js';
import { installSessionToken } from '../../packages/session/src/session-token.js';
const data = await (await fetch('./join-codes.json')).json();
const token = installSessionToken(window, { tabKey: 'grid-tab', sharedKey: 'grid-session', registryKey: 'grid-tabs' });
const $ = (id) => document.getElementById(id);
const status = (text) => { $('status').textContent = text; };
let transport, grid, timer;
const peers = new Set();
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
function stop() { $('digest').textContent = ''; for (const button of document.querySelectorAll('[data-dx]')) button.disabled = true; transport?.close(); transport = null; clearInterval(timer); peers.clear(); grid?.free(); grid = null; }
$('disconnect').onclick = () => { stop(); status('Disconnected'); };
$('join').onclick = () => {
  stop();
  transport = createRendezvousJoiner({ base: $('service').value, data, code: $('code').value, stamp: 'grid/1',
    getIdent: () => ({ token, name: 'Grid player' }), localise: false, onData: receive,
    onStatus: status, onError: status });
};
$('host').onclick = async () => {
  stop();
  try {
    const module = await import('./pkg/phoenix_grid.js'); await module.default(); grid = new module.GridHost();
    const saved = localStorage.getItem('grid-checkpoint'); if (saved) grid.restore(saved);
    transport = createRendezvousHost({ base: $('service').value, data, checkStamp: (stamp) => stamp === 'grid/1' ? { ok: true } : { ok: false, code: 'version-mismatch', detail: 'This host runs Grid 1' },
      onCode: (code) => { $('code').value = code.suffix; status(`Hosting ${code.suffix}`); }, onError: status,
      onConnection: (peer) => {
        peers.add(peer);
        let identified = false;
        peer.on('close', () => peers.delete(peer));
        peer.on('data', (raw) => {
          let message; try { message = JSON.parse(raw); } catch { return; }
          if (message.type === 'Identify') identified = typeof message.data?.token === 'string' && message.data.token.length > 0;
          if (!identified) return;
          if (message.type === 'Move') grid.move_piece(message.data?.dx, message.data?.dy);
          if (message.type === 'Identify' || message.type === 'Recover') peer.send(JSON.stringify({ type: 'Recovery', data: { checkpoint: grid.checkpoint() } }));
        });
      } });
    timer = setInterval(() => {
      grid.tick(); const raw = grid.state(); receive(JSON.parse(raw));
      for (const peer of peers) {
        if (peer.snapshotChannel?.readyState === 'open') { try { peer.snapshotChannel.send(raw); } catch {} }
        else peer.send(raw);
      }
      localStorage.setItem('grid-checkpoint', grid.checkpoint());
    }, 100);
  } catch (error) { stop(); status(`Cannot host: ${error.message || error}. Build the grid WASM first.`); }
};
for (const button of document.querySelectorAll('[data-dx]')) button.onclick = () => {
  const move = { dx: Number(button.dataset.dx), dy: Number(button.dataset.dy) };
  if (grid) grid.move_piece(move.dx, move.dy); else transport?.send('Move', move);
};
$('recover').onclick = () => { if (grid) status(`Local checkpoint at tick ${JSON.parse(grid.checkpoint()).state.tick}`); else transport?.send('Recover'); };
paint({x:0,y:0,tick:0});
for (const button of document.querySelectorAll('[data-dx]')) button.disabled = true;
