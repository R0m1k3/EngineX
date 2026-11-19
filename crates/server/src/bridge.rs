//! Pont entre une session EngineX et le `codex app-server` de sa sandbox.
//!
//! Un `Hub` par session ouverte depuis le démarrage d'EngineX : il diffuse
//! les événements aux onglets (`broadcast`), garde l'état du tour en cours
//! pour le `snapshot` et la connexion à l'app-server quand une sandbox vit.
//! Pas de reconnexion : pont perdu = tour `failed`, sandbox supprimée ; le
//! message suivant relance une sandbox et reprend le fil (`thread/resume`).

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use enginex_codex::{Client, Incoming, Recorder, types};
use enginex_sandbox::{Handle, Progress};
use serde_json::{Map, Value, json};
use tokio::sync::{broadcast, mpsc::UnboundedReceiver};

use crate::{
    error::ApiError,
    mask::Stream,
    sandboxes,
    sessions::{self, SessionRow},
    snapshots::{self, SnapOpts},
    state::{AppState, now},
    v1, workspace,
};

#[derive(Clone, Debug)]
pub struct Event {
    pub name: &'static str,
    pub data: Value,
}

struct Conn {
    client: Client,
    thread_id: String,
    handle: Handle,
    preset: String,
}

struct TurnState {
    id: Option<String>,
    user: Value,
    user_stored: bool,
    /// Éléments démarrés ou terminés, dans l'ordre d'arrivée.
    items: Vec<Value>,
    partial: BTreeMap<String, Map<String, Value>>,
    streams: HashMap<(String, &'static str), Stream>,
    usage: Option<Value>,
    started_at: i64,
    /// `user` (message de Michael), `verify` (correction demandée par EngineX)
    /// ou `memory` (mise à jour de la mémoire en fin de session).
    origin: &'static str,
    /// Instantané d'avant le tour (avant que Codex ne connaisse l'id du tour).
    snap_pre: Option<String>,
    begun: Instant,
}

struct PendingReq {
    rid: String,
    id: Value,
    method: String,
    params: Value,
}

/// Vérification en cours (`verify.rs`).
pub struct VerifyState {
    pub turn_id: String,
    /// `processId` de l'étape en cours.
    pub process: Option<String>,
    pub cancelled: bool,
}

struct HubState {
    sandbox: (String, Option<String>),
    verify: Option<VerifyState>,
    /// Tours de correction lancés depuis le dernier message de Michael.
    fixes_done: u32,
    /// Terminaux ouverts : `processId` → sortie vers le WebSocket.
    terminals: HashMap<String, Arc<crate::terminal::Sink>>,
    conn: Option<Conn>,
    /// Incrémenté à chaque connexion ou arrêt : un lecteur périmé se tait.
    generation: u64,
    turn: Option<TurnState>,
    requests: Vec<PendingReq>,
    last_activity: Instant,
    /// Empreinte de chaque fichier mémoire lu ou écrit par la session (outils
    /// MCP), depuis l'ouverture de la connexion : `mem_write` l'exige à jour.
    last_read: HashMap<String, String>,
    /// Empreinte du bloc mémoire passé dans les instructions de la connexion
    /// (`thread/start` ou `thread/resume`) : Codex le reprend s'il compacte.
    conn_memory_sha: Option<String>,
    /// Heure d'inactivité d'avant le tour de mémoire, rendue à sa fin.
    memory_idle_base: Option<Instant>,
}

pub struct Hub {
    pub id: String,
    tx: broadcast::Sender<Event>,
    st: Mutex<HubState>,
    start: tokio::sync::Mutex<()>,
    /// Un seul envoi de `thread/revert` à la fois.
    revert: tokio::sync::Mutex<()>,
    recorder: Mutex<Option<Arc<Recorder>>>,
}

/// Le hub d'une session (créé au premier accès).
pub fn hub(state: &AppState, session_id: &str) -> Arc<Hub> {
    let mut hubs = state.0.hubs.lock().expect("verrou");
    hubs.entry(session_id.to_string())
        .or_insert_with(|| {
            Arc::new(Hub {
                id: session_id.to_string(),
                tx: broadcast::channel(1024).0,
                st: Mutex::new(HubState {
                    sandbox: ("stopped".into(), None),
                    verify: None,
                    fixes_done: 0,
                    terminals: HashMap::new(),
                    conn: None,
                    generation: 0,
                    turn: None,
                    requests: Vec::new(),
                    last_activity: Instant::now(),
                    last_read: HashMap::new(),
                    conn_memory_sha: None,
                    memory_idle_base: None,
                }),
                start: tokio::sync::Mutex::new(()),
                revert: tokio::sync::Mutex::new(()),
                recorder: Mutex::new(None),
            })
        })
        .clone()
}

pub fn hubs(state: &AppState) -> Vec<Arc<Hub>> {
    state.0.hubs.lock().expect("verrou").values().cloned().collect()
}

pub fn forget(state: &AppState, session_id: &str) {
    state.0.hubs.lock().expect("verrou").remove(session_id);
}

fn rid_of(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn stream_field(method: &str) -> Option<&'static str> {
    match method {
        "item/agentMessage/delta" => Some("text"),
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => Some("reasoning"),
        "item/commandExecution/outputDelta" => Some("output"),
        _ => None,
    }
}

impl Hub {
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    fn emit(&self, name: &'static str, data: Value) {
        let _ = self.tx.send(Event { name, data });
    }

    fn set_sandbox(&self, s: &str, detail: Option<String>) {
        self.st.lock().expect("verrou").sandbox = (s.to_string(), detail.clone());
        let mut d = json!({ "state": s });
        if let Some(x) = detail {
            d["detail"] = json!(x);
        }
        self.emit("sandbox", d);
    }

    /// Empreinte du fichier mémoire `rel` lu ou écrit en dernier par la session.
    pub fn last_read(&self, rel: &str) -> Option<String> {
        self.st.lock().expect("verrou").last_read.get(rel).cloned()
    }

