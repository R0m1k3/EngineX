//! Requête Responses → requête `/v1/chat/completions` de `llama-server`.
//!
//! Règles (plan de l'étape 3, §2) :
//! - `developer`/`system` du début fusionnés en un `system` ; plus loin, ils
//!   deviennent des `user` préfixés `[Consigne système]` (gabarits stricts) ;
//! - `user` successifs fusionnés ;
//! - `reasoning` → `reasoning_content` du message `assistant` qui suit ;
//! - `function_call` → `tool_calls` du message `assistant` en cours ;
//! - `function_call_output` → message `tool` (images dans un `user` après) ;
//! - outils `namespace` aplatis (`names.rs`) ; le reste ignoré et compté.

use std::collections::BTreeMap;

use enginex_hub::sampling::Sampling;
use serde_json::{Map, Value, json};

use crate::names::NameMap;
use crate::sampling;
use crate::types::{Input, ResponsesRequest};

pub struct Ctx {
    pub model_name: String,
    pub sampling: Sampling,
    pub vision: bool,
}

#[derive(Debug)]
pub struct Translated {
    pub body: Value,
    pub names: NameMap,
    /// Éléments et outils ignorés (`item:web_search_call`, `tool:web_search`).
    pub dropped: BTreeMap<String, usize>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TranslateError {
    #[error("seul le mode streamé est pris en charge (stream: true)")]
    StreamRequired,
    #[error("previous_response_id n'est pas pris en charge : envoyer l'historique complet")]
    PreviousResponse,
    #[error("Le modèle actif ne lit pas les images")]
    NoVision,
    #[error("{0}")]
    Invalid(String),
}

impl TranslateError {
    pub fn code(&self) -> &'static str {
        match self {
            TranslateError::StreamRequired => "stream_required",
            TranslateError::PreviousResponse => "previous_response_not_supported",
            TranslateError::NoVision => "no_vision",
            TranslateError::Invalid(_) => "invalid_request_error",
        }
    }
}

const SYSTEM_PREFIX: &str = "[Consigne système]\n";

/// Parties d'un message `user` : texte ou image.
#[derive(Clone)]
enum Part {
    Text(String),
    Image(String),
}

struct Builder<'a> {
    ctx: &'a Ctx,
    names: NameMap,
    dropped: BTreeMap<String, usize>,
    head: Vec<String>,
    in_head: bool,
    msgs: Vec<Msg>,
    reasoning: Option<String>,
    tool_images: Vec<Part>,
}

enum Msg {
    User(Vec<Part>),
    Assistant {
        content: Option<String>,
        reasoning: Option<String>,
        calls: Vec<Value>,
    },
    Tool {
        call_id: String,
        content: String,
    },
}

fn str_field<'v>(v: &'v Value, k: &str) -> Option<&'v str> {
    v.get(k).and_then(Value::as_str)
}

fn image_url(part: &Value) -> Option<String> {
    match part.get("image_url") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(o)) => o.get("url").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

impl<'a> Builder<'a> {
    fn new(ctx: &'a Ctx) -> Self {
        Builder {
            ctx,
            names: NameMap::new(),
            dropped: BTreeMap::new(),
            head: Vec::new(),
            in_head: true,
            msgs: Vec::new(),
            reasoning: None,
            tool_images: Vec::new(),
        }
    }

    fn drop(&mut self, what: String) {
        *self.dropped.entry(what).or_insert(0) += 1;
    }

    /// Parties d'un contenu (chaîne ou liste de parties).
    fn parts(&mut self, content: Option<&Value>) -> Result<Vec<Part>, TranslateError> {
        let mut out = Vec::new();
        match content {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => out.push(Part::Text(s.clone())),
            Some(Value::Array(items)) => {
                for p in items {
                    match str_field(p, "type").unwrap_or("") {
                        "input_text" | "output_text" | "text" | "refusal" => {
                            let t = str_field(p, "text").or_else(|| str_field(p, "refusal")).unwrap_or("");
                            out.push(Part::Text(t.to_string()));
                        }
                        "input_image" => {
                            if !self.ctx.vision {
                                return Err(TranslateError::NoVision);
                            }
                            match image_url(p) {
                                Some(url) => out.push(Part::Image(url)),
                                None => self.drop("part:input_image(file_id)".into()),
                            }
                        }
                        other => self.drop(format!("part:{other}")),
                    }
                }
            }
            Some(_) => return Err(TranslateError::Invalid("content : chaîne ou liste attendue".into())),
        }
        Ok(out)
    }

