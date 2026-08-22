use bevy::prelude::*;
use std::collections::VecDeque;

use crate::entities::spawner::RegionShapeSection;
use crate::modifiers::ShipModifiers;
use crate::regions::shape::RegionShape;

/// Resource indicating whether debug region wireframes are enabled.
#[derive(Resource)]
pub struct DebugRegionsEnabled(pub bool);

/// Resource indicating whether the modifier debug overlay is enabled.
#[derive(Resource, Default)]
pub struct DebugOverlayEnabled(pub bool);

/// Resource indicating whether the simulation clock is paused.
///
/// Deliberately NOT a debug-only concept even though it is registered here
/// alongside the overlays (issue #939): the host settings menu exposes pause
/// on its **Gameplay** tab, which is not build-gated, so this resource and
/// everything that drives it must survive into a demo build where the
/// Debug/Cheat tab is absent. The settings menu is its only driver, through
/// the `DebugToggleKind::Pause` pending toggle.
#[derive(Resource, Default)]
pub struct SimulationPaused(pub bool);

/// Resource indicating whether the damage debug overlay is enabled.
#[derive(Resource, Default)]
pub struct DebugDamageEnabled(pub bool);

/// Resource indicating whether the entity behavior debug overlay is enabled.
#[derive(Resource, Default)]
pub struct DebugEntitiesEnabled(pub bool);

/// Resource indicating whether the entity inspector overlay is enabled.
#[derive(Resource, Default)]
pub struct DebugEntityInspectorEnabled(pub bool);

/// Maximum number of damage log entries retained.
pub const DAMAGE_LOG_CAPACITY: usize = 10;

/// A single damage event recorded for the damage debug overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageLogEntry {
    /// Human-readable description of the damage source (e.g. asteroid uuid,
    /// region uuid, weapon name).
    pub source: String,
    /// Shield arc label hit, or `None` when shields were bypassed / absent.
    pub shield_arc: Option<String>,
    /// Total damage amount before shield absorption (hull + shield combined).
    pub amount: f32,
}

/// Ring-buffer of the most recent damage events.
///
/// Always retains up to `DAMAGE_LOG_CAPACITY` entries, newest at the front.
/// Populated by damage application sites; read by the damage overlay system.
#[derive(Resource, Default)]
pub struct DamageLog {
    pub entries: VecDeque<DamageLogEntry>,
}

impl DamageLog {
    /// Push a new entry to the front, evicting the oldest when at capacity.
    pub fn push(&mut self, entry: DamageLogEntry) {
        self.entries.push_front(entry);
        while self.entries.len() > DAMAGE_LOG_CAPACITY {
            self.entries.pop_back();
        }
    }

    /// Format the log as a multi-line string for display.
    pub fn format(&self) -> String {
        if self.entries.is_empty() {
            return "(no damage)".to_string();
        }
        let mut out = String::from("DAMAGE LOG (newest first)\n");
        for (i, e) in self.entries.iter().enumerate() {
            let arc = e.shield_arc.as_deref().unwrap_or("—");
            out.push_str(&format!(
                "{:>2}. {:<24} arc={:<10} dmg={:.1}\n",
                i + 1,
                e.source,
                arc,
                e.amount
            ));
        }
        out
    }
}

// ── The phone client's settings route (issue #940) ──────────────────────────
//
// The host page flips these resources through `bridge::drain_debug_toggles`,
// which drains a thread-local the `wasm_toggle_*` exports fill. A phone has no
// WASM to call, so it sends a top-level `ClientMessage` and the systems below
// are its equivalent: two drains and one read-back broadcast.
//
// The two drains are `#[cfg(not(phoenix_demo_build))]`, along with the messages
// they read, so a demo binary contains neither. Their separation is the point:
// `ToggleDebugFlag` carries diagnostics that a demo simply has no use for,
// while `TogglePause` is a lever any one of N strangers could pull to freeze
// everyone else's mission. They go together here only because both answers
// happen to be "not in the demo" — the host's own pause (issue #939) is
// untouched in every build.
//
// All three live in `PreUpdate`, frame-driven, for the reason spelled out on
// `ClientMessage::TogglePause` — pausing starves `FixedUpdate`, so anything
// that has to undo a pause (or report one) cannot be in the fixed schedule.
// That also makes the read-back reach a phone whose ship is frozen solid.

