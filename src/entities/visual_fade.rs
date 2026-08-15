//! Fading a visual in or out, and the arrival flourish built on it.
//!
//! Two presentation problems with one mechanism behind them:
//!
//! * **An LOD tier change is a hard cut.** `update_mesh_lod` despawned the old
//!   tier's child and spawned the new one's in the same frame, so a hull
//!   crossing a switch distance changed silhouette between one frame and the
//!   next. The fix is a brief window where BOTH tiers are on screen, the
//!   outgoing one fading out while the incoming one fades in.
//! * **A mid-mission spawn pops.** Reinforcements arrive fully formed the frame
//!   their GLB finishes streaming, which is both a visual jolt and a lie about
//!   when they got there — the arrival is really the async asset resolving. The
//!   fix is the same fade, plus a scale-in, so the appearance reads as an event.
//!
//! Both are [`VisualFade`] on the visual's own root — the `SceneRoot` child of a
//! GLB level, the `Mesh3d` child of a procedural one, the root of a billboard —
//! never on the entity. An entity's transform is simulation state; a visual's
//! child transform is not, which is the same reason a procedural LOD level puts
//! its rotation on the child.
//!
//! # How a shared material is faded without fading everything else
//!
//! A GLB's materials are ASSETS, shared by every entity rendering that GLB: all
//! 32 rocks of a size class hold the same handles. Writing alpha into them would
//! fade the whole field. So the fade takes a per-visual COPY of each material it
//! touches ([`FadeMaterialSwap`]), drives alpha on the copy, and hands the
//! originals back when it finishes. A fade-out drops its copies with the entity;
//! a fade-in restores the shared handles, so nothing is left holding a clone.
//!
//! An opaque material's copy fades through `AlphaMode::AlphaToCoverage` rather
//! than `Blend`: coverage keeps the mesh in the opaque pass with depth writes
//! intact, so a half-faded hull does not show its own far side through itself.
//! Where MSAA is off, coverage degrades to a cutoff — the swap goes back to
//! looking like the hard cut it replaced, which is the right way for this to
//! fail. A material that was ALREADY translucent (a billboard's atlas quad)
//! keeps its own alpha mode.
//!
//! Presentation-only, and registered only under `SimPluginOptions::render`: a
//! headless run has no visuals to fade and never schedules any of this.

use bevy::prelude::*;

/// Marks a mesh whose alpha is written by something OTHER than the fade, so the
/// fade leaves it alone instead of fighting for the channel.
///
/// One writer per material, always. A far-LOD billboard's pose quads are the
/// case this exists for: they dissolve between two yaw captures through the same
/// alpha the fade would use, so their orienting system folds
/// [`VisualFade::alpha`] into the pose weights itself and carries this marker to
/// say so.
#[derive(Component, Debug, Clone, Copy)]
pub struct SelfDrivenAlpha;

/// Which way a [`VisualFade`] is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeDirection {
    /// Toward fully visible; the visual survives and its materials are handed
    /// back at the end.
    In,
    /// Toward fully invisible; the visual is despawned at the end.
    Out,
}

/// A visual mid-transition. Lives on the visual's root child.
#[derive(Component, Debug, Clone, Copy)]
pub struct VisualFade {
    /// Seconds elapsed into the window.
    pub elapsed: f32,
    /// Length of the window in seconds, as authored (`[render] lod_fade_secs`
    /// / `materialise_secs`). Copied onto the component at the moment the fade
    /// starts, so a config reload mid-fade cannot change a window already
    /// running.
    pub duration: f32,
    pub direction: FadeDirection,
    /// The fraction of full size the visual starts at, for an arrival that
    /// scales in as well as fading in. `None` — every cross-fade — leaves the
    /// transform alone, which matters: an LOD tier's scale is the thing
    /// `tier_parent_scale` exists to get right, and a fade must not be a second
    /// writer of it.
    pub scale_in_from: Option<f32>,
}

impl VisualFade {
    /// A cross-fade in, for the incoming tier of an LOD switch.
    pub fn fade_in(duration: f32) -> Self {
        Self {
            elapsed: 0.0,
            duration,
            direction: FadeDirection::In,
            scale_in_from: None,
        }
    }

