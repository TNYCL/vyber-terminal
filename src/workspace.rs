use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn command(program: &str) -> Command {
    let cmd = Command::new(program);
    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut cmd = cmd;
        cmd.creation_flags(0x08000000);
        cmd
    };
    cmd
}

pub fn git(root: &Path, args: &[&str]) -> Result<Output> {
    command("git")
        .args(["--no-pager", "-c", "core.quotepath=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .context("Git could not be started")
}

pub fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let out = git(root, args)?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
}

pub fn repository_root(path: &Path) -> Option<PathBuf> {
    git_text(path, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from)
}

#[cfg(test)]
thread_local! {
    static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "Used by macOS interaction tests.")
)]
pub struct TestDataDir(Option<PathBuf>);
#[cfg(test)]
impl TestDataDir {
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "Used by macOS interaction tests.")
    )]
    pub fn set(path: PathBuf) -> Self {
        Self(TEST_DATA_DIR.with(|value| value.replace(Some(path))))
    }
}
#[cfg(test)]
impl Drop for TestDataDir {
    fn drop(&mut self) {
        TEST_DATA_DIR.with(|value| value.replace(self.0.take()));
    }
}
pub fn data_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_DATA_DIR.with(|value| value.borrow().clone()) {
        return path;
    }
    if let Some(dir) = std::env::var_os("VYBER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Vyber")
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub relative: String,
    pub depth: usize,
    pub directory: bool,
    /// A nested Git repository (a folder with its own `.git`), offered as a tree root.
    #[serde(default)]
    pub repository: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileListing {
    pub entries: Vec<FileEntry>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NameSearchResult {
    pub entries: Vec<FileEntry>,
    pub errors: Vec<String>,
    pub truncated: bool,
}

pub const NAME_SEARCH_LIMIT: usize = 300;

/// A folder's path within a workspace, accepting equivalent canonical and
/// ordinary Windows paths while preserving lexical directory-link paths.
pub fn relative_folder(root: &Path, folder: &Path) -> Option<PathBuf> {
    if let Ok(relative) = folder.strip_prefix(root)
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Some(relative.to_path_buf());
    }
    let root = fs::canonicalize(root).ok()?;
    let folder = fs::canonicalize(folder).ok()?;
    folder.strip_prefix(root).ok().map(Path::to_path_buf)
}

#[cfg(test)]
pub fn scan_files(root: &Path) -> Vec<FileEntry> {
    scan_files_with(root, &[]).entries
}

/// Lists every immediate child of the root and requested expanded folders.
/// Ancestors of requested folders are listed too, so a restored nested root
/// can be reached without recursively indexing the whole workspace.
pub fn scan_files_with(root: &Path, include: &[PathBuf]) -> FileListing {
    let mut result = FileListing::default();
    let mut folders = BTreeSet::from([root.to_path_buf()]);
    for folder in include {
        let Ok(relative) = folder.strip_prefix(root) else {
            continue;
        };
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            continue;
        }
        let mut path = root.to_path_buf();
        for part in relative.components() {
            path.push(part);
            folders.insert(path.clone());
        }
    }
    for folder in folders {
        result.entries.extend(
            read_children(root, &folder, &mut result.errors, None)
                .into_iter()
                .map(|(entry, _)| entry),
        );
    }
    result.entries.sort_by(tree_order);
    result.entries.dedup_by(|a, b| a.path == b.path);
    result
}

/// Returns visible filesystem entries, retaining links even when their target
/// is unavailable. The link flag keeps background searches from following
/// directory links into cycles; explicit tree expansion can still open them.
fn read_children(
    root: &Path,
    folder: &Path,
    errors: &mut Vec<String>,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Vec<(FileEntry, bool)> {
    let mut children = Vec::new();
    let directory = match fs::read_dir(folder) {
        Ok(directory) => directory,
        Err(error) => {
            errors.push(format!("Cannot read {}: {error}", folder.display()));
            return children;
        }
    };
    for entry in directory {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("Cannot list {}: {error}", folder.display()));
                continue;
            }
        };
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                errors.push(format!("Cannot inspect {}: {error}", path.display()));
                continue;
            }
        };
        let linked = kind.is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            // Junctions and other directory reparse points can lead back into
            // the workspace even when their FileType is reported as a folder.
            linked
                || fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| metadata.file_attributes() & 0x400 != 0)
        };
        let directory = kind.is_dir() || (linked && path.is_dir());
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let depth = relative.matches('/').count();
        children.push((
            FileEntry {
                repository: directory && path.join(".git").exists(),
                path,
                depth,
                relative,
                directory,
            },
            linked,
        ));
    }
    children.sort_by(|a, b| tree_order(&a.0, &b.0));
    children
}

/// Searches all filesystem names without Git ignore rules or depth limits.
/// Work is cancelled between entries and directories, and stops after the
/// first extra match proves that the displayed result limit was reached.
pub fn search_files(
    root: &Path,
    query: &str,
    cancelled: &std::sync::atomic::AtomicBool,
) -> NameSearchResult {
    let mut result = NameSearchResult::default();
    if query.trim().is_empty() {
        return result;
    }
    let mut folders = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(folder) = folders.pop_front() {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        for (entry, linked) in read_children(root, &folder, &mut result.errors, Some(cancelled)) {
            if cancelled.load(Ordering::Relaxed) {
                return result;
            }
            if entry.directory && !linked {
                folders.push_back(entry.path.clone());
            }
            if fuzzy_score(query, &entry.relative).is_some() {
                if result.entries.len() == NAME_SEARCH_LIMIT {
                    result.truncated = true;
                    return result;
                }
                result.entries.push(entry);
            }
        }
    }
    result
}

/// Depth-first tree order: folders before files at every level, then natural
/// name order. Comparing whole path strings would put `foo-bar` and `foo.rs`
/// between `foo/` and its children because `-` and `.` sort before `/`.
pub fn tree_order(a: &FileEntry, b: &FileEntry) -> std::cmp::Ordering {
    path_order(&a.relative, a.directory, &b.relative, b.directory)
}

/// [`tree_order`] for bare relative paths.
pub fn path_order(a: &str, a_directory: bool, b: &str, b_directory: bool) -> std::cmp::Ordering {
    let mut left = a.split('/');
    let mut right = b.split('/');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x == y => continue,
            (Some(x), Some(y)) => {
                // A segment followed by more segments is a folder on this path.
                let x_dir = left.clone().next().is_some() || a_directory;
                let y_dir = right.clone().next().is_some() || b_directory;
                return y_dir.cmp(&x_dir).then_with(|| natural_cmp(x, y));
            }
        }
    }
}

