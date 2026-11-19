//! File d'attente commune des tours (étape 7a).
//!
//! Un seul modèle tient sur les GPU : les tours des sessions, des crons et
//! des agents passent l'un après l'autre (autant à la fois que de slots
//! `-np` du preset chargé). L'unité est le **tour** (tour, vérification,
//! corrections : tant que la session est occupée), pas la requête : deux tours
//! qui alterneraient leurs requêtes dans un slot jetteraient le cache de prompt
//! à chaque alternance.
//!
//! Un *bail* donne le droit d'envoyer des requêtes au modèle chargé. Ordre de
//! la file : priorité (0 = Michael, 1 = fond), puis, à priorité égale, les
//! demandes du modèle déjà chargé (une demande qui attend un autre modèle
//! passe quand même après [`SWAP_AFTER`]), puis l'ancienneté. Quand la tête de
//! file veut un autre preset, plus aucun bail n'est donné ; dès que les baux
//! sont rendus, la file charge ce preset.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use enginex_engine::supervisor::EngineState;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::{bridge, state::AppState};

/// Une demande d'un autre modèle passe devant celles du modèle chargé après ce délai.
pub const SWAP_AFTER: Duration = Duration::from_secs(15 * 60);

/// Priorité des tours de Michael (et des clés d'API externes).
pub const USER: u8 = 0;
/// Priorité des tours de fond (cron, agent, mémoire de fin de session).
pub const BACKGROUND: u8 = 1;

/// Qui tient ou attend un bail.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    Session(String),
    /// Requête d'une clé d'API externe.
    Request(u64),
}

#[derive(Clone, Debug)]
pub struct Ask {
    pub who: Who,
    pub priority: u8,
    /// Preset voulu ; `None` = le modèle chargé, quel qu'il soit.
    pub preset: Option<i64>,
    /// Libellé lisible (« session X », « cron Y », « clé Z »).
    pub label: String,
}

struct Waiter {
    ticket: u64,
    ask: Ask,
    since: Instant,
    since_unix: i64,
}

struct Holder {
    ask: Ask,
    since_unix: i64,
}

#[derive(Default)]
struct Inner {
    holders: HashMap<Who, Holder>,
    waiting: Vec<Waiter>,
    /// Preset en cours de chargement par la file.
    swapping: Option<i64>,
    /// Chargements échoués : (preset, dernier ticket concerné, message, quand).
    failed: Vec<(i64, u64, String, Instant)>,
    /// Slots (`-np`) de chaque preset lancé.
    slots: HashMap<i64, usize>,
    next_ticket: u64,
}

#[derive(Default)]
pub struct Queue {
    inner: Mutex<Inner>,
    notify: Notify,
    next_request: AtomicU64,
}

enum Action {
    Grant(u64),
    Swap(i64),
    Wait,
}

/// Preset chargé (ou en chargement, ou en échec) selon le superviseur.
fn loaded(state: &AppState) -> Option<i64> {
    match state.supervisor().state() {
        EngineState::Stopped | EngineState::Stopping => None,
        s => s.preset_id(),
    }
}

impl Inner {
    fn needs_swap(&self, ask: &Ask, loaded: Option<i64>) -> bool {
        match ask.preset {
            Some(p) => Some(p) != loaded,
            None => false,
        }
    }

    /// Prochaine action de la file (même résultat pour tous les appelants).
    fn decide(&self, loaded: Option<i64>) -> Action {
        if self.swapping.is_some() {
            return Action::Wait;
        }
        let Some(head) = self.waiting.iter().min_by_key(|w| {
            let swap = self.needs_swap(&w.ask, loaded);
            (w.ask.priority, swap && w.since.elapsed() < SWAP_AFTER, w.ticket)
        }) else {
            return Action::Wait;
        };
        if self.needs_swap(&head.ask, loaded) {
            // Tête de file sur un autre modèle : on attend que les baux soient rendus.
            return match (self.holders.is_empty(), head.ask.preset) {
                (true, Some(p)) => Action::Swap(p),
                _ => Action::Wait,
            };
        }
        let slots = loaded.and_then(|p| self.slots.get(&p).copied()).unwrap_or(1).max(1);
        // Bail déjà tenu par la même session : pas un slot de plus.
        if self.holders.len() < slots || self.holders.contains_key(&head.ask.who) {
            Action::Grant(head.ticket)
        } else {
            Action::Wait
        }
    }

