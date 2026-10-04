// Claude, on its own Messages API: web search, PDFs and images, and the
// server-side fallback that answers a policy decline with another model in the
// same call instead of a dead end.
//
// Agent turns send client tools instead, and keep Claude's thinking blocks:
// they go back unchanged, in place, on every later request (the API drops
// what the model can't read), so the history stays append-only.

use serde_json::{json, Value};

use super::{exchange, CallError, ModelInfo, Part, Reply, Request, Role, Stop, ToolCall, Turn, Wire};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

const ANTHROPIC_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Room for adaptive thinking and a full answer, short of HTTP timeouts.
const MAX_TOKENS: u32 = 16_000;
/// An agent may write a whole file in one tool call, after thinking.
const AGENT_MAX_TOKENS: u32 = 32_000;

/// The server-side fallback is offered on these models.
fn uses_fallbacks(model: &str) -> bool {
    ["claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"]
        .iter()
        .any(|m| model == *m)
}

/// Models from before Claude 4.6 (and every Haiku) only take the basic web
/// search tool; newer ones take the one that filters results as it goes.
fn legacy_search(model: &str) -> bool {
    if model.contains("haiku") || model.starts_with("claude-3") {
        return true;
    }
    for family in ["claude-opus-4", "claude-sonnet-4"] {
        if let Some(rest) = model.strip_prefix(family) {
            let Some(minor) = rest.strip_prefix('-') else { return rest.is_empty() };
            let mut chars = minor.chars();
            return match (chars.next(), chars.next()) {
                // A date right after the major version: Claude 4.0.
                (Some(_), Some(c)) if c.is_ascii_digit() => true,
                (Some(d), _) => d.is_ascii_digit() && d <= '5',
                (None, _) => true,
            };
        }
    }
    false
}

fn web_search_tool(model: &str) -> Value {
    let kind = if legacy_search(model) { "web_search_20250305" } else { "web_search_20260209" };
    json!({ "type": kind, "name": "web_search", "max_uses": 5 })
}

fn block(part: &Part) -> Option<Value> {
    Some(match part {
        // Claude refuses empty text blocks.
        Part::Text { text } if text.is_empty() => return None,
        Part::Text { text } => json!({ "type": "text", "text": text }),
        Part::Image { mime, base64 } => json!({
            "type": "image",
            "source": { "type": "base64", "media_type": mime, "data": base64 },
        }),
        Part::Pdf { base64, .. } => json!({
            "type": "document",
            "source": { "type": "base64", "media_type": "application/pdf", "data": base64 },
        }),
        Part::ToolCall(call) => json!({
            "type": "tool_use",
            "id": call.id,
            "name": call.name,
            "input": if call.input.is_object() { call.input.clone() } else { json!({}) },
        }),
        Part::ToolResult { id, output, is_error } => json!({
            "type": "tool_result",
            "tool_use_id": id,
            "content": if output.is_empty() { "(no output)" } else { output.as_str() },
            "is_error": is_error,
        }),
        // Thinking from any Claude model goes back; other providers' data never.
        Part::Opaque { wire: Wire::Anthropic, data, .. } => data.clone(),
        Part::Opaque { .. } => return None,
    })
}

fn message(turn: &Turn) -> Value {
    let role = if turn.role == Role::Assistant { "assistant" } else { "user" };
    if turn.role == Role::Assistant && turn.is_plain_text() {
        return json!({ "role": role, "content": turn.text() });
    }
    let content: Vec<Value> = turn.parts.iter().filter_map(block).collect();
    json!({ "role": role, "content": content })
}

pub fn body(request: &Request) -> Value {
    if let Some(tools) = request.tools {
        // An agent turn: client tools only, nothing server-side, so every
        // block in the history is one we can send back.
        return json!({
            "model": request.model,
            "max_tokens": AGENT_MAX_TOKENS,
            "system": request.system,
            "tools": tools
                .iter()
                .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema }))
                .collect::<Vec<_>>(),
            "messages": request.turns.iter().map(message).collect::<Vec<_>>(),
            "cache_control": { "type": "ephemeral" },
        });
    }
    let mut body = json!({
        "model": request.model,
        "max_tokens": MAX_TOKENS,
        "system": request.system,
        "tools": [web_search_tool(request.model)],
        "messages": request.turns.iter().map(message).collect::<Vec<_>>(),
    });
    if uses_fallbacks(request.model) {
        body["fallbacks"] = json!("default");
    }
    body
}

