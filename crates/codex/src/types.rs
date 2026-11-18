//! Champs du protocole app-server lus ou écrits par EngineX (Codex 0.170).
//! Le reste reste en `serde_json::Value`, ce qui tolère les champs ajoutés.

use serde_json::{Value, json};

/// Notifications relayées à l'UI (liste blanche du pont).
pub const WHITELIST: &[&str] = &[
    "turn/started",
    "turn/completed",
    "item/started",
    "item/completed",
    "item/agentMessage/delta",
    "item/reasoning/textDelta",
    "item/reasoning/summaryTextDelta",
    "item/commandExecution/outputDelta",
    "turn/plan/updated",
    "thread/tokenUsage/updated",
    "serverRequest/resolved",
    "error",
];

/// Requêtes du serveur que l'UI sait traiter.
pub const SERVER_REQUESTS: &[&str] = &[
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/tool/requestUserInput",
];

/// Mode d'approbation d'une session.
pub fn approval_policy(mode: &str) -> &'static str {
    match mode {
        "prudent" => "untrusted",
        _ => "never",
    }
}

pub fn thread_start(cwd: &str, approval: &str) -> Value {
    json!({"cwd": cwd, "sandbox": "danger-full-access", "approvalPolicy": approval})
}

pub fn thread_resume(thread_id: &str, cwd: &str, approval: &str) -> Value {
    json!({"threadId": thread_id, "cwd": cwd, "sandbox": "danger-full-access", "approvalPolicy": approval})
}

pub fn text_input(text: &str) -> Value {
    json!({"type": "text", "text": text, "text_elements": []})
}

pub fn local_image(path: &str) -> Value {
    json!({"type": "localImage", "path": path})
}

pub fn turn_start(thread_id: &str, input: Vec<Value>, approval: &str) -> Value {
    json!({"threadId": thread_id, "input": input, "approvalPolicy": approval})
}

/// `result.thread.id` de `thread/start` et `thread/resume`.
pub fn thread_id(result: &Value) -> Option<&str> {
    result.pointer("/thread/id").and_then(Value::as_str)
}

/// `result.turn.id` de `turn/start`.
pub fn turn_id(result: &Value) -> Option<&str> {
    result.pointer("/turn/id").and_then(Value::as_str)
}

/// Id du tour d'une notification (`turnId` ou `turn.id`).
pub fn notif_turn(params: &Value) -> Option<&str> {
    params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/turn/id").and_then(Value::as_str))
}

/// `thread/turns/list` : tous les tours avec leurs éléments, du plus ancien
/// au plus récent.
pub fn turns_list(thread_id: &str) -> Value {
    json!({"threadId": thread_id, "itemsView": "full", "sortDirection": "asc", "limit": 100})
}
