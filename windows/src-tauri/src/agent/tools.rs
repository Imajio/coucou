// The tools a session's model may call, what each one is about to do (shown to
// the user before anything that changes a file, runs a command or reaches the
// network), and running them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::watch;

use super::search;
use super::shell::{self, Shell};
use super::workspace::Workspace;
use crate::llm::{ToolCall, ToolSpec};

/// What a tool does, which decides when the user is asked first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads the disk.
    Read,
    /// Creates or changes a file.
    Write,
    /// Runs a command.
    Exec,
    /// Reaches the network.
    Network,
    /// Handled by the session itself: the plan, delegation.
    Internal,
}

pub struct ToolDef {
    pub name: &'static str,
    pub access: Access,
    description: &'static str,
    schema: fn() -> Value,
}

const MAX_READ_LINES: usize = 2000;
const MAX_LINE: usize = 2000;
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
const MAX_LIST: usize = 500;
const MAX_FETCH: usize = 30_000;
/// What the approval card shows of a change at most.
const MAX_DETAIL_LINES: usize = 60;

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "read_file",
        access: Access::Read,
        description: "Reads a text file and returns it with line numbers (`   12\tline`). Reads up to 2000 lines from `offset` (1-based); call again with a later offset for more. Read a file before you edit it.",
        schema: || json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path, relative to the session folder or absolute." },
                "offset": { "type": "integer", "description": "First line to read, 1-based. Default 1." },
                "limit": { "type": "integer", "description": "How many lines. Default 2000." }
            },
            "required": ["path"]
        }),
    },
    ToolDef {
        name: "write_file",
        access: Access::Write,
        description: "Creates a file, or replaces all of an existing one, with `content`. Missing folders are created. Prefer edit_file to change part of an existing file.",
        schema: || json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string", "description": "The whole new content of the file." }
            },
            "required": ["path", "content"]
        }),
    },
    ToolDef {
        name: "edit_file",
        access: Access::Write,
        description: "Replaces `old_string` with `new_string` in a file. `old_string` must match the file exactly, indentation included, and appear only once unless `replace_all` is true; include enough surrounding lines to make it unique.",
        schema: || json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "replace_all": { "type": "boolean", "description": "Replace every occurrence. Default false." }
            },
            "required": ["path", "old_string", "new_string"]
        }),
    },
    ToolDef {
        name: "list_dir",
        access: Access::Read,
        description: "Lists a folder: sub-folders end with `/`, files show their size and when they last changed.",
        schema: || json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "Folder. Default: the session folder." } }
        }),
    },
    ToolDef {
        name: "glob",
        access: Access::Read,
        description: "Finds files by name pattern, most recently changed first: `**/*.ts`, `src/**/test_*.py`, `*.{json,yaml}`. A pattern without `/` matches file names anywhere. Dependency and build folders (node_modules, target, .git...) are skipped.",
        schema: || json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string", "description": "Folder to search. Default: the session folder." }
            },
            "required": ["pattern"]
        }),
    },
    ToolDef {
        name: "grep",
        access: Access::Read,
        description: "Searches file contents with a regular expression (Rust regex syntax) and returns `path:line: text` for each matching line.",
        schema: || json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string", "description": "Folder or file to search. Default: the session folder." },
                "glob": { "type": "string", "description": "Only files matching this glob, e.g. `*.rs`." },
                "ignore_case": { "type": "boolean" }
            },
            "required": ["pattern"]
        }),
    },
    ToolDef {
        name: "run_command",
        access: Access::Exec,
        description: "Runs one shell command in the session folder and returns its output and exit code. Each call starts fresh in the session folder (a `cd` does not carry over). Nothing can be typed into it: use non-interactive flags. Default timeout 120 s, at most 600.",
        schema: || json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout_seconds": { "type": "integer" }
            },
            "required": ["command"]
        }),
    },
    ToolDef {
        name: "web_fetch",
        access: Access::Network,
        description: "Downloads a web page (http or https) and returns its text without the markup.",
        schema: || json!({
            "type": "object",
            "properties": { "url": { "type": "string" } },
            "required": ["url"]
        }),
    },
    ToolDef {
        name: "todo_write",
        access: Access::Internal,
        description: "Keeps the plan the user sees next to your avatar. Send the whole list each time, with each step's status. Use it for any task of three steps or more; keep exactly one step in_progress while you work.",
        schema: || json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        }),
    },
    ToolDef {
        name: "delegate",
        access: Access::Internal,
        description: "Hands a self-contained piece of work to a new session with its own role, which appears as its own avatar. It works in the same folder, sees none of this conversation, and its final report comes back as this tool's result. Say everything it needs in `task`. Roles: engineer, reviewer, researcher, tester, writer.",
        schema: || json!({
            "type": "object",
            "properties": {
                "role": { "type": "string", "enum": ["engineer", "reviewer", "researcher", "tester", "writer"] },
                "task": { "type": "string" },
                "name": { "type": "string", "description": "A short name for the new session." }
            },
            "required": ["role", "task"]
        }),
    },
];

