/// Shield system for the ship. Consists of one or more `ShieldFacing` arcs.
///
/// By default ships have four facings (Fore / Port / Aft / Starboard), each
/// spanning 90°. The number of arcs is configurable: fewer arcs means wider
/// but fewer facings.
///
/// ## Hit detection
/// The facing that absorbs a hit is determined by the *attacker bearing*
/// expressed as an angle **relative to the ship's own yaw** in the range
/// `(-π, π]`, measured anti-clockwise from the ship's forward (+Z) axis.
/// Each facing covers an equal arc of the full circle starting from the
/// forward direction (angle 0).
///
/// ## Offline mechanic
/// When a facing's HP reaches 0 it **collapses**: it goes offline for
/// `offline_duration` seconds. While offline it absorbs no damage; any hit to
/// that facing passes straight through to the hull.
///
/// `offline_duration` is a *no-damage delay*, not a recharge time. When it
/// expires the facing comes back online **at zero HP** and climbs from there at
/// its authored `regen_per_sec` (issue #788). It used to snap straight back to
/// `max_hp`, which meant a collapse cost a ship nothing but the offline window
/// and made "wait until shields are back to 75%" an unanswerable question —
/// there was no interval during which a shield was partially recovered.
///
/// This is one shared function: the player ship and every NPC collapse and
/// recover identically, and nothing here branches on who owns the hull
/// (AGENTS.md rule #6).
///
/// ## Regen
/// Each facing regenerates `regen_per_sec` HP per second while online.
/// Regen is capped at `max_hp`. A facing that is hit while still ramping is
/// knocked back to 0 and collapses again for a fresh `offline_duration`, so
/// sustained fire keeps a shield down rather than letting it flicker back.
use crate::simmath;
use std::f32::consts::TAU;

/// A snapshot of a single shield facing, suitable for serialisation and UI.
#[derive(Clone, Debug, PartialEq)]
pub struct ShieldFacingSnapshot {
    /// Stable arc id (may be empty for legacy `ShieldSystem::new` paths).
    pub id: String,
    /// Human-readable label (e.g. "Fore", "Starboard").
    pub label: String,
    pub hp: i32,
    pub max_hp: i32,
    pub online: bool,
    /// Remaining offline seconds (0.0 when online).
    pub offline_remaining: f32,
    /// Whether this facing is the currently focused arc.
    pub is_focused: bool,
    /// Arc centre bearing in degrees.
    pub center_deg: f32,
    /// Arc angular width in degrees.
    pub width_deg: f32,
    /// Hit-routing priority. Higher value wins when multiple arcs cover the same bearing.
    pub priority: u32,
}

/// Configuration for the entire shield system.
#[derive(Clone, Debug)]
pub struct ShieldConfig {
    /// Number of equally-spaced facings. Must be ≥ 1.
    pub num_facings: usize,
    pub max_hp: i32,
    pub regen_per_sec: f32,
    /// How long (seconds) a facing stays offline after its HP is depleted.
    pub offline_duration: f32,
}

impl Default for ShieldConfig {
    fn default() -> Self {
        Self {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 2.0,
            offline_duration: 10.0,
        }
    }
}

/// Configuration for shield focus bonuses and penalties.
///
/// When the Shields console operator focuses one facing:
/// - That facing gets `bonus_max_hp` extra capacity and `bonus_regen` regen.
/// - The other three facings lose `penalty_max_hp` capacity and `penalty_regen` regen.
/// - Non-focused facings decay HP at `decay_rate` per second when above their
///   reduced maximum.
/// - Incoming damage on the focused facing is scaled by `focused_damage_multiplier`
///   (e.g. 0.7 = 30% reduction), and on non-focused facings by
///   `unfocused_damage_multiplier` (e.g. 1.25 = 25% increase).
#[derive(Clone, Debug)]
pub struct ShieldFocusConfig {
    /// Extra max HP applied to the focused facing.
    pub bonus_max_hp: i32,
    /// Extra regen per second applied to the focused facing.
    pub bonus_regen: f32,
    /// Max HP subtracted from each non-focused facing.
    pub penalty_max_hp: i32,
    /// Regen per second subtracted from each non-focused facing.
    pub penalty_regen: f32,
    /// HP per second decay applied to non-focused facings when above reduced max.
    pub decay_rate: f32,
    /// Damage multiplier applied to incoming damage on the focused arc.
    /// 1.0 = no change, 0.7 = 30% reduction.
    pub focused_damage_multiplier: f32,
    /// Damage multiplier applied to incoming damage on non-focused arcs
    /// (when another arc is focused). 1.0 = no change, 1.25 = 25% increase.
    pub unfocused_damage_multiplier: f32,
}

