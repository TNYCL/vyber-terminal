//! What a Review source changed: the files with their line counts, and the
//! before/after bytes of each file, read from Git in one batch per chunk.
//!
//! Sources are an agent turn (two Vyber snapshots), the working tree against
//! HEAD or the index, the index against HEAD, one commit, or the branch
//! against its merge base with the default branch. Nothing here writes to
//! `.git`; reverts only touch working-tree files and keep a recovery copy.
use crate::{
    tasks::TaskReview,
    workspace::{self, DiffLine},
};
use anyhow::{Context, Result, bail};
use gpui::HighlightStyle;
use gpui_kit::component::{
    Rope,
    highlighter::{HighlightTheme, SyntaxHighlighter},
};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const READ_LIMIT: u64 = 32 * 1024 * 1024;
const TEXT_LIMIT: usize = 8 * 1024 * 1024;
const HIGHLIGHT_LIMIT: usize = 2 * 1024 * 1024;

/// What the Review panel compares.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// An agent turn or a checkpoint review, by task id.
    Turn(String),
    /// Working tree against HEAD, untracked files included.
    Uncommitted,
    /// Working tree against the index, untracked files included.
    Unstaged,
    /// Index against HEAD.
    Staged,
    Commit {
        hash: String,
        title: String,
    },
    /// Working tree against the merge base with the default branch.
    Branch,
}

impl Source {
    /// The after side is the working tree (or a snapshot of it), so files and
    /// hunks can be reverted.
    pub fn editable(&self) -> bool {
        !matches!(self, Self::Staged | Self::Commit { .. })
    }
    /// Reload when files change on disk.
    pub fn follows_worktree(&self) -> bool {
        matches!(self, Self::Uncommitted | Self::Unstaged | Self::Branch)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileChange {
    /// Workspace-relative path, or repository-relative for turns.
    pub path: String,
    pub absolute: PathBuf,
    /// `A` added, `D` deleted or `M` modified.
    pub letter: char,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Blob {
    #[default]
    Missing,
    Bytes(Vec<u8>),
    TooLarge(u64),
}

impl Blob {
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }
    fn size(&self) -> u64 {
        match self {
            Self::Missing => 0,
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::TooLarge(size) => *size,
        }
    }
}

#[derive(Clone, Debug)]
enum Side {
    /// A Git object name prefix: `<tree-ish>:` or `:` for the index.
    Object(String),
    Worktree,
}

/// The files a source changed and where to read both sides of each.
pub struct Plan {
    pub files: Vec<FileChange>,
    /// Folder the file paths are relative to.
    pub root: PathBuf,
    /// What the changes are compared with, such as `HEAD` or `origin/main`.
    pub base: String,
    git: Git,
    before: Side,
    after: Side,
    /// Repository-relative path of each file, for object reads.
    objects: Vec<String>,
}

#[derive(Clone)]
struct Git {
    dir: PathBuf,
    env: Vec<(&'static str, PathBuf)>,
}

impl Git {
    fn command(&self) -> Command {
        let mut command = workspace::command("git");
        command
            .args(["--no-pager", "-c", "core.quotepath=false"])
            .current_dir(&self.dir)
            .env("GIT_OPTIONAL_LOCKS", "0");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
    fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .context("Git could not be started")?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(out.stdout)
    }
    fn text(&self, args: &[&str]) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.run(args)?).trim().to_owned())
    }
    /// Reads many objects through one `git cat-file --batch`.
    fn read(&self, specs: &[String]) -> Result<Vec<Blob>> {
        if specs.is_empty() {
            return Ok(vec![]);
        }
        let mut child = self
            .command()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Git could not be started")?;
        let mut stdin = child.stdin.take().context("Git input pipe")?;
        let mut input = Vec::new();
        for spec in specs {
            // A newline would split the request; such a path reads as missing.
            input.extend(
                if spec.contains('\n') {
                    "vyber-unreadable-path"
                } else {
                    spec
                }
                .as_bytes(),
            );
            input.push(b'\n');
        }
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let mut reader = BufReader::new(child.stdout.take().context("Git output pipe")?);
        let mut blobs = Vec::with_capacity(specs.len());
        let result = (|| -> Result<()> {
            for _ in specs {
                let mut header = Vec::new();
                if reader.read_until(b'\n', &mut header)? == 0 {
                    bail!("Git stopped before reading every file");
                }
                let header = String::from_utf8_lossy(&header);
                let header = header.trim_end();
                if header.ends_with(" missing") || header.ends_with(" ambiguous") {
                    blobs.push(Blob::Missing);
                    continue;
                }
                let mut fields = header.rsplitn(3, ' ');
                let size: u64 = fields
                    .next()
                    .and_then(|s| s.parse().ok())
                    .context("Unexpected Git object header")?;
                let kind = fields.next().unwrap_or_default();
                if size > READ_LIMIT {
                    std::io::copy(&mut (&mut reader).take(size + 1), &mut std::io::sink())?;
                    blobs.push(Blob::TooLarge(size));
                    continue;
                }
                let mut bytes = vec![0; size as usize];
                reader.read_exact(&mut bytes)?;
                let mut newline = [0];
                reader.read_exact(&mut newline)?;
                blobs.push(if kind == "blob" {
                    Blob::Bytes(bytes)
                } else {
                    Blob::Missing
                });
            }
            Ok(())
        })();
        drop(reader);
        let _ = writer.join();
        let _ = child.wait();
        result.map(|()| blobs)
    }
}

