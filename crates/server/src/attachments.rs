//! Images des sessions : pièces jointes (réduites à 1568 px, PNG ou JPEG 90)
//! dans `<data>/sessions/<id>/attachments/`, et images produites par l'agent
//! sous le topic (png, jpeg, webp, gif ; pas de SVG).

use std::{io::Cursor, path::PathBuf};

use axum::{
    Json,
    body::Bytes,
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use image::{GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{error::ApiError, sandboxes, sessions, state::AppState, workspace};

/// Taille maximale acceptée (l'UI refuse au-delà).
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Limite du corps de la route (au-delà : 413 d'axum).
pub const BODY_LIMIT: usize = 21 * 1024 * 1024;
const MAX_SIDE: u32 = 1568;

fn dir(state: &AppState, session_id: &str) -> PathBuf {
    sandboxes::session_dir(state, session_id).join("attachments")
}

/// Nom de fichier produit par `upload` (`<uuid>.png|jpg`).
fn valid_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    uuid::Uuid::parse_str(stem).is_ok() && (ext == "png" || ext == "jpg")
}

pub fn exists(state: &AppState, session_id: &str, name: &str) -> bool {
    valid_name(name) && dir(state, session_id).join(name).is_file()
}

pub fn url(session_id: &str, name: &str) -> String {
    format!("/api/sessions/{session_id}/attachments/{name}")
}

fn format_of(headers: &HeaderMap) -> Option<ImageFormat> {
    let ct = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    match ct.split(';').next()?.trim() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/webp" => Some(ImageFormat::WebP),
        "image/gif" => Some(ImageFormat::Gif),
        _ => None,
    }
}

pub async fn upload(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    sessions::row(&state, &id).await?;
    if body.len() > MAX_BYTES {
        return Err(ApiError::AttachmentInvalid(true));
    }
    let fmt = format_of(&headers).ok_or(ApiError::AttachmentInvalid(false))?;
    let out = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, &'static str, u32, u32), ApiError> {
        let img = image::load_from_memory_with_format(&body, fmt).map_err(|_| ApiError::AttachmentInvalid(false))?;
        let (w, h) = img.dimensions();
        let img = if w.max(h) > MAX_SIDE {
            img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let (w, h) = img.dimensions();
        let mut buf = Vec::new();
        let ext = if img.color().has_alpha() {
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| ApiError::Internal(e.into()))?;
            "png"
        } else {
            let rgb = img.to_rgb8();
            JpegEncoder::new_with_quality(&mut buf, 90)
                .encode_image(&rgb)
                .map_err(|e| ApiError::Internal(e.into()))?;
            "jpg"
        };
        Ok((buf, ext, w, h))
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))??;
    let (bytes, ext, w, h) = out;
    let d = dir(&state, &id);
    std::fs::create_dir_all(&d)?;
    let name = format!("{}.{ext}", uuid::Uuid::new_v4());
    std::fs::write(d.join(&name), bytes)?;
    Ok(Json(
        json!({"id": name, "name": name, "url": url(&id, &name), "width": w, "height": h}),
    ))
}

fn image_type(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn send_file(path: &std::path::Path, ctype: &'static str) -> Result<Response, ApiError> {
    let bytes = std::fs::read(path).map_err(|_| ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        bytes,
    )
        .into_response())
}

pub async fn serve(
    State(state): State<AppState>,
    UrlPath((id, name)): UrlPath<(String, String)>,
) -> Result<Response, ApiError> {
    if !valid_name(&name) {
        return Err(ApiError::NotFound);
    }
    let p = dir(&state, &id).join(&name);
    send_file(&p, image_type(&p).ok_or(ApiError::NotFound)?)
}

#[derive(Deserialize)]
pub struct FileQuery {
    path: String,
}

/// Image sous le topic (chemin relatif, `/workspace/…` ou chemin du topic),
/// liens symboliques résolus, rien hors du topic.
pub async fn file(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<FileQuery>,
) -> Result<Response, ApiError> {
    let r = sessions::row(&state, &id).await?;
    let topic = workspace::topic(&state, r.topic_id).await?;
    let root = topic.dir.canonicalize().map_err(|_| ApiError::NotFound)?;
    let raw = q.path.trim();
    let rel = raw
        .strip_prefix("/workspace/")
        .map(str::to_string)
        .or_else(|| {
            std::path::Path::new(raw)
                .strip_prefix(&topic.dir)
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_else(|| raw.to_string());
    if std::path::Path::new(&rel).is_absolute() {
        return Err(ApiError::NotFound);
    }
    let p = root.join(rel).canonicalize().map_err(|_| ApiError::NotFound)?;
    if !p.starts_with(&root) || !p.is_file() {
        return Err(ApiError::NotFound);
    }
    let ctype = image_type(&p).ok_or(ApiError::NotFound)?;
    send_file(&p, ctype)
}
