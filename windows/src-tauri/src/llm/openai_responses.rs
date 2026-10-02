// OpenAI's Responses API: the one OpenAI recommends, and the only one that
// serves its Codex models. Requests are sent with `store: false`, so OpenAI
// keeps no copy of the conversation for later retrieval.

use serde_json::{json, Value};

use super::{exchange, CallError, Part, Request, Role, Turn};

fn data_url(mime: &str, base64: &str) -> String {
    format!("data:{mime};base64,{base64}")
}

fn item(turn: &Turn) -> Value {
    if turn.role == Role::Assistant {
        return json!({ "role": "assistant", "content": turn.text() });
    }
    if turn.parts.iter().all(|p| matches!(p, Part::Text(_))) {
        return json!({ "role": "user", "content": turn.text() });
    }
    let content: Vec<Value> = turn
        .parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => json!({ "type": "input_text", "text": text }),
            Part::Image { mime, base64 } => json!({ "type": "input_image", "image_url": data_url(mime, base64) }),
            Part::Pdf { name, base64 } => json!({
                "type": "input_file",
                "filename": name,
                "file_data": data_url("application/pdf", base64),
            }),
        })
        .collect();
    json!({ "role": "user", "content": content })
}

pub fn body(request: &Request) -> Value {
    json!({
        "model": request.model,
        "instructions": request.system,
        "input": request.turns.iter().map(item).collect::<Vec<_>>(),
        "store": false,
    })
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

pub async fn send(client: &reqwest::Client, request: &Request<'_>) -> Result<String, CallError> {
    let mut call = client.post(format!("{}/responses", request.base_url)).json(&body(request));
    if let Some(key) = request.key {
        call = call.bearer_auth(key);
    }
    reply_text(&exchange(call).await?)
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
                    Part::Pdf { name: "spec.pdf".into(), base64: "UERG".into() },
                    Part::Text("Review it".into()),
                ],
            },
            Turn { role: Role::Assistant, parts: vec![Part::Text("Looks fine.".into())] },
            Turn { role: Role::User, parts: vec![Part::Text("Sure?".into())] },
        ]
    }

    #[test]
    fn history_becomes_input_items_and_nothing_is_stored() {
        let turns = turns();
        let request = Request { base_url: "x", key: None, model: "gpt-5-codex", system: "be nice", turns: &turns, reads_pdf: true };
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
        let request = Request { base_url: &base, key: Some("sk-o"), model: "gpt-5", system: "s", turns: &turns, reads_pdf: true };
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
