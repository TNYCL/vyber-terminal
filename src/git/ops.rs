//! Git commands that change a repository. They run one at a time per
//! repository, never wait for an editor or a terminal prompt, and each one is
//! recorded in the Git output log. Files Vyber overwrites or deletes keep a
//! recovery copy first.
use crate::workspace;
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
pub struct Failure {
    pub message: String,
    /// The remote rejected or could not ask for credentials.
    pub auth: bool,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            auth: false,
        }
    }
}

pub type Outcome<T = String> = Result<T, Failure>;

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: String,
    pub repo: String,
    pub command: String,
    pub ok: bool,
    pub output: String,
}

static LOG: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
static LOCKS: Mutex<Option<HashMap<PathBuf, Arc<Mutex<()>>>>> = Mutex::new(None);

/// The most recent commands, oldest first.
pub fn log() -> Vec<LogEntry> {
    LOG.lock().unwrap().iter().cloned().collect()
}

fn record(repo: &Path, command: String, ok: bool, output: &str) {
    let mut output = output.trim().to_string();
    if output.len() > 4000 {
        let mut cut = 4000;
        while !output.is_char_boundary(cut) {
            cut -= 1;
        }
        output.truncate(cut);
        output.push_str(" …");
    }
    let mut log = LOG.lock().unwrap();
    log.push_back(LogEntry {
        time: chrono::Local::now().format("%H:%M:%S").to_string(),
        repo: crate::project::folder_name(repo),
        command,
        ok,
        output,
    });
    while log.len() > 400 {
        log.pop_front();
    }
}

fn lock(repo: &Path) -> Arc<Mutex<()>> {
    let mut locks = LOCKS.lock().unwrap();
    locks
        .get_or_insert_with(HashMap::new)
        .entry(repo.to_path_buf())
        .or_default()
        .clone()
}

#[derive(Clone, Copy, PartialEq)]
pub enum Access {
    Local,
    /// Talks to a remote; `true` lets a credential manager show its window.
    Network(bool),
}

const AUTH_HINTS: [&str; 9] = [
    "authentication failed",
    "could not read username",
    "could not read password",
    "terminal prompts disabled",
    "permission denied (publickey",
    "invalid username or password",
    "access denied",
    "the requested url returned error: 403",
    "host key verification failed",
];

fn describe(args: &[&str], paths: usize) -> String {
    let mut text = String::from("git");
    for arg in args {
        text.push(' ');
        if arg.contains(' ') {
            text.push('"');
            text.push_str(arg);
            text.push('"');
        } else {
            text.push_str(arg);
        }
    }
    if paths > 0 {
        text.push_str(&format!(" ({paths} {})", if paths == 1 { "path" } else { "paths" }));
    }
    text
}

/// Runs one Git command in `repo` and returns its output. `input` is fed to
/// standard input; `paths` only tells the log how many paths it carries.
fn git(repo: &Path, args: &[&str], access: Access, input: Option<&[u8]>, paths: usize) -> Outcome {
    let guard = lock(repo);
    let _held = guard.lock().unwrap();
    let mut command = workspace::command("git");
    command
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false"])
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", ":")
        .env("GIT_SEQUENCE_EDITOR", ":")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if access == Access::Network(false) {
        command
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let described = describe(args, paths);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            let message = format!("Git could not be started: {e}");
            record(repo, described, false, &message);
            return Err(Failure::new(message));
        }
    };
    let writer = input.map(|bytes| {
        let mut stdin = child.stdin.take();
        let bytes = bytes.to_vec();
        std::thread::spawn(move || {
            if let Some(stdin) = stdin.as_mut() {
                let _ = stdin.write_all(&bytes);
            }
        })
    });
    let out = child.wait_with_output()?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{}\n{}", stdout.trim(), stderr.trim()).trim().to_string();
    record(repo, described, out.status.success(), &combined);
    if out.status.success() {
        Ok(stdout)
    } else {
        let lower = combined.to_lowercase();
        Err(Failure {
            auth: matches!(access, Access::Network(_)) && AUTH_HINTS.iter().any(|h| lower.contains(h)),
            // Conflict reports go to standard output, errors to standard error.
            message: combined,
        })
    }
}

fn local(repo: &Path, args: &[&str]) -> Outcome {
    git(repo, args, Access::Local, None, 0)
}

