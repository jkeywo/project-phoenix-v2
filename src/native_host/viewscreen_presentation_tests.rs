use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("phoenix-viewscreen-presentation-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn an_untouched_machine_follows_its_own_system_preferences() {
    let store = ViewscreenPresentationStore::at(scratch("absent"));
    assert_eq!(store.load(), ViewscreenPresentation::following_system());
    assert!(store.load().is_default());
}

#[test]
fn a_saved_record_survives_the_process_that_wrote_it() {
    // The whole claim of the slice on this side: the operator turns the
    // shared screen up, the host restarts, and the room is still readable.
    let dir = scratch("round-trip");
    let store = ViewscreenPresentationStore::at(&dir);
    let chosen = ViewscreenPresentation {
        text_scale_percent: Some(150),
        contrast: Some(true),
        ..ViewscreenPresentation::following_system()
    };
    store.save(&chosen).expect("save");

    // A second store over the same directory is the next launch: nothing of
    // the first one's memory is available to it except the file.
    let next_launch = ViewscreenPresentationStore::at(&dir);
    assert_eq!(next_launch.load(), chosen);
}

#[test]
fn each_field_is_saved_and_reset_on_its_own() {
    // Per-setting reset (PRD #1418 story 17): returning contrast to the
    // system must leave the text size exactly where the operator put it.
    let dir = scratch("per-setting");
    let store = ViewscreenPresentationStore::at(&dir);
    store
        .save(&ViewscreenPresentation {
            text_scale_percent: Some(175),
            contrast: Some(false),
            ..ViewscreenPresentation::following_system()
        })
        .expect("save");
    let mut record = store.load();
    record.contrast = None;
    store.save(&record).expect("save");

    let reloaded = store.load();
    assert_eq!(reloaded.text_scale_percent, Some(175));
    assert_eq!(reloaded.contrast, None);
}

#[test]
fn resetting_everything_leaves_no_file_behind() {
    let dir = scratch("reset-all");
    let store = ViewscreenPresentationStore::at(&dir);
    store
        .save(&ViewscreenPresentation {
            text_scale_percent: Some(200),
            contrast: Some(true),
            ..ViewscreenPresentation::following_system()
        })
        .expect("save");
    assert!(store.path().exists());

    store
        .save(&ViewscreenPresentation::following_system())
        .expect("reset");
    assert!(!store.path().exists());
    assert_eq!(store.load(), ViewscreenPresentation::following_system());
    // …and resetting twice is not an error, because an operator pressing it
    // again is not doing anything wrong.
    store
        .save(&ViewscreenPresentation::following_system())
        .expect("reset again");
}

#[test]
fn each_effect_is_saved_and_reset_on_its_own() {
    // Issue #1428, and it is the same claim per-setting reset makes above,
    // asked of the three effects: turning the shake off must not disturb a
    // flash the operator softened, and handing one back to the machine must
    // not hand the others back with it.
    let dir = scratch("effects");
    let store = ViewscreenPresentationStore::at(&dir);
    store
        .save(&ViewscreenPresentation {
            shake_percent: Some(0),
            flash_percent: Some(30),
            decorative_motion_percent: Some(40),
            ..ViewscreenPresentation::following_system()
        })
        .expect("save");

    let mut record = store.load();
    assert_eq!(record.shake_percent, Some(0));
    assert_eq!(record.flash_percent, Some(30));
    record.flash_percent = None;
    store.save(&record).expect("save");

    let reloaded = store.load();
    assert_eq!(reloaded.flash_percent, None, "reset returns to following");
    assert_eq!(
        reloaded.shake_percent,
        Some(0),
        "a chosen zero is not a reset"
    );
    assert_eq!(reloaded.decorative_motion_percent, Some(40));
}

#[test]
fn a_display_with_only_its_effects_chosen_is_not_default() {
    // The file is removed only when NOTHING is chosen; an operator who has
    // turned the room's shake off has chosen something, and the next launch
    // has to find it.
    let mut record = ViewscreenPresentation::following_system();
    assert!(record.is_default());
    record.shake_percent = Some(0);
    assert!(!record.is_default());

    let dir = scratch("effects-only");
    let store = ViewscreenPresentationStore::at(&dir);
    store.save(&record).expect("save");
    assert!(store.path().exists());
    assert_eq!(store.load().shake_percent, Some(0));
}

#[test]
fn an_effect_beyond_full_is_clamped_to_full() {
    let dir = scratch("effect-clamp");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join(FILE_NAME), "shake_percent = 900\n").expect("write");
    let store = ViewscreenPresentationStore::at(&dir);
    assert_eq!(store.load().shake_percent, Some(MAX_EFFECT_PERCENT));
}

