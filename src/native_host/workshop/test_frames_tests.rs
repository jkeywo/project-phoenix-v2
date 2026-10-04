use super::*;
use std::io::{BufRead, Cursor};
fn image() -> Vec<u8> {
    let mut bytes = vec![0; 33];
    bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    bytes[12..16].copy_from_slice(b"IHDR");
    bytes[16..20].copy_from_slice(&WIDTH.to_be_bytes());
    bytes[20..24].copy_from_slice(&HEIGHT.to_be_bytes());
    bytes
}
#[test]
fn frames_are_bounded_and_leave_the_following_status_record_intact() {
    let pixels = image();
    let mut wire = Vec::new();
    write_frame(&mut wire, &pixels).unwrap();
    wire.extend_from_slice(b"PHOENIX_WORKSHOP_TEST next\n");
    let mut input = Cursor::new(wire);
    let mut header = String::new();
    input.read_line(&mut header).unwrap();
    assert_eq!(read_frame(&header, &mut input).unwrap(), pixels);
    let mut status = String::new();
    input.read_line(&mut status).unwrap();
    assert_eq!(status, "PHOENIX_WORKSHOP_TEST next\n");
    for header in [
        format!("{PREFIX}{WIDTH} {HEIGHT} {}", MAX_BYTES + 1),
        format!("{PREFIX}0 {HEIGHT} 33"),
        format!("{PREFIX}{WIDTH} {HEIGHT} -1"),
        format!("{PREFIX}{WIDTH} {HEIGHT} 33 extra"),
    ] {
        assert!(read_frame(&header, &mut Cursor::new(&pixels)).is_err());
    }
    assert!(read_frame(
        &format!("{PREFIX}{WIDTH} {HEIGHT} 33"),
        &mut Cursor::new(&pixels[..32])
    )
    .is_err());
    assert!(read_frame(
        &format!("{PREFIX}{WIDTH} {HEIGHT} 33"),
        &mut Cursor::new(vec![0; 33])
    )
    .is_err());
}
#[test]
fn presentation_round_trip_refuses_unknown_authority_channels_and_large_records() {
    use crate::workshop::test_protocol::TestPresentation;
    let mut payload = TestPresentation {
        sequence: 3,
        role_presets: "[]".into(),
        ..Default::default()
    };
    payload
        .channels
        .insert("gm_entity".into(), "{\"entities\":[]}".into());
    let json = crate::core::codec::to_json(&payload).unwrap();
    let mut wire = Vec::new();
    write_presentation(&mut wire, &json).unwrap();
    let mut input = Cursor::new(wire);
    let mut header = String::new();
    input.read_line(&mut header).unwrap();
    let bytes = read_presentation(&header, &mut input).unwrap();
    assert_eq!(
        crate::core::codec::from_json_bytes::<crate::workshop::test_protocol::TestPresentation>(
            &bytes
        )
        .unwrap(),
        payload
    );
    payload
        .channels
        .insert("execute_script".into(), "bad".into());
    let json = crate::core::codec::to_json(&payload).unwrap();
    assert!(read_presentation(
        &format!("{PRESENTATION_PREFIX}{}", json.len()),
        &mut Cursor::new(json.as_bytes())
    )
    .is_err());
    assert!(read_presentation(
        &format!("{PRESENTATION_PREFIX}{}", MAX_BYTES + 1),
        &mut Cursor::new([])
    )
    .is_err());
    assert!(read_presentation(
        &format!("{PRESENTATION_PREFIX}20"),
        &mut Cursor::new(b"short")
    )
    .is_err());
}
#[test]
fn latest_frame_replaces_one_resource_and_drop_retires_the_generation() {
    let documents = HostedDocuments::default();
    let first = FrameRoute::new(documents.clone(), "http://127.0.0.1:7");
    let publish = first.publisher();
    publish(vec![1]);
    publish(vec![2]);
    assert_eq!(documents.len(), 1);
    assert_eq!(documents.resource(&first.path).unwrap().body.as_ref(), &[2]);
    assert!(!documents.resource(&first.path).unwrap().immutable);
    let second = FrameRoute::new(documents.clone(), "http://127.0.0.1:7");
    assert_ne!(first.url, second.url);
    drop(publish);
    drop(first);
    assert_eq!(documents.len(), 0);
}
