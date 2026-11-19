//! Navigateur piloté en CDP : le Chromium de la stack (`ENGINEX_BROWSER_URL`),
//! un onglet par session d'EngineX, ouvert au premier outil `browser_*` et
//! fermé avec la sandbox. Michael regarde l'onglet (screencast) et peut
//! prendre la main (connexion à un site) : les outils attendent alors.
//!
//! Repris de Loki (`rust/loki-rs/src/browser/`, MIT, voir NOTICE.md) :
//! scripts injectés, repères numérotés, snapshot, recherche, clics, saisie,
//! touches, défilement, capture quadrillée. Changé : connexion à un
//! navigateur distant au lieu d'un lancement local, onglets par session,
//! screencast et entrées de l'utilisateur.

pub mod grid;
pub mod js;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Map, Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// `cuViewW`, `cuViewH` : le viewport (et donc la capture), fixe.
pub const VIEW_W: i64 = 1280;
pub const VIEW_H: i64 = 800;
/// `cuMaxOutput`.
pub const MAX_OUTPUT: usize = 8000;

/// Les outils servis ici.
pub fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "browser_open"
            | "browser_snapshot"
            | "browser_find"
            | "browser_click"
            | "browser_click_xy"
            | "browser_type"
            | "browser_key"
            | "browser_scroll"
            | "browser_screenshot"
    )
}

/// `cuAutoShotTool` : les outils après lesquels la page est capturée pour
/// l'aperçu.
pub fn auto_shot_tool(name: &str) -> bool {
    matches!(
        name,
        "browser_open"
            | "browser_click"
            | "browser_click_xy"
            | "browser_type"
            | "browser_key"
            | "browser_scroll"
            | "browser_find"
    )
}

/// `capCUOutput`.
pub fn cap_output(s: &str) -> String {
    match s.char_indices().nth(MAX_OUTPUT) {
        None => s.to_string(),
        Some((i, _)) => format!("{}\n…[tronqué]", &s[..i]),
    }
}

/// `errCUDisabled` : clic sur un élément désactivé. Vu sur un configurateur :
/// le modèle recliquait sans fin un « Continuer » grisé faute d'un choix
/// obligatoire.
pub const CLICK_DISABLED: &str = "élément désactivé : le site attend d'abord autre chose (un choix obligatoire non coché, un champ vide). Cherche les options sans [coché] au-dessus (browser_find), ne reclique pas";

/// `cuRefInfo` : position et état d'un élément marqué.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct RefInfo {
    x: f64,
    y: f64,
    /// Désactivé.
    dis: bool,
    /// Case radio / à cocher.
    tog: bool,
    /// Cochée.
    on: bool,
}

/// `cuRepeat` : garde-fou anti-boucle. Une même action qui ne change rien,
/// plusieurs fois de suite, reçoit un avertissement clair (vu : plus de 60
/// actions sur un bouton grisé, « page inchangée » à chaque fois).
#[derive(Debug, Default)]
pub struct Repeat {
    key: String,
    n: usize,
}

impl Repeat {
    /// Note le résultat `out` de l'action `key` et y ajoute l'avertissement
    /// à partir de la deuxième répétition sans effet.
    pub fn note(&mut self, key: &str, mut out: String) -> String {
        let no_effect = out.contains("page inchangée") || out.starts_with("[refusé]");
        if !no_effect {
            self.key.clear();
            self.n = 0;
            return out;
        }
        if key == self.key {
            self.n += 1;
        } else {
            self.key = key.to_string();
            self.n = 1;
        }
        if self.n >= 2 {
            out.push_str(&format!(
                "\n\n⚠ Même action sans effet {} fois de suite : tu tournes en rond. Change d'approche (une option obligatoire non cochée ? browser_find sur les choix ; un autre bouton ; ou dis à l'utilisateur ce qui bloque).",
                self.n
            ));
        }
        out
    }
}

/// Le texte de `browser_find` : les éléments (cliquables) puis les passages
/// de texte simple (à lire) qui contiennent la requête.
pub fn format_find(query: &str, refs: &[String], texts: &[String]) -> String {
    if refs.is_empty() && texts.is_empty() {
        return format!(
            "Rien ne contient « {query} » sur cette page, ni élément ni texte. Essaie un autre mot, ou c'est peut-être ailleurs (autre page/lien)."
        );
    }
    let mut b = String::new();
    if !refs.is_empty() {
        b.push_str(&format!(
            "{} élément(s) contenant « {query} » (le 1er est amené dans la vue) — clique par numéro :\n{}",
            refs.len(),
            refs.join("\n")
        ));
    }
    if !texts.is_empty() {
        if !b.is_empty() {
            b.push_str("\n\n");
        }
        b.push_str(&format!(
            "Texte de la page contenant « {query} » (à lire, pas à cliquer) :\n- {}",
            texts.join("\n- ")
        ));
    }
    b
}

// --- Garde-fous de fin de tâche -------------------------------------------------
//
// Vu sur une boutique qui refuse les navigateurs automatiques : le modèle
// configurait tout correctement, tombait sur une page d'erreur au moment de
// commander, puis recommençait tout le parcours et tentait des adresses
// devinées, sans jamais répondre. Deux signaux l'aident à conclure ; le tour
// les ajoute (il tient le compte de ses actions), après la coupe de
// [`cap_output`] pour qu'ils ne soient jamais tronqués.

/// `cuActionBudget` : actions du navigateur dans un tour avant le rappel de
/// conclure.
pub const ACTION_BUDGET: usize = 25;

