//! Page Fichiers : tout `<data>/workspace` (projets, topics, skills, tâches).
//!
//! Les topics sont écrits par les sandboxes : comme le panneau de session,
//! tout passe par des descripteurs (`files::open_beneath`, `openat`,
//! `renameat2`, `unlinkat`) sans jamais suivre de lien symbolique. Les dossiers
//! gérés par EngineX (projets, topics, mémoire) ne se créent, ne se déplacent
//! et ne se suppriment pas ici.

use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{ApiError, ApiJson},
    files::{self, open_beneath, open_root},
    snapshots,
    state::AppState,
    system::{set_setting, setting},
};

/// Texte ouvert au plus dans l'éditeur.
pub const MAX_EDIT: usize = 2 * 1024 * 1024;
/// Fichier envoyé au plus.
pub const UPLOAD_LIMIT: usize = 200 * 1024 * 1024;
/// Chemins par suppression ou archive.
const MAX_PATHS: usize = 500;
pub const DISK_KEY: &str = "disk.alert_pct";
const DISK_DEFAULT: i64 = 90;

pub fn root(state: &AppState) -> PathBuf {
    state.config().data_dir.join("workspace")
}

fn cstr(s: &str) -> std::io::Result<CString> {
    CString::new(s).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

/// Chemin relatif à la racine (vide = racine) ; `..`, `.git`, absolu refusés.
fn rel_of(raw: &str) -> Result<String, ApiError> {
    let r = raw.trim().trim_matches('/');
    if r.is_empty() {
        return Ok(String::new());
    }
    if snapshots::valid_rel(r) {
        Ok(r.to_string())
    } else {
        Err(ApiError::NotFound)
    }
}

/// Dossier géré par EngineX : racine, `projects`, un projet, ses `topics`, un
/// topic, la mémoire, les skills.
pub fn managed(rel: &str) -> bool {
    let p: Vec<&str> = if rel.is_empty() {
        vec![]
    } else {
        rel.split('/').collect()
    };
    matches!(
        p.as_slice(),
        [] | ["projects"]
            | ["memory"]
            | ["skills"]
            | ["projects", _]
            | ["projects", _, "topics"]
            | ["projects", _, "topics", _]
    )
}

/// Mémoire : lue ici, écrite par la page Mémoire (révisions).
pub fn read_only(rel: &str) -> bool {
    rel == "memory" || rel.starts_with("memory/")
}

/// Cible d'une écriture (création, destination d'un déplacement).
fn writable(rel: &str) -> Result<(), ApiError> {
    if rel.is_empty() || managed(rel) || read_only(rel) {
        return Err(ApiError::Protected);
    }
    Ok(())
}

fn split(rel: &str) -> (&str, &str) {
    rel.rsplit_once('/').unwrap_or(("", rel))
}

fn parent_fd(root: &OwnedFd, rel: &str) -> std::io::Result<(OwnedFd, CString)> {
    let (parent, name) = split(rel);
    let pfd = open_beneath(root, parent, libc::O_PATH | libc::O_DIRECTORY)?;
    Ok((pfd, cstr(name)?))
}

fn io_err(e: std::io::Error) -> ApiError {
    match e.raw_os_error() {
        Some(libc::ENOENT) | Some(libc::ELOOP) | Some(libc::EXDEV) | Some(libc::ENOTDIR) => ApiError::NotFound,
        Some(libc::EEXIST) => ApiError::NameTaken,
        Some(libc::ENOTEMPTY) => ApiError::NotEmpty,
        Some(libc::EINVAL) => ApiError::BadRequest,
        Some(libc::ENOSPC) | Some(libc::EDQUOT) => ApiError::DiskFull,
        _ => ApiError::Internal(e.into()),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ApiError> + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
}

/// Version d'un fichier pour l'éditeur : mtime en nanosecondes.
fn version_of(m: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    m.mtime() * 1_000_000_000 + m.mtime_nsec()
}

#[derive(Deserialize)]
pub struct PathQuery {
    #[serde(default)]
    path: String,
    download: Option<String>,
}

/// Un niveau de l'arbre.
pub async fn tree(State(state): State<AppState>, Query(q): Query<PathQuery>) -> Result<Json<Value>, ApiError> {
    let dir = root(&state);
    let rel = rel_of(&q.path)?;
    let out = blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let fd = open_beneath(&root, &rel, libc::O_RDONLY | libc::O_DIRECTORY).map_err(|_| ApiError::NotFound)?;
        let mut entries = files::list_dir(fd)?;
        let truncated = entries.len() > files::MAX_ENTRIES;
        entries.sort_by(|a, b| {
            (a["kind"] != "dir").cmp(&(b["kind"] != "dir")).then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
            })
        });
        entries.truncate(files::MAX_ENTRIES);
        for e in &mut entries {
            let name = e["name"].as_str().unwrap_or("").to_string();
            let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            e["managed"] = json!(managed(&child));
            e["readonly"] = json!(read_only(&child));
        }
        Ok(json!({"path": rel, "entries": entries, "truncated": truncated, "readonly": read_only(&rel)}))
    })
    .await?;
    Ok(Json(out))
}

