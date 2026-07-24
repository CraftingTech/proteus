use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../proteus-ui/dist"]
struct UiAssets;

pub async fn static_handler(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = UiAssets::get(path) {
        return embed_response(path, file.data.as_ref());
    }

    match UiAssets::get("index.html") {
        Some(index) => embed_response("index.html", index.data.as_ref()),
        None => (StatusCode::NOT_FOUND, "UI assets missing").into_response(),
    }
}

fn embed_response(path: &str, bytes: &[u8]) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    ([(header::CONTENT_TYPE, mime.as_ref())], bytes.to_vec()).into_response()
}
