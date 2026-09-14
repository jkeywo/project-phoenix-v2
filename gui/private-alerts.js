import { commsPriority, effectiveThreadId, isLiveCommsMessage, COMMS_PRIORITY } from './comms-state.js';
import { playerFor, playerStation } from './client-router.js';
import { CHANGE_DOMAINS } from './reducer-result.js';
import { soughtSystemHosts } from './console-state.js';

const ATTENTION = new Set(['pending_comms', 'eligible_beat', 'idle_npc']);
const HEALTH = new Set(['station_disconnected', 'ship_peer_lost', 'operator_disconnected', 'recovery_failed']);
// A malformed/oversized presentation is silently baselined, never truncated in
// a way that could make an evicted current row sound new on the next frame.
const MAX_ROWS = 4096;
const generationOf = value => Number.isSafeInteger(value) && value >= 0 ? value : null;

/** Current comparison only. No alert objects, text, history or replay queue
 * survive their source projection. Every edge is consumed before output. */
function lane(emit) {
  let scope = null, generation = null, previous = null;
  return {
    reset() { scope = null; generation = null; previous = null; },
    sample({ key, stamp, rows, value, edge, allowed = () => true }) {
      const nextGeneration = generationOf(stamp);
      if (!key || nextGeneration === null || !Array.isArray(rows) || rows.length > MAX_ROWS) {
        this.reset(); return false;
      }
      if (scope === key && generation !== null && nextGeneration < generation) return false;
      const next = new Map(rows.filter(row => typeof row?.id === 'string' && row.id)
        .map(row => [row.id, value(row)]));
      let sound = false;
      if (scope === key && generation === nextGeneration && previous !== null) {
        sound = rows.some(row => edge(row, previous.get(row.id)) && allowed(row));
      }
      scope = key; generation = nextGeneration; previous = next;
      if (sound) emit();
      return sound;
    },
  };
}

/** Closed A6 mapping. Native adapters reuse this JS owner and inject the A5
 * private output; neither platform derives another gameplay alert list. */
export function createPrivateAlerts({ audio } = {}) {
  const emit = () => { try { audio?.actionable(); } catch (_) { /* Output never owns commands. */ } };
  const comms = lane(emit), attention = lane(emit), health = lane(emit);
  return {
    reset() { comms.reset(); attention.reset(); health.reset(); },
    comms({ key, generation, messages }) {
      const latest = new Map();
      for (const row of Array.isArray(messages) ? messages : []) latest.set(effectiveThreadId(row), row);
      return comms.sample({ key, stamp: generation, rows: messages,
        value: row => commsPriority(row),
        edge: (row, previous) => latest.get(effectiveThreadId(row)) === row && isLiveCommsMessage(row)
          && (commsPriority(row) === COMMS_PRIORITY.CRITICAL || (commsPriority(row) === COMMS_PRIORITY.URGENT && !row.is_read))
          && (previous === undefined || (previous === COMMS_PRIORITY.URGENT && commsPriority(row) === COMMS_PRIORITY.CRITICAL)),
      });
    },
    attention({ key, generation, occurrences, held = false, visible = () => true }) {
      return attention.sample({ key, stamp: generation, rows: occurrences,
        value: row => row.band,
        edge: (row, previous) => ATTENTION.has(row.category) && row.band === 'urgent' && previous !== 'urgent',
        allowed: row => !held && visible(row),
      });
    },
    health({ key, generation, alerts }) {
      return health.sample({ key, stamp: generation, rows: alerts,
        value: () => true,
        edge: (row, previous) => previous === undefined && HEALTH.has(row.kind) && row.severity === 'disconnected',
      });
    },
  };
}

/** Entitlement uses the actual fine System and its already permitted board.
 * A Comms family visible for Intel does not make that reader the human host. */
export function privateCommsSource(state, uiState, token, connected) {
  const player = playerFor(uiState, token), station = playerStation(uiState, token);
  if (!connected || uiState?.phase !== 'InProgress' || !station || !player?.connected
      || player.spectator || player.afk) return null;
  const hosts = soughtSystemHosts(state);
  const owned = state.stationSystems?.[station] || [];
  const system = Object.keys(state.systemKinds || {}).find(id => state.systemKinds[id] === 'comms'
    && state.blackboardKinds?.[id] === 'Comms' && state.controlSources?.[id] === 'Human'
    && (hosts[id] ? hosts[id] === station : owned.includes(id)));
  if (!system || !Array.isArray(state.blackboards?.[system]?.messages)) return null;
  return { key: JSON.stringify([token, station, system]), generation: state.blackboardPresentation?.[system],
    messages: state.blackboards[system].messages };
}

/** Parent Station adapter, after the ordinary reducers/router have folded the
 * payload. Tab selection and output availability are deliberately not inputs. */
export function createStationPrivateAlerts({ audio } = {}) {
  const alerts = createPrivateAlerts({ audio });
  return {
    reset: alerts.reset,
    update({ state, uiState, token, connected, changes }) {
      if (changes?.changedDomains?.has(CHANGE_DOMAINS.WELCOME) || changes?.changedDomains?.has(CHANGE_DOMAINS.ROUND)) alerts.reset();
      const source = privateCommsSource(state, uiState, token, connected);
      if (!source) { alerts.reset(); return false; }
      return alerts.comms(source);
    },
  };
}

/** A returned document starts from the next current sample, even when the
 * browser suspended network delivery while hidden. No queued alert is replayed. */
export function attachPrivateAlertLifecycle(alerts, win) {
  const reset = () => alerts.reset();
  win.document.addEventListener('visibilitychange', reset);
  win.addEventListener('pagehide', reset);
  win.addEventListener('pageshow', reset);
  return () => {
    win.document.removeEventListener('visibilitychange', reset);
    win.removeEventListener('pagehide', reset);
    win.removeEventListener('pageshow', reset);
    alerts.reset();
  };
}

if (typeof window !== 'undefined') window.PrivateAlerts = { createStationPrivateAlerts, attachPrivateAlertLifecycle };
