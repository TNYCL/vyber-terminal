//! The file panel that opens over a terminal: a workspace tree, editor and
//! previews on the file side, and a Review tab for Git and task changes.
mod document;
mod review;
mod tree;

use crate::{
    changeset::{self, Commit, FileChange, FileDiff},
    tasks::TaskReview,
    theme::{self, BORDER, PANEL},
    workspace::{self, Change, Checkpoint, FileEntry},
};
use gpui::{prelude::*, *};
use gpui_kit::component::input::{EditorState, InputEvent, InputState};
use notify::Watcher;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const NOTICE_TIME: Duration = Duration::from_secs(4);
const SIDEBAR_MIN: f32 = 190.;
const SIDEBAR_MAX: f32 = 460.;
const TEXT_LIMIT: usize = 8 * 1024 * 1024;
const READ_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BrowserState {
    pub visible: bool,
    pub expanded: BTreeSet<String>,
    pub docs: Vec<SavedDocument>,
    pub active: Option<PathBuf>,
    pub follow: bool,
    pub pinned: bool,
    #[serde(default)]
    pub wide: bool,
    #[serde(default)]
    pub tree_root: Option<String>,
    /// Panel width as a share of the window.
    #[serde(default)]
    pub panel_width: Option<f32>,
    /// Sidebar widths and hidden flags: `[files, review]`.
    #[serde(default)]
    pub sidebar_widths: Option<[f32; 2]>,
    #[serde(default)]
    pub hidden_sidebars: [bool; 2],
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedDocument {
    path: PathBuf,
    preview: bool,
    pinned: bool,
    scroll: (f32, f32),
    draft: Option<String>,
    baseline: Option<Vec<u8>>,
}
#[derive(Clone, PartialEq)]
enum Kind {
    Text,
    Image,
    /// Binary, too large or not UTF-8: shown as an information card.
    Unsupported(SharedString),
}
struct Document {
    path: PathBuf,
    editor: Entity<EditorState>,
    baseline: Vec<u8>,
    text: SharedString,
    kind: Kind,
    size: u64,
    preview: bool,
    dirty: bool,
    conflict: bool,
    pinned: bool,
    scroll: ScrollHandle,
    _subscription: Subscription,
}
impl Document {
    fn markdown(&self) -> bool {
        self.path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                ["md", "markdown", "mdx"]
                    .iter()
                    .any(|m| e.eq_ignore_ascii_case(m))
            })
    }
}
enum Message {
    Index(Vec<FileEntry>, Vec<Change>, String),
    Loaded(PathBuf, Result<Vec<u8>, String>, u64, bool),
    Changed(PathBuf, Option<Vec<u8>>),
    Checkpoint(Result<Checkpoint, String>),
    Notice(String),
    Saved(PathBuf, Vec<u8>, SharedString, Result<(), String>),
    Search(String, Vec<(PathBuf, usize, String)>),
    Tasks(Vec<TaskReview>),
    /// A review load: its file list (root, base, files), diffs in chunks,
    /// then the end, or an error. The first number is the load generation.
    ReviewList(u64, PathBuf, String, Vec<FileChange>),
    ReviewDiffs(u64, Vec<(usize, Arc<FileDiff>)>),
    ReviewDone(u64),
    ReviewError(u64, String),
    Commits(Result<Vec<Commit>, String>),
    /// A revert finished; the message is shown and the review reloads.
    Reverted(String),
}
#[derive(Clone, Copy, PartialEq)]
enum View {
    Files,
    Review,
}
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Root,
    Open,
    Source,
    More,
}

pub enum BrowserEvent {
    /// A review comment to type into this terminal's input.
    Comment(String),
}
impl EventEmitter<BrowserEvent> for Browser {}

/// Dragging the sidebar's left edge.
#[derive(Clone)]
struct SidebarResize;
impl Render for SidebarResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The sidebar sliding open or closed for one view (0 files, 1 review).
#[derive(Clone, Copy)]
struct Slide {
    view: usize,
    shown: bool,
    from: f32,
    start: Instant,
}
impl Slide {
    const DURATION: f32 = 0.24;
    fn settled(view: usize, shown: bool) -> Self {
        Self {
            view,
            shown,
            from: f32::from(u8::from(shown)),
            start: Instant::now() - Duration::from_secs(1),
        }
    }
    fn progress(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / Self::DURATION).min(1.)
    }
    fn value(&self) -> f32 {
        let to = f32::from(u8::from(self.shown));
        self.from + (to - self.from) * theme::ease_out(self.progress())
    }
}

