//! Git state for the Git panel, read with the `git` command line so the
//! user's own configuration, hooks and credential helpers apply exactly as
//! in a terminal. Everything here reads; `ops` writes.
pub mod graph;
pub mod ops;

use crate::{project, tasks::matches_root, workspace};
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A repository or linked worktree shown in the Git panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Repo {
    pub path: PathBuf,
    pub name: String,
    /// The main repository of a linked worktree.
    pub worktree_of: Option<PathBuf>,
    /// The project's primary folder, or the workspace's own repository.
    pub primary: bool,
    /// Web page of the repository on its host, for commit links.
    pub web: Option<String>,
}

impl Repo {
    pub fn is_worktree(&self) -> bool {
        self.worktree_of.is_some()
    }
}

/// Runs git in `dir` for reading: no optional locks, no prompts.
pub fn read(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = workspace::command("git")
        .args(["--no-pager", "-c", "core.quotepath=false", "-c", "color.ui=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

fn read_text(dir: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&read(dir, args)?).trim_end().to_owned())
}

/// The main worktree of the repository at `path` (itself unless `path` is a
/// linked worktree).
pub fn main_worktree(path: &Path) -> Option<PathBuf> {
    let common = read_text(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let common = PathBuf::from(common);
    if common.file_name().is_some_and(|n| n == ".git") {
        common.parent().map(Path::to_path_buf)
    } else {
        // A bare repository has no main worktree.
        None
    }
}

/// The repositories the Git panel shows for a terminal in `root`: the
/// project's source folders when `root` belongs to a project, otherwise the
/// repository around `root` and those below it. Linked worktrees follow the
/// repository they belong to.
pub fn repositories(root: &Path) -> Vec<Repo> {
    let own = workspace::repository_root(root);
    let owner = own.as_deref().and_then(main_worktree);
    let project = project::for_path(root).or_else(|| owner.as_deref().and_then(project::for_path));
    let mut mains: Vec<(PathBuf, bool)> = Vec::new();
    match &project {
        Some(project) => {
            for (i, folder) in project.folders.iter().enumerate() {
                if project::is_repository(folder) && !project::is_linked_worktree(folder) {
                    mains.push((folder.clone(), i == 0));
                }
            }
        }
        None => {
            let base = owner.clone().or_else(|| own.clone());
            let scan = base.clone().unwrap_or_else(|| root.to_path_buf());
            if let Some(base) = base {
                mains.push((base, true));
            }
            for nested in project::discover(&scan) {
                mains.push((nested, false));
            }
        }
    }
    // A repository outside the project (a worktree elsewhere, say) still shows.
    if let Some(owner) = &owner
        && !mains.iter().any(|(p, _)| same_path(p, owner))
    {
        mains.push((owner.clone(), mains.is_empty()));
    }
    let mut repos: Vec<Repo> = Vec::new();
    for (path, primary) in mains {
        if repos.iter().any(|r| same_path(&r.path, &path)) {
            continue;
        }
        let worktrees = worktrees(&path).unwrap_or_default();
        let web = read_text(&path, &["remote"])
            .ok()
            .and_then(|remotes| {
                let remotes: Vec<&str> = remotes.lines().collect();
                let remote = remotes
                    .iter()
                    .find(|r| **r == "origin")
                    .or(remotes.first())?
                    .to_string();
                web_url(&path, &remote)
            });
        repos.push(Repo {
            name: project::folder_name(&path),
            path: path.clone(),
            worktree_of: None,
            primary,
            web: web.clone(),
        });
        for worktree in worktrees.into_iter().skip(1) {
            if worktree.bare || repos.iter().any(|r| same_path(&r.path, &worktree.path)) {
                continue;
            }
            repos.push(Repo {
                name: project::folder_name(&worktree.path),
                path: worktree.path,
                worktree_of: Some(path.clone()),
                primary: false,
                web: web.clone(),
            });
        }
    }
    repos
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    matches_root(a, b) && matches_root(b, a)
}

// ---- Status ------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    /// The old path of a rename or copy.
    pub from: Option<String>,
    /// `M`, `A`, `D`, `R`, `C`, `U` (untracked) or `!` (conflict).
    pub letter: char,
    /// How a conflict came about, such as "both modified".
    pub conflict: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation {
    Merge,
    Rebase { step: usize, total: usize },
    CherryPick,
    Revert,
    Bisect,
}

impl Operation {
    pub fn label(&self) -> String {
        match self {
            Self::Merge => "Merging".into(),
            Self::Rebase { step, total } if *total > 0 => format!("Rebasing {step}/{total}"),
            Self::Rebase { .. } => "Rebasing".into(),
            Self::CherryPick => "Cherry-picking".into(),
            Self::Revert => "Reverting".into(),
            Self::Bisect => "Bisecting".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// `None` before the first commit.
    pub oid: Option<String>,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub stashes: usize,
    pub staged: Vec<Entry>,
    pub unstaged: Vec<Entry>,
    pub conflicts: Vec<Entry>,
    pub operation: Option<Operation>,
    pub remotes: Vec<String>,
}

impl Status {
    pub fn changes(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.conflicts.len()
    }
    pub fn head_label(&self) -> String {
        match (&self.branch, &self.oid) {
            (Some(branch), _) => branch.clone(),
            (None, Some(oid)) => format!("{} (detached)", &oid[..oid.len().min(7)]),
            (None, None) => "HEAD".into(),
        }
    }
}

fn letter(code: u8) -> char {
    match code {
        b'A' => 'A',
        b'D' => 'D',
        b'R' => 'R',
        b'C' => 'C',
        _ => 'M',
    }
}

fn conflict(xy: &[u8]) -> &'static str {
    match xy {
        b"DD" => "both deleted",
        b"AU" => "added by us",
        b"UD" => "deleted by them",
        b"UA" => "added by them",
        b"DU" => "deleted by us",
        b"AA" => "both added",
        _ => "both modified",
    }
}

/// Parses `git status --porcelain=v2 -z --branch --show-stash`.
pub fn parse_status(bytes: &[u8]) -> Status {
    let mut status = Status::default();
    let mut fields = bytes.split(|b| *b == 0);
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(record);
        if let Some(header) = text.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.oid = Some(value.into()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.into()),
                "branch.upstream" => status.upstream = Some(value.into()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                "stash" => status.stashes = value.parse().unwrap_or(0),
                _ => {}
            }
            continue;
        }
        let kind = record[0];
        let (xy, path, from) = match kind {
            b'1' => {
                let parts: Vec<&str> = text.splitn(9, ' ').collect();
                if parts.len() < 9 {
                    continue;
                }
                (parts[1].as_bytes().to_vec(), parts[8].to_string(), None)
            }
            b'2' => {
                let parts: Vec<&str> = text.splitn(10, ' ').collect();
                let from = fields.next().map(|f| String::from_utf8_lossy(f).into_owned());
                if parts.len() < 10 {
                    continue;
                }
                (parts[1].as_bytes().to_vec(), parts[9].to_string(), from)
            }
            b'u' => {
                let parts: Vec<&str> = text.splitn(11, ' ').collect();
                if parts.len() < 11 {
                    continue;
                }
                status.conflicts.push(Entry {
                    path: parts[10].into(),
                    from: None,
                    letter: '!',
                    conflict: Some(conflict(parts[1].as_bytes())),
                });
                continue;
            }
            b'?' => {
                status.unstaged.push(Entry {
                    path: text[2..].into(),
                    from: None,
                    letter: 'U',
                    conflict: None,
                });
                continue;
            }
            _ => continue,
        };
        if xy.len() < 2 {
            continue;
        }
        if xy[0] != b'.' {
            status.staged.push(Entry {
                path: path.clone(),
                from: from.clone(),
                letter: letter(xy[0]),
                conflict: None,
            });
        }
        if xy[1] != b'.' {
            status.unstaged.push(Entry {
                path,
                // The worktree side of a staged rename has no old path.
                from: None,
                letter: letter(xy[1]),
                conflict: None,
            });
        }
    }
    let order = |a: &Entry, b: &Entry| workspace::natural_cmp(&a.path, &b.path);
    status.staged.sort_by(order);
    status.unstaged.sort_by(order);
    status.conflicts.sort_by(order);
    status
}

/// The repository's Git directory (a worktree has its own).
pub fn git_dir(path: &Path) -> Option<PathBuf> {
    let dot = path.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = fs::read_to_string(&dot).ok()?;
    let dir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    Some(if dir.is_absolute() { dir } else { path.join(dir) })
}

pub fn operation(git_dir: &Path) -> Option<Operation> {
    let number = |path: PathBuf| -> usize {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    if git_dir.join("rebase-merge").is_dir() {
        let dir = git_dir.join("rebase-merge");
        return Some(Operation::Rebase {
            step: number(dir.join("msgnum")),
            total: number(dir.join("end")),
        });
    }
    if git_dir.join("rebase-apply").is_dir() {
        let dir = git_dir.join("rebase-apply");
        return Some(Operation::Rebase {
            step: number(dir.join("next")),
            total: number(dir.join("last")),
        });
    }
    if git_dir.join("MERGE_HEAD").exists() {
        return Some(Operation::Merge);
    }
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Some(Operation::CherryPick);
    }
    if git_dir.join("REVERT_HEAD").exists() {
        return Some(Operation::Revert);
    }
    if git_dir.join("BISECT_LOG").exists() {
        return Some(Operation::Bisect);
    }
    None
}

/// Everything the Changes list needs for one repository.
pub fn status(path: &Path) -> Result<Status> {
    let bytes = read(
        path,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--show-stash",
            "--untracked-files=all",
        ],
    )?;
    let mut status = parse_status(&bytes);
    status.operation = git_dir(path).as_deref().and_then(operation);
    status.remotes = read_text(path, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    Ok(status)
}

/// The message Git prepared for a merge, revert or cherry-pick in progress.
pub fn prepared_message(path: &Path) -> Option<String> {
    let dir = git_dir(path)?;
    let text = fs::read_to_string(dir.join("MERGE_MSG")).ok()?;
    let message: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    let message = message.join("\n").trim().to_string();
    (!message.is_empty()).then_some(message)
}

// ---- Worktrees -----------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// Parses `git worktree list --porcelain -z`. The first entry is the main one.
pub fn parse_worktrees(bytes: &[u8]) -> Vec<Worktree> {
    let mut list = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in bytes.split(|b| *b == 0) {
        let text = String::from_utf8_lossy(field);
        if text.is_empty() {
            list.extend(current.take());
            continue;
        }
        let (key, value) = text.split_once(' ').unwrap_or((&text, ""));
        if key == "worktree" {
            list.extend(current.take());
            current = Some(Worktree {
                path: PathBuf::from(value),
                ..Default::default()
            });
            continue;
        }
        let Some(tree) = current.as_mut() else {
            continue;
        };
        match key {
            "HEAD" => tree.head = value.into(),
            "branch" => {
                tree.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).into())
            }
            "bare" => tree.bare = true,
            "detached" => tree.detached = true,
            "locked" => tree.locked = true,
            "prunable" => tree.prunable = true,
            _ => {}
        }
    }
    list.extend(current);
    list
}