/// Last `DebugState` broadcast, so the read-back is emitted on change rather
/// than every frame.
///
/// `None` until the first broadcast, which is what makes a client that joins
/// mid-session get one: the state is re-sent whenever it differs from what was
/// last *sent*, and a reset of this resource forces a resend.
///
/// The tuple is `(flags, paused, god_mode)`, matching the message's fields.
#[derive(Resource, Default)]
#[allow(clippy::type_complexity)]
pub struct LastReportedDebugState(
    pub Option<(Vec<(crate::core::messages::DebugFlag, bool)>, bool, bool)>,
);

/// Decide which flags a batch of inbound messages actually flips.
///
/// Pure and Bevy-free so the authority question is testable without an App:
/// a message counts only when its sender is a connected player. There is no
/// per-flag question left to ask — every `DebugFlag` is diagnostic-only, and
/// the build gate is the `#[cfg]` on this function and on the message itself.
/// Returns the toggle kinds in arrival order; `apply_pending_toggles` dedupes
/// them, exactly as it does for the host page's own pending set.
#[cfg(not(phoenix_demo_build))]
pub fn admitted_flag_toggles<'a>(
    messages: impl IntoIterator<Item = (&'a str, crate::core::messages::DebugFlag)>,
    is_connected: impl Fn(&str) -> bool,
) -> Vec<crate::server::bridge::DebugToggleKind> {
    messages
        .into_iter()
        .filter(|(token, _)| is_connected(token))
        .map(|(_, flag)| crate::server::bridge::DebugToggleKind::from(flag))
        .collect()
}

/// Drain `ClientMessage::ToggleDebugFlag` from connected phones and apply it.
///
/// **Not compiled into a demo build**, and neither is the message it reads.
///
/// Reads raw `InboundMessage` rather than `AdmittedCommands` deliberately —
/// see the variant's doc for why these never cross command admission. The
/// authority check is not skipped, it is [`admitted_flag_toggles`].
///
/// The flag-flipping itself is `bridge::apply_pending_toggles`, the same pure
/// function the host page's drain calls, so a flag flipped from a phone and one
/// flipped from the host page cannot diverge. `paused` is passed to it as a
/// throwaway local: no `DebugFlag` maps to `DebugToggleKind::Pause` any more,
/// so this drain can no longer touch the clock even by accident.
#[cfg(not(phoenix_demo_build))]
#[allow(clippy::too_many_arguments)]
pub fn drain_client_debug_flags(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    sessions: Res<crate::lobby::Sessions>,
    mut regions: ResMut<DebugRegionsEnabled>,
    mut overlay: ResMut<DebugOverlayEnabled>,
    mut damage: ResMut<DebugDamageEnabled>,
    mut entities: ResMut<DebugEntitiesEnabled>,
    mut inspector: ResMut<DebugEntityInspectorEnabled>,
    mut station_activity: ResMut<crate::debug::DebugStationActivityEnabled>,
) {
    let mut requests: Vec<(String, crate::core::messages::DebugFlag)> = Vec::new();
    for ev in reader.read() {
        if let crate::core::messages::ClientMessage::ToggleDebugFlag { flag } = &ev.msg {
            requests.push((ev.token.clone(), *flag));
        }
    }
    if requests.is_empty() {
        return;
    }

    let pending = admitted_flag_toggles(
        requests.iter().map(|(token, flag)| (token.as_str(), *flag)),
        |token| sessions.0.players().iter().any(|p| p.token == token),
    );
    if pending.is_empty() {
        return;
    }

    let mut unreachable_pause = false;
    let pause_changed = crate::server::bridge::apply_pending_toggles(
        pending,
        &mut regions.0,
        &mut overlay.0,
        &mut unreachable_pause,
        &mut damage.0,
        &mut entities.0,
        &mut inspector.0,
        &mut station_activity.0,
    );
    debug_assert!(
        !pause_changed,
        "no DebugFlag maps to DebugToggleKind::Pause — pause is ClientMessage::TogglePause"
    );
}

/// Count the pause requests in a batch that are honoured.
///
/// Pure and Bevy-free, and separate from [`admitted_flag_toggles`] because it
/// answers a different question about a different message. A request counts
/// only when its sender is a connected player; the build gate is the `#[cfg]`
/// on this function and on `ClientMessage::TogglePause` itself.
///
/// Returns a count rather than a bool because pause is a *toggle*: two taps in
/// one frame are two flips, which is a no-op, and collapsing them to "someone
/// asked" would turn a double-tap into a pause.
#[cfg(not(phoenix_demo_build))]
pub fn admitted_pause_toggles<'a>(
    tokens: impl IntoIterator<Item = &'a str>,
    is_connected: impl Fn(&str) -> bool,
) -> usize {
    tokens.into_iter().filter(|t| is_connected(t)).count()
}