/// Case-insensitive comparison that orders digit runs by value (`file2` < `file10`).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut run = String::new();
        while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
            run.push(c);
            it.next();
        }
        run
    }
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (x, y) = (digits(&mut a), digits(&mut b));
                let (tx, ty) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if order.is_ne() {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase()).then(x.cmp(&y));
                if order.is_ne() {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub status: String,
    /// Added and removed lines; zero for binary files and Git status entries.
    #[serde(default)]
    pub additions: usize,
    #[serde(default)]
    pub deletions: usize,
}
impl Change {
    /// One-letter summary of a Git status (`M`, `A`, `D`, `U` untracked, `!` conflict).
    pub fn letter(&self) -> char {
        let status = self.status.trim();
        let bytes = self.status.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        match (x, y) {
            (b'?', _) => 'U',
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => '!',
            (b'D', _) | (_, b'D') => 'D',
            (b'A', _) => 'A',
            (b'R', _) => 'R',
            _ if status.len() == 1 => status.chars().next().unwrap_or('M'),
            _ => 'M',
        }
    }
}

pub fn status(root: &Path) -> Result<Vec<Change>> {
    let out = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ],
    )?;
    if !out.status.success() {
        bail!("Not a Git workspace");
    }
    let mut parts = out.stdout.split(|b| *b == 0);
    let mut result = Vec::new();
    let prefix = git_text(root, &["rev-parse", "--show-prefix"])?;
    while let Some(part) = parts.next() {
        if part.len() < 4 {
            continue;
        }
        let status = String::from_utf8_lossy(&part[..2]).into_owned();
        let path = String::from_utf8_lossy(&part[3..]);
        if let Some(path) = path.strip_prefix(&prefix) {
            result.push(Change {
                path: path.into(),
                status: status.clone(),
                ..Default::default()
            });
        }
        if status.contains('R') || status.contains('C') {
            let _ = parts.next();
        }
    }
    Ok(result)
}

/// Git status of `root` and of each repository folder in `nested`, with
/// nested paths made relative to `root`.
pub fn status_with(root: &Path, nested: &[PathBuf]) -> Result<Vec<Change>> {
    let mut changes = status(root).unwrap_or_default();
    let mut found = !changes.is_empty() || git_text(root, &["rev-parse", "--git-dir"]).is_ok();
    for folder in nested {
        let Ok(relative) = folder.strip_prefix(root) else {
            continue;
        };
        let prefix = relative.to_string_lossy().replace('\\', "/");
        if let Ok(inner) = status(folder) {
            found = true;
            changes.extend(inner.into_iter().map(|mut c| {
                c.path = format!("{prefix}/{}", c.path);
                c
            }));
        }
    }
    if !found {
        bail!("Not a Git workspace");
    }
    Ok(changes)
}

#[derive(Clone, Debug, Default)]
pub struct DiffLine {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    pub kind: char,
    /// Syntax colors as byte ranges of `text`, sorted and non-overlapping.
    pub spans: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)>,
}

pub fn diff_lines_context(old: &[u8], new: &[u8], context: usize) -> Vec<DiffLine> {
    if old.contains(&0) || new.contains(&0) {
        return vec![DiffLine {
            text: "Binary file changed".into(),
            kind: ' ',
            ..Default::default()
        }];
    }
    // Compare lines regardless of CRLF/LF so a checkout with autocrlf does not
    // show every line as changed; line numbers are the same either way.
    let old = String::from_utf8_lossy(old).replace("\r\n", "\n");
    let new = String::from_utf8_lossy(new).replace("\r\n", "\n");
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_secs(2))
        .diff_lines(&old, &new);
    let mut result = Vec::new();
    for (group_ix, group) in diff.grouped_ops(context).iter().enumerate() {
        if group_ix > 0 {
            result.push(DiffLine {
                text: "··· unchanged lines ···".into(),
                kind: '@',
                ..Default::default()
            });
        }
        for op in group {
            for change in diff.iter_changes(op) {
                result.push(DiffLine {
                    old: change.old_index().map(|i| i + 1),
                    new: change.new_index().map(|i| i + 1),
                    text: change.value().trim_end_matches(['\r', '\n']).to_owned(),
                    kind: match change.tag() {
                        similar::ChangeTag::Delete => '-',
                        similar::ChangeTag::Insert => '+',
                        _ => ' ',
                    },
                    spans: Vec::new(),
                });
            }
        }
    }
    result
}

/// Parses `git diff --numstat -z --no-renames` into path → (added, removed).
/// Binary files report `-` and count as zero.
pub fn parse_numstat(output: &[u8]) -> std::collections::HashMap<String, (usize, usize)> {
    output
        .split(|b| *b == 0)
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let mut parts = text.splitn(3, '\t');
            let added = parts.next()?.trim().parse().unwrap_or(0);
            let removed = parts.next()?.parse().unwrap_or(0);
            let path = parts.next().filter(|p| !p.is_empty())?;
            Some((path.to_owned(), (added, removed)))
        })
        .collect()
}