/// `cuErrorTitle` : titre (`# …`) d'une page d'erreur ou de blocage.
fn error_title(line: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?i)^# .*(not found|page introuvable|introuvable|404|403|access denied|accès refusé|forbidden|robot|captcha|are you human|unusual traffic|blocked|bloqué)")
            .expect("motif des pages de blocage")
    })
    .is_match(line)
}

/// Le résultat semble-t-il une page d'erreur ou de blocage (titre dans les
/// premières lignes) ?
pub fn looks_blocked(out: &str) -> bool {
    out.splitn(6, '\n').any(|l| error_title(l.trim()))
}

/// `cuGuard` : les rappels ajoutés au résultat de la `n`-ième action du
/// navigateur du tour.
pub fn guard(mut out: String, n: usize) -> String {
    if looks_blocked(&out) {
        out.push_str("\n\n⚠ Le site a répondu par une page d'erreur ou de blocage. Après une action normale, cela veut souvent dire qu'il refuse les navigateurs automatiques. Ne refais pas le même parcours : une seule autre piste simple au plus, sinon réponds à l'utilisateur avec ce que tu as vu et ce qui bloque.");
    }
    if n >= ACTION_BUDGET {
        out.push_str(&format!(
            "\n\n⏱ {n} actions dans le navigateur pour cette tâche. Si tu n'approches pas clairement du but, arrête et réponds avec ce que tu sais (ce que tu as vérifié, ce qui bloque)."
        ));
    }
    out
}

/// Un nombre comme `json.Marshal` l'écrit (`640`, pas `640.0`).
/// Erreurs de page gardées par onglet (les plus vieilles partent).
pub const CONSOLE_MAX: usize = 40;
const CONSOLE_LINE: usize = 300;

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Une ligne pour le modèle à partir d'un événement CDP : exception non
/// attrapée, `console.error` / `console.assert`, entrée de journal de niveau
/// erreur (ressource en 404 ou 500, CSP…). `None` : rien à signaler.
pub fn console_line(method: &str, p: &Value) -> Option<String> {
    let at = |url: &str, line: Option<i64>| -> String {
        if url.is_empty() {
            return String::new();
        }
        match line {
            Some(l) => format!(" ({url}:{})", l + 1),
            None => format!(" ({url})"),
        }
    };
    let line = match method {
        "Runtime.exceptionThrown" => {
            let d = &p["exceptionDetails"];
            let what = d["exception"]["description"]
                .as_str()
                .and_then(|s| s.lines().next())
                .or_else(|| d["text"].as_str())
                .unwrap_or("erreur");
            format!(
                "exception : {}{}",
                clip(what, CONSOLE_LINE),
                at(d["url"].as_str().unwrap_or(""), d["lineNumber"].as_i64())
            )
        }
        "Runtime.consoleAPICalled" => {
            let kind = p["type"].as_str().unwrap_or("");
            if !matches!(kind, "error" | "assert") {
                return None;
            }
            let args: Vec<String> = p["args"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| match &a["value"] {
                    Value::String(s) => s.clone(),
                    Value::Null => a["description"]
                        .as_str()
                        .and_then(|s| s.lines().next())
                        .unwrap_or("")
                        .to_string(),
                    v => v.to_string(),
                })
                .collect();
            let f = &p["stackTrace"]["callFrames"][0];
            format!(
                "console.{kind} : {}{}",
                clip(&args.join(" "), CONSOLE_LINE),
                at(f["url"].as_str().unwrap_or(""), f["lineNumber"].as_i64())
            )
        }
        "Log.entryAdded" => {
            let e = &p["entry"];
            // L'icône du site demandée d'office par le navigateur n'est pas un défaut de l'app.
            let favicon = e["url"].as_str().is_some_and(|u| u.ends_with("/favicon.ico"));
            if e["level"] != "error" || favicon {
                return None;
            }
            format!(
                "{}{}",
                clip(e["text"].as_str().unwrap_or(""), CONSOLE_LINE),
                at(
                    e["url"].as_str().unwrap_or(""),
                    e["lineNumber"].as_i64().filter(|l| *l > 0)
                )
            )
        }
        _ => return None,
    };
    Some(line)
}

/// Les erreurs de page en bloc : lignes répétées à la suite regroupées
/// (`×N`), 15 lignes au plus. Vide s'il n'y en a pas.
pub fn format_console(lines: &[String]) -> String {
    let mut grouped: Vec<(String, usize)> = Vec::new();
    for l in lines {
        match grouped.last_mut() {
            Some((prev, n)) if prev == l => *n += 1,
            _ => grouped.push((l.clone(), 1)),
        }
    }
    if grouped.is_empty() {
        return String::new();
    }
    let extra = grouped.len().saturating_sub(15);
    let mut out = String::from("Erreurs de la page (console du navigateur) :");
    for (l, n) in grouped.iter().take(15) {
        out.push_str("\n- ");
        out.push_str(l);
        if *n > 1 {
            out.push_str(&format!(" (×{n})"));
        }
    }
    if extra > 0 {
        out.push_str(&format!("\n… et {extra} autre(s)"));
    }
    out
}

fn go_num(f: f64) -> Value {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        json!(f as i64)
    } else {
        json!(f)
    }
}

// --- Session CDP ---------------------------------------------------------------

type Reply = Result<Value, String>;

