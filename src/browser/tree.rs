//! The workspace tree on the Files side: root picker, filter, tree rows,
//! fuzzy file matches and content-search results.
use super::{Browser, Menu, Message, View};
use crate::theme::observe;
use crate::{icons, theme::*, workspace};
use gpui::{prelude::*, *};
use gpui_kit::component::{Sizable, input::Input};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const ROW: f32 = 28.;
const INDENT: f32 = 12.;
const PAD: f32 = 10.;
const NAME: u32 = 0xcccccc;

/// Data for one visible row, cloned so the virtual list can own it.
#[derive(Clone)]
struct Row {
    path: PathBuf,
    relative: String,
    name: SharedString,
    depth: usize,
    directory: bool,
    expanded: bool,
    status: Option<char>,
    changed: bool,
    selected: bool,
    active: bool,
    /// Depth at which to draw the guide line of the active file's folder.
    guide: Option<usize>,
    /// File-name character positions matched by the filter.
    matched: Vec<usize>,
    /// Folder shown after the name in filter results.
    parent: Option<SharedString>,
    filtered: bool,
}

pub fn status_color(letter: char) -> u32 {
    match letter {
        'U' | 'A' | 'R' => ADDED,
        'D' => DELETED,
        '!' => CONFLICT,
        _ => MODIFIED,
    }
}

impl Browser {
    /// Keep lazy directory reads in sync with both expansion and a restored
    /// nested root. This is independent of Git/project repository discovery.
    pub(super) fn sync_tree_scope(&mut self) {
        let mut folders = self
            .expanded
            .iter()
            .map(|relative| self.root.join(relative))
            .collect::<Vec<_>>();
        if let Some(root) = &self.tree_root {
            folders.push(self.root.join(root));
        }
        if self.scope.set_tree(folders) {
            self.loading = true;
        }
    }

    /// Filename search has its own cancellable index: collapsed folders are
    /// still searched without making ordinary tree rendering recursive.
    pub(super) fn search_names(&mut self, cx: &mut Context<Self>) {
        self.name_cancel.store(true, Ordering::Relaxed);
        self.name_cancel = Arc::new(AtomicBool::new(false));
        self.name_generation = self.name_generation.wrapping_add(1);
        self.pending_name_open = None;
        let generation = self.name_generation;
        let query = self.filter_query(cx);
        self.name_query = query.clone();
        self.name_results.clear();
        self.name_errors.clear();
        self.name_truncated = false;
        self.name_loading = !query.is_empty();
        if query.is_empty() {
            cx.notify();
            return;
        }
        let root = self.root.clone();
        let folder = self
            .tree_root
            .as_ref()
            .map_or_else(|| root.clone(), |relative| root.join(relative));
        let sender = self.sender.clone();
        let cancelled = self.name_cancel.clone();
        std::thread::spawn(move || {
            let mut result = workspace::search_files(&folder, &query, &cancelled);
            for entry in &mut result.entries {
                entry.relative = entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .to_string_lossy()
                    .replace('\\', "/");
                entry.depth = entry.relative.matches('/').count();
            }
            if !cancelled.load(Ordering::Relaxed) {
                let _ = sender.send(Message::NameSearch(generation, query, result));
            }
        });
        cx.notify();
    }

    pub(super) fn name_search_loaded(
        &mut self,
        generation: u64,
        query: String,
        result: workspace::NameSearchResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation != self.name_generation || query != self.filter_query(cx) {
            return;
        }
        self.name_query = query;
        self.name_results = result.entries;
        self.name_errors = result.errors;
        self.name_truncated = result.truncated;
        self.name_loading = false;
        if self.pending_name_open.take() == Some(generation) && !self.open_best_match(window, cx) {
            self.search_contents(cx);
        }
        cx.notify();
    }

