//! Page Git : dépôts des topics (graphe, branches, avance / retard, détail
//! d'un commit) et actions (fetch, pull, push, changer de branche, fusionner,
//! commit).
//!
//! Le dossier `.git` d'un topic est écrit par la sandbox : sa config peut
//! lancer des commandes (`core.fsmonitor`, filtres, hooks) ou détourner un
//! jeton (`url.*.insteadOf`, `http.sslVerify`, `credential.helper`). Donc :
//! - lecture dans EngineX avec `gix` (aucune commande lancée, aucun filtre) ;
//! - opérations locales dans le conteneur d'aide, sans réseau ni jeton ;
//! - fetch et push dans un dépôt nu créé par le conteneur (config vierge),
//!   qui lit les objets du topic par `alternates`, vers l'URL vérifiée par
//!   EngineX ; le jeton n'est donné qu'à ce dépôt-là.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use axum::{
    Json,
    extract::{Path as UrlPath, Query, State},
};
use enginex_sandbox::{Backend, HelperSpec};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    bridge,
    error::{ApiError, ApiJson},
    forge::{self, Remote},
    sandboxes, sessions, snapshots,
    state::AppState,
    workspace,
};

/// Commits rendus au plus pour le graphe.
const MAX_COMMITS: usize = 1000;
const DEFAULT_COMMITS: usize = 300;
/// Diff rendu au plus pour un commit.
const MAX_DIFF: usize = 512 * 1024;
/// Fichier ignoré dans le diff au-delà.
const MAX_BLOB: usize = 1024 * 1024;
const LOCAL_TIMEOUT: Duration = Duration::from_secs(120);
const NET_TIMEOUT: Duration = Duration::from_secs(600);

// ---------------------------------------------------------------- scripts

/// git sur le dépôt du topic (sans réseau ni jeton) : aucun hook, aucune
/// surveillance, propriétaire non contrôlé (la sandbox écrit en uid 1000).
macro_rules! local_prelude {
    () => {
        r#"set -eu
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null HOME=/tmp GIT_TERMINAL_PROMPT=0
g() { git -c safe.directory='*' -c core.hooksPath=/dev/null -c core.fsmonitor=false \
  -c commit.gpgsign=false -c submodule.recurse=false "$@"; }
cd "$REPO"
"#
    };
}

/// Dépôt nu vierge du conteneur, objets du topic en `alternates`, refs
/// `origin` connues posées pour la négociation ; jeton par un credential
/// helper qui ne lit que `GIT_TOKEN`.
macro_rules! net_prelude {
    () => {
        r#"set -eu
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null HOME=/tmp GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=/bin/false SSH_ASKPASS=/bin/false
b=$(mktemp -d)
git init -q --bare "$b"
printf '%s\n' "$REPO/.git/objects" > "$b/objects/info/alternates"
n() { git --git-dir="$b" -c credential.helper= \
  -c 'credential.helper=!f() { test "$1" = get || exit 0; test -n "${GIT_TOKEN:-}" || exit 0; echo username=x-access-token; echo "password=$GIT_TOKEN"; }; f' \
  -c core.hooksPath=/dev/null -c protocol.file.allow=always -c fetch.unpackLimit=1 -c transfer.unpackLimit=1 "$@"; }
printf '%s' "${HAVES:-}" | while read -r sha ref; do
  if [ -n "$sha" ]; then n update-ref "$ref" "$sha" 2>/dev/null || true; fi
done
"#
    };
}

