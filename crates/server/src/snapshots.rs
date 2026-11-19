//! Instantanés : un dépôt git fantôme par dossier de travail, hors de tout
//! montage de sandbox (`<data>/snapshots/topic-<id>.git` ou
//! `session-<id>.git` pour une copie séparée).
//!
//! Aucune commande git ne tourne dans le processus EngineX : `git add` fait
//! `lstat` puis `open` sans `O_NOFOLLOW`, une sandbox pourrait donc faire
//! entrer un fichier d'EngineX dans un instantané. Chaque opération est un
//! script fixe lancé dans un conteneur d'aide (`Backend::run_helper`) qui ne
//! voit que le dossier de travail (`$WORK`) et le dépôt fantôme (`$SNAP`).
//!
//! Verrou de dossier : chaque fonction publique prend le verrou du dossier
//! une fois (le verrou tokio n'est pas réentrant) ; un retour arrière le
//! garde du début à la fin.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use enginex_sandbox::HelperSpec;
use serde_json::{Value, json};

use crate::{error::ApiError, sandboxes, sessions::SessionRow, state::AppState, workspace};

/// Délai d'un passage git.
pub const PASS_TIMEOUT: Duration = Duration::from_secs(120);
/// Entrées gardées dans `chat_turns.changes`.
pub const MAX_CHANGES: usize = 500;
/// Taille maximale d'un diff de fichier rendu à l'UI.
pub const MAX_DIFF: usize = 512 * 1024;

/// Début de chaque script : dépôt créé au besoin, variables et options `-c`
/// communes (transmises aux git enfants, elles l'emportent sur la config d'un
/// dépôt imbriqué), aucune identité venue de l'environnement.
macro_rules! prelude {
    () => {
        r#"set -eu
if [ ! -f "$SNAP/HEAD" ]; then
  git init -q --bare --template= "$SNAP"
  mkdir -p "$SNAP/info"
  printf '%s\n' node_modules/ .venv/ venv/ target/ __pycache__/ .pytest_cache/ .mypy_cache/ \
    .next/ .nuxt/ .turbo/ .parcel-cache/ .cache/ .gradle/ .enginex/cache/ > "$SNAP/info/exclude"
fi
export GIT_DIR="$SNAP" GIT_WORK_TREE="$WORK" GIT_INDEX_FILE="$SNAP/index" \
  GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null HOME="$SNAP" GIT_TERMINAL_PROMPT=0 \
  GIT_LITERAL_PATHSPECS=1 GIT_OPTIONAL_LOCKS=0
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
g() { git -c core.bare=false -c core.hooksPath=/dev/null -c core.fsmonitor=false \
  -c core.quotepath=off -c core.autocrlf=false -c commit.gpgsign=false -c diff.ignoreSubmodules=all \
  -c status.submoduleSummary=false -c submodule.recurse=false -c core.untrackedCache=false \
  -c gc.autoDetach=false -c user.name=EngineX -c user.email=enginex@localhost \
  -c core.excludesFile="$SNAP/enginex-skip" "$@"; }
"#
    };
}