impl Default for ShieldFocusConfig {
    fn default() -> Self {
        Self {
            bonus_max_hp: 50,
            bonus_regen: 5.0,
            penalty_max_hp: 25,
            penalty_regen: 1.0,
            decay_rate: 10.0,
            focused_damage_multiplier: 1.0,
            unfocused_damage_multiplier: 1.0,
        }
    }
}

/// A single shield facing arc.
#[derive(Clone, Debug)]
pub struct ShieldFacing {
    /// Stable arc id from the ship TOML `[[shield_arc]]` block (e.g. `"fore"`,
    /// `"all"`). Empty when constructed via legacy `ShieldSystem::new` from
    /// a `ShieldConfig` (the evenly-spaced-facings backwards-compat path).
    pub id: String,
    pub label: String,
    pub hp: i32,
    pub max_hp: i32,
    pub regen_per_sec: f32,
    pub offline_duration: f32,
    /// Remaining seconds of offline time. 0.0 means the facing is online.
    pub offline_remaining: f32,
    /// Whether this facing is the currently focused arc.
    pub is_focused: bool,
    /// Sub-integer regen accumulator. Carries fractional HP across frames so
    /// that regen rates below 1 HP/frame are applied correctly.
    hp_frac: f32,
    /// Arc centre bearing in degrees (0 = fore, 90 = starboard, 180 = aft,
    /// 270 = port). Used by [`ShieldSystem::facing_index_for_bearing`] to
    /// route incoming damage.
    pub center_deg: f32,
    /// Arc angular width in degrees. Sum of all arc widths on a ship should
    /// tile the 360° circle; overlap or gaps are the designer's problem.
    pub width_deg: f32,
    /// Per-arc baseline max HP before focus modifiers. Preserves the arc's
    /// declared TOML override (or the ship-wide default when no override was
    /// given) so `ShieldSystem::recalculate_focus` can rebuild `max_hp` from
    /// this arc's own baseline rather than clobbering it with the ship-wide
    /// value. Set once at construction and never mutated at runtime.
    pub base_max_hp: i32,
    /// Per-arc baseline regen/sec before focus modifiers. See `base_max_hp`.
    pub base_regen_per_sec: f32,
    /// Hit-routing priority. Higher value wins when multiple arcs cover the same bearing.
    /// Set at construction from TOML config and never mutated at runtime.
    pub priority: u32,
    /// Damage multiplier applied to incoming damage on this facing.
    /// Set by `ShieldSystem::recalculate_focus` based on focus state.
    pub damage_multiplier: f32,
}

impl ShieldFacing {
    fn new(
        label: impl Into<String>,
        max_hp: i32,
        regen_per_sec: f32,
        offline_duration: f32,
    ) -> Self {
        Self {
            id: String::new(),
            label: label.into(),
            hp: max_hp,
            max_hp,
            regen_per_sec,
            offline_duration,
            offline_remaining: 0.0,
            is_focused: false,
            hp_frac: 0.0,
            center_deg: 0.0,
            width_deg: 0.0,
            base_max_hp: max_hp,
            base_regen_per_sec: regen_per_sec,
            priority: 1,
            damage_multiplier: 1.0,
        }
    }

