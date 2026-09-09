//! Entity schema: weapons. Public paths remain in the parent module.
use super::*;

/// Stable identifier for a phaser bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.phaser_banks]]`. Used on the wire
/// to address a specific bank (e.g. `FirePhaser { bank: "port" }`).
pub type PhaserBankId = String;

/// One `[[weapons_console.phaser_banks]]` entry. Defines a single phaser
/// bank's orientation on the ship, its full fire arc (used for manual
/// fire validity and for the radar arc overlay), its narrower auto-fire
/// arc (used by `console_ai` for autonomous firing decisions), and its
/// effective beam range.
///
/// `facing_deg` is the bank's centre bearing in ship-local degrees:
/// `0` = forward (−Z), `90` = starboard (+X), `180` = aft (+Z),
/// `-90` / `270` = port. Wraps freely; only the wrapped direction
/// matters.
///
/// `fire_arc_deg` is the full arc width centred on `facing_deg`. A bank
/// with `facing_deg = -90`, `fire_arc_deg = 180` covers the port
/// hemisphere from forward to aft. Values must be in `(0, 360]`.
///
/// `auto_arc_deg` is the (narrower) auto-fire window, also centred on
/// `facing_deg`. Must satisfy `0 < auto_arc_deg <= fire_arc_deg`.
///
/// `beam_range` is in world units. When `0.0`, falls back to
/// `PhaserCombatConfig::DEFAULT_PHASER_RANGE`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PhaserBankConfig {
    pub id: PhaserBankId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    pub auto_arc_deg: f32,
    #[serde(default)]
    pub beam_range: f32,
    /// Damage applied to the target per second of active beam.
    /// When `0.0`, falls back to `PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC`.
    #[serde(default)]
    pub beam_damage_per_sec: f32,
    /// Active beam duration in seconds. When `0.0`, falls back to
    /// `PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS`.
    #[serde(default)]
    pub beam_duration_secs: f32,
    /// Post-beam cooldown in seconds. When `0.0`, falls back to
    /// `PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS`.
    #[serde(default)]
    pub cooldown_secs: f32,
    /// Per-cycle jitter on this bank's firing rhythm, as a fraction
    /// (issue #929). `0.33` means +/-33 %.
    ///
    /// Each time the bank LIGHTS, one factor is drawn uniformly from
    /// `[1 - cycle_jitter, 1 + cycle_jitter)` — half-open at the top, because the
    /// draw is `ship::damage::unit_f32`, whose `1.0` is unreachable by
    /// construction — and applied to BOTH that cycle's
    /// firing duration AND the cooldown that follows it. Linked deliberately: a
    /// cycle that burns longer also rests longer, so the mean duty cycle is
    /// exactly `beam_duration_secs / (beam_duration_secs + cooldown_secs)`
    /// whatever the jitter is, and only the PHASE moves.
    ///
    /// What that buys is de-synchronisation. Two banks that light together stay
    /// together for ever on a fixed cadence, fire together and go cold together,
    /// and a shield arc regenerating through the synchronised dead window
    /// recovers everything; with jitter their phases random-walk apart and the
    /// hull's coverage of the target becomes near-continuous. The mechanism is
    /// general and the adoption is per-hull.
    ///
    /// `0.0` (the default, and what every hull but `alliance_cruiser` authors)
    /// is EXACTLY the fixed cycle that predates this field — no draw is taken at
    /// all, so a hull that does not author it does not touch the seeded stream.
    /// Must be in `[0.0, 1.0)`; at 1.0 a cycle could be drawn to zero length.
    #[serde(default)]
    pub cycle_jitter: f32,
    /// RGBA beam colour as a 4-element float array `[r, g, b, a]` in 0.0–1.0.
    /// When absent (empty vec), the renderer falls back to `beam_render::DEFAULT_BEAM_COLOR`.
    #[serde(default)]
    pub beam_color: Vec<f32>,
    /// Fraction of beam damage that bypasses shields. When `None`, defaults to `0.0`.
    /// Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: Option<f32>,
    /// Optional rig-marker name linking this bank to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When resolvable, downstream
    /// systems may use the marker's position/direction as the beam origin;
    /// when absent or unresolved they fall back to the hull-offset default.
    #[serde(default)]
    pub marker: Option<String>,
    /// Inline stateless AI policy for this bank's open-fire decision
    /// (issue #781). When authored it is validated at content load and drives
    /// `ai_phaser_auto_fire`'s per-bank fire gate; when absent the canonical
    /// [`default_phaser_bank_ai_config`] (unconditional fire) is synthesised at
    /// spawn so baseline auto-fire is preserved. An explicit `idle = true` is the
    /// per-bank opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

