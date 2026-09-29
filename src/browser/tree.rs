//! The workspace tree on the Files side: root picker, filter, tree rows,
//! fuzzy file matches and content-search results.
use super::{Browser, Menu, View};
use crate::{icons, theme::*, workspace};
use gpui::{prelude::*, *};
use gpui_kit::component::{Sizable, input::Input};
use std::path::PathBuf;

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
            .files
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.directory && e.relative.starts_with(&prefix))
            .filter_map(|(i, e)| {
                workspace::fuzzy_score(query, &e.relative[prefix.len()..])
                    .map(|(score, positions)| (score, i, positions))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| {
                workspace::natural_cmp(&self.files[a.1].relative, &self.files[b.1].relative)
            })
        });
        matches.truncate(300);
        matches.into_iter().map(|(_, i, p)| (i, p)).collect()
    }
    pub(super) fn open_best_match(&mut self, cx: &mut Context<Self>) -> bool {
        let query = self.filter_query(cx);
        if query.is_empty() {
            return false;
        }
        match self.filtered(&query).first() {
            Some((index, _)) => {
                let path = self.files[*index].path.clone();
                self.open(path, true, cx);
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
        let row = |index: usize, depth: usize, matched: Vec<usize>, flat: bool| {
            let entry = &self.files[index];
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
            }
        };
        if query.is_empty() {
            self.visible_rows()
                .into_iter()
                .map(|(i, depth)| row(i, depth, vec![], false))
                .collect()
        } else {
            self.filtered(&query)
                .into_iter()
                .map(|(i, matched)| row(i, 0, matched, true))
                .collect()
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
                } else if let Some((parent, _)) = row.relative.rsplit_once('/') {
                    if let Some(p) = rows.iter().position(|r| r.relative == parent) {
                        select(self, p);
                    }
                }
            }
            "enter" => {
                let Some(i) = current else { return };
                let row = rows[i].clone();
                if row.directory {
                    if !self.expanded.remove(&row.relative) {
                        self.expanded.insert(row.relative);
                    }
                } else {
                    self.open(row.path, true, cx);
                }
            }
            _ => return,
        }
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
        let repositories = self
            .files
            .iter()
            .filter(|f| f.repository)
            .map(|f| f.relative.clone())
            .collect::<Vec<_>>();
        let can_switch = !repositories.is_empty();
        let showing_search = !query.is_empty() && query == self.search_query;
        let body = if showing_search {
            self.search_list(cx)
        } else {
            self.tree_list(&query, cx)
        };
        div()
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
                            .h(px(30.))
                            .px_2p5()
                            .rounded_lg()
                            .bg(rgb(SURFACE))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .text_size(px(12.5))
                            .text_color(rgb(TEXT))
                            .when(can_switch, |s| {
                                s.cursor_pointer()
                                    .hover(|s| s.bg(rgb(HOVER)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_menu(Menu::Root);
                                        cx.notify();
                                    }))
                            })
                            .child(icon(ui("folder"), TEXT_2, 14.))
                            .child(div().flex_1().min_w_0().truncate().child(root_name))
                            .when(can_switch, |s| {
                                s.child(icon(ui("chevrons-up-down"), MUTED, 13.))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .h(px(30.))
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
                                        .text_size(px(12.5)),
                                ),
                            ),
                    ),
            )
            .child(body)
            .when(self.menu == Some(Menu::Root), |s| {
                s.child(self.root_menu(repositories, cx))
            })
            .into_any_element()
    }

    fn root_menu(&self, repositories: Vec<String>, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace_name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let current = self.tree_root.clone();
        let item = |id: usize, label: String, detail: Option<String>, value: Option<String>| {
            let checked = current == value;
            div()
                .id(("root-option", id))
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.))
                .px_2()
                .rounded_md()
                .text_size(px(12.5))
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(HOVER)))
                .child(icon(
                    ui(if value.is_some() { "folder-git-2" } else { "folder" }),
                    TEXT_2,
                    14.,
                ))
                .child(div().min_w_0().truncate().child(label))
                .when_some(detail, |s, d| {
                    s.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(d),
                    )
                })
                .child(div().flex_1())
                .when(checked, |s| s.child(icon(ui("check"), TEXT, 14.)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tree_root = value.clone();
                    this.menu = None;
                    if let Some(root) = &value {
                        this.expanded.insert(root.clone());
                    }
                    this.tree_scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }))
        };
        let mut items = vec![item(0, workspace_name, None, None)];
        for (i, relative) in repositories.into_iter().enumerate() {
            let (parent, name) = match relative.rsplit_once('/') {
                Some((parent, name)) => (Some(parent.to_string()), name.to_string()),
                None => (None, relative.clone()),
            };
            items.push(item(i + 1, name, parent, Some(relative)));
        }
        div()
            .id("root-menu")
            .absolute()
            .occlude()
            .top(px(44.))
            .left_2()
            .right_2()
            .max_h(px(320.))
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
    }

    fn tree_list(&mut self, query: &str, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows(cx);
        if self.reveal && query.is_empty() {
            if let Some(i) = rows.iter().position(|r| r.selected) {
                self.tree_scroll
                    .scroll_to_item(i, ScrollStrategy::Nearest);
            }
            self.reveal = false;
        }
        let entity = cx.entity();
        let flat = !query.is_empty();
        let count = rows.len();
        let placeholder = if self.loading {
            Some("Indexing files…".to_string())
        } else if count == 0 && flat {
            Some(format!("No files match “{query}”"))
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
                        .h(px(ROW))
                        .mx_1()
                        .px(px(PAD - 4.))
                        .rounded_md()
                        .text_size(px(12.5))
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
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child("⇧↵"),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.search_contents(cx))),
                )
            })
            .when_some(placeholder, |s, text| {
                s.child(
                    div()
                        .px(px(PAD + 4.))
                        .py_2()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child(text),
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
                    .h(px(ROW))
                    .px(px(PAD))
                    .text_size(px(11.5))
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
                                .h(px(48.))
                                .mx_1()
                                .px(px(PAD - 4.))
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
                                        .text_size(px(12.))
                                        .child(icon(icon_path, color, 14.))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(NAME))
                                                .child(relative),
                                        )
                                        .child(
                                            div()
                                                .text_color(rgb(MUTED))
                                                .child(format!(":{line}")),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl(px(20.))
                                        .truncate()
                                        .font_family(mono_font())
                                        .text_size(px(11.))
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
        .h(px(ROW))
        .px_1()
        .child(
            div()
                .id("row")
                .size_full()
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(left - 4.))
                .pr_2()
                .rounded_md()
                .text_size(px(12.5))
                .text_color(rgb(name_color))
                .cursor_pointer()
                .when(row.active, |s| s.bg(rgb(SELECTED)))
                .when(row.selected && !row.active, |s| s.bg(rgb(HOVER)))
                .when(!row.active, |s| s.hover(|s| s.bg(rgb(HOVER))))
                .child(div().w(px(16.)).flex().justify_center().child(lead))
                .child(div().min_w_0().truncate().child(name))
                .when_some(row.parent.clone(), |s, parent| {
                    s.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(parent),
                    )
                })
                .child(div().flex_1())
                .when_some(row.status, |s, letter| {
                    s.child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(status_color(letter)))
                            .child(letter.to_string()),
                    )
                })
                .when(row.changed, |s| {
                    s.child(div().size(px(5.)).rounded_full().bg(rgb(MODIFIED)).opacity(0.7))
                }),
        )
        .when_some(row.guide, |s, depth| {
            s.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(PAD + depth.saturating_sub(1) as f32 * INDENT + 7.5))
                    .w(px(1.))
                    .bg(rgb(FAINT)),
            )
        })
        .on_click(move |event, window, cx| {
            entity.update(cx, |view, cx| {
                window.focus(&view.focus, cx);
                view.tree_selected = Some(row.relative.clone());
                if row.directory {
                    if !view.expanded.remove(&row.relative) {
                        view.expanded.insert(row.relative.clone());
                    }
                    cx.notify();
                } else {
                    view.open(row.path.clone(), event.click_count() > 1, cx);
                }
            });
        })
        .into_any_element()
}
