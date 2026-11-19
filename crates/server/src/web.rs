//! The web UI, embedded from `web/dist` at compile time (read from disk in
//! debug builds). Copied from Presencia (`server/src/web.rs`, see NOTICE.md).
//! Paths outside `/api` are looked up here first; `None` lets the router's
//! fallback answer 404 or 405. No single-page fallback.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// `allow_missing`: debug builds run without the UI (build.rs refuses a
/// release build without it).
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Dist;

/// Served at `/` when the UI was not built (debug builds only).
const PLACEHOLDER: &str = "<!doctype html><html lang=\"fr\"><meta charset=\"utf-8\"><title>EngineX</title>\
                           <p>Interface non construite : web/dist/index.html manque (voir le README).</p></html>\n";

/// Vite names its build files after their content, so they never change.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else if path.starts_with("fonts/") {
        "public, max-age=2592000"
    } else {
        "no-cache"
    }
}

fn content_type(mime: &str) -> HeaderValue {
    let text = mime.starts_with("text/") || mime == "application/javascript";
    let value = if text {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };
    HeaderValue::from_str(&value).unwrap_or(HeaderValue::from_static("application/octet-stream"))
}

/// RFC 9110 weak comparison: `W/` is ignored, `*` matches anything (a proxy
/// that compresses on the fly turns our strong ETag into a weak one).
fn matches(if_none_match: &[u8], etag: &str) -> bool {
    if_none_match.split(|b| *b == b',').any(|t| {
        let t = t.trim_ascii();
        t == b"*" || t.strip_prefix(b"W/").unwrap_or(t) == etag.as_bytes()
    })
}

/// The embedded file for a GET or HEAD on `path`, or `None` to let the router
/// answer.
pub fn serve(method: &Method, path: &str, headers: &HeaderMap) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    let rel = match path.strip_prefix('/')? {
        "" => "index.html",
        p => p,
    };
    // rust-embed only knows its own files, but in debug builds it reads from
    // disk: never hand it a path that could climb out of the folder.
    if rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") || rel.contains('\\') {
        return None;
    }
    let (data, mime, etag, encoding) = match Dist::get(rel) {
        Some(file) => {
            let mime = file.metadata.mimetype().to_string();
            // Version compressée au build (`.br`, `.gz`) si le client l'accepte.
            let accept = headers
                .get(header::ACCEPT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let packed = [("br", "br"), ("gzip", "gz")]
                .into_iter()
                .filter(|(enc, _)| accepts(accept, enc))
                .find_map(|(enc, ext)| Dist::get(&format!("{rel}.{ext}")).map(|f| (enc, f)));
            match packed {
                Some((enc, f)) => {
                    let hash = f.metadata.sha256_hash();
                    let etag = format!("\"{}-{enc}\"", hex(&hash[..16]));
                    (f.data, mime, etag, Some(enc))
                }
                None => {
                    let hash = file.metadata.sha256_hash();
                    let etag = format!("\"{}\"", hex(&hash[..16]));
                    (file.data, mime, etag, None)
                }
            }
        }
        None if rel == "index.html" => (
            PLACEHOLDER.as_bytes().into(),
            "text/html".to_string(),
            "\"placeholder\"".to_string(),
            None,
        ),
        None => return None,
    };

    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|v| matches(v.as_bytes(), &etag));
    let mut res = if fresh {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        // Borrowed static data in release builds: no copy.
        let mut res = Response::new(Body::from(data));
        res.headers_mut().insert(header::CONTENT_TYPE, content_type(&mime));
        res
    };
    let h = res.headers_mut();
    if let Some(enc) = encoding {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
    }
    h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("hex etag"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control(rel)));
    Some(res)
}

/// `Accept-Encoding` contient `enc` sans `q=0`.
fn accepts(accept: &str, enc: &str) -> bool {
    accept.split(',').any(|t| {
        let mut it = t.split(';');
        it.next().is_some_and(|n| n.trim().eq_ignore_ascii_case(enc))
            && !it.any(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
                    .is_some_and(|q| q == 0.0)
            })
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_types_and_etags() {
        assert_eq!(
            cache_control("assets/index-3fa2.js"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control("fonts/inter-latin.woff2"), "public, max-age=2592000");
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("js/app.js"), "no-cache");
        assert_eq!(content_type("text/css"), "text/css; charset=utf-8");
        assert_eq!(content_type("font/woff2"), "font/woff2");
        assert!(matches(b"\"a\"", "\"a\""));
        assert!(accepts("gzip, deflate, br, zstd", "br"));
        assert!(accepts("gzip;q=0.5", "gzip"));
        assert!(!accepts("br;q=0, gzip", "br"));
        assert!(!accepts("identity", "gzip"));
        assert!(matches(b"W/\"a\"", "\"a\""));
        assert!(matches(b"\"b\", W/\"a\"", "\"a\""));
        assert!(matches(b"*", "\"a\""));
        assert!(!matches(b"\"b\"", "\"a\""));
    }
}
