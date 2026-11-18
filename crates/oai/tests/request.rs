mod common;

use common::{fixture, golden};
use enginex_hub::sampling::Sampling;
use enginex_oai::{Ctx, NameMap, ResponsesRequest, TranslateError, to_chat};
use serde_json::{Value, json};

fn ctx() -> Ctx {
    Ctx {
        model_name: "qwen".into(),
        sampling: Sampling {
            temperature: Some(0.6),
            top_k: Some(20.0),
            ..Sampling::default()
        },
        vision: false,
    }
}

fn req(v: Value) -> ResponsesRequest {
    serde_json::from_value(v).unwrap()
}

fn base(input: Value) -> Value {
    json!({"model": "x", "stream": true, "input": input})
}

fn msgs(v: Value, ctx: &Ctx) -> Vec<Value> {
    to_chat(&req(v), ctx).unwrap().body["messages"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn codex_request_matches_expected() {
    let r = req(fixture("codex-0.170-request.json"));
    let t = to_chat(&r, &ctx()).unwrap();
    golden("expected-chat-request.json", &t.body);
    let m = t.body["messages"].as_array().unwrap();
    assert_eq!(m[0]["role"], "system");
    assert_eq!(m.len(), 2, "2 developer fusionnés + 2 user fusionnés");
    let names: Vec<&str> = t.body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"exec_command"));
    assert!(names.contains(&"mcp__test__echo"));
    assert!(
        t.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["function"].get("strict").is_none())
    );
    assert_eq!(t.body["stream_options"], json!({"include_usage": true}));
    assert_eq!(
        t.names.split("mcp__test__echo"),
        (Some("mcp__test".into()), "echo".into())
    );
}

#[test]
fn deterministic_bytes() {
    let r = req(fixture("codex-0.170-request.json"));
    let a = serde_json::to_vec(&to_chat(&r, &ctx()).unwrap().body).unwrap();
    let b = serde_json::to_vec(&to_chat(&r, &ctx()).unwrap().body).unwrap();
    assert_eq!(a, b);
}

#[test]
fn turn2_prefix_identical() {
    let m1 = msgs(fixture("codex-0.170-request.json"), &ctx());
    let m2 = msgs(fixture("codex-0.170-request-turn2.json"), &ctx());
    assert!(m2.len() > m1.len());
    for (i, m) in m1.iter().enumerate() {
        assert_eq!(
            serde_json::to_string(m).unwrap(),
            serde_json::to_string(&m2[i]).unwrap(),
            "message {i}"
        );
    }
}

#[test]
fn late_developer_and_user_merge() {
    let m = msgs(
        base(json!([
            {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "A"}]},
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "u1"}]},
            {"type": "message", "role": "user", "content": "u2"},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "r"}]},
            {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "B"}]},
            {"type": "message", "role": "user", "content": "u3"},
        ])),
        &ctx(),
    );
    assert_eq!(m.len(), 4);
    assert_eq!(m[0], json!({"role": "system", "content": "A"}));
    assert_eq!(m[1], json!({"role": "user", "content": "u1\nu2"}));
    assert_eq!(m[3], json!({"role": "user", "content": "[Consigne système]\nB\nu3"}));
}

#[test]
fn reasoning_two_calls_two_outputs() {
    let m = msgs(
        base(json!([
            {"type": "message", "role": "user", "content": "go"},
            {"type": "reasoning", "id": "rs_x", "summary": [], "content": [{"type": "reasoning_text", "text": "je pense"}],
                "encrypted_content": "zzz"},
            {"type": "function_call", "call_id": "c1", "name": "exec_command", "arguments": "{\"cmd\":\"ls\"}"},
            {"type": "function_call", "call_id": "c2", "name": "echo", "namespace": "mcp__test", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c1", "output": "a.txt"},
            {"type": "function_call_output", "call_id": "c2", "output": [{"type": "input_text", "text": "echo: hi"}]},
        ])),
        &ctx(),
    );
    assert_eq!(m.len(), 4);
    assert_eq!(m[1]["role"], "assistant");
    assert_eq!(m[1]["reasoning_content"], "je pense");
    assert_eq!(m[1]["content"], Value::Null);
    let calls = m[1]["tool_calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["function"]["arguments"], "{\"cmd\":\"ls\"}");
    assert_eq!(calls[1]["function"]["name"], "mcp__test__echo");
    assert_eq!(m[2], json!({"role": "tool", "tool_call_id": "c1", "content": "a.txt"}));
    assert_eq!(
        m[3],
        json!({"role": "tool", "tool_call_id": "c2", "content": "echo: hi"})
    );
}

#[test]
fn reasoning_without_text_is_skipped() {
    let m = msgs(
        base(json!([
            {"type": "message", "role": "user", "content": "go"},
            {"type": "reasoning", "summary": []},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]},
        ])),
        &ctx(),
    );
    assert!(m[1].get("reasoning_content").is_none());
    assert_eq!(m[1]["content"], "ok");
}

