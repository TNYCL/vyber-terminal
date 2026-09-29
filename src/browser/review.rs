//! The Review tab, laid out like Codex's review pane: every file a source
//! changed in one scrolling diff, the changed-file tree beside it, and a menu
//! to pick the source (the last agent turn, the working tree, the index, a
//! commit or the branch).
use super::{Browser, BrowserEvent, Menu, Message, View, document::empty_state};
use crate::{
    changeset::{self, Blob, Commit, FileChange, FileDiff, Restore, Source},
    icons,
    tasks::{BASELINE_NOTE, TaskReview},
    theme::*,
    workspace::{Checkpoint, DiffLine},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Sizable, Theme,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const ADDED_BG: u32 = 0x0f2418;
const ADDED_TEXT: u32 = 0xc3e8cd;
const DELETED_BG: u32 = 0x2a1316;
const DELETED_TEXT: u32 = 0xf2c3c7;
const CONTEXT_TEXT: u32 = 0xc8c8c8;
const EMPTY_BG: u32 = 0x0d0d0d;
const MATCH_BG: u32 = 0x6b5418;
/// Unchanged lines shown around each change.
const CONTEXT: usize = 3;
/// Hidden runs shorter than this are shown instead of folded.
const MIN_GAP: usize = 4;
const HEADER_H: f32 = 36.;
const LINE_H: f32 = 20.;
const GAP_H: f32 = 30.;
const NOTE_H: f32 = 32.;
const COMMENT_H: f32 = 46.;
const SPACER_H: f32 = 12.;
const TREE_ROW: f32 = 28.;
const GUTTER: f32 = 48.;
/// Files read and diffed per message while a review loads.
const CHUNK: usize = 12;

/// One row of the diff list. Rows are small and comparable so a reload
/// can splice only what changed and keep the scroll position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Row {
    Header(usize),
    /// Loading, binary, too large or unchanged content.
    Note(usize),
    Gap {
        file: usize,
        start: usize,
        end: usize,
    },
    Line {
        file: usize,
        line: usize,
    },
    Pair {
        file: usize,
        old: Option<usize>,
        new: Option<usize>,
    },
    Comment {
        file: usize,
        line: usize,
    },
    Spacer(usize),
}

impl Row {
    fn file(self) -> usize {
        match self {
            Row::Header(file) | Row::Note(file) | Row::Spacer(file) => file,
            Row::Gap { file, .. }
            | Row::Line { file, .. }
            | Row::Pair { file, .. }
            | Row::Comment { file, .. } => file,
        }
    }
}

#[derive(Clone)]
pub(super) struct ReviewFile {
    pub change: FileChange,
    pub diff: Option<Arc<FileDiff>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Submenu {
    Turns,
    Commits,
}

pub(super) struct ReviewState {
    /// The Git view's review (the other one is the Review tab's).
    pub(super) git: bool,
    /// Repository the Git view compares; `None` is the workspace root.
    pub(super) repo: Option<PathBuf>,
    pub(super) source: Option<Source>,
    /// Switch to each new agent turn as it starts.
    follow_last: bool,
    pub(super) generation: Arc<AtomicU64>,
    loading: bool,
    error: Option<String>,
    base: String,
    /// Folder the file paths are relative to.
    root: PathBuf,
    files: Rc<Vec<ReviewFile>>,
    rows: Rc<Vec<Row>>,
    pub(super) rows_dirty: bool,
    list: ListState,
    /// Collapsed files, by path.
    collapsed: HashSet<String>,
    /// Unfolded runs of unchanged lines, by path and first line.
    expanded: HashSet<(String, usize)>,
    closed_dirs: HashSet<String>,
    filter: Entity<InputState>,
    find: Entity<InputState>,
    find_open: bool,
    matches: Vec<usize>,
    match_index: Option<usize>,
    split: bool,
    all_lines: bool,
    wrap: bool,
    comment: Option<(usize, usize)>,
    comment_input: Entity<InputState>,
    commits: Option<Result<Vec<Commit>, String>>,
    submenu: Option<Submenu>,
    /// A load is due: `true` starts over, `false` refreshes in place.
    pub(super) pending: Option<bool>,
    reload_at: Option<Instant>,
    reload_first: Option<Instant>,
    confirm_revert: Option<Instant>,
    tree_scroll: UniformListScrollHandle,
    /// A file to scroll to once it is listed.
    pub(super) reveal_path: Option<String>,
}

impl ReviewState {
    pub(super) fn new(
        git: bool,
        window: &mut Window,
        cx: &mut Context<Browser>,
    ) -> (Self, Vec<Subscription>) {
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter files…")
                .clean_on_escape()
        });
        let find = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Find in diff")
                .clean_on_escape()
        });
        let comment_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Comment for the agent · Enter adds it to the terminal input")
        });
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.review.rows_dirty = true;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&find, window, |this, _, event, _, cx| match event {
                InputEvent::Change => {
                    this.review.match_index = None;
                    this.update_matches(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { shift, .. } => this.next_match(!*shift, cx),
                _ => {}
            }),
            cx.subscribe_in(&comment_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit_comment(window, cx);
                }
            }),
        ];
        let state = Self {
            git,
            repo: None,
            source: None,
            follow_last: true,
            // Loads of the two reviews never share a generation number.
            generation: Arc::new(AtomicU64::new(if git { 1 << 40 } else { 0 })),
            loading: false,
            error: None,
            base: String::new(),
            root: PathBuf::new(),
            files: Rc::new(vec![]),
            rows: Rc::new(vec![]),
            rows_dirty: false,
            list: ListState::new(0, ListAlignment::Top, px(800.)),
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            closed_dirs: HashSet::new(),
            filter,
            find,
            find_open: false,
            matches: vec![],
            match_index: None,
            split: false,
            all_lines: false,
            wrap: false,
            comment: None,
            comment_input,
            commits: None,
            submenu: None,
            pending: None,
            reload_at: None,
            reload_first: None,
            confirm_revert: None,
            tree_scroll: UniformListScrollHandle::new(),
            reveal_path: None,
        };
        (state, subscriptions)
    }
}

impl Browser {
    // ---- Source and loading -------------------------------------------------

    pub(super) fn reset_review(&mut self) {
        let review = &mut self.review;
        review.generation.fetch_add(1, Ordering::Relaxed);
        review.source = None;
        review.follow_last = true;
        review.files = Rc::new(vec![]);
        review.rows = Rc::new(vec![]);
        review.list.reset(0);
        review.loading = false;
        review.error = None;
        review.pending = None;
        review.reload_at = None;
        review.reload_first = None;
        review.collapsed.clear();
        review.expanded.clear();
        review.comment = None;
        review.commits = None;
        review.matches.clear();
    }

    /// The newest agent turn; checkpoint reviews are not turns.
    fn latest_turn(&self) -> Option<&TaskReview> {
        self.tasks.iter().rev().find(|t| t.agent != "Manual")
    }

    /// Folder the review's Git sources read from.
    pub(super) fn review_root(&self) -> PathBuf {
        self.review.repo.clone().unwrap_or_else(|| self.root.clone())
    }

    fn shown_task(&self) -> Option<&TaskReview> {
        match &self.review.source {
            Some(Source::Turn(id)) => self.tasks.iter().find(|t| &t.id == id),
            _ => None,
        }
    }

    fn default_source(&self) -> Source {
        if self.review.git {
            return Source::Uncommitted;
        }
        self.latest_turn()
            .map(|t| Source::Turn(t.id.clone()))
            .unwrap_or(Source::Uncommitted)
    }

    /// Opens the Review tab, loading the last turn (or the working tree) the
    /// first time.
    pub fn show_review(&mut self, cx: &mut Context<Self>) {
        self.view = View::Review;
        self.view_changed();
        self.menu = None;
        if self.review.source.is_none() {
            self.review.source = Some(self.default_source());
            self.review.pending = Some(true);
        }
        self.review_tick(cx);
        cx.notify();
    }

    /// Opens the Review tab on one turn, as the terminal's turn badge does.
    pub fn show_turn(&mut self, task: TaskReview, cx: &mut Context<Self>) {
        let id = task.id.clone();
        self.upsert_task(task);
        let latest = self.latest_turn().is_some_and(|t| t.id == id);
        self.view = View::Review;
        self.view_changed();
        self.set_source(Source::Turn(id), latest, cx);
    }

    /// Returns whether the task is new.
    fn upsert_task(&mut self, task: TaskReview) -> bool {
        match self.tasks.iter_mut().find(|t| t.id == task.id) {
            Some(old) => {
                *old = task;
                false
            }
            None => {
                self.tasks.push(task);
                true
            }
        }
    }

    pub fn update_task(&mut self, task: TaskReview, cx: &mut Context<Self>) {
        let id = task.id.clone();
        let starts = task.active && task.after.is_none();
        let new = self.upsert_task(task);
        // Turns belong to the Review tab's review, even while the Git view shows.
        let swap = self.review.git;
        if swap {
            std::mem::swap(&mut self.review, &mut self.parked);
        }
        let shown = matches!(&self.review.source, Some(Source::Turn(s)) if *s == id);
        if new && starts && self.review.follow_last && self.review.source.is_some() {
            self.review.source = Some(Source::Turn(id));
            self.review.pending = Some(true);
        } else if shown {
            self.schedule_reload();
        }
        if swap {
            std::mem::swap(&mut self.review, &mut self.parked);
        }
        cx.notify();
    }

