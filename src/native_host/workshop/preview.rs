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

    pub(super) fn delivery(&self) -> (HostedDocuments, String) {
        (self.documents.clone(), self.origin.clone())
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
#[path = "preview_tests.rs"]
mod tests;