    /// Constructor used by [`ShieldSystem::from_arcs`]. Carries the full arc
    /// geometry so damage routing and blackboard publish can source
    /// `center_deg` / `width_deg` from the facing itself.
    fn new_arc(
        id: impl Into<String>,
        label: impl Into<String>,
        max_hp: i32,
        regen_per_sec: f32,
        offline_duration: f32,
        center_deg: f32,
        width_deg: f32,
        priority: u32,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hp: max_hp,
            max_hp,
            regen_per_sec,
            offline_duration,
            offline_remaining: 0.0,
            is_focused: false,
            hp_frac: 0.0,
            center_deg,
            width_deg,
            base_max_hp: max_hp,
            base_regen_per_sec: regen_per_sec,
            priority,
            damage_multiplier: 1.0,
        }
    }

    /// Whether this facing is currently active (not offline).
    pub fn is_online(&self) -> bool {
        self.offline_remaining <= 0.0
    }

    /// The sub-integer regen/decay accumulator — the fractional HP carried
    /// between fixed-timestep ticks (and the negative fractional decay an
    /// over-max non-focused arc bleeds off). Private to the facing because
    /// only `tick_with_regen_scale` mutates it during the sim; exposed
    /// read-only so the snapshot layer can capture it (issue #997 follow-up).
    /// A resume that dropped it would forgive the sub-tick regen debt exactly
    /// the way a dropped beam `damage_accumulator` would — see
    /// [`crate::snapshot::WeaponState::shield_charge`].
    pub fn hp_frac(&self) -> f32 {
        self.hp_frac
    }

    /// Apply `amount` damage to this facing.
    ///
    /// The incoming `amount` is scaled by `self.damage_multiplier` before
    /// being subtracted from HP, so focus-based damage reduction/increase
    /// is applied here.
    ///
    /// Returns the damage that passed through to the hull:
    /// - If the facing is offline, all damage passes through (unscaled).
    /// - If the facing absorbs the hit and HP drops to 0, the facing goes offline
    ///   and any overflow damage passes through to the hull.
    pub fn apply_damage(&mut self, amount: i32) -> i32 {
        if !self.is_online() {
            return amount;
        }
        let effective = (amount as f32 * self.damage_multiplier).round().max(0.0) as i32;
        if effective <= self.hp {
            self.hp -= effective;
            // `effective > 0` matters since #788: a facing may now legitimately
            // sit at 0 HP while ONLINE (the first tick of its regen ramp), and a
            // zero-damage hit on it must not re-arm the offline timer.
            if self.hp == 0 && effective > 0 {
                self.offline_remaining = self.offline_duration;
            }
            0
        } else {
            let overflow = effective - self.hp;
            self.hp = 0;
            self.offline_remaining = self.offline_duration;
            overflow
        }
    }

    /// Advance the facing by `dt` seconds: tick offline timer and regen at the
    /// arc's authored rate.
    pub fn tick(&mut self, dt: f32) {
        self.tick_with_regen_scale(dt, 1.0);
    }

    /// [`Self::tick`] with the regen rate scaled by `regen_scale` (issue #952).
    ///
    /// `1.0` is the arc's authored `regen_per_sec`. The scale is
    /// [`crate::core::messages::ModifierSlot::ShieldRegen`], driven by the `shields`
    /// power group. It deliberately does NOT touch the OFFLINE timer: how long
    /// a collapsed facing stays down is the arc's authored `offline_duration`
    /// and a damage-control property, not something the reactor buys.
    pub fn tick_with_regen_scale(&mut self, dt: f32, regen_scale: f32) {
        if !self.is_online() {
            self.offline_remaining = (self.offline_remaining - dt).max(0.0);
            if self.is_online() {
                // Just came back online. The facing restarts from EMPTY and
                // climbs at `regen_per_sec` from here (issue #788) — the offline
                // window is the authored no-damage delay, not a free recharge.
                self.hp = 0;
                self.hp_frac = 0.0;
            }
        } else {
            // Regen while online, accumulating fractional HP across frames.
            self.hp_frac += self.regen_per_sec * regen_scale.max(0.0) * dt;
            let whole = self.hp_frac as i32;
            if whole > 0 {
                self.hp = (self.hp + whole).min(self.max_hp);
                self.hp_frac -= whole as f32;
            }
            // Keep hp_frac from drifting if already at max.
            if self.hp >= self.max_hp {
                self.hp_frac = 0.0;
            }
        }
    }

    pub fn snapshot(&self) -> ShieldFacingSnapshot {
        ShieldFacingSnapshot {
            id: self.id.clone(),
            label: self.label.clone(),
            hp: self.hp,
            max_hp: self.max_hp,
            online: self.is_online(),
            offline_remaining: self.offline_remaining,
            is_focused: self.is_focused,
            center_deg: self.center_deg,
            width_deg: self.width_deg,
            priority: self.priority,
        }
    }
}

/// Default facing labels for 1, 2, 3, or 4 arcs, as `strings.csv` ids (issue
/// #977). The label rides the wire on `ShieldFacing.label` → the facing
/// snapshot and the `ShieldFacingDown`/`Restored` coordination payloads, and is
/// resolved to display text ("Fore" / "Aft" / …) by `localiseTree` at the
/// client boundary — Rust composes no English. Kept in lockstep with
/// [`default_arc_id`], which emits the parallel lowercase arc ids.
///
/// Facings are indexed going **counter-clockwise** (anti-clockwise) from forward:
/// Fore(0) → Port(1) → Aft(2) → Starboard(3)
///
/// The `_` (5+ arcs) branch derives a per-index id `shield.facing.arc_<n>`;
/// `strings.csv` carries rows through `arc_7`, enough for the evenly-spaced
/// legacy constructor's realistic range (production authors arcs via
/// [`ShieldSystem::from_arcs`]).
fn default_label(index: usize, num_facings: usize) -> String {
    match num_facings {
        1 => "shield.facing.all".to_string(),
        2 => match index {
            0 => "shield.facing.fore".to_string(),
            _ => "shield.facing.aft".to_string(),
        },
        3 => match index {
            0 => "shield.facing.fore".to_string(),
            1 => "shield.facing.port".to_string(),
            _ => "shield.facing.starboard".to_string(),
        },
        4 => match index {
            0 => "shield.facing.fore".to_string(),
            1 => "shield.facing.port".to_string(),
            2 => "shield.facing.aft".to_string(),
            _ => "shield.facing.starboard".to_string(),
        },
        _ => format!("shield.facing.arc_{}", index),
    }
}