    pub(super) fn select_tree_root(
        &mut self,
        root: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tree_root = root;
        self.menu = None;
        self.tree_selected = None;
        self.quick_look = false;
        self.sidebar_hidden[0] = false;
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_cancel.store(true, Ordering::Relaxed);
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_query.clear();
        self.search_results.clear();
        self.search_names(cx);
        self.sync_tree_scope();
        self.tree_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }
    /// Tree root prefix (`dir/`) and its depth when a nested repository is shown.
    fn tree_prefix(&self) -> (String, usize) {
        match &self.tree_root {
            Some(root) => (format!("{root}/"), root.matches('/').count() + 1),
            None => (String::new(), 0),
        }
    }
    /// Visible tree entries as (index into `files`, display depth), in order.
    /// Relies on `files` being in depth-first tree order.
    fn visible_rows(&self) -> Vec<(usize, usize)> {
        let (prefix, base) = self.tree_prefix();
        let mut rows = Vec::new();
        let mut collapsed: Option<usize> = None;
        for (i, entry) in self.files.iter().enumerate() {
            if !entry.relative.starts_with(&prefix) {
                continue;
            }
            if let Some(depth) = collapsed {
                if entry.depth > depth {
                    continue;
                }
                collapsed = None;
            }
            rows.push((i, entry.depth - base));
            if entry.directory && !self.expanded.contains(&entry.relative) {
                collapsed = Some(entry.depth);
            }
        }
        rows
    }
    /// Files matching the filter, best first, with matched name positions.
    fn filtered(&self, query: &str) -> Vec<(usize, Vec<usize>)> {
        let (prefix, _) = self.tree_prefix();
        let mut matches = self
            .name_results
            .iter()
            .enumerate()
            .filter(|(_, e)| e.relative.starts_with(&prefix))
            .filter_map(|(i, e)| {
                workspace::fuzzy_score(query, &e.relative[prefix.len()..])
                    .map(|(score, positions)| (score, i, positions))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| {
                workspace::natural_cmp(
                    &self.name_results[a.1].relative,
                    &self.name_results[b.1].relative,
                )
            })
        });
        matches.into_iter().map(|(_, i, p)| (i, p)).collect()
    }
    pub(super) fn open_best_match(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let query = self.filter_query(cx);
        if query.is_empty() {
            return false;
        }
        if query != self.name_query {
            self.search_names(cx);
        }
        if self.name_loading {
            self.pending_name_open = Some(self.name_generation);
            return true;
        }
        match self.filtered(&query).first() {
            Some((index, _)) => {
                let entry = self.name_results[*index].clone();
                if entry.directory {
                    self.select_tree_root(Some(entry.relative), window, cx);
                } else {
                    self.open(entry.path, true, cx);
                }
                true
            }
            None => false,
        }
    }
    fn rows(&self, cx: &App) -> Vec<Row> {
        let query = self.filter_query(cx);
        let active = self
            .active_document()
            .map(|d| self.relative(&d.path))
            .unwrap_or_default();
        let (prefix, base) = self.tree_prefix();
        let active_parent = active
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .filter(|p| p.len() + 1 > prefix.len());
        let row = |entry: &workspace::FileEntry, depth: usize, matched: Vec<usize>, flat: bool| {
            let name = entry
                .relative
                .rsplit('/')
                .next()
                .unwrap_or(&entry.relative)
                .to_string();
            let guide = active_parent.as_ref().and_then(|parent| {
                (!flat && entry.relative.starts_with(&format!("{parent}/")))
                    .then(|| parent.matches('/').count() + 1 - base)
            });
            Row {
                path: entry.path.clone(),
                relative: entry.relative.clone(),
                name: name.into(),
                depth,
                directory: entry.directory,
                expanded: self.expanded.contains(&entry.relative),
                status: self.statuses.get(&entry.relative).copied(),
                changed: entry.directory && self.changed_dirs.contains(&entry.relative),
                selected: self.tree_selected.as_deref() == Some(entry.relative.as_str()),
                active: entry.relative == active,
                guide,
                matched,
                parent: flat.then(|| {
                    entry.relative[prefix.len()..]
                        .rsplit_once('/')
                        .map(|(p, _)| SharedString::from(p.to_string()))
                        .unwrap_or_default()
                }),
                filtered: flat,
            }
        };
        if query.is_empty() {
            self.visible_rows()
                .into_iter()
                .map(|(i, depth)| row(&self.files[i], depth, vec![], false))
                .collect()
        } else if self.name_query == query {
            self.filtered(&query)
                .into_iter()
                .map(|(i, matched)| row(&self.name_results[i], 0, matched, true))
                .collect()
        } else {
            vec![]
        }
    }
    /// Keyboard navigation while the tree has focus.
    pub(super) fn tree_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) {
            return;
        }
        let key = event.keystroke.key.as_str();
        // An open Git dialog or menu takes Escape (cancel) and Enter (its
        // main button) first.
        if let Some(confirm) = &self.git.confirm {
            match key {
                "escape" => self.dismiss_confirm(),
                "enter" => {
                    let main = confirm
                        .buttons
                        .iter()
                        .find(|b| b.style == super::scm::Style::Primary)
                        .map(|b| b.action.clone());
                    match main {
                        Some(Some(action)) => self.perform(action, window, cx),
                        Some(None) => self.dismiss_confirm(),
                        None => return,
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "escape" && self.git.menu.is_some() {
            self.git.menu = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "escape" {
            self.quick_look = false;
            self.menu = None;
            cx.notify();
            return;
        }
        if key == "space" {
            self.quick_look = !self.quick_look;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.view != View::Files || event.keystroke.modifiers.modified() {
            return;
        }
        let rows = self.rows(cx);
        if rows.is_empty() {
            return;
        }
        let current = rows.iter().position(|r| r.selected);
        let select = |this: &mut Self, index: usize| {
            this.tree_selected = Some(rows[index].relative.clone());
            this.tree_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        };
        match key {
            "down" => select(self, current.map_or(0, |i| (i + 1).min(rows.len() - 1))),
            "up" => select(self, current.map_or(0, |i| i.saturating_sub(1))),
            "home" => select(self, 0),
            "end" => select(self, rows.len() - 1),
            "right" => {
                let Some(i) = current else { return };
                let row = &rows[i];
                if row.directory && !row.expanded {
                    self.expanded.insert(row.relative.clone());
                } else if row.directory && i + 1 < rows.len() {
                    select(self, i + 1);
                }
            }
            "left" => {
                let Some(i) = current else { return };
                let row = &rows[i];
                if row.directory && row.expanded {
                    self.expanded.remove(&row.relative);
                } else if let Some((parent, _)) = row.relative.rsplit_once('/')
                    && let Some(p) = rows.iter().position(|r| r.relative == parent)
                {
                    select(self, p);
                }
            }
            "enter" => {
                let Some(i) = current else { return };
                let row = rows[i].clone();
                if row.directory {
                    if row.filtered {
                        self.select_tree_root(Some(row.relative), window, cx);
                    } else if !self.expanded.remove(&row.relative) {
                        self.expanded.insert(row.relative);
                    }
                } else {
                    self.open(row.path, true, cx);
                }
            }
            _ => return,
        }
        self.sync_tree_scope();
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn files_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let query = self.filter_query(cx);
        let filter_focused = self.filter.read(cx).focus_handle(cx).is_focused(window);
        let root_name: SharedString = match &self.tree_root {
            Some(root) => root.rsplit('/').next().unwrap_or(root).to_string().into(),
            None => self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.root.to_string_lossy().to_string())
                .into(),
        };
        let folders = self
            .files
            .iter()
            .filter(|f| f.directory)
            .map(|f| f.relative.clone())
            .collect::<Vec<_>>();
        let showing_search = !query.is_empty() && query == self.search_query;
        let body = if showing_search {
            self.search_list(cx)
        } else {
            self.tree_list(&query, cx)
        };
        div()
            .id("files-sidebar")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_l_1()
            .border_color(rgb(DIVIDER))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .px_2()
                    .pt_2()
                    .pb_1p5()
                    .child(
                        div()
                            .id("tree-root")
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(rpx(30.))
                            .px_2p5()
                            .rounded_lg()
                            .bg(rgb(SURFACE))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .text_size(rpx(12.5))
                            .text_color(rgb(TEXT))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_menu(Menu::Root);
                                // Worktrees come from the repositories' Git data.
                                if !this.git.discovered {
                                    this.load_repos();
                                }
                                cx.notify();
                            }))
                            .child(icon(ui("folder"), TEXT_2, 14.))
                            .child(div().flex_1().min_w_0().truncate().child(root_name))
                            .child(icon(ui("chevrons-up-down"), MUTED, 13.))
                            .map(observe),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .h(rpx(30.))
                            .flex_shrink_0()
                            .pl_2p5()
                            .pr_1()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(if filter_focused { 0x3a3a3a } else { BORDER }))
                            .child(icon(ui("search"), MUTED, 13.))
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.filter)
                                        .small()
                                        .appearance(false)
                                        .cleanable(true)
                                        .text_size(rpx(12.5)),
                                ),
                            ),
                    ),
            )
            .child(body)
            .when(self.menu == Some(Menu::Root), |s| {
                s.child(menu_in(
                    "root-menu-in",
                    44.,
                    self.root_menu(folders, cx).into_any_element(),
                ))
            })
            .map(observe)
            .into_any_element()
    }

    fn root_menu(&self, folders: Vec<String>, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace_name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.root.to_string_lossy().to_string());
        let current = self.tree_root.clone();
        let item = |id: usize, label: String, detail: Option<String>, value: Option<String>| {
            let checked = current == value;
            div()
                .id(("root-option", id))
                .flex()
                .items_center()
                .gap_2()
                .h(rpx(28.))
                .flex_shrink_0()
                .px_2()
                .rounded_md()
                .text_size(rpx(12.5))
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(HOVER)))
                .child(icon(ui("folder"), TEXT_2, 14.))
                .child(div().min_w_0().truncate().child(label))
                .when_some(detail, |s, d| {
                    s.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(11.))
                            .text_color(rgb(MUTED))
                            .child(d),
                    )
                })
                .child(div().flex_1())
                .when(checked, |s| s.child(icon(ui("check"), TEXT, 14.)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_tree_root(value.clone(), window, cx);
                }))
                .map(observe)
        };
        let mut items = vec![item(0, workspace_name, None, None).into_any_element()];
        if let Some(root) = &current {
            let parent = root.rsplit_once('/').map(|(parent, _)| parent.to_string());
            items.push(item(1, "Parent folder".into(), None, parent).into_any_element());
        } else if let Some(parent) = self.root.parent() {
            let parent = parent.to_path_buf();
            items.push(
                div()
                    .id("root-parent")
                    .h(rpx(40.))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_size(rpx(12.5))
                    .text_color(rgb(TEXT))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .child(icon(ui("folder"), TEXT_2, 14.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child("Parent folder")
                            .child(
                                div()
                                    .text_size(rpx(11.))
                                    .text_color(rgb(MUTED))
                                    .child("Opens in a new tab"),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        cx.emit(super::BrowserEvent::OpenFolder(parent.clone()));
                        cx.notify();
                    }))
                    .map(observe)
                    .into_any_element(),
            );
        }
        let (prefix, base) = self.tree_prefix();
        for (i, relative) in folders
            .into_iter()
            .filter(|relative| {
                relative.starts_with(&prefix) && relative.matches('/').count() == base
            })
            .enumerate()
        {
            let (parent, name) = match relative.rsplit_once('/') {
                Some((parent, name)) => (Some(parent.to_string()), name.to_string()),
                None => (None, relative.clone()),
            };
            items.push(item(i + 2, name, parent, Some(relative)).into_any_element());
        }
        // Worktrees and project folders outside this folder: those inside it
        // become the tree root, the others open in a new tab.
        let heading = |text: &'static str| {
            div()
                .px_2()
                .flex_shrink_0()
                .pt_2()
                .pb_1()
                .text_size(rpx(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(MUTED))
                .child(text)
                .into_any_element()
        };
        let link = |id: usize, name: &'static str, label: String, detail: String, path: PathBuf| {
            let scoped = workspace::relative_folder(&self.root, &path)
                .map(|r| r.to_string_lossy().replace('\\', "/"));
            let same_root = scoped.as_deref() == Some("");
            let relative = scoped.filter(|r| !r.is_empty());
            let checked =
                (same_root && current.is_none()) || (relative.is_some() && current == relative);
            div()
                .id(("root-link", id))
                .flex()
                .items_center()
                .gap_2()
                .h(rpx(28.))
                .flex_shrink_0()
                .px_2()
                .rounded_md()
                .text_size(rpx(12.5))
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(HOVER)))
                .child(icon(ui(name), TEXT_2, 14.))
                .child(div().min_w_0().truncate().child(label))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(11.))
                        .text_color(rgb(MUTED))
                        .child(if relative.is_some() || same_root {
                            detail
                        } else {
                            format!("{detail} · opens in a new tab")
                        }),
                )
                .child(div().flex_1())
                .when(checked, |s| s.child(icon(ui("check"), TEXT, 14.)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.menu = None;
                    if same_root {
                        this.select_tree_root(None, window, cx);
                        return;
                    }
                    match &relative {
                        Some(relative) => {
                            this.scope.set_extra(vec![path.clone()]);
                            this.select_tree_root(Some(relative.clone()), window, cx);
                        }
                        None => cx.emit(super::BrowserEvent::OpenFolder(path.clone())),
                    }
                    cx.notify();
                }))
                .map(observe)
                .into_any_element()
        };
        let worktrees: Vec<_> = self
            .git
            .repos
            .iter()
            .filter(|r| r.repo.is_worktree())
            .map(|r| {
                let owner = r
                    .repo
                    .worktree_of
                    .as_deref()
                    .map(crate::project::folder_name)
                    .unwrap_or_default();
                (
                    super::scm::repo_label(&r.repo, r.status()),
                    format!("worktree of {owner}"),
                    r.repo.path.clone(),
                )
            })
            .collect();
        if !worktrees.is_empty() {
            items.push(heading("WORKTREES"));
            for (i, (label, detail, path)) in worktrees.into_iter().enumerate() {
                items.push(link(i, "git-fork", label, detail, path));
            }
        }
        let outside: Vec<PathBuf> = crate::project::for_path(&self.root)
            .map(|p| {
                p.folders
                    .into_iter()
                    .filter(|f| !crate::tasks::matches_root(f, &self.root))
                    .collect()
            })
            .unwrap_or_default();
        if !outside.is_empty() {
            items.push(heading("PROJECT"));
            for (i, path) in outside.into_iter().enumerate() {
                let name = crate::project::folder_name(&path);
                items.push(link(
                    1000 + i,
                    "folder-git-2",
                    name,
                    "project folder".into(),
                    path,
                ));
            }
        }
        div()
            .id("root-menu")
            .absolute()
            .occlude()
            .top_0()
            .left_2()
            .right_2()
            .max_h(relative(1.))
            .overflow_y_scroll()
            .p_1()
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(rgb(0x121212))
            .border_1()
            .border_color(rgb(BORDER))
            .shadow_xl()
            .children(items)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_menu();
                cx.notify();
            }))
            .map(observe)
    }

    fn tree_list(&mut self, query: &str, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows(cx);
        if self.reveal && query.is_empty() {
            if let Some(i) = rows.iter().position(|r| r.selected) {
                self.tree_scroll.scroll_to_item(i, ScrollStrategy::Nearest);
                self.reveal = false;
            } else if !self.loading {
                self.reveal = false;
            }
        }
        let entity = cx.entity();
        let flat = !query.is_empty();
        let count = rows.len();
        let errors = if flat {
            &self.name_errors
        } else {
            &self.tree_errors
        };
        let placeholder = if flat && self.name_loading {
            Some("Searching file and folder names…".to_string())
        } else if !flat && self.loading {
            Some("Loading folders…".to_string())
        } else if count == 0 && !errors.is_empty() {
            None
        } else if count == 0 && flat {
            Some(format!("No file or folder names match “{query}”"))
        } else if count == 0 {
            Some("This folder is empty".to_string())
        } else {
            None
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(flat, |s| {
                s.child(
                    div()
                        .id("search-contents")
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(rpx(ROW))
                        .mx_1()
                        .px(rpx(PAD - 4.))
                        .rounded_md()
                        .text_size(rpx(12.5))
                        .text_color(rgb(TEXT_2))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                        .child(icon(ui("text-search"), TEXT_2, 14.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(format!("Search contents for “{query}”")),
                        )
                        .child(div().text_size(rpx(11.)).text_color(rgb(MUTED)).child("⇧↵"))
                        .on_click(cx.listener(|this, _, _, cx| this.search_contents(cx))),
                )
            })
            .when_some(placeholder, |s, text| {
                s.child(
                    div()
                        .px(rpx(PAD + 4.))
                        .py_2()
                        .text_size(rpx(12.))
                        .text_color(rgb(MUTED))
                        .child(text),
                )
            })
            .when(flat && self.name_truncated, |s| {
                s.child(
                    div()
                        .px(rpx(PAD + 4.))
                        .py_1()
                        .text_size(rpx(11.5))
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "Showing the first {} matches. Refine the filter to see more.",
                            workspace::NAME_SEARCH_LIMIT
                        )),
                )
            })
            .children(errors.iter().take(3).map(|error| {
                div()
                    .px(rpx(PAD + 4.))
                    .py_1()
                    .text_size(rpx(11.5))
                    .text_color(rgb(WARNING))
                    .child(error.clone())
            }))
            .when(errors.len() > 3, |s| {
                s.child(
                    div()
                        .px(rpx(PAD + 4.))
                        .py_1()
                        .text_size(rpx(11.5))
                        .text_color(rgb(WARNING))
                        .child(format!(
                            "{} more folders could not be read.",
                            errors.len() - 3
                        )),
                )
            })
            .child(
                uniform_list("file-tree", count, move |range, _, _| {
                    range
                        .map(|i| tree_row(i, rows[i].clone(), entity.clone()))
                        .collect()
                })
                .track_scroll(&self.tree_scroll)
                .flex_1()
                .pb_2(),
            )
            .into_any_element()
    }

    fn search_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let results = self.search_results.clone();
        let root = self.root.clone();
        let entity = cx.entity();
        let count = results.len();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(rpx(ROW))
                    .px(rpx(PAD))
                    .text_size(rpx(11.5))
                    .text_color(rgb(MUTED))
                    .child(icon(ui("text-search"), MUTED, 13.))
                    .child(
                        div()
                            .flex_1()
                            .child(format!("{count} matches in file contents")),
                    )
                    .child(icon_button("clear-search", "x", "Back to files").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.search_query.clear();
                            this.search_results.clear();
                            cx.notify();
                        }),
                    )),
            )
            .child(
                uniform_list("text-matches", count, move |range, _, _| {
                    range
                        .map(|i| {
                            let (path, line, text) = results[i].clone();
                            let (icon_path, color) = icons::file_icon(&path);
                            let relative = path
                                .strip_prefix(&root)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .replace('\\', "/");
                            let view = entity.clone();
                            div()
                                .id(("result", i))
                                .h(rpx(48.))
                                .mx_1()
                                .px(rpx(PAD - 4.))
                                .flex()
                                .flex_col()
                                .justify_center()
                                .gap_0p5()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(HOVER)))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .text_size(rpx(12.))
                                        .child(icon(icon_path, color, 14.))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(NAME))
                                                .child(relative),
                                        )
                                        .child(
                                            div().text_color(rgb(MUTED)).child(format!(":{line}")),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl(rpx(20.))
                                        .truncate()
                                        .font_family(mono_font())
                                        .text_size(rpx(11.))
                                        .text_color(rgb(TEXT_2))
                                        .child(text),
                                )
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |v, cx| {
                                        v.open_at(path.clone(), line, window, cx)
                                    })
                                })
                        })
                        .collect()
                })
                .flex_1()
                .pb_2(),
            )
            .into_any_element()
    }
}