/// Drain `ClientMessage::TogglePause` from connected phones and apply it.
///
/// **Not compiled into a demo build**, and neither is the message it reads.
/// That is the whole of the protection: a demo phone's pause is not refused
/// here, it never decodes. See the variant's doc for why N strangers each
/// holding the pause lever is a different risk from one host operator holding
/// it, and why the host's own cog keeps working in every build.
///
/// No station, captaincy or `GamePhase` check, deliberately — in a dev build
/// the whole point is that whoever is testing can stop the clock from the phone
/// in their hand, whatever they are holding.
#[cfg(not(phoenix_demo_build))]
pub fn drain_client_pause(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    sessions: Res<crate::lobby::Sessions>,
    mut paused: ResMut<SimulationPaused>,
    mut virtual_time: ResMut<Time<bevy::time::Virtual>>,
) {
    let tokens: Vec<String> = reader
        .read()
        .filter(|ev| matches!(ev.msg, crate::core::messages::ClientMessage::TogglePause))
        .map(|ev| ev.token.clone())
        .collect();
    if tokens.is_empty() {
        return;
    }

    let flips = admitted_pause_toggles(tokens.iter().map(String::as_str), |token| {
        sessions.0.players().iter().any(|p| p.token == token)
    });
    if flips % 2 == 0 {
        return;
    }

    paused.0 = !paused.0;
    // Identical side effect to the host page's drain: pausing `Time<Virtual>`
    // starves the fixed accumulator and freezes the whole sim. See
    // `bridge::drain_debug_toggles` for what that costs.
    if paused.0 {
        virtual_time.pause();
    } else {
        virtual_time.unpause();
    }
}

/// Broadcast `ServerMessage::DebugState` whenever the reported state changes.
///
/// Present in **every** build, unlike the two drains above. It is a read-back,
/// and a read-back grants no authority: a demo phone learns the host paused the
/// mission and still has no message with which to pause it. Keeping it
/// un-gated also means the flags a host flips from its own cog stay visible to
/// whatever tooling is watching, in any build.
///
/// One reporter for three writers: the overlay flags, `SimulationPaused` — both
/// reachable from the host's cog in every build and from a phone in a dev build
/// — and `GodMode`, flipped by an admitted command in `FixedUpdate` (issue
/// #900). Reading them here means the phone's Debug/Cheat tab paints from the
/// simulation rather than from its own optimism, which is the point: a demo
/// build has no route at all and the button has to show that.
///
/// It also owns the resend on `Identify`: a peer that just joined has no idea
/// what the flags are, and this system only speaks on change. That belongs here
/// rather than in a drain because this is the only system that writes
/// [`LastReportedDebugState`] — and because the drains are not in a demo build,
/// where a joining phone still deserves the read-back.
///
/// Writes `OutboundMessage` directly rather than `SimOutbox` because that outbox
/// is drained in `FixedUpdate`: a client that had just paused the game would
/// never be told it worked.
#[allow(clippy::too_many_arguments)]
pub fn report_debug_state(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    regions: Res<DebugRegionsEnabled>,
    overlay: Res<DebugOverlayEnabled>,
    paused: Res<SimulationPaused>,
    damage: Res<DebugDamageEnabled>,
    entities: Res<DebugEntitiesEnabled>,
    inspector: Res<DebugEntityInspectorEnabled>,
    station_activity: Res<crate::debug::DebugStationActivityEnabled>,
    god_mode: Option<Res<crate::server_app::GodMode>>,
    mut last: ResMut<LastReportedDebugState>,
    mut writer: MessageWriter<crate::lobby::OutboundMessage>,
) {
    use crate::core::messages::DebugFlag;

    // Forgetting what was last sent makes the compare below re-announce the
    // current state to everyone, which is cheap and is the only sync a joining
    // phone gets. That resend and the new peer's own `Welcome` flush in the
    // same frame, in this order, so `gui/sim-state.js` PRESERVES `debugFlags`
    // across a `Welcome` reset rather than clearing it — see the field's
    // own comment.
    if reader.read().any(|ev| {
        matches!(
            ev.msg,
            crate::core::messages::ClientMessage::Identify { .. }
        )
    }) {
        last.0 = None;
    }

    let god_mode = god_mode.map(|g| g.0).unwrap_or(false);
    let flags: Vec<(DebugFlag, bool)> = DebugFlag::ALL
        .iter()
        .map(|flag| {
            let on = match flag {
                DebugFlag::Regions => regions.0,
                DebugFlag::Modifiers => overlay.0,
                DebugFlag::Damage => damage.0,
                DebugFlag::Entities => entities.0,
                DebugFlag::Inspector => inspector.0,
                DebugFlag::StationActivity => station_activity.0,
            };
            (*flag, on)
        })
        .collect();

    let current = (flags, paused.0, god_mode);
    if last.0.as_ref() == Some(&current) {
        return;
    }
    last.0 = Some(current.clone());
    writer.write(crate::lobby::OutboundMessage {
        target: crate::lobby::Target::All,
        msg: crate::core::messages::ServerMessage::DebugState {
            flags: current.0,
            paused: current.1,
            god_mode: current.2,
        },
        delivery: crate::core::messages::DeliveryClass::Reliable,
    });
}

