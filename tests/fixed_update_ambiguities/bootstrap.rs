//! The one-time allowance is the reviewed first census, not a fresh capture of
//! whatever debt a candidate branch has accumulated. Later trusted Git ledgers
//! use the ordinary access-subset/multiplicity ratchet in the parent test.
use project_phoenix::headless::determinism_audit::Ambiguity;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

// Original capture and byte-identical native-pre-r1: 1,968 rows, SHA256
// 49312c588121cb38f9a1b203a34b01e2e9936dae36380af5f55b2c0b052236fe.
// Existing Ambiguity serialization produces 437,412 compact UTF-8 bytes.
// The Git blob is over those bytes: no BOM/newline, filters or deduplication.
const INITIAL_ROWS: usize = 1_968;
const INITIAL_BLOB: &str = "a3a15b158431852c9b3c3bf890dbe8929e9e02ec";

pub(super) fn enforce_first_allowance(root: &Path, rows: &[Ambiguity]) -> Result<(), String> {
    require_fingerprint(root, rows, INITIAL_ROWS, INITIAL_BLOB)
}

fn require_fingerprint(
    root: &Path,
    rows: &[Ambiguity],
    expected_rows: usize,
    expected_blob: &str,
) -> Result<(), String> {
    if rows.len() != expected_rows {
        return Err(format!(
            "first allowance must retain the reviewed {expected_rows} rows, found {}",
            rows.len()
        ));
    }
    // This is the production Ambiguity type's Serialize implementation, not a
    // parallel encoder. Parsing precedes this call, so checkout line endings
    // and pretty-print whitespace never enter the checked bytes.
    let bytes = serde_json::to_vec(rows).map_err(|error| error.to_string())?;
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start local Git fingerprint: {error}"))?;
    // Taking then dropping ChildStdin closes the stream. Always reap the child,
    // including a broken-pipe error; no Git object is written (there is no -w).
    let written = child.stdin.take().expect("piped stdin").write_all(&bytes);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for local Git fingerprint: {error}"))?;
    written.map_err(|error| format!("cannot send canonical census to Git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "local Git fingerprint failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let observed = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    if observed.trim() != expected_blob {
        return Err(format!(
            "first allowance differs from the reviewed capture: expected {expected_blob}, found {}",
            observed.trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_phoenix::headless::determinism_audit::{parse_census, uncovered};
    use serde::Deserialize;

    // Independently pinned Git blob of these three typed rows. A duplicate is
    // deliberate: a same-length substitution can still enlarge its quota.
    const FIXTURE_BLOB: &str = "a89e03be965b6b798995d1de18d3176c54382724";
    fn fixture() -> Vec<Ambiguity> {
        let repeated = Ambiguity {
            systems: ["alpha".into(), "beta".into()],
            access: vec!["R".into()],
        };
        vec![
            repeated.clone(),
            repeated,
            Ambiguity {
                systems: ["alpha".into(), "gamma".into()],
                access: vec!["S".into()],
            },
        ]
    }

    fn check(rows: &[Ambiguity]) -> Result<(), String> {
        require_fingerprint(Path::new(env!("CARGO_MANIFEST_DIR")), rows, 3, FIXTURE_BLOB)
    }

    #[test]
    fn first_introduction_accepts_the_pinned_canonical_capture() {
        check(&fixture()).unwrap();
    }

    #[test]
    fn first_introduction_rejects_changed_access_and_duplicate_quotas() {
        let original = fixture();
        let mut changed_access = original.clone();
        changed_access[0].access = vec!["Q".into()];
        assert!(check(&changed_access).is_err());

        let mut extra_access = original.clone();
        extra_access[0].access.push("S".into());
        extra_access.sort();
        assert!(check(&extra_access).is_err());

        let mut extra_instance = original.clone();
        extra_instance.push(original[0].clone());
        extra_instance.sort();
        assert!(check(&extra_instance).is_err());

        let mut replaced_instance = original.clone();
        replaced_instance[2] = original[0].clone();
        assert_eq!(replaced_instance.len(), original.len());
        assert!(check(&replaced_instance).is_err());

        // Initial introduction is exact; subsequent shrinkage is handled by
        // the trusted-base branch, not by changing this first-capture pin.
        assert!(check(&original[..2]).is_err());
    }

    #[test]
    fn first_introduction_ignores_json_whitespace_but_keeps_duplicate_rows() {
        let pretty = serde_json::to_string_pretty(&fixture()).unwrap();
        let input = format!(" \r\n{}\r\n\t", pretty.replace('\n', "\r\n"));
        let parsed = parse_census(&input).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], parsed[1]);
        check(&parsed).unwrap();
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Expansion {
        systems: [String; 2],
        added_access: Vec<String>,
        complete_old_access: Vec<String>,
        complete_new_access: Vec<String>,
    }

    #[test]
    fn all_seventeen_declared_access_expansions_exceed_the_old_allowance() {
        let cases: Vec<Expansion> = serde_json::from_str(include_str!(
            "../fixtures/determinism/initial-access-expansions.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 17);
        let mut rng = 0;
        let mut mint = 0;
        let mut old_rows = Vec::new();
        let mut new_rows = Vec::new();
        for case in cases {
            match case.added_access.as_slice() {
                [name] if name == "project_phoenix::sim_rng::SimRng" => rng += 1,
                [name] if name == "project_phoenix::world_id::WorldIdMint" => mint += 1,
                other => panic!("unexpected original declaration expansion: {other:?}"),
            }
            let old = Ambiguity {
                systems: case.systems.clone(),
                access: case.complete_old_access,
            };
            let new = Ambiguity {
                systems: case.systems,
                access: case.complete_new_access,
            };
            let mut expected = old.access.clone();
            expected.extend(case.added_access);
            expected.sort();
            assert_eq!(new.access, expected);
            assert_eq!(
                uncovered(std::slice::from_ref(&new), std::slice::from_ref(&old)),
                vec![new.clone()],
                "a repeated pair with broader access is new debt"
            );
            assert!(uncovered(std::slice::from_ref(&old), std::slice::from_ref(&new)).is_empty());
            old_rows.push(old);
            new_rows.push(new);
        }
        assert_eq!((rng, mint), (7, 10));
        assert_eq!(uncovered(&new_rows, &old_rows).len(), 17);
    }
}