/// A command that takes its pathspecs from standard input, so any number of
/// paths fits and none is read as a pattern. (`GIT_LITERAL_PATHSPECS` would
/// do the same for every command, but it also stops `stash -u` from
/// removing the untracked files it saved.)
fn with_paths(repo: &Path, args: &[&str], paths: &[String]) -> Outcome {
    let mut full = args.to_vec();
    full.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
    let mut input = Vec::new();
    for path in paths {
        input.extend(b":(literal)");
        input.extend(path.as_bytes());
        input.push(0);
    }
    git(repo, &full, Access::Local, Some(&input), paths.len())
}

fn has_head(repo: &Path) -> bool {
    super::read(repo, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]).is_ok()
}

// ---- Changes ------------------------------------------------------------------

pub fn stage(repo: &Path, paths: &[String]) -> Outcome {
    with_paths(repo, &["add", "-A"], paths)
}

pub fn stage_all(repo: &Path) -> Outcome {
    local(repo, &["add", "-A"])
}

pub fn unstage(repo: &Path, paths: &[String]) -> Outcome {
    if has_head(repo) {
        with_paths(repo, &["restore", "--staged"], paths)
    } else {
        with_paths(repo, &["rm", "--cached", "-r", "-q"], paths)
    }
}

pub fn unstage_all(repo: &Path) -> Outcome {
    if has_head(repo) {
        local(repo, &["reset", "-q"])
    } else {
        local(repo, &["rm", "--cached", "-r", "-q", "."])
    }
}

/// Puts tracked files back to their staged (or committed) content and deletes
/// untracked files. Every file keeps a recovery copy first.
pub fn discard(repo: &Path, tracked: &[String], untracked: &[String]) -> Outcome {
    for path in tracked.iter().chain(untracked) {
        let full = workspace::safe_path(repo, path)?;
        if full.is_file() {
            workspace::keep_recovery(&full, Some(&std::fs::read(&full)?))?;
        }
    }
    if !tracked.is_empty() {
        with_paths(repo, &["restore", "--worktree"], tracked)?;
    }
    for path in untracked {
        let full = workspace::safe_path(repo, path)?;
        if full.is_file() {
            std::fs::remove_file(&full)?;
        }
        record(repo, format!("delete {path}"), true, "untracked file removed; recovery copy kept");
    }
    Ok(String::new())
}

/// Resolves a conflicted file with one side's version (`ours` is the branch
/// being merged into) and marks it resolved. A side that deleted the file
/// deletes it.
pub fn take_side(repo: &Path, path: &str, ours: bool) -> Outcome {
    let side = if ours { "--ours" } else { "--theirs" };
    let paths = [path.to_string()];
    match with_paths(repo, &["checkout", side], &paths) {
        Ok(_) => with_paths(repo, &["add", "-A"], &paths),
        Err(e) if e.message.contains("does not have") => with_paths(repo, &["rm", "-q"], &paths),
        Err(e) => Err(e),
    }
}

/// The index content of `path`, or `None` when the index has no such file.
pub fn index_bytes(repo: &Path, path: &str) -> Outcome<Option<Vec<u8>>> {
    let listed = super::read(repo, &["ls-files", "-s", "-z", "--", &format!(":(literal){path}")])?;
    if listed.is_empty() {
        return Ok(None);
    }
    Ok(Some(super::read(repo, &["cat-file", "blob", &format!(":{path}")])?))
}

/// Writes `bytes` as the staged content of `path`, if the index still holds
/// `expected`. Git's clean filters apply as with `git add`.
pub fn write_index(repo: &Path, path: &str, expected: Option<&[u8]>, bytes: &[u8]) -> Outcome {
    if index_bytes(repo, path)?.as_deref() != expected {
        return Err(Failure::new(format!(
            "{path} changed in the index since this diff was read. Refresh and try again."
        )));
    }
    let mode = super::read(repo, &["ls-files", "-s", "--", &format!(":(literal){path}")])
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out).split(' ').next().map(str::to_owned))
        .filter(|m| m.len() == 6)
        .unwrap_or_else(|| "100644".into());
    let path_arg = format!("--path={path}");
    let hash = git(
        repo,
        &["hash-object", "-w", "--stdin", &path_arg],
        Access::Local,
        Some(bytes),
        0,
    )?;
    let info = format!("{mode},{},{path}", hash.trim());
    local(repo, &["update-index", "--add", "--cacheinfo", &info])
}