pub fn worktrees(path: &Path) -> Result<Vec<Worktree>> {
    Ok(parse_worktrees(&read(
        path,
        &["worktree", "list", "--porcelain", "-z"],
    )?))
}

// ---- Refs ----------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    pub kind: RefKind,
    /// Short name, such as `main`, `origin/main` or `v1.0`.
    pub name: String,
    pub oid: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// The upstream branch was deleted on the remote.
    pub gone: bool,
    /// Committer time, seconds since the epoch.
    pub time: i64,
    pub subject: String,
    pub head: bool,
    /// Where the branch is checked out, when that is another worktree.
    pub worktree: Option<PathBuf>,
}

const REF_FORMAT: &str = "%(refname)%1f%(refname:short)%1f%(objectname)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(committerdate:unix)%1f%(contents:subject)%1f%(HEAD)%1f%(worktreepath)%1f%(symref)";

pub fn parse_refs(text: &str) -> Vec<Ref> {
    let mut refs = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\x1f').collect();
        if f.len() < 10 || !f[9].is_empty() {
            // Too short, or a symbolic ref such as origin/HEAD.
            continue;
        }
        let kind = if f[0].starts_with("refs/heads/") {
            RefKind::Local
        } else if f[0].starts_with("refs/remotes/") {
            RefKind::Remote
        } else if f[0].starts_with("refs/tags/") {
            RefKind::Tag
        } else {
            continue;
        };
        let mut ahead = 0;
        let mut behind = 0;
        for part in f[4].split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                behind = n.parse().unwrap_or(0);
            }
        }
        refs.push(Ref {
            kind,
            name: f[1].into(),
            oid: f[2].into(),
            upstream: (!f[3].is_empty()).then(|| f[3].into()),
            ahead,
            behind,
            gone: f[4] == "gone",
            time: f[5].parse().unwrap_or(0),
            subject: f[6].into(),
            head: f[7] == "*",
            worktree: (!f[8].is_empty()).then(|| PathBuf::from(f[8])),
        });
    }
    refs.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(b.head.cmp(&a.head))
            .then(b.time.cmp(&a.time))
    });
    refs
}

