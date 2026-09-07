//! Direct/internal GM damage and healing on one Entity (issue #1310, PRD #930
//! milestone M2 Directing).
//!
//! A GM names an entity and an absolute amount. The effect is *internal*: it
//! bypasses shields entirely and lands on [`crate::ship::damage::SystemHull`],
//! where the ordinary weighted distribution decides which system absorbs it —
//! the same distribution a beam hit uses, and the same one that reduces an
//! NPC's single-entry hull (a legacy scalar `hull_integrity` becomes a
//! one-system `SystemHull` at spawn, so "the NPC scalar path" is the ordinary
//! path applied to a hull of one).
//!
//! # Two phases, for the reason a Fire has two
//!
//! [`crate::gm_action::apply_due_actions`] runs in `PreUpdate`, outside
//! `SimSet`, and holds none of the destruction lifecycle's parameters. It
//! therefore RESOLVES the effect at the agreed apply tick — target lookup,
//! clamping, the discarded overflow, whether the hit is lethal — and arms
//! [`PendingGmDirectEffects`]; [`apply_gm_direct_effects`] then applies exactly
//! that resolved amount inside `SimSet::Damage`, where destruction, despawn,
//! balance events and the world registry are already this simulation's
//! business. Building a second damage path into `PreUpdate` is the one shape
//! that would make GM damage differ from weapon damage.
//!
//! Resolving in the reducer is also what makes the reported numbers exact: the
//! arm carries an already-clamped amount, so what the applier lands and what
//! the durable result says cannot drift apart.
//!
//! # Randomness
//!
//! The distribution draws from [`crate::sim_rng::with_event_generator`] keyed
//! on the grant's canonical [`crate::gm_action::GmActionOrder`] sequence, not
//! from a running [`crate::sim_rng::SimStream`]. See that function for why.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::messages::{GamePhase, ServerMessage};
use crate::entities::spawner::EntitySystemHull;
use crate::gm_action::GmActionOrder;
use crate::lobby::Target;
use crate::server_app::{AsteroidUuid, GameOverReason, SimOutbox};

/// Which part of the target an effect is scoped to.
///
/// One mechanic whose scope narrows, not three action families: the PASM entity
/// `gm-t2-directed-world-actions` describes a single directed effect, so the
/// scope is a FIELD of that mechanic. Variants are APPEND-ONLY — the grant is
/// postcard-encoded into the deterministic digest, which writes an enum by
/// variant index, so inserting a variant would silently move the digest of
/// every past run that recorded one after it.
///
/// The tagging is serde's DEFAULT (external): `"entity"`, `{"station": "..."}`,
/// `{"system": "..."}`. `#[serde(tag = ...)]` is deliberately not used —
/// internally and adjacently tagged enums buffer content through
/// `deserialize_any`, which postcard (not self-describing) cannot answer, and
/// this type has to survive a postcard round trip in the snapshot.
///
/// The narrowed scopes are ship-local authoring keys resolved against the
/// TARGET's own hull and [`crate::ship::config::ShipConfig`] at the agreed
/// apply tick — see [`scope_systems`] — never against a global registry, so a
/// Station id means the same thing here it means everywhere else on that hull.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmDirectEffectScope {
    /// The whole hull, using the ordinary weighted distribution.
    Entity,
    /// The Systems that Station authored as its own, using the same weighted
    /// distribution restricted to them.
    Station(crate::core::messages::StationId),
    /// Exactly one System. No distribution runs at all — there is nothing to
    /// distribute across, and the spill loop of a one-entry walk is the
    /// identity.
    System(crate::core::messages::SystemId),
}

/// Why a scope names nothing this hull can be asked to damage or heal.
///
/// Its own enum rather than a [`crate::gm_action::GmActionRefusalReason`] so
/// this module stays the one that knows how a scope resolves, and the reducer
/// stays the one that knows how a refusal is spelled on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GmDirectEffectScopeError {
    /// The target has no `[[station]]` with that id — including every target
    /// that authors no stations at all (a structure, an asteroid, a bare
    /// `[behaviour]` hull).
    UnknownStation,
    /// The target's hull tracks no System with that id. Deliberately asked of
    /// the HULL rather than of the ship config: a `[[system]]` with no
    /// `[[hull.system_hull]]` entry (every Alliance radar) is authored but not
    /// damageable, and "nothing there answers to that name" is the same answer
    /// the operator needs for both.
    UnknownSystem,
    /// The Station exists and owns Systems, but this hull tracks none of them —
    /// the scoped spelling of [`crate::gm_action::GmActionRefusalReason::TargetNotDamageable`].
    NotDamageable,
}

/// The Systems a scope permits an effect to touch, or `None` for the whole
/// hull.
///
/// The ONE place a scope becomes a set of Systems. The reducer calls it to
/// decide the refusal and to measure the totals a result is resolved against;
/// [`apply_gm_direct_effects`] calls it again at the damage phase to restrict
/// the distribution. Two derivations of "which Systems does this Station own"
/// is exactly the drift that would let a result promise what the world does not
/// do, so there is one.
///
/// The returned order is the HULL's declaration order, not the ship config's,
/// because that is the order the weighted walk consumes and a set whose order
/// came from somewhere else would make the draw depend on which caller built
/// it.
pub fn scope_systems(
    scope: &GmDirectEffectScope,
    hull: &crate::ship::damage::SystemHull,
    config: Option<&crate::ship::config::ShipConfig>,
) -> Result<Option<Vec<crate::core::messages::SystemId>>, GmDirectEffectScopeError> {
    match scope {
        GmDirectEffectScope::Entity => Ok(None),
        GmDirectEffectScope::System(system) => {
            if hull.get(system).is_some() {
                Ok(Some(vec![system.clone()]))
            } else {
                Err(GmDirectEffectScopeError::UnknownSystem)
            }
        }
        GmDirectEffectScope::Station(station) => {
            let config = config.ok_or(GmDirectEffectScopeError::UnknownStation)?;
            if config.station(station).is_none() {
                return Err(GmDirectEffectScopeError::UnknownStation);
            }
            let owned: std::collections::BTreeSet<&crate::core::messages::SystemId> = config
                .systems_for_station(station)
                .map(|system| &system.id)
                .collect();
            let systems: Vec<crate::core::messages::SystemId> = hull
                .iter()
                .map(|(id, _)| id)
                .filter(|id| owned.contains(id))
                .cloned()
                .collect();
            if systems.is_empty() {
                Err(GmDirectEffectScopeError::NotDamageable)
            } else {
                Ok(Some(systems))
            }
        }
    }
}

/// `(current, max)` over the Systems a resolved scope names.
pub fn scope_totals(
    hull: &crate::ship::damage::SystemHull,
    systems: Option<&[crate::core::messages::SystemId]>,
) -> (f32, f32) {
    (
        hull.total_current_within(systems),
        hull.total_max_within(systems),
    )
}

/// Whether two resolved scopes can touch the same System.
///
/// `None` is the whole hull and therefore overlaps everything. Used only by
/// [`PendingGmDirectEffects::project_totals`], where the answer decides whether
/// an already-armed effect is part of what the next press measures against.
fn scopes_overlap(
    left: Option<&[crate::core::messages::SystemId]>,
    right: Option<&[crate::core::messages::SystemId]>,
) -> bool {
    match (left, right) {
        (None, _) | (_, None) => true,
        (Some(left), Some(right)) => left.iter().any(|id| right.contains(id)),
    }
}

/// Whether `inner` is entirely inside `outer` — `None` is the whole hull.
///
/// The companion to [`scopes_overlap`], and the question that decides whether an
/// armed effect's amount is EXACT within the scope now being measured: an arm
/// contained by this scope lands every point it carries here, whichever of its
/// Systems absorbs them, so the whole figure can be projected. An arm that is
/// wider — or only partly overlapping — lands an unknown fraction here, and only
/// the damage phase's keyed generator knows which.
fn scope_contains(
    outer: Option<&[crate::core::messages::SystemId]>,
    inner: Option<&[crate::core::messages::SystemId]>,
) -> bool {
    match (outer, inner) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(outer), Some(inner)) => inner.iter().all(|id| outer.contains(id)),
    }
}

/// Damage or healing. Same scope, same clamping, same reporting — the sign is
/// the only difference, which is why this is one action family rather than two.
///
/// Append-only for [`GmDirectEffectScope`]'s reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmDirectEffectKind {
    Damage,
    Heal,
}

/// Hull points are carried across the wire, the journal and the digest as
/// THOUSANDTHS of a point.
///
/// An integer, because [`crate::gm_action::GmAction`] derives `Eq` — the
/// journal, the grant, the mesh frame and the durable result all compare and
/// hash — and an `f32` field would take that away from every one of them. It
/// is also the honest shape for a value a human types into a box: a GM asks
/// for "25", not for the nearest binary float to 25.
pub const MILLI_HP_PER_HP: f32 = 1000.0;

/// A live hull amount as milli-HP, saturating rather than wrapping.
pub fn hp_to_milli(hp: f32) -> u32 {
    if !hp.is_finite() || hp <= 0.0 {
        return 0;
    }
    let scaled = (hp * MILLI_HP_PER_HP).round();
    if scaled >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled as u32
    }
}