fn read_worktree(path: &Path) -> Blob {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() > READ_LIMIT => Blob::TooLarge(meta.len()),
        Ok(meta) if meta.is_file() => fs::read(path).map(Blob::Bytes).unwrap_or_default(),
        _ => Blob::Missing,
    }
}

fn letter(status: &str) -> char {
    match status.chars().next() {
        Some('A') => 'A',
        Some('D') => 'D',
        _ => 'M',
    }
}

pub fn sort_files(files: &mut [FileChange]) {
    files.sort_by(|a, b| workspace::path_order(&a.path, false, &b.path, false));
}

/// Plans a Git source (anything but a turn) for the workspace at `root`.
pub fn git_plan(root: &Path, source: &Source) -> Result<Plan> {
    let git = Git {
        dir: root.to_owned(),
        env: vec![],
    };
    git.run(&["rev-parse", "--is-inside-work-tree"])
        .context("Not a Git workspace")?;
    let prefix = git.text(&["rev-parse", "--show-prefix"])?;
    let head = git
        .text(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .ok()
        .filter(|h| !h.is_empty());
    let empty_tree = || git.text(&["hash-object", "-t", "tree", "--stdin"]);
    let head_or_empty = || head.clone().map(Ok).unwrap_or_else(empty_tree);
    let (args, before, after, untracked, base) = match source {
        Source::Uncommitted => {
            let base = head_or_empty()?;
            let side = Side::Object(format!("{base}:"));
            (vec![base], side, Side::Worktree, true, "HEAD".to_string())
        }
        Source::Unstaged => (
            vec![],
            Side::Object(":".into()),
            Side::Worktree,
            true,
            "index".into(),
        ),
        Source::Staged => {
            let base = head_or_empty()?;
            let side = Side::Object(format!("{base}:"));
            (
                vec!["--cached".into(), base],
                side,
                Side::Object(":".into()),
                false,
                "HEAD".into(),
            )
        }
        Source::Commit { hash, .. } => {
            let parent = git
                .text(&["rev-parse", "--verify", "--quiet", &format!("{hash}^")])
                .ok()
                .filter(|h| !h.is_empty())
                .map(Ok)
                .unwrap_or_else(empty_tree)?;
            (
                vec![parent.clone(), hash.clone()],
                Side::Object(format!("{parent}:")),
                Side::Object(format!("{hash}:")),
                false,
                hash.chars().take(7).collect(),
            )
        }
        Source::Branch => {
            let (base, name) = branch_base(&git, head.as_deref())?;
            let side = Side::Object(format!("{base}:"));
            (vec![base], side, Side::Worktree, true, name)
        }
        Source::Turn(_) => bail!("Turns are read from their snapshots"),
    };
    let diff = |kind: &str| -> Result<Vec<u8>> {
        let mut full = vec!["diff", "--no-renames", "--relative", "-z", kind];
        full.extend(args.iter().map(String::as_str));
        git.run(&full)
    };
    let names = diff("--name-status")?;
    let stats = workspace::parse_numstat(&diff("--numstat")?);
    let mut files = Vec::new();
    let parts: Vec<_> = names.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    for pair in parts.as_chunks::<2>().0 {
        let path = String::from_utf8_lossy(pair[1]).into_owned();
        let (additions, deletions) = stats.get(&path).copied().unwrap_or_default();
        files.push(FileChange {
            absolute: root.join(&path),
            letter: letter(&String::from_utf8_lossy(pair[0])),
            path,
            additions,
            deletions,
        });
    }
    if untracked {
        let out = git.run(&["ls-files", "--others", "--exclude-standard", "-z"])?;
        for path in out.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            let path = String::from_utf8_lossy(path).into_owned();
            if files.iter().any(|f| f.path == path) {
                continue;
            }
            let absolute = root.join(&path);
            let additions = match read_worktree(&absolute) {
                Blob::Bytes(bytes) => workspace::count_lines(&bytes),
                _ => 0,
            };
            files.push(FileChange {
                path,
                absolute,
                letter: 'A',
                additions,
                deletions: 0,
            });
        }
    }
    sort_files(&mut files);
    let objects = files
        .iter()
        .map(|f| format!("{prefix}{}", f.path))
        .collect();
    Ok(Plan {
        files,
        root: root.to_owned(),
        base,
        git,
        before,
        after,
        objects,
    })
}

