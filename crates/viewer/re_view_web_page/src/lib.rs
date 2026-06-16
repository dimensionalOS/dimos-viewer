//! Native Web Page View.

mod backend;
mod lifecycle;
#[cfg(all(not(target_arch = "wasm32"), feature = "native_webview"))]
#[path = "native_backend.rs"]
pub mod native_backend;
#[cfg(not(all(not(target_arch = "wasm32"), feature = "native_webview")))]
#[path = "native_backend_stub.rs"]
pub mod native_backend;
#[cfg(debug_assertions)]
pub mod testing;
mod url_policy;
mod view_class;

pub use view_class::WebPageView;
