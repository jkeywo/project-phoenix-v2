import { createFleetMember, createFleetOwner } from './fleet-session.js';
import { HOST_ROLE_GM, HOST_ROLE_SHIP_GM } from './host-mesh.js';

/**
 * Native host-mesh control plane. The embedded surface owns the network link;
 * every simulation fact crosses the existing native lobby bridge, so this is
 * one technical peer controlling the Rust process rather than a browser sim.
 */
export function createNativeFleetPeer({
  send, log = console.log, createOwner = createFleetOwner, createMember = createFleetMember,
} = {}) {
  let handle = null;
  let configured = false;
  let socket = null;
  let nextRosterGeneration = 1;
  const pendingRosters = new Map();
  let config = null;

  const bridgeSocket = () => {
    const value = {
      readyState: 1,
      bufferedAmount: 0,
      send(frame) { send({ kind: 'fleet_wire_send', frame }); },
      close() {
        if (value.readyState === 3) return;
        value.readyState = 3;
        value.onclose?.();
      },
      onopen: null,
      onmessage: null,
      onerror: null,
      onclose: null,
    };
    socket = value;
    queueMicrotask(() => value.readyState === 1 && value.onopen?.());
    return value;
  };

  const emit = (record) => send(record);
  const authenticateFrame = (raw, slot) => {
    try {
      const frame = JSON.parse(raw);
      return Number(frame?.d?.from) === Number(slot);
    } catch (_) {
      return false;
    }
  };
  const sameStamp = (left, right) => {
    try {
      const a = typeof left === 'string' ? JSON.parse(left) : left;
      const b = typeof right === 'string' ? JSON.parse(right) : right;
      return a?.protocol === b?.protocol
        && a?.content_id === b?.content_id
        && a?.content_epoch === b?.content_epoch;
    } catch (_) {
      return false;
    }
  };

  return {
    configure(raw) {
      if (configured) return false;
      try { config = typeof raw === 'string' ? JSON.parse(raw) : raw; } catch (_) { return false; }
      if (!config || typeof config.base !== 'string' || !config.base) return false;
      const credentials = Array.isArray(config.credentials) ? [...config.credentials] : [];
      if (!credentials.length) return false;
      configured = true;
      // Older/native-owner configurations predate the explicit mode field;
      // absence therefore retains the shipped owner behaviour.
      if (config.owner === false) return true;
      handle = createOwner({
        base: config.base,
        iceServers: [],
        transports: ['ws-relay'],
        factories: {
          socket: bridgeSocket,
          peer: () => { throw new Error('native fleet is relay-only'); },
        },
        maxSlots: config.max_slots,
        maxNameLength: config.max_name_length,
        maxShipPathLength: config.max_ship_path_length,
        role: HOST_ROLE_SHIP_GM,
        ownerOperatorId: config.operator_id,
        credentialFactory: () => {
          const credential = credentials.shift();
          if (!credential) throw new Error('native fleet GM credential pool is exhausted');
          return credential;
        },
        name: config.gm_name || 'GM',
        ship: { template_path: config.ship_path || null, name: config.ship_name || '' },
        checkStamp: (stamp) => sameStamp(stamp, config.stamp)
          ? { ok: true }
          : { ok: false, code: 'version-mismatch' },
        authenticateFrame,
        onCode: (code) => emit({ kind: 'fleet_code', code: code.full, suffix: code.suffix }),
        onSimulationRoster: (roster) => {
          const generation = nextRosterGeneration++;
          emit({ kind: 'fleet_roster', generation, roster: JSON.stringify(roster) });
          return new Promise(resolve => pendingRosters.set(generation, resolve));
        },
        onSimulationFrame: (frame, authenticated_slot) => emit({
          kind: 'fleet_frame', frame, authenticated_slot,
        }),
        onStartGrant: (grant) => emit({ kind: 'fleet_start_grant', grant }),
        onHostLost: (slot) => emit({ kind: 'fleet_host_lost', slot }),
        onSlotClaimed: (slot) => emit({ kind: 'fleet_slot_claimed', slot }),
        onError: (reason, detail) => emit({ kind: 'fleet_fault', reason, detail: detail || '' }),
        onLog: log,
      });
      return true;
    },

    join(code, data, reconnect = null) {
      if (!configured || !config || handle) return false;
      handle = createMember({
        base: config.base,
        code,
        data,
        stamp: config.stamp,
        iceServers: [],
        transports: ['ws-relay'],
        factories: {
          socket: bridgeSocket,
          peer: () => { throw new Error('native fleet is relay-only'); },
        },
        role: HOST_ROLE_GM,
        name: config.gm_name || 'GM',
        reconnectCredential: reconnect && reconnect.reconnectCredential,
        claim: reconnect && reconnect.claim,
        // createFleetMember can announce identity synchronously while its
        // factory call is still assigning `handle`. Defer one microtask so the
        // persisted reconnect claim contains the admitted mesh slot as well as
        // the private capability.
        onIdentity: identity => queueMicrotask(() => emit({
          kind: 'fleet_identity',
          identity: { ...identity, claim: handle?.slot || null },
        })),
        onSimulationRoster: (roster) => {
          const generation = nextRosterGeneration++;
          emit({ kind: 'fleet_roster', generation, roster: JSON.stringify(roster) });
          return new Promise(resolve => pendingRosters.set(generation, resolve));
        },
        onGmJoinBootstrap: (id, roster) => {
          const generation = nextRosterGeneration++;
          emit({ kind: 'fleet_gm_bootstrap', id, generation, roster: JSON.stringify(roster) });
          return new Promise(resolve => pendingRosters.set(generation, resolve));
        },
        onSimulationFrame: (frame, authenticated_slot) => emit({
          kind: 'fleet_frame', frame, authenticated_slot,
        }),
        onStartPolicy: policy => emit({ kind: 'fleet_start_policy', policy }),
        onForceResult: result => emit({ kind: 'fleet_force_result', result }),
        onGmJoinPending: ({ request }) => emit({ kind: 'fleet_gm_join_pending', request }),
        onGmJoinStatus: status => emit({ kind: 'fleet_gm_join_status', status }),
        onStatus: status => emit({ kind: 'fleet_join_status', status }),
        onError: (reason, detail) => emit({ kind: 'fleet_fault', reason, detail: detail || '' }),
        onLog: log,
      });
      return true;
    },

    update(raw) {
      if (!handle) return false;
      let state;
      try { state = typeof raw === 'string' ? JSON.parse(raw) : raw; } catch (_) { return false; }
      handle.update({ ship: state.ship, ready: !!state.ship_ready });
      handle.setCrewReadiness(state.crew || {}, state.station_ratings || []);
      handle.setGmReady(!!state.gm_ready);
      handle.setStartValidation(!!state.validation);
      if (state.roster_result) {
        const resolve = pendingRosters.get(state.roster_result.generation);
        if (resolve) {
          pendingRosters.delete(state.roster_result.generation);
          resolve(state.roster_result.accepted
            ? true : (state.roster_result.reason || 'fleet-adoption-refused'));
        }
      }
      for (const frame of state.frames || []) handle.broadcast(frame);
      if (state.force_start) handle.forceStart?.();
      return true;
    },

    close() {
      if (handle) handle.close();
      handle = null;
      configured = false;
    },

    receive(frame) {
      if (socket?.readyState === 1) socket.onmessage?.({ data: frame });
    },

    get role() { return handle?.role || HOST_ROLE_SHIP_GM; },
  };
}
