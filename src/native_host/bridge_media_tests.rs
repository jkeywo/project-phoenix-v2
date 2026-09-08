//! Pure media-profile tests for [`super`] (issue #1126).
//!
//! No device, no OS media API, no hardware — these run in the ordinary
//! `cargo test` CI job. They cover the acceptance criteria that have logic in
//! them: stable device identity across a re-enumeration, the per-surface
//! camera/microphone/output assignment, the parse/validate failure taxonomy
//! (wrong-kind, malformed id, duplicate, unconsented share), the explicit-consent
//! sharing warning, the deterministic default when the operator has not chosen,
//! and the naming of a device that is missing or denied without making the
//! surface unusable.

use super::*;

fn cam(name: &str) -> RawMediaDevice {
    RawMediaDevice {
        kind: MediaKind::Camera,
        name: Some(name.to_string()),
        hardware_id: None,
        default: false,
        availability: DeviceAvailability::Available,
    }
}

fn mic(name: &str) -> RawMediaDevice {
    RawMediaDevice {
        kind: MediaKind::Microphone,
        name: Some(name.to_string()),
        hardware_id: None,
        default: false,
        availability: DeviceAvailability::Available,
    }
}

fn out(name: &str) -> RawMediaDevice {
    RawMediaDevice {
        kind: MediaKind::Output,
        name: Some(name.to_string()),
        hardware_id: None,
        default: false,
        availability: DeviceAvailability::Available,
    }
}

// ── stable identity ─────────────────────────────────────────────────────────

#[test]
fn a_device_identity_is_kind_then_name() {
    let discovered = identify_media(&[
        cam("Logitech BRIO"),
        mic("Blue Yeti"),
        out("Bridge Speakers"),
    ]);
    assert_eq!(discovered[0].identity.as_str(), "camera:Logitech BRIO");
    assert_eq!(discovered[1].identity.as_str(), "mic:Blue Yeti");
    assert_eq!(discovered[2].identity.as_str(), "output:Bridge Speakers");
    // And the kind is recoverable from the identity — the property validation
    // leans on.
    assert_eq!(discovered[0].identity.kind(), Some(MediaKind::Camera));
    assert_eq!(discovered[1].identity.kind(), Some(MediaKind::Microphone));
    assert_eq!(discovered[2].identity.kind(), Some(MediaKind::Output));
}

#[test]
fn identity_survives_reenumeration_in_another_order() {
    // The whole point of a stable identity: the same devices, enumerated in a
    // different order, keep the same identities so a profile still refers to them.
    let before = identify_media(&[cam("BRIO"), mic("Yeti"), out("Speakers")]);
    let after = identify_media(&[out("Speakers"), cam("BRIO"), mic("Yeti")]);
    let set = |v: &[DiscoveredMediaDevice]| {
        v.iter()
            .map(|d| d.identity.as_str().to_string())
            .collect::<std::collections::HashSet<_>>()
    };
    assert_eq!(set(&before), set(&after));
}

#[test]
fn a_camera_and_a_mic_of_the_same_name_are_distinct_identities() {
    // Same NAME, different KIND — the kind tag keeps them apart, so a headset that
    // reports one name for its mic and its output is two devices, not a clash.
    let discovered = identify_media(&[mic("Comms Headset"), out("Comms Headset")]);
    assert_ne!(discovered[0].identity, discovered[1].identity);
    assert_eq!(discovered[0].identity.as_str(), "mic:Comms Headset");
    assert_eq!(discovered[1].identity.as_str(), "output:Comms Headset");
}

#[test]
fn two_identical_devices_are_disambiguated_by_ordinal() {
    // Two of the same model of the same kind are indistinguishable by name, so
    // they fall back to an enumeration-order suffix — the documented limit.
    let discovered = identify_media(&[mic("USB PnP Mic"), mic("USB PnP Mic")]);
    assert_ne!(discovered[0].identity, discovered[1].identity);
    assert_eq!(discovered[0].identity.as_str(), "mic:USB PnP Mic#1");
    assert_eq!(discovered[1].identity.as_str(), "mic:USB PnP Mic#2");
}

#[test]
fn two_identical_devices_prefer_a_hardware_id_when_present() {
    // When the OS gives a hardware-stable id, the collision suffix uses it rather
    // than the volatile enumeration ordinal.
    let mut a = mic("USB PnP Mic");
    a.hardware_id = Some("USB\\VID_1234&PID_5678\\A".to_string());
    let mut b = mic("USB PnP Mic");
    b.hardware_id = Some("USB\\VID_1234&PID_5678\\B".to_string());
    let discovered = identify_media(&[a, b]);
    assert_eq!(
        discovered[0].identity.as_str(),
        "mic:USB PnP Mic#USB\\VID_1234&PID_5678\\A"
    );
    assert_eq!(
        discovered[1].identity.as_str(),
        "mic:USB PnP Mic#USB\\VID_1234&PID_5678\\B"
    );
}

