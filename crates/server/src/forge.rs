//! Forges : GitHub et Gitea, pour le push (URL + jeton) et les pull
//! requests (API).
//!
//! L'URL `origin` est écrite dans le dépôt par la sandbox : un jeton ne part
//! que vers l'hôte qui lui correspond (`GITHUB_TOKEN` / `GH_TOKEN` vers
//! github.com, `GITEA_TOKEN` vers l'hôte de `GITEA_URL`), jamais ailleurs.

use std::collections::HashMap;

use reqwest::Method;
use serde_json::{Value, json};

use crate::{error::ApiError, state::AppState, vault};

/// Diff d'une pull request rendu au plus.
const MAX_DIFF: usize = 512 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Github,
    Gitea,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Github => "github",
            Kind::Gitea => "gitea",
        }
    }
}

/// Remote résolu : URL de push (https, sans identifiants), jeton éventuel,
/// API de la forge.
#[derive(Clone)]
pub struct Remote {
    pub kind: Option<Kind>,
    pub push_url: String,
    pub display: String,
    pub token: Option<String>,
    pub api: Option<String>,
    pub owner: String,
    pub name: String,
}

struct Parsed {
    scheme: String,
    /// Hôte avec son port éventuel, en minuscules.
    host: String,
    path: String,
}

/// URL sans identifiants (`https://jeton@hôte/…` → `https://hôte/…`).
pub fn display_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
        return if path.is_empty() {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}/{path}")
        };
    }
    url.to_string()
}

fn parse(url: &str) -> Option<Parsed> {
    let url = url.trim();
    if url.is_empty() || url.starts_with('-') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => {
            // `git@hôte:owner/repo.git`
            let (auth, path) = url.split_once(':')?;
            let host = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
            return checked(Parsed {
                scheme: "https".into(),
                host: host.to_ascii_lowercase(),
                path: path.trim_start_matches('/').to_string(),
            });
        }
    };
    let (auth, path) = rest.split_once('/')?;
    let host = auth
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(auth)
        .to_ascii_lowercase();
    let scheme = match scheme.as_str() {
        "https" | "http" => scheme,
        // ssh://git@hôte:22/… : poussé en https (le port ssh ne sert pas).
        "ssh" | "git+ssh" => {
            return checked(Parsed {
                scheme: "https".into(),
                host: host.split(':').next().unwrap_or(&host).to_string(),
                path: path.to_string(),
            });
        }
        _ => return None,
    };
    checked(Parsed {
        scheme,
        host,
        path: path.to_string(),
    })
}

/// Hôte et chemin sans caractère qui changerait le sens de l'URL.
fn checked(p: Parsed) -> Option<Parsed> {
    let host_ok = !p.host.is_empty()
        && !p.host.starts_with(['-', '.'])
        && p.host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    let path_ok = p
        .path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '~'))
        && !p.path.split('/').any(|c| c == "..");
    (host_ok && path_ok).then_some(p)
}

/// `owner/repo` (dernier couple du chemin, `.git` retiré).
fn owner_repo(path: &str) -> Option<(String, String)> {
    let p = path.trim_matches('/');
    let p = p.strip_suffix(".git").unwrap_or(p);
    let mut it = p.rsplitn(2, '/');
    let name = it.next()?.to_string();
    let owner = it.next()?.rsplit('/').next()?.to_string();
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(&owner) && ok(&name) && name != "." && name != "..").then_some((owner, name))
}

