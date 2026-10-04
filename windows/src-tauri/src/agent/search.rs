// Finding files and text in a session's folder: the glob and grep tools.
// Build output and dependency folders are skipped, as a person would.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use regex::{Regex, RegexBuilder};

/// Folders nobody means when they search a project.
const SKIPPED: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", ".next", ".venv", "venv", "__pycache__", ".idea",
    ".gradle", ".turbo", ".cache",
];
/// Files above this are not searched for text.
const MAX_GREP_FILE: u64 = 2 * 1024 * 1024;
/// A walk stops after this many files, so a huge tree can't stall a session.
const MAX_WALK: usize = 50_000;

pub const MAX_GLOB_RESULTS: usize = 200;
pub const MAX_GREP_RESULTS: usize = 200;

/// Every file under `root`, skipped folders aside.
pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                let name = entry.file_name();
                if !SKIPPED.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                    stack.push(path);
                }
            } else if kind.is_file() {
                files.push(path);
                if files.len() >= MAX_WALK {
                    return files;
                }
            }
        }
    }
    files
}

/// A glob as a regex over `/`-separated relative paths: `**` crosses folders,
/// `*` and `?` stay within one, `{a,b}` picks one, `[...]` is a class. A
/// pattern without `/` matches a file's name wherever it is.
pub fn glob_regex(pattern: &str) -> Result<Regex, String> {
    let pattern = pattern.trim().replace('\\', "/");
    if pattern.is_empty() {
        return Err("The glob pattern is empty.".into());
    }
    let anywhere = !pattern.contains('/');
    let mut re = String::from(if anywhere { "(?:^|/)" } else { "^" });
    let chars: Vec<char> = pattern.trim_start_matches("./").chars().collect();
    let mut i = 0;
    let mut braces = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    re.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    re.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '{' => {
                braces += 1;
                re.push_str("(?:");
            }
            '}' if braces > 0 => {
                braces -= 1;
                re.push(')');
            }
            ',' if braces > 0 => re.push('|'),
            '[' => {
                let end = chars[i + 1..].iter().position(|&x| x == ']').map(|p| i + 1 + p);
                match end {
                    Some(end) if end > i + 1 => {
                        let mut class: String = chars[i + 1..end].iter().collect();
                        if let Some(rest) = class.strip_prefix('!') {
                            class = format!("^{rest}");
                        }
                        re.push('[');
                        re.push_str(&class.replace('\\', "\\\\"));
                        re.push(']');
                        i = end + 1;
                        continue;
                    }
                    _ => re.push_str("\\["),
                }
            }
            other => re.push_str(&regex::escape(&other.to_string())),
        }
        i += 1;
    }
    if braces > 0 {
        return Err(format!("Unclosed {{ in {pattern}."));
    }
    re.push('$');
    RegexBuilder::new(&re)
        .case_insensitive(cfg!(windows))
        .build()
        .map_err(|e| format!("Bad glob {pattern}: {e}"))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Files matching `pattern` under `root`, most recently changed first.
pub fn glob(root: &Path, pattern: &str) -> Result<(Vec<String>, bool), String> {
    let re = glob_regex(pattern)?;
    let mut hits: Vec<(SystemTime, String)> = walk(root)
        .into_iter()
        .filter_map(|p| {
            let rel = relative(root, &p);
            re.is_match(&rel).then(|| (modified(&p), rel))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let more = hits.len() > MAX_GLOB_RESULTS;
    Ok((hits.into_iter().take(MAX_GLOB_RESULTS).map(|(_, p)| p).collect(), more))
}

/// Lines matching `pattern` (a regex), as `path:line: text`.
pub fn grep(root: &Path, pattern: &str, file_glob: Option<&str>, ignore_case: bool) -> Result<(Vec<String>, bool), String> {
    let re = RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| format!("Bad pattern: {e}"))?;
    let filter = file_glob.filter(|g| !g.trim().is_empty()).map(glob_regex).transpose()?;
    let files = if root.is_file() { vec![root.to_path_buf()] } else { walk(root) };
    let base = if root.is_file() { root.parent().unwrap_or(root) } else { root };
    let mut out = Vec::new();
    for path in files {
        let rel = relative(base, &path);
        if filter.as_ref().is_some_and(|f| !f.is_match(&rel)) {
            continue;
        }
        if std::fs::metadata(&path).map(|m| m.len() > MAX_GREP_FILE).unwrap_or(true) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if bytes.iter().take(8000).any(|&b| b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (n, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let line: String = line.trim_end().chars().take(300).collect();
                out.push(format!("{rel}:{}: {line}", n + 1));
                if out.len() > MAX_GREP_RESULTS {
                    out.truncate(MAX_GREP_RESULTS);
                    return Ok((out, true));
                }
            }
        }
    }
    Ok((out, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coucou-search-{name}-{}", std::process::id()));
        for (path, text) in [
            ("src/main.rs", "fn main() {\n    println!(\"hello\");\n}\n"),
            ("src/lib/util.rs", "pub fn Hello() {}\n"),
            ("README.md", "# Hello\n"),
            ("node_modules/x/index.js", "hello"),
            ("target/debug/out.rs", "hello"),
        ] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    #[test]
    fn globs_read_like_shell_globs() {
        let m = |p: &str, path: &str| glob_regex(p).unwrap().is_match(path);
        assert!(m("*.rs", "src/lib/util.rs"));
        assert!(m("src/*.rs", "src/main.rs"));
        assert!(!m("src/*.rs", "src/lib/util.rs"));
        assert!(m("src/**/*.rs", "src/main.rs"));
        assert!(m("src/**/*.rs", "src/lib/util.rs"));
        assert!(m("**/*.{ts,tsx}", "a/b/c.tsx"));
        assert!(!m("**/*.{ts,tsx}", "a/b/c.js"));
        assert!(m("file?.[ch]", "dir/file1.c"));
        assert!(!m("file?.[!ch]", "dir/file1.c"));
        assert!(m("a+b(1).txt", "a+b(1).txt"));
        assert!(glob_regex("{a,b").is_err());
    }

    #[test]
    fn glob_and_grep_skip_build_folders() {
        let dir = tree("skip");
        let (files, more) = glob(&dir, "**/*.rs").unwrap();
        let mut files = files;
        files.sort();
        assert_eq!(files, ["src/lib/util.rs", "src/main.rs"]);
        assert!(!more);
        let (lines, _) = grep(&dir, "hello", None, true).unwrap();
        let mut lines = lines;
        lines.sort();
        assert_eq!(lines, ["README.md:1: # Hello", "src/lib/util.rs:1: pub fn Hello() {}", "src/main.rs:2:     println!(\"hello\");"]);
        let (lines, _) = grep(&dir, "hello", Some("*.md"), true).unwrap();
        assert_eq!(lines, ["README.md:1: # Hello"]);
        let (lines, _) = grep(&dir.join("src/main.rs"), "main", None, false).unwrap();
        assert_eq!(lines, ["main.rs:1: fn main() {"]);
        assert!(grep(&dir, "(", None, false).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
