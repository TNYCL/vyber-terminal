//! The Edit project dialog, laid out like Codex's: a name, the source folders
//! with the primary one marked, Add folder, and Remove local project.
use crate::{
    project::{self, Project},
    theme::{self, icon, ui},
    workspace,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Sizable,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use std::path::{Path, PathBuf};

// Colors sampled from the Codex dialog.
const CARD: u32 = 0x2a2a2a;
const FIELD: u32 = 0x2d2d2d;
const EDGE: u32 = 0x3d3d3d;
const LINE: u32 = 0x393939;
const FOCUS: u32 = 0x3a83f7;
const REMOVE_BG: u32 = 0x413131;
const REMOVE_TEXT: u32 = 0xff6764;
const QUIET: u32 = 0x898989;

pub enum ProjectDialogEvent {
    /// Closed; `true` when the project list changed.
    Close(bool),
}

pub struct ProjectDialog {
    id: Option<String>,
    name: Entity<InputState>,
    folders: Vec<PathBuf>,
    /// Where the folders came from, shown under the title.
    source: Option<String>,
    error: Option<String>,
    confirm_remove: bool,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<ProjectDialogEvent> for ProjectDialog {}

impl ProjectDialog {
    /// Opens the project of `root`, or proposes one: the repository around
    /// `root` with the repositories inside it, or the matching Codex project.
    pub fn new(root: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let existing = project::for_path(root);
        let (id, name, folders, source) = match existing {
            Some(p) => (Some(p.id), p.name, p.folders, None),
            None => {
                let primary = workspace::repository_root(root)
                    .filter(|r| !project::is_linked_worktree(r))
                    .unwrap_or_else(|| root.to_path_buf());
                let codex = project::codex_projects().into_iter().find(|p| {
                    p.primary()
                        .is_some_and(|f| crate::git::same_path(f, &primary))
                });
                match codex {
                    Some(p) => (None, p.name, p.folders, Some("Folders from your Codex project".into())),
                    None => {
                        let mut folders = vec![primary.clone()];
                        folders.extend(project::discover(&primary));
                        let source = (folders.len() > 1).then(|| {
                            format!("{} repositories found inside", folders.len() - 1)
                        });
                        (None, project::folder_name(&primary), folders, source)
                    }
                }
            }
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Project name")
                .default_value(name)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save(cx);
            }
        });
        Self {
            id,
            name: input,
            folders,
            source,
            error: None,
            confirm_remove: false,
            focus: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    fn add_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add to project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update(cx, |this, cx| {
                    for path in paths {
                        if !this.folders.iter().any(|f| crate::git::same_path(f, &path)) {
                            this.folders.push(path);
                        }
                    }
                    this.error = None;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("Give the project a name".into());
            cx.notify();
            return;
        }
        if self.folders.is_empty() {
            self.error = Some("Add at least one folder".into());
            cx.notify();
            return;
        }
        if let Some(missing) = self.folders.iter().find(|f| !f.is_dir()) {
            self.error = Some(format!("{} doesn't exist anymore", missing.display()));
            cx.notify();
            return;
        }
        let project = Project {
            id: self.id.clone().unwrap_or_else(project::new_id),
            name,
            folders: self.folders.clone(),
        };
        match project::upsert(project) {
            Ok(()) => cx.emit(ProjectDialogEvent::Close(true)),
            Err(e) => {
                self.error = Some(format!("Couldn't save: {e}"));
                cx.notify();
            }
        }
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        if !self.confirm_remove {
            self.confirm_remove = true;
            cx.notify();
            return;
        }
        match project::remove(&id) {
            Ok(()) => cx.emit(ProjectDialogEvent::Close(true)),
            Err(e) => {
                self.error = Some(format!("Couldn't remove: {e}"));
                cx.notify();
            }
        }
    }

    fn folder_row(&self, index: usize, path: &Path, cx: &mut Context<Self>) -> impl IntoElement {
        let primary = index == 0;
        let group: SharedString = format!("project-folder-{index}").into();
        let tooltip: SharedString = path.display().to_string().into();
        let repository = project::is_repository(path);
        div()
            .id(("project-folder", index))
            .group(group.clone())
            .h(px(48.))
            .flex()
            .items_center()
            .gap_3()
            .pl(px(14.))
            .pr(px(12.))
            .when(index > 0, |s| s.border_t_1().border_color(rgb(LINE)))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(icon(ui(if repository { "folder-git-2" } else { "folder" }), 0xb0b0b0, 16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.5))
                    .text_color(rgb(0xf0f0f0))
                    .child(project::folder_name(path)),
            )
            .when(primary, |s| {
                s.child(
                    div()
                        .h(px(26.))
                        .px_2p5()
                        .flex()
                        .items_center()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x3e3e3e))
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0xb8b8b8))
                        .child("Primary"),
                )
            })
            .when(!primary, |s| {
                s.child(
                    div()
                        .id(("project-make-primary", index))
                        .h(px(26.))
                        .px_2p5()
                        .flex()
                        .items_center()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x3e3e3e))
                        .text_size(px(12.))
                        .text_color(rgb(0xb8b8b8))
                        .opacity(0.)
                        .group_hover(group.clone(), |s| s.opacity(1.))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x353535)))
                        .child("Set as primary")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let folder = this.folders.remove(index);
                            this.folders.insert(0, folder);
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .id(("project-remove-folder", index))
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x353535)))
                    .child(icon(ui("x"), 0x888888, 14.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.folders.remove(index);
                        cx.notify();
                    })),
            )
    }
}