pub async fn raw(State(state): State<AppState>, Query(q): Query<PathQuery>) -> Result<Response, ApiError> {
    let rel = rel_of(&q.path)?;
    if rel.is_empty() {
        return Err(ApiError::NotFound);
    }
    files::serve(root(&state), rel, matches!(q.download.as_deref(), Some("1" | "true"))).await
}

/// Texte pour l'éditeur, non masqué (Michael édite ses propres fichiers) :
/// `{text, truncated, binary, version, readonly}`.
pub async fn get_text(State(state): State<AppState>, Query(q): Query<PathQuery>) -> Result<Json<Value>, ApiError> {
    let dir = root(&state);
    let rel = rel_of(&q.path)?;
    if rel.is_empty() {
        return Err(ApiError::NotFound);
    }
    blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let fd = open_beneath(&root, &rel, libc::O_RDONLY | libc::O_NOFOLLOW).map_err(|_| ApiError::NotFound)?;
        let f = File::from(fd);
        let m = f.metadata().map_err(|_| ApiError::NotFound)?;
        if !m.is_file() {
            return Err(ApiError::NotFound);
        }
        let mut buf = Vec::new();
        f.take(MAX_EDIT as u64 + 1).read_to_end(&mut buf)?;
        let truncated = buf.len() > MAX_EDIT;
        buf.truncate(MAX_EDIT);
        let binary = buf[..buf.len().min(8192)].contains(&0) || std::str::from_utf8(&buf).is_err() && !truncated;
        let text = if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&buf).into_owned()
        };
        Ok(Json(json!({
            "text": text,
            "truncated": truncated,
            "binary": binary,
            "version": version_of(&m),
            "size": m.len(),
            "readonly": read_only(&rel) || truncated || binary,
        })))
    })
    .await
}

#[derive(Deserialize)]
pub struct TextBody {
    path: String,
    text: String,
    /// Version lue ; absente = création (refusée si le fichier existe).
    version: Option<i64>,
}

