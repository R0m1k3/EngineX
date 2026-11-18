//! Requête Responses telle qu'EngineX la lit. Les éléments de `input` restent
//! des `Value` lus selon leur champ `type` (un champ ajouté par Codex ne doit
//! pas faire refuser la requête).

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesRequest {
    #[serde(default)]
    pub input: Input,
    pub instructions: Option<String>,
    #[serde(default)]
    pub tools: Vec<Value>,
    pub tool_choice: Option<Value>,
    pub parallel_tool_calls: Option<bool>,
    pub max_output_tokens: Option<u64>,
    pub text: Option<Value>,
    pub previous_response_id: Option<String>,
    pub stream: Option<bool>,
    /// Champs ignorés (`store`, `include`, `reasoning`, `model`…).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Input {
    Text(String),
    Items(Vec<Value>),
}

impl Default for Input {
    fn default() -> Self {
        Input::Items(Vec::new())
    }
}