/// Server-only plugin that draws region shape wireframes when enabled.
///
/// The `enabled` field is typically set from the `?debug_regions=1` URL parameter
/// on WASM (via `bridge.rs`), or directly in tests.
pub struct DebugOverlayPlugin {
    pub enabled: bool,
}

impl Plugin for DebugOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DebugRegionsEnabled(self.enabled));
        app.init_resource::<DebugOverlayEnabled>();
        app.init_resource::<SimulationPaused>();
        app.init_resource::<DebugDamageEnabled>();
        app.init_resource::<DamageLog>();
        app.init_resource::<DebugEntitiesEnabled>();
        app.init_resource::<DebugEntityInspectorEnabled>();
        if should_install_region_wireframes() {
            app.add_systems(
                Update,
                draw_region_wireframes.run_if(|r: Res<DebugRegionsEnabled>| r.0),
            );
        }
        app.add_systems(
            PostUpdate,
            write_debug_state.run_if(|r: Res<DebugOverlayEnabled>| r.0),
        );
        app.add_systems(
            PostUpdate,
            write_damage_log.run_if(|r: Res<DebugDamageEnabled>| r.0),
        );
        app.add_systems(
            PostUpdate,
            write_entity_debug_state.run_if(|r: Res<DebugEntitiesEnabled>| r.0),
        );
        app.add_systems(
            PostUpdate,
            update_entity_inspector.run_if(|r: Res<DebugEntityInspectorEnabled>| r.0),
        );
    }
}

/// Returns `true` when running under Playwright/WebDriver automation (WASM only).
///
/// On native and non-server builds this always returns `false`, so non-WASM
/// tests and native simulation apps keep full gizmo rendering without any
/// special setup.
///
/// Uses `navigator.webdriver` (set by Playwright / Selenium) to detect
/// automation. When the property is absent or the detection fails, the safe
/// default (`false` — not automation) is returned. Callers in automation
/// mode should skip any functionality that depends on renderer resources
/// (e.g. `Gizmos`) which are not available under `MinimalPlugins`.
#[cfg(target_arch = "wasm32")]
pub fn is_playwright_automation() -> bool {
    web_sys::window()
        .and_then(|w| {
            let nav = w.navigator();
            js_sys::Reflect::get(&nav, &"webdriver".into())
                .ok()
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false)
}

/// Native / non-server fallback — no automation possible.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_playwright_automation() -> bool {
    false
}

fn should_install_region_wireframes() -> bool {
    !is_playwright_automation()
}

/// Reads `ShipModifiers` from the LocalShip entity and writes the formatted
/// debug text to the WASM thread-local `DEBUG_STATE_STRING`.
///
/// Only runs when `DebugOverlayEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn write_debug_state(modifiers_q: Query<&ShipModifiers, With<crate::server_app::LocalShip>>) {
    if let Some(modifiers) = modifiers_q.iter().next() {
        let text = modifiers.format_debug();
        crate::server::bridge::set_debug_state_string(text);
    }
}

/// Native / test stub — does nothing (no thread-locals available outside WASM).
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn write_debug_state(_modifiers_q: Query<&ShipModifiers, With<crate::server_app::LocalShip>>) {}

/// Reads the `DamageLog` resource and writes the formatted text to the WASM
/// thread-local `DAMAGE_LOG_STRING` for the damage overlay.
///
/// Only runs when `DebugDamageEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn write_damage_log(log: Res<DamageLog>) {
    let text = log.format();
    crate::server::bridge::set_damage_log_string(text);
}

/// Native / test stub — does nothing.
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn write_damage_log(_log: Res<DamageLog>) {}