/// Écrit un fichier temporaire à côté de `rel` puis le renomme : jamais de
/// fichier à moitié écrit, jamais de lien suivi.
fn write_atomic(root: &OwnedFd, rel: &str, data: &mut dyn Read, expect: Option<Option<i64>>) -> Result<i64, ApiError> {
    let (pfd, name) = parent_fd(root, rel).map_err(io_err)?;
    let tmp = format!(".enginex-{}.tmp", uuid::Uuid::new_v4().simple());
    let ctmp = cstr(&tmp)?;
    // SAFETY: descripteur de dossier valide, nom C valide.
    let fd = unsafe {
        libc::openat(
            pfd.as_raw_fd(),
            ctmp.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd < 0 {
        return Err(io_err(std::io::Error::last_os_error()));
    }
    // SAFETY: fd vient d'être ouvert.
    let mut f = unsafe { File::from_raw_fd(fd) };
    let unlink_tmp = || {
        // SAFETY: descripteur de dossier valide, nom C valide.
        unsafe { libc::unlinkat(pfd.as_raw_fd(), ctmp.as_ptr(), 0) };
    };
    let copied = std::io::copy(data, &mut f).and_then(|_| f.flush());
    if let Err(e) = copied {
        unlink_tmp();
        return Err(io_err(e));
    }
    // Contrôle de version juste avant le renommage.
    let current = current_version(&pfd, &name);
    let flags = match expect {
        None => 0,
        Some(None) => libc::RENAME_NOREPLACE,
        Some(Some(v)) => {
            if current != Some(Some(v)) {
                unlink_tmp();
                return Err(ApiError::FileChanged);
            }
            0
        }
    };
    if matches!(current, Some(None)) {
        // Ce n'est pas un fichier ordinaire (dossier, lien) : on ne remplace pas.
        unlink_tmp();
        return Err(ApiError::NameTaken);
    }
    let r = if flags == 0 {
        rename_at(&pfd, &ctmp, &pfd, &name)
    } else {
        rename_noreplace(&pfd, &ctmp, &pfd, &name, true)
    };
    if let Err(e) = r {
        unlink_tmp();
        return Err(io_err(e));
    }
    let m = f.metadata()?;
    Ok(version_of(&m))
}

fn rename_at(fa: &OwnedFd, na: &CString, fb: &OwnedFd, nb: &CString) -> std::io::Result<()> {
    // SAFETY: descripteurs de dossier valides, noms C valides.
    if unsafe { libc::renameat(fa.as_raw_fd(), na.as_ptr(), fb.as_raw_fd(), nb.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Renomme sans écraser. Les systèmes sans `RENAME_NOREPLACE` (FUSE, dont le
/// partage utilisateur d'Unraid) passent par `linkat` (atomique, fichier
/// seulement) ou, pour un dossier, par un contrôle d'absence juste avant.
fn rename_noreplace(fa: &OwnedFd, na: &CString, fb: &OwnedFd, nb: &CString, is_file: bool) -> std::io::Result<()> {
    // SAFETY: descripteurs de dossier valides, noms C valides.
    let r = unsafe {
        libc::renameat2(
            fa.as_raw_fd(),
            na.as_ptr(),
            fb.as_raw_fd(),
            nb.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if r == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    if !matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS)) {
        return Err(e);
    }
    if is_file {
        // SAFETY: descripteurs de dossier valides, noms C valides.
        if unsafe { libc::linkat(fa.as_raw_fd(), na.as_ptr(), fb.as_raw_fd(), nb.as_ptr(), 0) } == 0 {
            // SAFETY: idem.
            unsafe { libc::unlinkat(fa.as_raw_fd(), na.as_ptr(), 0) };
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EPERM) && e.raw_os_error() != Some(libc::ENOTSUP) {
            return Err(e);
        }
    }
    if current_version(fb, nb).is_some() {
        return Err(std::io::Error::from_raw_os_error(libc::EEXIST));
    }
    rename_at(fa, na, fb, nb)
}

/// `None` absent, `Some(None)` autre chose qu'un fichier, `Some(Some(v))`
/// fichier en version `v`.
fn current_version(pfd: &OwnedFd, name: &CString) -> Option<Option<i64>> {
    // SAFETY: structure C simple, remplie par fstatat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: descripteur valide, nom C valide.
    let r = unsafe { libc::fstatat(pfd.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
    if r != 0 {
        return None;
    }
    if st.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Some(None);
    }
    Some(Some(st.st_mtime * 1_000_000_000 + st.st_mtime_nsec))
}

pub async fn put_text(State(state): State<AppState>, ApiJson(b): ApiJson<TextBody>) -> Result<Json<Value>, ApiError> {
    let rel = rel_of(&b.path)?;
    writable(&rel)?;
    if b.text.len() > MAX_EDIT {
        return Err(ApiError::TooLarge);
    }
    let dir = root(&state);
    let version = blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        write_atomic(&root, &rel, &mut b.text.as_bytes(), Some(b.version))
    })
    .await?;
    Ok(Json(json!({"version": version})))
}

#[derive(Deserialize)]
pub struct UploadQuery {
    path: String,
    #[serde(default)]
    overwrite: bool,
}

/// Envoi d'un fichier (corps brut, en flux vers un fichier temporaire).
pub async fn upload(
    State(state): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let rel = rel_of(&q.path)?;
    writable(&rel)?;
    // Le corps est lu ici (async) et passé au fil bloquant par un tube borné.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes_chunk::Chunk, ()>>(16);
    let dir = root(&state);
    let overwrite = q.overwrite;
    let rel2 = rel.clone();
    let writer = tokio::task::spawn_blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let mut r = bytes_chunk::ChanReader::new(rx);
        let res = write_atomic(&root, &rel2, &mut r, if overwrite { None } else { Some(None) });
        if r.failed {
            return Err(ApiError::BadRequest);
        }
        res
    });
    let mut stream = body.into_data_stream();
    let mut total = 0usize;
    let mut too_large = false;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(c) => {
                total += c.len();
                if total > UPLOAD_LIMIT {
                    too_large = true;
                    let _ = tx.send(Err(())).await;
                    break;
                }
                if tx.send(Ok(c.to_vec())).await.is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = tx.send(Err(())).await;
                break;
            }
        }
    }
    drop(tx);
    let r = writer.await.map_err(|e| ApiError::Internal(e.into()))?;
    if too_large {
        return Err(ApiError::TooLarge);
    }
    let version = r?;
    Ok(Json(json!({"path": rel, "version": version})))
}

mod bytes_chunk {
    pub type Chunk = Vec<u8>;

    /// Lecteur bloquant sur un canal de morceaux ; `Err` = envoi interrompu.
    pub struct ChanReader {
        rx: tokio::sync::mpsc::Receiver<Result<Chunk, ()>>,
        buf: Chunk,
        pos: usize,
        pub failed: bool,
    }

    impl ChanReader {
        pub fn new(rx: tokio::sync::mpsc::Receiver<Result<Chunk, ()>>) -> Self {
            Self {
                rx,
                buf: Vec::new(),
                pos: 0,
                failed: false,
            }
        }
    }

    impl std::io::Read for ChanReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            while self.pos >= self.buf.len() {
                match self.rx.blocking_recv() {
                    Some(Ok(c)) => {
                        self.buf = c;
                        self.pos = 0;
                    }
                    Some(Err(())) => {
                        self.failed = true;
                        return Err(std::io::Error::other("envoi interrompu"));
                    }
                    None => return Ok(0),
                }
            }
            let n = out.len().min(self.buf.len() - self.pos);
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }
}

