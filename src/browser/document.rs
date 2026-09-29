//! Tab strip, breadcrumb bar and the document area (editor, Markdown,
//! images, unsupported files and the empty state).
use super::{Browser, Kind, Menu, View};
use crate::{icons, platform, theme::*};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    input::Editor,
    text::{TextView, TextViewStyle},
};

impl Browser {
    pub(super) fn tab_strip(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let review_active = self.view == View::Review;
        let change_count = self.changes.len();
        let tabs = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, doc)| {
                let active = !review_active && i == self.active_doc;
                let name = doc
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let (icon_path, color) = icons::file_icon(&doc.path);
                let group: SharedString = format!("doc-tab-{i}").into();
                let dirty = doc.dirty;
                tab(("doc", i), active)
                    .group(group.clone())
                    .max_w(px(220.))
                    .child(icon(icon_path, color, 14.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .when(!doc.pinned, |s| s.italic())
                            .child(name),
                    )
                    .child(
                        div()
                            .id(("close-doc", i))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(18.))
                            .flex_shrink_0()
                            .rounded(px(4.))
                            .hover(|s| s.bg(rgb(SELECTED)))
                            .when(dirty, |s| {
                                s.child(div().size(px(7.)).rounded_full().bg(rgb(TEXT_2)))
                            })
                            .when(!dirty, |s| {
                                s.child(
                                    icon(ui("x"), if active { TEXT_2 } else { PANEL }, 13.)
                                        .group_hover(group, |s| s.text_color(rgb(TEXT_2))),
                                )
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_doc(i, cx);
                            })),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| this.close_doc(i, cx)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_doc = i;
                        this.view = View::Files;
                        this.menu = None;
                        if let Some(doc) = this.docs.get(i) {
                            this.tree_selected = Some(this.relative(&doc.path));
                            this.reveal = true;
                        }
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        div()
            .h(px(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .child(
                tab("review-tab", review_active)
                    .child(icon(
                        ui("git-compare"),
                        if review_active { TEXT } else { TEXT_2 },
                        14.,
                    ))
                    .child("Review")
                    .when(change_count > 0, |s| {
                        s.child(
                            div()
                                .px_1p5()
                                .rounded(px(4.))
                                .bg(rgb(SELECTED))
                                .text_size(px(10.5))
                                .text_color(rgb(TEXT_2))
                                .child(change_count.to_string()),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view = View::Review;
                        this.menu = None;
                        cx.notify();
                    })),
            )
            .when(!self.docs.is_empty(), |s| {
                s.child(div().w(px(1.)).h(px(18.)).mx_0p5().bg(rgb(BORDER)))
            })
            .child(
                div()
                    .id("doc-tabs")
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
            .child(
                icon_button("new-doc", "plus", "Find file  (Ctrl+Shift+P)").on_click(
                    cx.listener(|this, _, window, cx| this.focus_filter(window, cx)),
                ),
            )
            .child(div().flex_1().min_w(px(8.)))
            .child(
                icon_button(
                    "panel-width",
                    if self.wide { "minimize-2" } else { "maximize-2" },
                    if self.wide { "Restore panel" } else { "Expand panel" },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.wide = !this.wide;
                    cx.notify();
                })),
            )
            .child(
                icon_button(
                    "follow",
                    "eye",
                    if self.follow {
                        "Following changes · click to stop"
                    } else {
                        "Follow changed files"
                    },
                )
                .when(self.follow, |s| s.bg(rgb(SELECTED)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.follow = !this.follow;
                    this.say(if this.follow {
                        "Following changed files"
                    } else {
                        "Stopped following"
                    });
                    cx.notify();
                })),
            )
            .child(
                icon_button(
                    "pin",
                    if self.pinned { "pin-off" } else { "pin" },
                    if self.pinned {
                        "Unpin preview"
                    } else {
                        "Pin preview · keep this file while following"
                    },
                )
                .when(self.pinned, |s| s.bg(rgb(SELECTED)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.pinned = !this.pinned;
                    cx.notify();
                })),
            )
            .child(
                icon_button("close-panel", "panel-right-close", "Close panel  (Ctrl+Shift+B)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.visible = false;
                        this.menu = None;
                        cx.notify();
                    })),
            )
    }

    pub(super) fn document_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(doc) = self.active_document() else {
            return empty_state(
                "files",
                "Open file",
                "Select a file from the workspace tree",
                Some("Ctrl+Shift+P finds a file by name"),
            );
        };
        let path = doc.path.clone();
        let kind = doc.kind.clone();
        let markdown = doc.markdown() && kind == Kind::Text;
        let preview = doc.preview;
        let conflict = doc.conflict;
        let dirty = doc.dirty;
        let size = doc.size;
        let relative = self.relative(&path);
        let body = match &kind {
            Kind::Image => div()
                .id("image-preview")
                .size_full()
                .overflow_scroll()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .child(
                    img(path.clone())
                        .w(relative_size(self.zoom))
                        .h(relative_size(self.zoom))
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Kind::Unsupported(reason) => self.unsupported(&path, reason.clone(), size, cx),
            Kind::Text if markdown && preview => {
                let source = doc.editor.read(cx).value();
                let copy = source.clone();
                div()
                    .relative()
                    .size_full()
                    .child(
                        div()
                            .id(("markdown", self.active_doc))
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&doc.scroll)
                            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                                if this.follow {
                                    this.follow = false;
                                    this.say("Follow paused while reading");
                                    cx.notify();
                                }
                            }))
                            .px_8()
                            .pt_5()
                            .pb_12()
                            .text_size(px(14.))
                            .line_height(rems(1.6))
                            .text_color(rgb(0xd6d6d6))
                            .child(
                                TextView::markdown(("markdown-text", self.active_doc), source)
                                    .style(markdown_style())
                                    .selectable(true),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_3()
                            .right_4()
                            .child(icon_button("copy-markdown", "copy", "Copy Markdown").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy.to_string(),
                                    ));
                                    this.say("Markdown copied");
                                    cx.notify();
                                }),
                            )),
                    )
                    .into_any_element()
            }
            Kind::Text => div()
                .size_full()
                .child(
                    Editor::new(&doc.editor)
                        .appearance(false)
                        .bordered(false)
                        .h_full(),
                )
                .into_any_element(),
        };
        let _ = window;
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(self.breadcrumb(&relative, &kind, markdown, preview, dirty, cx))
            .when(conflict, |s| {
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(34.))
                        .px_3()
                        .bg(rgb(WARNING_BG))
                        .text_color(rgb(WARNING))
                        .text_size(px(12.))
                        .child(icon(ui("triangle-alert"), WARNING, 14.))
                        .child(
                            div()
                                .flex_1()
                                .child("Changed on disk · your unsaved edits are protected"),
                        )
                        .child(
                            text_button("reload", Some("refresh-cw"), "Reload from disk").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.reload_from_disk(window, cx)
                                }),
                            ),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
            .when(self.menu == Some(Menu::Open), |s| {
                s.child(self.open_menu(path.clone(), cx))
            })
            .into_any_element()
    }

    fn breadcrumb(
        &self,
        relative: &str,
        kind: &Kind,
        markdown: bool,
        preview: bool,
        dirty: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let root_name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let parts = relative.split('/').collect::<Vec<_>>();
        let mut crumbs: Vec<AnyElement> = vec![];
        let mut crumb = |id: usize, label: String, folder: Option<String>, last: bool| {
            if id > 0 {
                crumbs.push(
                    icon(ui("chevron-right"), FAINT, 12.)
                        .mx_0p5()
                        .into_any_element(),
                );
            }
            crumbs.push(
                div()
                    .id(("crumb", id))
                    .flex_shrink_0()
                    .px_1()
                    .rounded(px(4.))
                    .text_color(rgb(if last { TEXT } else { MUTED }))
                    .when(last, |s| s.font_weight(FontWeight::SEMIBOLD))
                    .when(!last, |s| {
                        s.cursor_pointer()
                            .hover(|s| s.text_color(rgb(TEXT_2)).bg(rgb(HOVER)))
                    })
                    .child(label)
                    .when_some(folder, |s, folder| {
                        s.on_click(cx.listener(move |this, _, _, cx| {
                            this.view = View::Files;
                            if folder.is_empty() {
                                this.tree_root = None;
                            } else {
                                this.expanded
                                    .extend(crate::workspace::expanded_parents(&format!(
                                        "{folder}/x"
                                    )));
                                this.tree_selected = Some(folder.clone());
                                this.reveal = true;
                            }
                            cx.notify();
                        }))
                    })
                    .into_any_element(),
            );
        };
        crumb(0, root_name, Some(String::new()), false);
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let folder = (!last).then(|| parts[..=i].join("/"));
            crumb(i + 1, part.to_string(), folder, last);
        }
        let copy_path = relative.to_string();
        div()
            .h(px(36.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pl_3()
            .pr_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .text_size(px(12.5))
            .child(
                div()
                    .id("breadcrumb")
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(crumbs),
            )
            .when(*kind == Kind::Image, |s| {
                s.child(icon_button("zoom-out", "zoom-out", "Zoom out").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom = (this.zoom / 1.25).max(0.25);
                        cx.notify();
                    },
                )))
                .child(
                    div()
                        .w(px(40.))
                        .text_center()
                        .text_size(px(11.5))
                        .text_color(rgb(MUTED))
                        .child(format!("{:.0}%", self.zoom * 100.)),
                )
                .child(icon_button("zoom-in", "zoom-in", "Zoom in").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom = (this.zoom * 1.25).min(8.);
                        cx.notify();
                    },
                )))
                .child(icon_button("zoom-fit", "scan", "Fit to panel").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom = 1.;
                        cx.notify();
                    },
                )))
            })
            .when(markdown, |s| {
                s.child(
                    text_button(
                        "view-source",
                        Some(if preview { "code" } else { "book-open" }),
                        if preview { "View source" } else { "Preview" },
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(doc) = this.docs.get_mut(this.active_doc) {
                            doc.preview = !doc.preview;
                        }
                        cx.notify();
                    })),
                )
            })
            .when(dirty, |s| {
                s.child(
                    text_button("save", Some("save"), "Save")
                        .text_color(rgb(TEXT))
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                )
            })
            .child(icon_button("copy-path", "copy", "Copy relative path").on_click(
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()));
                    this.say(format!("Copied {copy_path}"));
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .id("open-with")
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(26.))
                    .pl_2()
                    .pr_1p5()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .text_size(px(12.))
                    .text_color(rgb(TEXT))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .child(icon(ui("external-link"), TEXT_2, 13.))
                    .child("Open")
                    .child(icon(ui("chevron-down"), MUTED, 13.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_menu(Menu::Open);
                        cx.notify();
                    })),
            )
    }

    fn open_menu(&self, path: std::path::PathBuf, cx: &mut Context<Self>) -> impl IntoElement {
        let reveal_label = if cfg!(target_os = "macos") {
            "Reveal in Finder"
        } else {
            "Reveal in File Explorer"
        };
        let items: [(&str, &str, fn(&std::path::Path) -> std::io::Result<()>); 3] = [
            ("app-window", "Open with default app", platform::open_default),
            ("folder-search", reveal_label, platform::reveal),
            ("code", "Open in VS Code", platform::open_in_code),
        ];
        div()
            .id("open-menu")
            .absolute()
            .occlude()
            .top(px(38.))
            .right_2()
            .w(px(230.))
            .p_1()
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(rgb(0x121212))
            .border_1()
            .border_color(rgb(BORDER))
            .shadow_xl()
            .children(items.into_iter().enumerate().map(|(i, (name, label, run))| {
                let path = path.clone();
                div()
                    .id(("open-option", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(30.))
                    .px_2()
                    .rounded_md()
                    .text_size(px(12.5))
                    .text_color(rgb(TEXT))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .child(icon(ui(name), TEXT_2, 14.))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.launch(run, path.clone());
                        cx.notify();
                    }))
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_menu();
                cx.notify();
            }))
    }

    fn unsupported(
        &self,
        path: &std::path::Path,
        reason: SharedString,
        size: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (icon_path, color) = icons::file_icon(path);
        let open = path.to_path_buf();
        let reveal = path.to_path_buf();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(icon(icon_path, color, 36.))
            .child(
                div()
                    .mt_2()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child(name),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(MUTED))
                    .child(format!("{reason} · {}", human_size(size))),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .gap_2()
                    .child(
                        text_button("unsupported-open", Some("app-window"), "Open with default app")
                            .border_1()
                            .border_color(rgb(BORDER))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.launch(platform::open_default, open.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        text_button("unsupported-reveal", Some("folder-search"), "Show in folder")
                            .border_1()
                            .border_color(rgb(BORDER))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.launch(platform::reveal, reveal.clone());
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}

fn tab(id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap_1p5()
        .h(px(28.))
        .pl_2p5()
        .pr_1()
        .rounded_lg()
        .border_1()
        .text_size(px(12.5))
        .cursor_pointer()
        .when(active, |s| {
            s.bg(rgb(SURFACE))
                .border_color(rgb(0x2c2c2c))
                .text_color(rgb(TEXT))
        })
        .when(!active, |s| {
            s.border_color(rgba(0x00000000))
                .text_color(rgb(MUTED))
                .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT_2)))
        })
}