/// Stable identifier for a blaster bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.blaster_banks]]` (issue #631).
pub type BlasterBankId = String;

/// One `[[weapons_console.blaster_banks]]` entry (issue #631).
///
/// A blaster bank fires straight-flying projectiles in data-driven volleys
/// with linear motion prediction at fire time — no homing, no mid-flight
/// correction.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward). Note: do NOT
/// add `serde(deny_unknown_fields)` here — future issues will add more
/// fields (recoil, screenshake, visual variants).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BlasterBankConfig {
    pub id: BlasterBankId,
    #[serde(default)]
    pub facing_deg: f32,
    #[serde(default = "default_blaster_fire_arc_deg")]
    pub fire_arc_deg: f32,
    #[serde(default = "default_blaster_volley_count")]
    pub volley_count: u32,
    #[serde(default = "default_blaster_volley_interval_secs")]
    pub volley_interval_secs: f32,
    /// Post-volley cooldown in seconds.
    #[serde(default = "default_blaster_cooldown_secs")]
    pub cooldown_secs: f32,
    /// Charge time before firing begins. `0` = instant (click-to-fire);
    /// `>0` = hold-to-fire (reserved for a later issue).
    #[serde(default)]
    pub charge_time_secs: f32,
    #[serde(default = "default_blaster_projectile_speed")]
    pub projectile_speed: f32,
    #[serde(default = "default_blaster_collision_radius")]
    pub collision_radius: f32,
    #[serde(default = "default_blaster_visual_scale")]
    pub visual_scale: f32,
    #[serde(default = "default_blaster_damage")]
    pub damage: i32,
    /// Fraction `[0.0, 1.0]` of damage that bypasses shields entirely.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Recoil impulse magnitude (reserved for a later issue).
    #[serde(default)]
    pub recoil_impulse: f32,
    /// Screenshake magnitude (reserved for a later issue).
    #[serde(default)]
    pub screenshake_magnitude: f32,
    /// Optional rig-marker name linking this bank to a mount point. In the
    /// single-barrel (backward-compat) case this is the sole projectile origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #765). Each entry is a rig-marker
    /// name; a barrel-index pattern step addresses these by position. When
    /// empty the bank has one implicit barrel = `marker` (unchanged behaviour).
    #[serde(default)]
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #765). A step fires its listed
    /// barrel indices simultaneously at `offset_secs`; successive steps at
    /// increasing offsets alternate. When empty the bank fires the uniform
    /// `volley_count` volley from the single implicit barrel (unchanged).
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    /// Maximum range in world units. Projectile lifespan is computed per-bank
    /// as `range / projectile_speed`. Use `default_blaster_range` (35.0) when
    /// absent from TOML.
    #[serde(default = "default_blaster_range")]
    pub range: f32,
    /// Inline stateless AI policy for this bank's open-fire decision
    /// (issue #781). When authored it is validated at content load and drives
    /// `tick_blaster_auto_fire`'s per-bank fire gate; when absent the canonical
    /// [`default_blaster_bank_ai_config`] (unconditional fire) is synthesised at
    /// spawn so baseline auto-fire is preserved. An explicit `idle = true` is the
    /// per-bank opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