    /// A cross-fade out, for the outgoing tier of an LOD switch. The visual is
    /// despawned when the window closes.
    pub fn fade_out(duration: f32) -> Self {
        Self {
            elapsed: 0.0,
            duration,
            direction: FadeDirection::Out,
            scale_in_from: None,
        }
    }

    /// An arrival: fade in from nothing while growing from `from` of full size.
    pub fn materialise(duration: f32, from: f32) -> Self {
        Self {
            elapsed: 0.0,
            duration,
            direction: FadeDirection::In,
            scale_in_from: Some(from.clamp(0.0, 1.0)),
        }
    }

    /// How far through the window, in `[0, 1]`. A non-positive duration is
    /// already over — that is how an authored `0` disables the effect without a
    /// second switch to read.
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }

    /// The alpha this fade wants its visual drawn at, in `[0, 1]`.
    pub fn alpha(&self) -> f32 {
        match self.direction {
            FadeDirection::In => self.progress(),
            FadeDirection::Out => 1.0 - self.progress(),
        }
    }

    /// The fraction of full size this fade wants its visual drawn at. `1.0`
    /// unless the fade is an arrival, which eases out of `scale_in_from` so the
    /// growth is quick at first and settles rather than arriving at speed.
    pub fn scale_factor(&self) -> f32 {
        match self.scale_in_from {
            None => 1.0,
            Some(from) => {
                let t = self.progress();
                let eased = 1.0 - (1.0 - t) * (1.0 - t);
                from + (1.0 - from) * eased
            }
        }
    }

    /// True once the window has closed.
    pub fn finished(&self) -> bool {
        self.progress() >= 1.0
    }
}

/// One mesh whose material this fade has taken a private copy of.
struct SwappedMaterial {
    mesh: Entity,
    /// The SHARED asset the mesh carried before the fade — handed back at the
    /// end of a fade-in, and never written to in between.
    original: Handle<StandardMaterial>,
    /// This fade's own copy, which is the only material it writes alpha into.
    ///
    /// Held here rather than re-read off the entity each frame, and that is not
    /// a convenience: `MeshMaterial3d` is inserted through a COMMAND, so on the
    /// frame a mesh is first swapped the entity still carries `original`.
    /// Reading the entity would write this fade's alpha straight into the
    /// shared asset — fading every other entity that draws with it.
    fading: Handle<StandardMaterial>,
}

/// The material copies a fading visual's meshes are drawing with, and the shared
/// assets they came from. Also the record of which descendants have already been
/// swapped: a GLB's `SceneRoot` populates its children over several frames, so
/// the walk repeats until the scene has finished arriving.
#[derive(Component, Default)]
pub struct FadeMaterialSwap(Vec<SwappedMaterial>);

/// The full local scale a materialising visual is growing toward — captured
/// once, at the first frame of the fade, because it is whatever the spawn put
/// there (a GLB child carries its rig's `[base].scale`, a billboard root its
/// quad's world width and height).
#[derive(Component)]
pub struct FadeTargetScale(Vec3);