    pub(super) fn set_source(&mut self, source: Source, follow_last: bool, cx: &mut Context<Self>) {
        self.review.source = Some(source);
        self.review.follow_last = follow_last;
        self.review.pending = Some(true);
        self.review.submenu = None;
        self.review.confirm_revert = None;
        self.menu = None;
        self.review_tick(cx);
        cx.notify();
    }

    /// Files on disk or the Git index changed.
    pub(super) fn review_worktree_changed(&mut self, index: bool) {
        let Some(source) = &self.review.source else {
            return;
        };
        if source.follows_worktree() || (index && *source == Source::Staged) {
            self.schedule_reload();
        }
    }

    fn schedule_reload(&mut self) {
        let now = Instant::now();
        self.review.reload_first.get_or_insert(now);
        self.review.reload_at = Some(now);
    }

    pub(super) fn review_reload_now(&mut self) {
        self.review.pending.get_or_insert(false);
    }

    /// Starts a due load while the Review tab is visible. Returns whether
    /// anything changed that needs a redraw.
    pub(super) fn review_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if self
            .review
            .confirm_revert
            .is_some_and(|at| at.elapsed() > Duration::from_secs(4))
        {
            self.review.confirm_revert = None;
            changed = true;
        }
        if !self.visible || !matches!(self.view, View::Review | View::Git) {
            return changed;
        }
        let due = self
            .review
            .reload_at
            .is_some_and(|at| at.elapsed() > Duration::from_millis(350))
            || self
                .review
                .reload_first
                .is_some_and(|at| at.elapsed() > Duration::from_secs(2));
        let fresh = match self.review.pending.take() {
            Some(fresh) => fresh,
            None if due => false,
            None => return changed,
        };
        self.load_review(fresh, cx);
        true
    }

    fn load_review(&mut self, fresh: bool, cx: &mut Context<Self>) {
        let source = match &self.review.source {
            Some(source) => source.clone(),
            None => {
                let source = self.default_source();
                self.review.source = Some(source.clone());
                source
            }
        };
        let generation = self.review.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.review.reload_at = None;
        self.review.reload_first = None;
        if fresh {
            self.review.files = Rc::new(vec![]);
            self.review.rows = Rc::new(vec![]);
            self.review.list.reset(0);
            self.review.expanded.clear();
            self.review.collapsed.clear();
            self.review.comment = None;
            self.review.matches.clear();
            self.review.base.clear();
        }
        self.review.error = None;
        self.review.loading = true;
        let task = self.shown_task().cloned();
        let root = self.review_root();
        if let Source::Commit { hash, .. } = &source {
            self.load_commit_info(root.clone(), hash.clone());
        }
        let sender = self.sender.clone();
        let current = self.review.generation.clone();
        let theme = Theme::global(cx).highlight_theme.clone();
        std::thread::spawn(move || {
            let plan = match &source {
                Source::Turn(_) => task
                    .ok_or_else(|| anyhow::anyhow!("This turn is no longer in the task history"))
                    .and_then(|task| changeset::turn_plan(&task)),
                source => changeset::git_plan(&root, source),
            };
            let plan = match plan {
                Ok(plan) => plan,
                Err(e) => {
                    let _ = sender.send(Message::ReviewError(generation, e.to_string()));
                    return;
                }
            };
            let _ = sender.send(Message::ReviewList(
                generation,
                plan.root.clone(),
                plan.base.clone(),
                plan.files.clone(),
            ));
            let mut start = 0;
            while start < plan.files.len() {
                if current.load(Ordering::Relaxed) != generation {
                    return;
                }
                let end = (start + CHUNK).min(plan.files.len());
                let diffs = match plan.read(start..end) {
                    Ok(blobs) => blobs
                        .into_iter()
                        .enumerate()
                        .map(|(n, (before, after))| {
                            let path = &plan.files[start + n].path;
                            let diff = changeset::file_diff(path, before, after, Some(&theme));
                            (start + n, Arc::new(diff))
                        })
                        .collect(),
                    Err(e) => {
                        let _ = sender.send(Message::ReviewError(generation, e.to_string()));
                        return;
                    }
                };
                if sender.send(Message::ReviewDiffs(generation, diffs)).is_err() {
                    return;
                }
                start = end;
            }
            let _ = sender.send(Message::ReviewDone(generation));
        });
        cx.notify();
    }

    fn current_generation(&self, generation: u64) -> bool {
        self.review.generation.load(Ordering::Relaxed) == generation
    }

    pub(super) fn review_listed(
        &mut self,
        generation: u64,
        root: PathBuf,
        base: String,
        files: Vec<FileChange>,
    ) {
        if !self.current_generation(generation) {
            return;
        }
        // Keep showing the previous diff of each file until its new one arrives.
        let previous: HashMap<String, Arc<FileDiff>> = self
            .review
            .files
            .iter()
            .filter_map(|f| f.diff.clone().map(|d| (f.change.path.clone(), d)))
            .collect();
        if let Some((file, _)) = self.review.comment {
            let same = self.review.files.get(file).map(|f| &f.change.path)
                == files.get(file).map(|f| &f.path);
            if !same {
                self.review.comment = None;
            }
        }
        self.review.files = Rc::new(
            files
                .into_iter()
                .map(|change| ReviewFile {
                    diff: previous.get(&change.path).cloned(),
                    change,
                })
                .collect(),
        );
        self.review.root = root;
        self.review.base = base;
        self.review.rows_dirty = true;
    }

    pub(super) fn review_diffs(&mut self, generation: u64, diffs: Vec<(usize, Arc<FileDiff>)>) {
        if !self.current_generation(generation) {
            return;
        }
        let files = Rc::make_mut(&mut self.review.files);
        for (index, diff) in diffs {
            if let Some(file) = files.get_mut(index) {
                file.diff = Some(diff);
            }
        }
        self.review.rows_dirty = true;
    }

    pub(super) fn review_done(&mut self, generation: u64) {
        if self.current_generation(generation) {
            self.review.loading = false;
        }
    }

    pub(super) fn review_failed(&mut self, generation: u64, error: String) {
        if self.current_generation(generation) {
            self.review.loading = false;
            self.review.error = Some(error);
            self.review.files = Rc::new(vec![]);
            self.review.rows_dirty = true;
        }
    }

    pub(super) fn review_commits(&mut self, commits: Result<Vec<Commit>, String>) {
        self.review.commits = Some(commits);
    }

    fn load_commits(&mut self) {
        let root = self.review_root();
        let sender = self.sender.clone();
        self.review.commits = None;
        std::thread::spawn(move || {
            let _ = sender.send(Message::Commits(
                changeset::commits(&root).map_err(|e| e.to_string()),
            ));
        });
    }

    // ---- Rows ----------------------------------------------------------------

    fn filter_text(&self, cx: &App) -> String {
        self.review.filter.read(cx).value().trim().to_lowercase()
    }

    fn build_rows(&self, cx: &App) -> Vec<Row> {
        let query = self.filter_text(cx);
        let review = &self.review;
        let mut rows = Vec::new();
        for (i, file) in review.files.iter().enumerate() {
            let path = &file.change.path;
            if !query.is_empty() && !path.to_lowercase().contains(&query) {
                continue;
            }
            rows.push(Row::Header(i));
            if review.collapsed.contains(path) {
                rows.push(Row::Spacer(i));
                continue;
            }
            let diff = match &file.diff {
                Some(diff) if diff.note.is_none() && !diff.lines.is_empty() => diff,
                _ => {
                    rows.push(Row::Note(i));
                    rows.push(Row::Spacer(i));
                    continue;
                }
            };
            let lines = &diff.lines;
            let split = review.split && file.change.letter == 'M';
            let comment = review.comment.filter(|(f, _)| *f == i).map(|(_, line)| line);
            let mut visible = vec![review.all_lines; lines.len()];
            if !review.all_lines {
                for (j, line) in lines.iter().enumerate() {
                    if matches!(line.kind, '+' | '-') {
                        let to = (j + CONTEXT + 1).min(lines.len());
                        visible[j.saturating_sub(CONTEXT)..to].fill(true);
                    }
                }
            }
            let mut j = 0;
            while j < lines.len() {
                let start = j;
                let shown = visible[j];
                while j < lines.len() && visible[j] == shown {
                    j += 1;
                }
                if shown || j - start < MIN_GAP || review.expanded.contains(&(path.clone(), start))
                {
                    push_lines(&mut rows, i, lines, start..j, split, comment);
                } else {
                    rows.push(Row::Gap {
                        file: i,
                        start,
                        end: j,
                    });
                }
            }
            rows.push(Row::Spacer(i));
        }
        rows
    }

    /// Rebuilds the rows after a change and splices only the part that
    /// differs into the list, so the scroll position stays put.
    fn sync_rows(&mut self, cx: &App) {
        if !self.review.rows_dirty {
            return;
        }
        self.review.rows_dirty = false;
        let rows = self.build_rows(cx);
        let old = self.review.rows.clone();
        let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
        let room = old.len().min(rows.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(rows.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        self.review
            .list
            .splice(prefix..old.len() - suffix, rows.len() - prefix - suffix);
        if self.review.wrap {
            self.review.list.remeasure();
        }
        self.review.rows = Rc::new(rows);
        self.update_matches(cx);
        if let Some(path) = self.review.reveal_path.clone()
            && let Some(file) = self.review.files.iter().position(|f| f.change.path == path)
            && let Some(row) = self.review.rows.iter().position(|r| *r == Row::Header(file))
        {
            self.review.reveal_path = None;
            self.review.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
        }
    }

    fn row_texts(&self, row: Row) -> [Option<&str>; 2] {
        let line = |file: usize, index: Option<usize>| {
            index.and_then(|i| {
                self.review.files[file]
                    .diff
                    .as_ref()
                    .and_then(|d| d.lines.get(i))
                    .map(|l| l.text.as_str())
            })
        };
        match row {
            Row::Header(file) => [Some(self.review.files[file].change.path.as_str()), None],
            Row::Line { file, line: index } => [line(file, Some(index)), None],
            Row::Pair { file, old, new } => [line(file, old), line(file, new)],
            _ => [None, None],
        }
    }

    fn update_matches(&mut self, cx: &App) {
        let query = self.review.find.read(cx).value().trim().to_ascii_lowercase();
        let matches = if query.is_empty() {
            vec![]
        } else {
            self.review
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    self.row_texts(**row)
                        .iter()
                        .flatten()
                        .any(|text| text.to_ascii_lowercase().contains(&query))
                })
                .map(|(i, _)| i)
                .collect()
        };
        if self.review.match_index.is_some_and(|i| i >= matches.len()) {
            self.review.match_index = None;
        }
        self.review.matches = matches;
    }

    fn next_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.review.matches.len();
        if count == 0 {
            return;
        }
        let index = match (self.review.match_index, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
        };
        self.review.match_index = Some(index);
        self.review
            .list
            .scroll_to_reveal_item(self.review.matches[index]);
        cx.notify();
    }

    fn is_change(&self, row: Row) -> bool {
        let changed = |file: usize, index: Option<usize>| {
            index.is_some_and(|i| {
                self.review.files[file]
                    .diff
                    .as_ref()
                    .and_then(|d| d.lines.get(i))
                    .is_some_and(|l| l.kind != ' ')
            })
        };
        match row {
            Row::Line { file, line } => changed(file, Some(line)),
            Row::Pair { file, old, new } => changed(file, old) || changed(file, new),
            _ => false,
        }
    }

    fn jump_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.review.rows.clone();
        let starts: Vec<usize> = (0..rows.len())
            .filter(|&i| self.is_change(rows[i]) && (i == 0 || !self.is_change(rows[i - 1])))
            .collect();
        let top = self.review.list.logical_scroll_top().item_ix;
        let target = if forward {
            starts.iter().find(|&&s| s.saturating_sub(2) > top)
        } else {
            starts.iter().rev().find(|&&s| s.saturating_sub(2) < top)
        };
        match target {
            Some(&row) => self.review.list.scroll_to(ListOffset {
                item_ix: row.saturating_sub(2),
                offset_in_item: px(0.),
            }),
            None => self.say(if forward {
                "No more changes below"
            } else {
                "No more changes above"
            }),
        }
        cx.notify();
    }

    fn reveal_file(&mut self, file: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.review.rows.iter().position(|r| *r == Row::Header(file)) {
            self.review.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    fn current_file(&self) -> Option<usize> {
        let top = self.review.list.logical_scroll_top().item_ix;
        self.review.rows.get(top).map(|row| row.file())
    }

    fn toggle_file(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.review.collapsed.remove(&path) {
            self.review.collapsed.insert(path);
        }
        self.review.rows_dirty = true;
        cx.notify();
    }

    fn expand_gap(&mut self, file: usize, start: usize, cx: &mut Context<Self>) {
        if let Some(f) = self.review.files.get(file) {
            self.review.expanded.insert((f.change.path.clone(), start));
            self.review.rows_dirty = true;
        }
        cx.notify();
    }

    // ---- Comments -------------------------------------------------------------

    fn open_comment(&mut self, file: usize, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.review.comment = Some((file, line));
        self.review.rows_dirty = true;
        self.review
            .comment_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review.comment = None;
        self.review.rows_dirty = true;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn submit_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, index)) = self.review.comment else {
            return;
        };
        let text = self.review.comment_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(file) = self.review.files.get(file) else {
            return;
        };
        let path = &file.change.path;
        let location = match file.diff.as_ref().and_then(|d| d.lines.get(index)) {
            Some(line) if line.kind == '-' => match line.old {
                Some(old) => format!("{path} (removed line {old})"),
                None => path.clone(),
            },
            Some(line) => match line.new {
                Some(new) => format!("{path}:{new}"),
                None => path.clone(),
            },
            None => path.clone(),
        };
        cx.emit(BrowserEvent::Comment(format!("{location} — {text}")));
        self.review
            .comment_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.say("Added to the terminal input · send it from the terminal");
        self.close_comment(window, cx);
    }

    // ---- Reverts --------------------------------------------------------------

    fn can_revert(&mut self) -> bool {
        if self.has_dirty() {
            self.say("Save your open editor changes before reverting.");
            return false;
        }
        if self.shown_task().is_some_and(|t| t.active) {
            self.say("Wait for the turn to finish before reverting.");
            return false;
        }
        true
    }

    fn run_revert(&mut self, done: String, work: impl FnOnce() -> anyhow::Result<()> + Send + 'static) {
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Message::Reverted(match work() {
                Ok(()) => done,
                Err(e) => e.to_string(),
            }));
        });
    }

    fn revert_file(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.can_revert() {
            cx.notify();
            return;
        }
        let Some(file) = self.review.files.get(index).cloned() else {
            return;
        };
        let task = self.shown_task().cloned();
        let root = self.review.root.clone();
        let name = file.change.path.rsplit('/').next().unwrap_or_default().to_string();
        self.run_revert(format!("Reverted {name} · recovery copy saved"), move || {
            if let Some(task) = task {
                let before = task.before.ok_or_else(|| anyhow::anyhow!("No start snapshot"))?;
                let after = task.after.ok_or_else(|| anyhow::anyhow!("No end snapshot"))?;
                return before.restore_file(&after, &file.change.path);
            }
            let diff = file
                .diff
                .ok_or_else(|| anyhow::anyhow!("Wait until the file has loaded"))?;
            changeset::restore(&[Restore {
                root,
                path: file.change.path.clone(),
                expected: diff.after.clone(),
                target: diff.before.clone(),
            }])
        });
        cx.notify();
    }

    fn revert_hunk(&mut self, index: usize, hunk: usize, cx: &mut Context<Self>) {
        if !self.can_revert() {
            cx.notify();
            return;
        }
        let Some(file) = self.review.files.get(index).cloned() else {
            return;
        };
        let Some(diff) = file.diff.clone() else {
            return;
        };
        let Some(hunk) = diff.hunks.get(hunk).cloned() else {
            return;
        };
        let root = self.review.root.clone();
        self.run_revert("Change reverted · recovery copy saved".into(), move || {
            let before = diff.before.bytes().unwrap_or_default();
            let after = diff.after.bytes().unwrap_or_default();
            let bytes = changeset::revert_hunk(before, after, &hunk)?;
            // Reverting the only change of an added file removes the file again.
            let target = if bytes.is_empty() && diff.before == Blob::Missing {
                Blob::Missing
            } else {
                Blob::Bytes(bytes)
            };
            changeset::restore(&[Restore {
                root,
                path: file.change.path.clone(),
                expected: diff.after.clone(),
                target,
            }])
        });
        cx.notify();
    }

    fn revert_all(&mut self, cx: &mut Context<Self>) {
        if self
            .review
            .confirm_revert
            .is_none_or(|at| at.elapsed() > Duration::from_secs(4))
        {
            self.review.confirm_revert = Some(Instant::now());
            cx.notify();
            return;
        }
        self.review.confirm_revert = None;
        self.menu = None;
        if !self.can_revert() {
            cx.notify();
            return;
        }
        let count = self.review.files.len();
        let done = format!("Reverted {count} files · recovery copies saved");
        if let Some(task) = self.shown_task().cloned() {
            self.run_revert(done, move || {
                let before = task.before.ok_or_else(|| anyhow::anyhow!("No start snapshot"))?;
                let after = task.after.ok_or_else(|| anyhow::anyhow!("No end snapshot"))?;
                before.restore_task(&after)
            });
        } else {
            let root = self.review.root.clone();
            let mut restores = Vec::new();
            for file in self.review.files.iter() {
                let Some(diff) = &file.diff else {
                    self.say("Wait until every file has loaded, then try again.");
                    cx.notify();
                    return;
                };
                restores.push(Restore {
                    root: root.clone(),
                    path: file.change.path.clone(),
                    expected: diff.after.clone(),
                    target: diff.before.clone(),
                });
            }
            self.run_revert(done, move || changeset::restore(&restores));
        }
        cx.notify();
    }

    // ---- Staging ------------------------------------------------------------------

    /// Stages (or unstages) one hunk of a modified file by writing the index
    /// content with only that hunk changed.
    fn stage_hunk(&mut self, index: usize, hunk: usize, stage: bool, cx: &mut Context<Self>) {
        let Some(file) = self.review.files.get(index).cloned() else {
            return;
        };
        let Some(diff) = file.diff.clone() else {
            return;
        };
        let Some(hunk) = diff.hunks.get(hunk).cloned() else {
            return;
        };
        let root = self.review.root.clone();
        let done = if stage { "Change staged" } else { "Change unstaged" };
        self.run_revert(done.into(), move || {
            let (top, path) = changeset::repository_path(&root, &file.change.path)?;
            let before = diff.before.bytes().unwrap_or_default();
            let after = diff.after.bytes().unwrap_or_default();
            if stage {
                let bytes = changeset::apply_hunk(before, after, &hunk)?;
                crate::git::ops::write_index(&top, &path, Some(before), &bytes)?;
            } else {
                let bytes = changeset::revert_hunk(before, after, &hunk)?;
                crate::git::ops::write_index(&top, &path, Some(after), &bytes)?;
            }
            Ok(())
        });
        cx.notify();
    }

    pub(super) fn stage_file(&mut self, index: usize, stage: bool, cx: &mut Context<Self>) {
        let Some(file) = self.review.files.get(index).cloned() else {
            return;
        };
        let root = self.review.root.clone();
        let name = file.change.path.rsplit('/').next().unwrap_or_default().to_string();
        let done = format!("{} {name}", if stage { "Staged" } else { "Unstaged" });
        self.run_revert(done, move || {
            let (top, path) = changeset::repository_path(&root, &file.change.path)?;
            if stage {
                crate::git::ops::stage(&top, &[path])?;
            } else {
                crate::git::ops::unstage(&top, &[path])?;
            }
            Ok(())
        });
        cx.notify();
    }

    fn copy_patch(&mut self, cx: &mut Context<Self>) {
        let files: Vec<_> = self
            .review
            .files
            .iter()
            .filter_map(|f| f.diff.as_deref().map(|d| (f.change.clone(), d)))
            .collect();
        let patch = changeset::patch(&files);
        cx.write_to_clipboard(ClipboardItem::new_string(patch));
        self.menu = None;
        self.say(format!("Patch copied · {} files", files.len()));
        cx.notify();
    }

    fn review_since_checkpoint(&mut self, cx: &mut Context<Self>) {
        let Some(before) = self.checkpoint.clone() else {
            return;
        };
        let root = self.review_root();
        self.menu = None;
        self.say("Comparing with checkpoint…");
        cx.spawn(async move |entity, cx| {
            let result = cx
                .background_spawn(async move {
                    let after = Checkpoint::capture(&root, "Checkpoint comparison")?;
                    let changes = before.changes_to(&after)?;
                    let task = TaskReview {
                        id: before.id.clone(),
                        agent: "Manual".into(),
                        label: format!("Since checkpoint · {}", before.created),
                        root,
                        session: String::new(),
                        before: Some(before),
                        after: Some(after),
                        changes,
                        active: false,
                        warning: "Includes all workspace edits since this checkpoint.".into(),
                    };
                    crate::tasks::persist(&task);
                    anyhow::Ok(task)
                })
                .await;
            let _ = entity.update(cx, |view, cx| match result {
                Ok(task) => {
                    let id = task.id.clone();
                    view.upsert_task(task);
                    view.set_source(Source::Turn(id), false, cx);
                    view.say("Checkpoint review ready");
                }
                Err(e) => {
                    view.say(e.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // ---- Rendering ------------------------------------------------------------

    fn source_label(&self) -> (String, String) {
        let base = &self.review.base;
        match &self.review.source {
            Some(Source::Turn(id)) => {
                let task = self.tasks.iter().find(|t| &t.id == id);
                let detail = task.map(|t| t.label.clone()).unwrap_or_default();
                let label = if self.review.follow_last
                    && self.latest_turn().is_some_and(|t| &t.id == id)
                {
                    "Last Turn".to_string()
                } else if task.is_some_and(|t| t.agent == "Manual") {
                    "Checkpoint".to_string()
                } else {
                    "Turn".to_string()
                };
                (label, detail)
            }
            Some(Source::Uncommitted) => ("Uncommitted".into(), "vs HEAD".into()),
            Some(Source::Unstaged) => ("Unstaged".into(), "vs index".into()),
            Some(Source::Staged) => ("Staged".into(), "vs HEAD".into()),
            Some(Source::Commit { hash, title }) => (
                hash.chars().take(7).collect(),
                title.clone(),
            ),
            Some(Source::Branch) => (
                "Branch".into(),
                if base.is_empty() {
                    String::new()
                } else {
                    format!("vs {base}")
                },
            ),
            None => ("Review".into(), String::new()),
        }
    }

    fn review_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let review = &self.review;
        let additions: usize = review.files.iter().map(|f| f.change.additions).sum();
        let deletions: usize = review.files.iter().map(|f| f.change.deletions).sum();
        let (label, detail) = self.source_label();
        let active = self.shown_task().is_some_and(|t| t.active);
        let has_files = !review.files.is_empty();
        let find_count = match (review.matches.len(), review.match_index) {
            (0, _) => String::new(),
            (n, Some(i)) => format!("{}/{n}", i + 1),
            (n, None) => n.to_string(),
        };
        let split = review.split;
        let all_lines = review.all_lines;
        let wrap = review.wrap;
        let find_open = review.find_open;
        let find = review.find.clone();
        let source_open = self.menu == Some(Menu::Source);
        div()
            .h(px(46.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .pl_3()
            .pr_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .child(
                div()
                    .id("review-source")
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1p5()
                    .h(px(28.))
                    .pl_3()
                    .pr_2()
                    .rounded_full()
                    .bg(rgb(if source_open { SELECTED } else { SURFACE }))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(12.5))
                    .text_color(rgb(TEXT))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(SELECTED)))
                    .when(active, |s| {
                        s.child(
                            div()
                                .size(px(6.))
                                .rounded_full()
                                .bg(rgb(TEXT_2))
                                .with_animation(
                                    "review-turn-active",
                                    Animation::new(Duration::from_millis(1400))
                                        .repeat()
                                        .with_easing(pulsating_between(0.25, 1.)),
                                    |el, t| el.opacity(t),
                                ),
                        )
                    })
                    .child(label)
                    .child(icon(ui("chevron-down"), MUTED, 13.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_menu(Menu::Source);
                        this.review.submenu = None;
                        cx.notify();
                    })),
            )
            .when(has_files, |s| {
                s.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.5))
                        .text_color(rgb(ADDED))
                        .child(format!("+{additions}")),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.5))
                        .text_color(rgb(DELETED))
                        .child(format!("−{deletions}")),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(detail),
            )
            .when(find_open, |s| {
                s.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1p5()
                        .w(px(230.))
                        .h(px(28.))
                        .pl_2()
                        .pr_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x3a3a3a))
                        .child(icon(ui("search"), MUTED, 13.))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&find)
                                    .small()
                                    .appearance(false)
                                    .text_size(px(12.5)),
                            ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child(find_count),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_0p5()
                    .p_0p5()
                    .rounded_full()
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        tool("review-more", "ellipsis", "More actions", self.menu == Some(Menu::More))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_menu(Menu::More);
                                this.review.confirm_revert = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        tool("review-find", "text-search", "Find in diff", find_open).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.review.find_open = !this.review.find_open;
                                if this.review.find_open {
                                    this.review
                                        .find
                                        .update(cx, |input, cx| input.focus(window, cx));
                                } else {
                                    this.review
                                        .find
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    this.review.matches.clear();
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(tool("review-refresh", "refresh-cw", "Refresh", false).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.review.pending.get_or_insert(false);
                            this.review_tick(cx);
                        }),
                    ))
                    .child(
                        tool("review-prev", "arrow-up", "Previous change", false)
                            .on_click(cx.listener(|this, _, _, cx| this.jump_change(false, cx))),
                    )
                    .child(
                        tool("review-next", "arrow-down", "Next change", false)
                            .on_click(cx.listener(|this, _, _, cx| this.jump_change(true, cx))),
                    )
                    .child(
                        tool(
                            "review-context",
                            if all_lines { "fold-vertical" } else { "unfold-vertical" },
                            if all_lines {
                                "Fold unchanged lines"
                            } else {
                                "Show every line"
                            },
                            all_lines,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review.all_lines = !this.review.all_lines;
                            this.review.rows_dirty = true;
                            cx.notify();
                        })),
                    )
                    .child(
                        tool(
                            "review-split",
                            "columns-2",
                            if split { "Unified view" } else { "Split view" },
                            split,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review.split = !this.review.split;
                            this.review.rows_dirty = true;
                            cx.notify();
                        })),
                    )
                    .child(
                        tool(
                            "review-wrap",
                            "wrap-text",
                            if wrap { "Don't wrap long lines" } else { "Wrap long lines" },
                            wrap,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review.wrap = !this.review.wrap;
                            this.review.list.remeasure();
                            cx.notify();
                        })),
                    ),
            )
    }

    fn review_banner(&self) -> Option<AnyElement> {
        let task = self.shown_task()?;
        let warning = task.warning.replace(BASELINE_NOTE, "").trim().to_string();
        let text = if !warning.is_empty() {
            warning
        } else if task.active {
            "The agent is still working · this review updates as files change".into()
        } else {
            return None;
        };
        let color = if task.warning.replace(BASELINE_NOTE, "").trim().is_empty() {
            MUTED
        } else {
            WARNING
        };
        Some(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .h(px(30.))
                .px_3()
                .border_b_1()
                .border_color(rgb(DIVIDER))
                .when(color == WARNING, |s| s.bg(rgb(WARNING_BG)))
                .text_size(px(11.5))
                .text_color(rgb(color))
                .child(icon(
                    ui(if color == WARNING { "triangle-alert" } else { "circle-dot" }),
                    color,
                    13.,
                ))
                .child(div().min_w_0().truncate().child(text))
                .into_any_element(),
        )
    }

    fn empty_review(&self) -> AnyElement {
        if let Some(error) = &self.review.error {
            return empty_state("triangle-alert", "Can't show these changes", error, None);
        }
        if self.review.loading {
            return empty_state("git-compare", "Reading changes…", "", None);
        }
        let active = self.shown_task().is_some_and(|t| t.active);
        let base = format!("Nothing differs from {}", self.review.base);
        let (title, detail, hint) = match &self.review.source {
            Some(Source::Turn(_)) if active => (
                "No changes yet",
                "The agent hasn't changed any files in this turn so far",
                None,
            ),
            Some(Source::Turn(_)) => ("No changes in this turn", "No files were changed", None),
            Some(Source::Unstaged) => (
                "No unstaged changes",
                "The working tree matches the index",
                None,
            ),
            Some(Source::Staged) => ("Nothing staged", "The index matches HEAD", None),
            Some(Source::Commit { .. }) => (
                "No file changes",
                "This commit changes nothing in this folder",
                None,
            ),
            Some(Source::Branch) => ("No branch changes", base.as_str(), None),
            _ => (
                "No uncommitted changes",
                "The working tree matches HEAD",
                (self.tasks.is_empty() && !self.review.git).then_some(
                    "Agent turns appear here when Claude or Codex starts a request in this folder",
                ),
            ),
        };
        empty_state("git-compare", title, detail, hint)
    }

    pub(super) fn review_content(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.sync_rows(cx);
        let bar = self.review_bar(cx).into_any_element();
        let banner = self.review_banner();
        let body = if self.review.files.is_empty() {
            self.empty_review()
        } else if self.review.rows.is_empty() {
            empty_state("search", "No files match", "Change the filter to see more files", None)
        } else {
            self.review_list(cx)
        };
        let card = self.commit_card(cx);
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .children(banner)
            .children(card)
            .child(div().flex_1().min_h_0().child(body))
            .when(self.menu == Some(Menu::Source), |s| s.child(self.source_menu(cx)))
            .when(self.menu == Some(Menu::More), |s| s.child(self.more_menu(cx)))
            .into_any_element()
    }

    fn review_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let editable = self
            .review
            .source
            .as_ref()
            .is_some_and(Source::editable);
        let (stage_files, stage_hunks) = match self.review.source {
            Some(Source::Unstaged) => (Some(true), Some(true)),
            Some(Source::Uncommitted) => (Some(true), None),
            Some(Source::Staged) => (Some(false), Some(false)),
            _ => (None, None),
        };
        let context = Rc::new(RowContext {
            rows: self.review.rows.clone(),
            files: self.review.files.clone(),
            entity: cx.entity(),
            collapsed: self.review.collapsed.clone(),
            wrap: self.review.wrap,
            editable,
            stage_files,
            stage_hunks,
            query: if self.review.find_open {
                self.review.find.read(cx).value().trim().to_ascii_lowercase()
            } else {
                String::new()
            },
            comment_input: self.review.comment_input.clone(),
        });
        list(self.review.list.clone(), move |ix, _, _| {
            render_row(ix, &context)
        })
        .size_full()
        .into_any_element()
    }

    fn source_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.review.source.clone();
        let follow = self.review.follow_last;
        let latest = self.latest_turn().cloned();
        let turn_count = self.tasks.len();
        let is = |s: &Source| source.as_ref() == Some(s);
        let last_checked = follow && latest.as_ref().is_some_and(|t| is(&Source::Turn(t.id.clone())));
        // Agent turns belong to the terminal, not to a repository of the Git view.
        let turns = !self.review.git;
        let mut menu = menu_panel("review-source-menu").w(px(250.));
        if turns {
        menu = menu.child(
            menu_item(
                "source-last",
                "Last Turn",
                Some(
                    latest
                        .as_ref()
                        .map(|t| t.label.clone())
                        .unwrap_or_else(|| "No agent turns yet".into()),
                ),
                last_checked,
                latest.is_some(),
                false,
            )
            .when_some(latest.map(|t| t.id), |s, id| {
                s.on_click(cx.listener(move |this, _, _, cx| {
                    this.set_source(Source::Turn(id.clone()), true, cx)
                }))
            }),
        );
        if turn_count > 1 {
            menu = menu.child(
                menu_item(
                    "source-turns",
                    "Earlier turns",
                    None,
                    !last_checked && matches!(source, Some(Source::Turn(_))),
                    true,
                    true,
                )
                .when(self.review.submenu == Some(Submenu::Turns), |s| s.bg(rgb(HOVER)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.review.submenu = Some(Submenu::Turns);
                    cx.notify();
                })),
            );
        }
        menu = menu.child(separator());
        }
        for (id, label, value) in [
            ("source-uncommitted", "Uncommitted", Source::Uncommitted),
            ("source-unstaged", "Unstaged", Source::Unstaged),
            ("source-staged", "Staged", Source::Staged),
        ] {
            let checked = is(&value);
            menu = menu.child(
                menu_item(id, label, None, checked, true, false).on_click(cx.listener(
                    move |this, _, _, cx| this.set_source(value.clone(), false, cx),
                )),
            );
        }
        menu = menu
            .child(separator())
            .child(
                menu_item(
                    "source-committed",
                    "Committed",
                    None,
                    matches!(source, Some(Source::Commit { .. })),
                    true,
                    true,
                )
                .when(self.review.submenu == Some(Submenu::Commits), |s| s.bg(rgb(HOVER)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.review.submenu = Some(Submenu::Commits);
                    this.load_commits();
                    cx.notify();
                })),
            )
            .child(
                menu_item(
                    "source-branch",
                    "Branch",
                    Some("Everything since the default branch".into()),
                    is(&Source::Branch),
                    true,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_source(Source::Branch, false, cx))),
            );
        // Open each submenu level with the item that opened it.
        let turns_row = 44.;
        let commits_row = if turns {
            turns_row + if turn_count > 1 { 30. } else { 0. } + 9. + 90. + 9.
        } else {
            4. + 90. + 9.
        };
        let submenu = match self.review.submenu {
            Some(Submenu::Turns) => Some(div().mt(px(turns_row)).child(self.turns_menu(cx))),
            Some(Submenu::Commits) => Some(div().mt(px(commits_row)).child(self.commits_menu(cx))),
            None => None,
        };
        div()
            .id("review-source-menus")
            .absolute()
            .occlude()
            .top(px(42.))
            .left(px(10.))
            .flex()
            .items_start()
            .gap_1()
            .child(menu)
            .children(submenu)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_menu();
                this.review.submenu = None;
                cx.notify();
            }))
    }

    fn turns_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match &self.review.source {
            Some(Source::Turn(id)) if !self.review.follow_last => Some(id.clone()),
            _ => None,
        };
        let items = self
            .tasks
            .iter()
            .rev()
            .take(40)
            .enumerate()
            .map(|(i, task)| {
                let id = task.id.clone();
                let files = task.changes.len();
                let time = task.before.as_ref().map(|b| b.created.clone()).unwrap_or_default();
                let detail = format!(
                    "{time} · {files} {}{}",
                    if files == 1 { "file" } else { "files" },
                    if task.active { " · working" } else { "" }
                );
                menu_item(
                    ("turn", i),
                    task.label.clone(),
                    Some(detail),
                    shown.as_ref() == Some(&task.id),
                    true,
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_source(Source::Turn(id.clone()), false, cx)
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>();
        menu_panel("review-turns-menu")
            .w(px(330.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .children(items)
    }

    fn commits_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match &self.review.source {
            Some(Source::Commit { hash, .. }) => Some(hash.clone()),
            _ => None,
        };
        let body: Vec<AnyElement> = match &self.review.commits {
            None => vec![menu_note("Reading commits…")],
            Some(Err(e)) => vec![menu_note(if e.contains("does not have any commits") {
                "No commits yet"
            } else {
                "Commits are unavailable here"
            })],
            Some(Ok(commits)) if commits.is_empty() => vec![menu_note("No commits touch this folder")],
            Some(Ok(commits)) => commits
                .iter()
                .enumerate()
                .map(|(i, commit)| {
                    let source = Source::Commit {
                        hash: commit.hash.clone(),
                        title: commit.title.clone(),
                    };
                    menu_item(
                        ("commit", i),
                        commit.title.clone(),
                        Some(format!("{} · {}", commit.short, commit.when)),
                        shown.as_ref() == Some(&commit.hash),
                        true,
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_source(source.clone(), false, cx)
                    }))
                    .into_any_element()
                })
                .collect(),
        };
        menu_panel("review-commits-menu")
            .w(px(330.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .children(body)
    }

    fn more_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editable = self
            .review
            .source
            .as_ref()
            .is_some_and(Source::editable);
        let count = self.review.files.len();
        let confirming = self.review.confirm_revert.is_some();
        let revert_label = if confirming {
            format!("Click again to revert {count} files")
        } else {
            "Revert all changes".to_string()
        };
        let has_checkpoint = self.checkpoint.is_some();
        let checkpoints = !self.review.git;
        div()
            .id("review-more-menus")
            .absolute()
            .occlude()
            .top(px(42.))
            .right(px(10.))
            .child(
                menu_panel("review-more-menu")
                    .w(px(250.))
                    .child(
                        menu_item("more-copy", "Copy as patch", None, false, count > 0, false)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_patch(cx))),
                    )
                    .child(
                        menu_item(
                            "more-revert",
                            revert_label,
                            (!editable).then(|| "This view is read-only".to_string()),
                            false,
                            editable && count > 0,
                            false,
                        )
                        .when(confirming, |s| s.text_color(rgb(DELETED)))
                        .when(editable && count > 0, |s| {
                            s.on_click(cx.listener(|this, _, _, cx| this.revert_all(cx)))
                        }),
                    )
                    .when(checkpoints, |s| s.child(separator()))
                    .when(checkpoints, |s| s.child(
                        menu_item(
                            "more-checkpoint",
                            "Take checkpoint",
                            Some("Ctrl+Shift+K".into()),
                            false,
                            true,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            this.checkpoint(cx);
                        })),
                    ))
                    .when(checkpoints, |s| s.child(
                        menu_item(
                            "more-since",
                            "Review since checkpoint",
                            (!has_checkpoint).then(|| "Take a checkpoint first".to_string()),
                            false,
                            has_checkpoint,
                            false,
                        )
                        .when(has_checkpoint, |s| {
                            s.on_click(
                                cx.listener(|this, _, _, cx| this.review_since_checkpoint(cx)),
                            )
                        }),
                    )),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_menu();
                this.review.confirm_revert = None;
                cx.notify();
            }))
    }

    pub(super) fn review_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = Rc::new(self.tree_rows(cx));
        let files = self.review.files.clone();
        let current = self.current_file();
        let entity = cx.entity();
        let focused = self
            .review
            .filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let count = rows.len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_l_1()
            .border_color(rgb(DIVIDER))
            .child(
                div().px_2().pt_2().pb_1p5().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .h(px(30.))
                        .pl_2p5()
                        .pr_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(if focused { 0x3a3a3a } else { BORDER }))
                        .child(icon(ui("search"), MUTED, 13.))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.review.filter)
                                    .small()
                                    .appearance(false)
                                    .cleanable(true)
                                    .text_size(px(12.5)),
                            ),
                        ),
                ),
            )
            .child(
                uniform_list("review-tree", count, move |range, _, _| {
                    range
                        .map(|i| tree_row(i, &rows[i], &files, current, entity.clone()))
                        .collect()
                })
                .track_scroll(&self.review.tree_scroll)
                .flex_1()
                .pb_2(),
            )
            .into_any_element()
    }

    fn tree_rows(&self, cx: &App) -> Vec<TreeRow> {
        #[derive(Default)]
        struct Node {
            name: String,
            key: String,
            dirs: Vec<Node>,
            files: Vec<usize>,
        }
        fn letters(node: &Node, files: &[ReviewFile], out: &mut HashSet<char>) {
            out.extend(node.files.iter().map(|&i| files[i].change.letter));
            for dir in &node.dirs {
                letters(dir, files, out);
            }
        }
        fn walk(
            node: &Node,
            depth: usize,
            rows: &mut Vec<TreeRow>,
            files: &[ReviewFile],
            closed: &HashSet<String>,
        ) {
            for dir in &node.dirs {
                // Single-folder chains read as one row, like `cady-core / src`.
                let mut dir = dir;
                let mut name = dir.name.clone();
                while dir.files.is_empty() && dir.dirs.len() == 1 {
                    dir = &dir.dirs[0];
                    name = format!("{name} / {}", dir.name);
                }
                let mut set = HashSet::new();
                letters(dir, files, &mut set);
                let color = match (set.len(), set.iter().next()) {
                    (1, Some('A')) => ADDED,
                    (1, Some('D')) => DELETED,
                    _ => MODIFIED,
                };
                let open = !closed.contains(&dir.key);
                rows.push(TreeRow::Dir {
                    key: dir.key.clone(),
                    name,
                    depth,
                    open,
                    color,
                });
                if open {
                    walk(dir, depth + 1, rows, files, closed);
                }
            }
            for &file in &node.files {
                rows.push(TreeRow::File { file, depth });
            }
        }
        let query = self.filter_text(cx);
        let files = &self.review.files;
        let mut root = Node::default();
        for (i, file) in files.iter().enumerate() {
            let path = &file.change.path;
            if !query.is_empty() && !path.to_lowercase().contains(&query) {
                continue;
            }
            let mut node = &mut root;
            let mut key = String::new();
            let parts: Vec<&str> = path.split('/').collect();
            for part in &parts[..parts.len() - 1] {
                if !key.is_empty() {
                    key.push('/');
                }
                key.push_str(part);
                let index = match node.dirs.iter().position(|d| d.name == *part) {
                    Some(index) => index,
                    None => {
                        node.dirs.push(Node {
                            name: part.to_string(),
                            key: key.clone(),
                            ..Default::default()
                        });
                        node.dirs.len() - 1
                    }
                };
                node = &mut node.dirs[index];
            }
            node.files.push(i);
        }
        let mut rows = Vec::new();
        walk(&root, 0, &mut rows, files, &self.review.closed_dirs);
        rows
    }
}