pub fn def(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The specs sent to the model for a role's tools.
pub fn specs(names: &[&str]) -> Vec<ToolSpec> {
    names
        .iter()
        .filter_map(|n| def(n))
        .map(|t| ToolSpec { name: t.name.to_string(), description: t.description.to_string(), schema: (t.schema)() })
        .collect()
}

fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing `{key}`."))
}

fn opt_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

fn opt_u64(input: &Value, key: &str) -> Option<u64> {
    input.get(key).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)))
}

/// What a call is about to do, worked out from its input before it runs.
#[derive(Debug, Clone)]
pub struct Plan {
    pub access: Access,
    /// Paths it touches, resolved.
    pub paths: Vec<PathBuf>,
    /// One line for the activity feed and the approval card.
    pub summary: String,
    /// The change or the command in full, for the approval card.
    pub detail: String,
}

impl Plan {
    /// Whether it reaches outside the session folder.
    pub fn outside(&self, ws: &Workspace) -> bool {
        self.paths.iter().any(|p| !ws.contains(p))
    }
}

/// Reads the call's input and says what it will do; an Err is answered to the
/// model as a failed call without asking anyone.
pub fn plan(ws: &Workspace, call: &ToolCall) -> Result<Plan, String> {
    if let Some(raw) = &call.bad_arguments {
        let shown: String = raw.chars().take(200).collect();
        return Err(format!("The arguments were not valid JSON ({shown}). Send them again as a JSON object."));
    }
    let tool = def(&call.name).ok_or_else(|| format!("There is no tool named {}.", call.name))?;
    let input = &call.input;
    let at = |key: &str| -> Result<PathBuf, String> { ws.resolve(opt_str(input, key).unwrap_or("")) };
    let plan = |paths: Vec<PathBuf>, summary: String, detail: String| Plan { access: tool.access, paths, summary, detail };
    Ok(match tool.name {
        "read_file" => {
            let path = ws.resolve(str_arg(input, "path")?)?;
            plan(vec![path.clone()], format!("Read {}", ws.show(&path)), String::new())
        }
        "write_file" => {
            let path = ws.resolve(str_arg(input, "path")?)?;
            let content = str_arg(input, "content")?;
            let detail = write_preview(&path, content);
            let verb = if path.exists() { "Overwrite" } else { "Create" };
            plan(vec![path.clone()], format!("{verb} {}", ws.show(&path)), detail)
        }
        "edit_file" => {
            let path = ws.resolve(str_arg(input, "path")?)?;
            let old = str_arg(input, "old_string")?;
            let new = str_arg(input, "new_string")?;
            plan(vec![path.clone()], format!("Edit {}", ws.show(&path)), edit_preview(old, new))
        }
        "list_dir" => {
            let path = at("path")?;
            plan(vec![path.clone()], format!("List {}", ws.show(&path)), String::new())
        }
        "glob" => {
            let path = at("path")?;
            let pattern = str_arg(input, "pattern")?;
            plan(vec![path.clone()], format!("Find {pattern} in {}", ws.show(&path)), String::new())
        }
        "grep" => {
            let path = at("path")?;
            let pattern = str_arg(input, "pattern")?;
            plan(vec![path.clone()], format!("Search \"{pattern}\" in {}", ws.show(&path)), String::new())
        }
        "run_command" => {
            let command = str_arg(input, "command")?.trim();
            if command.is_empty() {
                return Err("The command is empty.".into());
            }
            let first = command.lines().next().unwrap_or("");
            let summary: String = first.chars().take(120).collect();
            plan(Vec::new(), format!("Run {summary}"), command.to_string())
        }
        "web_fetch" => {
            let url = str_arg(input, "url")?.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("Only http and https addresses can be fetched.".into());
            }
            plan(Vec::new(), format!("Fetch {url}"), url.to_string())
        }
        "todo_write" => plan(Vec::new(), "Update the plan".into(), String::new()),
        "delegate" => {
            let role = str_arg(input, "role")?;
            let task = str_arg(input, "task")?;
            let first: String = task.lines().next().unwrap_or("").chars().take(100).collect();
            plan(Vec::new(), format!("Delegate to a {role}: {first}"), task.to_string())
        }
        other => return Err(format!("There is no tool named {other}.")),
    })
}

