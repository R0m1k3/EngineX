//! Terminal d'une session : WebSocket navigateur ↔ EngineX ↔ `command/exec`
//! en PTY dans la sandbox. La sortie est masquée **avant** le tampon d'envoi
//! (borné en octets) : des octets jetés ne coupent jamais une valeur du
//! coffre en dehors du masquage.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{
        Path as UrlPath, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use enginex_codex::types;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::{
    auth, bridge,
    error::ApiError,
    mask::{ByteStream, Masker},
    sessions,
    state::AppState,
};

/// Tampon d'envoi d'un terminal.
pub const BUFFER: usize = 4 << 20;

#[derive(Debug)]
pub enum Out {
    Data(Vec<u8>),
    Dropped(u64),
    Exit { code: Option<i64>, reason: Option<String> },
}

#[derive(Default)]
struct SinkState {
    items: VecDeque<Out>,
    bytes: usize,
    dropped: u64,
    stream: ByteStream,
    closed: bool,
}

/// Sortie d'un terminal : masquage en flux puis file bornée en octets.
#[derive(Default)]
pub struct Sink {
    st: Mutex<SinkState>,
    notify: Notify,
}

impl Sink {
    /// Octets bruts de `command/exec/outputDelta`.
    pub fn push(&self, m: &Masker, raw: &[u8]) {
        let mut st = self.st.lock().expect("verrou");
        if st.closed {
            return;
        }
        let out = st.stream.push(m, raw);
        Self::queue(&mut st, out);
        drop(st);
        self.notify.notify_one();
    }

    fn queue(st: &mut SinkState, out: Vec<u8>) {
        if out.is_empty() {
            return;
        }
        if st.bytes + out.len() > BUFFER {
            st.dropped += out.len() as u64;
            return;
        }
        if st.dropped > 0 {
            let n = std::mem::take(&mut st.dropped);
            st.items.push_back(Out::Dropped(n));
        }
        st.bytes += out.len();
        st.items.push_back(Out::Data(out));
    }

    /// Fin : reste du flux masqué, puis `exit`.
    pub fn finish(&self, m: &Masker, code: Option<i64>, reason: Option<String>) {
        let mut st = self.st.lock().expect("verrou");
        if st.closed {
            return;
        }
        let out = st.stream.flush(m);
        Self::queue(&mut st, out);
        if st.dropped > 0 {
            let n = std::mem::take(&mut st.dropped);
            st.items.push_back(Out::Dropped(n));
        }
        st.items.push_back(Out::Exit { code, reason });
        st.closed = true;
        drop(st);
        self.notify.notify_one();
    }

    async fn next(&self) -> Out {
        loop {
            let notified = self.notify.notified();
            {
                let mut st = self.st.lock().expect("verrou");
                if let Some(o) = st.items.pop_front() {
                    if let Out::Data(d) = &o {
                        st.bytes -= d.len();
                    }
                    return o;
                }
            }
            notified.await;
        }
    }
}

fn text(v: Value) -> Message {
    Message::Text(v.to_string().into())
}

/// Montée WebSocket (`Origin` obligatoire et de la même origine).
pub async fn ws(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !auth::origin_ok(&state, &headers) {
        return Err(ApiError::ForbiddenOrigin);
    }
    sessions::row(&state, &id).await?;
    state.0.sandboxes.backend()?;
    Ok(upgrade.on_upgrade(move |socket| run(state, id, socket)))
}

async fn run(state: AppState, id: String, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let hub = bridge::hub(&state, &id);
    let _ = tx.send(text(json!({"type": "status", "state": "starting"}))).await;
    let (client, cwd) = match hub.ready(&state).await {
        Ok(c) => c,
        Err(e) => {
            let msg = state.masker().mask(&e).into_owned();
            let _ = tx.send(text(json!({"type": "error", "message": msg}))).await;
            let _ = tx.close().await;
            return;
        }
    };
    let pid = format!("term-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let sink = Arc::new(Sink::default());
    for old in hub.add_terminal(&state, &pid, sink.clone()) {
        let _ = client
            .request("command/exec/terminate", types::exec_terminate(&old))
            .await;
    }
    let _ = tx.send(text(json!({"type": "status", "state": "ready"}))).await;
    // Le processus : la réponse n'arrive qu'à sa fin.
    {
        let (client, cwd, pid, sink, state) = (client.clone(), cwd.clone(), pid.clone(), sink.clone(), state.clone());
        tokio::spawn(async move {
            let mut params = types::command_exec(&["bash", "-l"], &cwd, Some(&pid), Some((24, 80)), None);
            // Sans cela, Codex cesse d'envoyer la sortie après 1 Mio.
            params["disableOutputCap"] = json!(true);
            let r = client.request_with_timeout("command/exec", params, None).await;
            let m = state.masker();
            match r {
                Ok(v) => sink.finish(&m, v["exitCode"].as_i64(), None),
                Err(e) => sink.finish(&m, None, Some(m.mask(&e.to_string()).into_owned())),
            }
        });
    }
    let out_task = {
        let sink = sink.clone();
        tokio::spawn(async move {
            loop {
                let msg = match sink.next().await {
                    Out::Data(d) => Message::Binary(d.into()),
                    Out::Dropped(n) => text(json!({"type": "dropped", "bytes": n})),
                    Out::Exit { code, reason } => {
                        let _ = tx
                            .send(text(json!({"type": "exit", "code": code, "reason": reason})))
                            .await;
                        let _ = tx.close().await;
                        return;
                    }
                };
                if tx.send(msg).await.is_err() {
                    return;
                }
            }
        })
    };
    while let Some(Ok(m)) = rx.next().await {
        match m {
            Message::Binary(b) => {
                hub.touch_activity();
                // Les premières frappes peuvent précéder le démarrage du
                // processus (`command/exec` envoyé par une autre tâche) :
                // nouvel essai pendant 2 s au plus.
                let params = types::exec_write(&pid, &STANDARD.encode(&b));
                for i in 0..20 {
                    match client.request("command/exec/write", params.clone()).await {
                        Err(enginex_codex::Error::Rpc { .. }) if i < 19 && !out_task.is_finished() => {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        _ => break,
                    }
                }
            }
            Message::Text(t) => {
                let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                if v["type"] == "resize"
                    && let (Some(c), Some(r)) = (v["cols"].as_u64(), v["rows"].as_u64())
                {
                    let (c, r) = (c.clamp(2, 1000) as u16, r.clamp(2, 1000) as u16);
                    let _ = client
                        .request("command/exec/resize", types::exec_resize(&pid, r, c))
                        .await;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
        if out_task.is_finished() {
            break;
        }
    }
    // Fermeture du WebSocket : le processus est terminé.
    hub.remove_terminal(&pid);
    let _ = client
        .request("command/exec/terminate", types::exec_terminate(&pid))
        .await;
    sink.finish(&state.masker(), None, Some("fermé".into()));
    let _ = out_task.await;
}