fn tree_row(i: usize, row: Row, entity: Entity<Browser>) -> AnyElement {
    let name_color = match row.status {
        Some(letter) => status_color(letter),
        None if row.active || row.selected => TEXT,
        None => NAME,
    };
    let left = PAD + row.depth as f32 * INDENT;
    let mut name = StyledText::new(row.name.clone());
    if !row.matched.is_empty() {
        let highlights = row
            .name
            .char_indices()
            .enumerate()
            .filter(|(n, _)| row.matched.contains(n))
            .map(|(_, (byte, c))| {
                (
                    byte..byte + c.len_utf8(),
                    HighlightStyle {
                        color: Some(rgb(TEXT).into()),
                        font_weight: Some(FontWeight::SEMIBOLD),
                        ..Default::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        name = name.with_highlights(highlights);
    }
    let lead = if row.directory {
        icon(
            ui(if row.expanded {
                "chevron-down"
            } else {
                "chevron-right"
            }),
            MUTED,
            14.,
        )
    } else {
        let (path, color) = icons::file_icon(&row.path);
        icon(path, color, 15.)
    };
    div()
        .id(("file", i))
        .relative()
        .h(rpx(ROW))
        .px_1()
        .child(
            div()
                .id("row")
                .size_full()
                .flex()
                .items_center()
                .gap(rpx(6.))
                .pl(rpx(left - 4.))
                .pr_2()
                .rounded_md()
                .text_size(rpx(12.5))
                .text_color(rgb(name_color))
                .cursor_pointer()
                .when(row.active, |s| s.bg(rgb(SELECTED)))
                .when(row.selected && !row.active, |s| s.bg(rgb(HOVER)))
                .when(!row.active, |s| s.hover(|s| s.bg(rgb(HOVER))))
                .child(div().w(rpx(16.)).flex().justify_center().child(lead))
                .child(div().min_w_0().truncate().child(name))
                .when_some(row.parent.clone(), |s, parent| {
                    s.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(11.))
                            .text_color(rgb(MUTED))
                            .child(parent),
                    )
                })
                .child(div().flex_1())
                .when_some(row.status, |s, letter| {
                    s.child(
                        div()
                            .text_size(rpx(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(status_color(letter)))
                            .child(letter.to_string()),
                    )
                })
                .when(row.changed, |s| {
                    s.child(
                        div()
                            .size(rpx(5.))
                            .rounded_full()
                            .bg(rgb(MODIFIED))
                            .opacity(0.7),
                    )
                }),
        )
        .when_some(row.guide, |s, depth| {
            s.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(rpx(PAD + depth.saturating_sub(1) as f32 * INDENT + 7.5))
                    .w(px(1.))
                    .bg(rgb(FAINT)),
            )
        })
        .on_click(move |event, window, cx| {
            entity.update(cx, |view, cx| {
                window.focus(&view.focus, cx);
                view.tree_selected = Some(row.relative.clone());
                if row.directory {
                    if row.filtered {
                        view.select_tree_root(Some(row.relative.clone()), window, cx);
                    } else if !view.expanded.remove(&row.relative) {
                        view.expanded.insert(row.relative.clone());
                    }
                    view.sync_tree_scope();
                    cx.notify();
                } else {
                    view.open(row.path.clone(), event.click_count() > 1, cx);
                }
            });
        })
        .map(observe)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{browser::BrowserState, config::Config};
    use core::prelude::v1::test;
    use gpui::{
        AnyWindowHandle, AppContext, Bounds, Entity, Point, ScrollDelta, TestAppContext,
        WindowBounds, WindowHandle, WindowOptions,
    };
    use gpui_kit::test::TestWindowExt;

    fn window(root: PathBuf, cx: &mut TestAppContext) -> WindowHandle<Browser> {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Config::default());
            crate::theme::apply(cx);
        });
        cx.add_window(move |window, cx| Browser::new(root, window, cx))
    }

    fn short_window(root: PathBuf, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Browser>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Config::default());
            crate::theme::apply(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(480.), px(180.)),
                    })),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    let listing = workspace::scan_files_with(&root, &[]);
                    let browser = cx.new(|cx| Browser::new(root, window, cx));
                    browser.update(cx, |browser, cx| {
                        browser.visible = true;
                        browser.files = listing.entries;
                        browser.tree_errors = listing.errors;
                        browser.loading = false;
                        cx.notify();
                    });
                    browser
                },
            )
            .unwrap()
        })
    }

    #[gpui_kit::test]
    fn ordinary_root_picker_clicks_and_scrolls_without_shrinking_in_a_short_panel(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        std::fs::create_dir(root.join(".hidden")).unwrap();
        for i in 0..24 {
            std::fs::create_dir(root.join(format!("folder{i:02}"))).unwrap();
        }
        std::fs::write(root.join(".gitignore"), ".hidden/\nfolder*/\n").unwrap();
        let (handle, browser) = short_window(root.clone(), cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(browser.read(cx).git.repos.is_empty());
            window.click("tree-root", cx);
            assert!(browser.read(cx).menu == Some(Menu::Root));
            let sidebar = window.find("files-sidebar").bounds();
            let menu = window.find("root-menu").bounds();
            let trigger = window.find("tree-root").bounds();
            assert!(menu.top() >= trigger.bottom());
            assert!(menu.bottom() <= sidebar.bottom());
            assert!(menu.left() >= sidebar.left() && menu.right() <= sidebar.right());
            let scale = f32::from(window.rem_size()) / crate::theme::REM;
            let first = window.find(("root-option", 0usize));
            assert!(first.visible());
            assert!(first.bounds().size.height >= px(27. * scale));
            window.scroll(
                "root-menu",
                ScrollDelta::Pixels(point(px(0.), px(-10000.))),
                cx,
            );
            let last = window.find(("root-option", 26usize));
            assert!(last.visible());
            assert!(last.bounds().size.height >= px(27. * scale));
            assert!(last.bounds().bottom() <= menu.bottom() + px(1.));
            window.click(("root-option", 26usize), cx);
            assert!(browser.read(cx).menu.is_none());
            assert_eq!(browser.read(cx).tree_root.as_deref(), Some("folder23"));
            assert_eq!(browser.read(cx).root, root);
            assert!(
                browser
                    .read(cx)
                    .scope
                    .tree
                    .lock()
                    .unwrap()
                    .contains(&root.join("folder23"))
            );
            window.click("tree-root", cx);
            window.click(("root-option", 0usize), cx);
            assert!(browser.read(cx).tree_root.is_none());
            window.click("tree-root", cx);
            window.scroll(
                "root-menu",
                ScrollDelta::Pixels(point(px(0.), px(10000.))),
                cx,
            );
            let hidden = window.find(("root-option", 2usize));
            assert!(hidden.visible());
            window.click(("root-option", 2usize), cx);
            assert_eq!(browser.read(cx).tree_root.as_deref(), Some(".hidden"));
            assert!(!browser.read(cx).sidebar_hidden[0]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn document_reveal_waits_for_lazy_index_then_scrolls_the_selected_file_into_view(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        for i in 0..24 {
            std::fs::create_dir(root.join(format!("a{i:02}"))).unwrap();
        }
        std::fs::create_dir(root.join("z-folder")).unwrap();
        let file = root.join("z-folder/leaf.txt");
        std::fs::write(&file, "leaf").unwrap();
        let (handle, browser) = short_window(root.clone(), cx);
        cx.update_window(handle, |_, window, cx| {
            browser.update(cx, |browser, cx| {
                // Deliver the lazy Index deterministically while keeping the
                // real render, message application and scroll behavior.
                browser.stop.store(true, Ordering::Relaxed);
                let (sender, receiver) = std::sync::mpsc::channel();
                browser.sender = sender;
                browser.receiver = receiver;
                browser.reveal_document(&file, cx);
            });
            assert!(browser.read(cx).loading);
            window.render_frame(cx);
            assert!(browser.read(cx).reveal);
            assert_eq!(
                browser
                    .read(cx)
                    .tree_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .y,
                px(0.)
            );
            browser.update(cx, |browser, cx| {
                let folders = browser.scope.tree.lock().unwrap().clone();
                browser
                    .sender
                    .send(Message::Index(
                        workspace::scan_files_with(&root, &folders),
                        vec![],
                        String::new(),
                    ))
                    .unwrap();
                browser.poll(window, cx);
            });
            assert!(!browser.read(cx).loading);
            let index = {
                let browser = browser.read(cx);
                browser
                    .visible_rows()
                    .iter()
                    .position(|(index, _)| browser.files[*index].path == file)
                    .unwrap()
            };
            window.render_frame(cx);
            assert!(!browser.read(cx).reveal);
            let sidebar = window.find("files-sidebar").bounds();
            let scroll = browser.read(cx).tree_scroll.0.borrow();
            assert!(scroll.last_item_size.unwrap().item.height <= sidebar.size.height);
            assert!(
                scroll.base_handle.offset().y < px(0.),
                "selected index {index}, viewport {:?}, offset {:?}",
                scroll.last_item_size,
                scroll.base_handle.offset()
            );
            drop(scroll);
            let selected = window.find(("file", index));
            assert!(selected.visible());
            assert!(selected.bounds().bottom() <= window.find("files-sidebar").bounds().bottom());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn directory_filter_result_opens_visible_tree_without_changing_terminal_root(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        let handle = window(dir.path().to_path_buf(), cx);
        handle
            .update(cx, |browser, window, cx| {
                browser
                    .filter
                    .update(cx, |input, cx| input.set_value("target", window, cx));
                browser.name_query = "target".into();
                browser.name_results = workspace::scan_files_with(dir.path(), &[]).entries;
                browser.name_loading = false;
                browser.sidebar_hidden[0] = true;
                browser.quick_look = true;
                let old_search = browser.search_cancel.clone();
                assert!(browser.open_best_match(window, cx));
                assert_eq!(browser.root, dir.path());
                assert_eq!(browser.tree_root.as_deref(), Some("target"));
                assert!(browser.filter_query(cx).is_empty());
                assert!(!browser.sidebar_hidden[0]);
                assert!(!browser.quick_look);
                assert!(old_search.load(Ordering::Relaxed));
                assert!(
                    browser
                        .scope
                        .tree
                        .lock()
                        .unwrap()
                        .contains(&dir.path().join("target"))
                );
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn enter_waits_for_current_name_search_and_ignores_older_results(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        let handle = window(dir.path().to_path_buf(), cx);
        handle
            .update(cx, |browser, window, cx| {
                browser
                    .filter
                    .update(cx, |input, cx| input.set_value("target", window, cx));
                browser.name_query = "target".into();
                browser.name_generation = 7;
                browser.name_loading = true;
                assert!(browser.open_best_match(window, cx));
                assert_eq!(browser.pending_name_open, Some(7));
                let entries = workspace::scan_files_with(dir.path(), &[]).entries;
                browser.name_search_loaded(
                    6,
                    "target".into(),
                    workspace::NameSearchResult {
                        entries: entries.clone(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert!(browser.name_loading);
                assert!(browser.name_results.is_empty());
                assert_eq!(browser.pending_name_open, Some(7));
                browser.name_search_loaded(
                    7,
                    "target".into(),
                    workspace::NameSearchResult {
                        entries,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert_eq!(browser.tree_root.as_deref(), Some("target"));
                assert_eq!(browser.pending_name_open, None);
                assert!(browser.filter_query(cx).is_empty());
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn restored_ignored_root_rebuilds_lazy_directory_scope(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".worktree/feature/src")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), ".worktree/\n").unwrap();
        let handle = window(dir.path().to_path_buf(), cx);
        handle
            .update(cx, |browser, _, cx| {
                browser.restore_state(
                    BrowserState {
                        tree_root: Some(".worktree/feature".into()),
                        ..Default::default()
                    },
                    cx,
                );
                let folders = browser.scope.tree.lock().unwrap().clone();
                assert!(folders.contains(&dir.path().join(".worktree/feature")));
                browser.files = workspace::scan_files_with(dir.path(), &folders).entries;
                let rows = browser.visible_rows();
                assert_eq!(rows.len(), 1);
                assert_eq!(browser.files[rows[0].0].relative, ".worktree/feature/src");
            })
            .unwrap();
    }
}
