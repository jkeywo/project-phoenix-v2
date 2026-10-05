pub use phoenix_simulation::workshop::*;
#[cfg(all(target_arch = "wasm32", feature = "server"))]
#[path = "../workshop_host/test_browser.rs"]
pub(crate) mod test_browser;
