// Chat with any AI provider: Claude, OpenAI (GPT and Codex models), Gemini,
// OpenRouter, a local Ollama, or any OpenAI-compatible endpoint.
//
// Three pieces, like Spring AI's ChatClient in miniature:
// - a provider table: adding a provider is one entry, not new code;
// - a provider-neutral conversation, so switching provider or model in the
//   middle of a chat just sends the same history in the other format;
// - one adapter per wire format. Almost every vendor speaks OpenAI's Chat
//   Completions, so three formats cover all of them: Anthropic Messages (Claude,
//   with web search and PDFs), OpenAI Responses (OpenAI, which serves its Codex
//   models only there) and Chat Completions (everything else).
//
// Everything runs here, never in the island: API keys stay in the OS credential
// store and file bytes never cross the IPC boundary.
//
// The same adapters also run the agent sessions (crate::agent): `complete` sends
// a conversation with client tools and returns the model's text and tool calls,
// in the same provider-neutral parts the history is kept in.

mod anthropic;
mod openai_chat;
mod openai_responses;

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::secrets;

/// One request may take a while: reasoning models think before they answer.
const TIMEOUT: Duration = Duration::from_secs(120);
/// An agent turn can write a whole file after thinking it through.
const AGENT_TIMEOUT: Duration = Duration::from_secs(600);
const MODELS_TIMEOUT: Duration = Duration::from_secs(15);
/// Text and code files are inlined; anything larger is skipped.
const MAX_INLINE_TEXT: u64 = 200_000;

pub const DEFAULT_PROVIDER: &str = "anthropic";
pub const DEFAULT_MODEL: &str = anthropic::DEFAULT_MODEL;

// ── Providers ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Wire {
    Anthropic,
    OpenAiResponses,
    OpenAiChat,
}

#[derive(Debug)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub wire: Wire,
    /// Empty for the custom endpoint, whose address lives in the settings.
    pub base_url: &'static str,
    /// Credential store entry holding the key, if the provider takes one.
    pub key: Option<&'static str>,
    pub key_required: bool,
    /// Reads PDFs itself. Elsewhere a PDF becomes a note saying it is there.
    pub reads_pdf: bool,
    /// Searches the web on its own (Claude's server-side web search tool).
    pub web_search: bool,
    /// Where to get a key, or the app itself for a local server.
    pub key_url: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "anthropic",
        name: "Claude",
        wire: Wire::Anthropic,
        base_url: "https://api.anthropic.com/v1",
        key: Some("anthropic-api-key"),
        key_required: true,
        reads_pdf: true,
        web_search: true,
        key_url: "https://console.anthropic.com/settings/keys",
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        wire: Wire::OpenAiResponses,
        base_url: "https://api.openai.com/v1",
        key: Some("openai-api-key"),
        key_required: true,
        reads_pdf: true,
        web_search: false,
        key_url: "https://platform.openai.com/api-keys",
    },
    Provider {
        id: "google",
        name: "Gemini",
        wire: Wire::OpenAiChat,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        key: Some("google-api-key"),
        key_required: true,
        reads_pdf: false,
        web_search: false,
        key_url: "https://aistudio.google.com/apikey",
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        wire: Wire::OpenAiChat,
        base_url: "https://openrouter.ai/api/v1",
        key: Some("openrouter-api-key"),
        key_required: true,
        reads_pdf: true,
        web_search: false,
        key_url: "https://openrouter.ai/keys",
    },
    Provider {
        id: "ollama",
        name: "Ollama",
        wire: Wire::OpenAiChat,
        base_url: "http://localhost:11434/v1",
        key: None,
        key_required: false,
        reads_pdf: false,
        web_search: false,
        key_url: "https://ollama.com/download",
    },
    Provider {
        id: "custom",
        name: "Custom endpoint",
        wire: Wire::OpenAiChat,
        base_url: "",
        key: Some("custom-api-key"),
        key_required: false,
        reads_pdf: false,
        web_search: false,
        key_url: "",
    },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// What the island may know about a provider: never the key itself.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub key_name: Option<&'static str>,
    pub key_required: bool,
    pub has_key: bool,
    pub reads_pdf: bool,
    pub web_search: bool,
    pub key_url: &'static str,
}

