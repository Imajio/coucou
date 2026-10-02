// Sending a dropped file by email, from the island's mail card. Nothing is sent
// without the Send click, as on macOS.
//
// - With a Resend key and a sender address saved, Resend sends it, file attached.
// - Otherwise the default mail app opens a new message, file attached, through
//   Simple MAPI (Outlook, Thunderbird, eM Client…); the user sends it there.
// - Without such an app, a mailto: link opens the message and Explorer shows the
//   file to attach by hand.

use serde_json::{json, Value};

use crate::{llm, platform, secrets};

const RESEND_ENDPOINT: &str = "https://api.resend.com/emails";
/// Resend's own ceiling for a message is 40 MB once encoded.
const MAX_ATTACHMENT: u64 = 25 * 1024 * 1024;

/// One plain address: something@domain.tld, nothing that could smuggle a header.
pub fn valid_address(to: &str) -> bool {
    let to = to.trim();
    let Some((local, domain)) = to.split_once('@') else { return false };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !to.chars().any(|c| c.is_whitespace() || c.is_control() || "<>,;\"".contains(c))
}

/// The JSON Resend takes; `attachment` is (file name, bytes).
pub fn resend_body(from: &str, to: &str, subject: &str, body: &str, attachment: Option<(&str, &[u8])>) -> Value {
    let mut payload = json!({
        "from": from,
        "to": [to],
        "subject": subject,
        // Resend refuses an empty text part.
        "text": if body.trim().is_empty() { " " } else { body },
    });
    if let Some((name, bytes)) = attachment {
        payload["attachments"] = json!([{ "filename": name, "content": llm::base64(bytes) }]);
    }
    payload
}

/// `mailto:` with the subject and body encoded, for when no mail app takes MAPI.
pub fn mailto_url(to: &str, subject: &str, body: &str) -> String {
    fn encode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    format!("mailto:{}?subject={}&body={}", encode(to.trim()), encode(subject), encode(body))
}

/// How the message left: "sent" (Resend), "compose" (the mail app has it open),
/// "cancelled" (closed there unsent) or "mailto" (attach the file by hand).
#[tauri::command]
pub async fn mail_send(to: String, subject: String, body: String, path: Option<String>) -> Result<String, String> {
    let to = to.trim().to_string();
    if !valid_address(&to) {
        return Err("That doesn't look like an email address.".into());
    }
    let path = path.filter(|p| !p.is_empty());
    let name = path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let subject = if subject.trim().is_empty() { name.clone() } else { subject };

    match (secrets::get("resend-api-key"), secrets::get("resend-from")) {
        (Some(key), Some(from)) => send_with_resend(&key, &from, &to, &subject, &body, path.as_deref(), &name).await,
        (Some(_), None) => Err("Add the Resend sender address in Settings → Integrations to send with Resend.".into()),
        _ => {
            let outcome = tauri::async_runtime::spawn_blocking(move || {
                platform::compose_mail(&to, &subject, &body, path.as_deref())
                    .unwrap_or_else(|| {
                        platform::open_url(&mailto_url(&to, &subject, &body));
                        if let Some(dir) = path.as_deref().and_then(|p| std::path::Path::new(p).parent()) {
                            platform::reveal_folder(&dir.to_string_lossy());
                        }
                        "mailto".to_string()
                    })
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(outcome)
        }
    }
}

async fn send_with_resend(
    key: &str,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
    path: Option<&str>,
    name: &str,
) -> Result<String, String> {
    let bytes = match path {
        Some(p) => {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            if size > MAX_ATTACHMENT {
                return Err("The file is too big to send by email (25 MB at most).".into());
            }
            Some(std::fs::read(p).map_err(|e| format!("Can't read the file: {e}"))?)
        }
        None => None,
    };
    let payload = resend_body(from, to, subject, body, bytes.as_deref().map(|b| (name, b)));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(RESEND_ENDPOINT)
        .bearer_auth(key)
        .json(&payload)
        .send()
        .await
        .map_err(|_| "Can't reach Resend. Check the connection.".to_string())?;
    let status = response.status();
    if status.is_success() {
        return Ok("sent".into());
    }
    let text = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| llm::error_message(&v))
        .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
    Err(format!("Resend refused it: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_addresses_go_through() {
        assert!(valid_address("ada@example.com"));
        assert!(valid_address("  first.last+tag@sub.example.org "));
        for bad in ["", "ada", "ada@", "@example.com", "ada@example", "a b@example.com", "a@b.com, c@d.com", "x@y.com\nBcc: z@w.com", "<a@b.com>"] {
            assert!(!valid_address(bad), "{bad:?}");
        }
    }

    #[test]
    fn resend_gets_the_file_attached_and_never_an_empty_text() {
        let v = resend_body("me@example.com", "you@example.com", "Report", "", Some(("r.pdf", b"PDF")));
        assert_eq!(v["to"][0], "you@example.com");
        assert_eq!(v["text"], " ");
        assert_eq!(v["attachments"][0]["filename"], "r.pdf");
        assert_eq!(v["attachments"][0]["content"], "UERG");
        assert!(resend_body("a@b.co", "c@d.co", "s", "hi", None).get("attachments").is_none());
    }

    #[test]
    fn mailto_links_keep_spaces_and_lines_intact() {
        assert_eq!(
            mailto_url("you@example.com", "Q3 report", "Hi,\nsee attached & thanks"),
            "mailto:you%40example.com?subject=Q3%20report&body=Hi%2C%0Asee%20attached%20%26%20thanks"
        );
    }
}
