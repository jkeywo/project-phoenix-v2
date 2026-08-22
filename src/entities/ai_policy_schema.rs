//! The fine-system AI policy schema — the authored TOML language every
//! fine-system AI host (Captain, Helm, weapon banks, Shields, Comms, Nav,
//! Sensors, Power) is declared in, and its load-time validation.
//!
//! Surface: the channel/verb/selector-source name tables that are the language's
//! vocabulary; the `FineSystemAi*Toml` types (selector, rule, state, transition,
//! config) plus `ScoreTermToml`; and the `validate_fine_system_ai_*` entry points
//! that lower authored TOML into `crate::ai::policy` runtime types.
//!
//! Extracted verbatim from `entities::config` (#1196) and re-exported from it, so
//! every `entities::config::<name>` path is unchanged. Pure schema: no Bevy
//! systems, no per-tick state. Invariants — parsing, defaults, and validation
//! errors are exactly what `entities::config` produced; each verb string decodes
//! to one `AiPolicyVerb`; history windows resolve to positive whole ticks; rule
//! and transition priorities are unambiguous (#774).

use serde::{Deserialize, Serialize};

/// The `red_alert` output channel: the one channel the Captain policy drives.
pub const CAPTAIN_RED_ALERT_CHANNEL: &str = "red_alert";
/// The `set_red_alert` verb: the one typed verb the Captain policy emits.
pub const CAPTAIN_SET_RED_ALERT_VERB: &str = "set_red_alert";

// ── Captain first-contact facts (issue #912) ─────────────────────────────────
//
// Before #912 the Captain host seeded exactly ONE fact, `secs_since_combat`,
// and that timer starts only on damage taken, hostile fire taken, or own weapon
// fired. Since #872 a backfilled Alliance hull's own weapons hold fire until Red
// Alert, so the loop closed: such a hull could only ever RETURN fire, and the
// authoring surface could not express first contact at all. These two readings
// are what an authored guard needs to open an engagement, and they are seeded by
// `operate_captain_ai` — no Rust branch decides the alert.
//
// BOTH are seeded UNCONDITIONALLY, every evaluation. An absent fact makes every
// comparison against it read false, which is indistinguishable from "clear" and
// hides a guard that was never wired up (the #779 shape).

/// `1.0` when this ship has a faction-hostile contact in the shared
/// `WorldSnapshot`, `0.0` when it has none. Always seeded.
pub const CAPTAIN_HOSTILE_CONTACT_FACT: &str = "hostile_contact";
/// Planar distance to that nearest hostile contact, world units. Always seeded,
/// and reads `0.0` when there is no contact at all — which is precisely why an
/// authored guard must pair it with [`CAPTAIN_HOSTILE_CONTACT_FACT`] rather than
/// comparing it against a threshold on its own.
pub const CAPTAIN_HOSTILE_RANGE_FACT: &str = "hostile_range";

// ── Helm fine-system AI policy channels/verbs (issue #779) ────────────────────
//
// Engines and Steering are the first *continuous* fine actuators to move onto
// the data-authored #775 policy spine. Each drives a single output channel with
// a single value-less **mode** verb: the verb decides *whether* to actuate this
// tick, while the continuous thrust/yaw magnitude stays sourced from the shared
// `DesiredMotion` planner fact (AGENTS.md rule #11 — no geometry pinned here).

/// The `longitudinal` output channel: the Engines fine system's thrust axis.
pub const HELM_LONGITUDINAL_CHANNEL: &str = "longitudinal";
/// The `actuate_desired_travel` verb: the Engines mode verb. Its presence tells
/// the host to emit `SetThrust` with the scalar decoded from the planner's
/// `desired_velocity_local`; its absence ("hold") emits nothing.
pub const HELM_ACTUATE_DESIRED_TRAVEL_VERB: &str = "actuate_desired_travel";

/// The `yaw` output channel: the Steering fine system's turn axis.
pub const HELM_YAW_CHANNEL: &str = "yaw";
/// The `actuate_desired_facing` verb: the Steering mode verb. Its presence tells
/// the host to emit `SetSteering` with the scalar decoded from the planner's
/// `desired_facing_local`; its absence ("hold") emits nothing.
pub const HELM_ACTUATE_DESIRED_FACING_VERB: &str = "actuate_desired_facing";
/// The `hold_committed_heading` verb: the Steering fine system's SECOND mode
/// verb (issue #883). Its presence tells the host to fly the heading frozen in
/// this system's own `memory(escape_heading_rad)` at the last committed
/// transition, rather than re-solving the facing against a moving target.
/// Distinct from a "hold" (no rule fires), which holds the last steering
/// COMMAND and would keep a non-zero yaw turning for ever.
pub const HELM_HOLD_COMMITTED_HEADING_VERB: &str = "hold_committed_heading";
/// The `hold_recovery_orbit` verb: the Steering fine system's THIRD mode verb
/// (issue #788). Its presence tells the host to fly a tangent of the safe ring
/// around the current target — radius derived from that target's own direct-fire
/// reach plus this hull's authored `safe_range_margin`, circulation direction
/// taken from this system's host-written `memory(orbit_direction)`.
pub const HELM_HOLD_RECOVERY_ORBIT_VERB: &str = "hold_recovery_orbit";
/// The `pivot_to_reengage` verb: the Steering fine system's FOURTH mode verb
/// (issue #788). Tracks the target like `actuate_desired_facing`, but the host
/// pairs it with the authored `reengage_speed` throttle rather than the approach
/// throttle — the cut-thrust pivot that ends a recovery and starts the next run.
pub const HELM_PIVOT_TO_REENGAGE_VERB: &str = "pivot_to_reengage";
/// The `hold_combat_orbit` verb: the Steering fine system's FIFTH mode verb
/// (issue #790). Its presence tells the host to fly a tangent of a ring around
/// the current target whose radius is the hull's own authored
/// `combat_orbit_range` — a fighting range, not a standoff derived from the
/// target's reach — with the circulation direction taken from this system's
/// host-written `memory(orbit_direction)`.
pub const HELM_HOLD_COMBAT_ORBIT_VERB: &str = "hold_combat_orbit";
/// The `hold_torpedo_bearing` verb: the Steering fine system's SIXTH mode verb
/// (issue #791). Tracks the target's live position like `actuate_desired_facing`,
/// but the host pairs it with the authored `torpedo_bearing_speed` throttle
/// rather than with doctrine travel — the bow-on, thrust-cut hold a hull flies
/// while a fixed forward tube lines up on a shield facing that has gone down.
///
/// Deliberately NOT a reuse of `pivot_to_reengage`, whose geometry is the same
/// but whose host gate is the six-scalar shield-RECOVERY parameter set: a hull
/// with no standoff doctrine would have to invent all six to borrow it.
pub const HELM_HOLD_TORPEDO_BEARING_VERB: &str = "hold_torpedo_bearing";
/// The `hold_artillery_position` verb: the Steering fine system's SEVENTH mode
/// verb (issue #792). Tells the host to hold translational station on the
/// authored `artillery_hold_speed` while pivoting the bow onto a PREDICTIVE
/// intercept solution — where the target will be when this hull's own artillery
/// bolt arrives, not where it is now.
///
/// Deliberately NOT a reuse of `pivot_to_reengage` (whose host gate is the six
/// shield-RECOVERY scalars, all describing a standoff ring derived from the
/// TARGET's reach) nor of `hold_torpedo_bearing` (which tracks the target's live
/// position with no lead at all — the right answer for a fixed tube at knife
/// range and the wrong one for a slow bolt with seconds of flight time).
pub const HELM_HOLD_ARTILLERY_POSITION_VERB: &str = "hold_artillery_position";

/// The verbs a Steering (`yaw`) policy may emit
/// (issues #779, #883, #788, #790, #791, #792).
pub const HELM_STEERING_VERBS: &[&str] = &[
    HELM_ACTUATE_DESIRED_FACING_VERB,
    HELM_HOLD_COMMITTED_HEADING_VERB,
    HELM_HOLD_RECOVERY_ORBIT_VERB,
    HELM_PIVOT_TO_REENGAGE_VERB,
    HELM_HOLD_COMBAT_ORBIT_VERB,
    HELM_HOLD_TORPEDO_BEARING_VERB,
    HELM_HOLD_ARTILLERY_POSITION_VERB,
];

// ── Helm secondary fine-actuator AI policy channels/verbs (issue #780) ────────
//
// Lateral thrust, bounded vertical thrust, impulse, and boost move onto the same
// #775 policy spine. Each drives a single output channel with a single value-less
// MODE verb: the verb decides *whether* to actuate this tick, while the
// continuous magnitude / engage-vs-cancel decision stays sourced host-side from
// geometry, capability, and the shared hazard assessment (AGENTS.md rule #11 —
// no gameplay scalar pinned in the verb).

/// The `lateral` output channel: the Lateral Thrust fine system's dodge axis.
pub const HELM_LATERAL_CHANNEL: &str = "lateral";
/// The `actuate_lateral_thrust` verb: the Lateral Thrust mode verb. Its presence
/// lets the host emit `LateralThrustInput` with the magnitude from the shared
/// hazard surface (or docking translation); its absence ("hold") emits nothing.
pub const HELM_ACTUATE_LATERAL_THRUST_VERB: &str = "actuate_lateral_thrust";

/// The `vertical` output channel: the Vertical Thrust fine system's climb axis.
pub const HELM_VERTICAL_CHANNEL: &str = "vertical";
/// The `actuate_vertical_thrust` verb: the Vertical Thrust mode verb. Its
/// presence lets the host emit `VerticalThrustInput` with the climb/return
/// magnitude gated on the authored `VerticalMovementMode`; its absence emits
/// nothing.
pub const HELM_ACTUATE_VERTICAL_THRUST_VERB: &str = "actuate_vertical_thrust";