fn limit_lines(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max {
        return text.to_string();
    }
    format!("{}\n[... {} more lines]", lines[..max].join("\n"), lines.len() - max)
}

fn prefixed(text: &str, mark: char) -> String {
    text.lines().map(|l| format!("{mark} {l}")).collect::<Vec<_>>().join("\n")
}

fn edit_preview(old: &str, new: &str) -> String {
    let half = MAX_DETAIL_LINES / 2;
    format!("{}\n{}", limit_lines(&prefixed(old, '-'), half), limit_lines(&prefixed(new, '+'), half))
}

fn write_preview(path: &Path, content: &str) -> String {
    let new_lines = content.lines().count();
    let head = match std::fs::read_to_string(path) {
        Ok(old) => format!("Replaces {} lines with {new_lines}:", old.lines().count()),
        Err(_) => format!("New file, {new_lines} lines:"),
    };
    format!("{head}\n{}", limit_lines(&prefixed(content, '+'), MAX_DETAIL_LINES))
}

/// What a tool needs while it runs.
pub struct Context<'a> {
    pub ws: &'a Workspace,
    pub shell: &'a Shell,
    pub stop: watch::Receiver<bool>,
}

/// Runs a call with a file, search, command or network tool. Ok is the
/// result text; Err the error the model reads.
pub async fn run(ctx: Context<'_>, call: &ToolCall) -> Result<String, String> {
    let input = call.input.clone();
    let ws = ctx.ws.clone();
    match call.name.as_str() {
        "read_file" | "write_file" | "edit_file" | "list_dir" | "glob" | "grep" => {
            let name = call.name.clone();
            tauri::async_runtime::spawn_blocking(move || run_fs(&ws, &name, &input))
                .await
                .map_err(|e| e.to_string())?
        }
        "run_command" => {
            let command = str_arg(&input, "command")?;
            let secs = opt_u64(&input, "timeout_seconds").unwrap_or(shell::DEFAULT_TIMEOUT).clamp(1, shell::MAX_TIMEOUT);
            let out = shell::run(ctx.shell, command, ws.root(), Duration::from_secs(secs), ctx.stop).await?;
            if out.failed() { Err(out.report()) } else { Ok(out.report()) }
        }
        "web_fetch" => fetch(str_arg(&input, "url")?).await,
        other => Err(format!("{other} can't run here.")),
    }
}

