//! The Git view's state and actions: every repository of the terminal's
//! project with its changes, commit box and history, laid out like Cursor's
//! source control. Git work runs on background threads; results come back as
//! messages through [`Browser::poll`].
use super::{Browser, BrowserEvent, Message, View};
use crate::{
    changeset::Source,
    git::{self, Commit, CommitInfo, Operation, Ref, RefKind, Repo, Stash, Status, graph, ops},
    project,
    tasks::matches_root,
    workspace,
};
use gpui::{prelude::*, *};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

pub(super) const SIDEBAR: usize = 2;
const STATUS_EVERY: Duration = Duration::from_millis(2500);
const FETCH_EVERY: Duration = Duration::from_secs(5 * 60);
pub(super) const GRAPH_PAGE: usize = 200;
/// Files shown per group before "… more".
pub(super) const GROUP_LIMIT: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Group {
    Merge,
    Staged,
    Changes,
}

pub(super) struct RepoView {
    pub repo: Repo,
    pub status: Option<Result<Status, String>>,
}

impl RepoView {
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref().and_then(|s| s.as_ref().ok())
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Detail {
    Diff,
    Output,
}

pub(super) struct GraphState {
    pub repo: Option<PathBuf>,
    pub all: bool,
    pub commits: Vec<Commit>,
    pub rows: Vec<graph::Row>,
    pub loading: bool,
    pub more: bool,
    pub generation: u64,
    pub selected: Option<String>,
    pub incoming: HashSet<String>,
    pub outgoing: HashSet<String>,
    pub error: Option<String>,
    pub scroll: UniformListScrollHandle,
}

/// What a finished Git command leads to.
pub(super) enum After {
    Nothing,
    Notice(String),
    /// The commit box of the repository is emptied, then maybe `next` runs.
    Committed(Option<GitAction>),
    /// The commit box gets this text (after undoing a commit).
    Message(String),
    Ask(Confirm),
}

/// How to handle a failed Git command.
#[derive(Clone)]
pub(super) enum OnError {
    Show,
    /// Offer to stash, carry or discard local changes and switch anyway.
    Checkout(String, CheckoutKind),
    /// Offer `branch -D`.
    DeleteBranch(String),
    /// Conflicts are expected: refresh and point at them.
    Conflicts,
    /// Background work: the output log has it.
    Quiet,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum CheckoutKind {
    Local,
    Remote,
    Detached,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum CheckoutMode {
    Normal,
    Stash,
    Migrate,
    Force,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Style {
    Primary,
    Danger,
    Plain,
}

#[derive(Clone)]
pub(super) struct Button {
    pub label: String,
    pub style: Style,
    pub action: Option<GitAction>,
}

impl Button {
    pub fn new(label: impl Into<String>, style: Style, action: Option<GitAction>) -> Self {
        Self {
            label: label.into(),
            style,
            action,
        }
    }
    pub fn cancel() -> Self {
        Self::new("Cancel", Style::Plain, None)
    }
}

#[derive(Clone)]
pub(super) struct Confirm {
    pub title: String,
    pub message: String,
    /// Command output, shown in a monospaced box.
    pub detail: Option<String>,
    pub buttons: Vec<Button>,
}

/// Everything the Git view can do, from menus, buttons, pickers and dialogs.
/// Actions without a repository apply to the selected one.
#[derive(Clone, Debug)]
pub(super) enum GitAction {
    Refresh,
    SelectRepo(PathBuf),
    SelectCommit(String),
    ShowChanges(PathBuf, Option<String>, bool),
    Fetch,
    /// Pull (with rebase), after the running-agent warning.
    Pull(bool, bool),
    /// Push (forced with lease), after its confirmation.
    Push(bool, bool),
    Sync(bool),
    Publish(Option<String>),
    Commit { amend: bool, then: Option<Box<GitAction>>, stage_all: bool },
    UndoCommit(bool),
    StageAll(PathBuf),
    UnstageAll(PathBuf),
    DiscardAll(PathBuf, bool),
    Stage(PathBuf, Vec<String>),
    Unstage(PathBuf, Vec<String>),
    Discard { repo: PathBuf, tracked: Vec<String>, untracked: Vec<String>, confirmed: bool },
    TakeSide(PathBuf, String, bool),
    Ask(Ask),
    Checkout { target: String, kind: CheckoutKind, mode: CheckoutMode, confirmed: bool },
    CreateBranch { name: String, from: Option<String> },
    RenameBranch(String, String),
    DeleteBranch(String, bool),
    DeleteRemoteBranch(String, bool),
    Merge(String),
    Rebase(String, bool),
    Sequence(Operation, &'static str),
    CherryPick(String),
    RevertCommit(String),
    Reset { target: String, mode: &'static str, confirmed: bool },
    CreateTag { name: String, target: Option<String>, message: Option<String> },
    DeleteTag(String, bool),
    PushTags,
    Stash { message: String, untracked: bool, staged: bool, confirmed: bool },
    StashAct { action: &'static str, name: String, confirmed: bool },
    StashClear(bool),
    AddRemote(String, String),
    RemoveRemote(String, bool),
    AddWorktree { branch: String, new: bool, path: PathBuf },
    RemoveWorktree(PathBuf, bool, bool),
    PruneWorktrees,
    OpenTab(PathBuf),
    RevealInFiles(PathBuf),
    ShowOutput,
    HideOutput,
    EditProject,
    Init,
    Copy(String),
    OpenUrl(String),
    RunInTerminal(String),
    ToggleGraphScope,
    LoadMoreHistory,
}

/// A quick pick like Cursor's: a filter box over a list, or a text prompt.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ask {
    Checkout,
    NewBranch(Option<String>),
    NewBranchFrom,
    Detached,
    Merge,
    Rebase,
    RenameBranch,
    RenameTo(String),
    DeleteBranch,
    DeleteRemoteBranch,
    Publish,
    AddRemote,
    RemoteUrl(String),
    RemoveRemote,
    StashMessage { untracked: bool, staged: bool },
    Stash(&'static str),
    CreateTag(Option<String>),
    TagMessage(String, Option<String>),
    DeleteTag,
    Worktree,
    WorktreeNewBranch,
    WorktreePath { branch: String, new: bool },
    RemoveWorktree,
    OpenWorktree,
    Reset(String),
    SelectRepo,
}

impl Ask {
    /// Pickers listing branches, tags or stashes.
    fn needs_refs(&self) -> bool {
        matches!(
            self,
            Ask::Checkout
                | Ask::NewBranchFrom
                | Ask::Detached
                | Ask::Merge
                | Ask::Rebase
                | Ask::RenameBranch
                | Ask::DeleteBranch
                | Ask::DeleteRemoteBranch
                | Ask::DeleteTag
                | Ask::Stash(_)
                | Ask::Worktree
        )
    }
}

#[derive(Clone)]
pub(super) struct QuickItem {
    pub icon: &'static str,
    pub label: String,
    /// Muted text after the label.
    pub detail: String,
    /// Right-aligned text, such as an age.
    pub right: String,
    /// A section header shown above the first item of each section.
    pub section: Option<&'static str>,
    pub action: GitAction,
}

pub(super) struct Quick {
    pub ask: Ask,
    pub title: String,
    pub input: Entity<InputState>,
    pub items: Vec<QuickItem>,
    pub selected: usize,
    /// Enter takes the typed text rather than a list item.
    pub text: bool,
    pub loading: bool,
    pub scroll: ScrollHandle,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum MenuKind {
    Repo,
    CommitOptions,
    Commit,
}

#[derive(Clone, PartialEq, Debug)]
pub(super) struct GitMenu {
    pub kind: MenuKind,
    /// Open submenu index.
    pub sub: Option<usize>,
    /// Where the menu opens, relative to the panel.
    pub at: Point<Pixels>,
    /// The commit a commit menu is for.
    pub commit: Option<String>,
}

pub(super) struct GitState {
    pub repos: Vec<RepoView>,
    pub selected: Option<PathBuf>,
    pub discovered: bool,
    pub discovering: bool,
    pub status_at: Option<Instant>,
    pub refreshing: bool,
    pub stale: bool,
    pub collapsed: HashSet<PathBuf>,
    pub closed: HashSet<(PathBuf, Group)>,
    pub messages: HashMap<PathBuf, Entity<TextareaState>>,
    pub busy: HashMap<PathBuf, String>,
    pub graph: GraphState,
    pub graph_open: bool,
    /// Share of the sidebar's height the changes list takes.
    pub split: f32,
    pub quick: Option<Quick>,
    pub confirm: Option<Confirm>,
    pub menu: Option<GitMenu>,
    pub detail: Detail,
    pub info: Option<(String, Result<CommitInfo, String>)>,
    pub fetched: HashMap<PathBuf, Instant>,
    pub refs: HashMap<PathBuf, Vec<Ref>>,
    pub stashes: HashMap<PathBuf, Vec<Stash>>,
    pub list_scroll: ScrollHandle,
    pub output_scroll: ScrollHandle,
    pub subscriptions: Vec<Subscription>,
    pub quick_subscription: Option<Subscription>,
    /// The Files or Review view the Files button goes back to.
    pub files_view: Option<View>,
}

impl GitState {
    pub fn new() -> Self {
        Self {
            repos: vec![],
            selected: None,
            discovered: false,
            discovering: false,
            status_at: None,
            refreshing: false,
            stale: false,
            collapsed: HashSet::new(),
            closed: HashSet::new(),
            messages: HashMap::new(),
            busy: HashMap::new(),
            graph: GraphState {
                repo: None,
                all: false,
                commits: vec![],
                rows: vec![],
                loading: false,
                more: false,
                generation: 0,
                selected: None,
                incoming: HashSet::new(),
                outgoing: HashSet::new(),
                error: None,
                scroll: UniformListScrollHandle::new(),
            },
            graph_open: true,
            split: 0.56,
            quick: None,
            confirm: None,
            menu: None,
            detail: Detail::Diff,
            info: None,
            fetched: HashMap::new(),
            refs: HashMap::new(),
            stashes: HashMap::new(),
            list_scroll: ScrollHandle::new(),
            output_scroll: ScrollHandle::new(),
            subscriptions: vec![],
            quick_subscription: None,
            files_view: None,
        }
    }
    pub fn repo(&self, path: &Path) -> Option<&RepoView> {
        self.repos.iter().find(|r| r.repo.path == path)
    }
    pub fn current(&self) -> Option<&RepoView> {
        self.selected.as_deref().and_then(|p| self.repo(p))
    }
    /// Changes across every repository, for the title bar badge.
    pub fn change_count(&self) -> usize {
        self.repos
            .iter()
            .filter_map(RepoView::status)
            .map(Status::changes)
            .sum()
    }
}

// ---- Commit message drafts, shared by every panel ------------------------------

static DRAFTS: Mutex<Option<HashMap<PathBuf, String>>> = Mutex::new(None);

fn drafts_file() -> PathBuf {
    workspace::data_dir().join("git-drafts.json")
}

fn draft(repo: &Path) -> String {
    let mut drafts = DRAFTS.lock().unwrap();
    let map = drafts.get_or_insert_with(|| {
        std::fs::read(drafts_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    });
    map.get(repo).cloned().unwrap_or_default()
}

fn set_draft(repo: &Path, text: &str) {
    let mut drafts = DRAFTS.lock().unwrap();
    let map = drafts.get_or_insert_with(HashMap::new);
    if text.trim().is_empty() {
        map.remove(repo);
    } else {
        map.insert(repo.to_path_buf(), text.to_string());
    }
    if let Ok(bytes) = serde_json::to_vec(map) {
        let _ = std::fs::write(drafts_file(), bytes);
    }
}

pub(super) fn repo_label(repo: &Repo, status: Option<&Status>) -> String {
    match (repo.is_worktree(), status.and_then(|s| s.branch.clone())) {
        (true, Some(branch)) => branch,
        _ => repo.name.clone(),
    }
}

impl Browser {
    // ---- Showing -------------------------------------------------------------

    /// Opens the panel on the Git view.
    pub fn show_git(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.view = View::Git;
        self.menu = None;
        self.view_changed();
        if !self.git.discovered {
            self.load_repos();
        }
        self.git.stale = true;
        cx.notify();
    }

    pub fn git_open(&self) -> bool {
        self.visible && self.view == View::Git
    }

    /// Opens the panel on the file side (the files or the Review tab,
    /// whichever showed last).
    pub fn show_files(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.view = self.git.files_view.unwrap_or(View::Files);
        self.view_changed();
        cx.notify();
    }

    /// The project list changed: look for repositories again.
    pub fn project_changed(&mut self, cx: &mut Context<Self>) {
        self.git.discovered = false;
        self.git.stale = true;
        cx.notify();
    }

    /// Changes in the workspace (and its project repositories).
    pub fn change_count(&self) -> usize {
        if self.git.discovered {
            self.git.change_count()
        } else {
            self.changes.len()
        }
    }

    pub(super) fn load_repos(&mut self) {
        if self.git.discovering {
            return;
        }
        self.git.discovering = true;
        let root = self.root.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Message::GitRepos(git::repositories(&root)));
        });
    }

    pub(super) fn git_repos_loaded(&mut self, repos: Vec<Repo>, window: &mut Window, cx: &mut Context<Self>) {
        self.git.discovering = false;
        self.git.discovered = true;
        let mut old: HashMap<PathBuf, RepoView> = self
            .git
            .repos
            .drain(..)
            .map(|r| (r.repo.path.clone(), r))
            .collect();
        self.git.repos = repos
            .into_iter()
            .map(|repo| {
                let status = old.remove(&repo.path).and_then(|r| r.status);
                RepoView { repo, status }
            })
            .collect();
        let valid = self
            .git
            .selected
            .as_ref()
            .is_some_and(|s| self.git.repo(s).is_some());
        if !valid {
            // The repository the terminal is in, the deepest match first.
            let root = self.root.clone();
            let chosen = self
                .git
                .repos
                .iter()
                .filter(|r| matches_root(&root, &r.repo.path))
                .max_by_key(|r| r.repo.path.components().count())
                .or_else(|| self.git.repos.first())
                .map(|r| r.repo.path.clone());
            if let Some(path) = chosen {
                self.select_repo(path, window, cx);
            }
        }
        self.git.stale = true;
    }

    pub(super) fn select_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.git.selected.as_ref() != Some(&path);
        self.git.selected = Some(path.clone());
        self.message_input(&path, window, cx);
        if changed || self.git.graph.repo.as_ref() != Some(&path) {
            self.git.graph.selected = None;
            self.load_graph(false);
        }
        // The diff side follows the selected repository.
        if changed || self.parked_or_active_git_repo() != Some(path.clone()) {
            self.with_git_review(|this| {
                this.review.repo = Some(path.clone());
                this.review.source = Some(Source::Uncommitted);
                this.review.pending = Some(true);
            });
        }
        self.git.detail = Detail::Diff;
        cx.notify();
    }

    fn parked_or_active_git_repo(&self) -> Option<PathBuf> {
        if self.review.git {
            self.review.repo.clone()
        } else {
            self.parked.repo.clone()
        }
    }

    /// Runs `f` with the Git view's review as `self.review`.
    pub(super) fn with_git_review(&mut self, f: impl FnOnce(&mut Self)) {
        let swap = !self.review.git;
        if swap {
            std::mem::swap(&mut self.review, &mut self.parked);
        }
        f(self);
        if swap {
            std::mem::swap(&mut self.review, &mut self.parked);
        }
    }

    /// Makes `self.review` the review of the current view.
    pub(super) fn view_changed(&mut self) {
        let git = self.view == View::Git;
        if !git {
            self.git.files_view = Some(self.view);
        }
        if self.review.git != git {
            std::mem::swap(&mut self.review, &mut self.parked);
            // It may have missed changes while parked.
            if self
                .review
                .source
                .as_ref()
                .is_some_and(|s| s.follows_worktree() || *s == Source::Staged)
            {
                self.review_reload_now();
            }
        }
    }

    /// The commit box of a repository, created with its saved draft.
    pub(super) fn message_input(
        &mut self,
        repo: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some(input) = self.git.messages.get(repo) {
            return input.clone();
        }
        let text = draft(repo);
        // The box sits above every repository, so it names the one it commits to.
        let name = self
            .git
            .repo(repo)
            .map(|r| repo_label(&r.repo, r.status()))
            .unwrap_or_else(|| project::folder_name(repo));
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .placeholder(format!("Message for {name} (Ctrl+Enter to commit)"))
                .default_value(text)
        });
        let key = repo.to_path_buf();
        let subscription = cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
            match event {
                InputEvent::Change => set_draft(&key, &input.read(cx).value()),
                InputEvent::PressEnter { secondary: true, .. } => {
                    // Ctrl+Enter also typed a newline; the commit trims it.
                    this.git.selected = Some(key.clone());
                    this.perform(
                        GitAction::Commit {
                            amend: false,
                            then: None,
                            stage_all: false,
                        },
                        window,
                        cx,
                    );
                }
                _ => {}
            }
        });
        self.git.subscriptions.push(subscription);
        self.git.messages.insert(repo.to_path_buf(), input.clone());
        input
    }

