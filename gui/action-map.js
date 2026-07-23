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

import { dispatchRepairTeam, setRepairPriority } from './repair-dispatch.js';
import {
  sendHelmInput,
  startImpulseCharge,
  cancelImpulse,
  toggleBoost,
  setBoost,
} from './helm-dispatch.js';

export const ACTION_MAP = Object.freeze({
  /** Fire a specific phaser bank (issue #846: via ControlSystem envelope). */
  fire_phaser: (a, send) => {
    if (!a.bank) return;
    send('ControlSystem', {
      target: `phaser-${a.bank}`,
      payload: { type: 'FirePhaser' },
    });
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

  /** Fire a torpedo from a tube, optionally targeting a UUID (issue #846: via ControlSystem envelope). */
  fire_torpedo: (a, send) => {
    var tube = a.tube || 'fore';
    var sysId = 'torpedo-tube-' + String(tube).replace(/_/g, '-');
    send('ControlSystem', {
      target: sysId,
      payload: { type: 'FireTorpedo', data: { target_uuid: a.target_uuid || null } },
    });
  },

  /** Begin loading a torpedo tube (issue #846: via ControlSystem envelope). */
  load_tube: (a, send) => {
    if (!a.tube) return;
    var sysId = 'torpedo-tube-' + String(a.tube).replace(/_/g, '-');
    send('ControlSystem', {
      target: sysId,
      payload: { type: 'LoadTube' },
    });
  },

  /** Unload (or cancel loading of) a torpedo tube (issue #846: via ControlSystem envelope). */
  unload_tube: (a, send) => {
    if (!a.tube) return;
    var sysId = 'torpedo-tube-' + String(a.tube).replace(/_/g, '-');
    send('ControlSystem', {
      target: sysId,
      payload: { type: 'UnloadTube' },
    });
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

  /** Lock the weapon / sensor target to a specific entity UUID.
   *  Wire target is 'tactical-radar' (issue #801): target lock lives on the
   *  tactical radar fine system; the coarse 'tactical' id is a station id,
   *  not a wire target. */
  set_target: (a, send, mutate) => {
    if (a.uuid) {
      mutate({ weaponsTarget: a.uuid });
      send('ControlSystem', {
        target: 'tactical-radar',
        payload: { type: 'SetTarget', data: { uuid: a.uuid } },
      });
    }
  },

  /** Switch phaser firing mode (Auto / Manual / etc.).
   *  Wire target is 'phaser-control' (issue #801): the ship-wide phaser
   *  settings system. */
  set_phaser_mode: (a, send) => {
    if (a.mode)
      send('ControlSystem', {
        target: 'phaser-control',
        payload: { type: 'SetPhaserMode', data: { mode: a.mode } },
      });
  },

  /** Switch the view-screen to a named camera marker or non-camera mode. */
  set_view: (a, send) => {
    if (!a.direction) return;
    var mode;
    if (['Fore', 'Port', 'Starboard', 'Aft'].includes(a.direction) || a.direction.startsWith('camera_') || a.direction === 'cinematic') {
      mode = { kind: 'Camera', data: a.direction };
    } else {
      mode = { kind: a.direction };
    }
    send('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode } },
    });
  },

  /** Set the ship's Red Alert to an explicit desired state (issue #748).
   *  The button computes `active` = !currentDisplayedActive, so a stale,
   *  duplicated, or retried command is idempotent (the host assigns, it does
   *  not invert). */
  set_red_alert: (a, send) => {
    send('ControlSystem', {
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: !!a.active } },
    });
  },

  /** Toggle Captain priority boost on an objective (issue #675). */
  set_objective_priority: (a, send) => {
    if (!a.id) return;
    send('ControlSystem', {
      target: 'captain',
      payload: { type: 'SetObjectivePriority', data: { id: a.id } },
    });
  },

  /** Send helm thrust / steering inputs.
   *
   *  One joystick action fans out to the two per-axis wire messages (issue
   *  #801): SetThrust -> 'helm-thrust' and SetSteering -> 'helm-steering'.
   *  Admission gates each axis on its own declared system, so a ship with an
   *  AI-held throttle and a human-held stick admits exactly the axis the
   *  human owns. Component emitters (ph-helm-joystick) are unchanged.
   */
  helm_input: (a, send) => {
    sendHelmInput(a.thrust, a.steering, send);
  },

  /** Set helm via analog joystick (ph-helm-joystick component).
   *  Same per-axis fan-out as helm_input (issue #801); the joystick's yaw
   *  maps to the steering axis. */
  set_helm: (a, send) => {
    sendHelmInput(a.thrust, a.yaw, send);
  },

  /** Begin charging the impulse drive. Targets 'helm-impulse' (issue #801). */
  start_impulse_charge: (a, send) => {
    startImpulseCharge(send);
  },

  /** Cancel an active impulse charge. Targets 'helm-impulse' (issue #801). */
  cancel_impulse: (a, send) => {
    cancelImpulse(send);
  },

  /** Toggle the boost drive on/off. Targets 'helm-boost' (issue #801). */
  toggle_boost: (a, send) => {
    toggleBoost(send);
  },

  /** Explicitly set boost on or off (hold-to-boost). Targets 'helm-boost'. */
  set_boost: (a, send) => {
    setBoost(a.active, send);
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
    dispatchRepairTeam(a.team_idx, a.target, send);
  },

  /**
   * Set on-site repair priority for a team (issue #739). Only takes effect
   * when the team is in `Repairing` state. Requires a team_idx and priority.
   */
  set_repair_priority: (a, send) => {
    if (a.team_idx != null && a.priority != null) {
      setRepairPriority(a.team_idx, a.priority, send);
    }
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
      const data = { x: a.x, z: a.z };
      if (typeof a.source_uuid === 'string' && a.source_uuid.length > 0) {
        data.source_uuid = a.source_uuid;
      }
      send('ControlSystem', {
        target: 'navigation',
        payload: { type: 'SetNavigationWaypoint', data },
      });
    }
  },

  /** Clear the shared custom navigation waypoint. */
  clear_navigation_waypoint: (a, send) => {
    send('ControlSystem', {
      target: 'navigation',
      payload: { type: 'ClearNavigationWaypoint' },
    });
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
    if (a.target_uuid) {
      send('ControlSystem', {
        target: 'comms',
        payload: { type: 'Hail', data: { target_uuid: a.target_uuid } },
      });
    }
  },

  /** Mark a comms message as selected / read. */
  select_comms_message: (a, send) => {
    if (a.message_id) {
      send('ControlSystem', {
        target: 'comms',
        payload: { type: 'SelectCommsMessage', data: { message_id: a.message_id } },
      });
    }
  },

  /** Send a pre-written response to a comms message. */
  respond_to_message: (a, send) => {
    if (a.message_id) {
      send('ControlSystem', {
        target: 'comms',
        payload: {
          type: 'RespondToMessage',
          data: { message_id: a.message_id, response_index: a.response_index },
        },
      });
    }
  },

  /** Clear all read/acknowledged comms messages from the inbox. */
  clear_comms: (a, send) => {
    send('ControlSystem', {
      target: 'comms',
      payload: { type: 'ClearComms' },
    });
  },

  /** Send the selected comms message to the view screen. */
  show_on_screen: (a, send) => {
    if (a.message_id) {
      send('ControlSystem', {
        target: 'comms',
        payload: { type: 'ShowOnScreen', data: { message_id: a.message_id } },
      });
    }
  },

  /** Set the phaser frequency to an explicit value (0.0–1.0).
   *  Wire target is 'phaser-control' (issue #801). */
  set_phaser_frequency: (a, send) => {
    if (typeof a.frequency === 'number') {
      send('ControlSystem', {
        target: 'phaser-control',
        payload: { type: 'SetPhaserFrequency', data: { frequency: a.frequency } },
      });
    }
  },

  /** Set lateral thrust via analog joystick (ph-lateral-thrust-joystick component). */
  set_lateral_thrust: (a, send) => {
    send('ControlSystem', {
      target: 'helm-lateral-thrust',
      payload: {
        type: 'LateralThrustInput',
        data: { lateral: a.lateral || 0 },
      },
    });
  },

  /**
   * Return everyone to the Lobby from the GameOver screen (issue #822).
   *
   * Sent by the host page's game-over overlay and the phone client's
   * game-over overlay alike; the server only honours it during GameOver.
   */
  return_to_lobby: (a, send) => {
    send('ReturnToLobby');
  },

  /**
   * Propose a scenario in the QR-first pre-scenario flow (issue #755).
   *
   * Sent by BOTH the host page's own selection UI and connected phones over
   * the same action map. The host-runtime arbiter applies first-valid-wins
   * against the pre-load catalog; there is no token gate so server or phone
   * participants alike can make the first valid selection. After Game Over the
   * return re-enters this same flow for the second round (issue #756).
   */
  select_scenario: (a, send) => {
    if (!a.scenario_id) return;
    send('SelectScenario', { scenario_id: a.scenario_id });
  },

  /**
   * Propose a player ship in the QR-first pre-scenario flow (issue #755).
   *
   * Validated by the arbiter against the locked scenario's offered ships.
   */
  select_player_ship: (a, send) => {
    if (!a.template_path) return;
    send('SelectPlayerShip', { template_path: a.template_path });
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
