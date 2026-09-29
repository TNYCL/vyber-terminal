//! Stable group tabs, local split titles and their small popovers.
use super::*;
use gpui_kit::component::{
    Sizable,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};

#[derive(Clone, Copy)]
pub(super) enum TabMenu {
    Group { id: usize, at: Point<Pixels> },
    List { at: Point<Pixels> },
    ConfirmClose { id: usize, at: Point<Pixels> },
}

pub(super) struct RenameTab {
    id: usize,
    input: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Clone, Copy)]
enum TabCommand {
    Rename,
    SplitRight,
    SplitDown,
    Close,
}

fn tool_button(id: impl Into<ElementId>, icon: &str, label: String) -> Stateful<Div> {
    bar_button(id, "")
        .size(px(22.))
        .rounded_sm()
        .child(theme::icon(theme::ui(icon), theme::TEXT_2, 13.))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
}

fn drag_scroll_offset(offset: f32, max: f32, pointer: f32, width: f32) -> f32 {
    let direction = if pointer < 28. {
        1.
    } else if pointer > width - 28. {
        -1.
    } else {
        0.
    };
    (offset + direction * 7.).clamp(-max.max(0.), 0.)
}

impl Vyber {
    pub(super) fn scroll_dragged_tabs(&mut self, window: &mut Window) {
        let Some(drag) = &self.tab_drag else { return };
        let bounds = self.tab_scroll.bounds();
        let pointer = window.mouse_position();
        if !bounds.contains(&pointer) {
            return;
        }
        let offset = self.tab_scroll.offset();
        let x = drag_scroll_offset(
            f32::from(offset.x),
            f32::from(self.tab_scroll.max_offset().x),
            f32::from(pointer.x - bounds.left()),
            f32::from(bounds.size.width),
        );
        if x == f32::from(offset.x) {
            return;
        }
        let offset = point(px(x), offset.y);
        self.tab_scroll.set_offset(offset);
        // A stationary pointer can now be over a different tab. Move the
        // insertion marker with the content rather than keep a stale target.
        self.drop_hint = None;
        for (index, layout) in self.tabs.iter().enumerate() {
            if let Some(tab) = self.tab_scroll.bounds_for_item(index) {
                let position = pointer - offset;
                if tab.contains(&position) {
                    let anchor = layout.first();
                    if anchor != drag.id && !(drag.group && layout.contains(drag.id)) {
                        self.drop_hint = Some(DropHint::Tab {
                            id: anchor,
                            after: position.x > tab.center().x,
                        });
                    }
                    break;
                }
            }
        }
        window.request_animation_frame();
    }