/// Local branches, remote branches and tags, the current branch first and
/// the rest newest first.
pub fn refs(path: &Path) -> Result<Vec<Ref>> {
    let format = format!("--format={REF_FORMAT}");
    Ok(parse_refs(&read_text(
        path,
        &["for-each-ref", &format, "refs/heads", "refs/remotes", "refs/tags"],
    )?))
}

// ---- Stashes ----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Stash {
    /// `stash@{0}`.
    pub name: String,
    pub oid: String,
    pub time: i64,
    pub message: String,
}

pub fn parse_stashes(bytes: &[u8]) -> Vec<Stash> {
    bytes
        .split(|b| *b == 0)
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let text = text.trim_matches('\n');
            let f: Vec<&str> = text.split('\x1f').collect();
            (f.len() >= 4).then(|| Stash {
                name: f[0].into(),
                oid: f[1].into(),
                time: f[2].parse().unwrap_or(0),
                message: f[3].into(),
            })
        })
        .collect()
}

pub fn stashes(path: &Path) -> Result<Vec<Stash>> {
    Ok(parse_stashes(&read(
        path,
        &["stash", "list", "-z", "--format=%gd%x1f%H%x1f%ct%x1f%gs"],
    )?))
}

// ---- History ------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Label {
    /// The branch HEAD points at.
    Head(String),
    Detached,
    Local(String),
    Remote(String),
    Tag(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub labels: Vec<Label>,
    pub subject: String,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(7)]
    }
}

