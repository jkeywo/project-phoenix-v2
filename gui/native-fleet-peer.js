import { createFleetMember, createFleetOwner } from './fleet-session.js';
import { transportLeversFromLocation } from './transport-levers.js';
import { HOST_ROLE_GM, HOST_ROLE_SHIP, HOST_ROLE_SHIP_GM } from './host-mesh.js';

/**
 * Native host-mesh control plane. The embedded surface owns the network link;
 * every simulation fact crosses the existing native lobby bridge, so this is
 * one technical peer controlling the Rust process rather than a browser sim.
 */
export function createNativeFleetPeer({
  send, log = console.log, createOwner = createFleetOwner, createMember = createFleetMember,
  onRoster = () => {}, onDiag = () => {}, onHealth = () => {},
} = {}) {
  let handle = null;
  let configured = false;
  let socket = null;
  let nextRosterGeneration = 1;
  const pendingRosters = new Map();
  let config = null;
  const publishedControl = new Map();
  let latestControl = null;
  const pendingWire = [];
  let wireReady = false;
  let wireFailed = false;

  const bridgeSocket = () => {
    wireReady = false;
    let messageHandler = null;
    const drainWire = () => {
      if (!wireReady || typeof messageHandler !== 'function') return;
      while (pendingWire.length && value.readyState === 1) {
        messageHandler({ data: pendingWire.shift() });
      }
    };
    const value = {
      readyState: wireFailed ? 3 : 1,
      bufferedAmount: 0,
      send(frame) { send({ kind: 'fleet_wire_send', frame }); },
      close() {
        if (value.readyState === 3) return;
        value.readyState = 3;
        value.onclose?.();
      },
      onopen: null,
      get onmessage() { return messageHandler; },
      set onmessage(handler) { messageHandler = handler; drainWire(); },
      onerror: null,
      onclose: null,
    };
    socket = value;
    queueMicrotask(() => {
      if (value.readyState !== 1) return;
      value.onopen?.();
      wireReady = true;
      drainWire();
    });
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
  // Both runtime producers use Rust's DeliveryStamp::to_field. Rust also
  // validates the native configured identity, so equality needs no second
  // parser and cannot admit two matching but unidentified content sets.
  const sameStamp = stamp => config.stamp_valid === true
    && typeof stamp === 'string' && stamp === config.stamp;

  const publishControl = state => {
    // Health ticks and simulation frames are frequent; unchanged lobby
    // control values must not fan out fresh roster frames on every tick.
    const changed = (key, value, publish) => {
      const json = JSON.stringify(value);
      if (publishedControl.get(key) === json) return;
      publish();
      publishedControl.set(key, json);
    };
    const ship = { ship: state.ship, ready: !!state.ship_ready };
    changed('ship', ship, () => handle.update(ship));
    const crew = state.crew || {}, ratings = state.station_ratings || [];
    changed('crew', [crew, ratings], () => handle.setCrewReadiness(crew, ratings));
    changed('gm', !!state.gm_ready, () => handle.setGmReady(!!state.gm_ready));
    changed('validation', !!state.validation, () => handle.setStartValidation(!!state.validation));
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
        name: config.ship_name || config.gm_name || 'GM',
        ship: { template_path: config.ship_path || null, name: config.ship_name || '' },
        checkStamp: (stamp) => sameStamp(stamp)
          ? { ok: true }
          : { ok: false, code: 'version-mismatch' },
        authenticateFrame,
        onCode: (code) => emit({ kind: 'fleet_code', code: code.full, suffix: code.suffix }),
        onRoster,
        onDiag,
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

    join(code, data, reconnect = null, role = HOST_ROLE_GM) {
      if (!configured || !config || handle) return false;
      if (role !== HOST_ROLE_GM && role !== HOST_ROLE_SHIP) return false;
      handle = createMember({
        base: config.base,
        code,
        data,
        stamp: config.stamp,
        iceServers: [],
        transports: ['ws-relay'],
        // Member joiners select transport through levers, including when a
        // browser owner also advertises WebRTC that native cannot provide.
        levers: transportLeversFromLocation('?transport=ws-relay'),
        factories: {
          socket: bridgeSocket,
          peer: () => { throw new Error('native fleet is relay-only'); },
        },
        role,
        onRoster,
        onDiag,
        ship: role === HOST_ROLE_SHIP
          ? { template_path: config.ship_path || null, name: config.ship_name || '' } : undefined,
        name: role === HOST_ROLE_SHIP ? config.ship_name || '' : config.gm_name || 'GM',
        reconnectCredential: reconnect && reconnect.reconnectCredential,
        claim: reconnect && reconnect.claim,
        onWelcome: () => {
          // An initial patch may precede transport admission. Republish the
          // current snapshot once after Welcome, even if Rust has no newer
          // update. The cached snapshot excludes frames and one-shot edges.
          publishedControl.clear();
          queueMicrotask(() => { if (handle && latestControl) publishControl(latestControl); });
          emit({ kind: 'fleet_join_status', status: 'admitted' });
        },
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
      if (state.health) onHealth(state.health);
      latestControl = {ship:state.ship,ship_ready:state.ship_ready,crew:state.crew,
        station_ratings:state.station_ratings,gm_ready:state.gm_ready,validation:state.validation};
      publishControl(latestControl);
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
      publishedControl.clear();
      latestControl = null;
      configured = false;
      pendingWire.length = 0;
      wireReady = false;
      wireFailed = false;
      socket = null;
    },

    receive(frame) {
      if (wireFailed) return;
      if (wireReady && socket?.readyState === 1 && typeof socket.onmessage === 'function') {
        socket.onmessage({ data: frame });
      } else if (pendingWire.length < 64) {
        pendingWire.push(frame);
      } else {
        wireFailed = true;
        pendingWire.length = 0;
        socket?.close();
        emit({ kind: 'fleet_fault', reason: 'pending-frame-overflow', detail: '' });
      }
    },

    get role() { return handle?.role || HOST_ROLE_SHIP_GM; },
  };
}
