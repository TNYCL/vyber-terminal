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
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
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

pub fn data_dir() -> PathBuf {
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

pub fn scan_files(root: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .filter_entry(|e| {
            !e.path().components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some(".git" | "target" | "node_modules")
                )
            })
        })
        .max_depth(Some(12))
        .build()
        .flatten()
    {
        if entries.len() >= 30_000 {
            break;
        }
        let path = entry.path();
        if path == root
            || path.components().any(|c| {
                c.as_os_str() == ".git"
                    || c.as_os_str() == "target"
                    || c.as_os_str() == "node_modules"
            })
        {
            continue;
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let depth = entry.depth().saturating_sub(1);
        entries.push(FileEntry {
            path: path.to_owned(),
            repository: kind.is_dir() && depth < 3 && path.join(".git").exists(),
            depth,
            relative,
            directory: kind.is_dir(),
        });
    }
    entries.sort_by(tree_order);
    entries
}

/// Depth-first tree order: folders before files at every level, then natural
/// name order. Comparing whole path strings would put `foo-bar` and `foo.rs`
/// between `foo/` and its children because `-` and `.` sort before `/`.
pub fn tree_order(a: &FileEntry, b: &FileEntry) -> std::cmp::Ordering {
    let mut left = a.relative.split('/');
    let mut right = b.relative.split('/');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x == y => continue,
            (Some(x), Some(y)) => {
                // A segment followed by more segments is a folder on this path.
                let x_dir = left.clone().next().is_some() || a.directory;
                let y_dir = right.clone().next().is_some() || b.directory;
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
            });
        }
        if status.contains('R') || status.contains('C') {
            let _ = parts.next();
        }
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffScope {
    All,
    Unstaged,
    Staged,
}
impl DiffScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All changes",
            Self::Unstaged => "Unstaged",
            Self::Staged => "Staged",
        }
    }
}

fn git_path(root: &Path, path: &str) -> Result<String> {
    let prefix = git_text(root, &["rev-parse", "--show-prefix"])?;
    Ok(format!("{prefix}{path}"))
}
pub fn before_content(root: &Path, path: &str, scope: DiffScope) -> Result<Option<Vec<u8>>> {
    let path = git_path(root, path)?;
    let repository = repository_root(root).context("Not a Git workspace")?;
    let root = repository.as_path();
    let spec = if scope == DiffScope::Unstaged {
        format!(":{path}")
    } else {
        format!("HEAD:{path}")
    };
    let out = git(root, &["show", &spec])?;
    if out.status.success() {
        Ok(Some(out.stdout))
    } else {
        let head = git(root, &["rev-parse", "--verify", "HEAD"])?;
        if scope != DiffScope::Unstaged && !head.status.success() {
            return Ok(None);
        }
        let list = if scope == DiffScope::Unstaged {
            git(root, &["ls-files", "--stage", "-z", "--", &path])?
        } else {
            git(root, &["ls-tree", "-z", "HEAD", "--", &path])?
        };
        if list.status.success() && list.stdout.is_empty() {
            Ok(None)
        } else {
            bail!(
                "Cannot read Git base: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        }
    }
}
pub fn after_content(root: &Path, path: &str, scope: DiffScope) -> Result<Option<Vec<u8>>> {
    if scope == DiffScope::Staged {
        before_content(root, path, DiffScope::Unstaged)
    } else {
        read_optional(&root.join(path))
    }
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    pub kind: char,
}

pub fn diff_lines_context(old: &[u8], new: &[u8], context: usize) -> Vec<DiffLine> {
    if old.contains(&0) || new.contains(&0) {
        return vec![DiffLine {
            old: None,
            new: None,
            text: "Binary file changed".into(),
            kind: ' ',
        }];
    }
    let old = String::from_utf8_lossy(old);
    let new = String::from_utf8_lossy(new);
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_secs(2))
        .diff_lines(&old, &new);
    let mut result = Vec::new();
    for (group_ix, group) in diff.grouped_ops(context).iter().enumerate() {
        if group_ix > 0 {
            result.push(DiffLine {
                old: None,
                new: None,
                text: "··· unchanged lines ···".into(),
                kind: '@',
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
                });
            }
        }
    }
    result
}

pub fn paired_lines(lines: &[DiffLine]) -> Vec<(Option<DiffLine>, Option<DiffLine>)> {
    let mut rows = vec![];
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            '-' => {
                let begin = i;
                while i < lines.len() && lines[i].kind == '-' {
                    i += 1;
                }
                let middle = i;
                while i < lines.len() && lines[i].kind == '+' {
                    i += 1;
                }
                for n in 0..(middle - begin).max(i - middle) {
                    rows.push((
                        lines.get(begin + n).filter(|_| begin + n < middle).cloned(),
                        lines.get(middle + n).filter(|_| middle + n < i).cloned(),
                    ));
                }
            }
            '+' => {
                rows.push((None, Some(lines[i].clone())));
                i += 1;
            }
            _ => {
                rows.push((Some(lines[i].clone()), Some(lines[i].clone())));
                i += 1;
            }
        }
    }
    rows
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
}

