//! The Bevy adapter for helm docking (issue #1159).
//!
//! Gathers the live world into the plain values the pure sibling
//! [`crate::dock::mating`] takes, applies what comes back, and drives the own
//! ship onto the mated pose — the per-ship [`DockControl`] component, the
//! per-hull [`DockMarkers`] resolved from the rig sidecar at spawn, the
//! fixed-tick systems that take the dock/undock commands, decide whether the
//! dock holds, fly the own ship onto its mate, and publish the blackboard.
//! Nothing here decides geometry or eligibility itself: rule 10, the split the
//! tractor keeps between `coupling` and `server`.
//!
//! # It moves the OWN ship only
//!
//! Unlike the tractor's `move_coupled_target`, which writes the TARGET's
//! position, docking writes only the OWN ship's `ShipPhysics`/`Transform` — the
//! ship the crew are flying slides onto the berth; the structure it mates with is
//! never touched. So the tractor-review coordination note (a target moved by two
//! couplers at once) does not apply to the target. The one hull that could be
//! written by both is a docking ship that is also under someone's tractor; to
//! keep that deterministic this system is ordered `after` the tractor rig, so the
//! dock's own-ship placement is the last writer that tick. A docker holds at most
//! one target — `docking_target` is a single `Option` — which the umbilical (#1160)
//! reads; berth-side occupancy (one docker per target) is not enforced here.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::damage::DamageTier;
use crate::dock::mating::{nearest_viable_pair, DockConfig, DockMarker, DockRefusal, Pose};
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::messages::{
    DockBlackboard, PowerGroupId, SystemAffinity, SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::ship::power::{power_level_for, ShipPowerSystem};
use crate::ship::state::ShipPhysics;
use crate::system_registry::{dock_system_id, DOCK_SYSTEM_ID};
use crate::world::server::WorldContentRuntime;

/// One hull's dock markers, resolved into the hull's OWN frame at spawn (issue
/// #1159).
///
/// Attached to every entity whose `[dock]` table opted it into docking AND whose
/// rig sidecar declared `dock`-prefixed `[markers.<name>]` blocks. The base rig
/// is folded in once here, so each marker is a ship-local point a live
/// `Transform` maps to world — the same composition
/// [`crate::model_rig::ModelMarkers::resolve_world_position`] performs, done
/// once at spawn so the fixed tick never re-reads the sidecar. A hull that
/// declares no dock markers carries no `DockMarkers` and can never be docked
/// with.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct DockMarkers {
    /// The dock markers in the hull's own frame, in a stable authored order
    /// (sorted by name at spawn) so nearest-pair indices are deterministic.
    pub markers: Vec<DockMarker>,
}

impl DockMarkers {
    /// Whether this hull declares any dock markers. A hull with none can never
    /// be docked with.
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }
}

/// One ship's active dock control (issue #1159): the authored dock terms, the
/// power group it draws from, and the live engage/dock state.
///
/// Inserted at spawn only on a hull that authored a `[dock]` table AND a
/// `kind = "dock"` `[[system]]`. A hull with neither carries no component and is
/// byte-identical to one built before this existed (AGENTS.md rule 11). A hull
/// that authors a `[dock]` table but no dock system is DOCKABLE (it carries
/// [`DockMarkers`]) but not an active docker — it can be mated with, it just has
/// no control of its own.
///
/// `engaged` is the operator's INTENT to hold a dock; `docked` is whether the
/// mate has actually formed. `docking_target` is the hull the manoeuvre is
/// closing on or holding, latched at `Dock` time. On any interruption `engaged`,
/// `docked` and `docking_target` clear together — the crew watch the dock end.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct DockControl {
    /// The authored dock terms — range, engage distance, approach speed, mate
    /// tolerance, undock clearance, minimum power level.
    pub config: DockConfig,
    /// The power group the dock `[[system]]` declared, resolved once at spawn.
    pub power_group: PowerGroupId,
    /// The operator's standing intent to hold a dock. Set true by `Dock`, false
    /// by `Undock` and by every interruption.
    pub engaged: bool,
    /// Whether the two hulls are actually mated this tick. Only ever true while
    /// `engaged` and `docking_target` is set.
    pub docked: bool,
    /// The hull the manoeuvre is closing on or holding, `Some(target-uuid)` while
    /// engaged, `None` when idle or undocking.
    pub docking_target: Option<String>,
    /// Where the ship is backing out to during an undock, `Some(clear-pose)`
    /// until it arrives and returns to ordinary flight. Not saved and not folded
    /// — a transient the next tick recomputes the effect of.
    pub undock_target: Option<Vec3>,
    /// The nearest dockable hull within the authored range this tick, resolved
    /// for the console so the contextual control can appear. A projection, not
    /// saved.
    pub available_target: Option<String>,
    /// Why the last dock could not form or was ended — the reason the console
    /// shows, retained until the operator acts again. `None` when idle, forming
    /// cleanly, or docked.
    pub last_refusal: Option<DockRefusal>,
}

