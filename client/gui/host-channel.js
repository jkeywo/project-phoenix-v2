/**
 * gui/host-channel.js — the host page's Host Channel dispatcher (issue #1225,
 * extracted from server.html's formerly-inline `_hostChannelHandlers` table).
 *
 * One flush system in Rust (`flush_host_channels`) drains every host-page
 * channel and calls `window.__hostChannel(name, payload)` — installed
 * through the #1224 latch in server.html's classic prelude. `createHostChannel`
 * builds the function that latch hands off to: a handler table routes each
 * named channel (hud, lobby, chatter, audio_config, audio_cue, audio_level,
 * shake) to the page's existing render/audio functions. Adding a channel is
 * one Rust table row + one entry in the `handlers` object server.html passes
 * in.
 *
 * Payload shapes: JSON string for hud/lobby/chatter/audio_config/audio_cue;
 * a bare number for audio_level; a two-element [x, y] array for shake.
 *
 * JS must not assume any cross-channel ordering: even though a single flush
 * walks the channels in a fixed order, that order is an implementation
 * detail — e.g. the audio config push and the Loading → InProgress edge may
 * still land in either order within a frame (see server.html's
 * startGameAudio).
 *
 * ## Localisation boundary (issue #949)
 *
 * Every host-channel payload is built from authored DATA, so — exactly like a
 * peer message — it can carry string ids: a world's `[global] title` on the
 * lobby channel, a `game_over` trigger's `message` on the hud channel. A
 * phone crosses localiseTree() once, in gui/connection-manager.js, at the
 * point a peer message is decoded. This dispatcher is the host's equivalent
 * boundary — the single place every channel payload enters the page — so the
 * ids are resolved HERE rather than at each render site. Fixing it per render
 * site is what left `world.combat_test.global.title` on #lobby-title after
 * the scenario buttons were fixed (issue #949): two call sites found, and no
 * reason to think a third would not appear.
 *
 * Same rule as localiseTree: substitute only what the table actually holds.
 * Machine tokens (`phase`, station-rating names, the audio cue `kind`),
 * player-typed names and prose a mod pack authored literally all pass through
 * untouched — every id in strings.csv is dotted, so none of them can collide.
 *
 * Before the strings module has evaluated there is nothing to substitute
 * WITH, so the payload passes through unchanged, exactly as t() does on the
 * host page.
 *
 * This module is plain, I/O-free JS (no `window`, no DOM) so it imports
 * cleanly under vitest's node environment — tests/client/host-channel-
 * localisation.test.js exercises it directly rather than scraping server.html
 * as text.
 */

/**
 * Resolve string ids anywhere inside one host-channel payload.
 *
 * Channels whose payload is a JSON string (hud/lobby/chatter/audio_config/
 * audio_cue) are resolved inside the JSON and re-encoded, because each
 * handler owns its own parse. The numeric taps (audio_level's bare number,
 * shake's [x, y]) carry no text but still cross the same call, so the rule
 * holds for every channel without a per-channel list to keep in step.
 *
 * @param {*} payload the raw value Rust sent over the channel
 * @param {{localiseTree: function}|undefined|null} strings the page's string
 *   table API (gui/strings.js's `{ t, has, localiseTree }`), or undefined
 *   before it has loaded
 * @returns {*} the payload with every known string id resolved
 */
export function localiseHostPayload(payload, strings) {
  if (!strings || typeof strings.localiseTree !== 'function') return payload;
  if (typeof payload !== 'string') return strings.localiseTree(payload);
  try {
    return JSON.stringify(strings.localiseTree(JSON.parse(payload)));
  } catch (e) {
    // Not JSON: hand the handler exactly what Rust sent and let it report.
    return payload;
  }
}

/**
 * Build the Host Channel dispatcher: `(name, payload) => void`.
 *
 * @param {{
 *   handlers: Record<string, (localisedPayload: *) => void>,
 *   strings?: {localiseTree: function}|null,
 * }} opts
 * @returns {(name: string, payload: *) => void}
 */
export function createHostChannel({ handlers, strings }) {
  return function hostChannelDispatch(name, payload) {
    const handler = handlers[name];
    if (handler) {
      handler(localiseHostPayload(payload, strings));
    } else {
      console.warn('[Phoenix] unhandled host channel:', name);
    }
  };
}