/// Default arc IDs for 1, 2, 3, or 4 arcs — used by `ShieldSystem::new`
/// (the legacy evenly-spaced-facings constructor) to populate the
/// `ShieldFacing.id` field. Kept in lockstep with [`default_label`].
fn default_arc_id(index: usize, num_facings: usize) -> String {
    match num_facings {
        1 => "all".to_string(),
        2 => match index {
            0 => "fore".to_string(),
            _ => "aft".to_string(),
        },
        3 => match index {
            0 => "fore".to_string(),
            1 => "port".to_string(),
            _ => "starboard".to_string(),
        },
        4 => match index {
            0 => "fore".to_string(),
            1 => "port".to_string(),
            2 => "aft".to_string(),
            _ => "starboard".to_string(),
        },
        _ => format!("arc-{}", index),
    }
}

/// Runtime config for a single shield arc, produced by translating a
/// `[[shield_arc]]` TOML block into a value that `ShieldSystem::from_arcs`
/// can consume. Separate from the TOML struct in `entities/config.rs` so
/// `weapons/shield.rs` remains Bevy-free and TOML-schema-agnostic.
#[derive(Clone, Debug)]
pub struct ArcRuntimeConfig {
    /// Stable arc id (from `[[shield_arc]] id`).
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Arc centre bearing in degrees (0 = fore, 90 = starboard).
    pub center_deg: f32,
    /// Arc angular width in degrees.
    pub width_deg: f32,
    /// Per-arc override for max HP; falls back to ship-wide default when `None`.
    pub max_hp: Option<i32>,
    /// Per-arc override for regen/sec.
    pub regen_per_sec: Option<f32>,
    /// Per-arc override for offline duration.
    pub offline_duration: Option<f32>,
    /// Hit-routing priority. Higher value wins when multiple arcs cover the same bearing.
    /// Default 1 (matches `default_arc_priority()` in `entities/config.rs`).
    pub priority: u32,
}

/// The complete shield system, owning all facings.
pub struct ShieldSystem {
    pub facings: Vec<ShieldFacing>,
    /// Which facing (index) is currently focused by the Shields console.
    /// `None` means no focus (all facings at base values).
    pub focused_facing: Option<usize>,
    /// Configuration for focus bonus/penalty/decay.
    pub focus_config: ShieldFocusConfig,
    /// Ship-wide default max HP recorded at construction. Retained purely
    /// as introspection metadata (e.g. for debug UIs) — `recalculate_focus`
    /// consults each facing's own `base_max_hp` instead so per-arc TOML
    /// overrides survive focus changes.
    pub base_max_hp: i32,
    /// Ship-wide default regen/sec recorded at construction. See
    /// `base_max_hp`.
    pub base_regen_per_sec: f32,
}

impl ShieldSystem {
    /// Create a new shield system from the given config (evenly-spaced arcs).
    ///
    /// Legacy constructor kept for tests and any code path that pre-dates
    /// `[[shield_arc]]` TOML blocks. Production ship spawns should use
    /// [`ShieldSystem::from_arcs`] which reads designer-authored arcs.
    pub fn new(config: &ShieldConfig) -> Self {
        assert!(config.num_facings >= 1);
        let n = config.num_facings as f32;
        let width_deg = 360.0 / n;
        let facings = (0..config.num_facings)
            .map(|i| {
                let center_deg = ((i as f32) * width_deg) % 360.0;
                let mut f = ShieldFacing::new(
                    default_label(i, config.num_facings),
                    config.max_hp,
                    config.regen_per_sec,
                    config.offline_duration,
                );
                f.id = default_arc_id(i, config.num_facings);
                // Convert index -> counter-clockwise centre bearing.
                // Facing 0 (Fore) at 0°, facing 1 goes counter-clockwise so
                // Port maps to 270° (i.e. -90°) in the 4-facing default. The
                // legacy `facing_index_for_bearing` used
                //   angle = (-bearing).rem_euclid(TAU); shifted += TAU/(2n)
                // which rotates through Port(1) → Aft(2) → Starboard(3), so
                // facing 1's centre in world-bearing terms sits at -width_deg
                // (i.e. 360-width_deg). Match that convention here.
                f.center_deg = if i == 0 {
                    0.0
                } else {
                    (360.0 - center_deg) % 360.0
                };
                f.width_deg = width_deg;
                f
            })
            .collect();
        Self {
            facings,
            focused_facing: None,
            focus_config: ShieldFocusConfig::default(),
            base_max_hp: config.max_hp,
            base_regen_per_sec: config.regen_per_sec,
        }
    }

