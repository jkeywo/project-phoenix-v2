use super::*;
use phoenix_runtime::HostSlot;

#[test]
fn browser_and_native_share_typed_admission_and_recovery() {
    let mut grid = Grid::new(HostSlot(1), []);
    let mut admission = BrowserAdmission::default();
    let first = admission.open();
    assert!(admission
        .receive(
            &mut grid,
            &first,
            r#"{"type":"Identify","data":{"token":"A"}}"#
        )
        .is_err());
    for token in ["", "bad token", "bad\n", &"x".repeat(65)] {
        let raw = serde_json::json!({"type":"Identify","data":{"token":token,"name":"player"}})
            .to_string();
        assert!(admission.receive(&mut grid, &first, &raw).is_err());
    }
    assert!(admission
        .receive(
            &mut grid,
            &first,
            r#"{"type":"Move","data":{"dx":1,"dy":0}}"#
        )
        .unwrap()
        .output
        .is_none());
    assert!(admission.recipients().is_empty());
    let identify = r#"{"type":"Identify","data":{"token":"A","name":"player"}}"#;
    assert!(matches!(
        admission
            .receive(&mut grid, &first, identify)
            .unwrap()
            .output,
        Some(Output::Recovery { .. })
    ));
    assert!(admission
        .receive(
            &mut grid,
            &first,
            r#"{"type":"Identify","data":{"token":"B","name":"player"}}"#
        )
        .is_err());
    for raw in [
        r#"{"type":"Move","data":{"dx":1.5,"dy":0}}"#,
        r#"{"type":"Move","data":{"dx":"1","dy":0}}"#,
        "null",
    ] {
        assert!(admission.receive(&mut grid, &first, raw).is_err());
        assert!(GridProtocol::decode_client(raw).is_err());
    }
    let second = admission.open();
    assert_eq!(
        admission
            .receive(&mut grid, &second, identify)
            .unwrap()
            .previous,
        Some(first.clone())
    );
    assert_eq!(admission.recipients(), vec![second.clone()]);
    assert!(admission.receive(&mut grid, &first, identify).is_err());
    admission.close(&first);
    assert_eq!(admission.recipients(), vec![second.clone()]);
    let checkpoint = match apply(&mut grid, Input::Recover).unwrap().unwrap() {
        Output::Recovery { checkpoint } => checkpoint,
        _ => panic!(),
    };
    assert_eq!(checkpoint, grid.checkpoint().unwrap());
    admission.close(&second);
    assert!(admission.recipients().is_empty());
}