impl DockControl {
    /// A fresh, idle dock control carrying its authored terms and resolved power
    /// group.
    pub fn new(config: DockConfig, power_group: PowerGroupId) -> Self {
        Self {
            config,
            power_group,
            engaged: false,
            docked: false,
            docking_target: None,
            undock_target: None,
            available_target: None,
            last_refusal: None,
        }
    }

    /// The uuid of the hull this ship is DOCKED to, or `None` when it is not
    /// docked. This is the relationship the umbilical (#1160) gates on: two hulls
    /// are docked when one's `docked_partner()` names the other.
    pub fn docked_partner(&self) -> Option<&str> {
        if self.docked {
            self.docking_target.as_deref()
        } else {
            None
        }
    }

    /// The persistable half — the engage/dock state and the docked target — for
    /// the snapshot payload. The authored config and power group ride the
    /// template and are re-derived on spawn, exactly as the tractor leaves its
    /// coupling terms out of `TractorSaveState`.
    pub fn save_state(&self) -> DockSaveState {
        DockSaveState {
            engaged: self.engaged,
            docked: self.docked,
            docking_target: self.docking_target.clone(),
        }
    }

    /// Reseed the engage/dock state and docked target from a restored snapshot,
    /// onto a control that already carries its authored config and resolved power
    /// group from the fresh spawn.
    ///
    /// The last refusal and the undock target are deliberately NOT restored: both
    /// are projections the next `tick_dock` re-derives from the resumed world.
    pub fn restore(&mut self, save: &DockSaveState) {
        self.engaged = save.engaged;
        self.docked = save.docked;
        self.docking_target = save.docking_target.clone();
        self.undock_target = None;
        self.available_target = None;
        self.last_refusal = None;
    }
}

/// The snapshot-carried half of a [`DockControl`] (issue #1159): the engage/dock
/// state and the docked target, and nothing else.
///
/// `Default` is the idle control — not engaged, not docked — which is what a hull
/// that authored a dock and never used it captures, so a resume of such a ship
/// restores byte-identically and folds the same number.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DockSaveState {
    #[serde(default)]
    pub engaged: bool,
    #[serde(default)]
    pub docked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docking_target: Option<String>,
}

/// Registers the dock systems and its admitted-command consumer (issue #1159).
/// Added by `WorldPlugin` alongside `TractorPlugin`.
pub struct DockPlugin;

