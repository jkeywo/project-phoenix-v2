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

  /** Fire a torpedo from a tube, optionally targeting a UUID. */
  fire_torpedo: (a, send) => {
    send('FireTorpedo', { tube: a.tube || 'fore', target_uuid: a.target_uuid || null });
  },

  /** Lock the weapon / sensor target to a specific entity UUID. */
  set_target: (a, send) => {
    if (a.uuid) send('SetTarget', { uuid: a.uuid });
  },

  /** Switch phaser firing mode (Auto / Manual / etc.). */
  set_phaser_mode: (a, send) => {
    if (a.mode) send('SetPhaserMode', { mode: a.mode });
  },

  /** Switch the view-screen to a named Camera direction. */
  set_view: (a, send) => {
    // ClientMessage::SetView { mode: ViewMode::Camera(direction) }
    // serialises as {"type":"SetView","data":{"mode":{"kind":"Camera","data":"Fore"}}}
    if (a.direction) send('SetView', { mode: { kind: 'Camera', data: a.direction } });
  },

  /** Toggle red alert status. */
  toggle_red_alert: (a, send) => {
    send('ToggleRedAlert');
  },

  /** Send helm thrust / steering inputs. */
  helm_input: (a, send) => {
    send('HelmInput', { thrust: a.thrust || 0, steering: a.steering || 0 });
  },

  /** Begin charging the impulse drive. */
  start_impulse_charge: (a, send) => {
    send('StartImpulseCharge');
  },

  /** Cancel an active impulse charge. */
  cancel_impulse: (a, send) => {
    send('CancelImpulse');
  },

  /** Switch the view-screen to the radar mode. */
  set_radar_view: (a, send) => {
    send('SetView', { mode: { kind: 'Radar' } });
  },

  /** Dispatch a repair team to a console. */
  dispatch_repair_team: (a, send) => {
    send('DispatchRepairTeam', { team_idx: a.team_idx, console: a.target });
  },

  /** Increase power allocation to a console. */
  increase_power: (a, send) => {
    if (a.target) send('IncreasePower', { console: a.target });
  },

  /** Decrease power allocation to a console. */
  decrease_power: (a, send) => {
    if (a.target) send('DecreasePower', { console: a.target });
  },

  /** Focus shields on a specific facing. */
  set_shield_focus: (a, send) => {
    send('SetShieldFocus', { facing: a.facing || null });
  },

  /**
   * Select a science target.  Mutates local `state.sensorsTarget` so the
   * sensor display updates before the server acks the message.
   */
  set_sensors_target: (a, send, mutate) => {
    if (a.uuid) {
      mutate({ sensorsTarget: a.uuid });
      send('SetScienceTarget', { uuid: a.uuid });
    }
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