const FETCH: &str = concat!(
    net_prelude!(),
    r#"n fetch -q --no-tags --prune "$URL" "+refs/heads/*:refs/remotes/origin/*"
mkdir -p "$REPO/.git/objects/pack"
for p in "$b"/objects/pack/*.pack; do
  [ -e "$p" ] || continue
  for x in pack idx; do
    f="${p%.pack}.$x"; base=$(basename "$f")
    [ -e "$REPO/.git/objects/pack/$base" ] || cp "$f" "$REPO/.git/objects/pack/$base"
  done
done
n for-each-ref --format='%(objectname) %(refname)' refs/remotes/origin/
"#
);

const PUSH: &str = concat!(
    net_prelude!(),
    r#"if ! n push --porcelain "$URL" "$SHA:refs/heads/$BRANCH" > "$b/out" 2>&1; then
  if grep -q '^!' "$b/out"; then cat "$b/out" >&2; exit 20; fi
  cat "$b/out" >&2; exit 1
fi
"#
);

/// Opérations locales (`OP`) ; codes : 10 modifications en cours, 11 conflit,
/// 12 pas en avance rapide, 13 rien à valider.
const LOCAL: &str = concat!(
    local_prelude!(),
    r#"clean() { [ -z "$(g status --porcelain --untracked-files=no)" ] || exit 10; }
case "$OP" in
refs)
  printf '%s' "${REFS:-}" | while read -r sha ref; do if [ -n "$sha" ]; then g update-ref "$ref" "$sha"; fi; done
  printf '%s' "${DELS:-}" | while read -r ref; do if [ -n "$ref" ]; then g update-ref -d "$ref"; fi; done
  if [ -n "${UPSTREAM:-}" ]; then
    g config "branch.$UPSTREAM.remote" origin
    g config "branch.$UPSTREAM.merge" "refs/heads/$UPSTREAM"
  fi
  ;;
pull)
  g merge -q --ff-only "$REF" > /dev/null 2>&1 || exit 12
  ;;
checkout)
  clean
  if g show-ref -q --verify "refs/heads/$BRANCH"; then
    g switch -q "$BRANCH"
  else
    g switch -q -c "$BRANCH" --track "origin/$BRANCH"
  fi
  ;;
merge)
  clean
  if ! g merge -q --no-ff --no-edit "$BRANCH" > /dev/null 2>&1; then
    g merge --abort > /dev/null 2>&1 || true
    exit 11
  fi
  ;;
branch)
  g branch "$BRANCH" "$SHA"
  ;;
commit)
  g add -A
  [ -n "$(g status --porcelain)" ] || exit 13
  g commit -q -m "$MSG"
  g rev-parse HEAD
  ;;
*) exit 2 ;;
esac
"#
);

// ---------------------------------------------------------------- dépôts

/// Dossier de travail git : celui d'un topic ou d'une copie séparée.
#[derive(Clone, Debug)]
pub struct Repo {
    pub dir: PathBuf,
    pub project_id: i64,
    /// Nom des conteneurs d'aide (id de session ou `git-<topic>`).
    pub helper_id: String,
    /// Topic (sessions vivantes à vérifier avant de changer de branche).
    pub topic_id: Option<i64>,
}

/// `.git` doit être un vrai dossier (ni lien, ni fichier `gitdir:`).
pub fn has_git_dir(dir: &Path) -> bool {
    crate::files::kind_beneath(dir, ".git") == Some(true)
}

async fn topic_repo(state: &AppState, topic_id: i64) -> Result<Repo, ApiError> {
    let t = workspace::topic(state, topic_id).await?;
    if !has_git_dir(&t.dir) {
        return Err(ApiError::NotGit);
    }
    Ok(Repo {
        dir: t.dir,
        project_id: t.project_id,
        helper_id: format!("git-{topic_id:06}"),
        topic_id: Some(topic_id),
    })
}

pub async fn session_repo(state: &AppState, session_id: &str) -> Result<Repo, ApiError> {
    let r = sessions::row(state, session_id).await?;
    let f = snapshots::folder(state, &r).await?;
    if !has_git_dir(&f.work) {
        return Err(ApiError::NotGit);
    }
    let t = workspace::topic(state, r.topic_id).await?;
    Ok(Repo {
        dir: f.work,
        project_id: t.project_id,
        helper_id: session_id.to_string(),
        topic_id: r.copy_branch.is_none().then_some(r.topic_id),
    })
}

fn open(dir: &Path) -> Result<gix::Repository, ApiError> {
    gix::open_opts(dir.join(".git"), gix::open::Options::isolated())
        .map_err(|e| ApiError::Git("git_failed", format!("dépôt illisible : {e}")))
}

fn gerr(e: impl std::fmt::Display) -> ApiError {
    ApiError::Git("git_failed", e.to_string())
}

/// URL `origin` du dépôt (lue, jamais exécutée).
fn origin_url(repo: &gix::Repository) -> Option<String> {
    repo.config_snapshot()
        .string("remote.origin.url")
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn short(name: &str) -> &str {
    name.strip_prefix("refs/heads/")
        .or_else(|| name.strip_prefix("refs/remotes/"))
        .or_else(|| name.strip_prefix("refs/tags/"))
        .unwrap_or(name)
}

/// Nombre de commits de `a` absents de `b` (borné).
fn count_not_in(repo: &gix::Repository, a: gix::ObjectId, b: gix::ObjectId) -> usize {
    match repo.rev_walk([a]).with_hidden([b]).all() {
        Ok(w) => w.take(10_000).filter(|r| r.is_ok()).count(),
        Err(_) => 0,
    }
}

struct RefInfo {
    name: String,
    sha: gix::ObjectId,
}

fn refs_with(repo: &gix::Repository, prefix: &str) -> Result<Vec<RefInfo>, ApiError> {
    let platform = repo.references().map_err(gerr)?;
    let iter = platform.prefixed(prefix).map_err(gerr)?;
    let mut out = Vec::new();
    for r in iter {
        let Ok(mut r) = r else { continue };
        let name = r.name().as_bstr().to_string();
        if name.ends_with("/HEAD") {
            continue;
        }
        let Ok(id) = r.peel_to_id() else { continue };
        out.push(RefInfo { name, sha: id.detach() });
    }
    Ok(out)
}

/// Branches, amont, graphe.
fn read_repo(dir: &Path, limit: usize) -> Result<Value, ApiError> {
    let repo = open(dir)?;
    let head_name = repo.head_name().ok().flatten().map(|n| n.as_bstr().to_string());
    let head_sha = repo.head_id().ok().map(|i| i.detach());
    let locals = refs_with(&repo, "refs/heads/")?;
    let remotes = refs_with(&repo, "refs/remotes/")?;
    let tags = refs_with(&repo, "refs/tags/")?;
    let by_name: HashMap<&str, gix::ObjectId> = remotes.iter().map(|r| (r.name.as_str(), r.sha)).collect();

    let mut branches = Vec::new();
    for b in &locals {
        let full: &gix::refs::FullNameRef = b.name.as_str().try_into().map_err(gerr)?;
        let upstream = repo
            .branch_remote_tracking_ref_name(full, gix::remote::Direction::Fetch)
            .and_then(|r| r.ok())
            .map(|n| n.as_bstr().to_string());
        let up_sha = upstream.as_deref().and_then(|u| by_name.get(u).copied());
        let (ahead, behind) = match up_sha {
            Some(u) => (count_not_in(&repo, b.sha, u), count_not_in(&repo, u, b.sha)),
            None => (0, 0),
        };
        branches.push(json!({
            "name": short(&b.name),
            "sha": b.sha.to_string(),
            "current": head_name.as_deref() == Some(b.name.as_str()),
            "upstream": upstream.as_deref().map(short),
            "upstream_gone": upstream.is_some() && up_sha.is_none(),
            "ahead": ahead,
            "behind": behind,
        }));
    }
    let remote_list: Vec<Value> = remotes
        .iter()
        .map(|r| json!({"name": short(&r.name), "sha": r.sha.to_string()}))
        .collect();

    // Décorations du graphe.
    let mut deco: HashMap<gix::ObjectId, Vec<Value>> = HashMap::new();
    for (list, kind) in [(&locals, "branch"), (&remotes, "remote"), (&tags, "tag")] {
        for r in list.iter() {
            deco.entry(r.sha)
                .or_default()
                .push(json!({"name": short(&r.name), "kind": kind}));
        }
    }
    let mut tips: Vec<gix::ObjectId> = locals.iter().chain(&remotes).chain(&tags).map(|r| r.sha).collect();
    if let Some(h) = head_sha {
        tips.push(h);
    }
    tips.sort();
    tips.dedup();
    let mut commits = Vec::new();
    let mut truncated = false;
    let mut infos = Vec::new();
    if !tips.is_empty() {
        let walk = repo
            .rev_walk(tips)
            .sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default()))
            .all()
            .map_err(gerr)?;
        for info in walk {
            let Ok(info) = info else { continue };
            if infos.len() >= limit {
                truncated = true;
                break;
            }
            let parents: Vec<gix::ObjectId> = info.parent_ids.iter().copied().collect();
            infos.push((info.id, parents, info.commit_time.unwrap_or(0)));
        }
    }
    for (id, parents) in topo_order(infos) {
        {
            let Ok(c) = repo.find_commit(id) else { continue };
            let (author, email, time) = match c.author() {
                Ok(a) => (
                    a.name.to_string(),
                    a.email.to_string(),
                    a.time().map(|t| t.seconds).unwrap_or(0),
                ),
                Err(_) => (String::new(), String::new(), 0),
            };
            let subject = c.message().map(|m| m.summary().to_string()).unwrap_or_default();
            commits.push(json!({
                "sha": id.to_string(),
                "parents": parents.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                "author": author,
                "email": email,
                "time": time,
                "subject": subject,
                "refs": deco.remove(&id).unwrap_or_default(),
            }));
        }
    }
    let url = origin_url(&repo);
    Ok(json!({
        "head": head_name.as_deref().map(short),
        "head_sha": head_sha.map(|s| s.to_string()),
        "detached": head_name.is_none() && head_sha.is_some(),
        "branches": branches,
        "remotes": remote_list,
        "commits": commits,
        "truncated": truncated,
        "origin": url.as_deref().map(forge::display_url),
    }))
}

/// Ordre du graphe (`git log --date-order`) : un commit après tous ses
/// enfants présents, le plus récent d'abord parmi ceux qui sont prêts.
fn topo_order(infos: Vec<(gix::ObjectId, Vec<gix::ObjectId>, i64)>) -> Vec<(gix::ObjectId, Vec<gix::ObjectId>)> {
    use std::collections::BinaryHeap;
    let index: HashMap<gix::ObjectId, usize> = infos.iter().enumerate().map(|(i, c)| (c.0, i)).collect();
    let mut children = vec![0usize; infos.len()];
    for (_, parents, _) in &infos {
        for p in parents {
            if let Some(&j) = index.get(p) {
                children[j] += 1;
            }
        }
    }
    // (date, rang inverse du parcours) : à date égale, l'ordre du parcours.
    let mut ready: BinaryHeap<(i64, std::cmp::Reverse<usize>)> = children
        .iter()
        .enumerate()
        .filter(|(_, n)| **n == 0)
        .map(|(i, _)| (infos[i].2, std::cmp::Reverse(i)))
        .collect();
    let mut out = Vec::with_capacity(infos.len());
    while let Some((_, std::cmp::Reverse(i))) = ready.pop() {
        for p in &infos[i].1 {
            if let Some(&j) = index.get(p) {
                children[j] -= 1;
                if children[j] == 0 {
                    ready.push((infos[j].2, std::cmp::Reverse(j)));
                }
            }
        }
        out.push((infos[i].0, infos[i].1.clone()));
    }
    out
}

fn blob_text(repo: &gix::Repository, id: gix::ObjectId) -> Option<Option<String>> {
    if id.is_null() {
        return Some(Some(String::new()));
    }
    let o = repo.find_object(id).ok()?;
    let data = &o.data;
    if data.len() > MAX_BLOB {
        return Some(None);
    }
    if data[..data.len().min(8192)].contains(&0) {
        return Some(None);
    }
    Some(Some(String::from_utf8_lossy(data).into_owned()))
}

/// Fichiers et diff d'un commit par rapport à son premier parent.
fn read_commit(dir: &Path, sha: &str) -> Result<Value, ApiError> {
    let repo = open(dir)?;
    let id = gix::ObjectId::from_hex(sha.as_bytes()).map_err(|_| ApiError::NotFound)?;
    let c = repo.find_commit(id).map_err(|_| ApiError::NotFound)?;
    let tree = c.tree().map_err(gerr)?;
    let parent = c.parent_ids().next().map(|p| p.detach());
    let ptree = match parent {
        Some(p) => Some(repo.find_commit(p).map_err(gerr)?.tree().map_err(gerr)?),
        None => None,
    };
    let opts = gix::diff::Options::default().with_rewrites(None);
    let changes = repo
        .diff_tree_to_tree(ptree.as_ref(), Some(&tree), opts)
        .map_err(gerr)?;
    let mut files = Vec::new();
    let mut diff = String::new();
    let mut diff_truncated = false;
    use gix::object::tree::diff::ChangeDetached as C;
    for ch in changes {
        let (path, status, old, new, mode) = match ch {
            C::Addition {
                location,
                id,
                entry_mode,
                ..
            } => (location, "added", gix::ObjectId::null(id.kind()), id, entry_mode),
            C::Deletion {
                location,
                id,
                entry_mode,
                ..
            } => (location, "deleted", id, gix::ObjectId::null(id.kind()), entry_mode),
            C::Modification {
                location,
                previous_id,
                id,
                entry_mode,
                ..
            } => (location, "modified", previous_id, id, entry_mode),
            C::Rewrite {
                location,
                source_id,
                id,
                entry_mode,
                ..
            } => (location, "modified", source_id, id, entry_mode),
        };
        if mode.is_tree() {
            continue;
        }
        let path = path.to_string();
        let (a, b) = (blob_text(&repo, old), blob_text(&repo, new));
        let (mut add, mut del) = (0usize, 0usize);
        let binary = !matches!((&a, &b), (Some(Some(_)), Some(Some(_))));
        if let (Some(Some(a)), Some(Some(b))) = (&a, &b) {
            let td = similar::TextDiff::from_lines(a.as_str(), b.as_str());
            for op in td.iter_all_changes() {
                match op.tag() {
                    similar::ChangeTag::Insert => add += 1,
                    similar::ChangeTag::Delete => del += 1,
                    similar::ChangeTag::Equal => {}
                }
            }
            if !diff_truncated {
                let u = td
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{path}"), &format!("b/{path}"))
                    .to_string();
                if diff.len() + u.len() > MAX_DIFF {
                    diff_truncated = true;
                } else {
                    diff.push_str(&format!("diff --git a/{path} b/{path}\n"));
                    diff.push_str(&u);
                }
            }
        } else if !diff_truncated {
            diff.push_str(&format!(
                "diff --git a/{path} b/{path}\nFichiers binaires ou trop gros différents\n"
            ));
        }
        files.push(json!({"path": path, "status": status, "additions": add, "deletions": del, "binary": binary}));
    }
    let (author, email, time) = match c.author() {
        Ok(a) => (
            a.name.to_string(),
            a.email.to_string(),
            a.time().map(|t| t.seconds).unwrap_or(0),
        ),
        Err(_) => (String::new(), String::new(), 0),
    };
    Ok(json!({
        "sha": id.to_string(),
        "parents": c.parent_ids().map(|p| p.to_string()).collect::<Vec<_>>(),
        "author": author,
        "email": email,
        "time": time,
        "message": c.message_raw().map(|m| m.to_string()).unwrap_or_default(),
        "files": files,
        "diff": diff,
        "diff_truncated": diff_truncated,
    }))
}

/// Ce dont fetch et push ont besoin, lu dans le dépôt : URL `origin`, refs
/// `origin/*` connues, sha d'une branche.
struct NetInfo {
    url: String,
    haves: String,
    branch_sha: Option<String>,
    current: Option<String>,
}

fn net_info(dir: &Path, branch: Option<&str>) -> Result<NetInfo, ApiError> {
    let repo = open(dir)?;
    let url = origin_url(&repo).ok_or_else(|| ApiError::Git("no_remote", "aucun remote origin".into()))?;
    let haves: String = refs_with(&repo, "refs/remotes/origin/")?
        .iter()
        .map(|r| format!("{} {}\n", r.sha, r.name))
        .collect();
    let current = repo
        .head_name()
        .ok()
        .flatten()
        .map(|n| short(&n.as_bstr().to_string()).to_string());
    let branch_sha = match branch.or(current.as_deref()) {
        Some(b) => repo
            .find_reference(format!("refs/heads/{b}").as_str())
            .ok()
            .and_then(|mut r| r.peel_to_id().ok())
            .map(|i| i.to_string()),
        None => None,
    };
    Ok(NetInfo {
        url,
        haves,
        branch_sha,
        current,
    })
}

fn upstream_of(dir: &Path, branch: &str) -> Result<Option<String>, ApiError> {
    let repo = open(dir)?;
    let full = format!("refs/heads/{branch}");
    let full: &gix::refs::FullNameRef = full.as_str().try_into().map_err(gerr)?;
    Ok(repo
        .branch_remote_tracking_ref_name(full, gix::remote::Direction::Fetch)
        .and_then(|r| r.ok())
        .map(|n| n.as_bstr().to_string()))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ApiError> + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
}

// ---------------------------------------------------------------- conteneurs

async fn helper(
    state: &AppState,
    repo: &Repo,
    script: &'static str,
    mut env: Vec<(String, String)>,
    network: bool,
    timeout: Duration,
) -> Result<enginex_sandbox::HelperOutcome, ApiError> {
    let backend = state.0.sandboxes.backend()?;
    env.push(("LANG".into(), "C.UTF-8".into()));
    let spec = HelperSpec {
        session_id: repo.helper_id.clone(),
        owner: state.0.sandboxes.instance().to_string(),
        binds: vec![(repo.dir.clone(), "REPO", true)],
        script,
        env,
        network,
    };
    backend
        .run_helper(&spec, timeout)
        .await
        .map_err(|e| ApiError::Git("git_failed", state.masker().mask(&format!("{e:#}")).into_owned()))
}

fn fail(state: &AppState, o: &enginex_sandbox::HelperOutcome) -> ApiError {
    let detail = state.masker().mask(o.stderr.trim()).into_owned();
    let code = match o.code {
        10 => "dirty",
        11 => "merge_conflict",
        12 => "not_fast_forward",
        13 => "nothing_to_commit",
        20 => "push_rejected",
        _ => "git_failed",
    };
    ApiError::Git(code, detail)
}

async fn local(state: &AppState, repo: &Repo, op: &str, mut env: Vec<(String, String)>) -> Result<String, ApiError> {
    env.push(("OP".into(), op.into()));
    env.extend(sandboxes::git_env(state).await);
    let o = helper(state, repo, LOCAL, env, false, LOCAL_TIMEOUT).await?;
    if o.code != 0 {
        return Err(fail(state, &o));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// URL et jeton du remote ; une URL locale n'est permise qu'au backend local
/// (tests, développement).
async fn remote_for(state: &AppState, repo: &Repo, url: &str) -> Result<Remote, ApiError> {
    let local_ok = matches!(*state.0.sandboxes.backend()?, Backend::Local(_));
    forge::resolve(state, repo.project_id, url, local_ok).await
}

fn net_env(remote: &Remote, haves: String) -> Vec<(String, String)> {
    let mut env = vec![("URL".into(), remote.push_url.clone()), ("HAVES".into(), haves)];
    if let Some(t) = &remote.token {
        env.push(("GIT_TOKEN".into(), t.clone()));
    }
    env
}

/// Fetch de `origin` puis mise à jour des refs `origin/*` du dépôt.
async fn fetch_locked(state: &AppState, repo: &Repo) -> Result<usize, ApiError> {
    let dir = repo.dir.clone();
    let info = blocking(move || net_info(&dir, None)).await?;
    let remote = remote_for(state, repo, &info.url).await?;
    let o = helper(
        state,
        repo,
        FETCH,
        net_env(&remote, info.haves.clone()),
        true,
        NET_TIMEOUT,
    )
    .await?;
    if o.code != 0 {
        return Err(fail(state, &o));
    }
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let mut refs = String::new();
    let mut seen = std::collections::HashSet::new();
    for line in out.lines() {
        let Some((sha, name)) = line.split_once(' ') else {
            continue;
        };
        if !snapshots::valid_rev(sha) || !snapshots::valid_rev(name) || !name.starts_with("refs/remotes/origin/") {
            continue;
        }
        seen.insert(name.to_string());
        refs.push_str(&format!("{sha} {name}\n"));
    }
    let dels: String = info
        .haves
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(_, n)| n))
        .filter(|n| !seen.contains(*n))
        .map(|n| format!("{n}\n"))
        .collect();
    local(state, repo, "refs", vec![("REFS".into(), refs), ("DELS".into(), dels)]).await?;
    Ok(seen.len())
}

/// Push d'une branche vers `origin` (jamais forcé), puis `origin/<branche>` et
/// l'amont posés dans le dépôt.
pub async fn push_locked(state: &AppState, repo: &Repo, branch: Option<&str>) -> Result<Value, ApiError> {
    let dir = repo.dir.clone();
    let b = branch.map(str::to_string);
    let info = blocking(move || net_info(&dir, b.as_deref())).await?;
    let branch = branch
        .map(str::to_string)
        .or(info.current.clone())
        .ok_or_else(|| ApiError::Git("no_branch", "HEAD détachée : choisir une branche".into()))?;
    if !snapshots::valid_rev(&branch) {
        return Err(ApiError::BadRequest);
    }
    let sha = info.branch_sha.clone().ok_or(ApiError::NotFound)?;
    let remote = remote_for(state, repo, &info.url).await?;
    let mut env = net_env(&remote, info.haves.clone());
    env.push(("SHA".into(), sha.clone()));
    env.push(("BRANCH".into(), branch.clone()));
    let o = helper(state, repo, PUSH, env, true, NET_TIMEOUT).await?;
    if o.code != 0 {
        return Err(fail(state, &o));
    }
    let dir = repo.dir.clone();
    let br = branch.clone();
    let has_up = blocking(move || upstream_of(&dir, &br)).await?.is_some();
    let mut env = vec![("REFS".into(), format!("{sha} refs/remotes/origin/{branch}\n"))];
    if !has_up {
        env.push(("UPSTREAM".into(), branch.clone()));
    }
    local(state, repo, "refs", env).await?;
    Ok(json!({"branch": branch, "sha": sha, "remote": remote.display}))
}

/// Une session vivante (tour ou vérification) travaille dans le dossier du
/// topic : pas de changement de branche ni de fusion sous elle.
async fn ensure_idle(state: &AppState, repo: &Repo) -> Result<(), ApiError> {
    let Some(topic_id) = repo.topic_id else { return Ok(()) };
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM chat_sessions WHERE topic_id = ? AND copy_branch IS NULL AND archived = 0")
            .bind(topic_id)
            .fetch_all(state.db())
            .await?;
    for h in bridge::hubs(state) {
        if ids.contains(&h.id) && h.busy() {
            return Err(ApiError::TurnInProgress);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- routes

/// Dépôts = topics dont le dossier a un `.git`.
pub async fn repos(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT t.id, p.slug, p.name, t.slug, t.name FROM topics t JOIN projects p ON p.id = t.project_id \
         ORDER BY p.name, t.name",
    )
    .fetch_all(state.db())
    .await?;
    let mut out = Vec::new();
    for (id, ps, pn, ts, tn) in rows {
        let dir = workspace::topic_path(&state, &ps, &ts);
        if !has_git_dir(&dir) {
            continue;
        }
        let summary = blocking(move || {
            let repo = open(&dir)?;
            let head = repo
                .head_name()
                .ok()
                .flatten()
                .map(|n| short(&n.as_bstr().to_string()).to_string());
            Ok(json!({"head": head, "origin": origin_url(&repo).as_deref().map(forge::display_url)}))
        })
        .await
        .unwrap_or_else(|e| json!({"error": e.to_string()}));
        out.push(json!({
            "topic_id": id,
            "project": pn,
            "topic": tn,
            "path": format!("projects/{ps}/topics/{ts}"),
            "head": summary["head"],
            "origin": summary["origin"],
        }));
    }
    Ok(Json(json!({"repos": out})))
}

#[derive(Deserialize)]
pub struct RepoQuery {
    limit: Option<usize>,
}

pub async fn repo(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<Value>, ApiError> {
    let r = topic_repo(&state, id).await?;
    let limit = q.limit.unwrap_or(DEFAULT_COMMITS).clamp(1, MAX_COMMITS);
    let dir = r.dir.clone();
    let mut v = blocking(move || read_repo(&dir, limit)).await?;
    v["forge"] = json!(forge::kind_of(&state, r.project_id, v["origin"].as_str()).await);
    Ok(Json(v))
}

pub async fn commit(
    State(state): State<AppState>,
    UrlPath((id, sha)): UrlPath<(i64, String)>,
) -> Result<Json<Value>, ApiError> {
    let r = topic_repo(&state, id).await?;
    if sha.len() < 4 || sha.len() > 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::NotFound);
    }
    let dir = r.dir.clone();
    Ok(Json(blocking(move || read_commit(&dir, &sha)).await?))
}

pub async fn fetch(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let r = topic_repo(&state, id).await?;
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    let n = fetch_locked(&state, &r).await?;
    Ok(Json(json!({"remote_branches": n})))
}

/// Fetch puis avance rapide de la branche courante sur son amont.
pub async fn pull(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let r = topic_repo(&state, id).await?;
    ensure_idle(&state, &r).await?;
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    fetch_locked(&state, &r).await?;
    let dir = r.dir.clone();
    let (current, up) = blocking(move || {
        let info = net_info(&dir, None)?;
        let cur = info
            .current
            .ok_or_else(|| ApiError::Git("no_branch", "HEAD détachée".into()))?;
        let up = upstream_of(&dir, &cur)?;
        Ok((cur, up))
    })
    .await?;
    let up = up.ok_or_else(|| ApiError::Git("no_upstream", format!("{current} n'a pas d'amont")))?;
    local(&state, &r, "pull", vec![("REF".into(), up)]).await?;
    Ok(Json(json!({"branch": current})))
}

#[derive(Deserialize)]
pub struct BranchBody {
    branch: Option<String>,
}

pub async fn push(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<BranchBody>,
) -> Result<Json<Value>, ApiError> {
    let r = topic_repo(&state, id).await?;
    if let Some(br) = &b.branch
        && !snapshots::valid_rev(br)
    {
        return Err(ApiError::BadRequest);
    }
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    Ok(Json(push_locked(&state, &r, b.branch.as_deref()).await?))
}

#[derive(Deserialize)]
pub struct NeedBranch {
    branch: String,
    sha: Option<String>,
}

fn check_branch(b: &str) -> Result<(), ApiError> {
    if snapshots::valid_rev(b) && !b.starts_with('-') && !b.ends_with(".lock") && !b.ends_with('/') {
        Ok(())
    } else {
        Err(ApiError::BadRequest)
    }
}

pub async fn checkout(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<NeedBranch>,
) -> Result<Json<Value>, ApiError> {
    check_branch(&b.branch)?;
    let r = topic_repo(&state, id).await?;
    ensure_idle(&state, &r).await?;
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    local(&state, &r, "checkout", vec![("BRANCH".into(), b.branch.clone())]).await?;
    Ok(Json(json!({"branch": b.branch})))
}

pub async fn merge(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<NeedBranch>,
) -> Result<Json<Value>, ApiError> {
    check_branch(&b.branch)?;
    let r = topic_repo(&state, id).await?;
    ensure_idle(&state, &r).await?;
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    local(&state, &r, "merge", vec![("BRANCH".into(), b.branch.clone())]).await?;
    Ok(Json(json!({"merged": b.branch})))
}

/// Nouvelle branche sur un commit (par défaut HEAD).
pub async fn create_branch(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<NeedBranch>,
) -> Result<Json<Value>, ApiError> {
    check_branch(&b.branch)?;
    let sha = b.sha.clone().unwrap_or_else(|| "HEAD".into());
    if !snapshots::valid_rev(&sha) {
        return Err(ApiError::BadRequest);
    }
    let r = topic_repo(&state, id).await?;
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    local(
        &state,
        &r,
        "branch",
        vec![("BRANCH".into(), b.branch.clone()), ("SHA".into(), sha)],
    )
    .await?;
    Ok(Json(json!({"branch": b.branch})))
}

// ---- Session (vue Modifications) ----

#[derive(Deserialize)]
pub struct CommitBody {
    message: String,
}

fn check_message(m: &str) -> Result<String, ApiError> {
    let m = m.trim();
    if m.is_empty() || m.len() > 10_000 || m.contains('\0') {
        return Err(ApiError::BadRequest);
    }
    Ok(m.to_string())
}

/// État git du dossier de la session : branche, amont, remote, forge.
pub async fn session_status(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let r = match session_repo(&state, &id).await {
        Ok(r) => r,
        Err(ApiError::NotGit) => return Ok(Json(json!({"git": false}))),
        Err(e) => return Err(e),
    };
    let dir = r.dir.clone();
    let v = blocking(move || {
        let repo = open(&dir)?;
        let head = repo
            .head_name()
            .ok()
            .flatten()
            .map(|n| short(&n.as_bstr().to_string()).to_string());
        let url = origin_url(&repo);
        let mut ahead = None;
        if let Some(h) = &head {
            let full = format!("refs/heads/{h}");
            if let Ok(full) = <&gix::refs::FullNameRef>::try_from(full.as_str()) {
                let up = repo
                    .branch_remote_tracking_ref_name(full, gix::remote::Direction::Fetch)
                    .and_then(|r| r.ok())
                    .map(|n| n.as_bstr().to_string());
                let tip = repo.head_id().ok().map(|i| i.detach());
                if let (Some(up), Some(tip)) = (up, tip)
                    && let Ok(mut u) = repo.find_reference(up.as_str())
                    && let Ok(uid) = u.peel_to_id()
                {
                    ahead = Some(count_not_in(&repo, tip, uid.detach()));
                }
            }
        }
        Ok(json!({"git": true, "branch": head, "origin": url.as_deref().map(forge::display_url), "ahead": ahead}))
    })
    .await?;
    let mut v = v;
    v["forge"] = json!(forge::kind_of(&state, r.project_id, v["origin"].as_str()).await);
    Ok(Json(v))
}

pub async fn session_commit(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    ApiJson(b): ApiJson<CommitBody>,
) -> Result<Json<Value>, ApiError> {
    let msg = check_message(&b.message)?;
    let r = session_repo(&state, &id).await?;
    if bridge::hub(&state, &id).busy() {
        return Err(ApiError::TurnInProgress);
    }
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    let sha = local(&state, &r, "commit", vec![("MSG".into(), msg)]).await?;
    Ok(Json(json!({"sha": sha})))
}

pub async fn session_push(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let r = session_repo(&state, &id).await?;
    let _g = snapshots::lock_dir(&state, &r.dir).await;
    Ok(Json(push_locked(&state, &r, None).await?))
}

#[derive(Deserialize)]
pub struct SessionPrBody {
    title: String,
    #[serde(default)]
    body: String,
    base: Option<String>,
}

/// Push de la branche de la session puis pull request vers `base` (par
/// défaut la branche par défaut du dépôt distant).
pub async fn session_pull_request(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    ApiJson(b): ApiJson<SessionPrBody>,
) -> Result<Json<Value>, ApiError> {
    let r = session_repo(&state, &id).await?;
    let pushed = {
        let _g = snapshots::lock_dir(&state, &r.dir).await;
        push_locked(&state, &r, None).await?
    };
    let dir = r.dir.clone();
    let url = blocking(move || {
        let repo = open(&dir)?;
        origin_url(&repo).ok_or_else(|| ApiError::Git("no_remote", "aucun remote origin".into()))
    })
    .await?;
    let remote = remote_for(&state, &r, &url).await?;
    let head = pushed["branch"].as_str().unwrap_or_default().to_string();
    let pr = forge::create_pull(&state, &remote, &b.title, &b.body, &head, b.base.as_deref()).await?;
    Ok(Json(pr))
}

// ---- Pull requests de la page Git ----

async fn topic_remote(state: &AppState, id: i64) -> Result<Remote, ApiError> {
    let r = topic_repo(state, id).await?;
    let dir = r.dir.clone();
    let url = blocking(move || {
        let repo = open(&dir)?;
        origin_url(&repo).ok_or_else(|| ApiError::Git("no_remote", "aucun remote origin".into()))
    })
    .await?;
    remote_for(state, &r, &url).await
}

#[derive(Deserialize)]
pub struct PullsQuery {
    state: Option<String>,
}

pub async fn pulls(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    Query(q): Query<PullsQuery>,
) -> Result<Json<Value>, ApiError> {
    let remote = topic_remote(&state, id).await?;
    let st = match q.state.as_deref() {
        Some("closed") => "closed",
        Some("all") => "all",
        _ => "open",
    };
    Ok(Json(json!({"pulls": forge::list_pulls(&state, &remote, st).await?})))
}

pub async fn pull_detail(
    State(state): State<AppState>,
    UrlPath((id, n)): UrlPath<(i64, u64)>,
) -> Result<Json<Value>, ApiError> {
    let remote = topic_remote(&state, id).await?;
    Ok(Json(forge::get_pull(&state, &remote, n).await?))
}

#[derive(Deserialize)]
pub struct CommentBody {
    body: String,
}

pub async fn pull_comment(
    State(state): State<AppState>,
    UrlPath((id, n)): UrlPath<(i64, u64)>,
    ApiJson(b): ApiJson<CommentBody>,
) -> Result<Json<Value>, ApiError> {
    let body = check_message(&b.body)?;
    let remote = topic_remote(&state, id).await?;
    Ok(Json(forge::comment(&state, &remote, n, &body).await?))
}

#[derive(Deserialize)]
pub struct MergeBody {
    method: Option<String>,
}

pub async fn pull_merge(
    State(state): State<AppState>,
    UrlPath((id, n)): UrlPath<(i64, u64)>,
    ApiJson(b): ApiJson<MergeBody>,
) -> Result<Json<Value>, ApiError> {
    let method = match b.method.as_deref() {
        None | Some("merge") => "merge",
        Some("squash") => "squash",
        Some("rebase") => "rebase",
        _ => return Err(ApiError::BadRequest),
    };
    let remote = topic_remote(&state, id).await?;
    Ok(Json(forge::merge_pull(&state, &remote, n, method).await?))
}

#[derive(Deserialize)]
pub struct CreatePrBody {
    title: String,
    #[serde(default)]
    body: String,
    head: String,
    base: Option<String>,
}

pub async fn pull_create(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<CreatePrBody>,
) -> Result<Json<Value>, ApiError> {
    check_branch(&b.head)?;
    if let Some(base) = &b.base {
        check_branch(base)?;
    }
    let remote = topic_remote(&state, id).await?;
    Ok(Json(
        forge::create_pull(&state, &remote, &b.title, &b.body, &b.head, b.base.as_deref()).await?,
    ))
}