fn run_fs(ws: &Workspace, name: &str, input: &Value) -> Result<String, String> {
    let at = |key: &str| ws.resolve(opt_str(input, key).unwrap_or(""));
    match name {
        "read_file" => {
            let path = ws.resolve(str_arg(input, "path")?)?;
            read(&path, opt_u64(input, "offset").unwrap_or(1) as usize, opt_u64(input, "limit").unwrap_or(MAX_READ_LINES as u64) as usize)
        }
        "write_file" => {
            let path = ws.resolve(str_arg(input, "path")?)?;
            let content = str_arg(input, "content")?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("Can't create {}: {e}", parent.display()))?;
            }
            let existed = path.exists();
            std::fs::write(&path, content).map_err(|e| format!("Can't write {}: {e}", ws.show(&path)))?;
            Ok(format!("{} {} ({} lines).", if existed { "Replaced" } else { "Created" }, ws.show(&path), content.lines().count()))
        }
        "edit_file" => {
            let path = ws.resolve(str_arg(input, "path")?)?;
            let old = str_arg(input, "old_string")?;
            let new = str_arg(input, "new_string")?;
            let all = input.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("Can't read {}: {e}", ws.show(&path)))?;
            let updated = apply_edit(&text, old, new, all)?;
            std::fs::write(&path, &updated.0).map_err(|e| format!("Can't write {}: {e}", ws.show(&path)))?;
            Ok(format!("Edited {} ({} replacement{}).", ws.show(&path), updated.1, if updated.1 == 1 { "" } else { "s" }))
        }
        "list_dir" => list(ws, &at("path")?),
        "glob" => {
            let root = at("path")?;
            let (files, more) = search::glob(&root, str_arg(input, "pattern")?)?;
            if files.is_empty() {
                return Ok("No files match.".into());
            }
            let mut out = files.join("\n");
            if more {
                out.push_str(&format!("\n[Only the first {} shown: narrow the pattern.]", search::MAX_GLOB_RESULTS));
            }
            Ok(out)
        }
        "grep" => {
            let root = at("path")?;
            let ignore_case = input.get("ignore_case").and_then(Value::as_bool).unwrap_or(false);
            let (lines, more) = search::grep(&root, str_arg(input, "pattern")?, opt_str(input, "glob"), ignore_case)?;
            if lines.is_empty() {
                return Ok("No matches.".into());
            }
            let mut out = lines.join("\n");
            if more {
                out.push_str(&format!("\n[Only the first {} matches shown: narrow the search.]", search::MAX_GREP_RESULTS));
            }
            Ok(out)
        }
        other => Err(format!("{other} is not a file tool.")),
    }
}

/// `old` replaced by `new`: exactly one occurrence unless `all`.
pub fn apply_edit(text: &str, old: &str, new: &str, all: bool) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err("`old_string` is empty: use write_file to create a file.".into());
    }
    if old == new {
        return Err("`old_string` and `new_string` are the same.".into());
    }
    // Files with Windows line ends still match an edit written with \n.
    let crlf = text.contains("\r\n") && !old.contains("\r\n");
    let (old, new) = if crlf { (old.replace('\n', "\r\n"), new.replace('\n', "\r\n")) } else { (old.to_string(), new.to_string()) };
    let count = text.matches(&old).count();
    match (count, all) {
        (0, _) => Err("`old_string` was not found in the file. Read the file again and copy the text exactly.".into()),
        (1, _) | (_, true) => Ok((text.replace(&old, &new), count)),
        (n, false) => Err(format!("`old_string` appears {n} times. Add surrounding lines to make it unique, or set replace_all.")),
    }
}

fn read(path: &Path, offset: usize, limit: usize) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| format!("There is no file at {}.", path.display()))?;
    if meta.is_dir() {
        return Err(format!("{} is a folder: use list_dir.", path.display()));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err("The file is over 10 MB: search it with grep instead.".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Err("This is a binary file.".into());
    }
    let text = String::from_utf8_lossy(&bytes);
    if text.is_empty() {
        return Ok("(empty file)".into());
    }
    let offset = offset.max(1);
    let limit = limit.clamp(1, MAX_READ_LINES);
    let total = text.lines().count();
    let mut out = String::new();
    for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
        let line = line.trim_end_matches('\r');
        let shown: String = line.chars().take(MAX_LINE).collect();
        out.push_str(&format!("{:>6}\t{shown}\n", i + 1));
    }
    if out.is_empty() {
        return Err(format!("The file has {total} lines; offset {offset} is past the end."));
    }
    let last = (offset - 1 + limit).min(total);
    if last < total {
        out.push_str(&format!("[Lines {offset}-{last} of {total}. Read on with offset {}.]", last + 1));
    }
    Ok(out.trim_end().to_string())
}