// ---- Commits ------------------------------------------------------------------

fn message_file(message: &str) -> Outcome<PathBuf> {
    let dir = workspace::data_dir().join("tmp");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "commit-{}-{}.txt",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&path, message.as_bytes())?;
    Ok(path)
}

/// Commits the index. An empty message with `amend` keeps the last message.
pub fn commit(repo: &Path, message: &str, amend: bool) -> Outcome {
    let message = message.trim();
    let file = (!message.is_empty()).then(|| message_file(message)).transpose()?;
    let file_arg = file.as_ref().map(|f| f.to_string_lossy().into_owned());
    let mut args = vec!["commit"];
    match &file_arg {
        // Lines starting with `#` are part of the message, as typed.
        Some(file) => args.extend(["--cleanup=whitespace", "-F", file.as_str()]),
        None => args.push("--no-edit"),
    }
    if amend {
        args.push("--amend");
    }
    let result = local(repo, &args);
    if let Some(file) = file {
        let _ = std::fs::remove_file(file);
    }
    result
}

/// Undoes the last commit and keeps its changes staged. Returns its message.
pub fn undo_commit(repo: &Path) -> Outcome {
    let message = super::read(repo, &["log", "-1", "--format=%B"])
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .unwrap_or_default();
    if super::read(repo, &["rev-parse", "--verify", "--quiet", "HEAD~1"]).is_ok() {
        local(repo, &["reset", "--soft", "HEAD~1"])?;
    } else {
        // The first commit: leave the branch unborn with everything staged.
        local(repo, &["update-ref", "-d", "HEAD"])?;
    }
    Ok(message)
}

// ---- Branches -----------------------------------------------------------------

pub fn switch(repo: &Path, target: &str, detach: bool, discard_changes: bool) -> Outcome {
    let mut args = vec!["switch"];
    if detach {
        args.push("--detach");
    }
    if discard_changes {
        args.push("--discard-changes");
    }
    args.push(target);
    local(repo, &args)
}

/// Checks out a remote branch as a new local branch that tracks it.
pub fn switch_tracking(repo: &Path, remote_branch: &str) -> Outcome {
    local(repo, &["switch", "--track", remote_branch])
}

pub fn create_branch(repo: &Path, name: &str, from: Option<&str>, checkout: bool) -> Outcome {
    let mut args = if checkout {
        vec!["switch", "-c", name]
    } else {
        vec!["branch", name]
    };
    if let Some(from) = from {
        args.push(from);
    }
    local(repo, &args)
}

pub fn rename_branch(repo: &Path, old: &str, new: &str) -> Outcome {
    local(repo, &["branch", "-m", old, new])
}

pub fn delete_branch(repo: &Path, name: &str, force: bool) -> Outcome {
    local(repo, &["branch", if force { "-D" } else { "-d" }, name])
}

pub fn merge(repo: &Path, target: &str) -> Outcome {
    local(repo, &["merge", "--no-edit", target])
}

pub fn rebase(repo: &Path, onto: &str) -> Outcome {
    local(repo, &["rebase", onto])
}

/// Continues, aborts or skips the merge, rebase, cherry-pick or revert in
/// progress.
pub fn sequence(repo: &Path, operation: super::Operation, action: &str) -> Outcome {
    let command = match operation {
        super::Operation::Merge => "merge",
        super::Operation::Rebase { .. } => "rebase",
        super::Operation::CherryPick => "cherry-pick",
        super::Operation::Revert => "revert",
        super::Operation::Bisect => return local(repo, &["bisect", "reset"]),
    };
    let flag = format!("--{action}");
    if command == "merge" && action == "continue" {
        return local(repo, &["commit", "--no-edit"]);
    }
    local(repo, &[command, &flag])
}

pub fn cherry_pick(repo: &Path, hash: &str, merge: bool) -> Outcome {
    if merge {
        local(repo, &["cherry-pick", "-m", "1", hash])
    } else {
        local(repo, &["cherry-pick", hash])
    }
}

pub fn revert_commit(repo: &Path, hash: &str, merge: bool) -> Outcome {
    if merge {
        local(repo, &["revert", "--no-edit", "-m", "1", hash])
    } else {
        local(repo, &["revert", "--no-edit", hash])
    }
}

