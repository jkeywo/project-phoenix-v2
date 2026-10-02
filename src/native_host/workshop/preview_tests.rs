use super::*;
use crate::workshop::{
    provider::preview_snapshot::PreviewSnapshot, test_protocol::PreviewSelection,
};
use std::collections::BTreeMap;

#[test]
fn each_capture_has_immutable_members_and_retires_the_previous_generation() {
    let documents = HostedDocuments::default();
    let mut routes = PreviewRoutes::new(documents.clone(), "http://127.0.0.1:7".into());
    let response = routes.publish(PreviewSnapshot {
        files: BTreeMap::from([
            ("assets/entities/star.toml".into(), b"[star]\n".to_vec()),
            ("assets/models/only.glb".into(), vec![1, 2, 3]),
        ]),
        selection: PreviewSelection {
            entity: Some("assets/entities/star.toml".into()),
            gizmos: true,
            ..Default::default()
        },
        revision: "first".into(),
    });
    let Response::Preview {
        capture,
        base_url,
        paths,
        ..
    } = response
    else {
        panic!("expected preview descriptor")
    };
    assert_eq!(
        paths,
        ["assets/entities/star.toml", "assets/models/only.glb"]
    );
    assert!(base_url.ends_with(&format!("/{capture}/")));
    let first = format!("/workshop-preview-capture/{capture}/0");
    let resource = documents.resource(&first).unwrap();
    assert!(resource.immutable);
    assert_eq!(resource.body.as_ref(), b"[star]\n");

    let previous = capture;
    let next = routes.publish(PreviewSnapshot {
        files: BTreeMap::from([("assets/models/next.glb".into(), vec![9])]),
        selection: PreviewSelection {
            model: Some("assets/models/next.glb".into()),
            ..Default::default()
        },
        revision: "second".into(),
    });
    assert!(documents.resource(&first).is_none());
    routes.release(&previous);
    assert!(
        routes.active().is_some(),
        "a late old release cannot retire the current capture"
    );
    let Response::Preview { capture, .. } = next else {
        unreachable!()
    };
    routes.release(&capture);
    assert!(routes.active().is_none());
    assert!(documents.is_empty());
}
