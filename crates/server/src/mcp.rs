//! Serveur MCP intégré (`POST /mcp`, HTTP « streamable », réponses JSON) :
//! outils mémoire, recall et web pour Codex dans les sandboxes. Seule la clé
//! interne d'une sandbox est acceptée ; elle fixe la session, donc le projet
//! et le topic. Une requête JSON-RPC par appel, notifications → 202.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Map, Value, json};

use crate::{
    board, bridge,
    error::V1Error,
    memory::{self, File, Level, Refusal},
    recall, sandboxes, sessions,
    state::{AppState, now},
    v1, webtools, workspace,
};

const BODY_LIMIT: usize = 1024 * 1024;
const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26"];
const TEXT_MAX: usize = 500;
const NEW_MAX: usize = 2000;
const MSG_MAX: usize = 2000;

pub fn router(state: &AppState) -> Router<AppState> {
    let _ = state;
    Router::new()
        .route(
            "/mcp",
            post(handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .delete(|| async { StatusCode::OK }),
        )
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

/// Session appelante.
struct Caller {
    session_id: String,
    project: String,
    topic: String,
    project_id: i64,
    topic_id: i64,
}

async fn caller(state: &AppState, peer: std::net::SocketAddr, headers: &HeaderMap) -> Result<Caller, V1Error> {
    let (_, internal, name) = v1::authenticate(state, peer, headers).await?;
    let forbidden = |m: &str| V1Error::new(StatusCode::FORBIDDEN, "key_scope", m);
    if !internal {
        return Err(forbidden("le serveur MCP n'accepte que la clé d'une sandbox"));
    }
    let prefix = sandboxes::key_name("");
    let Some(sid) = name.strip_prefix(&prefix) else {
        return Err(forbidden("clé interne sans session"));
    };
    let row = sessions::row(state, sid)
        .await
        .map_err(|_| forbidden("session inconnue"))?;
    let t = workspace::topic(state, row.topic_id)
        .await
        .map_err(|_| forbidden("topic inconnu"))?;
    Ok(Caller {
        session_id: sid.to_string(),
        project: t.project_slug,
        topic: t.slug,
        project_id: t.project_id,
        topic_id: t.id,
    })
}

fn rpc_result(id: &Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})).into_response()
}

async fn handle(State(state): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let who = match caller(&state, v1::peer_of(&parts.extensions), &parts.headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let body = match axum::body::to_bytes(body, BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return rpc_error(&Value::Null, -32700, "JSON illisible"),
    };
    let Some(obj) = msg.as_object() else {
        return rpc_error(&Value::Null, -32600, "une requête JSON-RPC par appel");
    };
    let method = obj.get("method").and_then(Value::as_str).unwrap_or("");
    let Some(id) = obj.get("id").cloned() else {
        // Notification (`notifications/initialized`…) ou réponse : rien à rendre.
        return StatusCode::ACCEPTED.into_response();
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let version = if PROTOCOLS.contains(&asked) {
                asked
            } else {
                PROTOCOLS[0]
            };
            let mut res = rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "enginex", "version": env!("CARGO_PKG_VERSION")},
                }),
            );
            if let Ok(v) = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()) {
                res.headers_mut().insert("mcp-session-id", v);
            }
            res
        }
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(&id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let Some(schema) = tools().into_iter().find(|t| t["name"] == name) else {
                return rpc_error(&id, -32602, &format!("outil inconnu : {name}"));
            };
            let args = match params.get("arguments") {
                None | Some(Value::Null) => Value::Object(Map::new()),
                Some(a) => a.clone(),
            };
            if let Err(e) = validate(&args, &schema["inputSchema"]) {
                return rpc_error(&id, -32602, &e);
            }
            let out = call(&state, &who, name, &args).await;
            let (text, is_error) = match out {
                Ok(t) => (t, false),
                Err(t) => (t, true),
            };
            rpc_result(
                &id,
                json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
            )
        }
        _ => rpc_error(&id, -32601, &format!("méthode inconnue : {method}")),
    }
}