impl Plugin for DockPlugin {
    fn build(&self, app: &mut App) {
        // The dock `[[system]]` is an admitted-command consumer: `handle_dock_
        // commands` reads `Dock` / `Undock` for it, so admission fans those
        // commands into every ship's `AdmittedCommands` each tick.
        // Gated AI decider (issue #1162); `register_ai_cadence` is idempotent.
        crate::ai::cadence::register_ai_cadence(app);
        app.register_admitted_consumer(ConsumerMatcher::exact(DOCK_SYSTEM_ID));
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Helm dock AI (issue #1162): on the shared AI cadence
                // (rule 7), emitting Dock / Undock BEFORE `handle_dock_commands`
                // consumes the tick.
                operate_dock_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_dock_commands),
                handle_dock_commands.in_set(crate::sim_sets::SimSet::Input),
                // Decide the dock and fly the own ship onto its mate, after the
                // tractor rig so a hull that is both a docking ship and a tractor
                // target has a deterministic last writer (see module docs).
                tick_dock
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::tractor::server::move_coupled_target),
                publish_dock_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── The dock / undock commands ───────────────────────────────────────────────

/// Runs in `SimSet::Input` and reads `AdmittedCommands` for the dock system
/// (issue #1159). It sets only intent; `tick_dock` decides whether a mate forms
/// and records any refusal.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak and stripped the source, so nothing here asks who sent the command
/// (AGENTS.md rule 6). `Dock` latches the nearest available target `tick_dock`
/// published last tick; `Undock` ends the mate and starts the backing manoeuvre.
pub fn handle_dock_commands(
    mut ships: Query<(
        &crate::messages::AdmittedCommands,
        &Transform,
        &mut DockControl,
    )>,
) {
    for (admitted, transform, mut dock) in ships.iter_mut() {
        for cmd in admitted.for_target(DOCK_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::Dock if !dock.engaged => {
                    match dock.available_target.clone() {
                        Some(target) => {
                            dock.engaged = true;
                            dock.docked = false;
                            dock.docking_target = Some(target);
                            dock.undock_target = None;
                            dock.last_refusal = None;
                        }
                        // Nothing dockable in range — refuse by name rather than
                        // engaging a manoeuvre with no berth to reach. A hull that
                        // declares no dock markers of its own publishes no
                        // available target either, and `tick_dock` surfaces the
                        // sharper "no markers" reason for it while idle.
                        None => dock.last_refusal = Some(DockRefusal::NoTarget),
                    }
                }
                SystemControlPayload::Undock if dock.engaged || dock.docked => {
                    // Back straight out along the ship's own heading, away from
                    // the berth, then return to ordinary flight. The clear pose is
                    // fixed now, while the ship still sits on the mate, so the
                    // manoeuvre needs no further reference to the target.
                    let back = transform.rotation * Vec3::new(0.0, 0.0, 1.0); // local +Z = astern
                    let clear = transform.translation + back * dock.config.undock_clear_distance;
                    dock.engaged = false;
                    dock.docked = false;
                    dock.docking_target = None;
                    dock.undock_target = Some(clear);
                    dock.last_refusal = None;
                }
                _ => {}
            }
        }
    }
}

/// Marks a ship the backfill dock host has DOCKED to serve a `Transfer`
/// directive (issue #1162). Inserted when the host engages the dock, removed when
/// it undocks; the host undocks only while it is present, so it never undocks a
/// dock a console engaged on the same AI-operated system. Not folded/snapshotted:
/// re-adopted from the still-present directive on resume.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct DockAiEngaged;

/// Resolve a directive's named target to the UUID a dock's `available_target`
/// carries (issue #1162): a world entity NAME through the runtime map, or the
/// value itself when it is already a UUID (or the runtime is absent).
fn resolve_dock_target(name: &str, runtime: Option<&WorldContentRuntime>) -> String {
    runtime
        .and_then(|rt| rt.name_to_uuid.get(name).cloned())
        .unwrap_or_else(|| name.to_string())
}

