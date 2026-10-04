// OpenAI's Chat Completions format, which nearly every other vendor speaks:
// Gemini (through Google's OpenAI-compatible endpoint), OpenRouter, Ollama,
// LM Studio, Groq, DeepSeek, Mistral, xAI and the like.
//
// Tool calls keep the server's own call object and send it back as it came, so
// whatever a vendor adds to it (Gemini's thought signatures) survives the loop.

use serde_json::{json, Value};

use super::{exchange, parse_arguments, pdf_note, CallError, ModelInfo, Part, Reply, Request, Role, Stop, ToolCall, Turn};

fn data_url(mime: &str, base64: &str) -> String {
    format!("data:{mime};base64,{base64}")
}

/// The call as the server sent it, with the id and arguments we keep.
fn tool_call(call: &ToolCall) -> Value {
    let mut value = call.raw.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
    value["id"] = json!(call.id);
    value["type"] = json!("function");
    value["function"] = json!({ "name": call.name, "arguments": call.arguments() });
    value
}

/// One turn as messages: an assistant turn is one message, its tool calls
/// included; tool results each become a `tool` message, then any other parts.
fn messages(turn: &Turn, reads_pdf: bool) -> Vec<Value> {
    if turn.role == Role::Assistant {
        let calls: Vec<Value> = turn
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall(c) => Some(tool_call(c)),
                _ => None,
            })
            .collect();
        let text = turn.text();
        if calls.is_empty() {
            return vec![json!({ "role": "assistant", "content": text })];
        }
        let content = if text.is_empty() { Value::Null } else { json!(text) };
        return vec![json!({ "role": "assistant", "content": content, "tool_calls": calls })];
    }
    let mut out: Vec<Value> = turn
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::ToolResult { id, output, .. } => Some(json!({ "role": "tool", "tool_call_id": id, "content": output })),
            _ => None,
        })
        .collect();
    let rest: Vec<&Part> = turn
        .parts
        .iter()
        .filter(|p| !matches!(p, Part::ToolResult { .. } | Part::ToolCall(_) | Part::Opaque { .. }))
        .collect();
    if rest.is_empty() {
        return out;
    }
    // Plain text stays a plain string: the form every server accepts.
    if rest.iter().all(|p| matches!(p, Part::Text { .. })) {
        let text = Turn { role: Role::User, parts: rest.into_iter().cloned().collect() }.text();
        out.push(json!({ "role": "user", "content": text }));
        return out;
    }
    let content: Vec<Value> = rest
        .into_iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(json!({ "type": "text", "text": text })),
            Part::Image { mime, base64 } => {
                Some(json!({ "type": "image_url", "image_url": { "url": data_url(mime, base64) } }))
            }
            Part::Pdf { name, base64 } if reads_pdf => Some(json!({
                "type": "file",
                "file": { "filename": name, "file_data": data_url("application/pdf", base64) },
            })),
            Part::Pdf { name, .. } => Some(json!({ "type": "text", "text": pdf_note(name) })),
            _ => None,
        })
        .collect();
    out.push(json!({ "role": "user", "content": content }));
    out
}

pub fn body(request: &Request) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.turns.iter().flat_map(|t| self::messages(t, request.reads_pdf)));
    // No max_tokens: each server's default fits, and some reasoning models
    // reject the parameter outright.
    let mut body = json!({ "model": request.model, "messages": messages });
    if let Some(tools) = request.tools {
        body["tools"] = tools
            .iter()
            .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema } }))
            .collect();
    }
    body
}

pub fn reply_text(response: &Value) -> Result<String, CallError> {
    let choice = response
        .pointer("/choices/0")
        .ok_or_else(|| CallError::Unexpected("no choices".into()))?;
    let message = choice.get("message").unwrap_or(&Value::Null);
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str).filter(|r| !r.is_empty()) {
        return Err(CallError::Declined(refusal.to_string()));
    }
    let text = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        // A few servers return content as a list of parts.
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    if text.trim().is_empty() && choice.get("finish_reason").and_then(Value::as_str) == Some("content_filter") {
        return Err(CallError::Declined("The provider's content filter stopped this answer.".into()));
    }
    Ok(text)
}