/// Everything a diff row needs, shared by the list's render callback.
struct RowContext {
    rows: Rc<Vec<Row>>,
    files: Rc<Vec<ReviewFile>>,
    entity: Entity<Browser>,
    collapsed: HashSet<String>,
    wrap: bool,
    editable: bool,
    /// Files can be staged (`true`) or unstaged (`false`) from this source.
    stage_files: Option<bool>,
    /// The same for single hunks of modified files.
    stage_hunks: Option<bool>,
    query: String,
    comment_input: Entity<InputState>,
}

fn push_lines(
    rows: &mut Vec<Row>,
    file: usize,
    lines: &[DiffLine],
    range: Range<usize>,
    split: bool,
    comment: Option<usize>,
) {
    let push = |rows: &mut Vec<Row>, row: Row, covers: [Option<usize>; 2]| {
        rows.push(row);
        if let Some(line) = comment.filter(|c| covers.contains(&Some(*c))) {
            rows.push(Row::Comment { file, line });
        }
    };
    if !split {
        for line in range {
            push(rows, Row::Line { file, line }, [Some(line), None]);
        }
        return;
    }
    let mut k = range.start;
    while k < range.end {
        match lines[k].kind {
            '-' => {
                let start = k;
                while k < range.end && lines[k].kind == '-' {
                    k += 1;
                }
                let middle = k;
                while k < range.end && lines[k].kind == '+' {
                    k += 1;
                }
                for n in 0..(middle - start).max(k - middle) {
                    let old = (start + n < middle).then_some(start + n);
                    let new = (middle + n < k).then_some(middle + n);
                    push(rows, Row::Pair { file, old, new }, [old, new]);
                }
            }
            '+' => {
                push(rows, Row::Pair { file, old: None, new: Some(k) }, [Some(k), None]);
                k += 1;
            }
            _ => {
                push(rows, Row::Pair { file, old: Some(k), new: Some(k) }, [Some(k), None]);
                k += 1;
            }
        }
    }
}

