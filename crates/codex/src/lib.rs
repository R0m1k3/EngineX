//! Client JSON-RPC 2.0 de `codex app-server` sur WebSocket (un message JSON
//! par trame texte, jeton en `Authorization: Bearer`).
//!
//! `connect` rend le client et le flux des messages venus du serveur
//! (notifications et requêtes). Quand la connexion tombe, les requêtes en
//! attente échouent (`Error::Closed`) et le flux se termine.

pub mod record;
pub mod types;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

pub use record::Recorder;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connexion à l'app-server impossible : {0}")]
    Connect(String),
    #[error("délai dépassé pour {0}")]
    Timeout(String),
    #[error("connexion à l'app-server fermée")]
    Closed,
    #[error("erreur de l'app-server {code} : {message}")]
    Rpc { code: i64, message: String },
}

/// Message venu du serveur.
#[derive(Debug, Clone)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// Requête du serveur : répondre par `Client::respond(id, …)`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

type Pending = Arc<Mutex<HashMap<i64, (String, oneshot::Sender<Result<Value, Error>>)>>>;

#[derive(Clone)]
pub struct Client {
    out: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next: Arc<AtomicI64>,
    timeout: Duration,
    recorder: Option<Arc<Recorder>>,
}

/// Se connecte à `url` (`ws://…`) avec le jeton `token`.
pub async fn connect(
    url: &str,
    token: &str,
    timeout: Duration,
    recorder: Option<Arc<Recorder>>,
) -> Result<(Client, mpsc::UnboundedReceiver<Incoming>), Error> {
    let mut req = url.into_client_request().map_err(|e| Error::Connect(e.to_string()))?;
    let auth = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| Error::Connect(e.to_string()))?;
    req.headers_mut().insert("authorization", auth);
    let (ws, _) = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| Error::Timeout("connexion".into()))?
        .map_err(|e| Error::Connect(e.to_string()))?;
    let (mut sink, mut stream) = ws.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<Incoming>();
    let pending: Pending = Arc::default();

    tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let p = pending.clone();
    let rec = recorder.clone();
    let pong = out.clone();
    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                Ok(Message::Ping(d)) => {
                    let _ = pong.send(Message::Pong(d));
                    continue;
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                tracing::warn!("app-server : message illisible");
                continue;
            };
            let method = v.get("method").and_then(Value::as_str).map(str::to_string);
            let id = v.get("id").cloned().filter(|i| !i.is_null());
            match (method, id) {
                (Some(method), Some(id)) => {
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    if let Some(r) = &rec {
                        r.server_message(&v);
                    }
                    if in_tx.send(Incoming::Request { id, method, params }).is_err() {
                        break;
                    }
                }
                (Some(method), None) => {
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    if let Some(r) = &rec {
                        r.server_message(&v);
                    }
                    if in_tx.send(Incoming::Notification { method, params }).is_err() {
                        break;
                    }
                }
                (None, Some(id)) => {
                    let Some(n) = id.as_i64() else { continue };
                    let Some((_, tx)) = p.lock().expect("verrou").remove(&n) else {
                        continue;
                    };
                    let res = match v.get("error") {
                        Some(e) if !e.is_null() => Err(Error::Rpc {
                            code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                            message: e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
                        }),
                        _ => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    if let (Some(r), Ok(result)) = (&rec, &res) {
                        r.response(n, result);
                    }
                    let _ = tx.send(res);
                }
                (None, None) => {}
            }
        }
        // Connexion perdue : toutes les attentes échouent.
        for (_, (_, tx)) in p.lock().expect("verrou").drain() {
            let _ = tx.send(Err(Error::Closed));
        }
    });

    Ok((
        Client {
            out,
            pending,
            next: Arc::new(AtomicI64::new(1)),
            timeout,
            recorder,
        },
        in_rx,
    ))
}

impl Client {
    fn send_json(&self, v: &Value) -> Result<(), Error> {
        self.out
            .send(Message::Text(v.to_string().into()))
            .map_err(|_| Error::Closed)
    }

    /// Envoie une requête et attend sa réponse (délai du client).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, Error> {
        self.request_with_timeout(method, params, Some(self.timeout)).await
    }

    /// Comme `request`, avec un autre délai (`None` : pas de délai, pour
    /// `command/exec` qui ne répond qu'à la fin du processus).
    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, Error> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("verrou")
            .insert(id, (method.to_string(), tx));
        if let Some(r) = &self.recorder {
            r.request_sent(id, method);
        }
        if let Err(e) = self.send_json(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            self.pending.lock().expect("verrou").remove(&id);
            return Err(e);
        }
        let Some(timeout) = timeout else {
            return rx.await.unwrap_or(Err(Error::Closed));
        };
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(Error::Closed),
            Err(_) => {
                self.pending.lock().expect("verrou").remove(&id);
                Err(Error::Timeout(method.to_string()))
            }
        }
    }

    pub fn notify(&self, method: &str, params: Option<Value>) -> Result<(), Error> {
        let mut v = json!({"jsonrpc": "2.0", "method": method});
        if let Some(p) = params {
            v["params"] = p;
        }
        if let Some(r) = &self.recorder {
            r.notification_sent(method);
        }
        self.send_json(&v)
    }

    /// Réponse à une requête du serveur.
    pub fn respond(&self, id: &Value, result: Value) -> Result<(), Error> {
        if let Some(r) = &self.recorder {
            r.reply(id);
        }
        self.send_json(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// Erreur en réponse à une requête du serveur.
    pub fn respond_error(&self, id: &Value, code: i64, message: &str) -> Result<(), Error> {
        self.send_json(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
    }

    /// Poignée de main : `initialize` puis `initialized`.
    pub async fn initialize(&self, version: &str) -> Result<Value, Error> {
        let r = self
            .request(
                "initialize",
                json!({
                    "clientInfo": {"name": "enginex", "title": "EngineX", "version": version},
                    "capabilities": {"experimentalApi": false, "requestAttestation": false},
                }),
            )
            .await?;
        self.notify("initialized", None)?;
        Ok(r)
    }

    /// Ferme la connexion (les attentes échouent).
    pub fn close(&self) {
        let _ = self.out.send(Message::Close(None));
    }

    pub fn is_closed(&self) -> bool {
        self.out.is_closed()
    }
}