/// Advance every running fade: alpha onto the visual's own material copies,
/// scale for an arrival, and the teardown or hand-back when the window closes.
///
/// `Without<SelfDrivenAlpha>` on the material walk is what keeps the one-writer
/// rule — see that marker.
pub fn drive_visual_fades(
    time: Res<Time>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    children: Query<&Children>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>, Without<SelfDrivenAlpha>>,
    mut fading: Query<(
        Entity,
        &mut VisualFade,
        &mut Transform,
        Option<&mut FadeMaterialSwap>,
        Option<&FadeTargetScale>,
    )>,
) {
    let dt = time.delta_secs();
    for (root, mut fade, mut transform, swap, target_scale) in fading.iter_mut() {
        fade.elapsed += dt;

        // Arrival scaling, against the size the spawn actually produced rather
        // than an assumed 1 — a GLB child carries its rig's base scale.
        if fade.scale_in_from.is_some() {
            let full = match target_scale {
                Some(FadeTargetScale(scale)) => *scale,
                None => {
                    let full = transform.scale;
                    commands.entity(root).insert(FadeTargetScale(full));
                    full
                }
            };
            transform.scale = full * fade.scale_factor();
        }

        // Take a private copy of every material this visual draws with, and
        // drive alpha on the copy. Repeated each frame: a scene's children
        // appear over several frames, and a late arrival must not stay opaque
        // through a fade that has already started.
        let alpha = fade.alpha();
        let mut swapped: Vec<SwappedMaterial> = swap
            .map(|s| std::mem::take(&mut s.into_inner().0))
            .unwrap_or_default();
        for mesh in visual_meshes(root, &children) {
            if swapped.iter().any(|s| s.mesh == mesh) {
                continue;
            }
            let Ok(handle) = mesh_materials.get(mesh) else {
                continue;
            };
            let Some(source) = materials.get(&handle.0).cloned() else {
                // The material asset has not loaded yet — try again next frame.
                continue;
            };
            let mut copy = source;
            copy.alpha_mode = fade_alpha_mode(copy.alpha_mode);
            let fading = materials.add(copy);
            commands.entity(mesh).insert(MeshMaterial3d(fading.clone()));
            swapped.push(SwappedMaterial {
                mesh,
                original: handle.0.clone(),
                fading,
            });
        }

        for swap in swapped.iter() {
            if let Some(mat) = materials.get_mut(&swap.fading) {
                mat.base_color = mat.base_color.with_alpha(alpha);
            }
        }

        if !fade.finished() {
            commands.entity(root).insert(FadeMaterialSwap(swapped));
            continue;
        }

        match fade.direction {
            // The window closed on an outgoing tier: it and its material copies
            // go together.
            FadeDirection::Out => commands.entity(root).try_despawn(),
            FadeDirection::In => {
                // Hand the shared assets back, drop the copies, and leave the
                // visual exactly as an un-faded spawn would have left it.
                for swap in swapped {
                    commands
                        .entity(swap.mesh)
                        .insert(MeshMaterial3d(swap.original));
                }
                if let Some(FadeTargetScale(full)) = target_scale {
                    transform.scale = *full;
                }
                commands
                    .entity(root)
                    .remove::<(VisualFade, FadeMaterialSwap, FadeTargetScale)>();
            }
        }
    }
}

/// The visual root and every descendant beneath it, so a `SceneRoot`'s whole
/// mesh tree is reached and not just its top entity.
fn visual_meshes(root: Entity, children: &Query<&Children>) -> Vec<Entity> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        if let Ok(kids) = children.get(out[i]) {
            out.extend(kids.iter());
        }
        i += 1;
    }
    out
}

/// The alpha mode a material's fade COPY draws in.
///
/// An opaque or masked material fades through coverage, which keeps it in the
/// opaque pass with depth writes — a half-faded hull that had switched to
/// `Blend` would show its own interior. Anything already translucent keeps the
/// mode it was authored with; a billboard's atlas needs its own `Blend` for the
/// transparent background the capture writes, and re-deciding that here would
/// throw it away.
fn fade_alpha_mode(original: AlphaMode) -> AlphaMode {
    match original {
        AlphaMode::Opaque | AlphaMode::Mask(_) => AlphaMode::AlphaToCoverage,
        other => other,
    }
}

