import { createWorkshopTest } from '../editor/workshop-test.js';
import { validateTestBreakpoint } from '../editor/workshop-test-breakpoint.js';
import { t } from './strings.js';

/** Shared offline controls over the native child/browser iframe capability. */
export function mountWorkshopTestPanel({ root, provider, draft: getDraft, busy: isBusy,
  changed = () => {}, leave = () => {}, openSource = () => {}, win = root.ownerDocument.defaultView }) {
  if (!provider?.test) return { node: null, viewNode: null, traceNode: null, refresh() {}, held: () => false,
    testing: () => false, dispose() {} };
  const doc = root.ownerDocument;
  const panel = doc.createElement('section');
  panel.className = 'workshop-test';
  const viewPanel = doc.createElement('section');
  viewPanel.className = 'workshop-test-document';
  const tracePanel = doc.createElement('section');
  tracePanel.className = 'workshop-test-trace';
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
  const breakpointEnabled = make('input', 'breakpoint-enabled'); breakpointEnabled.type = 'checkbox';
  const breakpointKind = make('select', 'breakpoint-kind');
  for (const [value, id] of [['flag', 'workshop.test_breakpoint_flag'], ['counter', 'workshop.test_breakpoint_counter']]) {
    const option = doc.createElement('option'); option.value = value; option.textContent = t(id); breakpointKind.append(option);
  }
  const breakpointName = make('input', 'breakpoint-name'); breakpointName.maxLength = 128;
  const breakpointLayer = make('select', 'breakpoint-layer');
  const breakpointFlagValue = make('select', 'breakpoint-flag-value');
  for (const [value, id] of [['true', 'workshop.test_breakpoint_true'], ['false', 'workshop.test_breakpoint_false']]) {
    const option = doc.createElement('option'); option.value = value; option.textContent = t(id); breakpointFlagValue.append(option);
  }
  const breakpointComparison = make('select', 'breakpoint-comparison');
  for (const value of ['eq', 'ne', 'ge', 'gt', 'le', 'lt']) {
    const option = doc.createElement('option'); option.value = value; option.textContent = value; breakpointComparison.append(option);
  }
  const breakpointValue = make('input', 'breakpoint-value'); breakpointValue.type = 'number'; breakpointValue.step = '1'; breakpointValue.value = '1';
  let choices = { worlds: [], ships: [], layers: {} }, choicesReady = false;
  const breakpoint = () => validateTestBreakpoint(!breakpointEnabled.checked ? null : {
    layer: breakpointLayer.value || null,
    condition: breakpointKind.value === 'flag'
      ? { kind: 'flag', name: breakpointName.value.trim(), value: breakpointFlagValue.value === 'true' }
      : { kind: 'counter', name: breakpointName.value.trim(), comparison: breakpointComparison.value,
          value: Number(breakpointValue.value) },
  }, choices.layers?.[world.value] || []);
  const validSelection = () => choicesReady && choices.worlds.includes(world.value)
    && choices.ships.includes(ship.value) && seed.value.trim() !== ''
    && Number.isSafeInteger(Number(seed.value)) && Number(seed.value) >= 0
    && (() => { try { breakpoint(); return true; } catch (_) { return false; } })();
  const selection = () => {
    const number = Number(seed.value);
    if (!validSelection()) throw new Error(t('workshop.test_selection_invalid'));
    return { world: world.value, ship: ship.value, seed: number, breakpoint: breakpoint() };
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
  const paintBreakpointLayers = () => {
    const current = breakpointLayer.value;
    const rootOption = doc.createElement('option'); rootOption.value = '';
    rootOption.textContent = t('workshop.test_trace_root');
    const layers = choices.layers?.[world.value] || [];
    breakpointLayer.replaceChildren(rootOption, ...layers.map(path => {
      const option = doc.createElement('option'); option.value = path; option.textContent = path; return option;
    }));
    breakpointLayer.value = layers.includes(current) ? current : '';
  };
  for (const candidate of [world, ship, seed, breakpointEnabled, breakpointKind, breakpointName,
    breakpointLayer, breakpointFlagValue, breakpointComparison, breakpointValue]) {
    candidate.addEventListener(candidate === breakpointName || candidate === breakpointValue ? 'input' : 'change',
      () => { localError = null; if (candidate === world) paintBreakpointLayers(); render(); });
  }
  const authoring = command('authoring', 'workshop.test_authoring', async () => {
    await session.authoring(); leave();
  });
  const returnTest = command('return', 'workshop.test_return', () => session.test());
  const pause = command('pause', 'workshop.test_pause', () => session.control({ command: 'pause' }));
  const resume = command('resume', 'workshop.test_resume', () => session.control({ command: 'resume' }));
  const step = command('step', 'workshop.test_step', () => session.control({ command: 'step' }));
  const stop = command('stop', 'workshop.test_stop', async () => {
    await session.stop(); leave();
  });
  const speed = make('select', 'speed');
  for (const value of [1, 2, 4, 8]) {
    const option = doc.createElement('option'); option.value = String(value); option.textContent = `${value}×`; speed.append(option);
  }
  speed.addEventListener('change', () => { void session.control({ command: 'rate', multiplier: Number(speed.value) }).catch(() => {}); });
  // Which observer this run is drawn for. A switch is not a restart: the same
  // simulation keeps running at the same tick while the surface changes.
  const view = make('select', 'view');
  view.addEventListener('change', () => {
    const value = view.value;
    const control = value === 'game-master'
      ? { command: 'view', view: { view: 'game-master' } }
      : { command: 'view', view: { view: 'ship', entity: value || null } };
    void session.control(control).catch(() => {});
  });
  panel.append(make('h2', 'heading', 'workshop.test_heading'), make('p', 'scope', 'workshop.test_scope'));
  const label = (target, id) => { const node = doc.createElement('label'); node.htmlFor = target.id; node.textContent = t(id); panel.append(node, target); };
  label(world, 'workshop.test_world'); label(ship, 'workshop.test_ship'); label(seed, 'workshop.test_seed');
  const breakpointFields = doc.createElement('fieldset'); breakpointFields.className = 'workshop-test-breakpoint';
  const breakpointLegend = doc.createElement('legend'); breakpointLegend.textContent = t('workshop.test_breakpoint');
  breakpointFields.append(breakpointLegend);
  const breakpointField = (target, id) => { const node = doc.createElement('label'); node.htmlFor = target.id;
    node.textContent = t(id); breakpointFields.append(node, target); };
  breakpointField(breakpointEnabled, 'workshop.test_breakpoint_enabled');
  breakpointField(breakpointKind, 'workshop.test_breakpoint_kind');
  breakpointField(breakpointName, 'workshop.test_breakpoint_name');
  breakpointField(breakpointLayer, 'workshop.test_breakpoint_layer');
  breakpointField(breakpointFlagValue, 'workshop.test_breakpoint_expected');
  breakpointField(breakpointComparison, 'workshop.test_breakpoint_comparison');
  breakpointField(breakpointValue, 'workshop.test_breakpoint_value');
  panel.append(breakpointFields);
  panel.append(start, returnTest, authoring, pause, resume, step);
  label(speed, 'workshop.test_speed'); label(view, 'workshop.test_view');
  panel.append(stop, status); root.append(panel);
  if (provider.test.mount) {
    const viewport = doc.createElement('div'); viewport.className = 'workshop-test-viewport';
    viewPanel.append(viewport); provider.test.mount(viewport, t('workshop.test_heading'));
  } else {
    const unavailable = doc.createElement('p');
    unavailable.textContent = t('workshop.test_scope');
    viewPanel.append(unavailable);
  }
  root.append(viewPanel);
  const traceHeading = make('h2', 'trace-heading', 'workshop.test_trace');
  const traceScope = make('p', 'trace-scope', 'workshop.test_trace_scope');
  const traceFilter = make('select', 'trace-filter');
  for (const [value, id] of [
    ['all', 'workshop.test_trace_all'], ['host-call', 'workshop.test_trace_calls'],
    ['flag-mutation', 'workshop.test_trace_flags'], ['callback', 'workshop.test_trace_callbacks'],
  ]) {
    const option = doc.createElement('option'); option.value = value; option.textContent = t(id); traceFilter.append(option);
  }
  const traceLabel = doc.createElement('label'); traceLabel.htmlFor = traceFilter.id;
  traceLabel.textContent = t('workshop.test_trace_filter');
  const traceStatus = make('p', 'trace-status'); traceStatus.setAttribute('role', 'status');
  const traceList = make('ol', 'trace-list');
  traceList.setAttribute('aria-label', t('workshop.test_trace'));
  tracePanel.append(traceHeading, traceScope, traceLabel, traceFilter, traceStatus, traceList);
  const breakpointHit = make('section', 'breakpoint-hit'); breakpointHit.setAttribute('aria-live', 'polite');
  const breakpointHitHeading = doc.createElement('h3'); breakpointHitHeading.textContent = t('workshop.test_breakpoint_hit');
  const breakpointHitCondition = make('p', 'breakpoint-hit-condition');
  const breakpointHitSource = make('button', 'breakpoint-hit-source'); breakpointHitSource.type = 'button';
  const breakpointHitTrace = make('ol', 'breakpoint-hit-trace');
  breakpointHit.append(breakpointHitHeading, breakpointHitCondition, breakpointHitSource, breakpointHitTrace);
  tracePanel.append(breakpointHit);
  root.append(tracePanel);
  let traceSignature = '';
  const callbackKind = kind => kind === 'callback-scheduled' || kind === 'callback-fired';
  const traceEventText = record => {
    if (record.kind === 'host-call') return t('workshop.test_trace_call', { function: record.function });
    if (record.kind === 'flag-mutation') return t('workshop.test_trace_flag', {
      name: record.name, before: String(record.before), after: String(record.after),
      scope: record.layer ? t('workshop.test_trace_layer', { layer: record.layer }) : t('workshop.test_trace_root'),
    });
    if (record.kind === 'callback-scheduled') return t('workshop.test_trace_scheduled', {
      function: record.function, tick: String(record.fire_tick),
    });
    return t('workshop.test_trace_fired', { function: record.function, tick: String(record.scheduled_tick) });
  };
  function paintTrace(run) {
    const records = Array.isArray(run?.trace) ? run.trace : [];
    const filter = traceFilter.value;
    const visible = records.filter(record => filter === 'all' || record.kind === filter
      || (filter === 'callback' && callbackKind(record.kind)));
    const signature = JSON.stringify([filter, visible]);
    if (signature === traceSignature) return;
    traceSignature = signature;
    traceList.replaceChildren(...visible.map(record => {
      const row = doc.createElement('li'); row.dataset.kind = record.kind;
      const identity = doc.createElement('span'); identity.className = 'workshop-test-trace-identity';
      identity.textContent = t('workshop.test_trace_identity', { tick: String(record.tick), order: String(record.order) });
      const event = doc.createElement('span'); event.className = 'workshop-test-trace-event';
      event.textContent = traceEventText(record);
      const location = `${record.source?.path || ''}${record.source?.line ? `:${record.source.line}` : ''}`;
      const source = doc.createElement('button'); source.type = 'button'; source.className = 'workshop-test-trace-source';
      source.textContent = location || t('workshop.test_trace_source_unavailable');
      source.disabled = !record.source?.path;
      source.addEventListener('click', () => {
        void session.authoring()
          .then(() => openSource(record.source.path, record.source.line))
          .catch(error => { localError = error; render(); });
      });
      row.append(identity, doc.createTextNode(' '), event, doc.createTextNode(' '), source);
      return row;
    }));
    traceStatus.textContent = t('workshop.test_trace_count', { shown: String(visible.length), total: String(records.length) });
  }
  function paintBreakpoint(run) {
    const hit = run?.breakpoint_hit;
    breakpointHit.hidden = !hit;
    if (!hit) { breakpointHitTrace.replaceChildren(); return; }
    const condition = hit.breakpoint.condition;
    const expected = condition.kind === 'flag' ? String(condition.value)
      : `${condition.comparison} ${condition.value}`;
    breakpointHitCondition.textContent = t('workshop.test_breakpoint_hit_detail', {
      name: condition.name, expected, current: String(hit.current), tick: String(hit.tick),
      scope: hit.breakpoint.layer || t('workshop.test_trace_root'),
    });
    const location = `${hit.source?.path || ''}${hit.source?.line ? `:${hit.source.line}` : ''}`;
    breakpointHitSource.textContent = location || t('workshop.test_trace_source_unavailable');
    breakpointHitSource.disabled = !hit.source?.path;
    breakpointHitSource.onclick = () => { if (hit.source?.path) void session.authoring()
      .then(() => openSource(hit.source.path, hit.source.line)).catch(error => { localError = error; render(); }); };
    breakpointHitTrace.replaceChildren(...(hit.adjacent_trace || []).map(record => {
      const row = doc.createElement('li');
      row.textContent = `${t('workshop.test_trace_identity', {
        tick: String(record.tick), order: String(record.order),
      })} ${traceEventText(record)}`;
      return row;
    }));
  }
  traceFilter.addEventListener('change', () => { traceSignature = ''; paintTrace(session?.state().run); });
  /** Offer the omniscient desk and every simulated player ship this run has.
   *
   * Rebuilt only when the offer actually changes, so a selector the operator is
   * working does not reshuffle under them each time the status polls. */
  let offered = '';
  function paintViews(run) {
    const ships = Array.isArray(run?.ships) ? run.ships : [];
    const signature = JSON.stringify(ships);
    if (signature !== offered) {
      offered = signature;
      const options = [
        ['game-master', t('workshop.test_view_gm')],
        // `''` is the ship the Test launched with, which is what Start opens
        // on and the only entry a single-ship run needs.
        ['', t('workshop.test_view_launched')],
        ...ships.map(ship => [ship.entity, ship.name]),
      ];
      view.replaceChildren(...options.map(([value, text]) => {
        const option = doc.createElement('option');
        option.value = value; option.textContent = text; return option;
      }));
    }
    const current = run?.view?.view === 'game-master' ? 'game-master' : (run?.view?.entity || '');
    if (doc.activeElement !== view) view.value = current;
  }

  function render() {
    if (!session) return;
    const state = session.state(), draft = getDraft();
    const busy = isBusy() || state.busy;
    const testing = state.mode === 'test';
    const running = !!state.run;
    panel.setAttribute('aria-busy', String(busy));
    start.disabled = busy || catalogPending || !draft || !validSelection();
    start.textContent = t(running ? 'workshop.test_restart' : 'workshop.test_start');
    world.disabled = ship.disabled = seed.disabled = busy || testing;
    breakpointEnabled.disabled = breakpointKind.disabled = breakpointName.disabled = breakpointLayer.disabled = busy || testing;
    breakpointFlagValue.disabled = breakpointComparison.disabled = breakpointValue.disabled = busy || testing;
    const counter = breakpointKind.value === 'counter';
    breakpointFlagValue.hidden = breakpointFlagValue.previousElementSibling.hidden = counter;
    breakpointComparison.hidden = breakpointComparison.previousElementSibling.hidden = !counter;
    breakpointValue.hidden = breakpointValue.previousElementSibling.hidden = !counter;
    authoring.hidden = false; authoring.disabled = busy;
    returnTest.hidden = testing || !running; returnTest.disabled = busy;
    pause.disabled = busy || !testing || state.run?.paused || state.run?.starting;
    resume.disabled = busy || !testing || !state.run?.paused || state.run?.starting;
    step.disabled = resume.disabled;
    speed.disabled = busy || !testing || state.run?.starting;
    view.disabled = speed.disabled;
    paintViews(state.run);
    paintTrace(state.run);
    paintBreakpoint(state.run);
    // A browser boot can wait on large captured assets or a GPU. Its explicit
    // cancellation retires an in-flight frame before the serialized Stop.
    const cancellable = state.busy && typeof provider.test.cancelStart === 'function';
    stop.disabled = !cancellable && (busy || !running);
    speed.value = String(state.run?.multiplier || 1);
    const lines = [t(testing ? 'workshop.test_mode' : 'workshop.test_authoring_mode')];
    if (state.starting || state.run?.starting) lines.push(t('workshop.test_starting'));
    else if (running) lines.push(t(state.run.paused ? 'workshop.test_held' : 'workshop.test_running', { tick: String(state.run.tick) }));
    if (state.stale) lines.push(t('workshop.test_stale'));
    if (choicesReady && !catalogPending && !validSelection()) lines.push(t('workshop.test_selection_invalid'));
    const error = localError || state.error;
    if (error) lines.push(String(error?.message || error));
    for (const finding of error?.report?.findings || []) lines.push(`${finding.file}${finding.line ? `:${finding.line}` : ''}: ${finding.message}`);
    status.textContent = lines.join(' '); status.setAttribute('role', error ? 'alert' : 'status');
  }
  session = createWorkshopTest({ provider: provider.test,
    snapshot: () => provider.test.capture ? provider.test.capture(getDraft()) : getDraft().toNativeSources(),
    onChange(state) { render(); changed(state); } });
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
        const files = Object.fromEntries(draft.paths().filter(path => !draft.isBinary(path) && /\.(toml|rhai)$/.test(path))
          .map(path => [path, draft.read(path)]));
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
          paintBreakpointLayers();
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
  return { node: panel, viewNode: viewPanel, traceNode: tracePanel, refresh, enter: () => session.test(),
    held: () => session.state().busy || session.state().mode === 'test', testing: () => session.state().mode === 'test',
    dispose() { disposed = true; win.clearInterval(timer); if (catalogTimer !== null) win.clearTimeout(catalogTimer);
      void session.dispose().catch(() => {}); panel.remove(); viewPanel.remove(); tracePanel.remove(); } };
}