/// A live hull amount as milli-HP, rounded UP.
///
/// Headroom is measured with this rather than [`hp_to_milli`] so a resolved
/// amount always COVERS the remainder it clamped to. A hull holding 30.0004
/// points rounds to 30000 milli, and applying exactly that would leave 0.0004
/// behind — so a result that promised `destroyed` would face a hull the damage
/// phase found still alive. Rounding the clamp up costs at most one milli-HP
/// of over-reporting and makes the promise keepable.
fn hp_to_milli_ceil(hp: f32) -> u32 {
    if !hp.is_finite() || hp <= 0.0 {
        return 0;
    }
    let scaled = (hp * MILLI_HP_PER_HP).ceil();
    if scaled >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled as u32
    }
}

/// Milli-HP back to hull points.
pub fn milli_to_hp(milli: u32) -> f32 {
    milli as f32 / MILLI_HP_PER_HP
}

/// What one direct effect will actually do, decided at the agreed apply tick.
///
/// It carries the KIND as well as the numbers so the durable fact is complete
/// on its own: the activity feed and the entity panel read the bounded result
/// surface, which outlives the grant that produced it, and "60000 milli-HP"
/// means opposite things for damage and for healing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmDirectEffectResult {
    pub kind: GmDirectEffectKind,
    /// Milli-HP that will actually land on the hull.
    pub applied_milli_hp: u32,
    /// Milli-HP thrown away because the hull had no room for them: healing
    /// beyond the maxima, damage beyond what is left to destroy.
    pub discarded_milli_hp: u32,
    /// Whether this effect takes the target's whole hull to zero.
    ///
    /// Decided from the target's own live totals, so it is the same statement
    /// on every peer — and it is the fact the GM page needs to warn before a
    /// press, which is why it rides the result rather than being recomputed
    /// from a percentage.
    ///
    /// It asks the WHOLE HULL even when the effect is scoped to one Station or
    /// System (issue #1311). Emptying a Station is not sinking a ship, and a
    /// result that said "destroyed" because the scope it was clamped against
    /// reached zero would put a kill in the feed the world never performed —
    /// the same question [`crate::ship::damage::apply_hull_damage_within`] asks
    /// of the hull rather than of the allow-list, asked one phase earlier.
    pub destroyed: bool,
}

/// Resolve an absolute request against one hull's totals.
///
/// Pure — no Bevy, no RNG, and no `SystemHull`, because the reducer resolves a
/// whole tick's grants against a hull it may not touch: each one is measured
/// against what the ones canonically before it left, so two GMs pressing one
/// target on one boundary get two honest answers instead of the same one twice.
///
/// The distribution decides WHICH systems move; this decides HOW MUCH moves at
/// all, and that half has to be settled before the applier runs so the durable
/// result can carry exact numbers.
pub fn resolve_direct_effect(
    kind: GmDirectEffectKind,
    requested_milli_hp: u32,
    current_hp: f32,
    max_hp: f32,
) -> GmDirectEffectResult {
    resolve_direct_effect_within(kind, requested_milli_hp, current_hp, max_hp, current_hp)
}

/// [`resolve_direct_effect`] where the clamp and the lethality ask DIFFERENT
/// totals (issue #1311).
///
/// `current_hp`/`max_hp` are the scope's — they decide how much lands and how
/// much is discarded, which is the whole of "healing mirrors scope and clamps".
/// `hull_current_hp` is the entity's, and it decides `destroyed` alone.
///
/// Passing the scope's own current for both is exactly [`resolve_direct_effect`],
/// which is what an `Entity`-scoped effect does, so the whole-hull path has no
/// second spelling. For a narrower scope the two differ, and the difference is
/// the fact the feed would otherwise get wrong: an applied amount that empties
/// a Station is lethal to the entity only when that Station held every point
/// the hull had left — which is precisely `applied == hull_current`, because
/// everything a scoped hit lands is inside the scope.
pub fn resolve_direct_effect_within(
    kind: GmDirectEffectKind,
    requested_milli_hp: u32,
    current_hp: f32,
    max_hp: f32,
    hull_current_hp: f32,
) -> GmDirectEffectResult {
    let headroom = match kind {
        GmDirectEffectKind::Damage => hp_to_milli_ceil(current_hp),
        GmDirectEffectKind::Heal => hp_to_milli_ceil(max_hp - current_hp),
    };
    let applied = requested_milli_hp.min(headroom);
    GmDirectEffectResult {
        kind,
        applied_milli_hp: applied,
        discarded_milli_hp: requested_milli_hp - applied,
        destroyed: kind == GmDirectEffectKind::Damage
            && applied > 0
            && applied >= hp_to_milli_ceil(hull_current_hp),
    }
}

/// The target's totals after `result` lands — what the next grant on the same
/// target in the same canonical drain resolves against.
pub fn totals_after(result: &GmDirectEffectResult, current_hp: f32, max_hp: f32) -> (f32, f32) {
    let applied = milli_to_hp(result.applied_milli_hp);
    let next = match result.kind {
        GmDirectEffectKind::Damage => (current_hp - applied).max(0.0),
        GmDirectEffectKind::Heal => (current_hp + applied).min(max_hp),
    };
    (next, max_hp)
}

/// One resolved effect that has crossed its canonical apply boundary and is
/// waiting for the ordinary damage phase to land it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGmDirectEffect {
    /// The logical tick the grant applied at — the drain key, exactly as
    /// [`crate::gm_puppet::PendingGmStationCommand::tick`] is.
    pub tick: u64,
    /// Canonical order, which is both the drain sort key and the ordinal the
    /// effect's generator is derived from.
    pub order: GmActionOrder,
    /// Stable `EntityUuid` of the target. No Bevy `Entity` crosses this
    /// boundary: the arm can outlive a tick, and an entity index does not.
    pub target: String,
    pub scope: GmDirectEffectScope,
    pub kind: GmDirectEffectKind,
    /// The amount the reducer already clamped. The applier lands this and does
    /// not re-derive it, so the durable result cannot disagree with the world.
    pub amount_milli_hp: u32,
}

/// Effects resolved in `PreUpdate` and awaiting this tick's damage phase.
///
/// Folded into the deterministic digest and captured in the snapshot ONLY when
/// non-empty, so a world in which no GM presses damage is byte-identical to
/// what it was before this issue existed.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGmDirectEffects(Vec<PendingGmDirectEffect>);

impl PendingGmDirectEffects {
    pub fn push(&mut self, effect: PendingGmDirectEffect) {
        self.0.push(effect);
    }

    pub fn entries(&self) -> &[PendingGmDirectEffect] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `target`'s totals WITHIN `scope`, once every already-armed effect that
    /// can reach that scope has landed.
    ///
    /// The reducer can be entered twice before the damage phase runs at all — a
    /// paused session is exactly that — and a second press resolved against the
    /// untouched hull would let two grants each claim the same last hull point.
    /// Walking the queue is the same projection [`totals_after`] performs
    /// within one drain, extended over the arms a previous one left.
    ///
    /// # Which overlapping arms count, and in which direction (issue #1311)
    ///
    /// An arm whose scope sits INSIDE this one is exact: a Station hit removes
    /// exactly its clamped amount from that Station's total, whichever of its
    /// Systems absorbs it, so a wider scope can project the whole figure and
    /// still be right to the last point. Those are applied unconditionally.
    ///
    /// An arm WIDER than this scope (or only partly overlapping it) is not
    /// knowable: only the damage phase's keyed generator decides how much of a
    /// whole-hull hit lands inside one Station, and drawing that here would
    /// both consume the generator twice and make the reducer's answer depend on
    /// a draw the applier has not made. Such an arm is therefore applied only
    /// when it moves this press's headroom the SAFE way — down — which is
    /// exactly when its kind equals `kind`. A Damage press's headroom is the
    /// projected current, so a damage arm shrinks it (safe) and a heal arm
    /// would GROW it (unsafe); a Heal press's headroom is `max - current`, so a
    /// heal arm shrinks it (safe) and a damage arm would grow it (unsafe). The
    /// unsafe direction is skipped rather than guessed, because raising a
    /// headroom on points the applier may never deliver is precisely how a
    /// durable result comes to promise hull the world then refuses to land.
    ///
    /// The rule is deliberately CONSERVATIVE in both directions: the narrow
    /// press can resolve to less than it might have got, never to more than the
    /// hull will honour.
    pub fn project_totals(
        &self,
        target: &str,
        scope: Option<&[crate::core::messages::SystemId]>,
        hull: &crate::ship::damage::SystemHull,
        config: Option<&crate::ship::config::ShipConfig>,
        kind: GmDirectEffectKind,
        current_hp: f32,
        max_hp: f32,
    ) -> (f32, f32) {
        let mut totals = (current_hp, max_hp);
        for armed in self.0.iter().filter(|effect| effect.target == target) {
            // An arm this hull can no longer resolve is an arm the applier will
            // also skip, so it takes nothing away from anyone.
            let Ok(armed_scope) = scope_systems(&armed.scope, hull, config) else {
                continue;
            };
            if !scopes_overlap(armed_scope.as_deref(), scope) {
                continue;
            }
            // An arm INSIDE this scope lands entirely within it, so it is
            // exact. An arm that is wider (or only partly overlapping) lands an
            // unknown FRACTION here, so it may only be applied in the direction
            // that SHRINKS this press's headroom — a damage arm against a
            // damage press, a heal arm against a heal press. The opposite kind
            // would raise the headroom on points the applier may never deliver,
            // and the durable result would promise what the hull refuses.
            if !scope_contains(scope, armed_scope.as_deref()) && armed.kind != kind {
                continue;
            }
            let landed = GmDirectEffectResult {
                kind: armed.kind,
                applied_milli_hp: armed.amount_milli_hp,
                discarded_milli_hp: 0,
                destroyed: false,
            };
            totals = totals_after(&landed, totals.0, totals.1);
        }
        totals
    }

