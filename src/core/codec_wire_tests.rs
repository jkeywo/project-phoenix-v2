//! Representative goldens from 12,560 passing differential cases and existing
//! typed-frame tests. Malformed strict refusal fields also pin the former typed
//! extraction rejection; malformed target/verb and nested command extras retain
//! the former forgiving extraction. No legacy implementation is retained.
use serde_json::Value;

#[derive(serde::Deserialize)]
struct CompatibilityCase {
    coverage: Vec<String>,
    raw: String,
    mesh: Option<Value>,
    gm: Option<Value>,
    encoded: Option<String>,
}

fn gm_value(request: crate::gm_action::GmActionRequest) -> Value {
    serde_json::json!({"operator_id":request.operator_id, "correlation":request.correlation, "action":request.action})
}

#[test]
fn mesh_and_gm_wire_compatibility_corpus() {
    let cases: Vec<CompatibilityCase> =
        serde_json::from_str(include_str!("codec_wire_cases.json")).unwrap();
    let mut mesh_families = std::collections::BTreeSet::new();
    let mut gm_families = std::collections::BTreeSet::new();
    for case in cases {
        let mesh = super::decode_mesh_frame(&case.raw);
        if let Some(frame) = &mesh {
            mesh_families.insert(frame.type_name());
        }
        let gm = super::decode_gm_action_request(&case.raw);
        if gm.is_some() {
            let raw: Value = serde_json::from_str(&case.raw).unwrap();
            gm_families.insert(raw["action"].as_str().unwrap().to_owned());
        }
        let encoded = mesh
            .as_ref()
            .map(|frame| super::encode_mesh_frame(frame).unwrap());
        assert_eq!(
            mesh.map(|frame| serde_json::to_value(frame).unwrap()),
            case.mesh,
            "mesh acceptance ({:?}): {}",
            case.coverage,
            case.raw
        );
        assert_eq!(
            gm.map(gm_value),
            case.gm,
            "GM acceptance ({:?}): {}",
            case.coverage,
            case.raw
        );
        assert_eq!(
            encoded, case.encoded,
            "exact mesh bytes ({:?}): {}",
            case.coverage, case.raw
        );
    }
    assert_eq!(
        mesh_families.len(),
        8,
        "every mesh family retains a valid fixture"
    );
    assert!(
        gm_families.len() >= 11,
        "all captured strict GM ingress families retain valid fixtures"
    );
}
