//! Tab strip, breadcrumb bar and the document area (editor, Markdown,
//! images, unsupported files and the empty state).
use super::{Browser, Kind, Menu, Message, View};
use crate::theme::observe;
use crate::{icons, platform, theme::*, workspace};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    input::Editor,
    text::{TextView, TextViewStyle},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static FOLDER_GENERATION: AtomicU64 = AtomicU64::new(0);

pub(super) struct FolderMenu {
    generation: u64,
    path: PathBuf,
    entries: Vec<workspace::FileEntry>,
    errors: Vec<String>,
    loading: bool,
}

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
                    .max_w(rpx(220.))
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
                            .size(rpx(18.))
                            .flex_shrink_0()
                            .rounded(rpx(4.))
                            .hover(|s| s.bg(rgb(SELECTED)))
                            .when(dirty, |s| {
                                s.child(div().size(rpx(7.)).rounded_full().bg(rgb(TEXT_2)))
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
                        this.select_document(i, cx);
                    }))
            })
            .collect::<Vec<_>>();
        div()
            .id("file-tabs-strip")
            .h(rpx(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .overflow_x_scroll()
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
                                .rounded(rpx(4.))
                                .bg(rgb(SELECTED))
                                .text_size(rpx(10.5))
                                .text_color(rgb(TEXT_2))
                                .child(change_count.to_string()),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.show_review(cx))),
            )
            .when(!self.docs.is_empty(), |s| {
                s.child(div().w(px(1.)).h(rpx(18.)).mx_0p5().bg(rgb(BORDER)))
            })
            .child(
                div()
                    .id("doc-tabs")
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
            .child(
                icon_button("new-doc", "plus", "Find file  (Ctrl+Shift+P)")
                    .on_click(cx.listener(|this, _, window, cx| this.focus_filter(window, cx))),
            )
            .child(self.dock_button(cx))
            .child({
                let open = self.sidebar_open();
                let label = match (self.view, open) {
                    (View::Review, true) => "Hide changed files",
                    (View::Review, false) => "Show changed files",
                    (_, true) => "Hide file tree",
                    (_, false) => "Show file tree",
                };
                icon_button("sidebar", "panel-right", label)
                    .when(open, |s| s.bg(rgb(SELECTED)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
            })
            .child(
                icon_button(
                    "panel-width",
                    if self.wide {
                        "minimize-2"
                    } else {
                        "maximize-2"
                    },
                    if self.wide {
                        "Restore panel"
                    } else {
                        "Expand panel"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.wide = !this.wide;
                    this.layout_changed(cx);
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
                icon_button("close-panel", "x", "Close panel  (Ctrl+Shift+B)")
                    .on_click(cx.listener(|this, _, _, cx| this.close_panel(cx))),
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
                            .text_size(rpx(14.))
                            .line_height(rems(1.6))
                            .text_color(rgb(0xd6d6d6))
                            .child(
                                TextView::markdown(("markdown-text", self.active_doc), source)
                                    .style(markdown_style())
                                    .selectable(true),
                            ),
                    )
                    .child(div().absolute().top_3().right_4().child(
                        icon_button("copy-markdown", "copy", "Copy Markdown").on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.to_string()));
                                this.say("Markdown copied");
                                cx.notify();
                            }),
                        ),
                    ))
                    .into_any_element()
            }
            Kind::Text => div()
                .size_full()
                .child(
                    Editor::new(&doc.editor)
                        .appearance(false)
                        .bordered(false)
                        .text_size(rpx(13.))
                        .h_full(),
                )
                .into_any_element(),
        };
        let _ = window;
        div()
            .id("document-content")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(self.breadcrumb(&path, &kind, markdown, preview, dirty, cx))
            .when(conflict, |s| {
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(rpx(34.))
                        .px_3()
                        .bg(rgb(WARNING_BG))
                        .text_color(rgb(WARNING))
                        .text_size(rpx(12.))
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
                s.child(menu_in(
                    "open-menu-in",
                    38.,
                    self.open_menu(path.clone(), cx).into_any_element(),
                ))
            })
            .when(matches!(self.menu, Some(Menu::Folder(_))), |s| {
                s.child(menu_in("folder-menu-in", 38., self.folder_menu(cx)))
            })
            .map(observe)
            .into_any_element()
    }

    fn breadcrumb(
        &self,
        path: &Path,
        kind: &Kind,
        markdown: bool,
        preview: bool,
        dirty: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut crumbs: Vec<AnyElement> = vec![];
        let mut crumb = |id: usize, label: String, folder: Option<PathBuf>, last: bool| {
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
                    .rounded(rpx(4.))
                    .text_color(rgb(if last { TEXT } else { MUTED }))
                    .when(last, |s| s.font_weight(FontWeight::SEMIBOLD))
                    .when(!last, |s| {
                        s.cursor_pointer()
                            .hover(|s| s.text_color(rgb(TEXT_2)).bg(rgb(HOVER)))
                    })
                    .child(label)
                    .when_some(folder, |s, folder| {
                        s.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_folder_menu(id, folder.clone(), cx);
                        }))
                    })
                    .map(observe)
                    .into_any_element(),
            );
        };
        let items = breadcrumb_items(&self.root, path);
        for (i, (label, folder)) in items.into_iter().enumerate() {
            let last = folder.is_none();
            crumb(i, label, folder, last);
        }
        let copy_path = self.relative(path);
        div()
            .id("document-toolbar")
            .h(rpx(36.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pl_3()
            .pr_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .text_size(rpx(12.5))
            .overflow_x_scroll()
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
                s.child(
                    icon_button("zoom-out", "zoom-out", "Zoom out").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.zoom = (this.zoom / 1.25).max(0.25);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    div()
                        .w(rpx(40.))
                        .text_center()
                        .text_size(rpx(11.5))
                        .text_color(rgb(MUTED))
                        .child(format!("{:.0}%", self.zoom * 100.)),
                )
                .child(
                    icon_button("zoom-in", "zoom-in", "Zoom in").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.zoom = (this.zoom * 1.25).min(8.);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    icon_button("zoom-fit", "scan", "Fit to panel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.zoom = 1.;
                            cx.notify();
                        },
                    )),
                )
            })
            .when(markdown, |s| {
                s.child(
                    icon_button(
                        "view-source",
                        if preview { "code" } else { "book-open" },
                        if preview { "View source" } else { "Preview" },
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(doc) = this.docs.get_mut(this.active_doc) {
                            doc.preview = !doc.preview;
                        }
                        this.focus_shown_tab(window, cx);
                        cx.notify();
                    }))
                    .map(observe),
                )
            })
            .when(dirty, |s| {
                s.child(
                    icon_button("save", "save", "Save  (Ctrl+S)")
                        .text_color(rgb(TEXT))
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                        .map(observe),
                )
            })
            .child(
                icon_button("copy-path", "copy", "Copy relative path")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()));
                        this.say(format!("Copied {copy_path}"));
                        cx.notify();
                    }))
                    .map(observe),
            )
            .child(
                icon_button("open-with", "external-link", "Open file with…")
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_menu(Menu::Open);
                        cx.notify();
                    }))
                    .map(observe),
            )
            .map(observe)
    }

    fn open_menu(&self, path: std::path::PathBuf, cx: &mut Context<Self>) -> impl IntoElement {
        let reveal_label = if cfg!(target_os = "macos") {
            "Reveal in Finder"
        } else if cfg!(windows) {
            "Reveal in File Explorer"
        } else {
            "Open containing folder"
        };
        type OpenAction = fn(&std::path::Path) -> std::io::Result<()>;
        let items: [(&str, &str, OpenAction); 3] = [
            (
                "app-window",
                "Open with default app",
                platform::open_default,
            ),
            ("folder-search", reveal_label, platform::reveal),
            ("code", "Open in VS Code", platform::open_in_code),
        ];
        div()
            .id("open-menu")
            .absolute()
            .occlude()
            .top_0()
            .right_0()
            .w(rpx(230.))
            .max_w(relative(1.))
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
            .children(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, label, run))| {
                        let path = path.clone();
                        div()
                            .id(("open-option", i))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(rpx(30.))
                            .flex_shrink_0()
                            .px_2()
                            .rounded_md()
                            .text_size(rpx(12.5))
                            .text_color(rgb(TEXT))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(HOVER)))
                            .child(icon(ui(name), TEXT_2, 14.))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.launch(run, path.clone());
                                cx.notify();
                            }))
                            .map(observe)
                    }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_menu();
                cx.notify();
            }))
            .map(observe)
    }

    fn toggle_folder_menu(&mut self, id: usize, path: PathBuf, cx: &mut Context<Self>) {
        let menu = Menu::Folder(id);
        let same =
            self.menu == Some(menu) && self.folder_menu.as_ref().is_some_and(|m| m.path == path);
        if same {
            self.dismiss_menu();
        } else {
            self.toggle_menu(menu);
            if self.menu == Some(menu) {
                self.load_folder_menu(path);
            }
        }
        cx.notify();
    }

    fn load_folder_menu(&mut self, path: PathBuf) {
        let generation = FOLDER_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        self.folder_menu = Some(FolderMenu {
            generation,
            path: path.clone(),
            entries: vec![],
            errors: vec![],
            loading: true,
        });
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let listing = workspace::scan_files_with(&path, &[]);
            let _ = sender.send(Message::FolderListed(generation, path, listing));
        });
    }

    pub(super) fn folder_listed(
        &mut self,
        generation: u64,
        path: PathBuf,
        listing: workspace::FileListing,
        cx: &mut Context<Self>,
    ) {
        if let Some(menu) = &mut self.folder_menu
            && menu.generation == generation
            && menu.path == path
        {
            menu.entries = listing.entries;
            menu.errors = listing.errors;
            menu.loading = false;
            cx.notify();
        }
    }

    fn folder_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = &self.folder_menu else {
            return div().into_any_element();
        };
        let path = menu.path.clone();
        let refresh = path.clone();
        let parent = path.parent().map(Path::to_path_buf);
        let name = platform::folder_name(&path);
        let count = menu.entries.len();
        let message = if menu.loading {
            Some("Reading folder…".to_owned())
        } else if !menu.errors.is_empty() {
            Some(menu.errors.join("\n"))
        } else if count == 0 {
            Some("This folder is empty".to_owned())
        } else {
            None
        };
        div()
            .id("folder-menu")
            .absolute()
            .occlude()
            .top_0()
            .left_0()
            .w(rpx(300.))
            .max_w(relative(1.))
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
            .child(
                div()
                    .h(rpx(30.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_button("folder-up", "arrow-up", "Parent folder")
                            .when_some(parent, |s, parent| {
                                s.on_click(cx.listener(move |this, _, _, cx| {
                                    this.load_folder_menu(parent.clone());
                                    cx.notify();
                                }))
                            })
                            .map(observe),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .child(
                        icon_button("folder-refresh", "refresh-cw", "Refresh folder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_folder_menu(refresh.clone());
                                cx.notify();
                            }))
                            .map(observe),
                    ),
            )
            .when_some(message, |s, message| {
                s.child(
                    div()
                        .px_2()
                        .py_1()
                        .flex_shrink_0()
                        .text_size(rpx(12.))
                        .text_color(rgb(MUTED))
                        .child(message),
                )
            })
            .when(count > 0, |s| {
                s.child(
                    uniform_list(
                        "folder-menu-entries",
                        count,
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            let Some(menu) = &this.folder_menu else {
                                return vec![];
                            };
                            range
                                .filter_map(|i| {
                                    menu.entries
                                        .get(i)
                                        .map(|entry| this.folder_option(i, entry, cx))
                                })
                                .collect()
                        }),
                    )
                    .h(rpx(count.min(10) as f32 * 30.))
                    .min_h_0(),
                )
            })
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_menu();
                cx.notify();
            }))
            .map(observe)
            .into_any_element()
    }

    fn folder_option(
        &self,
        id: usize,
        entry: &workspace::FileEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = entry.path.clone();
        let directory = entry.directory;
        let name = platform::folder_name(&path);
        let (icon_path, color) = if directory {
            (ui("folder"), TEXT_2)
        } else {
            icons::file_icon(&path)
        };
        div()
            .id(("folder-option", id))
            .h(rpx(30.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .text_size(rpx(12.5))
            .text_color(rgb(TEXT))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(HOVER)))
            .child(icon(icon_path, color, 14.))
            .child(div().flex_1().min_w_0().truncate().child(name))
            .when(directory, |s| {
                s.child(icon(ui("chevron-right"), MUTED, 12.))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if directory {
                    this.load_folder_menu(path.clone());
                } else {
                    this.select_tree_root(None, window, cx);
                    this.open(path.clone(), true, cx);
                }
                cx.notify();
            }))
            .map(observe)
            .into_any_element()
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
                    .text_size(rpx(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child(name),
            )
            .child(
                div()
                    .text_size(rpx(12.5))
                    .text_color(rgb(MUTED))
                    .child(format!("{reason} · {}", human_size(size))),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .gap_2()
                    .child(
                        text_button(
                            "unsupported-open",
                            Some("app-window"),
                            "Open with default app",
                        )
                        .border_1()
                        .border_color(rgb(BORDER))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.launch(platform::open_default, open.clone());
                            cx.notify();
                        })),
                    )
                    .child(
                        text_button(
                            "unsupported-reveal",
                            Some("folder-search"),
                            "Show in folder",
                        )
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

/// Workspace files begin with the workspace's name. External files use their
/// real drive/root and ancestors, so every folder crumb points to its own path.
fn breadcrumb_items(root: &Path, path: &Path) -> Vec<(String, Option<PathBuf>)> {
    if let Some(relative) = workspace::relative_folder(root, path) {
        let parts: Vec<_> = relative.components().collect();
        let mut folder = root.to_path_buf();
        let mut items = vec![(
            platform::folder_name(root),
            (!parts.is_empty()).then(|| folder.clone()),
        )];
        for (i, part) in parts.iter().enumerate() {
            folder.push(part.as_os_str());
            items.push((
                part.as_os_str().to_string_lossy().to_string(),
                (i + 1 < parts.len()).then(|| folder.clone()),
            ));
        }
        return items;
    }
    let mut ancestors: Vec<_> = path.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();
    ancestors
        .into_iter()
        .map(|folder| {
            let label = platform::folder_name(&folder);
            let target = (folder != path).then_some(folder);
            (label, target)
        })
        .collect()
}

fn tab(id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap_1p5()
        .h(rpx(28.))
        .pl_2p5()
        .pr_1()
        .rounded_lg()
        .border_1()
        .text_size(rpx(12.5))
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

pub(super) fn empty_state(name: &str, title: &str, detail: &str, hint: Option<&str>) -> AnyElement {
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
                .text_size(rpx(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(rpx(12.5))
                .text_color(rgb(MUTED))
                .child(detail.to_string()),
        )
        .when_some(hint, |s, hint| {
            s.child(
                div()
                    .mt_3()
                    .text_size(rpx(11.5))
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
        .heading_font_size(|level, rem| {
            rem * (match level {
                1 => 26.,
                2 => 20.,
                3 => 17.,
                4 => 15.,
                _ => 14.,
            } / REM)
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

#[cfg(test)]
pub(super) mod tests {
    use super::{Browser, Menu, PathBuf, View, breadcrumb_items};
    use crate::{changeset::Source, config::Config, theme, workspace};
    use gpui::{
        AnyWindowHandle, AppContext, Bounds, Entity, Point, ScrollDelta, TestAppContext,
        WindowBounds, WindowOptions, point, px, size,
    };
    use gpui_kit::test::TestWindowExt;

    pub(in crate::browser) fn window(
        root: PathBuf,
        path: PathBuf,
        dirty: bool,
        height: f32,
        review: bool,
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, Entity<Browser>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Config::default());
            theme::apply(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(288.), px(height)),
                    })),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    let browser = cx.new(|cx| Browser::new(root, window, cx));
                    browser.update(cx, |b, cx| {
                        b.visible = true;
                        b.sidebar_hidden = [true; 3];
                        b.loaded(path, Ok(b"# Original\n".to_vec()), 11, true, window, cx);
                        if dirty {
                            b.docs[0]
                                .editor
                                .update(cx, |ed, cx| ed.set_value("# Changed\n", window, cx));
                            b.docs[0].dirty = true;
                        }
                        if review {
                            b.view = View::Review;
                            b.review.source = Some(Source::Uncommitted);
                        }
                    });
                    browser
                },
            )
            .unwrap()
        })
    }

    fn deliver_folder(browser: &Entity<Browser>, cx: &mut gpui::App) {
        let (generation, path) = {
            let b = browser.read(cx);
            let menu = b.folder_menu.as_ref().unwrap();
            (menu.generation, menu.path.clone())
        };
        let listing = workspace::scan_files_with(&path, &[]);
        browser.update(cx, |b, cx| b.folder_listed(generation, path, listing, cx));
    }

    #[test]
    fn external_breadcrumbs_use_their_real_ancestors() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let path = temp.path().join("elsewhere").join("src").join("main.rs");
        let items = breadcrumb_items(&root, &path);
        assert!(
            items
                .iter()
                .all(|(_, target)| target.as_deref() != Some(root.as_path()))
        );
        assert!(items.iter().any(|(name, target)| {
            name == "elsewhere"
                && target.as_deref() == Some(temp.path().join("elsewhere").as_path())
        }));
        assert_eq!(items.last(), Some(&("main.rs".into(), None)));
        let nested = root.join("src").join("main.rs");
        let items = breadcrumb_items(&root, &nested);
        assert_eq!(items[0], ("workspace".into(), Some(root)));
        assert_eq!(items.len(), 3);
    }

    #[gpui_kit::test]
    fn open_menu_stays_inside_a_short_document_and_dirty_actions_remain_clickable(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("readme.md");
        std::fs::write(&path, "# Original\n").unwrap();
        let (handle, browser) = window(temp.path().to_owned(), path, true, 180., false, cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let toolbar = window.find("document-toolbar").bounds();
            for id in ["view-source", "save", "copy-path", "open-with"] {
                let button = window.find(id);
                assert!(button.visible());
                assert!(button.bounds().left() >= toolbar.left());
                assert!(button.bounds().right() <= toolbar.right());
            }
            window.click("open-with", cx);
            assert!(browser.read(cx).menu == Some(Menu::Open));
            let menu = window.find("open-menu").bounds();
            let content = window.find("document-content").bounds();
            assert!(menu.top() >= toolbar.bottom());
            assert!(menu.bottom() <= content.bottom());
            assert!(menu.left() >= content.left() && menu.right() <= content.right());
            window.scroll(
                "open-menu",
                ScrollDelta::Pixels(point(px(0.), px(-200.))),
                cx,
            );
            let last = window.find(("open-option", 2usize));
            assert!(last.visible());
            assert!(last.bounds().bottom() <= content.bottom());
            window.click("open-with", cx);
            assert!(browser.read(cx).menu.is_none());
            window.click("copy-path", cx);
            assert!(browser.read(cx).notice.starts_with("Copied "));
            window.click("save", cx);
            assert!(browser.read(cx).notice.starts_with("Saving"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn breadcrumb_dropdown_navigates_hidden_directories_and_opens_the_selected_file(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let hidden = root.join(".hidden");
        std::fs::create_dir_all(&hidden).unwrap();
        let selected = hidden.join("child.rs");
        std::fs::write(&selected, "fn main() {}\n").unwrap();
        let path = root.join("readme.md");
        std::fs::write(&path, "# Original\n").unwrap();
        let (handle, browser) = window(root.clone(), path, false, 240., false, cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("crumb", 0usize), cx);
            assert!(browser.read(cx).menu == Some(Menu::Folder(0)));
            deliver_folder(&browser, cx);
            window.render_frame(cx);
            let menu = window.find("folder-menu").bounds();
            let content = window.find("document-content").bounds();
            assert!(menu.bottom() <= content.bottom());
            let index = browser
                .read(cx)
                .folder_menu
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .position(|entry| entry.path == hidden)
                .unwrap();
            let old_generation = browser.read(cx).folder_menu.as_ref().unwrap().generation;
            window.click(("folder-option", index), cx);
            assert!(browser.read(cx).folder_menu.as_ref().unwrap().path == hidden);
            browser.update(cx, |b, cx| {
                b.folder_listed(
                    old_generation,
                    root.clone(),
                    workspace::scan_files_with(&root, &[]),
                    cx,
                );
                let menu = b.folder_menu.as_ref().unwrap();
                assert!(menu.loading && menu.entries.is_empty());
                assert!(menu.path == hidden);
            });
            deliver_folder(&browser, cx);
            window.render_frame(cx);
            window.click(("folder-option", 0usize), cx);
            assert!(browser.read(cx).menu.is_none());
            assert!(browser.read(cx).intended_document.as_ref() == Some(&selected));
        })
        .unwrap();
    }
}