/// `git reset --soft|--mixed|--hard`. A hard reset first keeps a recovery
/// copy of every changed file it would overwrite.
pub fn reset(repo: &Path, target: &str, mode: &str) -> Outcome {
    if mode == "hard" {
        let status = super::status(repo)?;
        for entry in status.staged.iter().chain(&status.unstaged) {
            let full = workspace::safe_path(repo, &entry.path)?;
            if full.is_file() {
                workspace::keep_recovery(&full, Some(&std::fs::read(&full)?))?;
            }
        }
    }
    local(repo, &["reset", &format!("--{mode}"), target])
}

pub fn create_tag(repo: &Path, name: &str, target: Option<&str>, message: Option<&str>) -> Outcome {
    let mut args = vec!["tag"];
    if let Some(message) = message.filter(|m| !m.trim().is_empty()) {
        args.extend(["-a", name, "-m", message]);
    } else {
        args.push(name);
    }
    if let Some(target) = target {
        args.push(target);
    }
    local(repo, &args)
}

pub fn delete_tag(repo: &Path, name: &str) -> Outcome {
    local(repo, &["tag", "-d", name])
}

// ---- Stashes ------------------------------------------------------------------

pub fn stash(repo: &Path, message: &str, untracked: bool, staged_only: bool) -> Outcome {
    let mut args = vec!["stash", "push"];
    if untracked {
        args.push("--include-untracked");
    }
    if staged_only {
        args.push("--staged");
    }
    if !message.trim().is_empty() {
        args.extend(["-m", message.trim()]);
    }
    local(repo, &args)
}

pub fn stash_action(repo: &Path, action: &str, name: &str) -> Outcome {
    local(repo, &["stash", action, name])
}

pub fn stash_clear(repo: &Path) -> Outcome {
    local(repo, &["stash", "clear"])
}

// ---- Remotes ------------------------------------------------------------------

pub fn fetch(repo: &Path, interactive: bool) -> Outcome {
    git(repo, &["fetch", "--all", "--prune"], Access::Network(interactive), None, 0)
}

pub fn pull(repo: &Path, rebase: bool) -> Outcome {
    let args: &[&str] = if rebase {
        &["pull", "--rebase"]
    } else {
        &["pull"]
    };
    git(repo, args, Access::Network(true), None, 0)
}

/// Pushes the current branch. `publish` sets its upstream on that remote.
pub fn push(repo: &Path, publish: Option<(&str, &str)>, force: bool) -> Outcome {
    let mut args = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    if let Some((remote, branch)) = publish {
        args.extend(["-u", remote, branch]);
    }
    git(repo, &args, Access::Network(true), None, 0)
}

pub fn push_tags(repo: &Path, remote: &str) -> Outcome {
    git(repo, &["push", remote, "--tags"], Access::Network(true), None, 0)
}

pub fn delete_remote_branch(repo: &Path, remote: &str, branch: &str) -> Outcome {
    git(repo, &["push", remote, "--delete", branch], Access::Network(true), None, 0)
}

pub fn add_remote(repo: &Path, name: &str, url: &str) -> Outcome {
    local(repo, &["remote", "add", name, url])
}

pub fn remove_remote(repo: &Path, name: &str) -> Outcome {
    local(repo, &["remote", "remove", name])
}

// ---- Worktrees and repositories --------------------------------------------------

/// Adds a worktree at `path` on a new branch (from `base`) or an existing one.
pub fn add_worktree(repo: &Path, path: &Path, branch: &str, new: bool, base: Option<&str>) -> Outcome {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "add"];
    if new {
        args.extend(["-b", branch, path.as_str()]);
        if let Some(base) = base {
            args.push(base);
        }
    } else {
        args.extend([path.as_str(), branch]);
    }
    local(repo, &args)
}

pub fn remove_worktree(repo: &Path, path: &Path, force: bool) -> Outcome {
    let path = path.to_string_lossy().into_owned();
    if force {
        local(repo, &["worktree", "remove", "--force", &path])
    } else {
        local(repo, &["worktree", "remove", &path])
    }
}

pub fn prune_worktrees(repo: &Path) -> Outcome {
    local(repo, &["worktree", "prune"])
}

pub fn init(path: &Path) -> Outcome {
    local(path, &["init"])
}