fn default_blaster_fire_arc_deg() -> f32 {
    90.0
}
fn default_blaster_volley_count() -> u32 {
    3
}
fn default_blaster_volley_interval_secs() -> f32 {
    0.15
}
fn default_blaster_cooldown_secs() -> f32 {
    3.0
}
fn default_blaster_projectile_speed() -> f32 {
    40.0
}
fn default_blaster_collision_radius() -> f32 {
    1.5
}
fn default_blaster_visual_scale() -> f32 {
    1.0
}
fn default_blaster_damage() -> i32 {
    20
}
fn default_blaster_range() -> f32 {
    35.0
}

impl BlasterBankConfig {
    /// Convert this TOML config into a runtime `crate::weapons::blaster::BlasterBankConfig`.
    pub fn to_runtime(&self) -> crate::weapons::blaster::BlasterBankConfig {
        crate::weapons::blaster::BlasterBankConfig {
            id: self.id.clone(),
            facing_deg: self.facing_deg,
            fire_arc_deg: self.fire_arc_deg,
            volley_count: self.volley_count,
            volley_interval_secs: self.volley_interval_secs,
            cooldown_secs: self.cooldown_secs,
            charge_time_secs: self.charge_time_secs,
            projectile_speed: self.projectile_speed,
            collision_radius: self.collision_radius,
            visual_scale: self.visual_scale,
            damage: self.damage,
            shield_pierce: self.shield_pierce,
            recoil_impulse: self.recoil_impulse,
            screenshake_magnitude: self.screenshake_magnitude,
            marker: self.marker.clone(),
            barrels: self.barrels.clone(),
            pattern: self.pattern.clone(),
            range: self.range,
        }
    }
}

/// Stable identifier for a torpedo tube, parsed verbatim from the TOML
/// `id` field on `[[torpedoes.tubes]]`. Used on the wire to address a
/// specific tube (e.g. `FireTorpedo { tube: "fore_port" }`).
pub type TorpedoTubeId = String;

/// One `[[torpedoes.tubes]]` entry. Defines a single torpedo tube's
/// orientation and launch arc. Ammo is **not** per-tube; the entire
/// ship draws from the shared `[torpedoes].count` pool.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoTubeConfig {
    pub id: TorpedoTubeId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    /// Per-tube load/unload time override in seconds. Falls back to the
    /// global `[torpedoes] load_time` when absent.
    #[serde(default)]
    pub load_time: Option<f32>,
    /// Optional rig-marker name linking this tube to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When absent or unresolved,
    /// callers fall back to the ship-centre launch origin. In the single-barrel
    /// (backward-compat) case this is the sole launch origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #766). Each entry is a rig-marker
    /// name; a barrel-index pattern step addresses these by position. When
    /// empty the tube has one implicit barrel = `marker` (unchanged behaviour).
    /// Reuses the exact schema blasters wired in issue #765.
    #[serde(default)]
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #766). A step lists barrel
    /// indices; successive steps at increasing offsets order the barrels a
    /// volley's rounds leave from. The pattern governs only WHICH barrel each
    /// launched round leaves from and in what order — never how many rounds
    /// exist: the magazine, `loaded_count`, and the burst cadence remain the
    /// sole authority over the torpedo count. When empty the tube launches
    /// from the single implicit barrel exactly as before.
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    /// Maximum number of torpedoes that can be loaded into this tube at once
    /// (volley capacity). Default `1` preserves existing single-shot
    /// behaviour. Values greater than 1 allow the tube to queue multiple
    /// torpedoes and fire them as a rapid burst.
    #[serde(default = "default_tube_volley_max")]
    pub volley_max: u32,
    /// How many rounds an AI-operated crew keeps loaded in this tube.
    ///
    /// The AI has no console to poke, so it issues the same
    /// `SetTorpedoVolleyTarget` command a human operator's console sends
    /// (see `console_ai::server::ai_torpedo_load`) and this is the count it
    /// asks for. Falls back to `[torpedoes] ai_volley_target`, then to
    /// [`Self::volley_max`] — a designer who says nothing gets "the AI keeps
    /// the tube as full as it can", which is the sane default for a hull that
    /// authored tubes at all. Clamped to `volley_max` at runtime.
    /// `Some(0)` disables AI loading for this tube.
    #[serde(default)]
    pub ai_target_count: Option<u32>,
    /// Inline stateless AI policy for this tube's load + launch decisions
    /// (issue #782). When authored it is validated at content load and drives
    /// `ai_torpedo_load`'s per-tube load gate and `ai_torpedo_auto_fire`'s
    /// per-tube launch gate; when absent the canonical
    /// [`default_torpedo_tube_ai_config`] (unconditional load + launch) is
    /// synthesised at spawn so baseline behaviour is preserved. An explicit
    /// `idle = true` is the per-tube opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

