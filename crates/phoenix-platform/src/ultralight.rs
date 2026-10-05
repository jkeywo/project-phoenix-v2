//! Concrete Ultralight surface lifecycle and pixel copy, without game roles.
use crate::{
    frames::{FrameRect, PixelMode},
    surface::{PaneSurface, PaneSurfaceError},
};
use vellum_ultralight::runtime::{PaneSession, PaneSpec, UltralightPane, UltralightRuntime};

pub struct UltralightSurface {
    view: UltralightPane,
    loaded: bool,
    pixels: PixelMode,
    drain_script: String,
    render_visible: bool,
    applied_visibility: Option<bool>,
}
impl UltralightSurface {
    pub fn new(view: UltralightPane, pixels: PixelMode, drain_script: String) -> Self {
        Self {
            view,
            loaded: false,
            pixels,
            drain_script,
            render_visible: true,
            applied_visibility: None,
        }
    }
    pub fn view_mut(&mut self) -> &mut UltralightPane {
        &mut self.view
    }
    pub fn set_visible(&mut self, visible: bool) {
        self.render_visible = visible;
        self.apply_visibility();
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.view.resize(width, height);
    }
    pub fn refresh_loaded(&mut self) -> bool {
        if !self.loaded && !self.view.is_loading() {
            self.loaded = true;
        }
        self.apply_visibility();
        self.loaded
    }
    fn apply_visibility(&mut self) {
        if !self.loaded || self.applied_visibility == Some(self.render_visible) {
            return;
        }
        // The SDK renders every dirty view, including ones Bevy does not
        // composite. Hide its document at the paint source, not merely the
        // copied texture. The DOM, subscriptions and reliable queues survive.
        let script = format!(
            "(()=>{{const root=document.documentElement;if(!root)throw Error('document loading');let style=document.getElementById('phoenix-native-visibility');if(!style){{style=document.createElement('style');style.id='phoenix-native-visibility';style.textContent='html[data-phoenix-native-hidden]{{display:none!important}}';document.head.appendChild(style);}}root.toggleAttribute('data-phoenix-native-hidden',{});}})()",
            !self.render_visible
        );
        if self.view.evaluate(&script).is_ok() {
            self.applied_visibility = Some(self.render_visible);
        }
    }
    pub fn copy_frame(
        &mut self,
        dst: &mut [u8],
        force: bool,
    ) -> Result<Option<FrameRect>, PaneSurfaceError> {
        // The transparent HUD takes the straight-alpha copy; every opaque
        // surface is moved verbatim into its BGRA texture — see
        // `PixelMode::texture_format`, which is minted from the same predicate.
        let copied = if self.pixels == PixelMode::StraightRgba {
            self.view.copy_frame(dst, force)
        } else {
            self.view.copy_frame_bgra(dst, force)
        };
        copied
            .map(|rect| rect.map(FrameRect::from))
            .map_err(|e| PaneSurfaceError::Frame(e.to_string()))
    }
}
impl PaneSurface for UltralightSurface {
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
        self.loaded = false;
        self.applied_visibility = None;
        self.view
            .load_url(url)
            .map_err(|e| PaneSurfaceError::Load(e.to_string()))
    }

    fn is_ready(&self) -> bool {
        self.loaded
    }

    fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
        self.view
            .evaluate(script)
            .map(|_| ())
            .map_err(|e| PaneSurfaceError::Script(e.to_string()))
    }

    fn drain(&mut self) -> Vec<String> {
        match self.view.evaluate(&self.drain_script) {
            Ok(drained) => vellum_ultralight::bridge::split_records(&drained)
                .into_iter()
                .map(str::to_string)
                .collect(),
            // A throw here is the same ordinary case a failed push is: the
            // page's own scripts have not run yet, so the drain function does
            // not exist. Next frame.
            Err(_) => Vec::new(),
        }
    }
}
/// Renderer ownership stays on the caller's owning thread; views must drop first.
pub struct UltralightDriver {
    runtime: UltralightRuntime,
}
impl UltralightDriver {
    pub fn new(runtime: UltralightRuntime) -> Self {
        Self { runtime }
    }
    pub fn update(&mut self) {
        self.runtime.update();
    }
    pub fn render(&mut self) {
        self.runtime.render();
    }
    pub fn create(
        &mut self,
        geometry: (u32, u32, f64),
        pixels: PixelMode,
        session: String,
        drain_script: String,
        url: &str,
    ) -> Result<UltralightSurface, PaneSurfaceError> {
        let spec = PaneSpec {
            width: geometry.0,
            height: geometry.1,
            device_scale: geometry.2,
            transparent: pixels == PixelMode::StraightRgba,
            session: Some(PaneSession::ephemeral(session)),
        };
        let view = self
            .runtime
            .create_pane(&spec)
            .map_err(|e| PaneSurfaceError::Load(e.to_string()))?;
        let mut surface = UltralightSurface::new(view, pixels, drain_script);
        surface.load(url)?;
        Ok(surface)
    }
}
