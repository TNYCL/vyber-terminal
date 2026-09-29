use crate::{
    browser::{Browser, BrowserEvent},
    layout::{self, DropEdge, Layout},
    project_dialog::{ProjectDialog, ProjectDialogEvent},
    tasks::{Monitor, TaskReview},
    terminal::{Terminal, TerminalEvent, TurnBadge},
    theme::{self, chip},
    workspace,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    TitleBar,
    resizable::{h_resizable, resizable_panel, v_resizable},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

actions!(
    vyber,
    [
        NewTerminal,
        SplitRight,
        SplitDown,
        ToggleFiles,
        SaveFile,
        QuickOpen,
        Checkpoint,
        OpenFolder,
        ClosePane,
        ZoomPane,
        SearchTerminal,
        NextPane,
        NextTab,
        PreviousTab,
        ShowShortcuts,
        CancelTabDrag,
        Quit,
        Settings,
        TerminalZoomIn,
        TerminalZoomOut,
        TerminalZoomReset,
        ToggleGit,
        EditProject
    ]
);

const MIN_FONT_SIZE: f32 = 8.;
const MAX_FONT_SIZE: f32 = 32.;

#[derive(Clone)]
struct TabDrag {
    id: usize,
    group: bool,
    label: String,
}
#[derive(Clone, Copy, PartialEq)]
enum DropHint {
    Pane { id: usize, edge: DropEdge },
    Tab { id: usize, after: bool },
    End,
}
impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        drag_label(self.label.clone())
    }
}
/// Dragging the file panel's left edge.
#[derive(Clone)]
struct PanelResize;
impl Render for PanelResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
/// The file panel never gets narrower than this.
const PANEL_MIN: f32 = 480.;
fn turn_badge(review: &TaskReview) -> TurnBadge {
    let warning = review
        .warning
        .replace(crate::tasks::BASELINE_NOTE, "")
        .trim()
        .to_string();
    TurnBadge {
        id: review.id.clone(),
        files: review.changes.len(),
        additions: review.changes.iter().map(|c| c.additions).sum(),
        deletions: review.changes.iter().map(|c| c.deletions).sum(),
        active: review.active,
        warning: (!warning.is_empty()).then_some(warning),
    }
}
fn drag_label(label: String) -> impl IntoElement {
    div()
        .px_3()
        .py_1()
        .rounded_md()
        .shadow_lg()
        .bg(rgb(0x101b2d))
        .border_1()
        .border_color(rgb(0x60a5fa))
        .text_color(rgb(0xdbeafe))
        .text_size(px(12.))
        .child(label)
}
fn bar_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(30.))
        .flex_shrink_0()
        .text_size(px(13.))
        .text_color(rgb(0x9b9b9b))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x171717)).text_color(rgb(0xffffff)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label.into())
}

struct Slot {
    terminal: Entity<Terminal>,
    browser: Entity<Browser>,
    /// Latest agent turn that ran in this terminal.
    turn: Option<TaskReview>,
    _subscriptions: Vec<Subscription>,
}
#[derive(Serialize, Deserialize, Default)]
struct SavedState {
    slots: Vec<SavedSlot>,
    tabs: Vec<Layout>,
    tab: usize,
    active: usize,
    /// Terminal font size chosen with the zoom shortcuts; `None` follows config.toml.
    #[serde(default)]
    font_size: Option<f32>,
}
#[derive(Serialize, Deserialize)]
struct SavedSlot {
    id: usize,
    root: PathBuf,
    browser: crate::browser::BrowserState,
}