// ---- Outils ----

fn level_prop() -> Value {
    json!({"type": "string", "enum": ["global", "project", "topic"],
           "description": "global, project (le projet de la session) ou topic (le topic de la session)"})
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        },
    })
}

pub fn tools() -> Vec<Value> {
    vec![
        tool(
            "mem_read",
            "Lit memory.md (notes de travail) ou instructions.md (consignes de Michael, lecture seule) d'un niveau.",
            json!({"level": level_prop(), "file": {"type": "string", "enum": ["memory", "instructions"]}}),
            &["level"],
        ),
        tool(
            "mem_add",
            "Ajoute un fait durable, daté, à la fin de memory.md : une ligne courte, sans secret.",
            json!({"level": level_prop(), "text": {"type": "string", "description": "le fait, sur une ligne (500 caractères au plus)"}}),
            &["level", "text"],
        ),
        tool(
            "mem_replace",
            "Remplace un extrait unique de memory.md (new vide : retire la ligne).",
            json!({"level": level_prop(), "old": {"type": "string"}, "new": {"type": "string"}}),
            &["level", "old", "new"],
        ),
        tool(
            "mem_write",
            "Réécrit tout memory.md (compactage). Lis-le d'abord avec mem_read.",
            json!({"level": level_prop(), "content": {"type": "string"}}),
            &["level", "content"],
        ),
        tool(
            "recall",
            "Cherche dans les messages des sessions passées (Michael et agent).",
            json!({
                "query": {"type": "string"},
                "scope": {"type": "string", "enum": ["all", "project", "topic"]},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20},
            }),
            &["query"],
        ),
        tool(
            "session_read",
            "Lit les messages d'une session (ids donnés par recall), dans l'ordre.",
            json!({
                "session_id": {"type": "string"},
                "from_seq": {"type": "integer", "minimum": 1},
                "limit": {"type": "integer", "minimum": 1, "maximum": 40},
            }),
            &["session_id"],
        ),
        tool(
            "web_search",
            "Recherche sur le web (SearXNG) : titres, adresses et extraits. Lis ensuite une page avec web_read.",
            json!({
                "query": {"type": "string"},
                "count": {"type": "integer", "minimum": 1, "maximum": 10},
            }),
            &["query"],
        ),
        tool(
            "web_read",
            "Lit une page web publique et la rend en texte (HTML converti). Adresses privées ou locales refusées.",
            json!({
                "url": {"type": "string", "description": "adresse http(s) complète"},
                "max_chars": {"type": "integer", "minimum": 1000, "maximum": webtools::READ_MAX_CHARS},
            }),
            &["url"],
        ),
        tool(
            "tasks_list",
            "Liste les tâches du tableau de Michael (kanban), avec leur id.",
            json!({
                "status": {"type": "string", "enum": ["todo", "doing", "done", "open", "all"],
                           "description": "open (défaut) = à faire + en cours"},
                "project": {"type": "string", "description": "nom du projet de tâches"},
            }),
            &[],
        ),
        tool(
            "tasks_add",
            "Ajoute une tâche au tableau de Michael.",
            json!({
                "title": {"type": "string"},
                "notes": {"type": "string"},
                "due": {"type": "string", "description": "échéance AAAA-MM-JJ"},
                "project": {"type": "string", "description": "nom du projet de tâches (créé s'il n'existe pas)"},
                "status": {"type": "string", "enum": ["todo", "doing", "done"]},
            }),
            &["title"],
        ),
        tool(
            "tasks_update",
            "Modifie une tâche (id donné par tasks_list) : statut, titre, notes, échéance (vide = sans).",
            json!({
                "id": {"type": "string"},
                "status": {"type": "string", "enum": ["todo", "doing", "done"]},
                "title": {"type": "string"},
                "notes": {"type": "string"},
                "due": {"type": "string"},
            }),
            &["id"],
        ),
    ]
}

