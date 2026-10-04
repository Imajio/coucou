// The run_command tool: one command in the session's folder, no console
// window, nothing to type into, and the whole process tree ended on a
// timeout or a stop so nothing outlives the session.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::watch;

pub const DEFAULT_TIMEOUT: u64 = 120;
pub const MAX_TIMEOUT: u64 = 600;
/// What the model gets back at most; the middle of a long log goes.
pub const MAX_OUTPUT: usize = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shell {
    Bash(PathBuf),
    PowerShell,
    Sh,
}

impl Shell {
    /// Git Bash when it is installed (models know bash best), else PowerShell.
    /// Never WSL's bash.exe from System32, which runs in another system.
    pub fn detect() -> Shell {
        if cfg!(windows) {
            let mut candidates: Vec<PathBuf> = Vec::new();
            for var in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
                if let Ok(base) = std::env::var(var) {
                    let base = PathBuf::from(base);
                    candidates.push(base.join("Git").join("bin").join("bash.exe"));
                    candidates.push(base.join("Programs").join("Git").join("bin").join("bash.exe"));
                }
            }
            match candidates.into_iter().find(|p| p.is_file()) {
                Some(bash) => Shell::Bash(bash),
                None => Shell::PowerShell,
            }
        } else {
            Shell::Sh
        }
    }

    /// How the system prompt names it.
    pub fn describe(&self) -> &'static str {
        match self {
            Shell::Bash(_) => "Git Bash (bash syntax, Unix paths such as /c/Users work too)",
            Shell::PowerShell => "Windows PowerShell 5.1 (PowerShell syntax; no && or ||)",
            Shell::Sh => "sh",
        }
    }

    fn command(&self, line: &str) -> tokio::process::Command {
        match self {
            Shell::Bash(bash) => {
                let mut c = tokio::process::Command::new(bash);
                c.arg("-c").arg(line);
                c
            }
            Shell::PowerShell => {
                let mut c = tokio::process::Command::new("powershell.exe");
                c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", line]);
                c
            }
            Shell::Sh => {
                let mut c = tokio::process::Command::new("sh");
                c.arg("-c").arg(line);
                c
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct CommandOutput {
    pub text: String,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stopped: bool,
}

impl CommandOutput {
    /// The tool result: output, then how it ended.
    pub fn report(&self) -> String {
        let mut out = if self.text.trim().is_empty() { "(no output)".to_string() } else { self.text.clone() };
        if self.timed_out {
            out.push_str("\n[The command took too long and was stopped.]");
        } else if self.stopped {
            out.push_str("\n[Stopped by the user.]");
        } else if let Some(code) = self.code.filter(|c| *c != 0) {
            out.push_str(&format!("\n[Exit code {code}]"));
        }
        out
    }

    pub fn failed(&self) -> bool {
        self.timed_out || self.stopped || self.code != Some(0)
    }
}

/// Keeps the start and the end of a long text, where errors usually are.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max / 3).collect();
    let tail: Vec<char> = text.chars().rev().take(max - max / 3).collect();
    let tail: String = tail.into_iter().rev().collect();
    format!("{head}\n[... output cut ...]\n{tail}")
}

/// Runs `line` in `cwd`. `stop` turning true ends it early, like the timeout.
pub async fn run(shell: &Shell, line: &str, cwd: &Path, timeout: Duration, mut stop: watch::Receiver<bool>) -> Result<CommandOutput, String> {
    let mut cmd = shell.command(line);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_COLOR", "1")
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Can't start the shell: {e}"))?;
    let pid = child.id();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let read_out = async move {
        let mut buf = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    };
    let read_err = async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    };
    let readers = tokio::spawn(async move { tokio::join!(read_out, read_err) });

    let mut timed_out = false;
    let mut stopped = false;
    let code = tokio::select! {
        status = child.wait() => status.ok().and_then(|s| s.code()),
        _ = tokio::time::sleep(timeout) => { timed_out = true; None }
        _ = async { while !*stop.borrow_and_update() { if stop.changed().await.is_err() { std::future::pending::<()>().await } } } => { stopped = true; None }
    };
    if timed_out || stopped {
        kill_tree(pid);
        let _ = child.kill().await;
    }
    let (out, err) = tokio::time::timeout(Duration::from_secs(5), readers)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();
    let mut text = String::from_utf8_lossy(&out).into_owned();
    let err = String::from_utf8_lossy(&err);
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok(CommandOutput { text: clip(text.trim_end(), MAX_OUTPUT), code, timed_out, stopped })
}

/// Ends a process and everything it started.
fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // The command leads its own process group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn long_output_keeps_its_start_and_end() {
        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let clipped = clip(&text, 300);
        assert!(clipped.starts_with("line 0\n"));
        assert!(clipped.ends_with("line 999\n"));
        assert!(clipped.contains("[... output cut ...]"));
        assert_eq!(clip("short", 300), "short");
    }

    #[test]
    fn a_command_runs_in_the_folder_and_reports_its_exit_code() {
        let shell = Shell::detect();
        let dir = std::env::temp_dir();
        let (_tx, rx) = watch::channel(false);
        let out = block_on(run(&shell, "echo coucou && exit 3", &dir, Duration::from_secs(60), rx)).unwrap();
        if shell == Shell::PowerShell {
            // PowerShell 5.1 has no &&: the parse error is the output.
            assert!(out.failed());
            return;
        }
        assert_eq!(out.text, "coucou");
        assert_eq!(out.code, Some(3));
        assert!(out.report().ends_with("[Exit code 3]"));
    }

    #[test]
    fn a_stop_ends_a_running_command() {
        let shell = Shell::detect();
        let (tx, rx) = watch::channel(false);
        let started = std::time::Instant::now();
        let line = if shell == Shell::PowerShell { "Start-Sleep 30" } else { "sleep 30" };
        let tmp = std::env::temp_dir();
        let out = block_on(async {
            let run = run(&shell, line, &tmp, Duration::from_secs(60), rx);
            let stop = async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let _ = tx.send(true);
            };
            tokio::join!(run, stop).0
        })
        .unwrap();
        assert!(out.stopped);
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn a_timeout_ends_a_running_command() {
        let shell = Shell::detect();
        let (_tx, rx) = watch::channel(false);
        let line = if shell == Shell::PowerShell { "Start-Sleep 30" } else { "sleep 30" };
        let out = block_on(run(&shell, line, &std::env::temp_dir(), Duration::from_millis(800), rx)).unwrap();
        assert!(out.timed_out);
        assert!(out.report().contains("too long"));
    }
}