async fn secrets(state: &AppState, project_id: i64) -> HashMap<String, String> {
    vault::env_for(state, project_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .collect()
}

fn gitea_base(secrets: &HashMap<String, String>) -> Option<Parsed> {
    let u = secrets.get("GITEA_URL")?.trim().trim_end_matches('/');
    parse(&format!("{u}/"))
}

/// Résout l'URL `origin`. `local_ok` : chemin local permis (backend local).
pub async fn resolve(state: &AppState, project_id: i64, url: &str, local_ok: bool) -> Result<Remote, ApiError> {
    let url = url.trim();
    if local_ok && (url.starts_with('/') || url.starts_with("file://")) {
        return Ok(Remote {
            kind: None,
            push_url: url.to_string(),
            display: url.to_string(),
            token: None,
            api: None,
            owner: String::new(),
            name: String::new(),
        });
    }
    let p = parse(url)
        .ok_or_else(|| ApiError::Git("no_remote", format!("URL non prise en charge : {}", display_url(url))))?;
    let secrets = secrets(state, project_id).await;
    let (owner, name) = owner_repo(&p.path).unwrap_or_default();
    let push_url = format!("{}://{}/{}", p.scheme, p.host, p.path.trim_start_matches('/'));
    let mut r = Remote {
        kind: None,
        display: push_url.clone(),
        push_url,
        token: None,
        api: None,
        owner,
        name,
    };
    if p.host == "github.com" && p.scheme == "https" {
        r.kind = Some(Kind::Github);
        r.api = Some(state.config().web.github_api.trim_end_matches('/').to_string());
        r.token = secrets.get("GITHUB_TOKEN").or_else(|| secrets.get("GH_TOKEN")).cloned();
    } else if let Some(g) = gitea_base(&secrets)
        && g.host == p.host
        && g.scheme == p.scheme
    {
        let prefix = g.path.trim_matches('/');
        // Gitea servi sous un chemin (`https://hôte/gitea`) : retiré du dépôt.
        let rel = p.path.trim_start_matches('/');
        let rel = if prefix.is_empty() {
            rel
        } else {
            rel.strip_prefix(prefix).unwrap_or(rel)
        };
        if let Some((o, n)) = owner_repo(rel) {
            r.owner = o;
            r.name = n;
        }
        r.kind = Some(Kind::Gitea);
        let base = if prefix.is_empty() {
            format!("{}://{}", g.scheme, g.host)
        } else {
            format!("{}://{}/{prefix}", g.scheme, g.host)
        };
        r.api = Some(format!("{base}/api/v1"));
        r.token = secrets.get("GITEA_TOKEN").cloned();
    }
    Ok(r)
}

/// Forge reconnue pour l'URL affichée (`null` sinon).
pub async fn kind_of(state: &AppState, project_id: i64, origin: Option<&str>) -> Option<&'static str> {
    let r = resolve(state, project_id, origin?, false).await.ok()?;
    r.kind.map(Kind::as_str)
}

fn need_api(r: &Remote) -> Result<(Kind, &str), ApiError> {
    match (r.kind, r.api.as_deref()) {
        (Some(k), Some(a)) if !r.owner.is_empty() => Ok((k, a)),
        _ => Err(ApiError::Git(
            "no_forge",
            "remote ni GitHub ni Gitea (GITEA_URL dans Clés)".into(),
        )),
    }
}