    /// Create a new shield system from designer-authored arc configs
    /// (issue #514).
    ///
    /// Each arc carries its own `id`, `label`, `center_deg`, and `width_deg`
    /// so the ship can have any number of arcs at any widths. Per-arc
    /// overrides for `max_hp` / `regen_per_sec` / `offline_duration` fall
    /// back to `ship_wide` values when `None` — that ship-wide bundle
    /// mirrors `[shields_console.base]`.
    ///
    /// Requires at least one arc. Panics on empty input to match
    /// `ShieldSystem::new`'s `num_facings >= 1` invariant.
    pub fn from_arcs(arcs: &[ArcRuntimeConfig], ship_wide: &ShieldConfig) -> Self {
        assert!(!arcs.is_empty());
        let facings: Vec<ShieldFacing> = arcs
            .iter()
            .map(|a| {
                ShieldFacing::new_arc(
                    a.id.clone(),
                    a.label.clone(),
                    a.max_hp.unwrap_or(ship_wide.max_hp),
                    a.regen_per_sec.unwrap_or(ship_wide.regen_per_sec),
                    a.offline_duration.unwrap_or(ship_wide.offline_duration),
                    a.center_deg,
                    a.width_deg,
                    a.priority,
                )
            })
            .collect();
        // Each facing already carries its own per-arc baseline in
        // `ShieldFacing::base_max_hp` / `base_regen_per_sec` (set by
        // `new_arc` from the per-arc override or the ship-wide fallback),
        // so `recalculate_focus` reads from the facing itself. The
        // ship-wide values on `ShieldSystem` are recorded here purely as
        // introspection metadata.
        Self {
            facings,
            focused_facing: None,
            focus_config: ShieldFocusConfig::default(),
            base_max_hp: ship_wide.max_hp,
            base_regen_per_sec: ship_wide.regen_per_sec,
        }
    }

    /// Set the focused facing by index, or `None` to clear focus.
    /// Recalculates each facing's effective max_hp, regen, and is_focused flag.
    pub fn set_focused_facing(&mut self, facing: Option<usize>) {
        self.focused_facing = facing;
        self.recalculate_focus();
    }

    /// Restore ONLY the runtime charge state of each facing from a snapshot,
    /// matching by arc `id`, then re-derive every focus-dependent field so the
    /// restored system is byte-identical to the captured one (issue #997
    /// follow-up).
    ///
    /// Each tuple is `(id, hp, hp_frac, offline_remaining, is_focused)`. This
    /// overwrites the charge the sim accumulated — `hp`, the fractional
    /// `hp_frac` accumulator, the `offline_remaining` timer, and the
    /// `is_focused` flag — and NOTHING the ship TOML rebuilt at spawn: arc
    /// ids, labels, `base_max_hp` / `base_regen_per_sec`, `priority`,
    /// `center_deg` / `width_deg`, `focus_config` and the ship-wide frequency
    /// are all left untouched. An id the snapshot does not mention is left as
    /// the bootstrap built it (the content digest is what refuses a save
    /// written against a different arc layout).
    ///
    /// `focused_facing` is re-derived from the restored `is_focused` flags —
    /// the console carries a single focus slot, so at most one facing is
    /// focused — and then [`Self::recalculate_focus`] rebuilds `max_hp`,
    /// `regen_per_sec` and `damage_multiplier` from each arc's own baseline.
    /// That recalc reads `focused_facing` and writes only the focus-derived
    /// fields; it never touches `hp` / `hp_frac` / `offline_remaining`, so the
    /// charge written just above survives it — the set order is load-bearing
    /// and is why the charge write comes first.
    pub fn restore_facings(&mut self, charge: &[(String, i32, f32, f32, bool)]) {
        for (id, hp, hp_frac, offline_remaining, is_focused) in charge {
            if let Some(facing) = self.facings.iter_mut().find(|f| &f.id == id) {
                facing.hp = *hp;
                facing.hp_frac = *hp_frac;
                facing.offline_remaining = *offline_remaining;
                facing.is_focused = *is_focused;
            }
        }
        self.focused_facing = self.facings.iter().position(|f| f.is_focused);
        self.recalculate_focus();
    }