/// An agent turn: the text, then each tool call with its arguments read.
pub fn reply(response: &Value) -> Result<Reply, CallError> {
    let text = reply_text(response)?;
    let choice = response.pointer("/choices/0").unwrap_or(&Value::Null);
    let mut parts = Vec::new();
    if !text.trim().is_empty() {
        parts.push(Part::text(text));
    }
    let calls = choice.pointer("/message/tool_calls").and_then(Value::as_array);
    for raw in calls.into_iter().flatten() {
        let name = raw.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string();
        let arguments = match raw.pointer("/function/arguments") {
            Some(Value::String(s)) => s.clone(),
            // A few servers send the object itself.
            Some(other) if !other.is_null() => other.to_string(),
            _ => String::new(),
        };
        let (input, bad_arguments) = parse_arguments(&arguments);
        let id = raw
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(super::new_call_id);
        parts.push(Part::ToolCall(ToolCall { id, name, input, bad_arguments, raw: Some(raw.clone()) }));
    }
    let stop = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("length") => Stop::Truncated,
        _ if parts.iter().any(|p| matches!(p, Part::ToolCall(_))) => Stop::ToolUse,
        _ => Stop::Done,
    };
    Ok(Reply { parts, stop })
}

pub async fn complete(client: &reqwest::Client, request: &Request<'_>) -> Result<Reply, CallError> {
    let mut call = client.post(format!("{}/chat/completions", request.base_url)).json(&body(request));
    if let Some(key) = request.key {
        call = call.bearer_auth(key);
    }
    reply(&exchange(call).await?)
}

pub async fn send(client: &reqwest::Client, request: &Request<'_>) -> Result<String, CallError> {
    let mut call = client.post(format!("{}/chat/completions", request.base_url)).json(&body(request));
    if let Some(key) = request.key {
        call = call.bearer_auth(key);
    }
    reply_text(&exchange(call).await?)
}

/// `GET /models`, as OpenAI, Gemini, OpenRouter and Ollama all serve it; newest
/// first when the server says when each model was made.
pub fn parse_models(response: &Value) -> Vec<ModelInfo> {
    let Some(items) = response.get("data").and_then(Value::as_array) else { return Vec::new() };
    let mut list: Vec<(i64, ModelInfo)> = items
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            let label = m
                .get("name")
                .or_else(|| m.get("display_name"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            let created = m.get("created").and_then(Value::as_i64).unwrap_or(0);
            Some((created, ModelInfo { id: id.into(), label: label.into() }))
        })
        .collect();
    // Stable: servers without dates keep their own order.
    list.sort_by(|a, b| b.0.cmp(&a.0));
    list.into_iter().map(|(_, m)| m).collect()
}

pub async fn models(client: &reqwest::Client, base_url: &str, key: Option<&str>) -> Result<Vec<ModelInfo>, CallError> {
    let mut call = client.get(format!("{base_url}/models"));
    if let Some(key) = key {
        call = call.bearer_auth(key);
    }
    Ok(parse_models(&exchange(call).await?))
}

#[cfg(test)]
mod tests {
    use super::super::testing::{block_on, client, serve_once, serve_sequence};
    use super::super::{ToolSpec, Wire};
    use super::*;

    fn turns() -> Vec<Turn> {
        vec![
            Turn {
                role: Role::User,
                parts: vec![
                    Part::Image { mime: "image/png".into(), base64: "iVBO".into() },
                    Part::Pdf { name: "a.pdf".into(), base64: "UERG".into() },
                    Part::text("What is this?"),
                ],
            },
            Turn { role: Role::Assistant, parts: vec![Part::text("A chart.")] },
            Turn { role: Role::User, parts: vec![Part::text("Thanks")] },
        ]
    }