/// Backfill Helm dock AI (issue #1162).
///
/// On an active `Transfer` directive (Helm affinity) naming a target the dock
/// reports available in range, engage the dock; with no such directive active,
/// undock. The concrete command is exactly the `Dock`/`Undock` a human helmsman
/// emits, sent through the SAME `emit_ai_command` seam so `handle_dock_commands`
/// never learns who spoke (AGENTS.md rule 6). "When the procedure requires it":
/// the dock waits until `available_target` names the ordered hull (published by
/// `tick_dock` once it is in range), exactly as the contextual control only
/// appears for a human then. Decides ONLY on the shared AI cadence (rule 7).
#[allow(clippy::type_complexity)]
pub fn operate_dock_ai(
    mut commands: Commands,
    sessions: Res<crate::lobby::Sessions>,
    runtime: Option<Res<WorldContentRuntime>>,
    mut ships: Query<(
        Entity,
        Option<&EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        &DockControl,
        &crate::server_app::ShipSystemBlackboards,
        Has<DockAiEngaged>,
        &mut crate::messages::AdmittedCommands,
    )>,
) {
    for (entity, uuid, sources, config, dock, blackboards, host_engaged, mut admitted) in
        ships.iter_mut()
    {
        if !sources.0.policy_for(&dock_system_id()).operate_ai {
            continue;
        }
        let directive_target: Option<String> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(vbb)) => crate::objectives::top_operate_directive(
                &vbb.scored_objectives,
                SystemAffinity::Helm,
                |d| crate::objectives::transfer_directive_target(d).is_some(),
            )
            .and_then(crate::objectives::transfer_directive_target)
            .map(str::to_string),
            _ => None,
        };

        let payload = match directive_target {
            // A transfer order is active: dock once the ordered hull is available
            // in range. Idempotent — a dock already engaged emits nothing. Claim
            // the dock as host-driven whenever it is engaged under this order.
            Some(name) => {
                let resolved = resolve_dock_target(&name, runtime.as_deref());
                let available_is_target = dock
                    .available_target
                    .as_deref()
                    .is_some_and(|a| a == resolved);
                let emit =
                    (available_is_target && !dock.engaged).then_some(SystemControlPayload::Dock);
                if (emit.is_some() || dock.engaged || dock.docked) && !host_engaged {
                    commands.entity(entity).insert(DockAiEngaged);
                }
                emit
            }
            // No transfer order: undock a dock THIS HOST engaged — never one a
            // console set on the same AI-operated system (no marker → leave it).
            None => {
                if (dock.engaged || dock.docked) && host_engaged {
                    commands.entity(entity).remove::<DockAiEngaged>();
                    Some(SystemControlPayload::Undock)
                } else {
                    None
                }
            }
        };

        if let Some(payload) = payload {
            emit_ai_command(
                uuid,
                dock_system_id(),
                payload,
                sources,
                &sessions,
                config,
                &mut admitted,
            );
        }
    }
}

// ── The dock tick ────────────────────────────────────────────────────────────

/// A hull carrying dock markers, read once per tick into plain values so the
/// borrow of the world is released before the own-ship write.
struct Candidate {
    uuid: String,
    pose: Pose,
    markers: Vec<DockMarker>,
}

/// What `tick_dock` decides for one own ship, applied in the write phase.
struct DockOutcome {
    entity: Entity,
    engaged: bool,
    docked: bool,
    docking_target: Option<String>,
    undock_target: Option<Vec3>,
    available_target: Option<String>,
    last_refusal: Option<DockRefusal>,
    /// The pose to write onto the own ship this tick, when the manoeuvre moved
    /// it. `None` leaves the ship where its own flight put it.
    new_pose: Option<Pose>,
}

