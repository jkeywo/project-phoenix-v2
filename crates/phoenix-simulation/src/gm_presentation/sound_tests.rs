use super::*;

#[test]
fn validated_catalog_ids_keep_the_same_live_dispatch_boundary() {
    let mut catalog = crate::sound_cues::bundled();
    for length in [129, 512] {
        let id = "a".repeat(length);
        catalog.cues[0].id = id.clone();
        assert!(catalog.validate_all().is_ok());
        let cue = PresentationCue::Sound { id, source: None };
        assert!(cue.valid());
        let codec = vellum_digest::ShareCodec::new("SOUND-CUE-");
        let encoded = codec.encode(&cue).unwrap();
        assert_eq!(codec.decode::<PresentationCue>(&encoded).unwrap(), cue);
    }
    catalog.cues[0].id = "a".repeat(513);
    assert!(catalog.validate_all().is_err());
    assert!(!PresentationCue::Sound {
        id: catalog.cues[0].id.clone(),
        source: None
    }
    .valid());
}