pub(super) fn empty_state(
    name: &str,
    title: &str,
    detail: &str,
    hint: Option<&str>,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1p5()
        .child(icon(ui(name), TEXT_2, 30.).mb_2())
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(rgb(MUTED))
                .child(detail.to_string()),
        )
        .when_some(hint, |s, hint| {
            s.child(
                div()
                    .mt_3()
                    .text_size(px(11.5))
                    .text_color(rgb(FAINT))
                    .child(hint.to_string()),
            )
        })
        .into_any_element()
}

fn markdown_style() -> TextViewStyle {
    let code_block = StyleRefinement::default()
        .bg(rgb(0x111111))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded_lg()
        .p_3();
    TextViewStyle::default()
        .paragraph_gap(rems(0.9))
        .heading_font_size(|level, _| {
            px(match level {
                1 => 26.,
                2 => 20.,
                3 => 17.,
                4 => 15.,
                _ => 14.,
            })
        })
        .code_block(code_block)
        .inline_code(HighlightStyle {
            background_color: Some(rgb(0x1c1c1c).into()),
            color: Some(rgb(0xe6c07b).into()),
            ..Default::default()
        })
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.),
        b => format!("{b} bytes"),
    }
}

fn relative_size(value: f32) -> DefiniteLength {
    relative(value)
}
