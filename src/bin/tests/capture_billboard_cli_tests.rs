use super::*;
use clap::Parser;

fn parse(args: &[&str]) -> Result<CaptureArgs, clap::Error> {
    CaptureArgs::try_parse_from(std::iter::once("capture-billboard").chain(args.iter().copied()))
}

#[test]
fn options_before_positionals_and_last_values_are_used() {
    let args = parse(&[
        "--views",
        "4",
        "--views=12",
        "--pitch",
        "-15",
        "ship model.glb",
        "out atlas.png",
    ])
    .unwrap();
    assert_eq!(args.model, "ship model.glb");
    assert_eq!(args.output, PathBuf::from("out atlas.png"));
    assert_eq!(args.views, 12);
    assert_eq!(args.resolution, 256);
    assert_eq!(args.pitch_deg, -15.0);
}

#[test]
fn end_of_options_allows_hyphenated_paths() {
    let args = parse(&["--", "--model.glb", "-out.png"]).unwrap();
    assert_eq!(args.model, "--model.glb");
    assert_eq!(args.output, PathBuf::from("-out.png"));
}

#[test]
fn malformed_arguments_fail_before_asset_loading() {
    for tail in [
        vec!["--views", "0"],
        vec!["--views", "1.5"],
        vec!["--resolution", "-1"],
        vec!["--pitch", "NaN"],
        vec!["--pitch", "inf"],
        vec!["--unknown"],
        vec!["extra"],
        vec!["--views"],
    ] {
        let mut args = vec!["missing.glb", "out.png"];
        args.extend(tail);
        assert!(parse(&args).is_err(), "{args:?}");
    }
    assert_eq!(
        parse(&["--help"]).unwrap_err().kind(),
        clap::error::ErrorKind::DisplayHelp
    );
}