    fn group_name(&self, index: usize, cx: &App) -> String {
        self.tab_state.groups[index]
            .name
            .clone()
            .unwrap_or_else(|| {
                self.slots.get(&self.tabs[index].first()).map_or_else(
                    || "Terminal".into(),
                    |slot| {
                        let terminal = slot.terminal.read(cx);
                        terminal
                            .root
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| terminal.root.display().to_string())
                    },
                )
            })
    }

    fn group_busy(&self, layout: &Layout) -> bool {
        layout.leaves().iter().any(|id| {
            self.slots
                .get(id)
                .and_then(|s| s.turn.as_ref())
                .is_some_and(|t| t.active)
        })
    }

    pub(super) fn group_tab(
        &self,
        index: usize,
        layout: &Layout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = &self.tab_state.groups[index];
        let id = group.id;
        let anchor = layout.first();
        let active = index == self.tab;
        let count = layout.leaves().len();
        let busy = self.group_busy(layout);
        let name = self.group_name(index, cx);
        let tooltip = format!(
            "{name} · {count} terminal{}\nDouble-click to rename · Right-click for actions",
            if count == 1 { "" } else { "s" },
        );
        let hover_group: SharedString = format!("workspace-tab-{id}").into();
        self.tab_drop_zone(anchor, cx)
            .id(("workspace-group", id))
            .group(hover_group.clone())
            .h(px(29.))
            .w(px(196.))
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(px(7.))
            .px(px(8.))
            .rounded_md()
            .border_1()
            .border_color(rgb(if active { 0x343434 } else { 0x000000 }))
            .bg(rgb(if active { theme::SELECTED } else { 0x000000 }))
            .text_color(rgb(if active { theme::TEXT } else { theme::TEXT_2 }))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(theme::icon(
                theme::ui(if count > 1 {
                    "columns-2"
                } else {
                    "square-terminal"
                }),
                if active { theme::LINK } else { theme::MUTED },
                14.,
            ))
            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
            // Both the count and the close button reserve space, so hovering
            // or selecting a tab never moves its neighbours.
            .child(
                div()
                    .w(px(20.))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .text_size(px(10.))
                    .text_color(rgb(theme::TEXT_2))
                    .when(count > 1, |s| s.child(count.to_string())),
            )
            .child(
                div()
                    .size(px(5.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(rgb(theme::LINK))
                    .opacity(if busy { 1. } else { 0. }),
            )
            .child(
                tool_button(
                    ("close-workspace", id),
                    "x",
                    format!(
                        "Close group · {count} terminal{}",
                        if count == 1 { "" } else { "s" }
                    ),
                )
                .opacity(if active { 1. } else { 0. })
                .group_hover(hover_group, |s| s.opacity(1.))
                .on_click(cx.listener(
                    move |this, event: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.request_close_group(id, event.position(), window, cx);
                    },
                )),
            )
            .when(active, |s| {
                s.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(8.))
                        .right(px(8.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(rgb(theme::LINK)),
                )
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if let Some(index) = this.tab_state.index(id) {
                    this.switch_tab(index, window, cx);
                    if event.click_count() == 2 {
                        this.start_rename_tab(id, window, cx);
                    }
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.show_shortcuts = false;
                    this.tab_menu = Some(TabMenu::Group {
                        id,
                        at: event.position,
                    });
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.request_close_group(id, event.position, window, cx);
                }),
            )
            .on_drag(
                TabDrag {
                    id: anchor,
                    group: true,
                    label: name,
                },
                |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
            .into_any_element()
    }

    /// Only split groups need a local terminal title. A lone terminal keeps
    /// the full height and can still be dragged from its group tab.
    pub(super) fn pane_title(&self, id: usize, cx: &mut Context<Self>) -> AnyElement {
        let terminal = self.slots[&id].terminal.read(cx);
        let active = self.active == id;
        let title = if terminal.title.trim().is_empty() {
            "Terminal".to_string()
        } else {
            terminal.title.clone()
        };
        let tooltip = format!("{}\nDrag to move this terminal", terminal.root.display());
        div()
            .id(("pane-title", id))
            .occlude()
            .h(px(27.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(6.))
            .bg(rgb(if active { 0x151515 } else { theme::PANEL }))
            .border_b_1()
            .border_color(rgb(if active { 0x30445c } else { theme::BORDER }))
            .text_color(rgb(if active { theme::TEXT } else { theme::MUTED }))
            .text_size(px(11.))
            .cursor_move()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(theme::icon(
                theme::ui("square-terminal"),
                if active { theme::LINK } else { theme::MUTED },
                12.,
            ))
            .child(div().min_w_0().truncate().child(title.clone()))
            .child(
                div()
                    .text_color(rgb(theme::MUTED))
                    .child(format!("· {}", id + 1)),
            )
            .when(terminal.exited, |s| {
                s.child(div().text_color(rgb(theme::WARNING)).child("exited"))
            })
            .child(div().flex_1())
            .child(
                tool_button(
                    ("zoom-terminal", id),
                    if self.zoomed {
                        "minimize-2"
                    } else {
                        "maximize-2"
                    },
                    "Focus / restore terminal".into(),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.focus_pane(id, window, cx);
                    this.zoomed = !this.zoomed;
                    cx.notify();
                })),
            )
            .child(
                tool_button(
                    ("close-terminal", id),
                    "x",
                    "Close this terminal only".into(),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_pane(id, window, cx);
                })),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.focus_pane(id, window, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_pane(id, window, cx);
                }),
            )
            .on_drag(
                TabDrag {
                    id,
                    group: false,
                    label: title,
                },
                |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
            .into_any_element()
    }

    fn request_close_group(
        &mut self,
        id: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_state.index(id) else {
            return;
        };
        if self.tabs[index].leaves().len() > 1 {
            self.tab_menu = Some(TabMenu::ConfirmClose { id, at });
            self.show_shortcuts = false;
            cx.notify();
        } else {
            self.tab_menu = None;
            self.close_tab(index, window, cx);
        }
    }

    fn run_tab_command(
        &mut self,
        id: usize,
        command: TabCommand,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_state.index(id) else {
            return;
        };
        self.tab_menu = None;
        match command {
            TabCommand::Rename => self.start_rename_tab(id, window, cx),
            TabCommand::SplitRight | TabCommand::SplitDown => {
                self.switch_tab(index, window, cx);
                self.zoomed = false;
                self.add_terminal(
                    self.current_root(cx),
                    true,
                    matches!(command, TabCommand::SplitDown),
                    window,
                    cx,
                );
            }
            TabCommand::Close => self.request_close_group(id, at, window, cx),
        }
    }

    fn start_rename_tab(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tab_state.index(id) else {
            return;
        };
        let name = self.group_name(index, cx);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Group name")
                .default_value(name)
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.finish_rename_tab(true, window, cx);
            }
        });
        self.tab_menu = None;
        self.show_shortcuts = false;
        self.rename_tab = Some(RenameTab {
            id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn finish_rename_tab(
        &mut self,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(rename) = self.rename_tab.take() {
            if save {
                self.tab_state
                    .rename(rename.id, rename.input.read(cx).value().as_ref());
                self.persist(cx);
            }
            if let Some(slot) = self.slots.get(&self.active) {
                window.focus(&slot.terminal.read(cx).focus.clone(), cx);
            }
            cx.notify();
        }
    }

    pub(super) fn toggle_tab_list(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.show_shortcuts = false;
        // Outside-click capture runs before the trigger's click handler.
        let just_closed = self
            .tab_list_dismissed
            .take()
            .is_some_and(|at| at.elapsed() < Duration::from_millis(500));
        self.tab_menu = if matches!(self.tab_menu, Some(TabMenu::List { .. })) || just_closed {
            None
        } else {
            Some(TabMenu::List {
                at: point(at.x, px(38.)),
            })
        };
        cx.notify();
    }

    pub(super) fn tab_overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = self.tab_menu {
            let mut card = div()
                .id("tab-popover")
                .occlude()
                .w(px(300.))
                .p_1()
                .flex()
                .flex_col()
                .rounded_lg()
                .bg(rgb(0x121212))
                .border_1()
                .border_color(rgb(0x333333))
                .shadow_xl()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if matches!(this.tab_menu, Some(TabMenu::List { .. })) {
                        this.tab_list_dismissed = Some(std::time::Instant::now());
                    }
                    this.tab_menu = None;
                    cx.notify();
                }));
            let (at, anchor) =
                match menu {
                    TabMenu::Group { id, at } => {
                        if let Some(index) = self.tab_state.index(id) {
                            card = card.child(
                                div()
                                    .px_2()
                                    .py_2()
                                    .text_size(px(11.))
                                    .text_color(rgb(theme::MUTED))
                                    .truncate()
                                    .child(self.group_name(index, cx)),
                            );
                            for (n, label, icon, command) in [
                                (0usize, "Rename group…", "pencil", TabCommand::Rename),
                                (1, "Split right", "columns-2", TabCommand::SplitRight),
                                (2, "Split down", "rows-2", TabCommand::SplitDown),
                                (3, "Close group…", "x", TabCommand::Close),
                            ] {
                                card = card.child(
                                    chip(("tab-command", n), label)
                                        .h(px(30.))
                                        .gap_2()
                                        .child(theme::icon(theme::ui(icon), theme::TEXT_2, 14.))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.run_tab_command(id, command, at, window, cx);
                                        })),
                                );
                            }
                        }
                        (at, Anchor::TopLeft)
                    }
                    TabMenu::List { at } => {
                        card = card.child(
                            div()
                                .px_2()
                                .py_2()
                                .text_size(px(11.))
                                .text_color(rgb(theme::MUTED))
                                .child("OPEN GROUPS"),
                        );
                        let rows = self
                            .tabs
                            .iter()
                            .enumerate()
                            .map(|(index, layout)| {
                                let id = self.tab_state.groups[index].id;
                                let active = index == self.tab;
                                let path = self.slots[&layout.first()]
                                    .terminal
                                    .read(cx)
                                    .root
                                    .display()
                                    .to_string();
                                div()
                                    .id(("tab-list-item", id))
                                    .px_2()
                                    .py_2()
                                    .rounded_md()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .cursor_pointer()
                                    .when(active, |s| s.bg(rgb(theme::SELECTED)))
                                    .hover(|s| s.bg(rgb(theme::HOVER)))
                                    .child(theme::icon(
                                        theme::ui(if active { "check" } else { "square-terminal" }),
                                        if active { theme::LINK } else { theme::MUTED },
                                        14.,
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(12.))
                                                    .child(self.group_name(index, cx)),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(theme::MUTED))
                                                    .child(path),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(theme::TEXT_2))
                                            .child(layout.leaves().len().to_string()),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(index) = this.tab_state.index(id) {
                                            this.switch_tab(index, window, cx);
                                        }
                                    }))
                            })
                            .collect::<Vec<_>>();
                        card = card.child(
                            div()
                                .id("tab-list-scroll")
                                .max_h(px(360.))
                                .overflow_y_scroll()
                                .children(rows),
                        );
                        (at, Anchor::TopRight)
                    }
                    TabMenu::ConfirmClose { id, at } => {
                        if let Some(index) = self.tab_state.index(id) {
                            let count = self.tabs[index].leaves().len();
                            card = card
                                .child(
                                    div().px_2().py_2().text_size(px(12.)).child(format!(
                                        "Close all {count} terminals in this group?"
                                    )),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .pb_2()
                                        .text_size(px(11.))
                                        .text_color(rgb(theme::MUTED))
                                        .child("Running sessions will be stopped."),
                                )
                                .child(
                                    chip("confirm-close-group", "Close group")
                                        .text_color(rgb(theme::DELETED))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.tab_menu = None;
                                            if let Some(index) = this.tab_state.index(id) {
                                                this.close_tab(index, window, cx);
                                            }
                                        })),
                                )
                                .child(chip("cancel-close-group", "Cancel").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.tab_menu = None;
                                        cx.notify();
                                    },
                                )));
                        }
                        (at, Anchor::TopLeft)
                    }
                };
            overlays.push(
                deferred(
                    anchored()
                        .position(at)
                        .anchor(anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(card),
                )
                .with_priority(2)
                .into_any_element(),
            );
        }
        if let Some(rename) = &self.rename_tab {
            overlays.push(
                div()
                    .id("rename-tab-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude()
                    .bg(rgba(0x00000088))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.finish_rename_tab(false, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .id("rename-tab-card")
                            .w(px(360.))
                            .p_4()
                            .rounded_lg()
                            .bg(rgb(theme::SURFACE))
                            .border_1()
                            .border_color(rgb(0x383838))
                            .shadow_xl()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Rename group"),
                            )
                            .capture_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "escape" {
                                        this.finish_rename_tab(false, window, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .child(Input::new(&rename.input).small())
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(theme::MUTED))
                                    .child("Leave empty to use the folder name."),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(chip("cancel-rename-tab", "Cancel").on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.finish_rename_tab(false, window, cx);
                                        }),
                                    ))
                                    .child(
                                        chip("save-rename-tab", "Save")
                                            .bg(rgb(theme::SELECTED))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.finish_rename_tab(true, window, cx);
                                            })),
                                    ),
                            ),
                    )
                    .into_any_element(),
            );
        }
        overlays
    }
}

#[cfg(test)]
mod tests {
    use super::drag_scroll_offset;

    #[test]
    fn dragging_scrolls_only_at_edges_and_stops_at_content_limits() {
        assert_eq!(drag_scroll_offset(-50., 100., 10., 200.), -43.);
        assert_eq!(drag_scroll_offset(-50., 100., 190., 200.), -57.);
        assert_eq!(drag_scroll_offset(-50., 100., 100., 200.), -50.);
        assert_eq!(drag_scroll_offset(0., 100., 10., 200.), 0.);
        assert_eq!(drag_scroll_offset(-100., 100., 190., 200.), -100.);
        assert_eq!(drag_scroll_offset(0., 0., 190., 200.), 0.);
    }
}