fn render_row(ix: usize, context: &RowContext) -> AnyElement {
    let Some(row) = context.rows.get(ix).copied() else {
        return div().into_any_element();
    };
    match row {
        Row::Header(file) => header_row(ix, file, context),
        Row::Note(file) => note_row(&context.files[file]),
        Row::Gap { file, start, end } => gap_row(ix, file, start, end, context),
        Row::Line { file, line } => line_row(ix, file, line, context),
        Row::Pair { file, old, new } => pair_row(ix, file, old, new, context),
        Row::Comment { .. } => comment_row(context),
        Row::Spacer(_) => div().h(px(SPACER_H)).into_any_element(),
    }
}

fn header_row(ix: usize, index: usize, context: &RowContext) -> AnyElement {
    let file = &context.files[index];
    let change = &file.change;
    let (folder, name) = match change.path.rsplit_once('/') {
        Some((folder, name)) => (format!("{folder}/"), name.to_string()),
        None => (String::new(), change.path.clone()),
    };
    let (icon_path, color) = icons::file_icon(Path::new(&change.path));
    let collapsed = context.collapsed.contains(&change.path);
    let deleted = change.letter == 'D';
    let path = change.path.clone();
    let absolute = change.absolute.clone();
    let toggle = context.entity.clone();
    let open = context.entity.clone();
    let revert = context.entity.clone();
    div()
        .id(("review-header", ix))
        .h(px(HEADER_H))
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .pl_2()
        .pr_2()
        .bg(rgb(PANEL))
        .border_t_1()
        .border_b_1()
        .border_color(rgb(DIVIDER))
        .text_size(px(12.5))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x101010)))
        .child(icon(
            ui(if collapsed { "chevron-right" } else { "chevron-down" }),
            MUTED,
            14.,
        ))
        .child(icon(icon_path, color, 15.))
        .child(
            div()
                .flex()
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(MUTED))
                        .child(folder),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(TEXT))
                        .font_weight(FontWeight::SEMIBOLD)
                        .when(deleted, |s| s.line_through())
                        .child(name),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(ADDED))
                .child(format!("+{}", change.additions)),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(DELETED))
                .child(format!("−{}", change.deletions)),
        )
        .child(div().flex_1())
        .when(!deleted, |s| {
            s.child(
                icon_button(("review-open", ix), "file-text", "Open file").on_click(
                    move |_, _, cx| {
                        cx.stop_propagation();
                        let absolute = absolute.clone();
                        open.update(cx, |view, cx| {
                            if absolute.is_file() {
                                view.open(absolute, true, cx);
                            } else {
                                view.say("This file is not in the working tree anymore.");
                            }
                            cx.notify();
                        });
                    },
                ),
            )
        })
        .when_some(context.stage_files, |s, stage| {
            let entity = context.entity.clone();
            s.child(
                icon_button(
                    ("review-stage", ix),
                    if stage { "plus" } else { "minus" },
                    if stage { "Stage file" } else { "Unstage file" },
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |view, cx| view.stage_file(index, stage, cx));
                }),
            )
        })
        .when(context.editable, |s| {
            s.child(
                icon_button(("review-revert", ix), "rotate-ccw", "Revert this file").on_click(
                    move |_, _, cx| {
                        cx.stop_propagation();
                        revert.update(cx, |view, cx| view.revert_file(index, cx));
                    },
                ),
            )
        })
        .on_click(move |_, _, cx| {
            let path = path.clone();
            toggle.update(cx, |view, cx| view.toggle_file(path, cx));
        })
        .into_any_element()
}