/// The `impulse` output channel: the Impulse fine system's engage/cancel axis.
pub const HELM_IMPULSE_CHANNEL: &str = "impulse";
/// The `engage_impulse` verb: the Impulse mode verb. Its presence permits the
/// impulse manoeuvre this tick; the host still applies the authored doctrine
/// `use_impulse` and the `decide_impulse` geometry. Its absence ("hold") emits
/// nothing.
pub const HELM_ENGAGE_IMPULSE_VERB: &str = "engage_impulse";

/// The `boost` output channel: the Boost fine system's engage axis.
pub const HELM_BOOST_CHANNEL: &str = "boost";
/// The `engage_boost` verb: the Boost mode verb. Its presence drives the ship's
/// boost active via the same admitted `SetBoost` a human uses; its absence
/// ("hold"/idle) leaves boost as it is.
pub const HELM_ENGAGE_BOOST_VERB: &str = "engage_boost";

// ── Weapon-bank fine-system AI policy channels/verbs (issue #781) ─────────────
//
// Each AI-capable phaser and blaster bank drives a single output channel with a
// single value-less ACTION verb: the verb decides *whether* to open fire this
// tick, while the target (the ship's authoritative combat lock), the firing
// bank, and the beam frequency all come from the host context — never the verb
// (AGENTS.md rule #11 — no fire thresholds/ranges/arcs/cooldowns pinned here;
// those stay TOML on the bank configs). Each bank enforces availability,
// cooldown, range, arc, and target validity host-side before resolving its
// policy, so the runtime only reports *whether the authored behaviour permits*
// firing an already-ready bank.

/// The `phaser_fire` output channel: a phaser bank's open-fire axis.
pub const PHASER_FIRE_CHANNEL: &str = "phaser_fire";
/// The `fire_phaser` verb: the phaser-bank fire verb. Its presence tells the
/// host to emit the same admitted `FirePhaser` a human does; its absence
/// ("hold"/idle) holds this bank's fire.
pub const PHASER_FIRE_VERB: &str = "fire_phaser";

/// The registered output channels a phaser bank policy may drive (issue #781).
pub const PHASER_BANK_CHANNELS: &[&str] = &[PHASER_FIRE_CHANNEL];
/// The registered verbs a phaser bank policy may emit (issue #781).
pub const PHASER_BANK_VERBS: &[&str] = &[PHASER_FIRE_VERB];

/// The `blaster_fire` output channel: a blaster bank's open-fire axis.
pub const BLASTER_FIRE_CHANNEL: &str = "blaster_fire";
/// The `fire_blaster` verb: the blaster-bank fire verb. Its presence tells the
/// host to emit the same admitted `ChargeBlasterStart` a human does; its absence
/// ("hold"/idle) holds this bank's volley.
pub const BLASTER_FIRE_VERB: &str = "fire_blaster";

/// The registered output channels a blaster bank policy may drive (issue #781).
pub const BLASTER_BANK_CHANNELS: &[&str] = &[BLASTER_FIRE_CHANNEL];
/// The registered verbs a blaster bank policy may emit (issue #781).
pub const BLASTER_BANK_VERBS: &[&str] = &[BLASTER_FIRE_VERB];

// ── Torpedo tube + magazine fine-system AI policy channels/verbs (issue #782) ─
//
// A torpedo tube is a two-stage pipeline owned by two fine systems: the TUBE
// decides whether to LOAD (reserve a round from the shared magazine) and whether
// to LAUNCH (fire an already-loaded round), while the shared MAGAZINE arbitrates
// whether to GRANT a pending reservation. Every verb is value-less: the tube, its
// authored volley target, the ship's authoritative combat lock, and all
// thresholds stay TOML/host-side, never in the verb (AGENTS.md rule #11). The
// host enforces loaded state, magazine availability, target validity, range, and
// arc before resolving these policies, so the runtime only reports *whether the
// authored behaviour permits* the load/launch/grant of an already-ready stage.

/// The `torpedo_load` output channel: a tube's load-a-round axis.
pub const TORPEDO_LOAD_CHANNEL: &str = "torpedo_load";
/// The `load_torpedo` verb. Its presence tells the host to emit the same admitted
/// `SetTorpedoVolleyTarget` a Tactical player does; its absence ("hold"/idle)
/// leaves the tube's volley target where it is.
pub const TORPEDO_LOAD_VERB: &str = "load_torpedo";

/// The `torpedo_launch` output channel: a tube's launch-a-loaded-round axis.
pub const TORPEDO_LAUNCH_CHANNEL: &str = "torpedo_launch";
/// The `launch_torpedo` verb. Its presence tells the host to emit the same
/// admitted `FireTorpedo` a human does; its absence ("hold"/idle) holds fire.
pub const TORPEDO_LAUNCH_VERB: &str = "launch_torpedo";

/// The registered output channels a torpedo tube policy may drive (issue #782).
pub const TORPEDO_TUBE_CHANNELS: &[&str] = &[TORPEDO_LOAD_CHANNEL, TORPEDO_LAUNCH_CHANNEL];
/// The registered verbs a torpedo tube policy may emit (issue #782).
pub const TORPEDO_TUBE_VERBS: &[&str] = &[TORPEDO_LOAD_VERB, TORPEDO_LAUNCH_VERB];

/// The `torpedo_magazine_grant` output channel: the shared magazine's
/// grant-a-claim axis, resolved inside the single magazine consumer.
pub const TORPEDO_MAGAZINE_CHANNEL: &str = "torpedo_magazine_grant";
/// The `grant_torpedo_round` verb. Its presence permits a pending
/// `ClaimTorpedoRound` reservation to proceed; its absence ("hold"/idle) refuses
/// the claim without touching the magazine counter.
pub const TORPEDO_MAGAZINE_GRANT_VERB: &str = "grant_torpedo_round";

// ── Torpedo conservation: magazine vs remaining mission (issue #943) ─────────
//
// The second channel the shared magazine drives, and the one place a torpedo
// launch is gated SYMMETRICALLY: it is resolved inside `handle_fire_torpedo`,
// the single consumer of `SystemControlPayload::FireTorpedo` for every ship,
// which admission reaches with the source identity already stripped. A human
// Tactical operator's launch and an AI backfill's launch pass through the same
// resolve, and nothing below admission may ask which one it was (AGENTS.md #6).
//
// The question it answers is not "may this weapon fire" — the tube's own
// `torpedo_launch` doctrine already answers that for the AI, and red alert
// answers it for the ship — but "can this ship AFFORD to spend a round here,
// given how much of the mission is still ahead". That measure is WORLD-scoped:
// the scenario publishes [`MISSION_THREAT_REMAINING_COUNTER`] and the host reads
// it off the ship's own layered flag chain, so the same hull paces differently
// in an eight-wave defence and in a single-target strike, with no per-hull
// constant anywhere.

/// The `torpedo_conservation` output channel: the shared magazine's
/// spend-a-round-here axis, resolved once per ship per tick, ahead of that
/// ship's admitted command loop.
pub const TORPEDO_CONSERVATION_CHANNEL: &str = "torpedo_conservation";
/// The `release_torpedo` verb. Its presence permits an already-authorised
/// launch to spend its round; its absence ("hold"/idle) holds the round for
/// later in the mission WITHOUT touching the magazine or the tube — the round
/// stays loaded and the same decision is offered again next tick.
///
/// A magazine policy that authors NO rule on
/// [`TORPEDO_CONSERVATION_CHANNEL`] is unconstrained: conservation is content,
/// so a hull (or a whole fleet) that never authors it fires exactly as it did
/// before this channel existed.
pub const TORPEDO_RELEASE_VERB: &str = "release_torpedo";

/// The WORLD counter a scenario publishes to say how much of the mission's
/// threat is still ahead of the ships flying it (issue #943).
///
/// Engine vocabulary, not a gameplay value: the NAME is fixed so a hull's
/// doctrine can be written once and paced by any scenario, while the NUMBER —
/// how much threat a mission poses, and when each unit of it is cleared — is
/// authored entirely in world TOML through the ordinary `set_flag_value` /
/// `increment_flag` trigger actions. `assets/worlds/combat_test.toml` sets it to
/// its eight-wave schedule and decrements it as each wave dies.
///
/// A world that publishes nothing leaves it at the unset default of `0`, which
/// the host reads as "no mission pressure" — see
/// [`TORPEDO_ROUNDS_PER_THREAT_FACT`] — so every existing scenario keeps
/// firing freely.
pub const MISSION_THREAT_REMAINING_COUNTER: &str = "mission_threat_remaining";