    /// Recalculate effective max_hp, regen_per_sec, and is_focused for all facings
    /// based on the current `focused_facing`.
    ///
    /// Each facing's effective values are computed from **that facing's own
    /// `base_max_hp` / `base_regen_per_sec`** (set at construction from the
    /// arc's TOML override, or the ship-wide default when no override was
    /// given). This preserves per-arc overrides across focus changes — the
    /// ship-wide `self.base_max_hp` / `self.base_regen_per_sec` fields are
    /// not consulted here.
    fn recalculate_focus(&mut self) {
        let fc = &self.focus_config;
        for (i, facing) in self.facings.iter_mut().enumerate() {
            if self.focused_facing == Some(i) {
                // Focused arc: bonus applied to this arc's own baseline.
                facing.max_hp = facing.base_max_hp + fc.bonus_max_hp;
                facing.regen_per_sec = facing.base_regen_per_sec + fc.bonus_regen;
                facing.is_focused = true;
                facing.damage_multiplier = fc.focused_damage_multiplier;
            } else if self.focused_facing.is_some() {
                // Another arc is focused: penalty on this arc's own baseline.
                facing.max_hp = (facing.base_max_hp - fc.penalty_max_hp).max(0);
                facing.regen_per_sec = (facing.base_regen_per_sec - fc.penalty_regen).max(0.0);
                facing.is_focused = false;
                facing.damage_multiplier = fc.unfocused_damage_multiplier;
            } else {
                // No focus: restore this arc's own baseline values.
                facing.max_hp = facing.base_max_hp;
                facing.regen_per_sec = facing.base_regen_per_sec;
                facing.is_focused = false;
                facing.damage_multiplier = 1.0;
            }
        }
    }