/// Prend un instantané de `$WORK` : gros fichiers exclus, dépôts imbriqués
/// sans commit ignorés le temps du passage, `add -A`, commit sans parent,
/// refs posées. Écrit le sha sur sa propre ligne.
macro_rules! snap_core {
    () => {
        r#": > "$SNAP/enginex-skip"
big=$(g ls-files -z --others --modified --exclude-standard | xargs -0 -r sh -c '
  for p; do s=$(stat -c %s -- "$WORK/$p" 2>/dev/null || echo 0)
    if [ "$s" -gt 52428800 ]; then printf "%s\n" "$p"; fi; done' _ || true)
if [ -n "$big" ]; then
  printf '%s\n' "$big" | while IFS= read -r p; do
    printf '/%s\n' "$p" >> "$SNAP/info/exclude"
    g rm --cached -q --ignore-unmatch -- "$p" > /dev/null
  done
fi
n=0
t=0
while ! err=$(g add -A 2>&1 > /dev/null); do
  p=$(printf '%s\n' "$err" | sed -n "s/^error: '\(.*\)' does not have a commit checked out$/\1/p" | head -n 1)
  if [ -n "$p" ] && [ "$n" -lt 50 ]; then
    printf '/%s\n' "${p%/}" >> "$SNAP/enginex-skip"
    n=$((n + 1))
  elif [ "$t" -lt 10 ] && printf '%s\n' "$err" | grep -q 'No such file or directory'; then
    # Fichier supprimé pendant le passage (compilation en cours) : on recommence.
    t=$((t + 1))
    sleep 0.2
  else
    printf '%s\n' "$err" >&2; exit 1
  fi
done
sha=$(g commit-tree "$(g write-tree)" -m "$LABEL")
if [ -n "${REF:-}" ]; then g update-ref "$REF" "$sha"; fi
if [ -n "${REF2:-}" ]; then g update-ref "$REF2" "$SHA2"; fi
printf '%s\n' "$sha"
"#
    };
}

const SNAPSHOT: &str = concat!(
    prelude!(),
    snap_core!(),
    r#"if [ -n "${BASE:-}" ]; then
  g diff --name-status -z --no-renames --ignore-submodules=all "$BASE" "$sha"
  printf -- '--numstat--\0'
  g diff --numstat -z --no-renames --ignore-submodules=all "$BASE" "$sha"
fi
"#
);

/// Instantané `undo` puis retour à `$SRC` : `read-tree -m -u` (fusion à deux
/// arbres) supprime ce qui manque dans `$SRC` et réécrit le reste ; git ne
/// suit pas un lien symbolique de dossier.
const RESTORE: &str = concat!(
    prelude!(),
    snap_core!(),
    r#"g read-tree -m -u "$sha" "$SRC"
"#
);

/// Contenu actuel, dans `$POST` et dans `$PRE`, du fichier `$P`.
const FILE_STATE: &str = concat!(
    prelude!(),
    r#"f="$WORK/$P"
if [ -L "$f" ]; then cur=$(printf '%s' "$(readlink -- "$f")" | g hash-object --stdin)
elif [ -f "$f" ]; then cur=$(g hash-object --path="$P" -- "$f")
else cur=absent; fi
post=$(g rev-parse -q --verify "$POST:$P" 2> /dev/null || echo absent)
pre=$(g rev-parse -q --verify "$PRE:$P" 2> /dev/null || echo absent)
printf '%s\n%s\n%s\n' "$cur" "$post" "$pre"
"#
);

/// Instantané `undo` puis `$P` remis comme dans `$PRE` (s'il y existe).
const UNDO_FILE: &str = concat!(
    prelude!(),
    snap_core!(),
    r#"if [ "$INPRE" = 1 ]; then g restore --source="$PRE" --worktree -- "$P"; fi
"#
);

const DIFF: &str = concat!(
    prelude!(),
    r#"g diff --numstat --no-renames --ignore-submodules=all "$A" "$B" -- "$P" | head -n 1
printf -- '--diff--\n'
g diff --no-color --no-ext-diff --no-textconv --no-renames --ignore-submodules=all "$A" "$B" -- "$P" | head -c 524289
"#
);

const SET_REF: &str = concat!(prelude!(), "g update-ref \"$REF\" \"$SHA\"\n");

const GC: &str = concat!(prelude!(), "g gc --auto --quiet\n");

const DROP_REFS: &str = concat!(
    prelude!(),
    r#"g for-each-ref --format='%(refname)' "refs/enginex/$SID/" | while IFS= read -r r; do g update-ref -d "$r"; done
g gc --auto --quiet
"#
);

// ---- Dossiers et verrous ----

/// Dossier de travail d'une session et son dépôt fantôme.
#[derive(Clone, Debug)]
pub struct Folder {
    pub session_id: String,
    pub work: PathBuf,
    pub snap: PathBuf,
}

/// Dossier de travail d'une session : sa copie séparée, sinon son topic.
pub async fn folder(state: &AppState, row: &SessionRow) -> Result<Folder, ApiError> {
    let (work, snap) = if row.copy_branch.is_some() {
        (copy_dir(state, &row.id), session_repo(state, &row.id))
    } else {
        let t = workspace::topic(state, row.topic_id).await?;
        (t.dir, topic_repo(state, row.topic_id))
    };
    Ok(Folder {
        session_id: row.id.clone(),
        work,
        snap,
    })
}

pub fn copy_dir(state: &AppState, session_id: &str) -> PathBuf {
    sandboxes::session_dir(state, session_id).join("work")
}

pub fn topic_repo(state: &AppState, topic_id: i64) -> PathBuf {
    state
        .config()
        .data_dir
        .join("snapshots")
        .join(format!("topic-{topic_id}.git"))
}

pub fn session_repo(state: &AppState, session_id: &str) -> PathBuf {
    state
        .config()
        .data_dir
        .join("snapshots")
        .join(format!("session-{session_id}.git"))
}

/// Un verrou async par dossier de travail (clé : chemin canonique).
#[derive(Default)]
pub struct FolderLocks(Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>);

/// Verrou d'un dossier tenu : les fonctions `*_locked` le demandent.
pub struct FolderGuard(#[allow(dead_code)] tokio::sync::OwnedMutexGuard<()>);

fn key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Attend puis prend le verrou du dossier.
pub async fn lock(state: &AppState, f: &Folder) -> FolderGuard {
    lock_dir(state, &f.work).await
}

/// Verrou d'un dossier de travail donné par son chemin (page Git).
pub async fn lock_dir(state: &AppState, dir: &Path) -> FolderGuard {
    let m = state
        .0
        .folder_locks
        .0
        .lock()
        .expect("verrou")
        .entry(key(dir))
        .or_default()
        .clone();
    FolderGuard(m.lock_owned().await)
}

// ---- Passages git ----

fn ref_name(session_id: &str, name: &str) -> String {
    format!("refs/enginex/{session_id}/{name}")
}

pub fn pre_ref(session_id: &str, turn_id: &str) -> String {
    ref_name(session_id, &format!("{turn_id}-pre"))
}

pub fn post_ref(session_id: &str, turn_id: &str) -> String {
    ref_name(session_id, &format!("{turn_id}-post"))
}

pub fn green_ref(session_id: &str) -> String {
    ref_name(session_id, "green")
}

fn undo_ref(session_id: &str) -> String {
    ref_name(session_id, &format!("undo-{}", crate::state::now_ms()))
}

/// Lance un script dans le conteneur d'aide ; stdout si code 0.
async fn run(
    state: &AppState,
    _g: &FolderGuard,
    f: &Folder,
    script: &'static str,
    env: Vec<(&str, String)>,
) -> anyhow::Result<Vec<u8>> {
    // Session supprimée entre-temps : ni dossier recréé, ni montage absent
    // (Docker le créerait).
    if !f.work.is_dir() {
        anyhow::bail!("dossier de travail absent : {}", f.work.display());
    }
    std::fs::create_dir_all(&f.snap)?;
    let backend = state
        .0
        .sandboxes
        .backend()
        .map_err(|_| anyhow::anyhow!("sandboxes indisponibles"))?;
    let spec = HelperSpec {
        session_id: f.session_id.clone(),
        owner: state.0.sandboxes.instance().to_string(),
        binds: vec![(f.work.clone(), "WORK", true), (f.snap.clone(), "SNAP", true)],
        script,
        env: env.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        network: false,
    };
    let out = backend.run_helper(&spec, PASS_TIMEOUT).await?;
    if out.code != 0 {
        anyhow::bail!("git (code {}) : {}", out.code, state.masker().mask(out.stderr.trim()));
    }
    Ok(out.stdout)
}

fn first_line(out: &[u8]) -> (String, &[u8]) {
    let end = out.iter().position(|b| *b == b'\n').unwrap_or(out.len());
    let line = String::from_utf8_lossy(&out[..end]).trim().to_string();
    (line, out.get(end + 1..).unwrap_or(&[]))
}

/// Instantané pris.
#[derive(Clone, Debug)]
pub struct Snap {
    pub sha: String,
    /// Modifications depuis `base` (si demandé) : `{files, truncated}`.
    pub changes: Option<Value>,
}

/// Options d'un instantané.
#[derive(Default)]
pub struct SnapOpts {
    /// Ref posée sur l'instantané.
    pub ref_name: Option<String>,
    /// Instantané de comparaison (modifications rendues).
    pub base: Option<String>,
    /// Seconde ref posée sur un sha déjà connu (le `pre` d'un tour).
    pub extra_ref: Option<(String, String)>,
}

pub async fn snapshot(state: &AppState, f: &Folder, label: &str, opts: SnapOpts) -> anyhow::Result<Snap> {
    let g = lock(state, f).await;
    snapshot_locked(state, &g, f, label, opts).await
}

pub async fn snapshot_locked(
    state: &AppState,
    g: &FolderGuard,
    f: &Folder,
    label: &str,
    opts: SnapOpts,
) -> anyhow::Result<Snap> {
    let mut env = vec![("LABEL", label.to_string())];
    if let Some(r) = &opts.ref_name {
        env.push(("REF", r.clone()));
    }
    if let Some(b) = &opts.base {
        env.push(("BASE", b.clone()));
    }
    if let Some((r, s)) = &opts.extra_ref {
        env.push(("REF2", r.clone()));
        env.push(("SHA2", s.clone()));
    }
    let out = run(state, g, f, SNAPSHOT, env).await?;
    let (sha, rest) = first_line(&out);
    if sha.len() < 40 {
        anyhow::bail!("instantané sans sha");
    }
    let changes = opts.base.as_ref().map(|_| parse_changes(rest));
    Ok(Snap { sha, changes })
}

/// Sortie `--name-status -z` puis `--numstat -z` → `{files, truncated}`.
pub fn parse_changes(out: &[u8]) -> Value {
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    let mut i = 0;
    let mut order: Vec<String> = Vec::new();
    let mut status: HashMap<String, String> = HashMap::new();
    while i < fields.len() {
        let s = String::from_utf8_lossy(fields[i]).to_string();
        if s == "--numstat--" {
            i += 1;
            break;
        }
        if s.is_empty() {
            i += 1;
            continue;
        }
        let Some(p) = fields.get(i + 1) else { break };
        let p = String::from_utf8_lossy(p).to_string();
        let code = s.chars().next().unwrap_or('M').to_string();
        if !status.contains_key(&p) {
            order.push(p.clone());
        }
        status.insert(p, code);
        i += 2;
    }
    let mut stats: HashMap<String, (Option<u64>, Option<u64>)> = HashMap::new();
    for field in &fields[i.min(fields.len())..] {
        let s = String::from_utf8_lossy(field);
        let mut parts = s.splitn(3, '\t');
        let (Some(a), Some(d), Some(p)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        stats.insert(p.to_string(), (a.parse().ok(), d.parse().ok()));
    }
    let truncated = order.len() > MAX_CHANGES;
    let files: Vec<Value> = order
        .into_iter()
        .take(MAX_CHANGES)
        .map(|p| {
            let (a, d) = stats.get(&p).cloned().unwrap_or((None, None));
            let binary = stats.contains_key(&p) && a.is_none();
            json!({"path": p, "status": status[&p], "added": a.unwrap_or(0), "deleted": d.unwrap_or(0), "binary": binary})
        })
        .collect();
    json!({"files": files, "truncated": truncated})
}

/// Diff d'un fichier entre deux instantanés : `(diff, tronqué, binaire)`.
pub async fn diff(state: &AppState, f: &Folder, a: &str, b: &str, path: &str) -> anyhow::Result<(String, bool, bool)> {
    let g = lock(state, f).await;
    let out = run(
        state,
        &g,
        f,
        DIFF,
        vec![("A", a.into()), ("B", b.into()), ("P", path.into())],
    )
    .await?;
    let (num, rest) = first_line(&out);
    let binary = num.starts_with("-\t-\t");
    let (_, body) = first_line(rest);
    let truncated = body.len() > MAX_DIFF;
    let body = &body[..body.len().min(MAX_DIFF)];
    let text = String::from_utf8_lossy(body);
    Ok((state.masker().mask(&text).into_owned(), truncated, binary))
}

/// Remet le dossier dans l'état de `source` (sha ou ref) ; rend l'instantané
/// d'avant (pour « Rétablir »).
pub async fn restore_to(state: &AppState, f: &Folder, source: &str) -> anyhow::Result<String> {
    let g = lock(state, f).await;
    let out = run(
        state,
        &g,
        f,
        RESTORE,
        vec![
            ("LABEL", "avant retour arrière".into()),
            ("REF", undo_ref(&f.session_id)),
            ("SRC", source.into()),
        ],
    )
    .await?;
    let (sha, _) = first_line(&out);
    Ok(sha)
}

/// Résultat de l'annulation d'un fichier.
pub enum UndoFile {
    /// Fichier remis ; instantané d'avant.
    Done(String),
    ChangedSince,
}

/// Remet `path` comme avant le tour (`pre`), en vérifiant d'abord qu'il n'a
/// pas changé depuis `post` (sauf `force`).
pub async fn undo_file(
    state: &AppState,
    f: &Folder,
    pre: &str,
    post: &str,
    path: &str,
    force: bool,
) -> anyhow::Result<UndoFile> {
    let g = lock(state, f).await;
    let out = run(
        state,
        &g,
        f,
        FILE_STATE,
        vec![("P", path.into()), ("PRE", pre.into()), ("POST", post.into())],
    )
    .await?;
    let text = String::from_utf8_lossy(&out);
    let mut lines = text.lines();
    let (cur, at_post, at_pre) = (
        lines.next().unwrap_or("").trim().to_string(),
        lines.next().unwrap_or("").trim().to_string(),
        lines.next().unwrap_or("").trim().to_string(),
    );
    if cur != at_post && !force {
        return Ok(UndoFile::ChangedSince);
    }
    let in_pre = at_pre != "absent";
    let out = run(
        state,
        &g,
        f,
        UNDO_FILE,
        vec![
            ("LABEL", "avant annulation d'un fichier".into()),
            ("REF", undo_ref(&f.session_id)),
            ("P", path.into()),
            ("PRE", pre.into()),
            ("INPRE", if in_pre { "1" } else { "0" }.into()),
        ],
    )
    .await?;
    let (sha, _) = first_line(&out);
    if !in_pre && cur != "absent" {
        crate::files::remove_beneath(&f.work, path)?;
    }
    Ok(UndoFile::Done(sha))
}

/// Pose la ref `name` sur `sha` (ref `green`).
pub async fn set_ref(state: &AppState, f: &Folder, name: &str, sha: &str) -> anyhow::Result<()> {
    let g = lock(state, f).await;
    run(state, &g, f, SET_REF, vec![("REF", name.into()), ("SHA", sha.into())]).await?;
    Ok(())
}

/// `git gc --auto` en tâche de fond.
pub fn gc_later(state: &AppState, f: &Folder) {
    let (state, f) = (state.clone(), f.clone());
    tokio::spawn(async move {
        let g = lock(&state, &f).await;
        // Session supprimée entre-temps : rien à compacter (ni à recréer).
        if !f.snap.join("HEAD").exists() {
            return;
        }
        if let Err(e) = run(&state, &g, &f, GC, vec![]).await {
            tracing::debug!("git gc : {e}");
        }
    });
}

/// Supprime les refs d'une session (et compacte).
pub async fn drop_refs(state: &AppState, f: &Folder) -> anyhow::Result<()> {
    if !f.snap.join("HEAD").exists() {
        return Ok(());
    }
    let g = lock(state, f).await;
    run(state, &g, f, DROP_REFS, vec![("SID", f.session_id.clone())]).await?;
    Ok(())
}

/// Chemin relatif acceptable : pas vide, pas absolu, pas de `..`, pas de
/// `.git`, pas de NUL.
pub fn valid_rel(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 4096
        && !p.starts_with('/')
        && !p.contains('\0')
        && p.split('/')
            .all(|c| !c.is_empty() && c != "." && c != ".." && c != ".git")
}

/// Garde-fou des refs et shas passés aux scripts.
pub fn valid_rev(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
        && !s.contains("..")
}
