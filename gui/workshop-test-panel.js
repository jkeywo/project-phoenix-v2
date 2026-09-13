import { createWorkshopTest } from '../editor/workshop-test.js';
import { t } from './strings.js';

/** Shared offline controls over the native child/browser iframe capability. */
export function mountWorkshopTestPanel({ root, provider, draft: getDraft, busy: isBusy,
  changed = () => {}, win = root.ownerDocument.defaultView }) {
  if (!provider?.test) return { refresh() {}, held: () => false, testing: () => false, dispose() {} };
  const doc = root.ownerDocument;
  const panel = doc.createElement('section');
  panel.className = 'workshop-test';
  const make = (tag, id, textId) => {
    const node = doc.createElement(tag); node.id = `workshop-test-${id}`;
    if (textId) node.textContent = t(textId);
    return node;
  };
  const command = (id, text, invoke) => {
    const node = make('button', id, text); node.type = 'button';
    node.addEventListener('click', () => { void invoke().catch(() => {}); });
    return node;
  };
  const status = make('p', 'status'); status.setAttribute('role', 'status');
  const world = make('select', 'world');
  const ship = make('select', 'ship');
  const seed = make('input', 'seed'); seed.type = 'number'; seed.min = '0'; seed.max = String(Number.MAX_SAFE_INTEGER); seed.step = '1'; seed.value = '1';
  let choices = { worlds: [], ships: [] }, choicesReady = false;
  const validSelection = () => choicesReady && choices.worlds.includes(world.value)
    && choices.ships.includes(ship.value) && seed.value.trim() !== ''
    && Number.isSafeInteger(Number(seed.value)) && Number(seed.value) >= 0;
  const selection = () => {
    const number = Number(seed.value);
    if (!validSelection()) throw new Error(t('workshop.test_selection_invalid'));
    return { world: world.value, ship: ship.value, seed: number };
  };
  let session;
  let previousDraft, previousRevision;
  let catalogGeneration = 0, catalogTimer = null, catalogPending = false;
  let localError = null, disposed = false;
  const start = command('start', 'workshop.test_start', async () => {
    localError = null;
    try { await session.start(selection()); }
    catch (error) { localError = error; render(); }
  });
  for (const field of [world, ship, seed]) field.addEventListener('change', () => { localError = null; render(); });
  const authoring = command('authoring', 'workshop.test_authoring', () => session.authoring());
  const returnTest = command('return', 'workshop.test_return', () => session.test());
  const pause = command('pause', 'workshop.test_pause', () => session.control({ command: 'pause' }));
  const resume = command('resume', 'workshop.test_resume', () => session.control({ command: 'resume' }));
  const step = command('step', 'workshop.test_step', () => session.control({ command: 'step' }));
  const stop = command('stop', 'workshop.test_stop', () => session.stop());
  const speed = make('select', 'speed');
  for (const value of [1, 2, 4, 8]) {
    const option = doc.createElement('option'); option.value = String(value); option.textContent = `${value}×`; speed.append(option);
  }
  speed.addEventListener('change', () => { void session.control({ command: 'rate', multiplier: Number(speed.value) }).catch(() => {}); });
  panel.append(make('h2', 'heading', 'workshop.test_heading'), make('p', 'scope', 'workshop.test_scope'));
  const label = (target, id) => { const node = doc.createElement('label'); node.htmlFor = target.id; node.textContent = t(id); panel.append(node, target); };
  label(world, 'workshop.test_world'); label(ship, 'workshop.test_ship'); label(seed, 'workshop.test_seed');
  panel.append(start, returnTest, authoring, pause, resume, step);
  label(speed, 'workshop.test_speed'); panel.append(stop, status); root.append(panel);
  function render() {
    if (!session) return;
    const state = session.state(), draft = getDraft();
    const busy = isBusy() || state.busy;
    const testing = state.mode === 'test';
    const running = !!state.run;
    start.disabled = busy || catalogPending || !draft || !validSelection();
    start.textContent = t(running ? 'workshop.test_restart' : 'workshop.test_start');
    world.disabled = ship.disabled = seed.disabled = busy || testing;
    authoring.hidden = !testing; authoring.disabled = busy;
    returnTest.hidden = testing || !running; returnTest.disabled = busy;
    pause.disabled = busy || !testing || state.run?.paused || state.run?.starting;
    resume.disabled = busy || !testing || !state.run?.paused || state.run?.starting;
    step.disabled = resume.disabled;
    speed.disabled = busy || !testing || state.run?.starting;
    stop.disabled = busy || !running;
    speed.value = String(state.run?.multiplier || 1);
    const lines = [t(testing ? 'workshop.test_mode' : 'workshop.test_authoring_mode')];
    if (running) lines.push(t(state.run.starting ? 'workshop.test_starting' : state.run.paused ? 'workshop.test_held' : 'workshop.test_running', { tick: String(state.run.tick) }));
    if (state.stale) lines.push(t('workshop.test_stale'));
    if (choicesReady && !catalogPending && !validSelection()) lines.push(t('workshop.test_selection_invalid'));
    const error = localError || state.error;
    if (error) lines.push(String(error?.message || error));
    for (const finding of error?.report?.findings || []) lines.push(`${finding.file}${finding.line ? `:${finding.line}` : ''}: ${finding.message}`);
    status.textContent = lines.join(' '); status.setAttribute('role', error ? 'alert' : 'status');
  }
  session = createWorkshopTest({ provider: provider.test, snapshot: () => getDraft().toNativeSources(),
    onChange() { render(); changed(); } });
  function refresh() {
    const draft = getDraft();
    if (draft !== previousDraft || draft?.sourceRevision !== previousRevision) {
      if (draft !== previousDraft) { world.replaceChildren(); ship.replaceChildren(); choicesReady = false; }
      previousDraft = draft; previousRevision = draft?.sourceRevision;
      session.changed();
      const generation = ++catalogGeneration;
      if (catalogTimer !== null) win.clearTimeout(catalogTimer);
      catalogPending = !!draft;
      if (!draft) { world.replaceChildren(); ship.replaceChildren(); }
      else catalogTimer = win.setTimeout(async () => {
        const files = Object.fromEntries(Object.entries(draft.toNativeSources())
          .filter(([path, value]) => typeof value === 'string' && /\.(toml|rhai)$/.test(path)));
        try {
          const catalog = await provider.test.catalog(files);
          if (disposed || generation !== catalogGeneration) return;
          choices = catalog; choicesReady = true;
          for (const [select, paths] of [[world, catalog.worlds], [ship, catalog.ships]]) {
            const current = select.value;
            select.replaceChildren(...paths.map(path => {
              const option = doc.createElement('option'); option.value = path; option.textContent = path; return option;
            }));
            if (current && !paths.includes(current)) {
              const stale = doc.createElement('option'); stale.value = current; stale.textContent = current; stale.disabled = true;
              select.append(stale);
            }
            if (current) select.value = current;
          }
          localError = null;
        } catch (error) {
          if (disposed || generation !== catalogGeneration) return;
          localError = error; choicesReady = false;
        } finally {
          if (!disposed && generation === catalogGeneration) { catalogPending = false; render(); }
        }
      }, 150);
    }
    render();
  }
  const timer = win.setInterval(() => { void session.poll().catch(() => {}); }, 500);
  refresh();
  return { refresh, held: () => session.state().busy || session.state().mode === 'test', testing: () => session.state().mode === 'test',
    dispose() { disposed = true; win.clearInterval(timer); if (catalogTimer !== null) win.clearTimeout(catalogTimer);
      void session.dispose().catch(() => {}); panel.remove(); } };
}