impl Checkpoint {
    pub fn capture(root: &Path, label: &str) -> Result<Self> {
        let root = repository_root(root).context("Checkpoints require a Git repository")?;
        let id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp_millis(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let store = data_dir().join("checkpoints");
        fs::create_dir_all(&store)?;
        let index = store.join(format!("{id}.index"));
        let repository_key = blake3::hash(root.to_string_lossy().as_bytes())
            .to_hex()
            .to_string();
        let objects = store.join(repository_key).join("objects");
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
        let run = |args: &[&str]| -> Result<String> {
            let output = command("git")
                .args(args)
                .current_dir(&root)
                .env("GIT_INDEX_FILE", &index)
                .env("GIT_OBJECT_DIRECTORY", &objects)
                .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &alternates)
                .output()?;
            if !output.status.success() {
                bail!("{}", String::from_utf8_lossy(&output.stderr));
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().into())
        };
        let result = (|| {
            if !index.exists() {
                run(&["read-tree", "--empty"])?;
            }
            run(&["add", "-A", "--", "."])?;
            // Store exact working-tree bytes, including CRLF/BOM. Clean filters must not
            // turn a task snapshot into a different file from the one on disk.
            let listed = command("git")
                .args(["ls-files", "--stage", "-z"])
                .current_dir(&root)
                .env("GIT_INDEX_FILE", &index)
                .output()?;
            if !listed.status.success() {
                bail!("Cannot enumerate snapshot index");
            }
            let mut entries = Vec::new();
            let mut names = Vec::new();
            for entry in listed.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                let tab = entry
                    .iter()
                    .position(|b| *b == b'\t')
                    .context("Invalid index entry")?;
                let mode = std::str::from_utf8(&entry[..6])?.to_owned();
                if mode != "100644" && mode != "100755" {
                    bail!(
                        "Snapshot contains a symlink or submodule; manual Git review remains available"
                    );
                }
                let path = std::str::from_utf8(&entry[tab + 1..])?.to_owned();
                names.extend(serde_json::to_string(&path)?.as_bytes());
                names.push(b'\n');
                entries.push((mode, path));
            }
            let run_input = |args: &[&str], input: Vec<u8>| -> Result<Vec<u8>> {
                use std::io::Write;
                let mut child = command("git")
                    .args(args)
                    .current_dir(&root)
                    .env("GIT_INDEX_FILE", &index)
                    .env("GIT_OBJECT_DIRECTORY", &objects)
                    .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &alternates)
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
            };
            if !entries.is_empty() {
                let hashes = run_input(
                    &["hash-object", "-w", "--no-filters", "--stdin-paths"],
                    names,
                )?;
                let hashes = std::str::from_utf8(&hashes)?.lines().collect::<Vec<_>>();
                if hashes.len() != entries.len() {
                    bail!("Incomplete raw snapshot");
                }
                let mut updates = Vec::new();
                for ((mode, path), hash) in entries.iter().zip(hashes) {
                    updates.extend(format!("{mode} {hash}\t{path}\0").as_bytes());
                }
                run_input(&["update-index", "-z", "--index-info"], updates)?;
            }
            let tree = run(&["write-tree"])?;
            let snapshot = Self {
                id: id.clone(),
                tree,
                root: root.clone(),
                objects: objects.clone(),
                alternates: alternates.clone(),
                label: label.into(),
                created: chrono::Local::now().format("%H:%M:%S").to_string(),
            };
            fs::write(
                store.join(format!("{id}.json")),
                serde_json::to_vec_pretty(&snapshot)?,
            )?;
            Ok(snapshot)
        })();
        let _ = fs::remove_file(&index);
        result
    }
    fn git(&self, args: &[&str]) -> Result<Output> {
        Ok(command("git")
            .args(["--no-pager", "-c", "core.quotepath=false"])
            .args(args)
            .current_dir(&self.root)
            .env("GIT_OBJECT_DIRECTORY", &self.objects)
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.alternates)
            .output()?)
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
        Ok(parts
            .chunks_exact(2)
            .map(|p| Change {
                status: String::from_utf8_lossy(p[0]).into(),
                path: String::from_utf8_lossy(p[1]).into(),
            })
            .collect())
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
        let bonus = if name_chars.starts_with(&query) { 40 } else { 20 };
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
        let order = entries.iter().map(|e| e.relative.as_str()).collect::<Vec<_>>();
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
                path: String::new(),
                status: s.into(),
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
        assert_eq!(
            before_content(&sub, "file.txt", DiffScope::Unstaged)?,
            Some(b"staged\n".to_vec())
        );
        assert_eq!(
            after_content(&sub, "file.txt", DiffScope::Unstaged)?,
            Some(b"working\n".to_vec())
        );
        assert_eq!(before_content(&sub, "new.txt", DiffScope::Unstaged)?, None);
        Ok(())
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