fn list(ws: &Workspace, dir: &Path) -> Result<String, String> {
    let entries = std::fs::read_dir(dir).map_err(|_| format!("There is no folder at {}.", ws.show(dir)))?;
    let mut rows: Vec<(bool, String)> = entries
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = e.metadata().ok();
            if meta.as_ref().is_some_and(|m| m.is_dir()) {
                (true, format!("{name}/"))
            } else {
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let changed = meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| format!(", changed {}", super::session::date_time(d.as_millis() as u64)))
                    .unwrap_or_default();
                (false, format!("{name} ({}{changed})", human_size(size)))
            }
        })
        .collect();
    if rows.is_empty() {
        return Ok("(empty folder)".into());
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let total = rows.len();
    let mut out: Vec<String> = rows.into_iter().take(MAX_LIST).map(|r| r.1).collect();
    if total > MAX_LIST {
        out.push(format!("[{} more entries]", total - MAX_LIST));
    }
    Ok(out.join("\n"))
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

async fn fetch(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (compatible; Coucou agent)")
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(url).send().await.map_err(|e| format!("Can't reach {url}: {e}"))?;
    let status = response.status();
    let html = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("html"));
    let body = response.text().await.map_err(|e| format!("Can't read the page: {e}"))?;
    let text = if html { html_to_text(&body) } else { body };
    let text = shell::clip(text.trim(), MAX_FETCH);
    if status.is_success() { Ok(text) } else { Err(format!("HTTP {}: {text}", status.as_u16())) }
}

