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
}

struct PendingReq {
    rid: String,
    id: Value,
    method: String,
    params: Value,
}

struct HubState {
    sandbox: (String, Option<String>),
    conn: Option<Conn>,
    /// Incrémenté à chaque connexion ou arrêt : un lecteur périmé se tait.
    generation: u64,
    turn: Option<TurnState>,
    requests: Vec<PendingReq>,
    last_activity: Instant,
}

pub struct Hub {
    pub id: String,
    tx: broadcast::Sender<Event>,
    st: Mutex<HubState>,
    start: tokio::sync::Mutex<()>,
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
                    conn: None,
                    generation: 0,
                    turn: None,
                    requests: Vec::new(),
                    last_activity: Instant::now(),
                }),
                start: tokio::sync::Mutex::new(()),
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

    pub fn running(&self) -> bool {
        self.st.lock().expect("verrou").turn.is_some()
    }

    pub fn has_conn(&self) -> bool {
        self.st.lock().expect("verrou").conn.is_some()
    }

    /// Durée d'inactivité d'une sandbox vivante sans tour ni requête.
    pub fn idle_for(&self) -> Option<Duration> {
        let st = self.st.lock().expect("verrou");
        (st.conn.is_some() && st.turn.is_none() && st.requests.is_empty()).then(|| st.last_activity.elapsed())
    }

    fn touch(&self) {
        self.st.lock().expect("verrou").last_activity = Instant::now();
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
    pub fn begin(&self, user: Value) -> Result<(), ApiError> {
        let mut st = self.st.lock().expect("verrou");
        if st.turn.is_some() {
            return Err(ApiError::TurnInProgress);
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
        });
        st.last_activity = Instant::now();
        drop(st);
        self.emit("user_message", json!({ "item": user }));
        Ok(())
    }

    /// Arrête la sandbox (inactivité, suppression, archivage, arrêt).
    pub async fn stop_conn(&self, state: &AppState, reason: &str) {
        let conn = {
            let mut st = self.st.lock().expect("verrou");
            st.generation += 1;
            st.conn.take()
        };
        let had = conn.is_some();
        if let Some(c) = conn {
            c.client.close();
        }
        sandboxes::stop(state, &self.id).await;
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
        let conn = match self.ensure_conn(&state).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("session {} : sandbox indisponible : {e}", self.id);
                self.set_sandbox("error", Some(e.clone()));
                self.fail_turn(&state, &e).await;
                return;
            }
        };
        let (client, thread_id, attachments) = conn;
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
        if !text.is_empty() {
            input.push(types::text_input(&text));
        }
        for name in &images {
            input.push(types::local_image(&format!("{attachments}/{name}")));
        }
        let policy = types::approval_policy(&approval);
        match client
            .request("turn/start", types::turn_start(&thread_id, input, policy))
            .await
        {
            Ok(r) => {
                if let Some(tid) = types::turn_id(&r) {
                    self.adopt_turn(&state, tid).await;
                }
            }
            Err(e) => self.fail_turn(&state, &format!("turn/start : {e}")).await,
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
        let (user_id, started) = {
            let mut st = self.st.lock().expect("verrou");
            let Some(t) = st.turn.as_mut() else { return };
            if t.id.is_some() {
                return;
            }
            t.id = Some(tid.to_string());
            (
                t.user_stored.then(|| t.user["id"].as_str().unwrap_or("").to_string()),
                t.started_at,
            )
        };
        if let Err(e) = sessions::insert_turn(state, &self.id, tid, started).await {
            tracing::warn!("tour {tid} : {e}");
        }
        if let Some(u) = user_id {
            let _ = sessions::set_item_turn(state, &self.id, &u, tid).await;
        }
    }

    /// Termine le tour ouvert en échec (avant ou pendant `turn/start`, ou pont
    /// perdu) ; le message de Michael reste dans l'historique.
    async fn fail_turn(&self, state: &AppState, error: &str) {
        let _ = self.store_user(state).await;
        let (turn, reqs) = {
            let mut st = self.st.lock().expect("verrou");
            (st.turn.take(), std::mem::take(&mut st.requests))
        };
        for r in reqs {
            self.emit("request_resolved", json!({ "rid": r.rid }));
        }
        let Some(t) = turn else { return };
        let tid = t.id.clone().unwrap_or_else(|| format!("ex-{}", uuid::Uuid::new_v4()));
        let error = state.masker().mask(error).into_owned();
        let _ = sessions::finish_turn(
            state,
            &self.id,
            &tid,
            t.started_at,
            "failed",
            Some(&error),
            t.usage.as_ref(),
        )
        .await;
        let _ = sessions::set_item_turn(state, &self.id, t.user["id"].as_str().unwrap_or(""), &tid).await;
        self.emit(
            "turn_stored",
            json!({"turn": {"id": tid, "status": "failed", "error": error, "usage": t.usage}}),
        );
        self.touch();
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
        let mut lost_context = false;
        let thread_id = match &session.thread_id {
            Some(tid) => match client
                .request("thread/resume", types::thread_resume(tid, &handle.cwd, policy))
                .await
            {
                Ok(_) => {
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
                .request("thread/start", types::thread_start(&handle.cwd, policy))
                .await
            {
                Ok(r) => match types::thread_id(&r) {
                    Some(t) => {
                        let t = t.to_string();
                        let _ = sessions::set_thread(state, &self.id, &t).await;
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
        for turn in list["data"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let Some(tid) = turn["id"].as_str() else { continue };
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

    async fn on_notification(&self, state: &AppState, method: &str, mut params: Value) {
        if !types::WHITELIST.contains(&method) {
            tracing::debug!("notification de Codex ignorée : {method}");
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

    async fn complete_turn(&self, state: &AppState, params: &Value) {
        let turn = &params["turn"];
        let tid = turn["id"].as_str().unwrap_or("").to_string();
        let status = match turn["status"].as_str() {
            Some(s @ ("completed" | "interrupted" | "failed")) => s.to_string(),
            _ => "completed".to_string(),
        };
        let error = turn["error"]["message"].as_str().map(str::to_string);
        let (t, reqs) = {
            let mut st = self.st.lock().expect("verrou");
            st.last_activity = Instant::now();
            (st.turn.take(), std::mem::take(&mut st.requests))
        };
        for r in reqs {
            self.emit("request_resolved", json!({ "rid": r.rid }));
        }
        let (started, usage) = t
            .as_ref()
            .map(|t| (t.started_at, t.usage.clone()))
            .unwrap_or((now(), None));
        if let Err(e) = sessions::finish_turn(
            state,
            &self.id,
            &tid,
            started,
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
            json!({"turn": {"id": tid, "status": status, "error": error, "usage": usage}}),
        );
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
    })
}
