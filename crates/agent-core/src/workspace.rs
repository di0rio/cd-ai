use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

/// What the UI needs to show an open workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct WorkspaceInfo {
    pub name: String,
    pub root: String,
}

/// A folder the user opened. Every path the agent touches must resolve inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

#[derive(Debug)]
pub enum WorkspaceError {
    NotFound(PathBuf),
    NotADirectory(PathBuf),
    OutsideWorkspace(PathBuf),
    Io(io::Error),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "pasta não encontrada: {}", path.display()),
            Self::NotADirectory(path) => write!(f, "não é uma pasta: {}", path.display()),
            Self::OutsideWorkspace(path) => {
                write!(f, "caminho fora do workspace: {}", path.display())
            }
            Self::Io(error) => write!(f, "erro de E/S: {error}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl Workspace {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let path = path.as_ref();
        let root = path.canonicalize().map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => WorkspaceError::NotFound(path.to_path_buf()),
            _ => WorkspaceError::Io(error),
        })?;
        if !root.is_dir() {
            return Err(WorkspaceError::NotADirectory(path.to_path_buf()));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a workspace-relative (or absolute) path to a canonical path inside the workspace.
    /// Paths that do not exist yet are allowed, so callers can create files.
    pub fn resolve(&self, path: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let requested = path.as_ref();
        let outside = || WorkspaceError::OutsideWorkspace(requested.to_path_buf());

        // 1. Lexical normalization. An absolute input starts from itself and must still land inside the root.
        let mut lexical = if requested.is_absolute() {
            PathBuf::new()
        } else {
            self.root.clone()
        };
        for component in requested.components() {
            match component {
                Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                    lexical.push(component)
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if !lexical.pop() {
                        return Err(outside());
                    }
                }
            }
        }
        if !lexical.starts_with(&self.root) {
            return Err(outside());
        }

        // 2. Let the OS resolve symlinks on the deepest part that exists; the rest are plain names to be created.
        let mut existing = lexical;
        let mut missing = Vec::new();
        while fs::symlink_metadata(&existing).is_err() {
            match existing.file_name() {
                Some(name) => missing.push(name.to_os_string()),
                None => return Err(outside()),
            }
            existing.pop();
        }
        // A dangling symlink fails to canonicalize and is refused: writing through it could land anywhere.
        let canonical = existing.canonicalize().map_err(|_| outside())?;
        if !canonical.starts_with(&self.root) {
            return Err(outside());
        }
        Ok(missing
            .into_iter()
            .rev()
            .fold(canonical, |path, name| path.join(name)))
    }

    pub fn info(&self) -> WorkspaceInfo {
        let root = self.root.to_string_lossy();
        WorkspaceInfo {
            name: self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.to_string()),
            // Strip the Windows verbatim prefix for display only; comparisons keep the canonical form.
            root: root.trim_start_matches(r"\\?\").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_missing_folder_is_not_found() {
        let dir = tempdir().unwrap();
        let result = Workspace::open(dir.path().join("nope"));
        assert!(matches!(result, Err(WorkspaceError::NotFound(_))));
    }

    #[test]
    fn open_file_is_not_a_directory() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, "x").unwrap();
        let result = Workspace::open(&file);
        assert!(matches!(result, Err(WorkspaceError::NotADirectory(_))));
    }

    #[test]
    fn resolves_existing_file_inside() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let main = src.join("main.rs");
        fs::write(&main, "fn main() {}").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let p = ws.resolve("src/main.rs").expect("should resolve inside");
        assert!(p.starts_with(ws.root()));
        assert!(p.ends_with("main.rs"));
    }

    #[test]
    fn resolves_root_for_dot() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let p = ws.resolve(".").expect("should resolve to root");
        assert_eq!(p, ws.root());
    }

    #[test]
    fn allows_parent_that_stays_inside() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let p = ws.resolve("src/../README.md").expect("should stay inside");
        assert_eq!(p, ws.root().join("README.md"));
    }

    #[test]
    fn rejects_parent_escape() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let result = ws.resolve("../outside.txt");
        assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))));
    }

    #[test]
    fn rejects_nested_parent_escape() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let result = ws.resolve("a/../../x");
        assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))));
    }

    #[test]
    fn rejects_absolute_path_outside() {
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let result = ws.resolve(other.path().join("x"));
        assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))));
    }

    #[test]
    fn allows_new_nested_file() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let p = ws
            .resolve("new/dir/file.txt")
            .expect("should allow new file");
        assert_eq!(p, ws.root().join("new").join("dir").join("file.txt"));
    }

    #[test]
    fn rejects_escape_through_missing_dirs() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let result = ws.resolve("new/../../x");
        assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_that_escapes() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let root = dir.path();
        let ws = Workspace::open(root).unwrap();
        symlink(other.path(), root.join("link")).unwrap();
        let result = ws.resolve("link/secret.txt");
        assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_dangling_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let root = dir.path();
        let ws = Workspace::open(root).unwrap();
        symlink(other.path().join("missing"), root.join("dangling")).unwrap();
        let result = ws.resolve("dangling");
        assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))));
    }

    #[test]
    fn info_uses_folder_name() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("meu-projeto");
        fs::create_dir(&sub).unwrap();
        let ws = Workspace::open(&sub).unwrap();
        let info = ws.info();
        assert_eq!(info.name, "meu-projeto");
        assert!(!info.root.starts_with(r"\\?\"));
    }
}