/// Line count as Git reports it for a new file: a last line without a
/// newline still counts. Binary content has no lines.
pub fn count_lines(bytes: &[u8]) -> usize {
    if bytes.contains(&0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    newlines + usize::from(bytes.last().is_some_and(|b| *b != b'\n'))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub tree: String,
    pub root: PathBuf,
    pub label: String,
    pub created: String,
    pub objects: PathBuf,
    pub alternates: PathBuf,
    /// Vyber's own repository, for a snapshot of a folder that has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_dir: Option<PathBuf>,
}

impl Checkpoint {
    /// Snapshots the working tree of `root`'s repository, including the
    /// project's repositories inside it. A folder that is no repository
    /// itself, such as one that holds several projects, gets a snapshot of
    /// the repositories below it instead.
    pub fn capture(root: &Path, label: &str) -> Result<Self> {
        let Some(repository) = repository_root(root) else {
            return Self::capture_group(root, label, None);
        };
        let nested = crate::project::nested_folders(&repository);
        Self::capture_with(&repository, label, &nested)
    }

    /// A later snapshot of the same folder. For a folder of repositories only
    /// those with a path in `changed` are read again; the others keep their
    /// files from `self`.
    pub fn refresh(&self, label: &str, changed: &[PathBuf]) -> Result<Self> {
        if self.git_dir.is_some() {
            Self::capture_group(&self.root, label, Some((self, changed)))
        } else {
            Self::capture(&self.root, label)
        }
    }

    /// [`Checkpoint::capture`] of the repository at `root` with the
    /// repositories in `nested` folded in.
    pub fn capture_with(root: &Path, label: &str, nested: &[PathBuf]) -> Result<Self> {
        let root = root.to_owned();
        let id = next_id();
        let store = data_dir().join("checkpoints");
        fs::create_dir_all(&store)?;
        let index = store.join(format!("{id}.index"));
        let objects = store.join(store_key(&root)).join("objects");
        fs::create_dir_all(&objects)?;
        let alternates = PathBuf::from(git_text(
            &root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
        )?);
        let real_index = PathBuf::from(git_text(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )?);
        if real_index.exists() {
            fs::copy(&real_index, &index)?;
        }
        let git = SnapshotGit {
            root: &root,
            index: &index,
            objects: &objects,
            alternates: &alternates,
            git_dir: None,
        };
        let result = (|| {
            if !index.exists() {
                git.run(&["read-tree", "--empty"])?;
            }
            git.run(&["add", "-A", "--", "."])?;
            // Store exact working-tree bytes, including CRLF/BOM. Clean filters must not
            // turn a task snapshot into a different file from the one on disk.
            let listed = git.command().args(["ls-files", "--stage", "-z"]).output()?;
            if !listed.status.success() {
                bail!("Cannot enumerate snapshot index");
            }
            let mut entries = Vec::new();
            // Repositories inside this one: project source folders (usually
            // ignored by it) and embedded repositories Git lists as gitlinks.
            // Their files join the snapshot under their folder.
            let mut nested: Vec<String> = nested
                .iter()
                .filter(|f| crate::project::is_repository(f))
                .filter_map(|f| f.strip_prefix(&root).ok())
                .map(|f| f.to_string_lossy().replace('\\', "/"))
                .collect();
            for entry in listed.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                let tab = entry
                    .iter()
                    .position(|b| *b == b'\t')
                    .context("Invalid index entry")?;
                let mode = std::str::from_utf8(&entry[..6])?.to_owned();
                let path = std::str::from_utf8(&entry[tab + 1..])?.to_owned();
                if mode == "160000" {
                    if embedded(&root.join(&path)) {
                        nested.push(path);
                    }
                    continue;
                }
                if mode != "100644" && mode != "100755" {
                    bail!("Snapshot contains a symlink; manual Git review remains available");
                }
                entries.push((mode, path));
            }
            nested.sort();
            nested.dedup();
            if !nested.is_empty() {
                entries
                    .retain(|(_, path)| !nested.iter().any(|n| path.starts_with(&format!("{n}/"))));
                for folder in &nested {
                    entries.extend(nested_entries(&root, folder)?.0);
                }
                // Start the snapshot index over so no gitlink is left in it.
                let _ = fs::remove_file(&index);
                git.run(&["read-tree", "--empty"])?;
            }
            Self {
                id: id.clone(),
                tree: git.write(&entries, Vec::new())?,
                root: root.clone(),
                objects: objects.clone(),
                alternates: alternates.clone(),
                git_dir: None,
                label: label.into(),
                created: chrono::Local::now().format("%H:%M:%S").to_string(),
            }
            .saved()
        })();
        let _ = fs::remove_file(&index);
        result
    }

    /// Snapshot of a folder that is no repository: the repositories below it
    /// (see [`crate::project::discover`]) side by side under their folders,
    /// kept in a bare repository of Vyber's own. With `previous`, a
    /// repository without a path in its list keeps its files from that
    /// snapshot instead of being read again.
    fn capture_group(
        root: &Path,
        label: &str,
        previous: Option<(&Self, &[PathBuf])>,
    ) -> Result<Self> {
        let repositories = crate::project::discover(root);
        if repositories.is_empty() {
            bail!("Checkpoints require a Git repository");
        }
        let root = root.to_owned();
        let id = next_id();
        let store = data_dir().join("checkpoints");
        let git_dir = store.join(store_key(&root));
        let objects = git_dir.join("objects");
        fs::create_dir_all(&objects)?;
        if !git_dir.join("HEAD").exists() {
            git_text(&git_dir, &["init", "-q", "--bare"])?;
        }
        let index = store.join(format!("{id}.index"));
        let git = SnapshotGit {
            root: &root,
            index: &index,
            objects: &objects,
            alternates: &objects,
            git_dir: Some(&git_dir),
        };
        let result = (|| {
            let mut entries = Vec::new();
            // Index lines, as `ls-tree` prints them, of the repositories kept.
            let mut kept = Vec::new();
            for repository in &repositories {
                let folder = repository
                    .strip_prefix(&root)
                    .unwrap_or(repository)
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some((before, changed)) = previous
                    && !changed
                        .iter()
                        .any(|p| crate::tasks::matches_root(p, repository))
                {
                    let tree = before.tree.as_str();
                    let listed =
                        before.git(&["ls-tree", "-r", "-z", "--full-tree", tree, "--", &folder])?;
                    if listed.status.success() && !listed.stdout.is_empty() {
                        kept.extend(listed.stdout);
                        continue;
                    }
                }
                entries.extend(repository_entries(&root, &folder)?);
            }
            let count = entries.len() + kept.iter().filter(|b| **b == 0).count();
            if count > GROUP_LIMIT {
                bail!(
                    "{count} files in the repositories below {}; start the agent in one of them",
                    root.display()
                );
            }
            git.run(&["read-tree", "--empty"])?;
            Self {
                id: id.clone(),
                tree: git.write(&entries, kept)?,
                root: root.clone(),
                objects: objects.clone(),
                alternates: objects.clone(),
                git_dir: Some(git_dir.clone()),
                label: label.into(),
                created: chrono::Local::now().format("%H:%M:%S").to_string(),
            }
            .saved()
        })();
        let _ = fs::remove_file(&index);
        result
    }

    /// Records the snapshot next to its objects, for Review to find again.
    fn saved(self) -> Result<Self> {
        fs::write(
            data_dir()
                .join("checkpoints")
                .join(format!("{}.json", self.id)),
            serde_json::to_vec_pretty(&self)?,
        )?;
        Ok(self)
    }
    fn git(&self, args: &[&str]) -> Result<Output> {
        let mut command = command("git");
        command
            .args(["--no-pager", "-c", "core.quotepath=false"])
            .args(args)
            .current_dir(&self.root);
        for (key, value) in self.object_env() {
            command.env(key, value);
        }
        Ok(command.output()?)
    }
    pub fn changes_to(&self, after: &Self) -> Result<Vec<Change>> {
        let out = self.git(&[
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            &self.tree,
            &after.tree,
        ])?;
        if !out.status.success() {
            bail!("Cannot compare checkpoints");
        }
        let parts: Vec<_> = out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .collect();
        let stats = self
            .git(&[
                "diff",
                "--numstat",
                "--no-renames",
                "-z",
                &self.tree,
                &after.tree,
            ])
            .map(|out| parse_numstat(&out.stdout))
            .unwrap_or_default();
        Ok(parts
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                let path: String = String::from_utf8_lossy(p[1]).into();
                let (additions, deletions) = stats.get(&path).copied().unwrap_or_default();
                Change {
                    status: String::from_utf8_lossy(p[0]).into(),
                    path,
                    additions,
                    deletions,
                }
            })
            .collect())
    }
    /// Environment that lets plain `git` commands read this snapshot's objects.
    pub fn object_env(&self) -> Vec<(&'static str, PathBuf)> {
        let mut env = vec![
            ("GIT_OBJECT_DIRECTORY", self.objects.clone()),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", self.alternates.clone()),
        ];
        if let Some(dir) = &self.git_dir {
            env.push(("GIT_DIR", dir.clone()));
        }
        env
    }
    pub fn checked_content(&self, path: &str) -> Result<Option<Vec<u8>>> {
        safe_path(&self.root, path)?;
        let list = self.git(&["ls-tree", "-z", &self.tree, "--", path])?;
        if !list.status.success() {
            bail!("Snapshot missing: its tree is unavailable");
        }
        if list.stdout.is_empty() {
            return Ok(None);
        }
        if !list.stdout.starts_with(b"100644 ") && !list.stdout.starts_with(b"100755 ") {
            bail!("Only regular files can be restored");
        }
        let out = self.git(&["show", &format!("{}:{path}", self.tree)])?;
        if !out.status.success() {
            bail!("Snapshot missing: file content is unavailable");
        }
        Ok(Some(out.stdout))
    }
    pub fn restore_task(&self, after: &Self) -> Result<()> {
        let changes = self.changes_to(after)?;
        // Validate every affected file before writing any file.
        for change in &changes {
            let path = safe_path(&self.root, &change.path)?;
            let expected = after.checked_content(&change.path)?;
            self.checked_content(&change.path)?;
            if read_optional(&path)? != expected {
                bail!(
                    "{} changed after this task; nothing was restored",
                    change.path
                );
            }
        }
        for change in &changes {
            self.restore_file(after, &change.path)?;
        }
        Ok(())
    }
    pub fn restore_file(&self, after: &Self, relative: &str) -> Result<()> {
        let path = safe_path(&self.root, relative)?;
        let expected = after.checked_content(relative)?;
        let actual = read_optional(&path)?;
        if actual != expected {
            bail!("File changed after this task. Review the current version before restoring.");
        }
        let original = self.checked_content(relative)?;
        let recovery = data_dir().join("recovery").join(format!(
            "{}-{}",
            chrono::Utc::now().timestamp_millis(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&recovery)?;
        fs::write(recovery.join("path.txt"), path.to_string_lossy().as_bytes())?;
        if let Some(bytes) = &actual {
            fs::write(recovery.join("content"), bytes)?;
        }
        match original {
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, bytes)?;
            }
            None => {
                if actual.is_some() {
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }
}

/// A snapshot of a folder of repositories stops at this many files.
const GROUP_LIMIT: usize = 20_000;

fn next_id() -> String {
    format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Folder name of a snapshotted folder's objects in the checkpoint store.
fn store_key(root: &Path) -> String {
    blake3::hash(root.to_string_lossy().as_bytes())
        .to_hex()
        .to_string()
}

/// Whether a snapshot folds in the repository at `path`. Linked worktrees,
/// such as those agents check out under `.claude/worktrees`, are other
/// checkouts of a repository rather than part of the folder they sit in.
fn embedded(path: &Path) -> bool {
    crate::project::is_repository(path) && !crate::project::is_linked_worktree(path)
}

/// Git on a snapshot's temporary index and object store.
struct SnapshotGit<'a> {
    root: &'a Path,
    index: &'a Path,
    objects: &'a Path,
    alternates: &'a Path,
    git_dir: Option<&'a Path>,
}

impl SnapshotGit<'_> {
    fn command(&self) -> Command {
        let mut git = command("git");
        git.current_dir(self.root)
            .env("GIT_INDEX_FILE", self.index)
            .env("GIT_OBJECT_DIRECTORY", self.objects)
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", self.alternates);
        if let Some(dir) = self.git_dir {
            git.env("GIT_DIR", dir);
        }
        git
    }
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = self.command().args(args).output()?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }
    fn run_input(&self, args: &[&str], input: Vec<u8>) -> Result<Vec<u8>> {
        use std::io::Write;
        let mut child = self
            .command()
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().context("Snapshot input pipe")?;
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let out = child.wait_with_output()?;
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("Snapshot writer failed"))??;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(out.stdout)
    }
    /// Stores the exact bytes of `entries` (mode, path), adds them and the
    /// ready index lines in `updates` to the index, and writes its tree.
    fn write(&self, entries: &[(String, String)], mut updates: Vec<u8>) -> Result<String> {
        if !entries.is_empty() {
            let mut names = Vec::new();
            for (_, path) in entries {
                names.extend(serde_json::to_string(path)?.as_bytes());
                names.push(b'\n');
            }
            let hashes = self.run_input(
                &["hash-object", "-w", "--no-filters", "--stdin-paths"],
                names,
            )?;
            let hashes = std::str::from_utf8(&hashes)?.lines().collect::<Vec<_>>();
            if hashes.len() != entries.len() {
                bail!("Incomplete raw snapshot");
            }
            for ((mode, path), hash) in entries.iter().zip(hashes) {
                updates.extend(format!("{mode} {hash}\t{path}\0").as_bytes());
            }
        }
        if !updates.is_empty() {
            self.run_input(&["update-index", "-z", "--index-info"], updates)?;
        }
        self.run(&["write-tree"])
    }
}

