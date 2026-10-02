// Everything that differs between operating systems, behind one set of names.
//
// The rest of the app calls `platform::…` and never touches Win32 or a Linux
// API directly. Each OS file exposes the same functions; the compiler picks one.

use std::path::PathBuf;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use self::linux::*;

/// What is playing, for the Music tab (see crate::media). Same functions on
/// every OS: `sessions`, `control`, `set_volume`, `artwork`.
#[cfg(windows)]
#[path = "windows_media.rs"]
pub mod media;
#[cfg(target_os = "linux")]
#[path = "linux_media.rs"]
pub mod media;

/// Wall-clock time in the user's time zone, for log lines and backup names.
pub struct LocalTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// A top-level window that could hold a session: its handle, owning process and title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopWindow {
    pub handle: isize,
    pub pid: u32,
    pub title: String,
}

/// The window of a session: the nearest of its host processes that owns a
/// visible window wins (the terminal or the editor holding the shell). When
/// that process has several (VS Code, one per folder), the one whose title
/// names the project does. `consoles` maps a process to the console hosts
/// (conhost, OpenConsole) drawing its window.
pub fn pick_session_window(
    host_pids: &[u32],
    consoles: &std::collections::HashMap<u32, Vec<u32>>,
    windows: &[TopWindow],
    project: Option<&str>,
) -> Option<isize> {
    let project = project.map(str::to_lowercase).filter(|p| !p.is_empty());
    for pid in host_pids {
        let mut owners = vec![*pid];
        owners.extend(consoles.get(pid).into_iter().flatten().copied());
        let hits: Vec<&TopWindow> = windows.iter().filter(|w| owners.contains(&w.pid)).collect();
        if hits.is_empty() {
            continue;
        }
        let named = project
            .as_deref()
            .and_then(|p| hits.iter().find(|w| w.title.to_lowercase().contains(p)));
        return Some(named.unwrap_or(&hits[0]).handle);
    }
    None
}

/// The window Mochi was dropped on, as the chat gets it for context.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub title: String,
    /// The page's address, for browsers.
    pub url: Option<String>,
}

/// Browsers whose address bar is read for the page's URL (executable stems).
pub const BROWSERS: &[&str] = &["msedge", "chrome", "brave", "opera", "vivaldi", "firefox", "chromium", "arc"];

/// A readable app name from its executable stem.
pub fn app_display_name(stem: &str) -> String {
    match stem.to_lowercase().as_str() {
        "code" => "Visual Studio Code".into(),
        "explorer" => "File Explorer".into(),
        "windowsterminal" => "Windows Terminal".into(),
        "winword" => "Word".into(),
        "excel" => "Excel".into(),
        "powerpnt" => "PowerPoint".into(),
        "outlook" | "olk" => "Outlook".into(),
        "notepad" => "Notepad".into(),
        "acrord32" | "acrobat" => "Adobe Acrobat".into(),
        _ => crate::media::friendly_app_name(stem),
    }
}

/// An address bar's text as a link: browsers hide the scheme, and a bar
/// holding search words (being edited) is no address at all.
pub fn address_to_url(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let lower = text.to_lowercase();
    if lower.contains("://") || ["about:", "edge:", "chrome:", "file:"].iter().any(|s| lower.starts_with(s)) {
        return Some(text.to_string());
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or("");
    let looks_like_host = host == "localhost" || host.starts_with("localhost:") || (host.contains('.') && !host.ends_with('.'));
    looks_like_host.then(|| format!("https://{text}"))
}

/// The user's home directory, where `.claude/settings.json` lives.
pub fn home_dir() -> PathBuf {
    std::env::var_os(HOME_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn w(handle: isize, pid: u32, title: &str) -> TopWindow {
        TopWindow { handle, pid, title: title.into() }
    }

    #[test]
    fn address_bars_become_links() {
        assert_eq!(address_to_url("github.com/Imajio/coucou").as_deref(), Some("https://github.com/Imajio/coucou"));
        assert_eq!(address_to_url("https://example.org/a?b=1").as_deref(), Some("https://example.org/a?b=1"));
        assert_eq!(address_to_url("localhost:1420/").as_deref(), Some("https://localhost:1420/"));
        assert_eq!(address_to_url("edge://settings").as_deref(), Some("edge://settings"));
        assert_eq!(address_to_url("how to bake bread"), None);
        assert_eq!(address_to_url("weather"), None);
        assert_eq!(address_to_url(""), None);
    }

    #[test]
    fn apps_get_their_usual_names() {
        assert_eq!(app_display_name("Code"), "Visual Studio Code");
        assert_eq!(app_display_name("msedge"), "Microsoft Edge");
        assert_eq!(app_display_name("WINWORD"), "Word");
        assert_eq!(app_display_name("figma"), "Figma");
    }

    #[test]
    fn the_nearest_host_with_a_window_wins() {
        // claude 30 → pwsh 20 → WindowsTerminal 10
        let windows = [w(1, 10, "PowerShell"), w(2, 99, "Something else")];
        assert_eq!(pick_session_window(&[30, 20, 10], &HashMap::new(), &windows, None), Some(1));
    }

    #[test]
    fn among_editor_windows_the_project_one_wins() {
        let windows = [w(1, 7, "notes - Visual Studio Code"), w(2, 7, "coucou - Visual Studio Code")];
        assert_eq!(pick_session_window(&[30, 7], &HashMap::new(), &windows, Some("Coucou")), Some(2));
        assert_eq!(pick_session_window(&[30, 7], &HashMap::new(), &windows, Some("other")), Some(1));
    }

    #[test]
    fn a_classic_console_is_found_through_its_host() {
        // cmd 20 is drawn by conhost 21, which owns the window.
        let consoles: HashMap<u32, Vec<u32>> = [(20, vec![21])].into();
        let windows = [w(5, 21, "Command Prompt - claude")];
        assert_eq!(pick_session_window(&[30, 20], &consoles, &windows, None), Some(5));
        assert_eq!(pick_session_window(&[30], &consoles, &windows, None), None);
    }
}
