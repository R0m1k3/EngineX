//! Skills Codex : `<data>/workspace/skills/<nom>/SKILL.md` (+ fichiers),
//! copiées dans `CODEX_HOME/skills` au démarrage de chaque sandbox. Les skills
//! livrées (`memory`, `memory-compaction`) sont posées au démarrage si elles
//! manquent, jamais écrasées.

use std::{
    io,
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{ApiError, ApiJson},
    state::AppState,
    system::{set_setting, setting},
    workspace::slugify,
};

pub const SHIPPED: &[(&str, &str)] = &[
    ("memory", include_str!("../skills/memory/SKILL.md")),
    (
        "memory-compaction",
        include_str!("../skills/memory-compaction/SKILL.md"),
    ),
];

const DISABLED_KEY: &str = "skills.disabled";
const MAX_SKILL: usize = 64 * 1024;

pub fn dir(state: &AppState) -> PathBuf {
    state.config().data_dir.join("workspace/skills")
}

fn valid_name(n: &str) -> bool {
    slugify(n).as_deref() == Some(n)
}

fn write_atomic(path: &Path, data: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("chemin sans dossier"))?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Pose les skills livrées absentes.
pub fn boot(state: &AppState) -> anyhow::Result<()> {
    let root = dir(state);
    std::fs::create_dir_all(&root)?;
    for (name, content) in SHIPPED {
        let d = root.join(name);
        if std::fs::symlink_metadata(&d).is_err() {
            write_atomic(&d.join("SKILL.md"), content)?;
        }
    }
    Ok(())
}

/// `(name, description)` du frontmatter `---` en tête (clés sur une ligne).
pub fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let (mut name, mut desc) = (None, None);
    for l in lines {
        if l.trim_end() == "---" {
            let unquote = |s: &str| s.trim().trim_matches('"').trim_matches('\'').trim().to_string();
            let name = unquote(name?);
            let desc = unquote(desc?);
            return (!name.is_empty() && !desc.is_empty()).then_some((name, desc));
        }
        if let Some(v) = l.strip_prefix("name:") {
            name = Some(v);
        } else if let Some(v) = l.strip_prefix("description:") {
            desc = Some(v);
        }
    }
    None
}

