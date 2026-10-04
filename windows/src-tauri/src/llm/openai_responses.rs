// OpenAI's Responses API: the one OpenAI recommends, and the only one that
// serves its Codex models. Requests are sent with `store: false`, so OpenAI
// keeps no copy of the conversation for later retrieval.
//
// Without a stored conversation, a reasoning model's reasoning items (they
// carry their encrypted content) are sent back with the tool results, as
// OpenAI asks, but only to the model that made them.

use serde_json::{json, Value};

use super::{exchange, parse_arguments, CallError, Part, Reply, Request, Role, Stop, ToolCall, Turn, Wire};

fn data_url(mime: &str, base64: &str) -> String {
    format!("data:{mime};base64,{base64}")
}

/// One turn as input items, in order. Text runs become messages; tool calls,
/// results and reasoning are items of their own.
fn items(turn: &Turn, model: &str) -> Vec<Value> {
    if turn.is_plain_text() {
        let role = if turn.role == Role::Assistant { "assistant" } else { "user" };
        return vec![json!({ "role": role, "content": turn.text() })];
    }
    let mut out = Vec::new();
    let mut content: Vec<Value> = Vec::new();
    let flush = |content: &mut Vec<Value>, out: &mut Vec<Value>| {
        if content.is_empty() {
            return;
        }
        let role = if turn.role == Role::Assistant { "assistant" } else { "user" };
        if turn.role == Role::Assistant {
            let text: Vec<&str> = content.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect();
            out.push(json!({ "role": role, "content": text.join("\n") }));
        } else {
            out.push(json!({ "role": role, "content": std::mem::take(content) }));
        }
        content.clear();
    };
    for part in &turn.parts {
        match part {
            Part::Text { text } if text.is_empty() => {}
            Part::Text { text } => content.push(json!({ "type": "input_text", "text": text })),
            Part::Image { mime, base64 } => content.push(json!({ "type": "input_image", "image_url": data_url(mime, base64) })),
            Part::Pdf { name, base64 } => content.push(json!({
                "type": "input_file",
                "filename": name,
                "file_data": data_url("application/pdf", base64),
            })),
            Part::ToolCall(call) => {
                flush(&mut content, &mut out);
                out.push(json!({ "type": "function_call", "call_id": call.id, "name": call.name, "arguments": call.arguments() }));
            }
            Part::ToolResult { id, output, .. } => {
                flush(&mut content, &mut out);
                out.push(json!({ "type": "function_call_output", "call_id": id, "output": output }));
            }
            Part::Opaque { wire: Wire::OpenAiResponses, model: made_by, data } if made_by == model => {
                flush(&mut content, &mut out);
                out.push(data.clone());
            }
            Part::Opaque { .. } => {}
        }
    }
    flush(&mut content, &mut out);
    out
}

pub fn body(request: &Request) -> Value {
    let mut body = json!({
        "model": request.model,
        "instructions": request.system,
        "input": request.turns.iter().flat_map(|t| items(t, request.model)).collect::<Vec<_>>(),
        "store": false,
    });
    if let Some(tools) = request.tools {
        body["tools"] = tools
            .iter()
            .map(|t| json!({ "type": "function", "name": t.name, "description": t.description, "parameters": t.schema }))
            .collect();
    }
    body
}

pub fn reply_text(response: &Value) -> Result<String, CallError> {
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::Unexpected("no output".into()))?;
    let mut text = Vec::new();
    for message in output.iter().filter(|o| o.get("type").and_then(Value::as_str) == Some("message")) {
        for part in message.get("content").and_then(Value::as_array).into_iter().flatten() {
            match part.get("type").and_then(Value::as_str) {
                Some("output_text") => text.extend(part.get("text").and_then(Value::as_str)),
                Some("refusal") => {
                    let why = part.get("refusal").and_then(Value::as_str).unwrap_or("The model declined.");
                    return Err(CallError::Declined(why.to_string()));
                }
                _ => {}
            }
        }
    }
    let text = text.join("");
    if text.trim().is_empty() && response.get("status").and_then(Value::as_str) == Some("incomplete") {
        let reason = response
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(CallError::Declined(format!("The answer stopped early ({reason}).")));
    }
    Ok(text)
}

/// An agent turn: reasoning items (kept for this model), text and tool calls.
pub fn reply(response: &Value, model: &str) -> Result<Reply, CallError> {
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::Unexpected("no output".into()))?;
    let mut parts = Vec::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                parts.push(Part::Opaque { wire: Wire::OpenAiResponses, model: model.to_string(), data: item.clone() })
            }
            Some("message") => {
                let text = reply_text(&json!({ "output": [item] }))?;
                if !text.is_empty() {
                    parts.push(Part::text(text));
                }
            }
            Some("function_call") => {
                let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let (input, bad_arguments) = parse_arguments(arguments);
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(super::new_call_id);
                let name = item.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                parts.push(Part::ToolCall(ToolCall { id, name, input, bad_arguments, raw: None }));
            }
            _ => {}
        }
    }
    let incomplete = response.get("status").and_then(Value::as_str) == Some("incomplete");
    let stop = if incomplete {
        Stop::Truncated
    } else if parts.iter().any(|p| matches!(p, Part::ToolCall(_))) {
        Stop::ToolUse
    } else {
        Stop::Done
    };
    Ok(Reply { parts, stop })
}

