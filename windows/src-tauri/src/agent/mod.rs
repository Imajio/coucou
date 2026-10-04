// Agent sessions: Coucou as a harness. Each session is one of Mochi's avatars
// with a role, a task, a folder and a model from any provider (llm::PROVIDERS),
// working through tools on the user's machine and asking before it changes
// anything it was not allowed to.
//
// The loop is the usual one: send the history and the role's tools, run the
// tool calls that come back (after the user's approval where the session's
// mode says so), send the results, until the model answers without a call.
// A session can hand work to a new session with another role (delegate); the
// child shows up as its own avatar and its final answer comes back as the
// tool's result.
//
// Sessions are saved under config_dir()/sessions, one file each, and come
// back when Coucou starts.

mod roles;
mod search;
mod session;
mod shell;
mod tools;
mod workspace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, watch};

use crate::llm::{self, Part, Role, Stop, ToolCall, Turn};
use crate::{log, platform};
use session::{needs_approval, now_ms, ApprovalRequest, Entry, Mode, SessionData, Status, Summary, Todo, ToolState};
use shell::Shell;
use tools::Access;
use workspace::Workspace;

pub const EVENT_SESSION: &str = "agent-session";
pub const EVENT_ENTRY: &str = "agent-entry";
pub const EVENT_REMOVED: &str = "agent-removed";
pub const EVENT_APPROVAL: &str = "agent-approval";

/// Model calls in one run before the session pauses for the user.
const MAX_STEPS: usize = 100;
/// A tool result the model gets at most.
const MAX_RESULT: usize = 40_000;
/// What the activity log keeps of a tool's output.
const MAX_LOG_OUTPUT: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    /// Allow, and don't ask again for this tool in this session.
    Always,
    Deny,
}

struct Pending {
    request: ApprovalRequest,
    reply: oneshot::Sender<Decision>,
}

struct Handle {
    data: Mutex<SessionData>,
    stop: watch::Sender<bool>,
    status: watch::Sender<Status>,
    pending: Mutex<Option<Pending>>,
}

impl Handle {
    fn new(data: SessionData) -> Arc<Self> {
        let status = data.status;
        Arc::new(Self {
            data: Mutex::new(data),
            stop: watch::channel(false).0,
            status: watch::channel(status).0,
            pending: Mutex::new(None),
        })
    }

    fn summary(&self) -> Summary {
        let approval = self.pending.lock().unwrap().as_ref().map(|p| p.request.clone());
        self.data.lock().unwrap().summary(approval)
    }

    fn id(&self) -> String {
        self.data.lock().unwrap().id.clone()
    }

    fn set_status(&self, status: Status) {
        self.data.lock().unwrap().status = status;
        self.status.send_replace(status);
    }
}

/// The sessions, shared by the commands and the running loops.
#[derive(Clone)]
pub struct Agents(Arc<Inner>);

struct Inner {
    sessions: Mutex<HashMap<String, Arc<Handle>>>,
    shell: Shell,
}

/// What the user fills in to start a session.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSession {
    #[serde(default)]
    pub name: Option<String>,
    pub role: String,
    /// The role's text as the user edited it; the role's own when empty.
    #[serde(default)]
    pub role_prompt: Option<String>,
    #[serde(default)]
    pub task: String,
    pub folder: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub mode: Mode,
}

/// A session's settings that can change while it lives.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPatch {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub mode: Option<Mode>,
}

/// What the new-session form starts from: the last choices.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Defaults {
    pub folder: String,
    pub role: String,
    pub mode: Option<Mode>,
    pub provider: String,
    pub model: String,
}

/// A session with its whole activity log, for the sessions window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub summary: Summary,
    pub log: Vec<Entry>,
    pub role_prompt: String,
    pub tools: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EntryEvent {
    session_id: String,
    index: usize,
    entry: Entry,
}

fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("{prefix}{:x}{:x}", now_ms(), NEXT.fetch_add(1, Ordering::Relaxed))
}

// ── Storage ───────────────────────────────────────────────────────────────────

fn store_dir() -> PathBuf {
    platform::config_dir().join("sessions")
}

