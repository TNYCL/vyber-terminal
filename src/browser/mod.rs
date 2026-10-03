//! The file panel that opens over a terminal: a workspace tree, editor and
//! previews on the file side, and a Review tab for Git and task changes.
mod document;
mod layout;
#[cfg(test)]
mod lifecycle_tests;
mod review;
mod scm;
mod scm_ui;
mod tree;

use crate::{
    changeset::{self, Commit, FileChange, FileDiff},
    config::{Config, PanelMode},
    git::{self as vcs, ops},
    tasks::TaskReview,
    theme::{self, BORDER, PANEL, rpx},
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

actions!(
    vyber_files,
    [CloseFile, NextFile, PreviousFile, FindInPanel]
);

/// Editor-style shortcuts inside the file panel; the terminal keeps Ctrl+W.
/// Ctrl+Tab is bound for the whole window, which hands it to a focused panel
/// (see [`Browser::cycle_tab`]): a binding without context outranks the
/// panel's own inside the editor.
pub fn bind_keys(cx: &mut App) {
    let context = Some("FileBrowser");
    let close = if cfg!(target_os = "macos") {
        "cmd-w"
    } else {
        "ctrl-w"
    };
    let find = if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    };
    cx.bind_keys([
        KeyBinding::new(close, CloseFile, context),
        KeyBinding::new("ctrl-pagedown", NextFile, context),
        KeyBinding::new("ctrl-pageup", PreviousFile, context),
        KeyBinding::new(find, FindInPanel, context),
    ]);
}

const NOTICE_TIME: Duration = Duration::from_secs(4);
const SIDEBAR_MIN: f32 = 190.;
/// Widest sidebar of each view: files, review, Git.
const SIDEBAR_MAX: [f32; 3] = [460., 460., 600.];
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
    /// Preview terminalin yanına gömülür; eski kayıtlar yüzen modda açılır.
    #[serde(default)]
    pub docked: bool,
    #[serde(default)]
    pub wide: bool,
    #[serde(default)]
    pub tree_root: Option<String>,
    /// Panel width as a share of the window.
    #[serde(default)]
    pub panel_width: Option<f32>,
    /// Sidebar widths and hidden flags: `[files, review, git]`.
    #[serde(default)]
    pub sidebar_widths: Option<Vec<f32>>,
    #[serde(default)]
    pub hidden_sidebars: Vec<bool>,
    /// The panel shows the Git view.
    #[serde(default)]
    pub git: bool,
    #[serde(default)]
    pub git_repo: Option<PathBuf>,
    #[serde(default)]
    pub git_split: Option<f32>,
    #[serde(default)]
    pub git_graph: Option<bool>,
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
    saving: bool,
    pending_disk_change: Option<Option<Vec<u8>>>,
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
    Index(workspace::FileListing, Vec<Change>, String),
    NameSearch(u64, String, workspace::NameSearchResult),
    FolderListed(u64, PathBuf, workspace::FileListing),
    Loaded(u64, PathBuf, Result<Vec<u8>, String>, u64),
    Changed(PathBuf),
    DocumentChanged(PathBuf, Option<Vec<u8>>),
    Checkpoint(Result<Checkpoint, String>),
    Notice(String),
    Saved(PathBuf, Vec<u8>, SharedString, Result<(), String>),
    Search(u64, String, Result<Vec<(PathBuf, usize, String)>, String>),
    Tasks(Vec<TaskReview>),
    /// A review load: its file list (root, base, files), diffs in chunks,
    /// then the end, or an error. The first number is the load generation.
    ReviewList(u64, PathBuf, String, Vec<FileChange>),
    ReviewDiffs(u64, Vec<(usize, Arc<FileDiff>)>),
    ReviewDone(u64),
    ReviewError(u64, String),
    Commits(Result<Vec<Commit>, String>),
    /// A revert or staging change finished; the message is shown and the
    /// review reloads.
    Reverted(String),
    GitRepos(Vec<vcs::Repo>),
    GitStatuses(Vec<(PathBuf, Result<vcs::Status, String>)>),
    /// Generation, repository, whether it appends, the page size asked, and
    /// the commits with the incoming and outgoing hashes.
    #[allow(clippy::type_complexity)]
    GitGraph(
        u64,
        PathBuf,
        bool,
        usize,
        Result<(Vec<vcs::Commit>, Vec<String>, Vec<String>), String>,
    ),
    GitCommitInfo(String, Result<vcs::CommitInfo, String>),
    GitRefs(PathBuf, Vec<vcs::Ref>, Vec<vcs::Stash>),
    /// A Git command finished: repository, whether it showed as busy, result.
    GitDone(PathBuf, bool, ops::Outcome<scm::After>, scm::OnError),
}
#[derive(Clone, Copy, PartialEq)]
enum View {
    Files,
    Review,
    Git,
}
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Root,
    Folder(usize),
    Open,
    Source,
    More,
}