pub async fn complete(client: &reqwest::Client, request: &Request<'_>) -> Result<Reply, CallError> {
    let mut call = client.post(format!("{}/responses", request.base_url)).json(&body(request));
    if let Some(key) = request.key {
        call = call.bearer_auth(key);
    }
    reply(&exchange(call).await?, request.model)
}

pub async fn send(client: &reqwest::Client, request: &Request<'_>) -> Result<String, CallError> {
    let mut call = client.post(format!("{}/responses", request.base_url)).json(&body(request));
    if let Some(key) = request.key {
        call = call.bearer_auth(key);
    }
    reply_text(&exchange(call).await?)
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
                    Part::Pdf { name: "spec.pdf".into(), base64: "UERG".into() },
                    Part::text("Review it"),
                ],
            },
            Turn { role: Role::Assistant, parts: vec![Part::text("Looks fine.")] },
            Turn { role: Role::User, parts: vec![Part::text("Sure?")] },
        ]
    }

    #[test]
    fn an_agent_turn_sends_back_reasoning_calls_and_outputs() {
        let (base, rx) = serve_sequence(vec![
            (200, r#"{"status":"completed","output":[{"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"enc"},{"type":"function_call","id":"fc_1","call_id":"call_A","name":"run_command","arguments":"{\"command\":\"ls\"}"}]}"#.into()),
            (200, r#"{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Two files."}]}]}"#.into()),
        ]);
        let tools = vec![ToolSpec { name: "run_command".into(), description: "Runs".into(), schema: json!({"type": "object"}) }];
        let mut history = vec![Turn { role: Role::User, parts: vec![Part::text("what is here?")] }];
        let req = Request { base_url: &base, key: Some("sk"), model: "gpt-5", system: "s", turns: &history, reads_pdf: true, tools: Some(&tools) };
        let first = block_on(complete(&client(), &req)).unwrap();
        assert_eq!(first.stop, Stop::ToolUse);
        assert_eq!(first.calls().next().unwrap().input, json!({"command": "ls"}));
        let sent = rx.recv().unwrap().json();
        assert_eq!(sent["tools"][0], json!({"type": "function", "name": "run_command", "description": "Runs", "parameters": {"type": "object"}}));
        assert_eq!(sent["store"], false);

        history.push(Turn { role: Role::Assistant, parts: first.parts });
        history.push(Turn { role: Role::User, parts: vec![Part::ToolResult { id: "call_A".into(), output: "a b".into(), is_error: false }] });
        let req = Request { base_url: &base, key: Some("sk"), model: "gpt-5", system: "s", turns: &history, reads_pdf: true, tools: Some(&tools) };
        assert_eq!(block_on(complete(&client(), &req)).unwrap().text(), "Two files.");
        let input = rx.recv().unwrap().json()["input"].clone();
        assert_eq!(input[1]["encrypted_content"], "enc");
        assert_eq!(input[2], json!({"type": "function_call", "call_id": "call_A", "name": "run_command", "arguments": "{\"command\":\"ls\"}"}));
        assert_eq!(input[3], json!({"type": "function_call_output", "call_id": "call_A", "output": "a b"}));

        // Another model never gets this one's reasoning.
        let other = Request { base_url: "x", key: None, model: "gpt-5-mini", system: "s", turns: &history, reads_pdf: true, tools: Some(&tools) };
        assert_eq!(body(&other)["input"][1]["type"], "function_call");
    }

    #[test]
    fn history_becomes_input_items_and_nothing_is_stored() {
        let turns = turns();
        let request = Request { base_url: "x", key: None, model: "gpt-5-codex", system: "be nice", turns: &turns, reads_pdf: true, tools: None };
        let body = body(&request);
        assert_eq!(body["instructions"], "be nice");
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][0]["content"][0]["type"], "input_file");
        assert_eq!(body["input"][0]["content"][0]["file_data"], "data:application/pdf;base64,UERG");
        assert_eq!(body["input"][1], json!({ "role": "assistant", "content": "Looks fine." }));
        assert_eq!(body["input"][2], json!({ "role": "user", "content": "Sure?" }));
    }

    #[test]
    fn a_reply_is_read_over_http_past_reasoning_items() {
        let (base, rx) = serve_once(
            200,
            r#"{"status":"completed","error":null,"output":[{"type":"reasoning","summary":[]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"All good.","annotations":[]}]}]}"#,
        );
        let turns = turns();
        let request = Request { base_url: &base, key: Some("sk-o"), model: "gpt-5", system: "s", turns: &turns, reads_pdf: true, tools: None };
        assert_eq!(block_on(send(&client(), &request)).unwrap(), "All good.");
        let got = rx.recv().unwrap();
        assert_eq!(got.request_line, "POST /responses HTTP/1.1");
        assert_eq!(got.header("authorization"), Some("Bearer sk-o"));
    }

    #[test]
    fn refusals_and_cut_answers_are_reported() {
        let refused = json!({"output":[{"type":"message","content":[{"type":"refusal","refusal":"Can't help."}]}]});
        assert_eq!(reply_text(&refused), Err(CallError::Declined("Can't help.".into())));
        let cut = json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[]});
        assert!(matches!(reply_text(&cut), Err(CallError::Declined(m)) if m.contains("max_output_tokens")));
        assert!(matches!(reply_text(&json!({})), Err(CallError::Unexpected(_))));
    }
}