/// Host-seeded fact name: every round this ship still HAS —
/// `TorpedoSystem::rounds_aboard`, i.e. the magazine plus the rounds already
/// moved out of it into the tubes.
///
/// Deliberately NOT `torpedoes_remaining`. That counter is debited when a load
/// *starts*, so a hull whose tube doctrine keeps its tubes topped up reads
/// permanently short by its parked volley — three of the destroyer's twelve —
/// and a reserve measured against it would strand exactly those rounds: the
/// counter can reach 0 with a full salvo still sitting in the tubes, and every
/// further launch is refused for the rest of the mission. Conservation is about
/// what the ship can still put in the water, which is this.
pub const TORPEDO_ROUNDS_ABOARD_FACT: &str = "rounds_aboard";
/// Host-seeded fact name: [`MISSION_THREAT_REMAINING_COUNTER`] as this ship's
/// own layer chain reads it, so a ship spawned into a sub-world paces against
/// that layer's mission rather than the base world's.
pub const TORPEDO_MISSION_THREAT_FACT: &str = "mission_threat_remaining";
/// Host-seeded fact name: [`TORPEDO_ROUNDS_ABOARD_FACT`] PER remaining unit of
/// mission threat — the derived ratio a conservation guard compares against an
/// authored reserve, because the predicate grammar compares one atom to one
/// operand and has no arithmetic of its own.
///
/// With no remaining threat published (an unpaced world, or a mission whose
/// threat is spent) the ratio is `f64::INFINITY`: unbounded rounds per remaining
/// threat is the honest answer to "how many can I spend on each of the zero
/// things left", and it makes `>= param(...)` fire, so the unpaced case is the
/// permissive one.
pub const TORPEDO_ROUNDS_PER_THREAT_FACT: &str = "rounds_per_threat";
/// Host-seeded fact name: how many of this ship's own `[behaviour].doctrine`
/// entries are a Destroy directive that NAMES its target
/// (`directive_target`) — the "homing in on one target" reading of the issue's
/// carve-out.
///
/// Counting doctrine entries outright cannot express that carve-out, because a
/// world's `spawn_entity` override APPENDS to the template's doctrine rather
/// than replacing it (`behaviour.doctrine` reconciles by `id`, and an
/// `InstanceOverride` may not tombstone what the template authored). So
/// combat_test's raid cruiser — the shipped case the issue describes — carries
/// its template's untargeted `destroy-hostiles` standing order alongside the
/// `assault-starbase` brief the world gives it, and reads as two objectives
/// however sole its actual brief is. What is singular about it is the NAMED
/// target: a hull ordered to kill one specific thing has one engagement, and
/// the untargeted standing order underneath is what it does with whatever is in
/// front of it, not a second engagement to hoard rounds for.
///
/// A hull with no named target at all reads 0 and is NOT carved out — the
/// player destroyer's brief (`destroy-hostiles` + `hold-station`) is open-ended,
/// which is precisely the ship #943 was filed about.
pub const TORPEDO_TARGETED_OBJECTIVE_COUNT_FACT: &str = "targeted_objective_count";

/// The registered output channels a torpedo magazine policy may drive (#782,
/// widened by #943).
pub const TORPEDO_MAGAZINE_CHANNELS: &[&str] =
    &[TORPEDO_MAGAZINE_CHANNEL, TORPEDO_CONSERVATION_CHANNEL];
/// The registered verbs a torpedo magazine policy may emit (issues #782, #943).
pub const TORPEDO_MAGAZINE_VERBS: &[&str] = &[TORPEDO_MAGAZINE_GRANT_VERB, TORPEDO_RELEASE_VERB];

// ── Shields focus fine-system AI policy channel/verb (issue #783) ─────────────
//
// The Shields fine system focuses ONE of the ship's four arcs at a time. The
// #783 conversion keeps the retained arc-ranking kernel (`tick_shield_focus_ai`:
// damage-concentration primary, health-imbalance fallback) as the 4-way argmax
// and lifts only the AUTHORED windows/thresholds and the gate (whether to act)
// into an inline stateless policy. The `focus_shield_arc` verb is value-less —
// which arc wins is the kernel's call from the host context, never the verb
// (AGENTS.md rule #11: the concentration %, windows, and health ratio are policy
// `param`s, not literals). This is the channel/verb model of #775/#779–#782, not
// the #776 selector: shield arcs are a fixed 4-set of in-ship indices, not UUID
// entities, so there is nothing for a candidate-source selector to union.

/// The `shield_focus` output channel: the Shields fine system's focus-an-arc axis.
pub const SHIELD_FOCUS_CHANNEL: &str = "shield_focus";
/// The `focus_shield_arc` verb. Its presence tells the host to run the retained
/// arc-ranking kernel and emit the same admitted `SetShieldArcFocus` a human
/// Shields operator does; its absence ("hold"/idle) leaves the focus where it is.
pub const SHIELD_FOCUS_VERB: &str = "focus_shield_arc";

/// The registered output channels a Shields focus policy may drive (issue #783).
pub const SHIELD_FOCUS_CHANNELS: &[&str] = &[SHIELD_FOCUS_CHANNEL];
/// The registered verbs a Shields focus policy may emit (issue #783).
pub const SHIELD_FOCUS_VERBS: &[&str] = &[SHIELD_FOCUS_VERB];

/// Authored policy-parameter name: the maximum recent-damage window (seconds)
/// the kernel measures concentration over. Read host-side from the Shields focus
/// policy `param` map (issue #783); the kernel's arg equivalent of the retained
/// typed `ShieldsAiConfigToml::damage_window_secs` knob.
pub const SHIELD_FOCUS_DAMAGE_WINDOW_PARAM: &str = "damage_window_secs";
/// Authored policy-parameter name: the minimum window (seconds) floor.
pub const SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM: &str = "min_damage_window_secs";
/// Authored policy-parameter name: the concentration threshold (0–100).
pub const SHIELD_FOCUS_DAMAGE_PCT_PARAM: &str = "damage_pct_threshold";
/// Authored policy-parameter name: the health-imbalance fallback ratio (0–100).
pub const SHIELD_FOCUS_HEALTH_RATIO_PARAM: &str = "health_ratio_threshold";

// ── Power group allocation fine-system AI policy verb (issue #784) ────────────
//
// The Power reactor fine system allocates the ship's battery budget across the
// ship's AUTHORED power groups. The #784 conversion moves Power onto the same
// inline stateless `FineSystemAiConfigToml` spine as #779–#783, with two
// novelties: (1) the output CHANNELS are the ship's `[power_groups.*]` keys —
// dynamic per-ship data, not a fixed const slice — so the valid-channel set is
// built at load from ship data (AC1 "no fixed catalogue"); (2) the
// `set_power_group_allocation` verb is the FIRST verb to carry a MAGNITUDE — an
// absolute target level — in its payload (every prior verb was value-less or the
// boolean `set_red_alert`). The applier re-clamps to the per-group `[1, max]`
// range and the ship-wide `total <= 8` cap, so an absolute level is safe and
// idempotent; the host skips the emit when `level == current`.

/// The `set_power_group_allocation` verb: set the rule's power group to an
/// absolute target level. Its magnitude is the authored per-rule `level`
/// payload — never an inline Rust number (AGENTS.md rule #11).
pub const POWER_SET_ALLOCATION_VERB: &str = "set_power_group_allocation";

/// Authored policy-parameter name: the battery reserve (0–100) BELOW which the
/// default helm channel gives its elevated point back (AC2). The shed floor: the
/// hold rule reads it, and the channel falls to its baseline underneath it.
pub const POWER_HELM_RESERVE_PARAM: &str = "min_reserve_helm";
/// Authored policy-parameter name: the battery reserve (0–100) the default helm
/// channel must be back OVER before it may elevate again (issue #1003).
///
/// The upper half of the pair, and always above [`POWER_HELM_RESERVE_PARAM`].
/// One threshold would be both the shed floor and the re-elevate floor, and the
/// channel would then flip on every tick the charge rested on it — the lower
/// total recharges past a single threshold inside one tick. See
/// `fragments/ai/fleet_baseline.toml`.
pub const POWER_HELM_RESTORE_PARAM: &str = "min_restore_helm";
/// Authored policy-parameter name: the battery reserve (0–100) BELOW which the
/// default weapons channel gives its elevated point back (AC2).
pub const POWER_WEAPONS_RESERVE_PARAM: &str = "min_reserve_weapons";
/// Authored policy-parameter name: the battery reserve (0–100) the default
/// weapons channel must be back OVER before it may elevate again (issue #1003).
/// Sibling of [`POWER_HELM_RESTORE_PARAM`]; see there for what the gap buys.
pub const POWER_WEAPONS_RESTORE_PARAM: &str = "min_restore_weapons";
/// Host-seeded fact name: current battery charge as a percentage (0–100). The
/// reserve guard `fact(battery_pct) >= param(min_reserve_*)` reads this; it is
/// the stateless brownout-avoidance predicate (AC5).
pub const POWER_BATTERY_PCT_FACT: &str = "battery_pct";
/// Host-seeded fact name: latest forward thrust (0.0–1.0) from `LastHelmInput`.
pub const POWER_THRUST_FACT: &str = "thrust";
/// Host-seeded fact name: red-alert state as `1.0`/`0.0`.
pub const POWER_RED_ALERT_FACT: &str = "red_alert";

// ── Comms dialogue-response fine-system AI policy channel/verb (issue #786) ───
//
// Comms is the FIRST fine system to author BOTH machines at once, because it
// owns two different decisions:
//
//   * WHO to hail — a variable, per-tick candidate set of real contacts keyed by
//     genuine entity UUID. That is #776 selector vocabulary (see
//     [`COMMS_SELECTOR_SOURCES`] / [`default_comms_target_selector_config`]).
//   * HOW to answer an open dialogue — a fixed, small, INDEX-addressed set
//     (`ActiveDialogue.current_node.responses`, addressed by `usize`). That is
//     the #775 channel/verb model, for the same reason Shields (#783) stayed on
//     it: there is no entity set for a candidate-source selector to union.
//
// The `respond_to_message` verb is the SECOND value-carrying verb (after #784's
// `set_power_group_allocation`): only the response INDEX rides the verb — WHICH
// message is being answered comes from the host context, never the policy.

/// The `comms_respond` output channel: the Comms fine system's
/// answer-an-open-dialogue axis, resolved once per message awaiting a response.
pub const COMMS_RESPOND_CHANNEL: &str = "comms_respond";
/// The `respond_to_message` verb: answer the message being resolved with the
/// authored `response_index` payload. Its presence tells the host to emit the
/// same admitted `RespondToMessage` a human Comms officer sends — through the
/// SAME `handle_respond_to_message` router, so trigger actions and follow-ups
/// fire identically for AI and human (AGENTS.md rule #6). Its absence
/// ("hold"/idle) leaves the dialogue open this tick.
pub const COMMS_RESPOND_VERB: &str = "respond_to_message";

/// The registered output channels a Comms response policy may drive (#786).
pub const COMMS_RESPOND_CHANNELS: &[&str] = &[COMMS_RESPOND_CHANNEL];
/// The registered verbs a Comms response policy may emit (issue #786).
pub const COMMS_RESPOND_VERBS: &[&str] = &[COMMS_RESPOND_VERB];

