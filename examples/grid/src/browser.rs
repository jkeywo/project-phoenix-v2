use crate::{Grid, Move};
use phoenix_runtime::HostSlot;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct GridHost {
    grid: Grid,
    admission: crate::protocol::BrowserAdmission,
}
#[wasm_bindgen]
impl GridHost {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            grid: Grid::new(HostSlot(1), []),
            admission: Default::default(),
        }
    }
    pub fn open_peer(&mut self) -> String {
        self.admission.open()
    }
    pub fn close_peer(&mut self, handle: &str) {
        self.admission.close(handle);
    }
    pub fn receive(&mut self, handle: &str, raw: &str) -> Result<String, JsValue> {
        self.admission
            .receive(&mut self.grid, handle, raw)
            .and_then(|reply| crate::codec::encode(&reply))
            .map_err(|e| e.into())
    }
    pub fn recipients(&self) -> Result<String, JsValue> {
        crate::codec::encode(&self.admission.recipients()).map_err(|e| e.into())
    }
    pub fn check_stamp(stamp: &str) -> Result<String, JsValue> {
        use phoenix_transport::relay::RelayProtocol;
        let verdict = match crate::protocol::GridProtocol::check_stamp(&(), Some(stamp)) {
            Ok(()) => serde_json::json!({"ok": true}),
            Err(reason) => {
                serde_json::json!({"ok": false, "code": reason.code, "detail": reason.detail})
            }
        };
        crate::codec::encode(&verdict).map_err(|e| e.into())
    }
    pub fn move_piece(&mut self, dx: i32, dy: i32) -> bool {
        self.grid.submit(Move { dx, dy }).is_some()
    }
    pub fn tick(&mut self) {
        self.grid.advance();
    }
    pub fn state(&self) -> Result<String, JsValue> {
        crate::codec::encode(&crate::protocol::Output::state(&self.grid)).map_err(|e| e.into())
    }
    pub fn checkpoint(&self) -> Result<String, JsValue> {
        self.grid.checkpoint().map_err(|e| e.into())
    }
    pub fn restore(&mut self, text: &str) -> Result<(), JsValue> {
        self.grid.restore(text).map_err(|e| e.into())
    }
}
impl Default for GridHost {
    fn default() -> Self {
        Self::new()
    }
}