async fn disabled(state: &AppState) -> Vec<String> {
    setting(state, DISABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Skill valide : dossier réel (pas un lien), `SKILL.md` lisible, frontmatter
/// dont le `name` est le nom du dossier.
fn read_skill(root: &Path, name: &str) -> Option<(String, String)> {
    if !valid_name(name) {
        return None;
    }
    let d = root.join(name);
    if !std::fs::symlink_metadata(&d).ok()?.is_dir() {
        return None;
    }
    let f = d.join("SKILL.md");
    if !std::fs::symlink_metadata(&f).ok()?.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(f).ok()?;
    let (n, desc) = parse_frontmatter(&content)?;
    (n == name).then_some((content, desc))
}

fn names(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

/// Copie les skills actives dans `codex_home/skills`, sans suivre de lien :
/// l'ancien `skills` (dossier, fichier ou lien) est retiré, le nouveau créé
/// par `create_dir`, chaque fichier par `File::create_new`.
pub async fn install_into(state: &AppState, codex_home: &Path) -> io::Result<()> {
    let off = disabled(state).await;
    let target = codex_home.join("skills");
    match std::fs::symlink_metadata(&target) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(&target)?,
        Ok(_) => std::fs::remove_file(&target)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::create_dir(&target)?;
    let root = dir(state);
    for name in names(&root) {
        if off.contains(&name) {
            continue;
        }
        if read_skill(&root, &name).is_none() {
            tracing::warn!("skill {name} invalide (SKILL.md ou frontmatter) : non installée");
            continue;
        }
        if let Err(e) = copy_tree(&root.join(&name), &target.join(&name)) {
            tracing::warn!("skill {name} : copie impossible : {e}");
        }
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if ft.is_file() {
            let mut from = std::fs::File::open(entry.path())?;
            let mut out = std::fs::File::create_new(&to)?;
            io::copy(&mut from, &mut out)?;
        } else {
            tracing::warn!(
                "skill : {} n'est ni un fichier ni un dossier, sauté",
                entry.path().display()
            );
        }
    }
    Ok(())
}

// ---- API ----

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let root = dir(&state);
    let off = disabled(&state).await;
    let skills: Vec<Value> = names(&root)
        .into_iter()
        .map(|n| {
            let skill = read_skill(&root, &n);
            json!({
                "name": n,
                "description": skill.as_ref().map(|s| &s.1),
                "valid": skill.is_some(),
                "shipped": SHIPPED.iter().any(|(s, _)| *s == n),
                "enabled": !off.contains(&n),
            })
        })
        .collect();
    Ok(Json(json!({ "skills": skills })))
}

pub async fn get(State(state): State<AppState>, UrlPath(name): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    if !valid_name(&name) {
        return Err(ApiError::NotFound);
    }
    let f = dir(&state).join(&name).join("SKILL.md");
    if !std::fs::symlink_metadata(&f).is_ok_and(|m| m.is_file()) {
        return Err(ApiError::NotFound);
    }
    let content = std::fs::read_to_string(f)?;
    Ok(Json(json!({
        "name": name,
        "content": content,
        "shipped": SHIPPED.iter().any(|(s, _)| *s == name),
        "enabled": !disabled(&state).await.contains(&name),
    })))
}

#[derive(Deserialize)]
pub struct PutBody {
    content: String,
}

/// Crée ou modifie `SKILL.md` (frontmatter vérifié, 64 Kio au plus).
pub async fn put(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
    ApiJson(b): ApiJson<PutBody>,
) -> Result<Json<Value>, ApiError> {
    if !valid_name(&name) {
        return Err(ApiError::InvalidName);
    }
    if b.content.len() > MAX_SKILL {
        return Err(ApiError::TooLarge);
    }
    match parse_frontmatter(&b.content) {
        Some((n, _)) if n == name => {}
        _ => return Err(ApiError::InvalidSkill),
    }
    let d = dir(&state).join(&name);
    if std::fs::symlink_metadata(&d).is_ok_and(|m| !m.is_dir()) {
        return Err(ApiError::InvalidSkill);
    }
    let content = state.masker().mask(&b.content).into_owned();
    write_atomic(&d.join("SKILL.md"), &content)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct EnabledBody {
    enabled: bool,
}

pub async fn put_enabled(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
    ApiJson(b): ApiJson<EnabledBody>,
) -> Result<Json<Value>, ApiError> {
    if !valid_name(&name) || !dir(&state).join(&name).is_dir() {
        return Err(ApiError::NotFound);
    }
    let mut off = disabled(&state).await;
    off.retain(|n| n != &name);
    if !b.enabled {
        off.push(name);
    }
    off.sort();
    set_setting(&state, DISABLED_KEY, Some(&serde_json::to_string(&off)?)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter() {
        assert_eq!(
            parse_frontmatter("---\nname: demo\ndescription: \"Fait X\"\n---\n# Corps"),
            Some(("demo".into(), "Fait X".into()))
        );
        assert_eq!(parse_frontmatter("---\ndescription: x\n---\n"), None);
        assert_eq!(parse_frontmatter("---\nname: demo\ndescription:\n---\n"), None);
        assert_eq!(parse_frontmatter("# Pas de frontmatter\nname: demo\n"), None);
        assert_eq!(parse_frontmatter("---\nname: demo\ndescription: x\n"), None);
        for (name, content) in SHIPPED {
            assert_eq!(parse_frontmatter(content).unwrap().0, *name);
        }
    }

    #[test]
    fn names_must_be_slugs() {
        assert!(valid_name("memory-compaction"));
        assert!(!valid_name("Memory"));
        assert!(!valid_name("../x"));
        assert!(!valid_name(""));
    }
}