    /// Determine the facing index hit by an attacker at `bearing_relative` radians
    /// (angle relative to the ship's own yaw, in (-π, π], anti-clockwise positive).
    ///
    /// Two-phase routing:
    /// 1. **In-arc pass** — iterate arcs in declaration order and return the
    ///    first arc whose window
    ///    `[center_deg - width_deg/2, center_deg + width_deg/2]` (mod 360°)
    ///    strictly contains the bearing. When multiple arcs contain the
    ///    bearing (overlapping widths), **declaration order wins**: the
    ///    earlier `[[shield_arc]]` block in the ship TOML takes the hit.
    ///    Overlap is a designer choice and this rule makes routing
    ///    deterministic; if a designer wants a narrower arc to override a
    ///    wider one they must declare the narrower arc first.
    ///    A strict `<` comparison is used (not `<=`) so arcs that share
    ///    exact boundaries do not both match; boundary bearings fall through
    ///    to phase 2.
    /// 2. **Centre-nearest fallback** — when no arc window strictly contains
    ///    the bearing (arc gaps or boundary bearings), pick the arc whose
    ///    centre is angularly closest. Tie-break prefers the arc whose
    ///    signed delta is more negative (clockwise from bearing) to match
    ///    the historical convention that -π/2 → Port(1) rather than Fore(0).
    pub fn facing_index_for_bearing(&self, bearing_relative: f32) -> usize {
        assert!(!self.facings.is_empty());
        // Convert relative bearing (-π..π] to world-bearing degrees
        // (0..360) with fore=0 and starboard=90 (i.e. clockwise from fore).
        let deg = bearing_relative.to_degrees().rem_euclid(360.0);

        // Signed angular distance in (-180, 180] between `deg` and `center`.
        let signed_delta = |center: f32| -> f32 {
            let center = center.rem_euclid(360.0);
            let mut delta = deg - center;
            while delta > 180.0 {
                delta -= 360.0;
            }
            while delta <= -180.0 {
                delta += 360.0;
            }
            delta
        };

        // ── Phase 1: priority-aware in-arc pass.
        // Find the highest priority value among all arcs that strictly contain
        // the bearing. Then return the first arc at that priority that is
        // online. If all arcs at the highest priority tier are offline, fall
        // through to the next lower priority tier, and so on. Declaration
        // order is the tie-break within a priority tier.
        //
        // Boundary handling: use strict `<` instead of `<=` so arcs share
        // boundaries cleanly — an arc whose edge exactly touches another
        // arc's edge defers to a later (narrower) arc if declared, or
        // falls into the centre-nearest phase below. This preserves the
        // 4-facing default's behaviour: at exact bearing -90° the fore
        // and port arcs both have `|delta| = 45` (their shared boundary),
        // so neither matches strictly and the centre-nearest fallback
        // picks port (see phase 2 tie-break).

        // Collect all (index, priority) for arcs that strictly contain the bearing.
        let in_arc: Vec<(usize, u32)> = self
            .facings
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                if f.width_deg <= 0.0 {
                    return None;
                }
                let half = f.width_deg * 0.5;
                let dist = signed_delta(f.center_deg).abs();
                if dist < half - 1e-4 {
                    Some((i, f.priority))
                } else {
                    None
                }
            })
            .collect();

        if !in_arc.is_empty() {
            // Walk priority tiers from highest to lowest.
            let remaining_priorities: Vec<u32> = {
                let mut ps: Vec<u32> = in_arc.iter().map(|(_, p)| *p).collect();
                ps.sort_unstable();
                ps.dedup();
                ps.reverse(); // highest first
                ps
            };

            for tier in &remaining_priorities {
                // First online arc at this priority tier (declaration order).
                let candidate = in_arc
                    .iter()
                    .filter(|(_, p)| p == tier)
                    .find(|(i, _)| self.facings[*i].is_online())
                    .map(|(i, _)| *i);
                if let Some(idx) = candidate {
                    return idx;
                }
                // All arcs at this tier are offline; try next lower tier.
            }

            // Every in-arc facing (all tiers) is offline — pick the highest-priority
            // offline arc (first declaration order) so damage routes to *something*
            // rather than bypassing to phase 2 / centre-nearest.
            let top_priority = remaining_priorities[0];
            return in_arc
                .iter()
                .filter(|(_, p)| *p == top_priority)
                .map(|(i, _)| *i)
                .next()
                .unwrap(); // in_arc is non-empty
        }

        // ── Phase 2: no in-arc match — pick centre-nearest.
        let mut best_idx = 0usize;
        let mut best_dist = f32::INFINITY;
        let mut best_signed = f32::INFINITY;
        for (i, f) in self.facings.iter().enumerate() {
            if f.width_deg <= 0.0 {
                continue;
            }
            let signed = signed_delta(f.center_deg);
            let dist = signed.abs();
            let is_closer = dist < best_dist - 1e-4;
            let is_tied = (dist - best_dist).abs() < 1e-4;
            if is_closer || (is_tied && signed < best_signed) {
                best_dist = dist;
                best_signed = signed;
                best_idx = i;
            }
        }

        // Fallback: no arc had a positive width — legacy even-spaced.
        if best_dist.is_infinite() {
            let n = self.facings.len() as f32;
            let angle = (-bearing_relative).rem_euclid(TAU);
            let shifted = (angle + TAU / (2.0 * n)).rem_euclid(TAU);
            let idx = (shifted / (TAU / n)) as usize;
            return idx.min(self.facings.len() - 1);
        }
        best_idx
    }

    /// HP of the ONE arc a shot fired from `(attacker_x, attacker_z)` would
    /// strike — the reading the fleet's torpedo doctrine is authored against.
    ///
    /// Resolves the attack bearing in this ship's own frame (arcs are authored
    /// relative to its facing) and routes it through
    /// [`Self::facing_index_for_bearing`], the SAME resolver `apply_damage` uses,
    /// so a predictive gate and the eventual hit agree about which arc is in the
    /// way.
    ///
    /// An OFFLINE arc reports `0`: it passes damage straight through to the
    /// hull, so it is not blocking the shot. That, and the per-arc question
    /// itself, are why this is not a sum over `facings` — a healthy rear arc
    /// must not veto a shot into a collapsed front one while the attacker sits
    /// dead ahead.
    ///
    /// Lives here rather than at either call site because both
    /// `console_ai::server::ai_torpedo_auto_fire` (seeding the tube's launch
    /// facts) and `console::weapons::tick_weapons_arc_request` (seeding the
    /// weapons doctrine's family-preference facts, issue #956) need exactly this
    /// number, and two copies of a bearing→arc resolution is two chances to
    /// disagree about which arc a shot meets.
    pub fn hp_facing_attacker(
        &self,
        attacker_x: f32,
        attacker_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> i32 {
        let incoming = attacker_bearing_relative(attacker_x, attacker_z, ship_x, ship_z, ship_yaw);
        let facing = &self.facings[self.facing_index_for_bearing(incoming)];
        if facing.is_online() {
            facing.hp
        } else {
            0
        }
    }

    /// Apply `amount` damage from `bearing_relative` (radians relative to ship yaw).
    ///
    /// Returns the hull passthrough damage (0 if shields fully absorbed the hit).
    pub fn apply_damage(&mut self, amount: i32, bearing_relative: f32) -> i32 {
        let idx = self.facing_index_for_bearing(bearing_relative);
        self.facings[idx].apply_damage(amount)
    }

    /// Apply `amount` damage uniformly across all shield facings.
    ///
    /// Each facing receives an equal share; any remainder is distributed
    /// one-at-a-time to the first `amount % N` facings. Returns the total
    /// hull passthrough damage (sum of overflow from each facing).
    pub fn apply_uniform_damage(&mut self, amount: i32) -> i32 {
        let n = self.facings.len();
        if n == 0 {
            return amount;
        }
        let base = amount / n as i32;
        let rem = amount % n as i32;
        let mut total_leak = 0i32;
        for i in 0..n {
            let facing_amount = base + if (i as i32) < rem { 1 } else { 0 };
            total_leak += self.facings[i].apply_damage(facing_amount);
        }
        total_leak
    }

    /// Advance all facings by `dt` seconds (regen + offline timers + focus decay).
    ///
    /// Non-focused facings whose HP exceeds their (reduced) effective max_hp decay at
    /// `focus_config.decay_rate` per second until they reach the cap.  While decaying
    /// toward the reduced maximum, regen is suppressed so the transition is gradual
    /// (rather than snapping to max_hp in a single tick).
    pub fn tick(&mut self, dt: f32) {
        self.tick_with_regen_scale(dt, 1.0);
    }

    /// [`Self::tick`] with every facing's regen rate scaled by `regen_scale`
    /// (issue #952 — the `shields` power group's
    /// [`crate::core::messages::ModifierSlot::ShieldRegen`]).
    ///
    /// Focus DECAY is left unscaled: it is the cost of having concentrated the
    /// grid somewhere, and letting reactor power soften it would turn the focus
    /// trade into a straight buff.
    pub fn tick_with_regen_scale(&mut self, dt: f32, regen_scale: f32) {
        for (i, facing) in self.facings.iter_mut().enumerate() {
            let is_decaying = self.focused_facing != Some(i) && facing.hp > facing.max_hp;

            if is_decaying {
                // Apply focus decay toward effective max_hp (no regen while decaying).
                // Accumulate fractional decay in hp_frac (negative while decaying) so
                // sub-integer rates are applied correctly across frames.
                facing.hp_frac -= self.focus_config.decay_rate * dt;
                let whole = facing.hp_frac.abs() as i32;
                if whole > 0 {
                    let target = facing.max_hp;
                    facing.hp = (facing.hp - whole).max(target);
                    facing.hp_frac += whole as f32; // remove the consumed integer part
                }
                // Clear accumulator once we've reached (or gone below) the reduced max.
                if facing.hp <= facing.max_hp {
                    facing.hp = facing.max_hp;
                    facing.hp_frac = 0.0;
                }
                // Still tick offline timer if applicable.
                if !facing.is_online() {
                    facing.offline_remaining = (facing.offline_remaining - dt).max(0.0);
                }
            } else {
                facing.tick_with_regen_scale(dt, regen_scale);
            }
        }
    }

    /// Snapshot all facings for broadcast.
    pub fn snapshot(&self) -> Vec<ShieldFacingSnapshot> {
        self.facings.iter().map(|f| f.snapshot()).collect()
    }

    /// Total shield health as a fraction of total capacity, `[0, 1]`
    /// (issue #788).
    ///
    /// The whole-ship reading a doctrine reasons about: "are my shields back?"
    /// is a question about the ship, not about one arc. A ship with no arcs (or
    /// no capacity) reads `0.0` — it has no shields to recover, which is the
    /// safe answer for a re-entry gate that must not open on a missing system.
    ///
    /// Now that a collapsed facing comes back at 0 HP and ramps, this value
    /// genuinely traverses the interval instead of jumping 1.0 → 0.0 → 1.0.
    pub fn fraction(&self) -> f32 {
        let total_max: i32 = self.facings.iter().map(|f| f.max_hp).sum();
        if total_max <= 0 {
            return 0.0;
        }
        let total_hp: i32 = self.facings.iter().map(|f| f.hp.max(0)).sum();
        (total_hp as f32 / total_max as f32).clamp(0.0, 1.0)
    }
}