#[derive(Deserialize)]
pub struct PathBody {
    path: String,
}

pub async fn mkdir(State(state): State<AppState>, ApiJson(b): ApiJson<PathBody>) -> Result<Json<Value>, ApiError> {
    let rel = rel_of(&b.path)?;
    writable(&rel)?;
    let dir = root(&state);
    blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let (pfd, name) = parent_fd(&root, &rel).map_err(io_err)?;
        // SAFETY: descripteur de dossier valide, nom C valide.
        if unsafe { libc::mkdirat(pfd.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
            return Err(io_err(std::io::Error::last_os_error()));
        }
        Ok(())
    })
    .await?;
    Ok(Json(json!({"path": b.path.trim_matches('/')})))
}

#[derive(Deserialize)]
pub struct MoveBody {
    from: String,
    to: String,
}

/// Renommer ou déplacer, sans jamais écraser.
pub async fn rename(State(state): State<AppState>, ApiJson(b): ApiJson<MoveBody>) -> Result<Json<Value>, ApiError> {
    let from = rel_of(&b.from)?;
    let to = rel_of(&b.to)?;
    writable(&from)?;
    writable(&to)?;
    let dir = root(&state);
    let to2 = to.clone();
    blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let (fa, na) = parent_fd(&root, &from).map_err(io_err)?;
        let (fb, nb) = parent_fd(&root, &to2).map_err(io_err)?;
        rename_noreplace(&fa, &na, &fb, &nb, false).map_err(io_err)
    })
    .await?;
    Ok(Json(json!({"path": to})))
}

#[derive(Deserialize)]
pub struct PathsBody {
    paths: Vec<String>,
}