/// Plans an agent turn from its start snapshot to its end (or live) snapshot.
/// Without an end snapshot the after side is the working tree.
pub fn turn_plan(task: &TaskReview) -> Result<Plan> {
    let before = task
        .before
        .as_ref()
        .context("This turn has no start snapshot, so its changes can't be shown")?;
    let git = Git {
        dir: before.root.clone(),
        env: before.object_env(),
    };
    let (changes, after) = match &task.after {
        Some(after) => (
            before.changes_to(after)?,
            Side::Object(format!("{}:", after.tree)),
        ),
        None => (task.changes.clone(), Side::Worktree),
    };
    let mut files = changes
        .iter()
        .map(|c| FileChange {
            path: c.path.clone(),
            absolute: before.root.join(&c.path),
            letter: letter(&c.status),
            additions: c.additions,
            deletions: c.deletions,
        })
        .collect::<Vec<_>>();
    sort_files(&mut files);
    let objects = files.iter().map(|f| f.path.clone()).collect();
    Ok(Plan {
        files,
        root: before.root.clone(),
        base: format!("snapshot at {}", before.created),
        git,
        before: Side::Object(format!("{}:", before.tree)),
        after,
        objects,
    })
}

impl Plan {
    /// Before and after bytes of `files[range]`.
    pub fn read(&self, range: Range<usize>) -> Result<Vec<(Blob, Blob)>> {
        let mut result = vec![(Blob::Missing, Blob::Missing); range.len()];
        let mut specs = Vec::new();
        let mut slots = Vec::new();
        for (n, i) in range.enumerate() {
            let file = &self.files[i];
            for (after, side) in [(false, &self.before), (true, &self.after)] {
                if (!after && file.letter == 'A') || (after && file.letter == 'D') {
                    continue;
                }
                match side {
                    Side::Object(prefix) => {
                        specs.push(format!("{prefix}{}", self.objects[i]));
                        slots.push((n, after));
                    }
                    Side::Worktree => {
                        let blob = read_worktree(&file.absolute);
                        if after {
                            result[n].1 = blob;
                        } else {
                            result[n].0 = blob;
                        }
                    }
                }
            }
        }
        for ((n, after), blob) in slots.into_iter().zip(self.git.read(&specs)?) {
            if after {
                result[n].1 = blob;
            } else {
                result[n].0 = blob;
            }
        }
        Ok(result)
    }
}

/// The repository around `root` and `relative`'s path inside it, for
/// commands that take repository paths (the index, `git add`).
pub fn repository_path(root: &Path, relative: &str) -> Result<(PathBuf, String)> {
    let git = Git {
        dir: root.to_owned(),
        env: vec![],
    };
    let top = PathBuf::from(git.text(&["rev-parse", "--show-toplevel"])?);
    let prefix = git.text(&["rev-parse", "--show-prefix"])?;
    Ok((top, format!("{prefix}{relative}")))
}

/// Merge base of HEAD with the default branch, and that branch's name.
fn branch_base(git: &Git, head: Option<&str>) -> Result<(String, String)> {
    let head = head.context("Branch review needs at least one commit")?;
    let current = git
        .text(&["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_default();
    let mut names = Vec::new();
    if let Ok(name) = git.text(&[
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ]) && !name.is_empty()
    {
        names.push(name);
    }
    for name in ["origin/main", "origin/master", "main", "master"] {
        if !names.iter().any(|n| n == name) {
            names.push(name.into());
        }
    }
    let mut on_default = false;
    for name in names {
        if name == current {
            on_default = true;
            continue;
        }
        if let Ok(base) = git.text(&["merge-base", head, &name])
            && !base.is_empty()
        {
            return Ok((base, name));
        }
    }
    if on_default {
        bail!(
            "You're on {current}, the default branch. Branch shows what a feature branch changed since it left {current}."
        )
    }
    bail!("No default branch to compare with (origin/main, main or master)")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub title: String,
    pub when: String,
}

/// Recent commits that touched the workspace folder, newest first.
pub fn commits(root: &Path) -> Result<Vec<Commit>> {
    let git = Git {
        dir: root.to_owned(),
        env: vec![],
    };
    let out = git.text(&[
        "log",
        "-n",
        "40",
        "--format=%H%x1f%h%x1f%s%x1f%cr",
        "--",
        ".",
    ])?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\x1f');
            Some(Commit {
                hash: parts.next()?.into(),
                short: parts.next()?.into(),
                title: parts.next()?.into(),
                when: parts.next().unwrap_or_default().into(),
            })
        })
        .collect())
}

/// A run of added and removed lines: its rows in [`FileDiff::lines`] and the
/// zero-based line ranges it covers in the old and new file.
#[derive(Clone, Debug, PartialEq)]
pub struct Hunk {
    pub lines: Range<usize>,
    pub old: Range<usize>,
    pub new: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct FileDiff {
    /// Every line of the file pair, unchanged lines included.
    pub lines: Vec<DiffLine>,
    pub hunks: Vec<Hunk>,
    /// Why there is no line diff: binary, too large or no content change.
    pub note: Option<String>,
    pub before: Blob,
    pub after: Blob,
}

fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MB", b as f64 / 1024. / 1024.),
        b if b >= 1024 => format!("{:.1} KB", b as f64 / 1024.),
        b => format!("{b} B"),
    }
}