// ── Weapons doctrine: which family the ship turns to present (issue #956) ─────
//
// `tick_weapons_arc_request` emits at most ONE channel-3 `ArcBearingRequest` per
// ship — "turn, so that this family can bear" — so it has to choose a family
// when several are equally unable to shoot. That choice used to be a Rust array,
// `[Phasers, Blasters, Torpedoes]`, documented as "structural, not a gameplay
// value"; which gun a ship manoeuvres to present is a tactical decision, so it
// is authored now, in `[weapons_console.ai]`.
//
// The channel is the RANK and the verb is the FAMILY. Three channels, one per
// place in the order, each a single ordinary channel decision with its own
// guards — so a doctrine can lead with its tubes while the target's striking arc
// is down and with its beams otherwise, which is the shape the issue's worked
// example asks for. The host resolves them in rank order, drops repeats, and
// walks the resulting list until a family actually qualifies; a rank nobody
// authors simply shortens the order.

/// The `arc_bearing_first` channel: the family this ship presents by preference.
pub const ARC_BEARING_FIRST_CHANNEL: &str = "arc_bearing_first";
/// The `arc_bearing_second` channel: the family it turns for when the first
/// cannot be the reason (no emitters, all offline, already bearing, out of
/// range).
pub const ARC_BEARING_SECOND_CHANNEL: &str = "arc_bearing_second";
/// The `arc_bearing_third` channel: the last family in the order.
pub const ARC_BEARING_THIRD_CHANNEL: &str = "arc_bearing_third";

/// The rank ladder in resolution order. The host reads exactly this slice, so
/// adding a rank is a one-line content-schema change rather than a host one.
pub const ARC_BEARING_CHANNELS: &[&str] = &[
    ARC_BEARING_FIRST_CHANNEL,
    ARC_BEARING_SECOND_CHANNEL,
    ARC_BEARING_THIRD_CHANNEL,
];

/// The `bring_phasers_to_bear` verb: name the phaser banks for this rank.
pub const BRING_PHASERS_TO_BEAR_VERB: &str = "bring_phasers_to_bear";
/// The `bring_blasters_to_bear` verb: name the blaster banks for this rank.
pub const BRING_BLASTERS_TO_BEAR_VERB: &str = "bring_blasters_to_bear";
/// The `bring_torpedoes_to_bear` verb: name the torpedo tubes for this rank.
pub const BRING_TORPEDOES_TO_BEAR_VERB: &str = "bring_torpedoes_to_bear";

/// The registered verbs a weapons-doctrine policy may emit (issue #956).
pub const WEAPONS_DOCTRINE_VERBS: &[&str] = &[
    BRING_PHASERS_TO_BEAR_VERB,
    BRING_BLASTERS_TO_BEAR_VERB,
    BRING_TORPEDOES_TO_BEAR_VERB,
];

/// Host-seeded fact name: HP of the target's shield arc a round from this ship
/// would strike, resolved through the target's own arc router. `<= 0` means the
/// arc is not blocking (down, offline, or absent entirely — an asteroid).
///
/// The reading the fleet's torpedo doctrine is authored against, seeded on the
/// weapons-doctrine snapshot (issue #956) as well as on the tube launch snapshot
/// (`seed_torpedo_tube_launch_facts`), so "lead with the tubes when the screen
/// is down" and "launch when the screen is down" ask the same question of the
/// same number.
pub const TARGET_FACING_SHIELDS_FACT: &str = "target_facing_shields";

// ── Per-system target selector sources (issue #776) ───────────────────────────

/// Candidate source: the ship's frozen combat lock (Tactical's designated
/// firing target), surfaced to the Sensors selector as the highest-priority
/// tier so Sensors mirrors what Tactical is engaging.
pub const SELECTOR_SOURCE_COMBAT_LOCK: &str = "combat-lock";
/// Candidate source: named `Destroy` objective targets resolved from the
/// scored objective pool.
pub const SELECTOR_SOURCE_OBJECTIVE_DESTROY: &str = "objective-destroy";
/// Candidate source: faction-hostile radar contacts inside the ship's horizon.
pub const SELECTOR_SOURCE_RADAR_CONTACTS: &str = "radar-contacts";

/// The registered candidate sources the Sensors target selector may union.
pub const SENSORS_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_COMBAT_LOCK,
    SELECTOR_SOURCE_OBJECTIVE_DESTROY,
    SELECTOR_SOURCE_RADAR_CONTACTS,
];

/// Candidate source: the ship's advisory **Science Target** — the Sensors
/// radar's selected target, surfaced from the frozen viewscreen blackboard
/// (issue #777). Tactical may strongly favour this pick through an authored
/// score bonus, but independently revalidates it before copying (AC2/AC3). It
/// is deliberately NOT the same as `combat-lock`: that is Tactical's OWN output
/// and is excluded to avoid circularity.
pub const SELECTOR_SOURCE_SENSORS_DESIGNATION: &str = "sensors-designation";
/// Candidate source: whoever last attacked this ship (`LastShipAttacker`).
pub const SELECTOR_SOURCE_LAST_ATTACKER: &str = "last-attacker";
/// Candidate source: the target named by an active per-verb operate directive
/// (`Tow`/`Stabilise`/`Escort`/`FieldRepair`), resolved from the scored
/// objective pool (issue #1162). Injected by `ai_target_selection` and tagged
/// `source_operate`; it is the DIRECTIVE-GATED way the AI reaches a non-hostile
/// lock. Inert when no operate directive is active — no candidate is added — so
/// combat auto-lock is unchanged and radar/scan auto-lock stays hostile-gated. A
/// hull opts into it by adding `... or candidate_fact(source_operate) > 0` to
/// its `[weapons_console.selector]` eligibility (fleet_baseline authors it).
pub const SELECTOR_SOURCE_OBJECTIVE_OPERATE: &str = "objective-operate";

/// The registered candidate sources the Tactical target selector may union
/// (issue #777). `combat-lock` is intentionally absent: it is Tactical's own
/// authoritative output, so unioning it would be circular. The ship's current
/// lock is instead surfaced by the host as an internal `source_retained`
/// retention candidate (not a cross-system source), so it too is absent here.
pub const TACTICAL_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_SENSORS_DESIGNATION,
    SELECTOR_SOURCE_OBJECTIVE_DESTROY,
    SELECTOR_SOURCE_LAST_ATTACKER,
    SELECTOR_SOURCE_RADAR_CONTACTS,
    SELECTOR_SOURCE_OBJECTIVE_OPERATE,
];

/// Candidate source: positive, Navigation-relevant (Helm-affinity) objective
/// destinations (issue #778). The Navigation host ranks the scored objective
/// pool, resolves the winner's directive to a destination — a fixed world
/// anchor (Reach / Retreat / Patrol) or a live entity anchor (Destroy) — and
/// surfaces it as the sole `reachable` candidate of this source.
pub const SELECTOR_SOURCE_NAV_OBJECTIVE: &str = "navigation-objectives";
/// Candidate source: live entities the Navigation chart shows, surfaced as
/// authorable entity-anchored destinations (issue #778). They do NOT carry the
/// `reachable` marker under the canonical policy, so by default they enrich a
/// coincident objective destination rather than independently steering the
/// ship; an author may re-tune the selector's eligibility to admit them.
pub const SELECTOR_SOURCE_CHART_CONTACTS: &str = "chart-contacts";

/// The registered candidate sources the Navigation target selector may union
/// (issue #778).
pub const NAVIGATION_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_NAV_OBJECTIVE,
    SELECTOR_SOURCE_CHART_CONTACTS,
];

/// Candidate source: stations the ship's coordination-delivered
/// `RepairRequestQueue` reports as damaged (issue #785). This is the AC1
/// "authoritative observable damage" surface: the Repair AI ranks only stations
/// a `RepairRequest` actually delivered — issue #830 deliberately removed the
/// raw hull poll, so a station nobody reported is not a candidate.
pub const SELECTOR_SOURCE_DAMAGED_STATIONS: &str = "damaged-stations";
/// Candidate source: the ownerless ship-wide `core` repair bucket (issue #785),
/// the second [`crate::core::messages::RepairTarget`] variant. Surfaced as a candidate
/// so an author can weight core repairs into the ranking; under the canonical
/// policy it only becomes eligible once a `RepairRequest` names it, mirroring
/// how `chart-contacts` enriches rather than independently steers Navigation.
pub const SELECTOR_SOURCE_CORE_BUCKET: &str = "core-bucket";

/// The registered candidate sources the Repair target selector may union
/// (issue #785).
pub const REPAIR_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_DAMAGED_STATIONS,
    SELECTOR_SOURCE_CORE_BUCKET,
];

/// Candidate source: positive, Comms-relevant `Hail` directives resolved from
/// the scored objective pool (issue #786). This is the AC1 surface: the Comms
/// AI ranks the hail orders it has actually been given, resolving each
/// directive's authored entity NAME to a real contact UUID before it can become
/// a candidate.
pub const SELECTOR_SOURCE_HAIL_OBJECTIVES: &str = "hail-objectives";
/// Candidate source: the authoritative comms contact list (issue #786) —
/// `CommsRuntime.contacts`, the same hailable roster a human Comms officer sees.
/// Under the canonical policy a contact is NOT independently eligible (the
/// default eligibility keys on `source_hail_objective`): it ENRICHES a
/// coincident hail directive with its live readings, exactly as
/// `chart-contacts` enriches a Navigation destination (#778). An author may
/// widen the eligibility to let the Comms AI hail on its own initiative.
pub const SELECTOR_SOURCE_COMMS_CONTACTS: &str = "comms-contacts";

/// The registered candidate sources the Comms hail selector may union
/// (issue #786).
pub const COMMS_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_HAIL_OBJECTIVES,
    SELECTOR_SOURCE_COMMS_CONTACTS,
];