fn note_row(file: &ReviewFile) -> AnyElement {
    let text = match &file.diff {
        None => "Loading…".to_string(),
        Some(diff) => diff
            .note
            .clone()
            .unwrap_or_else(|| "No content changes".into()),
    };
    div()
        .h(px(NOTE_H))
        .w_full()
        .flex()
        .items_center()
        .pl(px(GUTTER + 19.))
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text)
        .into_any_element()
}

fn gap_row(ix: usize, file: usize, start: usize, end: usize, context: &RowContext) -> AnyElement {
    let count = end - start;
    let entity = context.entity.clone();
    div()
        .id(("review-gap", ix))
        .h(px(GAP_H))
        .w_full()
        .px_2()
        .py(px(3.))
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .rounded_md()
                .bg(rgb(0x1c1c1c))
                .text_size(px(11.5))
                .text_color(rgb(TEXT_2))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0x242424)).text_color(rgb(TEXT)))
                .child(icon(ui("unfold-vertical"), MUTED, 12.))
                .child(format!(
                    "{count} unmodified {}",
                    if count == 1 { "line" } else { "lines" }
                )),
        )
        .on_click(move |_, _, cx| {
            entity.update(cx, |view, cx| view.expand_gap(file, start, cx));
        })
        .into_any_element()
}

