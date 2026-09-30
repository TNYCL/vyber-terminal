//! Drawing the Git view: the top strip, the source control sidebar (commit
//! box, changes and graph), the quick pick, dialogs, menus, the commit card
//! and the Git output log.
use super::{
    Browser, View,
    review::{menu_item, menu_panel, separator},
    scm::{
        Ask, CheckoutKind, CheckoutMode, Detail, FADE_TIME, Fold, GitAction, GitMenu, Group,
        MenuKind, RepoView, Style, repo_label, GROUP_LIMIT,
    },
    tree::status_color,
};
use crate::{
    git::{self, Entry, Label, Operation, Status, graph},
    icons, project,
    theme::*,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Sizable,
    input::{Input, Textarea},
    tooltip::Tooltip,
};
use std::{path::PathBuf, rc::Rc, time::Duration};

const ROW: f32 = 24.;
const GRAPH_ROW: f32 = 26.;
const LANE: f32 = 11.;
const PRIMARY_BG: u32 = 0xe6e6e6;
const PRIMARY_TEXT: u32 = 0x111111;

/// Dragging the line between the changes list and the graph.
#[derive(Clone)]
pub(super) struct GitSplit;
impl Render for GitSplit {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn spinner(size: f32, color: u32) -> impl IntoElement {
    icon(ui("loader-circle"), color, size).with_animation(
        "git-spinner",
        Animation::new(Duration::from_millis(900)).repeat(),
        |svg, t| svg.with_transformation(Transformation::rotate(percentage(t))),
    )
}

/// A chevron that turns from right (0) to down (1).
fn chevron(open: f32, color: u32, size: f32) -> Svg {
    icon(ui("chevron-right"), color, size)
        .with_transformation(Transformation::rotate(radians(std::f32::consts::FRAC_PI_2 * open)))
}

/// Shows `body` at `open` of its height (`full`), fading with it.
fn folding(body: Div, open: f32, full: f32) -> Div {
    if open >= 0.999 {
        body
    } else {
        div()
            .h(rpx(full * open))
            .overflow_hidden()
            .opacity(open)
            .child(body)
    }
}

/// Fades an overlay in and lets it settle a few pixels downwards.
fn appear<E: IntoElement + 'static>(id: impl Into<ElementId>, element: E) -> impl IntoElement {
    div().child(element).with_animation(
        id,
        Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
        |el, t| el.opacity(t).mt(rpx(-6. * (1. - t))),
    )
}

