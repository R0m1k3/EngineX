mod common;

use common::{counter_ids, fixture_path, golden};
use enginex_oai::{NameMap, Translator};
use serde_json::{Value, json};

fn names() -> NameMap {
    let mut n = NameMap::new();
    n.insert(None, "exec_command");
    n.insert(Some("mcp__test"), "echo");
    n
}

fn run(file: &str) -> Vec<Value> {
    let mut t = Translator::new("resp_1".into(), "qwen".into(), names(), counter_ids());
    let mut events = t.start();
    let raw = std::fs::read_to_string(fixture_path(file)).unwrap();
    let mut done = false;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        if line.trim() == "[DONE]" {
            done = true;
            break;
        }
        events.extend(t.push_chunk(&serde_json::from_str(line).unwrap()));
    }
    events.extend(t.finish(done));
    let seqs: Vec<u64> = events
        .iter()
        .map(|e| e.data["sequence_number"].as_u64().unwrap())
        .collect();
    assert_eq!(
        seqs,
        (0..events.len() as u64).collect::<Vec<_>>(),
        "sequence_number croissant"
    );
    for e in &events {
        assert_eq!(e.data["type"], e.name);
        assert!(e.to_sse().starts_with(&format!("event: {}\ndata: {{", e.name)));
    }
    events.into_iter().map(|e| e.data).collect()
}

fn names_of(ev: &[Value]) -> Vec<&str> {
    ev.iter().map(|e| e["type"].as_str().unwrap()).collect()
}

#[test]
fn text() {
    let ev = run("chat-chunks-text.jsonl");
    golden("expected-events-text.json", &json!(ev));
    let last = ev.last().unwrap();
    assert_eq!(last["type"], "response.completed");
    assert_eq!(last["response"]["output"][0]["content"][0]["text"], "Bonjour");
    assert_eq!(last["response"]["usage"]["input_tokens_details"]["cached_tokens"], 100);
    assert_eq!(last["response"]["usage"]["total_tokens"], 128);
}

#[test]
fn reasoning_then_tool() {
    let ev = run("chat-chunks-reasoning-tool.jsonl");
    golden("expected-events-reasoning-tool.json", &json!(ev));
    let out = &ev.last().unwrap()["response"]["output"];
    assert_eq!(out[0]["type"], "reasoning");
    assert_eq!(out[0]["content"][0]["text"], "Je dois lister.");
    assert_eq!(out[1]["type"], "function_call");
    assert_eq!(out[1]["name"], "echo");
    assert_eq!(out[1]["namespace"], "mcp__test");
    assert_eq!(out[1]["call_id"], "call_abc");
    assert_eq!(out[1]["arguments"], "{\"text\":\"hi\"}");
    let done: Vec<&Value> = ev.iter().filter(|e| e["type"] == "response.output_item.done").collect();
    assert_eq!(done[1]["output_index"], 1);
}

#[test]
fn parallel_tools() {
    let ev = run("chat-chunks-parallel-tools.jsonl");
    golden("expected-events-parallel-tools.json", &json!(ev));
    let out = &ev.last().unwrap()["response"]["output"];
    assert_eq!(out.as_array().unwrap().len(), 2);
    assert!(
        out[0]["call_id"].as_str().unwrap().starts_with("call_"),
        "id absent → call_ généré"
    );
    assert!(out[0].get("namespace").is_none());
    assert_eq!(out[1]["call_id"], "call_2");
    assert_eq!(out[1]["arguments"], "{\"cmd\":\"pwd\"}");
    // Usage absent → zéros.
    assert_eq!(ev.last().unwrap()["response"]["usage"]["input_tokens"], 0);
}

#[test]
fn truncated_call_becomes_a_notice() {
    let ev = run("chat-chunks-length-tool.jsonl");
    let types = names_of(&ev);
    // L'appel coupé n'est jamais « terminé » : Codex ne l'exécute ni ne le garde.
    assert!(
        !ev.iter()
            .any(|e| e["type"] == "response.output_item.done" && e["item"]["type"] == "function_call"),
        "{types:?}"
    );
    let last = ev.last().unwrap();
    assert_eq!(last["type"], "response.completed");
    let out = last["response"]["output"].as_array().unwrap();
    assert_eq!(out.len(), 2, "réflexion + message");
    assert_eq!(out[0]["type"], "reasoning");
    let text = out[1]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("32768 jetons"), "{text}");
    assert!(text.contains("`exec_command`"), "{text}");
    assert!(text.contains("plusieurs fichiers"), "{text}");
}

#[test]
fn complete_call_survives_length() {
    // Coupé juste après un appel complet : il reste transmis.
    let raw = std::fs::read_to_string(fixture_path("chat-chunks-reasoning-tool.jsonl")).unwrap();
    let mut t = Translator::new("resp_1".into(), "qwen".into(), names(), counter_ids());
    let mut ev = t.start();
    for line in raw.lines().filter(|l| !l.trim().is_empty() && l.trim() != "[DONE]") {
        let mut c: Value = serde_json::from_str(line).unwrap();
        if c["choices"][0]["finish_reason"] == "tool_calls" {
            c["choices"][0]["finish_reason"] = json!("length");
        }
        ev.extend(t.push_chunk(&c));
    }
    ev.extend(t.finish(true));
    let ev: Vec<Value> = ev.into_iter().map(|e| e.data).collect();
    let last = ev.last().unwrap();
    assert_eq!(last["type"], "response.incomplete");
    assert!(
        last["response"]["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["type"] == "function_call" && i["arguments"] == "{\"text\":\"hi\"}")
    );
}

#[test]
fn length_is_incomplete() {
    let ev = run("chat-chunks-length.jsonl");
    golden("expected-events-length.json", &json!(ev));
    let last = ev.last().unwrap();
    assert_eq!(last["type"], "response.incomplete");
    assert_eq!(last["response"]["incomplete_details"]["reason"], "max_output_tokens");
}

#[test]
fn error_is_failed() {
    let ev = run("chat-chunks-error.jsonl");
    golden("expected-events-error.json", &json!(ev));
    let last = ev.last().unwrap();
    assert_eq!(last["type"], "response.failed");
    assert_eq!(last["response"]["error"]["code"], "server_error");
    assert_eq!(last["response"]["error"]["message"], "slot crashed");
    assert_eq!(names_of(&ev).iter().filter(|n| **n == "response.failed").count(), 1);
}

#[test]
fn no_done_is_failed() {
    let ev = run("chat-chunks-no-done.jsonl");
    golden("expected-events-no-done.json", &json!(ev));
    assert_eq!(ev.last().unwrap()["type"], "response.failed");
}

#[test]
fn keepalive_and_fail() {
    let mut t = Translator::new("resp_1".into(), "qwen".into(), NameMap::new(), counter_ids());
    let start = t.start();
    assert_eq!(
        names_of(&start.iter().map(|e| e.data.clone()).collect::<Vec<_>>()),
        ["response.created", "response.in_progress"]
    );
    let k = t.keepalive();
    assert_eq!(k.name, "response.in_progress");
    assert_eq!(k.data["sequence_number"], 2);
    let f = t.fail("context_length_exceeded", "trop long");
    assert_eq!(f[0].data["response"]["error"]["code"], "context_length_exceeded");
    assert!(t.finish(true).is_empty(), "rien après la fin");
}
