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
//! touches ([`FadedMaterial`], hung on the mesh that owes the original back),
//! drives alpha on the copy, and hands the originals back when it finishes. A
//! fade-out drops its copies with the entity; a fade-in restores the shared
//! handles, so nothing is left holding a clone.
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

/// One mesh's place in a running fade: the shared asset it drew with before,
/// and the private copy it is drawing with now. Lives on the MESH.
///
/// Deliberately not a list on the fade's root. A `SceneRoot` populates its
/// children over several frames and can lose one just as asynchronously — a
/// scene instance respawn, an LOD ladder churning beneath a long window — so a
/// root-side `Vec<Entity>` is a cache of handles with no owner to invalidate
/// it. Hung off the mesh instead, the record is destroyed by the same despawn
/// that destroys the mesh, and the private copy it holds the last strong handle
/// to is freed with it. There is then no way to address a handback to an entity
/// that has gone.
#[derive(Component)]
pub struct FadedMaterial {
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

/// The full local scale a materialising visual is growing toward — captured
/// once, at the first frame of the fade, because it is whatever the spawn put
/// there (a GLB child carries its rig's `[base].scale`, a billboard root its
/// quad's world width and height).
#[derive(Component)]
pub struct FadeTargetScale(Vec3);

/// Advance every running fade: alpha onto the visual's own material copies,
/// scale for an arrival, and the teardown or hand-back when the window closes.
///
/// Each frame the fade re-derives which meshes it owns by walking the live
/// hierarchy under its root, and remembers nothing about them between frames
/// except what it hung on the meshes themselves. A mesh that has gone is simply
/// absent from this frame's walk: no record of it survives to be handed back to,
/// and its material copy died with it.
///
/// `Without<SelfDrivenAlpha>` on the material walk is what keeps the one-writer
/// rule — see that marker.
pub fn drive_visual_fades(
    time: Res<Time>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    children: Query<&Children>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>, Without<SelfDrivenAlpha>>,
    faded: Query<&FadedMaterial>,
    mut fading: Query<(
        Entity,
        &mut VisualFade,
        &mut Transform,
        Option<&FadeTargetScale>,
    )>,
) {
    let dt = time.delta_secs();
    for (root, mut fade, mut transform, target_scale) in fading.iter_mut() {
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
        // drive alpha on the copy. The walk repeats each frame because a
        // scene's children appear over several frames and a late arrival must
        // not stay opaque through a fade that has already started — and because
        // it is the walk, not a stored list, that decides which meshes this
        // fade still owns.
        let alpha = fade.alpha();
        let mut swapped: Vec<(Entity, Handle<StandardMaterial>)> = Vec::new();
        for mesh in visual_meshes(root, &children) {
            if let Ok(FadedMaterial { original, fading }) = faded.get(mesh) {
                // Already swapped on an earlier frame; drive alpha on its copy.
                if let Some(mat) = materials.get_mut(fading) {
                    mat.base_color = mat.base_color.with_alpha(alpha);
                }
                swapped.push((mesh, original.clone()));
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
            copy.base_color = copy.base_color.with_alpha(alpha);
            let fading = materials.add(copy);
            // Both halves of the swap land in one command: the entity draws
            // with the copy and carries the record of what it owes back.
            commands.entity(mesh).insert((
                MeshMaterial3d(fading.clone()),
                FadedMaterial {
                    original: handle.0.clone(),
                    fading,
                },
            ));
            swapped.push((mesh, handle.0.clone()));
        }

        if !fade.finished() {
            continue;
        }

        match fade.direction {
            // The window closed on an outgoing tier: it and its material copies
            // go together, the copies freed by the despawn of the meshes that
            // hold them.
            FadeDirection::Out => commands.entity(root).try_despawn(),
            FadeDirection::In => {
                // Hand the shared assets back, drop the copies, and leave the
                // visual exactly as an un-faded spawn would have left it. Every
                // entity here was matched by a query THIS frame, so none of
                // these commands can be addressed to a despawned mesh.
                for (mesh, original) in swapped {
                    commands
                        .entity(mesh)
                        .insert(MeshMaterial3d(original))
                        .remove::<FadedMaterial>();
                }
                if let Some(FadeTargetScale(full)) = target_scale {
                    transform.scale = *full;
                }
                commands
                    .entity(root)
                    .remove::<(VisualFade, FadeTargetScale)>();
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

#[cfg(test)]
#[path = "visual_fade_tests.rs"]
mod tests;