fn line_colors(kind: char) -> (u32, u32) {
    match kind {
        '+' => (ADDED_BG, ADDED_TEXT),
        '-' => (DELETED_BG, DELETED_TEXT),
        _ => (PANEL, CONTEXT_TEXT),
    }
}

/// Accent bar, line number and +/− marker of one side of a line.
fn line_lead(kind: char, number: Option<usize>) -> [Div; 3] {
    let changed = kind == '+' || kind == '-';
    let accent = if kind == '+' { ADDED } else { DELETED };
    [
        div()
            .w(px(3.))
            .flex_shrink_0()
            .when(changed, |s| s.bg(rgb(accent))),
        div()
            .w(px(GUTTER - 3.))
            .flex_shrink_0()
            .pr_2()
            .text_right()
            .text_size(px(11.))
            .text_color(rgb(if changed { 0x7a7a7a } else { 0x4a4a4a }))
            .child(number.map(|n| n.to_string()).unwrap_or_default()),
        div()
            .w(px(16.))
            .flex_shrink_0()
            .text_color(rgb(accent))
            .child(match kind {
                '+' => "+",
                '-' => "−",
                _ => "",
            }),
    ]
}

fn code_text(line: &DiffLine, context: &RowContext) -> Div {
    let (_, color) = line_colors(line.kind);
    let spans = if context.query.is_empty() {
        line.spans.clone()
    } else {
        let lower = line.text.to_ascii_lowercase();
        let marks = lower
            .match_indices(context.query.as_str())
            .map(|(i, m)| i..i + m.len())
            .collect::<Vec<_>>();
        overlay(
            &line.spans,
            &marks,
            HighlightStyle {
                background_color: Some(rgb(MATCH_BG).into()),
                ..Default::default()
            },
        )
    };
    div()
        .flex_1()
        .min_w_0()
        .pr_3()
        .text_color(rgb(color))
        .when(!context.wrap, |s| s.overflow_hidden().whitespace_nowrap())
        .child(StyledText::new(line.text.clone()).with_highlights(spans))
}