/// Decide every dock this tick and fly each engaged own ship onto (or off) its
/// mate (issue #1159).
///
/// Reads the own ship's dock markers, the candidate hulls' markers and poses, the
/// dock's power level and damage tier, hands the geometry to the pure
/// [`nearest_viable_pair`], and applies the verdict. The close-in motion is a
/// thin adapter over [`crate::ai::docking_close_manoeuvre`] — the same low-speed
/// mate the AI helm flies — reused for the direction, advanced at the authored
/// approach speed. Each interruption (drift past range, unpowered, disabled,
/// target lost) ends the dock cleanly.
#[allow(clippy::type_complexity)]
pub fn tick_dock(
    time: Res<Time>,
    mut set: ParamSet<(
        // Own-ship rows: everything the verdict needs off the docker itself.
        Query<(
            Entity,
            &DockControl,
            &Transform,
            Option<&DockMarkers>,
            Option<&ShipPowerSystem>,
            Option<&EntitySystemHull>,
            &EntityUuid,
        )>,
        // Every hull carrying dock markers, to resolve candidates and targets.
        Query<(&EntityUuid, &Transform, &DockMarkers)>,
        // Apply the verdict and any placement.
        Query<(&mut DockControl, &mut Transform, Option<&mut ShipPhysics>)>,
    )>,
) {
    let dt = time.delta_secs();

    // Every dockable hull, read once.
    let candidates: Vec<Candidate> = set
        .p1()
        .iter()
        .filter(|(_, _, m)| !m.is_empty())
        .map(|(uuid, transform, markers)| Candidate {
            uuid: uuid.0.clone(),
            pose: Pose {
                translation: transform.translation,
                rotation: transform.rotation,
            },
            markers: markers.markers.clone(),
        })
        .collect();

    // Decide each own ship without holding the write borrow.
    let mut outcomes: Vec<DockOutcome> = Vec::new();
    for (entity, dock, transform, markers, power, hull, uuid) in set.p0().iter() {
        let own_pose = Pose {
            translation: transform.translation,
            rotation: transform.rotation,
        };
        let own_markers: &[DockMarker] = markers.map(|m| m.markers.as_slice()).unwrap_or(&[]);
        let power_level = power
            .map(|p| power_level_for(&p.0, &dock.power_group))
            .unwrap_or(0);
        let disabled = hull
            .map(|h| {
                matches!(
                    h.0.tier_for(&dock_system_id()),
                    DamageTier::Disabled | DamageTier::Destroyed
                )
            })
            .unwrap_or(false);

        outcomes.push(decide_dock(
            entity,
            &uuid.0,
            dock,
            own_pose,
            own_markers,
            power_level,
            disabled,
            &candidates,
            dt,
        ));
    }

    // Apply.
    let mut writes = set.p2();
    for out in outcomes {
        let Ok((mut dock, mut transform, physics)) = writes.get_mut(out.entity) else {
            continue;
        };
        dock.engaged = out.engaged;
        dock.docked = out.docked;
        dock.docking_target = out.docking_target;
        dock.undock_target = out.undock_target;
        dock.available_target = out.available_target;
        dock.last_refusal = out.last_refusal;
        if let Some(pose) = out.new_pose {
            transform.translation = pose.translation;
            transform.rotation = pose.rotation;
            if let Some(mut physics) = physics {
                physics.x = pose.translation.x;
                physics.y = pose.translation.y;
                physics.z = pose.translation.z;
                let (yaw, _, _) = pose.rotation.to_euler(EulerRot::YXZ);
                physics.yaw = yaw;
                // A ship parked on (or backing off) a berth carries no residual
                // velocity — the same reason the tractor zeroes a released load.
                physics.forward_speed = 0.0;
                physics.lateral_speed = 0.0;
                physics.vertical_speed = 0.0;
            }
        }
    }
}