#[test]
fn a_unique_device_keeps_the_short_suffix_free_identity() {
    // Only genuine collisions pay the suffix; a lone device of a name+kind keeps
    // the readable key even if another device of a DIFFERENT kind shares its name.
    let discovered = identify_media(&[mic("Studio"), out("Studio"), cam("Studio")]);
    assert_eq!(discovered[0].identity.as_str(), "mic:Studio");
    assert_eq!(discovered[1].identity.as_str(), "output:Studio");
    assert_eq!(discovered[2].identity.as_str(), "camera:Studio");
}

#[test]
fn a_nameless_device_gets_a_stable_placeholder() {
    let mut m = mic("ignored");
    m.name = None;
    let discovered = identify_media(std::slice::from_ref(&m));
    assert_eq!(discovered[0].identity.as_str(), "mic:unnamed-mic");
}

// ── profile shape ───────────────────────────────────────────────────────────

fn entry(surface: &str, camera: Option<&str>, mics: &[&str], outs: &[&str]) -> MediaSurfaceEntry {
    MediaSurfaceEntry {
        surface: surface.to_string(),
        camera: camera.map(|c| c.to_string()),
        microphones: mics.iter().map(|m| m.to_string()).collect(),
        outputs: outs.iter().map(|o| o.to_string()).collect(),
        allow_shared: Vec::new(),
    }
}

#[test]
fn a_well_formed_assignment_validates_to_kind_checked_ids() {
    let media = validate_media(&[
        entry(
            "viewscreen",
            Some("camera:BRIO"),
            &["mic:Yeti"],
            &["output:Speakers"],
        ),
        entry("comms", None, &["mic:Headset"], &["output:Headset"]),
    ])
    .expect("validates");
    assert_eq!(media.surfaces.len(), 2);
    assert!(media.warnings.is_empty());
    let vs = &media.surfaces[0];
    assert_eq!(vs.camera.as_ref().unwrap().as_str(), "camera:BRIO");
    assert_eq!(vs.microphones[0].as_str(), "mic:Yeti");
    assert_eq!(vs.outputs[0].as_str(), "output:Speakers");
}

#[test]
fn a_surface_may_carry_more_than_one_mic_and_output() {
    let media = validate_media(&[entry(
        "comms",
        None,
        &["mic:Yeti", "mic:Boom"],
        &["output:Speakers", "output:Headset"],
    )])
    .expect("validates");
    assert_eq!(media.surfaces[0].microphones.len(), 2);
    assert_eq!(media.surfaces[0].outputs.len(), 2);
}

// ── failure taxonomy ────────────────────────────────────────────────────────