/// Reads all entities with `BehaviourSection` (i.e. AI-driven NPCs) and writes a
/// formatted table (name, position, current state) to the WASM thread-local for
/// the entity behavior overlay.
///
/// Only runs when `DebugEntitiesEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn write_entity_debug_state(
    entities: Query<(
        &crate::entities::spawner::BehaviourSection,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::console::weapons::TacticalRadarSelection>,
    )>,
) {
    let count = entities.iter().count();
    let mut out = format!("ENTITY BEHAVIOR ({} entities)\n", count);
    for (i, (_ai, transform, name, memory)) in entities.iter().enumerate() {
        let label = name.map(|n| n.0.as_str()).unwrap_or("<unnamed>");
        let p = transform.translation;
        // The ship's authoritative Tactical lock (issue #702). Was
        // `ShipAiMemory.target`, a private mirror that could disagree with what
        // the ship was actually shooting — so the overlay could report a target
        // the ship had not selected.
        let target_str = memory
            .and_then(|t| t.0.clone())
            .unwrap_or_else(|| "none".to_string());
        out.push_str(&format!(
            "{:>2}. {:<20} pos=({:>7.1},{:>7.1},{:>7.1})  target={}\n",
            i + 1,
            label,
            p.x,
            p.y,
            p.z,
            target_str
        ));
    }
    crate::server::bridge::set_entity_debug_string(out);
}

/// Native / test stub — does nothing.
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn write_entity_debug_state(
    _entities: Query<(
        &crate::entities::spawner::BehaviourSection,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::console::weapons::TacticalRadarSelection>,
    )>,
) {
}

/// Reads all non-asteroid entities plus the player ship resources and writes a
/// formatted entity inspector block to the WASM thread-local.
///
/// Displays: name, tags, position, distance from player, faction name, hull HP,
/// shield arcs (player ship only), comms hailability, and AI state.
///
/// Only runs when `DebugEntityInspectorEnabled` is true.
#[cfg(all(target_arch = "wasm32", feature = "server"))]
fn update_entity_inspector(
    entities: Query<
        (
            &Transform,
            &crate::entities::spawner::EntityName,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::comms::component::CommsRange>,
            Option<&crate::console::weapons::TacticalRadarSelection>,
            &crate::entities::spawner::EntityTagsSection,
        ),
        bevy::ecs::query::Without<crate::server_app::Asteroid>,
    >,
    ship_physics_q: Query<&crate::ship::state::ShipPhysics, With<crate::server_app::LocalShip>>,
    player_hull_q: Query<
        &crate::entities::spawner::EntitySystemHull,
        With<crate::server_app::LocalShip>,
    >,
    ship_shields_q: Query<&crate::server_app::ShipShields, With<crate::server_app::LocalShip>>,
    faction_registry: Res<crate::entities::config_cache::FactionRegistryResource>,
) {
    let Ok(ship_shields) = ship_shields_q.single() else {
        return;
    };
    let ship_phys = ship_physics_q.single().ok().copied().unwrap_or_default();
    let player_x = ship_phys.x;
    let player_z = ship_phys.z;

    let mut out = String::from("ENTITY INSPECTOR\n");
    out.push_str("────────────────────────────────────────────────────────────\n");

    // ── Player ship ────────────────────────────────────────────────────────
    out.push_str(&format!(
        "[Player Ship]  pos=({:>8.1}, {:>8.1})\n",
        player_x, player_z
    ));

    // Per-system hull from the LocalShip's EntitySystemHull component.
    let hull_entries: Vec<(crate::core::messages::SystemId, f32, f32)> = player_hull_q
        .single()
        .map(|h| {
            h.0.entries()
                .map(|(sid, cur, max)| (sid.clone(), cur, max))
                .collect()
        })
        .unwrap_or_default();
    if hull_entries.is_empty() {
        out.push_str("  hull: n/a\n");
    } else {
        out.push_str("  hull:");
        for (sid, cur, max) in &hull_entries {
            out.push_str(&format!("  {} {}/{}", sid.0, *cur as i32, *max as i32));
        }
        out.push('\n');
    }

    // Per-arc shields
    let facings = &ship_shields.0.facings;
    if facings.is_empty() {
        out.push_str("  shields: n/a\n");
    } else {
        out.push_str("  shields:");
        for f in facings {
            let pct = if f.max_hp > 0 {
                (f.hp as f32 / f.max_hp as f32 * 100.0) as i32
            } else {
                0
            };
            let status = if f.offline_remaining > 0.0 {
                " [OFFLINE]"
            } else {
                ""
            };
            let focus = if f.is_focused { "*" } else { "" };
            out.push_str(&format!(
                "  {}{} {}/{} ({}%){}",
                focus, f.label, f.hp, f.max_hp, pct, status
            ));
        }
        out.push('\n');
    }

    out.push_str("────────────────────────────────────────────────────────────\n");

    // ── World entities ─────────────────────────────────────────────────────
    let mut sorted: Vec<_> = entities.iter().collect();
    // Sort by distance from player for readability
    sorted.sort_by(|a, b| {
        let da = {
            let p = a.0.translation;
            let dx = p.x - player_x;
            let dz = p.z - player_z;
            dx * dx + dz * dz
        };
        let db = {
            let p = b.0.translation;
            let dx = p.x - player_x;
            let dz = p.z - player_z;
            dx * dx + dz * dz
        };
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });

    for (i, (transform, name, hull, faction_comp, comms_range, ai, tags)) in
        sorted.iter().enumerate()
    {
        let p = transform.translation;
        let dx = p.x - player_x;
        let dz = p.z - player_z;
        let dist = (dx * dx + dz * dz).sqrt();

        let tag_list = tags.0.join(", ");
        out.push_str(&format!("{:>2}. {}  [{}]\n", i + 1, name.0, tag_list));
        out.push_str(&format!(
            "    pos=({:>8.1}, {:>8.1})  dist={:>7.1}u\n",
            p.x, p.z, dist
        ));

        // Faction
        if let Some(fc) = faction_comp {
            let faction_name = faction_registry
                .0
                .get(&fc.0)
                .map(|f| f.name.as_str())
                .unwrap_or("<unknown>");
            out.push_str(&format!("    faction: {}\n", faction_name));
        }

        // Hull
        if let Some(h) = hull {
            let cur = h.0.total_current();
            let max = h.0.total_max();
            let pct = if max > 0.0 {
                (cur / max * 100.0) as i32
            } else {
                0
            };
            out.push_str(&format!(
                "    hull: {}/{} ({}%)\n",
                cur as i32, max as i32, pct
            ));
        }

        // Comms
        if let Some(range) = comms_range {
            let in_range = dist <= range.0;
            if in_range {
                out.push_str("    comms: hailable (in range)\n");
            } else {
                out.push_str(&format!("    comms: hailable (range {:.0}u)\n", range.0));
            }
        }

        // AI state
        if let Some(target) = ai {
            out.push_str(&format!(
                "    ai: target={}\n",
                target.0.clone().unwrap_or_else(|| "none".to_string())
            ));
        }
    }

    out.push_str("────────────────────────────────────────────────────────────\n");
    crate::server::bridge::set_entity_inspector_string(out);
}