/// Un onglet du navigateur distant : sa connexion CDP et le multiplexage
/// requête/réponse ; les événements partent aussi vers `events` (screencast).
pub struct Session {
    /// `http://<ip>:<port>` du navigateur (Chrome refuse un `Host` nommé).
    endpoint: String,
    target_id: String,
    out: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Reply>>>>,
    next: AtomicI64,
    /// Requêtes réseau en vol et dernier événement réseau (`settle`).
    net: Arc<Mutex<(i64, Option<Instant>)>>,
    /// Erreurs de la page (exceptions, `console.error`, ressources en échec)
    /// depuis la dernière lecture ([`Session::take_console`]).
    console: Arc<Mutex<VecDeque<String>>>,
    last_snap: Mutex<String>,
    /// `repeatKey` / `repeatN` : dernière action sans effet et son nombre de
    /// répétitions (`cuRepeat`).
    repeat: Mutex<Repeat>,
    closed: Arc<AtomicBool>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pub events: broadcast::Sender<Value>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `http://nom:port` → `http://ip:port` : Chrome n'accepte que `localhost` ou
/// une adresse IP dans `Host` (protection contre le rebinding DNS).
pub async fn resolve(url: &str) -> Result<String, String> {
    let rest = url
        .trim_end_matches('/')
        .strip_prefix("http://")
        .ok_or_else(|| format!("adresse du navigateur invalide ({url}) : http://hôte:port attendu"))?;
    let (host, port) = rest
        .rsplit_once(':')
        .and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h.to_string(), p)))
        .unwrap_or((rest.to_string(), 9222));
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("navigateur introuvable ({host}) : {e}"))?;
    let mut v4 = None;
    let mut any = None;
    for a in addrs {
        if a.is_ipv4() && v4.is_none() {
            v4 = Some(a);
        }
        any.get_or_insert(a);
    }
    let a = v4.or(any).ok_or_else(|| format!("navigateur introuvable ({host})"))?;
    Ok(format!("http://{a}"))
}