/// The answer's text, or why there is none. A policy decline comes back as
/// HTTP 200 with stop_reason "refusal".
pub fn reply_text(response: &Value) -> Result<String, CallError> {
    if response.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
        let why = response
            .pointer("/stop_details/explanation")
            .and_then(Value::as_str)
            .unwrap_or("Claude declined this one.");
        return Err(CallError::Declined(why.to_string()));
    }
    let blocks = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::Unexpected("no content".into()))?;
    Ok(blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// An agent turn's parts in order: thinking, text and tool calls.
pub fn reply(response: &Value, model: &str) -> Result<Reply, CallError> {
    let stop = response.get("stop_reason").and_then(Value::as_str).unwrap_or("");
    if stop == "refusal" {
        // A refusal can cut a tool call off mid-input: none of it runs.
        reply_text(response)?;
    }
    let blocks = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::Unexpected("no content".into()))?;
    let mut parts = Vec::new();
    for b in blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = b.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.is_empty() {
                    parts.push(Part::text(text));
                }
            }
            Some("tool_use") => parts.push(Part::ToolCall(ToolCall {
                id: b.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(super::new_call_id),
                name: b.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                input: b.get("input").filter(|i| i.is_object()).cloned().unwrap_or_else(|| json!({})),
                bad_arguments: None,
                raw: None,
            })),
            Some("thinking" | "redacted_thinking") => {
                parts.push(Part::Opaque { wire: Wire::Anthropic, model: model.to_string(), data: b.clone() })
            }
            _ => {}
        }
    }
    let stop = match stop {
        "max_tokens" | "model_context_window_exceeded" => Stop::Truncated,
        "tool_use" => Stop::ToolUse,
        _ if parts.iter().any(|p| matches!(p, Part::ToolCall(_))) => Stop::ToolUse,
        _ => Stop::Done,
    };
    Ok(Reply { parts, stop })
}

pub async fn complete(client: &reqwest::Client, request: &Request<'_>) -> Result<Reply, CallError> {
    let mut call = client
        .post(format!("{}/messages", request.base_url))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body(request));
    if let Some(key) = request.key {
        call = call.header("x-api-key", key);
    }
    reply(&exchange(call).await?, request.model)
}

pub async fn send(client: &reqwest::Client, request: &Request<'_>) -> Result<String, CallError> {
    let mut call = client
        .post(format!("{}/messages", request.base_url))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body(request));
    if let Some(key) = request.key {
        call = call.header("x-api-key", key);
    }
    if uses_fallbacks(request.model) {
        call = call.header("anthropic-beta", FALLBACK_BETA);
    }
    reply_text(&exchange(call).await?)
}