    fn position(&self, ticket: u64, loaded: Option<i64>) -> usize {
        let mut v: Vec<_> = self
            .waiting
            .iter()
            .map(|w| {
                let swap = self.needs_swap(&w.ask, loaded);
                (
                    (w.ask.priority, swap && w.since.elapsed() < SWAP_AFTER, w.ticket),
                    w.ticket,
                )
            })
            .collect();
        v.sort();
        v.iter().position(|(_, t)| *t == ticket).map_or(0, |i| i + 1)
    }
}

/// Bail tenu ; rendu à sa destruction.
pub struct Lease {
    state: AppState,
    who: Who,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.state.queue().release(&self.who);
    }
}

impl Queue {
    /// `-np` d'un preset au moment où il est lancé.
    pub fn note_launch(&self, preset: i64, parallel: u64) {
        let mut g = self.inner.lock().expect("file");
        g.slots.insert(preset, parallel.clamp(1, 8) as usize);
        drop(g);
        self.notify.notify_waiters();
    }

    pub fn holds(&self, who: &Who) -> bool {
        self.inner.lock().expect("file").holders.contains_key(who)
    }

    fn release(&self, who: &Who) {
        self.inner.lock().expect("file").holders.remove(who);
        self.notify.notify_waiters();
    }

    pub fn request_id(&self) -> Who {
        Who::Request(self.next_request.fetch_add(1, Ordering::Relaxed))
    }

    /// Attend un bail. `on_wait` reçoit l'état de l'attente à chaque changement
    /// (position, ce qui passe avant, chargement en cours) ; `cancel` résolu =
    /// demande retirée (`Err`).
    pub async fn acquire(
        &self,
        state: &AppState,
        ask: Ask,
        on_wait: &(dyn Fn(Option<Value>) + Send + Sync),
        cancel: impl std::future::Future<Output = ()>,
    ) -> Result<Lease, String> {
        tokio::pin!(cancel);
        let ticket = {
            let mut g = self.inner.lock().expect("file");
            g.next_ticket += 1;
            let ticket = g.next_ticket;
            g.waiting.push(Waiter {
                ticket,
                ask: ask.clone(),
                since: Instant::now(),
                since_unix: crate::state::now(),
            });
            ticket
        };
        // Retire la demande si l'attente est abandonnée (client parti, tour annulé).
        struct Leave<'a>(&'a Queue, u64, bool);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                if !self.2 {
                    self.0
                        .inner
                        .lock()
                        .expect("file")
                        .waiting
                        .retain(|w| w.ticket != self.1);
                    self.0.notify.notify_waiters();
                }
            }
        }
        let mut leave = Leave(self, ticket, false);
        let mut shown: Option<Value> = None;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let lo = loaded(state);
            let (action, view) = {
                let mut g = self.inner.lock().expect("file");
                g.failed.retain(|f| f.3.elapsed() < Duration::from_secs(600));
                if let Some(f) = g.failed.iter().find(|f| Some(f.0) == ask.preset && ticket <= f.1) {
                    let msg = f.2.clone();
                    g.waiting.retain(|w| w.ticket != ticket);
                    leave.2 = true;
                    drop(g);
                    self.notify.notify_waiters();
                    on_wait(None);
                    return Err(msg);
                }
                let action = g.decide(lo);
                if let Action::Grant(t) = action
                    && t == ticket
                {
                    g.waiting.retain(|w| w.ticket != ticket);
                    g.holders.insert(
                        ask.who.clone(),
                        Holder {
                            ask: ask.clone(),
                            since_unix: crate::state::now(),
                        },
                    );
                    leave.2 = true;
                    drop(g);
                    self.notify.notify_waiters();
                    if shown.is_some() {
                        on_wait(None);
                    }
                    return Ok(Lease {
                        state: state.clone(),
                        who: ask.who.clone(),
                    });
                }
                if let Action::Swap(p) = action {
                    g.swapping = Some(p);
                }
                (action, wait_view(&g, ticket, lo))
            };
            // Bail pour un autre : la libération qui l'a permis l'a déjà
            // réveillé (se réveiller soi-même ici tournerait sans fin).
            if let Action::Swap(p) = action {
                spawn_swap(state.clone(), p);
            }
            if shown.as_ref() != Some(&view) {
                on_wait(Some(view.clone()));
                shown = Some(view);
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = &mut cancel => {
                    on_wait(None);
                    return Err("retiré de la file d'attente".into());
                }
            }
        }
    }

    /// File pour le tableau de bord et `/api/queue`.
    pub fn snapshot(&self, state: &AppState) -> Value {
        let lo = loaded(state);
        let g = self.inner.lock().expect("file");
        let who = |w: &Who| match w {
            Who::Session(s) => json!({"session_id": s}),
            Who::Request(_) => json!({"session_id": null}),
        };
        let holders: Vec<Value> = g
            .holders
            .values()
            .map(|h| {
                let mut v = who(&h.ask.who);
                v["label"] = json!(h.ask.label);
                v["priority"] = json!(h.ask.priority);
                v["since"] = json!(h.since_unix);
                v
            })
            .collect();
        let mut waiting: Vec<(usize, Value)> = g
            .waiting
            .iter()
            .map(|w| {
                let mut v = who(&w.ask.who);
                v["label"] = json!(w.ask.label);
                v["priority"] = json!(w.ask.priority);
                v["preset_id"] = json!(w.ask.preset);
                v["since"] = json!(w.since_unix);
                v["needs_swap"] = json!(g.needs_swap(&w.ask, lo));
                (g.position(w.ticket, lo), v)
            })
            .collect();
        waiting.sort_by_key(|(p, _)| *p);
        let slots = lo.and_then(|p| g.slots.get(&p).copied()).unwrap_or(1);
        json!({
            "loaded": lo,
            "slots": slots,
            "swapping": g.swapping,
            "holders": holders,
            "waiting": waiting.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        })
    }
}