pub enum BrowserEvent {
    /// Ana yerleşim panelin yeni ölçüsünü ve görünürlüğünü tekrar hesaplar.
    LayoutChanged,
    /// A review comment to type into this terminal's input.
    Comment(String),
    /// Open a new terminal tab in this folder.
    OpenFolder(PathBuf),
    /// Type this command into the terminal without running it.
    RunInTerminal(String),
    /// Edit (or create) the project of this folder.
    EditProject(PathBuf),
    /// The panel was closed from the keyboard or its close button.
    Closed,
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

/// The sidebar sliding open or closed for one view (0 files, 1 review, 2 Git).
#[derive(Clone, Copy)]
struct Slide {
    view: usize,
    shown: bool,
    from: f32,
    start: Instant,
}

struct PendingLoad {
    generation: u64,
    pinned: bool,
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
    pub docked: bool,
    /// Panel covers the whole terminal area instead of the right side.
    pub wide: bool,
    /// Panel width as a share of the window; `None` is the default.
    pub panel_width: Option<f32>,
    sidebar_width: [f32; 3],
    sidebar_hidden: [bool; 3],
    sidebar_slide: Option<Slide>,
    /// When the view last changed, for the crossfade.
    view_serial: u64,
    shown_view: Option<View>,
    pub branch: String,
    pub tasks: Vec<TaskReview>,
    files: Vec<FileEntry>,
    tree_errors: Vec<String>,
    name_results: Vec<FileEntry>,
    name_query: String,
    name_errors: Vec<String>,
    name_truncated: bool,
    name_loading: bool,
    name_generation: u64,
    pending_name_open: Option<u64>,
    name_cancel: Arc<AtomicBool>,
    statuses: HashMap<String, char>,
    changed_dirs: HashSet<String>,
    search_query: String,
    search_results: Vec<(PathBuf, usize, String)>,
    search_generation: u64,
    search_cancel: Arc<AtomicBool>,
    changes: Vec<Change>,
    expanded: BTreeSet<String>,
    docs: Vec<Document>,
    active_doc: usize,
    filter: Entity<InputState>,
    view: View,
    review: review::ReviewState,
    /// The review of the other view (Review tab or Git view), kept aside.
    parked: review::ReviewState,
    git: scm::GitState,
    checkpoint: Option<Checkpoint>,
    follow: bool,
    notice: String,
    notice_at: Instant,
    notice_serial: u64,
    loading: bool,
    zoom: f32,
    /// How much the panel's text size scales it, for turning dragged pixels
    /// into widths that scale along.
    scale: f32,
    restore_docs: Vec<SavedDocument>,
    restore_active: Option<PathBuf>,
    pending_lines: HashMap<PathBuf, usize>,
    pending_loads: HashMap<PathBuf, PendingLoad>,
    load_generation: u64,
    intended_document: Option<PathBuf>,
    focus: FocusHandle,
    quick_look: bool,
    tree_root: Option<String>,
    tree_scroll: UniformListScrollHandle,
    tree_selected: Option<String>,
    reveal: bool,
    menu: Option<Menu>,
    folder_menu: Option<document::FolderMenu>,
    /// The menu an outside mouse-down just closed, so the click that follows on
    /// its own trigger button does not reopen it.
    menu_dismissed: Option<(Menu, Instant)>,
    tab_scroll: ScrollHandle,
    receiver: mpsc::Receiver<Message>,
    sender: mpsc::Sender<Message>,
    document_receiver: mpsc::Receiver<Message>,
    document_sender: mpsc::Sender<Message>,
    document_paths: Arc<std::sync::Mutex<HashMap<PathBuf, Vec<u8>>>>,
    document_stop: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    scope: Scope,
    _subscriptions: Vec<Subscription>,
}
impl Drop for Browser {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.document_stop.store(true, Ordering::Relaxed);
        self.name_cancel.store(true, Ordering::Relaxed);
        self.search_cancel.store(true, Ordering::Relaxed);
    }
}