fn http() -> reqwest::Client {
    // Réseau interne de la stack : jamais par un proxy du système.
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

impl Session {
    /// Nouvel onglet sur le navigateur `endpoint` (déjà résolu en IP).
    pub async fn attach(endpoint: &str) -> Result<Arc<Session>, String> {
        let t: Value = http()
            .put(format!("{endpoint}/json/new?about:blank"))
            .send()
            .await
            .map_err(|e| format!("navigateur injoignable ({endpoint}) : {e}"))?
            .json()
            .await
            .map_err(|e| format!("réponse du navigateur illisible : {e}"))?;
        let ws_url = t["webSocketDebuggerUrl"].as_str().unwrap_or("").to_string();
        let target_id = t["id"].as_str().unwrap_or("").to_string();
        if ws_url.is_empty() || target_id.is_empty() {
            return Err(format!("le navigateur n'a pas ouvert d'onglet : {t}"));
        }
        let cfg = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(64 << 20))
            .max_frame_size(Some(64 << 20));
        let ws = match tokio_tungstenite::connect_async_with_config(&ws_url, Some(cfg), false).await {
            Ok((ws, _)) => ws,
            Err(e) => {
                let _ = http().get(format!("{endpoint}/json/close/{target_id}")).send().await;
                return Err(format!("connexion CDP impossible : {e}"));
            }
        };
        let (mut sink, mut stream) = ws.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Reply>>>> = Default::default();
        let net: Arc<Mutex<(i64, Option<Instant>)>> = Default::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (events, _) = broadcast::channel(64);
        let writer = tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                if sink.send(WsMessage::Text(m.into())).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        let console: Arc<Mutex<VecDeque<String>>> = Default::default();
        let k2 = console.clone();
        let (p2, n2, c2, e2) = (pending.clone(), net.clone(), closed.clone(), events.clone());
        let reader = tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let data = match msg {
                    WsMessage::Text(t) => t.as_bytes().to_vec(),
                    WsMessage::Binary(b) => b.to_vec(),
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                let Ok(m) = serde_json::from_slice::<Value>(&data) else {
                    continue;
                };
                let id = m.get("id").and_then(Value::as_i64).unwrap_or(0);
                if id != 0 {
                    let tx = lock(&p2).remove(&id);
                    if let Some(tx) = tx {
                        let reply = match m.get("error").filter(|e| !e.is_null()) {
                            Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("").to_string()),
                            None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = tx.send(reply);
                    }
                    continue;
                }
                match m.get("method").and_then(Value::as_str) {
                    Some("Network.requestWillBeSent") => {
                        let mut n = lock(&n2);
                        n.0 += 1;
                        n.1 = Some(Instant::now());
                    }
                    Some("Network.loadingFinished" | "Network.loadingFailed") => {
                        let mut n = lock(&n2);
                        if n.0 > 0 {
                            n.0 -= 1;
                        }
                        n.1 = Some(Instant::now());
                    }
                    Some("Page.screencastFrame" | "Page.frameNavigated") => {
                        let _ = e2.send(m);
                    }
                    Some(method @ ("Runtime.exceptionThrown" | "Runtime.consoleAPICalled" | "Log.entryAdded")) => {
                        if let Some(line) = console_line(method, &m["params"]) {
                            let mut k = lock(&k2);
                            if k.len() >= CONSOLE_MAX {
                                k.pop_front();
                            }
                            k.push_back(line);
                        }
                    }
                    _ => {}
                }
            }
            c2.store(true, Ordering::SeqCst);
            lock(&p2).clear();
        });
        let s = Arc::new(Session {
            endpoint: endpoint.to_string(),
            target_id,
            out,
            pending,
            next: AtomicI64::new(0),
            net,
            console,
            last_snap: Mutex::new(String::new()),
            repeat: Mutex::new(Repeat::default()),
            closed,
            tasks: Mutex::new(vec![writer, reader]),
            events,
        });
        for m in [
            "Page.enable",
            "Runtime.enable",
            "DOM.enable",
            "Network.enable",
            "Log.enable",
        ] {
            let _ = s.call(m, None).await;
        }
        let _ = s
            .call(
                "Emulation.setDeviceMetricsOverride",
                Some(json!({ "width": VIEW_W, "height": VIEW_H, "deviceScaleFactor": 1, "mobile": false })),
            )
            .await;
        // `HeadlessChrome` dans l'agent utilisateur : beaucoup de sites refusent.
        if let Ok(v) = s.call("Browser.getVersion", None).await
            && let Some(ua) = v["userAgent"].as_str()
        {
            let _ = s
                .call(
                    "Network.setUserAgentOverride",
                    Some(json!({ "userAgent": ua.replace("HeadlessChrome", "Chrome") })),
                )
                .await;
        }
        let _ = s
            .call(
                "Page.addScriptToEvaluateOnNewDocument",
                Some(json!({ "source": js::SAME_TAB_JS })),
            )
            .await;
        Ok(s)
    }

    /// `call` : une commande et sa réponse (30 s au plus).
    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let mut payload = Map::new();
        payload.insert("id".into(), json!(id));
        payload.insert("method".into(), json!(method));
        if let Some(p) = params {
            payload.insert("params".into(), p);
        }
        if self.closed.load(Ordering::SeqCst) || self.out.send(Value::Object(payload).to_string()).is_err() {
            lock(&self.pending).remove(&id);
            return Err("connexion CDP fermée".into());
        }
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(msg))) => Err(format!("{method}: {msg}")),
            Ok(Err(_)) => Err("connexion CDP fermée".into()),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(format!("{method}: délai dépassé"))
            }
        }
    }

    /// L'onglet répond-il encore ?
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Ferme la connexion et l'onglet.
    pub async fn close(&self) {
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        let _ = http()
            .get(format!("{}/json/close/{}", self.endpoint, self.target_id))
            .send()
            .await;
    }

    /// Commande CDP sans attendre (accusés du screencast).
    pub fn send_nowait(&self, method: &str, params: Value) {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let msg = json!({ "id": id, "method": method, "params": params });
        let _ = self.out.send(msg.to_string());
    }

    /// `eval` : la valeur d'une expression (`None` : pas de valeur).
    pub async fn eval(&self, expr: &str) -> Result<Option<Value>, String> {
        let res = self
            .call(
                "Runtime.evaluate",
                Some(json!({ "expression": expr, "returnByValue": true, "awaitPromise": true })),
            )
            .await?;
        if let Some(d) = res.get("exceptionDetails").filter(|d| d.is_object()) {
            return Err(format!("JS: {}", d.get("text").and_then(Value::as_str).unwrap_or("")));
        }
        Ok(res.get("result").and_then(|r| r.get("value")).cloned())
    }

    /// `snapshot` : les éléments interactifs visibles, numérotés.
    pub async fn snapshot(&self) -> Result<String, String> {
        let val = self.eval(&js::snapshot_js()).await?;
        let snap = match val {
            None => return Err("snapshot illisible: unexpected end of JSON input".into()),
            Some(Value::Null) => Value::Object(Map::new()),
            Some(o @ Value::Object(_)) => o,
            Some(other) => {
                return Err(format!(
                    "snapshot illisible: json: cannot unmarshal {} into Go value of type loki.cuSnap",
                    go_kind(&other)
                ));
            }
        };
        let s = |k: &str| snap.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str| snap.get(k).and_then(Value::as_i64).unwrap_or(0);
        let refs: Vec<String> = snap
            .get("refs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|r| r.as_str().unwrap_or("").to_string())
            .collect();
        let mut b = format!("# {}\n{}\n\n", s("title"), s("url"));
        if refs.is_empty() {
            b.push_str("(aucun élément interactif détecté — page vide, en cours de chargement, ou rendue sur canvas ; réessaie browser_snapshot, ou utilise browser_screenshot pour voir)");
        } else {
            b.push_str(&format!(
                "{} élément(s) interactif(s) VISIBLE(s) — clique par numéro avec browser_click(ref) :\n",
                refs.len()
            ));
            b.push_str(&refs.join("\n"));
        }
        let (above, below) = (n("above"), n("below"));
        if below > 20 || above > 20 {
            b.push_str("\n\n");
            if below > 20 {
                b.push_str(&format!(
                    "↓ encore ~{below} px sous la vue — browser_scroll(down) pour révéler d'autres éléments. "
                ));
            }
            if above > 20 {
                b.push_str(&format!("↑ ~{above} px au-dessus — browser_scroll(up) pour y revenir."));
            }
        }
        Ok(b.trim_end_matches(' ').to_string())
    }

    /// `snapshotDedup` : une ligne courte si rien n'a changé.
    pub async fn snapshot_dedup(&self) -> Result<String, String> {
        let full = self.snapshot().await?;
        let mut last = lock(&self.last_snap);
        if *last == full {
            return Ok("(page inchangée depuis le dernier affichage — les mêmes numéros restent valides)".into());
        }
        *last = full.clone();
        Ok(full)
    }

    /// `find` : les éléments dont le rôle ou le nom contient la requête.
    pub async fn find(&self, query: &str) -> Result<String, String> {
        let expr = format!(
            "{}({})({})",
            js::HELPERS_JS,
            js::FIND_JS,
            serde_json::to_string(query).unwrap_or_default()
        );
        let val = self.eval(&expr).await?;
        let Some(v) = val else {
            return Err("recherche illisible: unexpected end of JSON input".into());
        };
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|r| r.as_str().unwrap_or("").to_string())
                .collect()
        };
        Ok(format_find(query, &list("refs"), &list("texts")))
    }

    /// `waitLoad` : chargement fini, puis réseau au calme.
    async fn wait_load(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(Value::String(s))) = self.eval("document.readyState").await
                && s == "complete"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        self.settle().await;
    }

    /// `settle` : plus de requête en vol depuis 600 ms (6 s au plus).
    async fn settle(&self) {
        let quiet = Duration::from_millis(600);
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            let (inflight, last) = *lock(&self.net);
            if inflight <= 0 && last.is_none_or(|t| t.elapsed() >= quiet) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    }

    /// `navigate` : une URL, « back » ou « forward ».
    pub async fn navigate(&self, url: &str) -> Result<(), String> {
        lock(&self.last_snap).clear();
        match url.trim().to_lowercase().as_str() {
            "back" => {
                let r = self.eval("history.back();'ok'").await;
                self.wait_load().await;
                return r.map(|_| ());
            }
            "forward" => {
                let r = self.eval("history.forward();'ok'").await;
                self.wait_load().await;
                return r.map(|_| ());
            }
            _ => {}
        }
        let low = url.trim().to_ascii_lowercase();
        if ["file:", "chrome:", "devtools:", "view-source:"]
            .iter()
            .any(|p| low.starts_with(p))
        {
            return Err("adresse refusée : seules les pages web (http, https, data) s'ouvrent ici".into());
        }
        let url = if url.contains("://") || low.starts_with("data:") || low.starts_with("about:") {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        let res = self.call("Page.navigate", Some(json!({ "url": url }))).await?;
        let error_text = res.get("errorText").and_then(Value::as_str).unwrap_or("").to_string();
        self.wait_load().await;
        let loc = self
            .eval("location.href")
            .await
            .ok()
            .flatten()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let failed = loc.contains("chrome-error") || (!error_text.is_empty() && error_text != "net::ERR_ABORTED");
        if failed {
            let reason = if error_text.is_empty() {
                "page injoignable".to_string()
            } else {
                error_text
            };
            return Err(format!(
                "chargement de {url} échoué ({reason}) — URL/hôte invalide, DNS, ou site bloqué ; corrige l'URL, essaie 'back', ou passe par un lien de la page"
            ));
        }
        Ok(())
    }

    /// `refInfo` : l'élément [ref] amené au centre (sa cible visible :
    /// l'étiquette d'une case cachée), sa position et son état.
    async fn ref_info(&self, r: i64) -> Result<RefInfo, String> {
        let expr = format!(
            "{}(function(){{var el=ajByRef({r});if(!el){{return null;}}var t=ajTarget(el);t.scrollIntoView({{block:'center',inline:'center'}});var r=t.getBoundingClientRect();return {{x:r.left+r.width/2,y:r.top+r.height/2,dis:!!(el.disabled||el.getAttribute('aria-disabled')==='true'),tog:ajToggle(el),on:ajOn(el)}};}})()",
            js::HELPERS_JS
        );
        let val = self.eval(&expr).await?;
        let p = match val {
            None | Some(Value::Null) => {
                return Err(format!(
                    "élément [{r}] introuvable — refais un browser_snapshot (la page a peut-être changé)"
                ));
            }
            Some(p) => p,
        };
        // Champs sans casse, comme `json.Unmarshal` dans une struct Go.
        let field = |k: &str| {
            p.as_object().and_then(|o| {
                o.get(k)
                    .or_else(|| o.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v))
            })
        };
        let f = |k: &str| field(k).and_then(Value::as_f64).unwrap_or(0.0);
        let b = |k: &str| field(k).and_then(Value::as_bool).unwrap_or(false);
        Ok(RefInfo {
            x: f("x"),
            y: f("y"),
            dis: b("dis"),
            tog: b("tog"),
            on: b("on"),
        })
    }

    async fn mouse_click(&self, x: f64, y: f64) -> Result<(), String> {
        for typ in ["mousePressed", "mouseReleased"] {
            self.call(
                "Input.dispatchMouseEvent",
                Some(json!({ "type": typ, "x": go_num(x), "y": go_num(y), "button": "left", "clickCount": 1 })),
            )
            .await?;
        }
        self.wait_load().await;
        Ok(())
    }

    /// `clickRef` : clique l'élément [ref] ; `Ok` porte une note quand une
    /// case n'a pas pris le clic (vide si tout va bien). Un élément désactivé
    /// est refusé ([`CLICK_DISABLED`]).
    pub async fn click_ref(&self, r: i64) -> Result<String, String> {
        let info = self.ref_info(r).await?;
        if info.dis {
            return Err(CLICK_DISABLED.into());
        }
        self.mouse_click(info.x, info.y).await?;
        if !info.tog || info.on {
            return Ok(String::new());
        }
        // Case qui devait se cocher : on vérifie, et sinon on la coche par le
        // DOM (un clic de souris peut tomber sur un voile ou une étiquette
        // décorative).
        match self.ref_info(r).await {
            Ok(a) if !a.on => {}
            _ => return Ok(String::new()),
        }
        let _ = self
            .eval(&format!(
                "{}(function(){{var el=ajByRef({r});if(el)el.click();}})()",
                js::HELPERS_JS
            ))
            .await;
        self.wait_load().await;
        Ok(match self.ref_info(r).await {
            Ok(a) if a.on => " (coché au second essai)".into(),
            _ => " — ATTENTION : la case ne s'est PAS cochée, le site ne l'a pas prise en compte".into(),
        })
    }

    /// `cuRepeat` pour cette session.
    fn repeat(&self, key: &str, out: String) -> String {
        lock(&self.repeat).note(key, out)
    }

    /// `clickXY`.
    pub async fn click_xy(&self, x: f64, y: f64) -> Result<(), String> {
        self.mouse_click(x, y).await
    }

    /// `typeText`.
    pub async fn type_text(&self, text: &str) -> Result<(), String> {
        self.call("Input.insertText", Some(json!({ "text": text })))
            .await
            .map(|_| ())
    }

    /// `typeInto` : un champ, ou l'option d'une liste déroulante native.
    pub async fn type_into(&self, r: i64, text: &str) -> Result<(), String> {
        let expr = format!(
            r#"{}(function(ref,val){{
  var el=ajByRef(ref);
  if(!el)return 'NOTFOUND';
  el.scrollIntoView({{block:'center'}});
  if(el.tagName.toLowerCase()==='select'){{
    var o=el.options,lv=val.toLowerCase(),k=-1;
    for(var i=0;i<o.length;i++){{var tx=(o[i].text||'').toLowerCase();if(tx===lv||tx.indexOf(lv)>=0||(o[i].value||'').toLowerCase()===lv){{k=i;break;}}}}
    if(k<0)return 'NOOPT';
    el.selectedIndex=k;el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));
    return 'SELECT';
  }}
  el.focus();
  try{{if(typeof el.select==='function')el.select();else if(window.getSelection){{var r=document.createRange();r.selectNodeContents(el);var s=getSelection();s.removeAllRanges();s.addRange(r);}}}}catch(e){{}}
  return 'TEXT';
}})({r},{})"#,
            js::HELPERS_JS,
            serde_json::to_string(text).unwrap_or_default()
        );
        let val = self.eval(&expr).await?;
        let v = val.map(|v| v.to_string()).unwrap_or_default();
        match v.trim_matches('"') {
            "NOTFOUND" => Err(format!("champ [{r}] introuvable — refais un browser_snapshot")),
            "NOOPT" => Err(format!(
                "aucune option de la liste [{r}] ne correspond à « {text} » — fais un browser_screenshot pour voir les choix"
            )),
            "SELECT" => Ok(()),
            _ => self.type_text(text).await,
        }
    }

    /// `pressKey`.
    pub async fn press_key(&self, name: &str) -> Result<(), String> {
        let (key, code, vk) = match name.trim().to_lowercase().as_str() {
            "enter" => ("Enter", "Enter", 13),
            "tab" => ("Tab", "Tab", 9),
            "backspace" => ("Backspace", "Backspace", 8),
            "escape" | "esc" => ("Escape", "Escape", 27),
            "delete" => ("Delete", "Delete", 46),
            "up" => ("ArrowUp", "ArrowUp", 38),
            "down" => ("ArrowDown", "ArrowDown", 40),
            "left" => ("ArrowLeft", "ArrowLeft", 37),
            "right" => ("ArrowRight", "ArrowRight", 39),
            "home" => ("Home", "Home", 36),
            "end" => ("End", "End", 35),
            "pageup" => ("PageUp", "PageUp", 33),
            "pagedown" => ("PageDown", "PageDown", 34),
            _ => {
                return Err(format!(
                    "touche inconnue « {name} » — connues : enter, tab, backspace, escape, delete, up/down/left/right, home, end, pageup, pagedown"
                ));
            }
        };
        for typ in ["rawKeyDown", "keyUp"] {
            self.call(
                "Input.dispatchKeyEvent",
                Some(json!({ "type": typ, "key": key, "code": code,
                    "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk })),
            )
            .await?;
        }
        self.wait_load().await;
        Ok(())
    }

    /// `scroll`.
    pub async fn scroll(&self, dir: &str) -> Result<(), String> {
        let (dx, dy) = match dir.trim().to_lowercase().as_str() {
            "down" | "" => (0, 600),
            "up" => (0, -600),
            "right" => (600, 0),
            "left" => (-600, 0),
            _ => {
                return Err(format!("direction inconnue « {dir} » (up, down, left, right)"));
            }
        };
        self.eval(&format!("window.scrollBy({dx},{dy});'ok'")).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }

    /// Les erreurs de page notées depuis le dernier appel (vidées).
    pub fn take_console(&self) -> Vec<String> {
        lock(&self.console).drain(..).collect()
    }

    /// Texte visible de la page (`innerText`), lignes vides retirées, `max`
    /// caractères au plus.
    pub async fn page_text(&self, max: usize) -> Option<String> {
        let v = self
            .eval("(document.body&&document.body.innerText||'').slice(0,20000)")
            .await
            .ok()
            .flatten()?;
        let lines: Vec<&str> = v.as_str()?.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let text = lines.join("\n");
        if text.chars().count() <= max {
            return Some(text);
        }
        let mut out: String = text.chars().take(max).collect();
        out.push_str("\n[… texte coupé]");
        Some(out)
    }

    /// `screenshot` : le viewport en PNG.
    pub async fn screenshot(&self) -> Result<Vec<u8>, String> {
        let res = self
            .call("Page.captureScreenshot", Some(json!({ "format": "png" })))
            .await?;
        let data = res.get("data").and_then(Value::as_str).unwrap_or("");
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| format!("illegal base64 data: {e}"))
    }
}

fn go_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// --- Le navigateur de la façade -------------------------------------------------

/// Le résultat d'un outil du navigateur.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Outcome {
    pub result: String,
    /// `browser_screenshot` : PNG quadrillé montré au modèle.
    pub image: Option<Vec<u8>>,
}