pub fn file_diff(
    path: &str,
    before: Blob,
    after: Blob,
    theme: Option<&HighlightTheme>,
) -> FileDiff {
    let old = before.bytes().unwrap_or_default();
    let new = after.bytes().unwrap_or_default();
    let too_large = matches!(before, Blob::TooLarge(_))
        || matches!(after, Blob::TooLarge(_))
        || old.len() > TEXT_LIMIT
        || new.len() > TEXT_LIMIT;
    let note = if too_large {
        Some(format!(
            "Too large to compare · {}",
            human(before.size().max(after.size()))
        ))
    } else if old.contains(&0) || new.contains(&0) {
        Some(match (&before, &after) {
            (Blob::Missing, _) => format!("Binary file added · {}", human(after.size())),
            (_, Blob::Missing) => format!("Binary file deleted · {}", human(before.size())),
            _ => format!(
                "Binary file changed · {} → {}",
                human(before.size()),
                human(after.size())
            ),
        })
    } else {
        None
    };
    if note.is_some() {
        return FileDiff {
            note,
            before,
            after,
            ..Default::default()
        };
    }
    let mut lines = workspace::diff_lines_context(old, new, usize::MAX / 4);
    if let Some(theme) = theme {
        let language = language(Path::new(path));
        if language != "plain_text" {
            let old_spans = line_spans(&String::from_utf8_lossy(old), language, theme);
            let new_spans = line_spans(&String::from_utf8_lossy(new), language, theme);
            for line in &mut lines {
                let spans = if line.kind == '-' {
                    line.old.and_then(|n| old_spans.get(n - 1))
                } else {
                    line.new.and_then(|n| new_spans.get(n - 1))
                };
                if let Some(spans) = spans {
                    line.spans = spans.clone();
                }
            }
        }
    }
    for line in &mut lines {
        expand_tabs(line);
    }
    let hunks = hunks(&lines);
    let note = lines.is_empty().then(|| {
        if old != new && !old.is_empty() && !new.is_empty() {
            "Only line endings changed".to_string()
        } else {
            "No content changes".to_string()
        }
    });
    FileDiff {
        lines,
        hunks,
        note,
        before,
        after,
    }
}

/// Syntax colors of each line of `text`, as byte ranges within that line
/// (without its line ending), sorted and non-overlapping.
pub fn line_spans(
    text: &str,
    language: &str,
    theme: &HighlightTheme,
) -> Vec<Vec<(Range<usize>, HighlightStyle)>> {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut result = vec![Vec::new(); starts.len()];
    if text.is_empty() || text.len() > HIGHLIGHT_LIMIT {
        return result;
    }
    let mut highlighter = SyntaxHighlighter::new(language);
    highlighter.update(None, &Rope::from_str(text), None);
    for (range, style) in highlighter.styles(&(0..text.len()), theme) {
        let mut line = starts
            .partition_point(|s| *s <= range.start)
            .saturating_sub(1);
        let mut from = range.start;
        while from < range.end && line < starts.len() {
            let start = starts[line];
            let mut end = starts.get(line + 1).map_or(text.len(), |s| s - 1);
            if text[..end].ends_with('\r') {
                end -= 1;
            }
            let to = range.end.min(end);
            if to > from {
                result[line].push((from - start..to - start, style));
            }
            line += 1;
            from = starts.get(line).copied().unwrap_or(range.end);
        }
    }
    for spans in &mut result {
        spans.sort_by_key(|(range, _)| range.start);
        let mut end = 0;
        spans.retain_mut(|(range, _)| {
            range.start = range.start.max(end);
            if range.start >= range.end {
                return false;
            }
            end = range.end;
            true
        });
    }
    result
}

/// Replaces tabs with spaces to the next multiple of four columns and moves
/// the syntax spans with the text.
fn expand_tabs(line: &mut DiffLine) {
    if !line.text.contains('\t') {
        return;
    }
    let mut text = String::with_capacity(line.text.len() + 16);
    // Byte offset in the new text for each byte offset in the old text.
    let mut map = Vec::with_capacity(line.text.len() + 1);
    let mut column = 0;
    for (offset, c) in line.text.char_indices() {
        while map.len() <= offset {
            map.push(text.len());
        }
        if c == '\t' {
            let width = 4 - column % 4;
            text.extend(std::iter::repeat_n(' ', width));
            column += width;
        } else {
            text.push(c);
            column += 1;
        }
    }
    while map.len() <= line.text.len() {
        map.push(text.len());
    }
    for (range, _) in &mut line.spans {
        *range = map[range.start]..map[range.end];
    }
    line.text = text;
}

pub fn hunks(lines: &[DiffLine]) -> Vec<Hunk> {
    let changed = |line: &DiffLine| matches!(line.kind, '+' | '-');
    let mut result = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !changed(&lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && changed(&lines[i]) {
            i += 1;
        }
        let run = &lines[start..i];
        let previous = start.checked_sub(1).map(|p| &lines[p]);
        let old_start = run
            .iter()
            .find_map(|l| l.old.filter(|_| l.kind == '-'))
            .map(|n| n - 1)
            .unwrap_or_else(|| previous.and_then(|p| p.old).unwrap_or(0));
        let new_start = run
            .iter()
            .find_map(|l| l.new.filter(|_| l.kind == '+'))
            .map(|n| n - 1)
            .unwrap_or_else(|| previous.and_then(|p| p.new).unwrap_or(0));
        let removed = run.iter().filter(|l| l.kind == '-').count();
        let added = run.len() - removed;
        result.push(Hunk {
            lines: start..i,
            old: old_start..old_start + removed,
            new: new_start..new_start + added,
        });
    }
    result
}