    /// Take every effect due at or before `tick`, in canonical order.
    ///
    /// Sorted by `(tick, order)` rather than by push order for
    /// [`crate::gm_puppet::PendingGmStationCommands::take_due`]'s reason: a
    /// restored snapshot's queue and a live one's must drain identically.
    pub fn take_due(&mut self, tick: u64) -> Vec<PendingGmDirectEffect> {
        let mut due = Vec::new();
        let mut future = Vec::new();
        for effect in std::mem::take(&mut self.0) {
            if effect.tick <= tick {
                due.push(effect);
            } else {
                future.push(effect);
            }
        }
        due.sort_by_key(|effect| (effect.tick, effect.order));
        self.0 = future;
        due
    }
}

/// The `weapon` label a GM direct hit carries into the balance ledger and the
/// activity feed's ordinary damage row.
///
/// A sentinel rather than a blank: the ledger keys rows by weapon, and a GM's
/// intervention is precisely the thing a balancer must be able to exclude.
pub const WEAPON_KIND_GM_DIRECT: &str = "gm.direct";

/// The ordinary damage-phase applier for resolved GM direct effects.
///
/// Deliberately shaped after [`crate::regions::server`]'s damage-zone site
/// rather than the beam's: both reproduce the same fleet-versus-NPC
/// destruction branch, and this one shares the zone's lack of a shooter, of a
/// bearing and of any shield routing at all. What it does NOT share is the
/// zone's `With<Ship>` filter — a GM may damage a structure or an authored
/// asteroid, and anything carrying an `EntitySystemHull` is a legitimate
/// target.
///
/// Damage is mirrored into [`crate::entities::spawner::EntityShipArcHull`] the
/// way every other hull-damage source mirrors it (issue #514): the per-arc pool
/// tracks TOTAL hull damage taken, it is authoritative snapshot state, and
/// `crate::ship::damage_sync::sync_console_damage_tiers` derives the
/// `shield-arc-<id>` offline entries from it. A GM hit that moved
/// `EntitySystemHull` alone would be the one hull hit in the game after which
/// the arc tiers stopped following the hull.
///
/// Healing is deliberately NOT mirrored. No production repair path restores arc
/// hull, and [`crate::ship::damage::ShipArcHull`] has no distributed restore to
/// call — only a per-arc `restore` — so inventing a distribution here would
/// make GM healing differ from every repair the crew can perform. Arc hull
/// refills exactly the way it already does, and #1311's Station/System scopes
/// inherit the same rule.
///
/// Fleet membership (`FleetSlotOf`), never `LocalShip`, decides the crewed-hull
/// branch — issue #1116's lesson, and sharper here than anywhere else: the GM
/// peer that pressed the button has no `LocalShip` at all.
///
/// The ONE `LocalShip` read in this system is the crew's own hit feedback:
/// `ServerMessage::DamageTaken` pushed to the outbox, exactly as the damage
/// zone pushes it. That message is presentation only — it drives the haptic
/// pulse (`gui/sim-state.js`) and the forcefield audio spike
/// (`crate::server::audio`) — and it is what makes a GM hit land on the crew's
/// senses like any other hit rather than in silence. Nothing FOLDED reads the
/// marker: not the destruction branch, not `GameOver`, not the applied amount,
/// not the durable result. `tests/local_ship_neutrality.rs` is the guard.
///
/// God Mode ([`crate::server_app::GodMode`]) is deliberately NOT consulted. God
/// Mode is a local debug toggle that makes this host's own ship invulnerable to
/// the SIMULATION; a GM's attributed, replicated, journalled command is not the
/// simulation shooting at them, and a peer whose developer overlay silently
/// swallowed another peer's grant would fold a different hull from everyone
/// else's.
#[allow(clippy::too_many_arguments)]
pub fn apply_gm_direct_effects(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut pending: ResMut<PendingGmDirectEffects>,
    mut targets: Query<(
        Entity,
        &crate::entities::spawner::EntityUuid,
        &mut EntitySystemHull,
        Option<&mut crate::entities::spawner::EntityShipArcHull>,
        Has<crate::lockstep::FleetSlotOf>,
        Has<crate::server_app::LocalShip>,
        Option<&AsteroidUuid>,
        // The Station→System ownership map a narrowed scope resolves against
        // (issue #1311). `Option` because a structure, an asteroid or an
        // authored marker carries no ship config at all, and a Station scope
        // aimed at one is refused rather than silently widened.
        Option<&crate::ship::components::ShipConfigComponent>,
    )>,
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    mut commands: Commands,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut destroyed_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>,
    >,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    mut world: Option<ResMut<crate::lobby::WorldResource>>,
    mut tracked: Option<ResMut<crate::server_app::TrackedEntities>>,
    log_cfg: Option<Res<crate::logging::LogFilterConfig>>,
) {
    if pending.is_empty() {
        return;
    }
    let now = tick.as_deref().map_or(0, |tick| tick.0);
    for effect in pending.take_due(now) {
        let Some((
            entity,
            uuid,
            mut hull,
            mut arc_hull,
            is_fleet_ship,
            is_local,
            asteroid,
            ship_config,
        )) = targets
            .iter_mut()
            .find(|(_, uuid, ..)| uuid.0 == effect.target)
        else {
            // The target left the world between the apply boundary and this
            // step. The durable result already said what was resolved; there is
            // nothing left to say it to.
            crate::pwarn!(
                log_cfg,
                crate::logging::LogCat::Damage,
                "GM direct effect target {} vanished before the damage phase",
                effect.target
            );
            continue;
        };
        // The scope becomes a System allow-list HERE, at the damage phase,
        // through the same `scope_systems` the reducer used to resolve the
        // amount — one derivation of "which Systems does this Station own", so
        // the durable result and the world cannot disagree about which hull
        // entries were even eligible. A scope this hull can no longer resolve
        // (its config replaced, its systems gone) is treated exactly as a
        // vanished target: the resolved result already said what was promised,
        // and there is nothing left to land it on.
        let scope = match scope_systems(&effect.scope, &hull.0, ship_config.map(|config| &config.0))
        {
            Ok(scope) => scope,
            Err(reason) => {
                crate::pwarn!(
                    log_cfg,
                    crate::logging::LogCat::Damage,
                    "GM direct effect scope on {} no longer resolves ({:?})",
                    effect.target,
                    reason
                );
                continue;
            }
        };
        let amount = milli_to_hp(effect.amount_milli_hp);
        let is_asteroid = asteroid.is_some();
        let (applied, destroyed) = crate::sim_rng::with_event_generator(
            sim_rng.as_deref(),
            crate::sim_rng::GM_DIRECT_EFFECT_EVENT,
            effect.order.sequence,
            |rng| match effect.kind {
                GmDirectEffectKind::Damage => {
                    // Straight to `apply_hull_damage_within`: no shield split
                    // and no pierce fraction. "Direct/internal" is exactly the
                    // absence of that ROUTING, not a new formula — so the
                    // applied amount still mirrors into the per-arc pool (issue
                    // #514) the way every other hull-damage source mirrors it,
                    // from inside this one keyed generator so both draws stay
                    // on it. The arc pool tracks TOTAL hull damage and has no
                    // Station map of its own, so a narrowed scope mirrors the
                    // same way a whole-hull hit does.
                    let result = crate::ship::damage::apply_hull_damage_within(
                        &mut hull.0,
                        scope.as_deref(),
                        amount,
                        rng,
                    );
                    if let Some(arc_hull) = arc_hull.as_mut() {
                        arc_hull.0.apply_damage(result.0, rng);
                    }
                    result
                }
                // No arc mirror: see this system's doc comment — arc hull has
                // no distributed restore, and no repair path the crew can reach
                // refills it either.
                GmDirectEffectKind::Heal => (
                    hull.0
                        .restore_distributed_within(scope.as_deref(), amount, rng),
                    false,
                ),
            },
        );
        let uuid = uuid.0.clone();

        if effect.kind == GmDirectEffectKind::Damage {
            crate::pinfo!(
                log_cfg,
                crate::logging::LogCat::Damage,
                entity = entity,
                "took {:.1} direct hull damage from a GM action",
                applied
            );
            if let Some(msgs) = balance_events.as_mut() {
                msgs.write(crate::core::balance::BalanceEvent::DamageApplied {
                    // No attacker: a GM is not a combatant, and inventing one
                    // would put a phantom row in every balance ledger. The
                    // attribution lives on the GM action result, which is where
                    // "who did this" belongs.
                    attacker: None,
                    victim: uuid.clone(),
                    victim_kind: if is_asteroid {
                        crate::core::balance::VictimKind::Asteroid
                    } else {
                        crate::core::balance::VictimKind::Ship
                    },
                    weapon: WEAPON_KIND_GM_DIRECT.to_string(),
                    amount,
                    shield_absorbed: 0.0,
                    hull_damage: applied,
                    system_hit: None,
                });
            }
            // The crew's own hit feedback, presentation only and never folded:
            // the haptic pulse and the forcefield audio spike both hang off
            // this message, so without it a GM hit on the crew's own ship would
            // be the one hull hit in the game that lands in silence. `shield`
            // is zero because a direct effect bypasses shields entirely. Same
            // shape and same `is_local` gate as the damage zone's.
            if is_local {
                if let Some(ob) = outbox.as_mut() {
                    ob.push_reliable((
                        Target::All,
                        ServerMessage::DamageTaken {
                            hull: applied,
                            shield: 0.0,
                        },
                    ));
                }
            }
        } else {
            crate::pinfo!(
                log_cfg,
                crate::logging::LogCat::Damage,
                entity = entity,
                "restored {:.1} hull from a GM action",
                applied
            );
        }

        if !destroyed {
            continue;
        }
        if is_fleet_ship {
            if let Some(ob) = outbox.as_mut() {
                ob.push_reliable((Target::All, ServerMessage::ShipDestroyed));
            }
            if let Some(reason) = game_over_reason.as_mut() {
                if reason.0.is_none() {
                    // Player-visible, so a `strings.csv` id rather than English
                    // — the same id every other ship-death site latches.
                    reason.0 = Some("server.game_over.ship_destroyed".into());
                    reason.1 = Some(crate::core::balance::Outcome::Defeat);
                    if let Some(msgs) = balance_events.as_mut() {
                        msgs.write(crate::core::balance::BalanceEvent::EntityDestroyed {
                            victim: uuid.clone(),
                            killer: None,
                        });
                    }
                }
            }
            if let Some(ns) = next_state.as_mut() {
                ns.set(GamePhase::GameOver);
            }
            continue;
        }
        // Non-crewed target: despawn and report, exactly as the zone and beam
        // kill paths do. A crewed hull is never despawned — the run ends and
        // the report reads from the wreck.
        if let Some(world) = world.as_mut() {
            world.0.entities.retain(|entity| entity.uuid != uuid);
        }
        if is_asteroid {
            if let Some(ob) = outbox.as_mut() {
                ob.push_reliable((
                    Target::All,
                    ServerMessage::AsteroidDestroyed { uuid: uuid.clone() },
                ));
            }
        } else {
            if let Some(msgs) = destroyed_events.as_mut() {
                msgs.write(crate::ai::server::AiEntityDestroyed {
                    entity_uuid: uuid.clone(),
                });
            }
            if let Some(ob) = outbox.as_mut() {
                ob.push_reliable((
                    Target::All,
                    ServerMessage::EntityDespawned { uuid: uuid.clone() },
                ));
            }
        }
        // Forget the uuid so the reconcile sweep does not re-emit
        // `EntityDespawned` (issue #838, single-emission invariant).
        if let Some(tracked) = tracked.as_mut() {
            tracked.forget(&uuid);
        }
        if let Some(msgs) = balance_events.as_mut() {
            msgs.write(crate::core::balance::BalanceEvent::EntityDestroyed {
                victim: uuid.clone(),
                killer: None,
            });
        }
        commands.entity(entity).try_despawn();
    }
}

