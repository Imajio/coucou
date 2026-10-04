// A session's folder: where its relative paths start, and the line between
// what it may touch on its own and what needs the user's say.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// The folder must exist; it is kept in its canonical form.
    pub fn open(folder: &str) -> Result<Self, String> {
        let folder = folder.trim();
        if folder.is_empty() {
            return Err("Pick the folder this session works in.".into());
        }
        let path = Path::new(folder);
        if !path.is_absolute() {
            return Err(format!("{folder} is not a full path to a folder."));
        }
        if !path.is_dir() {
            return Err(format!("There is no folder at {folder}."));
        }
        let root = canonical(path).ok_or_else(|| format!("Can't open {folder}."))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A path the model gave, made absolute (relative paths start in the
    /// folder) and free of `.` and `..`. Symbolic links are followed as far as
    /// the path exists, so a link can't hide a path outside the folder.
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(self.root.clone());
        }
        let given = Path::new(raw);
        let joined = if given.is_absolute() { given.to_path_buf() } else { self.root.join(given) };
        let lexical = normalize(&joined).ok_or_else(|| format!("{raw} goes above the top of the disk."))?;
        // Canonicalize the part that exists, keep the rest as written.
        let mut existing = lexical.clone();
        let mut rest = Vec::new();
        while !existing.exists() {
            match (existing.file_name().map(|n| n.to_os_string()), existing.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name);
                    existing = parent.to_path_buf();
                }
                _ => return Ok(lexical),
            }
        }
        let mut out = canonical(&existing).unwrap_or(existing);
        for name in rest.into_iter().rev() {
            out.push(name);
        }
        Ok(out)
    }

    /// Whether a resolved path is the folder or inside it.
    pub fn contains(&self, path: &Path) -> bool {
        starts_with(path, &self.root)
    }

    /// The path as the model should read it: relative inside the folder.
    pub fn show(&self, path: &Path) -> String {
        match path.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }
}

/// `std::fs::canonicalize` without Windows' `\\?\` prefix on ordinary paths.
pub fn canonical(path: &Path) -> Option<PathBuf> {
    let full = std::fs::canonicalize(path).ok()?;
    let text = full.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            return Some(PathBuf::from(rest));
        }
    }
    Some(full)
}

/// Removes `.` and `..` without touching the disk; None if `..` climbs past the root.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() || out.as_os_str().is_empty() {
                    return None;
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// Component-wise prefix test; case-insensitive where the file system is.
fn starts_with(path: &Path, root: &Path) -> bool {
    let mut path = path.components();
    for want in root.components() {
        let Some(got) = path.next() else { return false };
        let (a, b) = (got.as_os_str().to_string_lossy(), want.as_os_str().to_string_lossy());
        let same = if cfg!(windows) { a.eq_ignore_ascii_case(&b) } else { a == b };
        if !same {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coucou-ws-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    #[test]
    fn relative_paths_start_in_the_folder_and_dots_are_resolved() {
        let dir = temp("rel");
        let ws = Workspace::open(&dir.to_string_lossy()).unwrap();
        let inside = ws.resolve("src/../src/./main.rs").unwrap();
        assert!(ws.contains(&inside));
        assert_eq!(ws.show(&inside), "src/main.rs");
        assert_eq!(ws.show(&ws.resolve("").unwrap()), ".");
        let outside = ws.resolve("../elsewhere.txt").unwrap();
        assert!(!ws.contains(&outside));
        // A sibling folder that shares the name's start is not inside.
        let sibling = PathBuf::from(format!("{}-other", ws.root().display())).join("x");
        assert!(!ws.contains(&sibling));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_existing_absolute_folders_open() {
        assert!(Workspace::open("").is_err());
        assert!(Workspace::open("relative/folder").is_err());
        let missing = std::env::temp_dir().join("coucou-ws-missing-xyz");
        assert!(Workspace::open(&missing.to_string_lossy()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_without_case_and_without_the_verbatim_prefix() {
        let dir = temp("case");
        let ws = Workspace::open(&dir.to_string_lossy()).unwrap();
        assert!(!ws.root().to_string_lossy().starts_with(r"\\?\"));
        let upper = PathBuf::from(ws.root().to_string_lossy().to_uppercase()).join("SRC");
        assert!(ws.contains(&upper));
        std::fs::remove_dir_all(&dir).ok();
    }
}
