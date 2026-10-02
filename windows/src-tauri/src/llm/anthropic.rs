// Claude, on its own Messages API: web search, PDFs and images, and the
// server-side fallback that answers a policy decline with another model in the
// same call instead of a dead end.

use serde_json::{json, Value};

use super::{exchange, CallError, ModelInfo, Part, Request, Role, Turn};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

const ANTHROPIC_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Room for adaptive thinking and a full answer, short of HTTP timeouts.
const MAX_TOKENS: u32 = 16_000;

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

fn message(turn: &Turn) -> Value {
    match turn.role {
        Role::Assistant => json!({ "role": "assistant", "content": turn.text() }),
        Role::User => {
            let content: Vec<Value> = turn
                .parts
                .iter()
                .map(|part| match part {
                    Part::Text(text) => json!({ "type": "text", "text": text }),
                    Part::Image { mime, base64 } => json!({
                        "type": "image",
                        "source": { "type": "base64", "media_type": mime, "data": base64 },
                    }),
                    Part::Pdf { base64, .. } => json!({
                        "type": "document",
                        "source": { "type": "base64", "media_type": "application/pdf", "data": base64 },
                    }),
                })
                .collect();
            json!({ "role": "user", "content": content })
        }
    }
}

pub fn body(request: &Request) -> Value {
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
    use super::super::testing::{block_on, client, serve_once};
    use super::*;

    fn turns() -> Vec<Turn> {
        vec![
            Turn {
                role: Role::User,
                parts: vec![
                    Part::Pdf { name: "a.pdf".into(), base64: "UERG".into() },
                    Part::Text("File: a.pdf".into()),
                    Part::Text("Summarise".into()),
                ],
            },
            Turn { role: Role::Assistant, parts: vec![Part::Text("It is about X.".into())] },
            Turn { role: Role::User, parts: vec![Part::Text("And Y?".into())] },
        ]
    }

    fn request<'a>(base: &'a str, model: &'a str, turns: &'a [Turn]) -> Request<'a> {
        Request { base_url: base, key: Some("sk-test"), model, system: "be nice", turns, reads_pdf: true }
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