/// One authored additive utility term (`[[sensors_console.selector.score]]`,
/// issue #776): a guard expression plus the weight it contributes to a
/// candidate's score when it fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreTermToml {
    /// Guard expression over self/candidate/target fact contexts; the term
    /// contributes `weight` when it evaluates `true`.
    pub when: String,
    /// Weight added to the candidate's score when `when` fires. An authored
    /// field (AGENTS.md rule #11 permits authored gameplay values in TOML).
    pub weight: f32,
}

/// Inline per-system target selector for an AI-capable fine system that owns a
/// target (`[sensors_console.selector]`, issue #776).
///
/// Sibling to [`FineSystemAiConfigToml`]: where the #775 policy resolves a
/// verb per output channel, the selector answers "which entity is my target?".
/// It unions authored candidate `sources`, filters inside `horizon`, keeps
/// candidates whose `eligibility` guard fires, sums the additive `score`,
/// retains the current target within `switch_margin`, and returns the winning
/// contact — all as a pure function of the immutable per-tick snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiSelectorToml {
    /// Named numeric parameters referenced by the eligibility/score guards.
    #[serde(default)]
    pub param: std::collections::HashMap<String, f32>,
    /// Registered candidate-source ids this selector unions.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Effective horizon (planar distance) beyond which candidates are dropped.
    pub horizon: f32,
    /// Hysteresis margin: the current target is retained while its score is
    /// within this margin of the best candidate's score.
    pub switch_margin: f32,
    /// Candidate eligibility guard over self/candidate/target fact contexts.
    pub eligibility: String,
    /// Additive utility terms summed per eligible candidate.
    #[serde(default)]
    pub score: Vec<ScoreTermToml>,
}

impl FineSystemAiSelectorToml {
    /// Resolve this authored block into the pure typed
    /// [`crate::ai::selector::TargetSelector`] the runtime consumes.
    ///
    /// Returns a diagnostic `Err` on an unparseable eligibility/score guard;
    /// call [`validate_fine_system_ai_selector`] first at content-load time so
    /// this never fails after world activation.
    pub fn to_selector(&self) -> Result<crate::ai::selector::TargetSelector, String> {
        let mut params = crate::world::flags::AiParams::new();
        for (k, v) in &self.param {
            params.set(k, *v as f64);
        }
        let eligibility = crate::world::flags::parse_predicate(&self.eligibility)?;
        let mut score = Vec::with_capacity(self.score.len());
        for term in &self.score {
            let when = crate::world::flags::parse_predicate(&term.when)?;
            score.push(crate::ai::selector::ScoreTerm {
                when,
                weight: term.weight as f64,
            });
        }
        Ok(crate::ai::selector::TargetSelector {
            params,
            sources: self.sources.clone(),
            horizon: self.horizon,
            switch_margin: self.switch_margin,
            eligibility,
            score,
        })
    }
}

/// Validate an inline per-system target selector before world activation
/// (issue #776), mirroring [`validate_fine_system_ai_policy`].
///
/// Rejects:
///   - an unknown candidate source id,
///   - an unparseable `eligibility` or score `when` expression,
///   - a `param(...)` reference to a parameter the author never declared.
///
/// HOST-AGNOSTIC, for the same reason [`validate_fine_system_ai_policy`] is:
/// the `flag(...)`/`counter(...)` check needs the host, so it lives in
/// [`validate_fine_system_ai_selector_for`].
pub fn validate_fine_system_ai_selector(
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
) -> Result<(), String> {
    validate_selector_inner(cfg, valid_sources, None)
}

/// [`validate_fine_system_ai_selector`] for a NAMED host, additionally
/// rejecting a `flag(...)`/`counter(...)` reference in the `eligibility`
/// expression or any score term's `when` that the host could never evaluate
/// (issue #891 stage 1). Four of the five selector hosts pass `&[]`, so this is
/// the same trap the policy hosts carry, on a second surface.
pub fn validate_fine_system_ai_selector_for(
    host: &crate::entities::ai_flag_hosts::AiHost,
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
) -> Result<(), String> {
    validate_selector_inner(cfg, valid_sources, Some(host))
}

fn validate_selector_inner(
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
    host: Option<&crate::entities::ai_flag_hosts::AiHost>,
) -> Result<(), String> {
    for src in &cfg.sources {
        if !valid_sources.contains(&src.as_str()) {
            return Err(format!(
                "target selector references unknown source '{src}' (valid: {valid_sources:?})"
            ));
        }
    }
    let check_params = |pred: &crate::world::flags::Predicate, what: &str| -> Result<(), String> {
        if let Some(host) = host {
            host.check_guard(&format!("target selector {what}"), pred)?;
            host.check_facts(&format!("target selector {what}"), pred)?;
        }
        // Unlike the flag chain, this one needs no host to answer (issue #890):
        // a selector evaluates through `Predicate::evaluate_selector`, which
        // hands in a DEFAULT private bag — there is no per-fine-system history
        // for a per-candidate scoring pass to fold into, on any host. So the
        // rejection fires on the host-less path too, and a `history(...)` in an
        // eligibility or score term can never become a permanently-false term.
        if let Some(atom) = pred.history_atom() {
            return Err(format!(
                "target selector {what} reads {}, but a target selector is evaluated \
                 per candidate against a snapshot with no history bag: no window is \
                 folded for it, so the comparison would read false for ever. A \
                 windowed question belongs in the owning system's policy, which has \
                 one",
                atom.render()
            ));
        }
        let mut refs = Vec::new();
        pred.referenced_params(&mut refs);
        for name in refs {
            if !cfg.param.contains_key(&name) {
                return Err(format!(
                    "target selector {what} references undeclared parameter '{name}'"
                ));
            }
        }
        Ok(())
    };
    let eligibility = crate::world::flags::parse_predicate(&cfg.eligibility)
        .map_err(|e| format!("target selector has invalid `eligibility` expression: {e}"))?;
    check_params(&eligibility, "eligibility")?;
    for (idx, term) in cfg.score.iter().enumerate() {
        let when = crate::world::flags::parse_predicate(&term.when)
            .map_err(|e| format!("target selector score term {idx} has invalid `when`: {e}"))?;
        check_params(&when, &format!("score term {idx}"))?;
    }
    Ok(())
}

/// One authored inline policy rule (`[[captain_console.ai.rule]]`, issue #775).
///
/// A rule binds a `priority` and an output `channel` to a `when` predicate
/// (the shared `world::flags` grammar, extended with typed `fact(...)` atoms
/// and `param(...)` references) and a typed `verb`. `value` is the boolean the
/// verb applies for boolean-channel verbs such as `set_red_alert`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiRuleToml {
    /// Higher wins within a channel; ties resolve to the earliest-authored rule.
    pub priority: i32,
    /// Output channel this rule contributes to (e.g. `"red_alert"`).
    pub channel: String,
    /// Guard expression; the rule fires when it evaluates `true`.
    pub when: String,
    /// Typed verb applied when this rule wins its channel (e.g. `"set_red_alert"`).
    pub verb: String,
    /// Boolean output value for boolean-channel verbs. Defaults to `false`.
    #[serde(default)]
    pub value: bool,
    /// Magnitude payload for value-carrying verbs (issue #784). Currently the
    /// absolute target level for `set_power_group_allocation`; ignored by
    /// value-less and boolean verbs. Defaults to `0`.
    #[serde(default)]
    pub level: u8,
    /// Index payload for the `respond_to_message` verb (issue #786): WHICH of
    /// the open dialogue node's responses this rule answers with. Ignored by
    /// every other verb. Deliberately a separate field from `level` — the two
    /// address different things (a power magnitude vs. a position in a fixed
    /// response list), and fusing them would make an authored rule's meaning
    /// depend on its verb. Defaults to `0` (the first response), reproducing the
    /// retired channel-2 auto-response stub. A rule that should NOT answer this
    /// tick simply does not fire; there is no "don't respond" index.
    #[serde(default)]
    pub response_index: u8,
}

/// One authored state of an inline STATEFUL policy
/// (`[[<system>.ai.state]]`, issue #882).
///
/// A state carries its own continuous `rule` list — the very same
/// [`FineSystemAiRuleToml`] the stateless path uses, so a rule's meaning does
/// not change with where it is authored — and its own outgoing `transition`
/// list. Note [`FineSystemAiRuleToml`] deliberately gained NO `state` field:
/// it is `deny_unknown_fields`, and nesting rules under the state that owns
/// them keeps a rule's owning state unambiguous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiStateToml {
    /// Unique state id within this policy; referenced by `initial_state` and
    /// by every transition's `to`.
    pub id: String,
    /// Continuous per-channel rules that apply while this state is current.
    #[serde(default)]
    pub rule: Vec<FineSystemAiRuleToml>,
    /// Outgoing transitions, at most one of which fires per eligible tick.
    #[serde(default)]
    pub transition: Vec<FineSystemAiTransitionToml>,
    /// Whether this leg yields its solved facing to a channel-3
    /// `ArcBearingRequest` (issue #918).
    ///
    /// Defaults to `true`, which is exactly what every leg did before #918: a
    /// weapon family that cannot bear takes the facing and the ship turns to
    /// make it bear (#673-#684). A leg authors `false` when the heading it flies
    /// IS the manoeuvre — a broadside ring's tangent, a fly-through escape's
    /// frozen heading — and a request that arrives while it is flown is
    /// declined instead of overwriting it.
    ///
    /// Only a system with a `yaw` channel can consume this;
    /// [`validate_fine_system_ai_policy`] rejects a `false` authored on any
    /// other system, so a declaration that could never be read is a load error
    /// rather than a silent no-op.
    #[serde(default = "default_yields_to_arc_requests")]
    pub yields_to_arc_requests: bool,
}

/// The parse default for [`FineSystemAiStateToml::yields_to_arc_requests`]:
/// a leg that says nothing yields, as every leg did before issue #918.
pub(crate) fn default_yields_to_arc_requests() -> bool {
    true
}