/// Native / test stub — does nothing.
#[cfg(not(all(target_arch = "wasm32", feature = "server")))]
fn update_entity_inspector(
    _entities: Query<
        (
            &Transform,
            &crate::entities::spawner::EntityName,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::comms::component::CommsRange>,
            Option<&crate::console::weapons::TacticalRadarSelection>,
            &crate::entities::spawner::EntityTagsSection,
        ),
        bevy::ecs::query::Without<crate::server_app::Asteroid>,
    >,
    _ship_shields_q: Query<&crate::server_app::ShipShields, With<crate::server_app::LocalShip>>,
    _faction_registry: Res<crate::entities::config_cache::FactionRegistryResource>,
) {
}

/// Draws wireframe outlines for every region entity with a shape component.
fn draw_region_wireframes(regions: Query<(&Transform, &RegionShapeSection)>, mut gizmos: Gizmos) {
    for (transform, shape) in regions.iter() {
        let origin = transform.translation - Vec3::Y * 10.0;
        match &shape.0 {
            RegionShape::Sphere { radius } => {
                draw_sphere_wireframe(&mut gizmos, origin, *radius);
            }
            RegionShape::Box { half_extents, .. } => {
                draw_box_wireframe(&mut gizmos, origin, *half_extents);
            }
            RegionShape::Torus {
                inner_radius,
                outer_radius,
            } => {
                draw_torus_wireframe(&mut gizmos, origin, *inner_radius, *outer_radius);
            }
        }
    }
}