/// Where a new worktree of `repo` on `branch` goes: `<primary>/.worktree/`
/// when the project's primary folder ignores that folder (or is not a
/// repository), otherwise a `<name>.worktrees` folder beside it.
pub fn worktree_location(repo: &Path, branch: &str) -> PathBuf {
    let slug: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect();
    let name = format!("{}-{}", crate::project::folder_name(repo), slug.trim_matches('-'));
    let base = crate::project::for_path(repo)
        .and_then(|p| p.primary().map(Path::to_path_buf))
        .unwrap_or_else(|| repo.to_path_buf());
    let ignored = !crate::project::is_repository(&base)
        || super::read(&base, &["check-ignore", "-q", ".worktree/x"]).is_ok();
    if ignored {
        base.join(".worktree").join(name)
    } else {
        let parent = base.parent().map(Path::to_path_buf).unwrap_or_else(|| base.clone());
        parent
            .join(format!("{}.worktrees", crate::project::folder_name(&base)))
            .join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{self, Operation};
    use std::fs;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "core.autocrlf", "false"],
            &["config", "user.email", "t@example.com"],
            &["config", "user.name", "T"],
        ] {
            workspace::git_text(root, args).unwrap();
        }
        dir
    }

    #[test]
    fn stage_unstage_commit_amend_and_undo() -> Outcome<()> {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("a [1].txt"), "one\n")?;
        fs::write(root.join("b.txt"), "two\n")?;
        stage(root, &["a [1].txt".into()])?;
        let status = git::status(root)?;
        assert_eq!(status.staged.len(), 1);
        assert_eq!(status.staged[0].path, "a [1].txt");
        // Unstaging before the first commit uses `rm --cached`.
        unstage(root, &["a [1].txt".into()])?;
        assert!(git::status(root)?.staged.is_empty());
        stage_all(root)?;
        commit(root, "First\n\nBody line", false)?;
        let status = git::status(root)?;
        assert!(status.oid.is_some() && status.changes() == 0);
        fs::write(root.join("b.txt"), "two\nmore\n")?;
        stage_all(root)?;
        commit(root, "", true)?;
        let log = git::log(root, false, None, 0, 10)?;
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].subject, "First");
        let message = undo_commit(root)?;
        assert_eq!(message, "First\n\nBody line");
        let status = git::status(root)?;
        assert_eq!(status.oid, None);
        assert_eq!(status.staged.len(), 2);
        Ok(())
    }

    #[test]
    fn hunks_are_written_to_the_index_only_when_it_is_unchanged() -> Outcome<()> {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("f.txt"), "a\nb\nc\n")?;
        stage_all(root)?;
        commit(root, "base", false)?;
        fs::write(root.join("f.txt"), "a\nB\nc\nd\n")?;
        let index = index_bytes(root, "f.txt")?.unwrap();
        let worktree = fs::read(root.join("f.txt"))?;
        let diff = crate::changeset::file_diff(
            "f.txt",
            crate::changeset::Blob::Bytes(index.clone()),
            crate::changeset::Blob::Bytes(worktree.clone()),
            None,
        );
        assert_eq!(diff.hunks.len(), 2);
        let staged = crate::changeset::apply_hunk(&index, &worktree, &diff.hunks[1])?;
        write_index(root, "f.txt", Some(&index), &staged)?;
        assert_eq!(index_bytes(root, "f.txt")?.unwrap(), b"a\nb\nc\nd\n");
        // A stale expectation is refused.
        assert!(write_index(root, "f.txt", Some(&index), &staged).is_err());
        let status = git::status(root)?;
        assert_eq!((status.staged.len(), status.unstaged.len()), (1, 1));
        Ok(())
    }

    #[test]
    fn discard_keeps_recovery_and_removes_untracked() -> Outcome<()> {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("t.txt"), "kept\n")?;
        stage_all(root)?;
        commit(root, "base", false)?;
        fs::write(root.join("t.txt"), "edited\n")?;
        fs::write(root.join("new.txt"), "new\n")?;
        discard(root, &["t.txt".into()], &["new.txt".into()])?;
        assert_eq!(fs::read_to_string(root.join("t.txt"))?, "kept\n");
        assert!(!root.join("new.txt").exists());
        Ok(())
    }

    #[test]
    fn branches_merge_conflict_and_abort() -> Outcome<()> {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("f.txt"), "base\n")?;
        stage_all(root)?;
        commit(root, "base", false)?;
        create_branch(root, "feature", None, true)?;
        fs::write(root.join("f.txt"), "feature\n")?;
        stage_all(root)?;
        commit(root, "feature change", false)?;
        switch(root, "main", false, false)?;
        fs::write(root.join("f.txt"), "main\n")?;
        stage_all(root)?;
        commit(root, "main change", false)?;
        assert!(merge(root, "feature").is_err());
        let status = git::status(root)?;
        assert_eq!(status.operation, Some(Operation::Merge));
        assert_eq!(status.conflicts.len(), 1);
        sequence(root, Operation::Merge, "abort")?;
        assert_eq!(git::status(root)?.operation, None);
        let refs = git::refs(root)?;
        assert!(refs.iter().any(|r| r.name == "feature" && !r.head));
        rename_branch(root, "feature", "feature-2")?;
        assert!(delete_branch(root, "feature-2", false).is_err());
        delete_branch(root, "feature-2", true)?;
        Ok(())
    }

    #[test]
    fn switching_with_conflicting_changes_fails_and_stash_moves_them() -> Outcome<()> {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("f.txt"), "base\n")?;
        stage_all(root)?;
        commit(root, "base", false)?;
        create_branch(root, "other", None, true)?;
        fs::write(root.join("f.txt"), "other\n")?;
        stage_all(root)?;
        commit(root, "other", false)?;
        switch(root, "main", false, false)?;
        fs::write(root.join("f.txt"), "local edit\n")?;
        fs::write(root.join("untracked.txt"), "new\n")?;
        assert!(switch(root, "other", false, false).is_err());
        stash(root, "vyber: before switching", true, false)?;
        assert!(!root.join("untracked.txt").exists());
        switch(root, "other", false, false)?;
        assert_eq!(git::stashes(root)?.len(), 1);
        assert_eq!(git::status(root)?.stashes, 1);
        switch(root, "main", false, false)?;
        stash_action(root, "pop", "stash@{0}")?;
        assert_eq!(fs::read_to_string(root.join("f.txt"))?, "local edit\n");
        Ok(())
    }

    #[test]
    fn push_pull_and_publish_with_a_bare_remote() -> Outcome<()> {
        let remote = tempfile::tempdir()?;
        workspace::git_text(remote.path(), &["init", "-q", "--bare", "-b", "main"])?;
        let dir = repo();
        let root = dir.path();
        let url = remote.path().to_string_lossy().into_owned();
        add_remote(root, "origin", &url)?;
        fs::write(root.join("f.txt"), "x\n")?;
        stage_all(root)?;
        commit(root, "first", false)?;
        push(root, Some(("origin", "main")), false)?;
        let status = git::status(root)?;
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.remotes, vec!["origin".to_string()]);
        fs::write(root.join("f.txt"), "y\n")?;
        stage_all(root)?;
        commit(root, "second", false)?;
        assert_eq!(git::status(root)?.ahead, 1);
        push(root, None, false)?;
        fetch(root, false)?;
        pull(root, false)?;
        assert_eq!(git::status(root)?.ahead, 0);
        Ok(())
    }

    #[test]
    fn worktrees_are_added_listed_and_removed() -> Outcome<()> {
        let dir = repo();
        let root = dir.path().join("main");
        fs::create_dir_all(&root)?;
        workspace::git_text(&root, &["init", "-q", "-b", "main"])?;
        workspace::git_text(&root, &["config", "user.email", "t@example.com"])?;
        workspace::git_text(&root, &["config", "user.name", "T"])?;
        workspace::git_text(&root, &["commit", "-q", "--allow-empty", "-m", "first"])?;
        let place = dir.path().join(".worktree").join("main-feature");
        add_worktree(&root, &place, "feature", true, None)?;
        let list = git::worktrees(&root)?;
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].branch.as_deref(), Some("feature"));
        let refs = git::refs(&root)?;
        let feature = refs.iter().find(|r| r.name == "feature").unwrap();
        assert!(feature.worktree.is_some());
        remove_worktree(&root, &place, false)?;
        assert_eq!(git::worktrees(&root)?.len(), 1);
        Ok(())
    }

    #[test]
    fn every_command_is_logged() -> Outcome<()> {
        let dir = repo();
        let before = log().len();
        assert!(switch(dir.path(), "does-not-exist", false, false).is_err());
        let entries = log();
        assert!(entries.len() > before);
        let last = entries.iter().rev().find(|e| e.command.contains("does-not-exist")).unwrap();
        assert!(!last.ok);
        assert!(!last.output.is_empty());
        Ok(())
    }
}
