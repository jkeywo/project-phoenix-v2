/**
 * gui/action-map.js — Table-driven dispatch for `console_action` postMessages.
 *
 * Each entry in ACTION_MAP handles one `action.action` value forwarded from an
 * iframe console.  Handlers receive (action, send, mutate) where:
 *
 *   send(type, data?)  — enqueues a ClientMessage to the server
 *   mutate(patch)      — applies a partial update to the local client state
 *                        (currently only needed for `set_sensors_target`)
 *
 * All functions are pure (no side effects, no DOM dependency) so they can be
 * unit-tested in Node via Vitest.
 *
 * Exposed on window as:
 *   window.dispatchConsoleAction(action, send, mutate)
 *   window.ACTION_MAP  (for inspection / extension)
 */

export const ACTION_MAP = Object.freeze({
  /** Fire a specific phaser bank. */
  fire_phaser: (a, send) => {
    if (a.bank) send('FirePhaser', { bank: a.bank });
  },

  /** Fire a specific blaster bank (issue #631). */
  fire_blaster: (a, send) => {
    if (!a.bank) return;
    send('ControlSystem', {
      target: `blaster-${a.bank}`,
      payload: { type: 'FireBlaster' },
    });
  },

  /**
   * Begin the charge phase for a hold-to-fire blaster bank (issue #636).
   *
   * When `charge_time_secs == 0` (instant-fire banks) this behaves identically
   * to `fire_blaster`. When `charge_time_secs > 0` the bank enters a charge
   * phase and fires automatically when the charge completes.
   */
  charge_blaster_start: (a, send) => {
    if (!a.bank) return;
    send('ControlSystem', {
      target: `blaster-${a.bank}`,
      payload: { type: 'ChargeBlasterStart' },
    });
  },

  /**
   * Cancel an in-progress charge phase (issue #636).
   *
   * Resets charge progress to 0 with no cooldown and no ammo consumed.
   * Safe to send even when the bank is not currently charging (no-op on the
   * server).
   */
  charge_blaster_cancel: (a, send) => {
    if (!a.bank) return;
    send('ControlSystem', {
      target: `blaster-${a.bank}`,
      payload: { type: 'ChargeBlasterCancel' },
    });
  },

  /** Fire a torpedo from a tube, optionally targeting a UUID. */
  fire_torpedo: (a, send) => {
    send('FireTorpedo', { tube: a.tube || 'fore', target_uuid: a.target_uuid || null });
  },

  /** Begin loading a torpedo tube. */
  load_tube: (a, send) => {
    if (a.tube) send('LoadTube', { tube: a.tube });
  },

  /** Unload (or cancel loading of) a torpedo tube. */
  unload_tube: (a, send) => {
    if (a.tube) send('UnloadTube', { tube: a.tube });
  },

  /** Set the volley target count for a torpedo tube (issue #632).
   *  Sends a ControlSystem message to the tube's fine SystemId with
   *  payload type SetTorpedoVolleyTarget. */
  set_torpedo_volley_target: (a, send) => {
    if (a.tube == null || a.count == null) return;
    const sysId = 'torpedo-tube-' + String(a.tube).replace(/_/g, '-');
    send('ControlSystem', {
      target: sysId,
      payload: { type: 'SetTorpedoVolleyTarget', data: { count: a.count } },
    });
  },

  /** Lock the weapon / sensor target to a specific entity UUID. */
  set_target: (a, send) => {
    if (a.uuid)
      send('ControlSystem', {
        target: 'tactical',
        payload: { type: 'SetTarget', data: { uuid: a.uuid } },
      });
  },

  /** Switch phaser firing mode (Auto / Manual / etc.). */
  set_phaser_mode: (a, send) => {
    if (a.mode)
      send('ControlSystem', {
        target: 'tactical',
        payload: { type: 'SetPhaserMode', data: { mode: a.mode } },
      });
  },

  /** Switch the view-screen to a named camera marker or non-camera mode. */
  set_view: (a, send) => {
    if (!a.direction) return;
    var mode;
    if (['Fore', 'Port', 'Starboard', 'Aft'].includes(a.direction) || a.direction.startsWith('camera_')) {
      mode = { kind: 'Camera', data: a.direction };
    } else {
      mode = { kind: a.direction };
    }
    send('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode } },
    });
  },

  /** Toggle red alert status. */
  toggle_red_alert: (a, send) => {
    send('ControlSystem', {
      target: 'red-alert',
      payload: { type: 'ToggleRedAlert' },
    });
  },

  /** Send helm thrust / steering inputs. */
  helm_input: (a, send) => {
    send('ControlSystem', {
      target: 'helm',
      payload: {
        type: 'HelmInput',
        data: { thrust: a.thrust || 0, steering: a.steering || 0 },
      },
    });
  },

  /** Set helm via analog joystick (ph-helm-joystick component). */
  set_helm: (a, send) => {
    send('ControlSystem', {
      target: 'helm',
      payload: {
        type: 'HelmInput',
        data: { thrust: a.thrust || 0, steering: a.yaw || 0 },
      },
    });
  },

  /** Begin charging the impulse drive. */
  start_impulse_charge: (a, send) => {
    send('ControlSystem', {
      target: 'helm',
      payload: { type: 'StartImpulseCharge' },
    });
  },

  /** Cancel an active impulse charge. */
  cancel_impulse: (a, send) => {
    send('ControlSystem', {
      target: 'helm',
      payload: { type: 'CancelImpulse' },
    });
  },

  /** Toggle the boost drive on/off. */
  toggle_boost: (a, send) => {
    send('ControlSystem', {
      target: 'helm',
      payload: { type: 'ToggleBoost' },
    });
  },

  /** Explicitly set boost on or off (hold-to-boost). */
  set_boost: (a, send) => {
    send('ControlSystem', {
      target: 'helm',
      payload: {
        type: 'SetBoost',
        data: { active: !!a.active },
      },
    });
  },

  /** Switch the view-screen to the radar mode. */
  set_radar_view: (a, send) => {
    send('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'Radar' } } },
    });
  },

  /**
   * Dispatch a repair team to a system on the ship.
   *
   * Post issue #618: sends via the `ControlSystem` envelope targeting the
   * `repair` system. `a.target` is a lowercase station id (e.g. `'helm'`,
   * `'power'`); the `'core'` bucket for ownerless ship-wide systems maps to
   * `RepairTarget::Core` and is sent as `{ type: 'Core' }`.
   */
  dispatch_repair_team: (a, send) => {
    const target = a.target === 'core'
      ? { type: 'Core' }
      : { type: 'Station', data: a.target };
    send('ControlSystem', {
      target: 'repair',
      payload: {
        type: 'DispatchRepairTeam',
        data: { team_idx: a.team_idx, target },
      },
    });
  },

  /**
   * Set power allocation to an explicit level for a power group.
   *
   * The power panel sends the pre-calculated target level (current ± 1),
   * so the server receives a single absolute value and applies it via
   * PowerSystem::increase / decrease.
   *
   * `{ action: "set_power", console: "Power", target: "helm", level: 3 }`
   *
   * Wire target is `'power-reactor'` (issue #513): the reactor fine system
   * owns the allocation surface, and `handle_power_messages` reads only
   * `AdmittedCommands.for_target("power-reactor")`. Do NOT change back to
   * `'power'` — the coarse system id is retained only for the aggregate
   * blackboard, not for control input.
   */
  set_power: (a, send) => {
    if (a.target && typeof a.level === 'number') {
      send('ControlSystem', {
        target: 'power-reactor',
        payload: { type: 'SetPowerGroupAllocation', data: { group: a.target, level: a.level } },
      });
    }
  },

  /**
   * Focus shields on a specific arc (issue #514).
   *
   * Each arc has its own `SystemId("shield-arc-<arc_id>")` and receives a
   * `SetShieldArcFocus { focused: bool }` payload. The GUI already knows
   * which arc was clicked; pass its lowercase id in `a.arc_id`. `a.focused`
   * defaults to `true` (button press = "focus this arc"); pass `false` to
   * clear focus on the currently-focused arc.
   *
   * Wire target is `shield-arc-<arc_id>` (issue #514): the coarse
   * `shields` string is retained only for the aggregate blackboard, not
   * for control input. Do NOT change back to `'shields'` — the codec
   * routing test pins the wire shape.
   */
  set_shield_focus: (a, send) => {
    if (!a.arc_id) return;
    const focused = a.focused === undefined ? true : !!a.focused;
    send('ControlSystem', {
      target: `shield-arc-${a.arc_id}`,
      payload: { type: 'SetShieldArcFocus', data: { focused } },
    });
  },

  /** Switch the view-screen to navigation chart mode. */
  set_navigation_chart: (a, send) => {
    send('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'NavigationChart' } } },
    });
  },

  /**
   * Set the shared custom navigation waypoint.
   *
   * When `source_uuid` is provided (non-empty string), the waypoint is
   * anchored to that entity: the server overwrites x/z from the entity's
   * live transform every tick and auto-clears the waypoint on despawn.
   * Without `source_uuid`, the waypoint is a free position from
   * tap-to-place.
   */
  set_navigation_waypoint: (a, send) => {
    if (Number.isFinite(a.x) && Number.isFinite(a.z)) {
      const payload = { x: a.x, z: a.z };
      if (typeof a.source_uuid === 'string' && a.source_uuid.length > 0) {
        payload.source_uuid = a.source_uuid;
      }
      send('SetNavigationWaypoint', payload);
    }
  },

  /** Clear the shared custom navigation waypoint. */
  clear_navigation_waypoint: (a, send) => {
    send('ClearNavigationWaypoint');
  },

  /**
   * Select a science target.  Mutates local `state.sensorsTarget` so the
   * sensor display updates before the server acks the message.
   */
  set_sensors_target: (a, send, mutate) => {
    if (a.uuid) {
      mutate({ sensorsTarget: a.uuid });
      send('ControlSystem', { target: 'sensors', payload: { type: 'SetScienceTarget', data: { uuid: a.uuid } } });
    }
  },

  /** Open a comms channel to a contact by UUID. */
  hail: (a, send) => {
    if (a.target_uuid) send('Hail', { target_uuid: a.target_uuid });
  },

  /** Mark a comms message as selected / read. */
  select_comms_message: (a, send) => {
    if (a.message_id) send('SelectCommsMessage', { message_id: a.message_id });
  },

  /** Send a pre-written response to a comms message. */
  respond_to_message: (a, send) => {
    if (a.message_id) send('RespondToMessage', { message_id: a.message_id, response_index: a.response_index });
  },

  /** Clear all read/acknowledged comms messages from the inbox. */
  clear_comms: (a, send) => {
    send('ClearComms');
  },

  /** Send the selected comms message to the view screen. */
  show_on_screen: (a, send) => {
    if (a.message_id) send('ShowOnScreen', { message_id: a.message_id });
  },
});

/**
 * Dispatch a parsed console action to its handler.
 *
 * @param {{ action: string }} action  Parsed action payload from an iframe
 * @param {function}           send    fn(type, data?) — queues a ClientMessage
 * @param {function}           [mutate] fn(patch) — merges patch into local state
 */
export function dispatchConsoleAction(action, send, mutate) {
  if (!action || !action.action) return;
  const handler = ACTION_MAP[action.action];
  if (handler) handler(action, send, mutate || (() => {}));
}

// Expose for non-module inline scripts in client.html.
if (typeof window !== 'undefined') {
  window.dispatchConsoleAction = dispatchConsoleAction;
  window.ACTION_MAP = ACTION_MAP;
}
