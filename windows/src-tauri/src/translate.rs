// Translator tab, official half: Google Cloud Translation (the v2 "Basic" API)
// with the user's own API key, kept in the OS credential store like every other
// key. Sent only when the user asks for a translation.
//
// Without a key the island uses Google Translate's free web endpoint, and calls
// it itself (src/core/translate.ts): that endpoint answers browsers but turns
// the app's own HTTP client away as a bot.

use serde::Serialize;
use serde_json::{json, Value};

use crate::secrets;

/// Credential store entry holding the Google Cloud API key.
pub const KEY: &str = "google-translate-api-key";

const ENDPOINT: &str = "https://translation.googleapis.com/language/translate/v2";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Longest text sent in one go. A stray paste of a whole book should not turn
/// into a surprise on someone's Google Cloud bill.
pub const MAX_CHARS: usize = 5000;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    pub text: String,
    /// Language Google recognised when the source was "auto".
    pub detected_source: Option<String>,
}

/// `en`, `pt`, `zh-CN`, `zh-TW`, `auto`: letters and one optional region.
fn valid_code(code: &str) -> bool {
    let mut parts = code.split('-');
    let lang = parts.next().unwrap_or("");
    let region = parts.next();
    parts.next().is_none()
        && (2..=4).contains(&lang.len())
        && lang.chars().all(|c| c.is_ascii_alphabetic())
        && region.is_none_or(|r| (2..=4).contains(&r.len()) && r.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The JSON sent to Google; `source` "auto" leaves detection to Google.
pub fn request_body(text: &str, source: &str, target: &str) -> Value {
    let mut body = json!({ "q": text, "target": target, "format": "text" });
    if source != "auto" {
        body["source"] = json!(source);
    }
    body
}

/// Reads Google's answer, turning its common refusals into something a person
/// can act on.
pub fn parse_response(status: u16, body: &str) -> Result<Translation, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| format!("Google Translate answered with an error ({status})."))?;
    if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
        return Err(explain(status, message));
    }
    let first = value
        .pointer("/data/translations/0")
        .ok_or_else(|| "Google Translate sent no translation.".to_string())?;
    let text = first
        .get("translatedText")
        .and_then(Value::as_str)
        .ok_or_else(|| "Google Translate sent no translation.".to_string())?;
    Ok(Translation {
        text: text.to_string(),
        detected_source: first
            .get("detectedSourceLanguage")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn explain(status: u16, message: &str) -> String {
    let lower = message.to_lowercase();
    if lower.contains("api key not valid") || lower.contains("api_key_invalid") {
        "Google rejected the API key. Check it in Settings → Translator.".into()
    } else if lower.contains("has not been used") || lower.contains("is disabled") {
        "The Cloud Translation API is off for this key's project. Turn it on in the Google Cloud console.".into()
    } else if lower.contains("billing") {
        "The key's Google Cloud project needs billing turned on for Cloud Translation.".into()
    } else if status == 429 || lower.contains("quota") || lower.contains("rate limit") {
        "Google's translation quota is used up for now. Try again later.".into()
    } else if lower.contains("bad language pair") || lower.contains("invalid value") {
        "Google can't translate between these two languages.".into()
    } else {
        message.to_string()
    }
}

#[tauri::command]
pub async fn translate(text: String, source: String, target: String) -> Result<Translation, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Translation::default());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("That is over {MAX_CHARS} characters. Translate it in parts."));
    }
    if !valid_code(&target) || target == "auto" || !(source == "auto" || valid_code(&source)) {
        return Err("Unknown language.".into());
    }
    let key = secrets::get(KEY).ok_or("No Google Cloud Translation key is saved.")?;
    let client = reqwest::Client::builder().timeout(TIMEOUT).build().map_err(|e| e.to_string())?;
    translate_with_key(&client, &key, text, &source, &target).await
}

fn unreachable(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Google Translate did not answer in time.".to_string()
    } else {
        "Can't reach Google Translate. Check the connection.".to_string()
    }
}

/// The official Cloud Translation API, with the user's key.
async fn translate_with_key(
    client: &reqwest::Client,
    key: &str,
    text: &str,
    source: &str,
    target: &str,
) -> Result<Translation, String> {
    // The key goes in a header, never in the URL, so it can't end up in a log.
    let response = client
        .post(ENDPOINT)
        .header("X-Goog-Api-Key", key)
        .json(&request_body(text, source, target))
        .send()
        .await
        .map_err(unreachable)?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| e.to_string())?;
    parse_response(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_translation_is_read_with_the_detected_language() {
        let body = r#"{"data":{"translations":[{"translatedText":"Bonjour","detectedSourceLanguage":"en"}]}}"#;
        assert_eq!(
            parse_response(200, body),
            Ok(Translation { text: "Bonjour".into(), detected_source: Some("en".into()) })
        );
    }

    #[test]
    fn a_translation_without_detection_is_read_too() {
        let body = r#"{"data":{"translations":[{"translatedText":"Привет"}]}}"#;
        assert_eq!(parse_response(200, body).unwrap().detected_source, None);
    }

    #[test]
    fn google_refusals_become_advice() {
        let bad_key = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key."}}"#;
        assert!(parse_response(400, bad_key).unwrap_err().contains("Settings"));
        let disabled = r#"{"error":{"code":403,"message":"Cloud Translation API has not been used in project 1 before or it is disabled."}}"#;
        assert!(parse_response(403, disabled).unwrap_err().contains("Google Cloud console"));
        let other = r#"{"error":{"code":500,"message":"Backend error"}}"#;
        assert_eq!(parse_response(500, other).unwrap_err(), "Backend error");
        assert!(parse_response(502, "<html>").is_err());
        assert!(parse_response(200, r#"{"data":{"translations":[]}}"#).is_err());
    }

    #[test]
    fn auto_source_is_left_to_google() {
        assert_eq!(request_body("hi", "auto", "fr"), json!({ "q": "hi", "target": "fr", "format": "text" }));
        assert_eq!(request_body("hi", "en", "fr")["source"], "en");
    }

    #[test]
    fn language_codes_are_checked() {
        for ok in ["en", "fr", "zh-CN", "zh-TW", "auto", "haw"] {
            assert!(valid_code(ok), "{ok}");
        }
        for bad in ["", "e", "english!", "en-US-x", "../x", "en US"] {
            assert!(!valid_code(bad), "{bad}");
        }
    }
}

