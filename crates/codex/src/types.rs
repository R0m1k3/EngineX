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
    "command/exec/outputDelta",
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

pub fn thread_start(cwd: &str, approval: &str, developer_instructions: Option<&str>) -> Value {
    let mut v = json!({"cwd": cwd, "sandbox": "danger-full-access", "approvalPolicy": approval});
    if let Some(d) = developer_instructions {
        v["developerInstructions"] = json!(d);
    }
    v
}

/// `thread/revert` : retire le tour `before_turn_id` et les suivants.
pub fn thread_revert(thread_id: &str, before_turn_id: &str) -> Value {
    json!({"threadId": thread_id, "beforeTurnId": before_turn_id})
}

/// `command/exec` : `process_id` + `tty` pour un terminal (sortie en
/// `command/exec/outputDelta`), sinon mode tamponné (`stdout`/`stderr` dans
/// la réponse). Sans politique de Codex : la sandbox Docker est la frontière.
pub fn command_exec(
    command: &[&str],
    cwd: &str,
    process_id: Option<&str>,
    tty: Option<(u16, u16)>,
    timeout_ms: Option<u64>,
) -> Value {
    let mut v = json!({
        "command": command,
        "cwd": cwd,
        "sandboxPolicy": {"type": "dangerFullAccess"},
    });
    if let Some(p) = process_id {
        v["processId"] = json!(p);
    }
    if let Some((rows, cols)) = tty {
        v["tty"] = json!(true);
        v["size"] = json!({"rows": rows, "cols": cols});
    }
    match timeout_ms {
        Some(ms) => v["timeoutMs"] = json!(ms),
        None => v["disableTimeout"] = json!(true),
    }
    v
}

pub fn exec_write(process_id: &str, data_base64: &str) -> Value {
    json!({"processId": process_id, "deltaBase64": data_base64})
}

pub fn exec_resize(process_id: &str, rows: u16, cols: u16) -> Value {
    json!({"processId": process_id, "size": {"rows": rows, "cols": cols}})
}

pub fn exec_terminate(process_id: &str) -> Value {
    json!({"processId": process_id})
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