/// Snapshot entries of the repository at `root/folder` with the repositories
/// inside it folded in, one level deep, as a snapshot of that repository
/// holds them.
fn repository_entries(root: &Path, folder: &str) -> Result<Vec<(String, String)>> {
    let (mut entries, mut inner) = nested_entries(root, folder)?;
    inner.extend(
        crate::project::nested_folders(&root.join(folder))
            .iter()
            .filter(|f| crate::project::is_repository(f))
            .filter_map(|f| f.strip_prefix(root).ok())
            .map(|f| f.to_string_lossy().replace('\\', "/")),
    );
    inner.sort();
    inner.dedup();
    entries.retain(|(_, path)| !inner.iter().any(|n| path.starts_with(&format!("{n}/"))));
    for folder in &inner {
        entries.extend(nested_entries(root, folder)?.0);
    }
    Ok(entries)
}

/// Snapshot entries (mode, path under `root`) for the tracked and untracked,
/// not ignored, regular files of the repository at `root/folder`, and the
/// folders under `root` of the repositories embedded in it.
type NestedEntries = (Vec<(String, String)>, Vec<String>);
fn nested_entries(root: &Path, folder: &str) -> Result<NestedEntries> {
    let dir = root.join(folder);
    let is_file = |relative: &str| {
        fs::symlink_metadata(dir.join(relative)).is_ok_and(|m| m.file_type().is_file())
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut inner = Vec::new();
    let staged = git(&dir, &["ls-files", "--stage", "-z"])?;
    if !staged.status.success() {
        bail!("Cannot list files of {folder}");
    }
    for entry in staged.stdout.split(|b| *b == 0).filter(|p| p.len() > 7) {
        let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let mode = String::from_utf8_lossy(&entry[..6]).into_owned();
        let path = String::from_utf8_lossy(&entry[tab + 1..]).into_owned();
        if mode == "160000" && embedded(&dir.join(&path)) {
            inner.push(format!("{folder}/{path}"));
        }
        // Conflicted files list up to three stages; one entry is enough.
        if (mode == "100644" || mode == "100755") && is_file(&path) && seen.insert(path.clone()) {
            out.push((mode, format!("{folder}/{path}")));
        }
    }
    let others = git(&dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for path in others.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8_lossy(path).into_owned();
        // An untracked repository inside is listed as its folder.
        if let Some(repository) = path.strip_suffix('/') {
            if embedded(&dir.join(repository)) {
                inner.push(format!("{folder}/{repository}"));
            }
            continue;
        }
        if is_file(&path) && seen.insert(path.clone()) {
            out.push(("100644".into(), format!("{folder}/{path}")));
        }
    }
    inner.sort();
    inner.dedup();
    Ok((out, inner))
}

/// Keeps a copy of a file's current bytes under `recovery/` before Vyber
/// overwrites or deletes it.
pub fn keep_recovery(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    let recovery = data_dir().join("recovery").join(format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&recovery)?;
    fs::write(recovery.join("path.txt"), path.to_string_lossy().as_bytes())?;
    if let Some(bytes) = bytes {
        fs::write(recovery.join("content"), bytes)?;
    }
    Ok(())
}

pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn save_checked(path: &Path, baseline: &[u8], bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if fs::read(path)? != baseline {
        bail!("File changed on disk; compare or reload before saving.");
    }
    let parent = path.parent().context("File has no parent directory")?;
    let temp = parent.join(format!(
        ".vyber-save-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::set_permissions(&temp, fs::metadata(path)?.permissions())?;
        if fs::read(path)? != baseline {
            bail!("File changed while saving; your buffer is still intact.");
        }
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("Unsafe relative file path");
    }
    let mut path = root.to_owned();
    for part in relative.components() {
        if part.as_os_str() == ".git" {
            bail!("Git metadata cannot be restored");
        }
        path.push(part);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Symbolic links cannot be restored");
        }
    }
    Ok(path)
}

/// Scores a fuzzy match of `query` against a relative path and returns the
/// matched character indices within the file name. File-name matches,
/// contiguous runs and word starts score higher. `None` when nothing matches.
pub fn fuzzy_score(query: &str, relative: &str) -> Option<(i64, Vec<usize>)> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return None;
    }
    let lower = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<Vec<_>>();
    let greedy = |haystack: &[char]| -> Option<Vec<usize>> {
        let mut positions = Vec::with_capacity(query.len());
        let mut from = 0;
        for q in &query {
            let found = haystack[from..].iter().position(|c| c == q)? + from;
            positions.push(found);
            from = found + 1;
        }
        Some(positions)
    };
    let score = |positions: &[usize], haystack: &[char]| -> i64 {
        let mut score = 0;
        for (i, p) in positions.iter().enumerate() {
            if i > 0 && positions[i - 1] + 1 == *p {
                score += 8;
            }
            if *p == 0 || matches!(haystack[p - 1], '/' | '_' | '-' | '.' | ' ') {
                score += 6;
            }
        }
        score - positions.last().copied().unwrap_or(0) as i64 / 4
    };
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let name_chars = lower(name);
    if let Some(positions) = greedy(&name_chars) {
        let bonus = if name_chars.starts_with(&query) {
            40
        } else {
            20
        };
        return Some((score(&positions, &name_chars) + bonus, positions));
    }
    let path_chars = lower(relative);
    greedy(&path_chars).map(|positions| (score(&positions, &path_chars), Vec::new()))
}

