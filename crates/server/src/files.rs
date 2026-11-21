//! Panneau Fichiers et accès au disque sous un dossier écrit par une sandbox.
//!
//! La sandbox peut remplacer un dossier par un lien symbolique entre une
//! vérification et une ouverture : EngineX n'ouvre donc rien par
//! « canonicaliser puis ouvrir ». Tout passe par `open_beneath` (`openat2`
//! avec `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`)
//! depuis un descripteur du dossier de travail.

use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
};

use axum::{
    Json,
    body::Body,
    extract::{Path as UrlPath, Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{error::ApiError, sessions, snapshots, state::AppState};

/// Entrées rendues au plus par dossier.
pub const MAX_ENTRIES: usize = 1000;
/// Texte rendu au plus par `file/text`.
pub const MAX_TEXT: usize = 512 * 1024;

fn cstr(s: &str) -> std::io::Result<CString> {
    CString::new(s).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

/// Ouvre `dir` (dossier de travail) une fois, sans suivre de lien final.
pub fn open_root(dir: &Path) -> std::io::Result<OwnedFd> {
    let c = cstr(&dir.display().to_string())?;
    // SAFETY: chemin C valide ; le descripteur rendu est possédé par OwnedFd.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd vient d'être ouvert et n'appartient à personne d'autre.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Ouvre `rel` sous `root` sans jamais suivre de lien symbolique ni sortir
/// du dossier (`openat2`). `rel` vide = le dossier lui-même.
pub fn open_beneath(root: &OwnedFd, rel: &str, flags: i32) -> std::io::Result<OwnedFd> {
    let path = if rel.is_empty() { "." } else { rel };
    let c = cstr(path)?;
    // SAFETY: open_how est une structure C simple, tout à zéro est valide.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (flags | libc::O_CLOEXEC) as u64;
    how.resolve = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_MAGICLINKS;
    // SAFETY: appel système openat2 avec une structure open_how valide.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            c.as_ptr(),
            &how as *const libc::open_how,
            std::mem::size_of::<libc::open_how>(),
        )
    } as i32;
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd vient d'être ouvert et n'appartient à personne d'autre.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Supprime le fichier (ou le lien) `rel` sous `dir` : dossier parent ouvert
/// par `open_beneath`, puis `unlinkat` (un lien est supprimé comme lien).
pub fn remove_beneath(dir: &Path, rel: &str) -> std::io::Result<()> {
    let root = open_root(dir)?;
    let (parent, name) = match rel.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", rel),
    };
    let pfd = open_beneath(&root, parent, libc::O_PATH | libc::O_DIRECTORY)?;
    let c = cstr(name)?;
    // SAFETY: descripteur de dossier valide, nom C valide.
    let r = unsafe { libc::unlinkat(pfd.as_raw_fd(), c.as_ptr(), 0) };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e);
        }
    }
    Ok(())
}