/// Decide one own ship's dock for this tick, and where (if anywhere) to place it.
///
/// Pulled out of the system so the state machine is one place with no world
/// borrow in scope. The pure [`nearest_viable_pair`] owns the geometry; this
/// orders the interruptions and steps the approach/undock motion.
#[allow(clippy::too_many_arguments)]
fn decide_dock(
    entity: Entity,
    own_uuid: &str,
    dock: &DockControl,
    own_pose: Pose,
    own_markers: &[DockMarker],
    power_level: u8,
    disabled: bool,
    candidates: &[Candidate],
    dt: f32,
) -> DockOutcome {
    let cfg = &dock.config;
    let idle = |available: Option<String>, refusal: Option<DockRefusal>| DockOutcome {
        entity,
        engaged: false,
        docked: false,
        docking_target: None,
        undock_target: None,
        available_target: available,
        last_refusal: refusal,
        new_pose: None,
    };

    // The nearest viable partner within range, ignoring self — recomputed every
    // tick so the console control appears and disappears with proximity.
    let nearest_available = || -> Option<String> {
        let mut best: Option<(String, f32)> = None;
        for c in candidates {
            if c.uuid == own_uuid {
                continue;
            }
            if let Some(sol) = nearest_viable_pair(own_pose, own_markers, c.pose, &c.markers) {
                if sol.separation <= cfg.range
                    && best
                        .as_ref()
                        .map(|(_, s)| sol.separation < *s)
                        .unwrap_or(true)
                {
                    best = Some((c.uuid.clone(), sol.separation));
                }
            }
        }
        best.map(|(u, _)| u)
    };

    // ── Undocking: back straight out, then return to ordinary flight ──────────
    if let Some(clear) = dock.undock_target {
        let (stepped, arrived) = step_toward(own_pose.translation, clear, cfg.approach_speed, dt);
        if arrived {
            // Clear of the berth; resume ordinary flight and re-offer the control
            // if still near a partner.
            return DockOutcome {
                entity,
                engaged: false,
                docked: false,
                docking_target: None,
                undock_target: None,
                available_target: nearest_available(),
                last_refusal: None,
                new_pose: Some(Pose {
                    translation: stepped,
                    rotation: own_pose.rotation,
                }),
            };
        }
        return DockOutcome {
            entity,
            engaged: false,
            docked: false,
            docking_target: None,
            undock_target: Some(clear),
            available_target: None,
            last_refusal: None,
            new_pose: Some(Pose {
                translation: stepped,
                rotation: own_pose.rotation,
            }),
        };
    }

    // ── Idle: publish availability, no motion ─────────────────────────────────
    if !dock.engaged {
        // A hull that declares no dock markers can never dock, and says so the
        // moment it is idle — the console shows the reason rather than a control
        // that grips nothing.
        if own_markers.is_empty() {
            return idle(None, dock.last_refusal.or(Some(DockRefusal::NoDockMarkers)));
        }
        return idle(nearest_available(), dock.last_refusal);
    }

    // ── Engaged: approach or hold the mate ────────────────────────────────────
    let end = |refusal: DockRefusal| idle(None, Some(refusal));

    // Hardware and power before target acquisition — most-actionable-first, the
    // tractor's check order.
    if disabled {
        return end(DockRefusal::Disabled);
    }
    if power_level < cfg.min_power_level {
        return end(DockRefusal::Unpowered);
    }
    if own_markers.is_empty() {
        return end(DockRefusal::NoDockMarkers);
    }
    let Some(target_uuid) = dock.docking_target.clone() else {
        return end(DockRefusal::NoTarget);
    };
    let Some(target) = candidates.iter().find(|c| c.uuid == target_uuid) else {
        // The berth is gone — destroyed or despawned mid-manoeuvre.
        return end(DockRefusal::TargetLost);
    };
    let Some(sol) = nearest_viable_pair(own_pose, own_markers, target.pose, &target.markers) else {
        return end(DockRefusal::NoDockMarkers);
    };
    if sol.separation > cfg.range {
        // Drifted apart — while forming or while held, the same reason.
        return end(DockRefusal::OutOfRange);
    }

    // Distance from the ship origin to the mate pose it must reach.
    let to_mate = sol.mate.translation - own_pose.translation;
    let mate_dist = Vec3::new(to_mate.x, 0.0, to_mate.z).length();

    if !dock.docked && mate_dist > cfg.mate_tolerance {
        // Still closing: the thin adapter over `docking_close_manoeuvre`. Ask it
        // for the ship-local close-in intent onto the mate point, rotate that
        // back to world, and advance the ship at the authored approach speed.
        let step_dir = close_in_world_dir(own_pose, sol.mate.translation, cfg.engage_distance)
            // Beyond the engage distance the manoeuvre is not yet live; close
            // straight in (the probe authors engage_distance >= range, so this
            // fallback only guards a hull whose markers sit far off-centre).
            .unwrap_or_else(|| {
                let d = Vec3::new(to_mate.x, 0.0, to_mate.z);
                d.normalize_or_zero()
            });
        let step_len = (cfg.approach_speed * dt).min(mate_dist);
        let stepped = own_pose.translation + step_dir * step_len;
        return DockOutcome {
            entity,
            engaged: true,
            docked: false,
            docking_target: Some(target_uuid),
            undock_target: None,
            available_target: Some(target.uuid.clone()),
            last_refusal: None,
            new_pose: Some(Pose {
                translation: stepped,
                rotation: own_pose.rotation,
            }),
        };
    }

    // Mated: snap onto the exact mate pose (following the berth if it drifts
    // inside range) and hold — the docked relationship between the two hulls.
    DockOutcome {
        entity,
        engaged: true,
        docked: true,
        docking_target: Some(target_uuid.clone()),
        undock_target: None,
        available_target: Some(target_uuid),
        last_refusal: None,
        new_pose: Some(sol.mate),
    }
}