fn clean_paths(raw: &[String]) -> Result<Vec<String>, ApiError> {
    if raw.is_empty() || raw.len() > MAX_PATHS {
        return Err(ApiError::BadRequest);
    }
    let mut v = raw.iter().map(|p| rel_of(p)).collect::<Result<Vec<_>, _>>()?;
    if v.iter().any(|p| p.is_empty()) {
        return Err(ApiError::BadRequest);
    }
    v.sort();
    v.dedup();
    Ok(v)
}

/// Supprime `name` sous `dirfd`, dossiers compris, sans suivre de lien.
fn remove_at(dirfd: &OwnedFd, name: &CString) -> std::io::Result<()> {
    // SAFETY: structure C simple, remplie par fstatat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: descripteur valide, nom C valide.
    if unsafe { libc::fstatat(dirfd.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if st.st_mode & libc::S_IFMT == libc::S_IFDIR {
        // SAFETY: descripteur valide, nom C valide.
        let fd = unsafe {
            libc::openat(
                dirfd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fd vient d'être ouvert.
        let sub = unsafe { OwnedFd::from_raw_fd(fd) };
        for child in names_in(&sub)? {
            remove_at(&sub, &child)?;
        }
        // SAFETY: descripteur valide, nom C valide.
        if unsafe { libc::unlinkat(dirfd.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    } else if unsafe { libc::unlinkat(dirfd.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        // SAFETY (ci-dessus) : descripteur valide, nom C valide.
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Noms d'un dossier ouvert (sans `.` ni `..`), `.git` compris.
fn names_in(fd: &OwnedFd) -> std::io::Result<Vec<CString>> {
    // SAFETY: dup d'un descripteur valide.
    let dup = unsafe { libc::dup(fd.as_raw_fd()) };
    if dup < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: dup est un descripteur de dossier valide.
    let dir = unsafe { libc::fdopendir(dup) };
    if dir.is_null() {
        // SAFETY: dup n'a pas été pris par fdopendir.
        unsafe { libc::close(dup) };
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: rembobine le flux du descripteur partagé.
    unsafe { libc::rewinddir(dir) };
    let mut out = Vec::new();
    loop {
        // SAFETY: dir est un DIR* valide jusqu'à closedir.
        let ent = unsafe { libc::readdir(dir) };
        if ent.is_null() {
            break;
        }
        // SAFETY: d_name est une chaîne C terminée par NUL.
        let name = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) };
        let b = name.to_bytes();
        if b == b"." || b == b".." {
            continue;
        }
        out.push(name.to_owned());
    }
    // SAFETY: dir vient de fdopendir.
    unsafe { libc::closedir(dir) };
    Ok(out)
}

pub async fn delete(State(state): State<AppState>, ApiJson(b): ApiJson<PathsBody>) -> Result<Json<Value>, ApiError> {
    let paths = clean_paths(&b.paths)?;
    for p in &paths {
        writable(p)?;
    }
    let dir = root(&state);
    let n = blocking(move || {
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let mut n = 0;
        for p in &paths {
            let Ok((pfd, name)) = parent_fd(&root, p) else { continue };
            match remove_at(&pfd, &name) {
                Ok(()) => n += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(e)),
            }
        }
        Ok(n)
    })
    .await?;
    Ok(Json(json!({"deleted": n})))
}

#[derive(Deserialize)]
pub struct ZipQuery {
    /// Chemins séparés par des retours à la ligne.
    paths: String,
}

/// Ajoute `name` (sous `dirfd`) à l'archive sous le nom `arc`, sans suivre
/// de lien (les liens sont ignorés).
fn zip_add(
    z: &mut zip::ZipWriter<File>,
    dirfd: &OwnedFd,
    name: &CString,
    arc: &str,
    opts: zip::write::SimpleFileOptions,
) -> std::io::Result<()> {
    // SAFETY: descripteur valide, nom C valide.
    let fd = unsafe {
        libc::openat(
            dirfd.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        // Lien symbolique (ELOOP) ou disparu : ignoré.
        return if matches!(e.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOENT)) {
            Ok(())
        } else {
            Err(e)
        };
    }
    // SAFETY: fd vient d'être ouvert.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let f = File::from(owned.try_clone()?);
    let m = f.metadata()?;
    if m.is_dir() {
        z.add_directory(format!("{arc}/"), opts)
            .map_err(std::io::Error::other)?;
        for child in names_in(&owned)? {
            let cn = child.to_string_lossy().into_owned();
            zip_add(z, &owned, &child, &format!("{arc}/{cn}"), opts)?;
        }
    } else if m.is_file() {
        z.start_file(arc, opts).map_err(std::io::Error::other)?;
        std::io::copy(&mut &f, z)?;
    }
    Ok(())
}

pub async fn zip(State(state): State<AppState>, Query(q): Query<ZipQuery>) -> Result<Response, ApiError> {
    let raw: Vec<String> = q.paths.lines().map(str::to_string).filter(|s| !s.is_empty()).collect();
    let paths = clean_paths(&raw)?;
    let dir = root(&state);
    let tmp_dir = state.config().data_dir.join("tmp");
    let f = blocking(move || {
        std::fs::create_dir_all(&tmp_dir)?;
        let tmp = tempfile::tempfile_in(&tmp_dir)?;
        let root = open_root(&dir).map_err(|_| ApiError::NotFound)?;
        let mut z = zip::ZipWriter::new(tmp.try_clone()?);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        for p in &paths {
            let (pfd, name) = parent_fd(&root, p).map_err(io_err)?;
            let arc = split(p).1.to_string();
            zip_add(&mut z, &pfd, &name, &arc, opts).map_err(io_err)?;
        }
        z.finish().map_err(|e| ApiError::Internal(e.into()))?;
        Ok(tmp)
    })
    .await?;
    let len = f.metadata()?.len();
    let mut f = tokio::fs::File::from_std(f);
    use tokio::io::AsyncSeekExt;
    f.seek(std::io::SeekFrom::Start(0)).await?;
    let stream = tokio_util::io::ReaderStream::new(f);
    let mut res = Body::from_stream(stream).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"fichiers.zip\""),
    );
    Ok(res)
}

// ---------------------------------------------------------------- disque

/// Taille d'un arbre sans suivre de lien (approximative si l'arbre bouge).
fn du(path: &std::path::Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

static USAGE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

pub async fn alert_pct(state: &AppState) -> i64 {
    setting(state, DISK_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DISK_DEFAULT)
}

pub async fn set_alert_pct(state: &AppState, v: i64) -> Result<(), ApiError> {
    if !(50..=99).contains(&v) {
        return Err(ApiError::BadRequest);
    }
    set_setting(state, DISK_KEY, Some(&v.to_string())).await?;
    Ok(())
}

/// Volume de `/data` et tailles du workspace et des sessions (mises en cache
/// une minute) ; `alert` vrai au-delà du seuil réglé.
pub async fn usage_value(state: &AppState) -> Result<Value, ApiError> {
    let data = state.config().data_dir.clone();
    let sizes = {
        let cached = USAGE.lock().expect("usage").clone();
        match cached {
            Some((t, v)) if t.elapsed() < Duration::from_secs(60) => v,
            _ => {
                let d = data.clone();
                let v = blocking(move || {
                    Ok(json!({
                        "workspace_bytes": du(&d.join("workspace")),
                        "sessions_bytes": du(&d.join("sessions")),
                    }))
                })
                .await?;
                *USAGE.lock().expect("usage") = Some((Instant::now(), v.clone()));
                v
            }
        }
    };
    let c = cstr(&data.display().to_string())?;
    // SAFETY: structure C simple remplie par statvfs.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: chemin C valide.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(ApiError::Internal(std::io::Error::last_os_error().into()));
    }
    let total = st.f_blocks as u64 * st.f_frsize as u64;
    let free = st.f_bavail as u64 * st.f_frsize as u64;
    let used_pct = if total == 0 {
        0.0
    } else {
        (total - free) as f64 * 100.0 / total as f64
    };
    let threshold = alert_pct(state).await;
    Ok(json!({
        "total_bytes": total,
        "free_bytes": free,
        "used_pct": (used_pct * 10.0).round() / 10.0,
        "alert_pct": threshold,
        "alert": used_pct >= threshold as f64,
        "workspace_bytes": sizes["workspace_bytes"],
        "sessions_bytes": sizes["sessions_bytes"],
    }))
}

pub async fn usage(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(usage_value(&state).await?))
}