impl Default for FineSystemAiStateToml {
    /// Hand-written rather than derived for the reason
    /// [`FineSystemAiConfigToml::default`] is: a derived `bool` default is
    /// `false`, and `yields_to_arc_requests: false` is not "unauthored", it is
    /// a leg that declines channel-3 requests. `..Default::default()` has to
    /// mean the same thing as an omitted field in TOML.
    fn default() -> Self {
        Self {
            id: String::new(),
            rule: Vec::new(),
            transition: Vec::new(),
            yields_to_arc_requests: default_yields_to_arc_requests(),
        }
    }
}

/// One authored transition out of the enclosing state
/// (`[[<system>.ai.state.transition]]`, issue #882).
///
/// There is no `from`: the source is the state this table is nested in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiTransitionToml {
    /// Higher wins; ties resolve to the earliest-authored transition.
    pub priority: i32,
    /// The state id entered when this transition fires.
    pub to: String,
    /// Guard expression; the transition becomes eligible when it evaluates
    /// `true`. May read `memory(...)` and `state_time` as well as the usual
    /// facts/flags/params.
    pub when: String,
}

/// Inline AI policy for an AI-capable fine system
/// (`[captain_console.ai]`, issues #775, #882).
///
/// A system declares EITHER a policy (`param` + `rule`, and optionally the
/// #882 state machine) OR an explicit `idle = true`. An empty declaration
/// (`ai = {}`) is neither and is rejected by
/// [`validate_fine_system_ai_policy`] — silence is not a valid declaration.
///
/// ## Back-compat guarantee (issue #882)
///
/// Every field added by the stateful path is `#[serde(default)]`, so all
/// twelve shipped stateless blocks parse byte-identically and decode to a
/// policy whose `machine` is `None`. A block that authors no `state` never
/// enters the transition code path at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiConfigToml {
    /// How many shared AI base ticks (`[global] ai_tick_hz`) pass between two
    /// evaluations of this policy (PRD #774 §9, issue #889).
    ///
    /// `1` — the parse default — means "every base tick", which is what every
    /// shipped policy authors today. A larger integer lets a host such as
    /// Sensors, Power, Repair or Comms decide less often as **authored data**,
    /// instead of the second hardcoded Rust `Timer` that #889 retired. The
    /// field is typed `u32`, so a non-integer multiple of the base cadence is a
    /// TOML type error at load; `0` is rejected by
    /// [`validate_fine_system_ai_policy`] (a policy that never evaluates is an
    /// `idle = true` declaration, not a cadence).
    #[serde(default = "default_evaluate_every_ticks")]
    pub evaluate_every_ticks: u32,
    /// Explicit idle marker. Mutually exclusive with `rule` and with `state`.
    #[serde(default)]
    pub idle: bool,
    /// Named numeric parameters referenced by rule guards.
    #[serde(default)]
    pub param: std::collections::HashMap<String, f32>,
    /// Prioritised per-channel reactive rules (the stateless path).
    #[serde(default)]
    pub rule: Vec<FineSystemAiRuleToml>,
    /// The state entered on reset. Required when — and rejected unless —
    /// `state` is non-empty (issue #882).
    #[serde(default)]
    pub initial_state: Option<String>,
    /// The declared states of the OPTIONAL state machine (issue #882).
    /// Absent/empty ⇒ this is a stateless policy.
    #[serde(default)]
    pub state: Vec<FineSystemAiStateToml>,
    /// Typed private memory declarations: name → initial value (issue #882).
    /// Readable through the `memory(name)` atom by THIS fine system only.
    #[serde(default)]
    pub memory: std::collections::HashMap<String, f32>,
}

/// The parse default for [`FineSystemAiConfigToml::evaluate_every_ticks`]:
/// evaluate on every shared AI base tick.
pub(crate) fn default_evaluate_every_ticks() -> u32 {
    1
}

impl Default for FineSystemAiConfigToml {
    /// Hand-written rather than derived so that `..Default::default()` yields
    /// the same `evaluate_every_ticks` the TOML parse default supplies. A
    /// derived `0` would be a policy that never evaluates.
    fn default() -> Self {
        Self {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: Vec::new(),
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        }
    }
}

impl FineSystemAiConfigToml {
    /// Resolve this authored block into the pure typed [`crate::ai::policy::AiPolicy`]
    /// the runtime evaluator consumes.
    ///
    /// Returns a diagnostic `Err` on an unparseable guard or unknown verb; call
    /// [`validate_fine_system_ai_policy`] first at content-load time so this
    /// never fails after world activation.
    pub fn to_policy(&self) -> Result<crate::ai::policy::AiPolicy, String> {
        let mut params = crate::world::flags::AiParams::new();
        for (k, v) in &self.param {
            params.set(k, *v as f64);
        }
        let rules = decode_rules(&self.rule)?;
        // The OPTIONAL #882 state machine. No authored `state` tables ⇒ `None`,
        // which is what every shipped stateless block decodes to.
        let machine = if self.state.is_empty() {
            None
        } else {
            let mut states = Vec::with_capacity(self.state.len());
            for s in &self.state {
                let mut transitions = Vec::with_capacity(s.transition.len());
                for t in &s.transition {
                    transitions.push(crate::ai::policy::AiPolicyTransition {
                        priority: t.priority,
                        to: t.to.clone(),
                        when: crate::world::flags::parse_predicate(&t.when)?,
                    });
                }
                states.push(crate::ai::policy::AiPolicyState {
                    id: s.id.clone(),
                    rules: decode_rules(&s.rule)?,
                    transitions,
                    yields_to_arc_requests: s.yields_to_arc_requests,
                });
            }
            Some(crate::ai::policy::AiPolicyMachine {
                initial: self.initial_state.clone().ok_or_else(|| {
                    "ai policy declares states but no `initial_state`".to_string()
                })?,
                initial_memory: self.initial_memory(),
                states,
            })
        };
        Ok(crate::ai::policy::AiPolicy {
            params,
            rules,
            idle: self.idle,
            machine,
        })
    }

    /// The authored initial values of this policy's typed private memory
    /// (issue #882), as the runtime bag a fresh state component starts from.
    pub fn initial_memory(&self) -> crate::world::flags::AiPolicyMemory {
        let mut m = crate::world::flags::AiPolicyMemory::new();
        for (k, v) in &self.memory {
            m.set(k, *v as f64);
        }
        m
    }
}

/// Decode one authored rule list into typed policy rules (issue #882).
///
/// Shared by the top-level stateless `rule` list and by each state's own
/// `rule` list, so a rule decodes identically wherever it is authored.
fn decode_rules(
    src: &[FineSystemAiRuleToml],
) -> Result<Vec<crate::ai::policy::AiPolicyRule>, String> {
    let mut rules = Vec::with_capacity(src.len());
    for r in src {
        let when = crate::world::flags::parse_predicate(&r.when)?;
        rules.push(crate::ai::policy::AiPolicyRule {
            priority: r.priority,
            channel: r.channel.clone(),
            when,
            verb: decode_verb(r)?,
        });
    }
    Ok(rules)
}