    fn set_message(&mut self, repo: &Path, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.message_input(repo, window, cx);
        input.update(cx, |input, cx| input.set_value(text.to_string(), window, cx));
        set_draft(repo, text);
    }

    // ---- Refreshing ----------------------------------------------------------

    /// Called from `poll`: keeps statuses, history and fetches current while
    /// the Git view is open.
    pub(super) fn git_tick(&mut self) {
        if !self.git_open() {
            return;
        }
        if !self.git.discovered {
            self.load_repos();
            return;
        }
        let due = self
            .git
            .status_at
            .is_none_or(|at| at.elapsed() > STATUS_EVERY);
        if (self.git.stale || due) && !self.git.refreshing {
            self.refresh_statuses();
        }
        let due: Vec<PathBuf> = self
            .git
            .repos
            .iter()
            .filter(|r| !r.repo.is_worktree())
            .filter(|r| r.status().is_some_and(|s| !s.remotes.is_empty()))
            .filter(|r| {
                self.git
                    .fetched
                    .get(&r.repo.path)
                    .is_none_or(|at| at.elapsed() > FETCH_EVERY)
            })
            .map(|r| r.repo.path.clone())
            .collect();
        if !due.is_empty() {
            let enabled = crate::config::Config::load().git_autofetch;
            for path in due {
                self.git.fetched.insert(path.clone(), Instant::now());
                if enabled {
                    self.run_git(path, None, OnError::Quiet, move |repo| {
                        ops::fetch(repo, false)?;
                        Ok(After::Nothing)
                    });
                }
            }
        }
    }