fn wait_view(g: &Inner, ticket: u64, lo: Option<i64>) -> Value {
    let ahead: Vec<Value> = g
        .holders
        .values()
        .map(|h| {
            json!({"label": h.ask.label, "since": h.since_unix, "priority": h.ask.priority,
                "session_id": match &h.ask.who { Who::Session(s) => json!(s), _ => Value::Null }})
        })
        .collect();
    let me = g.waiting.iter().find(|w| w.ticket == ticket);
    json!({
        "position": g.position(ticket, lo),
        "running": ahead,
        "swapping": g.swapping,
        "needs_swap": me.is_some_and(|w| g.needs_swap(&w.ask, lo)),
    })
}

/// Charge `preset` pour la tête de file (même chemin que « Activer »).
fn spawn_swap(state: AppState, preset: i64) {
    tokio::spawn(async move {
        let r = crate::presets::load_for_queue(&state, preset).await;
        let q = state.queue();
        let mut g = q.inner.lock().expect("file");
        g.swapping = None;
        if let Err(e) = r {
            tracing::warn!("file : chargement du preset {preset} impossible : {e}");
            let upto = g.next_ticket;
            g.failed.push((preset, upto, e, Instant::now()));
        }
        drop(g);
        q.notify.notify_waiters();
    });
}

/// Priorité d'un tour selon son origine.
pub fn priority_of(origin: Option<&str>) -> u8 {
    match origin {
        Some("user") | Some("verify") => USER,
        _ => BACKGROUND,
    }
}

/// Bail d'une session, tenu tant qu'elle est occupée (tour, vérification,
/// corrections) ; rendu pendant qu'elle attend une réponse de Michael
/// (approbation), repris ensuite. Sans effet si la session en tient déjà un.
pub async fn hold_session(
    state: &AppState,
    hub: &Arc<bridge::Hub>,
    preset: Option<i64>,
    label: String,
    on_wait: &(dyn Fn(Option<Value>) + Send + Sync),
    cancel: impl std::future::Future<Output = ()>,
) -> Result<(), String> {
    let who = Who::Session(hub.id.clone());
    if state.queue().holds(&who) {
        return Ok(());
    }
    let ask = Ask {
        who,
        priority: priority_of(hub.turn_origin()),
        preset,
        label,
    };
    let lease = state.queue().acquire(state, ask.clone(), on_wait, cancel).await?;
    let (state, hub) = (state.clone(), hub.clone());
    tokio::spawn(async move {
        let mut lease = Some(lease);
        let mut idle = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if !hub.busy() {
                idle += 1;
                if idle >= 2 {
                    return;
                }
                continue;
            }
            idle = 0;
            if hub.waiting_on_user() {
                // Le GPU ne sert à rien pendant qu'on attend Michael.
                lease = None;
            } else if lease.is_none() {
                let mut a = ask.clone();
                a.priority = USER;
                a.preset = None;
                match state.queue().acquire(&state, a, &|_| {}, std::future::pending()).await {
                    Ok(l) => lease = Some(l),
                    Err(_) => return,
                }
            }
        }
    });
    Ok(())
}

/// `GET /api/queue` : baux tenus et file d'attente.
pub async fn api(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Value> {
    axum::Json(state.queue().snapshot(&state))
}