/// Decode one authored rule's `verb` (plus its payload fields) into the typed
/// [`crate::ai::policy::AiPolicyVerb`] (issue #882 extraction; the match body
/// is unchanged from #775–#786).
fn decode_verb(r: &FineSystemAiRuleToml) -> Result<crate::ai::policy::AiPolicyVerb, String> {
    Ok(match r.verb.as_str() {
        CAPTAIN_SET_RED_ALERT_VERB => crate::ai::policy::AiPolicyVerb::SetRedAlert(r.value),
        // Helm continuous-actuator mode verbs (issue #779): value-less;
        // the `value` field is ignored — the magnitude lives in the
        // planner fact, not the policy.
        HELM_ACTUATE_DESIRED_TRAVEL_VERB => crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
        HELM_ACTUATE_DESIRED_FACING_VERB => crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
        // The frozen-heading Steering mode verb (issue #883): also value-less —
        // the heading is host-written private memory, not an authored constant.
        HELM_HOLD_COMMITTED_HEADING_VERB => crate::ai::policy::AiPolicyVerb::HoldCommittedHeading,
        // The recovery-orbit and re-engage Steering mode verbs (issue #788):
        // value-less too — the ring's radius is derived from the TARGET's
        // reach and the circulation direction is host-written private memory,
        // neither of which an authored constant could express.
        HELM_HOLD_RECOVERY_ORBIT_VERB => crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit,
        HELM_PIVOT_TO_REENGAGE_VERB => crate::ai::policy::AiPolicyVerb::PivotToReengage,
        // The combat broadside orbit (issue #790): value-less too — the ring's
        // radius, throttle and spiral gain are authored Steering `param`s and
        // the circulation direction is host-written private memory.
        HELM_HOLD_COMBAT_ORBIT_VERB => crate::ai::policy::AiPolicyVerb::HoldCombatOrbit,
        // The torpedo-opportunity bow hold (issue #791): value-less too — the
        // throttle is an authored Steering `param`, and which shield is down,
        // which arc the tubes cover and whether a salvo is still in flight are
        // all host readings.
        HELM_HOLD_TORPEDO_BEARING_VERB => crate::ai::policy::AiPolicyVerb::HoldTorpedoBearing,
        // The artillery firing position (issue #792): value-less too — the hold
        // throttle and the range band are authored Steering `param`s, and the
        // lead speed is a host reading of the hull's own artillery bolt.
        HELM_HOLD_ARTILLERY_POSITION_VERB => crate::ai::policy::AiPolicyVerb::HoldArtilleryPosition,
        // Helm secondary-actuator mode verbs (issue #780): value-less,
        // like the travel-axis verbs above.
        HELM_ACTUATE_LATERAL_THRUST_VERB => crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        HELM_ACTUATE_VERTICAL_THRUST_VERB => crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        HELM_ENGAGE_IMPULSE_VERB => crate::ai::policy::AiPolicyVerb::EngageImpulse,
        HELM_ENGAGE_BOOST_VERB => crate::ai::policy::AiPolicyVerb::EngageBoost,
        // Weapon-bank action verbs (issue #781): value-less, like the
        // helm mode verbs. The `value` field is ignored — the target and
        // firing bank come from the host context, not the policy.
        PHASER_FIRE_VERB => crate::ai::policy::AiPolicyVerb::FirePhaser,
        BLASTER_FIRE_VERB => crate::ai::policy::AiPolicyVerb::FireBlaster,
        // Torpedo tube + magazine action verbs (issue #782): value-less,
        // like the weapon-bank verbs. The `value` field is ignored — the
        // tube, volley target, combat lock, and magazine come from the
        // host context, not the policy.
        TORPEDO_LOAD_VERB => crate::ai::policy::AiPolicyVerb::LoadTorpedo,
        TORPEDO_LAUNCH_VERB => crate::ai::policy::AiPolicyVerb::LaunchTorpedo,
        TORPEDO_MAGAZINE_GRANT_VERB => crate::ai::policy::AiPolicyVerb::GrantTorpedoRound,
        // Torpedo conservation verb (issue #943): value-less too — the
        // magazine level, the remaining mission threat and this ship's own
        // objective count are all host readings, and the reserve they are
        // compared against is an authored `param`.
        TORPEDO_RELEASE_VERB => crate::ai::policy::AiPolicyVerb::ReleaseTorpedo,
        // Shields focus action verb (issue #783): value-less, like the
        // weapon-bank verbs. The `value` field is ignored — which of the
        // four arcs is focused comes from the retained ranking kernel in
        // the host context, not the policy.
        SHIELD_FOCUS_VERB => crate::ai::policy::AiPolicyVerb::FocusShieldArc,
        // Power group allocation verb (issue #784): the FIRST verb to
        // carry a magnitude. The absolute target level is the authored
        // per-rule `level` payload, never an inline Rust number.
        POWER_SET_ALLOCATION_VERB => {
            crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(r.level)
        }
        // Comms dialogue-response verb (issue #786): the SECOND
        // value-carrying verb. Only the response INDEX rides the verb —
        // WHICH message is being answered comes from the host context.
        COMMS_RESPOND_VERB => crate::ai::policy::AiPolicyVerb::RespondToMessage(r.response_index),
        // Weapons doctrine family verbs (issue #956): value-less, like the
        // fire verbs. The RANK is the channel and the FAMILY is the verb; the
        // arcs, ranges and geometry the resulting `ArcBearingRequest` carries
        // are host readings of this ship's own emitters, never authored here.
        BRING_PHASERS_TO_BEAR_VERB => crate::ai::policy::AiPolicyVerb::BringPhasersToBear,
        BRING_BLASTERS_TO_BEAR_VERB => crate::ai::policy::AiPolicyVerb::BringBlastersToBear,
        BRING_TORPEDOES_TO_BEAR_VERB => crate::ai::policy::AiPolicyVerb::BringTorpedoesToBear,
        other => return Err(format!("unknown ai policy verb '{other}'")),
    })
}

/// Validate an inline fine-system AI policy before world activation
/// (issues #775, #882), mirroring [`validate_phaser_banks`] et al.
///
/// Rejects (stateless path, issue #775):
///   - a silent declaration (neither `idle` nor any `rule` nor any `state`),
///   - a contradictory declaration (`idle = true` alongside rules/states),
///   - a MIXED declaration: top-level `rule`s alongside `state`s (issue #883) —
///     a machine resolves only in-state, so those rules are silently dead,
///   - an unparseable `when` guard (reusing `parse_predicate`'s diagnostic),
///   - an unknown output `channel` or `verb`,
///   - a `param(...)` reference to a parameter the author never declared.
///
/// Additionally rejects (stateful path, issue #882 AC6):
///   - an `initial_state` naming a state that was never declared (and a
///     `state` list with no `initial_state` at all),
///   - a transition whose `to` names a state that was never declared,
///   - duplicate state ids,
///   - an unreachable state (no inbound transition and not the initial state),
///   - a `memory(...)` or `state_time` reference in a STATELESS policy, and a
///     `memory(...)` reference to a slot the author never declared.
///
/// Additionally rejects (issue #794, PRD #774's deterministic-policy
/// categories):
///   - two or more transitions out of ONE state authored at the same
///     `priority`,
///   - two or more rules on ONE output channel authored at the same `priority`
///     — within a state for a machine, within the top-level list for a
///     stateless policy.
///
/// Both are the same defect wearing two hats. Resolution is "highest priority
/// wins, ties to the earliest-authored", so a tie IS resolved — silently, by
/// where the author happened to put the table. The file then reads as though
/// the two entries were interchangeable when the runtime has already picked
/// one, and re-ordering the file changes behaviour without changing a value.
/// Distinct priorities cost the author one character and make the decision
/// legible. Note the scope on both: repeated priorities across DIFFERENT
/// channels (a load rule and a launch rule both at 0) never compete, and are
/// left alone.
///
/// HOST-AGNOSTIC. `flag(...)`/`counter(...)` guards only read true where the
/// host passes a populated flag-store chain, and most hosts pass `&[]`
/// (issue #891); that check needs to know WHICH host is being validated, so it
/// lives in [`validate_fine_system_ai_policy_for`]. Every production call site
/// in [`crate::entities::config::EntityConfig::from_toml`] uses that one — a test asserts it — and this
/// entry point is for ad-hoc and unit-test validation of a policy with no host.
pub fn validate_fine_system_ai_policy(
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
) -> Result<(), String> {
    validate_policy_inner(cfg, valid_channels, valid_verbs, None)
}

/// [`validate_fine_system_ai_policy`] for a NAMED host, additionally rejecting a
/// `flag(...)`/`counter(...)` guard the host could never evaluate (issue #891
/// stage 1 — see [`crate::entities::ai_flag_hosts`]).
pub fn validate_fine_system_ai_policy_for(
    host: &crate::entities::ai_flag_hosts::AiHost,
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
) -> Result<(), String> {
    validate_policy_inner(cfg, valid_channels, valid_verbs, Some(host))
}

fn validate_policy_inner(
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
    host: Option<&crate::entities::ai_flag_hosts::AiHost>,
) -> Result<(), String> {
    // Cadence first (issue #889): `evaluate_every_ticks` counts shared AI base
    // ticks, so it has to be a POSITIVE integer. `u32` already makes a
    // non-integer multiple of the base a TOML type error; zero would be a
    // policy that never evaluates, which is `idle = true`, not a cadence.
    if cfg.evaluate_every_ticks == 0 {
        return Err(
            "ai policy declares evaluate_every_ticks = 0: the value counts shared AI base \
             ticks between evaluations and must be a positive integer. A policy that should \
             never evaluate declares idle = true"
                .into(),
        );
    }
    if cfg.idle {
        if !cfg.rule.is_empty() {
            return Err("ai policy declares idle = true but also carries rules".into());
        }
        if !cfg.state.is_empty() {
            return Err("ai policy declares idle = true but also carries states".into());
        }
        return Ok(());
    }
    if cfg.rule.is_empty() && cfg.state.is_empty() {
        return Err(
            "ai policy is empty: declare at least one rule or state, or set idle = true".into(),
        );
    }
    // A policy is EITHER stateless (top-level `rule`) or a machine (`state`),
    // never both (issue #883, carried forward from the #882 review). A machine
    // resolves EXCLUSIVELY through `resolve_channel_in_state`, so top-level
    // rules on a stateful policy are silently dead code — and worse, the
    // `stateful` flag below makes a `memory(...)` reference inside one VALIDATE
    // while always evaluating false at runtime, because the stateless scan hands
    // `best_in` an empty memory bag. Both failures are silent, which is exactly
    // the class of defect #882's blocking bug belonged to, so the shape is
    // rejected at load rather than merely discouraged.
    if !cfg.rule.is_empty() && !cfg.state.is_empty() {
        return Err(format!(
            "ai policy declares both top-level rules ({}) and states ({}): a stateful \
             policy resolves only inside its current state, so the top-level rules \
             would never fire. Move them into the state(s) that should own them, or \
             delete the state machine",
            cfg.rule.len(),
            cfg.state.len()
        ));
    }
    let stateful = !cfg.state.is_empty();

    // ── Per-rule checks, run unchanged over the top-level rules and over each
    // state's own rules (issue #882 extends the loop's reach, not its body).
    let check_rule = |what: &str, r: &FineSystemAiRuleToml| -> Result<(), String> {
        if !valid_channels.contains(&r.channel.as_str()) {
            return Err(format!(
                "ai policy {what} has unknown channel '{}' (valid: {valid_channels:?})",
                r.channel
            ));
        }
        if !valid_verbs.contains(&r.verb.as_str()) {
            return Err(format!(
                "ai policy {what} has unknown verb '{}' (valid: {valid_verbs:?})",
                r.verb
            ));
        }
        let pred = crate::world::flags::parse_predicate(&r.when)
            .map_err(|e| format!("ai policy {what} has invalid `when` expression: {e}"))?;
        check_policy_predicate(cfg, stateful, host, &pred, what)
    };
    for (idx, r) in cfg.rule.iter().enumerate() {
        check_rule(&format!("rule {idx}"), r)?;
    }
    check_rule_priorities("", &cfg.rule)?;

    if !stateful {
        return Ok(());
    }

    // ── State-graph checks (issue #882 AC6) ─────────────────────────────────
    let mut seen: Vec<&str> = Vec::with_capacity(cfg.state.len());
    for s in &cfg.state {
        if seen.contains(&s.id.as_str()) {
            return Err(format!("ai policy declares duplicate state id '{}'", s.id));
        }
        seen.push(&s.id);
    }
    let Some(initial) = cfg.initial_state.as_deref() else {
        return Err("ai policy declares states but no `initial_state`".into());
    };
    if !seen.contains(&initial) {
        return Err(format!(
            "ai policy `initial_state` names undeclared state '{initial}' (declared: {seen:?})"
        ));
    }
    for s in &cfg.state {
        for (tidx, t) in s.transition.iter().enumerate() {
            let what = format!("state '{}' transition {tidx}", s.id);
            if !seen.contains(&t.to.as_str()) {
                return Err(format!(
                    "ai policy {what} targets undeclared state '{}' (declared: {seen:?})",
                    t.to
                ));
            }
            let pred = crate::world::flags::parse_predicate(&t.when)
                .map_err(|e| format!("ai policy {what} has invalid `when` expression: {e}"))?;
            check_policy_predicate(cfg, stateful, host, &pred, &what)?;
        }
        check_transition_priorities(&s.id, &s.transition)?;
        for (idx, r) in s.rule.iter().enumerate() {
            check_rule(&format!("state '{}' rule {idx}", s.id), r)?;
        }
        check_rule_priorities(&format!("state '{}' ", s.id), &s.rule)?;
        // A leg that declines channel-3 arc-bearing requests (issue #918) can
        // only be read by a system that steers, and the `yaw` channel is what
        // makes a system one. Authored anywhere else it is a declaration
        // nothing will ever consult — the silent-no-op class this validator
        // exists to turn into a load error.
        if !s.yields_to_arc_requests && !valid_channels.contains(&HELM_YAW_CHANNEL) {
            return Err(format!(
                "ai policy state '{}' declares yields_to_arc_requests = false, but this \
                 system drives {valid_channels:?} and an arc-bearing request is answered \
                 on the '{HELM_YAW_CHANNEL}' channel: nothing would ever read the \
                 declaration",
                s.id
            ));
        }
    }
    // Reachability is a FIXPOINT walk from `initial`, following transitions only
    // out of states already known reachable. A single pass over every state's
    // transitions would only catch zero-inbound orphans: a disconnected cluster
    // (`initial = a`; `b -> c`; `c -> b`) is targeted by transitions and would
    // pass, yet nothing can ever enter it. Every transition target is already
    // known to name a declared state by the loop above, so this walk cannot
    // wander off the graph.
    let mut reachable: Vec<&str> = vec![initial];
    let mut frontier: Vec<&str> = vec![initial];
    while let Some(current) = frontier.pop() {
        let Some(s) = cfg.state.iter().find(|s| s.id == current) else {
            continue;
        };
        for t in &s.transition {
            if !reachable.contains(&t.to.as_str()) {
                reachable.push(&t.to);
                frontier.push(&t.to);
            }
        }
    }
    for s in &cfg.state {
        if !reachable.contains(&s.id.as_str()) {
            return Err(format!(
                "ai policy declares unreachable state '{}': it is neither the \
                 initial state nor the target of any transition",
                s.id
            ));
        }
    }
    Ok(())
}

