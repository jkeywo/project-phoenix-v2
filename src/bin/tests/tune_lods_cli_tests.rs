use super::*;
use clap::Parser;

fn parse(args: &[&str]) -> Result<TuneArgs, clap::Error> {
    TuneArgs::try_parse_from(std::iter::once("tune-lods").chain(args.iter().copied()))
}

#[test]
fn range_defaults_and_reordered_options_are_pure() {
    let defaults = parse(&["missing.glb"]).unwrap();
    assert_eq!(defaults.resolution, (1920, 1080));
    assert_eq!(defaults.yaws, 4);
    assert_eq!(defaults.pitch_deg, 20.0);
    let args = parse(&[
        "--yaws",
        "2",
        "--resolution=1280X720",
        "--pitch",
        "-20",
        "missing model.glb",
        "--yaws=6",
    ])
    .unwrap();
    assert_eq!(args.yaws, 6);
    assert_eq!(args.resolution, (1280, 720));
    assert_eq!(args.distances, 12);
    assert_eq!(args.pitch_deg, -20.0);
    assert!(!args.decimate);
    assert!(args.out.is_none());
}

#[test]
fn decimate_requires_valid_candidates_and_distance() {
    assert!(parse(&["model.glb", "--decimate"]).is_err());
    assert!(parse(&[
        "model.glb",
        "--decimate",
        "--candidates=,,",
        "--distance=15"
    ])
    .is_err());
    let args = parse(&[
        "--decimate",
        "--candidates=c0.glb,c1.glb",
        "--distance=15",
        "--ref=base.glb",
        "--label=lod1",
        "model.glb",
    ])
    .unwrap();
    assert!(args.decimate);
    assert_eq!(args.distance, Some(15.0));
    assert_eq!(args.reference.as_deref(), Some("base.glb"));
    assert_eq!(args.label.as_deref(), Some("lod1"));
}

#[test]
fn bad_dimensions_counts_and_nonfinite_values_are_rejected() {
    for option in [
        "--resolution=0x720",
        "--resolution=1.5x720",
        "--resolution=1x2x3",
        "--distances=2",
        "--distances=3.5",
        "--yaws=0",
        "--pitch=NaN",
        "--distance=inf",
        "--distance=0",
        "--unknown",
    ] {
        assert!(parse(&["missing.glb", option]).is_err(), "{option}");
    }
    assert!(parse(&["missing.glb", "extra"]).is_err());
    assert!(parse(&["missing.glb", "--out"]).is_err());
    assert_eq!(parse(&["--", "-model.glb"]).unwrap().model, "-model.glb");
    assert_eq!(
        parse(&["--help"]).unwrap_err().kind(),
        clap::error::ErrorKind::DisplayHelp
    );
}