/// Contrôle des arguments selon le schéma (objet, clés connues, requis,
/// types `string`/`integer`, `enum`, bornes).
pub fn validate(args: &Value, schema: &Value) -> Result<(), String> {
    let Some(obj) = args.as_object() else {
        return Err("arguments : objet attendu".into());
    };
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    for k in obj.keys() {
        if !props.contains_key(k) {
            return Err(format!("argument inconnu : {k}"));
        }
    }
    for r in schema["required"].as_array().into_iter().flatten() {
        let r = r.as_str().unwrap_or("");
        if !obj.contains_key(r) {
            return Err(format!("argument manquant : {r}"));
        }
    }
    for (k, v) in obj {
        let p = &props[k];
        match p["type"].as_str() {
            Some("string") => {
                let Some(s) = v.as_str() else {
                    return Err(format!("{k} : texte attendu"));
                };
                if let Some(e) = p["enum"].as_array()
                    && !e.iter().any(|x| x == s)
                {
                    return Err(format!("{k} : valeur inconnue {s:?}"));
                }
            }
            Some("integer") => {
                let Some(n) = v.as_i64() else {
                    return Err(format!("{k} : entier attendu"));
                };
                if p["minimum"].as_i64().is_some_and(|m| n < m) || p["maximum"].as_i64().is_some_and(|m| n > m) {
                    return Err(format!("{k} : hors des bornes"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Contrôle du texte de `mem_add`.
pub fn check_fact(text: &str) -> Result<&str, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("text est vide".into());
    }
    if t.contains('\n') || t.contains('\r') {
        return Err("text doit tenir sur une ligne".into());
    }
    if t.chars().count() > TEXT_MAX {
        return Err(format!("text dépasse {TEXT_MAX} caractères : fais plus court"));
    }
    if t.starts_with("- ") {
        return Err("text ne commence pas par « - » : la date et le tiret sont ajoutés".into());
    }
    Ok(t)
}

fn too_large() -> String {
    "memory.md dépasse 16 Kio : compacte-le (skill memory-compaction, mem_write)".into()
}

fn refusal(r: Refusal) -> String {
    match r {
        Refusal::Changed => "memory.md a changé depuis ta lecture : relis-le (mem_read) et recommence".into(),
        Refusal::Msg(m) => m,
        Refusal::TooLarge => too_large(),
        Refusal::Internal(e) => {
            tracing::error!("mémoire : {e:#}");
            "erreur interne d'EngineX".into()
        }
    }
}

/// Ligne « vide » après un retrait : rien, ou seulement le tiret et la date.
fn blank_line(line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() || l == "-" {
        return true;
    }
    let Some(rest) = l.strip_prefix("- ") else {
        return false;
    };
    let rest = rest.trim_end_matches(':').trim();
    rest.len() == 10
        && rest
            .chars()
            .enumerate()
            .all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })
}

/// Remplace l'unique occurrence de `old` ; retire la ligne devenue vide.
pub fn replace_once(content: &str, old: &str, new: &str) -> Result<String, String> {
    if old.is_empty() {
        return Err("old est vide".into());
    }
    let n = content.matches(old).count();
    if n == 0 {
        return Err("texte introuvable dans memory.md : relis-le avec mem_read".into());
    }
    if n > 1 {
        return Err(format!("{n} occurrences : donne un extrait plus long"));
    }
    let at = content.find(old).expect("trouvé");
    let out = format!("{}{new}{}", &content[..at], &content[at + old.len()..]);
    let start = out[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = out[at + new.len()..]
        .find('\n')
        .map_or(out.len(), |i| at + new.len() + i);
    if blank_line(&out[start..end]) {
        let end = if end < out.len() { end + 1 } else { end };
        return Ok(format!("{}{}", &out[..start], &out[end..]));
    }
    Ok(out)
}

fn level_of(args: &Value) -> Level {
    Level::parse(args["level"].as_str().unwrap_or("")).unwrap_or(Level::Topic)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

async fn call(state: &AppState, who: &Caller, name: &str, args: &Value) -> Result<String, String> {
    let hub = bridge::hub(state, &who.session_id);
    let level = level_of(args);
    let author = format!("session:{}", who.session_id);
    let ctx = Some((who.session_id.as_str(), who.project.as_str(), who.topic.as_str()));
    let mem_rel = memory::rel_path(level, File::Memory, &who.project, &who.topic);
    // Après une écriture de la session : `last_read` suit s'il était à jour.
    let follow = |out: &memory::Outcome| {
        if out.changed && hub.last_read(&mem_rel).as_deref() == Some(out.before_sha.as_str()) {
            hub.set_last_read(&mem_rel, &out.after_sha);
        }
    };
    match name {
        "mem_read" => {
            let file = File::parse(args["file"].as_str().unwrap_or("memory")).unwrap_or(File::Memory);
            let rel = memory::rel_path(level, file, &who.project, &who.topic);
            let content = memory::read(state, &rel);
            hub.set_last_read(&rel, &memory::sha_hex(&content));
            Ok(if content.trim().is_empty() {
                "(vide)".into()
            } else {
                content
            })
        }
        "mem_add" => {
            let text = check_fact(args["text"].as_str().unwrap_or(""))?;
            let text = state.masker().mask(text).into_owned();
            let line = format!("- {} : {text}", memory::ymd(now()));
            let out = memory::update(state, &mem_rel, &author, ctx, |old| {
                if old.contains(&text) {
                    return Ok(None);
                }
                let mut new = old.to_string();
                if !new.is_empty() && !new.ends_with('\n') {
                    new.push('\n');
                }
                new.push_str(&line);
                new.push('\n');
                Ok(Some(new))
            })
            .await
            .map_err(refusal)?;
            if !out.changed {
                return Ok("déjà noté".into());
            }
            follow(&out);
            let mut msg = format!("noté : {line}");
            if out.size > memory::WARN_MEMORY {
                msg.push_str(&format!(
                    "\nmemory.md : {} Kio sur 16 ; compacte-le bientôt (skill memory-compaction)",
                    out.size.div_ceil(1024)
                ));
            }
            Ok(msg)
        }
        "mem_replace" => {
            let old_s = args["old"].as_str().unwrap_or("").to_string();
            let new_s = args["new"].as_str().unwrap_or("").to_string();
            if new_s.chars().count() > NEW_MAX {
                return Err(format!("new dépasse {NEW_MAX} caractères"));
            }
            let new_s = state.masker().mask(&new_s).into_owned();
            let out = memory::update(state, &mem_rel, &author, ctx, |old| {
                replace_once(old, &old_s, &new_s).map(Some).map_err(Refusal::Msg)
            })
            .await
            .map_err(refusal)?;
            follow(&out);
            Ok(if out.changed {
                "remplacé".into()
            } else {
                "rien n'a changé".into()
            })
        }
        "mem_write" => {
            let Some(seen) = hub.last_read(&mem_rel) else {
                return Err("lis d'abord memory.md avec mem_read".into());
            };
            let content = args["content"].as_str().unwrap_or("").to_string();
            let out = memory::update(state, &mem_rel, &author, ctx, |old| {
                if memory::sha_hex(old) != seen {
                    return Err(Refusal::Changed);
                }
                Ok(Some(content))
            })
            .await
            .map_err(refusal)?;
            hub.set_last_read(&mem_rel, &out.after_sha);
            Ok(format!("memory.md réécrit ({} octets)", out.size))
        }
        "recall" => {
            let scope = recall::Scope::parse(args["scope"].as_str().unwrap_or("all")).unwrap_or(recall::Scope::All);
            let limit = args["limit"].as_i64().unwrap_or(8);
            let q = args["query"].as_str().unwrap_or("");
            if recall::match_query(q, true).is_none() {
                return Err("query : donne au moins un mot de 2 lettres".into());
            }
            let hits = recall::search(state, q, scope, who.project_id, who.topic_id, limit)
                .await
                .map_err(|e| {
                    tracing::error!("recall : {e}");
                    "erreur de recherche".to_string()
                })?;
            if hits.is_empty() {
                return Ok("aucun résultat".into());
            }
            Ok(hits.iter().map(recall::Hit::line).collect::<Vec<_>>().join("\n"))
        }
        "session_read" => {
            let sid = args["session_id"].as_str().unwrap_or("");
            let from = args["from_seq"].as_i64().unwrap_or(1);
            let limit = args["limit"].as_i64().unwrap_or(20);
            let row = sessions::row(state, sid)
                .await
                .map_err(|_| format!("session inconnue : {sid}"))?;
            // Un de plus pour savoir s'il reste des messages.
            let msgs = recall::read_session(state, sid, from, limit + 1).await.map_err(|e| {
                tracing::error!("session_read : {e}");
                "erreur de lecture".to_string()
            })?;
            let mut out = vec![format!("« {} » (session {sid})", row.title)];
            for m in msgs.iter().take(limit as usize) {
                out.push(format!(
                    "n° {} {} : {}",
                    m.seq,
                    if m.from_user { "Michael" } else { "agent" },
                    truncate(&m.text, MSG_MAX)
                ));
            }
            if msgs.len() as i64 > limit {
                out.push(format!("(suite : from_seq = {})", msgs[limit as usize].seq));
            } else if msgs.is_empty() {
                out.push("(aucun message)".into());
            }
            Ok(out.join("\n"))
        }
        "web_search" => {
            let q = args["query"].as_str().unwrap_or("").trim();
            if q.is_empty() {
                return Err("query est vide".into());
            }
            let count = args["count"].as_i64().unwrap_or(8) as usize;
            let hits = webtools::search(state, q, count).await?;
            if hits.is_empty() {
                return Ok(format!("Aucun résultat pour « {q} »."));
            }
            Ok(hits
                .iter()
                .enumerate()
                .map(|(i, h)| format!("{}. {}\n   {}\n   {}", i + 1, h.title, h.url, truncate(&h.snippet, 300)))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "web_read" => {
            let url = args["url"].as_str().unwrap_or("");
            let max = args["max_chars"]
                .as_i64()
                .map_or(webtools::READ_DEFAULT_CHARS, |n| n as usize);
            let page = webtools::read(state, url).await?;
            Ok(format!(
                "Adresse : {}\nType : {}\n\n{}",
                page.url,
                if page.content_type.is_empty() {
                    "?"
                } else {
                    &page.content_type
                },
                webtools::cut(&page.text, max)
            ))
        }
        "tasks_list" => {
            let b = board::read(state).await.map_err(board::Refusal::message)?;
            let status = args["status"].as_str().unwrap_or("open");
            let project = args["project"].as_str().map(str::trim);
            let pid = match project {
                Some(p) => Some(
                    b.projects
                        .iter()
                        .find(|x| x.name.eq_ignore_ascii_case(p))
                        .map(|x| x.id.clone())
                        .ok_or_else(|| format!("projet de tâches « {p} » introuvable"))?,
                ),
                None => None,
            };
            let mut tasks: Vec<&board::Task> = b
                .tasks
                .iter()
                .filter(|t| match status {
                    "all" => true,
                    "open" => t.status != board::Status::Done,
                    s => board::Status::parse(s) == Some(t.status),
                })
                .filter(|t| pid.is_none() || t.project_id == pid)
                .collect();
            tasks.sort_by(|a, b| {
                (a.status as u8, a.order)
                    .partial_cmp(&(b.status as u8, b.order))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if tasks.is_empty() {
                return Ok("(aucune tâche)".into());
            }
            let shown: Vec<String> = tasks.iter().take(200).map(|t| board::line(&b, t)).collect();
            let more = tasks.len().saturating_sub(200);
            Ok(if more > 0 {
                format!("{}\n(et {more} autres)", shown.join("\n"))
            } else {
                shown.join("\n")
            })
        }
        "tasks_add" => {
            let by = format!("session:{}", who.session_id);
            let t = board::update(state, |b| {
                let project_id = match args["project"].as_str().map(str::trim).filter(|p| !p.is_empty()) {
                    Some(p) => Some(b.project_by_name(p)?),
                    None => None,
                };
                let new = board::NewTask {
                    title: args["title"].as_str().unwrap_or("").to_string(),
                    notes: args["notes"].as_str().map(str::to_string),
                    status: args["status"].as_str().and_then(board::Status::parse),
                    project_id,
                    due: args["due"]
                        .as_str()
                        .filter(|d| !d.trim().is_empty())
                        .map(str::to_string),
                };
                let t = b.add_task(new, &by)?;
                Ok(board::line(b, &t))
            })
            .await
            .map_err(board::Refusal::message)?;
            Ok(format!("Tâche ajoutée : {t}"))
        }
        "tasks_update" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            let mut patch = args.as_object().cloned().unwrap_or_default();
            patch.remove("id");
            if patch.is_empty() {
                return Err("rien à modifier : donne status, title, notes ou due".into());
            }
            let t = board::update(state, |b| {
                let t = b.patch_task(&id, &patch)?;
                Ok(board::line(b, &t))
            })
            .await
            .map_err(board::Refusal::message)?;
            Ok(format!("Tâche modifiée : {t}"))
        }
        _ => Err(format!("outil inconnu : {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(name: &str) -> Value {
        tools().into_iter().find(|t| t["name"] == name).unwrap()["inputSchema"].clone()
    }

    #[test]
    fn argument_validation() {
        let s = schema("mem_add");
        assert!(validate(&json!({"level": "topic", "text": "x"}), &s).is_ok());
        assert!(validate(&json!({"level": "galaxie", "text": "x"}), &s).is_err());
        assert!(validate(&json!({"level": "topic"}), &s).is_err());
        assert!(validate(&json!({"level": "topic", "text": "x", "file": "instructions"}), &s).is_err());
        assert!(validate(&json!({"level": "topic", "text": 3}), &s).is_err());
        assert!(validate(&json!([]), &s).is_err());
        let r = schema("recall");
        assert!(validate(&json!({"query": "a", "limit": 20}), &r).is_ok());
        assert!(validate(&json!({"query": "a", "limit": 21}), &r).is_err());
        assert!(validate(&json!({"query": "a", "limit": "3"}), &r).is_err());
    }

    #[test]
    fn fact_checks() {
        assert_eq!(check_fact("  port 8095 ").unwrap(), "port 8095");
        assert!(check_fact("deux\nlignes").is_err());
        assert!(check_fact(&"x".repeat(501)).is_err());
        assert!(check_fact(&"é".repeat(500)).is_ok());
        assert!(check_fact("- déjà une puce").is_err());
        assert!(check_fact("   ").is_err());
    }

    #[test]
    fn replacement() {
        let c = "- 2026-11-19 : port 8095\n- 2026-11-19 : proxy caddy\n";
        assert_eq!(
            replace_once(c, "8095", "8096").unwrap(),
            "- 2026-11-19 : port 8096\n- 2026-11-19 : proxy caddy\n"
        );
        assert_eq!(
            replace_once(c, "port 8095", "").unwrap(),
            "- 2026-11-19 : proxy caddy\n"
        );
        assert_eq!(
            replace_once(c, "- 2026-11-19 : proxy caddy", "").unwrap(),
            "- 2026-11-19 : port 8095\n"
        );
        assert!(replace_once(c, "absent", "x").unwrap_err().contains("introuvable"));
        assert!(
            replace_once(c, "2026-11-19", "x")
                .unwrap_err()
                .starts_with("2 occurrences")
        );
    }
}