/// Message des outils pendant que Michael a la main.
pub const TAKEN_OVER: &str = "Michael utilise le navigateur en ce moment (connexion à un site, par exemple). N'insiste pas : attends qu'il te rende la main ou demande-lui où il en est, puis reprends avec browser_snapshot.";

/// Les onglets des sessions sur le navigateur de la stack.
#[derive(Default)]
pub struct Browser {
    /// `ENGINEX_BROWSER_URL` (`None` : pas de navigateur).
    url: Option<String>,
    tabs: tokio::sync::Mutex<HashMap<String, Arc<Session>>>,
    /// Sessions où Michael a pris la main.
    taken: Mutex<HashSet<String>>,
}

fn int_arg(args: &Map<String, Value>, k: &str) -> Option<i64> {
    args.get(k).and_then(Value::as_f64).map(|f| f as i64)
}

fn arg_str(args: &Map<String, Value>, k: &str) -> String {
    args.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Browser {
    pub fn new(url: Option<String>) -> Self {
        Self {
            url,
            ..Default::default()
        }
    }

    pub fn enabled(&self) -> bool {
        self.url.is_some()
    }

    /// `ENGINEX_BROWSER_URL` tel que configuré.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Version du navigateur (page Système, onglet Navigateur).
    pub async fn version(&self) -> Result<Value, String> {
        let url = self
            .url
            .as_deref()
            .ok_or("navigateur non configuré (ENGINEX_BROWSER_URL)")?;
        let ep = resolve(url).await?;
        http()
            .get(format!("{ep}/json/version"))
            .send()
            .await
            .map_err(|e| format!("navigateur injoignable : {e}"))?
            .json()
            .await
            .map_err(|e| format!("réponse illisible : {e}"))
    }

    /// L'onglet de la session, ouvert au besoin ; un ping vérifie qu'il répond.
    pub async fn get(&self, sid: &str) -> Result<Arc<Session>, String> {
        let url = self
            .url
            .as_deref()
            .ok_or("navigateur non configuré : ajoute le service browser de la stack et ENGINEX_BROWSER_URL")?;
        let mut tabs = self.tabs.lock().await;
        if let Some(s) = tabs.get(sid) {
            if !s.is_closed()
                && s.call(
                    "Runtime.evaluate",
                    Some(json!({ "expression": "1", "returnByValue": true })),
                )
                .await
                .is_ok()
            {
                return Ok(s.clone());
            }
            s.close().await;
            tabs.remove(sid);
        }
        let ep = resolve(url).await?;
        let s = Session::attach(&ep).await?;
        tabs.insert(sid.to_string(), s.clone());
        Ok(s)
    }

    /// L'onglet de la session s'il est ouvert (fenêtre Regarder).
    pub async fn current(&self, sid: &str) -> Option<Arc<Session>> {
        self.tabs.lock().await.get(sid).filter(|s| !s.is_closed()).cloned()
    }

    /// Ferme l'onglet de la session (sandbox arrêtée, session supprimée).
    pub async fn close(&self, sid: &str) {
        let s = self.tabs.lock().await.remove(sid);
        if let Some(s) = s {
            s.close().await;
        }
        lock(&self.taken).remove(sid);
    }

    pub fn taken(&self, sid: &str) -> bool {
        lock(&self.taken).contains(sid)
    }

    pub fn set_taken(&self, sid: &str, on: bool) {
        let mut t = lock(&self.taken);
        if on {
            t.insert(sid.to_string());
        } else {
            t.remove(sid);
        }
    }

    /// Un outil `browser_*` (`None` pour un autre). `vision` : le modèle voit-il
    /// les images.
    pub async fn exec(&self, sid: &str, name: &str, args: &Map<String, Value>, vision: bool) -> Option<Outcome> {
        if !is_tool(name) {
            return None;
        }
        if self.taken(sid) {
            return Some(Outcome {
                result: format!("[attente] {TAKEN_OVER}"),
                image: None,
            });
        }
        let result = match name {
            "browser_open" => self.tool_open(sid, args).await,
            "browser_snapshot" => self.tool_snapshot(sid).await,
            "browser_find" => self.tool_find(sid, args).await,
            "browser_click" => self.tool_click(sid, args).await,
            "browser_click_xy" => self.tool_click_xy(sid, args).await,
            "browser_type" => self.tool_type(sid, args).await,
            "browser_key" => self.tool_key(sid, args).await,
            "browser_scroll" => self.tool_scroll(sid, args).await,
            "browser_screenshot" => return Some(self.tool_screenshot(sid, vision).await),
            _ => return None,
        };
        Some(Outcome {
            result: cap_output(&result),
            image: None,
        })
    }

    async fn tool_open(&self, sid: &str, args: &Map<String, Value>) -> String {
        let url = arg_str(args, "url");
        if url.trim().is_empty() {
            return "[erreur] url manquante".into();
        }
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        if let Err(e) = s.navigate(&url).await {
            return format!("[erreur] navigation : {e}");
        }
        s.snapshot_dedup().await.unwrap_or_else(|e| format!("[erreur] {e}"))
    }

    async fn tool_snapshot(&self, sid: &str) -> String {
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        s.snapshot_dedup().await.unwrap_or_else(|e| format!("[erreur] {e}"))
    }

    async fn tool_find(&self, sid: &str, args: &Map<String, Value>) -> String {
        let text = arg_str(args, "text");
        if text.trim().is_empty() {
            return "[erreur] texte de recherche manquant".into();
        }
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        s.find(&text).await.unwrap_or_else(|e| format!("[erreur] {e}"))
    }

    async fn tool_click(&self, sid: &str, args: &Map<String, Value>) -> String {
        let Some(r) = int_arg(args, "ref") else {
            return "[erreur] ref manquant (numéro de l'élément)".into();
        };
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        let key = format!("click {r}");
        let note = match s.click_ref(r).await {
            Ok(n) => n,
            Err(e) if e == CLICK_DISABLED => return s.repeat(&key, format!("[refusé] [{r}] {e}")),
            Err(e) => return format!("[erreur] {e}"),
        };
        let snap = s.snapshot_dedup().await.unwrap_or_default();
        s.repeat(&key, format!("[ok] cliqué [{r}]{note}\n\n{snap}"))
    }

    async fn tool_type(&self, sid: &str, args: &Map<String, Value>) -> String {
        let text = arg_str(args, "text");
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        let r = match int_arg(args, "ref") {
            Some(r) => s.type_into(r, &text).await,
            None => s.type_text(&text).await,
        };
        if let Err(e) = r {
            return format!("[erreur] {e}");
        }
        if args.get("submit").and_then(Value::as_bool) == Some(true)
            && let Err(e) = s.press_key("Enter").await
        {
            return format!("[erreur] {e}");
        }
        let snap = s.snapshot_dedup().await.unwrap_or_default();
        format!("[ok] saisi « {text} »\n\n{snap}")
    }

    async fn tool_key(&self, sid: &str, args: &Map<String, Value>) -> String {
        let key = arg_str(args, "key");
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        if let Err(e) = s.press_key(&key).await {
            return format!("[erreur] {e}");
        }
        let snap = s.snapshot_dedup().await.unwrap_or_default();
        format!("[ok] touche {key}\n\n{snap}")
    }

    async fn tool_click_xy(&self, sid: &str, args: &Map<String, Value>) -> String {
        let (Some(x), Some(y)) = (
            args.get("x").and_then(Value::as_f64),
            args.get("y").and_then(Value::as_f64),
        ) else {
            return "[erreur] coordonnées x et y requises (en pixels du screenshot)".into();
        };
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        if let Err(e) = s.click_xy(x, y).await {
            return format!("[erreur] {e}");
        }
        let snap = s.snapshot_dedup().await.unwrap_or_default();
        s.repeat(
            &format!("xy {x:.0} {y:.0}"),
            format!("[ok] clic en ({x:.0}, {y:.0})\n\n{snap}"),
        )
    }

    async fn tool_scroll(&self, sid: &str, args: &Map<String, Value>) -> String {
        let dir = arg_str(args, "direction");
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return format!("[erreur] {e}"),
        };
        if let Err(e) = s.scroll(&dir).await {
            return format!("[erreur] {e}");
        }
        s.snapshot_dedup().await.unwrap_or_default()
    }

    /// Capture quadrillée pour le modèle (repères tous les 100 px).
    async fn tool_screenshot(&self, sid: &str, vision: bool) -> Outcome {
        let err = |e: String| Outcome {
            result: format!("[erreur] {e}"),
            image: None,
        };
        if !vision {
            return err(
                "le modèle actif ne voit pas les images (preset sans mmproj) : utilise browser_snapshot".into(),
            );
        }
        let s = match self.get(sid).await {
            Ok(s) => s,
            Err(e) => return err(e),
        };
        let png = match s.screenshot().await {
            Ok(p) => p,
            Err(e) => return err(e),
        };
        Outcome {
            result: format!(
                "[ok] capture de la page ({VIEW_W}x{VIEW_H} px, 1:1 avec la page). Une grille de repères (lignes et chiffres tous les 100 px) est surimprimée pour viser : si un élément visible n'est pas dans la liste numérotée (bandeau de cookies en iframe, canvas), clique-le avec browser_click_xy(x, y) lus sur la grille."
            ),
            image: Some(grid::overlay_grid(&png)),
        }
    }
}