impl Default for ShieldSystem {
    fn default() -> Self {
        let mut sys = Self::new(&ShieldConfig::default());
        sys.focus_config = ShieldFocusConfig::default();
        sys
    }
}

/// Compute the bearing of an attacker position relative to the ship's yaw.
///
/// `attacker_x`, `attacker_z` — world-space position of the attacker (or the
///   point from which the hit originates).
/// `ship_x`, `ship_z` — world-space position of the ship.
/// `ship_yaw` — ship's current yaw in radians (0 = forward along –Z axis).
///
/// Returns the bearing in `(-π, π]` measured anti-clockwise from the ship's
/// forward direction, consistent with `facing_index_for_bearing`.
pub fn attacker_bearing_relative(
    attacker_x: f32,
    attacker_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> f32 {
    // World-space direction from ship to attacker.
    let dx = attacker_x - ship_x;
    let dz = attacker_z - ship_z;

    // World-space bearing of the attacker (atan2 in XZ plane).
    // We use atan2(dx, -dz) so that "forward" (dx=0, dz<0) gives 0.
    let world_bearing = simmath::atan2(dx, -dz);

    // Subtract ship yaw to get bearing relative to the ship's own frame.
    // Then normalise to (-π, π].
    let relative = world_bearing - ship_yaw;
    let tau = std::f32::consts::TAU;

    ((relative + std::f32::consts::PI).rem_euclid(tau)) - std::f32::consts::PI
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "shield_tests.rs"]
mod tests;