/// Step a point toward a goal at `speed` units/second for `dt` seconds, bounded
/// so it never overshoots. Returns the new point and whether the goal was
/// reached this tick. Planar — the vertical is carried straight through.
fn step_toward(from: Vec3, to: Vec3, speed: f32, dt: f32) -> (Vec3, bool) {
    let delta = to - from;
    let planar = Vec3::new(delta.x, 0.0, delta.z);
    let dist = planar.length();
    let step = (speed * dt).max(0.0);
    if dist <= step || dist <= 1e-4 {
        return (Vec3::new(to.x, from.y, to.z), true);
    }
    (from + planar / dist * step, false)
}

/// The world-space unit direction the close-in docking manoeuvre would drive the
/// ship this tick, obtained from [`crate::ai::docking_close_manoeuvre`] and
/// rotated back out of the ship frame (issue #1159).
///
/// `docking_close_manoeuvre` returns the mate direction decomposed into the
/// ship's own `[starboard, aft]` axes; this reverses that decomposition —
/// `world = lateral·(cos,sin) + aft·(-sin,cos)` on `(x,z)` — so the caller
/// advances the ship along the very heading the AI helm's close-in manoeuvre
/// would. Returns `None` when the mate is beyond `engage_distance` (the manoeuvre
/// is not yet live) or coincident with the ship.
fn close_in_world_dir(own: Pose, mate: Vec3, engage_distance: f32) -> Option<Vec3> {
    let (yaw, _, _) = own.rotation.to_euler(EulerRot::YXZ);
    let [lateral, aft] = crate::ai::docking_close_manoeuvre(
        own.translation.x,
        own.translation.z,
        yaw,
        mate.x,
        mate.z,
        engage_distance,
        1.0,
    )?;
    let cos = crate::simmath::cos(yaw);
    let sin = crate::simmath::sin(yaw);
    let world_x = lateral * cos - aft * sin;
    let world_z = lateral * sin + aft * cos;
    let dir = Vec3::new(world_x, 0.0, world_z);
    (dir.length_squared() > 1e-8).then(|| dir.normalize())
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each dock-carrying ship's blackboard under its system id (issue
/// #1159).
///
/// Only ships that carry [`DockControl`] publish one, so a world whose hulls
/// author no `[dock]` system puts exactly the payload on the wire it did before
/// this existed. No English crosses: target names are world entity name ids and
/// the refusal is the pure module's `strings.csv` id.
pub fn publish_dock_blackboard(
    mut ships: Query<(&DockControl, &mut crate::server_app::ShipSystemBlackboards)>,
    named: Query<(&EntityUuid, &EntityName)>,
) {
    let key = dock_system_id();
    let name_of = |uuid: &Option<String>| -> Option<String> {
        uuid.as_ref().and_then(|u| {
            named
                .iter()
                .find(|(id, _)| &id.0 == u)
                .map(|(_, name)| name.0.clone())
        })
    };
    for (dock, mut blackboards) in ships.iter_mut() {
        let blackboard = SystemBlackboard::Dock(DockBlackboard {
            range: dock.config.range,
            available: dock.available_target.is_some() && !dock.docked,
            available_target: dock.available_target.clone(),
            available_target_name: name_of(&dock.available_target),
            engaged: dock.engaged,
            docked: dock.docked,
            docked_to: dock.docked_partner().map(|s| s.to_string()),
            docked_to_name: name_of(&dock.docked_partner().map(|s| s.to_string())),
            refusal: dock.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

/// The dock's published blackboard channel key — its system id (issue #1159).
pub fn dock_blackboard_key() -> SystemId {
    dock_system_id()
}

/// Resolve a hull's dock markers from its rig sidecar into its own frame (issue
/// #1159), for the spawn path to attach as a [`DockMarkers`] component.
///
/// A dock marker is any `[markers.<name>]` whose name begins with `dock` — the
/// same rig-marker vocabulary that already carries engines, cameras and weapon
/// hardpoints, identified by a reserved name prefix. The base rig is folded in
/// here so the stored markers are ship-local, and they are sorted by name so the
/// nearest-pair indices are stable across hosts. Returns an empty vec when the
/// sidecar declares no dock markers — the hull can then never be docked with.
pub fn resolve_dock_markers(rig: &crate::model_rig::ModelRig) -> DockMarkers {
    let base = rig.base_bevy_transform();
    let mut named: Vec<(&String, &crate::model_rig::Marker)> = rig
        .markers
        .iter()
        .filter(|(name, _)| name.starts_with(DOCK_MARKER_PREFIX))
        .collect();
    named.sort_by(|a, b| a.0.cmp(b.0));
    let markers = named
        .into_iter()
        .map(|(_, m)| {
            let position = base.transform_point(Vec3::from_array(m.position));
            let raw_dir = base.rotation * (base.scale * Vec3::from_array(m.direction));
            let direction = if raw_dir.length_squared() > 1e-8 {
                raw_dir.normalize()
            } else {
                raw_dir
            };
            DockMarker {
                position,
                direction,
            }
        })
        .collect();
    DockMarkers { markers }
}

/// The reserved rig-marker name prefix that flags a `[markers.<name>]` as a dock
/// marker (issue #1159). A naming convention over the shared marker vocabulary,
/// the way `phaser-<id>` and `shield-arc-<id>` are conventions over the system-id
/// vocabulary.
pub const DOCK_MARKER_PREFIX: &str = "dock";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::mating::DockConfig;

    fn config() -> DockConfig {
        DockConfig {
            range: 200.0,
            engage_distance: 400.0,
            approach_speed: 60.0,
            mate_tolerance: 4.0,
            undock_clear_distance: 120.0,
            min_power_level: 2,
        }
    }

    fn control() -> DockControl {
        DockControl::new(config(), PowerGroupId("dock".into()))
    }

    #[test]
    fn save_state_carries_engage_dock_and_target_only() {
        let mut c = control();
        c.engaged = true;
        c.docked = true;
        c.docking_target = Some("berth-1".into());
        c.last_refusal = Some(DockRefusal::OutOfRange);
        let save = c.save_state();
        assert!(save.engaged);
        assert!(save.docked);
        assert_eq!(save.docking_target.as_deref(), Some("berth-1"));
    }

    #[test]
    fn an_idle_control_saves_as_default() {
        assert_eq!(control().save_state(), DockSaveState::default());
    }

    #[test]
    fn docked_partner_is_only_the_target_while_docked() {
        let mut c = control();
        c.docking_target = Some("berth-1".into());
        c.engaged = true;
        assert_eq!(c.docked_partner(), None, "approaching is not yet docked");
        c.docked = true;
        assert_eq!(c.docked_partner(), Some("berth-1"));
    }

    #[test]
    fn resolve_dock_markers_takes_only_dock_prefixed_markers_in_order() {
        let rig = crate::model_rig::ModelRig::from_toml(
            r#"
            [markers.dock_fore]
            position = [0.0, 0.0, -5.0]
            direction = [0.0, 0.0, -1.0]
            [markers.engine_port]
            position = [-1.0, 0.0, 3.0]
            direction = [0.0, 0.0, 1.0]
            [markers.dock_aft]
            position = [0.0, 0.0, 5.0]
            direction = [0.0, 0.0, 1.0]
            "#,
        )
        .expect("rig parses");
        let dm = resolve_dock_markers(&rig);
        assert_eq!(dm.markers.len(), 2, "only the two dock_* markers");
        // Sorted by name: dock_aft (z=+5) then dock_fore (z=-5).
        assert!((dm.markers[0].position.z - 5.0).abs() < 1e-4);
        assert!((dm.markers[1].position.z + 5.0).abs() < 1e-4);
    }

    #[test]
    fn a_rig_with_no_dock_markers_resolves_empty() {
        let rig = crate::model_rig::ModelRig::from_toml(
            r#"
            [markers.engine_port]
            position = [-1.0, 0.0, 3.0]
            direction = [0.0, 0.0, 1.0]
            "#,
        )
        .expect("rig parses");
        assert!(resolve_dock_markers(&rig).is_empty());
    }
}