/// Reject two or more transitions out of one state sharing a `priority`
/// (issue #794, PRD #774's "competing equal-priority transitions").
///
/// The transition set of a state is a single winner-take-all race: the runtime
/// takes the highest-priority ELIGIBLE transition and breaks ties by authoring
/// order. So an authored tie is not an ambiguity the runtime chokes on — it is
/// a decision the runtime makes and the file does not record. Moving one of the
/// two tables past the other then changes which state the hull enters, with no
/// value anywhere in the file having changed.
///
/// The pair is reported rather than the count, and both `to` targets are named:
/// the author's next question is always "which two?", and two ties in a
/// six-transition state are otherwise indistinguishable from one.
fn check_transition_priorities(
    state_id: &str,
    transitions: &[FineSystemAiTransitionToml],
) -> Result<(), String> {
    for (i, a) in transitions.iter().enumerate() {
        for (j, b) in transitions.iter().enumerate().skip(i + 1) {
            if a.priority == b.priority {
                return Err(format!(
                    "ai policy state '{state_id}' declares transitions {i} (to '{}') and {j} \
                     (to '{}') at the same priority {}: equal-priority transitions out of one \
                     state are resolved by authoring order, so the file never says which wins. \
                     Give them distinct priorities",
                    a.to, b.to, a.priority
                ));
            }
        }
    }
    Ok(())
}

/// Reject two or more rules on one output channel sharing a `priority`
/// (issue #794, PRD #774's "competing equal-priority rules on one output
/// channel").
///
/// The sibling of [`check_transition_priorities`], and the same defect: channel
/// resolution is winner-take-all per channel with ties broken by authoring
/// order. The scope is deliberately (channel, priority) and NOT priority alone
/// — rules on different channels never compete, so a tube authoring a
/// `torpedo_load` rule and a `torpedo_launch` rule both at priority 0 is
/// ordinary content, not a tie.
///
/// `scope` is `""` for a stateless policy's top-level list, or
/// `"state '<id>' "` for a machine's per-state list, so the message reads as a
/// sentence either way.
fn check_rule_priorities(scope: &str, rules: &[FineSystemAiRuleToml]) -> Result<(), String> {
    for (i, a) in rules.iter().enumerate() {
        for (j, b) in rules.iter().enumerate().skip(i + 1) {
            if a.priority == b.priority && a.channel == b.channel {
                return Err(format!(
                    "ai policy {scope}declares rules {i} (verb '{}') and {j} (verb '{}') on \
                     channel '{}' at the same priority {}: equal-priority rules on one output \
                     channel are resolved by authoring order, so the file never says which \
                     wins. Give them distinct priorities",
                    a.verb, b.verb, a.channel, a.priority
                ));
            }
        }
    }
    Ok(())
}

/// Shared guard-expression checks for a policy predicate (issues #775, #882).
///
/// `param(...)` must be declared; `memory(...)` must be declared AND the policy
/// must be stateful; `state_time` requires a stateful policy. The stateless
/// rejections are AC6's "a memory or state-time reference in a stateless
/// policy" — private state is meaningless without a state machine to own it,
/// and silently reading `false` would be a trap.
///
/// `host` carries the same reasoning one step further (issue #891): a
/// `flag(...)`/`counter(...)` reference is meaningless on a host that evaluates
/// with an empty flag chain, and would likewise read `false` for ever. It is
/// `None` only for host-less validation (unit tests, ad-hoc checks), where the
/// question has no answer to give.
fn check_policy_predicate(
    cfg: &FineSystemAiConfigToml,
    stateful: bool,
    host: Option<&crate::entities::ai_flag_hosts::AiHost>,
    pred: &crate::world::flags::Predicate,
    what: &str,
) -> Result<(), String> {
    if let Some(host) = host {
        host.check_guard(&format!("ai policy {what}"), pred)?;
        host.check_facts(&format!("ai policy {what}"), pred)?;
    }
    let mut refs = Vec::new();
    pred.referenced_params(&mut refs);
    for name in refs {
        if !cfg.param.contains_key(&name) {
            return Err(format!(
                "ai policy {what} references undeclared parameter '{name}'"
            ));
        }
    }
    let mut mem_refs = Vec::new();
    pred.referenced_memory(&mut mem_refs);
    if !stateful && !mem_refs.is_empty() {
        return Err(format!(
            "ai policy {what} references memory('{}') but the policy declares no states: \
             private memory requires a stateful policy",
            mem_refs[0]
        ));
    }
    for name in mem_refs {
        if !cfg.memory.contains_key(&name) {
            return Err(format!(
                "ai policy {what} references undeclared memory '{name}'"
            ));
        }
    }
    if !stateful && pred.references_state_time() {
        return Err(format!(
            "ai policy {what} references state_time but the policy declares no states: \
             state time requires a stateful policy"
        ));
    }
    check_history_windows(cfg, stateful, pred, what)
}

/// Reject an authored `history(...)` atom the runtime could not honour
/// (issue #890).
///
/// Two rejections, and they close the two halves of the same trap:
///
/// * a history atom in a STATELESS policy. The window is per-fine-system
///   retained state carried on the same private bag as `memory(...)`, folded by
///   the host that ticks the state machine — a policy with no machine is never
///   ticked, so the window would never be advanced and the guard would read
///   false for ever. (`AiHost::check_guard` catches the sibling case: a stateful
///   policy on a host with no fold at all.)
/// * a window length that is not a positive whole number of shared AI ticks. A
///   literal is caught by the parser, which is the only place that sees it; a
///   `param(...)` can only be checked HERE, against its declared value, and a
///   fractional or zero one would silently disable the operator (a zero-capacity
///   window retains nothing and is never full — see [`crate::bounded_history`]).
fn check_history_windows(
    cfg: &FineSystemAiConfigToml,
    stateful: bool,
    pred: &crate::world::flags::Predicate,
    what: &str,
) -> Result<(), String> {
    let mut refs = Vec::new();
    pred.referenced_history(&mut refs);
    if refs.is_empty() {
        return Ok(());
    }
    if !stateful {
        return Err(format!(
            "ai policy {what} reads {} but the policy declares no states: a bounded \
             history window is per-fine-system retained state, advanced once per \
             shared AI tick by the host that ticks the state machine, so it requires \
             a stateful policy",
            refs[0].render()
        ));
    }
    for atom in &refs {
        let ticks = match &atom.window.ticks {
            crate::world::flags::Operand::Number(n) => *n,
            // An UNDECLARED parameter is already the caller's error above; this
            // arm only skips re-reporting it under a worse message.
            crate::world::flags::Operand::Param(name) => match cfg.param.get(name) {
                Some(value) => *value as f64,
                None => continue,
            },
        };
        if !ticks.is_finite() || ticks.fract() != 0.0 || ticks < 1.0 {
            return Err(format!(
                "ai policy {what} reads {} whose window length resolves to {ticks}: a \
                 history window counts shared AI ticks, so it must be a positive whole \
                 number",
                atom.render()
            ));
        }
    }
    Ok(())
}
