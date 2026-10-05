use crate::{Grid, Move};
use phoenix_runtime::HostSlot;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct GridHost(Grid);
#[wasm_bindgen]
impl GridHost {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(Grid::new(HostSlot(1), []))
    }
    pub fn move_piece(&mut self, dx: i32, dy: i32) -> bool {
        self.0.submit(Move { dx, dy }).is_some()
    }
    pub fn tick(&mut self) {
        self.0.advance();
    }
    pub fn state(&self) -> Result<String, JsValue> {
        crate::codec::encode(&crate::protocol::Output::state(&self.0)).map_err(|e| e.into())
    }
    pub fn checkpoint(&self) -> Result<String, JsValue> {
        self.0.checkpoint().map_err(|e| e.into())
    }
    pub fn restore(&mut self, text: &str) -> Result<(), JsValue> {
        self.0.restore(text).map_err(|e| e.into())
    }
}
impl Default for GridHost {
    fn default() -> Self {
        Self::new()
    }
}