fn default_tube_volley_max() -> u32 {
    1
}

/// Load-time validation for the Steering axis's MODIFIER param sets (#929).
///
/// The leg gates around these — [`COMBAT_ORBIT_PARAMS`], `TORPEDO_BEARING_PARAMS`
/// — answer a half-authored set by declining the whole arm at runtime, which is
/// the right answer for them: a leg that does not happen is a behaviour a
/// designer can watch not happen. Arc-keeping and the weak-broadside flip are
/// different in kind. They MODIFY a ring that is already running, so a
/// half-authored set produces a hull that flies a slightly wrong ring for ever
/// and never says why. That belongs at load, where the author is still looking.
///
/// Three claims, and each one is a hazard that was live before it was checked:
///
///   * either every scalar of a set or none of it. The mixed case is always a
///     mistake — nothing reads a lone `arc_keep_speed`.
///   * `arc_keep_speed > 0.0`. Zero does not mean "slow", it means STOP, and a
///     hull parked inside a hostile's guns is exactly the hazard
///     [`COMBAT_ORBIT_PARAMS`]'s all-or-nothing gate was written to prevent.
///     (No upper bound: authoring it ABOVE `combat_orbit_speed` is a legitimate,
///     if odd, "go faster near the edge" and the doctrine still flies.)
///   * `weak_shield_restore_hp >= weak_shield_flip_hp`. An inverted pair is not a
///     deadband; the latch would clear on the tick it set. This replaces a
///     silent `restore.max(flip)` clamp at the read site — substituting the
///     value an author "appears to have meant" is how a hull ends up flying a
///     doctrine nobody wrote down.
pub(crate) fn validate_helm_steering_param_sets(
    ai: &crate::entities::ai_policy_schema::FineSystemAiConfigToml,
) -> Result<(), String> {
    for set in [
        crate::ship::helm_ai::ARC_KEEP_PARAMS,
        crate::ship::helm_ai::WEAK_SHIELD_FLIP_PARAMS,
    ] {
        let present: Vec<&str> = set
            .iter()
            .copied()
            .filter(|name| ai.param.contains_key(*name))
            .collect();
        if !present.is_empty() && present.len() != set.len() {
            let missing: Vec<&str> = set
                .iter()
                .copied()
                .filter(|name| !ai.param.contains_key(*name))
                .collect();
            return Err(format!(
                "[helm_console.steering_ai.param] authors {present:?} without {missing:?}: \
                 these are read as one set, and a partially authored one modifies the \
                 fighting ring in a way nothing else in the hull can explain"
            ));
        }
    }
    if let Some(speed) = ai
        .param
        .get(crate::ship::helm_ai::ARC_KEEP_SPEED_PARAM)
        .copied()
    {
        if speed <= 0.0 {
            return Err(format!(
                "[helm_console.steering_ai.param] arc_keep_speed = {speed} must be > 0: \
                 zero is not a slow ring, it is a parked ship inside a hostile's guns"
            ));
        }
    }
    if let (Some(flip), Some(restore)) = (
        ai.param
            .get(crate::ship::helm_ai::WEAK_SHIELD_FLIP_HP_PARAM)
            .copied(),
        ai.param
            .get(crate::ship::helm_ai::WEAK_SHIELD_RESTORE_HP_PARAM)
            .copied(),
    ) {
        if restore < flip {
            return Err(format!(
                "[helm_console.steering_ai.param] weak_shield_restore_hp = {restore} is below \
                 weak_shield_flip_hp = {flip}: that is not a deadband, it is a latch that \
                 clears on the tick it sets"
            ));
        }
    }
    Ok(())
}