    pub fn set_last_read(&self, rel: &str, sha: &str) {
        self.st
            .lock()
            .expect("verrou")
            .last_read
            .insert(rel.to_string(), sha.to_string());
    }

    /// Origine du tour en cours.
    pub fn turn_origin(&self) -> Option<&'static str> {
        self.st.lock().expect("verrou").turn.as_ref().map(|t| t.origin)
    }

    /// Durée du tour de mémoire en cours, s'il y en a un.
    pub fn memory_turn_age(&self) -> Option<Duration> {
        let st = self.st.lock().expect("verrou");
        st.turn
            .as_ref()
            .filter(|t| t.origin == "memory")
            .map(|t| t.begun.elapsed())
    }

    pub fn running(&self) -> bool {
        self.st.lock().expect("verrou").turn.is_some()
    }

    /// Tour ou vérification en cours.
    pub fn busy(&self) -> bool {
        let st = self.st.lock().expect("verrou");
        st.turn.is_some() || st.verify.is_some()
    }

    pub fn verifying(&self) -> bool {
        self.st.lock().expect("verrou").verify.is_some()
    }

    /// Ouvre une vérification (refus si tour ou vérification en cours).
    pub(crate) fn verify_begin(&self, turn_id: &str) -> bool {
        let mut st = self.st.lock().expect("verrou");
        if st.turn.is_some() || st.verify.is_some() {
            return false;
        }
        st.verify = Some(VerifyState {
            turn_id: turn_id.to_string(),
            process: None,
            cancelled: false,
        });
        st.last_activity = Instant::now();
        true
    }