async fn call(
    state: &AppState,
    r: &Remote,
    method: Method,
    path: &str,
    body: Option<Value>,
    accept_diff: bool,
) -> Result<(u16, String), ApiError> {
    let (kind, api) = need_api(r)?;
    if method != Method::GET && r.token.is_none() {
        let key = if kind == Kind::Github {
            "GITHUB_TOKEN"
        } else {
            "GITEA_TOKEN"
        };
        return Err(ApiError::Git("no_token", format!("{key} absent des Clés")));
    }
    let url = format!("{api}/repos/{}/{}{path}", r.owner, r.name);
    let mut req = state
        .0
        .http
        .request(method, &url)
        .timeout(TIMEOUT)
        .header("User-Agent", "EngineX");
    req = match kind {
        Kind::Github => req
            .header(
                "Accept",
                if accept_diff {
                    "application/vnd.github.diff"
                } else {
                    "application/vnd.github+json"
                },
            )
            .header("X-GitHub-Api-Version", "2022-11-28"),
        Kind::Gitea => req.header("Accept", if accept_diff { "text/plain" } else { "application/json" }),
    };
    if let Some(t) = &r.token {
        req = match kind {
            Kind::Github => req.bearer_auth(t),
            Kind::Gitea => req.header("Authorization", format!("token {t}")),
        };
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req
        .send()
        .await
        .map_err(|e| ApiError::Git("forge_error", state.masker().mask(&e.to_string()).into_owned()))?;
    let status = res.status().as_u16();
    let text = res
        .text()
        .await
        .map_err(|e| ApiError::Git("forge_error", e.to_string()))?;
    Ok((status, text))
}

/// Appel JSON ; un code ≥ 400 devient `forge_error` (ou `merge_refused`)
/// avec le message de la forge.
async fn call_json(
    state: &AppState,
    r: &Remote,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ApiError> {
    let (status, text) = call(state, r, method, path, body, false).await?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 {
        let msg = v["message"].as_str().map(str::to_string).unwrap_or_else(|| {
            let t: String = text.chars().take(300).collect();
            t
        });
        let msg = state.masker().mask(&format!("{status} : {msg}")).into_owned();
        return Err(match status {
            404 => ApiError::NotFound,
            405 | 409 => ApiError::Git("merge_refused", msg),
            _ => ApiError::Git("forge_error", msg),
        });
    }
    Ok(v)
}

fn pr(v: &Value) -> Value {
    let merged = v["merged"].as_bool().unwrap_or(false) || v["merged_at"].is_string();
    let state = if merged {
        "merged"
    } else {
        v["state"].as_str().unwrap_or("open")
    };
    json!({
        "number": v["number"],
        "title": v["title"],
        "state": state,
        "draft": v["draft"].as_bool().unwrap_or(false),
        "author": v["user"]["login"],
        "created_at": v["created_at"],
        "updated_at": v["updated_at"],
        "head": v["head"]["ref"],
        "base": v["base"]["ref"],
        "body": v["body"].as_str().unwrap_or(""),
        "url": v["html_url"],
        "mergeable": v["mergeable"],
    })
}

pub async fn list_pulls(state: &AppState, r: &Remote, st: &str) -> Result<Vec<Value>, ApiError> {
    let (kind, _) = need_api(r)?;
    let q = match kind {
        Kind::Github => format!("/pulls?state={st}&per_page=50"),
        Kind::Gitea => format!("/pulls?state={st}&limit=50"),
    };
    let v = call_json(state, r, Method::GET, &q, None).await?;
    Ok(v.as_array().map(|a| a.iter().map(pr).collect()).unwrap_or_default())
}

pub async fn get_pull(state: &AppState, r: &Remote, n: u64) -> Result<Value, ApiError> {
    let (kind, _) = need_api(r)?;
    let mut out = pr(&call_json(state, r, Method::GET, &format!("/pulls/{n}"), None).await?);
    let (status, mut diff) = match kind {
        Kind::Github => call(state, r, Method::GET, &format!("/pulls/{n}"), None, true).await?,
        Kind::Gitea => call(state, r, Method::GET, &format!("/pulls/{n}.diff"), None, true).await?,
    };
    if status >= 400 {
        diff.clear();
    }
    let truncated = diff.len() > MAX_DIFF;
    if truncated {
        let mut cut = MAX_DIFF;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        diff.truncate(cut);
    }
    let comments = call_json(state, r, Method::GET, &format!("/issues/{n}/comments"), None).await?;
    let comments: Vec<Value> = comments
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    json!({
                        "author": c["user"]["login"],
                        "body": c["body"],
                        "created_at": c["created_at"],
                        "url": c["html_url"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out["diff"] = json!(diff);
    out["diff_truncated"] = json!(truncated);
    out["comments"] = json!(comments);
    Ok(out)
}

pub async fn comment(state: &AppState, r: &Remote, n: u64, body: &str) -> Result<Value, ApiError> {
    let c = call_json(
        state,
        r,
        Method::POST,
        &format!("/issues/{n}/comments"),
        Some(json!({"body": body})),
    )
    .await?;
    Ok(json!({"author": c["user"]["login"], "body": c["body"], "created_at": c["created_at"], "url": c["html_url"]}))
}

pub async fn merge_pull(state: &AppState, r: &Remote, n: u64, method: &str) -> Result<Value, ApiError> {
    let (kind, _) = need_api(r)?;
    match kind {
        Kind::Github => {
            call_json(
                state,
                r,
                Method::PUT,
                &format!("/pulls/{n}/merge"),
                Some(json!({"merge_method": method})),
            )
            .await?;
        }
        Kind::Gitea => {
            call_json(
                state,
                r,
                Method::POST,
                &format!("/pulls/{n}/merge"),
                Some(json!({"Do": method})),
            )
            .await?;
        }
    }
    Ok(json!({"merged": true, "number": n}))
}

pub async fn create_pull(
    state: &AppState,
    r: &Remote,
    title: &str,
    body: &str,
    head: &str,
    base: Option<&str>,
) -> Result<Value, ApiError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 256 || body.len() > 65_000 {
        return Err(ApiError::BadRequest);
    }
    let base = match base {
        Some(b) => b.to_string(),
        None => {
            let repo = call_json(state, r, Method::GET, "", None).await?;
            repo["default_branch"].as_str().unwrap_or("main").to_string()
        }
    };
    if base == head {
        return Err(ApiError::Git(
            "same_branch",
            format!("{head} est déjà la branche de base"),
        ));
    }
    let v = call_json(
        state,
        r,
        Method::POST,
        "/pulls",
        Some(json!({"title": title, "body": body, "head": head, "base": base})),
    )
    .await?;
    Ok(pr(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            display_url("https://x-access-token:abc@github.com/o/r.git"),
            "https://github.com/o/r.git"
        );
        let p = parse("git@github.com:R0m1k3/EngineX.git").unwrap();
        assert_eq!((p.scheme.as_str(), p.host.as_str()), ("https", "github.com"));
        assert_eq!(owner_repo(&p.path), Some(("R0m1k3".into(), "EngineX".into())));
        let p = parse("ssh://git@git.vonrodbox.eu:2222/Michael/maxxing.git").unwrap();
        assert_eq!(p.host, "git.vonrodbox.eu");
        assert_eq!(owner_repo(&p.path), Some(("Michael".into(), "maxxing".into())));
        assert!(parse("ext::sh -c evil").is_none());
        assert!(parse("--upload-pack=evil").is_none());
        assert!(parse("file:///etc").is_none());
        assert!(parse("https://evil host/o/r").is_none());
    }
}