fn draw_sphere_wireframe(gizmos: &mut Gizmos, origin: Vec3, radius: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    gizmos.circle(
        Isometry3d::new(origin, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), radius, color);
    gizmos.circle(
        Isometry3d::new(origin, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
}

fn draw_box_wireframe(gizmos: &mut Gizmos, origin: Vec3, half_extents: [f32; 3]) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    let [hx, hy, hz] = half_extents;
    let corners = [
        Vec3::new(-hx, -hy, -hz),
        Vec3::new(hx, -hy, -hz),
        Vec3::new(hx, -hy, hz),
        Vec3::new(-hx, -hy, hz),
        Vec3::new(-hx, hy, -hz),
        Vec3::new(hx, hy, -hz),
        Vec3::new(hx, hy, hz),
        Vec3::new(-hx, hy, hz),
    ]
    .map(|c| origin + c);
    let edges: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (i, j) in edges {
        gizmos.line(corners[i], corners[j], color);
    }
}

fn draw_torus_wireframe(gizmos: &mut Gizmos, origin: Vec3, inner_radius: f32, outer_radius: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    // Draw two horizontal circles representing the inner and outer edges of the torus
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), inner_radius, color);
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), outer_radius, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_regions_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(!enabled.0, "default should be disabled");
    }

    #[test]
    fn debug_regions_enabled_when_flag_set() {
        let plugin = DebugOverlayPlugin { enabled: true };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(enabled.0, "should be enabled when flag is set");
    }

    /// Toggling the resource from false → true should flip DebugRegionsEnabled.
    #[test]
    fn toggle_debug_regions_false_to_true() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        // Simulate what drain_debug_toggles does: flip the resource.
        app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = true;
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(enabled.0, "resource should be true after toggle");
    }

    /// Toggling the resource from true → false should flip DebugRegionsEnabled.
    #[test]
    fn toggle_debug_regions_true_to_false() {
        let plugin = DebugOverlayPlugin { enabled: true };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = false;
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(!enabled.0, "resource should be false after toggle");
    }

    // ── DebugOverlayEnabled tests ─────────────────────────────────────────

    #[test]
    fn debug_overlay_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugOverlayEnabled>();
        assert!(!enabled.0, "overlay should be disabled by default");
    }

    #[test]
    fn toggle_debug_overlay_false_to_true() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = true;
        let enabled = app.world().resource::<DebugOverlayEnabled>();
        assert!(enabled.0, "overlay should be enabled after toggle");
    }

    #[test]
    fn toggle_debug_overlay_true_to_false() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = true;
        app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = false;
        let enabled = app.world().resource::<DebugOverlayEnabled>();
        assert!(!enabled.0, "overlay should be disabled after second toggle");
    }

    // ── DamageLog tests ───────────────────────────────────────────────────

    fn entry(source: &str, arc: Option<&str>, amount: f32) -> DamageLogEntry {
        DamageLogEntry {
            source: source.to_string(),
            shield_arc: arc.map(|s| s.to_string()),
            amount,
        }
    }

    #[test]
    fn damage_log_starts_empty() {
        let log = DamageLog::default();
        assert!(log.entries.is_empty());
        assert_eq!(log.format(), "(no damage)");
    }

    #[test]
    fn damage_log_pushes_newest_to_front() {
        let mut log = DamageLog::default();
        log.push(entry("a", Some("Fore"), 1.0));
        log.push(entry("b", Some("Port"), 2.0));
        assert_eq!(log.entries.len(), 2);
        assert_eq!(log.entries[0].source, "b");
        assert_eq!(log.entries[1].source, "a");
    }

    #[test]
    fn damage_log_caps_at_capacity() {
        let mut log = DamageLog::default();
        for i in 0..(DAMAGE_LOG_CAPACITY + 5) {
            log.push(entry(&format!("s{}", i), None, i as f32));
        }
        assert_eq!(log.entries.len(), DAMAGE_LOG_CAPACITY);
        // Newest at front
        assert_eq!(
            log.entries[0].source,
            format!("s{}", DAMAGE_LOG_CAPACITY + 4)
        );
        // Oldest retained is the one DAMAGE_LOG_CAPACITY back from newest
        assert_eq!(log.entries[DAMAGE_LOG_CAPACITY - 1].source, "s5");
    }

    #[test]
    fn damage_log_format_includes_source_arc_and_amount() {
        let mut log = DamageLog::default();
        log.push(entry("asteroid-42", Some("Fore"), 12.5));
        log.push(entry("region-zone", None, 3.0));
        let text = log.format();
        assert!(text.contains("region-zone"));
        assert!(text.contains("asteroid-42"));
        assert!(text.contains("Fore"));
        assert!(text.contains("12.5"));
        assert!(text.contains("3.0"));
        // None arc renders as em-dash placeholder
        assert!(text.contains("—"));
    }

    #[test]
    fn debug_damage_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugDamageEnabled>();
        assert!(!enabled.0, "damage overlay should be disabled by default");
    }

    /// Every wire flag maps onto a distinct host-page toggle kind — the two
    /// vocabularies stay one-for-one, so no flag silently flips another's
    /// resource. Compiled in every build, because the conversion is: it is the
    /// shape of the two enums, not a route.
    #[test]
    fn every_flag_maps_to_its_own_toggle_kind() {
        let kinds: std::collections::HashSet<_> = crate::core::messages::DebugFlag::ALL
            .iter()
            .map(|f| crate::server::bridge::DebugToggleKind::from(*f))
            .collect();
        assert_eq!(kinds.len(), crate::core::messages::DebugFlag::ALL.len());
    }

    /// No `DebugFlag` reaches the clock. Pause left this enum precisely so the
    /// overlay drain could not touch `SimulationPaused` even by accident, and
    /// this is that claim, checked rather than asserted in prose.
    #[test]
    fn no_debug_flag_maps_to_the_pause_toggle() {
        for flag in crate::core::messages::DebugFlag::ALL {
            assert_ne!(
                crate::server::bridge::DebugToggleKind::from(flag),
                crate::server::bridge::DebugToggleKind::Pause,
                "{flag:?} would let the overlay drain stop the simulation clock"
            );
        }
    }

    // ── The phone client's settings routes (issue #940) ─────────────────────
    //
    // Gated as a whole: in a demo build neither drain exists, so there is
    // nothing here to test. That the routes are ABSENT there is pinned from
    // both builds by `codec`'s two
    // `*_route_is_absent_from_a_demo_build` tests, which ask the question
    // through the wire rather than through a predicate this module owns.

    #[cfg(not(phoenix_demo_build))]
    mod client_route {
        use super::*;
        use crate::core::messages::DebugFlag;

        /// Only tokens in this list count as connected players.
        fn connected<'a>(known: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
            move |token| known.contains(&token)
        }

        /// The happy path: a connected player's flags reach the pending set as
        /// the matching `DebugToggleKind`s, in submission order.
        #[test]
        fn a_connected_players_flag_is_admitted() {
            let kinds = admitted_flag_toggles(
                [("phone", DebugFlag::Regions), ("phone", DebugFlag::Damage)],
                connected(&["phone"]),
            );
            assert_eq!(
                kinds,
                vec![
                    crate::server::bridge::DebugToggleKind::Regions,
                    crate::server::bridge::DebugToggleKind::Damage,
                ]
            );
        }

        /// A token nobody registered is refused. The route widens *who* may
        /// flip a debug flag, not *whether* a sender has to exist.
        #[test]
        fn an_unregistered_token_is_refused() {
            assert!(
                admitted_flag_toggles([("ghost", DebugFlag::Regions)], connected(&["phone"]),)
                    .is_empty()
            );
        }

        /// An admitted batch flips exactly the resources it names, through the
        /// same pure function the host page's own drain uses — and never the
        /// clock, whatever it names.
        #[test]
        fn an_admitted_batch_flips_only_the_flags_it_names() {
            let (mut regions, mut overlay, mut paused) = (false, false, false);
            let (mut damage, mut entities, mut inspector) = (false, false, false);
            let mut station_activity = false;
            let pending =
                admitted_flag_toggles([("phone", DebugFlag::Damage)], connected(&["phone"]));
            let pause_changed = crate::server::bridge::apply_pending_toggles(
                pending,
                &mut regions,
                &mut overlay,
                &mut paused,
                &mut damage,
                &mut entities,
                &mut inspector,
                &mut station_activity,
            );
            assert!(damage, "the named flag must flip");
            assert!(!pause_changed);
            assert!(
                !regions && !overlay && !paused && !entities && !inspector && !station_activity
            );
        }

        /// Pause is a toggle, so an even number of admitted taps in one frame
        /// is a no-op and an odd number is one flip. Collapsing the batch to
        /// "someone asked" would turn a double-tap into a pause.
        #[test]
        fn pause_taps_are_counted_not_collapsed() {
            let known = ["phone", "other"];
            assert_eq!(admitted_pause_toggles(["phone"], connected(&known)), 1);
            assert_eq!(
                admitted_pause_toggles(["phone", "other"], connected(&known)),
                2
            );
        }

        /// The same identity rule the flags use: a token nobody registered
        /// cannot stop the clock, even in a dev build.
        #[test]
        fn an_unregistered_token_cannot_pause() {
            assert_eq!(
                admitted_pause_toggles(["ghost"], connected(&["phone"])),
                0,
                "an unidentified sender must not reach the simulation clock"
            );
        }
    }
}