pub struct Browser {
    pub root: PathBuf,
    pub visible: bool,
    /// Panel covers the whole terminal area instead of the right side.
    pub wide: bool,
    /// Panel width as a share of the window; `None` is the default.
    pub panel_width: Option<f32>,
    sidebar_width: [f32; 2],
    sidebar_hidden: [bool; 2],
    sidebar_slide: Option<Slide>,
    pub branch: String,
    pub tasks: Vec<TaskReview>,
    files: Vec<FileEntry>,
    statuses: HashMap<String, char>,
    changed_dirs: HashSet<String>,
    search_query: String,
    search_results: Vec<(PathBuf, usize, String)>,
    changes: Vec<Change>,
    expanded: BTreeSet<String>,
    docs: Vec<Document>,
    active_doc: usize,
    filter: Entity<InputState>,
    view: View,
    review: review::ReviewState,
    checkpoint: Option<Checkpoint>,
    follow: bool,
    pinned: bool,
    notice: String,
    notice_at: Instant,
    notice_serial: u64,
    loading: bool,
    zoom: f32,
    restore_docs: Vec<SavedDocument>,
    restore_active: Option<PathBuf>,
    pending_line: Option<(PathBuf, usize)>,
    focus: FocusHandle,
    quick_look: bool,
    tree_root: Option<String>,
    tree_scroll: UniformListScrollHandle,
    tree_selected: Option<String>,
    reveal: bool,
    menu: Option<Menu>,
    /// The menu an outside mouse-down just closed, so the click that follows on
    /// its own trigger button does not reopen it.
    menu_dismissed: Option<(Menu, Instant)>,
    tab_scroll: ScrollHandle,
    receiver: mpsc::Receiver<Message>,
    sender: mpsc::Sender<Message>,
    stop: Arc<AtomicBool>,
    _subscriptions: Vec<Subscription>,
}
impl Drop for Browser {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Browser {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        start_watcher(root.clone(), sender.clone(), stop.clone());
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter files…")
                .clean_on_escape()
        });
        let subscription = cx.subscribe_in(&filter, window, |this, _, event, window, cx| {
            match event {
                InputEvent::PressEnter { shift, .. } => {
                    if *shift || !this.open_best_match(cx) {
                        this.search_contents(cx);
                    }
                    window.focus(&this.focus, cx);
                }
                InputEvent::Change => this.reveal = this.filter_query(cx).is_empty(),
                _ => {}
            }
            cx.notify();
        });
        cx.spawn_in(window, async move |entity, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(200)).await;
                if entity
                    .update_in(cx, |view, window, cx| view.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let (review, review_subscriptions) = review::ReviewState::new(window, cx);
        let mut subscriptions = vec![subscription];
        subscriptions.extend(review_subscriptions);
        Self {
            root,
            visible: false,
            wide: false,
            panel_width: None,
            sidebar_width: [260., 250.],
            sidebar_hidden: [false, false],
            sidebar_slide: None,
            branch: String::new(),
            tasks: vec![],
            files: vec![],
            statuses: HashMap::new(),
            changed_dirs: HashSet::new(),
            search_query: String::new(),
            search_results: vec![],
            changes: vec![],
            expanded: BTreeSet::new(),
            docs: vec![],
            active_doc: 0,
            filter,
            view: View::Files,
            review,
            checkpoint: None,
            follow: false,
            pinned: false,
            notice: String::new(),
            notice_at: Instant::now(),
            notice_serial: 0,
            loading: true,
            zoom: 1.,
            restore_docs: vec![],
            restore_active: None,
            pending_line: None,
            focus: cx.focus_handle(),
            quick_look: false,
            tree_root: None,
            tree_scroll: UniformListScrollHandle::new(),
            tree_selected: None,
            reveal: false,
            menu: None,
            menu_dismissed: None,
            tab_scroll: ScrollHandle::new(),
            receiver,
            sender,
            stop,
            _subscriptions: subscriptions,
        }
    }
    fn say(&mut self, text: impl Into<String>) {
        self.notice = text.into();
        self.notice_at = Instant::now();
        self.notice_serial += 1;
    }
    fn toggle_menu(&mut self, menu: Menu) {
        let just_closed = self
            .menu_dismissed
            .take()
            .is_some_and(|(m, at)| m == menu && at.elapsed() < Duration::from_millis(500));
        self.menu = if self.menu == Some(menu) || just_closed {
            None
        } else {
            Some(menu)
        };
    }
    fn dismiss_menu(&mut self) {
        self.menu_dismissed = self.menu.take().map(|m| (m, Instant::now()));
    }
    fn filter_query(&self, cx: &App) -> String {
        self.filter.read(cx).value().trim().to_string()
    }
    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
    fn active_document(&self) -> Option<&Document> {
        self.docs.get(self.active_doc)
    }

    pub fn has_dirty(&self) -> bool {
        self.docs.iter().any(|d| d.dirty)
    }
    pub fn owns_focus(&self, window: &Window, cx: &App) -> bool {
        self.docs
            .iter()
            .any(|d| d.editor.read(cx).focus_handle(cx).is_focused(window))
    }
    pub fn state(&self, cx: &App) -> BrowserState {
        BrowserState {
            visible: self.visible,
            expanded: self.expanded.clone(),
            active: self.active_document().map(|d| d.path.clone()),
            follow: self.follow,
            pinned: self.pinned,
            wide: self.wide,
            tree_root: self.tree_root.clone(),
            panel_width: self.panel_width,
            sidebar_widths: Some(self.sidebar_width),
            hidden_sidebars: self.sidebar_hidden,
            docs: self
                .docs
                .iter()
                .map(|d| SavedDocument {
                    path: d.path.clone(),
                    preview: d.preview,
                    pinned: d.pinned,
                    scroll: (
                        f32::from(d.scroll.offset().x),
                        f32::from(d.scroll.offset().y),
                    ),
                    draft: d.dirty.then(|| d.editor.read(cx).value().to_string()),
                    baseline: d.dirty.then(|| d.baseline.clone()),
                })
                .collect(),
        }
    }
    pub fn restore_state(&mut self, state: BrowserState, cx: &mut Context<Self>) {
        self.visible = state.visible;
        self.expanded = state.expanded;
        self.follow = state.follow;
        self.pinned = state.pinned;
        self.wide = state.wide;
        self.tree_root = state.tree_root;
        self.panel_width = state.panel_width.filter(|w| (0.2..=1.).contains(w));
        if let Some(widths) = state.sidebar_widths {
            self.sidebar_width = widths.map(|w| w.clamp(SIDEBAR_MIN, SIDEBAR_MAX));
        }
        self.sidebar_hidden = state.hidden_sidebars;
        self.restore_docs = state.docs;
        let paths = self
            .restore_docs
            .iter()
            .map(|d| d.path.clone())
            .collect::<Vec<_>>();
        for path in paths {
            self.open(path, true, cx);
        }
        self.restore_active = state.active;
    }
    pub fn open_at(
        &mut self,
        path: PathBuf,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible = true;
        self.open(path.clone(), true, cx);
        if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
            doc.preview = false;
            doc.editor.update(cx, |ed, cx| {
                ed.set_cursor_position(
                    gpui_kit::component::input::Position::new(line.saturating_sub(1) as u32, 0),
                    window,
                    cx,
                )
            });
        } else {
            self.pending_line = Some((path, line));
        }
    }
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.view = View::Files;
        self.filter.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub fn change_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.stop.store(true, Ordering::Relaxed);
        self.stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        self.sender = sender;
        self.receiver = receiver;
        self.root = root.clone();
        self.files.clear();
        self.changes.clear();
        self.statuses.clear();
        self.changed_dirs.clear();
        self.expanded.clear();
        self.tasks.clear();
        self.reset_review();
        self.tree_root = None;
        self.tree_selected = None;
        self.loading = true;
        start_watcher(root, self.sender.clone(), self.stop.clone());
        cx.notify();
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let messages: Vec<_> = self.receiver.try_iter().collect();
        let mut changed = !messages.is_empty();
        let mut follow_path = None;
        for message in messages {
            match message {
                Message::Index(files, changes, branch) => {
                    if self.files != files {
                        self.files = files;
                    }
                    if self.changes != changes {
                        self.review_worktree_changed(true);
                        self.statuses = changes
                            .iter()
                            .map(|c| (c.path.clone(), c.letter()))
                            .collect();
                        self.changed_dirs = changes
                            .iter()
                            .flat_map(|c| workspace::expanded_parents(&c.path))
                            .collect();
                        self.changes = changes;
                    }
                    self.branch = branch;
                    self.loading = false;
                    if let Some(root) = &self.tree_root {
                        if !self.files.iter().any(|f| &f.relative == root) {
                            self.tree_root = None;
                        }
                    }
                }
                Message::Loaded(path, result, size, pinned) => {
                    self.loaded(path, result, size, pinned, window, cx)
                }
                Message::Changed(path, bytes) => {
                    self.review_worktree_changed(false);
                    if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
                        if doc.kind == Kind::Text && bytes.as_ref() != Some(&doc.baseline) {
                            if doc.dirty || bytes.is_none() {
                                doc.conflict = true;
                            } else if let Some(bytes) = bytes {
                                if let Ok(text) = String::from_utf8(bytes.clone()) {
                                    let text = text.trim_start_matches('\u{feff}').to_owned();
                                    doc.baseline = bytes;
                                    doc.text = text.clone().into();
                                    doc.editor.update(cx, |ed, cx| {
                                        let offset = ed.scroll_offset();
                                        ed.set_value(text, window, cx);
                                        ed.set_scroll_offset(offset, cx);
                                    });
                                }
                            }
                        }
                    }
                    follow_path = Some(path);
                }
                Message::Checkpoint(result) => match result {
                    Ok(snapshot) => {
                        self.checkpoint = Some(snapshot);
                        self.say(
                            "Checkpoint saved · Review ▸ ··· ▸ Review since checkpoint shows what changes from now",
                        );
                    }
                    Err(e) => self.say(e),
                },
                Message::ReviewList(generation, root, base, files) => {
                    self.review_listed(generation, root, base, files)
                }
                Message::ReviewDiffs(generation, diffs) => self.review_diffs(generation, diffs),
                Message::ReviewDone(generation) => self.review_done(generation),
                Message::ReviewError(generation, error) => self.review_failed(generation, error),
                Message::Commits(commits) => self.review_commits(commits),
                Message::Reverted(message) => {
                    self.say(message);
                    self.review_reload_now();
                }
                Message::Notice(message) => self.say(message),
                Message::Saved(path, bytes, value, result) => match result {
                    Ok(()) => {
                        if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
                            doc.baseline = bytes;
                            doc.text = value;
                            doc.dirty = doc.editor.read(cx).value() != doc.text;
                            doc.conflict = false;
                        }
                        self.say("Saved");
                    }
                    Err(e) => self.say(e),
                },
                Message::Search(query, results) => {
                    self.say(format!("{} matches in file contents", results.len()));
                    self.search_query = query;
                    self.search_results = results;
                }
                Message::Tasks(tasks) => {
                    for task in tasks {
                        if !self.tasks.iter().any(|t| t.id == task.id) {
                            self.tasks.push(task);
                        }
                    }
                }
            }
        }
        changed |= self.review_tick(cx);
        if self.follow
            && !self.pinned
            && !self
                .docs
                .iter()
                .any(|d| d.dirty || d.editor.read(cx).focus_handle(cx).is_focused(window))
        {
            if let Some(path) = follow_path {
                if path.is_file() {
                    self.open(path, false, cx);
                }
            }
        }
        if !self.notice.is_empty() && self.notice_at.elapsed() > NOTICE_TIME {
            self.notice.clear();
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
    fn loaded(
        &mut self,
        path: PathBuf,
        result: Result<Vec<u8>, String>,
        size: u64,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.docs.iter().position(|d| d.path == path) {
            self.active_doc = index;
            self.docs[index].pinned |= pinned;
            return;
        }
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let (kind, bytes, source) = match result {
            Err(error) => (Kind::Unsupported(error.into()), vec![], String::new()),
            Ok(bytes) if is_image(&path) => (Kind::Image, bytes, String::new()),
            Ok(bytes) if bytes.len() > TEXT_LIMIT => (
                Kind::Unsupported("Larger than the 8 MB text preview limit".into()),
                vec![],
                String::new(),
            ),
            Ok(bytes) if bytes.contains(&0) => (
                Kind::Unsupported("Binary file".into()),
                vec![],
                String::new(),
            ),
            Ok(bytes) => {
                match std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                {
                    Ok(text) => {
                        let text = text.to_owned();
                        (Kind::Text, bytes, text)
                    }
                    Err(_) => (
                        Kind::Unsupported(
                            "Not UTF-8 text · Vyber only edits UTF-8 files".into(),
                        ),
                        vec![],
                        String::new(),
                    ),
                }
            }
        };
        let text: SharedString = source.into();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .line_number(true)
                .searchable(true)
                .language(changeset::language(&path))
                .default_value(text.clone())
        });
        let watched_path = path.clone();
        let sub = cx.subscribe(&editor, move |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(doc) = view.docs.iter_mut().find(|d| d.path == watched_path) {
                    doc.dirty = doc.kind == Kind::Text && doc.editor.read(cx).value() != doc.text;
                    doc.pinned |= doc.dirty;
                }
                cx.notify();
            }
        });
        let mut doc = Document {
            preview: kind == Kind::Image || ["md", "markdown", "mdx"].contains(&&*extension),
            path,
            editor,
            baseline: bytes,
            text,
            kind,
            size,
            dirty: false,
            conflict: false,
            pinned,
            scroll: ScrollHandle::new(),
            _subscription: sub,
        };
        if let Some(i) = self.restore_docs.iter().position(|d| d.path == doc.path) {
            let saved = self.restore_docs.remove(i);
            doc.preview = saved.preview;
            doc.pinned = saved.pinned;
            doc.scroll
                .set_offset(point(px(saved.scroll.0), px(saved.scroll.1)));
            if let (Some(draft), Kind::Text) = (saved.draft, &doc.kind) {
                doc.conflict = saved.baseline.as_ref() != Some(&doc.baseline);
                if let Some(baseline) = saved.baseline {
                    doc.baseline = baseline;
                }
                doc.editor
                    .update(cx, |ed, cx| ed.set_value(draft, window, cx));
                doc.dirty = true;
                doc.pinned = true;
            }
        }
        if let Some(index) = self.docs.iter().position(|d| !d.pinned && !d.dirty) {
            self.docs[index] = doc;
            self.active_doc = index;
        } else {
            self.docs.push(doc);
            self.active_doc = self.docs.len() - 1;
        }
        if let Some(active) = &self.restore_active {
            if let Some(i) = self.docs.iter().position(|d| &d.path == active) {
                self.active_doc = i;
            }
        }
        if let Some((path, line)) = &self.pending_line {
            if let Some(doc) = self.docs.iter_mut().find(|d| &d.path == path) {
                doc.preview = false;
                let position =
                    gpui_kit::component::input::Position::new(line.saturating_sub(1) as u32, 0);
                doc.editor
                    .update(cx, |ed, cx| ed.set_cursor_position(position, window, cx));
                self.pending_line = None;
            }
        }
        self.tab_scroll.scroll_to_item(self.active_doc);
    }
    pub fn open(&mut self, path: PathBuf, pinned: bool, cx: &mut Context<Self>) {
        self.restore_active = None;
        self.view = View::Files;
        self.menu = None;
        let relative = self.relative(&path);
        self.expanded
            .extend(workspace::expanded_parents(&relative));
        self.tree_selected = Some(relative);
        self.reveal = true;
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active_doc = i;
            self.docs[i].pinned |= pinned;
            self.tab_scroll.scroll_to_item(i);
            cx.notify();
            return;
        }
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let (result, size) = match fs::metadata(&path) {
                Err(e) => (Err(e.to_string()), 0),
                Ok(m) if m.len() > READ_LIMIT => (
                    Err("Larger than the 32 MB preview limit".to_string()),
                    m.len(),
                ),
                Ok(m) => (fs::read(&path).map_err(|e| e.to_string()), m.len()),
            };
            let _ = sender.send(Message::Loaded(path, result, size, pinned));
        });
        cx.notify();
    }
    fn close_doc(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.docs.get(index).is_some_and(|d| d.dirty) {
            self.say("Save the document before closing it.");
        } else if index < self.docs.len() {
            self.docs.remove(index);
            if self.active_doc > index || self.active_doc >= self.docs.len() {
                self.active_doc = self.active_doc.saturating_sub(1);
            }
        }
        cx.notify();
    }
    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.active_document() else {
            return;
        };
        if !doc.dirty {
            return;
        }
        let value = doc.editor.read(cx).value();
        let mut text = value.to_string();
        if doc.text.contains("\r\n") {
            text = text.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        let mut bytes = vec![];
        if doc.baseline.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.extend([0xef, 0xbb, 0xbf]);
        }
        bytes.extend(text.as_bytes());
        let path = doc.path.clone();
        let baseline = doc.baseline.clone();
        let sender = self.sender.clone();
        self.say("Saving…");
        std::thread::spawn(move || {
            let result =
                workspace::save_checked(&path, &baseline, &bytes).map_err(|e| e.to_string());
            let _ = sender.send(Message::Saved(path, bytes, value, result));
        });
        cx.notify();
    }
    fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = self.docs.get_mut(self.active_doc) {
            if let Ok(bytes) = fs::read(&doc.path) {
                if let Ok(text) = String::from_utf8(bytes.clone()) {
                    let text = text.trim_start_matches('\u{feff}').to_owned();
                    doc.baseline = bytes;
                    doc.text = text.clone().into();
                    doc.dirty = false;
                    doc.conflict = false;
                    doc.editor.update(cx, |ed, cx| ed.set_value(text, window, cx));
                }
            }
        }
        cx.notify();
    }
    fn search_contents(&mut self, cx: &mut Context<Self>) {
        let query = self.filter_query(cx);
        if query.is_empty() {
            self.say("Type a search term in the file filter first.");
            cx.notify();
            return;
        }
        self.view = View::Files;
        self.say("Searching file contents…");
        let root = self.root.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let result = (|| -> anyhow::Result<Vec<(PathBuf, usize, String)>> {
                let mut child = workspace::command("rg")
                    .args([
                        "--json",
                        "--fixed-strings",
                        "--smart-case",
                        "--max-count",
                        "5",
                        "--glob",
                        "!target/**",
                        "--glob",
                        "!node_modules/**",
                        "--",
                        &query,
                        ".",
                    ])
                    .current_dir(&root)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("Search pipe unavailable"))?;
                let mut results = vec![];
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        if v["type"] == "match" {
                            let data = &v["data"];
                            if let (Some(path), Some(line)) =
                                (data["path"]["text"].as_str(), data["line_number"].as_u64())
                            {
                                results.push((
                                    root.join(path.trim_start_matches("./")),
                                    line as usize,
                                    data["lines"]["text"]
                                        .as_str()
                                        .unwrap_or("")
                                        .trim()
                                        .chars()
                                        .take(160)
                                        .collect(),
                                ));
                            }
                        }
                    }
                    if results.len() >= 200 {
                        let _ = child.kill();
                        break;
                    }
                }
                let _ = child.wait();
                Ok(results)
            })();
            let _ = sender.send(match result {
                Ok(results) => Message::Search(query, results),
                Err(e) => Message::Notice(format!("Text search needs ripgrep (rg): {e}")),
            });
        });
        cx.notify();
    }
    pub fn checkpoint(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.say("Taking checkpoint…");
        std::thread::spawn(move || {
            let _ = sender.send(Message::Checkpoint(
                Checkpoint::capture(&root, "Manual checkpoint").map_err(|e| e.to_string()),
            ));
        });
        cx.notify();
    }
    /// Runs a blocking launcher off the UI thread and reports failures.
    fn launch(&mut self, run: fn(&Path) -> std::io::Result<()>, path: PathBuf) {
        self.menu = None;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            if let Err(e) = run(&path) {
                let _ = sender.send(Message::Notice(e.to_string()));
            }
        });
    }
    fn sidebar_index(&self) -> usize {
        usize::from(self.view == View::Review)
    }
    /// Whether the current view's sidebar (file tree or changed files) is open.
    pub(super) fn sidebar_open(&self) -> bool {
        !self.sidebar_hidden[self.sidebar_index()]
    }
    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        let index = self.sidebar_index();
        self.sidebar_hidden[index] = !self.sidebar_hidden[index];
        cx.notify();
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.view {
            View::Review => self.review_content(window, cx),
            View::Files => self.document_content(window, cx),
        };
        let side = self.sidebar_index();
        let wanted = self.sidebar_open();
        let slide = match self.sidebar_slide {
            Some(s) if s.view == side && s.shown == wanted => s,
            // The sidebar slides from wherever it is; switching views does not animate.
            Some(s) if s.view == side && !cx.reduce_motion() => Slide {
                view: side,
                shown: wanted,
                from: s.value(),
                start: Instant::now(),
            },
            _ => Slide::settled(side, wanted),
        };
        self.sidebar_slide = Some(slide);
        if slide.progress() < 1. {
            window.request_animation_frame();
        }
        let shown = slide.value();
        let width = self.sidebar_width[side];
        let sidebar = (shown > 0.001 && !self.quick_look).then(|| match self.view {
            View::Review => self.review_sidebar(window, cx),
            View::Files => self.files_sidebar(window, cx),
        });
        let main = if !self.quick_look {
            div()
                .id("browser-main")
                .flex()
                .size_full()
                .child(div().flex_1().min_w_0().h_full().child(content))
                .when_some(sidebar, |s, sidebar| {
                    // The sidebar keeps its width and is revealed from the right
                    // edge, so it slides instead of squeezing its contents.
                    s.child(
                        div()
                            .relative()
                            .h_full()
                            .flex_shrink_0()
                            .w(px(width * shown))
                            .overflow_hidden()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .w(px(width))
                                    .child(sidebar),
                            )
                            .child(theme::resize_handle("sidebar-resize", SidebarResize).left_0()),
                    )
                })
                .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<SidebarResize>, _, cx| {
                    this.sidebar_width[side] = f32::from(e.bounds.right() - e.event.position.x)
                        .clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                    cx.notify();
                }))
                .into_any_element()
        } else {
            div()
                .size_full()
                .child(content)
                .with_animation(
                    "quick-look",
                    Animation::new(Duration::from_millis(150)),
                    |el, t| el.opacity(t),
                )
                .into_any_element()
        };
        div()
            .track_focus(&self.focus)
            .key_context("FileBrowser")
            .on_key_down(cx.listener(Self::tree_key))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .text_color(rgb(theme::TEXT))
            .child(self.tab_strip(cx))
            .child(div().flex_1().min_h_0().child(main))
            .when(!self.notice.is_empty(), |s| {
                s.child(
                    div()
                        .absolute()
                        .bottom_3()
                        .left_3()
                        .max_w(px(460.))
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .bg(rgb(theme::SURFACE))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .shadow_lg()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_2))
                        .child(self.notice.clone())
                        .with_animation(
                            ("notice", self.notice_serial),
                            Animation::new(Duration::from_millis(120)),
                            |el, t| el.opacity(t),
                        ),
                )
            })
    }
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico"
        )
    })
}

