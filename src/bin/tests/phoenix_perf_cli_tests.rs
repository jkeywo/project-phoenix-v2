use super::*;

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("phoenix-perf").chain(args.iter().copied()))
}

#[test]
fn all_subcommands_parse_without_io() {
    let defaults = parse(&["assets"]).unwrap();
    let Some(PerfCommand::Assets(defaults)) = defaults.command else {
        panic!("expected assets")
    };
    assert_eq!(defaults.root, ".");
    assert_eq!(defaults.capture, "-");
    for name in ["assets", "mesh"] {
        let args = parse(&[name, "--root", "first", "--root=last path", "--capture=-"]).unwrap();
        let measure = match args.command.unwrap() {
            PerfCommand::Assets(args) | PerfCommand::Mesh(args) => args,
            _ => panic!("expected measurement"),
        };
        assert_eq!(measure.root, "last path");
        assert_eq!(measure.capture, "-");
    }
    let args = parse(&[
        "report",
        "--capture=-",
        "--gate",
        "--gate",
        "--scenario=probe",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(PerfCommand::Report(ReportArgs { gate: true, .. }))
    ));
    assert!(parse(&["adopt", "--capture=-", "--out=-"]).is_ok());
    assert!(parse(&["adopt", "--artifact=missing", "--out-dir=baselines"]).is_ok());
    assert!(parse(&[]).unwrap().command.is_none());
}

#[test]
fn syntax_and_adoption_source_fail_before_io() {
    for args in [
        vec!["unknown"],
        vec!["assets", "--bad"],
        vec!["mesh", "extra"],
        vec!["report"],
        vec!["report", "--capture"],
        vec!["adopt"],
        vec!["adopt", "--capture=-", "--artifact=missing"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    for args in [
        vec!["--help"],
        vec!["report", "--help"],
        vec!["adopt", "--help"],
    ] {
        assert_eq!(
            parse(&args).unwrap_err().kind(),
            clap::error::ErrorKind::DisplayHelp
        );
    }
}