/// The after bytes with one hunk put back to its before lines. Every other
/// byte is kept; the restored lines take the file's line ending.
pub fn revert_hunk(before: &[u8], after: &[u8], hunk: &Hunk) -> Result<Vec<u8>> {
    let old: Vec<&[u8]> = before.split_inclusive(|b| *b == b'\n').collect();
    if hunk.old.end > old.len() {
        bail!("This change no longer matches the file. Refresh and try again.");
    }
    splice_lines(after, hunk.new.clone(), &old[hunk.old.clone()])
}

/// The before bytes with one hunk's after lines applied, as staging a hunk
/// writes it into the index.
pub fn apply_hunk(before: &[u8], after: &[u8], hunk: &Hunk) -> Result<Vec<u8>> {
    let new: Vec<&[u8]> = after.split_inclusive(|b| *b == b'\n').collect();
    if hunk.new.end > new.len() {
        bail!("This change no longer matches the file. Refresh and try again.");
    }
    splice_lines(before, hunk.old.clone(), &new[hunk.new.clone()])
}

/// The line ending most lines use, if any line has one.
fn line_ending(lines: &[&[u8]]) -> Option<&'static [u8]> {
    let crlf = lines.iter().filter(|l| l.ends_with(b"\r\n")).count();
    let lf = lines.iter().filter(|l| l.ends_with(b"\n")).count() - crlf;
    match (crlf, lf) {
        (0, 0) => None,
        (crlf, lf) if crlf > lf => Some(b"\r\n"),
        _ => Some(b"\n"),
    }
}

/// `target` with its lines `range` replaced by `lines`. Inserted lines take
/// `target`'s line ending, and every line but the last keeps one.
pub fn splice_lines(target: &[u8], range: Range<usize>, lines: &[&[u8]]) -> Result<Vec<u8>> {
    let old: Vec<&[u8]> = target.split_inclusive(|b| *b == b'\n').collect();
    if range.end > old.len() || range.start > range.end {
        bail!("This change no longer matches the file. Refresh and try again.");
    }
    let ending = line_ending(&old).or_else(|| line_ending(lines));
    let convert = |line: &[u8]| -> Vec<u8> {
        let body = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"));
        match (body, ending) {
            (Some(body), Some(ending)) => [body, ending].concat(),
            _ => line.to_vec(),
        }
    };
    let mut pieces: Vec<Vec<u8>> = old[..range.start].iter().map(|l| l.to_vec()).collect();
    pieces.extend(lines.iter().map(|l| convert(l)));
    pieces.extend(old[range.end..].iter().map(|l| l.to_vec()));
    let count = pieces.len();
    for piece in pieces.iter_mut().take(count.saturating_sub(1)) {
        if !piece.ends_with(b"\n") {
            piece.extend_from_slice(ending.unwrap_or(b"\n"));
        }
    }
    Ok(pieces.concat())
}

/// One file to put back: it must still hold `expected`, and gets `target`.
pub struct Restore {
    pub root: PathBuf,
    pub path: String,
    pub expected: Blob,
    pub target: Blob,
}

/// Writes every target after checking that no file changed since it was
/// read. Nothing is written if any check fails; each overwritten or deleted
/// file keeps a recovery copy.
pub fn restore(files: &[Restore]) -> Result<()> {
    let mut checked = Vec::with_capacity(files.len());
    for file in files {
        let path = workspace::safe_path(&file.root, &file.path)?;
        if matches!(file.expected, Blob::TooLarge(_)) || matches!(file.target, Blob::TooLarge(_)) {
            bail!("{} is too large to revert here", file.path);
        }
        let current = workspace::read_optional(&path)?;
        if current.as_deref() != file.expected.bytes() {
            bail!(
                "{} changed since this review; nothing was reverted. Refresh and try again.",
                file.path
            );
        }
        checked.push((path, current));
    }
    for (file, (path, current)) in files.iter().zip(checked) {
        workspace::keep_recovery(&path, current.as_deref())?;
        match &file.target {
            Blob::Bytes(bytes) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, bytes)?;
            }
            _ => {
                if current.is_some() {
                    fs::remove_file(&path)?;
                }
            }
        }
    }
    Ok(())
}

/// A unified patch of the loaded text files, like `git diff` prints it.
pub fn patch(files: &[(FileChange, &FileDiff)]) -> String {
    let mut out = String::new();
    for (file, diff) in files {
        let path = &file.path;
        out.push_str(&format!("diff --git a/{path} b/{path}\n"));
        match file.letter {
            'A' => out.push_str("new file mode 100644\n"),
            'D' => out.push_str("deleted file mode 100644\n"),
            _ => {}
        }
        let old = diff.before.bytes().unwrap_or_default();
        let new = diff.after.bytes().unwrap_or_default();
        if matches!(diff.before, Blob::TooLarge(_))
            || matches!(diff.after, Blob::TooLarge(_))
            || old.contains(&0)
            || new.contains(&0)
        {
            out.push_str(&format!("Binary files a/{path} and b/{path} differ\n"));
            continue;
        }
        let old = String::from_utf8_lossy(old);
        let new = String::from_utf8_lossy(new);
        let old_name = if file.letter == 'A' {
            "/dev/null".to_string()
        } else {
            format!("a/{path}")
        };
        let new_name = if file.letter == 'D' {
            "/dev/null".to_string()
        } else {
            format!("b/{path}")
        };
        out.push_str(
            &similar::TextDiff::from_lines(&old, &new)
                .unified_diff()
                .context_radius(3)
                .header(&old_name, &new_name)
                .to_string(),
        );
    }
    out
}