/// Validate a `[[weapons_console.phaser_banks]]` list parsed from TOML.
///
/// Rejects:
///   - empty list (caller may decide to fall back to a single hardcoded
///     bank — this validator returns `Err` so callers see the empty list)
///   - duplicate `id` values
///   - `fire_arc_deg` outside `(0, 360]`
///   - `auto_arc_deg` outside `(0, fire_arc_deg]`
pub fn validate_phaser_banks(banks: &[PhaserBankConfig]) -> Result<(), String> {
    if banks.is_empty() {
        return Err("phaser_banks list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate phaser bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "phaser bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        if !(b.auto_arc_deg > 0.0 && b.auto_arc_deg <= b.fire_arc_deg) {
            return Err(format!(
                "phaser bank '{}' has auto_arc_deg={} outside (0, fire_arc_deg={}]",
                b.id, b.auto_arc_deg, b.fire_arc_deg
            ));
        }
        // `cycle_jitter` scales BOTH halves of the cycle, so 1.0 admits a draw
        // of exactly zero — a beam that lights and expires in the same tick, and
        // a cooldown of no length at all. Rejected at load rather than clamped
        // at apply time, because a hull that authored it meant something by it.
        if !(b.cycle_jitter >= 0.0 && b.cycle_jitter < 1.0) {
            return Err(format!(
                "phaser bank '{}' has cycle_jitter={} outside [0.0, 1.0)",
                b.id, b.cycle_jitter
            ));
        }
    }
    Ok(())
}

/// Validate a `[[weapons_console.blaster_banks]]` list parsed from TOML
/// (issue #765).
///
/// An empty list is accepted — most hulls carry no blasters. Rejects:
///   - duplicate `id` values,
///   - `fire_arc_deg` outside `(0, 360]`,
///   - a barrel pattern that fires no barrels in a step, references a barrel
///     index beyond the declared barrel count, uses a negative offset, or is
///     omitted while more than one barrel is declared (see
///     [`crate::weapons::pattern::validate_barrel_pattern`]).
///
/// The barrel count is the authored `barrels.len()`, or `1` for the implicit
/// single-barrel (backward-compat) bank.
pub fn validate_blaster_banks(banks: &[BlasterBankConfig]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate blaster bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "blaster bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        let barrel_count = if b.barrels.is_empty() {
            1
        } else {
            b.barrels.len()
        };
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("blaster bank '{}'", b.id),
            barrel_count,
            &b.pattern,
        )?;
    }
    Ok(())
}