pub fn parse_models(response: &Value) -> Vec<ModelInfo> {
    response
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    let label = m.get("display_name").and_then(Value::as_str).unwrap_or(id);
                    Some(ModelInfo { id: id.into(), label: label.into() })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub async fn models(client: &reqwest::Client, base_url: &str, key: Option<&str>) -> Result<Vec<ModelInfo>, CallError> {
    let mut call = client
        .get(format!("{base_url}/models?limit=100"))
        .header("anthropic-version", ANTHROPIC_VERSION);
    if let Some(key) = key {
        call = call.header("x-api-key", key);
    }
    Ok(parse_models(&exchange(call).await?))
}

#[cfg(test)]
mod tests {
    use super::super::testing::{block_on, client, serve_once, serve_sequence};
    use super::super::ToolSpec;
    use super::*;

    fn turns() -> Vec<Turn> {
        vec![
            Turn {
                role: Role::User,
                parts: vec![
                    Part::Pdf { name: "a.pdf".into(), base64: "UERG".into() },
                    Part::text("File: a.pdf"),
                    Part::text("Summarise"),
                ],
            },
            Turn { role: Role::Assistant, parts: vec![Part::text("It is about X.")] },
            Turn { role: Role::User, parts: vec![Part::text("And Y?")] },
        ]
    }

    fn request<'a>(base: &'a str, model: &'a str, turns: &'a [Turn]) -> Request<'a> {
        Request { base_url: base, key: Some("sk-test"), model, system: "be nice", turns, reads_pdf: true, tools: None }
    }

    fn tools() -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "read_file".into(),
            description: "Reads a file".into(),
            schema: json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
        }]
    }

    #[test]
    fn an_agent_turn_runs_a_tool_and_sends_back_thinking_and_results() {
        let (base, rx) = serve_sequence(vec![
            (200, r#"{"content":[{"type":"thinking","thinking":"","signature":"sig1"},{"type":"text","text":""},{"type":"tool_use","id":"toolu_1","name":"read_file","input":{"path":"a.txt"}}],"stop_reason":"tool_use"}"#.into()),
            (200, r#"{"content":[{"type":"text","text":"It says hi."}],"stop_reason":"end_turn"}"#.into()),
        ]);
        let tools = tools();
        let mut history = vec![Turn { role: Role::User, parts: vec![Part::text("What is in a.txt?")] }];
        let mut req = request(&base, "claude-opus-5-5", &history);
        req.tools = Some(&tools);
        let first = block_on(complete(&client(), &req)).unwrap();
        assert_eq!(first.stop, Stop::ToolUse);
        // The empty text block is dropped, the thinking block kept.
        assert_eq!(first.parts.len(), 2);
        assert!(matches!(&first.parts[0], Part::Opaque { wire: Wire::Anthropic, .. }));
        let call = first.calls().next().unwrap().clone();
        assert_eq!((call.id.as_str(), call.input["path"].as_str()), ("toolu_1", Some("a.txt")));

        let sent = rx.recv().unwrap().json();
        assert_eq!(sent["tools"][0]["input_schema"]["required"][0], "path");
        assert_eq!(sent["max_tokens"], AGENT_MAX_TOKENS);
        assert!(sent.get("fallbacks").is_none());

        history.push(Turn { role: Role::Assistant, parts: first.parts });
        history.push(Turn { role: Role::User, parts: vec![Part::ToolResult { id: "toolu_1".into(), output: "hi".into(), is_error: false }] });
        let mut req = request(&base, "claude-opus-5-5", &history);
        req.tools = Some(&tools);
        let second = block_on(complete(&client(), &req)).unwrap();
        assert_eq!((second.stop, second.text()), (Stop::Done, "It says hi.".to_string()));
        let sent = rx.recv().unwrap().json();
        let assistant = &sent["messages"][1]["content"];
        assert_eq!(assistant[0], json!({"type": "thinking", "thinking": "", "signature": "sig1"}));
        assert_eq!(assistant[1]["type"], "tool_use");
        assert_eq!(sent["messages"][2]["content"][0], json!({"type": "tool_result", "tool_use_id": "toolu_1", "content": "hi", "is_error": false}));
    }

    #[test]
    fn a_cut_turn_and_a_refusal_never_run_tools() {
        let cut = json!({"content":[{"type":"tool_use","id":"t","name":"write_file","input":{}}],"stop_reason":"max_tokens"});
        assert_eq!(reply(&cut, "m").unwrap().stop, Stop::Truncated);
        let refused = json!({"content":[{"type":"tool_use","id":"t","name":"x","input":{}}],"stop_reason":"refusal","stop_details":{"explanation":"No."}});
        assert_eq!(reply(&refused, "m"), Err(CallError::Declined("No.".into())));
    }

    #[test]
    fn other_providers_data_stays_out_of_claude_requests() {
        let history = vec![
            Turn { role: Role::User, parts: vec![Part::text("go")] },
            Turn {
                role: Role::Assistant,
                parts: vec![
                    Part::Opaque { wire: Wire::OpenAiResponses, model: "gpt-5".into(), data: json!({"type": "reasoning"}) },
                    Part::ToolCall(ToolCall { id: "c1".into(), name: "read_file".into(), input: json!({}), bad_arguments: Some("{bad".into()), raw: None }),
                ],
            },
        ];
        let tools = tools();
        let mut req = request("x", "claude-opus-5-5", &history);
        req.tools = Some(&tools);
        let body = body(&req);
        assert_eq!(body["messages"][1]["content"], json!([{"type": "tool_use", "id": "c1", "name": "read_file", "input": {}}]));
    }

    #[test]
    fn search_tool_matches_the_model_generation() {
        for legacy in ["claude-haiku-4-5", "claude-sonnet-4-5", "claude-sonnet-4-5-20250929", "claude-opus-4-1", "claude-opus-4-20250514", "claude-3-7-sonnet-latest", "claude-sonnet-4"] {
            assert!(legacy_search(legacy), "{legacy}");
        }
        for current in ["claude-opus-5-5", "claude-sonnet-5-5", "claude-opus-4-6", "claude-opus-4-8", "claude-sonnet-4-6", "claude-fable-5-1"] {
            assert!(!legacy_search(current), "{current}");
        }
    }

    #[test]
    fn the_request_carries_history_files_and_tools() {
        let turns = turns();
        let body = body(&request("x", "claude-opus-5-5", &turns));
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["system"], "be nice");
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["tools"][0]["type"], "web_search_20260209");
        assert_eq!(body["messages"][0]["content"][0]["type"], "document");
        assert_eq!(body["messages"][0]["content"][2]["text"], "Summarise");
        assert_eq!(body["messages"][1], json!({ "role": "assistant", "content": "It is about X." }));
        let haiku = super::body(&request("x", "claude-haiku-4-5", &turns));
        assert!(haiku.get("fallbacks").is_none());
        assert_eq!(haiku["tools"][0]["type"], "web_search_20250305");
    }

    #[test]
    fn a_reply_is_read_over_http_with_its_headers() {
        let (base, rx) = serve_once(200, r#"{"content":[{"type":"server_tool_use"},{"type":"text","text":"Hello"},{"type":"text","text":"there"}],"stop_reason":"end_turn"}"#);
        let turns = turns();
        let text = block_on(send(&client(), &request(&base, "claude-opus-5-5", &turns))).unwrap();
        assert_eq!(text, "Hello\nthere");
        let got = rx.recv().unwrap();
        assert_eq!(got.request_line, "POST /messages HTTP/1.1");
        assert_eq!(got.header("x-api-key"), Some("sk-test"));
        assert_eq!(got.header("anthropic-beta"), Some(FALLBACK_BETA));
        assert_eq!(got.json()["messages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_decline_and_an_error_are_told_apart() {
        let declined = json!({"stop_reason": "refusal", "stop_details": {"explanation": "Not this."}, "content": []});
        assert_eq!(reply_text(&declined), Err(CallError::Declined("Not this.".into())));
        let (base, _rx) = serve_once(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        let turns = turns();
        let err = block_on(send(&client(), &request(&base, "claude-opus-5-5", &turns))).unwrap_err();
        assert_eq!(err, CallError::Status { code: 401, message: "invalid x-api-key".into() });
    }

    #[test]
    fn models_are_listed_with_their_names() {
        let (base, rx) = serve_once(200, r#"{"data":[{"id":"claude-opus-5-5","display_name":"Claude Opus 5.5"},{"id":"claude-x"}]}"#);
        let list = block_on(models(&client(), &base, Some("k"))).unwrap();
        assert_eq!(list[0], ModelInfo { id: "claude-opus-5-5".into(), label: "Claude Opus 5.5".into() });
        assert_eq!(list[1].label, "claude-x");
        assert_eq!(rx.recv().unwrap().request_line, "GET /models?limit=100 HTTP/1.1");
    }
}
