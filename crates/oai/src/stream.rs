//! Flux chat/completions de `llama-server` → événements Responses.
//!
//! Machine à états nourrie morceau par morceau. Chaque événement porte un
//! `sequence_number` croissant ; les éléments portent leur `output_index`.
//! Un seul élément texte (raisonnement ou message) est ouvert à la fois ; les
//! appels d'outils restent ouverts jusqu'à la fin du flux ou un autre contenu.

use std::collections::BTreeMap;

use rand::Rng;
use serde_json::{Value, json};

use crate::names::NameMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: &'static str,
    pub data: Value,
}

impl Event {
    pub fn to_sse(&self) -> String {
        format!("event: {}\ndata: {}\n\n", self.name, self.data)
    }
}

/// `<préfixe>_` + 24 caractères alphanumériques aléatoires.
pub fn random_id(prefix: &str) -> String {
    const ABC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::rng();
    let s: String = (0..24).map(|_| ABC[rng.random_range(0..ABC.len())] as char).collect();
    format!("{prefix}_{s}")
}

type Ids = Box<dyn FnMut(&str) -> String + Send>;

enum Open {
    Reasoning { id: String, index: usize, text: String },
    Message { id: String, index: usize, text: String },
}

struct Call {
    id: String,
    index: usize,
    call_id: String,
    name: String,
    args: String,
}

pub struct Translator {
    resp_id: String,
    model: String,
    names: NameMap,
    ids: Ids,
    created_at: u64,
    seq: u64,
    next_index: usize,
    /// Éléments terminés, rangés par `output_index`.
    output: BTreeMap<usize, Value>,
    open: Option<Open>,
    calls: BTreeMap<u64, Call>,
    usage: Option<Value>,
    /// `timings` de llama-server (débit, jetons de brouillon acceptés).
    timings: Option<Value>,
    finish_reason: Option<String>,
    closed: bool,
}

impl Translator {
    pub fn new(resp_id: String, model: String, names: NameMap, ids: Ids) -> Self {
        Translator {
            resp_id,
            model,
            names,
            ids,
            created_at: 0,
            seq: 0,
            next_index: 0,
            output: BTreeMap::new(),
            open: None,
            calls: BTreeMap::new(),
            usage: None,
            timings: None,
            finish_reason: None,
            closed: false,
        }
    }

    pub fn with_created_at(mut self, t: u64) -> Self {
        self.created_at = t;
        self
    }

    pub fn finish_reason(&self) -> Option<&str> {
        self.finish_reason.as_deref()
    }

    pub fn usage(&self) -> Option<&Value> {
        self.usage.as_ref()
    }