/// Parses `%D` printed with `--decorate=full`.
pub fn parse_labels(text: &str) -> Vec<Label> {
    let mut labels = Vec::new();
    for part in text.split(", ").filter(|p| !p.is_empty()) {
        if let Some(branch) = part.strip_prefix("HEAD -> ") {
            let name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
            labels.push(Label::Head(name.into()));
        } else if part == "HEAD" {
            labels.push(Label::Detached);
        } else if let Some(tag) = part.strip_prefix("tag: ") {
            labels.push(Label::Tag(tag.strip_prefix("refs/tags/").unwrap_or(tag).into()));
        } else if let Some(name) = part.strip_prefix("refs/heads/") {
            labels.push(Label::Local(name.into()));
        } else if let Some(name) = part.strip_prefix("refs/remotes/")
            && !name.ends_with("/HEAD")
        {
            labels.push(Label::Remote(name.into()));
        }
    }
    labels
}

const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%ct%x1f%D%x1f%s";

pub fn parse_log(bytes: &[u8]) -> Vec<Commit> {
    bytes
        .split(|b| *b == 0)
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let text = text.trim_start_matches('\n');
            let f: Vec<&str> = text.splitn(7, '\x1f').collect();
            (f.len() == 7 && f[0].len() >= 7).then(|| Commit {
                hash: f[0].into(),
                parents: f[1].split(' ').filter(|p| !p.is_empty()).map(str::to_owned).collect(),
                author: f[2].into(),
                email: f[3].into(),
                time: f[4].parse().unwrap_or(0),
                labels: parse_labels(f[5]),
                subject: f[6].trim_end().into(),
            })
        })
        .collect()
}