impl Render for ProjectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.name.read(cx).focus_handle(cx).is_focused(window);
        let rows = self
            .folders
            .clone()
            .iter()
            .enumerate()
            .map(|(i, path)| self.folder_row(i, path, cx).into_any_element())
            .collect::<Vec<_>>();
        let existing = self.id.is_some();
        div()
            .id("project-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x0000008c))
            .flex()
            .items_center()
            .justify_center()
            .font_family(theme::ui_font())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(ProjectDialogEvent::Close(false))),
            )
            .child(
                div()
                    .id("project-dialog")
                    .track_focus(&self.focus)
                    .w(px(520.))
                    .max_h(relative(0.9))
                    .flex()
                    .flex_col()
                    .px(px(21.))
                    .pt(px(20.))
                    .pb(px(19.))
                    .rounded(px(16.))
                    .bg(rgb(CARD))
                    .border_1()
                    .border_color(rgb(0x363636))
                    .shadow_xl()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            cx.emit(ProjectDialogEvent::Close(false));
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xffffff))
                                    .child(if existing { "Edit project" } else { "New project" }),
                            )
                            .child(
                                div()
                                    .id("project-close")
                                    .size(px(26.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x353535)))
                                    .child(icon(ui("x"), 0xb0b0b0, 15.))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(ProjectDialogEvent::Close(false))
                                    })),
                            ),
                    )
                    .when_some(self.source.clone(), |s, source| {
                        s.child(
                            div()
                                .mt_1()
                                .text_size(px(12.))
                                .text_color(rgb(QUIET))
                                .child(source),
                        )
                    })
                    .child(
                        div()
                            .mt(px(16.))
                            .h(px(40.))
                            .flex()
                            .items_center()
                            .rounded(px(10.))
                            .bg(rgb(FIELD))
                            .border_1()
                            .border_color(rgb(if focused { FOCUS } else { EDGE }))
                            .child(
                                div()
                                    .w(px(40.))
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .border_r_1()
                                    .border_color(rgb(if focused { 0x2f4f80 } else { EDGE }))
                                    .child(icon(ui("folder"), 0xb0b0b0, 16.)),
                            )
                            .child(
                                div().flex_1().min_w_0().px_2().child(
                                    Input::new(&self.name)
                                        .appearance(false)
                                        .small()
                                        .text_size(px(14.)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(22.))
                            .mb(px(10.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xf5f5f5))
                            .child("Source folders"),
                    )
                    .child(
                        div()
                            .id("project-folders")
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .overflow_y_scroll()
                            .rounded(px(10.))
                            .bg(rgb(FIELD))
                            .border_1()
                            .border_color(rgb(EDGE))
                            .children(rows)
                            .child(
                                div()
                                    .id("project-add-folder")
                                    .h(px(48.))
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap_3()
                                    .pl(px(14.))
                                    .when(!self.folders.is_empty(), |s| {
                                        s.border_t_1().border_color(rgb(LINE))
                                    })
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x323232)))
                                    .child(icon(ui("folder-plus"), 0xb0b0b0, 16.))
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .text_color(rgb(0xf0f0f0))
                                            .child("Add folder"),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_folders(window, cx)
                                    })),
                            ),
                    )
                    .when_some(self.error.clone(), |s, error| {
                        s.child(
                            div()
                                .mt_2()
                                .text_size(px(12.))
                                .text_color(rgb(REMOVE_TEXT))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .mt(px(22.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(existing, |s| {
                                s.child(
                                    div()
                                        .id("project-remove")
                                        .h(px(32.))
                                        .px_4()
                                        .flex()
                                        .items_center()
                                        .rounded(px(8.))
                                        .bg(rgb(REMOVE_BG))
                                        .text_size(px(13.5))
                                        .text_color(rgb(REMOVE_TEXT))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgb(0x4d3434)))
                                        .tooltip(|window, cx| {
                                            Tooltip::new("Only Vyber forgets the project; no folder is touched")
                                                .build(window, cx)
                                        })
                                        .child(if self.confirm_remove {
                                            "Click again to remove"
                                        } else {
                                            "Remove local project"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| this.remove(cx))),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("project-cancel")
                                    .h(px(32.))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.))
                                    .text_size(px(13.5))
                                    .text_color(rgb(QUIET))
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(rgb(0xd0d0d0)))
                                    .child("Cancel")
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(ProjectDialogEvent::Close(false))
                                    })),
                            )
                            .child(
                                div()
                                    .id("project-save")
                                    .h(px(32.))
                                    .px(px(15.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.))
                                    .bg(rgb(0xffffff))
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0x111111))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0xe8e8e8)))
                                    .child("Save")
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                            ),
                    ),
            )
    }
}
