use crate::{
    browser::{Browser, BrowserEvent},
    closing::{self, ClosePlan, CloseTarget},
    config::{Config, PANEL_FONT_SIZE, PanelMode},
    layout::{self, DropEdge, Layout},
    panel::{DEFAULT_FRACTION, WorkspaceBody},
    project_dialog::{ProjectDialog, ProjectDialogEvent},
    split::SplitPane,
    tab_state::TabState,
    tasks::{Monitor, TaskReview},
    terminal::{Terminal, TerminalEvent, TurnBadge},
    theme::{self, chip},
    workspace,
};
use gpui::{prelude::*, *};
use gpui_kit::component::TitleBar;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

mod tabs;
use tabs::{RenameTab, TabMenu};

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
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        ToggleGit,
        EditProject
    ]
);

/// Ctrl+1…8 go to that tab and Ctrl+9 to the last one, as in browsers.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = vyber, no_json)]
struct SelectTab(usize);

#[derive(Clone, Copy)]
pub enum WorkspaceLaunch {
    Restore,
    Empty,
}

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
pub(crate) struct PanelResize;
impl Render for PanelResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
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
        .h_full()
        .flex_shrink_0()
        .text_size(px(13.))
        .text_color(rgb(0x9b9b9b))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x171717)).text_color(rgb(0xffffff)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label.into())
}
fn icon_bar_button(id: impl Into<ElementId>, name: &str, active: bool) -> Stateful<Div> {
    let id = id.into();
    let group: SharedString = format!("bar-button-{id}").into();
    div()
        .id(id)
        .group(group.clone())
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .w(px(30.))
        .h_full()
        .flex_shrink_0()
        .text_size(px(13.))
        .text_color(rgb(0x9b9b9b))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x171717)).text_color(rgb(0xffffff)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            theme::icon(
                theme::ui(name),
                if active { 0xffffff } else { 0x9b9b9b },
                16.,
            )
            .group_hover(group, |s| s.text_color(rgb(0xffffff))),
        )
}

struct Slot {
    terminal: Entity<Terminal>,
    /// Refresh local titles without repainting the chrome on every PTY byte.
    title: String,
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
    #[serde(default)]
    tab_state: TabState,
    /// Terminal font size from the zoom shortcuts of older versions, moved
    /// to config.toml on start.
    #[serde(default, skip_serializing)]
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
    tab_state: TabState,
    tab_scroll: ScrollHandle,
    tab_menu: Option<TabMenu>,
    tab_list_dismissed: Option<std::time::Instant>,
    rename_tab: Option<RenameTab>,
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
    close_pending: bool,
    state_path: PathBuf,
    /// The workspace as last written, so an unchanged one costs no disk access.
    saved_state: std::cell::RefCell<Option<Vec<u8>>>,
    restoring: bool,
    #[cfg(test)]
    process_snapshot: Option<crate::processes::ProcessSnapshot>,
    drop_hint: Option<DropHint>,
    tab_drag: Option<TabDrag>,
    panel: Option<PanelMotion>,
    /// The panel's left edge is being dragged.
    resizing_panel: bool,
    /// How terminals follow size changes right now; see `Terminal::set_resize_interval`.
    resize_interval: Option<Duration>,
    config_stamp: Option<std::time::SystemTime>,
    project_dialog: Option<(Entity<ProjectDialog>, Subscription)>,
    /// The project dialog fading out after it closed.
    project_closing: Option<(Entity<ProjectDialog>, std::time::Instant)>,
    _subscriptions: Vec<Subscription>,
}