    fn request<'a>(base: &'a str, key: Option<&'a str>, turns: &'a [Turn], reads_pdf: bool) -> Request<'a> {
        Request { base_url: base, key, model: "gemini-2.5-flash", system: "be nice", turns, reads_pdf, tools: None }
    }

    #[test]
    fn an_agent_turn_keeps_the_servers_call_and_sends_results_as_tool_messages() {
        let (base, rx) = serve_sequence(vec![
            (200, r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"list_dir","arguments":"{\"path\":\".\"}"},"extra_content":{"google":{"thought_signature":"sig"}}},{"id":"","type":"function","function":{"name":"read_file","arguments":"{\"path\": \"a"}}]},"finish_reason":"tool_calls"}]}"#.into()),
            (200, r#"{"choices":[{"message":{"role":"assistant","content":"Done."},"finish_reason":"stop"}]}"#.into()),
        ]);
        let tools = vec![ToolSpec { name: "list_dir".into(), description: "Lists".into(), schema: json!({"type": "object"}) }];
        let mut history = vec![Turn { role: Role::User, parts: vec![Part::text("look around")] }];
        let mut req = request(&base, Some("k"), &history, false);
        req.tools = Some(&tools);
        let first = block_on(complete(&client(), &req)).unwrap();
        assert_eq!(first.stop, Stop::ToolUse);
        let calls: Vec<_> = first.calls().cloned().collect();
        assert_eq!(calls[0].input, json!({"path": "."}));
        // No id from the server: one is made. Unreadable arguments are kept as sent.
        assert!(calls[1].id.starts_with("call_"));
        assert_eq!(calls[1].bad_arguments.as_deref(), Some("{\"path\": \"a"));
        let sent = rx.recv().unwrap().json();
        assert_eq!(sent["tools"][0]["function"]["name"], "list_dir");

        history.push(Turn { role: Role::Assistant, parts: first.parts });
        history.push(Turn {
            role: Role::User,
            parts: vec![
                Part::ToolResult { id: "c1".into(), output: "a.txt".into(), is_error: false },
                Part::ToolResult { id: calls[1].id.clone(), output: "bad arguments".into(), is_error: true },
            ],
        });
        let mut req = request(&base, Some("k"), &history, false);
        req.tools = Some(&tools);
        let second = block_on(complete(&client(), &req)).unwrap();
        assert_eq!(second.text(), "Done.");
        let sent = rx.recv().unwrap().json();
        let m = sent["messages"].as_array().unwrap();
        assert_eq!(m[2]["content"], Value::Null);
        assert_eq!(m[2]["tool_calls"][0]["extra_content"]["google"]["thought_signature"], "sig");
        assert_eq!(m[2]["tool_calls"][1]["function"]["arguments"], "{\"path\": \"a");
        assert_eq!(m[3], json!({"role": "tool", "tool_call_id": "c1", "content": "a.txt"}));
        assert_eq!(m[4]["tool_call_id"], json!(calls[1].id));
        assert_eq!(m.len(), 5);
    }

    #[test]
    fn calls_from_other_wires_are_sent_in_this_format() {
        let history = vec![
            Turn {
                role: Role::Assistant,
                parts: vec![
                    Part::Opaque { wire: Wire::Anthropic, model: "claude".into(), data: json!({"type": "thinking"}) },
                    Part::text("Checking."),
                    Part::ToolCall(ToolCall { id: "toolu_9".into(), name: "grep".into(), input: json!({"pattern": "x"}), bad_arguments: None, raw: None }),
                ],
            },
            Turn { role: Role::User, parts: vec![Part::ToolResult { id: "toolu_9".into(), output: "".into(), is_error: false }, Part::text("go on")] },
        ];
        let body = body(&request("x", None, &history, false));
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m[1], json!({"role": "assistant", "content": "Checking.", "tool_calls": [{"id": "toolu_9", "type": "function", "function": {"name": "grep", "arguments": "{\"pattern\":\"x\"}"}}]}));
        assert_eq!(m[2]["role"], "tool");
        assert_eq!(m[3], json!({"role": "user", "content": "go on"}));
        assert_eq!(reply(&json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]})).unwrap().stop, Stop::Truncated);
    }

    #[test]
    fn history_becomes_system_user_and_assistant_messages() {
        let turns = turns();
        let body = body(&request("x", None, &turns, false));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0], json!({ "role": "system", "content": "be nice" }));
        assert_eq!(messages[1]["content"][0]["image_url"]["url"], "data:image/png;base64,iVBO");
        // A model that can't read PDFs is told the file is there.
        assert!(messages[1]["content"][1]["text"].as_str().unwrap().contains("a.pdf"));
        assert_eq!(messages[2], json!({ "role": "assistant", "content": "A chart." }));
        assert_eq!(messages[3], json!({ "role": "user", "content": "Thanks" }));
        assert!(body.get("max_tokens").is_none());

        let with_pdf = super::body(&request("x", None, &turns, true));
        assert_eq!(with_pdf["messages"][1]["content"][1]["file"]["file_data"], "data:application/pdf;base64,UERG");
    }

    #[test]
    fn a_reply_is_read_over_http_with_a_bearer_key() {
        let (base, rx) = serve_once(200, r#"{"choices":[{"message":{"role":"assistant","content":"Hi!"},"finish_reason":"stop"}]}"#);
        let turns = turns();
        assert_eq!(block_on(send(&client(), &request(&base, Some("k-1"), &turns, false))).unwrap(), "Hi!");
        let got = rx.recv().unwrap();
        assert_eq!(got.request_line, "POST /chat/completions HTTP/1.1");
        assert_eq!(got.header("authorization"), Some("Bearer k-1"));
        assert_eq!(got.json()["model"], "gemini-2.5-flash");
    }

    #[test]
    fn a_local_server_needs_no_key() {
        let (base, rx) = serve_once(200, r#"{"choices":[{"message":{"content":[{"type":"text","text":"local "},{"type":"text","text":"answer"}]}}]}"#);
        let turns = turns();
        assert_eq!(block_on(send(&client(), &request(&base, None, &turns, false))).unwrap(), "local answer");
        assert_eq!(rx.recv().unwrap().header("authorization"), None);
    }

    #[test]
    fn errors_refusals_and_filters_come_back_as_such() {
        let (base, _rx) = serve_once(400, r#"[{"error":{"code":400,"message":"API key not valid."}}]"#);
        let turns = turns();
        let err = block_on(send(&client(), &request(&base, Some("k"), &turns, false))).unwrap_err();
        assert_eq!(err, CallError::Status { code: 400, message: "API key not valid.".into() });
        assert!(matches!(reply_text(&json!({"choices":[{"message":{"content":null,"refusal":"No."}}]})), Err(CallError::Declined(_))));
        assert!(matches!(reply_text(&json!({"choices":[{"message":{"content":""},"finish_reason":"content_filter"}]})), Err(CallError::Declined(_))));
        assert!(matches!(reply_text(&json!({"id":"x"})), Err(CallError::Unexpected(_))));
    }

    #[test]
    fn models_come_newest_first_with_readable_names() {
        let (base, rx) = serve_once(200, r#"{"data":[{"id":"old","created":1},{"id":"openai/gpt-5","name":"OpenAI: GPT-5","created":9},{"id":"mid","created":5}]}"#);
        let list = block_on(models(&client(), &base, Some("k"))).unwrap();
        assert_eq!(list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["openai/gpt-5", "mid", "old"]);
        assert_eq!(list[0].label, "OpenAI: GPT-5");
        assert_eq!(rx.recv().unwrap().request_line, "GET /models HTTP/1.1");
    }
}
