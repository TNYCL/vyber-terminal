//! The Review tab: Git changes and agent tasks in the sidebar, the diff in
//! the main area.
use super::{Browser, ReviewMode, View, document::empty_state, tree::status_color};
use crate::{
    icons,
    theme::*,
    workspace::{self, Change, DiffLine, DiffScope},
};
use gpui::{prelude::*, *};

const ADDED_BG: u32 = 0x0f2418;
const ADDED_TEXT: u32 = 0xc3e8cd;
const DELETED_BG: u32 = 0x2a1316;
const DELETED_TEXT: u32 = 0xf2c3c7;
const CONTEXT_TEXT: u32 = 0xb4b4b4;
const GAP_BG: u32 = 0x0f141c;
const GAP_TEXT: u32 = 0x6f86a8;

impl Browser {
    pub(super) fn review_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.review_mode;
        let mut sidebar = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_l_1()
            .border_color(rgb(DIVIDER))
            .child(
                div().p_2().child(
                    segmented()
                        .child(
                            segment(
                                "mode-changes",
                                format!("Changes {}", self.changes.len()),
                                mode == ReviewMode::Changes,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review_mode = ReviewMode::Changes;
                                this.selected_task = None;
                                cx.notify();
                            })),
                        )
                        .child(
                            segment(
                                "mode-tasks",
                                format!("Tasks {}", self.tasks.len()),
                                mode == ReviewMode::Tasks,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review_mode = ReviewMode::Tasks;
                                cx.notify();
                            })),
                        ),
                ),
            );
        match mode {
            ReviewMode::Changes => {
                if !self.branch.is_empty() {
                    sidebar = sidebar.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_3()
                            .pb_2()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child(icon(ui("git-branch"), MUTED, 13.))
                            .child(div().truncate().child(self.branch.clone())),
                    );
                }
                sidebar = sidebar.child(
                    div().px_2().pb_2().child(
                        segmented().children(
                            [DiffScope::All, DiffScope::Unstaged, DiffScope::Staged].map(
                                |scope| {
                                    segment(
                                        scope.label(),
                                        match scope {
                                            DiffScope::All => "All",
                                            s => s.label(),
                                        },
                                        self.scope == scope,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.scope = scope;
                                        if let Some((path, _, _, _)) = &this.review {
                                            this.request_diff(path.clone(), cx);
                                        }
                                        cx.notify();
                                    }))
                                },
                            ),
                        ),
                    ),
                );
                let changes = self
                    .changes
                    .iter()
                    .filter(|c| match self.scope {
                        DiffScope::All => true,
                        DiffScope::Unstaged => {
                            c.status.as_bytes().get(1).is_some_and(|b| *b != b' ')
                        }
                        DiffScope::Staged => c
                            .status
                            .as_bytes()
                            .first()
                            .is_some_and(|b| *b != b' ' && *b != b'?'),
                    })
                    .cloned()
                    .collect();
                let empty = if self.loading {
                    "Reading Git status…"
                } else {
                    "No changes in this view"
                };
                sidebar = sidebar.child(self.change_list(changes, empty, cx));
            }
            ReviewMode::Tasks => {
                sidebar = sidebar.child(
                    div()
                        .flex()
                        .gap_1()
                        .px_2()
                        .pb_2()
                        .child(
                            text_button("checkpoint", Some("flag"), "Checkpoint")
                                .border_1()
                                .border_color(rgb(BORDER))
                                .on_click(cx.listener(|this, _, _, cx| this.checkpoint(cx))),
                        )
                        .when(self.checkpoint.is_some(), |s| {
                            s.child(
                                text_button("since", Some("git-compare"), "Review since")
                                    .border_1()
                                    .border_color(rgb(BORDER))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.review_checkpoint(cx)),
                                    ),
                            )
                        }),
                );
                let mut list = div()
                    .id("task-list")
                    .flex()
                    .flex_col()
                    .px_1()
                    .max_h(relative(0.45))
                    .overflow_y_scroll();
                for (index, task) in self.tasks.iter().enumerate().rev() {
                    let selected = self.selected_task == Some(index);
                    list = list.child(
                        div()
                            .id(("task", index))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(30.))
                            .px_2()
                            .rounded_md()
                            .text_size(px(12.5))
                            .cursor_pointer()
                            .when(selected, |s| s.bg(rgb(SELECTED)))
                            .when(!selected, |s| s.hover(|s| s.bg(rgb(HOVER))))
                            .child(if task.active {
                                icon(ui("circle-dot"), ADDED, 14.)
                            } else {
                                icon(ui("circle-check"), MUTED, 14.)
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(if selected { TEXT } else { 0xcccccc }))
                                    .child(task.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child(task.agent.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_task = Some(index);
                                this.review = None;
                                this.diff_request += 1;
                                cx.notify();
                            })),
                    );
                }
                sidebar = sidebar.child(list);
                if let Some(task) = self.selected_task.and_then(|i| self.tasks.get(i)) {
                    let changes = task.changes.clone();
                    let warning = task.warning.clone();
                    sidebar = sidebar
                        .child(div().h(px(1.)).mx_2().my_2().bg(rgb(DIVIDER)))
                        .when(!warning.is_empty(), |s| {
                            s.child(
                                div()
                                    .px_3()
                                    .pb_2()
                                    .text_size(px(11.))
                                    .text_color(rgb(WARNING))
                                    .child(warning),
                            )
                        })
                        .child(self.change_list(changes, "This task changed no files", cx))
                        .child(
                            div().p_2().child(
                                text_button("undo-task", Some("rotate-ccw"), "Restore task")
                                    .border_1()
                                    .border_color(rgb(BORDER))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.restore_task(cx)),
                                    ),
                            ),
                        );
                } else if self.tasks.is_empty() {
                    sidebar = sidebar.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(
                                "Tasks appear when Claude or Codex starts a request in this \
                                 workspace. Checkpoints work with any command.",
                            ),
                    );
                }
            }
        }
        sidebar.into_any_element()
    }

    fn change_list(
        &self,
        changes: Vec<Change>,
        empty: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let count = changes.len();
        let selected = self.review.as_ref().map(|r| r.0.clone());
        div()
            .flex_1()
            .min_h_0()
            .when(count == 0, |s| {
                s.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(MUTED))
                        .text_size(px(12.))
                        .child(empty),
                )
            })
            .child(
                uniform_list("change-list", count, move |range, _, _| {
                    range
                        .map(|i| {
                            let change = changes[i].clone();
                            let letter = change.letter();
                            let entity = entity.clone();
                            let active = selected.as_deref() == Some(change.path.as_str());
                            let (name, folder) = match change.path.rsplit_once('/') {
                                Some((folder, name)) => (name.to_string(), folder.to_string()),
                                None => (change.path.clone(), String::new()),
                            };
                            let (icon_path, color) =
                                icons::file_icon(std::path::Path::new(&change.path));
                            div()
                                .id(("change", i))
                                .h(px(28.))
                                .px_1()
                                .child(
                                    div()
                                        .id("row")
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .px_2()
                                        .rounded_md()
                                        .text_size(px(12.5))
                                        .cursor_pointer()
                                        .when(active, |s| s.bg(rgb(SELECTED)))
                                        .when(!active, |s| s.hover(|s| s.bg(rgb(HOVER))))
                                        .child(icon(icon_path, color, 15.))
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .max_w(relative(0.7))
                                                .truncate()
                                                .text_color(rgb(status_color(letter)))
                                                .when(letter == 'D', |s| s.line_through())
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.))
                                                .text_color(rgb(MUTED))
                                                .child(folder),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(status_color(letter)))
                                                .child(letter.to_string()),
                                        ),
                                )
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |view, cx| {
                                        view.view = View::Review;
                                        view.request_diff(change.path.clone(), cx)
                                    })
                                })
                        })
                        .collect()
                })
                .size_full(),
            )
            .into_any_element()
    }

    pub(super) fn review_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some((path, lines, before, after)) = &self.review else {
            let (title, detail) = if self.review_mode == ReviewMode::Tasks {
                ("Review a task", "Choose a task, then a file it changed")
            } else if self.changes.is_empty() && !self.loading {
                ("No changes", "The working tree matches this view")
            } else {
                ("Review", "Select a changed file to compare")
            };
            return empty_state("git-compare", title, detail, None);
        };
        // Added and deleted files have nothing to pair, so show them in one column.
        let one_sided = before.is_none() || after.is_none();
        let split = self.split_diff && !one_sided;
        let path = path.clone();
        let additions = lines.iter().filter(|l| l.kind == '+').count();
        let deletions = lines.iter().filter(|l| l.kind == '-').count();
        let rows = if split {
            workspace::paired_lines(lines)
        } else {
            lines.iter().map(|l| (None, Some(l.clone()))).collect()
        };
        let hunks = rows
            .iter()
            .enumerate()
            .filter(|(i, r)| {
                let changed = |r: &(Option<DiffLine>, Option<DiffLine>)| {
                    r.0.as_ref()
                        .or(r.1.as_ref())
                        .is_some_and(|l| l.kind == '+' || l.kind == '-')
                        || r.1.as_ref().is_some_and(|l| l.kind == '+')
                };
                changed(r) && (*i == 0 || !changed(&rows[i - 1]))
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let previous = hunks.clone();
        let next = hunks.clone();
        let hunk_count = hunks.len();
        let width = if split { 1400. } else { 900. };
        let wide = self.wide_diff;
        let (icon_path, color) = icons::file_icon(std::path::Path::new(&path));
        let open_path = self.root.join(&path);
        let (folder, name) = match path.rsplit_once('/') {
            Some((folder, name)) => (format!("{folder}/"), name.to_string()),
            None => (String::new(), path.clone()),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_3()
                    .pr_2()
                    .border_b_1()
                    .border_color(rgb(DIVIDER))
                    .text_size(px(12.5))
                    .child(icon(icon_path, color, 15.))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .truncate()
                            .child(div().text_color(rgb(MUTED)).child(folder))
                            .child(
                                div()
                                    .text_color(rgb(TEXT))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(ADDED))
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(DELETED))
                            .child(format!("−{deletions}")),
                    )
                    .child(div().flex_1())
                    .when(!one_sided, |s| s.child(
                        segmented()
                            .w(px(150.))
                            .child(segment("diff-split", "Split", split).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.split_diff = true;
                                    cx.notify();
                                },
                            )))
                            .child(segment("diff-unified", "Unified", !split).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.split_diff = false;
                                    cx.notify();
                                }),
                            )),
                    ))
                    .child(
                        icon_button(
                            "diff-context",
                            if self.full_diff { "fold-vertical" } else { "unfold-vertical" },
                            if self.full_diff {
                                "Collapse unchanged lines"
                            } else {
                                "Show all lines"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.full_diff = !this.full_diff;
                            if let Some((path, _, _, _)) = &this.review {
                                this.request_diff(path.clone(), cx);
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_button(
                            "diff-width",
                            "columns-2",
                            if wide { "Fit lines to panel" } else { "Wide lines" },
                        )
                        .when(wide, |s| s.bg(rgb(SELECTED)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.wide_diff = !this.wide_diff;
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_button(
                            "prev-hunk",
                            "arrow-up",
                            format!("Previous change · {hunk_count} total"),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !previous.is_empty() {
                                this.diff_hunk = match this.diff_hunk {
                                    0 | usize::MAX => previous.len() - 1,
                                    h => (h - 1).min(previous.len() - 1),
                                };
                                this.diff_scroll
                                    .scroll_to_item(previous[this.diff_hunk], ScrollStrategy::Top);
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        icon_button("next-hunk", "arrow-down", "Next change").on_click(
                            cx.listener(move |this, _, _, cx| {
                                if !next.is_empty() {
                                    this.diff_hunk = this.diff_hunk.wrapping_add(1) % next.len();
                                    this.diff_scroll
                                        .scroll_to_item(next[this.diff_hunk], ScrollStrategy::Top);
                                    cx.notify();
                                }
                            }),
                        ),
                    )
                    .child(
                        icon_button("diff-open", "file-text", "Open file").on_click(cx.listener(
                            move |this, _, _, cx| {
                                if open_path.is_file() {
                                    this.open(open_path.clone(), true, cx);
                                } else {
                                    this.say("This file no longer exists in the working tree.");
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        icon_button("restore", "rotate-ccw", "Restore this file")
                            .on_click(cx.listener(|this, _, _, cx| this.restore(cx))),
                    ),
            )
            .child(
                div()
                    .id("diff-horizontal")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(
                        uniform_list("diff-lines", rows.len(), move |range, _, _| {
                            range
                                .map(|i| {
                                    let (left, right) = &rows[i];
                                    div()
                                        .w_full()
                                        .h(px(21.))
                                        .flex()
                                        .font_family(mono_font())
                                        .text_size(px(12.))
                                        .when(split, |s| {
                                            s.child(diff_cell(left, true))
                                                .child(div().w(px(1.)).h_full().bg(rgb(DIVIDER)))
                                        })
                                        .child(diff_cell(right, false))
                                })
                                .collect()
                        })
                        .track_scroll(&self.diff_scroll)
                        .w_full()
                        .when(wide, |s| s.w(px(width)))
                        .h_full(),
                    ),
            )
            .into_any_element()
    }
}

fn diff_cell(line: &Option<DiffLine>, old: bool) -> Div {
    let kind = line.as_ref().map(|l| l.kind).unwrap_or(' ');
    let number = line
        .as_ref()
        .and_then(|l| if old { l.old } else { l.new.or(l.old) });
    let text = line.as_ref().map(|l| l.text.clone()).unwrap_or_default();
    let (bg, fg, marker) = match kind {
        '-' => (DELETED_BG, DELETED_TEXT, "−"),
        '+' => (ADDED_BG, ADDED_TEXT, "+"),
        '@' => (GAP_BG, GAP_TEXT, ""),
        _ if line.is_none() => (0x0d0d0d, CONTEXT_TEXT, ""),
        _ => (PANEL, CONTEXT_TEXT, ""),
    };
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .h_full()
        .items_center()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .child(
            div()
                .w(px(44.))
                .flex_shrink_0()
                .pr_2()
                .text_right()
                .text_size(px(11.))
                .text_color(rgb(if kind == ' ' { 0x4a4a4a } else { MUTED }))
                .child(number.map(|n| n.to_string()).unwrap_or_default()),
        )
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .text_color(rgb(if kind == '+' { ADDED } else { DELETED }))
                .child(marker),
        )
        .child(div().whitespace_nowrap().child(text))
}
