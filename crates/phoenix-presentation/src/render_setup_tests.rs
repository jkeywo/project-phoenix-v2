use super::*;
use crate::entities::visual_fade::FadeDirection;
use bevy::ecs::world::CommandQueue;

fn tuning() -> RenderTuning {
    RenderTuning::default()
}

/// Apply a `[render]` block to a game camera and a companion UI camera the
/// way `RendererPlugin` does, and hand back both entities' components.
///
/// `seed_hdr` pre-sets the OPPOSITE state so each call has to change it,
/// which is what makes this a test of the reversibility both functions
/// claim rather than of a lucky initial state.
fn apply_to_pair(cfg: &RenderConfig, seed_hdr: bool) -> (bool, bool, bool, bool) {
    let mut world = World::new();
    let game = world.spawn_empty().id();
    let ui = world.spawn_empty().id();
    if seed_hdr {
        world.entity_mut(game).insert(Hdr);
        world.entity_mut(ui).insert(Hdr);
    }

    let mut queue = CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, &world);
        apply_render_config(&mut commands, game, cfg);
        apply_target_hdr(&mut commands, ui, cfg.hdr);
    }
    queue.apply(&mut world);

    (
        world.entity(game).contains::<Hdr>(),
        world.entity(ui).contains::<Hdr>(),
        world.entity(ui).contains::<Tonemapping>(),
        world.entity(ui).contains::<Bloom>(),
    )
}

/// The invariant the black-viewscreen regression broke.
///
/// Bevy keys its main-texture cache on `hdr`, so the game camera and the UI
/// camera that composites over it must give the same answer or they stop
/// sharing a texture and the UI camera's blit wipes the scene. Both
/// directions of the switch, because an authored `hdr = false` that reached
/// only one of them would split the pair exactly as badly as the default
/// `hdr = true` did.
#[test]
fn both_cameras_on_the_shared_canvas_agree_about_hdr() {
    for hdr in [true, false] {
        let cfg = RenderConfig {
            hdr,
            ..Default::default()
        };
        let (game_hdr, ui_hdr, _, _) = apply_to_pair(&cfg, !hdr);
        assert_eq!(game_hdr, hdr, "the game camera takes the authored hdr");
        assert_eq!(
            ui_hdr, hdr,
            "the UI camera sharing the canvas takes the same one"
        );
    }
}

/// The UI camera takes the marker and NOTHING else: it keeps the
/// `Tonemapping::None` Bevy requires onto every `Camera2d`, because the
/// game camera has already tonemapped the texture it draws into.
#[test]
fn the_ui_camera_takes_the_hdr_marker_but_not_the_display_transform() {
    let (_, ui_hdr, ui_tonemapping, ui_bloom) = apply_to_pair(&RenderConfig::default(), false);
    assert!(ui_hdr, "the marker is the point of the call");
    assert!(
        !ui_tonemapping,
        "a second display transform would tonemap the HUD and re-tonemap the scene"
    );
    assert!(!ui_bloom, "the UI layer has nothing above white to bloom");
}

/// Loading a mission is not an arrival. Every visual on the map is a first
/// visual at that point, and materialising all of them would be a light
/// show rather than the announcement of a reinforcement.
#[test]
fn a_visual_that_appears_before_the_mission_starts_just_appears() {
    assert!(tuning().arrival(true, false).is_none());
}

/// The PRD's case: a mid-mission spawn materialises rather than popping.
#[test]
fn a_visual_that_appears_mid_mission_materialises() {
    let fade = tuning().arrival(true, true).expect("a mid-mission arrival");
    assert_eq!(fade.direction, FadeDirection::In);
    assert!(
        fade.scale_in_from.is_some(),
        "an arrival scales in as well as fading in"
    );
}

/// A tier replacing another cross-fades in, whatever phase the game is in:
/// an LOD switch during loading should dissolve exactly as one later will.
#[test]
fn a_replacement_tier_cross_fades_in_whatever_the_phase() {
    for mid_mission in [false, true] {
        let fade = tuning()
            .arrival(false, mid_mission)
            .expect("a replacement tier cross-fades");
        assert_eq!(fade.direction, FadeDirection::In);
        assert!(
            fade.scale_in_from.is_none(),
            "a cross-fade must not touch the scale the tier just landed on"
        );
        assert_eq!(fade.duration, tuning().lod_fade_secs);
    }
}

/// The two windows are disabled independently — a world can keep its
/// cross-fades and drop the arrival flourish, or the other way round.
#[test]
fn a_zero_window_disables_only_its_own_effect() {
    let no_arrival = RenderTuning {
        materialise_secs: 0.0,
        ..tuning()
    };
    assert!(no_arrival.arrival(true, true).is_none());
    assert!(no_arrival.arrival(false, true).is_some());

    let no_cross_fade = RenderTuning {
        lod_fade_secs: 0.0,
        ..tuning()
    };
    assert!(no_cross_fade.arrival(false, true).is_none());
    assert!(no_cross_fade.arrival(true, true).is_some());
}

/// The whole `[render]` block, clamped into the resource the render path
/// reads: a designer cannot author a window that runs backwards or an
/// arrival that starts bigger than it ends.
#[test]
fn nonsense_authored_values_are_clamped_rather_than_trusted() {
    let got = RenderTuning::from_config(&RenderConfig {
        lod_fade_secs: -1.0,
        materialise_secs: -1.0,
        materialise_start_scale: 9.0,
        ..Default::default()
    });
    assert_eq!(got.lod_fade_secs, 0.0);
    assert_eq!(got.materialise_secs, 0.0);
    assert_eq!(got.materialise_start_scale, 1.0);
}