fn start_watcher(root: PathBuf, sender: mpsc::Sender<Message>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let _ = sender.send(Message::Tasks(crate::tasks::history(&root)));
        let (event_tx, event_rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = event_tx.send(event);
        })
        .ok();
        let watching = watcher
            .as_mut()
            .is_some_and(|w| w.watch(&root, notify::RecursiveMode::Recursive).is_ok());
        // Staging changes only touch `.git`; when that folder is outside the watched
        // root (a subfolder of a repository) fall back to polling Git more often.
        let fallback = if watching && root.join(".git").exists() {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(5)
        };
        let mut refresh = true;
        let mut last = Instant::now() - fallback;
        let mut paths = BTreeSet::new();
        while !stop.load(Ordering::Relaxed) {
            for event in event_rx.try_iter().flatten() {
                for path in event.paths {
                    if path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(".vyber-save-"))
                    {
                        continue;
                    }
                    let relative = path.strip_prefix(&root).unwrap_or(&path);
                    let mut parts = relative.components().map(|c| c.as_os_str());
                    if let Some(position) = parts.clone().position(|c| c == ".git") {
                        // Index, HEAD and ref updates change Git status but are not user files.
                        let rest = parts.nth(position + 1).and_then(|c| c.to_str());
                        if matches!(rest, Some("index" | "HEAD" | "refs" | "MERGE_HEAD")) {
                            refresh = true;
                        }
                        continue;
                    }
                    if relative.components().any(|c| {
                        matches!(c.as_os_str().to_str(), Some("target" | "node_modules"))
                    }) {
                        continue;
                    }
                    paths.insert(path);
                    refresh = true;
                }
            }
            if refresh && last.elapsed() > Duration::from_millis(450) {
                for path in std::mem::take(&mut paths) {
                    if !path.is_dir() {
                        let bytes = fs::metadata(&path)
                            .ok()
                            .filter(|m| m.len() < TEXT_LIMIT as u64)
                            .and_then(|_| fs::read(&path).ok());
                        let _ = sender.send(Message::Changed(path, bytes));
                    }
                }
                let files = workspace::scan_files(&root);
                let changes = workspace::status(&root).unwrap_or_default();
                let branch =
                    workspace::git_text(&root, &["branch", "--show-current"]).unwrap_or_default();
                if sender.send(Message::Index(files, changes, branch)).is_err() {
                    break;
                }
                refresh = false;
                last = Instant::now();
            }
            if last.elapsed() > fallback {
                refresh = true;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    });
}
