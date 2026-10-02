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