    fn refresh_statuses(&mut self) {
        self.git.refreshing = true;
        self.git.stale = false;
        self.git.status_at = Some(Instant::now());
        let repos: Vec<PathBuf> = self.git.repos.iter().map(|r| r.repo.path.clone()).collect();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let results = repos
                .into_iter()
                .map(|path| {
                    let status = git::status(&path).map_err(|e| e.to_string());
                    (path, status)
                })
                .collect();
            let _ = sender.send(Message::GitStatuses(results));
        });
    }

    pub(super) fn git_statuses_loaded(&mut self, results: Vec<(PathBuf, Result<Status, String>)>) {
        self.git.refreshing = false;
        let mut head_moved = false;
        for (path, status) in results {
            if let Some(view) = self.git.repos.iter_mut().find(|r| r.repo.path == path) {
                if self.git.graph.repo.as_ref() == Some(&path) {
                    let old = view.status().map(|s| (s.oid.clone(), s.upstream.clone(), s.behind));
                    let new = status
                        .as_ref()
                        .ok()
                        .map(|s| (s.oid.clone(), s.upstream.clone(), s.behind));
                    head_moved |= old.is_some() && old != new;
                }
                view.status = Some(status);
            }
        }
        if head_moved {
            self.load_graph(false);
        }
    }

    pub(super) fn load_graph(&mut self, more: bool) {
        let Some(repo) = self.git.selected.clone() else {
            return;
        };
        let graph = &mut self.git.graph;
        if graph.repo.as_ref() != Some(&repo) {
            graph.commits.clear();
            graph.rows.clear();
            graph.more = false;
        }
        graph.repo = Some(repo.clone());
        graph.loading = true;
        graph.error = None;
        graph.generation += 1;
        let generation = graph.generation;
        let skip = if more { graph.commits.len() } else { 0 };
        let count = if more {
            GRAPH_PAGE
        } else {
            graph.commits.len().max(GRAPH_PAGE)
        };
        let all = graph.all;
        let upstream = self
            .git
            .repo(&repo)
            .and_then(RepoView::status)
            .and_then(|s| s.upstream.clone());
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = git::log(&repo, all, upstream.as_deref(), skip, count)
                .map(|commits| {
                    let (incoming, outgoing) = if upstream.is_some() {
                        git::incoming_outgoing(&repo)
                    } else {
                        (vec![], vec![])
                    };
                    (commits, incoming, outgoing)
                })
                .map_err(|e| e.to_string());
            let _ = sender.send(Message::GitGraph(generation, repo, more, count, result));
        });
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn git_graph_loaded(
        &mut self,
        generation: u64,
        repo: PathBuf,
        more: bool,
        count: usize,
        result: Result<(Vec<Commit>, Vec<String>, Vec<String>), String>,
    ) {
        let graph = &mut self.git.graph;
        if graph.generation != generation || graph.repo.as_ref() != Some(&repo) {
            return;
        }
        graph.loading = false;
        match result {
            Ok((commits, incoming, outgoing)) => {
                graph.more = commits.len() >= count;
                if more {
                    graph.commits.extend(commits);
                } else {
                    graph.commits = commits;
                }
                graph.incoming = incoming.into_iter().collect();
                graph.outgoing = outgoing.into_iter().collect();
                let head = graph
                    .commits
                    .iter()
                    .find(|c| {
                        c.labels
                            .iter()
                            .any(|l| matches!(l, git::Label::Head(_) | git::Label::Detached))
                    })
                    .map(|c| c.hash.clone());
                graph.rows = graph::layout(
                    graph
                        .commits
                        .iter()
                        .map(|c| (c.hash.as_str(), c.parents.as_slice())),
                    head.as_deref(),
                );
            }
            Err(e) => graph.error = Some(e),
        }
    }

    pub(super) fn load_commit_info(&mut self, repo: PathBuf, hash: String) {
        if self.git.info.as_ref().is_some_and(|(h, i)| *h == hash && i.is_ok()) {
            return;
        }
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let info = git::commit_info(&repo, &hash).map_err(|e| e.to_string());
            let _ = sender.send(Message::GitCommitInfo(hash, info));
        });
    }

    fn load_refs(&mut self, repo: PathBuf) {
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let refs = git::refs(&repo).unwrap_or_default();
            let stashes = git::stashes(&repo).unwrap_or_default();
            let _ = sender.send(Message::GitRefs(repo, refs, stashes));
        });
    }

    pub(super) fn git_refs_loaded(&mut self, repo: PathBuf, refs: Vec<Ref>, stashes: Vec<Stash>) {
        self.git.refs.insert(repo.clone(), refs);
        self.git.stashes.insert(repo.clone(), stashes);
        if self.git.selected.as_ref() == Some(&repo)
            && let Some(quick) = &self.git.quick
            && quick.loading
        {
            let ask = quick.ask.clone();
            let items = self.quick_items(&ask);
            if let Some(quick) = &mut self.git.quick {
                quick.items = items;
                quick.loading = false;
                quick.selected = 0;
            }
        }
    }

    // ---- Running commands ----------------------------------------------------

    /// Runs Git work for `repo` on a thread. `label` marks the repository busy
    /// while it runs.
    pub(super) fn run_git(
        &mut self,
        repo: PathBuf,
        label: Option<&str>,
        on_error: OnError,
        work: impl FnOnce(&Path) -> ops::Outcome<After> + Send + 'static,
    ) {
        if let Some(label) = label {
            self.git.busy.insert(repo.clone(), label.to_string());
        }
        let sender = self.sender.clone();
        let busy = label.is_some();
        std::thread::spawn(move || {
            let result = work(&repo);
            let _ = sender.send(Message::GitDone(repo, busy, result, on_error));
        });
    }

    pub(super) fn git_done(
        &mut self,
        repo: PathBuf,
        busy: bool,
        result: ops::Outcome<After>,
        on_error: OnError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if busy {
            self.git.busy.remove(&repo);
            // Worktrees may have come or gone.
            self.load_repos();
        }
        self.git.stale = true;
        self.git.refs.remove(&repo);
        if self.git.graph.repo.as_ref() == Some(&repo) {
            self.load_graph(false);
        }
        if self.git_open() {
            self.review_reload_now();
        }
        match result {
            Ok(after) => match after {
                After::Nothing => {}
                After::Notice(text) => self.say(text),
                After::Committed(next) => {
                    self.set_message(&repo, "", window, cx);
                    self.say("Committed");
                    if let Some(next) = next {
                        self.git.selected = Some(repo);
                        self.perform(next, window, cx);
                    }
                }
                After::Message(text) => {
                    self.set_message(&repo, &text, window, cx);
                    self.say("Last commit undone · its changes are staged");
                }
                After::Ask(confirm) => self.git.confirm = Some(confirm),
            },
            Err(failure) => self.git_failed(repo, failure, on_error),
        }
        cx.notify();
    }

    fn git_failed(&mut self, repo: PathBuf, failure: ops::Failure, on_error: OnError) {
        let name = project::folder_name(&repo);
        match on_error {
            OnError::Quiet => {}
            OnError::Conflicts if failure.message.to_lowercase().contains("conflict") => {
                self.say(format!(
                    "{name} has conflicts · resolve them under Merge Changes, then continue"
                ));
            }
            OnError::DeleteBranch(branch) if failure.message.contains("not fully merged") => {
                self.git.confirm = Some(Confirm {
                    title: format!("Delete {branch}?"),
                    message: format!(
                        "{branch} has commits that aren't merged into the current branch. Deleting it loses them unless another branch or tag points at them."
                    ),
                    detail: None,
                    buttons: vec![
                        Button::cancel(),
                        Button::new(
                            "Delete Anyway",
                            Style::Danger,
                            Some(GitAction::DeleteBranch(branch, true)),
                        ),
                    ],
                });
            }
            OnError::Checkout(target, kind) => {
                let dirty = self
                    .git
                    .repo(&repo)
                    .and_then(RepoView::status)
                    .is_some_and(|s| s.changes() > 0);
                if dirty {
                    let action = |mode| {
                        Some(GitAction::Checkout {
                            target: target.clone(),
                            kind,
                            mode,
                            confirmed: true,
                        })
                    };
                    self.git.confirm = Some(Confirm {
                        title: format!("Can't switch to {target}"),
                        message: "Your local changes would be overwritten. Stash them, carry them over to the other branch, or discard them.".into(),
                        detail: Some(failure.message),
                        buttons: vec![
                            Button::cancel(),
                            Button::new("Force Switch", Style::Danger, action(CheckoutMode::Force)),
                            Button::new("Migrate Changes", Style::Plain, action(CheckoutMode::Migrate)),
                            Button::new("Stash & Switch", Style::Primary, action(CheckoutMode::Stash)),
                        ],
                    });
                } else {
                    self.show_failure(&repo, failure);
                }
            }
            _ => self.show_failure(&repo, failure),
        }
    }

    fn show_failure(&mut self, repo: &Path, failure: ops::Failure) {
        let name = project::folder_name(repo);
        let mut buttons = vec![Button::new("Show Output", Style::Plain, Some(GitAction::ShowOutput))];
        let last = ops::log().pop();
        if failure.auth
            && let Some(last) = &last
        {
            let command = format!(
                "git -C \"{}\" {}",
                repo.display(),
                last.command.trim_start_matches("git ")
            );
            buttons.push(Button::new(
                "Run in Terminal",
                Style::Plain,
                Some(GitAction::RunInTerminal(command)),
            ));
        }
        buttons.push(Button::new("OK", Style::Primary, None));
        self.git.confirm = Some(Confirm {
            title: format!("Git failed in {name}"),
            message: if failure.auth {
                "The remote needs you to sign in. Run the command in a terminal to enter your credentials once.".into()
            } else {
                last.map(|l| l.command).unwrap_or_default()
            },
            detail: Some(failure.message),
            buttons,
        });
    }

    // ---- Helpers ---------------------------------------------------------------

    /// Active agent turns that could be touching `repo`.
    fn agent_in(&self, repo: &Path) -> Option<String> {
        self.tasks
            .iter()
            .filter(|t| t.active)
            .find(|t| matches_root(&t.root, repo) || matches_root(repo, &t.root))
            .map(|t| t.agent.clone())
    }

    fn selected_status(&self) -> Option<Status> {
        self.git.current().and_then(RepoView::status).cloned()
    }

    fn worktree_of_branch(&self, repo: &Path, branch: &str) -> Option<PathBuf> {
        self.git
            .refs
            .get(repo)?
            .iter()
            .find(|r| r.kind == RefKind::Local && r.name == branch)?
            .worktree
            .clone()
            .filter(|w| !git::same_path(w, repo))
    }

    fn confirm_agent(&mut self, repo: &Path, what: &str, action: GitAction) -> bool {
        match self.agent_in(repo) {
            Some(agent) => {
                self.git.confirm = Some(Confirm {
                    title: format!("{agent} is working in {}", project::folder_name(repo)),
                    message: format!(
                        "{what} changes files under the running turn. Its review may then mix both."
                    ),
                    detail: None,
                    buttons: vec![
                        Button::cancel(),
                        Button::new("Continue Anyway", Style::Danger, Some(action)),
                    ],
                });
                true
            }
            None => false,
        }
    }

    // ---- Actions -----------------------------------------------------------------

    pub(super) fn perform(&mut self, action: GitAction, window: &mut Window, cx: &mut Context<Self>) {
        self.git.menu = None;
        self.git.confirm = None;
        let Some(repo) = self.git.selected.clone().or_else(|| match &action {
            GitAction::SelectRepo(p) => Some(p.clone()),
            _ => None,
        }) else {
            if matches!(action, GitAction::Init) {
                let root = self.root.clone();
                self.run_git(root, Some("Initializing…"), OnError::Show, |repo| {
                    ops::init(repo)?;
                    Ok(After::Notice("Repository initialized".into()))
                });
                self.git.discovered = false;
                self.load_repos();
            }
            cx.notify();
            return;
        };
        let status = self.selected_status();
        match action {
            GitAction::Refresh => {
                self.git.stale = true;
                self.git.refs.clear();
                self.load_repos();
                self.load_graph(false);
                self.review_reload_now();
            }
            GitAction::SelectRepo(path) => self.select_repo(path, window, cx),
            GitAction::SelectCommit(hash) => {
                self.git.graph.selected = Some(hash.clone());
                self.git.detail = Detail::Diff;
                let title = self
                    .git
                    .graph
                    .commits
                    .iter()
                    .find(|c| c.hash == hash)
                    .map(|c| c.subject.clone())
                    .unwrap_or_default();
                self.with_git_review(|this| {
                    this.review.repo = Some(repo.clone());
                });
                self.set_source(Source::Commit { hash, title }, false, cx);
            }
            GitAction::ShowChanges(path, file, staged) => {
                if self.git.selected.as_ref() != Some(&path) {
                    self.select_repo(path.clone(), window, cx);
                }
                self.git.detail = Detail::Diff;
                self.git.graph.selected = None;
                let source = if staged { Source::Staged } else { Source::Unstaged };
                if self.review.source.as_ref() != Some(&source) || self.review.repo.as_ref() != Some(&path) {
                    self.review.repo = Some(path.clone());
                    self.set_source(source, false, cx);
                }
                self.review.reveal_path = file;
                self.review.rows_dirty = true;
            }
            GitAction::Fetch => self.run_git(repo.clone(), Some("Fetching…"), OnError::Show, |repo| {
                ops::fetch(repo, true)?;
                Ok(After::Notice("Fetched".into()))
            }),
            GitAction::Pull(rebase, confirmed) => {
                if !confirmed && self.confirm_agent(&repo, "Pulling", GitAction::Pull(rebase, true)) {
                    return;
                }
                self.run_git(repo, Some("Pulling…"), OnError::Conflicts, move |repo| {
                    ops::pull(repo, rebase)?;
                    Ok(After::Notice("Pulled".into()))
                });
            }
            GitAction::Push(force, confirmed) => {
                let Some(status) = status else { return };
                if status.upstream.is_none() {
                    self.perform(GitAction::Publish(None), window, cx);
                    return;
                }
                if force && !confirmed {
                    let upstream = status.upstream.clone().unwrap_or_default();
                    self.git.confirm = Some(Confirm {
                        title: "Force push?".into(),
                        message: format!(
                            "This replaces {upstream} with your branch. Git refuses if someone else pushed to it since your last fetch (--force-with-lease)."
                        ),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new("Force Push", Style::Danger, Some(GitAction::Push(true, true))),
                        ],
                    });
                    return;
                }
                self.run_git(repo, Some("Pushing…"), OnError::Show, move |repo| {
                    ops::push(repo, None, force)?;
                    Ok(After::Notice("Pushed".into()))
                });
            }
            GitAction::Sync(confirmed) => {
                let Some(status) = status else { return };
                if status.upstream.is_none() {
                    self.perform(GitAction::Publish(None), window, cx);
                    return;
                }
                if !confirmed
                    && status.behind > 0
                    && self.confirm_agent(&repo, "Pulling", GitAction::Sync(true))
                {
                    return;
                }
                let (behind, ahead) = (status.behind, status.ahead);
                self.run_git(repo, Some("Syncing…"), OnError::Conflicts, move |repo| {
                    if behind > 0 || ahead == 0 {
                        ops::pull(repo, false)?;
                    }
                    if ahead > 0 || behind > 0 {
                        ops::push(repo, None, false)?;
                    }
                    Ok(After::Notice("Synced".into()))
                });
            }
            GitAction::Publish(remote) => {
                let Some(status) = status else { return };
                let Some(branch) = status.branch.clone() else {
                    self.say("Switch to a branch before publishing");
                    return;
                };
                let remote = match remote {
                    Some(remote) => remote,
                    None if status.remotes.len() == 1 => status.remotes[0].clone(),
                    None if status.remotes.is_empty() => {
                        self.say("Add a remote first · ··· ▸ Remote ▸ Add Remote");
                        cx.notify();
                        return;
                    }
                    None => {
                        self.open_quick(Ask::Publish, window, cx);
                        return;
                    }
                };
                self.run_git(repo, Some("Publishing…"), OnError::Show, move |repo| {
                    ops::push(repo, Some((&remote, &branch)), false)?;
                    Ok(After::Notice(format!("Published {branch} to {remote}")))
                });
            }
            GitAction::Commit {
                amend,
                then,
                stage_all,
            } => self.commit(repo, amend, then.map(|b| *b), stage_all, window, cx),
            GitAction::UndoCommit(confirmed) => {
                let Some(status) = status else { return };
                if status.oid.is_none() {
                    self.say("There is no commit to undo");
                } else if !confirmed && status.upstream.is_some() && status.ahead == 0 {
                    self.git.confirm = Some(Confirm {
                        title: "Undo a pushed commit?".into(),
                        message: format!(
                            "The last commit is already on {}. Undoing it keeps its changes staged, but pushing afterwards needs a force push.",
                            status.upstream.unwrap_or_default()
                        ),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new("Undo Commit", Style::Danger, Some(GitAction::UndoCommit(true))),
                        ],
                    });
                } else {
                    self.run_git(repo, Some("Undoing…"), OnError::Show, |repo| {
                        Ok(After::Message(ops::undo_commit(repo)?))
                    });
                }
            }
            GitAction::StageAll(path) => self.run_git(path, None, OnError::Show, |repo| {
                ops::stage_all(repo)?;
                Ok(After::Nothing)
            }),
            GitAction::UnstageAll(path) => self.run_git(path, None, OnError::Show, |repo| {
                ops::unstage_all(repo)?;
                Ok(After::Nothing)
            }),
            GitAction::DiscardAll(path, confirmed) => {
                let Some(status) = self.git.repo(&path).and_then(RepoView::status).cloned() else {
                    return;
                };
                let (untracked, tracked): (Vec<_>, Vec<_>) =
                    status.unstaged.iter().partition(|e| e.letter == 'U');
                let tracked: Vec<String> = tracked.iter().map(|e| e.path.clone()).collect();
                let untracked: Vec<String> = untracked.iter().map(|e| e.path.clone()).collect();
                self.perform(
                    GitAction::Discard {
                        repo: path,
                        tracked,
                        untracked,
                        confirmed,
                    },
                    window,
                    cx,
                );
                return;
            }
            GitAction::Stage(path, paths) => self.run_git(path, None, OnError::Show, move |repo| {
                ops::stage(repo, &paths)?;
                Ok(After::Nothing)
            }),
            GitAction::Unstage(path, paths) => self.run_git(path, None, OnError::Show, move |repo| {
                ops::unstage(repo, &paths)?;
                Ok(After::Nothing)
            }),
            GitAction::Discard {
                repo,
                tracked,
                untracked,
                confirmed,
            } => {
                let count = tracked.len() + untracked.len();
                if count == 0 {
                    return;
                }
                if !confirmed {
                    let name = if count == 1 {
                        tracked
                            .first()
                            .or(untracked.first())
                            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
                            .unwrap_or_default()
                    } else {
                        format!("{count} files")
                    };
                    let deleting = if untracked.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " {} untracked {} will be deleted.",
                            untracked.len(),
                            if untracked.len() == 1 { "file" } else { "files" }
                        )
                    };
                    self.git.confirm = Some(Confirm {
                        title: format!("Discard changes in {name}?"),
                        message: format!(
                            "Your edits go back to the staged or committed version.{deleting} Vyber keeps a recovery copy of every file in its data folder."
                        ),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new(
                                "Discard",
                                Style::Danger,
                                Some(GitAction::Discard {
                                    repo,
                                    tracked,
                                    untracked,
                                    confirmed: true,
                                }),
                            ),
                        ],
                    });
                    return;
                }
                self.run_git(repo, None, OnError::Show, move |repo| {
                    ops::discard(repo, &tracked, &untracked)?;
                    Ok(After::Notice(format!(
                        "Discarded {count} {} · recovery copies kept",
                        if count == 1 { "file" } else { "files" }
                    )))
                });
            }
            GitAction::TakeSide(path, file, ours) => self.run_git(path, None, OnError::Show, move |repo| {
                ops::take_side(repo, &file, ours)?;
                Ok(After::Notice(format!(
                    "Kept {} version of {file}",
                    if ours { "the current" } else { "the incoming" }
                )))
            }),
            GitAction::Ask(ask) => self.open_quick(ask, window, cx),
            GitAction::Checkout {
                target,
                kind,
                mode,
                confirmed,
            } => self.checkout(repo, target, kind, mode, confirmed, cx),
            GitAction::CreateBranch { name, from } => {
                let name = name.trim().to_string();
                self.run_git(repo, Some("Creating branch…"), OnError::Show, move |repo| {
                    ops::create_branch(repo, &name, from.as_deref(), true)?;
                    Ok(After::Notice(format!("Switched to new branch {name}")))
                });
            }
            GitAction::RenameBranch(old, new) => self.run_git(repo, None, OnError::Show, move |repo| {
                ops::rename_branch(repo, &old, new.trim())?;
                Ok(After::Notice(format!("Renamed {old} to {}", new.trim())))
            }),
            GitAction::DeleteBranch(name, force) => {
                let error = OnError::DeleteBranch(name.clone());
                self.run_git(repo, None, error, move |repo| {
                    ops::delete_branch(repo, &name, force)?;
                    Ok(After::Notice(format!("Deleted {name}")))
                });
            }
            GitAction::DeleteRemoteBranch(name, confirmed) => {
                let Some((remote, branch)) = name.split_once('/').map(|(r, b)| (r.to_string(), b.to_string())) else {
                    return;
                };
                if !confirmed {
                    self.git.confirm = Some(Confirm {
                        title: format!("Delete {name} on {remote}?"),
                        message: "The branch is removed from the remote for everyone.".into(),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new(
                                "Delete Remote Branch",
                                Style::Danger,
                                Some(GitAction::DeleteRemoteBranch(name, true)),
                            ),
                        ],
                    });
                    return;
                }
                self.run_git(repo, Some("Deleting…"), OnError::Show, move |repo| {
                    ops::delete_remote_branch(repo, &remote, &branch)?;
                    Ok(After::Notice(format!("Deleted {remote}/{branch}")))
                });
            }
            GitAction::Merge(target) => self.run_git(repo, Some("Merging…"), OnError::Conflicts, move |repo| {
                ops::merge(repo, &target)?;
                Ok(After::Notice(format!("Merged {target}")))
            }),
            GitAction::Rebase(onto, confirmed) => {
                if !confirmed
                    && self.confirm_agent(&repo, "Rebasing", GitAction::Rebase(onto.clone(), true))
                {
                    return;
                }
                self.run_git(repo, Some("Rebasing…"), OnError::Conflicts, move |repo| {
                    ops::rebase(repo, &onto)?;
                    Ok(After::Notice(format!("Rebased onto {onto}")))
                });
            }
            GitAction::Sequence(operation, step) => {
                let label = match step {
                    "abort" => "Aborting…",
                    "skip" => "Skipping…",
                    _ => "Continuing…",
                };
                let message = self
                    .git
                    .messages
                    .get(&repo)
                    .map(|m| m.read(cx).value().trim().to_string())
                    .unwrap_or_default();
                self.run_git(repo, Some(label), OnError::Conflicts, move |repo| {
                    if step == "continue" && operation == Operation::Merge && !message.is_empty() {
                        ops::commit(repo, &message, false)?;
                        return Ok(After::Committed(None));
                    }
                    ops::sequence(repo, operation, step)?;
                    Ok(After::Notice(format!("{} {}", operation.label(), step_label(step))))
                });
            }
            GitAction::CherryPick(hash) => {
                let merge = self.commit_is_merge(&hash);
                self.run_git(repo, Some("Cherry-picking…"), OnError::Conflicts, move |repo| {
                    ops::cherry_pick(repo, &hash, merge)?;
                    Ok(After::Notice(format!("Cherry-picked {}", short(&hash))))
                });
            }
            GitAction::RevertCommit(hash) => {
                let merge = self.commit_is_merge(&hash);
                self.run_git(repo, Some("Reverting…"), OnError::Conflicts, move |repo| {
                    ops::revert_commit(repo, &hash, merge)?;
                    Ok(After::Notice(format!("Reverted {}", short(&hash))))
                });
            }
            GitAction::Reset {
                target,
                mode,
                confirmed,
            } => {
                if !confirmed {
                    let hard = mode == "hard";
                    let mut message: String = match mode {
                        "soft" => "Commits after it are undone; their changes stay staged.".into(),
                        "mixed" => "Commits after it are undone; their changes stay in your files, unstaged.".into(),
                        _ => "Commits after it and every uncommitted change are thrown away. Vyber keeps recovery copies of changed files.".into(),
                    };
                    if let Some(agent) = self.agent_in(&repo).filter(|_| hard) {
                        message.push_str(&format!(" {agent} is working in this repository right now."));
                    }
                    self.git.confirm = Some(Confirm {
                        title: format!("Reset {} to {}?", status.map(|s| s.head_label()).unwrap_or_default(), short(&target)),
                        message,
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new(
                                format!("Reset ({mode})"),
                                if hard { Style::Danger } else { Style::Primary },
                                Some(GitAction::Reset {
                                    target,
                                    mode,
                                    confirmed: true,
                                }),
                            ),
                        ],
                    });
                    return;
                }
                self.run_git(repo, Some("Resetting…"), OnError::Show, move |repo| {
                    ops::reset(repo, &target, mode)?;
                    Ok(After::Notice(format!("Reset to {}", short(&target))))
                });
            }
            GitAction::CreateTag {
                name,
                target,
                message,
            } => self.run_git(repo, None, OnError::Show, move |repo| {
                ops::create_tag(repo, name.trim(), target.as_deref(), message.as_deref())?;
                Ok(After::Notice(format!("Tagged {}", name.trim())))
            }),
            GitAction::DeleteTag(name, confirmed) => {
                if !confirmed {
                    self.git.confirm = Some(Confirm {
                        title: format!("Delete tag {name}?"),
                        message: "Only the local tag is deleted.".into(),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new("Delete Tag", Style::Danger, Some(GitAction::DeleteTag(name, true))),
                        ],
                    });
                    return;
                }
                self.run_git(repo, None, OnError::Show, move |repo| {
                    ops::delete_tag(repo, &name)?;
                    Ok(After::Notice(format!("Deleted tag {name}")))
                });
            }
            GitAction::PushTags => {
                let remote = status
                    .and_then(|s| s.remotes.first().cloned())
                    .unwrap_or_else(|| "origin".into());
                self.run_git(repo, Some("Pushing tags…"), OnError::Show, move |repo| {
                    ops::push_tags(repo, &remote)?;
                    Ok(After::Notice("Tags pushed".into()))
                });
            }
            GitAction::Stash {
                message,
                untracked,
                staged,
                confirmed,
            } => {
                let again = GitAction::Stash {
                    message: message.clone(),
                    untracked,
                    staged,
                    confirmed: true,
                };
                if !confirmed && self.confirm_agent(&repo, "Stashing", again) {
                    return;
                }
                self.run_git(repo, Some("Stashing…"), OnError::Show, move |repo| {
                    ops::stash(repo, &message, untracked, staged)?;
                    Ok(After::Notice("Changes stashed".into()))
                });
            }
            GitAction::StashAct {
                action,
                name,
                confirmed,
            } => {
                if action == "drop" && !confirmed {
                    self.git.confirm = Some(Confirm {
                        title: format!("Drop {name}?"),
                        message: "The stashed changes are deleted.".into(),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new(
                                "Drop Stash",
                                Style::Danger,
                                Some(GitAction::StashAct {
                                    action,
                                    name,
                                    confirmed: true,
                                }),
                            ),
                        ],
                    });
                    return;
                }
                self.run_git(repo, None, OnError::Conflicts, move |repo| {
                    ops::stash_action(repo, action, &name)?;
                    Ok(After::Notice(format!(
                        "{} {name}",
                        match action {
                            "pop" => "Popped",
                            "drop" => "Dropped",
                            _ => "Applied",
                        }
                    )))
                });
            }
            GitAction::StashClear(confirmed) => {
                if !confirmed {
                    self.git.confirm = Some(Confirm {
                        title: "Drop every stash?".into(),
                        message: "All stashed changes of this repository are deleted.".into(),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new("Drop All", Style::Danger, Some(GitAction::StashClear(true))),
                        ],
                    });
                    return;
                }
                self.run_git(repo, None, OnError::Show, |repo| {
                    ops::stash_clear(repo)?;
                    Ok(After::Notice("Stashes dropped".into()))
                });
            }
            GitAction::AddRemote(name, url) => self.run_git(repo, None, OnError::Show, move |repo| {
                ops::add_remote(repo, name.trim(), url.trim())?;
                Ok(After::Notice(format!("Added remote {}", name.trim())))
            }),
            GitAction::RemoveRemote(name, confirmed) => {
                if !confirmed {
                    self.git.confirm = Some(Confirm {
                        title: format!("Remove remote {name}?"),
                        message: "Its remote-tracking branches are removed too. Nothing changes on the server.".into(),
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new("Remove", Style::Danger, Some(GitAction::RemoveRemote(name, true))),
                        ],
                    });
                    return;
                }
                self.run_git(repo, None, OnError::Show, move |repo| {
                    ops::remove_remote(repo, &name)?;
                    Ok(After::Notice(format!("Removed remote {name}")))
                });
            }
            GitAction::AddWorktree { branch, new, path } => {
                let main = self
                    .git
                    .repo(&repo)
                    .and_then(|r| r.repo.worktree_of.clone())
                    .unwrap_or(repo);
                self.run_git(main, Some("Creating worktree…"), OnError::Show, move |repo| {
                    ops::add_worktree(repo, &path, &branch, new, None)?;
                    Ok(After::Ask(Confirm {
                        title: "Worktree created".into(),
                        message: format!("{branch} is checked out in {}.", path.display()),
                        detail: None,
                        buttons: vec![
                            Button::new("Done", Style::Plain, None),
                            Button::new("Open in New Tab", Style::Primary, Some(GitAction::OpenTab(path))),
                        ],
                    }))
                });
                self.git.discovered = false;
            }
            GitAction::RemoveWorktree(path, force, confirmed) => {
                let owner = self
                    .git
                    .repos
                    .iter()
                    .find(|r| r.repo.path == path)
                    .and_then(|r| r.repo.worktree_of.clone())
                    .unwrap_or(repo);
                if !confirmed {
                    let dirty = self
                        .git
                        .repo(&path)
                        .and_then(RepoView::status)
                        .is_some_and(|s| s.changes() > 0);
                    self.git.confirm = Some(Confirm {
                        title: format!("Remove worktree {}?", project::folder_name(&path)),
                        message: if dirty {
                            "It has uncommitted changes, which are deleted with it. Its branch stays.".into()
                        } else {
                            "The folder is deleted; its branch stays.".into()
                        },
                        detail: None,
                        buttons: vec![
                            Button::cancel(),
                            Button::new(
                                "Remove Worktree",
                                Style::Danger,
                                Some(GitAction::RemoveWorktree(path, dirty, true)),
                            ),
                        ],
                    });
                    return;
                }
                if self.git.selected.as_ref() == Some(&path) {
                    self.git.selected = Some(owner.clone());
                }
                self.run_git(owner, Some("Removing worktree…"), OnError::Show, move |repo| {
                    ops::remove_worktree(repo, &path, force)?;
                    Ok(After::Notice("Worktree removed".into()))
                });
                self.git.discovered = false;
            }
            GitAction::PruneWorktrees => {
                self.run_git(repo, None, OnError::Show, |repo| {
                    ops::prune_worktrees(repo)?;
                    Ok(After::Notice("Stale worktrees pruned".into()))
                });
                self.git.discovered = false;
            }
            GitAction::OpenTab(path) => cx.emit(BrowserEvent::OpenFolder(path)),
            GitAction::RevealInFiles(path) => {
                self.view = View::Files;
                self.view_changed();
                if let Ok(relative) = path.strip_prefix(&self.root) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    if !relative.is_empty() {
                        self.scope.set_extra(vec![path.clone()]);
                        self.tree_root = Some(relative);
                    }
                } else {
                    cx.emit(BrowserEvent::OpenFolder(path));
                }
            }
            GitAction::ShowOutput => {
                self.git.detail = Detail::Output;
                self.git.output_scroll.scroll_to_bottom();
            }
            GitAction::HideOutput => self.git.detail = Detail::Diff,
            GitAction::EditProject => cx.emit(BrowserEvent::EditProject(self.root.clone())),
            GitAction::Init => {}
            GitAction::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.say(format!("Copied {}", if text.len() > 40 { "to clipboard" } else { &text }));
            }
            GitAction::OpenUrl(url) => cx.open_url(&url),
            GitAction::RunInTerminal(command) => {
                cx.emit(BrowserEvent::RunInTerminal(command));
                self.say("Typed into the terminal · press Enter there to run it");
            }
            GitAction::ToggleGraphScope => {
                self.git.graph.all = !self.git.graph.all;
                self.load_graph(false);
            }
            GitAction::LoadMoreHistory => self.load_graph(true),
        }
        cx.notify();
    }

    fn commit_is_merge(&self, hash: &str) -> bool {
        self.git
            .graph
            .commits
            .iter()
            .find(|c| c.hash == hash)
            .is_some_and(|c| c.parents.len() > 1)
    }

    fn commit(
        &mut self,
        repo: PathBuf,
        amend: bool,
        then: Option<GitAction>,
        stage_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(status) = self.selected_status() else {
            return;
        };
        let input = self.message_input(&repo, window, cx);
        let mut message = input.read(cx).value().trim().to_string();
        if message.is_empty() && status.operation.is_some() {
            message = git::prepared_message(&repo).unwrap_or_default();
        }
        if message.is_empty() && !amend {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.say("Type a commit message first");
            return;
        }
        if !status.conflicts.is_empty() {
            self.say("Resolve the conflicts under Merge Changes before committing");
            return;
        }
        let nothing_staged = status.staged.is_empty();
        if nothing_staged && !amend && !stage_all && status.operation.is_none() {
            if status.unstaged.is_empty() {
                self.say("There are no changes to commit");
                return;
            }
            self.git.confirm = Some(Confirm {
                title: "Nothing is staged".into(),
                message: "Stage all your changes and commit them directly?".into(),
                detail: None,
                buttons: vec![
                    Button::cancel(),
                    Button::new(
                        "Stage All & Commit",
                        Style::Primary,
                        Some(GitAction::Commit {
                            amend,
                            then: then.map(Box::new),
                            stage_all: true,
                        }),
                    ),
                ],
            });
            return;
        }
        self.run_git(repo, Some("Committing…"), OnError::Show, move |repo| {
            if stage_all {
                ops::stage_all(repo)?;
            }
            ops::commit(repo, &message, amend)?;
            Ok(After::Committed(then))
        });
    }

    fn checkout(
        &mut self,
        repo: PathBuf,
        target: String,
        kind: CheckoutKind,
        mode: CheckoutMode,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        let action = GitAction::Checkout {
            target: target.clone(),
            kind,
            mode,
            confirmed: true,
        };
        if !confirmed && self.confirm_agent(&repo, "Switching branches", action) {
            return;
        }
        let local = match kind {
            CheckoutKind::Remote => target.split_once('/').map(|(_, b)| b.to_string()),
            _ => Some(target.clone()),
        };
        if kind != CheckoutKind::Detached
            && let Some(branch) = &local
            && let Some(place) = self.worktree_of_branch(&repo, branch)
        {
            self.git.confirm = Some(Confirm {
                title: format!("{branch} is checked out in another worktree"),
                message: format!(
                    "Git allows a branch in one worktree at a time. It is open in {}.",
                    place.display()
                ),
                detail: None,
                buttons: vec![
                    Button::cancel(),
                    Button::new("Open in New Tab", Style::Primary, Some(GitAction::OpenTab(place))),
                ],
            });
            cx.notify();
            return;
        }
        let local_exists = local.as_ref().is_some_and(|b| {
            self.git
                .refs
                .get(&repo)
                .is_some_and(|refs| refs.iter().any(|r| r.kind == RefKind::Local && &r.name == b))
        });
        let label = format!("Switching to {target}…");
        let error = OnError::Checkout(target.clone(), kind);
        let dirty_paths: Vec<String> = self
            .selected_status()
            .map(|s| {
                s.staged
                    .iter()
                    .chain(&s.unstaged)
                    .map(|e| e.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.run_git(repo, Some(&label), error, move |repo| {
            let switch = |repo: &Path, discard: bool| match kind {
                CheckoutKind::Detached => ops::switch(repo, &target, true, discard),
                CheckoutKind::Remote if !local_exists => {
                    if discard {
                        ops::switch(repo, local.as_deref().unwrap_or(&target), false, true)
                            .or_else(|_| ops::switch_tracking(repo, &target))
                    } else {
                        ops::switch_tracking(repo, &target)
                    }
                }
                _ => ops::switch(repo, local.as_deref().unwrap_or(&target), false, discard),
            };
            match mode {
                CheckoutMode::Normal => {
                    switch(repo, false)?;
                }
                CheckoutMode::Stash => {
                    ops::stash(repo, &format!("Vyber: before switching to {target}"), true, false)?;
                    switch(repo, false)?;
                    return Ok(After::Notice(format!(
                        "Switched to {target} · your changes are in the latest stash"
                    )));
                }
                CheckoutMode::Migrate => {
                    ops::stash(repo, &format!("Vyber: carried to {target}"), true, false)?;
                    switch(repo, false)?;
                    if let Err(e) = ops::stash_action(repo, "pop", "stash@{0}") {
                        return Ok(After::Notice(format!(
                            "Switched to {target} · your changes conflicted and stay in the latest stash ({})",
                            e.message.lines().next().unwrap_or_default()
                        )));
                    }
                    return Ok(After::Notice(format!("Switched to {target} with your changes")));
                }
                CheckoutMode::Force => {
                    for path in &dirty_paths {
                        if let Ok(full) = workspace::safe_path(repo, path)
                            && full.is_file()
                        {
                            workspace::keep_recovery(&full, std::fs::read(&full).ok().as_deref())?;
                        }
                    }
                    switch(repo, true)?;
                    return Ok(After::Notice(format!(
                        "Switched to {target} · discarded changes have recovery copies"
                    )));
                }
            }
            Ok(After::Notice(format!("Switched to {target}")))
        });
    }

    // ---- Quick pick ----------------------------------------------------------

    pub(super) fn open_quick(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.selected.clone() else {
            return;
        };
        self.git.menu = None;
        let (title, placeholder, text, value) = quick_prompt(&ask, self);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => this.accept_quick(window, cx),
            InputEvent::Change => {
                if let Some(quick) = &mut this.git.quick {
                    quick.selected = 0;
                    quick.scroll.scroll_to_item(0);
                }
                cx.notify();
            }
            _ => {}
        });
        let needs_refs = ask.needs_refs() && !self.git.refs.contains_key(&repo);
        let items = if needs_refs { vec![] } else { self.quick_items(&ask) };
        if needs_refs || matches!(ask, Ask::Checkout) {
            self.load_refs(repo);
        }
        self.git.quick = Some(Quick {
            ask,
            title,
            input,
            items,
            selected: 0,
            text,
            loading: needs_refs,
            scroll: ScrollHandle::new(),
        });
        self.git.quick_subscription = Some(subscription);
        cx.notify();
    }

    pub(super) fn close_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.git.quick = None;
        self.git.quick_subscription = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Items matching the typed filter, with their index in `items`.
    pub(super) fn quick_visible(&self, cx: &App) -> Vec<usize> {
        let Some(quick) = &self.git.quick else {
            return vec![];
        };
        if quick.text {
            return (0..quick.items.len()).collect();
        }
        let query = quick.input.read(cx).value().trim().to_lowercase();
        quick
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty()
                    || item.label.to_lowercase().contains(&query)
                    || item.detail.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn move_quick(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.quick_visible(cx).len();
        if let Some(quick) = &mut self.git.quick
            && count > 0
        {
            quick.selected = (quick.selected as isize + delta).rem_euclid(count as isize) as usize;
            quick.scroll.scroll_to_item(quick.selected);
            cx.notify();
        }
    }

    pub(super) fn accept_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(quick) = &self.git.quick else {
            return;
        };
        let typed = quick.input.read(cx).value().trim().to_string();
        let ask = quick.ask.clone();
        let action = if quick.text {
            if typed.is_empty() && !matches!(ask, Ask::StashMessage { .. } | Ask::TagMessage(..)) {
                return;
            }
            text_action(&ask, typed)
        } else {
            let visible = self.quick_visible(cx);
            let Some(index) = visible.get(quick.selected.min(visible.len().saturating_sub(1))) else {
                return;
            };
            Some(quick.items[*index].action.clone())
        };
        self.close_quick(window, cx);
        if let Some(action) = action {
            self.perform(action, window, cx);
        }
    }

    pub(super) fn quick_items(&self, ask: &Ask) -> Vec<QuickItem> {
        let Some(repo) = self.git.selected.clone() else {
            return vec![];
        };
        let refs = self.git.refs.get(&repo).cloned().unwrap_or_default();
        let item = |icon, label: String, detail: String, right: String, section, action| QuickItem {
            icon,
            label,
            detail,
            right,
            section,
            action,
        };
        let ref_item = |r: &Ref, section: Option<&'static str>, action: GitAction| {
            let mut detail = r.subject.clone();
            if r.kind == RefKind::Local && (r.ahead > 0 || r.behind > 0) {
                detail = format!("↓{} ↑{} · {detail}", r.behind, r.ahead);
            }
            item(
                match r.kind {
                    RefKind::Local => "git-branch",
                    RefKind::Remote => "cloud",
                    RefKind::Tag => "tag",
                },
                r.name.clone(),
                detail,
                git::age(r.time),
                section,
                action,
            )
        };
        let refs_of = |kinds: &[RefKind], skip_current: bool| -> Vec<&Ref> {
            refs.iter()
                .filter(|r| kinds.contains(&r.kind))
                .filter(|r| !(skip_current && r.head))
                .collect()
        };
        let mut items = Vec::new();
        let sections = |r: &Ref| match r.kind {
            RefKind::Local => Some("Branches"),
            RefKind::Remote => Some("Remote branches"),
            RefKind::Tag => Some("Tags"),
        };
        match ask {
            Ask::Checkout => {
                items.push(item("plus", "Create new branch…".into(), String::new(), String::new(), None, GitAction::Ask(Ask::NewBranch(None))));
                items.push(item("plus", "Create new branch from…".into(), String::new(), String::new(), None, GitAction::Ask(Ask::NewBranchFrom)));
                items.push(item("git-commit-horizontal", "Checkout detached…".into(), String::new(), String::new(), None, GitAction::Ask(Ask::Detached)));
                for r in refs_of(&[RefKind::Local, RefKind::Remote, RefKind::Tag], false) {
                    let kind = match r.kind {
                        RefKind::Local => CheckoutKind::Local,
                        RefKind::Remote => CheckoutKind::Remote,
                        RefKind::Tag => CheckoutKind::Detached,
                    };
                    let mut entry = ref_item(
                        r,
                        sections(r),
                        GitAction::Checkout {
                            target: r.name.clone(),
                            kind,
                            mode: CheckoutMode::Normal,
                            confirmed: false,
                        },
                    );
                    if r.head {
                        entry.right = "current".into();
                    } else if r.worktree.is_some() {
                        entry.right = "in a worktree".into();
                    }
                    items.push(entry);
                }
            }
            Ask::NewBranchFrom | Ask::Detached | Ask::Merge | Ask::Rebase => {
                for r in refs_of(&[RefKind::Local, RefKind::Remote, RefKind::Tag], matches!(ask, Ask::Merge | Ask::Rebase)) {
                    let action = match ask {
                        Ask::NewBranchFrom => GitAction::Ask(Ask::NewBranch(Some(r.name.clone()))),
                        Ask::Detached => GitAction::Checkout {
                            target: r.name.clone(),
                            kind: CheckoutKind::Detached,
                            mode: CheckoutMode::Normal,
                            confirmed: false,
                        },
                        Ask::Merge => GitAction::Merge(r.name.clone()),
                        _ => GitAction::Rebase(r.name.clone(), false),
                    };
                    items.push(ref_item(r, sections(r), action));
                }
            }
            Ask::RenameBranch => {
                for r in refs_of(&[RefKind::Local], false) {
                    items.push(ref_item(r, None, GitAction::Ask(Ask::RenameTo(r.name.clone()))));
                }
            }
            Ask::DeleteBranch => {
                for r in refs_of(&[RefKind::Local], true) {
                    items.push(ref_item(r, None, GitAction::DeleteBranch(r.name.clone(), false)));
                }
            }
            Ask::DeleteRemoteBranch => {
                for r in refs_of(&[RefKind::Remote], false) {
                    items.push(ref_item(r, None, GitAction::DeleteRemoteBranch(r.name.clone(), false)));
                }
            }
            Ask::DeleteTag => {
                for r in refs_of(&[RefKind::Tag], false) {
                    items.push(ref_item(r, None, GitAction::DeleteTag(r.name.clone(), false)));
                }
            }
            Ask::Publish | Ask::RemoveRemote => {
                for remote in self.selected_status().map(|s| s.remotes).unwrap_or_default() {
                    let action = if *ask == Ask::Publish {
                        GitAction::Publish(Some(remote.clone()))
                    } else {
                        GitAction::RemoveRemote(remote.clone(), false)
                    };
                    items.push(item("cloud", remote, String::new(), String::new(), None, action));
                }
            }
            Ask::Stash(action) => {
                for stash in self.git.stashes.get(&repo).cloned().unwrap_or_default() {
                    items.push(item(
                        "archive",
                        stash.message.clone(),
                        stash.name.clone(),
                        git::age(stash.time),
                        None,
                        GitAction::StashAct {
                            action,
                            name: stash.name.clone(),
                            confirmed: false,
                        },
                    ));
                }
            }
            Ask::Worktree => {
                items.push(item(
                    "plus",
                    "Create new branch…".into(),
                    "A new branch from the current one".into(),
                    String::new(),
                    None,
                    GitAction::Ask(Ask::WorktreeNewBranch),
                ));
                for r in refs_of(&[RefKind::Local], false) {
                    if r.head || r.worktree.is_some() {
                        continue;
                    }
                    items.push(ref_item(
                        r,
                        Some("Existing branches"),
                        GitAction::Ask(Ask::WorktreePath {
                            branch: r.name.clone(),
                            new: false,
                        }),
                    ));
                }
            }
            Ask::RemoveWorktree | Ask::OpenWorktree => {
                for view in self.git.repos.iter().filter(|r| r.repo.is_worktree()) {
                    let label = repo_label(&view.repo, view.status());
                    let action = if *ask == Ask::RemoveWorktree {
                        GitAction::RemoveWorktree(view.repo.path.clone(), false, false)
                    } else {
                        GitAction::OpenTab(view.repo.path.clone())
                    };
                    items.push(item("git-fork", label, view.repo.path.display().to_string(), String::new(), None, action));
                }
            }
            Ask::Reset(target) => {
                for (mode, detail) in [
                    ("soft", "Keep the changes of later commits staged"),
                    ("mixed", "Keep the changes of later commits, unstaged"),
                    ("hard", "Throw away later commits and every uncommitted change"),
                ] {
                    items.push(item(
                        if mode == "hard" { "triangle-alert" } else { "rotate-ccw" },
                        format!("Reset ({mode})"),
                        detail.into(),
                        String::new(),
                        None,
                        GitAction::Reset {
                            target: target.clone(),
                            mode,
                            confirmed: false,
                        },
                    ));
                }
            }
            Ask::SelectRepo => {
                for view in &self.git.repos {
                    let status = view.status();
                    let branch = status.map(Status::head_label).unwrap_or_default();
                    let changes = status.map(Status::changes).unwrap_or(0);
                    items.push(item(
                        if view.repo.is_worktree() { "git-fork" } else { "folder-git-2" },
                        repo_label(&view.repo, status),
                        if view.repo.is_worktree() { view.repo.name.clone() } else { branch },
                        if changes > 0 { format!("{changes} changed") } else { String::new() },
                        None,
                        GitAction::SelectRepo(view.repo.path.clone()),
                    ));
                }
            }
            _ => {}
        }
        items
    }
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
}