    pub fn timings(&self) -> Option<&Value> {
        self.timings.as_ref()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn ev(&mut self, name: &'static str, mut data: Value) -> Event {
        data["type"] = json!(name);
        data["sequence_number"] = json!(self.seq);
        self.seq += 1;
        Event { name, data }
    }

    fn response(&self, status: &str) -> Value {
        json!({
            "id": self.resp_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": self.output.values().cloned().collect::<Vec<_>>(),
        })
    }

    pub fn start(&mut self) -> Vec<Event> {
        let r = self.response("in_progress");
        let a = self.ev("response.created", json!({ "response": r.clone() }));
        let b = self.ev("response.in_progress", json!({ "response": r }));
        vec![a, b]
    }

    /// Événement de maintien (Codex ne compte pas les commentaires SSE).
    pub fn keepalive(&mut self) -> Event {
        let r = self.response("in_progress");
        self.ev("response.in_progress", json!({ "response": r }))
    }

    fn close_open(&mut self, out: &mut Vec<Event>) {
        match self.open.take() {
            Some(Open::Reasoning { id, index, text }) => {
                let e = self.ev(
                    "response.reasoning_text.done",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "text": text}),
                );
                out.push(e);
                let item = json!({"type": "reasoning", "id": id, "summary": [],
                    "content": [{"type": "reasoning_text", "text": text}]});
                out.push(self.ev(
                    "response.output_item.done",
                    json!({"output_index": index, "item": item.clone()}),
                ));
                self.output.insert(index, item);
            }
            Some(Open::Message { id, index, text }) => {
                let base = json!({"item_id": id, "output_index": index, "content_index": 0});
                let mut d = base.clone();
                d["text"] = json!(text);
                d["logprobs"] = json!([]);
                out.push(self.ev("response.output_text.done", d));
                let part = json!({"type": "output_text", "text": text, "annotations": []});
                let mut d = base;
                d["part"] = part.clone();
                out.push(self.ev("response.content_part.done", d));
                let item = json!({"type": "message", "id": id, "role": "assistant", "status": "completed",
                    "content": [part]});
                out.push(self.ev(
                    "response.output_item.done",
                    json!({"output_index": index, "item": item.clone()}),
                ));
                self.output.insert(index, item);
            }
            None => {}
        }
    }

    fn call_item(&self, c: &Call, status: &str) -> Value {
        let (ns, name) = self.names.split(&c.name);
        let mut item = json!({"type": "function_call", "id": c.id, "call_id": c.call_id, "name": name,
            "arguments": c.args, "status": status});
        if let Some(ns) = ns {
            item["namespace"] = json!(ns);
        }
        item
    }

    fn close_calls(&mut self, out: &mut Vec<Event>) {
        let calls = std::mem::take(&mut self.calls);
        for c in calls.into_values() {
            let e = self.ev(
                "response.function_call_arguments.done",
                json!({"item_id": c.id, "output_index": c.index, "arguments": c.args}),
            );
            out.push(e);
            let item = self.call_item(&c, "completed");
            out.push(self.ev(
                "response.output_item.done",
                json!({"output_index": c.index, "item": item.clone()}),
            ));
            self.output.insert(c.index, item);
        }
    }

    fn text_delta(&mut self, reasoning: bool, delta: &str, out: &mut Vec<Event>) {
        let same = matches!(
            (&self.open, reasoning),
            (Some(Open::Reasoning { .. }), true) | (Some(Open::Message { .. }), false)
        );
        if !same {
            self.close_open(out);
            self.close_calls(out);
            let index = self.next_index;
            self.next_index += 1;
            if reasoning {
                let id = (self.ids)("rs");
                let item = json!({"type": "reasoning", "id": id, "summary": [], "content": []});
                out.push(self.ev(
                    "response.output_item.added",
                    json!({"output_index": index, "item": item}),
                ));
                self.open = Some(Open::Reasoning {
                    id,
                    index,
                    text: String::new(),
                });
            } else {
                let id = (self.ids)("msg");
                let item = json!({"type": "message", "id": id, "role": "assistant", "status": "in_progress",
                    "content": []});
                out.push(self.ev(
                    "response.output_item.added",
                    json!({"output_index": index, "item": item}),
                ));
                let part = json!({"type": "output_text", "text": "", "annotations": []});
                out.push(self.ev(
                    "response.content_part.added",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "part": part}),
                ));
                self.open = Some(Open::Message {
                    id,
                    index,
                    text: String::new(),
                });
            }
        }
        let (id, index) = match self.open.as_mut() {
            Some(Open::Reasoning { id, index, text }) | Some(Open::Message { id, index, text }) => {
                text.push_str(delta);
                (id.clone(), *index)
            }
            None => unreachable!("élément ouvert juste au-dessus"),
        };
        let name = if reasoning {
            "response.reasoning_text.delta"
        } else {
            "response.output_text.delta"
        };
        let mut d = json!({"item_id": id, "output_index": index, "content_index": 0, "delta": delta});
        if !reasoning {
            d["logprobs"] = json!([]);
        }
        out.push(self.ev(name, d));
    }

    fn tool_delta(&mut self, tc: &Value, out: &mut Vec<Event>) {
        let key = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
        let f = tc.get("function");
        let name_piece = f.and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or("");
        let args_piece = f.and_then(|f| f.get("arguments")).and_then(Value::as_str).unwrap_or("");
        if !self.calls.contains_key(&key) {
            self.close_open(out);
            let index = self.next_index;
            self.next_index += 1;
            let id = (self.ids)("fc");
            let call_id = match tc.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                Some(s) => s.to_string(),
                None => (self.ids)("call"),
            };
            let c = Call {
                id,
                index,
                call_id,
                name: name_piece.to_string(),
                args: String::new(),
            };
            let mut item = self.call_item(&c, "in_progress");
            item["arguments"] = json!("");
            out.push(self.ev(
                "response.output_item.added",
                json!({"output_index": index, "item": item}),
            ));
            self.calls.insert(key, c);
        } else if !name_piece.is_empty()
            && let Some(c) = self.calls.get_mut(&key)
        {
            c.name.push_str(name_piece);
        }
        if !args_piece.is_empty() {
            let (id, index) = {
                let c = self.calls.get_mut(&key).expect("appel ouvert");
                c.args.push_str(args_piece);
                (c.id.clone(), c.index)
            };
            out.push(self.ev(
                "response.function_call_arguments.delta",
                json!({"item_id": id, "output_index": index, "delta": args_piece}),
            ));
        }
    }

    /// Un morceau `data:` (déjà décodé) du flux chat/completions.
    pub fn push_chunk(&mut self, chunk: &Value) -> Vec<Event> {
        let mut out = Vec::new();
        if self.closed {
            return out;
        }
        if let Some(err) = chunk.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            return self.fail("server_error", &msg);
        }
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(u.clone());
        }
        if let Some(t) = chunk.get("timings").filter(|t| t.is_object()) {
            self.timings = Some(t.clone());
        }
        let Some(choice) = chunk.get("choices").and_then(Value::as_array).and_then(|c| c.first()) else {
            return out;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(r) = delta
                .get("reasoning_content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                self.text_delta(true, r, &mut out);
            }
            if let Some(c) = delta.get("content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                self.text_delta(false, c, &mut out);
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc in tcs {
                    self.tool_delta(tc, &mut out);
                }
            }
        }
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(f.to_string());
        }
        out
    }

    fn usage_json(&self) -> Value {
        let u = self.usage.clone().unwrap_or(Value::Null);
        let n = |p: &str| u.pointer(p).and_then(Value::as_u64).unwrap_or(0);
        let input = n("/prompt_tokens");
        let output = n("/completion_tokens");
        let total = u.get("total_tokens").and_then(Value::as_u64).unwrap_or(input + output);
        json!({
            "input_tokens": input,
            "input_tokens_details": {"cached_tokens": n("/prompt_tokens_details/cached_tokens")},
            "output_tokens": output,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": total,
        })
    }

    /// Retire les appels dont les arguments ne sont pas un objet JSON complet.
    fn take_truncated_calls(&mut self) -> Vec<Call> {
        let keys: Vec<u64> = self
            .calls
            .iter()
            .filter(|(_, c)| !args_complete(&c.args))
            .map(|(k, _)| *k)
            .collect();
        keys.into_iter().filter_map(|k| self.calls.remove(&k)).collect()
    }

    fn truncated_notice(&self, calls: &[Call]) -> String {
        let limit = self
            .usage
            .as_ref()
            .and_then(|u| u.get("completion_tokens"))
            .and_then(Value::as_u64)
            .map(|n| format!(" ({n} jetons)"))
            .unwrap_or_default();
        let what: Vec<String> = calls
            .iter()
            .map(|c| {
                format!(
                    "`{}` ({} caractères d'arguments)",
                    self.names.split(&c.name).1,
                    c.args.chars().count()
                )
            })
            .collect();
        format!(
            "[EngineX] Réponse coupée à la limite de sortie{limit} au milieu de l'appel {} : \
             il n'a pas été exécuté. Découpe le programme en plusieurs fichiers de 400 lignes au plus, \
             chacun écrit en entier en une commande, puis reprends la tâche.",
            what.join(", ")
        )
    }

    /// Fin du flux de `llama-server` ; `done_seen` = `[DONE]` reçu.
    pub fn finish(&mut self, done_seen: bool) -> Vec<Event> {
        if self.closed {
            return Vec::new();
        }
        if !done_seen {
            return self.fail("server_error", "le flux de llama-server s'est arrêté avant la fin");
        }
        let mut out = Vec::new();
        self.close_open(&mut out);
        let cut = self.finish_reason.as_deref() == Some("length");
        // Un appel coupé par la limite a des arguments illisibles : transmis,
        // il resterait dans l'historique de Codex et llama.cpp refuserait
        // ensuite chaque requête du fil (erreur 500). On le remplace par un
        // message qui explique au modèle comment s'y prendre, et la réponse se
        // termine normalement.
        let truncated = if cut { self.take_truncated_calls() } else { Vec::new() };
        if !truncated.is_empty() {
            let notice = self.truncated_notice(&truncated);
            self.text_delta(false, &notice, &mut out);
            self.close_open(&mut out);
        }
        self.close_calls(&mut out);
        self.closed = true;
        let usage = self.usage_json();
        if cut && truncated.is_empty() {
            let mut r = self.response("incomplete");
            r["incomplete_details"] = json!({"reason": "max_output_tokens"});
            r["usage"] = usage;
            out.push(self.ev("response.incomplete", json!({ "response": r })));
        } else {
            let mut r = self.response("completed");
            r["usage"] = usage;
            out.push(self.ev("response.completed", json!({ "response": r })));
        }
        out
    }

    pub fn fail(&mut self, code: &str, message: &str) -> Vec<Event> {
        if self.closed {
            return Vec::new();
        }
        self.closed = true;
        let mut r = self.response("failed");
        r["error"] = json!({"code": code, "message": message});
        vec![self.ev("response.failed", json!({ "response": r }))]
    }
}

/// Arguments d'appel utilisables : un objet JSON complet.
pub fn args_complete(args: &str) -> bool {
    matches!(serde_json::from_str::<Value>(args), Ok(Value::Object(_)))
}