    /// Étape en cours ; `false` si la vérification a été annulée.
    pub(crate) fn verify_step(&self, process: Option<String>) -> bool {
        let mut st = self.st.lock().expect("verrou");
        st.last_activity = Instant::now();
        match st.verify.as_mut() {
            Some(v) if !v.cancelled => {
                v.process = process;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn verify_cancelled(&self) -> bool {
        self.st
            .lock()
            .expect("verrou")
            .verify
            .as_ref()
            .is_none_or(|v| v.cancelled)
    }

    pub(crate) fn verify_end(&self) {
        let mut st = self.st.lock().expect("verrou");
        st.verify = None;
        st.last_activity = Instant::now();
    }

    /// Annule la vérification en cours (étape terminée par
    /// `command/exec/terminate`).
    pub async fn cancel_verify(&self) {
        let target = {
            let mut st = self.st.lock().expect("verrou");
            let client = st.conn.as_ref().map(|c| c.client.clone());
            match st.verify.as_mut() {
                Some(v) => {
                    v.cancelled = true;
                    v.process.clone().zip(client)
                }
                None => None,
            }
        };
        if let Some((pid, client)) = target {
            let _ = client
                .request("command/exec/terminate", types::exec_terminate(&pid))
                .await;
        }
    }

    pub(crate) fn fixes_done(&self) -> u32 {
        self.st.lock().expect("verrou").fixes_done
    }

    pub(crate) fn add_fix(&self) {
        self.st.lock().expect("verrou").fixes_done += 1;
    }

    /// Client et dossier de travail de la sandbox vivante.
    pub(crate) fn client_cwd(&self) -> Option<(Client, String)> {
        let st = self.st.lock().expect("verrou");
        st.conn.as_ref().map(|c| (c.client.clone(), c.handle.cwd.clone()))
    }

    /// Connexion prête (sandbox démarrée au besoin) : client et dossier.
    pub(crate) async fn ready(self: &Arc<Self>, state: &AppState) -> Result<(Client, String), String> {
        self.ensure_conn(state).await?;
        self.client_cwd().ok_or_else(|| "sandbox arrêtée".to_string())
    }

    pub(crate) fn emit_event(&self, name: &'static str, data: Value) {
        self.emit(name, data);
    }

    /// Inscrit un terminal ; rend les `processId` des anciens (un seul
    /// terminal par session : l'ancien est terminé).
    pub(crate) fn add_terminal(&self, state: &AppState, pid: &str, sink: Arc<crate::terminal::Sink>) -> Vec<String> {
        let m = state.masker();
        let mut st = self.st.lock().expect("verrou");
        let old: Vec<String> = st
            .terminals
            .drain()
            .map(|(k, s)| {
                s.finish(&m, None, Some("autre terminal ouvert".into()));
                k
            })
            .collect();
        st.terminals.insert(pid.to_string(), sink);
        st.last_activity = Instant::now();
        old
    }

    pub(crate) fn touch_activity(&self) {
        self.touch();
    }

    /// Ferme les terminaux (sandbox arrêtée).
    fn close_terminals(&self, state: &AppState, reason: &str) {
        let m = state.masker();
        let sinks: Vec<_> = self
            .st
            .lock()
            .expect("verrou")
            .terminals
            .drain()
            .map(|(_, s)| s)
            .collect();
        for s in sinks {
            s.finish(&m, None, Some(reason.to_string()));
        }
    }

    /// Sortie d'un `command/exec` en flux : vers son terminal, jamais dans
    /// l'historique ni le SSE.
    fn on_exec_output(&self, state: &AppState, params: &Value) {
        let pid = params["processId"].as_str().unwrap_or("");
        let sink = {
            let mut st = self.st.lock().expect("verrou");
            st.last_activity = Instant::now();
            st.terminals.get(pid).cloned()
        };
        let Some(sink) = sink else { return };
        let raw = params["deltaBase64"]
            .as_str()
            .and_then(|b| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b).ok())
            .unwrap_or_default();
        sink.push(&state.masker(), &raw);
    }

    pub(crate) fn remove_terminal(&self, pid: &str) {
        let mut st = self.st.lock().expect("verrou");
        st.terminals.remove(pid);
        st.last_activity = Instant::now();
    }

    pub fn has_conn(&self) -> bool {
        self.st.lock().expect("verrou").conn.is_some()
    }

    /// Durée d'inactivité d'une sandbox vivante sans tour ni requête.
    pub fn idle_for(&self) -> Option<Duration> {
        let st = self.st.lock().expect("verrou");
        (st.conn.is_some() && st.turn.is_none() && st.verify.is_none() && st.requests.is_empty())
            .then(|| st.last_activity.elapsed())
    }

    fn touch(&self) {
        self.st.lock().expect("verrou").last_activity = Instant::now();
    }

    /// Fin d'un tour de mémoire : la session redevient inactive depuis l'heure
    /// d'avant ce tour (arrêt au prochain passage d'`idle_loop`).
    fn end_memory_turn(&self) {
        let mut st = self.st.lock().expect("verrou");
        st.last_activity = st.memory_idle_base.take().unwrap_or_else(Instant::now);
    }

    /// État pour un nouvel abonné.
    pub fn snapshot(&self, session: Value) -> Value {
        let st = self.st.lock().expect("verrou");
        let turn = st.turn.as_ref().map(|t| {
            let mut items = vec![t.user.clone()];
            items.extend(t.items.iter().cloned());
            json!({"id": t.id, "status": "inProgress", "items": items, "partial": t.partial})
        });
        let requests: Vec<Value> = st
            .requests
            .iter()
            .map(|r| json!({"rid": r.rid, "method": r.method, "params": r.params}))
            .collect();
        json!({
            "session": session,
            "sandbox": {"state": st.sandbox.0, "detail": st.sandbox.1},
            "turn": turn,
            "requests": requests,
        })
    }

    /// Ouvre un tour (refus si un tour tourne déjà).
    pub fn begin(&self, user: Value, origin: &'static str) -> Result<(), ApiError> {
        let mut st = self.st.lock().expect("verrou");
        if st.turn.is_some() {
            return Err(ApiError::TurnInProgress);
        }
        if origin == "user" {
            st.fixes_done = 0;
        }
        st.turn = Some(TurnState {
            id: None,
            user: user.clone(),
            user_stored: false,
            items: Vec::new(),
            partial: BTreeMap::new(),
            streams: HashMap::new(),
            usage: None,
            started_at: now(),
            origin,
            snap_pre: None,
            begun: Instant::now(),
        });
        st.last_activity = Instant::now();
        drop(st);
        self.emit("user_message", json!({ "item": user }));
        Ok(())
    }

    /// Arrête la sandbox (inactivité, suppression, archivage, arrêt).
    pub async fn stop_conn(self: &Arc<Self>, state: &AppState, reason: &str) {
        let conn = {
            let mut st = self.st.lock().expect("verrou");
            st.generation += 1;
            st.conn.take()
        };
        self.finish_stop(state, conn, reason).await;
    }

    /// Arrêt pour inactivité sans course avec un nouveau tour : sous le verrou
    /// `start` (celui d'`ensure_conn`), la connexion n'est retirée que si la
    /// session est toujours inactive depuis `idle`. Un `begin` arrivé ensuite
    /// redémarre une sandbox après cet arrêt.
    pub async fn stop_if_idle(self: &Arc<Self>, state: &AppState, idle: Duration, reason: &str) -> bool {
        let _g = self.start.lock().await;
        let conn = {
            let mut st = self.st.lock().expect("verrou");
            let still = st.conn.is_some()
                && st.turn.is_none()
                && st.verify.is_none()
                && st.requests.is_empty()
                && st.last_activity.elapsed() >= idle;
            if !still {
                return false;
            }
            st.generation += 1;
            st.conn.take()
        };
        self.finish_stop(state, conn, reason).await;
        true
    }

    /// Tour de mémoire de fin de session, lancé à la place de l'arrêt pour
    /// inactivité ; l'appelant n'attend pas sa fin.
    pub fn start_memory_turn(self: &Arc<Self>, state: &AppState) -> bool {
        let base = self.st.lock().expect("verrou").last_activity;
        let user = json!({
            "type": "userMessage",
            "id": format!("ex-{}", uuid::Uuid::new_v4()),
            "text": crate::memory::END_PROMPT,
            "images": [],
            "origin": "memory",
        });
        if self.begin(user, "memory").is_err() {
            return false;
        }
        self.st.lock().expect("verrou").memory_idle_base = Some(base);
        let (me, st) = (self.clone(), state.clone());
        tokio::spawn(async move { me.run_turn(st, crate::memory::END_PROMPT.to_string(), Vec::new()).await });
        true
    }

    /// Interrompt le tour de mémoire en cours pour laisser passer Michael (ou
    /// une autre session) et attend sa fin (5 s au plus). `true` : plus de tour.
    pub async fn preempt_memory_turn(&self) -> bool {
        if self.turn_origin() != Some("memory") {
            return !self.running();
        }
        // Michael prend la main : l'inactivité repart de maintenant.
        self.st.lock().expect("verrou").memory_idle_base = None;
        if let Err(e) = self.interrupt().await {
            tracing::warn!("session {} : interruption du tour de mémoire : {e}", self.id);
        }
        for _ in 0..50 {
            if !self.running() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        !self.running()
    }

    /// Suite commune des arrêts : sandbox, terminaux, tour resté ouvert.
    async fn finish_stop(self: &Arc<Self>, state: &AppState, conn: Option<Conn>, reason: &str) {
        let had = conn.is_some();
        if let Some(c) = conn {
            c.client.close();
        }
        sandboxes::stop(state, &self.id).await;
        self.close_terminals(state, reason);
        if had {
            self.set_sandbox("stopped", Some(reason.to_string()));
        }
        // Un tour encore ouvert ne peut plus finir.
        if self.running() {
            self.fail_turn(state, &format!("sandbox arrêtée ({reason})")).await;
        }
    }

    // ---- Tour ----

    /// Lance le tour ouvert par `begin` : connexion (sandbox démarrée si
    /// besoin), message stocké, `turn/start`.
    pub async fn run_turn(self: Arc<Self>, state: AppState, text: String, images: Vec<String>) {
        // Instantané d'avant le tour, en parallèle du démarrage de la sandbox
        // (Codex ne lance rien avant `turn/start`).
        let folder = match sessions::row(&state, &self.id).await {
            Ok(r) => snapshots::folder(&state, &r).await.ok(),
            Err(_) => None,
        };
        let pre = async {
            match &folder {
                Some(f) => snapshots::snapshot(&state, f, "avant le message", SnapOpts::default())
                    .await
                    .map(|s| s.sha)
                    .map_err(|e| format!("{e:#}")),
                None => Err("dossier de travail introuvable".to_string()),
            }
        };
        let (conn, pre) = tokio::join!(self.ensure_conn(&state), pre);
        match pre {
            Ok(sha) => {
                if let Some(t) = self.st.lock().expect("verrou").turn.as_mut() {
                    t.snap_pre = Some(sha);
                }
            }
            Err(e) => tracing::warn!("session {} : instantané d'avant le tour impossible : {e}", self.id),
        }
        let conn = match conn {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("session {} : sandbox indisponible : {e}", self.id);
                self.set_sandbox("error", Some(e.clone()));
                self.fail_turn(&state, &e).await;
                return;
            }
        };
        let (client, thread_id, attachments) = conn;
        if let Err(e) = self.apply_pending_revert(&state, &client, &thread_id).await {
            self.fail_turn(&state, &e).await;
            return;
        }
        let row = sessions::row(&state, &self.id).await.ok();
        let note = row.as_ref().and_then(|r| r.pending_note.clone());
        // Mémoire modifiée depuis le dernier envoi : bloc joint en tête du tour.
        let mut refresh = None;
        if let Some(r) = &row
            && let Ok(t) = workspace::topic(&state, r.topic_id).await
        {
            let (bloc, sha) = crate::memory::block(&state, &t.project_slug, &t.slug);
            if r.memory_seen.as_deref() != Some(sha.as_str()) {
                refresh = Some((bloc, sha));
            }
        }
        if let Some(n) = &note
            && let Some(t) = self.st.lock().expect("verrou").turn.as_mut()
        {
            t.user["note"] = json!(n);
        }
        let first = sessions::item_count(&state, &self.id).await.unwrap_or(1) == 0;
        if let Err(e) = self.store_user(&state).await {
            self.fail_turn(&state, &format!("base de données : {e}")).await;
            return;
        }
        if first {
            let title: String = state.masker().mask(text.trim()).chars().take(60).collect();
            if !title.is_empty() {
                let _ = sessions::set_title(&state, &self.id, &title).await;
                if let Err(e) = client
                    .request("thread/name/set", json!({"threadId": thread_id, "name": title}))
                    .await
                {
                    tracing::debug!("thread/name/set : {e}");
                }
            }
        }
        let approval = sessions::approval_of(&state, &self.id)
            .await
            .unwrap_or_else(|_| "auto".into());
        let mut input = Vec::new();
        if let Some((bloc, _)) = &refresh {
            input.push(types::text_input(&crate::memory::refresh_text(bloc)));
        }
        if let Some(n) = &note {
            input.push(types::text_input(n));
        }
        if !text.is_empty() {
            input.push(types::text_input(&text));
        }
        for name in &images {
            input.push(types::local_image(&format!("{attachments}/{name}")));
        }
        // Tour de mémoire : toujours prudent (commandes et fichiers refusés par
        // le pont), même en mode auto.
        let policy = if self.turn_origin() == Some("memory") {
            "untrusted"
        } else {
            types::approval_policy(&approval)
        };
        // Noté avant l'envoi : la fin du tour (lue par la boucle de
        // notifications) peut arriver avant la réponse à `turn/start`.
        if let Some((_, sha)) = &refresh {
            let _ = sessions::set_session_field(&state, &self.id, "memory_seen", Some(sha)).await;
        }
        match client
            .request("turn/start", types::turn_start(&thread_id, input, policy))
            .await
        {
            Ok(r) => {
                if let Some(tid) = types::turn_id(&r) {
                    self.adopt_turn(&state, tid).await;
                }
                if note.is_some() {
                    let _ = sessions::set_session_field(&state, &self.id, "pending_note", None).await;
                }
                let _ = sessions::set_session_field(&state, &self.id, "undo_snap", None).await;
            }
            Err(e) => {
                if refresh.is_some() {
                    let prev = row.as_ref().and_then(|r| r.memory_seen.as_deref());
                    let _ = sessions::set_session_field(&state, &self.id, "memory_seen", prev).await;
                }
                self.fail_turn(&state, &format!("turn/start : {e}")).await
            }
        }
    }

    async fn store_user(&self, state: &AppState) -> Result<(), sqlx::Error> {
        let (user, tid) = {
            let st = self.st.lock().expect("verrou");
            match &st.turn {
                Some(t) if !t.user_stored => (t.user.clone(), t.id.clone()),
                _ => return Ok(()),
            }
        };
        let item_id = user["id"].as_str().unwrap_or("").to_string();
        sessions::store_item(state, &self.id, tid.as_deref(), &item_id, "userMessage", &user).await?;
        if let Some(t) = self.st.lock().expect("verrou").turn.as_mut() {
            t.user_stored = true;
        }
        Ok(())
    }

    /// Id du tour connu (réponse à `turn/start` ou `turn/started`).
    async fn adopt_turn(&self, state: &AppState, tid: &str) {
        let (user_id, started, origin, pre) = {
            let mut st = self.st.lock().expect("verrou");
            let Some(t) = st.turn.as_mut() else { return };
            if t.id.is_some() {
                return;
            }
            t.id = Some(tid.to_string());
            (
                t.user_stored.then(|| t.user["id"].as_str().unwrap_or("").to_string()),
                t.started_at,
                t.origin,
                t.snap_pre.clone(),
            )
        };
        if let Err(e) = sessions::insert_turn(state, &self.id, tid, started, origin, pre.as_deref()).await {
            tracing::warn!("tour {tid} : {e}");
        }
        if let Some(u) = user_id {
            let _ = sessions::set_item_turn(state, &self.id, &u, tid).await;
        }
    }

    /// Termine le tour ouvert en échec (avant ou pendant `turn/start`, ou pont
    /// perdu) ; le message de Michael reste dans l'historique.
    async fn fail_turn(self: &Arc<Self>, state: &AppState, error: &str) {
        let _ = self.store_user(state).await;
        let (turn, reqs) = {
            let mut st = self.st.lock().expect("verrou");
            (st.turn.take(), std::mem::take(&mut st.requests))
        };
        for r in reqs {
            self.emit("request_resolved", json!({ "rid": r.rid }));
        }
        let Some(t) = turn else { return };
        let memory_turn = t.origin == "memory";
        let tid = t.id.clone().unwrap_or_else(|| format!("ex-{}", uuid::Uuid::new_v4()));
        let error = state.masker().mask(error).into_owned();
        let _ = sessions::finish_turn(
            state,
            &self.id,
            &tid,
            t.started_at,
            t.origin,
            "failed",
            Some(&error),
            t.usage.as_ref(),
        )
        .await;
        let _ = sessions::set_item_turn(state, &self.id, t.user["id"].as_str().unwrap_or(""), &tid).await;
        self.emit(
            "turn_stored",
            json!({"turn": {"id": tid, "status": "failed", "error": error, "usage": t.usage, "origin": t.origin}}),
        );
        if memory_turn {
            self.end_memory_turn();
        } else {
            self.touch();
        }
        if let Some(pre) = t.snap_pre {
            let (me, st) = (self.clone(), state.clone());
            tokio::spawn(async move {
                me.post_snapshot(&st, &tid, &pre).await;
            });
        }
    }

    /// Instantané d'après le tour : refs `-pre` et `-post`, modifications
    /// stockées et émises (`changes`).
    pub(crate) async fn post_snapshot(&self, state: &AppState, tid: &str, pre: &str) -> Option<Value> {
        let folder = snapshots::folder(state, &sessions::row(state, &self.id).await.ok()?)
            .await
            .ok()?;
        let opts = SnapOpts {
            ref_name: Some(snapshots::post_ref(&self.id, tid)),
            base: Some(pre.to_string()),
            extra_ref: Some((snapshots::pre_ref(&self.id, tid), pre.to_string())),
        };
        match snapshots::snapshot(state, &folder, "après le message", opts).await {
            Ok(s) => {
                let changes = s.changes.unwrap_or_else(|| json!({"files": [], "truncated": false}));
                if let Err(e) =
                    sessions::set_turn_snaps(state, &self.id, tid, Some(pre), Some(&s.sha), Some(&changes)).await
                {
                    tracing::warn!("tour {tid} : instantané non stocké : {e}");
                }
                self.emit("changes", json!({"turn_id": tid, "changes": changes}));
                snapshots::gc_later(state, &folder);
                Some(changes)
            }
            Err(e) => {
                tracing::warn!("session {} : instantané d'après le tour impossible : {e:#}", self.id);
                None
            }
        }
    }

    /// Envoie `thread/revert` si un retour de conversation attend. `Err` :
    /// connexion perdue ou délai (le retour reste en attente).
    async fn apply_pending_revert(&self, state: &AppState, client: &Client, thread_id: &str) -> Result<(), String> {
        let _g = self.revert.lock().await;
        let Some(tid) = sessions::row(state, &self.id).await.ok().and_then(|r| r.pending_revert) else {
            return Ok(());
        };
        match client
            .request("thread/revert", types::thread_revert(thread_id, &tid))
            .await
        {
            Ok(_) => {
                let _ = sessions::set_session_field(state, &self.id, "pending_revert", None).await;
                // Le tour retiré pouvait porter le dernier envoi du bloc mémoire.
                let _ = sessions::set_session_field(state, &self.id, "memory_seen", None).await;
                sessions::emit_session(state, &self.id).await;
                Ok(())
            }
            Err(enginex_codex::Error::Rpc { message, .. }) => {
                let _ = sessions::set_session_field(state, &self.id, "pending_revert", None).await;
                sessions::emit_session(state, &self.id).await;
                let msg = state.masker().mask(&message).into_owned();
                self.notice(
                    state,
                    &format!("Codex n'a pas pu retirer ces messages de son fil : {msg}"),
                )
                .await;
                Ok(())
            }
            Err(e) => Err(format!("retour de conversation non appliqué par Codex : {e}")),
        }
    }

    /// Retour de conversation demandé sandbox vivante : envoyé tout de suite.
    pub async fn revert_now(self: &Arc<Self>, state: &AppState) {
        let conn = {
            let st = self.st.lock().expect("verrou");
            st.conn.as_ref().map(|c| (c.client.clone(), c.thread_id.clone()))
        };
        if let Some((client, thread)) = conn
            && let Err(e) = self.apply_pending_revert(state, &client, &thread).await
        {
            tracing::warn!("session {} : {e} (réessayé au prochain message)", self.id);
        }
    }

    /// Ligne d'EngineX dans le fil (stockée, hors tour).
    async fn notice(&self, state: &AppState, text: &str) {
        let item = json!({"type": "notice", "id": format!("ex-{}", uuid::Uuid::new_v4()), "text": text});
        let iid = item["id"].as_str().unwrap_or("").to_string();
        let _ = sessions::store_item(state, &self.id, None, &iid, "notice", &item).await;
        self.emit("notice", json!({ "item": item }));
    }

    pub async fn interrupt(&self) -> Result<(), ApiError> {
        for _ in 0..50 {
            let target = {
                let st = self.st.lock().expect("verrou");
                match (&st.conn, &st.turn) {
                    (_, None) => return Ok(()),
                    (Some(c), Some(t)) => t.id.clone().map(|tid| (c.client.clone(), c.thread_id.clone(), tid)),
                    (None, Some(_)) => None,
                }
            };
            if let Some((client, thread, tid)) = target {
                client
                    .request("turn/interrupt", json!({"threadId": thread, "turnId": tid}))
                    .await
                    .map_err(|e| ApiError::Internal(anyhow::anyhow!("turn/interrupt : {e}")))?;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(ApiError::Busy)
    }

    /// Réponse de l'UI à une requête de Codex.
    pub fn answer(&self, rid: &str, result: Value) -> Result<(), ApiError> {
        let (client, id) = {
            let mut st = self.st.lock().expect("verrou");
            let pos = st
                .requests
                .iter()
                .position(|r| r.rid == rid)
                .ok_or(ApiError::NotFound)?;
            let client = st.conn.as_ref().map(|c| c.client.clone()).ok_or(ApiError::NotFound)?;
            let r = st.requests.remove(pos);
            st.last_activity = Instant::now();
            (client, r.id)
        };
        client
            .respond(&id, result)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("réponse à Codex : {e}")))?;
        self.emit("request_resolved", json!({ "rid": rid }));
        Ok(())
    }

    // ---- Connexion ----

    /// Connexion prête (sandbox démarrée, fil ouvert) : `(client, fil,
    /// dossier des pièces jointes)`. Erreur : texte pour l'UI.
    async fn ensure_conn(self: &Arc<Self>, state: &AppState) -> Result<(Client, String, String), String> {
        let _g = self.start.lock().await;
        let active = match v1::active(state).await {
            Ok(Some(a)) => a,
            Ok(None) => return Err("aucun preset actif".into()),
            Err(e) => return Err(e.message),
        };
        let label = sandboxes::preset_label(&active);
        let restart = {
            let st = self.st.lock().expect("verrou");
            match &st.conn {
                Some(c) if c.preset == label && !c.client.is_closed() => {
                    return Ok((c.client.clone(), c.thread_id.clone(), c.handle.attachments.clone()));
                }
                Some(_) => true,
                None => false,
            }
        };
        if restart {
            self.set_sandbox("restarting", Some("redémarrage de la sandbox (nouveau modèle)".into()));
            let conn = {
                let mut st = self.st.lock().expect("verrou");
                st.generation += 1;
                st.conn.take()
            };
            if let Some(c) = conn {
                c.client.close();
            }
            sandboxes::stop(state, &self.id).await;
            self.close_terminals(state, "redémarrage de la sandbox");
        }
        let session = sessions::row(state, &self.id).await.map_err(|e| format!("{e:?}"))?;
        let topic = workspace::topic(state, session.topic_id)
            .await
            .map_err(|e| format!("topic : {e:?}"))?;
        if !restart {
            self.set_sandbox("creating", None);
        }
        let me = self.clone();
        let progress = move |p: Progress| match p {
            Progress::Pulling { done, total } => {
                me.st.lock().expect("verrou").sandbox = ("pulling".into(), None);
                me.emit(
                    "sandbox",
                    json!({"state": "pulling", "progress": {"done": done, "total": total}}),
                );
            }
            Progress::Starting => me.set_sandbox("starting", None),
        };
        let (handle, token) = sandboxes::start(state, &self.id, &topic, &active, &progress)
            .await
            .map_err(|e| format!("démarrage de la sandbox : {e:#}"))?;
        self.set_sandbox("starting", None);
        let t = &state.config().timings.sandbox;
        if !wait_ready(state, &handle.readyz_url, t.ready_wait).await {
            let logs = sandboxes::logs(state, &self.id).await;
            sandboxes::stop(state, &self.id).await;
            let logs = state.masker().mask(&logs).into_owned();
            return Err(format!("sandbox_timeout : app-server pas prêt à temps\n{logs}"));
        }
        let recorder = state.config().sandbox.record_dir.as_ref().map(|_| {
            let mut r = self.recorder.lock().expect("verrou");
            match &*r {
                Some(rec) => {
                    rec.launch();
                    rec.clone()
                }
                None => {
                    let rec = Arc::new(Recorder::new(types::WHITELIST));
                    *r = Some(rec.clone());
                    rec
                }
            }
        });
        let fail = |e: String| async move {
            sandboxes::stop(state, &self.id).await;
            e
        };
        let (client, rx) = match enginex_codex::connect(&handle.ws_url, &token, t.rpc_timeout, recorder).await {
            Ok(c) => c,
            Err(e) => return Err(fail(e.to_string()).await),
        };
        if let Err(e) = client.initialize(env!("CARGO_PKG_VERSION")).await {
            return Err(fail(e.to_string()).await);
        }
        let policy = types::approval_policy(&session.approval);
        let (instructions, memory_sha) = crate::memory::instructions(state, &topic.project_slug, &topic.slug);
        let mut lost_context = false;
        let thread_id = match &session.thread_id {
            Some(tid) => match client
                .request(
                    "thread/resume",
                    types::thread_resume(tid, &handle.cwd, policy, Some(&instructions)),
                )
                .await
            {
                Ok(_) => {
                    if let Err(e) = self.apply_pending_revert(state, &client, tid).await {
                        return Err(fail(e).await);
                    }
                    self.catch_up(state, &client, tid).await;
                    tid.clone()
                }
                Err(e) => {
                    tracing::warn!("session {} : thread/resume refusé ({e}), nouveau fil", self.id);
                    lost_context = true;
                    String::new()
                }
            },
            None => String::new(),
        };
        let thread_id = if thread_id.is_empty() {
            match client
                .request(
                    "thread/start",
                    types::thread_start(&handle.cwd, policy, Some(&instructions)),
                )
                .await
            {
                Ok(r) => match types::thread_id(&r) {
                    Some(t) => {
                        let t = t.to_string();
                        let _ = sessions::set_thread(state, &self.id, &t).await;
                        // Le bloc est dans le message developer du nouveau fil.
                        let _ = sessions::set_session_field(state, &self.id, "memory_seen", Some(&memory_sha)).await;
                        // Nouveau fil : plus rien à retirer chez Codex.
                        let _ = sessions::set_session_field(state, &self.id, "pending_revert", None).await;
                        sessions::emit_session(state, &self.id).await;
                        t
                    }
                    None => return Err(fail("thread/start sans id".into()).await),
                },
                Err(e) => return Err(fail(format!("thread/start : {e}")).await),
            }
        } else {
            thread_id
        };
        let generation = {
            let mut st = self.st.lock().expect("verrou");
            st.generation += 1;
            st.conn = Some(Conn {
                client: client.clone(),
                thread_id: thread_id.clone(),
                handle: handle.clone(),
                preset: label,
            });
            st.last_activity = Instant::now();
            st.last_read.clear();
            st.conn_memory_sha = Some(memory_sha);
            st.generation
        };
        let detail =
            lost_context.then(|| "Codex a perdu le contexte de ce fil : nouveau fil (l'historique reste)".into());
        self.set_sandbox("ready", detail);
        let me = self.clone();
        let st2 = state.clone();
        tokio::spawn(async move { me.read_loop(st2, rx, generation).await });
        Ok((client, thread_id, handle.attachments))
    }

    /// Après `thread/resume` : stocke les éléments terminés que le pont
    /// n'avait pas stockés (dédoublonnés par `item_id`).
    async fn catch_up(&self, state: &AppState, client: &Client, thread_id: &str) {
        let list = match client.request("thread/turns/list", types::turns_list(thread_id)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("thread/turns/list : {e}");
                return;
            }
        };
        let mut list = list;
        state.masker().mask_json(&mut list);
        let mut added = 0;
        let reverted = sessions::reverted(state, &self.id).await.unwrap_or_default();
        for turn in list["data"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let Some(tid) = turn["id"].as_str() else { continue };
            if reverted.contains(tid) {
                continue;
            }
            let status = turn["status"].as_str().unwrap_or("completed");
            if status != "inProgress" {
                let _ = sessions::insert_turn_status(state, &self.id, tid, status).await;
            }
            for item in turn["items"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let kind = item["type"].as_str().unwrap_or("");
                let Some(iid) = item["id"].as_str() else { continue };
                if kind == "userMessage" {
                    continue;
                }
                if sessions::store_item(state, &self.id, Some(tid), iid, kind, item)
                    .await
                    .unwrap_or(false)
                {
                    added += 1;
                }
            }
        }
        if added > 0 {
            tracing::info!("session {} : {added} élément(s) rattrapé(s)", self.id);
        }
    }

    async fn read_loop(self: Arc<Self>, state: AppState, mut rx: UnboundedReceiver<Incoming>, generation: u64) {
        while let Some(m) = rx.recv().await {
            if self.st.lock().expect("verrou").generation != generation {
                return;
            }
            match m {
                Incoming::Notification { method, params } => self.on_notification(&state, &method, params).await,
                Incoming::Request { id, method, params } => self.on_request(&state, id, method, params),
            }
        }
        // Connexion perdue.
        let lost = {
            let mut st = self.st.lock().expect("verrou");
            if st.generation != generation {
                false
            } else {
                st.generation += 1;
                st.conn = None;
                true
            }
        };
        if lost {
            tracing::warn!("session {} : pont perdu", self.id);
            sandboxes::stop(&state, &self.id).await;
            self.close_terminals(&state, "sandbox perdue");
            self.fail_turn(&state, "sandbox perdue").await;
            self.set_sandbox("stopped", Some("connexion à la sandbox perdue".into()));
        }
    }

    fn on_request(&self, state: &AppState, id: Value, method: String, mut params: Value) {
        if !types::SERVER_REQUESTS.contains(&method.as_str()) {
            tracing::debug!("requête de Codex ignorée : {method}");
            let client = self.st.lock().expect("verrou").conn.as_ref().map(|c| c.client.clone());
            if let Some(c) = client {
                let _ = c.respond_error(&id, -32601, "non pris en charge par EngineX");
            }
            return;
        }
        // Tour de mémoire : personne devant l'écran, le pont refuse lui-même.
        let memory = {
            let st = self.st.lock().expect("verrou");
            st.turn
                .as_ref()
                .is_some_and(|t| t.origin == "memory")
                .then(|| st.conn.as_ref().map(|c| c.client.clone()))
                .flatten()
        };
        if let Some(c) = memory {
            let answer = if method == "item/tool/requestUserInput" {
                json!({"answers": {}})
            } else {
                json!({"decision": "decline"})
            };
            if let Err(e) = c.respond(&id, answer) {
                tracing::warn!("session {} : réponse à {method} : {e}", self.id);
            }
            return;
        }
        state.masker().mask_json(&mut params);
        let rid = rid_of(&id);
        {
            let mut st = self.st.lock().expect("verrou");
            st.requests.push(PendingReq {
                rid: rid.clone(),
                id,
                method: method.clone(),
                params: params.clone(),
            });
            st.last_activity = Instant::now();
        }
        self.emit("request", json!({"rid": rid, "method": method, "params": params}));
    }

    async fn on_notification(self: &Arc<Self>, state: &AppState, method: &str, mut params: Value) {
        if !types::WHITELIST.contains(&method) {
            tracing::debug!("notification de Codex ignorée : {method}");
            return;
        }
        if method == "command/exec/outputDelta" {
            self.on_exec_output(state, &params);
            return;
        }
        let masker = state.masker();
        if let Some(field) = stream_field(method) {
            let item_id = params["itemId"].as_str().unwrap_or("").to_string();
            let delta = params["delta"].as_str().unwrap_or("").to_string();
            let out = {
                let mut st = self.st.lock().expect("verrou");
                st.last_activity = Instant::now();
                let Some(t) = st.turn.as_mut() else { return };
                let out = t
                    .streams
                    .entry((item_id.clone(), field))
                    .or_default()
                    .push(&masker, &delta);
                if !out.is_empty() {
                    let p = t.partial.entry(item_id).or_default();
                    let prev = p.get(field).and_then(Value::as_str).unwrap_or("").to_string();
                    p.insert(field.into(), json!(prev + &out));
                }
                out
            };
            if out.is_empty() {
                return;
            }
            params["delta"] = json!(out);
            self.emit("codex", json!({"method": method, "params": params}));
            return;
        }
        masker.mask_json(&mut params);
        match method {
            "turn/started" => {
                if let Some(tid) = types::notif_turn(&params) {
                    self.adopt_turn(state, tid).await;
                }
            }
            "item/started" | "item/completed" => {
                let item = params["item"].clone();
                if item["type"] == "userMessage" {
                    return;
                }
                let iid = item["id"].as_str().unwrap_or("").to_string();
                let tid = params["turnId"].as_str().map(str::to_string);
                {
                    let mut st = self.st.lock().expect("verrou");
                    st.last_activity = Instant::now();
                    if let Some(t) = st.turn.as_mut() {
                        match t.items.iter_mut().find(|i| i["id"] == iid.as_str()) {
                            Some(slot) => *slot = item.clone(),
                            None => t.items.push(item.clone()),
                        }
                        if method == "item/completed" {
                            t.partial.remove(&iid);
                            t.streams.retain(|(id, _), _| *id != iid);
                        }
                    }
                }
                if method == "item/completed" && item["type"] == "contextCompaction" {
                    // Codex vient de refaire son message developer avec les
                    // instructions de la connexion (essai E9).
                    let sha = self.st.lock().expect("verrou").conn_memory_sha.clone();
                    if let Some(sha) = sha {
                        let _ = sessions::set_session_field(state, &self.id, "memory_seen", Some(&sha)).await;
                    }
                }
                if method == "item/completed" {
                    let kind = item["type"].as_str().unwrap_or("unknown").to_string();
                    if let Err(e) = sessions::store_item(state, &self.id, tid.as_deref(), &iid, &kind, &item).await {
                        tracing::warn!("élément {iid} non stocké : {e}");
                    }
                }
            }
            "thread/tokenUsage/updated" => {
                if let Some(t) = self.st.lock().expect("verrou").turn.as_mut() {
                    t.usage = Some(params["tokenUsage"].clone());
                }
            }
            "serverRequest/resolved" => {
                let rid = rid_of(&params["requestId"]);
                let removed = {
                    let mut st = self.st.lock().expect("verrou");
                    let before = st.requests.len();
                    st.requests.retain(|r| r.rid != rid);
                    before != st.requests.len()
                };
                if removed {
                    self.emit("request_resolved", json!({ "rid": rid }));
                }
            }
            _ => {}
        }
        self.emit("codex", json!({"method": method, "params": params.clone()}));
        if method == "turn/completed" {
            self.complete_turn(state, &params).await;
        }
    }

    async fn complete_turn(self: &Arc<Self>, state: &AppState, params: &Value) {
        let turn = &params["turn"];
        let tid = turn["id"].as_str().unwrap_or("").to_string();
        let status = match turn["status"].as_str() {
            Some(s @ ("completed" | "interrupted" | "failed")) => s.to_string(),
            _ => "completed".to_string(),
        };
        let error = turn["error"]["message"].as_str().map(str::to_string);
        let (t, reqs) = {
            let mut st = self.st.lock().expect("verrou");
            let t = st.turn.take();
            st.last_activity = match &t {
                Some(t) if t.origin == "memory" => st.memory_idle_base.take().unwrap_or_else(Instant::now),
                _ => Instant::now(),
            };
            (t, std::mem::take(&mut st.requests))
        };
        for r in reqs {
            self.emit("request_resolved", json!({ "rid": r.rid }));
        }
        let (started, usage, origin, pre) = t
            .as_ref()
            .map(|t| (t.started_at, t.usage.clone(), t.origin, t.snap_pre.clone()))
            .unwrap_or((now(), None, "user", None));
        if let Err(e) = sessions::finish_turn(
            state,
            &self.id,
            &tid,
            started,
            origin,
            &status,
            error.as_deref(),
            usage.as_ref(),
        )
        .await
        {
            tracing::warn!("tour {tid} non stocké : {e}");
        }
        if let Some(t) = &t
            && !t.user_stored
        {
            let _ = sessions::store_item(
                state,
                &self.id,
                Some(&tid),
                t.user["id"].as_str().unwrap_or(""),
                "userMessage",
                &t.user,
            )
            .await;
        }
        let _ = sessions::touch(state, &self.id).await;
        self.emit(
            "turn_stored",
            json!({"turn": {"id": tid, "status": status, "error": error, "usage": usage, "origin": origin}}),
        );
        self.write_record(state);
        crate::verify::spawn_after_turn(state.clone(), self.clone(), tid, status, origin, pre);
    }

    pub(crate) fn record(&self, state: &AppState) {
        self.write_record(state);
    }

    fn write_record(&self, state: &AppState) {
        let Some(dir) = &state.config().sandbox.record_dir else {
            return;
        };
        let Some(rec) = self.recorder.lock().expect("verrou").clone() else {
            return;
        };
        let (cwd, home) = {
            let st = self.st.lock().expect("verrou");
            let cwd = st.conn.as_ref().map(|c| c.handle.cwd.clone()).unwrap_or_default();
            (
                cwd,
                // Dossier de la session entier (CODEX_HOME et pièces jointes) :
                // aucun chemin du serveur dans les fichiers attendus.
                sandboxes::session_dir(state, &self.id).display().to_string(),
            )
        };
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join(format!("{}.jsonl", self.id)), rec.to_jsonl(&cwd, &home));
    }
}

async fn wait_ready(state: &AppState, url: &str, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        let ok = state
            .0
            .http
            .get(url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if ok {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Arrêt d'EngineX : toutes les sandboxes supprimées en parallèle.
pub async fn shutdown(state: &AppState) {
    let hubs = hubs(state);
    let tasks = hubs.into_iter().map(|h| {
        let st = state.clone();
        async move { h.stop_conn(&st, "arrêt d'EngineX").await }
    });
    futures_util::future::join_all(tasks).await;
}

/// Session dont la ligne est `row` en JSON (snapshot).
pub fn session_json(r: &SessionRow, running: bool) -> Value {
    json!({
        "id": r.id, "topic_id": r.topic_id, "title": r.title, "thread_id": r.thread_id,
        "approval": r.approval, "archived": r.archived != 0,
        "created_at": r.created_at, "updated_at": r.updated_at, "running": running,
        "copy_branch": r.copy_branch, "pending_note": r.pending_note,
        "pending_revert": r.pending_revert.is_some(), "undo": r.undo_snap.is_some(),
    })
}