fn step_label(step: &str) -> &'static str {
    match step {
        "abort" => "aborted",
        "skip" => "skipped a commit",
        _ => "continued",
    }
}

/// Title, placeholder, whether Enter takes typed text, and a starting value.
fn quick_prompt(ask: &Ask, browser: &Browser) -> (String, String, bool, String) {
    let text = |title: &str, placeholder: &str, value: String| (title.to_string(), placeholder.to_string(), true, value);
    let pick = |title: &str, placeholder: &str| (title.to_string(), placeholder.to_string(), false, String::new());
    match ask {
        Ask::Checkout => pick("Switch branch", "Select a branch or tag to check out"),
        Ask::NewBranch(Some(from)) => text(&format!("New branch from {from}"), "Branch name", String::new()),
        Ask::NewBranch(None) => text("New branch", "Branch name", String::new()),
        Ask::NewBranchFrom => pick("Create branch from…", "Select a ref to start the branch from"),
        Ask::Detached => pick("Checkout detached", "Select a ref to check out without a branch"),
        Ask::Merge => pick("Merge into the current branch", "Select a branch to merge"),
        Ask::Rebase => pick("Rebase the current branch", "Select a branch to rebase onto"),
        Ask::RenameBranch => pick("Rename branch", "Select a branch to rename"),
        Ask::RenameTo(old) => text(&format!("Rename {old}"), "New branch name", old.clone()),
        Ask::DeleteBranch => pick("Delete branch", "Select a branch to delete"),
        Ask::DeleteRemoteBranch => pick("Delete remote branch", "Select a remote branch to delete"),
        Ask::Publish => pick("Publish branch", "Select a remote to publish to"),
        Ask::AddRemote => text("Add remote", "Remote name, such as origin", String::new()),
        Ask::RemoteUrl(name) => text(&format!("URL of {name}"), "https://… or git@…", String::new()),
        Ask::RemoveRemote => pick("Remove remote", "Select a remote"),
        Ask::StashMessage { .. } => text("Stash", "Message (optional)", String::new()),
        Ask::Stash(action) => pick(
            match *action {
                "pop" => "Pop stash",
                "drop" => "Drop stash",
                _ => "Apply stash",
            },
            "Select a stash",
        ),
        Ask::CreateTag(_) => text("Create tag", "Tag name", String::new()),
        Ask::TagMessage(name, _) => text(&format!("Message for {name}"), "Message (optional; empty makes a lightweight tag)", String::new()),
        Ask::DeleteTag => pick("Delete tag", "Select a tag"),
        Ask::Worktree => pick("Create worktree", "Select the branch the worktree checks out"),
        Ask::WorktreeNewBranch => text("New branch for the worktree", "Branch name", String::new()),
        Ask::WorktreePath { branch, .. } => {
            let repo = browser.git.selected.clone().unwrap_or_default();
            let main = browser
                .git
                .repo(&repo)
                .and_then(|r| r.repo.worktree_of.clone())
                .unwrap_or(repo);
            text(
                &format!("Worktree folder for {branch}"),
                "Folder path",
                ops::worktree_location(&main, branch).display().to_string(),
            )
        }
        Ask::RemoveWorktree => pick("Remove worktree", "Select a worktree"),
        Ask::OpenWorktree => pick("Open worktree", "Select a worktree to open in a new tab"),
        Ask::Reset(target) => pick(&format!("Reset to {}", short(target)), "Select how to reset"),
        Ask::SelectRepo => pick("Repository", "Select a repository"),
    }
}