/// A page of history in topological order. `all` covers every branch, tag
/// and remote; otherwise HEAD and its upstream.
pub fn log(path: &Path, all: bool, upstream: Option<&str>, skip: usize, count: usize) -> Result<Vec<Commit>> {
    let skip = format!("--skip={skip}");
    let count = format!("--max-count={count}");
    let mut args = vec![
        "log",
        "-z",
        "--topo-order",
        "--decorate=full",
        LOG_FORMAT,
        &skip,
        &count,
    ];
    if all {
        args.extend(["--branches", "--remotes", "--tags", "HEAD"]);
    } else {
        args.push("HEAD");
        if let Some(upstream) = upstream {
            args.push(upstream);
        }
    }
    args.push("--");
    match read(path, &args) {
        Ok(bytes) => Ok(parse_log(&bytes)),
        // A repository without commits has no history yet.
        Err(e) if e.to_string().contains("does not have any commits") => Ok(vec![]),
        Err(e) if e.to_string().contains("unknown revision") => Ok(vec![]),
        Err(e) => Err(e),
    }
}

/// Everything about one commit for its detail card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommitInfo {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub authored: i64,
    pub committer: String,
    pub committed: i64,
    pub labels: Vec<Label>,
    pub message: String,
}

pub fn commit_info(path: &Path, hash: &str) -> Result<CommitInfo> {
    let text = read_text(
        path,
        &[
            "show",
            "-s",
            "--decorate=full",
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%D%x1f%B",
            hash,
            "--",
        ],
    )?;
    let f: Vec<&str> = text.splitn(9, '\x1f').collect();
    if f.len() < 9 {
        bail!("Unexpected commit format");
    }
    Ok(CommitInfo {
        hash: f[0].into(),
        parents: f[1].split(' ').filter(|p| !p.is_empty()).map(str::to_owned).collect(),
        author: f[2].into(),
        email: f[3].into(),
        authored: f[4].parse().unwrap_or(0),
        committer: f[5].into(),
        committed: f[6].parse().unwrap_or(0),
        labels: parse_labels(f[7]),
        message: f[8].trim().into(),
    })
}

/// Commits on HEAD's upstream that HEAD lacks (incoming) and the reverse.
pub fn incoming_outgoing(path: &Path) -> (Vec<String>, Vec<String>) {
    let list = |range: &str| -> Vec<String> {
        read_text(path, &["rev-list", "--max-count=500", range, "--"])
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    };
    (list("HEAD..@{upstream}"), list("@{upstream}..HEAD"))
}

/// A web page for a commit or branch comparison when the remote is on a known
/// host (GitHub, GitLab or Bitbucket), from the remote's URL.
pub fn web_url(path: &Path, remote: &str) -> Option<String> {
    let url = read_text(path, &["remote", "get-url", remote]).ok()?;
    let url = url.trim().trim_end_matches(".git").trim_end_matches('/');
    let https = if let Some(rest) = url.strip_prefix("git@") {
        format!("https://{}", rest.replacen(':', "/", 1))
    } else if let Some(rest) = url.strip_prefix("ssh://git@") {
        format!("https://{rest}")
    } else if url.starts_with("https://") || url.starts_with("http://") {
        // Drop credentials a URL might carry.
        match url.split_once("://") {
            Some((scheme, rest)) => format!(
                "{scheme}://{}",
                rest.rsplit_once('@').map_or(rest, |(_, host)| host)
            ),
            None => url.to_string(),
        }
    } else {
        return None;
    };
    ["github.com", "gitlab.com", "bitbucket.org"]
        .iter()
        .any(|host| https.contains(host))
        .then_some(https)
}