impl Browser {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (document_sender, document_receiver) = mpsc::channel();
        let document_stop = Arc::new(AtomicBool::new(false));
        let document_paths = Arc::new(std::sync::Mutex::new(HashMap::new()));
        start_document_watcher(
            document_paths.clone(),
            document_sender.clone(),
            document_stop.clone(),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let scope = Scope::default();
        start_watcher(root.clone(), sender.clone(), stop.clone(), scope.clone());
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter files…")
                .clean_on_escape()
        });
        let subscription = cx.subscribe_in(&filter, window, |this, _, event, window, cx| {
            match event {
                InputEvent::PressEnter { shift, .. } => {
                    if *shift || !this.open_best_match(window, cx) {
                        this.search_contents(cx);
                    }
                    window.focus(&this.focus, cx);
                }
                InputEvent::Change => {
                    this.reveal = this.filter_query(cx).is_empty();
                    this.search_generation += 1;
                    this.search_cancel.store(true, Ordering::Relaxed);
                    this.search_query.clear();
                    this.search_results.clear();
                    this.search_names(cx);
                }
                _ => {}
            }
            cx.notify();
        });
        cx.spawn_in(window, async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if entity
                    .update_in(cx, |view, window, cx| view.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let (review, review_subscriptions) = review::ReviewState::new(false, window, cx);
        let (parked, parked_subscriptions) = review::ReviewState::new(true, window, cx);
        let mut subscriptions = vec![subscription];
        subscriptions.extend(review_subscriptions);
        subscriptions.extend(parked_subscriptions);
        Self {
            root,
            visible: false,
            docked: cx.global::<Config>().panel_mode == PanelMode::Dock,
            wide: false,
            panel_width: None,
            sidebar_width: [260., 250., 340.],
            sidebar_hidden: [false; 3],
            sidebar_slide: None,
            view_serial: 0,
            shown_view: None,
            branch: String::new(),
            tasks: vec![],
            files: vec![],
            tree_errors: vec![],
            name_results: vec![],
            name_query: String::new(),
            name_errors: vec![],
            name_truncated: false,
            name_loading: false,
            name_generation: 0,
            pending_name_open: None,
            name_cancel: Arc::new(AtomicBool::new(false)),
            statuses: HashMap::new(),
            changed_dirs: HashSet::new(),
            search_query: String::new(),
            search_results: vec![],
            search_generation: 0,
            search_cancel: Arc::new(AtomicBool::new(false)),
            changes: vec![],
            expanded: BTreeSet::new(),
            docs: vec![],
            active_doc: 0,
            filter,
            view: View::Files,
            review,
            parked,
            git: scm::GitState::new(),
            checkpoint: None,
            follow: false,
            notice: String::new(),
            notice_at: Instant::now(),
            notice_serial: 0,
            loading: true,
            zoom: 1.,
            scale: 1.,
            restore_docs: vec![],
            restore_active: None,
            pending_lines: HashMap::new(),
            pending_loads: HashMap::new(),
            load_generation: 0,
            intended_document: None,
            focus: cx.focus_handle(),
            quick_look: false,
            tree_root: None,
            tree_scroll: UniformListScrollHandle::new(),
            tree_selected: None,
            reveal: false,
            menu: None,
            folder_menu: None,
            menu_dismissed: None,
            tab_scroll: ScrollHandle::new(),
            receiver,
            sender,
            document_receiver,
            document_sender,
            document_paths,
            document_stop,
            stop,
            scope,
            _subscriptions: subscriptions,
        }
    }
    /// Whether the panel or anything in it has keyboard focus.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }
    /// Shows `text` briefly at the bottom of the panel.
    pub fn announce(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.say(text);
        cx.notify();
    }
    /// Floats every file panel over its terminal, or docks it beside the
    /// terminal, which then gets narrower.
    pub(super) fn dock_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let docked = cx.global::<Config>().panel_mode == PanelMode::Dock;
        theme::icon_button(
            "panel-mode",
            if docked { "panel-float" } else { "panel-dock" },
            if docked {
                "Float over terminal"
            } else {
                "Dock beside terminal"
            },
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_docked(cx)))
    }
    fn close_file(&mut self, _: &CloseFile, window: &mut Window, cx: &mut Context<Self>) {
        self.close_front(window, cx);
    }
    /// Ctrl+W closes the file in front, or the panel when no file is.
    pub fn close_front(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view == View::Files && self.active_doc < self.docs.len() {
            self.close_doc(self.active_doc, cx);
            self.focus_shown_tab(window, cx);
        } else {
            self.close_panel(cx);
        }
    }
    fn next_file(&mut self, _: &NextFile, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cycle_tab(1, window, cx) {
            cx.propagate();
        }
    }
    fn previous_file(&mut self, _: &PreviousFile, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cycle_tab(-1, window, cx) {
            cx.propagate();
        }
    }
    /// Moves along the tab strip (Review, then the open files), wrapping
    /// around. Source control has no tabs: there it returns false and leaves
    /// the shortcut to the window.
    pub fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.view == View::Git {
            return false;
        }
        let count = self.docs.len() as isize + 1;
        let current = if self.view == View::Review || self.docs.is_empty() {
            0
        } else {
            self.active_doc as isize + 1
        };
        match (current + step).rem_euclid(count) {
            0 => self.show_review(cx),
            next => {
                let index = next as usize - 1;
                self.select_document(index, cx);
            }
        }
        self.focus_shown_tab(window, cx);
        cx.notify();
        true
    }
    fn find_in_panel(&mut self, _: &FindInPanel, window: &mut Window, cx: &mut Context<Self>) {
        match self.view {
            View::Review => self.open_find(window, cx),
            // The editor has its own find; elsewhere there is nothing to search.
            _ => cx.propagate(),
        }
    }
    /// Keeps the keyboard in the panel after the shown tab changed: in the
    /// editor when the tab shows one, else on the panel itself.
    fn focus_shown_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self
            .docs
            .get(self.active_doc)
            .filter(|d| {
                self.view == View::Files && d.kind == Kind::Text && !(d.preview && d.markdown())
            })
            .map(|d| d.editor.clone());
        match editor {
            Some(editor) => editor.update(cx, |e, cx| e.focus(window, cx)),
            None => window.focus(&self.focus, cx),
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
        let relative = path
            .strip_prefix(&self.root)
            .map(Path::to_owned)
            .ok()
            .or_else(|| {
                path.canonicalize()
                    .ok()?
                    .strip_prefix(self.root.canonicalize().ok()?)
                    .ok()
                    .map(Path::to_owned)
            });
        relative
            .as_deref()
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
    pub fn root_matches(&self, root: &Path) -> bool {
        self.root == root
            || self
                .root
                .canonicalize()
                .ok()
                .zip(root.canonicalize().ok())
                .is_some_and(|(left, right)| left == right)
    }
    fn active_document(&self) -> Option<&Document> {
        self.docs.get(self.active_doc)
    }

    pub fn has_dirty(&self) -> bool {
        self.docs.iter().any(|d| d.dirty) || self.restore_docs.iter().any(|doc| doc.draft.is_some())
    }
    pub fn owns_focus(&self, window: &Window, cx: &App) -> bool {
        self.docs
            .iter()
            .any(|d| d.editor.read(cx).focus_handle(cx).is_focused(window))
    }
    #[cfg(test)]
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "Used by macOS interaction tests.")
    )]
    pub(crate) fn test_document(
        &mut self,
        path: PathBuf,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible = true;
        self.view = View::Files;
        self.loaded(
            path,
            Ok(text.as_bytes().to_vec()),
            text.len() as u64,
            true,
            window,
            cx,
        );
        if let Some(doc) = self.docs.get_mut(self.active_doc) {
            doc.preview = false;
            doc.editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        cx.notify();
    }
    pub fn close_active_document(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.view != View::Files || self.active_document().is_none() {
            return false;
        }
        let focused = self.owns_focus(window, cx);
        // Kirli dosya kapanmayı reddederse komut panel veya terminale geçmez.
        self.close_doc(self.active_doc, cx);
        if focused {
            let focus = self
                .active_document()
                .map(|doc| doc.editor.read(cx).focus_handle(cx))
                .unwrap_or_else(|| self.focus.clone());
            window.focus(&focus, cx);
        }
        true
    }
    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.pending_loads
            .retain(|path, _| self.restore_docs.iter().any(|doc| &doc.path == path));
        self.pending_lines
            .retain(|path, _| self.pending_loads.contains_key(path));
        self.intended_document = self.active_document().map(|doc| doc.path.clone());
        self.menu = None;
        self.git.menu = None;
        cx.emit(BrowserEvent::Closed);
        self.layout_changed(cx);
    }
    pub(super) fn layout_changed(&self, cx: &mut Context<Self>) {
        cx.emit(BrowserEvent::LayoutChanged);
        cx.notify();
    }
    pub(super) fn toggle_docked(&mut self, cx: &mut Context<Self>) {
        let docked = cx.global::<Config>().panel_mode != PanelMode::Dock;
        Config::update(cx, |config| {
            config.panel_mode = if docked {
                PanelMode::Dock
            } else {
                PanelMode::Overlay
            };
        });
        self.docked = docked;
        if self.docked {
            self.wide = false;
        }
        self.layout_changed(cx);
    }
    pub fn state(&self, cx: &App) -> BrowserState {
        // The app persists immediately after restoration. Keep saved drafts
        // that are still awaiting their asynchronous reads in that snapshot.
        let mut docs = self.restore_docs.clone();
        for doc in &self.docs {
            let saved = SavedDocument {
                path: doc.path.clone(),
                preview: doc.preview,
                pinned: doc.pinned,
                scroll: (
                    f32::from(doc.scroll.offset().x),
                    f32::from(doc.scroll.offset().y),
                ),
                draft: doc.dirty.then(|| doc.editor.read(cx).value().to_string()),
                baseline: doc.dirty.then(|| doc.baseline.clone()),
            };
            if let Some(index) = docs.iter().position(|saved| saved.path == doc.path) {
                if !matches!(doc.kind, Kind::Unsupported(_)) || docs[index].draft.is_none() {
                    docs[index] = saved;
                }
            } else {
                docs.push(saved);
            }
        }
        let active = self
            .restore_active
            .clone()
            .filter(|path| docs.iter().any(|doc| &doc.path == path))
            .or_else(|| self.active_document().map(|doc| doc.path.clone()));
        BrowserState {
            visible: self.visible,
            expanded: self.expanded.clone(),
            active,
            follow: self.follow,
            pinned: false,
            docked: cx.global::<Config>().panel_mode == PanelMode::Dock,
            wide: self.wide,
            tree_root: self.tree_root.clone(),
            panel_width: self.panel_width,
            sidebar_widths: Some(self.sidebar_width.to_vec()),
            hidden_sidebars: self.sidebar_hidden.to_vec(),
            git: self.view == View::Git,
            git_repo: self.git.selected.clone(),
            git_split: Some(self.git.split),
            git_graph: Some(self.git.graph_open),
            docs,
        }
    }
    pub fn restore_state(&mut self, state: BrowserState, cx: &mut Context<Self>) {
        self.visible = state.visible;
        self.expanded = state.expanded;
        // Eski dosya kilidi, gömme moduna dönüşmeden takip kapalı olarak korunur.
        self.follow = state.follow && !state.pinned;
        // Yerel kayıtlardaki gömülü panel tercihi ortak yapılandırmaya taşınır.
        if state.docked && cx.global::<Config>().panel_mode != PanelMode::Dock {
            Config::update(cx, |config| config.panel_mode = PanelMode::Dock);
        }
        self.docked = cx.global::<Config>().panel_mode == PanelMode::Dock;
        self.wide = state.wide;
        self.tree_root = state.tree_root;
        self.panel_width = state.panel_width.filter(|w| (0.2..=1.).contains(w));
        if let Some(widths) = state.sidebar_widths {
            for (i, width) in widths.into_iter().enumerate().take(3) {
                self.sidebar_width[i] = width.clamp(SIDEBAR_MIN, SIDEBAR_MAX[i]);
            }
        }
        for (i, hidden) in state.hidden_sidebars.into_iter().enumerate().take(3) {
            self.sidebar_hidden[i] = hidden;
        }
        if state.git {
            self.view = View::Git;
            self.view_changed();
        }
        self.git.selected = state.git_repo;
        if let Some(split) = state.git_split {
            self.git.split = split.clamp(0.15, 0.85);
        }
        if let Some(open) = state.git_graph {
            self.git.graph_open = open;
        }
        self.restore_docs = state.docs;
        let paths = self
            .restore_docs
            .iter()
            .map(|d| d.path.clone())
            .collect::<Vec<_>>();
        self.restore_active = state.active.or_else(|| paths.first().cloned());
        self.intended_document = self.restore_active.clone();
        for path in paths {
            self.queue_load(path, true);
        }
        self.sync_tree_scope();
    }
    pub fn open_at(
        &mut self,
        path: PathBuf,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible = true;
        self.pending_lines.insert(path.clone(), line);
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
            self.pending_lines.remove(&path);
        }
    }
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.view = View::Files;
        self.sidebar_hidden[0] = false;
        self.quick_look = false;
        self.menu = None;
        self.restore_active = None;
        self.intended_document = self.active_document().map(|doc| doc.path.clone());
        self.filter.update(cx, |input, cx| input.focus(window, cx));
        self.layout_changed(cx);
    }
    pub fn change_root(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.root_matches(&root) {
            return;
        }
        self.stop.store(true, Ordering::Relaxed);
        self.stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        self.sender = sender;
        self.receiver = receiver;
        self.root = root.clone();
        self.files.clear();
        self.tree_errors.clear();
        self.name_cancel.store(true, Ordering::Relaxed);
        self.name_generation += 1;
        self.pending_name_open = None;
        self.name_loading = false;
        self.name_results.clear();
        self.name_query.clear();
        self.name_errors.clear();
        self.name_truncated = false;
        self.search_cancel.store(true, Ordering::Relaxed);
        self.search_generation += 1;
        self.search_query.clear();
        self.search_results.clear();
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.menu = None;
        self.folder_menu = None;
        self.menu_dismissed = None;
        self.changes.clear();
        self.statuses.clear();
        self.changed_dirs.clear();
        self.expanded.clear();
        self.tasks.clear();
        self.reset_review();
        self.git = scm::GitState::new();
        self.tree_root = None;
        self.tree_selected = None;
        self.reveal = false;
        self.tree_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.loading = true;
        self.scope = Scope::default();
        self.sync_tree_scope();
        start_watcher(
            root,
            self.sender.clone(),
            self.stop.clone(),
            self.scope.clone(),
        );
        cx.notify();
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let messages: Vec<_> = self
            .receiver
            .try_iter()
            .chain(self.document_receiver.try_iter())
            .collect();
        let mut changed = !messages.is_empty();
        let mut follow_path = None;
        for message in messages {
            match message {
                Message::Index(listing, changes, branch) => {
                    self.tree_errors = listing.errors;
                    if self.files != listing.entries {
                        self.files = listing.entries;
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
                    self.git.stale = true;
                    if let Some(root) = &self.tree_root
                        && !self.files.iter().any(|f| &f.relative == root)
                        && !self.root.join(root).is_dir()
                    {
                        self.tree_root = None;
                        self.tree_selected = None;
                        self.sync_tree_scope();
                        self.search_names(cx);
                    }
                }
                Message::NameSearch(generation, query, listing) => {
                    self.name_search_loaded(generation, query, listing, window, cx);
                }
                Message::FolderListed(generation, path, listing) => {
                    self.folder_listed(generation, path, listing, cx)
                }
                Message::Loaded(generation, path, result, size) => {
                    if self
                        .pending_loads
                        .get(&path)
                        .is_some_and(|pending| pending.generation == generation)
                    {
                        let pending = self.pending_loads.remove(&path).unwrap();
                        self.loaded(path, result, size, pending.pinned, window, cx);
                    }
                }
                Message::Changed(path) => {
                    self.review_worktree_changed(false);
                    follow_path = Some(path);
                }
                Message::DocumentChanged(path, bytes) => {
                    self.document_changed(&path, bytes, window, cx)
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
                Message::ReviewList(generation, root, base, files) => self
                    .with_review_of(generation, |this| {
                        this.review_listed(generation, root, base, files)
                    }),
                Message::ReviewDiffs(generation, diffs) => {
                    self.with_review_of(generation, |this| this.review_diffs(generation, diffs))
                }
                Message::ReviewDone(generation) => {
                    self.with_review_of(generation, |this| this.review_done(generation))
                }
                Message::ReviewError(generation, error) => {
                    self.with_review_of(generation, |this| this.review_failed(generation, error))
                }
                Message::Commits(commits) => self.review_commits(commits),
                Message::Reverted(message) => {
                    self.say(message);
                    self.review_reload_now();
                    self.git.stale = true;
                }
                Message::GitRepos(repos) => self.git_repos_loaded(repos, window, cx),
                Message::GitStatuses(results) => self.git_statuses_loaded(results),
                Message::GitGraph(generation, repo, more, count, result) => {
                    self.git_graph_loaded(generation, repo, more, count, result)
                }
                Message::GitCommitInfo(hash, info) => self.git.info = Some((hash, info)),
                Message::GitRefs(repo, refs, stashes) => self.git_refs_loaded(repo, refs, stashes),
                Message::GitDone(repo, busy, result, on_error) => {
                    self.git_done(repo, busy, result, on_error, window, cx)
                }
                Message::Notice(message) => self.say(message),
                Message::Saved(path, bytes, value, result) => match result {
                    Ok(()) => {
                        let mut pending_disk_change = None;
                        if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
                            doc.saving = false;
                            doc.size = bytes.len() as u64;
                            doc.baseline = bytes;
                            doc.text = value;
                            doc.dirty = doc.editor.read(cx).value() != doc.text;
                            doc.conflict = false;
                            pending_disk_change = doc.pending_disk_change.take();
                        }
                        if let Some(bytes) = pending_disk_change {
                            self.document_changed(&path, bytes, window, cx);
                        }
                        self.sync_document_watch();
                        self.say("Saved");
                    }
                    Err(e) => {
                        let mut pending_disk_change = None;
                        if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
                            doc.saving = false;
                            pending_disk_change = doc.pending_disk_change.take();
                        }
                        if let Some(bytes) = pending_disk_change {
                            self.document_changed(&path, bytes, window, cx);
                        }
                        self.say(e);
                    }
                },
                Message::Search(generation, query, results) => {
                    if generation == self.search_generation && query == self.filter_query(cx) {
                        match results {
                            Ok(results) => {
                                self.say(if results.len() == 200 {
                                    "Showing up to 200 matches in file contents".to_string()
                                } else {
                                    format!("{} matches in file contents", results.len())
                                });
                                self.search_query = query;
                                self.search_results = results;
                            }
                            Err(error) => self.say(format!("Text search failed: {error}")),
                        }
                    }
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
        self.view_changed();
        changed |= self.review_tick(cx);
        self.git_tick();
        if self.follow
            && !self
                .docs
                .iter()
                .any(|d| d.dirty || d.editor.read(cx).focus_handle(cx).is_focused(window))
            && let Some(path) = follow_path
            && path.is_file()
        {
            self.open(path, false, cx);
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
        let activate = self.intended_document.as_ref() == Some(&path)
            && (self.view == View::Files || self.restore_active.as_ref() == Some(&path));
        if !activate && !pinned && !self.restore_docs.iter().any(|doc| doc.path == path) {
            self.pending_lines.remove(&path);
            return;
        }
        if let Some(index) = self.docs.iter().position(|d| d.path == path) {
            if activate {
                self.active_doc = index;
            }
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
                        Kind::Unsupported("Not UTF-8 text · Vyber only edits UTF-8 files".into()),
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
        let text = editor.read(cx).value();
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
            saving: false,
            pending_disk_change: None,
            conflict: false,
            pinned,
            scroll: ScrollHandle::new(),
            _subscription: sub,
        };
        if let Some(i) = self.restore_docs.iter().position(|d| d.path == doc.path) {
            let retain_draft =
                matches!(doc.kind, Kind::Unsupported(_)) && self.restore_docs[i].draft.is_some();
            let saved = if retain_draft {
                self.restore_docs[i].clone()
            } else {
                self.restore_docs.remove(i)
            };
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
        if let Some(line) = self.pending_lines.remove(&doc.path) {
            doc.preview = false;
            doc.editor.update(cx, |editor, cx| {
                editor.set_cursor_position(
                    gpui_kit::component::input::Position::new(line.saturating_sub(1) as u32, 0),
                    window,
                    cx,
                )
            });
        }
        let index = if activate && !pinned {
            self.docs
                .iter()
                .position(|d| !d.pinned && !d.dirty && !d.saving)
        } else {
            None
        };
        let index = if let Some(index) = index {
            self.docs[index] = doc;
            index
        } else {
            self.docs.push(doc);
            self.docs.len() - 1
        };
        if activate {
            self.active_doc = index;
            self.tab_scroll.scroll_to_item(index);
        }
        self.sync_document_watch();
    }
    /// Selects a loaded document and records the user's choice before other
    /// asynchronous reads complete.
    pub(super) fn select_document(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(index) else {
            return;
        };
        let path = doc.path.clone();
        self.restore_active = None;
        self.intended_document = Some(path.clone());
        self.active_doc = index;
        self.view = View::Files;
        self.menu = None;
        self.reveal_document(&path, cx);
        self.tab_scroll.scroll_to_item(index);
        cx.notify();
    }
    fn reveal_document(&mut self, path: &Path, cx: &mut Context<Self>) {
        let relative = self.relative(path);
        if !Path::new(&relative).is_absolute() {
            if self
                .tree_root
                .as_ref()
                .is_some_and(|root| !relative.starts_with(&format!("{root}/")))
            {
                self.tree_root = None;
                self.search_cancel.store(true, Ordering::Relaxed);
                self.search_generation += 1;
                self.search_query.clear();
                self.search_results.clear();
                self.search_names(cx);
            }
            self.expanded.extend(workspace::expanded_parents(&relative));
            self.tree_selected = Some(relative);
            self.reveal = true;
            self.sync_tree_scope();
        }
    }
    pub fn open(&mut self, path: PathBuf, pinned: bool, cx: &mut Context<Self>) {
        self.restore_active = None;
        self.intended_document = Some(path.clone());
        self.view = View::Files;
        self.menu = None;
        self.reveal_document(&path, cx);
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            if !matches!(self.docs[i].kind, Kind::Unsupported(_)) {
                self.docs[i].pinned |= pinned;
                self.select_document(i, cx);
                return;
            }
            self.docs.remove(i);
            self.active_doc = self
                .active_doc
                .saturating_sub(usize::from(self.active_doc >= i));
        }
        self.queue_load(path, pinned);
        cx.notify();
    }
    fn queue_load(&mut self, path: PathBuf, pinned: bool) {
        if let Some(pending) = self.pending_loads.get_mut(&path) {
            pending.pinned |= pinned;
            return;
        }
        self.load_generation += 1;
        let generation = self.load_generation;
        self.pending_loads
            .insert(path.clone(), PendingLoad { generation, pinned });
        let sender = self.document_sender.clone();
        std::thread::spawn(move || {
            let (result, size) = match fs::metadata(&path) {
                Err(e) => (Err(format!("Cannot open {}: {e}", path.display())), 0),
                Ok(m) if m.len() > READ_LIMIT => (
                    Err("Larger than the 32 MB preview limit".to_string()),
                    m.len(),
                ),
                Ok(m) => (fs::read(&path).map_err(|e| e.to_string()), m.len()),
            };
            let _ = sender.send(Message::Loaded(generation, path, result, size));
        });
    }
    fn close_doc(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.docs.get(index).is_some_and(|doc| {
            self.restore_docs
                .iter()
                .any(|saved| saved.path == doc.path && saved.draft.is_some())
        }) {
            self.say(
                "Restore this file and reopen it to recover your unsaved draft before closing.",
            );
        } else if self.docs.get(index).is_some_and(|d| d.dirty) {
            self.say("Save the document before closing it.");
        } else if index < self.docs.len() {
            let path = self.docs.remove(index).path;
            self.pending_loads.remove(&path);
            self.pending_lines.remove(&path);
            if self.active_doc > index || self.active_doc >= self.docs.len() {
                self.active_doc = self.active_doc.saturating_sub(1);
            }
            if self.intended_document.as_ref() == Some(&path) {
                self.intended_document = self.active_document().map(|doc| doc.path.clone());
            }
            self.sync_document_watch();
        }
        cx.notify();
    }
    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.active_document() else {
            return;
        };
        if !doc.dirty || doc.saving || self.view != View::Files {
            return;
        }
        let value = doc.editor.read(cx).value();
        let mut text = value.to_string();
        if doc.baseline.windows(2).any(|pair| pair == b"\r\n") {
            text = text.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        let mut bytes = vec![];
        if doc.baseline.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.extend([0xef, 0xbb, 0xbf]);
        }
        bytes.extend(text.as_bytes());
        let path = doc.path.clone();
        let baseline = doc.baseline.clone();
        let sender = self.document_sender.clone();
        self.docs[self.active_doc].saving = true;
        self.say("Saving…");
        std::thread::spawn(move || {
            let result =
                workspace::save_checked(&path, &baseline, &bytes).map_err(|e| e.to_string());
            let _ = sender.send(Message::Saved(path, bytes, value, result));
        });
        cx.notify();
    }
    fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = self.docs.get_mut(self.active_doc)
            && let Ok(bytes) = fs::read(&doc.path)
            && let Ok(text) = String::from_utf8(bytes.clone())
        {
            let text = text.trim_start_matches('\u{feff}').to_owned();
            doc.size = bytes.len() as u64;
            doc.baseline = bytes;
            doc.dirty = false;
            doc.conflict = false;
            doc.editor
                .update(cx, |ed, cx| ed.set_value(text, window, cx));
            doc.text = doc.editor.read(cx).value();
        }
        self.sync_document_watch();
        cx.notify();
    }
    fn sync_document_watch(&self) {
        *self.document_paths.lock().unwrap() = self
            .docs
            .iter()
            .filter(|doc| doc.kind == Kind::Text)
            .map(|doc| (doc.path.clone(), doc.baseline.clone()))
            .collect();
    }
    fn document_changed(
        &mut self,
        path: &Path,
        bytes: Option<Vec<u8>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.iter_mut().find(|doc| doc.path == path) else {
            return;
        };
        if doc.kind != Kind::Text {
            return;
        }
        if doc.saving {
            if bytes.as_ref() != Some(&doc.baseline) {
                doc.pending_disk_change = Some(bytes);
            }
            return;
        }
        if bytes.as_ref() == Some(&doc.baseline) {
            return;
        }
        if doc.dirty || bytes.is_none() {
            doc.conflict = true;
        } else if let Some(bytes) = bytes {
            match std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)) {
                Ok(text) if !bytes.contains(&0) => {
                    let text = text.to_owned();
                    doc.size = bytes.len() as u64;
                    doc.baseline = bytes;
                    doc.editor.update(cx, |editor, cx| {
                        let offset = editor.scroll_offset();
                        editor.set_value(text, window, cx);
                        editor.set_scroll_offset(offset, cx);
                    });
                    doc.text = doc.editor.read(cx).value();
                    doc.conflict = false;
                }
                _ => doc.conflict = true,
            }
        }
        self.sync_document_watch();
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
        self.search_cancel.store(true, Ordering::Relaxed);
        self.search_cancel = Arc::new(AtomicBool::new(false));
        self.search_generation += 1;
        let generation = self.search_generation;
        let cancelled = self.search_cancel.clone();
        let root = self
            .tree_root
            .as_ref()
            .map(|relative| self.root.join(relative))
            .unwrap_or_else(|| self.root.clone());
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result =
                search_contents_in(&root, &query, &cancelled).map_err(|error| error.to_string());
            if !cancelled.load(Ordering::Relaxed) {
                let _ = sender.send(Message::Search(generation, query, result));
            }
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
        let sender = self.document_sender.clone();
        std::thread::spawn(move || {
            if let Err(e) = run(&path) {
                let _ = sender.send(Message::Notice(e.to_string()));
            }
        });
    }
    fn sidebar_index(&self) -> usize {
        match self.view {
            View::Files => 0,
            View::Review => 1,
            View::Git => scm::SIDEBAR,
        }
    }
    /// Runs `f` with the review that started load `generation` as
    /// `self.review`, whichever view is showing.
    fn with_review_of(&mut self, generation: u64, f: impl FnOnce(&mut Self)) {
        let parked = self.parked.generation.load(Ordering::Relaxed) == generation
            && self.review.generation.load(Ordering::Relaxed) != generation;
        if parked {
            std::mem::swap(&mut self.review, &mut self.parked);
        }
        f(self);
        if parked {
            std::mem::swap(&mut self.review, &mut self.parked);
        }
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
        self.scale = f32::from(window.rem_size()) / theme::REM;
        self.view_changed();
        if self.shown_view != Some(self.view) {
            self.shown_view = Some(self.view);
            self.view_serial += 1;
        }
        let content = match self.view {
            View::Review => self.review_content(window, cx),
            View::Files => self.document_content(window, cx),
            View::Git => self.git_content(window, cx),
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
            View::Git => self.git_sidebar(window, cx),
        });
        let main = if !self.quick_look {
            let browser = cx.entity();
            div()
                .size_full()
                .child(
                    layout::SidebarLayout::new(
                        content,
                        sidebar,
                        width * self.scale,
                        shown,
                        self.scale,
                    )
                    .on_resize(move |width, _, cx| {
                        browser.update(cx, |this, cx| {
                            this.sidebar_width[side] = width.clamp(SIDEBAR_MIN, SIDEBAR_MAX[side]);
                            cx.notify();
                        });
                    }),
                )
                .with_animation(
                    ("browser-view", self.view_serial),
                    Animation::new(Duration::from_millis(140)),
                    |el, t| el.opacity(0.35 + 0.65 * t),
                )
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
            .on_action(cx.listener(Self::close_file))
            .on_action(cx.listener(Self::next_file))
            .on_action(cx.listener(Self::previous_file))
            .on_action(cx.listener(Self::find_in_panel))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .text_size(rpx(12.))
            .text_color(rgb(theme::TEXT))
            .child(if self.view == View::Git {
                self.git_strip(cx)
            } else {
                self.tab_strip(cx).into_any_element()
            })
            .child(div().flex_1().min_h_0().child(main))
            .when(self.view == View::Git, |s| {
                s.children(self.git_overlays(window, cx))
            })
            .when(!self.notice.is_empty(), |s| {
                s.child(
                    div()
                        .absolute()
                        .bottom_3()
                        .left_3()
                        .max_w(rpx(460.))
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .bg(rgb(theme::SURFACE))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .shadow_lg()
                        .text_size(rpx(12.))
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

/// Open text documents stay observed even when the terminal moves elsewhere.
/// Metadata checks avoid repeatedly reading unchanged buffers.
fn start_document_watcher(
    paths: Arc<std::sync::Mutex<HashMap<PathBuf, Vec<u8>>>>,
    sender: mpsc::Sender<Message>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut observed = HashMap::new();
        while !stop.load(Ordering::Relaxed) {
            let registered: Vec<_> = paths
                .lock()
                .unwrap()
                .iter()
                .map(|(path, baseline)| {
                    (
                        path.clone(),
                        (!observed.contains_key(path)).then(|| baseline.clone()),
                    )
                })
                .collect();
            observed.retain(|path, _| registered.iter().any(|(current, _)| current == path));
            for (path, baseline) in registered {
                let metadata = fs::metadata(&path).ok();
                let stamp = metadata
                    .as_ref()
                    .map(|metadata| (metadata.len(), metadata.modified().ok()));
                if observed.get(&path) == Some(&stamp) {
                    continue;
                }
                let bytes = metadata
                    .filter(|metadata| metadata.len() <= TEXT_LIMIT as u64)
                    .and_then(|_| fs::read(&path).ok());
                observed.insert(path.clone(), stamp);
                if baseline
                    .as_ref()
                    .is_none_or(|baseline| bytes.as_ref() != Some(baseline))
                    && sender.send(Message::DocumentChanged(path, bytes)).is_err()
                {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

fn search_contents_in(
    root: &Path,
    query: &str,
    cancelled: &AtomicBool,
) -> anyhow::Result<Vec<(PathBuf, usize, String)>> {
    use std::io::{BufRead, Read};
    let mut child = workspace::command("rg")
        .args([
            "--json",
            "--hidden",
            "--no-ignore",
            "--fixed-strings",
            "--smart-case",
            "--max-count",
            "5",
            "--",
            query,
            ".",
        ])
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| anyhow::anyhow!("Cannot start ripgrep (rg): {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Search pipe unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("Search error pipe unavailable"))?;
    let (sender, receiver) = mpsc::sync_channel(32);
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.take(64 * 1024).read_to_string(&mut text);
        text
    });
    let mut results = vec![];
    let mut truncated = false;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                    && value["type"] == "match"
                {
                    let data = &value["data"];
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
                    if results.len() >= 200 {
                        truncated = true;
                        let _ = child.kill();
                        break;
                    }
                }
            }
            Ok(Err(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(receiver);
                let _ = reader.join();
                let _ = errors.join();
                return Err(error.into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    drop(receiver);
    let _ = reader.join();
    let errors = errors.join().unwrap_or_default();
    if !truncated
        && !cancelled.load(Ordering::Relaxed)
        && !status.success()
        && status.code() != Some(1)
    {
        anyhow::bail!("ripgrep exited with {status}: {}", errors.trim());
    }
    Ok(results)
}

/// Expanded tree folders and the project's other roots to list explicitly.
#[derive(Clone, Default)]
struct Scope {
    extra: Arc<std::sync::Mutex<Vec<PathBuf>>>,
    tree: Arc<std::sync::Mutex<Vec<PathBuf>>>,
    rescan: Arc<AtomicBool>,
}
impl Scope {
    /// Every folder to list, and those among them that are repositories of
    /// their own (whose status is read separately).
    fn folders(&self, root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut all = crate::project::nested_folders(root);
        let extra = self.extra.lock().unwrap().clone();
        let tree = self.tree.lock().unwrap().clone();
        for folder in extra.iter().chain(tree.iter()) {
            if !all.contains(folder) {
                all.push(folder.clone());
            }
        }
        let repos = all
            .iter()
            .filter(|f| crate::project::is_repository(f))
            .cloned()
            .collect();
        (all, repos)
    }
    fn set_extra(&self, folders: Vec<PathBuf>) {
        *self.extra.lock().unwrap() = folders;
        self.rescan.store(true, Ordering::Relaxed);
    }
    fn set_tree(&self, mut folders: Vec<PathBuf>) -> bool {
        folders.sort();
        folders.dedup();
        let mut current = self.tree.lock().unwrap();
        if *current != folders {
            *current = folders;
            self.rescan.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

fn start_watcher(
    root: PathBuf,
    sender: mpsc::Sender<Message>,
    stop: Arc<AtomicBool>,
    scope: Scope,
) {
    std::thread::spawn(move || {
        let history_root = root.clone();
        let history_sender = sender.clone();
        std::thread::spawn(move || {
            let _ = history_sender.send(Message::Tasks(crate::tasks::history(&history_root)));
        });
        let (event_tx, event_rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = event_tx.send(event);
        });
        let mut watch_errors = vec![];
        let watcher = match watcher {
            Ok(mut watcher) => match watcher.watch(&root, notify::RecursiveMode::Recursive) {
                Ok(()) => Some(watcher),
                Err(error) => {
                    watch_errors.push(format!(
                        "Live updates unavailable; refreshing periodically: {error}"
                    ));
                    None
                }
            },
            Err(error) => {
                watch_errors.push(format!(
                    "Live updates unavailable; refreshing periodically: {error}"
                ));
                None
            }
        };
        let watching = watcher.is_some();
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
        let mut revision = crate::project::revision();
        while !stop.load(Ordering::Relaxed) {
            if scope.rescan.swap(false, Ordering::Relaxed) || revision != crate::project::revision()
            {
                revision = crate::project::revision();
                refresh = true;
                last = Instant::now() - fallback;
            }
            for event in event_rx.try_iter() {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        refresh = true;
                        let warning =
                            format!("Live update failed; refreshing periodically: {error}");
                        log::warn!("{warning}");
                        if watch_errors.len() < 3 && !watch_errors.contains(&warning) {
                            watch_errors.push(warning);
                        }
                        continue;
                    }
                };
                for path in event.paths {
                    if path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(".vyber-save-"))
                    {
                        continue;
                    }
                    // Git administration refreshes the tree/status without
                    // opening metadata through Follow. Explicitly opened
                    // metadata documents use the independent document watcher.
                    if path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .components()
                        .any(|component| component.as_os_str() == ".git")
                    {
                        refresh = true;
                        continue;
                    }
                    paths.insert(path);
                    refresh = true;
                }
            }
            if refresh && last.elapsed() > Duration::from_millis(450) {
                for path in std::mem::take(&mut paths) {
                    if !path.is_dir() {
                        let _ = sender.send(Message::Changed(path));
                    }
                }
                let (folders, repos) = scope.folders(&root);
                let mut files = workspace::scan_files_with(&root, &folders);
                files.errors.extend(watch_errors.iter().cloned());
                let changes = workspace::status_with(&root, &repos).unwrap_or_default();
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

#[cfg(test)]
mod state_tests {
    use super::BrowserState;

    #[test]
    fn legacy_workspace_keeps_overlay_and_document_pin_separate() {
        let state: BrowserState = serde_json::from_str(
            r#"{"visible":true,"expanded":[],"docs":[],"active":null,"follow":true,"pinned":true}"#,
        )
        .unwrap();
        assert!(!state.docked);
        assert!(state.pinned);
        assert!(state.follow);
    }

    #[test]
    fn docked_workspace_preserves_mode_width_and_follow_setting() {
        let state = BrowserState {
            visible: true,
            docked: true,
            follow: true,
            panel_width: Some(0.61),
            ..Default::default()
        };
        let encoded = serde_json::to_string(&state).unwrap();
        let restored: BrowserState = serde_json::from_str(&encoded).unwrap();
        assert!(restored.visible && restored.docked && restored.follow);
        assert!(!restored.pinned);
        assert_eq!(restored.panel_width, state.panel_width);
    }
}