/// Rescale a visual so it keeps the world size it had while its PARENT takes a
/// different one — what an outgoing LOD tier needs, because the tier scale
/// lives on the entity transform both tiers hang from and the incoming tier is
/// about to claim it.
///
/// Component-wise, and a degenerate incoming axis leaves that axis alone rather
/// than dividing into a non-finite scale.
pub fn parent_scale_correction(outgoing_parent: Vec3, incoming_parent: Vec3) -> Vec3 {
    let axis = |out: f32, inc: f32| if inc.abs() > 1e-6 { out / inc } else { 1.0 };
    Vec3::new(
        axis(outgoing_parent.x, incoming_parent.x),
        axis(outgoing_parent.y, incoming_parent.y),
        axis(outgoing_parent.z, incoming_parent.z),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_in_runs_from_invisible_to_visible() {
        let mut fade = VisualFade::fade_in(0.4);
        assert_eq!(fade.alpha(), 0.0);
        fade.elapsed = 0.2;
        assert!((fade.alpha() - 0.5).abs() < 1e-6);
        fade.elapsed = 0.4;
        assert_eq!(fade.alpha(), 1.0);
        assert!(fade.finished());
    }

    #[test]
    fn a_fade_out_runs_the_other_way() {
        let mut fade = VisualFade::fade_out(0.4);
        assert_eq!(fade.alpha(), 1.0);
        fade.elapsed = 0.4;
        assert_eq!(fade.alpha(), 0.0);
        assert!(fade.finished());
    }

    /// The two halves of a cross-fade must always sum to one unit of coverage,
    /// or the pair reads as a dip to dark (or a double-exposure) at the switch
    /// distance instead of a swap.
    #[test]
    fn the_two_halves_of_a_cross_fade_always_sum_to_one() {
        for step in 0..=10 {
            let elapsed = step as f32 * 0.03;
            let incoming = VisualFade {
                elapsed,
                ..VisualFade::fade_in(0.3)
            };
            let outgoing = VisualFade {
                elapsed,
                ..VisualFade::fade_out(0.3)
            };
            assert!(
                (incoming.alpha() + outgoing.alpha() - 1.0).abs() < 1e-5,
                "at {elapsed}s the pair covered {} of one visual",
                incoming.alpha() + outgoing.alpha()
            );
        }
    }

    /// An authored duration of zero is the off switch: the window is over
    /// before it starts, so the swap is the same-frame cut it always was.
    #[test]
    fn a_zero_duration_fade_is_already_finished() {
        let fade = VisualFade::fade_in(0.0);
        assert!(fade.finished());
        assert_eq!(fade.alpha(), 1.0);
        assert_eq!(VisualFade::fade_out(0.0).alpha(), 0.0);
    }

    /// Overrunning the window (a long frame) clamps rather than overshooting
    /// into a negative or above-one alpha.
    #[test]
    fn overrunning_the_window_clamps() {
        let fade = VisualFade {
            elapsed: 10.0,
            ..VisualFade::fade_out(0.2)
        };
        assert_eq!(fade.alpha(), 0.0);
        let fade = VisualFade {
            elapsed: 10.0,
            ..VisualFade::fade_in(0.2)
        };
        assert_eq!(fade.alpha(), 1.0);
    }

    /// An arrival starts small and lands at full size — and lands there
    /// exactly, so nothing is left permanently a hair off its authored scale.
    #[test]
    fn an_arrival_grows_from_its_start_fraction_to_full_size() {
        let mut fade = VisualFade::materialise(0.6, 0.25);
        assert!((fade.scale_factor() - 0.25).abs() < 1e-6);
        fade.elapsed = 0.6;
        assert!((fade.scale_factor() - 1.0).abs() < 1e-6);
    }

    /// The arrival easing settles rather than arriving at speed: past the
    /// half-way point it is already most of the way to full size.
    #[test]
    fn an_arrival_eases_out() {
        let fade = VisualFade {
            elapsed: 0.3,
            ..VisualFade::materialise(0.6, 0.0)
        };
        assert!(
            fade.scale_factor() > 0.5,
            "an eased arrival is past half size at half time, got {}",
            fade.scale_factor()
        );
    }

    /// A cross-fade must never touch the transform: the tier scale is what
    /// `tier_parent_scale` exists to get right and a second writer of it is the
    /// flash the LOD work has already had to fix once.
    #[test]
    fn a_cross_fade_leaves_scale_alone() {
        assert_eq!(VisualFade::fade_in(0.3).scale_factor(), 1.0);
        assert_eq!(VisualFade::fade_out(0.3).scale_factor(), 1.0);
        assert!(VisualFade::fade_in(0.3).scale_in_from.is_none());
    }

    /// Opaque geometry fades through coverage so it keeps depth writes; an
    /// already-translucent material keeps the mode it was authored with.
    #[test]
    fn opaque_materials_fade_through_coverage_and_translucent_ones_do_not_change() {
        assert_eq!(
            fade_alpha_mode(AlphaMode::Opaque),
            AlphaMode::AlphaToCoverage
        );
        assert_eq!(
            fade_alpha_mode(AlphaMode::Mask(0.5)),
            AlphaMode::AlphaToCoverage
        );
        assert_eq!(fade_alpha_mode(AlphaMode::Blend), AlphaMode::Blend);
        assert_eq!(fade_alpha_mode(AlphaMode::Add), AlphaMode::Add);
    }

    /// The outgoing tier holds the world size it had while the entity takes the
    /// incoming tier's scale. A hull ladder's near tier folds in nothing and its
    /// far tiers the whole `[base].scale` (0.75 on the destroyer), so an
    /// uncorrected outgoing near tier would visibly GROW as it faded.
    #[test]
    fn an_outgoing_tier_keeps_its_world_size_across_a_hull_ladder_switch() {
        let near = Vec3::ONE;
        let far = Vec3::splat(0.75);
        let correction = parent_scale_correction(near, far);
        assert!(
            (correction * far - near).length() < 1e-6,
            "the corrected child under the incoming parent scale must reach the \
             outgoing world size, got {:?}",
            correction * far
        );
    }

    /// The pipeline convention pulls the other way — the parent stays at 1 and
    /// the child carries the base scale — so the correction must be able to go
    /// both directions, not just shrink.
    #[test]
    fn the_correction_works_in_both_directions() {
        let a = Vec3::new(2.0, 4.0, 8.0);
        let b = Vec3::new(1.0, 1.0, 1.0);
        assert!((parent_scale_correction(a, b) * b - a).length() < 1e-6);
        assert!((parent_scale_correction(b, a) * a - b).length() < 1e-6);
    }

    /// A degenerate incoming scale carries no usable ratio; leave the outgoing
    /// visual as it is rather than produce a non-finite scale.
    #[test]
    fn a_degenerate_incoming_scale_corrects_by_nothing() {
        let got = parent_scale_correction(Vec3::splat(3.0), Vec3::ZERO);
        assert_eq!(got, Vec3::ONE);
        assert!(got.is_finite());
    }

    // ── The driver, over a real world ────────────────────────────────────

    mod driver {
        use super::*;
        use std::time::Duration;

        /// A world with the driver scheduled, a manual clock, and one SHARED
        /// material drawn by two entities — the situation a GLB's materials
        /// are actually in, where every rock of a size class holds the same
        /// handles.
        fn fixture() -> (App, Handle<StandardMaterial>, Entity, Entity) {
            let mut app = App::new();
            app.insert_resource(Time::<()>::default())
                .insert_resource(Assets::<StandardMaterial>::default())
                .add_systems(Update, drive_visual_fades);
            let shared = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color: Color::WHITE,
                    alpha_mode: AlphaMode::Opaque,
                    ..default()
                });
            let fading = app
                .world_mut()
                .spawn((MeshMaterial3d(shared.clone()), Transform::default()))
                .id();
            let bystander = app
                .world_mut()
                .spawn((MeshMaterial3d(shared.clone()), Transform::default()))
                .id();
            (app, shared, fading, bystander)
        }

        fn advance(app: &mut App, secs: f32) {
            app.world_mut()
                .resource_mut::<Time<()>>()
                .advance_by(Duration::from_secs_f32(secs));
            app.update();
        }

        fn alpha_of(app: &App, entity: Entity) -> f32 {
            let handle = app
                .world()
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .expect("the entity draws with something");
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(&handle.0)
                .expect("its material exists")
                .base_color
                .alpha()
        }

        /// The load-bearing claim of the whole mechanism: fading one visual
        /// must not fade every other entity that draws with the same GLB.
        #[test]
        fn fading_one_visual_leaves_everything_sharing_its_material_alone() {
            let (mut app, shared, fading, bystander) = fixture();
            app.world_mut()
                .entity_mut(fading)
                .insert(VisualFade::fade_out(0.4));

            advance(&mut app, 0.2);

            assert!(
                (alpha_of(&app, fading) - 0.5).abs() < 1e-4,
                "the fading visual is half-way out, got {}",
                alpha_of(&app, fading)
            );
            assert_eq!(
                alpha_of(&app, bystander),
                1.0,
                "an entity that merely shares the material must be untouched"
            );
            assert_eq!(
                app.world()
                    .resource::<Assets<StandardMaterial>>()
                    .get(&shared)
                    .unwrap()
                    .base_color
                    .alpha(),
                1.0,
                "the SHARED asset itself must never be written to"
            );
        }

        /// Opaque geometry fades through coverage on the copy — so a
        /// half-faded hull keeps its depth writes and does not show its own
        /// far side through itself.
        #[test]
        fn the_copy_fades_through_coverage_and_the_shared_asset_stays_opaque() {
            let (mut app, shared, fading, _) = fixture();
            app.world_mut()
                .entity_mut(fading)
                .insert(VisualFade::fade_out(0.4));
            advance(&mut app, 0.1);

            let handle = app
                .world()
                .get::<MeshMaterial3d<StandardMaterial>>(fading)
                .unwrap();
            assert_ne!(handle.0, shared, "the fade draws with its own copy");
            let assets = app.world().resource::<Assets<StandardMaterial>>();
            assert_eq!(
                assets.get(&handle.0).unwrap().alpha_mode,
                AlphaMode::AlphaToCoverage
            );
            assert_eq!(assets.get(&shared).unwrap().alpha_mode, AlphaMode::Opaque);
        }

        /// A fade-out ends in a despawn — that is how the outgoing LOD tier
        /// finally leaves, and nothing else despawns it.
        #[test]
        fn a_fade_out_despawns_its_visual_when_the_window_closes() {
            let (mut app, _, fading, _) = fixture();
            app.world_mut()
                .entity_mut(fading)
                .insert(VisualFade::fade_out(0.2));
            advance(&mut app, 0.1);
            assert!(app.world().get_entity(fading).is_ok());
            advance(&mut app, 0.2);
            assert!(
                app.world().get_entity(fading).is_err(),
                "the outgoing tier must not outlive its window"
            );
        }

        /// A fade-in hands the shared asset back and takes its own components
        /// off, so a visual that has arrived is indistinguishable from one that
        /// never faded — no copy left holding memory, no component left for a
        /// later system to trip over.
        #[test]
        fn a_fade_in_restores_the_shared_material_and_clears_itself() {
            let (mut app, shared, fading, _) = fixture();
            app.world_mut()
                .entity_mut(fading)
                .insert(VisualFade::fade_in(0.2));
            advance(&mut app, 0.1);
            assert!((alpha_of(&app, fading) - 0.5).abs() < 1e-4);

            advance(&mut app, 0.2);
            let handle = app
                .world()
                .get::<MeshMaterial3d<StandardMaterial>>(fading)
                .unwrap();
            assert_eq!(handle.0, shared, "the shared asset is handed back");
            assert!(app.world().get::<VisualFade>(fading).is_none());
            assert!(app.world().get::<FadeMaterialSwap>(fading).is_none());
        }

        /// An arrival scales against whatever the spawn produced — a GLB child
        /// carries its rig's `[base].scale`, so growing toward 1 would shrink
        /// it — and lands back on exactly that scale.
        #[test]
        fn an_arrival_grows_into_the_scale_the_spawn_produced() {
            let (mut app, _, fading, _) = fixture();
            let authored = Vec3::splat(0.75);
            app.world_mut().entity_mut(fading).insert((
                Transform::from_scale(authored),
                VisualFade::materialise(0.4, 0.25),
            ));

            advance(&mut app, 0.0);
            let start = app.world().get::<Transform>(fading).unwrap().scale;
            assert!(
                (start - authored * 0.25).length() < 1e-5,
                "an arrival starts at a quarter of its OWN size, got {start:?}"
            );

            advance(&mut app, 0.5);
            let landed = app.world().get::<Transform>(fading).unwrap().scale;
            assert!(
                (landed - authored).length() < 1e-6,
                "an arrival lands on exactly its authored scale, got {landed:?}"
            );
        }

        /// A visual whose meshes arrive LATE — which is every `SceneRoot`,
        /// because a scene populates its children over several frames — is
        /// still caught by the fade rather than left at full alpha.
        #[test]
        fn a_mesh_that_arrives_mid_fade_joins_the_fade() {
            let (mut app, shared, fading, _) = fixture();
            app.world_mut()
                .entity_mut(fading)
                .insert(VisualFade::fade_out(1.0));
            advance(&mut app, 0.1);

            let late = app
                .world_mut()
                .spawn((MeshMaterial3d(shared.clone()), Transform::default()))
                .id();
            app.world_mut().entity_mut(fading).add_child(late);
            advance(&mut app, 0.4);

            assert!(
                (alpha_of(&app, late) - 0.5).abs() < 1e-4,
                "a late child fades with its root, got {}",
                alpha_of(&app, late)
            );
        }
    }
}
