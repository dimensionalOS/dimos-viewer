//! No-op native backend used when embedded native webviews are not compiled in.

use re_viewer_context::ViewId;

use crate::backend::WebViewBounds;

#[derive(Debug, Default)]
pub struct NativeWebViewBackend;

pub struct NativeWebView;

#[derive(Debug)]
pub enum NativeWebViewError {
    Unavailable,
}

impl std::fmt::Display for NativeWebViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("native webview support is not available"),
        }
    }
}

impl std::error::Error for NativeWebViewError {}

impl NativeWebViewBackend {
    pub(crate) fn create_child(
        &self,
        _url: &str,
        _bounds: WebViewBounds,
    ) -> Result<NativeWebView, NativeWebViewError> {
        let _ = self;
        Err(NativeWebViewError::Unavailable)
    }
}

pub(crate) fn has_native_parent_window() -> bool {
    false
}

pub(crate) fn insert(_view_id: ViewId, _webview: NativeWebView) {}

pub(crate) fn destroy(_view_id: ViewId) {}

pub(crate) fn set_bounds(_view_id: ViewId, _bounds: WebViewBounds) {}

pub(crate) fn set_visible(_view_id: ViewId, _visible: bool) {}

pub(crate) fn go_back(_view_id: ViewId) {}

pub(crate) fn go_forward(_view_id: ViewId) {}

pub(crate) fn reload(_view_id: ViewId) {}

pub(crate) fn navigate_to(_view_id: ViewId, _url: &str) {}