/// Adds `mark` over `marks`, splitting the syntax spans so the result stays
/// sorted and non-overlapping.
fn overlay(
    spans: &[(Range<usize>, HighlightStyle)],
    marks: &[Range<usize>],
    mark: HighlightStyle,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if marks.is_empty() {
        return spans.to_vec();
    }
    let mut points: Vec<usize> = spans
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .chain(marks.iter().flat_map(|r| [r.start, r.end]))
        .collect();
    points.sort_unstable();
    points.dedup();
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let base = spans
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, s)| *s);
        let marked = marks.iter().any(|r| r.start <= a && b <= r.end);
        let style = match (base, marked) {
            (Some(style), true) => style.highlight(mark),
            (Some(style), false) => style,
            (None, true) => mark,
            (None, false) => continue,
        };
        out.push((a..b, style));
    }
    out
}

fn row_frame(id: (&'static str, usize), wrap: bool) -> Stateful<Div> {
    div()
        .id(id)
        .group("review-line")
        .relative()
        .w_full()
        .flex()
        .when(!wrap, |s| s.h(px(LINE_H)))
        .when(wrap, |s| s.min_h(px(LINE_H)))
        .font_family(mono_font())
        .text_size(px(12.))
        .line_height(px(LINE_H))
}

/// The hover button that opens a comment on a line.
fn comment_button(ix: usize, file: usize, line: usize, context: &RowContext) -> impl IntoElement {
    let entity = context.entity.clone();
    div()
        .id(("review-comment", ix))
        .absolute()
        .left(px(5.))
        .top(px(2.))
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(rgb(0xe6e6e6))
        .opacity(0.)
        .group_hover("review-line", |s| s.opacity(1.))
        .cursor_pointer()
        .tooltip(|window, cx| Tooltip::new("Comment on this line").build(window, cx))
        .child(icon(ui("plus"), 0x111111, 12.))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity.update(cx, |view, cx| view.open_comment(file, line, window, cx));
        })
}

/// A small text button that appears on a hovered line of a hunk.
fn hunk_action(
    id: (&'static str, usize),
    name: &str,
    label: &'static str,
    on_click: impl Fn(&mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(18.))
        .px_1p5()
        .flex()
        .items_center()
        .gap_1()
        .rounded(px(4.))
        .bg(rgb(0x262626))
        .border_1()
        .border_color(rgb(0x3a3a3a))
        .font_family(ui_font())
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(rgb(TEXT_2))
        .hover(|s| s.bg(rgb(0x303030)).text_color(rgb(TEXT)))
        .cursor_pointer()
        .child(icon(ui(name), TEXT_2, 11.))
        .child(label)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            on_click(cx);
        })
}

/// The hover buttons that stage, unstage or revert the change a line
/// belongs to.
fn hunk_button(ix: usize, file: usize, line: usize, context: &RowContext) -> Option<AnyElement> {
    let stage = context
        .stage_hunks
        .filter(|_| context.files[file].change.letter == 'M');
    if !context.editable && stage.is_none() {
        return None;
    }
    let diff = context.files[file].diff.as_ref()?;
    let position = diff.hunks.partition_point(|h| h.lines.start <= line).checked_sub(1)?;
    if !diff.hunks[position].lines.contains(&line) {
        return None;
    }
    let stage_entity = context.entity.clone();
    let revert_entity = context.entity.clone();
    Some(
        div()
            .absolute()
            .right(px(10.))
            .top(px(1.))
            .flex()
            .gap_1()
            .opacity(0.)
            .group_hover("review-line", |s| s.opacity(1.))
            .when_some(stage, |s, stage| {
                s.child(hunk_action(
                    ("review-stage-hunk", ix),
                    if stage { "plus" } else { "minus" },
                    if stage { "Stage" } else { "Unstage" },
                    move |cx| {
                        stage_entity.update(cx, |view, cx| {
                            view.stage_hunk(file, position, stage, cx)
                        })
                    },
                ))
            })
            .when(context.editable, |s| {
                s.child(hunk_action(
                    ("review-revert-hunk", ix),
                    "rotate-ccw",
                    "Revert",
                    move |cx| {
                        revert_entity.update(cx, |view, cx| view.revert_hunk(file, position, cx))
                    },
                ))
            })
            .into_any_element(),
    )
}

fn line_row(ix: usize, file: usize, index: usize, context: &RowContext) -> AnyElement {
    let Some(line) = context.files[file]
        .diff
        .as_ref()
        .and_then(|d| d.lines.get(index))
    else {
        return div().into_any_element();
    };
    let (bg, _) = line_colors(line.kind);
    let number = if line.kind == '-' {
        line.old
    } else {
        line.new.or(line.old)
    };
    let changed = line.kind != ' ';
    row_frame(("review-line", ix), context.wrap)
        .bg(rgb(bg))
        .children(line_lead(line.kind, number))
        .child(code_text(line, context))
        .child(comment_button(ix, file, index, context))
        .when(changed, |s| s.children(hunk_button(ix, file, index, context)))
        .into_any_element()
}