    fn text_of(parts: &[Part]) -> String {
        parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                Part::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn push_user(&mut self, parts: Vec<Part>) {
        if parts.is_empty() {
            return;
        }
        if let Some(Msg::User(prev)) = self.msgs.last_mut() {
            prev.extend(parts);
        } else {
            self.msgs.push(Msg::User(parts));
        }
    }

    fn flush_tool_images(&mut self) {
        if !self.tool_images.is_empty() {
            let imgs = std::mem::take(&mut self.tool_images);
            self.push_user(imgs);
        }
    }

    fn item(&mut self, item: &Value) -> Result<(), TranslateError> {
        let kind = match str_field(item, "type") {
            Some(t) => t,
            None if item.get("role").is_some() => "message",
            None => return Err(TranslateError::Invalid("élément d'input sans type".into())),
        };
        if kind != "function_call_output" {
            self.flush_tool_images();
        }
        match kind {
            "message" => self.message(item)?,
            "reasoning" => {
                self.in_head = false;
                let from = |field: &str| -> Option<String> {
                    let parts = item.get(field)?.as_array()?;
                    let texts: Vec<&str> = parts.iter().filter_map(|p| str_field(p, "text")).collect();
                    (!texts.is_empty()).then(|| texts.join("\n"))
                };
                if let Some(text) = from("content").or_else(|| from("summary")) {
                    self.reasoning = Some(match self.reasoning.take() {
                        Some(prev) => format!("{prev}\n{text}"),
                        None => text,
                    });
                }
            }
            "function_call" => {
                self.in_head = false;
                let name = str_field(item, "name").unwrap_or("");
                let flat = self.names.insert(str_field(item, "namespace"), name);
                let call = json!({
                    "id": str_field(item, "call_id").unwrap_or(""),
                    "type": "function",
                    "function": {"name": flat, "arguments": str_field(item, "arguments").unwrap_or("")},
                });
                let pending = self.reasoning.take();
                match self.msgs.last_mut() {
                    Some(Msg::Assistant { calls, reasoning, .. }) => {
                        calls.push(call);
                        if let Some(r) = pending {
                            *reasoning = Some(match reasoning.take() {
                                Some(prev) => format!("{prev}\n{r}"),
                                None => r,
                            });
                        }
                    }
                    _ => self.msgs.push(Msg::Assistant {
                        content: None,
                        reasoning: pending,
                        calls: vec![call],
                    }),
                }
            }
            "function_call_output" => {
                self.in_head = false;
                let call_id = str_field(item, "call_id").unwrap_or("").to_string();
                let (text, images) = match item.get("output") {
                    Some(Value::String(s)) => (s.clone(), Vec::new()),
                    Some(Value::Array(_)) => {
                        let parts = self.parts(item.get("output"))?;
                        let imgs = parts.iter().filter(|p| matches!(p, Part::Image(_))).cloned().collect();
                        (Self::text_of(&parts), imgs)
                    }
                    Some(Value::Object(o)) => (
                        o.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
                        Vec::new(),
                    ),
                    _ => (String::new(), Vec::new()),
                };
                self.msgs.push(Msg::Tool { call_id, content: text });
                self.tool_images.extend(images);
            }
            other => self.drop(format!("item:{other}")),
        }
        Ok(())
    }

    fn message(&mut self, item: &Value) -> Result<(), TranslateError> {
        let role = str_field(item, "role").unwrap_or("user");
        let parts = self.parts(item.get("content"))?;
        match role {
            "developer" | "system" => {
                let text = Self::text_of(&parts);
                if self.in_head {
                    self.head.push(text);
                } else {
                    self.push_user(vec![Part::Text(format!("{SYSTEM_PREFIX}{text}"))]);
                }
            }
            "assistant" => {
                self.in_head = false;
                let reasoning = self.reasoning.take();
                self.msgs.push(Msg::Assistant {
                    content: Some(Self::text_of(&parts)),
                    reasoning,
                    calls: Vec::new(),
                });
            }
            _ => {
                self.in_head = false;
                // Un raisonnement non suivi d'une réponse est perdu.
                self.reasoning = None;
                self.push_user(parts);
            }
        }
        Ok(())
    }

    fn messages(&mut self) -> Vec<Value> {
        self.flush_tool_images();
        let mut out = Vec::with_capacity(self.msgs.len() + 1);
        if !self.head.is_empty() {
            out.push(json!({"role": "system", "content": self.head.join("\n\n")}));
        }
        for m in &self.msgs {
            out.push(match m {
                Msg::User(parts) => {
                    if parts.iter().all(|p| matches!(p, Part::Text(_))) {
                        json!({"role": "user", "content": Self::text_of(parts)})
                    } else {
                        let content: Vec<Value> = parts
                            .iter()
                            .map(|p| match p {
                                Part::Text(t) => json!({"type": "text", "text": t}),
                                Part::Image(u) => json!({"type": "image_url", "image_url": {"url": u}}),
                            })
                            .collect();
                        json!({"role": "user", "content": content})
                    }
                }
                Msg::Assistant {
                    content,
                    reasoning,
                    calls,
                } => {
                    let mut o = Map::new();
                    o.insert("role".into(), json!("assistant"));
                    o.insert(
                        "content".into(),
                        content.clone().map(Value::String).unwrap_or(Value::Null),
                    );
                    if let Some(r) = reasoning {
                        o.insert("reasoning_content".into(), json!(r));
                    }
                    if !calls.is_empty() {
                        o.insert("tool_calls".into(), Value::Array(calls.clone()));
                    }
                    Value::Object(o)
                }
                Msg::Tool { call_id, content } => {
                    json!({"role": "tool", "tool_call_id": call_id, "content": content})
                }
            });
        }
        out
    }

    fn tool(&mut self, flat: String, t: &Value) -> Value {
        let mut f = Map::new();
        f.insert("name".into(), json!(flat));
        if let Some(d) = t.get("description").filter(|d| !d.is_null()) {
            f.insert("description".into(), d.clone());
        }
        f.insert(
            "parameters".into(),
            t.get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
        );
        json!({"type": "function", "function": f})
    }

    fn tools(&mut self, tools: &[Value]) -> Vec<Value> {
        let mut out = Vec::new();
        for t in tools {
            match str_field(t, "type").unwrap_or("") {
                "function" => {
                    let flat = self.names.insert(None, str_field(t, "name").unwrap_or(""));
                    out.push(self.tool(flat, t));
                }
                "namespace" => {
                    let ns = str_field(t, "name").unwrap_or("").to_string();
                    for sub in t
                        .get("tools")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                    {
                        if str_field(sub, "type") == Some("function") {
                            let flat = self.names.insert(Some(&ns), str_field(sub, "name").unwrap_or(""));
                            out.push(self.tool(flat, sub));
                        } else {
                            self.drop(format!("tool:{}", str_field(sub, "type").unwrap_or("?")));
                        }
                    }
                }
                other => self.drop(format!("tool:{other}")),
            }
        }
        out
    }
}

fn tool_choice(choice: &Value, tools: &[Value], names: &NameMap) -> Value {
    match choice {
        Value::String(s) if matches!(s.as_str(), "auto" | "none" | "required") => choice.clone(),
        Value::Object(o) => {
            let name = o.get("name").and_then(Value::as_str);
            let ns = o.get("namespace").and_then(Value::as_str);
            let exists = name.is_some_and(|n| {
                tools.iter().any(|t| {
                    let flat = t.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
                    names.split(flat) == (ns.map(str::to_string), n.to_string())
                })
            });
            json!(if exists { "required" } else { "auto" })
        }
        _ => json!("auto"),
    }
}

/// Traduit une requête Responses en corps chat/completions.
pub fn to_chat(req: &ResponsesRequest, ctx: &Ctx) -> Result<Translated, TranslateError> {
    if req.stream != Some(true) {
        return Err(TranslateError::StreamRequired);
    }
    if req.previous_response_id.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(TranslateError::PreviousResponse);
    }
    let mut b = Builder::new(ctx);
    let tools = b.tools(&req.tools);
    if let Some(i) = req.instructions.as_deref().filter(|s| !s.is_empty()) {
        b.head.push(i.to_string());
    }
    match &req.input {
        Input::Text(t) => {
            b.in_head = false;
            b.push_user(vec![Part::Text(t.clone())]);
        }
        Input::Items(items) => {
            for it in items {
                b.item(it)?;
            }
        }
    }
    let messages = b.messages();
    let mut body = Map::new();
    body.insert("model".into(), json!(ctx.model_name));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({"include_usage": true}));
    if !tools.is_empty() {
        if let Some(c) = &req.tool_choice {
            body.insert("tool_choice".into(), tool_choice(c, &tools, &b.names));
        }
        if let Some(p) = req.parallel_tool_calls {
            body.insert("parallel_tool_calls".into(), json!(p));
        }
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(n) = req.max_output_tokens {
        body.insert("max_tokens".into(), json!(n));
    }
    if let Some(fmt) = req.text.as_ref().and_then(|t| t.get("format")) {
        match str_field(fmt, "type") {
            Some("json_schema") => {
                let mut js = Map::new();
                js.insert(
                    "name".into(),
                    fmt.get("name").cloned().unwrap_or_else(|| json!("output")),
                );
                js.insert("schema".into(), fmt.get("schema").cloned().unwrap_or_else(|| json!({})));
                if let Some(s) = fmt.get("strict") {
                    js.insert("strict".into(), s.clone());
                }
                body.insert(
                    "response_format".into(),
                    json!({"type": "json_schema", "json_schema": js}),
                );
            }
            Some("json_object") => {
                body.insert("response_format".into(), json!({"type": "json_object"}));
            }
            _ => {}
        }
    }
    // Échantillonnage du client (le preset l'emporte ensuite).
    for k in ["temperature", "top_p"] {
        if let Some(v) = req.extra.get(k).filter(|v| v.is_number()) {
            body.insert(k.into(), v.clone());
        }
    }
    let mut body = Value::Object(body);
    sampling::inject(&mut body, &ctx.sampling);
    Ok(Translated {
        body,
        names: b.names,
        dropped: b.dropped,
    })
}

/// Lit et traduit un corps brut (erreur `invalid_request_error` si illisible).
pub fn parse_and_translate(raw: &[u8], ctx: &Ctx) -> Result<Translated, TranslateError> {
    let req: ResponsesRequest =
        serde_json::from_slice(raw).map_err(|e| TranslateError::Invalid(format!("JSON illisible : {e}")))?;
    to_chat(&req, ctx)
}