#[test]
fn a_file_written_before_the_effects_existed_still_reads() {
    // An older build's file names two keys. `#[serde(default)]` means the
    // three effects arrive as "follow this machine" rather than the whole
    // record failing to parse and the display losing its text size too.
    let dir = scratch("older-file");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(
        dir.join(FILE_NAME),
        "text_scale_percent = 150\ncontrast = true\n",
    )
    .expect("write");
    let record = ViewscreenPresentationStore::at(&dir).load();
    assert_eq!(record.text_scale_percent, Some(150));
    assert_eq!(record.effect_percents(), [None, None, None]);
}

#[test]
fn the_seed_script_carries_the_effects_as_fractions() {
    // The page's `readInjectedViewscreenPresentation` normalises `0..=1`
    // fractions or `null`; the host stores whole percent. The conversion
    // happens once, here, at the seam.
    let script = presentation_script(&ViewscreenPresentation {
        shake_percent: Some(0),
        flash_percent: Some(30),
        ..ViewscreenPresentation::following_system()
    });
    assert!(script.contains("\"shake\":0"), "{script}");
    assert!(script.contains("\"flash\":0.3"), "{script}");
    assert!(script.contains("\"decorativeMotion\":null"), "{script}");
    // The injection-safety invariant still holds: every field is a finite
    // clamped number, a bool or null, so nothing here can close the script.
    assert!(!script.contains("</"), "{script}");
}

#[test]
fn a_hand_edited_file_is_clamped_rather_than_honoured() {
    let dir = scratch("hand-edited");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(
        dir.join(FILE_NAME),
        "text_scale_percent = 4000\ncontrast = true\n",
    )
    .expect("write");
    let store = ViewscreenPresentationStore::at(&dir);
    // 40x is a size nothing in this fleet has verified a layout at; the
    // largest supported one is what the operator gets.
    assert_eq!(
        store.load().text_scale_percent,
        Some(MAX_TEXT_SCALE_PERCENT)
    );
    assert_eq!(store.load().contrast, Some(true));
}

#[test]
fn a_corrupt_or_foreign_file_reads_as_nothing_saved() {
    let dir = scratch("corrupt");
    std::fs::create_dir_all(&dir).expect("dir");
    for body in ["this is not toml at all {{{", "unexpected_key = 3\n"] {
        std::fs::write(dir.join(FILE_NAME), body).expect("write");
        let store = ViewscreenPresentationStore::at(&dir);
        assert_eq!(store.load(), ViewscreenPresentation::following_system());
    }
}

#[test]
fn the_seed_script_is_one_well_formed_assignment_with_no_injection_sink() {
    let script = presentation_script(&ViewscreenPresentation {
        text_scale_percent: Some(125),
        contrast: Some(false),
        ..ViewscreenPresentation::following_system()
    });
    assert_eq!(
            script,
            "window.PhoenixViewscreenPresentation = {\"textScale\":1.25,\"contrast\":false,\"shake\":null,\"flash\":null,\"decorativeMotion\":null};"
        );
    // The invariant the module note states, asserted rather than trusted:
    // nothing that could close the script element or open a template can
    // reach the injected string, because every field is a number or a bool.
    assert!(!script.contains("</script"));
    assert!(!script.contains("${"));
    assert!(!script.contains('\n'));
}

#[test]
fn a_machine_with_nothing_chosen_seeds_nulls_rather_than_values() {
    // `null`, not `1` and `false`: the page must be able to tell "follow this
    // machine" from "somebody chose 100% and standard contrast", because the
    // first one still moves when the OS preference does.
    assert_eq!(
            presentation_script(&ViewscreenPresentation::following_system()),
            "window.PhoenixViewscreenPresentation = {\"textScale\":null,\"contrast\":null,\"shake\":null,\"flash\":null,\"decorativeMotion\":null};"
        );
}

#[test]
fn an_out_of_range_size_is_clamped_before_it_reaches_the_page() {
    let script = presentation_script(&ViewscreenPresentation {
        text_scale_percent: Some(900),
        contrast: None,
        ..ViewscreenPresentation::following_system()
    });
    assert!(script.contains(&format!("\"textScale\":{SUPPORTED_TEXT_SCALE_MAX}")));
    // …and the floor is guarded in the other direction, where there is no
    // exposed control at all: only a hand-edited file can ask for it.
    let tiny = presentation_script(&ViewscreenPresentation {
        text_scale_percent: Some(10),
        contrast: None,
        ..ViewscreenPresentation::following_system()
    });
    assert!(tiny.contains(&format!("\"textScale\":{SUPPORTED_TEXT_SCALE_MIN}")));
}
