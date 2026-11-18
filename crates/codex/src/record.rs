//! Enregistrement des échanges avec l'app-server, au format des scripts du
//! faux app-server (`fake-llama-server app-server`, une étape JSON par ligne) :
//! - requête envoyée → `{"expect": méthode, "result": réponse}` ;
//! - notification envoyée → `{"expect": méthode}` ;
//! - message reçu du serveur → `{"send": message}` (notifications hors liste
//!   blanche omises) ;
//! - réponse à une requête du serveur → `{"expect_reply": "$req:N"}` ;
//! - `launch()` → `{"launch": true}` (nouveau processus).
//!
//! À l'écriture, les ids vus sont remplacés par des repères dans l'ordre
//! d'apparition (`$thread`, `$turn:N`, `$item:N`, `$req:N`), les horodatages
//! par `$now`, le dossier de travail par `$cwd` et le CODEX_HOME par `$home`.

use std::{collections::HashMap, sync::Mutex};

use serde_json::{Map, Value, json};

pub struct Recorder {
    whitelist: Vec<String>,
    steps: Mutex<Vec<Step>>,
}

#[derive(Clone, Debug)]
enum Step {
    Expect {
        id: Option<i64>,
        method: String,
        result: Option<Value>,
    },
    Send(Value),
    Reply(Value),
    Launch,
}

impl Recorder {
    pub fn new(whitelist: &[&str]) -> Self {
        Self {
            whitelist: whitelist.iter().map(|s| s.to_string()).collect(),
            steps: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn request_sent(&self, id: i64, method: &str) {
        self.steps.lock().expect("verrou").push(Step::Expect {
            id: Some(id),
            method: method.into(),
            result: None,
        });
    }

    pub(crate) fn response(&self, id: i64, result: &Value) {
        let mut s = self.steps.lock().expect("verrou");
        for st in s.iter_mut().rev() {
            if let Step::Expect {
                id: Some(i), result: r, ..
            } = st
                && *i == id
            {
                *r = Some(result.clone());
                return;
            }
        }
    }

    pub(crate) fn notification_sent(&self, method: &str) {
        self.steps.lock().expect("verrou").push(Step::Expect {
            id: None,
            method: method.into(),
            result: None,
        });
    }

    pub(crate) fn server_message(&self, v: &Value) {
        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
        let is_request = v.get("id").is_some_and(|i| !i.is_null());
        if !is_request && !self.whitelist.iter().any(|w| w == method) {
            return;
        }
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("jsonrpc");
        }
        self.steps.lock().expect("verrou").push(Step::Send(v));
    }

    pub(crate) fn reply(&self, id: &Value) {
        self.steps.lock().expect("verrou").push(Step::Reply(id.clone()));
    }

    /// Sépare deux lancements de l'app-server.
    pub fn launch(&self) {
        self.steps.lock().expect("verrou").push(Step::Launch);
    }

    /// Étapes en JSON brut (ids réels), avant remplacement.
    fn raw(&self) -> Vec<Value> {
        self.steps
            .lock()
            .expect("verrou")
            .iter()
            .map(|s| match s {
                Step::Expect {
                    id: Some(_),
                    method,
                    result,
                } => {
                    json!({"expect": method, "result": result.clone().unwrap_or(Value::Null)})
                }
                Step::Expect { id: None, method, .. } => json!({"expect": method}),
                Step::Send(v) => json!({"send": v}),
                Step::Reply(id) => json!({"expect_reply": id}),
                Step::Launch => json!({"launch": true}),
            })
            .collect()
    }

    /// Script JSONL avec les repères.
    pub fn to_jsonl(&self, cwd: &str, home: &str) -> String {
        let mut steps = self.raw();
        let mut ids = Ids::default();
        for s in &steps {
            ids.collect(s, None);
        }
        let mut out = String::new();
        for s in &mut steps {
            ids.replace(s, None, cwd, home);
            out.push_str(&s.to_string());
            out.push('\n');
        }
        out
    }

    /// Suite `(type d'étape, méthode)` : sert au contrôle de dérive.
    pub fn shape(&self) -> Vec<(String, String)> {
        shape_of(&self.raw())
    }
}

/// Suite `(type d'étape, méthode)` d'un script (lignes JSON).
pub fn shape_of(steps: &[Value]) -> Vec<(String, String)> {
    steps
        .iter()
        .filter_map(|s| {
            if let Some(m) = s.get("expect").and_then(Value::as_str) {
                Some(("expect".into(), m.into()))
            } else if let Some(v) = s.get("send") {
                let kind = if v.get("id").is_some() { "request" } else { "send" };
                Some((
                    kind.into(),
                    v.get("method").and_then(Value::as_str).unwrap_or("").into(),
                ))
            } else if s.get("expect_reply").is_some() {
                Some(("expect_reply".into(), String::new()))
            } else if s.get("launch").is_some() {
                Some(("launch".into(), String::new()))
            } else {
                None
            }
        })
        .collect()
}

#[derive(Default)]
struct Ids {
    map: HashMap<String, String>,
    threads: usize,
    turns: usize,
    items: usize,
    reqs: HashMap<String, String>,
}

impl Ids {
    fn add(&mut self, kind: &str, id: &str) {
        if id.is_empty() || self.map.contains_key(id) {
            return;
        }
        let marker = match kind {
            "thread" => {
                self.threads += 1;
                if self.threads == 1 {
                    "$thread".to_string()
                } else {
                    format!("$thread:{}", self.threads)
                }
            }
            "turn" => {
                self.turns += 1;
                format!("$turn:{}", self.turns)
            }
            _ => {
                self.items += 1;
                format!("$item:{}", self.items)
            }
        };
        self.map.insert(id.to_string(), marker);
    }