/// Lit au plus `max` octets de `rel` sous `dir` ; `None` si absent, lien ou
/// pas un fichier.
pub fn read_beneath(dir: &Path, rel: &str, max: usize) -> Option<Vec<u8>> {
    let root = open_root(dir).ok()?;
    let fd = open_beneath(&root, rel, libc::O_RDONLY | libc::O_NOFOLLOW).ok()?;
    let f = File::from(fd);
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    f.take(max as u64 + 1).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Type de `rel` sous `dir` sans suivre de lien : `Some(true)` dossier,
/// `Some(false)` fichier, `None` absent ou autre.
pub fn kind_beneath(dir: &Path, rel: &str) -> Option<bool> {
    let root = open_root(dir).ok()?;
    let fd = open_beneath(&root, rel, libc::O_PATH | libc::O_NOFOLLOW).ok()?;
    let m = File::from(fd).metadata().ok()?;
    if m.is_dir() {
        Some(true)
    } else if m.is_file() {
        Some(false)
    } else {
        None
    }
}

/// Écrit `data` dans `rel` sous `dir` (fichier créé, jamais écrasé), en
/// créant les dossiers manquants sans suivre de lien.
pub fn create_beneath(dir: &Path, rel: &str, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let root = open_root(dir)?;
    let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let mut cur = String::new();
    for comp in parent.split('/').filter(|c| !c.is_empty()) {
        let pfd = open_beneath(&root, &cur, libc::O_PATH | libc::O_DIRECTORY)?;
        let c = cstr(comp)?;
        // SAFETY: descripteur de dossier valide, nom C valide.
        let r = unsafe { libc::mkdirat(pfd.as_raw_fd(), c.as_ptr(), 0o755) };
        if r != 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(e);
            }
        }
        cur = if cur.is_empty() {
            comp.to_string()
        } else {
            format!("{cur}/{comp}")
        };
    }
    let pfd = open_beneath(&root, &cur, libc::O_PATH | libc::O_DIRECTORY)?;
    let c = cstr(name)?;
    // SAFETY: descripteur de dossier valide, nom C valide ; O_EXCL refuse un lien.
    let fd = unsafe {
        libc::openat(
            pfd.as_raw_fd(),
            c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd vient d'être ouvert.
    let mut f = unsafe { File::from_raw_fd(fd) };
    f.write_all(data)
}

/// Chemin demandé → chemin relatif au dossier de travail (`/workspace/…`,
/// chemin absolu du dossier, ou relatif).
fn relative(raw: &str, work: &Path) -> Option<String> {
    let raw = raw.trim();
    let rel = if raw == "/workspace" {
        String::new()
    } else if let Some(r) = raw.strip_prefix("/workspace/") {
        r.to_string()
    } else if let Ok(r) = Path::new(raw).strip_prefix(work) {
        r.display().to_string()
    } else {
        raw.to_string()
    };
    let rel = rel.trim_matches('/').to_string();
    (rel.is_empty() || snapshots::valid_rel(&rel)).then_some(rel)
}

async fn work_of(state: &AppState, id: &str) -> Result<std::path::PathBuf, ApiError> {
    let r = sessions::row(state, id).await?;
    Ok(snapshots::folder(state, &r).await?.work)
}

#[derive(Deserialize)]
pub struct PathQuery {
    #[serde(default)]
    path: String,
    download: Option<String>,
}

/// Un niveau de l'arbre : dossiers d'abord, `.git` caché.
pub async fn tree(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let work = work_of(&state, &id).await?;
    let rel = relative(&q.path, &work).ok_or(ApiError::NotFound)?;
    let out = tokio::task::spawn_blocking(move || -> Result<Value, ApiError> {
        let root = open_root(&work).map_err(|_| ApiError::NotFound)?;
        let fd = open_beneath(&root, &rel, libc::O_RDONLY | libc::O_DIRECTORY).map_err(|_| ApiError::NotFound)?;
        let mut entries = list_dir(fd)?;
        let truncated = entries.len() > MAX_ENTRIES;
        entries.sort_by(|a, b| {
            let da = a["kind"] != "dir";
            let db = b["kind"] != "dir";
            da.cmp(&db).then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
            })
        });
        entries.truncate(MAX_ENTRIES);
        Ok(json!({"path": rel, "entries": entries, "truncated": truncated}))
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))??;
    Ok(Json(out))
}

/// Entrées d'un dossier ouvert (`fstatat` sans suivre les liens).
pub(crate) fn list_dir(fd: OwnedFd) -> Result<Vec<Value>, ApiError> {
    let raw = fd.as_raw_fd();
    // SAFETY: fdopendir prend possession du descripteur dupliqué.
    let dup = unsafe { libc::dup(raw) };
    if dup < 0 {
        return Err(ApiError::NotFound);
    }
    // SAFETY: dup est un descripteur de dossier valide.
    let dir = unsafe { libc::fdopendir(dup) };
    if dir.is_null() {
        // SAFETY: dup n'a pas été pris par fdopendir.
        unsafe { libc::close(dup) };
        return Err(ApiError::NotFound);
    }
    let mut out = Vec::new();
    loop {
        // SAFETY: dir est un DIR* valide jusqu'à closedir.
        let ent = unsafe { libc::readdir(dir) };
        if ent.is_null() {
            break;
        }
        // SAFETY: d_name est une chaîne C terminée par NUL.
        let name = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) };
        let Ok(name_s) = name.to_str() else { continue };
        if name_s == "." || name_s == ".." || name_s == ".git" {
            continue;
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: descripteur valide, nom C valide, st écrit par l'appel.
        let r = unsafe { libc::fstatat(raw, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        if r != 0 {
            continue;
        }
        let kind = match st.st_mode & libc::S_IFMT {
            libc::S_IFDIR => "dir",
            libc::S_IFLNK => "link",
            libc::S_IFREG => "file",
            _ => continue,
        };
        out.push(json!({"name": name_s, "kind": kind, "size": st.st_size, "mtime": st.st_mtime}));
        if out.len() > MAX_ENTRIES * 4 {
            break;
        }
    }
    // SAFETY: dir vient de fdopendir.
    unsafe { libc::closedir(dir) };
    Ok(out)
}

/// Type affiché en ligne, ou `None` (téléchargé).
fn inline_type(rel: &str) -> Option<&'static str> {
    let ext = rel.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        _ => return None,
    })
}

const CSP: &str = "default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'; sandbox";
const CSP_PDF: &str = "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; frame-ancestors 'self'";

