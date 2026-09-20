//! Loopback-only immutable resources for one native Workshop preview capture.
//! The JSON bridge carries paths and a nonce, never the merged asset bytes.
use crate::{
    delivery::{http, serve::HostedDocuments},
    workshop::provider::{preview_snapshot::PreviewSnapshot, Response},
};

const PREFIX: &str = "/workshop-preview-capture/";

pub struct PreviewRoutes {
    documents: HostedDocuments,
    origin: String,
    active: Option<ActiveCapture>,
}

struct ActiveCapture {
    id: String,
    routes: Vec<String>,
}

impl PreviewRoutes {
    pub fn new(documents: HostedDocuments, origin: String) -> Self {
        Self {
            documents,
            origin: origin.trim_end_matches('/').to_owned(),
            active: None,
        }
    }

    #[allow(clippy::disallowed_methods)] // Unpredictable URL capability, never simulation identity.
    pub fn publish(&mut self, snapshot: PreviewSnapshot) -> Response {
        self.retire();
        let id = uuid::Uuid::new_v4().to_string();
        let base_path = format!("{PREFIX}{id}/");
        let paths: Vec<String> = snapshot.files.keys().cloned().collect();
        let mut routes = Vec::with_capacity(paths.len());
        for (index, bytes) in snapshot.files.into_values().enumerate() {
            let route = format!("{base_path}{index}");
            self.documents.publish_bytes(
                route.clone(),
                bytes,
                http::content_type_for(&paths[index]),
                true,
            );
            routes.push(route);
        }
        self.active = Some(ActiveCapture {
            id: id.clone(),
            routes,
        });
        Response::Preview {
            capture: id,
            base_url: format!("{}{base_path}", self.origin),
            paths,
            revision: snapshot.revision,
            selection: snapshot.selection,
        }
    }

    pub fn release(&mut self, id: &str) {
        if self.active.as_ref().is_some_and(|active| active.id == id) {
            self.retire();
        }
    }

    pub fn retire(&mut self) {
        if let Some(active) = self.active.take() {
            for route in active.routes {
                self.documents.withdraw(&route);
            }
        }
    }

    #[cfg(test)]
    pub fn active(&self) -> Option<&str> {
        self.active.as_ref().map(|active| active.id.as_str())
    }
}

impl Drop for PreviewRoutes {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
mod tests {
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
}
