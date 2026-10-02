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
        assert!(app.world().get::<FadedMaterial>(fading).is_none());
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

    /// The defect this module shipped with, and the reason a native host
    /// filled its log with `Entity despawned: ... its index now has
    /// generation N` a few seconds into `combat_test`.
    ///
    /// A scene populates its children over several frames, and it can also
    /// LOSE one mid-fade — a scene instance respawn, an LOD churn beneath a
    /// long window. The swap record must not outlive the mesh it describes:
    /// a dead mesh's private material copy dies with it and there is
    /// nothing left to hand back, while the handback itself must never be
    /// addressed to an entity whose index has since been recycled.
    ///
    /// The error handler is set to PANIC here on purpose. A command applied
    /// to a stale handle is only ever visible as an error — Bevy compares
    /// generations, so a recycled index is rejected rather than silently
    /// written — and the whole point of the fix is that the error is never
    /// raised.
    #[test]
    fn a_mesh_that_dies_mid_fade_is_never_handed_a_material_back() {
        let (mut app, shared, fading, _) = fixture();
        app.insert_resource(bevy::ecs::error::DefaultErrorHandler(
            bevy::ecs::error::panic,
        ));
        let doomed = app
            .world_mut()
            .spawn((MeshMaterial3d(shared.clone()), Transform::default()))
            .id();
        let survivor = app
            .world_mut()
            .spawn((MeshMaterial3d(shared.clone()), Transform::default()))
            .id();
        app.world_mut()
            .entity_mut(fading)
            .add_children(&[doomed, survivor]);
        app.world_mut()
            .entity_mut(fading)
            .insert(VisualFade::fade_in(1.0));

        // Both children are swapped onto private copies.
        advance(&mut app, 0.1);
        assert_ne!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(doomed)
                .unwrap()
                .0,
            shared,
            "the doomed child is drawing with a copy before it dies"
        );

        // The mesh goes, and its index is recycled by a NEW child of the
        // same visual — exactly the `414v2 -> generation 3` shape from the
        // host log.
        app.world_mut().entity_mut(doomed).despawn();
        let recycled = app
            .world_mut()
            .spawn((MeshMaterial3d(shared.clone()), Transform::default()))
            .id();
        app.world_mut().entity_mut(fading).add_child(recycled);
        assert_eq!(
            recycled.index(),
            doomed.index(),
            "the fixture needs the index reused for this to be the reported bug"
        );

        advance(&mut app, 0.4);
        // Closing the window hands the shared assets back. Nothing may be
        // addressed to `doomed`.
        advance(&mut app, 1.0);

        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(survivor)
                .unwrap()
                .0,
            shared,
            "a surviving mesh still gets its shared asset back"
        );
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(recycled)
                .unwrap()
                .0,
            shared,
            "so does the one that took the dead mesh's index"
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