pub fn providers() -> Vec<ProviderInfo> {
    PROVIDERS
        .iter()
        .map(|p| ProviderInfo {
            id: p.id,
            name: p.name,
            key_name: p.key,
            key_required: p.key_required,
            has_key: p.key.is_some_and(secrets::present),
            reads_pdf: p.reads_pdf,
            web_search: p.web_search,
            key_url: p.key_url,
        })
        .collect()
}

/// A provider ready to be called: its address and key resolved.
pub struct Target {
    pub provider: &'static Provider,
    pub base_url: String,
    pub key: Option<String>,
}

impl Target {
    /// `custom_base_url` is the settings' address for the custom endpoint.
    pub fn resolve(provider_id: &str, custom_base_url: &str) -> Result<Self, String> {
        let provider = provider(provider_id).ok_or_else(|| format!("Unknown provider {provider_id}."))?;
        let base_url = if provider.base_url.is_empty() {
            normalize_base_url(custom_base_url)
                .ok_or("Set the address of your OpenAI-compatible endpoint in Settings → AI providers.")?
        } else {
            provider.base_url.to_string()
        };
        let key = provider.key.and_then(secrets::get);
        if provider.key_required && key.is_none() {
            return Err(format!("Add your {} API key in Settings → AI providers.", provider.name));
        }
        Ok(Self { provider, base_url, key })
    }
}

/// `http(s)://host[:port]/path`, without a trailing slash; anything else is refused.
pub fn normalize_base_url(raw: &str) -> Option<String> {
    let url = raw.trim().trim_end_matches('/');
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(url.to_string())
}

// ── Conversation ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a message, in no provider's format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Part {
    Text { text: String },
    Image { mime: String, base64: String },
    Pdf { name: String, base64: String },
    /// The model asks for a tool to run (assistant turns).
    ToolCall(ToolCall),
    /// What a tool gave back, in the user turn right after its call.
    #[serde(rename_all = "camelCase")]
    ToolResult { id: String, output: String, is_error: bool },
    /// Provider data that goes back unchanged to the wire that made it and
    /// nowhere else: Claude's thinking blocks, OpenAI's reasoning items.
    Opaque { wire: Wire, model: String, data: Value },
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Part::Text { text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Always an object; `{}` when the model sent nothing usable.
    pub input: Value,
    /// The arguments as sent, when they were not valid JSON: the call is
    /// answered with an error and replayed as it came.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bad_arguments: Option<String>,
    /// The provider's own call object (Chat Completions), replayed as is so
    /// fields such as Gemini's thought signatures go back with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// A client tool the model may call.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the input object.
    pub schema: Value,
}

/// Why the model stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Finished its answer.
    Done,
    /// Waits for the results of its tool calls.
    ToolUse,
    /// Ran out of output tokens: any tool call in it may be cut off.
    Truncated,
}

/// An agent turn: the assistant's parts, in order, and why it stopped.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub parts: Vec<Part>,
    pub stop: Stop,
}

impl Reply {
    pub fn calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.parts.iter().filter_map(|p| match p {
            Part::ToolCall(c) => Some(c),
            _ => None,
        })
    }

    pub fn text(&self) -> String {
        Turn { role: Role::Assistant, parts: self.parts.clone() }.text()
    }
}

