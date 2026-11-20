//! # Embedded Web Dashboard Static Assets (`assets_http.rs`)
//!
//! ## Overview
//! Serves embedded PWA HTML pages, stylesheets, ES modules, web manifest, SVG icon,
//! and Service Worker with strict content-type and cache-control headers.
//!
//! ## Search Tags
//! #web-assets, #pwa-manifest, #service-worker, #embedded-static

use super::auth_http::require_auth;
use super::{resolve_ctx, WebAppCtx};
use axum::{
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Extension,
};
use std::sync::Arc;

pub(crate) async fn handle_dashboard_page(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if require_auth(&ctx, &headers).await.is_err() {
        return (StatusCode::SEE_OTHER, [(header::LOCATION, "/dashboard/login")]).into_response();
    }
    let mut res = Html(include_str!("assets/dashboard.html")).into_response();
    if let Ok(hv) = header::HeaderValue::from_str("no-store, no-cache, must-revalidate") {
        res.headers_mut().insert(header::CACHE_CONTROL, hv);
    }
    res
}

pub(crate) async fn handle_login_page() -> Response {
    let mut res = Html(include_str!("assets/login.html")).into_response();
    if let Ok(hv) = header::HeaderValue::from_str("no-store, no-cache, must-revalidate") {
        res.headers_mut().insert(header::CACHE_CONTROL, hv);
    }
    res
}

pub(crate) async fn handle_manifest() -> Response {
    typed_asset("application/manifest+json", include_str!("assets/manifest.webmanifest"))
}

pub(crate) async fn handle_sw() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("service-worker-allowed"), "/dashboard"),
        ],
        include_str!("assets/sw.js"),
    )
        .into_response()
}

pub(crate) async fn handle_icon() -> Response {
    typed_asset("image/svg+xml", include_str!("assets/icon.svg"))
}

pub(crate) async fn handle_asset(Path(file): Path<String>) -> Response {
    match file.as_str() {
        "dashboard.css" => typed_asset("text/css; charset=utf-8", include_str!("assets/dashboard.css")),
        "app.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/app.js")),
        "i18n.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/i18n.js")),
        "chat.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/chat.js")),
        "slash.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/slash.js")),
        "inspector.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/inspector.js")),
        "onboarding.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/onboarding.js")),
        "voice.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/voice.js")),
        "markdown.js" => typed_asset("application/javascript; charset=utf-8", include_str!("assets/markdown.js")),
        _ => (StatusCode::NOT_FOUND, "Asset not found").into_response(),
    }
}

fn typed_asset(mime: &'static str, body: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}