#[test]
fn a_wrong_kind_device_in_a_slot_is_refused_with_a_clear_explanation() {
    // A microphone dropped into the camera slot — the archetypal wrong-kind error.
    let err = validate_media(&[entry("viewscreen", Some("mic:Yeti"), &[], &[])]).unwrap_err();
    assert_eq!(
        err,
        MediaError::WrongKind {
            surface: "viewscreen".to_string(),
            expected: MediaKind::Camera,
            found: MediaKind::Microphone,
            id: "mic:Yeti".to_string(),
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("viewscreen"), "{msg}");
    assert!(msg.contains("camera"), "{msg}");
    assert!(msg.contains("microphone"), "{msg}");
}

#[test]
fn an_output_in_the_microphone_slot_is_refused() {
    let err = validate_media(&[entry("comms", None, &["output:Speakers"], &[])]).unwrap_err();
    assert_eq!(
        err,
        MediaError::WrongKind {
            surface: "comms".to_string(),
            expected: MediaKind::Microphone,
            found: MediaKind::Output,
            id: "output:Speakers".to_string(),
        }
    );
}

#[test]
fn a_camera_in_the_output_slot_is_refused() {
    let err = validate_media(&[entry("comms", None, &[], &["camera:BRIO"])]).unwrap_err();
    assert_eq!(
        err,
        MediaError::WrongKind {
            surface: "comms".to_string(),
            expected: MediaKind::Output,
            found: MediaKind::Camera,
            id: "camera:BRIO".to_string(),
        }
    );
}

#[test]
fn an_id_with_no_kind_tag_is_refused_as_malformed() {
    let err = validate_media(&[entry("viewscreen", Some("Logitech BRIO"), &[], &[])]).unwrap_err();
    assert_eq!(
        err,
        MediaError::MalformedId {
            surface: "viewscreen".to_string(),
            expected: MediaKind::Camera,
            id: "Logitech BRIO".to_string(),
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("no known device kind"), "{msg}");
}

#[test]
fn an_id_with_an_unknown_kind_tag_is_refused_as_malformed() {
    // A plausible-looking but wrong tag is malformed, not a wrong-kind — there is
    // no kind to have gotten wrong.
    let err = validate_media(&[entry("comms", None, &["speaker:Thing"], &[])]).unwrap_err();
    assert_eq!(
        err,
        MediaError::MalformedId {
            surface: "comms".to_string(),
            expected: MediaKind::Microphone,
            id: "speaker:Thing".to_string(),
        }
    );
}

#[test]
fn the_same_device_twice_on_one_surface_is_refused() {
    let err = validate_media(&[entry("comms", None, &["mic:Yeti", "mic:Yeti"], &[])]).unwrap_err();
    assert_eq!(
        err,
        MediaError::DuplicateDevice {
            surface: "comms".to_string(),
            id: "mic:Yeti".to_string(),
        }
    );
}

#[test]
fn two_surfaces_of_the_same_name_are_refused() {
    let err = validate_media(&[
        entry("comms", None, &["mic:A"], &[]),
        entry("comms", None, &["mic:B"], &[]),
    ])
    .unwrap_err();
    assert_eq!(
        err,
        MediaError::DuplicateSurface {
            surface: "comms".to_string(),
        }
    );
}

// ── explicit sharing (acceptance criterion 2) ───────────────────────────────

#[test]
fn a_device_shared_without_consent_is_refused() {
    // The same mic on two surfaces, neither opting in — refused, naming both
    // surfaces and telling the operator how to allow it or split it.
    let err = validate_media(&[
        entry("viewscreen", None, &["mic:Yeti"], &[]),
        entry("comms", None, &["mic:Yeti"], &[]),
    ])
    .unwrap_err();
    assert_eq!(
        err,
        MediaError::SharedWithoutConsent {
            id: "mic:Yeti".to_string(),
            surfaces: vec!["viewscreen".to_string(), "comms".to_string()],
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("explicit choice"), "{msg}");
    assert!(msg.contains("allow_shared"), "{msg}");
}

#[test]
fn a_device_shared_with_consent_from_every_surface_validates_with_a_warning() {
    let mut a = entry("viewscreen", None, &["mic:Yeti"], &[]);
    a.allow_shared = vec!["mic:Yeti".to_string()];
    let mut b = entry("comms", None, &["mic:Yeti"], &[]);
    b.allow_shared = vec!["mic:Yeti".to_string()];
    let media = validate_media(&[a, b]).expect("consented share validates");
    assert_eq!(
        media.warnings,
        vec![MediaWarning::Contention {
            id: "mic:Yeti".to_string(),
            surfaces: vec!["viewscreen".to_string(), "comms".to_string()],
        }]
    );
    assert!(media.warnings[0]
        .to_string()
        .contains("shared by explicit consent"));
}

#[test]
fn a_share_consented_by_only_one_of_two_surfaces_is_still_refused() {
    // Consent must come from EVERY surface using the device, not just one.
    let mut a = entry("viewscreen", None, &["mic:Yeti"], &[]);
    a.allow_shared = vec!["mic:Yeti".to_string()];
    let b = entry("comms", None, &["mic:Yeti"], &[]);
    let err = validate_media(&[a, b]).unwrap_err();
    assert_eq!(
        err,
        MediaError::SharedWithoutConsent {
            id: "mic:Yeti".to_string(),
            surfaces: vec!["viewscreen".to_string(), "comms".to_string()],
        }
    );
}

#[test]
fn distinct_devices_across_surfaces_need_no_consent() {
    // The ordinary case — each surface its own hardware — is no contention at all.
    let media = validate_media(&[
        entry(
            "viewscreen",
            Some("camera:BRIO"),
            &["mic:Yeti"],
            &["output:Speakers"],
        ),
        entry("comms", None, &["mic:Headset"], &["output:Headset"]),
    ])
    .expect("validates");
    assert!(media.warnings.is_empty());
}

// ── resolution against present devices (acceptance criterion 4) ──────────────

fn validated(entries: &[MediaSurfaceEntry]) -> ValidatedMedia {
    validate_media(entries).expect("test fixture validates")
}

#[test]
fn a_matching_assignment_resolves_to_its_devices_with_no_problems() {
    let media = validated(&[entry(
        "viewscreen",
        Some("camera:BRIO"),
        &["mic:Yeti"],
        &["output:Speakers"],
    )]);
    let discovered = identify_media(&[cam("BRIO"), mic("Yeti"), out("Speakers")]);
    let resolved = resolve_media(&media, &discovered);
    assert!(!resolved.has_problems());
    let vs = &resolved.surfaces[0];
    assert_eq!(vs.camera.as_ref().unwrap().as_str(), "camera:BRIO");
    assert_eq!(vs.microphones[0].as_str(), "mic:Yeti");
    assert_eq!(vs.outputs[0].as_str(), "output:Speakers");
}

#[test]
fn a_missing_device_is_named_and_the_surface_stays_usable() {
    // The camera is unplugged; the profile still assigns it. The camera slot is
    // reported empty, and the surface keeps its mic and output — never unusable.
    let media = validated(&[entry(
        "viewscreen",
        Some("camera:BRIO"),
        &["mic:Yeti"],
        &["output:Speakers"],
    )]);
    let discovered = identify_media(&[mic("Yeti"), out("Speakers")]); // no BRIO
    let resolved = resolve_media(&media, &discovered);
    assert_eq!(
        resolved.problems,
        vec![MediaProblem::DeviceMissing {
            surface: "viewscreen".to_string(),
            kind: MediaKind::Camera,
            id: "camera:BRIO".to_string(),
        }]
    );
    let vs = &resolved.surfaces[0];
    assert!(
        vs.camera.is_none(),
        "the missing camera slot is left unfilled"
    );
    assert_eq!(
        vs.microphones[0].as_str(),
        "mic:Yeti",
        "the mic still resolves"
    );
    assert_eq!(
        vs.outputs[0].as_str(),
        "output:Speakers",
        "the output still resolves"
    );
    assert!(resolved.problems[0].to_string().contains("not connected"));
}

#[test]
fn a_denied_device_is_named_distinctly_from_a_missing_one() {
    let media = validated(&[entry("comms", None, &["mic:Yeti"], &[])]);
    let mut denied = mic("Yeti");
    denied.availability = DeviceAvailability::Denied;
    let discovered = identify_media(&[denied]);
    let resolved = resolve_media(&media, &discovered);
    assert_eq!(
        resolved.problems,
        vec![MediaProblem::DeviceDenied {
            surface: "comms".to_string(),
            kind: MediaKind::Microphone,
            id: "mic:Yeti".to_string(),
        }]
    );
    assert!(resolved.surfaces[0].microphones.is_empty());
    assert!(resolved.problems[0].to_string().contains("denied"));
}

#[test]
fn a_present_but_unassigned_device_is_not_a_problem() {
    // A bridge need not use every device it can see; an extra mic is not an error.
    let media = validated(&[entry("comms", None, &["mic:Yeti"], &[])]);
    let discovered = identify_media(&[mic("Yeti"), mic("Spare"), cam("BRIO")]);
    let resolved = resolve_media(&media, &discovered);
    assert!(!resolved.has_problems());
}

// ── deterministic default (operator has not chosen) ─────────────────────────

#[test]
fn the_default_assignment_picks_the_os_default_of_each_kind() {
    let mut default_mic = mic("Yeti");
    default_mic.default = true;
    let discovered = identify_media(&[
        cam("BRIO"),
        mic("Boom"), // enumerated first, but NOT the default
        default_mic, // the OS default mic
        out("Speakers"),
    ]);
    let entries = default_media_assignment(&["viewscreen"], &discovered);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.camera.as_deref(), Some("camera:BRIO"));
    assert_eq!(
        e.microphones,
        vec!["mic:Yeti".to_string()],
        "the OS default mic wins over the first"
    );
    assert_eq!(e.outputs, vec!["output:Speakers".to_string()]);
    // And what it generates validates.
    assert!(validate_media(&entries).is_ok());
}

#[test]
fn the_default_falls_back_to_the_first_of_a_kind_when_none_is_flagged() {
    let discovered = identify_media(&[mic("First"), mic("Second"), out("Speakers")]);
    let entries = default_media_assignment(&["comms"], &discovered);
    assert_eq!(entries[0].microphones, vec!["mic:First".to_string()]);
    // No camera present: the slot is simply left empty, not invented.
    assert_eq!(entries[0].camera, None);
}

#[test]
fn the_default_for_many_surfaces_shares_the_one_devices_by_consent() {
    // A single-device box gives every surface the same device; the generated
    // default consents to that share so it validates with a warning, not an error.
    let discovered = identify_media(&[cam("BRIO"), mic("Yeti"), out("Speakers")]);
    let entries = default_media_assignment(&["viewscreen", "comms"], &discovered);
    assert_eq!(entries.len(), 2);
    // Both surfaces got the one camera; the share is consented.
    assert!(entries[0].allow_shared.contains(&"camera:BRIO".to_string()));
    assert!(entries[1].allow_shared.contains(&"camera:BRIO".to_string()));
    let media = validate_media(&entries).expect("the generated default validates");
    assert!(
        !media.warnings.is_empty(),
        "the shared devices warn about contention"
    );
}

#[test]
fn the_default_for_a_single_surface_needs_no_consent() {
    let discovered = identify_media(&[cam("BRIO"), mic("Yeti"), out("Speakers")]);
    let entries = default_media_assignment(&["viewscreen"], &discovered);
    assert!(entries[0].allow_shared.is_empty());
    assert!(validate_media(&entries).unwrap().warnings.is_empty());
}

#[test]
fn the_default_skips_a_denied_device() {
    // A present-but-denied device is not a usable default; the slot is left empty.
    let mut denied = cam("BRIO");
    denied.availability = DeviceAvailability::Denied;
    let discovered = identify_media(&[denied, mic("Yeti")]);
    let entries = default_media_assignment(&["viewscreen"], &discovered);
    assert_eq!(entries[0].camera, None, "a denied camera is not chosen");
    assert_eq!(entries[0].microphones, vec!["mic:Yeti".to_string()]);
}

// ── setup report ────────────────────────────────────────────────────────────

fn report_profile(entries: Vec<MediaSurfaceEntry>) -> super::super::bridge_profile::BridgeProfile {
    let mut p = super::super::bridge_profile::BridgeProfile::empty();
    p.media = entries;
    p
}

#[test]
fn the_media_report_lists_devices_by_kind() {
    let discovered = identify_media(&[cam("BRIO"), mic("Yeti"), out("Speakers")]);
    let report = render_media_setup_report(&discovered, None);
    assert!(report.contains("camera:BRIO"), "{report}");
    assert!(report.contains("mic:Yeti"), "{report}");
    assert!(report.contains("output:Speakers"), "{report}");
    assert!(
        report.contains("No media assignments in the profile."),
        "{report}"
    );
}

#[test]
fn the_media_report_notes_the_absent_backend_when_no_devices() {
    let report = render_media_setup_report(&[], None);
    assert!(report.contains("no media-device backend"), "{report}");
}

#[test]
fn the_media_report_validates_assignments_even_with_no_backend() {
    // Authoring by hand still gets the wrong-kind check, even though no live
    // device could be enumerated.
    let profile = report_profile(vec![entry("viewscreen", Some("mic:Yeti"), &[], &[])]);
    let report = render_media_setup_report(&[], Some(&profile));
    assert!(report.contains("Media assignments are invalid"), "{report}");
    assert!(report.contains("camera"), "{report}");
}

#[test]
fn the_media_report_surfaces_a_missing_device_when_devices_are_present() {
    let profile = report_profile(vec![entry(
        "viewscreen",
        Some("camera:BRIO"),
        &["mic:Yeti"],
        &[],
    )]);
    let discovered = identify_media(&[mic("Yeti")]); // no camera
    let report = render_media_setup_report(&discovered, Some(&profile));
    assert!(report.contains("Media problems:"), "{report}");
    assert!(report.contains("not connected"), "{report}");
}

#[test]
fn the_media_report_confirms_a_clean_match_and_shows_a_warning() {
    let mut a = entry("viewscreen", None, &["mic:Yeti"], &[]);
    a.allow_shared = vec!["mic:Yeti".to_string()];
    let mut b = entry("comms", None, &["mic:Yeti"], &[]);
    b.allow_shared = vec!["mic:Yeti".to_string()];
    let profile = report_profile(vec![a, b]);
    let discovered = identify_media(&[mic("Yeti")]);
    let report = render_media_setup_report(&discovered, Some(&profile));
    assert!(report.contains("warning:"), "{report}");
    assert!(
        report.contains("Media assignments match the connected devices."),
        "{report}"
    );
}

#[test]
fn a_completed_empty_output_scan_reports_missing_outputs_not_an_absent_backend() {
    let profile = report_profile(vec![entry(
        "comms",
        Some("camera:BRIO"),
        &["mic:Yeti"],
        &["output:Headset"],
    )]);
    let report = render_output_setup_report(&[], Some(&profile));
    assert!(report.contains("Output backend: CPAL"));
    assert!(report.contains("output:Headset"));
    assert!(report.contains("not connected"));
    assert!(!report.contains(enumerate_note()));
    assert!(!report.contains("assigned camera"));
    assert!(!report.contains("assigned microphone"));
}