pub fn expanded_parents(path: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let mut value = Path::new(path).parent();
    while let Some(p) = value {
        if !p.as_os_str().is_empty() {
            set.insert(p.to_string_lossy().replace('\\', "/"));
        }
        value = p.parent();
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(relative: &str, directory: bool) -> FileEntry {
        FileEntry {
            path: PathBuf::from(relative),
            relative: relative.into(),
            depth: relative.matches('/').count(),
            directory,
            repository: false,
        }
    }
    #[test]
    fn tree_order_keeps_children_under_their_folder() {
        let mut entries = vec![
            entry("foo.rs", false),
            entry("foo-bar", true),
            entry("foo/b.rs", false),
            entry("foo", true),
            entry("README.md", false),
            entry("foo/a", true),
            entry("foo/a/z.txt", false),
            entry("file10.txt", false),
            entry("file2.txt", false),
            entry(".gitignore", false),
        ];
        entries.sort_by(tree_order);
        let order = entries
            .iter()
            .map(|e| e.relative.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                "foo",
                "foo/a",
                "foo/a/z.txt",
                "foo/b.rs",
                "foo-bar",
                ".gitignore",
                "file2.txt",
                "file10.txt",
                "foo.rs",
                "README.md"
            ]
        );
    }
    #[test]
    fn fuzzy_prefers_file_names() {
        let (name_score, positions) = fuzzy_score("brow", "src/browser.rs").unwrap();
        assert_eq!(positions, vec![0, 1, 2, 3]);
        let (path_score, positions) = fuzzy_score("srcb", "src/browser.rs").unwrap();
        assert!(positions.is_empty());
        assert!(name_score > path_score);
        assert!(fuzzy_score("xyz", "src/browser.rs").is_none());
    }
    #[test]
    fn status_letters() {
        let letter = |s: &str| {
            Change {
                status: s.into(),
                ..Default::default()
            }
            .letter()
        };
        assert_eq!(letter("??"), 'U');
        assert_eq!(letter(" M"), 'M');
        assert_eq!(letter("A "), 'A');
        assert_eq!(letter(" D"), 'D');
        assert_eq!(letter("UU"), '!');
        assert_eq!(letter("D"), 'D');
    }
    #[test]
    fn git_changes_work_from_a_subdirectory() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git_text(root, &["init"])?;
        git_text(root, &["config", "core.autocrlf", "false"])?;
        fs::create_dir(root.join("sub"))?;
        fs::write(root.join("sub/file.txt"), "staged\n")?;
        git_text(root, &["add", "sub/file.txt"])?;
        fs::write(root.join("sub/file.txt"), "working\n")?;
        let sub = root.join("sub");
        assert_eq!(status(&sub)?[0].path, "file.txt");
        Ok(())
    }
    fn init(path: &Path) -> Result<()> {
        fs::create_dir_all(path)?;
        git_text(path, &["init", "-q", "-b", "main"])?;
        git_text(path, &["config", "core.autocrlf", "false"])?;
        git_text(path, &["config", "user.email", "test@example.com"])?;
        git_text(path, &["config", "user.name", "Test"])?;
        Ok(())
    }
    /// A playground repository that ignores `api/` (its own repository) and
    /// embeds `lib/` without ignoring it.
    fn playground() -> Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        init(root)?;
        fs::write(root.join(".gitignore"), "/api/\n/.worktree/\n")?;
        fs::write(root.join("notes.md"), "notes\n")?;
        init(&root.join("api"))?;
        fs::write(root.join("api/.gitignore"), "target/\n")?;
        fs::create_dir_all(root.join("api/src"))?;
        fs::write(root.join("api/src/main.rs"), "fn main() {}\n")?;
        git_text(&root.join("api"), &["add", "."])?;
        git_text(&root.join("api"), &["commit", "-q", "-m", "api"])?;
        fs::create_dir_all(root.join("api/target"))?;
        fs::write(root.join("api/target/out.bin"), "built")?;
        init(&root.join("lib"))?;
        fs::write(root.join("lib/lib.rs"), "pub fn a() {}\n")?;
        git_text(&root.join("lib"), &["add", "."])?;
        git_text(&root.join("lib"), &["commit", "-q", "-m", "lib"])?;
        Ok(dir)
    }
    #[test]
    fn tree_lists_ignored_folders_and_only_expanded_contents() -> Result<()> {
        let dir = playground()?;
        let root = dir.path();
        let plain: Vec<_> = scan_files(root).into_iter().map(|e| e.relative).collect();
        assert!(plain.iter().any(|p| p == "api"));
        assert!(plain.iter().any(|p| p == ".git"));
        assert!(!plain.iter().any(|p| p == "api/src/main.rs"));
        let listing = scan_files_with(root, &[root.join("api/src"), root.join("api/target")]);
        assert!(listing.errors.is_empty());
        let entries = listing.entries;
        let paths: Vec<_> = entries.iter().map(|e| e.relative.as_str()).collect();
        assert!(paths.contains(&"api/src/main.rs"));
        assert!(paths.contains(&"api/target/out.bin"));
        let api = entries.iter().find(|e| e.relative == "api").unwrap();
        assert!(api.repository && api.directory && api.depth == 0);
        let main = entries
            .iter()
            .find(|e| e.relative == "api/src/main.rs")
            .unwrap();
        assert_eq!(main.depth, 2);
        // Tree order keeps the folder's children right after it.
        let at = paths.iter().position(|p| *p == "api").unwrap();
        assert!(paths[at + 1].starts_with("api/"));
        fs::write(root.join("api/src/main.rs"), "fn main() { run() }\n")?;
        let changes = status_with(root, &[root.join("api")])?;
        assert!(
            changes
                .iter()
                .any(|c| c.path == "api/src/main.rs" && c.letter() == 'M')
        );
        Ok(())
    }

    #[test]
    fn tree_root_siblings_do_not_depend_on_the_size_of_an_unopened_subtree() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        fs::create_dir(root.join("a-large"))?;
        for i in 0..1000 {
            fs::write(root.join(format!("a-large/file-{i}.txt")), "")?;
        }
        for folder in ["target", "node_modules", ".hidden", "z-last"] {
            fs::create_dir(root.join(folder))?;
        }
        fs::write(
            root.join(".gitignore"),
            "target/\nnode_modules/\n.hidden/\nz-last/\n",
        )?;
        let listing = scan_files_with(root, &[]);
        assert!(listing.errors.is_empty());
        let paths: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.relative.as_str())
            .collect();
        for folder in ["a-large", "target", "node_modules", ".hidden", "z-last"] {
            assert!(paths.contains(&folder));
        }
        assert_eq!(paths.len(), 6);
        assert!(!paths.iter().any(|path| path.contains('/')));
        Ok(())
    }

    #[test]
    fn tree_expansion_has_no_depth_limit_and_accepts_a_root_named_target() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("target");
        let deepest = (0..16).fold(root.clone(), |path, i| path.join(format!("d{i}")));
        fs::create_dir_all(&deepest)?;
        fs::write(deepest.join("leaf.txt"), "leaf")?;
        let listing = scan_files_with(&root, std::slice::from_ref(&deepest));
        assert!(listing.errors.is_empty());
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.path == deepest.join("leaf.txt"))
        );
        assert!(listing.entries.iter().any(|entry| entry.relative == "d0"));
        Ok(())
    }

    #[test]
    fn tree_reports_missing_requested_folders_without_losing_readable_siblings() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("present.txt"), "")?;
        let listing = scan_files_with(dir.path(), &[dir.path().join("missing")]);
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.relative == "present.txt")
        );
        assert_eq!(listing.errors.len(), 1);
        assert!(listing.errors[0].contains("missing"));
        let missing_root = scan_files_with(&dir.path().join("missing-root"), &[]);
        assert!(missing_root.entries.is_empty());
        assert_eq!(missing_root.errors.len(), 1);
        Ok(())
    }

    #[test]
    fn name_search_finds_unexpanded_ignored_files_and_reports_its_limit() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        fs::create_dir(root.join("node_modules"))?;
        fs::write(root.join(".gitignore"), "node_modules/\n")?;
        for i in 0..=NAME_SEARCH_LIMIT {
            fs::write(root.join(format!("node_modules/needle-{i}.txt")), "")?;
        }
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let result = search_files(root, "needle", &cancelled);
        assert_eq!(result.entries.len(), NAME_SEARCH_LIMIT);
        assert!(result.truncated);
        assert!(result.errors.is_empty());
        assert!(
            result
                .entries
                .iter()
                .all(|entry| entry.relative.starts_with("node_modules/"))
        );
        cancelled.store(true, Ordering::Relaxed);
        let result = search_files(root, "needle", &cancelled);
        assert!(result.entries.is_empty());
        assert!(!result.truncated);
        Ok(())
    }

    #[test]
    fn name_search_reaches_deep_folders_and_reports_read_errors() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let deepest = (0..16).fold(dir.path().to_path_buf(), |path, i| {
            path.join(format!("d{i}"))
        });
        fs::create_dir_all(&deepest)?;
        fs::write(deepest.join("needle.txt"), "")?;
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let result = search_files(dir.path(), "needle", &cancelled);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].path, deepest.join("needle.txt"));
        assert!(!result.truncated);
        let missing = search_files(&dir.path().join("missing"), "needle", &cancelled);
        assert!(missing.entries.is_empty());
        assert_eq!(missing.errors.len(), 1);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn tree_keeps_symlinks_and_search_does_not_follow_directory_cycles() -> Result<()> {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        fs::create_dir(root.join("source"))?;
        fs::write(root.join("source/needle.txt"), "")?;
        symlink(root.join("source"), root.join("alias"))?;
        symlink(root, root.join("source/back"))?;
        symlink(root.join("missing"), root.join("dangling"))?;
        let listing = scan_files_with(root, &[root.join("alias")]);
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.relative == "alias" && entry.directory)
        );
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.relative == "alias/needle.txt")
        );
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.relative == "dangling")
        );
        let result = search_files(root, "needle", &std::sync::atomic::AtomicBool::new(false));
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].relative, "source/needle.txt");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn tree_expands_windows_junctions_without_recursing_through_search_cycles() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = fs::canonicalize(dir.path())?;
        let source = root.join("source");
        let junction = root.join("alias");
        let back = source.join("back");
        fs::create_dir(&source)?;
        fs::write(source.join("needle.txt"), "")?;
        for (link, target) in [(&junction, &source), (&back, &root)] {
            assert!(link.starts_with(&root) && target.starts_with(&root));
            let output = command("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let listing = scan_files_with(&root, std::slice::from_ref(&junction));
        assert!(listing.errors.is_empty());
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.relative == "alias" && entry.directory)
        );
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.relative == "alias/needle.txt")
        );
        let result = search_files(&root, "needle", &std::sync::atomic::AtomicBool::new(false));
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].relative, "source/needle.txt");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn folder_scope_accepts_canonical_windows_paths() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let folder = dir.path().join("child");
        fs::create_dir(&folder)?;
        let canonical = fs::canonicalize(folder)?;
        assert_eq!(
            relative_folder(dir.path(), &canonical),
            Some(PathBuf::from("child"))
        );
        let outside = tempfile::tempdir()?;
        assert!(relative_folder(dir.path(), outside.path()).is_none());
        Ok(())
    }
    #[test]
    fn snapshots_fold_in_nested_repositories() -> Result<()> {
        let dir = playground()?;
        let root = dir.path();
        let nested = [root.join("api")];
        let before = Checkpoint::capture_with(root, "start", &nested)?;
        fs::write(root.join("api/src/main.rs"), "fn main() { run() }\n")?;
        fs::write(root.join("api/src/new.rs"), "new\n")?;
        fs::write(root.join("lib/lib.rs"), "pub fn b() {}\n")?;
        fs::write(root.join("api/target/out.bin"), "rebuilt")?;
        let after = Checkpoint::capture_with(root, "end", &nested)?;
        let mut paths: Vec<_> = before
            .changes_to(&after)?
            .into_iter()
            .map(|c| (c.path, c.status))
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                ("api/src/main.rs".to_string(), "M".to_string()),
                ("api/src/new.rs".to_string(), "A".to_string()),
                ("lib/lib.rs".to_string(), "M".to_string()),
            ]
        );
        before.restore_file(&after, "api/src/main.rs")?;
        assert_eq!(
            fs::read_to_string(root.join("api/src/main.rs"))?,
            "fn main() {}\n"
        );
        Ok(())
    }
    fn paths(before: &Checkpoint, after: &Checkpoint) -> Result<Vec<(String, String)>> {
        let mut paths: Vec<_> = before
            .changes_to(after)?
            .into_iter()
            .map(|c| (c.path, c.status))
            .collect();
        paths.sort();
        Ok(paths)
    }
    #[test]
    fn snapshots_leave_out_linked_worktrees() -> Result<()> {
        let dir = playground()?;
        let root = dir.path();
        git_text(root, &["add", ".gitignore", "notes.md"])?;
        git_text(root, &["commit", "-q", "-m", "start"])?;
        // An agent's worktree inside the repository, which does not ignore it.
        let worktree = ".claude/worktrees/agent";
        git_text(root, &["worktree", "add", "-q", worktree, "-b", "agent"])?;
        let before = Checkpoint::capture(root, "start")?;
        fs::write(root.join(".claude/worktrees/agent/notes.md"), "agent\n")?;
        fs::write(root.join(".claude/worktrees/agent/new.md"), "new\n")?;
        fs::write(root.join("notes.md"), "edited\n")?;
        let after = Checkpoint::capture(root, "end")?;
        assert_eq!(
            paths(&before, &after)?,
            vec![("notes.md".to_string(), "M".to_string())]
        );
        Ok(())
    }
    #[test]
    fn folders_of_repositories_get_a_snapshot() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        init(&root.join("a"))?;
        fs::write(root.join("a/a.txt"), "a\n")?;
        // An untracked repository inside `a`, as project source folders are.
        init(&root.join("a/inner"))?;
        fs::write(root.join("a/inner/inner.txt"), "inner\n")?;
        init(&root.join("group/b"))?;
        fs::write(root.join("group/b/b.txt"), "b\n")?;
        fs::write(root.join("loose.txt"), "not in a repository\n")?;
        let before = Checkpoint::capture(root, "start")?;
        assert!(before.git_dir.is_some());
        fs::write(root.join("a/a.txt"), "a2\n")?;
        fs::write(root.join("a/inner/inner.txt"), "inner2\n")?;
        fs::write(root.join("group/b/new.txt"), "new\n")?;
        fs::write(root.join("loose.txt"), "edited\n")?;
        let after = Checkpoint::capture(root, "end")?;
        assert_eq!(
            paths(&before, &after)?,
            vec![
                ("a/a.txt".to_string(), "M".to_string()),
                ("a/inner/inner.txt".to_string(), "M".to_string()),
                ("group/b/new.txt".to_string(), "A".to_string()),
            ]
        );
        // A refresh reads again only the repositories with a changed path.
        fs::write(root.join("group/b/b.txt"), "b2\n")?;
        let unchanged = after.refresh("live", &[root.join("a/a.txt")])?;
        assert!(paths(&after, &unchanged)?.is_empty());
        let refreshed = after.refresh("live", &[root.join("group/b/b.txt")])?;
        assert_eq!(
            paths(&after, &refreshed)?,
            vec![("group/b/b.txt".to_string(), "M".to_string())]
        );
        before.restore_file(&after, "a/a.txt")?;
        assert_eq!(fs::read_to_string(root.join("a/a.txt"))?, "a\n");
        Ok(())
    }
    #[test]
    fn numstat_and_line_counts() {
        let stats = parse_numstat(b"3\t1\tsrc/a.rs\0-\t-\timage.png\0");
        assert_eq!(stats["src/a.rs"], (3, 1));
        assert_eq!(stats["image.png"], (0, 0));
        assert_eq!(count_lines(b"a\nb"), 2);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\0b\n"), 0);
    }
    #[test]
    fn checked_save_preserves_bytes_and_rejects_external_change() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("note.md");
        let first = b"\xef\xbb\xbfhello\r\n";
        let second = b"\xef\xbb\xbfchanged\r\n";
        fs::write(&path, first)?;
        save_checked(&path, first, second)?;
        assert_eq!(fs::read(&path)?, second);
        assert!(save_checked(&path, first, b"stale").is_err());
        assert_eq!(fs::read(&path)?, second);
        assert_eq!(fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }
    #[test]
    fn consecutive_tasks_preserve_raw_bytes_and_git_objects() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git_text(root, &["init"])?;
        git_text(root, &["config", "core.autocrlf", "true"])?;
        fs::write(root.join("a.txt"), b"\xef\xbb\xbfzero\r\n")?;
        git_text(root, &["add", "a.txt"])?;
        let objects = |p: &Path| -> BTreeSet<String> {
            ignore::WalkBuilder::new(p.join(".git/objects"))
                .hidden(false)
                .build()
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.path().display().to_string())
                .collect()
        };
        let initial_objects = objects(root);
        let index = fs::read(root.join(".git/index"))?;
        let first = Checkpoint::capture(root, "first start")?;
        fs::write(root.join("a.txt"), b"\xef\xbb\xbffirst\r\n")?;
        let second = Checkpoint::capture(root, "second start")?;
        fs::write(root.join("b.txt"), "second\n")?;
        let end = Checkpoint::capture(root, "second end")?;
        assert_eq!(
            second
                .changes_to(&end)?
                .iter()
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>(),
            vec!["b.txt"]
        );
        assert_eq!(objects(root), initial_objects);
        assert_eq!(fs::read(root.join(".git/index"))?, index);
        git_text(root, &["gc", "--prune=now"])?;
        second.restore_task(&end)?;
        assert!(!root.join("b.txt").exists());
        assert_eq!(fs::read(root.join("a.txt"))?, b"\xef\xbb\xbffirst\r\n");
        first.restore_file(&second, "a.txt")?;
        assert_eq!(fs::read(root.join("a.txt"))?, b"\xef\xbb\xbfzero\r\n");
        Ok(())
    }
    #[test]
    fn task_restore_preflights_every_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git_text(root, &["init"])?;
        fs::write(root.join("a"), "one")?;
        fs::write(root.join("b"), "one")?;
        let start = Checkpoint::capture(root, "before")?;
        fs::write(root.join("a"), "two")?;
        fs::write(root.join("b"), "two")?;
        let end = Checkpoint::capture(root, "after")?;
        fs::write(root.join("b"), "later")?;
        assert!(start.restore_task(&end).is_err());
        assert_eq!(fs::read_to_string(root.join("a"))?, "two");
        assert_eq!(fs::read_to_string(root.join("b"))?, "later");
        Ok(())
    }
    #[test]
    fn checkpoint_preserves_index_and_restores_only_its_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git_text(root, &["init"])?;
        fs::write(root.join("a.txt"), "staged\n")?;
        fs::write(root.join(".gitignore"), "target/\n")?;
        git_text(root, &["add", "a.txt"])?;
        let index = fs::read(root.join(".git/index"))?;
        let first = Checkpoint::capture(root, "Before")?;
        fs::write(root.join("a.txt"), "staged\nchanged\n")?;
        fs::write(root.join("new.txt"), "new\n")?;
        fs::create_dir(root.join("target"))?;
        fs::write(root.join("target/ignored"), "ignore")?;
        let second = Checkpoint::capture(root, "After")?;
        let changes = first.changes_to(&second)?;
        assert_eq!(changes.len(), 2);
        assert_eq!(index, fs::read(root.join(".git/index"))?);
        assert_eq!(git_text(root, &["stash", "list"])?, "");
        first.restore_file(&second, "a.txt")?;
        first.restore_file(&second, "new.txt")?;
        assert_eq!(fs::read_to_string(root.join("a.txt"))?, "staged\n");
        assert!(!root.join("new.txt").exists());
        assert!(root.join("target/ignored").exists());
        Ok(())
    }
    #[test]
    fn restore_rejects_later_edits_and_unsafe_paths() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git_text(root, &["init"])?;
        fs::write(root.join("a"), "one")?;
        let before = Checkpoint::capture(root, "before")?;
        fs::write(root.join("a"), "two")?;
        let after = Checkpoint::capture(root, "after")?;
        fs::write(root.join("a"), "three")?;
        assert!(before.restore_file(&after, "a").is_err());
        assert!(safe_path(root, "../outside").is_err());
        assert!(safe_path(root, ".git/config").is_err());
        assert_eq!(fs::read_to_string(root.join("a"))?, "three");
        Ok(())
    }
}