/// Clear the direct-effect arm at a new fleet/run boundary.
///
/// Called from [`crate::gm_action::reset`] for the reason the armed Fire set is
/// cleared there: an arm is a GM command's pending effect and belongs to the
/// run whose grant authorised it, never to the next one.
pub fn reset(world: &mut World) {
    world.insert_resource(PendingGmDirectEffects::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::SystemId;
    use crate::ship::damage::SystemHull;

    fn hull(systems: &[(&str, f32)]) -> SystemHull {
        SystemHull::from_config(
            &systems
                .iter()
                .map(|(id, max)| (SystemId((*id).into()), *max))
                .collect::<Vec<_>>(),
        )
    }

    /// A ship config whose `[[system]]` rows carry the authored Station
    /// ownership a narrowed scope resolves against (issue #1311).
    ///
    /// Parsed from TOML rather than built from struct literals so the
    /// ownership under test is the SAME field a shipped hull authors, read the
    /// same way — a hand-built `SystemInstanceConfig` would let this test pass
    /// while `[[system]] station = "..."` meant something else.
    fn ship_config(systems: &[(&str, Option<&str>)]) -> crate::ship::config::ShipConfig {
        let mut toml = String::new();
        let mut stations: Vec<&str> = systems.iter().filter_map(|(_, station)| *station).collect();
        stations.sort_unstable();
        stations.dedup();
        for station in stations {
            toml.push_str(&format!(
                r#"
[[station]]
id = "{station}"
name = "station.{station}.display_name"
description = "station.{station}.description"
rank = "Lieutenant"
"#
            ));
        }
        for (system, station) in systems {
            toml.push_str(&format!(
                r#"
[[system]]
id = "{system}"
kind = "{system}"
"#
            ));
            if let Some(station) = station {
                toml.push_str(&format!("station = \"{station}\"\n"));
            }
        }
        toml::from_str(&toml).expect("a well-formed authoring fixture")
    }

    /// Every answer a scope can give about which Systems it names, in one
    /// place — because this is the function BOTH the reducer and the damage
    /// phase call, and a disagreement between them is exactly what "Station
    /// scope never affects a System outside its authored ownership" forbids.
    #[test]
    fn a_scope_names_only_the_systems_its_authoring_gives_it() {
        let target = hull(&[
            ("impulse-drive", 60.0),
            ("phaser-bank", 40.0),
            ("core", 30.0),
        ]);
        let config = ship_config(&[
            ("impulse-drive", Some("helm")),
            ("phaser-bank", Some("tactical")),
            // Authored under no Station at all, the courier's ownerless bucket.
            ("core", None),
            // Authored but NOT tracked by the hull — every Alliance radar is
            // one. It must not widen `helm`'s scope by being owned by it.
            ("nav-radar", Some("helm")),
        ]);

        assert_eq!(
            scope_systems(&GmDirectEffectScope::Entity, &target, Some(&config)),
            Ok(None),
            "the whole hull is the absence of a restriction, not a list of everything"
        );
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::Station(crate::core::messages::StationId("helm".into())),
                &target,
                Some(&config)
            ),
            Ok(Some(vec![SystemId("impulse-drive".into())])),
            "an authored System the hull does not track is not a damageable one"
        );
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::System(SystemId("phaser-bank".into())),
                &target,
                Some(&config)
            ),
            Ok(Some(vec![SystemId("phaser-bank".into())])),
        );
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::System(SystemId("core".into())),
                &target,
                Some(&config)
            ),
            Ok(Some(vec![SystemId("core".into())])),
            "an ownerless System is still a legitimate System-scope target"
        );
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::Station(crate::core::messages::StationId("science".into())),
                &target,
                Some(&config)
            ),
            Err(GmDirectEffectScopeError::UnknownStation),
        );
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::System(SystemId("nav-radar".into())),
                &target,
                Some(&config)
            ),
            Err(GmDirectEffectScopeError::UnknownSystem),
            "authored but undamageable and absent are the same honest answer"
        );
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::Station(crate::core::messages::StationId("helm".into())),
                &target,
                None
            ),
            Err(GmDirectEffectScopeError::UnknownStation),
            "a target with no ship config authors no Station, so it has none"
        );
        assert_eq!(
            scope_systems(&GmDirectEffectScope::Entity, &target, None),
            Ok(None),
            "...and the whole hull never needed one"
        );

        // A Station that owns only systems this hull does not track is the
        // SCOPED spelling of an undamageable target, not an unknown Station.
        let radar_only = hull(&[("impulse-drive", 60.0)]);
        assert_eq!(
            scope_systems(
                &GmDirectEffectScope::Station(crate::core::messages::StationId("science".into())),
                &radar_only,
                Some(&ship_config(&[
                    ("impulse-drive", Some("helm")),
                    ("nav-radar", Some("science")),
                ]))
            ),
            Err(GmDirectEffectScopeError::NotDamageable),
        );
    }

    /// The clamp is the scope's and the lethality is the hull's, which is the
    /// one thing a narrowed effect must not conflate: emptying a Station is
    /// not sinking a ship (issue #1311).
    #[test]
    fn a_scoped_resolution_clamps_to_its_scope_but_kills_only_the_whole_hull() {
        // 25 asked of a Station holding 20, on a hull holding 60.
        let station =
            resolve_direct_effect_within(GmDirectEffectKind::Damage, 25_000, 20.0, 60.0, 60.0);
        assert_eq!(station.applied_milli_hp, 20_000, "clamped by the SCOPE");
        assert_eq!(station.discarded_milli_hp, 5_000);
        assert!(
            !station.destroyed,
            "40 points elsewhere on the hull are still alive"
        );

        // The same press when that Station holds every point the hull has left.
        let last =
            resolve_direct_effect_within(GmDirectEffectKind::Damage, 25_000, 20.0, 60.0, 20.0);
        assert_eq!(last.applied_milli_hp, 20_000);
        assert!(
            last.destroyed,
            "a scoped hit that empties the last living Systems IS the kill"
        );

        // Healing is never lethal whatever the totals say.
        assert!(
            !resolve_direct_effect_within(GmDirectEffectKind::Heal, 25_000, 0.0, 60.0, 0.0)
                .destroyed
        );

        // The whole-hull path is exactly the scoped one with one total.
        for (requested, current, max) in [
            (25_000u32, 100.0f32, 100.0f32),
            (250_000, 40.0, 100.0),
            (1, 0.0, 100.0),
        ] {
            assert_eq!(
                resolve_direct_effect(GmDirectEffectKind::Damage, requested, current, max),
                resolve_direct_effect_within(
                    GmDirectEffectKind::Damage,
                    requested,
                    current,
                    max,
                    current
                ),
                "`Entity` scope must have no second spelling"
            );
        }
    }

    /// A scoped total measures the scope, not the hull it sits in — which is
    /// what makes the clamp, the overflow figure and the lethality preview
    /// answer the question the operator actually asked.
    #[test]
    fn scope_totals_measure_the_scope_rather_than_the_hull() {
        let mut target = hull(&[("impulse-drive", 60.0), ("phaser-bank", 40.0)]);
        target.set_hp(&SystemId("impulse-drive".into()), 20.0);
        assert_eq!(scope_totals(&target, None), (60.0, 100.0));
        assert_eq!(
            scope_totals(&target, Some(&[SystemId("impulse-drive".into())])),
            (20.0, 60.0)
        );
        assert_eq!(
            scope_totals(&target, Some(&[SystemId("phaser-bank".into())])),
            (40.0, 40.0)
        );
    }

    #[test]
    fn damage_within_the_hull_is_applied_whole_and_discards_nothing() {
        let result = resolve_direct_effect(GmDirectEffectKind::Damage, 25_000, 100.0, 100.0);
        assert_eq!(
            result,
            GmDirectEffectResult {
                kind: GmDirectEffectKind::Damage,
                applied_milli_hp: 25_000,
                discarded_milli_hp: 0,
                destroyed: false,
            }
        );
    }

    #[test]
    fn damage_beyond_the_remaining_hull_is_lethal_and_reports_the_overflow() {
        let result = resolve_direct_effect(GmDirectEffectKind::Damage, 250_000, 100.0, 100.0);
        assert_eq!(
            result,
            GmDirectEffectResult {
                kind: GmDirectEffectKind::Damage,
                applied_milli_hp: 100_000,
                discarded_milli_hp: 150_000,
                destroyed: true,
            }
        );
    }

    #[test]
    fn damage_exactly_equal_to_the_remaining_hull_is_lethal_with_no_overflow() {
        let result = resolve_direct_effect(GmDirectEffectKind::Damage, 100_000, 100.0, 100.0);
        assert!(result.destroyed);
        assert_eq!(result.discarded_milli_hp, 0);
    }

    #[test]
    fn healing_clamps_at_the_maxima_and_reports_the_discarded_remainder() {
        let mut target = hull(&[("helm", 100.0), ("power", 100.0)]);
        target.set_hp(&SystemId("helm".into()), 40.0);
        let result = resolve_direct_effect(
            GmDirectEffectKind::Heal,
            250_000,
            target.total_current(),
            target.total_max(),
        );
        assert_eq!(
            result,
            GmDirectEffectResult {
                kind: GmDirectEffectKind::Heal,
                applied_milli_hp: 60_000,
                discarded_milli_hp: 190_000,
                destroyed: false,
            }
        );
    }

    #[test]
    fn healing_an_undamaged_hull_applies_nothing_and_discards_everything() {
        let result = resolve_direct_effect(GmDirectEffectKind::Heal, 10_000, 100.0, 100.0);
        assert_eq!(result.applied_milli_hp, 0);
        assert_eq!(result.discarded_milli_hp, 10_000);
        assert!(!result.destroyed);
    }

    #[test]
    fn damaging_an_already_destroyed_hull_applies_nothing_and_is_not_a_second_kill() {
        let result = resolve_direct_effect(GmDirectEffectKind::Damage, 5_000, 0.0, 100.0);
        assert_eq!(result.applied_milli_hp, 0);
        assert_eq!(result.discarded_milli_hp, 5_000);
        assert!(!result.destroyed);
    }

    /// A hull with no systems at all has no totals to work with; the reducer
    /// refuses such a target outright, and the resolution agrees.
    #[test]
    fn a_hull_with_no_systems_absorbs_nothing_at_all() {
        let empty = SystemHull::default();
        let result = resolve_direct_effect(
            GmDirectEffectKind::Damage,
            1_000,
            empty.total_current(),
            empty.total_max(),
        );
        assert_eq!(result.applied_milli_hp, 0);
        assert_eq!(result.discarded_milli_hp, 1_000);
    }

    /// A hull holding a fractional remainder still keeps the promise: a result
    /// that says `destroyed` resolves an amount that actually empties it, and
    /// the damage phase agrees.
    #[test]
    fn a_lethal_resolution_covers_a_fractional_remainder() {
        let result = resolve_direct_effect(GmDirectEffectKind::Damage, 40_000, 30.0004, 100.0);
        assert!(result.destroyed);
        assert_eq!(result.applied_milli_hp, 30_001);
        assert!(
            milli_to_hp(result.applied_milli_hp) >= 30.0004,
            "the clamp rounds UP so nothing is left behind a lethal promise"
        );

        let mut target = hull(&[("helm", 100.0)]);
        target.set_hp(&SystemId("helm".into()), 30.0004);
        let (_, destroyed) = crate::ship::damage::apply_hull_damage(
            &mut target,
            milli_to_hp(result.applied_milli_hp),
            &mut crate::sim_rng::unseeded_test_rng(),
        );
        assert!(destroyed);
    }

    /// The projection a reducer walks forward: what one effect leaves for the
    /// next grant on the same target in the same canonical drain.
    #[test]
    fn totals_after_an_effect_are_what_the_next_grant_measures_against() {
        let hit = resolve_direct_effect(GmDirectEffectKind::Damage, 60_000, 100.0, 100.0);
        assert_eq!(totals_after(&hit, 100.0, 100.0), (40.0, 100.0));
        let heal = resolve_direct_effect(GmDirectEffectKind::Heal, 60_000, 40.0, 100.0);
        assert_eq!(totals_after(&heal, 40.0, 100.0), (100.0, 100.0));
    }

    /// The reducer can run twice before the damage phase does — a paused
    /// session is exactly that — so a second press must measure against what
    /// the arms already waiting will take.
    #[test]
    fn already_armed_effects_are_part_of_what_the_next_press_measures_against() {
        let mut pending = PendingGmDirectEffects::default();
        pending.push(PendingGmDirectEffect {
            tick: 1,
            order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
            target: "npc-1".into(),
            scope: GmDirectEffectScope::Entity,
            kind: GmDirectEffectKind::Damage,
            amount_milli_hp: 60_000,
        });
        let target = hull(&[("helm", 100.0)]);
        assert_eq!(
            pending.project_totals(
                "npc-1",
                None,
                &target,
                None,
                GmDirectEffectKind::Damage,
                100.0,
                100.0
            ),
            (40.0, 100.0)
        );
        assert_eq!(
            pending.project_totals(
                "npc-1",
                None,
                &target,
                None,
                GmDirectEffectKind::Heal,
                100.0,
                100.0
            ),
            (40.0, 100.0),
            "a whole-hull arm is CONTAINED by a whole-hull press, so it is exact \
             for a heal press too — the kind only suppresses arms wider than \
             the scope being measured"
        );
        assert_eq!(
            pending.project_totals(
                "npc-2",
                None,
                &target,
                None,
                GmDirectEffectKind::Damage,
                100.0,
                100.0
            ),
            (100.0, 100.0),
            "another hull's queue is not this one's"
        );
    }

    #[test]
    fn the_queue_drains_due_effects_in_canonical_order_and_retains_the_future() {
        let effect = |tick: u64, sequence: u64| PendingGmDirectEffect {
            tick,
            order: GmActionOrder::new(crate::command_admission::HostSlot(1), sequence),
            target: format!("uuid-{sequence}"),
            scope: GmDirectEffectScope::Entity,
            kind: GmDirectEffectKind::Damage,
            amount_milli_hp: 1_000,
        };
        let mut pending = PendingGmDirectEffects::default();
        pending.push(effect(4, 3));
        pending.push(effect(2, 2));
        pending.push(effect(9, 1));

        let due = pending.take_due(4);
        assert_eq!(
            due.iter().map(|e| e.order.sequence).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(pending.entries().len(), 1);
        assert_eq!(pending.entries()[0].order.sequence, 1);
    }

    #[test]
    fn milli_hp_round_trips_a_typed_amount() {
        assert_eq!(hp_to_milli(25.0), 25_000);
        assert!((milli_to_hp(25_000) - 25.0).abs() < f32::EPSILON);
        assert_eq!(hp_to_milli(-4.0), 0);
        assert_eq!(hp_to_milli(f32::NAN), 0);
    }

    // -- The ordinary damage-phase applier -----------------------------------

    fn damage_app(tick: u64, seed: u64) -> App {
        let mut app = App::new();
        app.insert_resource(crate::sim_tick::SimTick(tick))
            .insert_resource(crate::sim_rng::SimRng::new(
                seed,
                crate::sim_rng::SeedSource::Cli,
            ))
            .init_resource::<PendingGmDirectEffects>()
            .init_resource::<crate::server_app::GameOverReason>()
            .init_resource::<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>()
            .init_resource::<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>()
            .add_systems(Update, apply_gm_direct_effects);
        app
    }

    fn spawn_hull(app: &mut App, uuid: &str, systems: &[(&str, f32)], fleet: bool) -> Entity {
        let hull = crate::entities::spawner::EntitySystemHull(hull(systems));
        let mut entity = app
            .world_mut()
            .spawn((crate::entities::spawner::EntityUuid(uuid.into()), hull));
        if fleet {
            entity.insert(crate::lockstep::FleetSlotOf(
                crate::command_admission::HostSlot(1),
            ));
        }
        entity.id()
    }

    /// [`spawn_hull`] plus the `ShipConfigComponent` a narrowed scope resolves
    /// its Station ownership against (issue #1311).
    fn spawn_stationed_hull(
        app: &mut App,
        uuid: &str,
        systems: &[(&str, f32, Option<&str>)],
    ) -> Entity {
        let entity = spawn_hull(
            app,
            uuid,
            &systems
                .iter()
                .map(|(id, max, _)| (*id, *max))
                .collect::<Vec<_>>(),
            false,
        );
        let config = ship_config(
            &systems
                .iter()
                .map(|(id, _, station)| (*id, *station))
                .collect::<Vec<_>>(),
        );
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::ship::components::ShipConfigComponent(config));
        entity
    }

    /// Attach the per-arc hull pool a crewed hull's `[[shield_arc]]` blocks
    /// give it (issue #514) — `alliance_courier.toml` declares two.
    fn attach_arc_hull(app: &mut App, entity: Entity, arcs: &[(&str, f32)]) {
        let arc_hull = crate::ship::damage::ShipArcHull::from_entries(
            arcs.iter()
                .map(|(id, max)| {
                    (
                        (*id).to_string(),
                        crate::ship::damage::ArcHullEntry {
                            current: *max,
                            max: *max,
                            tier_config: crate::ship::damage::ConsoleTierConfig::default(),
                        },
                    )
                })
                .collect(),
        );
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::entities::spawner::EntityShipArcHull(arc_hull));
    }

    fn arc_total(app: &App, entity: Entity) -> f32 {
        app.world()
            .entity(entity)
            .get::<crate::entities::spawner::EntityShipArcHull>()
            .expect("a live arc pool")
            .0
            .iter()
            .map(|(_, entry)| entry.current)
            .sum()
    }

    fn arm(
        app: &mut App,
        sequence: u64,
        tick: u64,
        target: &str,
        kind: GmDirectEffectKind,
        amount_milli_hp: u32,
    ) {
        arm_scoped(
            app,
            sequence,
            tick,
            target,
            GmDirectEffectScope::Entity,
            kind,
            amount_milli_hp,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn arm_scoped(
        app: &mut App,
        sequence: u64,
        tick: u64,
        target: &str,
        scope: GmDirectEffectScope,
        kind: GmDirectEffectKind,
        amount_milli_hp: u32,
    ) {
        app.world_mut()
            .resource_mut::<PendingGmDirectEffects>()
            .push(PendingGmDirectEffect {
                tick,
                order: GmActionOrder::new(crate::command_admission::HostSlot(1), sequence),
                target: target.into(),
                scope,
                kind,
                amount_milli_hp,
            });
    }

    fn station(id: &str) -> GmDirectEffectScope {
        GmDirectEffectScope::Station(crate::core::messages::StationId(id.into()))
    }

    fn system(id: &str) -> GmDirectEffectScope {
        GmDirectEffectScope::System(SystemId(id.into()))
    }

    fn total_current(app: &App, entity: Entity) -> f32 {
        app.world()
            .entity(entity)
            .get::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull")
            .0
            .total_current()
    }

    /// Pre-damage one System so a scoped repair has somewhere to go.
    fn set_system_hp(app: &mut App, entity: Entity, id: &str, hp: f32) {
        let mut target = app.world_mut().entity_mut(entity);
        let mut hull = target
            .get_mut::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull");
        hull.0.set_hp(&SystemId(id.into()), hp);
    }

    /// The live HP of one System, which is how a scoped test states "this one
    /// moved and that one did not" without inferring it from a total.
    fn system_current(app: &App, entity: Entity, id: &str) -> f32 {
        app.world()
            .entity(entity)
            .get::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull")
            .0
            .get(&SystemId(id.into()))
            .expect("a tracked system")
            .current
    }

    fn balance_events(app: &mut App) -> Vec<crate::core::balance::BalanceEvent> {
        let messages = app
            .world_mut()
            .resource_mut::<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>();
        messages.iter_current_update_messages().cloned().collect()
    }

    /// A direct hit lands the resolved amount, bypasses nothing but shields
    /// (there are none here to bypass), and reports through the SAME structured
    /// balance event a beam hit reports through -- which is what makes the
    /// crew-facing damage row appear with no extra plumbing.
    #[test]
    fn a_direct_hit_lands_the_armed_amount_and_reports_an_ordinary_damage_event() {
        let mut app = damage_app(4, 77);
        let target = spawn_hull(&mut app, "npc-1", &[("helm", 60.0), ("power", 40.0)], false);
        arm(&mut app, 1, 4, "npc-1", GmDirectEffectKind::Damage, 25_000);
        app.update();

        assert!((total_current(&app, target) - 75.0).abs() < 0.01);
        let events = balance_events(&mut app);
        assert!(events.iter().any(|event| matches!(
            event,
            crate::core::balance::BalanceEvent::DamageApplied {
                attacker: None,
                weapon,
                hull_damage,
                shield_absorbed,
                ..
            } if weapon == WEAPON_KIND_GM_DIRECT
                && (*hull_damage - 25.0).abs() < 0.01
                && *shield_absorbed == 0.0
        )));
        assert!(app.world().resource::<PendingGmDirectEffects>().is_empty());
    }

    /// The acceptance criterion itself, measured at the DAMAGE PHASE rather
    /// than at the reducer: a Station-scoped hit larger than everything that
    /// Station owns empties exactly those Systems and leaves every sibling
    /// untouched — no weighted selection, no float-precision fallback and no
    /// spill can reach one (issue #1311).
    ///
    /// The overrun is deliberate. A hit sized to fit would pass even if the
    /// allow-list were consulted only for the FIRST draw; only an amount the
    /// scope cannot absorb exercises the spill loop, which is the step that
    /// would otherwise walk into a sibling.
    #[test]
    fn a_station_scoped_hit_empties_its_own_systems_and_never_reaches_a_sibling() {
        let mut app = damage_app(4, 77);
        let target = spawn_stationed_hull(
            &mut app,
            "npc-1",
            &[
                ("impulse-drive", 40.0, Some("helm")),
                ("manoeuvre-thrusters", 20.0, Some("helm")),
                ("phaser-bank", 40.0, Some("tactical")),
                ("core", 30.0, None),
            ],
        );
        arm_scoped(
            &mut app,
            1,
            4,
            "npc-1",
            station("helm"),
            GmDirectEffectKind::Damage,
            250_000,
        );
        app.update();

        assert_eq!(system_current(&app, target, "impulse-drive"), 0.0);
        assert_eq!(system_current(&app, target, "manoeuvre-thrusters"), 0.0);
        assert_eq!(
            system_current(&app, target, "phaser-bank"),
            40.0,
            "another Station's System is invisible to a Station-scoped walk"
        );
        assert_eq!(
            system_current(&app, target, "core"),
            30.0,
            "and so is a System no Station owns"
        );
        let events = balance_events(&mut app);
        assert!(
            events.iter().any(|event| matches!(
                event,
                crate::core::balance::BalanceEvent::DamageApplied { weapon, hull_damage, .. }
                    if weapon == WEAPON_KIND_GM_DIRECT && (*hull_damage - 60.0).abs() < 0.01
            )),
            "the ordinary structured damage event still reports what LANDED"
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                crate::core::balance::BalanceEvent::EntityDestroyed { .. }
            )),
            "70 hull points are still alive, so the entity is not destroyed"
        );
    }

    /// System scope never spills to a sibling — the same rule with a scope of
    /// exactly one, where "spill" would be the whole rest of the hull.
    #[test]
    fn a_system_scoped_hit_stops_at_its_own_system() {
        let mut app = damage_app(4, 4242);
        let target = spawn_stationed_hull(
            &mut app,
            "npc-1",
            &[
                ("impulse-drive", 40.0, Some("helm")),
                ("manoeuvre-thrusters", 20.0, Some("helm")),
                ("phaser-bank", 40.0, Some("tactical")),
            ],
        );
        arm_scoped(
            &mut app,
            1,
            4,
            "npc-1",
            system("impulse-drive"),
            GmDirectEffectKind::Damage,
            100_000,
        );
        app.update();

        assert_eq!(system_current(&app, target, "impulse-drive"), 0.0);
        assert_eq!(
            system_current(&app, target, "manoeuvre-thrusters"),
            20.0,
            "a sibling on the SAME Station is still outside a System scope"
        );
        assert_eq!(system_current(&app, target, "phaser-bank"), 40.0);
        assert!((total_current(&app, target) - 60.0).abs() < 0.01);
    }

    /// Healing mirrors scope: a Station repair fills its own Systems and
    /// cannot quietly refill another Station's, and it still clamps at each
    /// maximum rather than overfilling one to spend the amount.
    #[test]
    fn a_station_scoped_repair_mirrors_the_scope_and_clamps_at_the_maxima() {
        let mut app = damage_app(4, 99);
        let target = spawn_stationed_hull(
            &mut app,
            "npc-1",
            &[
                ("impulse-drive", 40.0, Some("helm")),
                ("manoeuvre-thrusters", 20.0, Some("helm")),
                ("phaser-bank", 40.0, Some("tactical")),
            ],
        );
        set_system_hp(&mut app, target, "impulse-drive", 0.0);
        set_system_hp(&mut app, target, "manoeuvre-thrusters", 5.0);
        set_system_hp(&mut app, target, "phaser-bank", 1.0);
        arm_scoped(
            &mut app,
            1,
            4,
            "npc-1",
            station("helm"),
            GmDirectEffectKind::Heal,
            250_000,
        );
        app.update();

        assert_eq!(
            system_current(&app, target, "impulse-drive"),
            40.0,
            "a System at zero has the largest headroom and is revived first"
        );
        assert_eq!(system_current(&app, target, "manoeuvre-thrusters"), 20.0);
        assert_eq!(
            system_current(&app, target, "phaser-bank"),
            1.0,
            "another Station's wreck is not repaired by this Station's repair"
        );
    }

    /// A Station-scoped hit that empties the LAST living Systems still ends the
    /// entity: destruction is a fact about the whole hull, which is why the
    /// scope narrows the distribution and not the question.
    #[test]
    fn a_scoped_hit_that_empties_the_last_living_systems_still_destroys_the_entity() {
        let mut app = damage_app(4, 7);
        let target = spawn_stationed_hull(
            &mut app,
            "npc-1",
            &[
                ("impulse-drive", 40.0, Some("helm")),
                ("phaser-bank", 40.0, Some("tactical")),
            ],
        );
        set_system_hp(&mut app, target, "phaser-bank", 0.0);
        arm_scoped(
            &mut app,
            1,
            4,
            "npc-1",
            station("helm"),
            GmDirectEffectKind::Damage,
            40_000,
        );
        app.update();

        assert!(
            balance_events(&mut app).iter().any(|event| matches!(
                event,
                crate::core::balance::BalanceEvent::EntityDestroyed { victim, .. }
                    if victim == "npc-1"
            )),
            "the ordinary destruction path still runs for a scoped killing blow"
        );
    }

    /// An arm whose scope this hull can no longer resolve — its ship config
    /// replaced between the apply tick and the damage phase — is dropped
    /// exactly as a vanished target is, rather than silently widening back to
    /// the whole hull.
    #[test]
    fn an_arm_whose_scope_no_longer_resolves_is_dropped_rather_than_widened() {
        let mut app = damage_app(4, 11);
        let target = spawn_stationed_hull(
            &mut app,
            "npc-1",
            &[
                ("impulse-drive", 40.0, Some("helm")),
                ("phaser-bank", 40.0, Some("tactical")),
            ],
        );
        arm_scoped(
            &mut app,
            1,
            4,
            "npc-1",
            station("science"),
            GmDirectEffectKind::Damage,
            40_000,
        );
        app.update();

        assert!((total_current(&app, target) - 80.0).abs() < 0.01);
        assert!(app.world().resource::<PendingGmDirectEffects>().is_empty());
    }

    /// Two GMs pressing overlapping scopes on one boundary. The narrower press
    /// measures against what the wider arm will take, bounded at zero, so its
    /// durable result can never promise more than the hull will honour — and a
    /// press on a DISJOINT scope is unaffected by either.
    ///
    /// Round-1 review (#1311): an arm WIDER than the press only lands an
    /// unknown fraction inside it, so it may be projected only in the direction
    /// that shrinks the press's headroom. Same kind shrinks it; the OPPOSITE
    /// kind would grow it, on points the applier may never deliver, so it is
    /// skipped — the two mixed-kind cases at the end of this test.
    #[test]
    fn an_overlapping_arm_is_what_the_next_scoped_press_measures_against() {
        let target = hull(&[
            ("impulse-drive", 40.0),
            ("manoeuvre-thrusters", 20.0),
            ("phaser-bank", 40.0),
        ]);
        let config = ship_config(&[
            ("impulse-drive", Some("helm")),
            ("manoeuvre-thrusters", Some("helm")),
            ("phaser-bank", Some("tactical")),
        ]);
        let helm = scope_systems(&station("helm"), &target, Some(&config)).unwrap();
        let drive = scope_systems(&system("impulse-drive"), &target, Some(&config)).unwrap();
        let phaser = scope_systems(&system("phaser-bank"), &target, Some(&config)).unwrap();

        let mut pending = PendingGmDirectEffects::default();
        pending.push(PendingGmDirectEffect {
            tick: 1,
            order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
            target: "npc-1".into(),
            scope: station("helm"),
            kind: GmDirectEffectKind::Damage,
            amount_milli_hp: 50_000,
        });

        assert_eq!(
            pending.project_totals(
                "npc-1",
                helm.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Damage,
                60.0,
                60.0
            ),
            (10.0, 60.0),
            "the same scope is exact: 50 of this Station's 60 points are spoken for"
        );
        assert_eq!(
            pending.project_totals(
                "npc-1",
                drive.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Damage,
                40.0,
                40.0
            ),
            (0.0, 40.0),
            "a scope INSIDE the arm cannot know which of its Systems absorbs it, \
             so it takes the conservative answer and never over-promises"
        );
        assert_eq!(
            pending.project_totals(
                "npc-1",
                phaser.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Damage,
                40.0,
                40.0
            ),
            (40.0, 40.0),
            "a DISJOINT scope is untouched by the arm — this is the whole point \
             of resolving overlap rather than subtracting every queued effect"
        );
        assert_eq!(
            pending.project_totals(
                "npc-1",
                None,
                &target,
                Some(&config),
                GmDirectEffectKind::Damage,
                100.0,
                100.0
            ),
            (50.0, 100.0),
            "the whole hull overlaps everything, exactly as it did before scopes"
        );

        // A WIDER arm of the OPPOSITE kind (#1311 round-1 review). Only the
        // damage phase decides how much of a whole-hull effect lands inside one
        // Station, so projecting it whole here would move the narrow press's
        // headroom the UNSAFE way — up — and the durable result would promise
        // hull points the world never lands.
        let mut healed = PendingGmDirectEffects::default();
        healed.push(PendingGmDirectEffect {
            tick: 1,
            order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
            target: "npc-1".into(),
            scope: GmDirectEffectScope::Entity,
            kind: GmDirectEffectKind::Heal,
            amount_milli_hp: 40_000,
        });
        assert_eq!(
            healed.project_totals(
                "npc-1",
                helm.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Damage,
                10.0,
                60.0
            ),
            (10.0, 60.0),
            "a whole-hull HEAL arm must not raise a Station DAMAGE press's \
             projected current: the heal may land every one of its 40 points \
             outside this Station, and a press clamped to 50 would report an \
             amount the helm systems cannot absorb"
        );
        assert_eq!(
            healed.project_totals(
                "npc-1",
                helm.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Heal,
                10.0,
                60.0
            ),
            (50.0, 60.0),
            "the same wider arm against a HEAL press shrinks the headroom, \
             which is the safe direction, so it still counts whole"
        );

        let mut hit = PendingGmDirectEffects::default();
        hit.push(PendingGmDirectEffect {
            tick: 1,
            order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
            target: "npc-1".into(),
            scope: GmDirectEffectScope::Entity,
            kind: GmDirectEffectKind::Damage,
            amount_milli_hp: 40_000,
        });
        assert_eq!(
            hit.project_totals(
                "npc-1",
                helm.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Heal,
                10.0,
                60.0
            ),
            (10.0, 60.0),
            "a whole-hull DAMAGE arm must not grow a Station HEAL press's \
             headroom: the damage may land entirely on another Station, and \
             the healed points would then be discarded by the hull"
        );
        assert_eq!(
            hit.project_totals(
                "npc-1",
                helm.as_deref(),
                &target,
                Some(&config),
                GmDirectEffectKind::Damage,
                10.0,
                60.0
            ),
            (0.0, 60.0),
            "against a DAMAGE press the same arm shrinks the headroom, so it \
             counts whole and bounds at zero"
        );
    }

    /// The per-arc hull pool (issue #514) follows a GM hit exactly as it
    /// follows a beam, a torpedo, a collision and a damage zone.
    ///
    /// `EntityShipArcHull` tracks TOTAL hull damage taken, it is authoritative
    /// snapshot state, and `sync_console_damage_tiers` derives the
    /// `shield-arc-<id>` offline entries from it — so a GM hit that moved
    /// `EntitySystemHull` alone would leave the arc tiers stuck at whatever
    /// they read before the hit. Both pools move by the SAME applied amount.
    #[test]
    fn a_direct_hit_moves_the_per_arc_hull_pool_by_the_same_amount() {
        let mut app = damage_app(4, 4242);
        let target = spawn_hull(
            &mut app,
            "player-1",
            &[("helm", 60.0), ("power", 40.0)],
            true,
        );
        attach_arc_hull(&mut app, target, &[("fore", 50.0), ("aft", 50.0)]);
        arm(
            &mut app,
            1,
            4,
            "player-1",
            GmDirectEffectKind::Damage,
            25_000,
        );
        app.update();

        assert!((total_current(&app, target) - 75.0).abs() < 0.01);
        assert!(
            (arc_total(&app, target) - 75.0).abs() < 0.01,
            "the arc pool absorbs the same 25 points the system hull did"
        );
    }

    /// Healing does NOT touch the arc pool, and that is deliberate: no repair
    /// path the crew can reach restores arc hull, and `ShipArcHull` has no
    /// distributed restore at all. GM healing behaves like every other repair
    /// rather than inventing a distribution of its own.
    #[test]
    fn a_direct_heal_leaves_the_per_arc_hull_pool_where_the_damage_left_it() {
        let mut app = damage_app(4, 7);
        let target = spawn_hull(
            &mut app,
            "player-1",
            &[("helm", 60.0), ("power", 40.0)],
            true,
        );
        attach_arc_hull(&mut app, target, &[("fore", 50.0), ("aft", 50.0)]);
        arm(
            &mut app,
            1,
            4,
            "player-1",
            GmDirectEffectKind::Damage,
            25_000,
        );
        app.update();
        arm(&mut app, 2, 4, "player-1", GmDirectEffectKind::Heal, 25_000);
        app.update();

        assert!((total_current(&app, target) - 100.0).abs() < 0.01);
        assert!(
            (arc_total(&app, target) - 75.0).abs() < 0.01,
            "arc hull only ever refills the way the ordinary repair path refills it"
        );
    }

    /// The crew feel a GM hit on their own hull the way they feel every other
    /// hit: `DamageTaken` on the outbox drives the haptic pulse and the
    /// forcefield audio spike. Presentation only, never folded — the gate is
    /// the ONE `LocalShip` read in this system.
    #[test]
    fn a_direct_hit_on_the_local_hull_reports_ordinary_crew_hit_feedback() {
        let mut app = damage_app(4, 31);
        app.init_resource::<crate::server_app::SimOutbox>();
        let target = spawn_hull(
            &mut app,
            "player-1",
            &[("helm", 60.0), ("power", 40.0)],
            true,
        );
        app.world_mut()
            .entity_mut(target)
            .insert(crate::server_app::LocalShip);
        arm(
            &mut app,
            1,
            4,
            "player-1",
            GmDirectEffectKind::Damage,
            25_000,
        );
        app.update();

        let outbox = app.world().resource::<crate::server_app::SimOutbox>();
        assert!(
            outbox.iter().any(|(_, message)| matches!(
                message,
                ServerMessage::DamageTaken { hull, shield }
                    if (*hull - 25.0).abs() < 0.01 && *shield == 0.0
            )),
            "a direct hit bypasses shields, so the crew feedback is all hull"
        );
    }

    /// A hull that is not this host's own crew's stays silent on the outbox.
    ///
    /// The GM peer has no `LocalShip` at all, so this is also the shape every
    /// GM peer sees for every target: the fold is identical on both, and only
    /// the crew host emits the pulse.
    #[test]
    fn a_direct_hit_on_another_hull_reports_no_crew_hit_feedback() {
        let mut app = damage_app(4, 31);
        app.init_resource::<crate::server_app::SimOutbox>();
        spawn_hull(&mut app, "npc-1", &[("helm", 60.0), ("power", 40.0)], false);
        arm(&mut app, 1, 4, "npc-1", GmDirectEffectKind::Damage, 25_000);
        app.update();

        let outbox = app.world().resource::<crate::server_app::SimOutbox>();
        assert!(
            !outbox
                .iter()
                .any(|(_, message)| matches!(message, ServerMessage::DamageTaken { .. })),
            "another hull's damage is not this crew's hit feedback"
        );
    }

    /// The distribution is the ordinary weighted one: the same seed and the
    /// same canonical order land on the same systems on every peer, while the
    /// seed genuinely moves WHICH system absorbs the hit. The TOTAL never
    /// moves — that is resolved before the draw, which is why the durable
    /// result can promise it.
    #[test]
    fn the_distribution_is_seeded_but_the_total_is_not() {
        let spread = |seed: u64, sequence: u64| {
            let mut app = damage_app(1, seed);
            let target = spawn_hull(
                &mut app,
                "npc-1",
                &[("helm", 50.0), ("power", 50.0), ("shields", 50.0)],
                false,
            );
            arm(
                &mut app,
                sequence,
                1,
                "npc-1",
                GmDirectEffectKind::Damage,
                30_000,
            );
            app.update();
            app.world()
                .entity(target)
                .get::<crate::entities::spawner::EntitySystemHull>()
                .expect("a live hull")
                .0
                .entries()
                .map(|(_, current, _)| current)
                .collect::<Vec<_>>()
        };
        assert_eq!(spread(1234, 1), spread(1234, 1));

        let shapes: Vec<Vec<f32>> = (0..8u64).map(|seed| spread(seed, 1)).collect();
        for shape in &shapes {
            let total: f32 = shape.iter().sum();
            assert!(
                (total - 120.0).abs() < 0.01,
                "the resolved amount lands whole however it is distributed"
            );
        }
        assert!(
            shapes.iter().any(|shape| shape != &shapes[0]),
            "which system absorbs a GM hit is a function of the run seed"
        );
    }

    /// The "NPC scalar path" is the ordinary path applied to a hull of one.
    ///
    /// `entities::spawner` turns a legacy scalar `hull_integrity` into a
    /// single-entry `SystemHull` at spawn, so there is no second formula to
    /// write and no second branch to take: the weighted walk has exactly one
    /// candidate, and every hull point lands on it.
    #[test]
    fn a_single_entry_hull_takes_the_whole_hit_on_its_one_system() {
        let mut app = damage_app(2, 909);
        let target = spawn_hull(&mut app, "npc-1", &[("captain", 40.0)], false);
        arm(&mut app, 1, 2, "npc-1", GmDirectEffectKind::Damage, 15_000);
        app.update();

        let hull = app
            .world()
            .entity(target)
            .get::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull");
        assert!((hull.0.current_for(&SystemId("captain".into())).unwrap() - 25.0).abs() < 0.01);
        assert!(!hull.0.is_destroyed());
    }

    /// Healing is the mirror: it fills, clamps at the maxima, and can bring a
    /// destroyed system back -- the one route out of `Destroyed` there is.
    #[test]
    fn a_direct_heal_refills_the_hull_and_revives_a_destroyed_system() {
        let mut app = damage_app(2, 5);
        let target = spawn_hull(&mut app, "npc-1", &[("helm", 40.0), ("power", 60.0)], false);
        app.world_mut()
            .entity_mut(target)
            .get_mut::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull")
            .0
            .set_hp(&SystemId("helm".into()), 0.0);
        arm(&mut app, 1, 2, "npc-1", GmDirectEffectKind::Heal, 40_000);
        app.update();

        let hull = app
            .world()
            .entity(target)
            .get::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull");
        assert!((hull.0.total_current() - 100.0).abs() < 0.01);
        assert_eq!(
            hull.0.tier_for(&SystemId("helm".into())),
            crate::ship::damage::DamageTier::Operational
        );
    }

    /// A non-crewed kill follows the ordinary NPC destruction path: despawn,
    /// `AiEntityDestroyed`, `EntityDespawned` and the structured destruction
    /// event a balance ledger and the GM feed both already read.
    #[test]
    fn a_lethal_direct_hit_despawns_an_npc_through_the_ordinary_destruction_path() {
        let mut app = damage_app(3, 11);
        app.init_resource::<crate::server_app::SimOutbox>();
        app.init_resource::<crate::server_app::TrackedEntities>();
        let target = spawn_hull(&mut app, "npc-1", &[("helm", 20.0)], false);
        arm(&mut app, 1, 3, "npc-1", GmDirectEffectKind::Damage, 20_000);
        app.update();

        assert!(app.world().get_entity(target).is_err(), "the NPC despawned");
        let events = balance_events(&mut app);
        assert!(events.iter().any(|event| matches!(
            event,
            crate::core::balance::BalanceEvent::EntityDestroyed { victim, killer: None }
                if victim == "npc-1"
        )));
        assert_eq!(
            app.world_mut()
                .resource_mut::<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>(
                )
                .iter_current_update_messages()
                .map(|event| event.entity_uuid.clone())
                .collect::<Vec<_>>(),
            vec!["npc-1".to_string()]
        );
    }

    /// A crewed hull is NEVER despawned: the run ends and the report reads from
    /// the wreck. Keyed on fleet membership, never `LocalShip` -- the GM peer
    /// that pressed the button has no local ship at all.
    #[test]
    fn a_lethal_direct_hit_on_a_fleet_hull_ends_the_run_without_despawning_it() {
        let mut app = damage_app(6, 3);
        let target = spawn_hull(&mut app, "player-1", &[("helm", 30.0)], true);
        arm(
            &mut app,
            1,
            6,
            "player-1",
            GmDirectEffectKind::Damage,
            30_000,
        );
        app.update();

        assert!(app.world().get_entity(target).is_ok(), "the wreck remains");
        let reason = app.world().resource::<crate::server_app::GameOverReason>();
        assert_eq!(reason.0.as_deref(), Some("server.game_over.ship_destroyed"));
        assert_eq!(reason.1, Some(crate::core::balance::Outcome::Defeat));
    }

    /// An arm whose target left the world between the apply boundary and the
    /// damage phase is dropped, not retried and not panicked over.
    #[test]
    fn an_arm_whose_target_vanished_is_dropped() {
        let mut app = damage_app(8, 2);
        arm(&mut app, 1, 8, "gone", GmDirectEffectKind::Damage, 1_000);
        app.update();
        assert!(app.world().resource::<PendingGmDirectEffects>().is_empty());
    }

    /// A future boundary is not this tick's business.
    #[test]
    fn an_effect_armed_for_a_later_tick_is_retained() {
        let mut app = damage_app(1, 2);
        let target = spawn_hull(&mut app, "npc-1", &[("helm", 50.0)], false);
        arm(&mut app, 1, 9, "npc-1", GmDirectEffectKind::Damage, 10_000);
        app.update();
        assert!((total_current(&app, target) - 50.0).abs() < f32::EPSILON);
        assert_eq!(
            app.world()
                .resource::<PendingGmDirectEffects>()
                .entries()
                .len(),
            1
        );
    }
}