/// Supprime des fichiers ou dossiers du dossier de travail de la session
/// (panneau Fichiers). Refusé pendant un tour ; `.git` reste intouchable.
pub async fn delete(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    crate::error::ApiJson(b): crate::error::ApiJson<crate::wsfiles::PathsBody>,
) -> Result<Json<Value>, ApiError> {
    let r = sessions::row(&state, &id).await?;
    let paths = crate::wsfiles::clean_paths(&b.paths)?;
    if paths.iter().any(|p| p == ".git" || p.starts_with(".git/")) {
        return Err(ApiError::BadRequest);
    }
    if crate::bridge::hub(&state, &id).busy() {
        return Err(ApiError::TurnInProgress);
    }
    let f = snapshots::folder(&state, &r).await?;
    let _g = snapshots::lock(&state, &f).await;
    let n = crate::wsfiles::delete_beneath(f.work.clone(), paths).await?;
    Ok(Json(json!({ "deleted": n })))
}

/// Zip de dossiers ou fichiers du dossier de travail de la session (panneau
/// Fichiers), ou du dossier entier si `paths` est vide ; `.git` exclu.
pub async fn zip(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<crate::wsfiles::ZipQuery>,
) -> Result<Response, ApiError> {
    let work = work_of(&state, &id).await?;
    let raw = q.list();
    let paths = if raw.is_empty() {
        Vec::new()
    } else {
        crate::wsfiles::clean_paths(&raw)?
    };
    if paths.iter().any(|p| p == ".git" || p.starts_with(".git/")) {
        return Err(ApiError::BadRequest);
    }
    let name = match paths.as_slice() {
        [one] => one.rsplit('/').next().unwrap_or(one).to_string(),
        [] => work
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => "fichiers".into(),
    };
    crate::wsfiles::zip_beneath(&state, work, paths, &name, true).await
}

/// Fichier brut, envoyé en flux depuis le descripteur ouvert.
pub async fn file(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<PathQuery>,
) -> Result<Response, ApiError> {
    let work = work_of(&state, &id).await?;
    let rel = relative(&q.path, &work)
        .filter(|r| !r.is_empty())
        .ok_or(ApiError::NotFound)?;
    serve(work, rel, matches!(q.download.as_deref(), Some("1" | "true"))).await
}

/// Envoie `rel` sous `dir` en flux (en ligne si le type s'y prête, sinon en
/// pièce jointe), avec une CSP qui n'exécute rien.
pub async fn serve(dir: std::path::PathBuf, rel: String, download: bool) -> Result<Response, ApiError> {
    let (f, len) = tokio::task::spawn_blocking({
        let rel = rel.clone();
        move || -> Result<(File, u64), ApiError> {
            let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
            let fd = open_beneath(&root, &rel, libc::O_RDONLY | libc::O_NOFOLLOW).map_err(|_| ApiError::NotFound)?;
            let f = File::from(fd);
            let m = f.metadata().map_err(|_| ApiError::NotFound)?;
            if !m.is_file() {
                return Err(ApiError::NotFound);
            }
            Ok((f, m.len()))
        }
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))??;
    let ctype = if download { None } else { inline_type(&rel) };
    let name = rel
        .rsplit('/')
        .next()
        .unwrap_or("fichier")
        .replace(['"', '\\', '\r', '\n'], "_");
    let stream = tokio_util::io::ReaderStream::new(tokio::fs::File::from_std(f));
    let mut res = Body::from_stream(stream).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    match ctype {
        Some(t) => {
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(t));
            let csp = if t == "application/pdf" { CSP_PDF } else { CSP };
            h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
        }
        None => {
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
            let ascii: String = name.chars().map(|c| if c.is_ascii() { c } else { '_' }).collect();
            if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{ascii}\"")) {
                h.insert(header::CONTENT_DISPOSITION, v);
            }
        }
    }
    Ok(res)
}

/// Aperçu texte, masqué : `{text, truncated, binary}`.
pub async fn text(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let work = work_of(&state, &id).await?;
    let rel = relative(&q.path, &work)
        .filter(|r| !r.is_empty())
        .ok_or(ApiError::NotFound)?;
    let bytes = tokio::task::spawn_blocking(move || read_beneath(&work, &rel, MAX_TEXT))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .ok_or(ApiError::NotFound)?;
    let truncated = bytes.len() > MAX_TEXT;
    let bytes = &bytes[..bytes.len().min(MAX_TEXT)];
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Ok(Json(json!({"text": "", "truncated": truncated, "binary": true})));
    }
    let text = String::from_utf8_lossy(bytes);
    let text = state.masker().mask(&text).into_owned();
    Ok(Json(json!({"text": text, "truncated": truncated, "binary": false})))
}
