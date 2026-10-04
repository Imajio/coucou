// A session as it is kept and shown: its settings, the provider-neutral
// history the model sees, the activity log the user sees, and the rules that
// decide when the user is asked.

use serde::{Deserialize, Serialize};

use super::roles::RoleDef;
use super::tools::{Access, Plan};
use super::workspace::Workspace;
use crate::llm::{Part, Role, Turn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Asks before any change, command or network call.
    Ask,
    /// Edits files in its folder on its own; asks for commands and the network.
    Edits,
    /// Does everything on its own, except writing outside its folder.
    Auto,
}

impl Mode {
    /// The stricter of two: a delegated session never gets more than its parent.
    pub fn at_most(self, other: Mode) -> Mode {
        fn rank(m: Mode) -> u8 {
            match m {
                Mode::Ask => 0,
                Mode::Edits => 1,
                Mode::Auto => 2,
            }
        }
        if rank(self) <= rank(other) { self } else { other }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Nothing asked yet, or waiting for the next message.
    Idle,
    Running,
    /// Waiting for the user to approve a tool call.
    Waiting,
    /// Answered; a follow-up can continue it.
    Done,
    Error,
    Stopped,
}

impl Status {
    pub fn busy(self) -> bool {
        matches!(self, Status::Running | Status::Waiting)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolState {
    Running,
    Waiting,
    Ok,
    Error,
    Denied,
}

/// One line of the activity log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Entry {
    User { text: String, at: u64 },
    Assistant { text: String, at: u64 },
    #[serde(rename_all = "camelCase")]
    Tool {
        call_id: String,
        tool: String,
        summary: String,
        /// The change or command, as approved.
        detail: String,
        state: ToolState,
        /// The start of what came back.
        output: String,
        /// Reaches outside the session folder.
        outside: bool,
        at: u64,
    },
    Note { text: String, at: u64 },
    Error { text: String, at: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub content: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionData {
    pub id: String,
    pub name: String,
    pub role: String,
    pub color: String,
    /// What the user said this session is (the role's text, edited or not).
    pub role_prompt: String,
    pub provider: String,
    pub model: String,
    pub folder: String,
    pub mode: Mode,
    pub status: Status,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Fixed when the session starts: Claude's thinking blocks are bound to
    /// the exact system prompt and tools they were made with.
    pub system: String,
    pub tools: Vec<String>,
    pub turns: Vec<Turn>,
    pub log: Vec<Entry>,
    #[serde(default)]
    pub todos: Vec<Todo>,
    /// Tools the user said to always allow in this session.
    #[serde(default)]
    pub always: Vec<String>,
}

/// A pending approval, as the island and the sessions window show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub request_id: String,
    pub session_id: String,
    pub session_name: String,
    pub color: String,
    pub tool: String,
    pub summary: String,
    pub detail: String,
    pub outside: bool,
}

/// What lists and avatars need, without the history.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub role: String,
    pub color: String,
    pub provider: String,
    pub model: String,
    pub folder: String,
    pub mode: Mode,
    pub status: Status,
    pub error: Option<String>,
    pub parent: Option<String>,
    pub updated_at: u64,
    /// The latest thing it did or said, for the avatar's ticker.
    pub activity: String,
    pub todos: Vec<Todo>,
    pub approval: Option<ApprovalRequest>,
    pub entries: usize,
}

/// A reply's first line without its Markdown marks, for a one-line preview.
pub fn plain_line(text: &str) -> String {
    let line = text
        .lines()
        .map(|l| l.trim().trim_start_matches(['#', '>', '-', '*', '+', ' ']).trim())
        .find(|l| !l.is_empty() && !l.starts_with("```"))
        .unwrap_or("");
    line.replace("**", "").replace('`', "")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl SessionData {
    pub fn summary(&self, approval: Option<ApprovalRequest>) -> Summary {
        Summary {
            id: self.id.clone(),
            name: self.name.clone(),
            role: self.role.clone(),
            color: self.color.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            folder: self.folder.clone(),
            mode: self.mode,
            status: self.status,
            error: self.error.clone(),
            parent: self.parent.clone(),
            updated_at: self.updated_at,
            activity: self.activity(),
            todos: self.todos.clone(),
            approval,
            entries: self.log.len(),
        }
    }

    /// The last line worth a glance: a tool's summary or the start of a reply.
    pub fn activity(&self) -> String {
        for e in self.log.iter().rev() {
            let line = match e {
                Entry::Tool { summary, .. } => summary.clone(),
                Entry::Assistant { text, .. } | Entry::User { text, .. } => plain_line(text),
                Entry::Error { text, .. } => text.clone(),
                Entry::Note { .. } => continue,
            };
            if !line.trim().is_empty() {
                return line.chars().take(140).collect();
            }
        }
        String::new()
    }

    /// The final answer: the last thing the model said.
    pub fn last_reply(&self) -> Option<String> {
        self.log.iter().rev().find_map(|e| match e {
            Entry::Assistant { text, .. } => Some(text.clone()),
            _ => None,
        })
    }

    pub fn push(&mut self, entry: Entry) -> usize {
        self.log.push(entry);
        self.updated_at = now_ms();
        self.log.len() - 1
    }

    /// After a crash or a stop between a tool call and its result: answers
    /// every unanswered call so the history is valid for any provider again.
    pub fn repair(&mut self) {
        let Some(last) = self.turns.last() else { return };
        if last.role != Role::Assistant {
            return;
        }
        let ids: Vec<String> = last
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall(c) => Some(c.id.clone()),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let parts = ids
            .into_iter()
            .map(|id| Part::ToolResult { id, output: "Cancelled: the session was stopped before this ran.".into(), is_error: true })
            .collect();
        self.turns.push(Turn { role: Role::User, parts });
    }
}

/// Whether a call waits for the user, given the session's mode and what the
/// user already said to always allow. Writing outside the folder always asks.
pub fn needs_approval(mode: Mode, plan: &Plan, ws: &Workspace, always: &[String], tool: &str) -> bool {
    let outside = plan.outside(ws);
    if plan.access == Access::Write && outside {
        return true;
    }
    if always.iter().any(|t| t == tool) {
        return false;
    }
    match (plan.access, mode) {
        (Access::Internal, _) => false,
        (Access::Read, Mode::Auto) => false,
        (Access::Read, _) => outside,
        (_, Mode::Auto) => false,
        (Access::Write, Mode::Edits) => false,
        _ => true,
    }
}

/// `YYYY-MM-DD HH:MM UTC` for a Unix time in ms.
pub fn date_time(ms: u64) -> String {
    let minutes = (ms / 60_000) % (24 * 60);
    format!("{} {:02}:{:02} UTC", date(ms), minutes / 60, minutes % 60)
}

/// `YYYY-MM-DD` for a Unix time in ms (UTC), without a date library.
pub fn date(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub struct PromptContext<'a> {
    pub name: &'a str,
    pub role: &'a RoleDef,
    pub role_prompt: &'a str,
    pub folder: &'a str,
    pub shell: &'a str,
    pub tools: &'a [&'a str],
    pub parent: Option<&'a str>,
    pub now: u64,
}

/// The session's system prompt, written once when it starts.
pub fn system_prompt(c: &PromptContext) -> String {
    let os = if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" };
    let mut p = format!(
        "You are \"{name}\", a {role} session in Coucou, a desktop app where the user runs several AI sessions side by side, each shown as an avatar.\n\n{prompt}\n\n",
        name = c.name,
        role = c.role.name.to_lowercase(),
        prompt = c.role_prompt.trim(),
    );
    p.push_str(&format!(
        "Environment:\n- Working folder: {folder} (relative paths start here)\n- Operating system: {os}\n- Shell for run_command: {shell}\n- Today: {date}\n\n",
        folder = c.folder,
        shell = c.shell,
        date = date(c.now),
    ));
    p.push_str(
        "How to work:\n\
- Work through your tools; never claim you did something you did not do with a tool.\n\
- Read a file before you change it. Prefer small edits over rewriting whole files.\n\
- Write long files in parts: create a short file, then add to it with edit_file, so no single call is too long.\n\
- Some calls wait for the user's approval. If the user denies one, don't try the same thing another way: say what you wanted to do and ask.\n\
- Only you see tool output. Put in your reply whatever the user needs from it.\n\
- Before a longer piece of work, say in a line what you are about to do. End with a short summary that stands on its own: what you did, what you found, what is left.\n\
- Replies may use Markdown (headings, lists, code blocks).\n",
    );
    if c.tools.contains(&"todo_write") {
        p.push_str("- For work of three steps or more, keep a plan with todo_write; the user sees it next to your avatar.\n");
    }
    if c.tools.contains(&"delegate") {
        p.push_str("- Delegate self-contained pieces of work with the delegate tool; each delegated session reports back to you.\n");
    }
    if let Some(parent) = c.parent {
        p.push_str(&format!(
            "\nThe session \"{parent}\" handed you this task. Your last message is your report to it: make it complete and self-contained.\n"
        ));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::roles::role;
    use crate::llm::ToolCall;
    use serde_json::json;
    use std::path::PathBuf;

    fn plan(access: Access, paths: Vec<PathBuf>) -> Plan {
        Plan { access, paths, summary: String::new(), detail: String::new() }
    }

    #[test]
    fn approvals_follow_the_mode_and_writes_outside_always_ask() {
        let dir = std::env::temp_dir().join(format!("coucou-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::open(&dir.to_string_lossy()).unwrap();
        let inside = vec![ws.resolve("a.txt").unwrap()];
        let outside = vec![ws.resolve("../b.txt").unwrap()];
        let none: Vec<String> = vec![];
        let ask = |mode, access, paths: &Vec<PathBuf>, always: &[String]| needs_approval(mode, &plan(access, paths.clone()), &ws, always, "t");

        assert!(!ask(Mode::Ask, Access::Read, &inside, &none));
        assert!(ask(Mode::Ask, Access::Read, &outside, &none));
        assert!(ask(Mode::Ask, Access::Write, &inside, &none));
        assert!(ask(Mode::Ask, Access::Exec, &vec![], &none));
        assert!(!ask(Mode::Ask, Access::Internal, &vec![], &none));
        assert!(!ask(Mode::Edits, Access::Write, &inside, &none));
        assert!(ask(Mode::Edits, Access::Exec, &vec![], &none));
        assert!(ask(Mode::Edits, Access::Network, &vec![], &none));
        assert!(!ask(Mode::Auto, Access::Exec, &vec![], &none));
        assert!(!ask(Mode::Auto, Access::Read, &outside, &none));
        // Never on its own outside the folder, whatever was said before.
        assert!(ask(Mode::Auto, Access::Write, &outside, &none));
        assert!(ask(Mode::Auto, Access::Write, &outside, &["t".to_string()]));
        assert!(!ask(Mode::Ask, Access::Exec, &vec![], &["t".to_string()]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_child_never_gets_more_freedom_than_its_parent() {
        assert_eq!(Mode::Auto.at_most(Mode::Ask), Mode::Ask);
        assert_eq!(Mode::Ask.at_most(Mode::Auto), Mode::Ask);
        assert_eq!(Mode::Edits.at_most(Mode::Auto), Mode::Edits);
    }

    #[test]
    fn previews_drop_markdown_marks() {
        assert_eq!(plain_line("## Done

I created **hello.txt**"), "Done");
        assert_eq!(plain_line("
- run `npm test`"), "run npm test");
        assert_eq!(plain_line("```
code"), "code");
    }

    #[test]
    fn dates_are_computed_without_a_library() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400_000), "2000-02-29");
        assert_eq!(date(1_791_158_400_000), "2026-10-05");
        assert_eq!(date_time(1_791_158_400_000 + 13 * 3_600_000 + 7 * 60_000), "2026-10-05 13:07 UTC");
    }

    #[test]
    fn an_unanswered_call_gets_a_cancelled_result() {
        let mut s = SessionData {
            id: "s".into(), name: "n".into(), role: "engineer".into(), color: "#fff".into(), role_prompt: String::new(),
            provider: "anthropic".into(), model: "m".into(), folder: ".".into(), mode: Mode::Ask, status: Status::Running,
            error: None, parent: None, created_at: 0, updated_at: 0, system: String::new(), tools: vec![],
            turns: vec![
                Turn { role: Role::User, parts: vec![Part::text("go")] },
                Turn { role: Role::Assistant, parts: vec![Part::ToolCall(ToolCall { id: "c1".into(), name: "read_file".into(), input: json!({}), bad_arguments: None, raw: None })] },
            ],
            log: vec![], todos: vec![], always: vec![],
        };
        s.repair();
        assert_eq!(s.turns.len(), 3);
        assert!(matches!(&s.turns[2].parts[0], Part::ToolResult { id, is_error: true, .. } if id == "c1"));
        s.repair();
        assert_eq!(s.turns.len(), 3);
    }

    #[test]
    fn the_system_prompt_names_the_folder_shell_and_report_duty() {
        let lead = role("lead").unwrap();
        let p = system_prompt(&PromptContext {
            name: "Planner", role: lead, role_prompt: lead.prompt, folder: "C:/proj", shell: "Git Bash",
            tools: &["todo_write", "delegate"], parent: Some("Boss"), now: 0,
        });
        assert!(p.contains("C:/proj") && p.contains("Git Bash") && p.contains("1970-01-01"));
        assert!(p.contains("delegate tool"));
        assert!(p.contains("\"Boss\" handed you this task"));
        assert!(!p.contains('\u{2014}') && !p.contains('\u{2013}'));
    }
}