/// The file panel's slide: `shown` fades and slides it in from the right,
/// `wide` grows it to the full width and `docked` moves the terminals' right
/// edge aside for it. Values run from `from` to the target.
#[derive(Clone, Copy)]
struct PanelMotion {
    slot: usize,
    shown: bool,
    wide: bool,
    docked: bool,
    /// (shown, wide, docked) as 0–1 when this motion started.
    from: (f32, f32, f32),
    start: std::time::Instant,
}
impl PanelMotion {
    const DURATION: f32 = 0.24;
    fn settled(slot: usize, shown: bool, wide: bool, docked: bool) -> Self {
        Self {
            slot,
            shown,
            wide,
            docked,
            from: (unit(shown), unit(wide), unit(docked)),
            start: std::time::Instant::now() - Duration::from_secs(1),
        }
    }
    fn progress(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / Self::DURATION).min(1.)
    }
    fn running(&self) -> bool {
        self.progress() < 1.
    }
    fn value(&self) -> (f32, f32, f32) {
        let t = theme::ease_out(self.progress());
        let to = (unit(self.shown), unit(self.wide), unit(self.docked));
        (
            self.from.0 + (to.0 - self.from.0) * t,
            self.from.1 + (to.1 - self.from.1) * t,
            self.from.2 + (to.2 - self.from.2) * t,
        )
    }
}
fn unit(on: bool) -> f32 {
    f32::from(u8::from(on))
}
impl Vyber {
    pub fn new(
        root: PathBuf,
        launch: WorkspaceLaunch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let roots = Arc::new(Mutex::new(Vec::new()));
        #[cfg(not(test))]
        let monitor = Monitor::start(roots.clone());
        #[cfg(test)]
        let monitor = Monitor::inactive();
        let mut app = Self {
            slots: HashMap::new(),
            tabs: vec![],
            tab: 0,
            active: 0,
            tab_state: TabState::default(),
            tab_scroll: ScrollHandle::new(),
            tab_menu: None,
            tab_list_dismissed: None,
            rename_tab: None,
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
            close_pending: false,
            state_path: workspace::data_dir().join("workspace.json"),
            saved_state: Default::default(),
            restoring: true,
            #[cfg(test)]
            process_snapshot: None,
            drop_hint: None,
            tab_drag: None,
            panel: None,
            resizing_panel: false,
            resize_interval: None,
            config_stamp: Config::modified(),
            project_dialog: None,
            project_closing: None,
            _subscriptions: vec![],
        };
        #[cfg(not(test))]
        let restore_allowed = std::env::args_os().nth(1).is_none();
        #[cfg(test)]
        let restore_allowed = true;
        let saved = if matches!(launch, WorkspaceLaunch::Empty) {
            Some(SavedState::default())
        } else if restore_allowed && crate::config::Config::load().restore_workspace {
            std::fs::read(&app.state_path)
                .ok()
                .and_then(|b| serde_json::from_slice::<SavedState>(&b).ok())
        } else {
            None
        };
        let preserve_empty = saved
            .as_ref()
            .is_some_and(|state| state.slots.is_empty() && state.tabs.is_empty());
        if let Some(saved) = saved {
            if let Some(size) = saved.font_size {
                Config::update(cx, |c| c.font_size = size);
            }
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
                let missing = tabs
                    .iter()
                    .flat_map(Layout::leaves)
                    .filter(|id| !app.slots.contains_key(id))
                    .collect::<Vec<_>>();
                for id in missing {
                    tabs = tabs.into_iter().filter_map(|t| t.remove(id)).collect();
                }
                if !tabs.is_empty() {
                    app.tabs = tabs;
                }
                app.tab = saved.tab.min(app.tabs.len() - 1);
                app.active = if app.tabs[app.tab].contains(saved.active) {
                    saved.active
                } else {
                    app.tabs[app.tab].first()
                };
                app.next = app.slots.keys().copied().max().unwrap_or(0) + 1;
                app.tab_state = saved.tab_state;
                app.tab_state.sync(&app.tabs, None);
                app.tab_state.focus(app.active);
                app.tab_scroll.scroll_to_item(app.tab);
                window.focus(&app.slots[&app.active].terminal.read(cx).focus.clone(), cx);
            }
        }
        if app.slots.is_empty() && !preserve_empty {
            app.add_terminal(root, false, false, window, cx);
        }
        app.normalize_selection();
        app.restoring = false;
        app.update_roots(cx);
        app.persist(cx);
        if app.slots.is_empty() {
            window.focus(&app.focus, cx);
        }
        app._subscriptions
            .push(cx.on_release(|this, cx| this.persist(cx)));
        app._subscriptions
            .push(cx.observe_global::<Config>(|this, cx| this.apply_config(cx)));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            if let Some(app) = weak.upgrade() {
                app.update(cx, |app, cx| {
                    app.request_close(CloseTarget::Window, window, cx)
                });
                false
            } else {
                true
            }
        });
        app._subscriptions
            .push(cx.observe_window_bounds(window, |_, _, cx| cx.notify()));
        cx.spawn_in(window, async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
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
        #[cfg(not(test))]
        let config = crate::config::Config::load();
        #[cfg(not(test))]
        let shell = config.shell.as_deref();
        #[cfg(test)]
        let shell = Some("/bin/sh");
        let backend = match Terminal::prepare(id, &root, shell) {
            Ok(backend) => backend,
            Err(error) => {
                self.notice = error.to_string();
                cx.notify();
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(id, &root, backend, cx));
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
            this.tab_state.focus(id);
            cx.notify();
        });
        let badge = cx.subscribe(
            &terminal,
            move |this, _, event: &TerminalEvent, cx| match event {
                TerminalEvent::OpenTurn(task) => this.open_turn(id, task.clone(), cx),
            },
        );
        let comments = cx.subscribe_in(
            &browser,
            window,
            move |this, _, event: &BrowserEvent, window, cx| match event {
                BrowserEvent::LayoutChanged => cx.notify(),
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
                BrowserEvent::Closed => {
                    if let Some(slot) = this.slots.get(&id) {
                        let focus = slot.terminal.read(cx).focus.clone();
                        window.focus(&focus, cx);
                    }
                }
            },
        );
        self.slots.insert(
            id,
            Slot {
                title: terminal.read(cx).title.clone(),
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
        self.zoomed = false;
        self.tab_state.sync(&self.tabs, None);
        self.tab_state.focus(id);
        self.tab_scroll.scroll_to_item(self.tab);
        self.tab_menu = None;
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
    /// The terminal an agent turn runs in, bound on the session's first
    /// observed turn. Claude Code names each session's process, so a Claude
    /// session belongs to the terminal whose shell that process runs under.
    /// Other sessions go to the terminal in their folder where Enter was
    /// pressed last among those that run the agent or whose typed line
    /// matches the prompt, a match first.
    fn turn_terminal(&mut self, review: &TaskReview, cx: &App) -> Option<usize> {
        if review.session.is_empty() || !review.is_from_terminal() {
            return None;
        }
        if let Some(id) = self.sessions.get(&review.session)
            && self.slots.contains_key(id)
        {
            return Some(*id);
        }
        let agent = (review.agent == "Claude")
            .then(|| crate::tasks::claude_process(&review.session))
            .flatten();
        if agent.is_none() && (!review.active || review.after.is_some()) {
            return None;
        }
        let processes = crate::platform::Processes::list();
        let id = if let Some(agent) = agent {
            self.slots
                .iter()
                .find(|(_, slot)| {
                    let shell = slot.terminal.read(cx).shell;
                    shell.is_some_and(|shell| processes.descends(agent, shell))
                })
                .map(|(id, _)| *id)?
        } else {
            let program = review.agent.to_lowercase();
            let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
            let prompt = normalize(review.label.split_once(" · ").map_or("", |(_, p)| p));
            let now = std::time::Instant::now();
            self.slots
                .iter()
                .filter_map(|(id, slot)| {
                    let terminal = slot.terminal.read(cx);
                    let submitted = terminal.last_submit?;
                    if now.duration_since(submitted) > Duration::from_secs(45)
                        || !crate::tasks::matches_root(&review.root, &terminal.root)
                    {
                        return None;
                    }
                    let runs = terminal
                        .shell
                        .is_some_and(|shell| processes.runs(shell, &program));
                    let typed = normalize(&terminal.last_line);
                    let matched = typed.len() >= 3
                        && prompt.len() >= 3
                        && (prompt.starts_with(&typed) || typed.starts_with(&prompt));
                    (runs || matched).then_some((matched, runs, submitted, *id))
                })
                .max_by_key(|(matched, runs, submitted, _)| (*matched, *runs, *submitted))?
                .3
        };
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
            .unwrap_or_else(|| crate::platform::startup_root(None))
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stamp = Config::modified();
        if stamp != self.config_stamp {
            self.config_stamp = stamp;
            // A file that does not parse keeps the settings in use.
            if let Some(config) = Config::read().filter(|c| c != cx.global::<Config>()) {
                cx.set_global(config);
            }
        }
        let mut changed = false;
        for (id, slot) in &mut self.slots {
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
            if let Some(message) = message
                && !window.is_window_active()
                && cx.global::<Config>().notifications
            {
                self.notifications.show(*id, message);
            }
            let root = slot.terminal.read(cx).root.clone();
            if root != slot.browser.read(cx).root {
                slot.browser.update(cx, |b, cx| b.change_root(root, cx));
                changed = true;
            }
            if let Some(target) = path {
                slot.browser
                    .update(cx, |b, cx| b.open_link(target, window, cx));
            }
            let title = &slot.terminal.read(cx).title;
            if &slot.title != title {
                slot.title = title.clone();
                changed = true;
            }
        }
        if changed {
            self.update_roots(cx);
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
            } else if !self.close_pending {
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
            // A late update of an earlier turn must not replace a newer one,
            // so two turns never take turns on the badge.
            if let Some(id) = self.turn_terminal(&review, cx)
                && let Some(slot) = self.slots.get_mut(&id)
                && slot.turn.as_ref().is_none_or(|t| {
                    t.id == review.id || (review.active && review.started >= t.started)
                })
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
                self.tab_state.focus(id);
                self.tab_scroll.scroll_to_item(self.tab);
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
        if self.restoring {
            return;
        }
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
            tab_state: self.tab_state.clone(),
            font_size: None,
        };
        if let Ok(bytes) = serde_json::to_vec(&state) {
            let mut saved = self.saved_state.borrow_mut();
            let saved =
                saved.get_or_insert_with(|| std::fs::read(&self.state_path).unwrap_or_default());
            if *saved != bytes {
                if let Some(dir) = self.state_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&self.state_path, &bytes) {
                    Ok(()) => *saved = bytes,
                    Err(e) => log::warn!("Save workspace: {e}"),
                }
            }
        }
    }
    pub(crate) fn new_terminal(
        &mut self,
        _: &NewTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                    this.project_closing = this
                        .project_dialog
                        .take()
                        .map(|(dialog, _)| (dialog, std::time::Instant::now()));
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
    fn keyboard_pane(&self, window: &Window, cx: &App) -> Option<usize> {
        self.slots
            .iter()
            .find_map(|(id, slot)| {
                (slot.terminal.read(cx).focus.contains_focused(window, cx)
                    || slot.browser.read(cx).has_focus(window, cx))
                .then_some(*id)
            })
            .or_else(|| self.selection().map(|(_, id)| id))
    }
    fn close(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_pending {
            return;
        }
        let Some(id) = self.keyboard_pane(window, cx) else {
            return;
        };
        if let Some(slot) = self.slots.get(&id)
            && slot.browser.read(cx).visible
        {
            let handled = slot
                .browser
                .update(cx, |browser, cx| browser.close_active_document(window, cx));
            if !handled {
                slot.browser
                    .update(cx, |browser, cx| browser.close_panel(cx));
                window.focus(&slot.terminal.read(cx).focus.clone(), cx);
            }
            self.persist(cx);
            cx.notify();
            return;
        }
        // Split grubundaki komşular aynı Cmd+W isteğine dahil edilmez.
        self.close_pane(id, window, cx);
    }
    fn close_pane(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(CloseTarget::Pane(id), window, cx);
    }
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.tabs.get(index) else {
            return;
        };
        let mut ids = layout.leaves();
        ids.sort_unstable();
        self.request_close(CloseTarget::Tab(ids), window, cx);
    }
    fn selection(&self) -> Option<(usize, usize)> {
        closing::selection(&self.tabs, self.active, self.tab)
    }
    fn normalize_selection(&mut self) {
        (self.tab, self.active) = self.selection().unwrap_or((0, 0));
        if self.tabs.is_empty() {
            self.zoomed = false;
            self.panel = None;
        }
    }
    fn remove_terminals(&mut self, ids: &[usize], window: &mut Window, cx: &mut Context<Self>) {
        let active_removed = ids.contains(&self.active);
        closing::remove_panes(&mut self.tabs, ids);
        for id in ids {
            self.slots.remove(id);
        }
        self.tab_state.sync(&self.tabs, None);
        self.normalize_selection();
        if active_removed && let Some(group) = self.tab_state.groups.get(self.tab) {
            self.active = group.active;
        }
        self.tab_state.focus(self.active);
        self.tab_scroll.scroll_to_item(self.tab);
        self.tab_menu = None;
        if active_removed {
            self.zoomed = false;
            self.panel = None;
            let focus = self
                .slots
                .get(&self.active)
                .map(|slot| slot.terminal.read(cx).focus.clone())
                .unwrap_or_else(|| self.focus.clone());
            window.focus(&focus, cx);
        }
        self.tab_state.focus(self.active);
        self.tab_scroll.scroll_to_item(self.tab);
        self.update_roots(cx);
        // Kapanmış sekmeler tekrar açılmasın diye sonuç düzeni kaydedilir.
        self.persist(cx);
        cx.notify();
    }
    fn zoom(&mut self, _: &ZoomPane, _: &mut Window, cx: &mut Context<Self>) {
        self.zoomed = !self.zoomed;
        cx.notify();
    }
    pub(crate) fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(CloseTarget::Quit, window, cx);
    }
    fn close_sessions(&self, ids: &[usize], cx: &App) -> Vec<(usize, Option<u32>)> {
        ids.iter()
            .filter_map(|id| {
                self.slots
                    .get(id)
                    .map(|slot| (*id, slot.terminal.read(cx).process.pid))
            })
            .collect()
    }
    fn plan_valid(&self, plan: &ClosePlan, cx: &App) -> bool {
        plan.valid(
            &self.tabs,
            self.slots.keys().copied().collect(),
            &self.close_sessions(&plan.ids, cx),
        )
    }
    fn request_close(&mut self, target: CloseTarget, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_pending {
            return;
        }
        let Some(ids) = target.ids(&self.tabs, self.slots.keys().copied().collect()) else {
            return;
        };
        let plan = ClosePlan {
            target,
            sessions: self.close_sessions(&ids, cx),
            ids,
        };
        if self.close_blocked(&plan, cx) {
            return;
        }
        let terminals = plan
            .ids
            .iter()
            .filter_map(|id| {
                let terminal = self.slots[id].terminal.read(cx);
                (!terminal.exited).then(|| {
                    (
                        format!(
                            "Terminal {} · {} · {}",
                            terminal.id + 1,
                            terminal.title,
                            crate::platform::folder_name(&terminal.root)
                        ),
                        terminal.process.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        if terminals.is_empty() {
            self.finish_close(&plan, window, cx);
            return;
        }
        self.close_pending = true;
        #[cfg(test)]
        let snapshot_override = self.process_snapshot.clone();
        let inspection = cx.background_executor().spawn(async move {
            #[cfg(not(test))]
            let snapshot = crate::processes::ProcessSnapshot::capture();
            #[cfg(test)]
            let snapshot = snapshot_override
                .map(Ok)
                .unwrap_or_else(crate::processes::ProcessSnapshot::capture);
            terminals
                .into_iter()
                .filter_map(|(label, process)| {
                    match snapshot
                        .as_ref()
                        .ok()
                        .and_then(|snapshot| snapshot.running(&process))
                    {
                        Some(names) if names.is_empty() => None,
                        Some(names) => Some(format!("{label}: {}", names.join(", "))),
                        None => Some(format!("{label}: process information unavailable")),
                    }
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |entity, cx| {
            let running = inspection.await;
            let answer = entity.update_in(cx, |this, window, cx| {
                if !this.plan_valid(&plan, cx) {
                    this.close_pending = false;
                    this.notice = "The terminal layout changed. Try closing again.".into();
                    cx.notify();
                    None
                } else if running.is_empty() {
                    this.close_pending = false;
                    this.finish_close(&plan, window, cx);
                    None
                } else {
                    let (message, confirm) = match &plan.target {
                        CloseTarget::Quit => ("Quit Vyber and stop running processes?", "Quit Vyber"),
                        CloseTarget::Window => ("Close this window and stop running processes?", "Close window"),
                        CloseTarget::Tab(_) => ("Close this group and stop running processes?", "Close group"),
                        CloseTarget::Pane(_) => ("Close this terminal and stop running processes?", "Close terminal"),
                    };
                    let detail = format!("Running terminal sessions:\n\n{}\n\nClosing will interrupt these sessions.", running.join("\n"));
                    Some(window.prompt(PromptLevel::Warning, message, Some(&detail), &[
                        PromptButton::cancel("Cancel"), PromptButton::new(confirm),
                    ], cx))
                }
            }).ok().flatten();
            if let Some(answer) = answer {
                let confirmed = matches!(answer.await, Ok(1));
                let _ = entity.update_in(cx, |this, window, cx| {
                    this.close_pending = false;
                    if confirmed {
                        this.finish_close(&plan, window, cx);
                    }
                });
            }
        }).detach();
    }
    fn close_blocked(&mut self, plan: &ClosePlan, cx: &mut Context<Self>) -> bool {
        if !matches!(plan.target, CloseTarget::Quit)
            && plan.ids.iter().any(|id| {
                self.slots
                    .get(id)
                    .is_some_and(|slot| slot.browser.read(cx).has_dirty())
            })
        {
            self.notice = "Save your editor changes before closing this terminal.".into();
            cx.notify();
            true
        } else {
            false
        }
    }
    fn finish_close(&mut self, plan: &ClosePlan, window: &mut Window, cx: &mut Context<Self>) {
        if !self.plan_valid(plan, cx) {
            self.notice = "The terminal layout changed. Try closing again.".into();
            cx.notify();
            return;
        }
        if self.close_blocked(plan, cx) {
            return;
        }
        match plan.target {
            CloseTarget::Pane(_) | CloseTarget::Tab(_) => {
                self.remove_terminals(&plan.ids, window, cx)
            }
            CloseTarget::Window => {
                self.remove_terminals(&plan.ids, window, cx);
                window.remove_window();
            }
            CloseTarget::Quit => {
                // Çıkışta açık düzen ve taslaklar bir sonraki başlangıç için korunur.
                self.persist(cx);
                crate::lifecycle::quit_application(cx);
            }
        }
    }
    pub(crate) fn settings(&mut self, _: &Settings, window: &mut Window, cx: &mut Context<Self>) {
        if self.slots.is_empty() {
            self.new_terminal(&NewTerminal, window, cx);
        }
        crate::config::Config::load();
        if let Some(slot) = self.slots.get(&self.active) {
            slot.browser.update(cx, |b, cx| {
                b.visible = true;
                b.open(crate::config::Config::path(), true, cx);
            });
        }
        cx.notify();
    }
    /// Settings changed in config.toml or from a shortcut.
    fn apply_config(&mut self, cx: &mut Context<Self>) {
        let config = cx.global::<Config>().clone();
        cx.set_reduce_motion(config.reduced_motion);
        for slot in self.slots.values() {
            slot.terminal.update(cx, |t, cx| {
                t.set_font(&config.font_family, config.font_size, cx)
            });
            slot.browser.update(cx, |browser, cx| {
                browser.docked = config.panel_mode == PanelMode::Dock;
                browser.layout_changed(cx);
            });
        }
        cx.notify();
    }
    /// The zoom shortcuts resize what has focus: the file panel, the Git
    /// panel or the terminals. `None` goes back to the default size.
    fn change_font_size(&mut self, step: Option<f32>, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = Config::default();
        let Some(browser) = self.focused_panel(window, cx) else {
            Config::update(cx, |c| {
                c.font_size = step.map_or(defaults.font_size, |step| c.font_size + step)
            });
            return;
        };
        let git = browser.read(cx).git_open();
        Config::update(cx, |c| {
            let size = if git {
                &mut c.git_font_size
            } else {
                &mut c.files_font_size
            };
            *size = step.map_or(PANEL_FONT_SIZE, |step| *size + step);
        });
        let config = cx.global::<Config>();
        let (name, size) = if git {
            ("Source control", config.git_font_size)
        } else {
            ("Files", config.files_font_size)
        };
        browser.update(cx, |b, cx| b.announce(format!("{name} · {size:.0} px"), cx));
    }
    fn increase_font_size(
        &mut self,
        _: &IncreaseFontSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.change_font_size(Some(1.), window, cx);
    }
    fn decrease_font_size(
        &mut self,
        _: &DecreaseFontSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.change_font_size(Some(-1.), window, cx);
    }
    fn reset_font_size(&mut self, _: &ResetFontSize, window: &mut Window, cx: &mut Context<Self>) {
        self.change_font_size(None, window, cx);
    }
    fn shortcuts(&mut self, _: &ShowShortcuts, _: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        self.show_shortcuts = !self.show_shortcuts;
        cx.notify();
    }
    fn search_terminal(&mut self, _: &SearchTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.slots.get(&self.active) {
            s.terminal.update(cx, |t, cx| t.toggle_search(window, cx));
        }
    }
    fn next_pane(&mut self, _: &NextPane, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.tabs.get(self.tab) else {
            return;
        };
        let ids = layout.leaves();
        if let Some(i) = ids.iter().position(|id| *id == self.active) {
            self.active = ids[(i + 1) % ids.len()];
            self.tab_state.focus(self.active);
            let focus = self.slots[&self.active].terminal.read(cx).focus.clone();
            window.focus(&focus, cx);
            cx.notify();
        }
    }
    /// The file panel showing now, when it has keyboard focus.
    fn focused_panel(&self, window: &Window, cx: &App) -> Option<Entity<Browser>> {
        self.panel
            .and_then(|m| self.slots.get(&m.slot))
            .map(|s| s.browser.clone())
            .filter(|b| b.read(cx).visible && b.read(cx).has_focus(window, cx))
    }
    /// Ctrl+Tab steps through the tabs of a focused file panel, else through
    /// the window's tabs.
    fn cycle_panel_tab(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.focused_panel(window, cx)
            .is_some_and(|b| b.update(cx, |b, cx| b.cycle_tab(step, window, cx)))
    }
    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.cycle_panel_tab(1, window, cx) {
            return;
        }
        if !self.tabs.is_empty() {
            self.switch_tab((self.tab + 1) % self.tabs.len(), window, cx);
        }
    }
    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.cycle_panel_tab(-1, window, cx) {
            return;
        }
        if !self.tabs.is_empty() {
            self.switch_tab(
                (self.tab + self.tabs.len() - 1) % self.tabs.len(),
                window,
                cx,
            );
        }
    }
    fn select_tab(&mut self, action: &SelectTab, window: &mut Window, cx: &mut Context<Self>) {
        let index = if action.0 >= 8 {
            self.tabs.len().saturating_sub(1)
        } else {
            action.0
        };
        if index < self.tabs.len() && index != self.tab {
            self.switch_tab(index, window, cx);
        }
    }
    fn switch_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tab_state.focus(self.active);
        self.tab = index;
        self.active = self.tab_state.groups[index].active;
        self.tab_scroll.scroll_to_item(index);
        self.tab_menu = None;
        self.zoomed = false;
        let focus = self.slots[&self.active].terminal.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }
    pub(crate) fn open_folder(
        &mut self,
        _: &OpenFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            self.tab_state.focus(id);
            self.tab_scroll.scroll_to_item(tab);
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
        let remaining = if drag.group {
            None
        } else {
            self.tabs
                .iter()
                .find(|t| t.contains(drag.id))
                .and_then(|t| t.leaves().into_iter().find(|id| *id != drag.id))
        };
        let moved = if drag.group {
            layout::reorder_group(&mut self.tabs, drag.id, target, after)
        } else if let Some(target) = target {
            layout::reorder_pane(&mut self.tabs, drag.id, target, after)
        } else {
            layout::move_pane_to_tab(&mut self.tabs, drag.id, None, true)
        };
        if moved {
            self.tab_state.sync(&self.tabs, remaining);
            self.finish_move(drag.id, window, cx);
        }
    }
    fn tab_drop_zone(&self, id: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let marker = match self.drop_hint {
            Some(DropHint::Tab { id: target, after }) if target == id => Some(after),
            _ => None,
        };
        div()
            .id(("tab-drop-zone", id))
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
                if let Some(DropHint::Tab { id: target, after }) = this.drop_hint
                    && target == id
                {
                    this.drop_on_tab(drag, Some(id), after, window, cx);
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
            .flex()
            .flex_col()
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
                    this.tab_state.sync(&this.tabs, Some(id));
                    this.finish_move(drag.id, window, cx);
                }
            }))
            .when(self.tabs[self.tab].leaves().len() > 1, |s| {
                s.child(self.pane_title(id, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(slot.terminal.clone()),
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
                vertical,
                ratio,
                first,
                second,
                ..
            } => {
                let a = self.layout(first, cx);
                let b = self.layout(second, cx);
                let anchor = second.first();
                SplitPane::new(anchor, *vertical, *ratio, a, b)
                    .on_resize(cx.listener(move |this, ratio: &f32, _, cx| {
                        if let Some(layout) = this.tabs.iter_mut().find(|l| l.contains(anchor))
                            && layout.resize_split(anchor, *ratio)
                        {
                            this.persist(cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            }
        }
    }
}
impl Render for Vyber {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drop_hint = None;
            self.tab_drag = None;
        } else {
            self.scroll_dragged_tabs(window);
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
            .unwrap_or_else(|| {
                div()
                    .id("empty-workspace")
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .text_color(rgb(0x9b9b9b))
                    .child(div().text_size(px(18.)).child("No terminals open"))
                    .child(
                        chip("empty-new-terminal", "New terminal  ⌘T / Ctrl+Shift+T").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.new_terminal(&NewTerminal, window, cx)
                            }),
                        ),
                    )
                    .into_any_element()
            });
        // A closed project dialog fades out over a few frames.
        let closing = self.project_closing.as_ref().and_then(|(dialog, at)| {
            let left = 1. - at.elapsed().as_secs_f32() / 0.14;
            (left > 0.).then(|| (dialog.clone(), left))
        });
        if closing.is_some() {
            window.request_animation_frame();
        } else {
            self.project_closing = None;
        }
        let browser = self.slots.get(&self.active).map(|s| s.browser.clone());
        let visible = browser.as_ref().is_some_and(|b| b.read(cx).visible);
        let git_open = browser.as_ref().is_some_and(|b| b.read(cx).git_open());
        let changes = browser.as_ref().map_or(0, |b| b.read(cx).change_count());
        let config = cx.global::<Config>();
        let docked = config.panel_mode == PanelMode::Dock;
        // A docked panel stays while another terminal of the tab has focus, so
        // clicking between splits does not reflow every terminal.
        let slot = self
            .panel
            .map(|m| m.slot)
            .filter(|id| {
                docked
                    && !visible
                    && self.slots.contains_key(id)
                    && self.tabs.get(self.tab).is_some_and(|l| l.contains(*id))
            })
            .unwrap_or(self.active);
        let panel = self.slots.get(&slot).map(|s| s.browser.clone());
        let (shown_target, wide, fraction, panel_git) =
            panel.as_ref().map_or((false, false, None, false), |b| {
                let b = b.read(cx);
                (b.visible, b.wide, b.panel_width, b.git_open())
            });
        let fraction = fraction.unwrap_or(DEFAULT_FRACTION);
        // The panel is laid out in rems, so one rem size scales all of it.
        let rem = px(theme::REM / PANEL_FONT_SIZE
            * if panel_git {
                config.git_font_size
            } else {
                config.files_font_size
            });
        let motion = match self.panel {
            Some(m)
                if m.slot == slot
                    && (m.shown, m.wide, m.docked) == (shown_target, wide, docked) =>
            {
                m
            }
            // Opening, closing, docking and resizing glide from wherever the panel is now.
            Some(m) if m.slot == slot && !cx.reduce_motion() => PanelMotion {
                slot,
                shown: shown_target,
                wide,
                docked,
                from: m.value(),
                start: std::time::Instant::now(),
            },
            // Another terminal's panel appears in place.
            _ => PanelMotion::settled(slot, shown_target, wide, docked),
        };
        self.panel = Some(motion);
        let (shown, wideness, dockness) = motion.value();
        if motion.running() {
            window.request_animation_frame();
        }
        let full = f32::from(window.viewport_size().width);
        // Terminals reflow once a slide ends rather than on every frame, and
        // at most every 100 ms while a docked panel's edge is dragged.
        if !cx.has_active_drag() {
            self.resizing_panel = false;
        }
        let interval = if dockness > 0. && motion.running() {
            Some(Duration::MAX)
        } else if docked && self.resizing_panel {
            Some(Duration::from_millis(100))
        } else {
            None
        };
        if interval != self.resize_interval {
            self.resize_interval = interval;
            for slot in self.slots.values() {
                slot.terminal
                    .update(cx, |t, cx| t.set_resize_interval(interval, cx));
            }
        }
        let mut body = WorkspaceBody::new(layout);
        body.browser = panel;
        body.visible = shown_target;
        body.docked = docked;
        body.dockness = dockness;
        body.rem = Some(rem);
        body.wide = wide;
        body.fraction = fraction;
        body.shown = shown;
        body.wideness = wideness;
        body.resizing = !motion.running();
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
            .top(px(38.))
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
                chip(
                    "menu-right",
                    if cfg!(target_os = "linux") {
                        "Split right     Ctrl+Shift+D"
                    } else {
                        "Split right     Ctrl+D / ⌘D"
                    },
                )
                .on_click(cx.listener(|this, _, w, cx| {
                    this.show_shortcuts = false;
                    this.split_right(&SplitRight, w, cx);
                })),
            )
            .child(
                chip(
                    "menu-down",
                    if cfg!(target_os = "linux") {
                        "Split down     Ctrl+Shift+E"
                    } else {
                        "Split down     Ctrl+Shift+D / ⌘⇧D"
                    },
                )
                .on_click(cx.listener(|this, _, w, cx| {
                    this.show_shortcuts = false;
                    this.split_down(&SplitDown, w, cx);
                })),
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
        let tab_overlays = self.tab_overlays(cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x000000))
            .text_color(rgb(0xdddddd))
            .font_family(crate::theme::ui_font())
            .text_size(px(12.))
            .track_focus(&self.focus)
            .on_modifiers_changed(cx.listener(|this, _: &ModifiersChangedEvent, window, cx| {
                // Modifier events follow keyboard focus. Also refresh the
                // hovered terminal while the file panel owns that focus.
                for (id, slot) in &this.slots {
                    if this
                        .tabs
                        .get(this.tab)
                        .is_some_and(|layout| layout.contains(*id))
                        && (!this.zoomed || *id == this.active)
                    {
                        slot.terminal
                            .update(cx, |terminal, cx| terminal.update_link_hover(window, cx));
                    }
                }
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if this.rename_tab.is_some() {
                        this.finish_rename_tab(false, window, cx);
                        cx.stop_propagation();
                    } else if this.tab_menu.take().is_some() {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<TabDrag>, _, cx| {
                // Capture runs parent-first: clear the old target before a matching child sets it.
                this.drop_hint = None;
                this.tab_drag = Some(event.drag(cx).clone());
            }))
            .on_drag_move(cx.listener(|this, _: &DragMoveEvent<PanelResize>, _, _| {
                this.resizing_panel = true;
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
            .on_action(cx.listener(Self::select_tab))
            .on_action(cx.listener(Self::shortcuts))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::settings))
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .child(
                TitleBar::new()
                    .h(px(theme::TITLE_BAR_HEIGHT))
                    .pl(px(theme::title_bar_padding(
                        cfg!(target_os = "macos"),
                        window.is_fullscreen(),
                    )))
                    .pr(px(6.))
                    .bg(rgb(0x000000))
                    .border_color(rgb(0x252525))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .size_full()
                            .min_w_0()
                            .child(icon_bar_button("menu", "menu", false).on_click(
                                cx.listener(|this, _, w, cx| this.shortcuts(&ShowShortcuts, w, cx)),
                            ))
                            .child(
                                div()
                                    .id("workspace-tabs")
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .flex_shrink_1()
                                    .max_w(px((full - 270.).max(196.)))
                                    .gap(px(3.))
                                    .h_full()
                                    .overflow_x_scroll()
                                    .track_scroll(&self.tab_scroll)
                                    .children(tabs),
                            )
                            .child(
                                bar_button("new-tab", "")
                                    .child(theme::icon(theme::ui("plus"), theme::TEXT_2, 15.))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            "New group  (Ctrl/Cmd+T)",
                                        )
                                        .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.new_terminal(&NewTerminal, w, cx)
                                    })),
                            )
                            .child(
                                bar_button("tab-list", "")
                                    .child(theme::icon(
                                        theme::ui("chevron-down"),
                                        theme::TEXT_2,
                                        14.,
                                    ))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new("All groups")
                                            .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                                        this.toggle_tab_list(event.position(), cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("tab-strip-tail")
                                    .flex_1()
                                    .h_full()
                                    .min_w(px(24.))
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
                                icon_bar_button("git-toggle", "git-branch", git_open)
                                    .when(changes > 0, |s| {
                                        s.w_auto().px(px(6.)).gap(px(4.)).child(
                                            div()
                                                .min_w(px(14.))
                                                .h(px(14.))
                                                .px(px(3.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(7.))
                                                .bg(rgb(0x2f2f2f))
                                                .text_size(px(9.5))
                                                .line_height(px(14.))
                                                .text_color(rgb(0xd6d6d6))
                                                .child(if changes > 99 {
                                                    "99+".to_string()
                                                } else {
                                                    changes.to_string()
                                                }),
                                        )
                                    })
                                    .when(git_open, |s| s.text_color(rgb(0xffffff)))
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
                                icon_bar_button(
                                    "files-toggle",
                                    "panel-right",
                                    visible && !git_open,
                                )
                                .when(visible && !git_open, |s| s.text_color(rgb(0xffffff)))
                                .when(visible && !git_open, |s| s.bg(rgb(0x1d1d1d)))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Files and review  (Ctrl+Shift+B)",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(
                                    |this, _, w, cx| this.toggle_files(&ToggleFiles, w, cx),
                                )),
                            ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .when(self.show_shortcuts, |s| s.child(menu))
            .children(tab_overlays)
            .when_some(self.project_dialog.as_ref(), |s, (dialog, _)| {
                s.child(dialog.clone())
            })
            .when_some(closing, |s, (dialog, left)| {
                s.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(left)
                        .child(dialog),
                )
            })
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
            if cfg!(target_os = "macos") {
                "cmd-d"
            } else if cfg!(windows) {
                "ctrl-d"
            } else {
                "ctrl-shift-d"
            },
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
            if cfg!(target_os = "macos") {
                "cmd-shift-h"
            } else {
                "ctrl-shift-h"
            },
            ShowShortcuts,
            None,
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else if cfg!(windows) {
                "alt-f4"
            } else {
                "ctrl-shift-q"
            },
            Quit,
            None,
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-,"
            } else {
                "ctrl-shift-,"
            },
            Settings,
            None,
        ),
    ]);
    if cfg!(windows) {
        cx.bind_keys([KeyBinding::new("ctrl-shift-d", SplitDown, None)]);
    }
    // Font zoom. GPUI turns a shifted digit or punctuation key into the
    // character it types, so Turkish Q Ctrl+Shift+0 arrives as ctrl-= and
    // Ctrl+Shift+4 as ctrl-+; US Ctrl+Shift+= arrives as ctrl-+.
    let zoom = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys((1..=9).map(|n| KeyBinding::new(&format!("{zoom}-{n}"), SelectTab(n - 1), None)));
    cx.bind_keys([
        KeyBinding::new(&format!("{zoom}-="), IncreaseFontSize, None),
        KeyBinding::new(&format!("{zoom}-+"), IncreaseFontSize, None),
        KeyBinding::new(&format!("{zoom}--"), DecreaseFontSize, None),
        KeyBinding::new(&format!("{zoom}-0"), ResetFontSize, None),
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
                MenuItem::action("Close file, panel or terminal", ClosePane),
            ]),
        ]);
        cx.bind_keys([
            KeyBinding::new("cmd-n", NewTerminal, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-shift-k", Checkpoint, None),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, SavedState};

    #[test]
    fn workspace_without_tab_metadata_is_still_readable() {
        let saved: SavedState =
            serde_json::from_str(r#"{"slots":[],"tabs":[{"Leaf":1}],"tab":0,"active":1}"#).unwrap();
        assert!(saved.tab_state.groups.is_empty());
        let mut state = saved.tab_state;
        state.sync(&saved.tabs, None);
        assert_eq!(state.groups[0].active, 1);
        assert!(state.groups[0].name.is_none());
    }

    #[test]
    fn workspace_round_trip_preserves_names_and_focused_splits() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, false);
        layout.resize_split(2, 0.85);
        let mut state = SavedState {
            tabs: vec![layout],
            active: 2,
            ..SavedState::default()
        };
        state.tab_state.sync(&state.tabs, None);
        state
            .tab_state
            .rename(state.tab_state.groups[0].id, "Derleme · Türkçe");
        state.tab_state.focus(2);
        let saved: SavedState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(saved.tab_state.groups[0].active, 2);
        assert_eq!(
            saved.tab_state.groups[0].name.as_deref(),
            Some("Derleme · Türkçe")
        );
        assert_eq!(saved.tabs[0].leaves(), vec![1, 2]);
        assert_eq!(saved.tabs, state.tabs);
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "app_tests.rs"]
mod interaction_tests;