/// "3 minutes ago"-style age of a Unix time.
pub fn age(time: i64) -> String {
    let seconds = (chrono::Utc::now().timestamp() - time).max(0);
    let (value, unit) = match seconds {
        s if s < 60 => return "now".into(),
        s if s < 3600 => (s / 60, "min"),
        s if s < 86_400 => (s / 3600, "hr"),
        s if s < 86_400 * 7 => (s / 86_400, "day"),
        s if s < 86_400 * 30 => (s / (86_400 * 7), "wk"),
        s if s < 86_400 * 365 => (s / (86_400 * 30), "mo"),
        s => (s / (86_400 * 365), "yr"),
    };
    format!("{value} {unit}{} ago", if value == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2_splits_staged_unstaged_renames_and_conflicts() {
        let raw = "# branch.oid 1234567890abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0# stash 3\0\
1 M. N... 100644 100644 100644 aaa bbb src/a b.rs\0\
1 .D N... 100644 100644 000000 aaa aaa gone.txt\0\
1 AM N... 000000 100644 100644 000 ccc new.rs\0\
2 R. N... 100644 100644 100644 aaa aaa R100 new name.rs\0old name.rs\0\
u UU N... 100644 100644 100644 100644 a b c both.rs\0\
? untracked file.txt\0";
        let status = parse_status(raw.as_bytes());
        assert_eq!(status.oid.as_deref(), Some("1234567890abcdef"));
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind, status.stashes), (2, 1, 3));
        let staged: Vec<_> = status.staged.iter().map(|e| (e.path.as_str(), e.letter)).collect();
        assert_eq!(staged, vec![("new name.rs", 'R'), ("new.rs", 'A'), ("src/a b.rs", 'M')]);
        assert_eq!(status.staged[0].from.as_deref(), Some("old name.rs"));
        let unstaged: Vec<_> = status.unstaged.iter().map(|e| (e.path.as_str(), e.letter)).collect();
        assert_eq!(
            unstaged,
            vec![("gone.txt", 'D'), ("new.rs", 'M'), ("untracked file.txt", 'U')]
        );
        assert_eq!(status.conflicts[0].conflict, Some("both modified"));
        assert_eq!(status.changes(), 7);
    }

    #[test]
    fn unborn_and_detached_heads() {
        let unborn = parse_status(b"# branch.oid (initial)\0# branch.head main\0");
        assert_eq!((unborn.oid, unborn.branch.as_deref()), (None, Some("main")));
        let detached = parse_status(b"# branch.oid abcdef1234\0# branch.head (detached)\0");
        assert_eq!(detached.branch, None);
        assert_eq!(detached.head_label(), "abcdef1 (detached)");
    }

    #[test]
    fn worktree_list_porcelain() {
        let raw = "worktree C:/w/cady\0HEAD aaa\0branch refs/heads/main\0\0\
worktree C:/w/.worktree/cady-x\0HEAD bbb\0branch refs/heads/feature/x\0locked\0\0\
worktree C:/w/gone\0HEAD ccc\0detached\0prunable gitdir file points to non-existent location\0\0";
        let list = parse_worktrees(raw.as_bytes());
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].branch.as_deref(), Some("feature/x"));
        assert!(list[1].locked);
        assert!(list[2].detached && list[2].prunable);
        assert_eq!(list[0].path, PathBuf::from("C:/w/cady"));
    }

    #[test]
    fn refs_skip_symbolic_and_sort_current_first() {
        let row = |fields: [&str; 10]| fields.join("\x1f");
        let text = [
            row(["refs/heads/old", "old", "a1", "", "", "100", "old work", " ", "", ""]),
            row(["refs/heads/main", "main", "b2", "origin/main", "ahead 1, behind 2", "50", "main work", "*", "C:/w/cady", ""]),
            row(["refs/remotes/origin/HEAD", "origin", "b2", "", "", "50", "x", " ", "", "refs/remotes/origin/main"]),
            row(["refs/remotes/origin/main", "origin/main", "c3", "", "", "70", "remote", " ", "", ""]),
            row(["refs/tags/v1", "v1", "d4", "", "", "10", "tag", " ", "", ""]),
            row(["refs/heads/gone", "gone", "e5", "origin/gone", "gone", "200", "g", " ", "C:/w/.worktree/g", ""]),
        ]
        .join("\n");
        let refs = parse_refs(&text);
        let names: Vec<_> = refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["main", "gone", "old", "origin/main", "v1"]);
        assert_eq!((refs[0].ahead, refs[0].behind), (1, 2));
        assert!(refs[0].head);
        assert!(refs[1].gone);
        assert_eq!(refs[1].worktree, Some(PathBuf::from("C:/w/.worktree/g")));
    }

    #[test]
    fn log_records_and_full_decorations() {
        let raw = "abcdef1234567\x1fparent1 parent2\x1fAda\x1fada@example.com\x1f1700000000\x1fHEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD, tag: refs/tags/v1, refs/heads/feature/x\x1fMerge branch 'x'\0\nfedcba7654321\x1f\x1fAda\x1fada@example.com\x1f1600000000\x1f\x1fFirst";
        let log = parse_log(raw.as_bytes());
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].parents.len(), 2);
        assert_eq!(
            log[0].labels,
            vec![
                Label::Head("main".into()),
                Label::Remote("origin/main".into()),
                Label::Tag("v1".into()),
                Label::Local("feature/x".into()),
            ]
        );
        assert!(log[1].parents.is_empty());
        assert_eq!(log[1].subject, "First");
    }

    #[test]
    fn stash_list_records() {
        let raw = b"stash@{0}\x1fabc\x1f1700000000\x1fWIP on main: 123 message\0\nstash@{1}\x1fdef\x1f1600000000\x1fOn main: saved";
        let list = parse_stashes(raw);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].name, "stash@{1}");
        assert_eq!(list[1].message, "On main: saved");
    }

    #[test]
    fn repositories_list_nested_repos_and_their_worktrees() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let init = |path: &Path| -> Result<()> {
            fs::create_dir_all(path)?;
            workspace::git_text(path, &["init", "-q", "-b", "main"])?;
            workspace::git_text(path, &["config", "user.email", "t@example.com"])?;
            workspace::git_text(path, &["config", "user.name", "T"])?;
            workspace::git_text(path, &["commit", "-q", "--allow-empty", "-m", "first"])?;
            Ok(())
        };
        init(root)?;
        fs::write(root.join(".gitignore"), "/api/\n/.worktree/\n")?;
        init(&root.join("api"))?;
        workspace::git_text(
            &root.join("api"),
            &["worktree", "add", "-q", "-b", "feature", "../.worktree/api-feature"],
        )?;
        let repos = repositories(root);
        let names: Vec<_> = repos
            .iter()
            .map(|r| (r.name.as_str(), r.primary, r.is_worktree()))
            .collect();
        assert_eq!(
            names,
            vec![
                (project::folder_name(root).as_str(), true, false),
                ("api", false, false),
                ("api-feature", false, true),
            ]
        );
        // A terminal inside the worktree sees the same repositories.
        let from_worktree = repositories(&root.join(".worktree/api-feature"));
        assert!(from_worktree.iter().any(|r| r.name == "api-feature"));
        let status = status(&root.join(".worktree/api-feature"))?;
        assert_eq!(status.branch.as_deref(), Some("feature"));
        Ok(())
    }

    #[test]
    fn web_urls_from_remote_forms() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        workspace::git_text(root, &["init", "-q"])?;
        for (i, url) in [
            "git@github.com:usecady/cady.git",
            "https://token@github.com/usecady/cady.git",
            "ssh://git@gitlab.com/group/repo.git",
        ]
        .iter()
        .enumerate()
        {
            workspace::git_text(root, &["remote", "add", &format!("r{i}"), url])?;
        }
        assert_eq!(web_url(root, "r0").as_deref(), Some("https://github.com/usecady/cady"));
        assert_eq!(web_url(root, "r1").as_deref(), Some("https://github.com/usecady/cady"));
        assert_eq!(web_url(root, "r2").as_deref(), Some("https://gitlab.com/group/repo"));
        Ok(())
    }
}