/// Session ids are made here; anything else is refused as a file name.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn save(data: &SessionData) {
    if !valid_id(&data.id) {
        return;
    }
    let dir = store_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::line(format!("sessions: can't create {}: {e}", dir.display()));
        return;
    }
    let Ok(json) = serde_json::to_vec(data) else { return };
    let tmp = dir.join(format!("{}.json.tmp", data.id));
    let path = dir.join(format!("{}.json", data.id));
    if std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
        log::line(format!("sessions: can't save {}", data.id));
    }
}

fn load_all() -> Vec<SessionData> {
    let Ok(entries) = std::fs::read_dir(store_dir()) else { return Vec::new() };
    let mut out: Vec<SessionData> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let text = std::fs::read(e.path()).ok()?;
            match serde_json::from_slice::<SessionData>(&text) {
                Ok(s) if valid_id(&s.id) => Some(s),
                _ => {
                    log::line(format!("sessions: skipped unreadable {}", e.path().display()));
                    None
                }
            }
        })
        .collect();
    out.sort_by_key(|s| s.created_at);
    out
}

fn defaults_path() -> PathBuf {
    store_dir().join("defaults.json")
}

fn load_defaults() -> Defaults {
    std::fs::read(defaults_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_defaults(d: &Defaults) {
    let _ = std::fs::create_dir_all(store_dir());
    if let Ok(json) = serde_json::to_vec_pretty(d) {
        let _ = std::fs::write(defaults_path(), json);
    }
}

// ── Events ────────────────────────────────────────────────────────────────────

fn emit_summary(app: &AppHandle, h: &Handle) {
    let _ = app.emit(EVENT_SESSION, h.summary());
}

fn emit_entry(app: &AppHandle, h: &Handle, index: usize) {
    let (session_id, entry) = {
        let d = h.data.lock().unwrap();
        let Some(entry) = d.log.get(index).cloned() else { return };
        (d.id.clone(), entry)
    };
    let _ = app.emit(EVENT_ENTRY, EntryEvent { session_id, index, entry });
}

/// Appends to the activity log and tells the windows.
fn log_entry(app: &AppHandle, h: &Handle, entry: Entry) -> usize {
    let index = h.data.lock().unwrap().push(entry);
    emit_entry(app, h, index);
    index
}

fn set_tool_state(app: &AppHandle, h: &Handle, index: usize, state: ToolState, output: Option<&str>) {
    {
        let mut d = h.data.lock().unwrap();
        if let Some(Entry::Tool { state: s, output: o, .. }) = d.log.get_mut(index) {
            *s = state;
            if let Some(text) = output {
                *o = shell::clip(text, MAX_LOG_OUTPUT);
            }
        }
        d.updated_at = now_ms();
    }
    emit_entry(app, h, index);
}

async fn stopped(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The chat settings: the custom endpoint's address and the default model.
fn chat_settings(app: &AppHandle) -> (String, String, String, std::collections::BTreeMap<String, String>) {
    let shared = app.state::<crate::Shared>();
    let s = shared.settings.lock().unwrap();
    (s.custom_base_url.clone(), s.provider.clone(), s.model.clone(), s.provider_models.clone())
}

enum Outcome {
    Done,
    Stopped,
    Error(String),
}

impl Agents {
    /// Loads the saved sessions. One that was working when Coucou closed is
    /// marked stopped, its history made valid again.
    pub fn load() -> Self {
        let mut map = HashMap::new();
        for mut data in load_all() {
            if data.status.busy() {
                data.status = Status::Stopped;
                data.repair();
                data.push(Entry::Note { text: "Coucou was closed while this session was working. Send a message to continue.".into(), at: now_ms() });
                save(&data);
            }
            map.insert(data.id.clone(), Handle::new(data));
        }
        Self(Arc::new(Inner { sessions: Mutex::new(map), shell: Shell::detect() }))
    }

    fn get(&self, id: &str) -> Result<Arc<Handle>, String> {
        self.0.sessions.lock().unwrap().get(id).cloned().ok_or_else(|| "This session no longer exists.".to_string())
    }

    pub fn list(&self) -> Vec<Summary> {
        let handles: Vec<Arc<Handle>> = self.0.sessions.lock().unwrap().values().cloned().collect();
        let mut list: Vec<Summary> = handles.iter().map(|h| h.summary()).collect();
        list.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        list
    }

    pub fn view(&self, id: &str) -> Result<SessionView, String> {
        let h = self.get(id)?;
        let summary = h.summary();
        let d = h.data.lock().unwrap();
        Ok(SessionView { summary, log: d.log.clone(), role_prompt: d.role_prompt.clone(), tools: d.tools.clone() })
    }

    pub fn approvals(&self) -> Vec<ApprovalRequest> {
        let handles: Vec<Arc<Handle>> = self.0.sessions.lock().unwrap().values().cloned().collect();
        handles.iter().filter_map(|h| h.pending.lock().unwrap().as_ref().map(|p| p.request.clone())).collect()
    }

    /// Starts a session; with a task it starts working at once.
    pub fn create(&self, app: &AppHandle, spec: NewSession, parent: Option<(String, String, Mode)>) -> Result<String, String> {
        let role = roles::role(&spec.role).ok_or_else(|| format!("There is no role named {}.", spec.role))?;
        let ws = Workspace::open(&spec.folder)?;
        let (_, chat_provider, chat_model, provider_models) = chat_settings(app);
        let provider = spec.provider.filter(|p| !p.trim().is_empty()).unwrap_or(chat_provider.clone());
        let provider_def = llm::provider(&provider).ok_or_else(|| format!("Unknown provider {provider}."))?;
        let model = spec
            .model
            .filter(|m| !m.trim().is_empty())
            .or_else(|| provider_models.get(&provider).cloned())
            .or_else(|| (provider == chat_provider).then(|| chat_model.clone()))
            .unwrap_or_default();
        if model.trim().is_empty() {
            return Err(format!("Pick a {} model for this session.", provider_def.name));
        }
        let delegated = parent.is_some();
        let tools = roles::tools_for(role, delegated);
        let name = match spec.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.chars().take(40).collect(),
            None => {
                let same = self.0.sessions.lock().unwrap().values().filter(|h| h.data.lock().unwrap().role == role.id).count();
                format!("{} {}", role.name, same + 1)
            }
        };
        let role_prompt = spec.role_prompt.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| role.prompt.to_string());
        let mode = match &parent {
            Some((_, _, parent_mode)) => spec.mode.at_most(*parent_mode),
            None => spec.mode,
        };
        let folder = ws.root().to_string_lossy().into_owned();
        let now = now_ms();
        let system = session::system_prompt(&session::PromptContext {
            name: &name,
            role,
            role_prompt: &role_prompt,
            folder: &folder,
            shell: self.0.shell.describe(),
            tools: &tools,
            parent: parent.as_ref().map(|p| p.1.as_str()),
            now,
        });
        let data = SessionData {
            id: new_id("s"),
            name,
            role: role.id.into(),
            color: role.color.into(),
            role_prompt,
            provider: provider.clone(),
            model: model.clone(),
            folder: folder.clone(),
            mode,
            status: Status::Idle,
            error: None,
            parent: parent.as_ref().map(|p| p.0.clone()),
            created_at: now,
            updated_at: now,
            system,
            tools: tools.iter().map(|t| t.to_string()).collect(),
            turns: Vec::new(),
            log: Vec::new(),
            todos: Vec::new(),
            always: Vec::new(),
        };
        let id = data.id.clone();
        save(&data);
        let h = Handle::new(data);
        self.0.sessions.lock().unwrap().insert(id.clone(), h.clone());
        if !delegated {
            save_defaults(&Defaults { folder, role: role.id.into(), mode: Some(mode), provider, model });
        }
        emit_summary(app, &h);
        if !spec.task.trim().is_empty() {
            self.send(app, &id, spec.task)?;
        }
        Ok(id)
    }

    /// A message from the user: the task or a follow-up. Starts the loop.
    pub fn send(&self, app: &AppHandle, id: &str, text: String) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("Write what the session should do.".into());
        }
        let h = self.get(id)?;
        {
            let mut d = h.data.lock().unwrap();
            if d.status.busy() {
                return Err("This session is still working. Stop it first or wait.".into());
            }
            d.repair();
            d.turns.push(Turn { role: Role::User, parts: vec![Part::text(text.clone())] });
            d.error = None;
        }
        h.stop.send_replace(false);
        h.set_status(Status::Running);
        log_entry(app, &h, Entry::User { text, at: now_ms() });
        save(&h.data.lock().unwrap());
        emit_summary(app, &h);
        let (app, agents) = (app.clone(), self.clone());
        tauri::async_runtime::spawn(async move { agents.drive(app, h).await });
        Ok(())
    }

    /// Stops a session and everything it delegated.
    pub fn stop(&self, id: &str) -> Result<(), String> {
        let h = self.get(id)?;
        h.stop.send_replace(true);
        let children: Vec<Arc<Handle>> = self
            .0
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.data.lock().unwrap().parent.as_deref() == Some(id))
            .cloned()
            .collect();
        for c in children {
            if c.data.lock().unwrap().status.busy() {
                let _ = self.stop(&c.id());
            }
        }
        Ok(())
    }

    pub fn delete(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let _ = self.stop(id);
        self.0.sessions.lock().unwrap().remove(id);
        if valid_id(id) {
            let _ = std::fs::remove_file(store_dir().join(format!("{id}.json")));
        }
        let _ = app.emit(EVENT_REMOVED, id.to_string());
        Ok(())
    }

    pub fn update(&self, app: &AppHandle, id: &str, patch: SessionPatch) -> Result<(), String> {
        let h = self.get(id)?;
        // The parent's mode first: never lock a session while holding another.
        let parent = h.data.lock().unwrap().parent.clone();
        let cap = parent.and_then(|p| self.get(&p).ok()).map(|p| p.data.lock().unwrap().mode);
        {
            let mut d = h.data.lock().unwrap();
            if let Some(name) = patch.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                d.name = name.chars().take(40).collect();
            }
            if let Some(provider) = patch.provider.filter(|p| !p.is_empty()) {
                if llm::provider(&provider).is_none() {
                    return Err(format!("Unknown provider {provider}."));
                }
                d.provider = provider;
            }
            if let Some(model) = patch.model.filter(|m| !m.trim().is_empty()) {
                d.model = model;
            }
            if let Some(mode) = patch.mode {
                // A delegated session stays within its parent's freedom.
                d.mode = cap.map_or(mode, |c| mode.at_most(c));
            }
            d.updated_at = now_ms();
        }
        save(&h.data.lock().unwrap());
        emit_summary(app, &h);
        Ok(())
    }

    /// The user's answer to an approval request.
    pub fn decide(&self, request_id: &str, decision: Decision) -> Result<(), String> {
        let handles: Vec<Arc<Handle>> = self.0.sessions.lock().unwrap().values().cloned().collect();
        for h in handles {
            let mut pending = h.pending.lock().unwrap();
            if pending.as_ref().is_some_and(|p| p.request.request_id == request_id) {
                let p = pending.take().unwrap();
                let _ = p.reply.send(decision);
                return Ok(());
            }
        }
        Err("This request was already answered.".into())
    }

    // ── The loop ──────────────────────────────────────────────────────────────

    async fn drive(self, app: AppHandle, h: Arc<Handle>) {
        let outcome = self.run(&app, &h).await;
        let (status, line) = match outcome {
            Outcome::Done => (Status::Done, None),
            Outcome::Stopped => (Status::Stopped, Some(Entry::Note { text: "Stopped.".into(), at: now_ms() })),
            Outcome::Error(e) => (Status::Error, Some(Entry::Error { text: e, at: now_ms() })),
        };
        {
            let mut d = h.data.lock().unwrap();
            d.repair();
            d.error = match &line {
                Some(Entry::Error { text, .. }) => Some(text.clone()),
                _ => None,
            };
        }
        if let Some(line) = line {
            log_entry(&app, &h, line);
        }
        *h.pending.lock().unwrap() = None;
        h.set_status(status);
        save(&h.data.lock().unwrap());
        emit_summary(&app, &h);
    }

    async fn run(&self, app: &AppHandle, h: &Arc<Handle>) -> Outcome {
        let folder = h.data.lock().unwrap().folder.clone();
        let ws = match Workspace::open(&folder) {
            Ok(ws) => ws,
            Err(e) => return Outcome::Error(e),
        };
        for _ in 0..MAX_STEPS {
            if *h.stop.borrow() {
                return Outcome::Stopped;
            }
            let (turns, system, tool_names, provider, model) = {
                let d = h.data.lock().unwrap();
                (d.turns.clone(), d.system.clone(), d.tools.clone(), d.provider.clone(), d.model.clone())
            };
            let (custom, ..) = chat_settings(app);
            let target = match llm::Target::resolve(&provider, &custom) {
                Ok(t) => t,
                Err(e) => return Outcome::Error(e),
            };
            let names: Vec<&str> = tool_names.iter().map(String::as_str).collect();
            let specs = tools::specs(&names);
            let mut stop_rx = h.stop.subscribe();
            let reply = tokio::select! {
                r = llm::complete(&target, &model, &system, &turns, &specs) => r,
                _ = stopped(&mut stop_rx) => return Outcome::Stopped,
            };
            let reply = match reply {
                Ok(r) => r,
                Err(e) => return Outcome::Error(e),
            };
            let text = reply.text();
            h.data.lock().unwrap().turns.push(Turn { role: Role::Assistant, parts: reply.parts.clone() });
            if !text.trim().is_empty() {
                log_entry(app, h, Entry::Assistant { text: text.trim().to_string(), at: now_ms() });
            }
            let calls: Vec<ToolCall> = reply.calls().cloned().collect();
            if calls.is_empty() {
                if reply.stop == Stop::Truncated {
                    log_entry(app, h, Entry::Note { text: "The answer was cut off at the model's output limit. Ask it to continue.".into(), at: now_ms() });
                }
                return Outcome::Done;
            }
            let mut results = Vec::new();
            for call in &calls {
                let (output, is_error) = if reply.stop == Stop::Truncated {
                    (
                        "Not run: your reply hit the output limit, so this call may be cut off. Do it again in smaller pieces: for a long file, write a short version first, then add to it with edit_file.".to_string(),
                        true,
                    )
                } else if *h.stop.borrow() {
                    ("Cancelled: the user stopped the session.".to_string(), true)
                } else {
                    self.call(app, h, &ws, call).await
                };
                results.push(Part::ToolResult { id: call.id.clone(), output: shell::clip(&output, MAX_RESULT), is_error });
            }
            {
                let mut d = h.data.lock().unwrap();
                d.turns.push(Turn { role: Role::User, parts: results });
                d.updated_at = now_ms();
            }
            save(&h.data.lock().unwrap());
            emit_summary(app, h);
        }
        log_entry(app, h, Entry::Note { text: format!("Paused after {MAX_STEPS} model calls. Send a message to let it go on."), at: now_ms() });
        Outcome::Done
    }

    /// One tool call: plan, approval, run. Returns the result for the model.
    async fn call(&self, app: &AppHandle, h: &Arc<Handle>, ws: &Workspace, call: &ToolCall) -> (String, bool) {
        let (allowed, mode, always) = {
            let d = h.data.lock().unwrap();
            (d.tools.iter().any(|t| *t == call.name), d.mode, d.always.clone())
        };
        let plan = match tools::plan(ws, call) {
            Ok(p) if allowed => p,
            Ok(_) => return (format!("This session's role has no {} tool.", call.name), true),
            Err(e) => {
                log_entry(app, h, Entry::Tool {
                    call_id: call.id.clone(),
                    tool: call.name.clone(),
                    summary: format!("{} (invalid call)", call.name),
                    detail: String::new(),
                    state: ToolState::Error,
                    output: e.clone(),
                    outside: false,
                    at: now_ms(),
                });
                return (e, true);
            }
        };
        let outside = plan.outside(ws);
        let index = log_entry(app, h, Entry::Tool {
            call_id: call.id.clone(),
            tool: call.name.clone(),
            summary: plan.summary.clone(),
            detail: plan.detail.clone(),
            state: ToolState::Running,
            output: String::new(),
            outside,
            at: now_ms(),
        });
        emit_summary(app, h);

        if needs_approval(mode, &plan, ws, &always, &call.name) {
            set_tool_state(app, h, index, ToolState::Waiting, None);
            match self.ask(app, h, call, &plan, outside).await {
                Some(Decision::Allow) => {}
                Some(Decision::Always) => {
                    let mut d = h.data.lock().unwrap();
                    if !d.always.contains(&call.name) {
                        d.always.push(call.name.clone());
                    }
                }
                Some(Decision::Deny) => {
                    set_tool_state(app, h, index, ToolState::Denied, Some("Denied by the user."));
                    return (
                        "The user denied this call. Don't try the same thing another way: say what you wanted to do and why, and ask how to proceed.".into(),
                        true,
                    );
                }
                None => {
                    set_tool_state(app, h, index, ToolState::Denied, Some("Stopped."));
                    return ("Cancelled: the user stopped the session.".into(), true);
                }
            }
            set_tool_state(app, h, index, ToolState::Running, None);
        }

        let result = if plan.access == Access::Internal {
            self.internal(app, h, call).await
        } else {
            let ctx = tools::Context { ws, shell: &self.0.shell, stop: h.stop.subscribe() };
            tools::run(ctx, call).await
        };
        match result {
            Ok(text) => {
                set_tool_state(app, h, index, ToolState::Ok, Some(&text));
                (text, false)
            }
            Err(text) => {
                set_tool_state(app, h, index, ToolState::Error, Some(&text));
                (text, true)
            }
        }
    }

    /// Waits for the user's decision; None when the session is stopped meanwhile.
    async fn ask(&self, app: &AppHandle, h: &Arc<Handle>, call: &ToolCall, plan: &tools::Plan, outside: bool) -> Option<Decision> {
        let (tx, rx) = oneshot::channel();
        let request = {
            let d = h.data.lock().unwrap();
            ApprovalRequest {
                request_id: new_id("r"),
                session_id: d.id.clone(),
                session_name: d.name.clone(),
                color: d.color.clone(),
                tool: call.name.clone(),
                summary: plan.summary.clone(),
                detail: plan.detail.clone(),
                outside,
            }
        };
        *h.pending.lock().unwrap() = Some(Pending { request: request.clone(), reply: tx });
        h.set_status(Status::Waiting);
        emit_summary(app, h);
        let _ = app.emit(EVENT_APPROVAL, request);
        let mut stop_rx = h.stop.subscribe();
        let decision = tokio::select! {
            d = rx => d.ok(),
            _ = stopped(&mut stop_rx) => None,
        };
        *h.pending.lock().unwrap() = None;
        h.set_status(Status::Running);
        emit_summary(app, h);
        decision
    }

    /// The tools the session handles itself: the plan and delegation.
    async fn internal(&self, app: &AppHandle, h: &Arc<Handle>, call: &ToolCall) -> Result<String, String> {
        match call.name.as_str() {
            "todo_write" => {
                let todos: Vec<Todo> = call
                    .input
                    .get("todos")
                    .and_then(Value::as_array)
                    .ok_or("Missing `todos`.")?
                    .iter()
                    .filter_map(|t| {
                        let content = t.get("content")?.as_str()?.trim();
                        let status = t.get("status").and_then(Value::as_str).unwrap_or("pending");
                        let status = if ["pending", "in_progress", "completed"].contains(&status) { status } else { "pending" };
                        (!content.is_empty()).then(|| Todo { content: content.chars().take(200).collect(), status: status.into() })
                    })
                    .take(30)
                    .collect();
                let done = todos.iter().filter(|t| t.status == "completed").count();
                let total = todos.len();
                h.data.lock().unwrap().todos = todos;
                emit_summary(app, h);
                Ok(format!("Plan updated ({done}/{total} done)."))
            }
            "delegate" => self.delegate(app, h, call).await,
            other => Err(format!("{other} is not handled here.")),
        }
    }

    async fn delegate(&self, app: &AppHandle, h: &Arc<Handle>, call: &ToolCall) -> Result<String, String> {
        let role = call.input.get("role").and_then(Value::as_str).unwrap_or("").to_string();
        let task = call.input.get("task").and_then(Value::as_str).unwrap_or("").to_string();
        let name = call.input.get("name").and_then(Value::as_str).map(str::to_string);
        let (parent, spec) = {
            let d = h.data.lock().unwrap();
            (
                (d.id.clone(), d.name.clone(), d.mode),
                NewSession {
                    name,
                    role,
                    role_prompt: None,
                    task,
                    folder: d.folder.clone(),
                    provider: Some(d.provider.clone()),
                    model: Some(d.model.clone()),
                    mode: d.mode,
                },
            )
        };
        let child_id = self.create(app, spec, Some(parent))?;
        let child = self.get(&child_id)?;
        let mut status_rx = child.status.subscribe();
        let mut stop_rx = h.stop.subscribe();
        loop {
            if !status_rx.borrow_and_update().busy() {
                break;
            }
            tokio::select! {
                r = status_rx.changed() => if r.is_err() { break },
                _ = stopped(&mut stop_rx) => {
                    let _ = self.stop(&child_id);
                    return Err("Cancelled: the user stopped the session.".into());
                }
            }
        }
        let (status, name, report, error) = {
            let d = child.data.lock().unwrap();
            (d.status, d.name.clone(), d.last_reply(), d.error.clone())
        };
        match status {
            Status::Done => Ok(format!("Report from {name}:\n\n{}", report.unwrap_or_else(|| "(no report)".into()))),
            Status::Stopped => Err(format!("The user stopped {name} before it finished.")),
            _ => Err(format!("{name} failed: {}", error.unwrap_or_else(|| "unknown error".into()))),
        }
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn agent_roles() -> Vec<roles::RoleInfo> {
    roles::infos()
}

#[tauri::command]
pub fn agent_list(agents: tauri::State<'_, Agents>) -> Vec<Summary> {
    agents.list()
}

#[tauri::command]
pub fn agent_get(agents: tauri::State<'_, Agents>, id: String) -> Result<SessionView, String> {
    agents.view(&id)
}

#[tauri::command]
pub fn agent_approvals(agents: tauri::State<'_, Agents>) -> Vec<ApprovalRequest> {
    agents.approvals()
}

#[tauri::command]
pub fn agent_defaults() -> Defaults {
    load_defaults()
}

#[tauri::command]
pub fn agent_create(app: AppHandle, agents: tauri::State<'_, Agents>, spec: NewSession) -> Result<String, String> {
    agents.create(&app, spec, None)
}

#[tauri::command]
pub fn agent_send(app: AppHandle, agents: tauri::State<'_, Agents>, id: String, text: String) -> Result<(), String> {
    agents.send(&app, &id, text)
}

#[tauri::command]
pub fn agent_stop(agents: tauri::State<'_, Agents>, id: String) -> Result<(), String> {
    agents.stop(&id)
}

#[tauri::command]
pub fn agent_delete(app: AppHandle, agents: tauri::State<'_, Agents>, id: String) -> Result<(), String> {
    agents.delete(&app, &id)
}

#[tauri::command]
pub fn agent_update(app: AppHandle, agents: tauri::State<'_, Agents>, id: String, patch: SessionPatch) -> Result<(), String> {
    agents.update(&app, &id, patch)
}

#[tauri::command]
pub fn agent_decide(agents: tauri::State<'_, Agents>, request_id: String, decision: Decision) -> Result<(), String> {
    agents.decide(&request_id, decision)
}

/// Whether a folder can host a session, for the new-session form.
#[tauri::command]
pub fn agent_check_folder(folder: String) -> Result<String, String> {
    Workspace::open(&folder).map(|ws| ws.root().to_string_lossy().into_owned())
}
