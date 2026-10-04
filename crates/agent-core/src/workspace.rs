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
                Component::Normal(name) if cfg!(windows) && windows_unsafe_name(name) => {
                    return Err(outside());
                }
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
        let lexical = self.rebase_displayed_root(lexical);
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

    /// On Windows the canonical root is `\\?\C:\...` while the UI and the prompt show `C:\...`; an
    /// absolute path copied from what the model was shown names the same folder.
    fn rebase_displayed_root(&self, path: PathBuf) -> PathBuf {
        let Some(shown) = self
            .root
            .to_str()
            .and_then(|root| root.strip_prefix(r"\\?\"))
        else {
            return path;
        };
        match path.strip_prefix(shown) {
            Ok(rest) => self.root.join(rest),
            Err(_) => path,
        }
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

/// Refuses a folder that is too broad to be a project: a filesystem root (`/`, `C:\`), the home
/// directory, or a folder that contains it (`/home`, `C:\Users`). The agent may read the whole
/// workspace, and a sandboxed command may write all of it, so opening `~` would hand over
/// `~/.ssh`, `~/.aws` and every other project. The message is for the user (pt-BR).
pub fn ensure_project_folder(path: &Path) -> Result<(), String> {
    ensure_project_folder_with(path, home_dir().as_deref())
}

fn ensure_project_folder_with(path: &Path, home: Option<&Path>) -> Result<(), String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let shown = canonical.to_string_lossy();
    let shown = shown.trim_start_matches(r"\\?\");
    let refuse = |why: &str| {
        Err(format!(
            "{shown}: {why}. Abra a pasta de um projeto, não uma pasta tão ampla."
        ))
    };
    if canonical.parent().is_none() {
        return refuse("é a raiz do sistema de arquivos");
    }
    if let Some(home) = home.and_then(|home| home.canonicalize().ok()) {
        if canonical == home {
            return refuse("é a sua pasta pessoal");
        }
        if home.starts_with(&canonical) {
            return refuse("contém a sua pasta pessoal");
        }
    }
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// Names Windows reads differently from what the path parser sees: `:` opens an alternate data
/// stream (`.env::$DATA` is `.env`, which would dodge the secret-file check) or swaps the drive,
/// trailing dots and spaces are dropped (`.git.` is `.git`), and device names (`NUL`, `COM1`,
/// `CON.txt`) open a device instead of a file. No legitimate project file needs any of them.
fn windows_unsafe_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    if name.contains(':') || name.ends_with('.') || name.ends_with(' ') {
        return true;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        let mut rest = stem.strip_prefix(prefix).into_iter().flat_map(str::chars);
        matches!((rest.next(), rest.next()), (Some(digit), None) if digit.is_ascii_digit() || "¹²³".contains(digit))
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn flags_names_windows_reinterprets() {
        for name in [
            ".env::$DATA",
            "file.txt:stream",
            "C:",
            ".git.",
            "dir ",
            "...",
            "NUL",
            "nul.txt",
            "Con",
            "COM1",
            "lpt9.log",
            "com¹",
            "CONOUT$",
        ] {
            assert!(windows_unsafe_name(name.as_ref()), "{name}");
        }
        for name in [
            ".env",
            ".git",
            "main.rs",
            "console.log",
            "com10",
            "comma",
            "lpt",
            "null",
            "a b",
        ] {
            assert!(!windows_unsafe_name(name.as_ref()), "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_aliases_of_a_workspace_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=x").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        for path in [
            ".env::$DATA",
            ".env:stream",
            ".env.",
            ".env ",
            "sub\\C:\\x",
            "C:x",
            "nul",
            "\\\\server\\share\\x",
            "\\\\.\\C:\\x",
        ] {
            assert!(
                matches!(ws.resolve(path), Err(WorkspaceError::OutsideWorkspace(_))),
                "{path}"
            );
        }
        assert!(ws.resolve(".env").is_ok());
    }

    #[test]
    fn refuses_roots_home_and_what_contains_home() {
        let outer = tempdir().unwrap();
        let home = outer.path().join("users").join("cauan");
        let project = home.join("projetos").join("app");
        fs::create_dir_all(&project).unwrap();
        let elsewhere = outer.path().join("trabalho").join("app");
        fs::create_dir_all(&elsewhere).unwrap();
        let check = |path: &Path| ensure_project_folder_with(path, Some(&home));

        // Home itself, and every folder above it up to the root.
        for broad in [&home, home.parent().unwrap(), outer.path()] {
            let error = check(broad).unwrap_err();
            assert!(error.contains("Abra a pasta de um projeto"), "{error}");
        }
        let root = outer.path().ancestors().last().unwrap().to_path_buf();
        let error = check(&root).unwrap_err();
        assert!(error.contains("raiz"), "{error}");
        let error = check(&home).unwrap_err();
        assert!(error.contains("pessoal"), "{error}");
        let error = check(home.parent().unwrap()).unwrap_err();
        assert!(error.contains("contém"), "{error}");

        // A project under home, or a folder that has nothing to do with it, is fine.
        assert!(check(&project).is_ok());
        assert!(check(&home.join("projetos")).is_ok());
        assert!(check(&elsewhere).is_ok());
        // No home known: only the root is refused.
        assert!(ensure_project_folder_with(&home, None).is_ok());
        assert!(ensure_project_folder_with(&root, None).is_err());
        // A folder that is not there is the caller's error, not a pass.
        assert!(check(&outer.path().join("nope")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_home_is_home() {
        let outer = tempdir().unwrap();
        let home = outer.path().join("home");
        fs::create_dir(&home).unwrap();
        let link = outer.path().join("atalho");
        std::os::unix::fs::symlink(&home, &link).unwrap();
        assert!(ensure_project_folder_with(&link, Some(&home)).is_err());
    }

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
    fn accepts_the_displayed_absolute_path() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        // The UI and the prompt show the root without the Windows verbatim prefix.
        let shown = PathBuf::from(ws.info().root).join("novo.txt");
        let p = ws
            .resolve(&shown)
            .expect("the root the user sees is inside");
        assert_eq!(p, ws.root().join("novo.txt"));
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