#[test]
fn tool_output_image_goes_to_user() {
    let mut c = ctx();
    c.vision = true;
    let m = msgs(
        base(json!([
            {"type": "message", "role": "user", "content": "go"},
            {"type": "function_call", "call_id": "c1", "name": "view_image", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c1", "output": [
                {"type": "input_text", "text": "voici"},
                {"type": "input_image", "image_url": "data:image/png;base64,AAAA"}]},
        ])),
        &c,
    );
    assert_eq!(m.len(), 4);
    assert_eq!(m[2], json!({"role": "tool", "tool_call_id": "c1", "content": "voici"}));
    assert_eq!(
        m[3],
        json!({"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}]})
    );
}

#[test]
fn errors() {
    let mut r = base(json!("hi"));
    r["stream"] = json!(false);
    assert_eq!(to_chat(&req(r), &ctx()).unwrap_err(), TranslateError::StreamRequired);
    let mut r = base(json!("hi"));
    r.as_object_mut().unwrap().remove("stream");
    assert_eq!(to_chat(&req(r), &ctx()).unwrap_err().code(), "stream_required");
    let mut r = base(json!("hi"));
    r["previous_response_id"] = json!("resp_1");
    assert_eq!(to_chat(&req(r), &ctx()).unwrap_err(), TranslateError::PreviousResponse);
    let r = base(json!([{"type": "message", "role": "user",
        "content": [{"type": "input_image", "image_url": "data:x"}]}]));
    let e = to_chat(&req(r), &ctx()).unwrap_err();
    assert_eq!(e.code(), "no_vision");
    assert_eq!(e.to_string(), "Le modèle actif ne lit pas les images");
    let e = enginex_oai::parse_and_translate(b"{not json", &ctx()).unwrap_err();
    assert_eq!(e.code(), "invalid_request_error");
}

#[test]
fn dropped_items_and_tools_counted() {
    let r = json!({"stream": true, "input": [
        {"type": "message", "role": "user", "content": "go"},
        {"type": "web_search_call", "id": "w"},
        {"type": "web_search_call", "id": "w2"},
        {"type": "custom_tool_call", "call_id": "x", "name": "apply_patch", "input": "..."}],
        "tools": [{"type": "web_search"}, {"type": "custom", "name": "apply_patch"},
            {"type": "function", "name": "f", "parameters": {"type": "object"}, "strict": true}]});
    let t = to_chat(&req(r), &ctx()).unwrap();
    assert_eq!(t.dropped["item:web_search_call"], 2);
    assert_eq!(t.dropped["item:custom_tool_call"], 1);
    assert_eq!(t.dropped["tool:web_search"], 1);
    assert_eq!(t.dropped["tool:custom"], 1);
    assert_eq!(t.body["tools"].as_array().unwrap().len(), 1);
}

#[test]
fn names_collision_and_long() {
    let mut n = NameMap::new();
    assert_eq!(n.insert(None, "a__b"), "a__b");
    assert_eq!(n.insert(Some("a"), "b"), "a__b_2");
    assert_eq!(n.insert(Some("a"), "b"), "a__b_2");
    assert_eq!(n.split("a__b_2"), (Some("a".into()), "b".into()));
    assert_eq!(n.split("a__b"), (None, "a__b".into()));
    let long = "x".repeat(80);
    let f1 = n.insert(Some("ns"), &long);
    assert_eq!(f1.chars().count(), 64);
    let mut n2 = NameMap::new();
    assert_eq!(n2.insert(Some("ns"), &long), f1);
    assert_eq!(n.split(&f1), (Some("ns".into()), long));
    assert_eq!(n.split("inconnu"), (None, "inconnu".into()));
}

#[test]
fn tool_choice_object() {
    let tools = json!([{"type": "function", "name": "f", "parameters": {}},
        {"type": "namespace", "name": "mcp__t", "tools": [{"type": "function", "name": "g", "parameters": {}}]}]);
    let mk = |choice: Value| {
        let r = json!({"stream": true, "input": "hi", "tools": tools, "tool_choice": choice});
        to_chat(&req(r), &ctx()).unwrap().body["tool_choice"].clone()
    };
    assert_eq!(mk(json!({"type": "function", "name": "f"})), "required");
    assert_eq!(
        mk(json!({"type": "function", "name": "g", "namespace": "mcp__t"})),
        "required"
    );
    assert_eq!(mk(json!({"type": "function", "name": "absent"})), "auto");
    assert_eq!(mk(json!("none")), "none");
}

#[test]
fn sampling_injection() {
    let mut r = base(json!("hi"));
    r["temperature"] = json!(0.2);
    let t = to_chat(&req(r), &ctx()).unwrap();
    assert_eq!(t.body["temperature"], 0.6);
    assert_eq!(t.body["top_k"], 20);
    // Champ client non couvert par le preset : passe par chat/completions brut.
    let mut body = json!({"temperature": 0.2, "top_k": 50});
    enginex_oai::sampling::inject(
        &mut body,
        &Sampling {
            temperature: Some(0.6),
            ..Sampling::default()
        },
    );
    assert_eq!(body, json!({"temperature": 0.6, "top_k": 50}));
    let mut body = json!({});
    enginex_oai::sampling::inject(
        &mut body,
        &Sampling {
            min_p: Some(0.05),
            ..Sampling::default()
        },
    );
    assert_eq!(body, json!({"min_p": 0.05}));
}

#[test]
fn text_format_and_max_tokens() {
    let mut r = base(json!("hi"));
    r["max_output_tokens"] = json!(100);
    r["text"] = json!({"format": {"type": "json_schema", "name": "s", "schema": {"type": "object"}, "strict": true}});
    let t = to_chat(&req(r), &ctx()).unwrap();
    assert_eq!(t.body["max_tokens"], 100);
    assert_eq!(
        t.body["response_format"],
        json!({"type": "json_schema", "json_schema": {"name": "s", "schema": {"type": "object"}, "strict": true}})
    );
}