// --- Fenêtre Regarder ------------------------------------------------------------

/// Démarre le screencast de l'onglet (JPEG, 1280 px de large au plus).
pub async fn start_screencast(s: &Session) -> Result<(), String> {
    s.call(
        "Page.startScreencast",
        Some(json!({ "format": "jpeg", "quality": 60, "maxWidth": VIEW_W, "maxHeight": VIEW_H, "everyNthFrame": 1 })),
    )
    .await
    .map(|_| ())
}

pub async fn stop_screencast(s: &Session) {
    let _ = s.call("Page.stopScreencast", None).await;
}

/// Entrée de Michael dans la fenêtre Regarder (coordonnées de la page).
pub async fn user_input(s: &Session, ev: &Value) -> Result<(), String> {
    let num = |k: &str| ev.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let modifiers = ev.get("modifiers").and_then(Value::as_i64).unwrap_or(0);
    match ev.get("type").and_then(Value::as_str).unwrap_or("") {
        "mouse" => {
            let kind = match ev.get("action").and_then(Value::as_str).unwrap_or("") {
                "down" => "mousePressed",
                "up" => "mouseReleased",
                "move" => "mouseMoved",
                other => return Err(format!("action souris inconnue : {other}")),
            };
            let button = if kind == "mouseMoved" { "none" } else { "left" };
            s.call(
                "Input.dispatchMouseEvent",
                Some(json!({ "type": kind, "x": num("x"), "y": num("y"), "button": button,
                    "clickCount": if kind == "mouseMoved" { 0 } else { 1 }, "modifiers": modifiers })),
            )
            .await
            .map(|_| ())
        }
        "wheel" => s
            .call(
                "Input.dispatchMouseEvent",
                Some(json!({ "type": "mouseWheel", "x": num("x"), "y": num("y"),
                    "deltaX": num("dx"), "deltaY": num("dy"), "modifiers": modifiers })),
            )
            .await
            .map(|_| ()),
        "text" => {
            let text = ev.get("text").and_then(Value::as_str).unwrap_or("");
            s.call("Input.insertText", Some(json!({ "text": text })))
                .await
                .map(|_| ())
        }
        "key" => {
            let key = ev.get("key").and_then(Value::as_str).unwrap_or("");
            s.press_key(key).await
        }
        "navigate" => {
            let url = ev.get("url").and_then(Value::as_str).unwrap_or("");
            s.navigate(url).await
        }
        "back" => s
            .call("Runtime.evaluate", Some(json!({ "expression": "history.back()" })))
            .await
            .map(|_| ()),
        other => Err(format!("entrée inconnue : {other}")),
    }
}

#[cfg(test)]
mod tests;