/// A call id for servers that send none, unique within the app's run.
pub fn new_call_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("call_{stamp:x}_{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Tool arguments as the OpenAI formats send them: a JSON string.
pub fn parse_arguments(raw: &str) -> (Value, Option<String>) {
    if raw.trim().is_empty() {
        return (Value::Object(Default::default()), None);
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => (Value::Object(map), None),
        Ok(Value::Null) => (Value::Object(Default::default()), None),
        _ => (Value::Object(Default::default()), Some(raw.to_string())),
    }
}

impl ToolCall {
    /// The arguments to send back as a string, exactly as they came if they
    /// could not be read.
    pub fn arguments(&self) -> String {
        self.bad_arguments.clone().unwrap_or_else(|| self.input.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Turn {
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Only text: the plain-string form every server accepts.
    pub fn is_plain_text(&self) -> bool {
        self.parts.iter().all(|p| matches!(p, Part::Text { .. }))
    }
}

/// The chat's history, kept provider-neutral so any model can pick it up.
#[derive(Default)]
pub struct Conversation {
    turns: Mutex<Vec<Turn>>,
}

impl Conversation {
    pub fn reset(&self) {
        self.turns.lock().unwrap().clear();
    }

    fn is_empty(&self) -> bool {
        self.turns.lock().unwrap().is_empty()
    }

    fn push(&self, turn: Turn) {
        self.turns.lock().unwrap().push(turn);
    }

    fn pop(&self) {
        self.turns.lock().unwrap().pop();
    }

    fn snapshot(&self) -> Vec<Turn> {
        self.turns.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatContext {
    File { name: String, path: String },
    Window { app_name: String, title: String, url: Option<String> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub text: String,
}

/// What every adapter needs for one call.
pub struct Request<'a> {
    pub base_url: &'a str,
    pub key: Option<&'a str>,
    pub model: &'a str,
    pub system: &'a str,
    pub turns: &'a [Turn],
    pub reads_pdf: bool,
    /// An agent turn's client tools. `None` is the chat: Claude's own web
    /// search and fallback model instead of client tools.
    pub tools: Option<&'a [ToolSpec]>,
}

const SYSTEM_PROMPT: &str = "You are Mochi, a personal AI assistant living at the top of the user's screen. \
You can help with absolutely anything: research, coding, finding places, recommendations, tasks, questions. \
Respond in the user's language. Be thorough and complete, use as much detail as the task requires. \
No markdown formatting (no **, no ##, no bullet dashes). Use plain text with line breaks.";

const WEB_SEARCH_PROMPT: &str = " You have web search access: use it for anything recent or that you are unsure about.";

pub fn system_prompt(web_search: bool) -> String {
    let mut prompt = SYSTEM_PROMPT.to_string();
    if web_search {
        prompt.push_str(WEB_SEARCH_PROMPT);
    }
    prompt
}

/// One chat turn with whichever provider and model are chosen. Returns the
/// assistant's text, or a message the island shows in the note view.
pub async fn send(
    conversation: &Conversation,
    target: &Target,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    if model.trim().is_empty() {
        return Err(format!("Pick a {} model for the chat first.", target.provider.name));
    }

    let mut parts = Vec::new();
    // File or window context rides along with the first message only.
    if conversation.is_empty() {
        parts.extend(context_parts(context.as_ref()));
    }
    parts.push(Part::text(query));
    conversation.push(Turn { role: Role::User, parts });

    let turns = conversation.snapshot();
    let system = system_prompt(target.provider.web_search);
    let request = Request {
        base_url: &target.base_url,
        key: target.key.as_deref(),
        model,
        system: &system,
        turns: &turns,
        reads_pdf: target.provider.reads_pdf,
        tools: None,
    };
    let client = client(TIMEOUT)?;
    let result = match target.provider.wire {
        Wire::Anthropic => anthropic::send(&client, &request).await,
        Wire::OpenAiResponses => openai_responses::send(&client, &request).await,
        Wire::OpenAiChat => openai_chat::send(&client, &request).await,
    }
    .map_err(|e| e.describe(target.provider));

    match result {
        Ok(text) if !text.trim().is_empty() => {
            let text = text.trim().to_string();
            conversation.push(Turn { role: Role::Assistant, parts: vec![Part::text(text.clone())] });
            Ok(ChatReply { text })
        }
        Ok(_) => {
            conversation.pop();
            Err("The model sent an empty answer. Try again or pick another model.".into())
        }
        Err(e) => {
            // Keep the history consistent with what the model saw.
            conversation.pop();
            Err(e)
        }
    }
}

/// One agent turn: the whole conversation, the role's system prompt and its
/// tools. The caller keeps the history and runs the tools.
pub async fn complete(
    target: &Target,
    model: &str,
    system: &str,
    turns: &[Turn],
    tools: &[ToolSpec],
) -> Result<Reply, String> {
    if model.trim().is_empty() {
        return Err(format!("Pick a {} model for this session first.", target.provider.name));
    }
    let request = Request {
        base_url: &target.base_url,
        key: target.key.as_deref(),
        model,
        system,
        turns,
        reads_pdf: target.provider.reads_pdf,
        tools: Some(tools),
    };
    let client = client(AGENT_TIMEOUT)?;
    match target.provider.wire {
        Wire::Anthropic => anthropic::complete(&client, &request).await,
        Wire::OpenAiResponses => openai_responses::complete(&client, &request).await,
        Wire::OpenAiChat => openai_chat::complete(&client, &request).await,
    }
    .map_err(|e| e.describe(target.provider))
}

// ── Models ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
}

/// The models a provider offers right now, asked of the provider itself so new
/// ones show up without an app update.
pub async fn models(target: &Target) -> Result<Vec<ModelInfo>, String> {
    let client = client(MODELS_TIMEOUT)?;
    let key = target.key.as_deref();
    let list = match target.provider.wire {
        Wire::Anthropic => anthropic::models(&client, &target.base_url, key).await,
        Wire::OpenAiResponses | Wire::OpenAiChat => {
            openai_chat::models(&client, &target.base_url, key)
                .await
                .map(|raw| filter_models(target.provider.id, raw))
        }
    }
    .map_err(|e| e.describe(target.provider))?;
    Ok(list)
}

/// Families that can't hold a chat: embeddings, speech, images, video…
pub fn filter_models(provider_id: &str, mut list: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let excluded: &[&str] = match provider_id {
        "openai" => &[
            "embed", "tts", "whisper", "dall-e", "audio", "realtime", "moderat", "transcribe",
            "image", "sora", "babbage", "davinci", "instruct", "computer-use", "search",
            "deep-research",
        ],
        "google" => &["embed", "imagen", "veo", "aqa", "tts", "audio", "live", "image"],
        _ => &["embed"],
    };
    for m in &mut list {
        if let Some(id) = m.id.strip_prefix("models/") {
            if m.label == m.id {
                m.label = id.to_string();
            }
            m.id = id.to_string();
        }
    }
    list.retain(|m| {
        let id = m.id.to_lowercase();
        !excluded.iter().any(|x| id.contains(x))
    });
    list
}

// ── Shared plumbing ───────────────────────────────────────────────────────────

/// How a call failed, before it is put in the provider's words.
#[derive(Debug, PartialEq)]
pub enum CallError {
    /// The request never got an answer.
    Network { timeout: bool },
    /// The provider answered with an error status and, usually, a message.
    Status { code: u16, message: String },
    /// The provider answered 200 but declined or stopped.
    Declined(String),
    /// An answer we can't read.
    Unexpected(String),
}

impl CallError {
    fn describe(self, provider: &Provider) -> String {
        match self {
            CallError::Network { timeout: true } => format!("{} did not answer in time.", provider.name),
            CallError::Network { timeout: false } if provider.id == "ollama" => {
                "Can't reach Ollama on this computer. Is it running?".into()
            }
            CallError::Network { timeout: false } => format!("Can't reach {}. Check the connection.", provider.name),
            CallError::Status { code: 401 | 403, message } => {
                format!("{} refused the key ({message}). Check it in Settings → AI providers.", provider.name)
            }
            CallError::Status { code: 404, message } => {
                format!("{}: {message} Pick another model.", provider.name)
            }
            CallError::Status { code: 429, message } => {
                format!("{} is rate-limiting or out of credit: {message}", provider.name)
            }
            CallError::Status { code, message } => format!("{} ({code}): {message}", provider.name),
            CallError::Declined(why) => why,
            CallError::Unexpected(what) => format!("{} sent an unexpected answer: {what}", provider.name),
        }
    }
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(timeout).build().map_err(|e| e.to_string())
}

/// Sends a prepared request and reads the JSON answer, or the error in it.
async fn exchange(request: reqwest::RequestBuilder) -> Result<Value, CallError> {
    let response = request
        .send()
        .await
        .map_err(|e| CallError::Network { timeout: e.is_timeout() })?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| CallError::Unexpected(e.to_string()))?;
    let value = serde_json::from_str::<Value>(&body).ok();
    if !(200..300).contains(&status) {
        let message = value
            .as_ref()
            .and_then(error_message)
            .unwrap_or_else(|| body.chars().take(200).collect::<String>().trim().to_string());
        return Err(CallError::Status { code: status, message });
    }
    let value = value.ok_or_else(|| CallError::Unexpected(body.chars().take(120).collect()))?;
    // Some gateways answer 200 with an error inside.
    if let Some(message) = error_message(&value) {
        return Err(CallError::Status { code: status, message });
    }
    Ok(value)
}

/// The error text in the shapes providers use: `{"error": {"message"}}`,
/// `[{"error": {...}}]` (Gemini), `{"error": "..."}` or `{"message": "..."}`.
pub fn error_message(value: &Value) -> Option<String> {
    let value = match value {
        Value::Array(items) => items.first()?,
        other => other,
    };
    let error = value.get("error").filter(|e| !e.is_null());
    let text = match error {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(e) => e.get("message").and_then(Value::as_str),
        None if value.get("choices").is_none() && value.get("output").is_none() && value.get("content").is_none() => {
            value.get("message").and_then(Value::as_str)
        }
        None => None,
    }?;
    Some(text.trim().to_string()).filter(|t| !t.is_empty())
}

/// The parts that describe what the chat is about: a dropped file, or a window.
fn context_parts(context: Option<&ChatContext>) -> Vec<Part> {
    match context {
        Some(ChatContext::File { name, path }) => {
            let mut parts = Vec::new();
            if let Some(part) = file_part(name, path) {
                parts.push(part);
            }
            parts.push(Part::text(format!("File: {name}")));
            parts
        }
        Some(ChatContext::Window { app_name, title, url }) => {
            let mut text = format!("Context: App: {app_name}, Window: {title}");
            if let Some(url) = url {
                text.push_str(&format!(", URL: {url}"));
            }
            vec![Part::text(text)]
        }
        None => Vec::new(),
    }
}

/// PDF and images travel as bytes; text and code are inlined.
fn file_part(name: &str, path: &str) -> Option<Part> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let image = match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    };
    if let Some(mime) = image {
        return Some(Part::Image { mime: mime.to_string(), base64: base64(&std::fs::read(path).ok()?) });
    }
    if ext == "pdf" {
        return Some(Part::Pdf { name: name.to_string(), base64: base64(&std::fs::read(path).ok()?) });
    }
    if std::fs::metadata(path).ok()?.len() > MAX_INLINE_TEXT {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    Some(Part::text(format!("File contents:\n{text}")))
}

/// What a model that can't read PDFs is told instead.
pub fn pdf_note(name: &str) -> String {
    format!("[The user attached the PDF \"{name}\", but this model can't read PDFs. Say so if the question needs it.]")
}

/// Small standalone base64 encoder: not worth another dependency.
/// Also used for Stripe's basic auth.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn every_provider_is_reachable_and_unique() {
        let mut ids: Vec<_> = PROVIDERS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PROVIDERS.len());
        assert_eq!(provider(DEFAULT_PROVIDER).unwrap().wire, Wire::Anthropic);
        for p in PROVIDERS {
            assert!(p.base_url.is_empty() || normalize_base_url(p.base_url).as_deref() == Some(p.base_url), "{}", p.id);
            assert!(!p.key_required || p.key.is_some(), "{}", p.id);
        }
    }

    #[test]
    fn custom_addresses_must_be_http() {
        assert_eq!(normalize_base_url(" http://localhost:1234/v1/ ").as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(normalize_base_url("https://api.groq.com/openai/v1").as_deref(), Some("https://api.groq.com/openai/v1"));
        for bad in ["", "localhost:1234", "ftp://x", "https://", "http://a b", "javascript:alert(1)"] {
            assert_eq!(normalize_base_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn web_search_is_only_promised_where_it_exists() {
        assert!(system_prompt(true).contains("web search"));
        assert!(!system_prompt(false).contains("web search"));
    }

    #[test]
    fn errors_are_read_in_every_shape() {
        assert_eq!(error_message(&json!({"error": {"message": "bad key"}})).as_deref(), Some("bad key"));
        assert_eq!(error_message(&json!([{"error": {"code": 400, "message": "gemini says no"}}])).as_deref(), Some("gemini says no"));
        assert_eq!(error_message(&json!({"error": "model not found"})).as_deref(), Some("model not found"));
        assert_eq!(error_message(&json!({"message": "plain"})).as_deref(), Some("plain"));
        assert_eq!(error_message(&json!({"error": null, "output": []})), None);
        assert_eq!(error_message(&json!({"choices": [{"message": {"content": "hi"}}]})), None);
    }

    #[test]
    fn tool_arguments_are_read_or_kept_as_sent() {
        assert_eq!(parse_arguments(r#"{"path":"a.txt"}"#), (json!({"path": "a.txt"}), None));
        assert_eq!(parse_arguments(""), (json!({}), None));
        assert_eq!(parse_arguments("null"), (json!({}), None));
        assert_eq!(parse_arguments(r#"{"path": "a.t"#), (json!({}), Some(r#"{"path": "a.t"#.into())));
        assert_eq!(parse_arguments("[1]").1.as_deref(), Some("[1]"));
        let call = ToolCall { id: "c".into(), name: "x".into(), input: json!({}), bad_arguments: Some("{oops".into()), raw: None };
        assert_eq!(call.arguments(), "{oops");
        assert_ne!(new_call_id(), new_call_id());
    }

    #[test]
    fn a_history_with_tools_survives_a_save() {
        let turns = vec![
            Turn { role: Role::User, parts: vec![Part::text("list files")] },
            Turn {
                role: Role::Assistant,
                parts: vec![
                    Part::Opaque { wire: Wire::Anthropic, model: "m".into(), data: json!({"type": "thinking", "signature": "s"}) },
                    Part::ToolCall(ToolCall { id: "t1".into(), name: "list_dir".into(), input: json!({"path": "."}), bad_arguments: None, raw: None }),
                ],
            },
            Turn { role: Role::User, parts: vec![Part::ToolResult { id: "t1".into(), output: "a.txt".into(), is_error: false }] },
        ];
        let saved = serde_json::to_string(&turns).unwrap();
        assert!(saved.contains(r#""type":"toolCall""#));
        assert_eq!(serde_json::from_str::<Vec<Turn>>(&saved).unwrap(), turns);
        assert_eq!(turns[1].text(), "");
        assert!(!turns[1].is_plain_text());
    }

    #[test]
    fn non_chat_models_are_left_out() {
        let m = |id: &str| ModelInfo { id: id.into(), label: id.into() };
        let openai = filter_models("openai", vec![m("gpt-5"), m("gpt-5-codex"), m("text-embedding-3-small"), m("whisper-1"), m("gpt-image-1")]);
        assert_eq!(openai.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gpt-5", "gpt-5-codex"]);
        let google = filter_models("google", vec![m("models/gemini-2.5-flash"), m("models/text-embedding-004"), m("models/imagen-4")]);
        assert_eq!(google, vec![ModelInfo { id: "gemini-2.5-flash".into(), label: "gemini-2.5-flash".into() }]);
    }

    #[test]
    fn a_dropped_file_becomes_parts_once() {
        let dir = std::env::temp_dir().join(format!("coucou-llm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, "hello").unwrap();
        let png = dir.join("pic.png");
        std::fs::write(&png, b"\x89PNG").unwrap();

        let parts = context_parts(Some(&ChatContext::File { name: "notes.txt".into(), path: txt.to_string_lossy().into() }));
        assert_eq!(parts, vec![Part::text("File contents:\nhello"), Part::text("File: notes.txt")]);
        let parts = context_parts(Some(&ChatContext::File { name: "pic.png".into(), path: png.to_string_lossy().into() }));
        assert_eq!(parts[0], Part::Image { mime: "image/png".into(), base64: base64(b"\x89PNG") });
        let parts = context_parts(Some(&ChatContext::Window { app_name: "Edge".into(), title: "Docs".into(), url: Some("https://x".into()) }));
        assert_eq!(parts, vec![Part::text("Context: App: Edge, Window: Docs, URL: https://x")]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