    fn add_req(&mut self, id: &Value) {
        let k = id.to_string();
        if !self.reqs.contains_key(&k) {
            let n = self.reqs.len() + 1;
            self.reqs.insert(k, format!("$req:{n}"));
        }
    }

    /// Repère les ids d'après leur clé (`threadId`, `thread.id`, `turn.id`,
    /// éléments de `items`…).
    fn collect(&mut self, v: &Value, parent: Option<&str>) {
        match v {
            Value::Object(o) => {
                if parent == Some("send")
                    && let Some(id) = o.get("id").filter(|i| !i.is_null())
                {
                    self.add_req(id);
                }
                if let Some(id) = o.get("id").and_then(Value::as_str) {
                    match parent {
                        Some("thread") => self.add("thread", id),
                        Some("turn") => self.add("turn", id),
                        Some("item") | Some("items") => self.add("item", id),
                        Some("data") if o.contains_key("items") => self.add("turn", id),
                        _ => {}
                    }
                }
                for (k, x) in o {
                    match (k.as_str(), x) {
                        ("threadId", Value::String(s)) => self.add("thread", s),
                        ("turnId", Value::String(s)) => self.add("turn", s),
                        ("itemId", Value::String(s)) => self.add("item", s),
                        _ => {}
                    }
                    self.collect(x, Some(k));
                }
            }
            Value::Array(a) => {
                for x in a {
                    self.collect(x, parent);
                }
            }
            _ => {}
        }
    }

    fn replace(&self, v: &mut Value, key: Option<&str>, cwd: &str, home: &str) {
        match v {
            Value::Object(o) => {
                let keys: Vec<String> = o.keys().cloned().collect();
                for k in keys {
                    let x = o.get_mut(&k).expect("clé");
                    if (k == "requestId" || k == "expect_reply" || (k == "id" && key == Some("send")))
                        && let Some(m) = self.reqs.get(&x.to_string())
                    {
                        *x = Value::String(m.clone());
                        continue;
                    }
                    if is_time_key(&k) && x.is_number() {
                        *x = Value::String("$now".into());
                        continue;
                    }
                    self.replace(x, Some(&k), cwd, home);
                }
            }
            Value::Array(a) => {
                for x in a {
                    self.replace(x, key, cwd, home);
                }
            }
            Value::String(s) => {
                if let Some(m) = self.map.get(s.as_str()) {
                    *s = m.clone();
                } else {
                    let mut t = s.clone();
                    if !home.is_empty() {
                        t = t.replace(home, "$home");
                    }
                    if !cwd.is_empty() {
                        t = t.replace(cwd, "$cwd");
                    }
                    *s = t;
                }
            }
            _ => {}
        }
    }
}

fn is_time_key(k: &str) -> bool {
    k.ends_with("At") || k.ends_with("AtMs") || k == "timestamp" || k.ends_with("Ms") && k.starts_with("started")
}

/// Remplace les repères d'une valeur de script par des valeurs concrètes
/// (utilisé par le faux app-server).
pub fn substitute(v: &Value, cwd: &str, home: &str, now_ms: i64) -> Value {
    match v {
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, x) in o {
                m.insert(k.clone(), substitute(x, cwd, home, now_ms));
            }
            Value::Object(m)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, cwd, home, now_ms)).collect()),
        Value::String(s) => marker(s, cwd, home, now_ms),
        x => x.clone(),
    }
}

fn marker(s: &str, cwd: &str, home: &str, now_ms: i64) -> Value {
    if s == "$now" {
        return json!(now_ms);
    }
    if s == "$thread" {
        return json!("thread-1");
    }
    let num = |p: &str| s.strip_prefix(p).and_then(|n| n.parse::<u64>().ok());
    if let Some(n) = num("$thread:") {
        return json!(format!("thread-{n}"));
    }
    if let Some(n) = num("$turn:") {
        return json!(format!("turn-{n}"));
    }
    if let Some(n) = num("$item:") {
        return json!(format!("item-{n}"));
    }
    if let Some(n) = num("$req:") {
        return json!(n);
    }
    json!(s.replace("$cwd", cwd).replace("$home", home))
}
