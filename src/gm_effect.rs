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

#[cfg(test)]
use crate::sim_rng::InstallSimRng;
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
#[path = "gm_effect_tests.rs"]
mod tests;