fn pair_row(
    ix: usize,
    file: usize,
    old: Option<usize>,
    new: Option<usize>,
    context: &RowContext,
) -> AnyElement {
    let Some(diff) = context.files[file].diff.as_ref() else {
        return div().into_any_element();
    };
    let side = |index: Option<usize>, left: bool| -> Div {
        let Some(line) = index.and_then(|i| diff.lines.get(i)) else {
            return div().flex_1().min_w_0().bg(rgb(EMPTY_BG));
        };
        let (bg, _) = line_colors(line.kind);
        let number = if left { line.old } else { line.new };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .bg(rgb(bg))
            .children(line_lead(line.kind, number))
            .child(code_text(line, context))
    };
    let anchor = new.or(old).unwrap_or_default();
    let changed = [old, new]
        .iter()
        .flatten()
        .any(|&i| diff.lines.get(i).is_some_and(|l| l.kind != ' '));
    let hunk_line = [old, new]
        .iter()
        .flatten()
        .copied()
        .find(|&i| diff.lines.get(i).is_some_and(|l| l.kind != ' '));
    // A context line has the same index on both sides.
    row_frame(("review-pair", ix), context.wrap)
        .child(side(old, true))
        .child(div().w(px(1.)).flex_shrink_0().bg(rgb(DIVIDER)))
        .child(side(new, false))
        .child(comment_button(ix, file, anchor, context))
        .when_some(hunk_line.filter(|_| changed), |s, line| {
            s.children(hunk_button(ix, file, line, context))
        })
        .into_any_element()
}

fn comment_row(context: &RowContext) -> AnyElement {
    let cancel = context.entity.clone();
    let add = context.entity.clone();
    div()
        .h(px(COMMENT_H))
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .pl(px(GUTTER + 19.))
        .pr_3()
        .bg(rgb(0x111111))
        .border_y_1()
        .border_color(rgb(BORDER))
        .font_family(ui_font())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(rgb(0x3a3a3a))
                .bg(rgb(PANEL))
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&context.comment_input)
                            .small()
                            .appearance(false)
                            .text_size(px(12.5)),
                    ),
                ),
        )
        .child(
            text_button("review-comment-cancel", None, "Cancel").on_click(move |_, window, cx| {
                cancel.update(cx, |view, cx| view.close_comment(window, cx));
            }),
        )
        .child(
            text_button("review-comment-add", Some("plus"), "Add to input")
                .border_1()
                .border_color(rgb(0x3a3a3a))
                .on_click(move |_, window, cx| {
                    add.update(cx, |view, cx| view.submit_comment(window, cx));
                }),
        )
        .into_any_element()
}

enum TreeRow {
    Dir {
        key: String,
        name: String,
        depth: usize,
        open: bool,
        color: u32,
    },
    File {
        file: usize,
        depth: usize,
    },
}

fn tree_row(
    i: usize,
    row: &TreeRow,
    files: &[ReviewFile],
    current: Option<usize>,
    entity: Entity<Browser>,
) -> AnyElement {
    match row {
        TreeRow::Dir {
            key,
            name,
            depth,
            open,
            color,
        } => {
            let key = key.clone();
            div()
                .id(("review-tree-row", i))
                .h(px(TREE_ROW))
                .px_1()
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .pl(px(6. + *depth as f32 * 12.))
                        .pr_2()
                        .rounded_md()
                        .text_size(px(12.5))
                        .text_color(rgb(0xdedede))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(HOVER)))
                        .child(icon(
                            ui(if *open { "chevron-down" } else { "chevron-right" }),
                            MUTED,
                            14.,
                        ))
                        .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        // A folder's dot is its files' state, dimmed like Codex.
                        .child(
                            div()
                                .size(px(6.))
                                .mr(px(4.))
                                .rounded_full()
                                .bg(rgb(*color))
                                .opacity(0.55),
                        ),
                )
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    entity.update(cx, |view, cx| {
                        if !view.review.closed_dirs.remove(&key) {
                            view.review.closed_dirs.insert(key);
                        }
                        cx.notify();
                    });
                })
                .into_any_element()
        }
        TreeRow::File { file, depth } => {
            let index = *file;
            let change = &files[index].change;
            let name = change.path.rsplit('/').next().unwrap_or_default().to_string();
            let (icon_path, color) = icons::file_icon(Path::new(&change.path));
            let active = current == Some(index);
            let deleted = change.letter == 'D';
            div()
                .id(("review-tree-row", i))
                .h(px(TREE_ROW))
                .px_1()
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .pl(px(6. + *depth as f32 * 12. + 20.))
                        .pr_2()
                        .rounded_md()
                        .text_size(px(12.5))
                        .text_color(rgb(if deleted {
                            MUTED
                        } else if active {
                            TEXT
                        } else {
                            0x9a9a9a
                        }))
                        .cursor_pointer()
                        .when(active, |s| s.bg(rgb(SELECTED)))
                        .when(!active, |s| s.hover(|s| s.bg(rgb(HOVER))))
                        .child(icon(icon_path, color, 15.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when(deleted, |s| s.line_through())
                                .child(name),
                        )
                        .child(status_badge(change.letter)),
                )
                .on_click(move |_, _, cx| {
                    entity.update(cx, |view, cx| view.reveal_file(index, cx));
                })
                .into_any_element()
        }
    }
}

/// Small outlined square: `+` added, a dot for modified, `−` deleted. The
/// marks are drawn, not typed, so they sit exactly in the middle.
fn status_badge(letter: char) -> Div {
    let color = match letter {
        'A' => ADDED,
        'D' => DELETED,
        _ => MODIFIED,
    };
    // 14px square with a 1px border leaves a 12px field; 2px bars at 5px and
    // 8px lengths at 2px land on whole pixels.
    let bar = |left: f32, top: f32, width: f32, height: f32| {
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .rounded(px(0.5))
            .bg(rgb(color))
    };
    let field = div().relative().size(px(12.));
    let mark = match letter {
        'A' => field.child(bar(2., 5., 8., 2.)).child(bar(5., 2., 2., 8.)),
        'D' => field.child(bar(2., 5., 8., 2.)),
        _ => field.child(bar(4., 4., 4., 4.).rounded(px(1.))),
    };
    div()
        .size(px(14.))
        .flex_shrink_0()
        .rounded(px(3.5))
        .border_1()
        .border_color(rgb(color))
        .child(mark)
}

/// An icon button in the review toolbar, highlighted while its mode is on.
fn tool(id: &'static str, name: &str, tooltip: &'static str, on: bool) -> Stateful<Div> {
    icon_button(id, name, tooltip)
        .rounded_full()
        .when(on, |s| s.bg(rgb(SELECTED)))
}

pub(super) fn menu_panel(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .p_1()
        .flex()
        .flex_col()
        .rounded_lg()
        .bg(rgb(0x121212))
        .border_1()
        .border_color(rgb(BORDER))
        .shadow_xl()
}

pub(super) fn separator() -> Div {
    div().h(px(1.)).mx_1().my_1().bg(rgb(BORDER))
}

fn menu_note(text: &'static str) -> AnyElement {
    div()
        .px_2()
        .py_1p5()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text)
        .into_any_element()
}

pub(super) fn menu_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    detail: Option<String>,
    checked: bool,
    enabled: bool,
    submenu: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .min_h(px(30.))
        .py_1()
        .px_2()
        .rounded_md()
        .text_size(px(12.5))
        .text_color(rgb(if enabled { TEXT } else { MUTED }))
        .when(enabled, |s| s.cursor_pointer().hover(|s| s.bg(rgb(HOVER))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().child(label.into()))
                .when_some(detail.filter(|d| !d.is_empty()), |s, detail| {
                    s.child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(detail),
                    )
                }),
        )
        .when(checked, |s| s.child(icon(ui("check"), TEXT, 14.)))
        .when(submenu, |s| s.child(icon(ui("chevron-right"), MUTED, 13.)))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would bring in GPUI's own `test` attribute.
    use super::{MATCH_BG, Row, overlay, push_lines};
    use crate::workspace::DiffLine;
    use gpui::{FontWeight, HighlightStyle, rgb};

    fn line(kind: char) -> DiffLine {
        DiffLine {
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn split_rows_pair_removed_and_added_lines() {
        let lines = [line(' '), line('-'), line('-'), line('+'), line(' '), line('+')];
        let mut rows = vec![];
        push_lines(&mut rows, 0, &lines, 0..lines.len(), true, Some(3));
        assert_eq!(
            rows,
            vec![
                Row::Pair { file: 0, old: Some(0), new: Some(0) },
                Row::Pair { file: 0, old: Some(1), new: Some(3) },
                Row::Comment { file: 0, line: 3 },
                Row::Pair { file: 0, old: Some(2), new: None },
                Row::Pair { file: 0, old: Some(4), new: Some(4) },
                Row::Pair { file: 0, old: None, new: Some(5) },
            ]
        );
    }

    #[test]
    fn overlay_splits_spans_without_overlaps() {
        let bold = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let mark = HighlightStyle {
            background_color: Some(rgb(MATCH_BG).into()),
            ..Default::default()
        };
        let marked = 2..6;
        let out = overlay(&[(0..4, bold)], std::slice::from_ref(&marked), mark);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![0..2, 2..4, 4..6]);
        assert_eq!(out[1].1.font_weight, Some(FontWeight::BOLD));
        assert!(out[1].1.background_color.is_some());
        assert!(out[0].1.background_color.is_none());
    }
}
