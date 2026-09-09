export class BrowserConnections {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        BrowserConnectionsFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_browserconnections_free(ptr, 0);
    }
    /**
     * Local FFI result, not a game message. The adapter sends an ordinary
     * JoinRefused for a refusal and closes `previous` only after this returns.
     * @param {string} handle
     * @param {string} token
     * @returns {string}
     */
    bind(handle, token) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(handle, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(token, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.browserconnections_bind(this.__wbg_ptr, ptr0, len0, ptr1, len1);
            deferred3_0 = ret[0];
            deferred3_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * @param {string} handle
     * @returns {string | undefined}
     */
    close(handle) {
        const ptr0 = passStringToWasm0(handle, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.browserconnections_close(this.__wbg_ptr, ptr0, len0);
        let v2;
        if (ret[0] !== 0) {
            v2 = getStringFromWasm0(ret[0], ret[1]).slice();
            wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        }
        return v2;
    }
    constructor() {
        const ret = wasm.browserconnections_new();
        this.__wbg_ptr = ret;
        BrowserConnectionsFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * @returns {string}
     */
    open() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.browserconnections_open(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Decode the existing outbound callback's Target spelling. Recipient
     * selection itself belongs entirely to ConnectionRegistry.
     * @param {string} target
     * @returns {string}
     */
    recipients(target) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ptr0 = passStringToWasm0(target, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.browserconnections_recipients(this.__wbg_ptr, ptr0, len0);
            deferred2_0 = ret[0];
            deferred2_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * @param {string} handle
     * @returns {string | undefined}
     */
    sender(handle) {
        const ptr0 = passStringToWasm0(handle, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.browserconnections_sender(this.__wbg_ptr, ptr0, len0);
        let v2;
        if (ret[0] !== 0) {
            v2 = getStringFromWasm0(ret[0], ret[1]).slice();
            wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        }
        return v2;
    }
}
if (Symbol.dispose) BrowserConnections.prototype[Symbol.dispose] = BrowserConnections.prototype.free;

/**
 * Re-export config preload functions from config_cache module.
 * @param {Function} callback
 */
export function set_config_request_callback(callback) {
    wasm.set_config_request_callback(callback);
}

/**
 * Called by JS once to register the single Host Channel callback (issue
 * #818). Bevy calls `callback(name: string, payload: any)` from
 * [`flush_host_channels`] for every host-page channel:
 *
 * - `"hud"`, `"lobby"`, `"chatter"`, `"audio_config"`, `"audio_cue"` —
 *   `payload` is a JSON string.
 * - `"shake"` — `payload` is a two-element `[x, y]` array (CSS pixels),
 *   emitted every frame.
 * - `"audio_level"` — `payload` is a bare number in 0.0–1.0, emitted on
 *   change only.
 *
 * Must be registered before `wasm_init()` so the first push is never missed.
 * JS must not assume any cross-channel ordering.
 * @param {Function} callback
 */
export function set_host_channel_callback(callback) {
    wasm.set_host_channel_callback(callback);
}

/**
 * Called by JS to register the outbound message callback.
 *
 * Bevy will invoke `callback(target: string, payload: string)` for every
 * outbound `ServerMessage`, where `target` is one of:
 * `"all"` — broadcast to every peer
 * `"token:<token>"` — send to one peer
 * `"except:<token>"` — broadcast excluding one peer
 * @param {Function} callback
 */
export function set_message_callback(callback) {
    wasm.set_message_callback(callback);
}

/**
 * Register the JS callback used by Rust to request runtime world/script content.
 *
 * The callback signature is: `callback(path: string)`. When called, JS must
 * fetch the TOML or Rhai source at `path` and deliver it via
 * `wasm_push_world_toml`.
 * @param {Function} callback
 */
export function set_world_fetch_callback(callback) {
    wasm.set_world_fetch_callback(callback);
}

/**
 * The active overlay stack + its path conflicts, for the host UI (issue #987).
 *
 * Returns `{ packs: [{ id, name, version, file_count, scenarios }], conflicts:
 * [{ path, winner, losers }] }`. `packs` is in load order (oldest → newest);
 * `scenarios` is the pack manifest's `[[scenario]]` id list. `conflicts` names,
 * for each authored path carried by two or more packs, the winning pack id and
 * the shadowed loser ids (load order). `server.html` renders the applied-pack
 * list with remove/reorder controls and the conflict summary from this.
 * @returns {any}
 */
export function wasm_active_pack_manifest() {
    const ret = wasm.wasm_active_pack_manifest();
    return ret;
}

/**
 * Validate an uploaded host mod-pack ZIP and, when accepted, PUSH it onto the
 * session-scoped overlay STACK (issues #760, #987).
 *
 * Called by the pre-scenario upload control on the host page with the raw
 * archive bytes. Validation is atomic (`world::mod_pack::validate_mod_pack`):
 * on ANY failure nothing is applied and the returned array carries error
 * findings; on success the pack is appended to the overlay stack (installing
 * pack B after pack A does NOT evict A) and an empty (or warning-only) array is
 * returned, after which JS re-reads `wasm_get_scenario_catalog` and
 * `wasm_active_pack_manifest`.
 *
 * The pack is validated against the ALREADY-ACTIVE stack (issue #987): a
 * duplicate pack id is rejected (`duplicate-pack-id`), an authored path shared
 * with an active pack warns (`overlapping-pack-path`), and the candidate's
 * composition may resolve a fragment supplied by an earlier active pack.
 *
 * Each finding is a JS object `{ severity, category, message, file, line }`.
 * Manifest root worlds — and the include fragments the pack's entity templates
 * pull in — resolve against the pack first, then the active stack, then base
 * content the host has already fetched (`cached_base_world_source` for worlds,
 * `raw_template_text` for entity/fragment TOML).
 *
 * **Absent from a demo build** (PRD #855, `build_flags::accepts_mod_pack_
 * uploads`). The public build ships a deliberately restricted catalogue —
 * combat_test with the Alliance Destroyer and Alliance Cruiser, curated by
 * `assets/scenarios.demo.toml` — and this is the one call that
 * widens it at runtime, adding whatever scenarios and hulls an uploaded ZIP
 * carries. Gating it with `#[cfg]` rather than a runtime refusal is the same
 * doctrine `command_admission::debug_route` follows and for the same reason:
 * the host page's upload button is hidden in a demo build
 * (`gui/build-flags.js`'s `offersModPackUpload`), a hidden button is a UI fact,
 * and UI facts are forgeable. With the export compiled out, the hidden control
 * and the closed route cannot come apart.
 *
 * The rest of the overlay surface (`wasm_clear_mod_pack`,
 * `wasm_remove_mod_pack`, `wasm_reorder_mod_packs`, `wasm_active_pack_
 * manifest`) is deliberately NOT gated: `server.html` calls those
 * unconditionally, and with nothing able to enter the stack they operate on an
 * empty one and answer emptily. Gating the entrance is the whole restriction;
 * gating the readers would only turn a no-op into a `TypeError`.
 * @param {Uint8Array} bytes
 * @returns {Array<any>}
 */
export function wasm_add_mod_pack(bytes) {
    const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_add_mod_pack(ptr0, len0);
    return ret;
}

/**
 * Queue one exact host-mesh start grant for fixed-tick application.
 * Returns `""` when queued or a stable machine refusal immediately.
 * @param {string} grant_json
 * @returns {string}
 */
export function wasm_apply_start_grant(grant_json) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(grant_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_apply_start_grant(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Queue one GM paused-transfer transaction for owner sequencing (#1293/#1294).
 * A first-time request reaches this only after visible acceptance; a reconnect
 * reaches it automatically after the private capability selected an existing
 * disconnected operator.
 * @param {bigint} join_id
 * @param {number} approved_by
 * @param {number} candidate_host
 * @param {string} operator_id
 * @param {string} scenario
 * @param {string} join_kind
 * @returns {boolean}
 */
export function wasm_begin_gm_join(join_id, approved_by, candidate_host, operator_id, scenario, join_kind) {
    const ptr0 = passStringToWasm0(operator_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(scenario, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ptr2 = passStringToWasm0(join_kind, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len2 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_begin_gm_join(join_id, approved_by, candidate_host, ptr0, len0, ptr1, len1, ptr2, len2);
    return ret !== 0;
}

/**
 * Read-only identity of the profile actually composed by [`wasm_init`].
 * @returns {string}
 */
export function wasm_boot_profile() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_boot_profile();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Judge a joining client's version stamp against this host's (issue #1111,
 * made mandatory in #1112).
 *
 * The host half of the Phoenix join handshake. `server.html` hands over the
 * `<protocol>/<content_id>/<content_epoch>` field a joiner declared over its
 * DataChannel and gets back `{"ok":true,…}` or `{"ok":false,"code":…,…}`; an
 * empty string means the client declared nothing, which is now REFUSED
 * (`client-stamp-missing`) rather than admitted — every client that can reach
 * a Phoenix host is a built Phoenix bundle carrying the field. See
 * [`crate::delivery::check_join_stamp`] for the full rule.
 *
 * This export is the reason the verdict is not re-implemented in JavaScript.
 * The rendezvous service's version advice is discovery help; the authority
 * stays `delivery::stamp::check_client_stamp`, the same pin the native host
 * enforces over HTTP.
 * @param {string} client_stamp
 * @returns {string}
 */
export function wasm_check_client_stamp(client_stamp) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(client_stamp, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_check_client_stamp(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Judge a joining SHIP HOST's version stamp against this host's (issue #1114).
 *
 * The fleet half of the same handshake, and a separate export rather than a
 * flag on the one above because the two answers genuinely differ: a host with
 * no manifest loaded admits a phone on the protocol alone and admits no ship
 * at all. See [`crate::delivery::check_host_stamp`] for why.
 *
 * Same `{"ok":…}` shape and the same `StampMismatch::code()` vocabulary, so
 * neither `gui/host-mesh.js` nor `gui/join-code.js`'s reason map needs a
 * fleet-only spelling of "that build does not match".
 * @param {string} peer_stamp
 * @returns {string}
 */
export function wasm_check_host_stamp(peer_stamp) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(peer_stamp, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_check_host_stamp(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Called by the OWNER page when a replacement machine has validly claimed a
 * disconnected fixed slot (issue #1120).
 *
 * `slot` is the `N` in the `slot-N` being reclaimed. The page has already checked
 * — in `gui/host-mesh.js`'s `admitHost` claim path — that the slot exists, is
 * frozen (post-mission-start), is currently disconnected, and that this is the
 * FIRST claim to reach the owner for it; this call is what turns that admission
 * into the fleet-wide, deterministic `SlotClaimFrame`. Queued for the next frame,
 * where `drain_mesh_inbound` mints the owner's next `claim_seq`, stamps the
 * current tick, records it in this host's own resolver and broadcasts it.
 * @param {number} slot
 */
export function wasm_claim_slot(slot) {
    wasm.wasm_claim_slot(slot);
}

/**
 * Discard the WHOLE host mod-pack overlay stack (issues #760 AC4, #987).
 *
 * Called on return-to-lobby (before the next scenario stage), so uploaded state
 * never leaks into a fresh selection or a same-page next round. A page reload
 * clears the thread-local anyway; this covers the same-page seams.
 */
export function wasm_clear_mod_pack() {
    wasm.wasm_clear_mod_pack();
}

/**
 * Request a named manual catalogue save at this peer's next fixed boundary.
 * Returns its stable internal slot id immediately; the status poll reports the
 * later Store outcome. Returns `""` and queues a local failure status when the
 * finite browser request queue is full.
 * @param {string} display_name
 * @returns {string}
 */
export function wasm_create_save_slot(display_name) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(display_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_create_save_slot(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * @returns {string}
 */
export function wasm_cross_target_probe() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_cross_target_probe();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Confirmation-bearing deletion backend. Passing `false` performs no Store
 * mutation and returns a stable code for the later UI adapter to localise.
 * @param {string} slot_id
 * @param {boolean} confirmed
 * @returns {string}
 */
export function wasm_delete_save_slot(slot_id, confirmed) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(slot_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_delete_save_slot(ptr0, len0, confirmed);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * This host's delivery version stamp, as the JSON `phoenix-host` serves at
 * `/host/stamp.json` (PRD #855).
 *
 * The browser host's half of the version pin: `server.html` can hand a peer
 * the same three numbers a native host publishes, encoded by the same
 * `codec::encode_delivery_stamp`, so "native and browser hosts consume the
 * same protocol contract" is checkable rather than asserted.
 * @returns {string}
 */
export function wasm_delivery_stamp() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_delivery_stamp();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * This host's own stamp as the three-part `<protocol>/<content_id>/<epoch>`
 * FIELD (issue #1114).
 *
 * [`wasm_delivery_stamp`] answers the same three numbers as a JSON object,
 * because that is what `/host/stamp.json` publishes. A ship host JOINING a
 * fleet has to present them in the compact form the handshake reads, and
 * having the page reassemble that string from the JSON would be a second,
 * quietly divergent spelling of a format `delivery::parse_stamp_field`
 * already owns — the same field a client bundle carries in its
 * `phoenix-client-stamp` meta tag.
 * @returns {string}
 */
export function wasm_delivery_stamp_field() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_delivery_stamp_field();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * The file name a host is offered for an exported save.
 *
 * Published rather than spelled in JS so the extension and the Rust constant
 * that explains it (`snapshot::EXPORT_FILE_NAME`) cannot drift apart.
 * @returns {string}
 */
export function wasm_export_file_name() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_export_file_name();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Export an existing selected slot through #866's one-shot artifact getter.
 * Compatibility is deliberately not a copy gate; only starting is gated.
 * @param {string} slot_id
 * @returns {string}
 */
export function wasm_export_save_slot(slot_id) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(slot_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_export_save_slot(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Queue an EXPORT of the running session (issue #866).
 *
 * The same queue, the same capture and the same tick boundary as
 * [`wasm_save_snapshot`]; only the destination differs. The resulting RON is
 * collected by [`wasm_take_exported_snapshot`] once the capture has been taken.
 */
export function wasm_export_snapshot() {
    wasm.wasm_export_snapshot();
}

/**
 * @param {string} path
 * @param {string} message
 */
export function wasm_fail_preload_fetch(path, message) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(message, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.wasm_fail_preload_fetch(ptr0, len0, ptr1, len1);
}

/**
 * Report a terminal runtime world/script fetch failure. Separate from
 * `wasm_push_world_toml` because an empty sibling Rhai file is valid.
 * @param {string} path
 * @param {string} message
 */
export function wasm_fail_world_fetch(path, message) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(message, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.wasm_fail_world_fetch(ptr0, len0, ptr1, len1);
}

/**
 * Poll the latest roster adoption attempt.
 *
 * Exact JSON: `{generation,status,reason}`, with status one of
 * `idle|pending|accepted|refused` and a null reason except on refusal.
 * @returns {string}
 */
export function wasm_fleet_join_status() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_fleet_join_status();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS (lobby "Launch AI Ship" button) to start the game with no
 * human players — all stations run under AI/backfill control.
 *
 * Only takes effect when the game is currently in the `Lobby` phase. The
 * flag is drained into a Bevy resource by `drain_force_start_input` on the
 * next `PreUpdate` frame; the actual phase transition is applied by
 * `apply_force_start` on the next `FixedUpdate` step (issue #907 — see that
 * function's doc for why the transition itself needs to be tick-scoped).
 */
export function wasm_force_start() {
    wasm.wasm_force_start();
}

/**
 * Called by JS each animation frame to read the latest AI doctrine-pool payload
 * as JSON while the panel is visible (issue #1149).
 *
 * Returns the raw JSON string `debug::ai_state::publish_ai_doctrine` wrote; the
 * dock parses it and draws a per-ship panel. Empty until the first publish.
 * @returns {string}
 */
export function wasm_get_ai_doctrine() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_ai_doctrine();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Return the list of available player ships for the currently loaded world.
 *
 * Returns a JS array of `{ template_path, label, class, hull_id, power_rating,
 * name }` objects. The label comes from the world's `[available_ships]` entry;
 * the remaining metadata is read from the cached entity config for each ship.
 * When the world has no `available_ships` list, returns an empty array — the
 * host should fall back to the hardcoded `assets/entities/alliance_cruiser.toml`.
 *
 * Uses `js_sys::Array` / `JsValue` to avoid manual JSON construction (which
 * would need escaping for `"` and `\` in template_path or label values).
 * @returns {Array<any>}
 */
export function wasm_get_available_ships() {
    const ret = wasm.wasm_get_available_ships();
    return ret;
}

/**
 * Called by JS each animation frame to read the latest console-latency payload
 * as JSON while the panel is visible (issue #1169).
 *
 * Returns the raw JSON string `debug::console_latency::publish_console_latency`
 * wrote; the dock parses it and draws a per-action table. Empty until the first
 * publish.
 * @returns {string}
 */
export function wasm_get_console_latency() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_console_latency();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS each animation frame to read the latest damage-log payload as
 * JSON while the surface is visible (issue #1150). The dock parses and renders
 * it; empty until the first publish.
 * @returns {string}
 */
export function wasm_get_damage_log() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_damage_log();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS while the settings cog is open, to read the debug flags the
 * simulation actually holds (issue #1169).
 *
 * Returns the JSON object `debug_overlay::report_debug_state` mirrors here —
 * `{"Regions":false,"ConsoleLatency":true,…}`, keyed by catalogue wire
 * names — or an empty string before the first report.
 *
 * # Why the cog needed a read-back at all
 *
 * The debug OUTPUT resources had no read-back export, so the cog painted from
 * its own module-local memory of what it had clicked. A phone flipping the same
 * flag left the two disagreeing, and for console latency that disagreement is
 * not cosmetic. The mirror is written by the
 * one system that already computes this set for the wire, so the host page and
 * a connected phone read the same answer derived from the same place.
 * @returns {string}
 */
export function wasm_get_debug_flags() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_debug_flags();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS each animation frame to read the latest modifier debug payload
 * as JSON while the surface is visible (issue #1150). The dock parses it and
 * renders the modifier sections; empty until the first publish.
 * @returns {string}
 */
export function wasm_get_debug_state() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_debug_state();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS each animation frame to read the latest entity-behavior payload
 * as JSON while the surface is visible (issue #1150). The dock parses and
 * renders it; empty until the first publish.
 * @returns {string}
 */
export function wasm_get_entity_debug_state() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_entity_debug_state();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS each animation frame to read the latest entity-inspector payload
 * as JSON while the surface is visible (issue #1150). The dock parses and
 * renders it; empty until the first publish.
 * @returns {string}
 */
export function wasm_get_entity_inspector() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_entity_inspector();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Return the scenario-authored GM role preset list for the currently loaded
 * world (issue #1319), as a JSON array of
 * `{ id, label, panels, quick_actions, contacts }` objects.
 *
 * Presentation only: the browser GM page (`gui/gm-role-presets.js`) filters
 * its own panels/quick actions against whichever preset a Game Master picks
 * and live-switches. Nothing here reaches `GmOperator`, the crew-public GM
 * roster, a `GmAction`, a snapshot, or the sim digest — see
 * `pasm/spec/design/gm-console-t2.yaml`'s `gm-t2-performing-surface`.
 *
 * Returns `"[]"` when the world declares none, or before a world has
 * loaded — the GM page falls back to the single built-in "All" preset,
 * which is never authored and always available.
 * @returns {string}
 */
export function wasm_get_gm_role_presets() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_gm_role_presets();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS to read the LocalShip's current God Mode state (issue #900),
 * e.g. to reflect it on the Debug panel button. Reads the mirror maintained
 * by `publish_god_mode`, since the authoritative value now lives in the
 * `GodMode` Bevy resource rather than a thread-local this function can touch
 * directly.
 * @returns {boolean}
 */
export function wasm_get_god_mode() {
    const ret = wasm.wasm_get_god_mode();
    return ret !== 0;
}

/**
 * Called by JS each frame to read back the instagib flag for the cog button
 * (issue #1181). Reads the `INSTAGIB_MIRROR` maintained by `publish_instagib`,
 * since the authoritative value now lives in the [`crate::server_app::Instagib`] Resource this
 * `World`-less function cannot touch directly (same pattern as
 * `wasm_get_god_mode`).
 * @returns {boolean}
 */
export function wasm_get_instagib() {
    const ret = wasm.wasm_get_instagib();
    return ret !== 0;
}

/**
 * Return the authoritative pre-load scenario/ship catalog.
 *
 * Unlike `wasm_get_available_ships` (which needs a loaded `WorldConfig`), this
 * reads the base scenario manifest pushed via `wasm_push_scenario_manifest`
 * and each referenced world TOML delivered via `wasm_push_world_toml`, so the
 * catalog is available *before* a root world is activated (issue #754).
 *
 * Returns a JS array of `{ id, world, label, description, source, ships: [...] }`
 * objects where each `ships` entry matches `wasm_get_available_ships`'s shape.
 * `source` (issue #990) is the pack id the scenario came from, or `"base"` for
 * a base-manifest scenario, so the phone picker can badge mod-supplied worlds.
 * Only scenarios whose world TOML has been delivered are catalogued; a
 * scenario whose world is still in flight is omitted until its TOML arrives.
 * Returns an empty array when no manifest has been pushed.
 *
 * Also runs `validate_manifest` over the base/demo manifest (issue #917) and
 * logs any findings as browser-console warnings under `LogCat::Config` — a
 * typo'd `ships` curation entry or similar is otherwise silently invisible,
 * since (unlike the mod-pack upload flow, which validates atomically at
 * `wasm_add_mod_pack`) nothing else ever calls `validate_manifest` on
 * this manifest. Findings are never fatal here, matching the
 * `missing-scenario-world` precedent below, where `build_merged_catalog`
 * simply skips an unresolvable entry rather than failing the whole catalog.
 * @returns {Array<any>}
 */
export function wasm_get_scenario_catalog() {
    const ret = wasm.wasm_get_scenario_catalog();
    return ret;
}

/**
 * Called by JS each animation frame to read the latest scenario-state payload
 * as JSON while the panel is visible (issue #1148).
 *
 * Returns the raw JSON string `debug::scenario::publish_scenario_state` wrote;
 * the dock parses it and draws a panel. Empty until the first publish.
 * @returns {string}
 */
export function wasm_get_scenario_state() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_scenario_state();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Return the Rhai host-fn signature registry for the scenario script editor
 * (issue #983, Rhai M5).
 *
 * The vocabulary a scenario author can call — the trigger builders and `on(..)`
 * the loading engine registers, plus the `ctx.effects` / `ctx.flags` /
 * `ctx.schedule` methods (and the delay-builder verbs) the runtime engine
 * registers — enumerated once in `world::script::authoring` so the editor's
 * autocomplete stays in step with what actually resolves at load and runtime.
 *
 * Returns a JS array of `{ name, receiver, category, summary, signature,
 * params: [...] }`. `receiver` is the `ctx` sub-object a method hangs off
 * (`"effects"` / `"flags"` / `"schedule"`), `"delay"` for the
 * `in_seconds(n).<verb>` builder verbs, or `""` for a top-level call.
 * @returns {Array<any>}
 */
export function wasm_get_script_host_fns() {
    const ret = wasm.wasm_get_script_host_fns();
    return ret;
}

/**
 * Called by JS each animation frame to read the latest station-activity payload
 * as JSON while the chart is visible (issue #1145).
 *
 * Returns the raw JSON string `debug::station_activity::publish_station_activity`
 * wrote; the dock parses it and draws a chart rather than printing it. Empty
 * until the first publish.
 * @returns {string}
 */
export function wasm_get_station_activity() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_get_station_activity();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Read-only absolute join progress for transport/public roster commit.
 * @returns {string}
 */
export function wasm_gm_join_status() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_gm_join_status();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS each animation frame to check whether the LocalShip currently
 * has a shared Navigation waypoint (issue #770, AC2). The host Debug panel
 * disables the teleport control while this returns `false`. Reads back the
 * value maintained by `publish_waypoint_existence`.
 * @returns {boolean}
 */
export function wasm_has_navigation_waypoint() {
    const ret = wasm.wasm_has_navigation_waypoint();
    return ret !== 0;
}

/**
 * Called by JS when a peer SHIP HOST's link closes, or when a survivor relays a
 * host-loss report (issue #1119).
 *
 * `slot` is the fleet slot ordinal (the `N` in `slot-N`) whose host vanished.
 * Queued for the next frame, where `drain_mesh_inbound` turns it into a
 * `HostLoss` observation: the simulation agrees the disconnect tick from that
 * host's own last watermark — the same on every survivor — and flips its ship
 * to Backfill there. Idempotent from the page's side too: reporting the same
 * slot twice, or a slot already backfilled, converges on the one transition.
 *
 * Deliberately separate from [`wasm_player_disconnected`]: a crew member
 * leaving flips one station on THIS host's own ship, while a host leaving flips
 * a whole PEER ship, at an agreed tick, on every surviving host at once.
 * @param {number} slot
 */
export function wasm_host_departed(slot) {
    wasm.wasm_host_departed(slot);
}

/**
 * Enter an imported file into this browser's own save catalogue, as a new
 * manual slot named `display_name` (issue #1363's AC2).
 *
 * Returns `""` when the row was written, or `"<class>\t<message>"` when it was
 * not — `damaged` for a file that is not a `Run` this build can parse, and
 * `not-stored` for a Store that would not take it. See
 * [`import_artifact_into_catalogue`] for why there is no third class, and in
 * particular why compatibility is not one.
 *
 * This is the half of #866's import that #1363 adds: the importer moved into
 * the catalogue's header, so importing is now an action ON the catalogue, and
 * an action on a list that does not change the list would be a control sitting
 * somewhere it does not belong. The staged direct boot below is unchanged and
 * still runs after this — the file both joins the list and starts, rather than
 * only starting.
 * @param {string} text
 * @param {string} display_name
 * @returns {string}
 */
export function wasm_import_save_slot(text, display_name) {
    let deferred3_0;
    let deferred3_1;
    try {
        const ptr0 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(display_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_import_save_slot(ptr0, len0, ptr1, len1);
        deferred3_0 = ret[0];
        deferred3_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
    }
}

/**
 * Called by JS on page load. Builds and runs the Bevy app.
 *
 * A [`boot::build`](crate::boot::build) adapter since issue #1219: the shared
 * core, the renderer axis (the real viewscreen stack for the host, the surrogate
 * for automation), and the world-ingestion order (the Rhai hashing-seed pin and
 * the content-ledger freeze — the browser's world itself arrives by the JS
 * preload, so the plan is [`WorldIngest::HostPreloaded`]) all come from
 * [`crate::boot`]. The two branches now differ by exactly one thing — the
 * [`BootProfile`] the WebDriver (`is_automation`) probe picks. What stays here is
 * the genuinely browser-only wiring boot has no reason to know about: the
 * WebDriver probe, the `?log=` URL parse, the JS ingress/egress `PreUpdate`/
 * `PostUpdate` seams, the debug-overlay/winit/audio wiring, and the thread-local
 * resource hand-offs.
 *
 * In WASM, `App::run()` hands control to requestAnimationFrame and returns
 * immediately, so this function does not block.
 */
export function wasm_init() {
    wasm.wasm_init();
}

/**
 * Called by JS to ask whether this page was built by the public demo deploy
 * (`PHOENIX_DEMO_BUILD=true`).
 *
 * The host settings menu hides its Debug/Cheat tab when this is true (issue
 * #939). See `crate::build_flags` for why this is its own flag rather than
 * `TRUNK_BUILD_RELEASE` (which the dev host also sets) or `debug_assertions`.
 * @returns {boolean}
 */
export function wasm_is_demo_build() {
    const ret = wasm.wasm_is_demo_build();
    return ret !== 0;
}

/**
 * Called by JS each frame to read back whether the simulation clock is
 * paused, so the settings menu can render pause vs. resume.
 *
 * Reads the `SIM_PAUSED` mirror rather than the resource: the toggle applies
 * a frame later, in `PreUpdate`, so a synchronous read-back right after the
 * click would report the stale value (same reasoning as `wasm_get_god_mode`).
 * @returns {boolean}
 */
export function wasm_is_paused() {
    const ret = wasm.wasm_is_paused();
    return ret !== 0;
}

/**
 * Check if preload is complete.
 * @returns {boolean}
 */
export function wasm_is_preload_complete() {
    const ret = wasm.wasm_is_preload_complete();
    return ret !== 0;
}

/**
 * Called by JS (or the viewscreen reduced-motion smoke) to query the
 * reduced-motion preference the host page last forwarded — the observable
 * proof that the profile value reached the WASM render path (issue #1173).
 * @returns {boolean}
 */
export function wasm_is_reduced_motion() {
    const ret = wasm.wasm_is_reduced_motion();
    return ret !== 0;
}

/**
 * Called by JS when the fleet roster freezes and the mission starts.
 *
 * `roster_json` is the frozen fleet: which host flies which hull, who is
 * aboard each and at what Station Rating, and which slot is this host's. Every
 * host in the fleet receives the identical roster, which is what lets them
 * spawn identical ships with identical identities and seed identical ratings.
 *
 * Returns the queued generation on success or a machine reason on immediate
 * decode refusal. Queueing is not acceptance: the page must poll
 * [`wasm_fleet_join_status`] and withhold grants until that same generation is
 * accepted by the Bevy-world drain.
 * @param {string} roster_json
 * @returns {string}
 */
export function wasm_join_fleet(roster_json) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(roster_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_join_fleet(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Leave the currently installed fleet at the next safe Bevy-world drain.
 *
 * The returned decimal generation is polled through
 * [`wasm_fleet_join_status`], exactly like a join. Calling Leave cancels any
 * not-yet-adopted join and clears old-generation edge/frame latches. A fresh
 * Lobby teardown is accepted; a mission that has started is refused with
 * `fleet-leave-not-lobby` and remains installed.
 * @returns {string}
 */
export function wasm_leave_fleet() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_leave_fleet();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Read this browser's private save catalogue. Objects are assembled through
 * `js_sys`; save metadata never takes the crate's JSON codec exception.
 * @returns {Array<any>}
 */
export function wasm_list_save_slots() {
    const ret = wasm.wasm_list_save_slots();
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Re-export config preload functions from config_cache module.
 * @param {string} path
 * @param {string} toml_str
 * @returns {any}
 */
export function wasm_load_config(path, toml_str) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_load_config(ptr0, len0, ptr1, len1);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Unified world loader: a single TOML file containing anchors, immediate
 * entity instances (asteroid fields, stations, NPCs, etc.), named [[entity]]
 * instances for trigger / comms anchors, [[trigger]] blocks, and [[comms]]
 * templates.
 *
 * Delegates to `config_cache::wasm_load_world`, which performs the unified
 * `parse_world` pass into the `WORLD_CONFIG` thread-local. After PRD #341
 * this is the only world loader — the legacy two-loader split is gone.
 *
 * `curated_ships` (issue #917) is the locked scenario's playable-hull
 * allowlist — the same `template_path` values as the catalog entry's
 * `ships` (`wasm_get_scenario_catalog`) — restricting which
 * `[[available_ships]]` hulls get preloaded. `server.html` passes `[]` when
 * no scenario was resolved through the catalog (e.g. the `?scenario=<path>`
 * dev bypass), which preloads every hull the world offers, unchanged from
 * pre-#917 behaviour.
 * @param {string} path
 * @param {string} toml_str
 * @param {string[]} curated_ships
 * @returns {any}
 */
export function wasm_load_world(path, toml_str, curated_ships) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ptr2 = passArrayJsValueToWasm0(curated_ships, wasm.__wbindgen_malloc);
    const len2 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_load_world(ptr0, len0, ptr1, len1, ptr2, len2);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * The fleet slot a host-mesh frame declares it came from, for the OWNER to
 * authenticate a member's frame before relaying it (issue #1120).
 *
 * The frame body is opaque to `gui/host-mesh.js` — it is Rust-minted — so the page
 * cannot read the declared `from` itself. This decodes it and returns the `N` in
 * `slot-N`, or `-1` for a frame this build cannot decode. The owner compares it to
 * the slot it bound the delivering connection to at join: a mismatch is a forged
 * origin, dropped at the star centre before it can reach a sibling, which is what
 * makes the mesh-boundary authentication real for members who cannot themselves
 * re-authenticate a relayed frame.
 * @param {string} json
 * @returns {number}
 */
export function wasm_mesh_frame_from(json) {
    const ptr0 = passStringToWasm0(json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_mesh_frame_from(ptr0, len0);
    return ret;
}

/**
 * What this host's fleet link looks like from the simulation's side (issue
 * #1116), as JSON for the operator surface and the smoke tests.
 *
 * ```json
 * { "in_fleet": true, "slot": 1, "tick": 412, "delay": 6,
 *   "stalled": false, "stalled_frames": 0, "waiting_on": [2],
 *   "peers_heard": [2], "agreed": true }
 * ```
 *
 * Read-only and derived — it reports the barrier and the digest exchange, and
 * changes neither. `waiting_on` is the diagnostic that turns "the mission
 * froze" into "slot 2 is behind", which is the difference between a bug report
 * and a fix.
 * @returns {string}
 */
export function wasm_mesh_status() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_mesh_status();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Which scenario an imported file belongs to, BEFORE any world is loaded on its
 * behalf (issue #866).
 *
 * Returns `"ok\t<scenario path>"`, or `"damaged\t<why>"` for a file that is not
 * a save this build can parse at all. Tab-separated for
 * [`wasm_snapshot_status`]'s reason: one string is the cheapest thing that
 * crosses this boundary, and the host page needs the CLASS as well as the
 * sentence — a damaged file and an incompatible one send a host to two
 * different places.
 *
 * Only the damaged class can be answered here, and that is the point of having
 * two calls rather than one. The version gate needs a content digest, a content
 * digest needs a loaded world, and which world to load is written inside the
 * file — so parsing has to come first and the gate second. Splitting them means
 * a damaged file is refused before this page loads a scenario on its behalf,
 * rather than after.
 * @param {string} text
 * @returns {string}
 */
export function wasm_peek_import(text) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_peek_import(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * The capture so far, as JSON, for a harness to pull out of the page.
 *
 * Non-destructive: the page keeps sampling afterwards, so a test may take a
 * reading at several points in a session. Returns an empty string when
 * nothing has been sampled, which a harness should treat as "not ready yet"
 * rather than as a zeroed measurement.
 * @param {string} scenario
 * @returns {string}
 */
export function wasm_perf_capture(scenario) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(scenario, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_perf_capture(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Called by JS when a peer connection closes.
 *
 * Queues a disconnect lifecycle event that Bevy processes next frame,
 * replacing the old workaround of dispatching a fake `ClearConsole` message.
 * @param {string} token
 */
export function wasm_player_disconnected(token) {
    const ptr0 = passStringToWasm0(token, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_player_disconnected(ptr0, len0);
}

/**
 * @returns {string}
 */
export function wasm_preload_error() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_preload_error();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Authoritative resident world source, including pack-only scenarios.
 * @param {string} path
 * @returns {string | undefined}
 */
export function wasm_preload_world_source(path) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_preload_world_source(ptr0, len0);
    let v2;
    if (ret[0] !== 0) {
        v2 = getStringFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    }
    return v2;
}

/**
 * Select the explicit production rendererless browser GM profile. The page
 * calls this before [`wasm_init`]; it deliberately does not depend on
 * `navigator.webdriver`.
 */
export function wasm_prepare_game_master() {
    wasm.wasm_prepare_game_master();
}

/**
 * Prepare a candidate's world topology without admitting it to the
 * authoritative roster or lockstep wait-set. Commit is the only code path
 * which installs those resources.
 * @param {bigint} join_id
 * @param {string} roster_json
 * @returns {boolean}
 */
export function wasm_prepare_gm_join_candidate(join_id, roster_json) {
    const ptr0 = passStringToWasm0(roster_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_prepare_gm_join_candidate(join_id, ptr0, len0);
    return ret !== 0;
}

/**
 * Put an imported file through the version gate and stage it for the boot that
 * is about to happen (issue #866). Call BEFORE `wasm_init`, exactly where
 * [`wasm_prepare_resume`] is called.
 *
 * Returns `""` when the file was accepted and is now pending, or
 * `"<class>\t<message>"` when it was not. The two classes are the two AC5
 * answers and they are deliberately not one:
 *
 * * `damaged` — the file is not a `Run` this build can parse. Truncated,
 *   hand-edited, or never a save. The host should pick another file.
 * * `incompatible` — the file is intact and this build cannot honour it. The
 *   message is `vellum_save::Moved`'s own sentence, verbatim, because it names
 *   WHICH dimension moved and to what, and phoenix has no vocabulary that would
 *   say more.
 *
 * A refusal stages nothing, so the page boots a normal new session — the same
 * promise [`wasm_prepare_resume`] makes, for the same reason.
 * @param {string} text
 * @returns {string}
 */
export function wasm_prepare_import(text) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_prepare_import(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Read `slot`, put it through the version gate, and hold it for the boot that
 * is about to happen. Call BEFORE `wasm_init`.
 *
 * Returns `""` when the save was accepted and is now pending, or the refusal
 * to show the host. A refusal leaves nothing staged, so the page boots into a
 * normal new session.
 * @param {string} slot
 * @returns {string}
 */
export function wasm_prepare_resume(slot) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(slot, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_prepare_resume(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Deliver an entity template to Rust for PRE-LOAD catalogue enrichment.
 *
 * `wasm_get_scenario_catalog` is read before any world is activated, so
 * `delivery::payload::ship_payload` finds no cached template and publishes
 * `template_path` + `label` and nothing else — the reason every hull card in
 * the picker badged `[UNKNOWN]` with no registry, mass or power rating. The
 * host page closes that gap by fetching each hull the first catalogue pass
 * names and delivering it here BEFORE reading the catalogue again.
 *
 * Pass `root = true` for a hull the catalogue named and `false` for an include
 * fragment. The return value is the canonical fragment paths still missing;
 * fetch those, deliver them the same way, and repeat until it comes back
 * empty. This store is separate from the preload's own and is read only by
 * `delivery::payload::ship_payload` — see `config_cache::push_catalog_template`.
 *
 * Pass an EMPTY `toml_str` when the fetch failed, the way `handleConfigRequest`
 * calls `wasm_load_config(path, '')` on a 404: a mod pack's own hull has no URL
 * at all, and that is what lets its text be taken from the session overlay
 * instead. With no overlay copy either the call is a no-op.
 * @param {string} path
 * @param {string} toml_str
 * @param {boolean} root
 * @returns {Array<any>}
 */
export function wasm_push_catalog_template(path, toml_str, root) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_push_catalog_template(ptr0, len0, ptr1, len1, root);
    return ret;
}

/**
 * Deliver the base scenario manifest (`assets/scenarios.toml`) to Rust.
 *
 * Called by JS during preload, before any world is loaded. Stored so
 * `wasm_get_scenario_catalog` can build the pre-load catalog (issue #754).
 *
 * It is ALSO this host's content identity: [`wasm_delivery_stamp`],
 * [`wasm_delivery_stamp_field`], [`wasm_check_client_stamp`] and
 * [`wasm_check_host_stamp`] all build their stamp from whatever was pushed
 * here, and an empty store stamps an identity that matches nothing (and, for a
 * fleet, is refused outright). So `server.html` pushes it on EVERY boot path —
 * `pushScenarioManifest()` — not only from the catalogue build the
 * `?scenario=` bypass skips.
 * @param {string} toml_str
 */
export function wasm_push_scenario_manifest(toml_str) {
    const ptr0 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_push_scenario_manifest(ptr0, len0);
}

/**
 * Deliver a preloaded or runtime-fetched model-rig sidecar TOML to Rust.
 *
 * Before boot, the entity-config preload calls this for every primary authored
 * rig and uses the return value as its completion signal; this records the
 * exact bytes before the content ledger freezes. The runtime world callback
 * reuses it for later/generated paths and ignores the return. Pass an empty
 * string when the sidecar is absent (404) so every target binds the same empty
 * bytes and proceeds with an identity rig.
 * @param {string} path
 * @param {string} toml_str
 * @returns {boolean}
 */
export function wasm_push_sidecar_toml(path, toml_str) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_push_sidecar_toml(ptr0, len0, ptr1, len1);
    return ret !== 0;
}

/**
 * Deliver runtime-fetched world TOML or sibling Rhai source to the Rust side.
 *
 * Called by JS after fetching a world/script path that Rust requested via the
 * `set_world_fetch_callback` callback.
 * @param {string} path
 * @param {string} toml_str
 */
export function wasm_push_world_toml(path, toml_str) {
    const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.wasm_push_world_toml(ptr0, len0, ptr1, len1);
}

/**
 * Called by JS with one host-mesh frame from another ship host (issue #1116),
 * tagged with the fleet slot the delivering connection was authenticated to
 * (issue #1120).
 *
 * `authenticated_slot` is the `N` in the `slot-N` the page bound this connection
 * to at join — the transport-level proof of who is speaking, which the simulation
 * checks the frame's own declared `from` against at the mesh boundary
 * (`lockstep::apply_mesh_inbox`). `0` (never a real fleet slot, which start at
 * `slot-1`) means the page could not resolve it, so the frame is trusted as it
 * was before this authentication existed.
 *
 * `json` is the `{ m, t, tick, d }` envelope `gui/host-mesh.js` decoded and
 * recognised as the simulation's. The page never reads the body; this is where it
 * is understood. A frame that is not one of those, or is of a revision this build
 * does not speak, is dropped here rather than guessed at, exactly as the JS
 * decoder drops one it does not recognise.
 * @param {number} authenticated_slot
 * @param {string} json
 */
export function wasm_receive_mesh_frame(authenticated_slot, json) {
    const ptr0 = passStringToWasm0(json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_receive_mesh_frame(authenticated_slot, ptr0, len0);
}

/**
 * Called by JS to deliver an inbound message from a peer into Bevy.
 *
 * `sender_token` — the session token of the sender (resolved by the JS
 * bridge from its peer-id → token map; for Identify it equals the token
 * inside the JSON payload).
 * `json` — a JSON-encoded `ClientMessage`.
 * @param {string} sender_token
 * @param {string} json
 */
export function wasm_receive_message(sender_token, json) {
    const ptr0 = passStringToWasm0(sender_token, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.wasm_receive_message(ptr0, len0, ptr1, len1);
}

/**
 * Queue the owner's terminal answer when an accepted candidate disconnects.
 * @param {bigint} join_id
 * @param {string} reason
 * @returns {boolean}
 */
export function wasm_refuse_gm_join(join_id, reason) {
    const ptr0 = passStringToWasm0(reason, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_refuse_gm_join(join_id, ptr0, len0);
    return ret !== 0;
}

/**
 * Remove the pack with `id` from the overlay stack (issue #987). Precedence for
 * every path it owned re-resolves automatically — the next pack down that
 * carries the path becomes the winner. Returns whether a pack was removed.
 * @param {string} id
 * @returns {boolean}
 */
export function wasm_remove_mod_pack(id) {
    const ptr0 = passStringToWasm0(id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_remove_mod_pack(ptr0, len0);
    return ret !== 0;
}

/**
 * Rename only a manual slot's sidecar. Returns empty on success.
 * @param {string} slot_id
 * @param {string} display_name
 * @returns {string}
 */
export function wasm_rename_save_slot(slot_id, display_name) {
    let deferred3_0;
    let deferred3_1;
    try {
        const ptr0 = passStringToWasm0(slot_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(display_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_rename_save_slot(ptr0, len0, ptr1, len1);
        deferred3_0 = ret[0];
        deferred3_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
    }
}

/**
 * Reorder the overlay stack to match `ids` (oldest → newest / lowest → highest
 * precedence), from the host reorder controls (issue #987). Ids not named keep
 * their relative order after the named ones; unknown ids are ignored.
 * @param {string[]} ids
 */
export function wasm_reorder_mod_packs(ids) {
    const ptr0 = passArrayJsValueToWasm0(ids, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_reorder_mod_packs(ptr0, len0);
}

/**
 * Whether a save is staged and waiting for the world to finish bootstrapping.
 *
 * Reads the `RESUME_PENDING_MIRROR` edge cache (issue #1181): once `wasm_init`
 * has handed the staged save to the shared restore driver, this
 * `World`-less getter can no longer read the Resource directly, so
 * `drain_snapshot_restore` mirrors its presence out each frame — true while the
 * save waits, false the moment it is applied or abandoned.
 * @returns {boolean}
 */
export function wasm_resume_pending() {
    const ret = wasm.wasm_resume_pending();
    return ret !== 0;
}

/**
 * Queue a save of the running session into `slot`.
 *
 * Returns immediately; the capture happens on the next fixed-tick boundary,
 * storage drains afterward in `PostUpdate`, and the outcome is read back
 * through [`wasm_snapshot_status`].
 * @param {string} slot
 */
export function wasm_save_snapshot(slot) {
    const ptr0 = passStringToWasm0(slot, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_save_snapshot(ptr0, len0);
}

/**
 * Publish the browser picker's current enriched catalogue through the same
 * typed message and pack projection as the native host. Taking the current
 * picker snapshot preserves asynchronous template enrichment and curation.
 * @param {string} scenarios_json
 * @param {string | null} [locked_scenario]
 * @param {string | null} [locked_ship]
 * @returns {string}
 */
export function wasm_scenario_catalog_message(scenarios_json, locked_scenario, locked_ship) {
    let deferred5_0;
    let deferred5_1;
    try {
        const ptr0 = passStringToWasm0(scenarios_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        var ptr1 = isLikeNone(locked_scenario) ? 0 : passStringToWasm0(locked_scenario, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len1 = WASM_VECTOR_LEN;
        var ptr2 = isLikeNone(locked_ship) ? 0 : passStringToWasm0(locked_ship, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len2 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_scenario_catalog_message(ptr0, len0, ptr1, len1, ptr2, len2);
        var ptr4 = ret[0];
        var len4 = ret[1];
        if (ret[3]) {
            ptr4 = 0; len4 = 0;
            throw takeFromExternrefTable0(ret[2]);
        }
        deferred5_0 = ptr4;
        deferred5_1 = len4;
        return getStringFromWasm0(ptr4, len4);
    } finally {
        wasm.__wbindgen_free(deferred5_0, deferred5_1, 1);
    }
}

/**
 * Compile a `.rhai` source (a sibling file's whole text, or a lifted inline
 * `[script.*]` block) under the sandbox and return editor diagnostics (issue
 * #983, Rhai M5).
 *
 * `line_offset` is added to every reported line so an inline block edited
 * inside its host TOML lands on the correct *document* line — the editor passes
 * the block's start line; a standalone `.rhai` file passes `0`. Returns a JS
 * array of `{ message, line, column, severity }` (empty when the source loads
 * clean). Uses the same loading-engine compile + top-level run as the
 * activation gate, so a source that is clean here is clean there.
 * @param {string} source
 * @param {number} line_offset
 * @returns {Array<any>}
 */
export function wasm_script_diagnostics(source, line_offset) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_script_diagnostics(ptr0, len0, line_offset);
    return ret;
}

/**
 * Store the host's chosen player ship template path.
 *
 * Must be called before `wasm_init()`. The path is used by
 * `update_session_with_config` and `spawn_game_start_entities` to
 * load the correct ship config and entity template.
 * @param {string} template_path
 */
export function wasm_select_ship(template_path) {
    const ptr0 = passStringToWasm0(template_path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_select_ship(ptr0, len0);
}

/**
 * Set one host diagnostic surface by its catalogue-owned wire name.
 *
 * This is the only host diagnostic mutation export. It carries an absolute
 * state derived from the authoritative readback, so a phone and the host
 * cannot race a relative local toggle into the opposite value. Unknown names
 * are rejected without queuing anything.
 *
 * Absent from a public-demo binary. Readback exports remain available there.
 * @param {string} wire_name
 * @param {boolean} enabled
 * @returns {boolean}
 */
export function wasm_set_debug_surface(wire_name, enabled) {
    const ptr0 = passStringToWasm0(wire_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_set_debug_surface(ptr0, len0, enabled);
    return ret !== 0;
}

/**
 * Enable or disable browser-mesh ownership of collective lobby start.
 * Enabling fails closed until [`wasm_set_fleet_start_validation`] supplies the
 * current local content/peer validation result.
 * @param {boolean} enabled
 * @returns {string}
 */
export function wasm_set_fleet_managed_lobby(enabled) {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_set_fleet_managed_lobby(enabled);
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Publish this host's independent validation gate for the next fixed-tick
 * grant. Readiness is not sent through this seam; it is ordered by the mesh
 * owner before the common grant.
 * @param {boolean} valid
 * @returns {string}
 */
export function wasm_set_fleet_start_validation(valid) {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_set_fleet_start_validation(valid);
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Replace the crew-public Game Master roster on the next frame (issue #1289).
 *
 * The host page owns the complete rendezvous projection and therefore sends a
 * complete array, never deltas. Each row is exactly
 * `{ id, name, connected, ready }`;
 * the codec rejects duplicate/unbounded rows and any private extra field.
 * Returns `""` on success or a stable machine reason on refusal.
 * @param {string} roster_json
 * @returns {string}
 */
export function wasm_set_gm_roster(roster_json) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(roster_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_set_gm_roster(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Called by JS to restrict logging to named entities, from `?log_entity=`.
 * Must be called before `wasm_init()` to take effect.
 *
 * Comma-separated display names, matched exactly then case-insensitively as a
 * substring — e.g. `?log_entity=Ironveil,Ashrender`.
 * @param {string} names
 */
export function wasm_set_log_entity(names) {
    const ptr0 = passStringToWasm0(names, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_set_log_entity(ptr0, len0);
}

/**
 * Called by JS to set the log category/level spec from `?log=` in the URL.
 * Must be called before `wasm_init()` to take effect.
 *
 * Same syntax as the headless runner's `--log`, e.g.
 * `?log=info,ai=debug,admit=trace`.
 * @param {string} spec
 */
export function wasm_set_log_spec(spec) {
    const ptr0 = passStringToWasm0(spec, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.wasm_set_log_spec(ptr0, len0);
}

/**
 * Called by the host page to forward its reduced-motion preference
 * (`window.matchMedia('(prefers-reduced-motion: reduce)').matches`) to the
 * viewscreen renderer (issue #1173). May be called before `wasm_init()` and at
 * any time after (e.g. from the media-query `change` listener): the value is
 * drained into `ViewscreenMotion` every frame by
 * `viewscreen_border::sync_reduced_motion`, so a runtime change takes effect
 * without a reload.
 * @param {boolean} enabled
 */
export function wasm_set_reduced_motion(enabled) {
    wasm.wasm_set_reduced_motion(enabled);
}

/**
 * The logical simulation tick count (issue #895) — the number of completed
 * `FixedUpdate` steps. Read back from the mirror maintained by
 * `publish_sim_tick` each frame.
 *
 * Returned as `f64` so JS receives a plain number rather than a `BigInt`;
 * at 60 Hz the count stays exactly representable for ~4.7 million years.
 * The smoke tests sample this twice to assert the sim advances on the
 * authored `[global] sim_tick_hz` clock rather than the rendered frame rate.
 * @returns {number}
 */
export function wasm_sim_tick() {
    const ret = wasm.wasm_sim_tick();
    return ret;
}

/**
 * @returns {string}
 */
export function wasm_simmath_battery() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_simmath_battery();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Take the oldest retained host-visible save or resume outcome. Each retained
 * outcome is reported exactly once; if an inactive poller fills the finite
 * outbox, the oldest status is replaced so the newest local refusal stays
 * visible.
 *
 * Returns `""` when there is nothing to report, else
 * `"<ok|error>\t<save|resume>\t<message>"`. Tab-separated rather than a status
 * *object* because this crosses a `wasm_bindgen` boundary into a
 * classic-script host page, and one string is the cheapest thing that crosses
 * it; the host page splits on the first two tabs. No field but the message can
 * contain one.
 *
 * For a refused resume the message is `vellum_save::Moved`'s own sentence,
 * verbatim — phoenix has no status vocabulary of its own to render it in.
 * @returns {string}
 */
export function wasm_snapshot_status() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_snapshot_status();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Stage a compatible local slot for the next fresh app boot. This is an alias
 * of the established pre-init resume gate; it never receives a `World`, so a
 * running session cannot be restored through it.
 * @param {string} slot_id
 * @returns {string}
 */
export function wasm_stage_save_slot(slot_id) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passStringToWasm0(slot_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasm_stage_save_slot(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Queue one typed, attributed GM action for privileged frame-driven
 * admission. The browser never receives a generic mutation route.
 * @param {string} request_json
 * @returns {boolean}
 */
export function wasm_submit_gm_action(request_json) {
    const ptr0 = passStringToWasm0(request_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_submit_gm_action(ptr0, len0);
    return ret !== 0;
}

/**
 * Take the exported save's text, if one is waiting.
 *
 * Returns `""` when there is nothing to collect. Taken rather than read, so a
 * host page polling this cannot download the same save twice.
 * @returns {string}
 */
export function wasm_take_exported_snapshot() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_take_exported_snapshot();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Everything this host wants to say to its fleet, as a JSON array of encoded
 * frames, taken and cleared.
 *
 * Polled by the page's own loop rather than pushed through a callback: the
 * fleet link is a socket the page owns, and a callback would hand it a tick
 * frame at whatever instant the simulation sealed it — inside a fixed step,
 * which is the one place JS must not be re-entered from.
 * @returns {string}
 */
export function wasm_take_mesh_frames() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_take_mesh_frames();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Drain one local fixed-tick grant result as JSON, or `""` when none waits.
 * @returns {string}
 */
export function wasm_take_start_result() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.wasm_take_start_result();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Called by JS (host Debug panel) to teleport the local ship onto the shared
 * Navigation waypoint (issue #770). A host-only simulation override, not a
 * client command: it sets a pending flag consumed by
 * `drain_teleport_to_waypoint` on the next `PreUpdate`, which directly writes
 * the LocalShip's authoritative `ShipPhysics.{x,z}`. Deliberately bypasses
 * command admission — this is a debug override, never replicated to clients.
 */
export function wasm_teleport_to_waypoint() {
    wasm.wasm_teleport_to_waypoint();
}

/**
 * Called by JS (host Debug panel God Mode button) to request a God Mode
 * flip (issue #900). Unlike the old thread-local this does NOT flip
 * anything itself: it queues a request that `drain_god_mode_toggle` turns
 * into a `ToggleGodMode` command crossing the normal admission boundary on
 * the next `PreUpdate` frame, so the flip carries a tick, lands in the
 * command log, and replays. The JS binding's signature is unchanged.
 */
export function wasm_toggle_god_mode() {
    wasm.wasm_toggle_god_mode();
}

/**
 * Called by JS (settings cog Debug/Cheat tab) to request an instagib flip.
 *
 * Queues a request that `drain_instagib_toggle` applies to the [`crate::server_app::Instagib`]
 * Resource on the next `PreUpdate` frame (issue #1181). The JS binding's
 * signature is unchanged; only the state it targets moved from a thread-local
 * into a Resource `tick_beams_apply_damage` reads through the scheduler.
 */
export function wasm_toggle_instagib() {
    wasm.wasm_toggle_instagib();
}

/**
 * Called by JS to pause/unpause the simulation clock.
 *
 * Sets a pending flag that is consumed by `drain_host_controls` in the next
 * `PreUpdate` frame, which pauses or unpauses `Time<Virtual>`.
 *
 * Named without `debug` (issue #939) because its only caller is the host
 * settings menu's **Gameplay** tab, which ships in the demo build where the
 * Debug/Cheat tab is gone. Nothing on this path is gated by
 * `PHOENIX_DEMO_BUILD`.
 */
export function wasm_toggle_pause() {
    wasm.wasm_toggle_pause();
}

/**
 * Called by JS with the chosen ship template path and the raw TOML content it
 * fetched from that path, to validate the `[[station]]`/`[[system]]` schema
 * before starting the server.
 *
 * The path is load-bearing: it is what the include closure is resolved
 * against. See [`validate_ship_stations`] for why the delivered text alone is
 * not enough.
 *
 * On success, stores the parsed `ShipStations` internally and returns
 * `Ok(JsValue::UNDEFINED)`. On failure, returns `Err(JsValue)` with a
 * human-readable error string. The crew transport should not start when this returns
 * an error.
 * @param {string} template_path
 * @param {string} toml_str
 * @returns {any}
 */
export function wasm_validate_stations(template_path, toml_str) {
    const ptr0 = passStringToWasm0(template_path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(toml_str, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_validate_stations(ptr0, len0, ptr1, len1);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Window_1535697a053fe988: function(arg0) {
            const ret = arg0.Window;
            return ret;
        },
        __wbg_Window_c7f91e3f80ae0a0e: function(arg0) {
            const ret = arg0.Window;
            return ret;
        },
        __wbg_WorkerGlobalScope_b9ad7f2d34707e2e: function(arg0) {
            const ret = arg0.WorkerGlobalScope;
            return ret;
        },
        __wbg___wbindgen_boolean_get_c3dd5c39f1b5a12b: function(arg0) {
            const v = arg0;
            const ret = typeof(v) === 'boolean' ? v : undefined;
            return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
        },
        __wbg___wbindgen_debug_string_07cb72cfcc952e2b: function(arg0, arg1) {
            const ret = debugString(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_is_function_2f0fd7ceb86e64c5: function(arg0) {
            const ret = typeof(arg0) === 'function';
            return ret;
        },
        __wbg___wbindgen_is_null_066086be3abe9bb3: function(arg0) {
            const ret = arg0 === null;
            return ret;
        },
        __wbg___wbindgen_is_undefined_244a92c34d3b6ec0: function(arg0) {
            const ret = arg0 === undefined;
            return ret;
        },
        __wbg___wbindgen_number_get_dd6d69a6079f26f1: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'number' ? obj : undefined;
            getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_string_get_965592073e5d848c: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'string' ? obj : undefined;
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_throw_9c75d47bf9e7731e: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg__wbg_cb_unref_158e43e869788cdc: function(arg0) {
            arg0._wbg_cb_unref();
        },
        __wbg_abort_87eb7f23cf4b73d1: function(arg0) {
            arg0.abort();
        },
        __wbg_activeElement_4afc74fc207bb2f3: function(arg0) {
            const ret = arg0.activeElement;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_activeTexture_b8a63f4b51a716a9: function(arg0, arg1) {
            arg0.activeTexture(arg1 >>> 0);
        },
        __wbg_activeTexture_df98f0476a8d2771: function(arg0, arg1) {
            arg0.activeTexture(arg1 >>> 0);
        },
        __wbg_addEventListener_a95e75babfc4f5a3: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            arg0.addEventListener(getStringFromWasm0(arg1, arg2), arg3);
        }, arguments); },
        __wbg_addListener_223ba2d16cad9260: function() { return handleError(function (arg0, arg1) {
            arg0.addListener(arg1);
        }, arguments); },
        __wbg_altKey_0a7b13357fc7557d: function(arg0) {
            const ret = arg0.altKey;
            return ret;
        },
        __wbg_altKey_6c67d807c153b5b3: function(arg0) {
            const ret = arg0.altKey;
            return ret;
        },
        __wbg_animate_8f41e2f47c7d04ab: function(arg0, arg1, arg2) {
            const ret = arg0.animate(arg1, arg2);
            return ret;
        },
        __wbg_appendChild_f8e0d8251588e3d1: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.appendChild(arg1);
            return ret;
        }, arguments); },
        __wbg_arrayBuffer_87e3ac06d961f7a0: function() { return handleError(function (arg0) {
            const ret = arg0.arrayBuffer();
            return ret;
        }, arguments); },
        __wbg_attachShader_18d37e6a1936237b: function(arg0, arg1, arg2) {
            arg0.attachShader(arg1, arg2);
        },
        __wbg_attachShader_ce0935c038866500: function(arg0, arg1, arg2) {
            arg0.attachShader(arg1, arg2);
        },
        __wbg_axes_80669e8633f8b14e: function(arg0) {
            const ret = arg0.axes;
            return ret;
        },
        __wbg_beginQuery_57423f952238d42b: function(arg0, arg1, arg2) {
            arg0.beginQuery(arg1 >>> 0, arg2);
        },
        __wbg_bindAttribLocation_da2a20a747100943: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.bindAttribLocation(arg1, arg2 >>> 0, getStringFromWasm0(arg3, arg4));
        },
        __wbg_bindAttribLocation_eff3edd4a7818b2a: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.bindAttribLocation(arg1, arg2 >>> 0, getStringFromWasm0(arg3, arg4));
        },
        __wbg_bindBufferRange_a1e77739561685ab: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.bindBufferRange(arg1 >>> 0, arg2 >>> 0, arg3, arg4, arg5);
        },
        __wbg_bindBuffer_a77c5c8cfa41f082: function(arg0, arg1, arg2) {
            arg0.bindBuffer(arg1 >>> 0, arg2);
        },
        __wbg_bindBuffer_baae5a34a697efa6: function(arg0, arg1, arg2) {
            arg0.bindBuffer(arg1 >>> 0, arg2);
        },
        __wbg_bindFramebuffer_5724927db7943266: function(arg0, arg1, arg2) {
            arg0.bindFramebuffer(arg1 >>> 0, arg2);
        },
        __wbg_bindFramebuffer_fb9ea036031ad65f: function(arg0, arg1, arg2) {
            arg0.bindFramebuffer(arg1 >>> 0, arg2);
        },
        __wbg_bindRenderbuffer_7e84f06129c44e35: function(arg0, arg1, arg2) {
            arg0.bindRenderbuffer(arg1 >>> 0, arg2);
        },
        __wbg_bindRenderbuffer_84ad4e2c1b3e50b2: function(arg0, arg1, arg2) {
            arg0.bindRenderbuffer(arg1 >>> 0, arg2);
        },
        __wbg_bindSampler_7259ad45d0345a23: function(arg0, arg1, arg2) {
            arg0.bindSampler(arg1 >>> 0, arg2);
        },
        __wbg_bindTexture_d4affe751f64c567: function(arg0, arg1, arg2) {
            arg0.bindTexture(arg1 >>> 0, arg2);
        },
        __wbg_bindTexture_f6ae9f2a0b12117c: function(arg0, arg1, arg2) {
            arg0.bindTexture(arg1 >>> 0, arg2);
        },
        __wbg_bindVertexArrayOES_b92f6239378bda5e: function(arg0, arg1) {
            arg0.bindVertexArrayOES(arg1);
        },
        __wbg_bindVertexArray_7dd4cc73efaa5b02: function(arg0, arg1) {
            arg0.bindVertexArray(arg1);
        },
        __wbg_blendColor_1bff6ee57033e115: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.blendColor(arg1, arg2, arg3, arg4);
        },
        __wbg_blendColor_cd047fc76ce752b0: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.blendColor(arg1, arg2, arg3, arg4);
        },
        __wbg_blendEquationSeparate_640fe636515888eb: function(arg0, arg1, arg2) {
            arg0.blendEquationSeparate(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_blendEquationSeparate_b401e331f08b4a35: function(arg0, arg1, arg2) {
            arg0.blendEquationSeparate(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_blendEquation_1dbe2aef71b7c075: function(arg0, arg1) {
            arg0.blendEquation(arg1 >>> 0);
        },
        __wbg_blendEquation_23d0345f106752af: function(arg0, arg1) {
            arg0.blendEquation(arg1 >>> 0);
        },
        __wbg_blendFuncSeparate_94c2b2c25a28ce3e: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.blendFuncSeparate(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4 >>> 0);
        },
        __wbg_blendFuncSeparate_e23244e1cc1ea452: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.blendFuncSeparate(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4 >>> 0);
        },
        __wbg_blendFunc_0836984f8f914802: function(arg0, arg1, arg2) {
            arg0.blendFunc(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_blendFunc_eb0a56441acebc3e: function(arg0, arg1, arg2) {
            arg0.blendFunc(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_blitFramebuffer_e7efe944be8d2b25: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10) {
            arg0.blitFramebuffer(arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0);
        },
        __wbg_blockSize_f2f0a46871d67efb: function(arg0) {
            const ret = arg0.blockSize;
            return ret;
        },
        __wbg_body_9a319c5d4ea2d0d8: function(arg0) {
            const ret = arg0.body;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_brand_3bc196a43eceb8af: function(arg0, arg1) {
            const ret = arg1.brand;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_brands_b7dcf262485c3e7c: function(arg0) {
            const ret = arg0.brands;
            return ret;
        },
        __wbg_bufferData_27fc020b0a028600: function(arg0, arg1, arg2, arg3) {
            arg0.bufferData(arg1 >>> 0, arg2, arg3 >>> 0);
        },
        __wbg_bufferData_611ad2765f706c85: function(arg0, arg1, arg2, arg3) {
            arg0.bufferData(arg1 >>> 0, arg2, arg3 >>> 0);
        },
        __wbg_bufferData_9cef1bde6d07b2e7: function(arg0, arg1, arg2, arg3) {
            arg0.bufferData(arg1 >>> 0, arg2, arg3 >>> 0);
        },
        __wbg_bufferData_d3f76b87295685cb: function(arg0, arg1, arg2, arg3) {
            arg0.bufferData(arg1 >>> 0, arg2, arg3 >>> 0);
        },
        __wbg_bufferSubData_11b45dd61c816637: function(arg0, arg1, arg2, arg3) {
            arg0.bufferSubData(arg1 >>> 0, arg2, arg3);
        },
        __wbg_bufferSubData_85fcbd0682ecfbe6: function(arg0, arg1, arg2, arg3) {
            arg0.bufferSubData(arg1 >>> 0, arg2, arg3);
        },
        __wbg_button_9121eff76035e6f3: function(arg0) {
            const ret = arg0.button;
            return ret;
        },
        __wbg_buttons_6d1f718b1b841b35: function(arg0) {
            const ret = arg0.buttons;
            return ret;
        },
        __wbg_buttons_c4b0491af6752e80: function(arg0) {
            const ret = arg0.buttons;
            return ret;
        },
        __wbg_call_761cb61423a6f121: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            const ret = arg0.call(arg1, arg2, arg3, arg4);
            return ret;
        }, arguments); },
        __wbg_call_a41d6421b30a32c5: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.call(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_call_a6d9545202d34317: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg0.call(arg1, arg2, arg3);
            return ret;
        }, arguments); },
        __wbg_cancelAnimationFrame_44f7b2b0c5c39988: function() { return handleError(function (arg0, arg1) {
            arg0.cancelAnimationFrame(arg1);
        }, arguments); },
        __wbg_cancelIdleCallback_babd9f2c9e0e274e: function(arg0, arg1) {
            arg0.cancelIdleCallback(arg1 >>> 0);
        },
        __wbg_cancel_65f38182e2eeac5c: function(arg0) {
            arg0.cancel();
        },
        __wbg_catch_f939343cb181958c: function(arg0, arg1) {
            const ret = arg0.catch(arg1);
            return ret;
        },
        __wbg_clearBufferfv_f3f9113132f1fcf2: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.clearBufferfv(arg1 >>> 0, arg2, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_clearBufferiv_d2f793f8673febc9: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.clearBufferiv(arg1 >>> 0, arg2, getArrayI32FromWasm0(arg3, arg4));
        },
        __wbg_clearBufferuiv_7b92c9e5c5786765: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.clearBufferuiv(arg1 >>> 0, arg2, getArrayU32FromWasm0(arg3, arg4));
        },
        __wbg_clearDepth_3856b90de145bade: function(arg0, arg1) {
            arg0.clearDepth(arg1);
        },
        __wbg_clearDepth_8bd1a97b6d503fee: function(arg0, arg1) {
            arg0.clearDepth(arg1);
        },
        __wbg_clearStencil_13383248806f46ce: function(arg0, arg1) {
            arg0.clearStencil(arg1);
        },
        __wbg_clearStencil_1e7ff35a31d7916a: function(arg0, arg1) {
            arg0.clearStencil(arg1);
        },
        __wbg_clearTimeout_491493c517cfff1c: function(arg0, arg1) {
            arg0.clearTimeout(arg1);
        },
        __wbg_clear_4ea2bcc891545cba: function(arg0, arg1) {
            arg0.clear(arg1 >>> 0);
        },
        __wbg_clear_aba32769af482a1b: function(arg0, arg1) {
            arg0.clear(arg1 >>> 0);
        },
        __wbg_clientWaitSync_5a73eb00e846b6e7: function(arg0, arg1, arg2, arg3) {
            const ret = arg0.clientWaitSync(arg1, arg2 >>> 0, arg3 >>> 0);
            return ret;
        },
        __wbg_close_1dd84b3ac8a28727: function() { return handleError(function (arg0) {
            const ret = arg0.close();
            return ret;
        }, arguments); },
        __wbg_close_f2f9163a4a555379: function(arg0) {
            arg0.close();
        },
        __wbg_code_5ad85ce0561e0bb5: function(arg0, arg1) {
            const ret = arg1.code;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_colorMask_360d34a1b73138ff: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.colorMask(arg1 !== 0, arg2 !== 0, arg3 !== 0, arg4 !== 0);
        },
        __wbg_colorMask_982ef6eda4803a18: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.colorMask(arg1 !== 0, arg2 !== 0, arg3 !== 0, arg4 !== 0);
        },
        __wbg_compileShader_50b61cd1b374d531: function(arg0, arg1) {
            arg0.compileShader(arg1);
        },
        __wbg_compileShader_bedba6a7869aa58d: function(arg0, arg1) {
            arg0.compileShader(arg1);
        },
        __wbg_compressedTexSubImage2D_79f87c415191cb5b: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8) {
            arg0.compressedTexSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8);
        },
        __wbg_compressedTexSubImage2D_a9f8677e599cf1d4: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8) {
            arg0.compressedTexSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8);
        },
        __wbg_compressedTexSubImage2D_eadf1d97b9426788: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.compressedTexSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8, arg9);
        },
        __wbg_compressedTexSubImage3D_101015bd664c7388: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.compressedTexSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10, arg11);
        },
        __wbg_compressedTexSubImage3D_fa1a576896bbdaa1: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10) {
            arg0.compressedTexSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10);
        },
        __wbg_connect_b0c6d44e9984ca8e: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.connect(arg1);
            return ret;
        }, arguments); },
        __wbg_connected_6561abd475cda6df: function(arg0) {
            const ret = arg0.connected;
            return ret;
        },
        __wbg_contains_89b774e57b8d9af4: function(arg0, arg1) {
            const ret = arg0.contains(arg1);
            return ret;
        },
        __wbg_contentRect_592c3033c92a2ee3: function(arg0) {
            const ret = arg0.contentRect;
            return ret;
        },
        __wbg_copyBufferSubData_6091c9cc936cc895: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.copyBufferSubData(arg1 >>> 0, arg2 >>> 0, arg3, arg4, arg5);
        },
        __wbg_copyTexSubImage2D_5562ca0ba8f1ef9d: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8) {
            arg0.copyTexSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8);
        },
        __wbg_copyTexSubImage2D_8950f8d58b0f216b: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8) {
            arg0.copyTexSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8);
        },
        __wbg_copyTexSubImage3D_c947f39e5a487ca6: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.copyTexSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9);
        },
        __wbg_copyToChannel_be740358a55f7ec4: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            arg0.copyToChannel(getArrayF32FromWasm0(arg1, arg2), arg3);
        }, arguments); },
        __wbg_createBufferSource_3114c0146231317b: function() { return handleError(function (arg0) {
            const ret = arg0.createBufferSource();
            return ret;
        }, arguments); },
        __wbg_createBuffer_5e53e4a1f2e73720: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg0.createBuffer(arg1 >>> 0, arg2 >>> 0, arg3);
            return ret;
        }, arguments); },
        __wbg_createBuffer_68a72615fda09cc7: function(arg0) {
            const ret = arg0.createBuffer();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createBuffer_88aa6747ef1e21b9: function(arg0) {
            const ret = arg0.createBuffer();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createElement_679cad83bb50288c: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.createElement(getStringFromWasm0(arg1, arg2));
            return ret;
        }, arguments); },
        __wbg_createFramebuffer_23e3175822f864b1: function(arg0) {
            const ret = arg0.createFramebuffer();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createFramebuffer_c2281f7a61864dc1: function(arg0) {
            const ret = arg0.createFramebuffer();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createImageBitmap_4f6b89afec926349: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.createImageBitmap(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_createObjectURL_ff4de9deb3f8d0a6: function() { return handleError(function (arg0, arg1) {
            const ret = URL.createObjectURL(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments); },
        __wbg_createProgram_932959b0abef3889: function(arg0) {
            const ret = arg0.createProgram();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createProgram_f56205ff1949c737: function(arg0) {
            const ret = arg0.createProgram();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createQuery_81134d4c0289efff: function(arg0) {
            const ret = arg0.createQuery();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createRenderbuffer_64db55d91178c45e: function(arg0) {
            const ret = arg0.createRenderbuffer();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createRenderbuffer_e1819b7725afd261: function(arg0) {
            const ret = arg0.createRenderbuffer();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createSampler_89b9dfd6d2672bdd: function(arg0) {
            const ret = arg0.createSampler();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createShader_195b98e391086cfb: function(arg0, arg1) {
            const ret = arg0.createShader(arg1 >>> 0);
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createShader_3ea04d442da25990: function(arg0, arg1) {
            const ret = arg0.createShader(arg1 >>> 0);
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createTexture_4663e5c6298a6e63: function(arg0) {
            const ret = arg0.createTexture();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createTexture_fa18817b4d49b838: function(arg0) {
            const ret = arg0.createTexture();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createVertexArrayOES_4861cd2ff06b47e8: function(arg0) {
            const ret = arg0.createVertexArrayOES();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_createVertexArray_565bc081065d93bc: function(arg0) {
            const ret = arg0.createVertexArray();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_ctrlKey_68f7b8620ddfccc8: function(arg0) {
            const ret = arg0.ctrlKey;
            return ret;
        },
        __wbg_ctrlKey_7b559591aa96b86e: function(arg0) {
            const ret = arg0.ctrlKey;
            return ret;
        },
        __wbg_cullFace_5858a2cdcb4d6678: function(arg0, arg1) {
            arg0.cullFace(arg1 >>> 0);
        },
        __wbg_cullFace_bc83cd82280de65c: function(arg0, arg1) {
            arg0.cullFace(arg1 >>> 0);
        },
        __wbg_currentTime_6bf7644ca0c23256: function(arg0) {
            const ret = arg0.currentTime;
            return ret;
        },
        __wbg_decode_0fda6acc41131019: function(arg0) {
            const ret = arg0.decode();
            return ret;
        },
        __wbg_deleteBuffer_340d7884968a79eb: function(arg0, arg1) {
            arg0.deleteBuffer(arg1);
        },
        __wbg_deleteBuffer_62138c27aeb02ca4: function(arg0, arg1) {
            arg0.deleteBuffer(arg1);
        },
        __wbg_deleteFramebuffer_9323713779c2b4c0: function(arg0, arg1) {
            arg0.deleteFramebuffer(arg1);
        },
        __wbg_deleteFramebuffer_d38950c53be54c1a: function(arg0, arg1) {
            arg0.deleteFramebuffer(arg1);
        },
        __wbg_deleteProgram_366007e5f2730fe6: function(arg0, arg1) {
            arg0.deleteProgram(arg1);
        },
        __wbg_deleteProgram_e06461448fa9fcd8: function(arg0, arg1) {
            arg0.deleteProgram(arg1);
        },
        __wbg_deleteQuery_9796d0734523df41: function(arg0, arg1) {
            arg0.deleteQuery(arg1);
        },
        __wbg_deleteRenderbuffer_74b7cdd428872286: function(arg0, arg1) {
            arg0.deleteRenderbuffer(arg1);
        },
        __wbg_deleteRenderbuffer_c423ff0c6692949e: function(arg0, arg1) {
            arg0.deleteRenderbuffer(arg1);
        },
        __wbg_deleteSampler_e4128c6eac83e159: function(arg0, arg1) {
            arg0.deleteSampler(arg1);
        },
        __wbg_deleteShader_79c915b05ea4ad40: function(arg0, arg1) {
            arg0.deleteShader(arg1);
        },
        __wbg_deleteShader_ccada46126dd1be7: function(arg0, arg1) {
            arg0.deleteShader(arg1);
        },
        __wbg_deleteSync_dfb44dc88ea1932e: function(arg0, arg1) {
            arg0.deleteSync(arg1);
        },
        __wbg_deleteTexture_6842b6a68ffbf944: function(arg0, arg1) {
            arg0.deleteTexture(arg1);
        },
        __wbg_deleteTexture_a65962a610fc9b21: function(arg0, arg1) {
            arg0.deleteTexture(arg1);
        },
        __wbg_deleteVertexArrayOES_4a422146dd3f144e: function(arg0, arg1) {
            arg0.deleteVertexArrayOES(arg1);
        },
        __wbg_deleteVertexArray_b61169e5f2c2ea0f: function(arg0, arg1) {
            arg0.deleteVertexArray(arg1);
        },
        __wbg_deltaMode_5590354c617f6678: function(arg0) {
            const ret = arg0.deltaMode;
            return ret;
        },
        __wbg_deltaX_aacd03436b6f8a73: function(arg0) {
            const ret = arg0.deltaX;
            return ret;
        },
        __wbg_deltaY_02a7c4ae29ceeff0: function(arg0) {
            const ret = arg0.deltaY;
            return ret;
        },
        __wbg_depthFunc_82a306f59663800e: function(arg0, arg1) {
            arg0.depthFunc(arg1 >>> 0);
        },
        __wbg_depthFunc_a57c17fc802d1235: function(arg0, arg1) {
            arg0.depthFunc(arg1 >>> 0);
        },
        __wbg_depthMask_41d40746e5457105: function(arg0, arg1) {
            arg0.depthMask(arg1 !== 0);
        },
        __wbg_depthMask_c3c5be00f8a01171: function(arg0, arg1) {
            arg0.depthMask(arg1 !== 0);
        },
        __wbg_depthRange_1d642629ac479679: function(arg0, arg1, arg2) {
            arg0.depthRange(arg1, arg2);
        },
        __wbg_depthRange_8cccdaa76e6e9aac: function(arg0, arg1, arg2) {
            arg0.depthRange(arg1, arg2);
        },
        __wbg_destination_a7fb84721246ff2f: function(arg0) {
            const ret = arg0.destination;
            return ret;
        },
        __wbg_devicePixelContentBoxSize_a24219b0eeafb92d: function(arg0) {
            const ret = arg0.devicePixelContentBoxSize;
            return ret;
        },
        __wbg_devicePixelRatio_3a60c85ae6458d68: function(arg0) {
            const ret = arg0.devicePixelRatio;
            return ret;
        },
        __wbg_disableVertexAttribArray_5bff9d65cf5682e0: function(arg0, arg1) {
            arg0.disableVertexAttribArray(arg1 >>> 0);
        },
        __wbg_disableVertexAttribArray_9daed4d59eb86bc4: function(arg0, arg1) {
            arg0.disableVertexAttribArray(arg1 >>> 0);
        },
        __wbg_disable_3827edd0ebc3906f: function(arg0, arg1) {
            arg0.disable(arg1 >>> 0);
        },
        __wbg_disable_b0f20ab1b990a65d: function(arg0, arg1) {
            arg0.disable(arg1 >>> 0);
        },
        __wbg_disconnect_a452e2b1ad76211b: function(arg0) {
            arg0.disconnect();
        },
        __wbg_disconnect_e719f257f8b5968f: function(arg0) {
            arg0.disconnect();
        },
        __wbg_document_69bb6a2f7927d532: function(arg0) {
            const ret = arg0.document;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_drawArraysInstancedANGLE_e78464097a007492: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.drawArraysInstancedANGLE(arg1 >>> 0, arg2, arg3, arg4);
        },
        __wbg_drawArraysInstanced_12b5ac123880f1e5: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.drawArraysInstanced(arg1 >>> 0, arg2, arg3, arg4);
        },
        __wbg_drawArrays_c160958534316d96: function(arg0, arg1, arg2, arg3) {
            arg0.drawArrays(arg1 >>> 0, arg2, arg3);
        },
        __wbg_drawArrays_d5a5cd7c06a36bac: function(arg0, arg1, arg2, arg3) {
            arg0.drawArrays(arg1 >>> 0, arg2, arg3);
        },
        __wbg_drawBuffersWEBGL_d978b4ef20df9e6e: function(arg0, arg1) {
            arg0.drawBuffersWEBGL(arg1);
        },
        __wbg_drawBuffers_5038e68debaf8a7b: function(arg0, arg1) {
            arg0.drawBuffers(arg1);
        },
        __wbg_drawElementsInstancedANGLE_bd601b8a575a0d76: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.drawElementsInstancedANGLE(arg1 >>> 0, arg2, arg3 >>> 0, arg4, arg5);
        },
        __wbg_drawElementsInstanced_a08ae5f7e875b98e: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.drawElementsInstanced(arg1 >>> 0, arg2, arg3 >>> 0, arg4, arg5);
        },
        __wbg_enableVertexAttribArray_16defb159a05d60a: function(arg0, arg1) {
            arg0.enableVertexAttribArray(arg1 >>> 0);
        },
        __wbg_enableVertexAttribArray_7d4003fc258faa30: function(arg0, arg1) {
            arg0.enableVertexAttribArray(arg1 >>> 0);
        },
        __wbg_enable_b4b249f77a13393c: function(arg0, arg1) {
            arg0.enable(arg1 >>> 0);
        },
        __wbg_enable_f95f0e6bcdef4ad4: function(arg0, arg1) {
            arg0.enable(arg1 >>> 0);
        },
        __wbg_endQuery_62edf1b38fcc333e: function(arg0, arg1) {
            arg0.endQuery(arg1 >>> 0);
        },
        __wbg_error_48655ee7e4756f8b: function(arg0) {
            console.error(arg0);
        },
        __wbg_error_825e3e7e65a41d31: function(arg0, arg1) {
            console.error(arg0, arg1);
        },
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_eval_b3ce086b62c3ca2e: function() { return handleError(function (arg0, arg1) {
            const ret = eval(getStringFromWasm0(arg0, arg1));
            return ret;
        }, arguments); },
        __wbg_exec_9e14c9a572abde98: function(arg0, arg1, arg2) {
            const ret = arg0.exec(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_exitFullscreen_8c9041386628e144: function(arg0) {
            arg0.exitFullscreen();
        },
        __wbg_exitPointerLock_9b04c08b9bd7a3ba: function(arg0) {
            arg0.exitPointerLock();
        },
        __wbg_fenceSync_09fc77121a1d209f: function(arg0, arg1, arg2) {
            const ret = arg0.fenceSync(arg1 >>> 0, arg2 >>> 0);
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_fetch_7422aa8fb42e7063: function(arg0, arg1, arg2) {
            const ret = arg0.fetch(getStringFromWasm0(arg1, arg2));
            return ret;
        },
        __wbg_fetch_dc020402ef5b5b70: function(arg0, arg1, arg2) {
            const ret = arg0.fetch(getStringFromWasm0(arg1, arg2));
            return ret;
        },
        __wbg_flush_0d413f47f0da2a94: function(arg0) {
            arg0.flush();
        },
        __wbg_flush_8de681f5248a68b9: function(arg0) {
            arg0.flush();
        },
        __wbg_focus_6fb3e144d2c12c7f: function() { return handleError(function (arg0) {
            arg0.focus();
        }, arguments); },
        __wbg_framebufferRenderbuffer_752640e03bd3d58a: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.framebufferRenderbuffer(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4);
        },
        __wbg_framebufferRenderbuffer_9f6574538b6fa528: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.framebufferRenderbuffer(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4);
        },
        __wbg_framebufferTexture2D_474e2bcbb9e69c73: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.framebufferTexture2D(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4, arg5);
        },
        __wbg_framebufferTexture2D_a4ba52d04ab93226: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.framebufferTexture2D(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4, arg5);
        },
        __wbg_framebufferTextureLayer_032548119c55333f: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.framebufferTextureLayer(arg1 >>> 0, arg2 >>> 0, arg3, arg4, arg5);
        },
        __wbg_framebufferTextureMultiviewOVR_3568fd6a3321abd2: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
            arg0.framebufferTextureMultiviewOVR(arg1 >>> 0, arg2 >>> 0, arg3, arg4, arg5, arg6);
        },
        __wbg_from_ff141b1e4c69b979: function(arg0) {
            const ret = Array.from(arg0);
            return ret;
        },
        __wbg_frontFace_040302cde4275976: function(arg0, arg1) {
            arg0.frontFace(arg1 >>> 0);
        },
        __wbg_frontFace_a50be5df32f82489: function(arg0, arg1) {
            arg0.frontFace(arg1 >>> 0);
        },
        __wbg_fullscreenElement_fd91f30160113ca8: function(arg0) {
            const ret = arg0.fullscreenElement;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_getBoundingClientRect_e0fb035288f4a416: function(arg0) {
            const ret = arg0.getBoundingClientRect();
            return ret;
        },
        __wbg_getBufferSubData_cfc147848ea9a204: function(arg0, arg1, arg2, arg3) {
            arg0.getBufferSubData(arg1 >>> 0, arg2, arg3);
        },
        __wbg_getCoalescedEvents_3e003f63d9ebbc05: function(arg0) {
            const ret = arg0.getCoalescedEvents;
            return ret;
        },
        __wbg_getCoalescedEvents_55ab8efd15ca0894: function(arg0) {
            const ret = arg0.getCoalescedEvents();
            return ret;
        },
        __wbg_getComputedStyle_041ecb5b5cae0ab8: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.getComputedStyle(arg1);
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getContext_6afffb087ba015e7: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg0.getContext(getStringFromWasm0(arg1, arg2), arg3);
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getContext_6ce4459fd5f498a9: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg0.getContext(getStringFromWasm0(arg1, arg2), arg3);
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getContext_f17252002286474d: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.getContext(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getExtension_6e629f74e6223ae8: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.getExtension(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getGamepads_015e883edab3d776: function() { return handleError(function (arg0) {
            const ret = arg0.getGamepads();
            return ret;
        }, arguments); },
        __wbg_getIndexedParameter_0dba1754b6a586e8: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.getIndexedParameter(arg1 >>> 0, arg2 >>> 0);
            return ret;
        }, arguments); },
        __wbg_getItem_f68808a9230dd173: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg1.getItem(getStringFromWasm0(arg2, arg3));
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments); },
        __wbg_getOwnPropertyDescriptor_dc88788f5dfd2fd3: function(arg0, arg1) {
            const ret = Object.getOwnPropertyDescriptor(arg0, arg1);
            return ret;
        },
        __wbg_getParameter_4249f979fb9b2034: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.getParameter(arg1 >>> 0);
            return ret;
        }, arguments); },
        __wbg_getParameter_8154b8b3c2249843: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.getParameter(arg1 >>> 0);
            return ret;
        }, arguments); },
        __wbg_getProgramInfoLog_88521473263984bd: function(arg0, arg1, arg2) {
            const ret = arg1.getProgramInfoLog(arg2);
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_getProgramInfoLog_f93553deba23cccc: function(arg0, arg1, arg2) {
            const ret = arg1.getProgramInfoLog(arg2);
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_getProgramParameter_3a2cbacda36e0528: function(arg0, arg1, arg2) {
            const ret = arg0.getProgramParameter(arg1, arg2 >>> 0);
            return ret;
        },
        __wbg_getProgramParameter_a00a3869258b814e: function(arg0, arg1, arg2) {
            const ret = arg0.getProgramParameter(arg1, arg2 >>> 0);
            return ret;
        },
        __wbg_getPropertyValue_feecd512625819d9: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg1.getPropertyValue(getStringFromWasm0(arg2, arg3));
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments); },
        __wbg_getQueryParameter_417092b320c7d84a: function(arg0, arg1, arg2) {
            const ret = arg0.getQueryParameter(arg1, arg2 >>> 0);
            return ret;
        },
        __wbg_getRandomValues_3f44b700395062e5: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_getRandomValues_ef12552bf5acd2fe: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_getShaderInfoLog_25f08216f6d590f6: function(arg0, arg1, arg2) {
            const ret = arg1.getShaderInfoLog(arg2);
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_getShaderInfoLog_b7bfd2186bdd39a2: function(arg0, arg1, arg2) {
            const ret = arg1.getShaderInfoLog(arg2);
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_getShaderParameter_96635c982831e95b: function(arg0, arg1, arg2) {
            const ret = arg0.getShaderParameter(arg1, arg2 >>> 0);
            return ret;
        },
        __wbg_getShaderParameter_d7c32caac818946c: function(arg0, arg1, arg2) {
            const ret = arg0.getShaderParameter(arg1, arg2 >>> 0);
            return ret;
        },
        __wbg_getSupportedExtensions_362130232fc99d22: function(arg0) {
            const ret = arg0.getSupportedExtensions();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_getSupportedProfiles_df08bd5d0fab9196: function(arg0) {
            const ret = arg0.getSupportedProfiles();
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_getSyncParameter_e41eea811d52b07c: function(arg0, arg1, arg2) {
            const ret = arg0.getSyncParameter(arg1, arg2 >>> 0);
            return ret;
        },
        __wbg_getUniformBlockIndex_0cfb97b93f26175b: function(arg0, arg1, arg2, arg3) {
            const ret = arg0.getUniformBlockIndex(arg1, getStringFromWasm0(arg2, arg3));
            return ret;
        },
        __wbg_getUniformLocation_1d6a81965f118597: function(arg0, arg1, arg2, arg3) {
            const ret = arg0.getUniformLocation(arg1, getStringFromWasm0(arg2, arg3));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_getUniformLocation_484ff1965b8e30f4: function(arg0, arg1, arg2, arg3) {
            const ret = arg0.getUniformLocation(arg1, getStringFromWasm0(arg2, arg3));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_get_41476db20fef99a8: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_get_652f640b3b0b6e3e: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_get_unchecked_be562b1421656321: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_has_3a6f31f647e0ba22: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.has(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_height_1d58cd47763299ec: function(arg0) {
            const ret = arg0.height;
            return ret;
        },
        __wbg_id_1411dbd0a5cdd1b3: function(arg0, arg1) {
            const ret = arg1.id;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_includes_169ece041f52c741: function(arg0, arg1, arg2) {
            const ret = arg0.includes(arg1, arg2);
            return ret;
        },
        __wbg_index_a59049b6b01dcc35: function(arg0) {
            const ret = arg0.index;
            return ret;
        },
        __wbg_inlineSize_b124532195785ca4: function(arg0) {
            const ret = arg0.inlineSize;
            return ret;
        },
        __wbg_instanceof_DomException_47098be3333e16f8: function(arg0) {
            let result;
            try {
                result = arg0 instanceof DOMException;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_HtmlCanvasElement_0ac74d5643067956: function(arg0) {
            let result;
            try {
                result = arg0 instanceof HTMLCanvasElement;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Response_370b83aa6c17e88a: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Response;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_WebGl2RenderingContext_fbfd73b8b9465e2d: function(arg0) {
            let result;
            try {
                result = arg0 instanceof WebGL2RenderingContext;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Window_4153c1818a1c0c0b: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Window;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_invalidateFramebuffer_f64698548fae8275: function() { return handleError(function (arg0, arg1, arg2) {
            arg0.invalidateFramebuffer(arg1 >>> 0, arg2);
        }, arguments); },
        __wbg_isIntersecting_bb0a21a1d5eed17b: function(arg0) {
            const ret = arg0.isIntersecting;
            return ret;
        },
        __wbg_isSecureContext_b0e9fd274e04b7b7: function(arg0) {
            const ret = arg0.isSecureContext;
            return ret;
        },
        __wbg_is_e9826d240a8d86ea: function(arg0, arg1) {
            const ret = Object.is(arg0, arg1);
            return ret;
        },
        __wbg_key_2e79b9dbd4550ab3: function(arg0, arg1) {
            const ret = arg1.key;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_key_b3963de1608adbbf: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg1.key(arg2 >>> 0);
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments); },
        __wbg_length_0a6ce016dc1460b0: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_ba3c032602efe310: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_e79ec247c0f923cd: function() { return handleError(function (arg0) {
            const ret = arg0.length;
            return ret;
        }, arguments); },
        __wbg_linkProgram_76940d17b54d375b: function(arg0, arg1) {
            arg0.linkProgram(arg1);
        },
        __wbg_linkProgram_ba72b321b45bac4c: function(arg0, arg1) {
            arg0.linkProgram(arg1);
        },
        __wbg_localStorage_11b5275c3ad2bab7: function() { return handleError(function (arg0) {
            const ret = arg0.localStorage;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_location_d080430e3f643f93: function(arg0) {
            const ret = arg0.location;
            return ret;
        },
        __wbg_log_0c201ade58bb55e1: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.log(getStringFromWasm0(arg0, arg1), getStringFromWasm0(arg2, arg3), getStringFromWasm0(arg4, arg5), getStringFromWasm0(arg6, arg7));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_log_ce2c4456b290c5e7: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.log(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_mapping_8ee611f6b9432478: function(arg0) {
            const ret = arg0.mapping;
            return (__wbindgen_enum_GamepadMappingType.indexOf(ret) + 1 || 3) - 1;
        },
        __wbg_mark_b4d943f3bc2d2404: function(arg0, arg1) {
            performance.mark(getStringFromWasm0(arg0, arg1));
        },
        __wbg_matchMedia_2b8a11e10a1d403d: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.matchMedia(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_matches_8fcf21e9ec34186b: function(arg0) {
            const ret = arg0.matches;
            return ret;
        },
        __wbg_maxChannelCount_4ce748bd1b924aaa: function(arg0) {
            const ret = arg0.maxChannelCount;
            return ret;
        },
        __wbg_measure_84362959e621a2c1: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            let deferred0_0;
            let deferred0_1;
            let deferred1_0;
            let deferred1_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                deferred1_0 = arg2;
                deferred1_1 = arg3;
                performance.measure(getStringFromWasm0(arg0, arg1), getStringFromWasm0(arg2, arg3));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
                wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
            }
        }, arguments); },
        __wbg_media_d5208759213aa162: function(arg0, arg1) {
            const ret = arg1.media;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_message_609b498da776cb30: function(arg0, arg1) {
            const ret = arg1.message;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_metaKey_ef659f8598121617: function(arg0) {
            const ret = arg0.metaKey;
            return ret;
        },
        __wbg_metaKey_f8e5beafe081f6d6: function(arg0) {
            const ret = arg0.metaKey;
            return ret;
        },
        __wbg_movementX_234cea13fe25dae4: function(arg0) {
            const ret = arg0.movementX;
            return ret;
        },
        __wbg_movementY_3a54512f6f23708b: function(arg0) {
            const ret = arg0.movementY;
            return ret;
        },
        __wbg_navigator_f3468c6dc9006b7c: function(arg0) {
            const ret = arg0.navigator;
            return ret;
        },
        __wbg_new_1633148561079d11: function(arg0, arg1, arg2, arg3) {
            const ret = new RegExp(getStringFromWasm0(arg0, arg1), getStringFromWasm0(arg2, arg3));
            return ret;
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_new_23949f1619fea73e: function() { return handleError(function () {
            const ret = new Image();
            return ret;
        }, arguments); },
        __wbg_new_251d7024c1a6e78b: function() { return handleError(function () {
            const ret = new MessageChannel();
            return ret;
        }, arguments); },
        __wbg_new_2fad8ca02fd00684: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_3baa8d9866155c79: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_new_51ff470dc2f61e27: function() { return handleError(function () {
            const ret = new AbortController();
            return ret;
        }, arguments); },
        __wbg_new_8454eee672b2ba6e: function(arg0) {
            const ret = new Uint8Array(arg0);
            return ret;
        },
        __wbg_new_9d5d53f7ab22b9f2: function() { return handleError(function (arg0) {
            const ret = new IntersectionObserver(arg0);
            return ret;
        }, arguments); },
        __wbg_new_9e1e0aabf3119786: function() { return handleError(function (arg0, arg1) {
            const ret = new Worker(getStringFromWasm0(arg0, arg1));
            return ret;
        }, arguments); },
        __wbg_new_c43478ae1b0a5028: function() { return handleError(function (arg0) {
            const ret = new ResizeObserver(arg0);
            return ret;
        }, arguments); },
        __wbg_new_with_context_options_35b54106e5e7abc6: function() { return handleError(function (arg0) {
            const ret = new lAudioContext(arg0);
            return ret;
        }, arguments); },
        __wbg_new_with_str_sequence_and_options_d582f60b3b1caf49: function() { return handleError(function (arg0, arg1) {
            const ret = new Blob(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_new_with_u8_clamped_array_a04fccdf314e082f: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = new ImageData(getClampedArrayU8FromWasm0(arg0, arg1), arg2 >>> 0);
            return ret;
        }, arguments); },
        __wbg_now_4f457f10f864aec5: function() {
            const ret = Date.now();
            return ret;
        },
        __wbg_now_b205f8c23840112e: function(arg0) {
            const ret = arg0.now();
            return ret;
        },
        __wbg_now_e7c6795a7f81e10f: function(arg0) {
            const ret = arg0.now();
            return ret;
        },
        __wbg_observe_08575843bc0fb2e0: function(arg0, arg1) {
            arg0.observe(arg1);
        },
        __wbg_observe_7f96207e77a944cc: function(arg0, arg1, arg2) {
            arg0.observe(arg1, arg2);
        },
        __wbg_observe_eb7083d82d325b9f: function(arg0, arg1) {
            arg0.observe(arg1);
        },
        __wbg_of_96154841226db59c: function(arg0, arg1) {
            const ret = Array.of(arg0, arg1);
            return ret;
        },
        __wbg_of_cc555051dc9558d3: function(arg0) {
            const ret = Array.of(arg0);
            return ret;
        },
        __wbg_offsetX_a9bf2ea7f0575ac9: function(arg0) {
            const ret = arg0.offsetX;
            return ret;
        },
        __wbg_offsetY_10e5433a1bbd4c01: function(arg0) {
            const ret = arg0.offsetY;
            return ret;
        },
        __wbg_parse_342d5616e14beccc: function() { return handleError(function (arg0, arg1) {
            const ret = JSON.parse(getStringFromWasm0(arg0, arg1));
            return ret;
        }, arguments); },
        __wbg_performance_3fcf6e32a7e1ed0a: function(arg0) {
            const ret = arg0.performance;
            return ret;
        },
        __wbg_performance_8e9fec534a95f99f: function(arg0) {
            const ret = arg0.performance;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_persisted_e198bc1b0ea7bac3: function(arg0) {
            const ret = arg0.persisted;
            return ret;
        },
        __wbg_pixelStorei_7feec34442803b9d: function(arg0, arg1, arg2) {
            arg0.pixelStorei(arg1 >>> 0, arg2);
        },
        __wbg_pixelStorei_c1200ded9741bf0c: function(arg0, arg1, arg2) {
            arg0.pixelStorei(arg1 >>> 0, arg2);
        },
        __wbg_play_3997a1be51d27925: function(arg0) {
            arg0.play();
        },
        __wbg_pointerId_18e43d42a0114b4d: function(arg0) {
            const ret = arg0.pointerId;
            return ret;
        },
        __wbg_pointerType_379748804334ff14: function(arg0, arg1) {
            const ret = arg1.pointerType;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_polygonOffset_47749ec8af0d2b41: function(arg0, arg1, arg2) {
            arg0.polygonOffset(arg1, arg2);
        },
        __wbg_polygonOffset_b95607b79068742b: function(arg0, arg1, arg2) {
            arg0.polygonOffset(arg1, arg2);
        },
        __wbg_port1_f00f1bead0ea7c97: function(arg0) {
            const ret = arg0.port1;
            return ret;
        },
        __wbg_port2_302d3e211aa10c79: function(arg0) {
            const ret = arg0.port2;
            return ret;
        },
        __wbg_postMessage_0613bb9fa4d46b40: function() { return handleError(function (arg0, arg1) {
            arg0.postMessage(arg1);
        }, arguments); },
        __wbg_postMessage_af4c9caebcb4a6ba: function() { return handleError(function (arg0, arg1, arg2) {
            arg0.postMessage(arg1, arg2);
        }, arguments); },
        __wbg_postTask_e2439afddcdfbb55: function(arg0, arg1, arg2) {
            const ret = arg0.postTask(arg1, arg2);
            return ret;
        },
        __wbg_pressed_f3474d2085f7d3f1: function(arg0) {
            const ret = arg0.pressed;
            return ret;
        },
        __wbg_pressure_2c261bc55ae4a3af: function(arg0) {
            const ret = arg0.pressure;
            return ret;
        },
        __wbg_preventDefault_2c34c219d9b04b86: function(arg0) {
            arg0.preventDefault();
        },
        __wbg_prototype_0d5bb2023db3bcfc: function() {
            const ret = ResizeObserverEntry.prototype;
            return ret;
        },
        __wbg_prototypesetcall_fd4050e806e1d519: function(arg0, arg1, arg2) {
            Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
        },
        __wbg_push_60a5366c0bb22a7d: function(arg0, arg1) {
            const ret = arg0.push(arg1);
            return ret;
        },
        __wbg_queryCounterEXT_59f99c87fee637c5: function(arg0, arg1, arg2) {
            arg0.queryCounterEXT(arg1, arg2 >>> 0);
        },
        __wbg_querySelector_a3b1f840e2672b49: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.querySelector(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_queueMicrotask_40ac6ffc2848ba77: function(arg0) {
            queueMicrotask(arg0);
        },
        __wbg_queueMicrotask_55a0060f6d1a75bc: function(arg0, arg1) {
            arg0.queueMicrotask(arg1);
        },
        __wbg_queueMicrotask_74d092439f6494c1: function(arg0) {
            const ret = arg0.queueMicrotask;
            return ret;
        },
        __wbg_random_fc287e2ecb3e2805: function() {
            const ret = Math.random();
            return ret;
        },
        __wbg_readBuffer_84ed375e14adc17b: function(arg0, arg1) {
            arg0.readBuffer(arg1 >>> 0);
        },
        __wbg_readPixels_11033ecd686150e1: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
            arg0.readPixels(arg1, arg2, arg3, arg4, arg5 >>> 0, arg6 >>> 0, arg7);
        }, arguments); },
        __wbg_readPixels_2a027d81502b271d: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
            arg0.readPixels(arg1, arg2, arg3, arg4, arg5 >>> 0, arg6 >>> 0, arg7);
        }, arguments); },
        __wbg_readPixels_4b968779f2667722: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
            arg0.readPixels(arg1, arg2, arg3, arg4, arg5 >>> 0, arg6 >>> 0, arg7);
        }, arguments); },
        __wbg_removeEventListener_2ce4c0697d2b692c: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            arg0.removeEventListener(getStringFromWasm0(arg1, arg2), arg3);
        }, arguments); },
        __wbg_removeItem_a5faee82be5c6ed1: function() { return handleError(function (arg0, arg1, arg2) {
            arg0.removeItem(getStringFromWasm0(arg1, arg2));
        }, arguments); },
        __wbg_removeListener_fa2197adb613b1e7: function() { return handleError(function (arg0, arg1) {
            arg0.removeListener(arg1);
        }, arguments); },
        __wbg_removeProperty_de2dc5ce92bc1069: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg1.removeProperty(getStringFromWasm0(arg2, arg3));
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments); },
        __wbg_renderbufferStorageMultisample_9da92038eb665169: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.renderbufferStorageMultisample(arg1 >>> 0, arg2, arg3 >>> 0, arg4, arg5);
        },
        __wbg_renderbufferStorage_05386df6e2563674: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.renderbufferStorage(arg1 >>> 0, arg2 >>> 0, arg3, arg4);
        },
        __wbg_renderbufferStorage_d6a0a682d9abfb81: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.renderbufferStorage(arg1 >>> 0, arg2 >>> 0, arg3, arg4);
        },
        __wbg_repeat_44413ad530bd5bfb: function(arg0) {
            const ret = arg0.repeat;
            return ret;
        },
        __wbg_requestAnimationFrame_d187174d7b146805: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.requestAnimationFrame(arg1);
            return ret;
        }, arguments); },
        __wbg_requestFullscreen_3f16e43f398ce624: function(arg0) {
            const ret = arg0.requestFullscreen();
            return ret;
        },
        __wbg_requestFullscreen_b977a3a0697e883c: function(arg0) {
            const ret = arg0.requestFullscreen;
            return ret;
        },
        __wbg_requestIdleCallback_3689e3e38f6cfc02: function(arg0) {
            const ret = arg0.requestIdleCallback;
            return ret;
        },
        __wbg_requestIdleCallback_77b25045445ff3e1: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.requestIdleCallback(arg1);
            return ret;
        }, arguments); },
        __wbg_requestPointerLock_7dbfa94574f241c1: function(arg0) {
            arg0.requestPointerLock();
        },
        __wbg_resolve_9feb5d906ca62419: function(arg0) {
            const ret = Promise.resolve(arg0);
            return ret;
        },
        __wbg_resume_60c7fdf589dd7208: function() { return handleError(function (arg0) {
            const ret = arg0.resume();
            return ret;
        }, arguments); },
        __wbg_revokeObjectURL_d718fc1cb4e2de0c: function() { return handleError(function (arg0, arg1) {
            URL.revokeObjectURL(getStringFromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_samplerParameterf_178aec788cd2ecdc: function(arg0, arg1, arg2, arg3) {
            arg0.samplerParameterf(arg1, arg2 >>> 0, arg3);
        },
        __wbg_samplerParameteri_e3b690956f1fe1b3: function(arg0, arg1, arg2, arg3) {
            arg0.samplerParameteri(arg1, arg2 >>> 0, arg3);
        },
        __wbg_scheduler_a17d41c9c822fc26: function(arg0) {
            const ret = arg0.scheduler;
            return ret;
        },
        __wbg_scheduler_b35fe73ba70e89cc: function(arg0) {
            const ret = arg0.scheduler;
            return ret;
        },
        __wbg_scissor_219285a5ff24f19f: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.scissor(arg1, arg2, arg3, arg4);
        },
        __wbg_scissor_927c37be50cfe886: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.scissor(arg1, arg2, arg3, arg4);
        },
        __wbg_sessionStorage_b8279afa7561137a: function() { return handleError(function (arg0) {
            const ret = arg0.sessionStorage;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_setAttribute_50dcf32d70e1628c: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            arg0.setAttribute(getStringFromWasm0(arg1, arg2), getStringFromWasm0(arg3, arg4));
        }, arguments); },
        __wbg_setItem_bb1a692eb19d66d0: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            arg0.setItem(getStringFromWasm0(arg1, arg2), getStringFromWasm0(arg3, arg4));
        }, arguments); },
        __wbg_setPointerCapture_2b94acd286b2f0af: function() { return handleError(function (arg0, arg1) {
            arg0.setPointerCapture(arg1);
        }, arguments); },
        __wbg_setProperty_d6673329a267577b: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            arg0.setProperty(getStringFromWasm0(arg1, arg2), getStringFromWasm0(arg3, arg4));
        }, arguments); },
        __wbg_setTimeout_5649894f2c7b3d11: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.setTimeout(arg1);
            return ret;
        }, arguments); },
        __wbg_setTimeout_d007c6f72100a5e1: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.setTimeout(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_5337f8ac82364a3f: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.set(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_box_94a804a5889d01da: function(arg0, arg1) {
            arg0.box = __wbindgen_enum_ResizeObserverBoxOptions[arg1];
        },
        __wbg_set_buffer_1bd0e833202ec144: function(arg0, arg1) {
            arg0.buffer = arg1;
        },
        __wbg_set_channelCount_c9f1afa2fb6e852b: function(arg0, arg1) {
            arg0.channelCount = arg1 >>> 0;
        },
        __wbg_set_cursor_8d686ff9dd99a325: function(arg0, arg1, arg2) {
            arg0.cursor = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_duration_bfef0b021dc8fd5b: function(arg0, arg1) {
            arg0.duration = arg1;
        },
        __wbg_set_height_77937c921db92223: function(arg0, arg1) {
            arg0.height = arg1 >>> 0;
        },
        __wbg_set_height_89a4ecd0f9cc3dfa: function(arg0, arg1) {
            arg0.height = arg1 >>> 0;
        },
        __wbg_set_iterations_b84d4d3302a291a0: function(arg0, arg1) {
            arg0.iterations = arg1;
        },
        __wbg_set_onended_7c5645e29c4e6eed: function(arg0, arg1) {
            arg0.onended = arg1;
        },
        __wbg_set_onmessage_36055e0a870abd64: function(arg0, arg1) {
            arg0.onmessage = arg1;
        },
        __wbg_set_premultiply_alpha_c2eaad433252efda: function(arg0, arg1) {
            arg0.premultiplyAlpha = __wbindgen_enum_PremultiplyAlpha[arg1];
        },
        __wbg_set_sample_rate_2684ef2c0111ef24: function(arg0, arg1) {
            arg0.sampleRate = arg1;
        },
        __wbg_set_src_437acc9e665412cd: function(arg0, arg1, arg2) {
            arg0.src = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_type_9cc8db71b8673ad7: function(arg0, arg1, arg2) {
            arg0.type = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_width_d2ec5d6689655fa9: function(arg0, arg1) {
            arg0.width = arg1 >>> 0;
        },
        __wbg_set_width_da52058a27694474: function(arg0, arg1) {
            arg0.width = arg1 >>> 0;
        },
        __wbg_shaderSource_0aa654ee0e007aa6: function(arg0, arg1, arg2, arg3) {
            arg0.shaderSource(arg1, getStringFromWasm0(arg2, arg3));
        },
        __wbg_shaderSource_d9de9139056756aa: function(arg0, arg1, arg2, arg3) {
            arg0.shaderSource(arg1, getStringFromWasm0(arg2, arg3));
        },
        __wbg_shiftKey_2380f1b5c0ab0a0d: function(arg0) {
            const ret = arg0.shiftKey;
            return ret;
        },
        __wbg_shiftKey_8896b6760df23dca: function(arg0) {
            const ret = arg0.shiftKey;
            return ret;
        },
        __wbg_signal_4643ce883b92b553: function(arg0) {
            const ret = arg0.signal;
            return ret;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_start_5f13015d0fce472e: function(arg0) {
            arg0.start();
        },
        __wbg_start_cd7b8ea71ca7ac8a: function() { return handleError(function (arg0, arg1) {
            arg0.start(arg1);
        }, arguments); },
        __wbg_static_accessor_GLOBAL_THIS_1c7f1bd6c6941fdb: function() {
            const ret = typeof globalThis === 'undefined' ? null : globalThis;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_GLOBAL_e039bc914f83e74e: function() {
            const ret = typeof global === 'undefined' ? null : global;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_SELF_8bf8c48c28420ad5: function() {
            const ret = typeof self === 'undefined' ? null : self;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_WINDOW_6aeee9b51652ee0f: function() {
            const ret = typeof window === 'undefined' ? null : window;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_status_157e67ab07d01f8a: function(arg0) {
            const ret = arg0.status;
            return ret;
        },
        __wbg_stencilFuncSeparate_4530c49bf8cb1460: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.stencilFuncSeparate(arg1 >>> 0, arg2 >>> 0, arg3, arg4 >>> 0);
        },
        __wbg_stencilFuncSeparate_bf34f60e3f110bfe: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.stencilFuncSeparate(arg1 >>> 0, arg2 >>> 0, arg3, arg4 >>> 0);
        },
        __wbg_stencilMaskSeparate_229cbef7cc83cadb: function(arg0, arg1, arg2) {
            arg0.stencilMaskSeparate(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_stencilMaskSeparate_9b1653193ff288f7: function(arg0, arg1, arg2) {
            arg0.stencilMaskSeparate(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_stencilMask_8c221e4c375209c5: function(arg0, arg1) {
            arg0.stencilMask(arg1 >>> 0);
        },
        __wbg_stencilMask_c5d4a74ffb068fe9: function(arg0, arg1) {
            arg0.stencilMask(arg1 >>> 0);
        },
        __wbg_stencilOpSeparate_3a474db0945a2c9e: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.stencilOpSeparate(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4 >>> 0);
        },
        __wbg_stencilOpSeparate_f9ac7d0ce34b49cc: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.stencilOpSeparate(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4 >>> 0);
        },
        __wbg_stringify_7fd5cae8859a6f10: function() { return handleError(function (arg0) {
            const ret = JSON.stringify(arg0);
            return ret;
        }, arguments); },
        __wbg_style_ad734f3851a343fb: function(arg0) {
            const ret = arg0.style;
            return ret;
        },
        __wbg_texImage2D_1d87cc5a34709e21: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texImage2D_8325ec05b789d75e: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texImage2D_bd39197f40b2fcce: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texImage3D_b99062125306e0a5: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10) {
            arg0.texImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8 >>> 0, arg9 >>> 0, arg10);
        }, arguments); },
        __wbg_texImage3D_cc1e3c97cd187460: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10) {
            arg0.texImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8 >>> 0, arg9 >>> 0, arg10);
        }, arguments); },
        __wbg_texParameteri_4a0747bf8e13f69d: function(arg0, arg1, arg2, arg3) {
            arg0.texParameteri(arg1 >>> 0, arg2 >>> 0, arg3);
        },
        __wbg_texParameteri_9e9659537a5f6420: function(arg0, arg1, arg2, arg3) {
            arg0.texParameteri(arg1 >>> 0, arg2 >>> 0, arg3);
        },
        __wbg_texStorage2D_68a718b3fe4fe8e1: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.texStorage2D(arg1 >>> 0, arg2, arg3 >>> 0, arg4, arg5);
        },
        __wbg_texStorage3D_8ddd8de7b3efc66d: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
            arg0.texStorage3D(arg1 >>> 0, arg2, arg3 >>> 0, arg4, arg5, arg6);
        },
        __wbg_texSubImage2D_050bb40fcaf0d432: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_10b80906c76b2340: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_316bed6ee52b841d: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_3422d34fb3b08ab7: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_8c565ab572b8e793: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_96f5b172e2bd5235: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_e474295e2473c615: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage2D_fd8f22b27fcc3390: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) {
            arg0.texSubImage2D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7 >>> 0, arg8 >>> 0, arg9);
        }, arguments); },
        __wbg_texSubImage3D_02cd8e0ce4a498bf: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_texSubImage3D_286dba65215a1ed5: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_texSubImage3D_63d52a5f007110c2: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_texSubImage3D_70bf1337a948082e: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_texSubImage3D_71d4eaf8afa1000b: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_texSubImage3D_8285b442f7afc502: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_texSubImage3D_aba4a822ce927a93: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10, arg11) {
            arg0.texSubImage3D(arg1 >>> 0, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9 >>> 0, arg10 >>> 0, arg11);
        }, arguments); },
        __wbg_then_20a157d939b514f5: function(arg0, arg1) {
            const ret = arg0.then(arg1);
            return ret;
        },
        __wbg_then_5ef9b762bc91555c: function(arg0, arg1, arg2) {
            const ret = arg0.then(arg1, arg2);
            return ret;
        },
        __wbg_toBlob_c93e74084edeb70e: function() { return handleError(function (arg0, arg1) {
            arg0.toBlob(arg1);
        }, arguments); },
        __wbg_transferFromImageBitmap_0c5883d05363f361: function(arg0, arg1) {
            arg0.transferFromImageBitmap(arg1);
        },
        __wbg_uniform1f_d9aa0dc2f3d488ff: function(arg0, arg1, arg2) {
            arg0.uniform1f(arg1, arg2);
        },
        __wbg_uniform1f_ea4312ab8da5d8c4: function(arg0, arg1, arg2) {
            arg0.uniform1f(arg1, arg2);
        },
        __wbg_uniform1i_8901d038c64b0846: function(arg0, arg1, arg2) {
            arg0.uniform1i(arg1, arg2);
        },
        __wbg_uniform1i_bbb9a97ff88cb229: function(arg0, arg1, arg2) {
            arg0.uniform1i(arg1, arg2);
        },
        __wbg_uniform1ui_567e99d35204c615: function(arg0, arg1, arg2) {
            arg0.uniform1ui(arg1, arg2 >>> 0);
        },
        __wbg_uniform2fv_2ac9861002424218: function(arg0, arg1, arg2, arg3) {
            arg0.uniform2fv(arg1, getArrayF32FromWasm0(arg2, arg3));
        },
        __wbg_uniform2fv_fc947a484cd09cba: function(arg0, arg1, arg2, arg3) {
            arg0.uniform2fv(arg1, getArrayF32FromWasm0(arg2, arg3));
        },
        __wbg_uniform2iv_1d17307290cff22b: function(arg0, arg1, arg2, arg3) {
            arg0.uniform2iv(arg1, getArrayI32FromWasm0(arg2, arg3));
        },
        __wbg_uniform2iv_a40dabbc376f9258: function(arg0, arg1, arg2, arg3) {
            arg0.uniform2iv(arg1, getArrayI32FromWasm0(arg2, arg3));
        },
        __wbg_uniform2uiv_ea3846a859bc1b16: function(arg0, arg1, arg2, arg3) {
            arg0.uniform2uiv(arg1, getArrayU32FromWasm0(arg2, arg3));
        },
        __wbg_uniform3fv_4c3ad296700bc6d2: function(arg0, arg1, arg2, arg3) {
            arg0.uniform3fv(arg1, getArrayF32FromWasm0(arg2, arg3));
        },
        __wbg_uniform3fv_4c4762e638099fa9: function(arg0, arg1, arg2, arg3) {
            arg0.uniform3fv(arg1, getArrayF32FromWasm0(arg2, arg3));
        },
        __wbg_uniform3iv_2a7a198f04b3402d: function(arg0, arg1, arg2, arg3) {
            arg0.uniform3iv(arg1, getArrayI32FromWasm0(arg2, arg3));
        },
        __wbg_uniform3iv_aa32a164a3182218: function(arg0, arg1, arg2, arg3) {
            arg0.uniform3iv(arg1, getArrayI32FromWasm0(arg2, arg3));
        },
        __wbg_uniform3uiv_c09a04d6f6c79d84: function(arg0, arg1, arg2, arg3) {
            arg0.uniform3uiv(arg1, getArrayU32FromWasm0(arg2, arg3));
        },
        __wbg_uniform4f_2e8758dde1755426: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.uniform4f(arg1, arg2, arg3, arg4, arg5);
        },
        __wbg_uniform4f_4fa9b0e1d5e37cc8: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.uniform4f(arg1, arg2, arg3, arg4, arg5);
        },
        __wbg_uniform4fv_24ac5b11edbfa9f7: function(arg0, arg1, arg2, arg3) {
            arg0.uniform4fv(arg1, getArrayF32FromWasm0(arg2, arg3));
        },
        __wbg_uniform4fv_2e2ddfcf5a547136: function(arg0, arg1, arg2, arg3) {
            arg0.uniform4fv(arg1, getArrayF32FromWasm0(arg2, arg3));
        },
        __wbg_uniform4iv_2103c8a85a8b0dd8: function(arg0, arg1, arg2, arg3) {
            arg0.uniform4iv(arg1, getArrayI32FromWasm0(arg2, arg3));
        },
        __wbg_uniform4iv_3cb8853c728f9a45: function(arg0, arg1, arg2, arg3) {
            arg0.uniform4iv(arg1, getArrayI32FromWasm0(arg2, arg3));
        },
        __wbg_uniform4uiv_46ee978fe8703aaf: function(arg0, arg1, arg2, arg3) {
            arg0.uniform4uiv(arg1, getArrayU32FromWasm0(arg2, arg3));
        },
        __wbg_uniformBlockBinding_bcefd2aef80c40ab: function(arg0, arg1, arg2, arg3) {
            arg0.uniformBlockBinding(arg1, arg2 >>> 0, arg3 >>> 0);
        },
        __wbg_uniformMatrix2fv_0c4f0f8be58e53fc: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix2fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix2fv_a832f1d01c1474e0: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix2fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix2x3fv_4751a02fab689bba: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix2x3fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix2x4fv_d5869e7ed3ec9948: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix2x4fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix3fv_18b77dec8d4083f6: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix3fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix3fv_37240e6bf86a07fe: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix3fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix3x2fv_5d97f011461fbdcd: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix3x2fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix3x4fv_c04455753c617f36: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix3x4fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix4fv_0669f12fa9ed38ab: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix4fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix4fv_174a0c07d7d262e6: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix4fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix4x2fv_52bb86fa40a5d268: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix4x2fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_uniformMatrix4x3fv_505928f7d73da1ba: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.uniformMatrix4x3fv(arg1, arg2 !== 0, getArrayF32FromWasm0(arg3, arg4));
        },
        __wbg_unobserve_4f22511e56c05d64: function(arg0, arg1) {
            arg0.unobserve(arg1);
        },
        __wbg_useProgram_330a8a331113dc40: function(arg0, arg1) {
            arg0.useProgram(arg1);
        },
        __wbg_useProgram_72d15c6d8466e299: function(arg0, arg1) {
            arg0.useProgram(arg1);
        },
        __wbg_userAgentData_31b8f893e8977e94: function(arg0) {
            const ret = arg0.userAgentData;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_userAgent_08b9a244999ff008: function() { return handleError(function (arg0, arg1) {
            const ret = arg1.userAgent;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments); },
        __wbg_value_5aded02f5e9f705d: function(arg0) {
            const ret = arg0.value;
            return ret;
        },
        __wbg_vertexAttribDivisorANGLE_1bec2625956dfe3e: function(arg0, arg1, arg2) {
            arg0.vertexAttribDivisorANGLE(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_vertexAttribDivisor_6b78656d66a0b972: function(arg0, arg1, arg2) {
            arg0.vertexAttribDivisor(arg1 >>> 0, arg2 >>> 0);
        },
        __wbg_vertexAttribIPointer_d7e970f0df5969cf: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.vertexAttribIPointer(arg1 >>> 0, arg2, arg3 >>> 0, arg4, arg5);
        },
        __wbg_vertexAttribPointer_53d25cb342bec3e0: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
            arg0.vertexAttribPointer(arg1 >>> 0, arg2, arg3 >>> 0, arg4 !== 0, arg5, arg6);
        },
        __wbg_vertexAttribPointer_734b53a3b8f492ca: function(arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
            arg0.vertexAttribPointer(arg1 >>> 0, arg2, arg3 >>> 0, arg4 !== 0, arg5, arg6);
        },
        __wbg_viewport_454df83d0d2cf558: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.viewport(arg1, arg2, arg3, arg4);
        },
        __wbg_viewport_d56ad9cd4b4e71ca: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.viewport(arg1, arg2, arg3, arg4);
        },
        __wbg_visibilityState_141b4fe0a806927f: function(arg0) {
            const ret = arg0.visibilityState;
            return (__wbindgen_enum_VisibilityState.indexOf(ret) + 1 || 3) - 1;
        },
        __wbg_webkitExitFullscreen_f487871f11a8185e: function(arg0) {
            arg0.webkitExitFullscreen();
        },
        __wbg_webkitFullscreenElement_4055d847f8ff064e: function(arg0) {
            const ret = arg0.webkitFullscreenElement;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_webkitRequestFullscreen_c4ec4df7be373ffd: function(arg0) {
            arg0.webkitRequestFullscreen();
        },
        __wbg_width_7b9880491bd7c987: function(arg0) {
            const ret = arg0.width;
            return ret;
        },
        __wbg_x_a513ba6369340a5f: function(arg0) {
            const ret = arg0.x;
            return ret;
        },
        __wbg_y_21b349c4a04a6c1a: function(arg0) {
            const ret = arg0.y;
            return ret;
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Externref], shim_idx: 107877, ret: Result(Unit), inner_ret: Some(Result(Unit)) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h899eb88a471b4e55);
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Externref], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0);
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("Array<any>"), NamedExternref("ResizeObserver")], shim_idx: 19728, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h8999e935b7f0f365);
            return ret;
        },
        __wbindgen_cast_0000000000000004: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("Array<any>")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_3);
            return ret;
        },
        __wbindgen_cast_0000000000000005: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("Event")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_4);
            return ret;
        },
        __wbindgen_cast_0000000000000006: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("FocusEvent")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_5);
            return ret;
        },
        __wbindgen_cast_0000000000000007: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("KeyboardEvent")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_6);
            return ret;
        },
        __wbindgen_cast_0000000000000008: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("PageTransitionEvent")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_7);
            return ret;
        },
        __wbindgen_cast_0000000000000009: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("PointerEvent")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_8);
            return ret;
        },
        __wbindgen_cast_000000000000000a: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("WheelEvent")], shim_idx: 19723, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_9);
            return ret;
        },
        __wbindgen_cast_000000000000000b: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Option(NamedExternref("Blob"))], shim_idx: 19733, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__hf5047f926a8b7c42);
            return ret;
        },
        __wbindgen_cast_000000000000000c: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [], shim_idx: 19736, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h2c42e7ad3ef275c0);
            return ret;
        },
        __wbindgen_cast_000000000000000d: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [], shim_idx: 57953, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__ha1939d9ab52a5a4b);
            return ret;
        },
        __wbindgen_cast_000000000000000e: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [], shim_idx: 99388, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h192dd3c69c0111b0);
            return ret;
        },
        __wbindgen_cast_000000000000000f: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000010: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(F32)) -> NamedExternref("Float32Array")`.
            const ret = getArrayF32FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000011: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(I16)) -> NamedExternref("Int16Array")`.
            const ret = getArrayI16FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000012: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(I32)) -> NamedExternref("Int32Array")`.
            const ret = getArrayI32FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000013: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(I8)) -> NamedExternref("Int8Array")`.
            const ret = getArrayI8FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000014: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(U16)) -> NamedExternref("Uint16Array")`.
            const ret = getArrayU16FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000015: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(U32)) -> NamedExternref("Uint32Array")`.
            const ret = getArrayU32FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000016: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(U8)) -> NamedExternref("Uint8Array")`.
            const ret = getArrayU8FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000017: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./project-phoenix_bg.js": import0,
    };
}

const lAudioContext = (typeof AudioContext !== 'undefined' ? AudioContext : (typeof webkitAudioContext !== 'undefined' ? webkitAudioContext : undefined));
function wasm_bindgen__convert__closures_____invoke__h2c42e7ad3ef275c0(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures_____invoke__h2c42e7ad3ef275c0(arg0, arg1);
}

function wasm_bindgen__convert__closures_____invoke__ha1939d9ab52a5a4b(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures_____invoke__ha1939d9ab52a5a4b(arg0, arg1);
}

function wasm_bindgen__convert__closures_____invoke__h192dd3c69c0111b0(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures_____invoke__h192dd3c69c0111b0(arg0, arg1);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_3(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_3(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_4(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_4(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_5(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_5(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_6(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_6(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_7(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_7(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_8(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_8(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_9(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h255e4ea23473a3c0_9(arg0, arg1, arg2);
}

function wasm_bindgen__convert__closures_____invoke__h899eb88a471b4e55(arg0, arg1, arg2) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__h899eb88a471b4e55(arg0, arg1, arg2);
    if (ret[1]) {
        throw takeFromExternrefTable0(ret[0]);
    }
}

function wasm_bindgen__convert__closures_____invoke__h8999e935b7f0f365(arg0, arg1, arg2, arg3) {
    wasm.wasm_bindgen__convert__closures_____invoke__h8999e935b7f0f365(arg0, arg1, arg2, arg3);
}

function wasm_bindgen__convert__closures_____invoke__hf5047f926a8b7c42(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__hf5047f926a8b7c42(arg0, arg1, isLikeNone(arg2) ? 0 : addToExternrefTable0(arg2));
}


const __wbindgen_enum_GamepadMappingType = ["", "standard"];


const __wbindgen_enum_PremultiplyAlpha = ["none", "premultiply", "default"];


const __wbindgen_enum_ResizeObserverBoxOptions = ["border-box", "content-box", "device-pixel-content-box"];


const __wbindgen_enum_VisibilityState = ["hidden", "visible"];
const BrowserConnectionsFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_browserconnections_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

const CLOSURE_DTORS = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(state => wasm.__wbindgen_destroy_closure(state.a, state.b));

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches && builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

function getArrayF32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayI16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayI32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayI8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

function getArrayU16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

function getClampedArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ClampedArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
    if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) {
        cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
    }
    return cachedFloat32ArrayMemory0;
}

let cachedInt16ArrayMemory0 = null;
function getInt16ArrayMemory0() {
    if (cachedInt16ArrayMemory0 === null || cachedInt16ArrayMemory0.byteLength === 0) {
        cachedInt16ArrayMemory0 = new Int16Array(wasm.memory.buffer);
    }
    return cachedInt16ArrayMemory0;
}

let cachedInt32ArrayMemory0 = null;
function getInt32ArrayMemory0() {
    if (cachedInt32ArrayMemory0 === null || cachedInt32ArrayMemory0.byteLength === 0) {
        cachedInt32ArrayMemory0 = new Int32Array(wasm.memory.buffer);
    }
    return cachedInt32ArrayMemory0;
}

let cachedInt8ArrayMemory0 = null;
function getInt8ArrayMemory0() {
    if (cachedInt8ArrayMemory0 === null || cachedInt8ArrayMemory0.byteLength === 0) {
        cachedInt8ArrayMemory0 = new Int8Array(wasm.memory.buffer);
    }
    return cachedInt8ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint16ArrayMemory0 = null;
function getUint16ArrayMemory0() {
    if (cachedUint16ArrayMemory0 === null || cachedUint16ArrayMemory0.byteLength === 0) {
        cachedUint16ArrayMemory0 = new Uint16Array(wasm.memory.buffer);
    }
    return cachedUint16ArrayMemory0;
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

let cachedUint8ClampedArrayMemory0 = null;
function getUint8ClampedArrayMemory0() {
    if (cachedUint8ClampedArrayMemory0 === null || cachedUint8ClampedArrayMemory0.byteLength === 0) {
        cachedUint8ClampedArrayMemory0 = new Uint8ClampedArray(wasm.memory.buffer);
    }
    return cachedUint8ClampedArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function makeMutClosure(arg0, arg1, f) {
    const state = { a: arg0, b: arg1, cnt: 1 };
    const real = (...args) => {

        // First up with a closure we increment the internal reference
        // count. This ensures that the Rust closure environment won't
        // be deallocated while we're invoking it.
        state.cnt++;
        const a = state.a;
        state.a = 0;
        try {
            return f(a, state.b, ...args);
        } finally {
            state.a = a;
            real._wbg_cb_unref();
        }
    };
    real._wbg_cb_unref = () => {
        if (--state.cnt === 0) {
            wasm.__wbindgen_destroy_closure(state.a, state.b);
            state.a = 0;
            CLOSURE_DTORS.unregister(state);
        }
    };
    CLOSURE_DTORS.register(real, state, state);
    return real;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayJsValueToWasm0(array, malloc) {
    const ptr = malloc(array.length * 4, 4) >>> 0;
    for (let i = 0; i < array.length; i++) {
        const add = addToExternrefTable0(array[i]);
        getDataViewMemory0().setUint32(ptr + 4 * i, add, true);
    }
    WASM_VECTOR_LEN = array.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedFloat32ArrayMemory0 = null;
    cachedInt16ArrayMemory0 = null;
    cachedInt32ArrayMemory0 = null;
    cachedInt8ArrayMemory0 = null;
    cachedUint16ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    cachedUint8ClampedArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('project-phoenix_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
