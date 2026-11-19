//! Recall : recherche plein texte dans les messages des sessions (FTS5,
//! `recall_fts` + `recall_docs`, tenus par les triggers de `0006_memory.sql`).
//! Les tours automatiques (`verify`, `memory`) et les tours retirés sont
//! filtrés à la lecture.

use crate::state::AppState;

const MAX_WORDS: usize = 12;

/// Requête FTS5 : mots (lettres et chiffres) de 2 caractères ou plus, au plus
/// 12, chacun en préfixe `"mot"*` ; ET (espaces) ou OU (`any`). `None` s'il ne
/// reste aucun mot.
pub fn match_query(q: &str, any: bool) -> Option<String> {
    let words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .take(MAX_WORDS)
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!words.is_empty()).then(|| words.join(if any { " OR " } else { " " }))
}

/// Condition commune : élément d'un tour de Michael (ou sans tour), non retiré.
/// Alias attendus : `i` (chat_items), `tu` (chat_turns, LEFT JOIN).
pub const VISIBLE: &str = "(tu.id IS NULL OR tu.origin = 'user') \
     AND json_extract(i.data, '$.origin') IS NULL \
     AND NOT EXISTS (SELECT 1 FROM chat_reverted r WHERE r.session_id = i.session_id AND r.turn_id = i.turn_id)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Project,
    Topic,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Self::All),
            "project" => Some(Self::Project),
            "topic" => Some(Self::Topic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub date: i64,
    pub project: String,
    pub topic: String,
    pub title: String,
    pub session_id: String,
    pub seq: i64,
    pub from_user: bool,
    pub snippet: String,
}

impl Hit {
    /// `[AAAA-MM-JJ] <projet>/<topic> · « <titre> » (session <id>, n° <seq>) Michael|agent : <extrait>`
    pub fn line(&self) -> String {
        format!(
            "[{}] {}/{} · « {} » (session {}, n° {}) {} : {}",
            crate::memory::ymd(self.date),
            self.project,
            self.topic,
            self.title,
            self.session_id,
            self.seq,
            if self.from_user { "Michael" } else { "agent" },
            self.snippet.replace('\n', " ")
        )
    }
}

type HitRow = (String, i64, String, String, i64, String, String, String);

async fn run(
    state: &AppState,
    m: &str,
    scope: Scope,
    project_id: i64,
    topic_id: i64,
    limit: i64,
) -> Result<Vec<Hit>, sqlx::Error> {
    let filter = match scope {
        Scope::All => "1 = 1",
        Scope::Project => "t.project_id = ?",
        Scope::Topic => "s.topic_id = ?",
    };
    let sql = format!(
        "SELECT i.session_id, i.seq, i.kind, snippet(recall_fts, 0, '«', '»', '…', 12), \
         COALESCE(tu.started_at, s.updated_at), p.slug, t.slug, s.title \
         FROM recall_fts f \
         JOIN recall_docs d ON d.id = f.rowid \
         JOIN chat_items i ON i.session_id = d.session_id AND i.item_id = d.item_id \
         JOIN chat_sessions s ON s.id = i.session_id \
         JOIN topics t ON t.id = s.topic_id \
         JOIN projects p ON p.id = t.project_id \
         LEFT JOIN chat_turns tu ON tu.session_id = i.session_id AND tu.id = i.turn_id \
         WHERE recall_fts MATCH ? AND {VISIBLE} AND {filter} \
         ORDER BY bm25(recall_fts) LIMIT ?"
    );
    let mut q = sqlx::query_as::<_, HitRow>(&sql).bind(m);
    match scope {
        Scope::All => {}
        Scope::Project => q = q.bind(project_id),
        Scope::Topic => q = q.bind(topic_id),
    }
    let rows = q.bind(limit).fetch_all(state.db()).await?;
    Ok(rows
        .into_iter()
        .map(|(session_id, seq, kind, snippet, date, project, topic, title)| Hit {
            date,
            project,
            topic,
            title,
            session_id,
            seq,
            from_user: kind == "userMessage",
            snippet,
        })
        .collect())
}

/// Recherche en ET, puis en OU si rien n'est trouvé. `scope` se rapporte au
/// projet et au topic de la session appelante.
pub async fn search(
    state: &AppState,
    q: &str,
    scope: Scope,
    project_id: i64,
    topic_id: i64,
    limit: i64,
) -> Result<Vec<Hit>, sqlx::Error> {
    let Some(all) = match_query(q, false) else {
        return Ok(Vec::new());
    };
    let hits = run(state, &all, scope, project_id, topic_id, limit).await?;
    if !hits.is_empty() {
        return Ok(hits);
    }
    match match_query(q, true) {
        Some(any) if any != all => run(state, &any, scope, project_id, topic_id, limit).await,
        _ => Ok(hits),
    }
}

#[derive(Debug, Clone)]
pub struct Msg {
    pub seq: i64,
    pub from_user: bool,
    pub text: String,
}

/// Messages d'une session dans l'ordre, à partir de `from_seq`, avec les mêmes
/// exclusions que la recherche.
pub async fn read_session(
    state: &AppState,
    session_id: &str,
    from_seq: i64,
    limit: i64,
) -> Result<Vec<Msg>, sqlx::Error> {
    let sql = format!(
        "SELECT i.seq, i.kind, COALESCE(json_extract(i.data, '$.text'), '') FROM chat_items i \
         LEFT JOIN chat_turns tu ON tu.session_id = i.session_id AND tu.id = i.turn_id \
         WHERE i.session_id = ? AND i.seq >= ? AND i.kind IN ('userMessage', 'agentMessage') AND {VISIBLE} \
         ORDER BY i.seq LIMIT ?"
    );
    let rows: Vec<(i64, String, String)> = sqlx::query_as(&sql)
        .bind(session_id)
        .bind(from_seq)
        .bind(limit)
        .fetch_all(state.db())
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !r.2.is_empty())
        .map(|(seq, kind, text)| Msg {
            seq,
            from_user: kind == "userMessage",
            text,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::match_query;

    #[test]
    fn query_forms() {
        assert_eq!(
            match_query("vérif port", false).as_deref(),
            Some("\"vérif\"* \"port\"*")
        );
        assert_eq!(
            match_query("vérif port", true).as_deref(),
            Some("\"vérif\"* OR \"port\"*")
        );
        assert_eq!(match_query("a \"b\" cc", false).as_deref(), Some("\"cc\"*"));
        assert_eq!(match_query("  ; a ", false), None);
        assert_eq!(match_query("", true), None);
        let many = (0..20).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        assert_eq!(match_query(&many, false).unwrap().matches('*').count(), 12);
        // Guillemets et opérateurs FTS ne passent pas : seuls les mots restent.
        assert_eq!(
            match_query("x\" OR NEAR(y", false).as_deref(),
            Some("\"OR\"* \"NEAR\"*")
        );
    }
}