/// A page's readable text: no scripts, styles or tags, entities decoded.
pub fn html_to_text(html: &str) -> String {
    // ASCII only: the same byte offsets as `html`.
    let lower = html.to_ascii_lowercase();
    let mut text = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < html.len() {
        if bytes[i] == b'<' {
            // Skip a whole script or style element.
            let mut skipped = false;
            for tag in ["script", "style", "noscript", "svg"] {
                if lower[i + 1..].starts_with(tag) {
                    let close = format!("</{tag}");
                    i = lower[i..].find(&close).map(|p| i + p).unwrap_or(html.len());
                    i = lower[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
                    skipped = true;
                    break;
                }
            }
            if skipped {
                continue;
            }
            let end = lower[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
            let tag = &lower[i..end];
            if ["<br", "<p", "</p", "<div", "</div", "<li", "<h1", "<h2", "<h3", "<h4", "<tr", "</tr", "<pre", "</pre"]
                .iter()
                .any(|t| tag.starts_with(t))
            {
                text.push('\n');
            }
            i = end;
            continue;
        }
        let next = html[i..].find('<').map(|p| i + p).unwrap_or(html.len());
        text.push_str(&html[i..next]);
        i = next;
    }
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&");
    let mut out = String::new();
    let mut blank = 0;
    for line in decoded.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, input: Value) -> ToolCall {
        ToolCall { id: "c".into(), name: name.into(), input, bad_arguments: None, raw: None }
    }

    fn folder(name: &str) -> (PathBuf, Workspace) {
        let dir = std::env::temp_dir().join(format!("coucou-tools-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::open(&dir.to_string_lossy()).unwrap();
        (dir, ws)
    }

    #[test]
    fn every_tool_has_a_valid_schema_and_a_unique_name() {
        let names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        for spec in specs(&names) {
            assert_eq!(spec.schema["type"], "object", "{}", spec.name);
        }
        assert_eq!(specs(&["read_file", "nope"]).len(), 1);
    }

    #[test]
    fn plans_say_what_will_change_before_it_does() {
        let (dir, ws) = folder("plan");
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let p = plan(&ws, &call("edit_file", json!({"path": "a.txt", "old_string": "two", "new_string": "deux"}))).unwrap();
        assert_eq!((p.access, p.summary.as_str(), p.detail.as_str()), (Access::Write, "Edit a.txt", "- two\n+ deux"));
        assert!(!p.outside(&ws));
        let p = plan(&ws, &call("write_file", json!({"path": "../x.txt", "content": "hi"}))).unwrap();
        assert!(p.outside(&ws));
        assert!(p.detail.starts_with("New file, 1 lines:"));
        let p = plan(&ws, &call("run_command", json!({"command": "npm test\necho done"}))).unwrap();
        assert_eq!((p.access, p.summary.as_str()), (Access::Exec, "Run npm test"));
        assert!(plan(&ws, &call("run_command", json!({"command": "  "}))).is_err());
        assert!(plan(&ws, &call("web_fetch", json!({"url": "file:///c:/secret"}))).is_err());
        assert!(plan(&ws, &call("read_file", json!({}))).unwrap_err().contains("path"));
        assert!(plan(&ws, &call("rm_rf", json!({}))).is_err());
        let mut bad = call("read_file", json!({}));
        bad.bad_arguments = Some("{oops".into());
        assert!(plan(&ws, &bad).unwrap_err().contains("not valid JSON"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edits_need_one_exact_match_and_keep_windows_line_ends() {
        assert_eq!(apply_edit("a b a", "b", "c", false).unwrap(), ("a c a".into(), 1));
        assert!(apply_edit("a b a", "a", "c", false).unwrap_err().contains("2 times"));
        assert_eq!(apply_edit("a b a", "a", "c", true).unwrap(), ("c b c".into(), 2));
        assert!(apply_edit("abc", "x", "y", false).unwrap_err().contains("not found"));
        assert!(apply_edit("abc", "", "y", false).is_err());
        assert_eq!(apply_edit("one\r\ntwo\r\n", "one\ntwo", "1\n2", false).unwrap().0, "1\r\n2\r\n");
    }

    #[test]
    fn file_tools_read_write_edit_and_list() {
        let (dir, ws) = folder("fs");
        let run = |name: &str, input: Value| run_fs(&ws, name, &input);
        assert_eq!(run("write_file", json!({"path": "src/a.rs", "content": "fn a() {}\nfn b() {}\n"})).unwrap(), "Created src/a.rs (2 lines).");
        assert_eq!(run("read_file", json!({"path": "src/a.rs"})).unwrap(), "     1\tfn a() {}\n     2\tfn b() {}");
        assert_eq!(run("read_file", json!({"path": "src/a.rs", "offset": 2, "limit": 1})).unwrap(), "     2\tfn b() {}");
        let partial = run("read_file", json!({"path": "src/a.rs", "limit": 1})).unwrap();
        assert!(partial.ends_with("[Lines 1-1 of 2. Read on with offset 2.]"));
        assert!(run("edit_file", json!({"path": "src/a.rs", "old_string": "fn b", "new_string": "fn c"})).is_ok());
        assert!(std::fs::read_to_string(dir.join("src/a.rs")).unwrap().contains("fn c()"));
        assert_eq!(run("list_dir", json!({})).unwrap(), "src/");
        assert!(run("list_dir", json!({"path": "src"})).unwrap().starts_with("a.rs ("));
        assert_eq!(run("glob", json!({"pattern": "*.rs"})).unwrap(), "src/a.rs");
        assert_eq!(run("grep", json!({"pattern": "fn c"})).unwrap(), "src/a.rs:2: fn c() {}");
        assert!(run("read_file", json!({"path": "missing.txt"})).is_err());
        assert!(run("read_file", json!({"path": "src"})).unwrap_err().contains("folder"));
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2]).unwrap();
        assert!(run("read_file", json!({"path": "bin.dat"})).unwrap_err().contains("binary"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pages_lose_their_markup() {
        let html = "<html><head><style>p{}</style><script>var x = '<p>';</script></head><body><h1>Title</h1><p>One &amp; two</p><ul><li>a</li><li>b</li></ul></body></html>";
        // At most one blank line between blocks.
        assert_eq!(html_to_text(html), "Title\nOne & two\n\na\nb");
    }
}