/// Syntax name for the editor and the diff highlighter.
pub fn language(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match name.as_str() {
        "cargo.lock" | "pipfile" | "poetry.lock" => return "toml",
        ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" | "pkgbuild" => return "bash",
        _ if name.starts_with(".env") => return "bash",
        _ => {}
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" => "rust",
        "go" => "go",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "py" | "pyi" | "pyw" => "python",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "html" | "htm" => "html",
        "css" => "css",
        "sh" | "bash" | "zsh" => "bash",
        "md" | "markdown" | "mdx" => "markdown",
        _ => "plain_text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Checkpoint, git_text};

    fn repo() -> Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        git_text(root, &["init", "-b", "main"])?;
        git_text(root, &["config", "core.autocrlf", "false"])?;
        git_text(root, &["config", "user.email", "test@example.com"])?;
        git_text(root, &["config", "user.name", "Test"])?;
        Ok(dir)
    }

    /// Path, letter, additions, deletions, before and after of every file.
    type Read = (String, char, usize, usize, Blob, Blob);

    fn read(plan: &Plan) -> Result<Vec<Read>> {
        let blobs = plan.read(0..plan.files.len())?;
        Ok(plan
            .files
            .iter()
            .zip(blobs)
            .map(|(f, (b, a))| (f.path.clone(), f.letter, f.additions, f.deletions, b, a))
            .collect())
    }

    fn bytes(text: &str) -> Blob {
        Blob::Bytes(text.as_bytes().to_vec())
    }

    #[test]
    fn git_sources_list_files_counts_and_both_sides() -> Result<()> {
        let dir = repo()?;
        let root = dir.path();
        fs::write(root.join("keep.txt"), "one\ntwo\n")?;
        fs::write(root.join("gone.txt"), "bye\n")?;
        git_text(root, &["add", "."])?;
        git_text(root, &["commit", "-m", "first"])?;
        fs::write(root.join("keep.txt"), "one\nTWO\nthree\n")?;
        git_text(root, &["add", "keep.txt"])?;
        fs::write(root.join("keep.txt"), "one\nTWO\nthree\nfour\n")?;
        fs::remove_file(root.join("gone.txt"))?;
        fs::write(root.join("new.txt"), "a\nb")?;

        let uncommitted = read(&git_plan(root, &Source::Uncommitted)?)?;
        assert_eq!(
            uncommitted,
            vec![
                ("gone.txt".into(), 'D', 0, 1, bytes("bye\n"), Blob::Missing),
                (
                    "keep.txt".into(),
                    'M',
                    3,
                    1,
                    bytes("one\ntwo\n"),
                    bytes("one\nTWO\nthree\nfour\n")
                ),
                ("new.txt".into(), 'A', 2, 0, Blob::Missing, bytes("a\nb")),
            ]
        );
        let staged = read(&git_plan(root, &Source::Staged)?)?;
        assert_eq!(
            staged,
            vec![(
                "keep.txt".into(),
                'M',
                2,
                1,
                bytes("one\ntwo\n"),
                bytes("one\nTWO\nthree\n")
            )]
        );
        let unstaged = read(&git_plan(root, &Source::Unstaged)?)?;
        assert_eq!(unstaged.len(), 3);
        assert_eq!(unstaged[1].4, bytes("one\nTWO\nthree\n"));
        assert_eq!(unstaged[1].2, 1);

        let head = git_text(root, &["rev-parse", "HEAD"])?;
        let commit = read(&git_plan(
            root,
            &Source::Commit {
                hash: head,
                title: "first".into(),
            },
        )?)?;
        assert_eq!(commit.len(), 2);
        assert!(commit.iter().all(|f| f.1 == 'A' && f.4 == Blob::Missing));
        let on_main = git_plan(root, &Source::Branch).err().unwrap().to_string();
        assert!(on_main.starts_with("You're on main, the default branch."));
        Ok(())
    }

    #[test]
    fn sources_work_from_a_subdirectory_and_branch_uses_the_merge_base() -> Result<()> {
        let dir = repo()?;
        let root = dir.path();
        fs::create_dir(root.join("sub"))?;
        fs::write(root.join("sub/file.txt"), "base\n")?;
        fs::write(root.join("top.txt"), "top\n")?;
        git_text(root, &["add", "."])?;
        git_text(root, &["commit", "-m", "base"])?;
        git_text(root, &["checkout", "-b", "feature"])?;
        fs::write(root.join("sub/file.txt"), "feature\n")?;
        git_text(root, &["commit", "-am", "feature"])?;
        fs::write(root.join("sub/extra.txt"), "x\n")?;
        fs::write(root.join("top.txt"), "changed\n")?;

        let sub = root.join("sub");
        let plan = git_plan(&sub, &Source::Branch)?;
        assert_eq!(plan.base, "main");
        let files = read(&plan)?;
        assert_eq!(
            files,
            vec![
                ("extra.txt".into(), 'A', 1, 0, Blob::Missing, bytes("x\n")),
                (
                    "file.txt".into(),
                    'M',
                    1,
                    1,
                    bytes("base\n"),
                    bytes("feature\n")
                ),
            ]
        );
        assert!(
            read(&git_plan(&sub, &Source::Uncommitted)?)?
                .iter()
                .all(|f| f.0 == "extra.txt")
        );
        assert_eq!(commits(&sub)?.len(), 2);
        Ok(())
    }

    #[test]
    fn unborn_repository_compares_with_the_empty_tree() -> Result<()> {
        let dir = repo()?;
        let root = dir.path();
        fs::write(root.join("a.txt"), "a\n")?;
        git_text(root, &["add", "a.txt"])?;
        let staged = read(&git_plan(root, &Source::Staged)?)?;
        assert_eq!(
            staged,
            vec![("a.txt".into(), 'A', 1, 0, Blob::Missing, bytes("a\n"))]
        );
        assert_eq!(read(&git_plan(root, &Source::Uncommitted)?)?.len(), 1);
        assert!(git_plan(root, &Source::Branch).is_err());
        Ok(())
    }

    #[test]
    fn turn_plan_reads_both_snapshots_with_line_counts() -> Result<()> {
        let dir = repo()?;
        let root = dir.path();
        fs::write(root.join("a.txt"), "one\n")?;
        let before = Checkpoint::capture(root, "start")?;
        fs::write(root.join("a.txt"), "one\ntwo\n")?;
        fs::write(root.join("b.txt"), "new\n")?;
        let after = Checkpoint::capture(root, "end")?;
        let task = TaskReview {
            id: "t".into(),
            agent: "Claude".into(),
            label: String::new(),
            root: root.to_owned(),
            session: String::new(),
            client: String::new(),
            started: 0,
            changes: before.changes_to(&after)?,
            before: Some(before),
            after: Some(after),
            active: false,
            warning: String::new(),
        };
        assert_eq!(task.changes[0].additions, 1);
        let files = read(&turn_plan(&task)?)?;
        assert_eq!(
            files,
            vec![
                (
                    "a.txt".into(),
                    'M',
                    1,
                    0,
                    bytes("one\n"),
                    bytes("one\ntwo\n")
                ),
                ("b.txt".into(), 'A', 1, 0, Blob::Missing, bytes("new\n")),
            ]
        );
        Ok(())
    }

    #[test]
    fn turn_plan_reads_a_folder_of_repositories() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        for name in ["api", "web"] {
            fs::create_dir(root.join(name))?;
            git_text(&root.join(name), &["init", "-q"])?;
            git_text(&root.join(name), &["config", "core.autocrlf", "false"])?;
            fs::write(root.join(name).join("a.txt"), "one\n")?;
        }
        let before = Checkpoint::capture(root, "start")?;
        fs::write(root.join("api/a.txt"), "one\ntwo\n")?;
        fs::write(root.join("web/b.txt"), "new\n")?;
        let mut task = TaskReview {
            id: "t".into(),
            agent: "Codex".into(),
            label: String::new(),
            root: root.to_owned(),
            session: String::new(),
            client: String::new(),
            started: 0,
            changes: vec![],
            before: Some(before.clone()),
            after: None,
            active: true,
            warning: String::new(),
        };
        let expected = vec![
            (
                "api/a.txt".into(),
                'M',
                1,
                0,
                bytes("one\n"),
                bytes("one\ntwo\n"),
            ),
            ("web/b.txt".into(), 'A', 1, 0, Blob::Missing, bytes("new\n")),
        ];
        // Live, the after side is the working tree; at the end, a snapshot.
        let live = before.refresh("live", &[root.join("api/a.txt"), root.join("web/b.txt")])?;
        task.changes = before.changes_to(&live)?;
        assert_eq!(read(&turn_plan(&task)?)?, expected);
        task.after = Some(Checkpoint::capture(root, "end")?);
        assert_eq!(read(&turn_plan(&task)?)?, expected);
        Ok(())
    }

    #[test]
    fn every_hunk_reverts_exactly() -> Result<()> {
        let before = b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj".to_vec();
        let after = b"a\r\nB\r\nc\r\nd\r\ne\r\nf\r\nnew\r\ng\r\nh\r\ni\r\nj\n".to_vec();
        let diff = file_diff(
            "x.txt",
            Blob::Bytes(before.clone()),
            Blob::Bytes(after.clone()),
            None,
        );
        assert_eq!(diff.hunks.len(), 3);
        let first = revert_hunk(&before, &after, &diff.hunks[0])?;
        assert_eq!(
            first,
            b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\nnew\r\ng\r\nh\r\ni\r\nj\n"
        );
        let second = revert_hunk(&before, &after, &diff.hunks[1])?;
        assert_eq!(second, b"a\r\nB\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj\n");
        let third = revert_hunk(&before, &after, &diff.hunks[2])?;
        assert_eq!(
            third,
            b"a\r\nB\r\nc\r\nd\r\ne\r\nf\r\nnew\r\ng\r\nh\r\ni\r\nj"
        );
        // Reverting all hunks one after another gives the original bytes.
        let mut current = after.clone();
        for index in (0..diff.hunks.len()).rev() {
            let hunk = &file_diff(
                "x.txt",
                Blob::Bytes(before.clone()),
                Blob::Bytes(current.clone()),
                None,
            )
            .hunks[index];
            current = revert_hunk(&before, &current, hunk)?;
        }
        assert_eq!(current, before);
        Ok(())
    }

    #[test]
    fn pure_insertions_and_deletions_at_the_edges() -> Result<()> {
        let before = b"x\ny\n".to_vec();
        let after = b"top\nx\ny\nbottom\n".to_vec();
        let diff = file_diff(
            "x.txt",
            Blob::Bytes(before.clone()),
            Blob::Bytes(after.clone()),
            None,
        );
        assert_eq!(
            diff.hunks,
            vec![
                Hunk {
                    lines: 0..1,
                    old: 0..0,
                    new: 0..1
                },
                Hunk {
                    lines: 3..4,
                    old: 2..2,
                    new: 3..4
                }
            ]
        );
        assert_eq!(
            revert_hunk(&before, &after, &diff.hunks[0])?,
            b"x\ny\nbottom\n"
        );
        assert_eq!(
            revert_hunk(&before, &after, &diff.hunks[1])?,
            b"top\nx\ny\n"
        );
        let removed = file_diff(
            "x.txt",
            Blob::Bytes(after.clone()),
            Blob::Bytes(before.clone()),
            None,
        );
        assert_eq!(
            revert_hunk(&after, &before, &removed.hunks[0])?,
            b"top\nx\ny\n"
        );
        Ok(())
    }

    #[test]
    fn restore_checks_every_file_first_and_keeps_bytes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        fs::write(root.join("a"), "after-a")?;
        fs::write(root.join("b"), "unexpected")?;
        let entry = |path: &str, expected: &str, target: Blob| Restore {
            root: root.to_owned(),
            path: path.into(),
            expected: bytes(expected),
            target,
        };
        let stale = [
            entry("a", "after-a", bytes("before-a")),
            entry("b", "after-b", bytes("before-b")),
        ];
        assert!(restore(&stale).is_err());
        assert_eq!(fs::read_to_string(root.join("a"))?, "after-a");
        restore(&[
            entry("a", "after-a", Blob::Missing),
            entry("b", "unexpected", bytes("x")),
        ])?;
        assert!(!root.join("a").exists());
        assert_eq!(fs::read_to_string(root.join("b"))?, "x");
        assert!(
            restore(&[Restore {
                root: root.to_owned(),
                path: "../escape".into(),
                expected: Blob::Missing,
                target: bytes("x"),
            }])
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn binary_and_identical_files_get_a_note() {
        let binary = file_diff("a.png", bytes("a\0b"), bytes("a\0c"), None);
        assert!(binary.note.unwrap().starts_with("Binary file changed"));
        let same = file_diff("a.txt", bytes("same\n"), bytes("same\n"), None);
        assert_eq!(same.note.as_deref(), Some("No content changes"));
    }

    #[test]
    fn rust_lines_get_syntax_spans_per_line() {
        let theme = HighlightTheme::default_dark();
        let text = "fn main() {\r\n    let x = \"hi\";\r\n}\n";
        let spans = line_spans(text, "rust", &theme);
        assert_eq!(spans.len(), 4);
        assert!(!spans[0].is_empty());
        let second = "    let x = \"hi\";";
        assert!(spans[1].iter().any(|(r, _)| &second[r.clone()] == "\"hi\""));
        assert!(spans[1].iter().all(|(r, _)| r.end <= second.len()));
        let diff = file_diff(
            "a.rs",
            bytes("fn a() {}\n"),
            bytes("fn b() {}\n"),
            Some(&theme),
        );
        assert!(diff.lines.iter().all(|l| !l.spans.is_empty()));
    }

    #[test]
    fn tabs_expand_to_four_columns_and_move_spans() {
        let mut line = DiffLine {
            text: "\tab\tc".into(),
            spans: vec![
                (1..3, HighlightStyle::default()),
                (4..5, HighlightStyle::default()),
            ],
            ..Default::default()
        };
        expand_tabs(&mut line);
        assert_eq!(line.text, "    ab  c");
        assert_eq!(line.spans[0].0, 4..6);
        assert_eq!(line.spans[1].0, 8..9);
    }

    #[test]
    fn patch_matches_git_format() {
        let file = FileChange {
            path: "a.txt".into(),
            absolute: PathBuf::from("a.txt"),
            letter: 'M',
            additions: 1,
            deletions: 1,
        };
        let diff = file_diff("a.txt", bytes("one\ntwo\n"), bytes("one\nthree\n"), None);
        let text = patch(&[(file, &diff)]);
        assert!(text.starts_with("diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@"));
        assert!(text.contains("-two\n+three\n"));
    }
}
