//! A browser document surface and a deterministic recording adapter.
/// Why a surface refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneSurfaceError {
    /// The document could not be navigated to.
    Load(String),
    /// A script could not be evaluated, or threw. The ordinary cause is a page
    /// whose own scripts have not finished running.
    Script(String),
    /// A frame copy failed (issue #1404): a surface that could not be locked, a
    /// buffer of the wrong length, a view that has stopped answering. Ordinarily
    /// transient — a run of them in a row is the crashed-view signal.
    Frame(String),
}

impl std::fmt::Display for PaneSurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaneSurfaceError::Load(detail) => write!(f, "load failed: {detail}"),
            PaneSurfaceError::Script(detail) => write!(f, "script failed: {detail}"),
            // Bare, unlike its siblings: every caller of a frame copy already
            // says "frame copy failed" and how many in a row, so a prefix here
            // would only repeat them.
            PaneSurfaceError::Frame(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for PaneSurfaceError {}

/// One pane's document, as the frame loop sees it.
pub trait PaneSurface {
    /// Navigate to `url`.
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError>;
    /// Whether the document has finished loading and may be pushed to.
    fn is_ready(&self) -> bool;
    /// Hand the page one encoded `ServerMessage`.
    fn push(&mut self, json: &str) -> Result<(), PaneSurfaceError>;
    /// Collect every record the page has queued since the last call.
    fn drain(&mut self) -> Vec<String>;
}

/// A [`PaneSurface`] that records instead of rendering.
///
/// The frame loop's own test double: it never needs an SDK, so everything
/// the game bridge pump decides is checked by the ordinary `cargo test` CI runs rather
/// than only by a human on a Windows machine with a GPU.
#[derive(Debug, Default)]
pub struct RecordingSurface {
    /// Whether the document reports itself loaded.
    pub ready: bool,
    /// Every URL this surface was asked to load.
    pub loaded: Vec<String>,
    /// Every script that was successfully pushed.
    pub pushed: Vec<String>,
    /// Records handed back on the next [`PaneSurface::drain`].
    pub queued_records: Vec<String>,
    /// Number of pushes to fail before the first success. Models the window
    /// between "the document loaded" and "its modules have run".
    pub failing_pushes: usize,
}

impl RecordingSurface {
    /// A surface whose document is already loaded.
    pub fn ready() -> Self {
        Self {
            ready: true,
            ..Default::default()
        }
    }

    /// Queue a record for the page to hand back.
    pub fn queue_record(&mut self, record: impl Into<String>) {
        self.queued_records.push(record.into());
    }
}

impl PaneSurface for RecordingSurface {
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
        self.loaded.push(url.to_string());
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn push(&mut self, json: &str) -> Result<(), PaneSurfaceError> {
        if self.failing_pushes > 0 {
            self.failing_pushes -= 1;
            return Err(PaneSurfaceError::Script(
                "window.__phoenixPaneApply is not a function".to_string(),
            ));
        }
        self.pushed.push(json.to_string());
        Ok(())
    }

    fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.queued_records)
    }
}