pub struct Vyber {
    slots: HashMap<usize, Slot>,
    tabs: Vec<Layout>,
    tab: usize,
    active: usize,
    next: usize,
    focus: FocusHandle,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    monitor: Monitor,
    /// Agent session → the terminal it runs in, learned from its first turn.
    sessions: HashMap<String, usize>,
    notice: String,
    notifications: crate::notifications::Notifications,
    last_persist: std::time::Instant,
    zoomed: bool,
    show_shortcuts: bool,
    drop_hint: Option<DropHint>,
    font_size: Option<f32>,
    panel: Option<PanelMotion>,
    project_dialog: Option<(Entity<ProjectDialog>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

/// The file panel's slide: `shown` fades and slides it in from the right,
/// `wide` grows it to the full width. Values run from `from` to the target.
#[derive(Clone, Copy)]
struct PanelMotion {
    slot: usize,
    shown: bool,
    wide: bool,
    /// (shown, wide) as 0–1 when this motion started.
    from: (f32, f32),
    start: std::time::Instant,
}
impl PanelMotion {
    const DURATION: f32 = 0.24;
    fn settled(slot: usize, shown: bool, wide: bool) -> Self {
        Self {
            slot,
            shown,
            wide,
            from: (f32::from(u8::from(shown)), f32::from(u8::from(wide))),
            start: std::time::Instant::now() - Duration::from_secs(1),
        }
    }
    fn progress(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / Self::DURATION).min(1.)
    }
    fn running(&self) -> bool {
        self.progress() < 1.
    }
    fn value(&self) -> (f32, f32) {
        let t = theme::ease_out(self.progress());
        let to = (f32::from(u8::from(self.shown)), f32::from(u8::from(self.wide)));
        (
            self.from.0 + (to.0 - self.from.0) * t,
            self.from.1 + (to.1 - self.from.1) * t,
        )
    }
}
impl Vyber {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let roots = Arc::new(Mutex::new(vec![root.clone()]));
        let monitor = Monitor::start(roots.clone());
        let mut app = Self {
            slots: HashMap::new(),
            tabs: vec![],
            tab: 0,
            active: 0,
            next: 0,
            focus: cx.focus_handle(),
            roots,
            monitor,
            sessions: HashMap::new(),
            notice: String::new(),
            notifications: crate::notifications::Notifications::new(),
            last_persist: std::time::Instant::now(),
            zoomed: false,
            show_shortcuts: false,
            drop_hint: None,
            font_size: None,
            panel: None,
            project_dialog: None,
            _subscriptions: vec![],
        };
        let saved = if std::env::args_os().nth(1).is_none()
            && crate::config::Config::load().restore_workspace
        {
            std::fs::read(workspace::data_dir().join("workspace.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<SavedState>(&b).ok())
        } else {
            None
        };
        if let Some(saved) = saved {
            app.font_size = saved
                .font_size
                .map(|size| size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE));
            for slot in saved.slots {
                if slot.root.is_dir() {
                    app.next = slot.id;
                    app.add_terminal(slot.root, false, false, window, cx);
                    if let Some(new) = app.slots.get(&slot.id) {
                        new.browser
                            .update(cx, |b, cx| b.restore_state(slot.browser, cx));
                    }
                }
            }
            if !app.slots.is_empty() {
                let mut tabs = saved.tabs;
                let missing = (0..app.next + 1)
                    .filter(|id| !app.slots.contains_key(id))
                    .collect::<Vec<_>>();
                for id in missing {
                    tabs = tabs.into_iter().filter_map(|t| t.remove(id)).collect();
                }
                if !tabs.is_empty() {
                    app.tabs = tabs;
                }
                app.tab = saved.tab.min(app.tabs.len() - 1);
                app.active = if app.slots.contains_key(&saved.active) {
                    saved.active
                } else {
                    app.tabs[app.tab].first()
                };
                app.next = app.slots.keys().copied().max().unwrap_or(0) + 1;
            }
        }
        if app.slots.is_empty() {
            app.add_terminal(root, false, false, window, cx);
        }
        app._subscriptions
            .push(cx.on_release(|this, cx| this.persist(cx)));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(app) = weak.upgrade() {
                app.read(cx).persist(cx);
            }
            true
        });
        cx.spawn_in(window, async move |entity, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(350)).await;
                if let Err(e) = entity.update_in(cx, |this, window, cx| this.poll(window, cx)) {
                    log::warn!("Workspace poll ended: {e}");
                    break;
                }
            }
        })
        .detach();
        app
    }
    fn add_terminal(
        &mut self,
        root: PathBuf,
        split: bool,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next;
        self.next += 1;
        let backend =
            match Terminal::prepare(id, &root, crate::config::Config::load().shell.as_deref()) {
                Ok(backend) => backend,
                Err(error) => {
                    self.notice = error.to_string();
                    cx.notify();
                    return;
                }
            };
        let terminal = cx.new(|cx| Terminal::new(id, &root, backend, cx));
        if let Some(size) = self.font_size {
            terminal.update(cx, |t, cx| t.set_font_size(size, cx));
        }
        let browser = cx.new(|cx| Browser::new(root.clone(), window, cx));
        let focus = terminal.read(cx).focus.clone();
        let focused = cx.on_focus(&focus, window, move |this, _, cx| {
            this.active = id;
            for (i, layout) in this.tabs.iter().enumerate() {
                if layout.contains(id) {
                    this.tab = i;
                    break;
                }
            }
            cx.notify();
        });
        let badge = cx.subscribe(&terminal, move |this, _, event: &TerminalEvent, cx| {
            match event {
                TerminalEvent::OpenTurn(task) => this.open_turn(id, task.clone(), cx),
            }
        });
        let comments = cx.subscribe_in(
            &browser,
            window,
            move |this, _, event: &BrowserEvent, window, cx| match event {
                BrowserEvent::Comment(text) => {
                    if let Some(slot) = this.slots.get(&id) {
                        slot.terminal.update(cx, |t, _| t.insert_comment(text));
                    }
                }
                BrowserEvent::RunInTerminal(command) => {
                    if let Some(slot) = this.slots.get(&id) {
                        slot.terminal.update(cx, |t, _| t.paste(command));
                        let focus = slot.terminal.read(cx).focus.clone();
                        window.focus(&focus, cx);
                    }
                }
                BrowserEvent::OpenFolder(path) => {
                    if path.is_dir() {
                        this.add_terminal(path.clone(), false, false, window, cx);
                    } else {
                        this.notice = format!("{} doesn't exist anymore", path.display());
                        cx.notify();
                    }
                }
                BrowserEvent::EditProject(root) => this.open_project(root.clone(), window, cx),
            },
        );
        self.slots.insert(
            id,
            Slot {
                terminal,
                browser,
                turn: None,
                _subscriptions: vec![focused, badge, comments],
            },
        );
        if split && !self.tabs.is_empty() {
            self.tabs[self.tab].split(self.active, id, vertical);
        } else {
            self.tabs.push(Layout::Leaf(id));
            self.tab = self.tabs.len() - 1;
        }
        self.active = id;
        window.focus(&focus, cx);
        self.update_roots(cx);
        self.persist(cx);
        cx.notify();
    }
    fn update_roots(&mut self, cx: &App) {
        *self.roots.lock().unwrap() = self
            .slots
            .values()
            .map(|s| s.terminal.read(cx).root.clone())
            .collect();
        let slots = &self.slots;
        self.sessions.retain(|_, id| slots.contains_key(id));
    }
    /// The terminal an agent turn runs in. A session is bound on its first
    /// observed turn to the terminal in that folder where Enter was pressed
    /// last (preferring one whose typed line matches the prompt), and stays
    /// bound. Vyber does not inspect processes, so this is a best guess.
    fn turn_terminal(&mut self, review: &TaskReview, cx: &App) -> Option<usize> {
        if review.session.is_empty() {
            return None;
        }
        if let Some(id) = self.sessions.get(&review.session)
            && self.slots.contains_key(id)
        {
            return Some(*id);
        }
        if !review.active || review.after.is_some() {
            return None;
        }
        let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let prompt = normalize(review.label.split_once(" · ").map_or("", |(_, p)| p));
        let now = std::time::Instant::now();
        let (_, _, id) = self
            .slots
            .iter()
            .filter_map(|(id, slot)| {
                let terminal = slot.terminal.read(cx);
                let submitted = terminal.last_submit?;
                if now.duration_since(submitted) > Duration::from_secs(45)
                    || !crate::tasks::matches_root(&review.root, &terminal.root)
                {
                    return None;
                }
                let typed = normalize(&terminal.last_line);
                let matched = typed.len() >= 3
                    && prompt.len() >= 3
                    && (prompt.starts_with(&typed) || typed.starts_with(&prompt));
                Some((matched, submitted, *id))
            })
            .max_by_key(|(matched, submitted, _)| (*matched, *submitted))?;
        self.sessions.insert(review.session.clone(), id);
        Some(id)
    }
    fn open_turn(&mut self, id: usize, task: String, cx: &mut Context<Self>) {
        let Some(slot) = self.slots.get(&id) else {
            return;
        };
        let turn = slot.turn.clone().filter(|t| t.id == task);
        slot.browser.update(cx, |b, cx| {
            b.visible = true;
            match turn {
                Some(turn) => b.show_turn(turn, cx),
                None => b.show_review(cx),
            }
        });
        self.persist(cx);
        cx.notify();
    }
    fn current_root(&self, cx: &App) -> PathBuf {
        self.slots
            .get(&self.active)
            .map(|s| s.terminal.read(cx).root.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        for (id, slot) in &self.slots {
            let (message, path) = slot.terminal.update(cx, |t, _| {
                let msg = t.notification.take().or_else(|| {
                    if std::mem::take(&mut t.bell) {
                        Some("Terminal needs your attention".into())
                    } else {
                        None
                    }
                });
                (msg, t.open_path.take())
            });
            if let Some(message) = message {
                if !window.is_window_active() && crate::config::Config::load().notifications {
                    self.notifications.show(*id, message);
                }
            }
            if let Some((path, line)) = path {
                slot.browser
                    .update(cx, |b, cx| b.open_at(path, line, window, cx));
            }
            let root = slot.terminal.read(cx).root.clone();
            if root != slot.browser.read(cx).root {
                slot.browser.update(cx, |b, cx| b.change_root(root, cx));
                changed = true;
            }
        }
        if changed {
            self.update_roots(cx);
        }
        if !self.slots.is_empty() && self.slots.values().all(|s| s.terminal.read(cx).exited) {
            // Keep the final layout and editor drafts for the next launch, then quit normally.
            self.persist(cx);
            cx.quit();
            return;
        }
        let exited = self
            .slots
            .iter()
            .filter_map(|(id, slot)| slot.terminal.read(cx).exited.then_some(*id))
            .collect::<Vec<_>>();
        for id in exited {
            if self.slots[&id].browser.read(cx).has_dirty() {
                let notice = "Shell exited. Save your editor changes before closing this terminal.";
                if self.notice != notice {
                    self.notice = notice.into();
                    changed = true;
                }
            } else {
                self.close_pane(id, window, cx);
            }
        }
        let reviews: Vec<_> = self.monitor.receiver.try_iter().collect();
        for review in reviews {
            for slot in self.slots.values() {
                let root = &slot.browser.read(cx).root;
                let path = root.to_string_lossy().replace('\\', "/").to_lowercase();
                let task_path = review
                    .root
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase();
                if path == task_path || task_path.starts_with(&(path + "/")) {
                    let review = review.clone();
                    slot.browser.update(cx, |b, cx| b.update_task(review, cx));
                }
            }
            // A late update of an earlier turn must not replace a newer one.
            if let Some(id) = self.turn_terminal(&review, cx)
                && let Some(slot) = self.slots.get_mut(&id)
                && slot
                    .turn
                    .as_ref()
                    .is_none_or(|t| t.id == review.id || review.active)
            {
                let badge = turn_badge(&review);
                slot.turn = Some(review);
                slot.terminal
                    .update(cx, |t, cx| t.set_badge(Some(badge), cx));
            }
            changed = true;
        }
        let notifications: Vec<_> = self.notifications.receiver.try_iter().collect();
        for id in notifications {
            if let Some(slot) = self.slots.get(&id) {
                self.active = id;
                self.tab = self
                    .tabs
                    .iter()
                    .position(|t| t.contains(id))
                    .unwrap_or(self.tab);
                window.activate_window();
                let focus = slot.terminal.read(cx).focus.clone();
                window.focus(&focus, cx);
                changed = true;
            }
        }
        if self.last_persist.elapsed() > Duration::from_secs(5) {
            self.persist(cx);
            self.last_persist = std::time::Instant::now();
        }
        if changed {
            cx.notify();
        }
    }
    fn persist(&self, cx: &App) {
        let mut slots = self
            .slots
            .iter()
            .map(|(id, s)| SavedSlot {
                id: *id,
                root: s.terminal.read(cx).root.clone(),
                browser: s.browser.read(cx).state(cx),
            })
            .collect::<Vec<_>>();
        slots.sort_by_key(|s| s.id);
        let state = SavedState {
            slots,
            tabs: self.tabs.clone(),
            tab: self.tab,
            active: self.active,
            font_size: self.font_size,
        };
        if let Ok(bytes) = serde_json::to_vec(&state) {
            let dir = workspace::data_dir();
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("workspace.json");
            if std::fs::read(&path).ok().as_ref() != Some(&bytes) {
                if let Err(e) = std::fs::write(path, bytes) {
                    log::warn!("Save workspace: {e}");
                }
            }
        }
    }
    fn new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.add_terminal(self.current_root(cx), false, false, window, cx);
    }
    fn split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.add_terminal(self.current_root(cx), true, false, window, cx);
    }
    fn split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.add_terminal(self.current_root(cx), true, true, window, cx);
    }
    fn toggle_files(&mut self, _: &ToggleFiles, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get(&self.active) {
            slot.browser.update(cx, |b, cx| {
                if b.git_open() {
                    // Switching from Git keeps the panel where it is.
                    b.show_files(cx);
                } else {
                    b.visible = !b.visible;
                }
                cx.notify();
            });
        }
        self.persist(cx);
        cx.notify();
    }
    fn toggle_git(&mut self, _: &ToggleGit, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get(&self.active) {
            slot.browser.update(cx, |b, cx| {
                if b.git_open() {
                    b.visible = false;
                } else {
                    b.show_git(cx);
                }
                cx.notify();
            });
        }
        self.persist(cx);
        cx.notify();
    }
    fn edit_project(&mut self, _: &EditProject, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.current_root(cx);
        self.open_project(root, window, cx);
    }
    /// Shows the project dialog for the project of `root` (or a new one).
    fn open_project(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.show_shortcuts = false;
        let dialog = cx.new(|cx| ProjectDialog::new(&root, window, cx));
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            |this, _, event: &ProjectDialogEvent, window, cx| match event {
                ProjectDialogEvent::Close(changed) => {
                    this.project_dialog = None;
                    if *changed {
                        for slot in this.slots.values() {
                            slot.browser.update(cx, |b, cx| b.project_changed(cx));
                        }
                    }
                    if let Some(slot) = this.slots.get(&this.active) {
                        let focus = slot.terminal.read(cx).focus.clone();
                        window.focus(&focus, cx);
                    }
                    cx.notify();
                }
            },
        );
        self.project_dialog = Some((dialog, subscription));
        cx.notify();
    }
    fn save(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get(&self.active) {
            if !slot.browser.read(cx).owns_focus(window, cx) {
                cx.propagate();
                return;
            }
            slot.browser.update(cx, |b, cx| b.save(cx));
        }
    }
    fn checkpoint(&mut self, _: &Checkpoint, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get(&self.active) {
            slot.browser.update(cx, |b, cx| b.checkpoint(cx));
        }
    }
    fn quick_open(&mut self, _: &QuickOpen, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get(&self.active) {
            slot.browser.update(cx, |b, cx| b.focus_filter(window, cx));
        }
        cx.notify();
    }
    fn close(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        self.close_pane(self.active, window, cx);
    }
    fn close_pane(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|layout| layout.contains(id)) else {
            return;
        };
        if self.slots.len() == 1 {
            if self.slots.values().any(|s| s.browser.read(cx).has_dirty()) {
                self.notice = "Save your editor changes before closing.".into();
                cx.notify();
                return;
            }
            self.persist(cx);
            cx.quit();
            return;
        }
        if self
            .slots
            .get(&id)
            .is_some_and(|s| s.browser.read(cx).has_dirty())
        {
            self.notice = "Save your editor changes before closing this terminal.".into();
            cx.notify();
            return;
        }
        if let Some(layout) = self.tabs[index].remove(id) {
            self.tabs[index] = layout;
        } else {
            self.tabs.remove(index);
        }
        self.slots.remove(&id);
        self.tab = self
            .tabs
            .iter()
            .position(|layout| layout.contains(self.active))
            .unwrap_or_else(|| index.min(self.tabs.len() - 1));
        if self.active == id {
            self.active = self.tabs[self.tab].first();
            self.zoomed = false;
            if let Some(slot) = self.slots.get(&self.active) {
                let focus = slot.terminal.read(cx).focus.clone();
                window.focus(&focus, cx);
            }
        }
        self.update_roots(cx);
        self.persist(cx);
        cx.notify();
    }
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.tabs.get(index).cloned() else {
            return;
        };
        let ids = self
            .slots
            .keys()
            .copied()
            .filter(|id| layout.contains(*id))
            .collect::<Vec<_>>();
        if ids
            .iter()
            .any(|id| self.slots[id].browser.read(cx).has_dirty())
        {
            self.notice = "Save your editor changes before closing this tab.".into();
            cx.notify();
            return;
        }
        if self.tabs.len() == 1 {
            self.persist(cx);
            cx.quit();
            return;
        }
        for id in ids {
            self.slots.remove(&id);
        }
        self.tabs.remove(index);
        self.tab = self.tab.min(self.tabs.len() - 1);
        self.active = self.tabs[self.tab].first();
        self.zoomed = false;
        let focus = self.slots[&self.active].terminal.read(cx).focus.clone();
        window.focus(&focus, cx);
        self.update_roots(cx);
        self.persist(cx);
        cx.notify();
    }
    fn zoom(&mut self, _: &ZoomPane, _: &mut Window, cx: &mut Context<Self>) {
        self.zoomed = !self.zoomed;
        cx.notify();
    }
    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.persist(cx);
        cx.quit();
    }
    fn settings(&mut self, _: &Settings, _: &mut Window, cx: &mut Context<Self>) {
        crate::config::Config::load();
        if let Some(slot) = self.slots.get(&self.active) {
            slot.browser.update(cx, |b, cx| {
                b.visible = true;
                b.open(crate::config::Config::path(), true, cx);
            });
        }
        cx.notify();
    }
    fn set_font_size(&mut self, size: Option<f32>, cx: &mut Context<Self>) {
        self.font_size = size.map(|size| size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE));
        let size = self
            .font_size
            .unwrap_or_else(|| crate::config::Config::load().font_size);
        for slot in self.slots.values() {
            slot.terminal.update(cx, |t, cx| t.set_font_size(size, cx));
        }
        self.persist(cx);
        cx.notify();
    }
    fn current_font_size(&self, cx: &App) -> f32 {
        self.slots
            .get(&self.active)
            .map(|s| s.terminal.read(cx).font_size())
            .unwrap_or_else(|| crate::config::Config::load().font_size)
    }
    fn zoom_in(&mut self, _: &TerminalZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        let size = self.current_font_size(cx) + 1.;
        self.set_font_size(Some(size), cx);
    }
    fn zoom_out(&mut self, _: &TerminalZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        let size = self.current_font_size(cx) - 1.;
        self.set_font_size(Some(size), cx);
    }
    fn zoom_reset(&mut self, _: &TerminalZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(None, cx);
    }
    fn shortcuts(&mut self, _: &ShowShortcuts, _: &mut Window, cx: &mut Context<Self>) {
        self.show_shortcuts = !self.show_shortcuts;
        cx.notify();
    }
    fn search_terminal(&mut self, _: &SearchTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.slots.get(&self.active) {
            s.terminal.update(cx, |t, cx| t.toggle_search(window, cx));
        }
    }
    fn next_pane(&mut self, _: &NextPane, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.tabs[self.tab].leaves();
        if let Some(i) = ids.iter().position(|id| *id == self.active) {
            self.active = ids[(i + 1) % ids.len()];
            let focus = self.slots[&self.active].terminal.read(cx).focus.clone();
            window.focus(&focus, cx);
            cx.notify();
        }
    }
    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_tab((self.tab + 1) % self.tabs.len(), window, cx);
    }
    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_tab(
            (self.tab + self.tabs.len() - 1) % self.tabs.len(),
            window,
            cx,
        );
    }
    fn switch_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = index;
        self.active = self.tabs[index].first();
        self.zoomed = false;
        let focus = self.slots[&self.active].terminal.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }
    fn open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open in Vyber".into()),
        });
        cx.spawn_in(window, async move |entity, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let path = path.clone();
                let _ = entity.update_in(cx, |this, window, cx| {
                    this.add_terminal(path.clone(), false, false, window, cx);
                    // A folder of repositories is worth a project; offer one.
                    if crate::project::for_path(&path).is_none()
                        && !crate::project::discover(&path).is_empty()
                    {
                        this.open_project(path, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn focus_pane(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().position(|t| t.contains(id)) {
            self.tab = tab;
            self.active = id;
            if let Some(slot) = self.slots.get(&id) {
                window.focus(&slot.terminal.read(cx).focus.clone(), cx);
            }
            cx.notify();
        }
    }
    fn finish_move(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_hint = None;
        self.zoomed = false;
        self.focus_pane(id, window, cx);
        self.persist(cx);
    }
    fn drop_on_tab(
        &mut self,
        drag: &TabDrag,
        target: Option<usize>,
        after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let moved = if drag.group {
            layout::reorder_group(&mut self.tabs, drag.id, target, after)
        } else if let Some(target) = target {
            layout::reorder_pane(&mut self.tabs, drag.id, target, after)
        } else {
            layout::move_pane_to_tab(&mut self.tabs, drag.id, None, true)
        };
        if moved {
            self.finish_move(drag.id, window, cx);
        }
    }
    fn tab_drop_zone(&self, id: usize, grip: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let marker = match self.drop_hint {
            Some(DropHint::Tab { id: target, after }) if target == id => Some(after),
            _ => None,
        };
        div()
            .id((
                if grip {
                    "group-grip-drop"
                } else {
                    "tab-drop-zone"
                },
                id,
            ))
            .relative()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                // GPUI sends these capture events to every registered drop zone.
                if !e.bounds.contains(&e.event.position) {
                    return;
                }
                let drag = e.drag(cx);
                if drag.id == id
                    || (drag.group
                        && this
                            .tabs
                            .iter()
                            .any(|t| t.contains(drag.id) && t.contains(id)))
                {
                    return;
                }
                let after = e.event.position.x > e.bounds.center().x;
                this.drop_hint = Some(DropHint::Tab { id, after });
                cx.notify();
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                if let Some(DropHint::Tab { id: target, after }) = this.drop_hint {
                    if target == id {
                        this.drop_on_tab(drag, Some(id), after, window, cx);
                    }
                }
            }))
            .when(cx.has_active_drag(), |s| {
                s.when_some(marker, |s, after| {
                    s.child(
                        div()
                            .absolute()
                            .top(px(3.))
                            .bottom(px(3.))
                            .w(px(2.))
                            .rounded_sm()
                            .bg(rgb(0x60a5fa))
                            .when(after, |s| s.right_0())
                            .when(!after, |s| s.left_0()),
                    )
                })
            })
    }
    fn pane(&self, id: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(slot) = self.slots.get(&id) else {
            return div().into_any_element();
        };
        let hint = match self.drop_hint {
            Some(DropHint::Pane { id: target, edge }) if target == id => Some(edge),
            _ => None,
        };
        div()
            .id(("pane", id))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                if !e.bounds.contains(&e.event.position) {
                    return;
                }
                let drag = e.drag(cx);
                if drag.id == id
                    || (drag.group
                        && this
                            .tabs
                            .iter()
                            .any(|t| t.contains(drag.id) && t.contains(id)))
                {
                    return;
                }
                let x = f32::from(e.event.position.x - e.bounds.origin.x)
                    / f32::from(e.bounds.size.width).max(1.);
                let y = f32::from(e.event.position.y - e.bounds.origin.y)
                    / f32::from(e.bounds.size.height).max(1.);
                this.drop_hint = Some(DropHint::Pane {
                    id,
                    edge: DropEdge::at(x, y),
                });
                cx.notify();
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                let Some(DropHint::Pane { id: target, edge }) = this.drop_hint else {
                    return;
                };
                if target != id {
                    return;
                }
                let moved = if drag.group {
                    layout::move_group_to_edge(&mut this.tabs, drag.id, id, edge)
                } else {
                    layout::move_to_edge(&mut this.tabs, drag.id, id, edge)
                };
                if moved {
                    this.finish_move(drag.id, window, cx);
                }
            }))
            .child(slot.terminal.clone())
            .child(
                div()
                    .id(("pane-drag-grip", id))
                    .absolute()
                    .occlude()
                    .top(px(1.))
                    .left(relative(0.5))
                    .ml(px(-15.))
                    .w(px(30.))
                    .h(px(15.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .bg(rgba(0x101010dd))
                    .text_color(rgb(0x666666))
                    .text_size(px(11.))
                    .cursor_move()
                    .hover(|s| s.bg(rgb(0x18304b)).text_color(rgb(0x93c5fd)))
                    .child("⋮⋮")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.focus_pane(id, window, cx)),
                    )
                    .on_drag(
                        TabDrag {
                            id,
                            group: false,
                            label: format!("Terminal {}", id + 1),
                        },
                        |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        },
                    ),
            )
            .when(cx.has_active_drag(), |s| {
                s.when_some(hint, |s, edge| {
                    let label = match edge {
                        DropEdge::Left => "Split left",
                        DropEdge::Right => "Split right",
                        DropEdge::Top => "Split above",
                        DropEdge::Bottom => "Split below",
                    };
                    s.child(
                        div()
                            .absolute()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgba(0x3b82f633))
                            .border_2()
                            .border_color(rgb(0x60a5fa))
                            .when(edge == DropEdge::Left, |s| {
                                s.left_0().top_0().bottom_0().w(relative(0.5))
                            })
                            .when(edge == DropEdge::Right, |s| {
                                s.right_0().top_0().bottom_0().w(relative(0.5))
                            })
                            .when(edge == DropEdge::Top, |s| {
                                s.top_0().left_0().right_0().h(relative(0.5))
                            })
                            .when(edge == DropEdge::Bottom, |s| {
                                s.bottom_0().left_0().right_0().h(relative(0.5))
                            })
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x102746))
                                    .text_size(px(12.))
                                    .text_color(rgb(0xdbeafe))
                                    .child(label),
                            )
                            .with_animation(
                                ("split-preview", id * 4 + edge as usize),
                                Animation::new(Duration::from_millis(90)),
                                |el, t| el.opacity(t),
                            ),
                    )
                })
            })
            .into_any_element()
    }
    fn layout(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        match layout {
            Layout::Leaf(id) => self.pane(*id, cx),
            Layout::Split {
                key,
                vertical,
                first,
                second,
            } => {
                let a = self.layout(first, cx);
                let b = self.layout(second, cx);
                let group = if *vertical {
                    v_resizable(("terminal-split", *key))
                } else {
                    h_resizable(("terminal-split", *key))
                };
                group
                    .child(resizable_panel().size_range(px(100.)..px(6000.)).child(a))
                    .child(resizable_panel().size_range(px(100.)..px(6000.)).child(b))
                    .into_any_element()
            }
        }
    }
    fn terminal_tab(&self, id: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(slot) = self.slots.get(&id) else {
            return div().into_any_element();
        };
        let terminal = slot.terminal.read(cx);
        let exited = terminal.exited;
        let folder = terminal
            .root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let label = format!("{} · {}", folder, id + 1);
        let drag = TabDrag {
            id,
            group: self
                .tabs
                .iter()
                .any(|t| matches!(t, Layout::Leaf(leaf) if *leaf == id)),
            label: label.clone(),
        };
        let active = self.active == id;
        self.tab_drop_zone(id, false, cx)
            .h(px(31.))
            .max_w(px(210.))
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap_2()
            .pl_2()
            .pr_1()
            .border_b_1()
            .border_color(rgb(if active { 0xcacaca } else { 0x222222 }))
            .bg(rgb(if active { 0x0e0e0e } else { 0x000000 }))
            .text_color(rgb(if active { 0xe8e8e8 } else { 0x888888 }))
            .text_size(px(12.))
            .cursor_move()
            .hover(|s| s.bg(rgb(0x182132)).text_color(rgb(0xe1edff)))
            .child(div().min_w_0().truncate().child(label))
            .when(exited, |s| {
                s.child(div().text_color(rgb(0xaa7777)).child("exited"))
            })
            .child(
                bar_button(("close-terminal", id), "×")
                    .w(px(18.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_pane(id, window, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.focus_pane(id, window, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_pane(id, window, cx);
                }),
            )
            .on_drag(drag, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .into_any_element()
    }
    fn group_tab(&self, index: usize, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let anchor = layout.first();
        let name = self
            .slots
            .get(&anchor)
            .map(|s| {
                s.terminal
                    .read(cx)
                    .root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|| "Terminal".into());
        let ids = layout.leaves();
        let label = if ids.len() > 1 {
            format!("{name} ({})", ids.len())
        } else {
            name
        };
        let drag = TabDrag {
            id: anchor,
            group: true,
            label: label.clone(),
        };
        let mut group = div()
            .id(("group-tab", anchor))
            .h(px(32.))
            .flex()
            .items_center()
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(0x2b2b2b));
        if index == self.tab {
            if ids.len() > 1 {
                group = group.child(
                    self.tab_drop_zone(anchor, true, cx)
                        .h(px(31.))
                        .w(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_move()
                        .text_color(rgb(0x888888))
                        .hover(|s| s.bg(rgb(0x182132)))
                        .child("⋮")
                        .on_drag(drag, |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        }),
                );
            }
            for id in ids {
                group = group.child(self.terminal_tab(id, cx));
            }
        } else {
            group = group.child(
                self.tab_drop_zone(anchor, false, cx)
                    .h(px(31.))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .max_w(px(180.))
                    .pl_2()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(0x888888))
                    .cursor_move()
                    .hover(|s| s.bg(rgb(0x182132)).text_color(rgb(0xe1edff)))
                    .child(div().truncate().child(label))
                    .child(
                        bar_button(("close-workspace", anchor), "×")
                            .w(px(20.))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(index, window, cx);
                            })),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.switch_tab(index, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(index, window, cx);
                        }),
                    )
                    .on_drag(drag, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    }),
            );
        }
        group.into_any_element()
    }
}
impl Render for Vyber {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drop_hint = None;
        }
        let layout = self
            .tabs
            .get(self.tab)
            .cloned()
            .map(|l| {
                if self.zoomed {
                    self.pane(self.active, cx)
                } else {
                    self.layout(&l, cx)
                }
            })
            .unwrap_or_else(|| div().into_any_element());
        let browser = self.slots.get(&self.active).map(|s| s.browser.clone());
        let visible = browser.as_ref().is_some_and(|b| b.read(cx).visible);
        let git_open = browser.as_ref().is_some_and(|b| b.read(cx).git_open());
        let changes = browser.as_ref().map_or(0, |b| b.read(cx).change_count());
        let wide = browser.as_ref().is_some_and(|b| b.read(cx).wide);
        let fraction = browser
            .as_ref()
            .and_then(|b| b.read(cx).panel_width)
            .unwrap_or(0.61);
        let motion = match self.panel {
            Some(m) if m.slot == self.active && (m.shown, m.wide) == (visible, wide) => m,
            // Opening, closing and resizing glide from wherever the panel is now.
            Some(m) if m.slot == self.active && !cx.reduce_motion() => PanelMotion {
                slot: self.active,
                shown: visible,
                wide,
                from: m.value(),
                start: std::time::Instant::now(),
            },
            // Another terminal's panel appears in place.
            _ => PanelMotion::settled(self.active, visible, wide),
        };
        self.panel = Some(motion);
        let (shown, wideness) = motion.value();
        if motion.running() {
            window.request_animation_frame();
        }
        let mut body = div().relative().size_full().bg(rgb(0x000000)).child(layout);
        if let Some(browser) = browser.filter(|_| shown > 0.001) {
            let full = f32::from(window.viewport_size().width);
            let narrow = (full * fraction).max(PANEL_MIN).min(full - 8.);
            let width = narrow + (full - 8. - narrow) * wideness;
            // Hidden means slid entirely past the right edge, so opening and
            // closing move the left edge like the full-width transition does.
            let shift = (1. - shown) * (width + 8.);
            body = body.child(
                div()
                    .absolute()
                    .when(visible, |s| s.occlude())
                    .top(px(4.))
                    .bottom(px(4.))
                    .right(px(4. - shift))
                    .w(px(width))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .shadow_xl()
                    .overflow_hidden()
                    .bg(rgb(theme::PANEL))
                    .child(browser),
            );
            // Drag the left edge to resize; the full-width panel has no edge to grab.
            if visible && !wide && !motion.running() {
                body = body.child(
                    theme::resize_handle("panel-resize", PanelResize)
                        .top(px(4.))
                        .bottom(px(4.))
                        .left(px(full - 4. - width - 4.)),
                );
            }
        }
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, layout)| self.group_tab(index, layout, cx))
            .collect::<Vec<_>>();
        let menu = div()
            .id("workspace-menu")
            .absolute()
            .occlude()
            .top(px(34.))
            .left(px(4.))
            .w(px(320.))
            .p_2()
            .rounded_md()
            .bg(rgb(0x121212))
            .border_1()
            .border_color(rgb(0x333333))
            .shadow_xl()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(
                chip("menu-new", "New workspace     Ctrl+Shift+T / ⌘T").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.new_terminal(&NewTerminal, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-right", "Split right     Ctrl+D / ⌘D").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.split_right(&SplitRight, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-down", "Split down     Ctrl+Shift+D / ⌘⇧D").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.split_down(&SplitDown, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-open", "Open folder     Ctrl+Shift+O / ⌘O").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.open_folder(&OpenFolder, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-zoom", "Focus / restore     Ctrl+Shift+Enter / ⌘Enter").on_click(
                    cx.listener(|this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.zoom(&ZoomPane, w, cx);
                    }),
                ),
            )
            .child(
                chip("menu-find", "Find in terminal     Ctrl+Shift+F / ⌘F").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.search_terminal(&SearchTerminal, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-files", "Find file     Ctrl+Shift+P / ⌘P").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.quick_open(&QuickOpen, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-checkpoint", "Checkpoint     Ctrl+Shift+K / ⌘⇧K").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.checkpoint(&Checkpoint, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-git", "Source control     Ctrl+Shift+G").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.toggle_git(&ToggleGit, w, cx);
                    },
                )),
            )
            .child(
                chip("menu-project", "Edit project…").on_click(cx.listener(|this, _, w, cx| {
                    this.edit_project(&EditProject, w, cx);
                })),
            )
            .child(
                chip("menu-settings", "Settings     Ctrl+Shift+, / ⌘,").on_click(cx.listener(
                    |this, _, w, cx| {
                        this.show_shortcuts = false;
                        this.settings(&Settings, w, cx);
                    },
                )),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.show_shortcuts = false;
                cx.notify();
            }));
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x000000))
            .text_color(rgb(0xdddddd))
            .font_family(if cfg!(windows) {
                "Segoe UI"
            } else {
                ".SystemUIFont"
            })
            .text_size(px(12.))
            .track_focus(&self.focus)
            .on_drag_move(cx.listener(|this, _: &DragMoveEvent<TabDrag>, _, _| {
                // Capture runs parent-first: clear the old target before a matching child sets it.
                this.drop_hint = None;
            }))
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<PanelResize>, window, cx| {
                let full = f32::from(window.viewport_size().width);
                let width = (full - 4. - f32::from(e.event.position.x))
                    .clamp(PANEL_MIN.min(full - 8.), full - 8.);
                if let Some(slot) = this.slots.get(&this.active) {
                    slot.browser
                        .update(cx, |b, _| b.panel_width = Some(width / full));
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CancelTabDrag, window, cx| {
                if cx.stop_active_drag(window) {
                    this.drop_hint = None;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(Self::new_terminal))
            .on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::split_down))
            .on_action(cx.listener(Self::toggle_files))
            .on_action(cx.listener(Self::toggle_git))
            .on_action(cx.listener(Self::edit_project))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::checkpoint))
            .on_action(cx.listener(Self::quick_open))
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::zoom))
            .on_action(cx.listener(Self::search_terminal))
            .on_action(cx.listener(Self::next_pane))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::shortcuts))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::settings))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .child(
                TitleBar::new()
                    .h(px(32.))
                    .pl(px(if cfg!(target_os = "macos") { 80. } else { 2. }))
                    .bg(rgb(0x000000))
                    .border_color(rgb(0x252525))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .size_full()
                            .min_w_0()
                            .child(bar_button("menu", "≡").on_click(
                                cx.listener(|this, _, w, cx| this.shortcuts(&ShowShortcuts, w, cx)),
                            ))
                            .child(
                                div()
                                    .id("workspace-tabs")
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .max_w(relative(0.85))
                                    .h_full()
                                    .overflow_x_scroll()
                                    .children(tabs),
                            )
                            .child(bar_button("new-tab", "+").on_click(
                                cx.listener(|this, _, w, cx| {
                                    this.new_terminal(&NewTerminal, w, cx)
                                }),
                            ))
                            .child(
                                div()
                                    .id("tab-strip-tail")
                                    .flex_1()
                                    .h_full()
                                    .min_w(px(40.))
                                    .flex()
                                    .items_center()
                                    .px_2()
                                    .when(cx.has_active_drag(), |s| s.occlude())
                                    .when(self.drop_hint == Some(DropHint::End), |s| {
                                        s.bg(rgb(0x12273d))
                                            .border_l_2()
                                            .border_color(rgb(0x60a5fa))
                                            .text_color(rgb(0x93c5fd))
                                            .child("New tab")
                                    })
                                    .on_drag_move(cx.listener(
                                        |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                                            if e.bounds.contains(&e.event.position) {
                                                this.drop_hint = Some(DropHint::End);
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .on_drop(cx.listener(|this, drag: &TabDrag, window, cx| {
                                        this.drop_on_tab(drag, None, true, window, cx);
                                    })),
                            )
                            .child(
                                bar_button("git-toggle", "")
                                    .relative()
                                    .child(theme::icon(
                                        theme::ui("git-branch"),
                                        if git_open { 0xffffff } else { 0x9b9b9b },
                                        16.,
                                    ))
                                    .when(changes > 0, |s| {
                                        s.child(
                                            div()
                                                .absolute()
                                                .top(px(3.))
                                                .right(px(0.))
                                                .min_w(px(14.))
                                                .h(px(14.))
                                                .px(px(3.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(7.))
                                                .bg(rgb(0x2f2f2f))
                                                .text_size(px(9.5))
                                                .text_color(rgb(0xd6d6d6))
                                                .child(if changes > 99 {
                                                    "99+".to_string()
                                                } else {
                                                    changes.to_string()
                                                }),
                                        )
                                    })
                                    .when(git_open, |s| s.bg(rgb(0x1d1d1d)))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            "Source control  (Ctrl+Shift+G)",
                                        )
                                        .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.toggle_git(&ToggleGit, w, cx)
                                    })),
                            )
                            .child(
                                bar_button("files-toggle", "")
                                    .child(theme::icon(
                                        theme::ui("panel-right"),
                                        if visible && !git_open { 0xffffff } else { 0x9b9b9b },
                                        16.,
                                    ))
                                    .when(visible && !git_open, |s| s.bg(rgb(0x1d1d1d)))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            "Files and review  (Ctrl+Shift+B)",
                                        )
                                        .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.toggle_files(&ToggleFiles, w, cx)
                                    })),
                            ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .when(self.show_shortcuts, |s| s.child(menu))
            .when_some(self.project_dialog.as_ref(), |s, (dialog, _)| s.child(dialog.clone()))
            .when(!self.notice.is_empty(), |s| {
                s.child(
                    div()
                        .absolute()
                        .occlude()
                        .bottom_2()
                        .left_2()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x24201a))
                        .text_size(px(12.))
                        .child(self.notice.clone())
                        .child(chip("dismiss-notice", "×").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.notice.clear();
                                cx.notify();
                            },
                        ))),
                )
            })
    }
}
pub fn bind_keys(cx: &mut App) {
    let prefix = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl-shift"
    };
    cx.bind_keys([
        KeyBinding::new("escape", CancelTabDrag, None),
        KeyBinding::new(&format!("{prefix}-t"), NewTerminal, None),
        KeyBinding::new(
            if cfg!(windows) { "ctrl-d" } else { "cmd-d" },
            SplitRight,
            Some("Terminal && !Input"),
        ),
        KeyBinding::new(&format!("{prefix}-e"), SplitDown, None),
        KeyBinding::new(&format!("{prefix}-b"), ToggleFiles, None),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-shift-g"
            } else {
                "ctrl-shift-g"
            },
            ToggleGit,
            None,
        ),
        KeyBinding::new(&format!("{prefix}-p"), QuickOpen, None),
        KeyBinding::new(&format!("{prefix}-k"), Checkpoint, None),
        KeyBinding::new(&format!("{prefix}-o"), OpenFolder, None),
        KeyBinding::new(&format!("{prefix}-w"), ClosePane, None),
        KeyBinding::new(&format!("{prefix}-f"), SearchTerminal, None),
        KeyBinding::new(&format!("{prefix}-enter"), ZoomPane, None),
        KeyBinding::new(&format!("{prefix}-]"), NextPane, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-s"
            } else {
                "ctrl-s"
            },
            SaveFile,
            Some("Input"),
        ),
        KeyBinding::new(
            if cfg!(windows) {
                "ctrl-shift-h"
            } else {
                "cmd-shift-h"
            },
            ShowShortcuts,
            None,
        ),
        KeyBinding::new(if cfg!(windows) { "alt-f4" } else { "cmd-q" }, Quit, None),
        KeyBinding::new(
            if cfg!(windows) {
                "ctrl-shift-,"
            } else {
                "cmd-,"
            },
            Settings,
            None,
        ),
    ]);
    if cfg!(windows) {
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-d", SplitDown, None),
            // Ctrl+T opens a tab like a browser; the shell no longer sees Ctrl+T.
            KeyBinding::new("ctrl-t", NewTerminal, None),
        ]);
    }
    // Terminal font zoom. GPUI turns a shifted digit or punctuation key into the
    // character it types, so Turkish Q Ctrl+Shift+0 arrives as ctrl-= and
    // Ctrl+Shift+4 as ctrl-+; US Ctrl+Shift+= arrives as ctrl-+.
    let zoom = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{zoom}-="), TerminalZoomIn, None),
        KeyBinding::new(&format!("{zoom}-+"), TerminalZoomIn, None),
        KeyBinding::new(&format!("{zoom}--"), TerminalZoomOut, None),
        KeyBinding::new(&format!("{zoom}-0"), TerminalZoomReset, None),
    ]);
    if cfg!(target_os = "macos") {
        cx.set_menus([
            Menu::new("Vyber").items([
                MenuItem::action("Settings…", Settings),
                MenuItem::action("Quit Vyber", Quit),
            ]),
            Menu::new("File").items([
                MenuItem::action("New tab", NewTerminal),
                MenuItem::action("Open folder…", OpenFolder),
                MenuItem::action("Close terminal", ClosePane),
            ]),
        ]);
        cx.bind_keys([
            KeyBinding::new("cmd-n", NewTerminal, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-shift-k", Checkpoint, None),
        ]);
    }
}