fn section_title(text: &'static str) -> Div {
    div()
        .text_size(rpx(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(text)
}

fn count_badge(count: usize) -> Div {
    div()
        .flex_shrink_0()
        .min_w(rpx(18.))
        .h(rpx(16.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .bg(rgb(SELECTED))
        .text_size(rpx(10.5))
        .text_color(rgb(TEXT_2))
        .child(count.to_string())
}

/// A small icon button that is always shown.
fn bar_action(id: impl Into<ElementId>, name: &str, tooltip: impl Into<SharedString>) -> Stateful<Div> {
    let tooltip = tooltip.into();
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(rpx(22.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x2a2a2a)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(icon(ui(name), TEXT_2, 14.))
}

/// A small icon button that shows on hover of its row group.
fn row_action(id: impl Into<ElementId>, name: &str, tooltip: &'static str, group: &SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .flex_shrink_0()
        .rounded(rpx(4.))
        .cursor_pointer()
        .opacity(0.)
        .group_hover(group.clone(), |s| s.opacity(1.))
        .hover(|s| s.bg(rgb(SELECTED)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(icon(ui(name), TEXT_2, 14.))
}

fn strip_pill(id: &'static str, name: &str, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1p5()
        .h(rpx(28.))
        .max_w(rpx(220.))
        .pl_2p5()
        .pr_2()
        .rounded_full()
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(BORDER))
        .text_size(rpx(12.5))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(SELECTED)))
        .child(icon(ui(name), TEXT_2, 14.))
        .child(div().min_w_0().truncate().child(label))
        .child(icon(ui("chevron-down"), MUTED, 13.))
}

fn dialog_button(id: impl Into<ElementId>, label: String, style: Style) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(rpx(30.))
        .px_3()
        .rounded_md()
        .text_size(rpx(12.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(label);
    match style {
        Style::Primary => base
            .bg(rgb(PRIMARY_BG))
            .text_color(rgb(PRIMARY_TEXT))
            .hover(|s| s.bg(rgb(0xffffff))),
        Style::Danger => base
            .bg(rgb(0x2a1316))
            .border_1()
            .border_color(rgb(0x5a2328))
            .text_color(rgb(0xff8a8a))
            .hover(|s| s.bg(rgb(0x3a1a1e))),
        Style::Plain => base
            .border_1()
            .border_color(rgb(0x333333))
            .text_color(rgb(TEXT))
            .hover(|s| s.bg(rgb(HOVER))),
    }
}

/// A ref name drawn as a small pill in the graph and on the commit card.
fn label_pill(label: &Label) -> Option<Div> {
    let (icon_name, text, color) = match label {
        Label::Head(name) => ("git-branch", name.clone(), graph::COLORS[0]),
        Label::Detached => ("git-commit-horizontal", "HEAD".to_string(), graph::COLORS[0]),
        Label::Local(name) => ("git-branch", name.clone(), 0xa3a3a3),
        Label::Remote(name) => ("cloud", name.clone(), 0xb180d7),
        Label::Tag(name) => ("tag", name.clone(), 0xd8b44a),
    };
    Some(
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .h(rpx(18.))
            .max_w(rpx(160.))
            .px_1p5()
            .rounded(rpx(9.))
            .border_1()
            .border_color(rgba((color << 8) | 0x66))
            .bg(rgba((color << 8) | 0x1a))
            .text_size(rpx(10.5))
            .text_color(rgb(color))
            .child(icon(ui(icon_name), color, 11.))
            .child(div().min_w_0().truncate().child(text)),
    )
}

/// Draws the lines and the dot of one graph row.
fn graph_cell(row: &graph::Row, head: bool, width: f32) -> impl IntoElement {
    let row = row.clone();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            // Painting works in pixels; the panel's text size scales the graph.
            let scale = f32::from(window.rem_size()) / REM;
            let lane_x = |lane: usize| bounds.left() + px((12. + lane as f32 * LANE) * scale);
            let top = bounds.top();
            let bottom = bounds.bottom();
            let middle = top + px(GRAPH_ROW / 2. * scale);
            let node = row.lane;
            for edge in &row.edges {
                let (from, to) = match edge.kind {
                    graph::EdgeKind::Through => (point(lane_x(edge.from), top), point(lane_x(edge.to), bottom)),
                    graph::EdgeKind::Into => (point(lane_x(edge.from), top), point(lane_x(node), middle)),
                    graph::EdgeKind::OutOf => (point(lane_x(node), middle), point(lane_x(edge.to), bottom)),
                };
                let mut path = PathBuilder::stroke(px(1.5 * scale));
                path.move_to(from);
                if from.x == to.x {
                    path.line_to(to);
                } else {
                    let bend = (from.y + to.y) / 2.;
                    path.cubic_bezier_to(to, point(from.x, bend), point(to.x, bend));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(graph::COLORS[edge.color % graph::COLORS.len()]));
                }
            }
            let color = rgb(graph::COLORS[row.color % graph::COLORS.len()]);
            let center = point(lane_x(node), middle);
            let radius = if head { 4.5 } else { 3.5 } * scale;
            let dot = |r: f32| Bounds {
                origin: point(center.x - px(r), center.y - px(r)),
                size: size(px(r * 2.), px(r * 2.)),
            };
            window.paint_quad(fill(dot(radius), color).corner_radii(px(radius)));
            if head || row.merge {
                // A ring: merges and HEAD read differently from plain commits.
                window.paint_quad(fill(dot(radius - 1.75 * scale), rgb(PANEL)).corner_radii(px(radius)));
            }
        },
    )
    .w(rpx(width))
    .h(rpx(GRAPH_ROW))
    .flex_shrink_0()
}

enum MenuEntry {
    Item {
        label: String,
        detail: Option<String>,
        action: Option<GitAction>,
    },
    Sub(String, Vec<MenuEntry>),
    Separator,
}

fn item(label: &str, action: GitAction) -> MenuEntry {
    MenuEntry::Item {
        label: label.into(),
        detail: None,
        action: Some(action),
    }
}

fn item_if(label: &str, enabled: bool, action: GitAction) -> MenuEntry {
    MenuEntry::Item {
        label: label.into(),
        detail: None,
        action: enabled.then_some(action),
    }
}

impl Browser {
    // ---- Top strip -------------------------------------------------------------

    pub(super) fn git_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let view = self.git.current();
        let status = view.and_then(RepoView::status).cloned();
        let name = view
            .map(|v| repo_label(&v.repo, status.as_ref()))
            .unwrap_or_else(|| "No repository".into());
        let branch = status.as_ref().map(Status::head_label);
        let busy = self
            .git
            .selected
            .as_ref()
            .and_then(|p| self.git.busy.get(p).cloned());
        let has_repo = view.is_some();
        let sync = status.as_ref().and_then(|s| {
            if s.upstream.is_some() {
                Some((format!("↓{} ↑{}", s.behind, s.ahead), "refresh-ccw", "Sync changes · pull, then push", GitAction::Sync(false)))
            } else if s.branch.is_some() && !s.remotes.is_empty() {
                Some(("Publish".to_string(), "cloud-upload", "Publish this branch", GitAction::Publish(None)))
            } else {
                None
            }
        });
        div()
            .h(rpx(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .child(
                strip_pill("git-repo-picker", "folder-git-2", name).on_click(cx.listener(
                    |this, _, window, cx| this.perform(GitAction::Ask(Ask::SelectRepo), window, cx),
                )),
            )
            .when_some(branch, |s, branch| {
                s.child(
                    strip_pill("git-branch-picker", "git-branch", branch)
                        .tooltip(|window, cx| Tooltip::new("Switch branch").build(window, cx))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.perform(GitAction::Ask(Ask::Checkout), window, cx)
                        })),
                )
            })
            .when_some(sync.filter(|_| busy.is_none()), |s, (label, name, tip, action)| {
                s.child(
                    text_button("git-sync", Some(name), label)
                        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.perform(action.clone(), window, cx)
                        })),
                )
            })
            .when_some(busy, |s, label| {
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .px_2()
                        .min_w_0()
                        .text_size(rpx(12.))
                        .text_color(rgb(TEXT_2))
                        .child(spinner(13., TEXT_2))
                        .child(div().truncate().child(label)),
                )
            })
            .child(div().flex_1().min_w(rpx(8.)))
            .when(has_repo, |s| {
                s.child(
                    icon_button("git-fetch", "cloud-download", "Fetch from all remotes")
                        .on_click(cx.listener(|this, _, window, cx| this.perform(GitAction::Fetch, window, cx))),
                )
                .child(
                    icon_button("git-refresh", "refresh-cw", "Refresh")
                        .on_click(cx.listener(|this, _, window, cx| this.perform(GitAction::Refresh, window, cx))),
                )
                .child(
                    icon_button("git-more", "ellipsis", "More Git actions")
                        .when(self.git.menu.as_ref().is_some_and(|m| m.kind == MenuKind::Repo), |s| {
                            s.bg(rgb(SELECTED))
                        })
                        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                            this.open_git_menu(MenuKind::Repo, event.position(), None);
                            cx.notify();
                        })),
                )
            })
            .child(self.dock_button(cx))
            .child({
                let open = self.sidebar_open();
                icon_button(
                    "sidebar",
                    "panel-right",
                    if open { "Hide source control" } else { "Show source control" },
                )
                .when(open, |s| s.bg(rgb(SELECTED)))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
            })
            .child(
                icon_button(
                    "panel-width",
                    if self.wide { "minimize-2" } else { "maximize-2" },
                    if self.wide { "Restore panel" } else { "Expand panel" },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.wide = !this.wide;
                    this.layout_changed(cx);
                })),
            )
            .child(
                icon_button("close-panel", "x", "Close panel  (Ctrl+Shift+G)")
                    .on_click(cx.listener(|this, _, _, cx| this.close_panel(cx))),
            )
            .into_any_element()
    }

    fn open_git_menu(&mut self, kind: MenuKind, at: Point<Pixels>, commit: Option<String>) {
        let same = self.git.menu.as_ref().is_some_and(|m| m.kind == kind && m.commit == commit);
        self.git.menu = (!same).then_some(GitMenu {
            kind,
            sub: None,
            at,
            commit,
        });
    }

    // ---- Main area ---------------------------------------------------------------

    pub(super) fn git_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.git.detail == Detail::Output {
            return self.output_view(cx);
        }
        if self.git.discovered && self.git.repos.is_empty() {
            return self.no_repository(cx);
        }
        if self.git.selected.is_none() {
            return super::document::empty_state("git-branch", "Looking for repositories…", "", None);
        }
        self.review_content(window, cx)
    }

    fn no_repository(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1p5()
            .child(icon(ui("folder-git-2"), TEXT_2, 30.).mb_2())
            .child(
                div()
                    .text_size(rpx(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child("No Git repository here"),
            )
            .child(
                div()
                    .text_size(rpx(12.5))
                    .text_color(rgb(MUTED))
                    .child("Initialize one, or add this folder's repositories to a project"),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .gap_2()
                    .child(dialog_button("git-init", "Initialize Repository".into(), Style::Primary).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.git.selected = None;
                            this.perform(GitAction::Init, window, cx)
                        }),
                    ))
                    .child(dialog_button("git-edit-project", "Edit Project…".into(), Style::Plain).on_click(
                        cx.listener(|this, _, _, cx| cx.emit(super::BrowserEvent::EditProject(this.root.clone()))),
                    )),
            )
            .into_any_element()
    }

    fn output_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let entries = git::ops::log();
        let rows = entries
            .iter()
            .map(|e| {
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .py_1p5()
                    .border_b_1()
                    .border_color(rgb(DIVIDER))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_shrink_0().text_color(rgb(MUTED)).child(e.time.clone()))
                            .child(div().flex_shrink_0().text_color(rgb(TEXT_2)).child(e.repo.clone()))
                            .child(
                                div()
                                    .min_w_0()
                                    .text_color(rgb(if e.ok { TEXT } else { DELETED }))
                                    .child(e.command.clone()),
                            ),
                    )
                    .when(!e.output.is_empty(), |s| {
                        s.child(div().pl(rpx(64.)).text_color(rgb(MUTED)).child(e.output.clone()))
                    })
            })
            .collect::<Vec<_>>();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(rpx(46.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .border_b_1()
                    .border_color(rgb(DIVIDER))
                    .child(
                        text_button("git-output-back", Some("arrow-left"), "Changes")
                            .on_click(cx.listener(|this, _, window, cx| this.perform(GitAction::HideOutput, window, cx))),
                    )
                    .child(div().text_size(rpx(13.)).text_color(rgb(TEXT)).child("Git Output"))
                    .child(
                        div()
                            .text_size(rpx(12.))
                            .text_color(rgb(MUTED))
                            .child("Every command Vyber ran, newest last"),
                    ),
            )
            .child(
                div()
                    .id("git-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.git.output_scroll)
                    .px_3()
                    .py_2()
                    .font_family(mono_font())
                    .text_size(rpx(11.5))
                    .when(rows.is_empty(), |s| {
                        s.child(div().text_color(rgb(MUTED)).child("No Git commands yet"))
                    })
                    .children(rows),
            )
            .into_any_element()
    }

    /// The commit's message, author, refs and actions above its diff.
    pub(super) fn commit_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(crate::changeset::Source::Commit { hash, .. }) = &self.review.source else {
            return None;
        };
        let (_, info) = self.git.info.as_ref().filter(|(h, _)| h == hash)?;
        let info = info.as_ref().ok()?.clone();
        let (subject, body) = match info.message.split_once('\n') {
            Some((subject, body)) => (subject.to_string(), body.trim().to_string()),
            None => (info.message.clone(), String::new()),
        };
        let date = chrono::DateTime::from_timestamp(info.authored, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%d %b %Y %H:%M").to_string())
            .unwrap_or_default();
        let initial = info.author.chars().next().unwrap_or('?').to_uppercase().to_string();
        let actions = self.view == View::Git;
        let web = self
            .git
            .current()
            .and_then(|r| r.repo.web.clone())
            .map(|web| format!("{web}/commit/{}", info.hash));
        let hash = info.hash.clone();
        let copy_hash = hash.clone();
        let parents = info.parents.clone();
        let mut action_row = div().flex().flex_wrap().gap_1().mt_2();
        if actions {
            let add = |row: Div, id: &'static str, name: &'static str, label: &'static str, action: GitAction| {
                row.child(
                    text_button(id, Some(name), label)
                        .border_1()
                        .border_color(rgb(BORDER))
                        .on_click(cx.listener(move |this, _, window, cx| this.perform(action.clone(), window, cx))),
                )
            };
            action_row = add(action_row, "card-checkout", "git-commit-horizontal", "Checkout", GitAction::Checkout {
                target: hash.clone(),
                kind: CheckoutKind::Detached,
                mode: CheckoutMode::Normal,
                confirmed: false,
            });
            action_row = add(action_row, "card-branch", "git-branch-plus", "Branch…", GitAction::Ask(Ask::NewBranch(Some(hash.clone()))));
            action_row = add(action_row, "card-cherry", "git-merge", "Cherry-pick", GitAction::CherryPick(hash.clone()));
            action_row = add(action_row, "card-revert", "undo-2", "Revert", GitAction::RevertCommit(hash.clone()));
            action_row = add(action_row, "card-reset", "rotate-ccw", "Reset…", GitAction::Ask(Ask::Reset(hash.clone())));
            action_row = add(action_row, "card-tag", "tag", "Tag…", GitAction::Ask(Ask::CreateTag(Some(hash.clone()))));
        }
        action_row = action_row.child(
            text_button("card-copy", Some("copy"), "Copy hash")
                .border_1()
                .border_color(rgb(BORDER))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.perform(GitAction::Copy(copy_hash.clone()), window, cx)
                })),
        );
        if let Some(url) = web {
            action_row = action_row.child(
                text_button("card-web", Some("external-link"), "Open on web")
                    .border_1()
                    .border_color(rgb(BORDER))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.perform(GitAction::OpenUrl(url.clone()), window, cx)
                    })),
            );
        }
        Some(
            div()
                .flex_shrink_0()
                .mx_3()
                .my_2()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(0x0f0f0f))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .child(subject),
                )
                .when(!body.is_empty(), |s| {
                    s.child(
                        div()
                            .max_h(rpx(140.))
                            .overflow_hidden()
                            .text_size(rpx(12.5))
                            .text_color(rgb(TEXT_2))
                            .child(body),
                    )
                })
                .child(
                    div()
                        .mt_1()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .text_size(rpx(12.))
                        .text_color(rgb(MUTED))
                        .child(
                            div()
                                .size(rpx(18.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgb(0x2a2a2a))
                                .text_size(rpx(10.))
                                .text_color(rgb(TEXT))
                                .child(initial),
                        )
                        .child(div().text_color(rgb(TEXT_2)).child(info.author.clone()))
                        .child("·")
                        .child(date)
                        .child("·")
                        .child(
                            div()
                                .id("card-hash")
                                .font_family(mono_font())
                                .cursor_pointer()
                                .hover(|s| s.text_color(rgb(TEXT)))
                                .child(info.hash.chars().take(8).collect::<String>())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.perform(GitAction::Copy(hash.clone()), window, cx)
                                })),
                        )
                        .children(parents.into_iter().enumerate().map(|(i, parent)| {
                            let target = parent.clone();
                            div()
                                .id(("card-parent", i))
                                .flex()
                                .gap_1()
                                .child(if i == 0 { "parent" } else { "+" })
                                .child(
                                    div()
                                        .font_family(mono_font())
                                        .cursor_pointer()
                                        .text_color(rgb(LINK))
                                        .hover(|s| s.underline())
                                        .child(parent.chars().take(7).collect::<String>()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.perform(GitAction::SelectCommit(target.clone()), window, cx)
                                }))
                        }))
                        .children(info.labels.iter().filter_map(label_pill)),
                )
                .child(action_row)
                .into_any_element(),
        )
    }

    // ---- Sidebar -----------------------------------------------------------------

    pub(super) fn git_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let project = project::for_path(&self.root);
        let title = project
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| project::folder_name(&self.root));
        let blocks = if !self.git.discovered {
            vec![div()
                .px_3()
                .py_2()
                .text_size(rpx(12.))
                .text_color(rgb(MUTED))
                .child("Looking for repositories…")
                .into_any_element()]
        } else if self.git.repos.is_empty() {
            vec![div()
                .px_3()
                .py_2()
                .text_size(rpx(12.))
                .text_color(rgb(MUTED))
                .child("No repositories in this folder")
                .into_any_element()]
        } else {
            let paths: Vec<PathBuf> = self
                .git
                .repos
                .iter()
                .filter(|r| {
                    r.repo
                        .worktree_of
                        .as_ref()
                        .is_none_or(|owner| !self.git.collapsed.contains(owner))
                })
                .map(|r| r.repo.path.clone())
                .collect();
            paths
                .into_iter()
                .enumerate()
                .map(|(i, path)| self.repo_block(i, path, cx))
                .collect()
        };
        if self.git.moving() {
            window.request_animation_frame();
        }
        let graph_open = self.git.graph_open;
        let graph = self.git.fold(&Fold::Graph, graph_open);
        let split = self.git.split;
        let selected = self
            .git
            .current()
            .and_then(|view| Some((view.repo.path.clone(), view.status()?.clone())));
        let commit_area = selected.map(|(path, status)| {
            let busy = self.git.busy.get(&path).cloned();
            {
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .pt_0p5()
                    .border_b_1()
                    .border_color(rgb(DIVIDER))
                    .when_some(status.operation, |s, operation| {
                        s.child(self.operation_banner(operation, &status, cx))
                    })
                    .child(self.commit_box(&path, &status, busy.as_deref(), window, cx))
                    .into_any_element()
            }
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_l_1()
            .border_color(rgb(DIVIDER))
            .child(
                div()
                    .h(rpx(34.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .pl_3()
                    .pr_1p5()
                    .group("scm-header")
                    .child(section_title("SOURCE CONTROL"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(11.5))
                            .text_color(rgb(TEXT_2))
                            .child(title),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button("scm-edit-project", "pencil", "Edit project…")
                            .on_click(cx.listener(|this, _, window, cx| this.perform(GitAction::EditProject, window, cx))),
                    )
                    .child(
                        icon_button("scm-collapse", "chevrons-down-up", "Collapse all").on_click(cx.listener(
                            |this, _, _, cx| {
                                let all: Vec<PathBuf> = this.git.repos.iter().map(|r| r.repo.path.clone()).collect();
                                let open = this.git.collapsed.len() >= all.len();
                                for path in &all {
                                    if this.git.collapsed.contains(path) == open {
                                        this.git.set_fold(Fold::Repo(path.clone()), open);
                                    }
                                }
                                if open {
                                    this.git.collapsed.clear();
                                } else {
                                    this.git.collapsed.extend(all);
                                }
                                cx.notify();
                            },
                        )),
                    ),
            )
            // The commit box stays at the top for the selected repository
            // instead of opening under whichever row was clicked.
            .children(commit_area)
            .child(
                div()
                    .id("scm-list")
                    // While the graph slides, the list's share eases between
                    // the split and the whole column.
                    .when(graph > 0.001, |s| s.h(relative(split + (1. - split) * (1. - graph))))
                    .when(graph <= 0.001, |s| s.flex_1())
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.git.list_scroll)
                    .pb_2()
                    .children(blocks),
            )
            .child(self.graph_header(graph_open, graph, cx))
            .when(graph > 0.001, |s| s.child(div().flex_1().min_h_0().flex().flex_col().opacity(graph).child(self.graph_list(cx))))
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<GitSplit>, _, cx| {
                let height = f32::from(e.bounds.size.height).max(1.);
                let y = f32::from(e.event.position.y - e.bounds.top()) - 34. * this.scale;
                this.git.split = (y / height).clamp(0.15, 0.85);
                cx.notify();
            }))
            .into_any_element()
    }

    fn repo_block(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) -> AnyElement {
        let Some(view) = self.git.repo(&path) else {
            return div().into_any_element();
        };
        let repo = view.repo.clone();
        let error = view.status.as_ref().and_then(|s| s.as_ref().err().cloned());
        let status = view.status().cloned();
        let selected = self.git.selected.as_ref() == Some(&path);
        let collapsed = self.git.collapsed.contains(&path);
        let open = self.git.fold(&Fold::Repo(path.clone()), !collapsed);
        let busy = self.git.busy.get(&path).cloned();
        let changes = status.as_ref().map(Status::changes).unwrap_or(0);
        let depth = usize::from(repo.is_worktree());
        let group: SharedString = format!("scm-repo-{index}").into();
        let label = repo_label(&repo, status.as_ref());
        let detail = if repo.is_worktree() {
            repo.name.clone()
        } else {
            status.as_ref().map(Status::head_label).unwrap_or_default()
        };
        // What the sync button says: pull and push counts, or publish.
        let sync = status.as_ref().and_then(|st| {
            let upstream = st.upstream.clone()?;
            let tip: SharedString = match (st.behind, st.ahead) {
                (0, 0) => format!("Up to date with {upstream} · click to sync").into(),
                (behind, ahead) => format!(
                    "{behind} {} to pull, {ahead} to push · click to sync with {upstream}",
                    if behind == 1 { "commit" } else { "commits" }
                )
                .into(),
            };
            let label = if st.behind == 0 && st.ahead == 0 {
                String::new()
            } else {
                format!("{}↓ {}↑", st.behind, st.ahead)
            };
            Some(("refresh-ccw", label, tip, GitAction::Sync(false)))
        })
        .or_else(|| {
            status
                .as_ref()
                .filter(|st| st.branch.is_some() && !st.remotes.is_empty())
                .map(|_| {
                    (
                        "cloud-upload",
                        "Publish".to_string(),
                        SharedString::from("Publish this branch to its remote"),
                        GitAction::Publish(None),
                    )
                })
        });
        let agent = self.tasks.iter().any(|t| {
            t.active && (crate::tasks::matches_root(&t.root, &path) || crate::tasks::matches_root(&path, &t.root))
        });
        let open_path = path.clone();
        let toggle_path = path.clone();
        let menu_path = path.clone();
        let sync_path = path.clone();
        let header = div()
            .id(("scm-repo", index))
            .group(group.clone())
            .h(rpx(28.))
            .mx_1()
            .flex()
            .items_center()
            .gap_1p5()
            .pl(rpx(6. + depth as f32 * 14.))
            .pr_1()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |s| s.bg(rgb(SELECTED)))
            .when(!selected, |s| s.hover(|s| s.bg(rgb(HOVER))))
            .child(
                div()
                    .id(("scm-repo-toggle", index))
                    .child(chevron(open, MUTED, 13.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let now_open = this.git.collapsed.remove(&toggle_path);
                        if !now_open {
                            this.git.collapsed.insert(toggle_path.clone());
                        }
                        this.git.set_fold(Fold::Repo(toggle_path.clone()), now_open);
                        cx.notify();
                    })),
            )
            .child(icon(
                ui(if repo.is_worktree() { "git-fork" } else { "folder-git-2" }),
                if selected { TEXT } else { TEXT_2 },
                14.,
            ))
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(rpx(170.))
                    .truncate()
                    .text_size(rpx(12.5))
                    .text_color(rgb(TEXT))
                    .when(selected, |s| s.font_weight(FontWeight::SEMIBOLD))
                    .child(label),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(rpx(11.5))
                    .text_color(rgb(MUTED))
                    .child(detail),
            )
            .when(agent, |s| {
                s.child(
                    div()
                        .size(rpx(6.))
                        .rounded_full()
                        .bg(rgb(TEXT_2))
                        .with_animation(
                            ("scm-agent", index),
                            Animation::new(Duration::from_millis(1400))
                                .repeat()
                                .with_easing(pulsating_between(0.25, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
            })
            .when(changes > 0, |s| s.child(count_badge(changes)))
            .when_some(busy.clone(), |s, _| s.child(div().px_1().child(spinner(12., TEXT_2))))
            .when_some(sync.filter(|_| busy.is_none()), |s, (name, label, tip, action)| {
                // Incoming and outgoing commits, always in view like Cursor's.
                s.child(
                    div()
                        .id(("scm-repo-sync", index))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .h(rpx(22.))
                        .px_1p5()
                        .rounded(rpx(4.))
                        .text_size(rpx(11.))
                        .text_color(rgb(TEXT_2))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x2a2a2a)).text_color(rgb(TEXT)))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .child(icon(ui(name), TEXT_2, 13.))
                        .when(!label.is_empty(), |s| s.child(label))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.git.selected = Some(sync_path.clone());
                            this.perform(action.clone(), window, cx);
                        })),
                )
            })
            .child(
                bar_action(("scm-repo-more", index), "ellipsis", "More actions").on_click(cx.listener(
                    move |this, event: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if this.git.selected.as_ref() != Some(&menu_path) {
                            this.select_repo(menu_path.clone(), window, cx);
                        }
                        this.open_git_menu(MenuKind::Repo, event.position(), None);
                        cx.notify();
                    },
                )),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.perform(GitAction::SelectRepo(open_path.clone()), window, cx);
            }));
        let block = div().flex().flex_col().child(header);
        if open <= 0.001 {
            return block.into_any_element();
        }
        let mut body = div().flex().flex_col();
        let mut height = 0.;
        if let Some(error) = error {
            body = body.child(
                div()
                    .pl(rpx(28. + depth as f32 * 14.))
                    .pr_2()
                    .py_1()
                    .text_size(rpx(11.5))
                    .text_color(rgb(WARNING))
                    .child(error.lines().next().unwrap_or_default().to_string()),
            );
            return block.child(folding(body, open, 24.)).into_any_element();
        }
        let Some(status) = status else {
            return block.into_any_element();
        };
        let groups: [(Group, &'static str, &Vec<Entry>); 3] = [
            (Group::Merge, "Merge Changes", &status.conflicts),
            (Group::Staged, "Staged Changes", &status.staged),
            (Group::Changes, "Changes", &status.unstaged),
        ];
        for (group_kind, title, entries) in groups {
            if entries.is_empty() {
                continue;
            }
            let (group, group_height) = self.change_group(index, &path, depth, group_kind, title, entries, cx);
            body = body.child(group);
            height += group_height;
        }
        block.child(folding(body, open, height)).into_any_element()
    }

    fn operation_banner(&self, operation: Operation, status: &Status, cx: &mut Context<Self>) -> AnyElement {
        let conflicts = status.conflicts.len();
        let text = if conflicts > 0 {
            format!(
                "{} · {conflicts} {}",
                operation.label(),
                if conflicts == 1 { "conflict" } else { "conflicts" }
            )
        } else {
            format!("{} · ready to continue", operation.label())
        };
        let button = |id: &'static str, label: &'static str, step: &'static str, primary: bool| {
            dialog_button(id, label.into(), if primary { Style::Primary } else { Style::Plain })
                .h(rpx(24.))
                .px_2()
                .text_size(rpx(11.5))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.perform(GitAction::Sequence(operation, step), window, cx)
                }))
        };
        div()
            .mx_2()
            .mb_1p5()
            .px_2()
            .py_1p5()
            .rounded_md()
            .bg(rgb(WARNING_BG))
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(rpx(12.))
                    .text_color(rgb(WARNING))
                    .child(icon(ui("git-merge"), WARNING, 13.))
                    .child(text),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(button("scm-op-abort", "Abort", "abort", false))
                    .when(matches!(operation, Operation::Rebase { .. } | Operation::CherryPick | Operation::Revert), |s| {
                        s.child(button("scm-op-skip", "Skip", "skip", false))
                    })
                    .when(conflicts == 0 && operation != Operation::Bisect, |s| {
                        s.child(button("scm-op-continue", "Continue", "continue", true))
                    }),
            )
            .into_any_element()
    }

    fn commit_box(
        &mut self,
        path: &std::path::Path,
        status: &Status,
        busy: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = self.message_input(path, window, cx);
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let changes = status.changes();
        let (label, action, enabled): (String, GitAction, bool) = if changes > 0 || status.operation.is_some() {
            (
                if status.operation == Some(Operation::Merge) { "Commit Merge" } else { "Commit" }.into(),
                GitAction::Commit {
                    amend: false,
                    then: None,
                    stage_all: false,
                },
                status.conflicts.is_empty(),
            )
        } else if status.upstream.is_some() && (status.ahead > 0 || status.behind > 0) {
            (
                format!("Sync Changes ↓{} ↑{}", status.behind, status.ahead),
                GitAction::Sync(false),
                true,
            )
        } else if status.branch.is_some() && status.upstream.is_none() && !status.remotes.is_empty() {
            ("Publish Branch".into(), GitAction::Publish(None), true)
        } else {
            (
                "Commit".into(),
                GitAction::Commit {
                    amend: false,
                    then: None,
                    stage_all: false,
                },
                false,
            )
        };
        let enabled = enabled && busy.is_none();
        let (bg, fg) = if enabled { (PRIMARY_BG, PRIMARY_TEXT) } else { (0x262626, MUTED) };
        div()
            .mx_2()
            .mt_0p5()
            .mb_2()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if focused { 0x3a3a3a } else { BORDER }))
                    .bg(rgb(0x101010))
                    .key_context("GitMessage")
                    .child(
                        Textarea::new(&input)
                            .appearance(false)
                            .bordered(false)
                            .small()
                            .text_size(rpx(12.5)),
                    ),
            )
            .child(
                div()
                    .h(rpx(28.))
                    .flex()
                    .rounded_md()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("scm-commit")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1p5()
                            .bg(rgb(bg))
                            .text_color(rgb(fg))
                            .text_size(rpx(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .when_some(busy, |s, busy| s.child(spinner(13., fg)).child(busy.to_string()))
                            .when(busy.is_none(), |s| {
                                s.child(icon(ui("check"), fg, 14.)).child(label)
                            })
                            .when(enabled, |s| {
                                s.cursor_pointer()
                                    .hover(|s| s.bg(rgb(0xffffff)))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.perform(action.clone(), window, cx)
                                    }))
                            }),
                    )
                    // One button: the options part shares the main part's color
                    // and is set off only by a thin line.
                    .child(
                        div()
                            .w(px(1.))
                            .h_full()
                            .py(rpx(6.))
                            .bg(rgb(bg))
                            .child(div().size_full().bg(rgb(if enabled { 0xb4b4b4 } else { 0x3a3a3a }))),
                    )
                    .child(
                        div()
                            .id("scm-commit-options")
                            .w(rpx(30.))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(bg))
                            .tooltip(|window, cx| Tooltip::new("More commit actions").build(window, cx))
                            .when(busy.is_none(), |s| {
                                s.cursor_pointer()
                                    .hover(|s| s.bg(rgb(if enabled { 0xffffff } else { 0x2e2e2e })))
                                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                                        this.open_git_menu(MenuKind::CommitOptions, event.position(), None);
                                        cx.notify();
                                    }))
                            })
                            .child(icon(ui("chevron-down"), fg, 14.)),
                    ),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn change_group(
        &self,
        repo_index: usize,
        repo: &std::path::Path,
        depth: usize,
        kind: Group,
        title: &'static str,
        entries: &[Entry],
        cx: &mut Context<Self>,
    ) -> (AnyElement, f32) {
        let closed = self.git.closed.contains(&(repo.to_path_buf(), kind));
        let open = self.git.fold(&Fold::Group(repo.to_path_buf(), kind), !closed);
        let group: SharedString = format!("scm-group-{repo_index}-{kind:?}").into();
        let key = (repo.to_path_buf(), kind);
        let all: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        let repo_path = repo.to_path_buf();
        let indent = 20. + depth as f32 * 14.;
        let header = div()
            .id(("scm-group", repo_index * 4 + kind as usize))
            .group(group.clone())
            .h(rpx(ROW))
            .mx_1()
            .flex()
            .items_center()
            .gap_1()
            .pl(rpx(indent - 4.))
            .pr_1()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(HOVER)))
            .child(chevron(open, MUTED, 12.))
            .child(div().text_size(rpx(12.)).text_color(rgb(TEXT_2)).child(title))
            .child(count_badge(entries.len()))
            .child(div().flex_1())
            .when(kind == Group::Staged, |s| {
                let repo = repo_path.clone();
                s.child(
                    bar_action(("scm-unstage-all", repo_index), "minus", "Unstage all changes").on_click(
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.perform(GitAction::UnstageAll(repo.clone()), window, cx)
                        }),
                    ),
                )
            })
            .when(kind == Group::Changes, |s| {
                let discard = repo_path.clone();
                let stage = repo_path.clone();
                s.child(
                    bar_action(("scm-discard-all", repo_index), "rotate-ccw", "Discard all changes").on_click(
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.perform(GitAction::DiscardAll(discard.clone(), false), window, cx)
                        }),
                    ),
                )
                .child(
                    bar_action(("scm-stage-all", repo_index), "plus", "Stage all changes").on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.perform(GitAction::StageAll(stage.clone()), window, cx)
                        },
                    )),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                let now_open = this.git.closed.remove(&key);
                if !now_open {
                    this.git.closed.insert(key.clone());
                }
                this.git.set_fold(Fold::Group(key.0.clone(), key.1), now_open);
                cx.notify();
            }));
        let _ = all;
        let column = div().flex().flex_col().child(header);
        let rows_height = (entries.len().min(GROUP_LIMIT) + usize::from(entries.len() > GROUP_LIMIT)) as f32 * ROW;
        if open <= 0.001 {
            return (column.into_any_element(), ROW);
        }
        let mut rows = div().flex().flex_col();
        let revealed = self.review.reveal_path.clone();
        for (i, entry) in entries.iter().take(GROUP_LIMIT).enumerate() {
            rows = rows.child(self.file_row(repo_index, repo, indent, kind, i, entry, revealed.as_deref(), cx));
        }
        if entries.len() > GROUP_LIMIT {
            rows = rows.child(
                div()
                    .pl(rpx(indent + 20.))
                    .h(rpx(ROW))
                    .flex()
                    .items_center()
                    .text_size(rpx(11.5))
                    .text_color(rgb(MUTED))
                    .child(format!("… {} more", entries.len() - GROUP_LIMIT)),
            );
        }
        (
            column.child(folding(rows, open, rows_height)).into_any_element(),
            ROW + rows_height * open,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn file_row(
        &self,
        repo_index: usize,
        repo: &std::path::Path,
        indent: f32,
        kind: Group,
        index: usize,
        entry: &Entry,
        revealed: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (folder, name) = match entry.path.rsplit_once('/') {
            Some((folder, name)) => (folder.to_string(), name.to_string()),
            None => (String::new(), entry.path.clone()),
        };
        let absolute = repo.join(&entry.path);
        let (icon_path, color) = icons::file_icon(&absolute);
        let group: SharedString = format!("scm-file-{repo_index}-{kind:?}-{index}").into();
        let id = (repo_index * 100_000 + (kind as usize) * 30_000 + index) as u64;
        let deleted = entry.letter == 'D';
        let staged = kind == Group::Staged;
        let current = self.git.selected.as_deref() == Some(repo)
            && self.git.detail == Detail::Diff
            && revealed == Some(entry.path.as_str());
        let repo_path = repo.to_path_buf();
        let path = entry.path.clone();
        let tooltip: SharedString = match (&entry.from, entry.conflict) {
            (_, Some(conflict)) => format!("{} · {conflict}", entry.path).into(),
            (Some(from), _) => format!("{from} → {}", entry.path).into(),
            _ => entry.path.clone().into(),
        };
        let mut row = div()
            .id(ElementId::Integer(id))
            .group(group.clone())
            .h(rpx(ROW))
            .mx_1()
            .flex()
            .items_center()
            .gap_1p5()
            .pl(rpx(indent + 14.))
            .pr_1()
            .rounded_md()
            .cursor_pointer()
            .when(current, |s| s.bg(rgb(SELECTED)))
            .when(!current, |s| s.hover(|s| s.bg(rgb(HOVER))))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(icon(icon_path, color, 14.))
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(rpx(200.))
                    .truncate()
                    .text_size(rpx(12.5))
                    .text_color(rgb(if deleted { MUTED } else { 0xd0d0d0 }))
                    .when(deleted, |s| s.line_through())
                    .child(name),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(rpx(11.))
                    .text_color(rgb(MUTED))
                    .child(folder),
            );
        if !deleted {
            let open = absolute.clone();
            row = row.child(
                row_action(("scm-open", id), "file-text", "Open file", &group).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open(open.clone(), true, cx);
                    },
                )),
            );
        }
        match kind {
            Group::Merge => {
                let ours = (repo_path.clone(), path.clone());
                let theirs = (repo_path.clone(), path.clone());
                let resolve = (repo_path.clone(), path.clone());
                row = row
                    .child(
                        row_action(("scm-ours", id), "arrow-left", "Keep the current version", &group).on_click(
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.perform(GitAction::TakeSide(ours.0.clone(), ours.1.clone(), true), window, cx)
                            }),
                        ),
                    )
                    .child(
                        row_action(("scm-theirs", id), "arrow-down", "Keep the incoming version", &group).on_click(
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.perform(GitAction::TakeSide(theirs.0.clone(), theirs.1.clone(), false), window, cx)
                            }),
                        ),
                    )
                    .child(
                        row_action(("scm-resolve", id), "plus", "Mark resolved (stage)", &group).on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.perform(GitAction::Stage(resolve.0.clone(), vec![resolve.1.clone()]), window, cx)
                            },
                        )),
                    );
            }
            Group::Staged => {
                let target = (repo_path.clone(), path.clone());
                row = row.child(
                    row_action(("scm-unstage", id), "minus", "Unstage", &group).on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.perform(GitAction::Unstage(target.0.clone(), vec![target.1.clone()]), window, cx)
                        },
                    )),
                );
            }
            Group::Changes => {
                let untracked = entry.letter == 'U';
                let discard = (repo_path.clone(), path.clone());
                let stage = (repo_path.clone(), path.clone());
                row = row
                    .child(
                        row_action(("scm-discard", id), "rotate-ccw", "Discard changes", &group).on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                let (tracked, untracked) = if untracked {
                                    (vec![], vec![discard.1.clone()])
                                } else {
                                    (vec![discard.1.clone()], vec![])
                                };
                                this.perform(
                                    GitAction::Discard {
                                        repo: discard.0.clone(),
                                        tracked,
                                        untracked,
                                        confirmed: false,
                                    },
                                    window,
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        row_action(("scm-stage", id), "plus", "Stage", &group).on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.perform(GitAction::Stage(stage.0.clone(), vec![stage.1.clone()]), window, cx)
                            },
                        )),
                    );
            }
        }
        let letter = entry.letter;
        let click_repo = repo_path.clone();
        let click_path = path.clone();
        let conflict_file = absolute.clone();
        row.child(
            div()
                .w(rpx(14.))
                .flex_shrink_0()
                .text_center()
                .text_size(rpx(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(status_color(letter)))
                .child(letter.to_string()),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if kind == Group::Merge {
                // Conflict markers are edited in the file itself.
                this.open(conflict_file.clone(), true, cx);
                return;
            }
            this.perform(
                GitAction::ShowChanges(click_repo.clone(), Some(click_path.clone()), staged),
                window,
                cx,
            );
        }))
        .into_any_element()
    }

    // ---- Graph -------------------------------------------------------------------

    fn graph_header(&self, open: bool, shown: f32, cx: &mut Context<Self>) -> AnyElement {
        let all = self.git.graph.all;
        let name = self
            .git
            .current()
            .map(|v| repo_label(&v.repo, v.status()))
            .unwrap_or_default();
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .when(open, |s| {
                s.child(
                    div()
                        .id("scm-split")
                        .h(rpx(5.))
                        .flex()
                        .items_center()
                        .cursor_row_resize()
                        .group("scm-split")
                        .child(
                            div()
                                .h(px(1.))
                                .w_full()
                                .bg(rgb(DIVIDER))
                                .group_hover("scm-split", |s| s.h(rpx(2.)).bg(rgb(0x4a4a4a))),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_drag(GitSplit, |value, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| value.clone())
                        }),
                )
            })
            .when(!open, |s| s.border_t_1().border_color(rgb(DIVIDER)))
            .child(
                div()
                    .id("scm-graph-header")
                    .h(rpx(30.))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .pl_2()
                    .pr_1p5()
                    .cursor_pointer()
                    .child(chevron(shown, MUTED, 13.))
                    .child(section_title("GRAPH"))
                    .child(div().min_w_0().flex_1().truncate().text_size(rpx(11.5)).text_color(rgb(TEXT_2)).child(name))
                    .when(open, |s| {
                        s.child(
                            div()
                                .id("scm-graph-scope")
                                .flex_shrink_0()
                                .h(rpx(20.))
                                .px_2()
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded(rpx(10.))
                                .border_1()
                                .border_color(rgb(BORDER))
                                .text_size(rpx(11.))
                                .text_color(rgb(TEXT_2))
                                .hover(|s| s.bg(rgb(HOVER)))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Show the current branch or every branch").build(window, cx)
                                })
                                .child(icon(ui("git-graph"), TEXT_2, 12.))
                                .child(if all { "All branches" } else { "Current" })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.perform(GitAction::ToggleGraphScope, window, cx)
                                })),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git.graph_open = !this.git.graph_open;
                        this.git.set_fold(Fold::Graph, this.git.graph_open);
                        if this.git.graph_open && this.git.graph.commits.is_empty() {
                            this.load_graph(false);
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn graph_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let graph = &self.git.graph;
        if graph.commits.is_empty() {
            let text = if let Some(error) = &graph.error {
                error.lines().next().unwrap_or_default().to_string()
            } else if graph.loading {
                "Reading history…".to_string()
            } else {
                "No commits yet".to_string()
            };
            return div()
                .flex_1()
                .px_3()
                .py_1()
                .text_size(rpx(12.))
                .text_color(rgb(MUTED))
                .child(text)
                .into_any_element();
        }
        let commits = Rc::new(graph.commits.clone());
        let rows = Rc::new(graph.rows.clone());
        let lanes = rows.iter().map(|r| r.width).max().unwrap_or(1).min(14);
        let width = 12. + lanes as f32 * LANE;
        let selected = graph.selected.clone();
        let incoming = Rc::new(graph.incoming.clone());
        let outgoing = Rc::new(graph.outgoing.clone());
        let more = graph.more;
        let count = commits.len() + usize::from(more);
        let entity = cx.entity();
        uniform_list("scm-graph", count, move |range, _, _| {
            range
                .map(|i| {
                    let Some(commit) = commits.get(i) else {
                        let entity = entity.clone();
                        return div()
                            .id("scm-graph-more")
                            .h(rpx(GRAPH_ROW))
                            .pl(rpx(width + 8.))
                            .flex()
                            .items_center()
                            .text_size(rpx(12.))
                            .text_color(rgb(LINK))
                            .cursor_pointer()
                            .hover(|s| s.underline())
                            .child("Load more commits")
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| this.perform(GitAction::LoadMoreHistory, window, cx))
                            })
                            .into_any_element();
                    };
                    let row = &rows[i];
                    let head = commit
                        .labels
                        .iter()
                        .any(|l| matches!(l, Label::Head(_) | Label::Detached));
                    let is_selected = selected.as_deref() == Some(commit.hash.as_str());
                    let marker = if incoming.contains(&commit.hash) {
                        Some(("arrow-down", "Incoming · on the upstream, not here yet"))
                    } else if outgoing.contains(&commit.hash) {
                        Some(("arrow-up", "Outgoing · not pushed yet"))
                    } else {
                        None
                    };
                    let tooltip: SharedString = format!(
                        "{}\n{} · {} · {}",
                        commit.subject,
                        commit.author,
                        git::age(commit.time),
                        commit.short()
                    )
                    .into();
                    let hash = commit.hash.clone();
                    let menu_hash = commit.hash.clone();
                    let click = entity.clone();
                    let context = entity.clone();
                    let more_labels = commit.labels.len().saturating_sub(2);
                    let labels: Vec<Div> = commit.labels.iter().take(2).filter_map(label_pill).collect();
                    div()
                        .id(("scm-commit-row", i))
                        .h(rpx(GRAPH_ROW))
                        .w_full()
                        .px_1()
                        .child(
                    div()
                        .id(("scm-commit-cell", i))
                        .size_full()
                        .pr_2()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .rounded_md()
                        .cursor_pointer()
                        .when(is_selected, |s| s.bg(rgb(SELECTED)))
                        .when(!is_selected, |s| s.hover(|s| s.bg(rgb(HOVER))))
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                        .child(graph_cell(row, head, width))
                        // The subject keeps its room; ref pills give way first.
                        .child(
                            div()
                                .min_w(rpx(72.))
                                .flex_shrink(1.)
                                .truncate()
                                .text_size(rpx(12.5))
                                .text_color(rgb(if head || is_selected { TEXT } else { 0xc4c4c4 }))
                                .child(commit.subject.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .min_w_0()
                                .max_w(rpx(190.))
                                .flex_shrink(12.)
                                .overflow_hidden()
                                .children(labels)
                                .when(more_labels > 0, |s| {
                                    s.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(rpx(10.5))
                                            .text_color(rgb(MUTED))
                                            .child(format!("+{more_labels}")),
                                    )
                                }),
                        )
                        .child(div().flex_1())
                        .when_some(marker, |s, (name, _)| s.child(icon(ui(name), TEXT_2, 12.)))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(rpx(11.))
                                .text_color(rgb(MUTED))
                                .child(git::age(commit.time)),
                        )
                        .on_click(move |_, window, cx| {
                            let hash = hash.clone();
                            click.update(cx, |this, cx| this.perform(GitAction::SelectCommit(hash), window, cx))
                        })
                        )
                        .on_mouse_down(MouseButton::Right, move |event, _, cx| {
                            let hash = menu_hash.clone();
                            let at = event.position;
                            context.update(cx, |this, cx| {
                                this.open_git_menu(MenuKind::Commit, at, Some(hash));
                                cx.notify();
                            })
                        })
                        .into_any_element()
                })
                .collect()
        })
        .track_scroll(&self.git.graph.scroll)
        .flex_1()
        .pb_2()
        .into_any_element()
    }

    // ---- Overlays ------------------------------------------------------------------

    /// Menus, the quick pick and dialogs, drawn over the whole panel.
    pub(super) fn git_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if self.git.menu.is_some() {
            out.push(self.menu_overlay(cx));
        }
        // A closed picker or dialog is drawn once more per frame, fading out.
        let fade = |at: std::time::Instant| 1. - (at.elapsed().as_secs_f32() / FADE_TIME).min(1.);
        if let Some((quick, at)) = self.git.closing_quick.take() {
            let left = fade(at);
            if left > 0. && self.git.quick.is_none() {
                self.git.quick = Some(quick);
                let ghost = self.quick_overlay(window, cx);
                let quick = self.git.quick.take();
                self.git.closing_quick = quick.map(|q| (q, at));
                out.push(div().absolute().top_0().left_0().size_full().opacity(left).child(ghost).into_any_element());
                window.request_animation_frame();
            }
        }
        if self.git.quick.is_some() {
            let element = self.quick_overlay(window, cx);
            out.push(div().absolute().top_0().left_0().size_full().child(element).with_animation(
                "git-quick-in",
                Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            ).into_any_element());
        }
        if let Some((confirm, at)) = self.git.closing_confirm.take() {
            let left = fade(at);
            if left > 0. && self.git.confirm.is_none() {
                self.git.confirm = Some(confirm);
                let ghost = self.confirm_overlay(cx);
                let confirm = self.git.confirm.take();
                self.git.closing_confirm = confirm.map(|c| (c, at));
                out.push(div().absolute().top_0().left_0().size_full().opacity(left).child(ghost).into_any_element());
                window.request_animation_frame();
            }
        }
        if self.git.confirm.is_some() {
            // Keys reach the panel so Escape and Enter answer the dialog.
            if !self.focus.is_focused(window) {
                window.focus(&self.focus, cx);
            }
            let element = self.confirm_overlay(cx);
            out.push(div().absolute().top_0().left_0().size_full().child(element).with_animation(
                "git-confirm-in",
                Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            ).into_any_element());
        }
        out
    }

    fn menu_entries(&self, kind: MenuKind, commit: Option<String>) -> Vec<MenuEntry> {
        let status = self.git.current().and_then(RepoView::status).cloned().unwrap_or_default();
        let repo = self.git.selected.clone().unwrap_or_default();
        let has_head = status.oid.is_some();
        let has_upstream = status.upstream.is_some();
        let has_remote = !status.remotes.is_empty();
        let on_branch = status.branch.is_some();
        let changes = status.changes() > 0;
        let rebasing = matches!(status.operation, Some(Operation::Rebase { .. }));
        match kind {
            MenuKind::CommitOptions => vec![
                item_if("Commit", changes, GitAction::Commit { amend: false, then: None, stage_all: false }),
                item_if("Commit (Amend)", has_head, GitAction::Commit { amend: true, then: None, stage_all: false }),
                item_if(
                    "Commit & Push",
                    changes && has_remote,
                    GitAction::Commit { amend: false, then: Some(Box::new(GitAction::Push(false, false))), stage_all: false },
                ),
                item_if(
                    "Commit & Sync",
                    changes && has_upstream,
                    GitAction::Commit { amend: false, then: Some(Box::new(GitAction::Sync(true))), stage_all: false },
                ),
                MenuEntry::Separator,
                item_if("Stage All & Commit", changes, GitAction::Commit { amend: false, then: None, stage_all: true }),
                item_if("Undo Last Commit", has_head, GitAction::UndoCommit(false)),
            ],
            MenuKind::Commit => {
                let hash = commit.unwrap_or_default();
                let web = self
                    .git
                    .current()
                    .and_then(|r| r.repo.web.clone())
                    .map(|w| format!("{w}/commit/{hash}"));
                let message = self
                    .git
                    .graph
                    .commits
                    .iter()
                    .find(|c| c.hash == hash)
                    .map(|c| c.subject.clone())
                    .unwrap_or_default();
                let mut entries = vec![
                    item("Show Changes", GitAction::SelectCommit(hash.clone())),
                    MenuEntry::Separator,
                    item(
                        "Checkout (Detached)",
                        GitAction::Checkout {
                            target: hash.clone(),
                            kind: CheckoutKind::Detached,
                            mode: CheckoutMode::Normal,
                            confirmed: false,
                        },
                    ),
                    item("Create Branch…", GitAction::Ask(Ask::NewBranch(Some(hash.clone())))),
                    item("Create Tag…", GitAction::Ask(Ask::CreateTag(Some(hash.clone())))),
                    MenuEntry::Separator,
                    item("Cherry-pick", GitAction::CherryPick(hash.clone())),
                    item("Revert Commit", GitAction::RevertCommit(hash.clone())),
                    item("Reset Current Branch Here…", GitAction::Ask(Ask::Reset(hash.clone()))),
                    MenuEntry::Separator,
                    item("Copy Commit Hash", GitAction::Copy(hash.clone())),
                    item("Copy Commit Message", GitAction::Copy(message)),
                ];
                if let Some(web) = web {
                    entries.push(item("Open on Web", GitAction::OpenUrl(web)));
                }
                entries
            }
            MenuKind::Repo => {
                let worktree = self.git.current().is_some_and(|r| r.repo.is_worktree());
                vec![
                    item_if("Pull", has_upstream, GitAction::Pull(false, false)),
                    item_if("Push", on_branch && has_remote, GitAction::Push(false, false)),
                    item("Checkout to…", GitAction::Ask(Ask::Checkout)),
                    item_if("Fetch", has_remote, GitAction::Fetch),
                    MenuEntry::Separator,
                    MenuEntry::Sub(
                        "Commit".into(),
                        vec![
                            item_if("Commit Staged", !status.staged.is_empty(), GitAction::Commit { amend: false, then: None, stage_all: false }),
                            item_if("Commit All", changes, GitAction::Commit { amend: false, then: None, stage_all: true }),
                            item_if("Commit (Amend)", has_head, GitAction::Commit { amend: true, then: None, stage_all: false }),
                            item_if("Undo Last Commit", has_head, GitAction::UndoCommit(false)),
                            item_if("Abort Rebase", rebasing, GitAction::Sequence(status.operation.unwrap_or(Operation::Merge), "abort")),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Changes".into(),
                        vec![
                            item_if("Stage All Changes", !status.unstaged.is_empty(), GitAction::StageAll(repo.clone())),
                            item_if("Unstage All Changes", !status.staged.is_empty(), GitAction::UnstageAll(repo.clone())),
                            item_if("Discard All Changes", !status.unstaged.is_empty(), GitAction::DiscardAll(repo.clone(), false)),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Pull, Push".into(),
                        vec![
                            item_if("Sync", has_upstream, GitAction::Sync(false)),
                            MenuEntry::Separator,
                            item_if("Pull", has_upstream, GitAction::Pull(false, false)),
                            item_if("Pull (Rebase)", has_upstream, GitAction::Pull(true, false)),
                            MenuEntry::Separator,
                            item_if("Push", on_branch && has_remote, GitAction::Push(false, false)),
                            item_if("Force Push (with lease)", has_upstream, GitAction::Push(true, false)),
                            MenuEntry::Separator,
                            item_if("Fetch", has_remote, GitAction::Fetch),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Branch".into(),
                        vec![
                            item_if("Merge…", has_head, GitAction::Ask(Ask::Merge)),
                            item_if("Rebase Branch…", has_head, GitAction::Ask(Ask::Rebase)),
                            MenuEntry::Separator,
                            item("Create Branch…", GitAction::Ask(Ask::NewBranch(None))),
                            item_if("Create Branch From…", has_head, GitAction::Ask(Ask::NewBranchFrom)),
                            item_if("Rename Branch…", has_head, GitAction::Ask(Ask::RenameBranch)),
                            item_if("Delete Branch…", has_head, GitAction::Ask(Ask::DeleteBranch)),
                            item_if("Delete Remote Branch…", has_remote, GitAction::Ask(Ask::DeleteRemoteBranch)),
                            MenuEntry::Separator,
                            item_if("Publish Branch…", on_branch && has_remote && !has_upstream, GitAction::Publish(None)),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Remote".into(),
                        vec![
                            item("Add Remote…", GitAction::Ask(Ask::AddRemote)),
                            item_if("Remove Remote…", has_remote, GitAction::Ask(Ask::RemoveRemote)),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Stash".into(),
                        vec![
                            item_if("Stash", changes, GitAction::Ask(Ask::StashMessage { untracked: false, staged: false })),
                            item_if("Stash (Include Untracked)", changes, GitAction::Ask(Ask::StashMessage { untracked: true, staged: false })),
                            item_if("Stash Staged", !status.staged.is_empty(), GitAction::Ask(Ask::StashMessage { untracked: false, staged: true })),
                            MenuEntry::Separator,
                            item_if("Apply Latest Stash", status.stashes > 0, GitAction::StashAct { action: "apply", name: "stash@{0}".into(), confirmed: false }),
                            item_if("Apply Stash…", status.stashes > 0, GitAction::Ask(Ask::Stash("apply"))),
                            item_if("Pop Latest Stash", status.stashes > 0, GitAction::StashAct { action: "pop", name: "stash@{0}".into(), confirmed: false }),
                            item_if("Pop Stash…", status.stashes > 0, GitAction::Ask(Ask::Stash("pop"))),
                            MenuEntry::Separator,
                            item_if("Drop Stash…", status.stashes > 0, GitAction::Ask(Ask::Stash("drop"))),
                            item_if("Drop All Stashes…", status.stashes > 0, GitAction::StashClear(false)),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Tags".into(),
                        vec![
                            item_if("Create Tag…", has_head, GitAction::Ask(Ask::CreateTag(None))),
                            item("Delete Tag…", GitAction::Ask(Ask::DeleteTag)),
                            item_if("Push Tags", has_remote, GitAction::PushTags),
                        ],
                    ),
                    MenuEntry::Sub(
                        "Worktrees".into(),
                        vec![
                            item_if("Create Worktree…", has_head, GitAction::Ask(Ask::Worktree)),
                            item("Open Worktree in New Tab…", GitAction::Ask(Ask::OpenWorktree)),
                            item("Remove Worktree…", GitAction::Ask(Ask::RemoveWorktree)),
                            item("Prune Worktrees", GitAction::PruneWorktrees),
                        ],
                    ),
                    MenuEntry::Separator,
                    item("Open in New Tab", GitAction::OpenTab(repo.clone())),
                    item("Reveal in Files", GitAction::RevealInFiles(repo.clone())),
                    item_if("Remove This Worktree…", worktree, GitAction::RemoveWorktree(repo.clone(), false, false)),
                    MenuEntry::Separator,
                    item("Show Git Output", GitAction::ShowOutput),
                    item("Edit Project…", GitAction::EditProject),
                ]
            }
        }
    }

    fn render_entries(
        &self,
        entries: &[MenuEntry],
        prefix: &'static str,
        submenu: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open_sub = self.git.menu.as_ref().and_then(|m| m.sub);
        let mut panel = menu_panel(prefix).w(rpx(if submenu { 230. } else { 250. }));
        for (i, entry) in entries.iter().enumerate() {
            panel = match entry {
                MenuEntry::Separator => panel.child(separator()),
                MenuEntry::Item { label, detail, action } => {
                    let enabled = action.is_some();
                    let action = action.clone();
                    panel.child(
                        menu_item((prefix, i), label.clone(), detail.clone(), false, enabled, false).when_some(
                            action,
                            |s, action| {
                                s.on_click(cx.listener(move |this, _, window, cx| {
                                    this.perform(action.clone(), window, cx)
                                }))
                            },
                        ),
                    )
                }
                MenuEntry::Sub(label, _) => panel.child(
                    menu_item((prefix, i), label.clone(), None, false, true, true)
                        .when(open_sub == Some(i), |s| s.bg(rgb(HOVER)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(menu) = &mut this.git.menu {
                                menu.sub = (menu.sub != Some(i)).then_some(i);
                            }
                            cx.notify();
                        })),
                ),
            };
        }
        panel
    }

    fn menu_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = self.git.menu.clone() else {
            return div().into_any_element();
        };
        let entries = self.menu_entries(menu.kind, menu.commit.clone());
        let main = self.render_entries(&entries, "git-menu", false, cx);
        // A submenu opens beside its item: 30px per item, 9px per separator.
        let submenu = menu.sub.and_then(|sub| {
            let MenuEntry::Sub(_, children) = entries.get(sub)? else {
                return None;
            };
            let offset: f32 = entries[..sub]
                .iter()
                .map(|e| if matches!(e, MenuEntry::Separator) { 9. } else { 30. })
                .sum();
            Some(div().mt(rpx(offset)).child(self.render_entries(children, "git-submenu", true, cx)))
        });
        let right = matches!(menu.kind, MenuKind::Repo | MenuKind::CommitOptions);
        deferred(
            anchored()
                .position(menu.at)
                .anchor(if right { Anchor::TopRight } else { Anchor::TopLeft })
                .snap_to_window_with_margin(px(8.))
                .child(appear(
                    "git-menu-in",
                    div()
                        .id("git-menus")
                        .occlude()
                        .mt_1()
                        .flex()
                        .when(right, |s| s.flex_row_reverse())
                        .items_start()
                        .gap_1()
                        .child(main)
                        .children(submenu)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.git.menu = None;
                            cx.notify();
                        })),
                )),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn quick_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.quick_visible(cx);
        let Some(quick) = &self.git.quick else {
            return div().into_any_element();
        };
        let selected = quick.selected.min(visible.len().saturating_sub(1));
        let focused = quick.input.read(cx).focus_handle(cx).is_focused(window);
        let mut last_section = None;
        let rows: Vec<AnyElement> = visible
            .iter()
            .enumerate()
            .map(|(n, &index)| {
                let item = &quick.items[index];
                let header = item
                    .section
                    .filter(|section| last_section != Some(*section))
                    .inspect(|section| last_section = Some(*section));
                let active = n == selected;
                div()
                    .flex()
                    .flex_col()
                    .when_some(header, |s, header| {
                        s.child(
                            div()
                                .h(rpx(24.))
                                .px_2()
                                .flex()
                                .items_end()
                                .pb_0p5()
                                .text_size(rpx(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(MUTED))
                                .child(header),
                        )
                    })
                    .child(
                        div()
                            .id(("quick-item", n))
                            .h(rpx(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .cursor_pointer()
                            .when(active, |s| s.bg(rgb(SELECTED)))
                            .when(!active, |s| s.hover(|s| s.bg(rgb(HOVER))))
                            .child(icon(ui(item.icon), if active { TEXT } else { TEXT_2 }, 14.))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .max_w(rpx(260.))
                                    .truncate()
                                    .text_size(rpx(12.5))
                                    .text_color(rgb(TEXT))
                                    .child(item.label.clone()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_size(rpx(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(item.detail.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(rpx(11.))
                                    .text_color(rgb(MUTED))
                                    .child(item.right.clone()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(quick) = &mut this.git.quick {
                                    quick.selected = n;
                                }
                                this.accept_quick(window, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let empty = if quick.loading {
            Some("Loading…")
        } else if rows.is_empty() && !quick.text {
            Some("Nothing matches")
        } else {
            None
        };
        let hint = quick.text.then_some("Enter to confirm · Esc to cancel");
        div()
            .id("git-quick-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x00000059))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_quick(window, cx)),
            )
            .child(
                div()
                    .absolute()
                    .top(rpx(50.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .px_4()
                    .child(
                        div()
                            .id("git-quick")
                            .w(rpx(560.))
                            .max_w_full()
                            .p_1p5()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .rounded_lg()
                            .bg(rgb(0x141414))
                            .border_1()
                            .border_color(rgb(0x2c2c2c))
                            .shadow_xl()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                match event.keystroke.key.as_str() {
                                    "escape" => {
                                        cx.stop_propagation();
                                        this.close_quick(window, cx);
                                    }
                                    "down" => {
                                        cx.stop_propagation();
                                        this.move_quick(1, cx);
                                    }
                                    "up" => {
                                        cx.stop_propagation();
                                        this.move_quick(-1, cx);
                                    }
                                    _ => {}
                                }
                            }))
                            .child(
                                div()
                                    .px_1p5()
                                    .pt_0p5()
                                    .text_size(rpx(11.))
                                    .text_color(rgb(MUTED))
                                    .child(quick.title.clone()),
                            )
                            .child(
                                div()
                                    .h(rpx(32.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(if focused { 0x3b82f6 } else { BORDER }))
                                    .bg(rgb(PANEL))
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            Input::new(&quick.input)
                                                .appearance(false)
                                                .small()
                                                .text_size(rpx(13.)),
                                        ),
                                    ),
                            )
                            .when_some(hint, |s, hint| {
                                s.child(div().px_1p5().text_size(rpx(11.)).text_color(rgb(FAINT)).child(hint))
                            })
                            .when(!rows.is_empty() || empty.is_some(), |s| {
                                s.child(
                                    div()
                                        .id("git-quick-list")
                                        .max_h(rpx(380.))
                                        .overflow_y_scroll()
                                        .track_scroll(&quick.scroll)
                                        .when_some(empty, |s, text| {
                                            s.child(
                                                div()
                                                    .px_2()
                                                    .py_1p5()
                                                    .text_size(rpx(12.))
                                                    .text_color(rgb(MUTED))
                                                    .child(text),
                                            )
                                        })
                                        .children(rows),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn confirm_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(confirm) = self.git.confirm.clone() else {
            return div().into_any_element();
        };
        let buttons = confirm
            .buttons
            .iter()
            .enumerate()
            .map(|(i, button)| {
                let action = button.action.clone();
                dialog_button(("git-confirm-button", i), button.label.clone(), button.style).on_click(cx.listener(
                    move |this, _, window, cx| match action.clone() {
                        Some(action) => this.perform(action, window, cx),
                        None => {
                            this.dismiss_confirm();
                            cx.notify();
                        }
                    },
                ))
            })
            .collect::<Vec<_>>();
        div()
            .id("git-confirm-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x00000073))
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .child(
                div()
                    .id("git-confirm")
                    .w(rpx(460.))
                    .max_w_full()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_xl()
                    .bg(rgb(0x161616))
                    .border_1()
                    .border_color(rgb(0x2c2c2c))
                    .shadow_xl()
                    .child(
                        div()
                            .text_size(rpx(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .child(confirm.title.clone()),
                    )
                    .when(!confirm.message.is_empty(), |s| {
                        s.child(
                            div()
                                .text_size(rpx(12.5))
                                .text_color(rgb(TEXT_2))
                                .child(confirm.message.clone()),
                        )
                    })
                    .when_some(confirm.detail.clone().filter(|d| !d.trim().is_empty()), |s, detail| {
                        s.child(
                            div()
                                .id("git-confirm-detail")
                                .max_h(rpx(180.))
                                .overflow_y_scroll()
                                .p_2()
                                .rounded_md()
                                .bg(rgb(0x0c0c0c))
                                .border_1()
                                .border_color(rgb(BORDER))
                                .font_family(mono_font())
                                .text_size(rpx(11.5))
                                .text_color(rgb(0xbdbdbd))
                                .child(detail),
                        )
                    })
                    .child(div().mt_2().flex().justify_end().gap_2().children(buttons)),
            )
            .into_any_element()
    }
}
