// OpenAI's Chat Completions format, which nearly every other vendor speaks:
// Gemini (through Google's OpenAI-compatible endpoint), OpenRouter, Ollama,
// LM Studio, Groq, DeepSeek, Mistral, xAI and the like.

use serde_json::{json, Value};

use super::{exchange, pdf_note, CallError, ModelInfo, Part, Request, Role, Turn};

fn data_url(mime: &str, base64: &str) -> String {
    format!("data:{mime};base64,{base64}")
}

fn message(turn: &Turn, reads_pdf: bool) -> Value {
    if turn.role == Role::Assistant {
        return json!({ "role": "assistant", "content": turn.text() });
    }
    // Plain text stays a plain string: the form every server accepts.
    if turn.parts.iter().all(|p| matches!(p, Part::Text(_))) {
        return json!({ "role": "user", "content": turn.text() });
    }
    let content: Vec<Value> = turn
        .parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => json!({ "type": "text", "text": text }),
            Part::Image { mime, base64 } => {
                json!({ "type": "image_url", "image_url": { "url": data_url(mime, base64) } })
            }
            Part::Pdf { name, base64 } if reads_pdf => json!({
                "type": "file",
                "file": { "filename": name, "file_data": data_url("application/pdf", base64) },
            }),
            Part::Pdf { name, .. } => json!({ "type": "text", "text": pdf_note(name) }),
        })
        .collect();
    json!({ "role": "user", "content": content })
}

pub fn body(request: &Request) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.turns.iter().map(|t| message(t, request.reads_pdf)));
    // No max_tokens: each server's default fits, and some reasoning models
    // reject the parameter outright.
    json!({ "model": request.model, "messages": messages })
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
    use super::super::testing::{block_on, client, serve_once};
    use super::*;

    fn turns() -> Vec<Turn> {
        vec![
            Turn {
                role: Role::User,
                parts: vec![
                    Part::Image { mime: "image/png", base64: "iVBO".into() },
                    Part::Pdf { name: "a.pdf".into(), base64: "UERG".into() },
                    Part::Text("What is this?".into()),
                ],
            },
            Turn { role: Role::Assistant, parts: vec![Part::Text("A chart.".into())] },
            Turn { role: Role::User, parts: vec![Part::Text("Thanks".into())] },
        ]
    }

    fn request<'a>(base: &'a str, key: Option<&'a str>, turns: &'a [Turn], reads_pdf: bool) -> Request<'a> {
        Request { base_url: base, key, model: "gemini-2.5-flash", system: "be nice", turns, reads_pdf }
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