/// Validate a `[[torpedoes.tubes]]` list parsed from TOML.
///
/// Rejects: empty list, duplicate `id`, `fire_arc_deg` outside `(0, 360]`, and
/// (issue #766) a barrel pattern that fires no barrels in a step, references a
/// barrel index beyond the declared barrel count, uses a negative offset, or is
/// omitted while more than one barrel is declared (see
/// [`crate::weapons::pattern::validate_barrel_pattern`]).
///
/// The barrel count is the authored `barrels.len()`, or `1` for the implicit
/// single-barrel (backward-compat) tube.
pub fn validate_torpedo_tubes(tubes: &[TorpedoTubeConfig]) -> Result<(), String> {
    if tubes.is_empty() {
        return Err("torpedo tubes list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for t in tubes {
        if !seen.insert(t.id.as_str()) {
            return Err(format!("duplicate torpedo tube id '{}'", t.id));
        }
        if !(t.fire_arc_deg > 0.0 && t.fire_arc_deg <= 360.0) {
            return Err(format!(
                "torpedo tube '{}' has fire_arc_deg={} outside (0, 360]",
                t.id, t.fire_arc_deg
            ));
        }
        let barrel_count = if t.barrels.is_empty() {
            1
        } else {
            t.barrels.len()
        };
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("torpedo tube '{}'", t.id),
            barrel_count,
            &t.pattern,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponsConsoleConfig {
    /// RGBA colour used by the client Tactical UI for torpedo fire-arc
    /// overlays. When absent, the `ShipClientConfig` default is used.
    #[serde(default)]
    pub torpedo_arc_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Per-bank phaser definitions parsed from
    /// `[[weapons_console.phaser_banks]]`. Each bank has its own facing,
    /// fire arc, auto-fire arc, range, damage, duration, cooldown, and colour.
    #[serde(default)]
    pub phaser_banks: Vec<PhaserBankConfig>,
    /// Per-bank blaster definitions parsed from
    /// `[[weapons_console.blaster_banks]]` (issue #631). Each bank has its own
    /// facing, fire arc, volley count, damage, shield pierce, and cooldown.
    #[serde(default)]
    pub blaster_banks: Vec<BlasterBankConfig>,
    /// Radar configuration for the Tactical console radar widget, from
    /// `[weapons_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
    /// Inline per-system target selector (issue #777). Loaded from
    /// `[weapons_console.selector]`; absent ⇒ the canonical
    /// [`default_tactical_target_selector_config`] is synthesised at spawn.
    /// Mirrors [`SensorsConsoleConfig::selector`] — the Tactical host ranks its
    /// own candidates independently and remains the sole writer of the
    /// authoritative `TacticalRadarSelection`.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Explicit Tactical-radar idle declaration (issue #781, AC6). When `true`
    /// the radar takes NO AI target selection — `ai_target_selection` clears any
    /// stale lock and skips the ship — even when a tactical fine system is
    /// AI-operated. This is the explicit AI-or-idle opt-out that distinguishes
    /// "the radar deliberately makes no AI selection" from "no selector authored
    /// → default selector". Defaults to `false` (radar runs its selector as
    /// before), so baseline behaviour is preserved.
    #[serde(default)]
    pub selector_idle: bool,
    /// Inline stateless AI policy for the ship's WEAPONS DOCTRINE fine system
    /// (`[weapons_console.ai]`, issue #956).
    ///
    /// The ship-level counterpart of the per-bank/per-tube `ai` blocks below:
    /// those say *when this emitter opens fire*, and this one says *which family
    /// the ship turns to bring to bear* when the target is in range but outside
    /// every arc of a family. It drives the channel-3 `ArcBearingRequest` Weapons
    /// sends Helm, over the `arc_bearing_first` / `arc_bearing_second` /
    /// `arc_bearing_third` channels — the rank ladder that replaced the Rust
    /// `[Phasers, Blasters, Torpedoes]` array in `tick_weapons_arc_request`.
    ///
    /// Authored in `fragments/ai/fleet_baseline.toml` for every hull that
    /// composes the ship-level spine, so a hull with no preference of its own
    /// resolves the FLEET BASELINE rather than an inline Rust order.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

/// Player-ship phaser combat tuning. All per-bank values (`beam_range`,
/// `beam_damage_per_sec`, `beam_duration_secs`, `cooldown_secs`,
/// `beam_color`, `shield_pierce`) live on each [`PhaserBankConfig`] entry.
///
/// `PhaserCombatConfig` is the player-path source of truth, installed
/// as a Bevy resource by `WeaponsPlugin` and overridden in
/// `spawn_game_start_entities` from the player ship's `[weapons_console]`
/// block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhaserCombatConfig {
    /// Per-bank facing/arc/range/damage/duration/cooldown/colour list,
    /// parsed from `[[weapons_console.phaser_banks]]` in TOML order. Empty
    /// if the ship has no banks configured. The Tactical UI also receives a
    /// stripped subset of these via `PhaserBankClientConfig`.
    pub banks: Vec<PhaserBankConfig>,
}

impl PhaserCombatConfig {
    /// Canonical baseline phaser values used when a bank's field is `0.0`
    /// (the "zero means absent" convention). Other modules needing the
    /// baseline alias these constants rather than restating the numbers.
    pub const DEFAULT_PHASER_RANGE: f32 = 40.0;
    pub const DEFAULT_BEAM_DURATION_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_COOLDOWN_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_DAMAGE_PER_SEC: f32 = 5.0;
}

impl PhaserCombatConfig {
    /// Build a `PhaserCombatConfig` from a parsed `[weapons_console]` block.
    /// All combat tuning is now per-bank; this method just clones the banks list.
    pub fn from_weapons_console(wc: &WeaponsConsoleConfig) -> Self {
        Self {
            banks: wc.phaser_banks.clone(),
        }
    }

    /// Look up a bank by its id. Returns `None` if not found.
    pub fn bank_by_id(&self, id: &str) -> Option<&PhaserBankConfig> {
        self.banks.iter().find(|b| b.id == id)
    }
}

/// Config block for the torpedo system in a ship TOML.
///
/// Loaded from `[torpedoes]` in the ship entity TOML (and any NPC ship TOML
/// that wishes to override the torpedo loadout). All fields are optional;
/// missing fields fall back to the same defaults as `TorpedoConfig::default()`.
///
/// `turn_rate_deg_per_sec` is in **degrees per second** for designer
/// readability; it is converted to radians by `to_runtime()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoesConfig {
    #[serde(default = "default_torpedo_count")]
    pub count: u32,
    /// What a round delivers when the facing arc it strikes is DOWN: the whole
    /// figure lands on the hull, unabsorbed and unpierced (issue #929).
    #[serde(default = "default_torpedo_damage_hull")]
    pub damage_hull: i32,
    /// What a round delivers when the facing arc it strikes is UP: offered to
    /// that arc through the same seam beam damage takes (issue #929). Authored
    /// well below `damage_hull` across the fleet — that gap is what the tubes'
    /// `fact(target_facing_shields) <= 0` launch gate is buying.
    #[serde(default = "default_torpedo_damage_shields")]
    pub damage_shields: i32,
    #[serde(default = "default_torpedo_speed")]
    pub speed: f32,
    /// Maximum turn rate in **degrees per second** (homing).
    /// Converted to radians by `to_runtime()`.
    #[serde(default = "default_torpedo_turn_rate_deg_per_sec")]
    pub turn_rate_deg_per_sec: f32,
    #[serde(default = "default_torpedo_lifespan")]
    pub lifespan: f32,
    #[serde(default = "default_torpedo_load_time")]
    pub load_time: f32,
    /// Proximity-detonation radius in world units.
    #[serde(default = "default_torpedo_detonation_radius")]
    pub detonation_radius: f32,
    /// Fraction of `damage_shields` that bypasses shields and adds to
    /// hull damage at detonation. Default `0.0` — `damage_shields` is
    /// fully absorbed by the facing shield quadrant. Clamped to `[0.0, 1.0]`
    /// at apply time.
    ///
    /// A lever on the shields-UP payload only: the shields-down payload
    /// (`damage_hull`) meets no screen and so has nothing to pierce
    /// (issue #929).
    ///
    /// CURRENTLY INERT ACROSS THE SHIPPED FLEET. No `[torpedoes]` block in
    /// `assets/entities/` authors it, so every hull runs at the `0.0` default and
    /// the whole shields-up payload goes to the arc. The `shield_pierce` values
    /// that DO appear in those files are on beam and blaster banks, which are a
    /// different field on a different struct. Note the scale before reaching for
    /// it: it splits `damage_shields`, so full pierce on an Alliance round is 4
    /// points where the shields-down branch is 40. Pinned by
    /// `console::weapons::server_tests::torpedo_shield_pierce_splits_the_shields_up_payload_only`,
    /// which exists precisely because an inert field is one nobody would notice
    /// breaking.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Per-tube torpedo definitions parsed from `[[torpedoes.tubes]]`.
    /// Each tube has its own facing and fire arc. Ammo is shared via
    /// the top-level `count` field. Empty when the ship has no explicit
    /// per-tube loadout.
    #[serde(default)]
    pub tubes: Vec<TorpedoTubeConfig>,
    /// Interval in seconds between successive torpedo launches in a burst
    /// volley. Applies to all tubes on the ship. Default `0.3s`.
    #[serde(default = "default_burst_interval_secs")]
    pub burst_interval_secs: f32,
    /// Ship-wide default for `[[torpedoes.tubes]] ai_target_count` — how many
    /// rounds an AI-operated crew keeps loaded in each tube. A per-tube
    /// `ai_target_count` overrides it; when both are absent each tube falls
    /// back to its own `volley_max`.
    #[serde(default)]
    pub ai_volley_target: Option<u32>,
    /// Inline stateless AI policy for the shared magazine's grant decision
    /// (issue #782, AC1). When authored it is validated at content load and
    /// resolved inside `handle_torpedo_magazine_inter_system` right before the
    /// authoritative `claim_magazine_round`; when absent the canonical
    /// [`default_torpedo_magazine_ai_config`] (unconditional grant) is
    /// synthesised at spawn so baseline claim behaviour is preserved. The offline
    /// gate stays the hard authority; this is a data-authored arbiter on top.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

fn default_burst_interval_secs() -> f32 {
    0.3
}

fn default_torpedo_count() -> u32 {
    10
}
fn default_torpedo_damage_hull() -> i32 {
    50
}
fn default_torpedo_damage_shields() -> i32 {
    5
}
fn default_torpedo_speed() -> f32 {
    15.0
}
fn default_torpedo_turn_rate_deg_per_sec() -> f32 {
    45.0
}
fn default_torpedo_lifespan() -> f32 {
    20.0
}
fn default_torpedo_load_time() -> f32 {
    10.0
}
fn default_torpedo_detonation_radius() -> f32 {
    5.0
}

impl Default for TorpedoesConfig {
    fn default() -> Self {
        Self {
            count: default_torpedo_count(),
            damage_hull: default_torpedo_damage_hull(),
            damage_shields: default_torpedo_damage_shields(),
            speed: default_torpedo_speed(),
            turn_rate_deg_per_sec: default_torpedo_turn_rate_deg_per_sec(),
            lifespan: default_torpedo_lifespan(),
            load_time: default_torpedo_load_time(),
            detonation_radius: default_torpedo_detonation_radius(),
            shield_pierce: 0.0,
            tubes: Vec::new(),
            burst_interval_secs: default_burst_interval_secs(),
            ai_volley_target: None,
            ai: None,
        }
    }
}

impl TorpedoesConfig {
    /// Convert this TOML config into a runtime `TorpedoConfig`.
    /// Performs the degrees → radians conversion on `turn_rate_deg_per_sec`.
    pub fn to_runtime(&self) -> crate::weapons::torpedo::TorpedoConfig {
        crate::weapons::torpedo::TorpedoConfig {
            count: self.count,
            damage_hull: self.damage_hull,
            damage_shields: self.damage_shields,
            speed: self.speed,
            turn_rate: self.turn_rate_deg_per_sec.to_radians(),
            lifespan: self.lifespan,
            load_time: self.load_time,
            detonation_radius: self.detonation_radius,
            shield_pierce: self.shield_pierce,
            burst_interval_secs: self.burst_interval_secs,
            ai_volley_target: self.ai_volley_target,
        }
    }
}
