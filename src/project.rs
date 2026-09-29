//! Projects: a named set of source folders, the first of which is primary,
//! like a Codex project. A project lets a workspace such as a playground show
//! repositories its own `.gitignore` hides, snapshot them with agent turns and
//! list them in the Git panel. Projects live in Vyber's data directory only;
//! nothing is written into the folders themselves.
use crate::{tasks::matches_root, workspace};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Source folders; the first one is primary.
    pub folders: Vec<PathBuf>,
}

impl Project {
    pub fn primary(&self) -> Option<&Path> {
        self.folders.first().map(PathBuf::as_path)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    projects: Vec<Project>,
}

static PROJECTS: RwLock<Option<Vec<Project>>> = RwLock::new(None);
static REVISION: AtomicU64 = AtomicU64::new(1);

fn file() -> PathBuf {
    workspace::data_dir().join("projects.json")
}

/// Every project, read from disk once and then kept in memory.
pub fn all() -> Vec<Project> {
    if let Some(list) = PROJECTS.read().unwrap().as_ref() {
        return list.clone();
    }
    let list = fs::read(file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
        .map(|s| s.projects)
        .unwrap_or_default();
    *PROJECTS.write().unwrap() = Some(list.clone());
    list
}

/// Changes whenever the project list is saved, so watchers can rescan.
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

/// Replaces the project list on disk (atomically) and in memory.
pub fn save(list: Vec<Project>) -> Result<()> {
    let path = file();
    let dir = path.parent().context("No data directory")?;
    fs::create_dir_all(dir)?;
    let temp = dir.join(format!("projects.json.{}.tmp", std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec_pretty(&Stored {
            projects: list.clone(),
        })?,
    )?;
    fs::rename(&temp, &path)?;
    *PROJECTS.write().unwrap() = Some(list);
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// A folder path with the platform's own separators, so the same folder is
/// always stored the same way.
fn normalize(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().replace('/', "\\"))
    } else {
        path.to_path_buf()
    }
}

/// Adds or replaces one project.
pub fn upsert(mut project: Project) -> Result<()> {
    project.folders = project.folders.iter().map(|f| normalize(f)).collect();
    let mut list = all();
    match list.iter_mut().find(|p| p.id == project.id) {
        Some(old) => *old = project,
        None => list.push(project),
    }
    save(list)
}

pub fn remove(id: &str) -> Result<()> {
    let mut list = all();
    list.retain(|p| p.id != id);
    save(list)
}

pub fn new_id() -> String {
    format!(
        "{:x}{:04x}",
        chrono::Utc::now().timestamp_millis(),
        std::process::id() & 0xffff
    )
}

/// The project a path belongs to: the one whose source folder holds it most
/// closely, so a nested repository wins over the playground around it only
/// when it is a project of its own.
pub fn for_path(path: &Path) -> Option<Project> {
    all()
        .into_iter()
        .filter_map(|p| {
            let depth = p
                .folders
                .iter()
                .filter(|f| matches_root(path, f))
                .map(|f| f.components().count())
                .max()?;
            Some((depth, p))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, p)| p)
}

/// Source folders of `root`'s project that sit strictly inside `root`. The
/// file tree shows them even when `root`'s `.gitignore` hides them, and
/// snapshots of `root` include their files.
pub fn nested_folders(root: &Path) -> Vec<PathBuf> {
    let Some(project) = for_path(root) else {
        return vec![];
    };
    let mut folders: Vec<PathBuf> = project
        .folders
        .into_iter()
        .filter(|f| matches_root(f, root) && !matches_root(root, f) && f.is_dir())
        .collect();
    folders.sort();
    folders.dedup();
    folders
}

/// Whether `path` holds its own Git repository or linked worktree.
pub fn is_repository(path: &Path) -> bool {
    path.join(".git").exists()
}

/// A linked worktree's `.git` is a file pointing into the main repository's
/// `.git/worktrees/`.
pub fn is_linked_worktree(path: &Path) -> bool {
    let git = path.join(".git");
    git.is_file()
        && fs::read_to_string(&git).is_ok_and(|text| {
            text.trim_start()
                .strip_prefix("gitdir:")
                .is_some_and(|dir| dir.replace('\\', "/").contains("/worktrees/"))
        })
}

/// Repositories below `root`, at most two folders deep, without descending
/// into a repository once found. Linked worktrees and build folders are left
/// out: they belong to their own repository.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| {
                !p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| matches!(n, ".git" | "target" | "node_modules"))
            })
            .collect();
        dirs.sort();
        for path in dirs {
            if is_repository(&path) {
                if !is_linked_worktree(&path) {
                    out.push(path);
                }
            } else if depth < 2 {
                walk(&path, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 1, &mut out);
    out
}

/// Projects defined in the Codex desktop app, read-only. The format is the
/// app's internal state, so anything unexpected yields an empty list.
pub fn codex_projects() -> Vec<Project> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let Ok(bytes) = fs::read(home.join(".codex").join(".codex-global-state.json")) else {
        return vec![];
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return vec![];
    };
    let Some(projects) = value["local-projects"].as_object() else {
        return vec![];
    };
    let mut list: Vec<Project> = projects
        .values()
        .filter_map(|p| {
            let folders: Vec<PathBuf> = p["rootPaths"]
                .as_array()?
                .iter()
                .filter_map(|f| f.as_str().map(PathBuf::from))
                .filter(|f| f.is_dir())
                .collect();
            let name = p["name"].as_str().map(str::to_owned).or_else(|| {
                folders
                    .first()
                    .and_then(|f| f.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
            })?;
            (!folders.is_empty()).then(|| Project {
                id: new_id(),
                name,
                folders,
            })
        })
        .collect();
    list.sort_by_key(|p| p.name.to_lowercase());
    list
}

/// Display name of a folder: its last path component.
pub fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_finds_nested_repositories_but_not_worktrees() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        for repo in ["a", "group/b"] {
            fs::create_dir_all(root.join(repo).join(".git"))?;
        }
        fs::create_dir_all(root.join("a/inner/.git"))?;
        fs::create_dir_all(root.join(".worktree/a-feature"))?;
        fs::write(
            root.join(".worktree/a-feature/.git"),
            format!("gitdir: {}/a/.git/worktrees/a-feature\n", root.display()),
        )?;
        fs::create_dir_all(root.join("target/x/.git"))?;
        let found = discover(root);
        assert_eq!(found, vec![root.join("a"), root.join("group").join("b")]);
        assert!(is_linked_worktree(&root.join(".worktree/a-feature")));
        Ok(())
    }

    #[test]
    fn the_first_folder_is_primary() {
        let project = Project {
            id: "p".into(),
            name: "play".into(),
            folders: vec![PathBuf::from("/w/play"), PathBuf::from("/w/play/api")],
        };
        assert_eq!(project.primary(), Some(Path::new("/w/play")));
        assert!(matches_root(Path::new("/w/play/api/src"), &project.folders[1]));
        assert!(!matches_root(Path::new("/w/player"), &project.folders[0]));
    }
}