/// The action for text typed into a prompt.
fn text_action(ask: &Ask, typed: String) -> Option<GitAction> {
    Some(match ask {
        Ask::WorktreeNewBranch => GitAction::Ask(Ask::WorktreePath {
            branch: typed,
            new: true,
        }),
        Ask::NewBranch(from) => GitAction::CreateBranch {
            name: typed,
            from: from.clone(),
        },
        Ask::RenameTo(old) => GitAction::RenameBranch(old.clone(), typed),
        Ask::AddRemote => GitAction::Ask(Ask::RemoteUrl(typed)),
        Ask::RemoteUrl(name) => GitAction::AddRemote(name.clone(), typed),
        Ask::StashMessage { untracked, staged } => GitAction::Stash {
            message: typed,
            untracked: *untracked,
            staged: *staged,
            confirmed: false,
        },
        Ask::CreateTag(target) => GitAction::Ask(Ask::TagMessage(typed, target.clone())),
        Ask::TagMessage(name, target) => GitAction::CreateTag {
            name: name.clone(),
            target: target.clone(),
            message: (!typed.is_empty()).then_some(typed),
        },
        Ask::WorktreePath { branch, new } => GitAction::AddWorktree {
            branch: branch.clone(),
            new: *new,
            path: PathBuf::from(typed),
        },
        _ => return None,
    })
}
