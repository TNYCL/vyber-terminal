use crate::app::{
    CheckForUpdates, NewTerminal, OpenFolder, Quit, SavedState, Settings, Vyber, WorkspaceLaunch,
};
use gpui::*;
use gpui_kit::component::TitleBar;
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    io::Write,
    path::PathBuf,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SavedBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
impl SavedBounds {
    pub(crate) fn capture(window: &Window) -> Self {
        let bounds = window.window_bounds().get_bounds();
        Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        }
    }
    fn bounds(&self) -> Option<Bounds<Pixels>> {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            .then(|| {
                Bounds::new(
                    point(px(self.x), px(self.y)),
                    size(px(self.width.max(160.)), px(self.height.max(200.))),
                )
            })
    }
}

#[derive(Clone)]
pub(crate) struct WorkspaceWindow {
    pub(crate) handle: AnyWindowHandle,
    pub(crate) content: WeakEntity<Vyber>,
}
struct ApplicationWindows {
    root: PathBuf,
    windows: Vec<WorkspaceWindow>,
    saved: RefCell<Vec<(WindowId, SavedState)>>,
    ignored: RefCell<HashSet<WindowId>>,
    saved_bytes: RefCell<Vec<u8>>,
    restoring: Cell<bool>,
    closing: bool,
    _closed: Subscription,
    #[cfg(test)]
    quit_requested: bool,
}
impl Global for ApplicationWindows {}

pub fn init(root: PathBuf, cx: &mut App) {
    crate::runtime::init(cx);
    let closed = cx.on_window_closed(|cx, id| {
        cx.global_mut::<ApplicationWindows>()
            .windows
            .retain(|w| w.handle.window_id() != id);
        crate::runtime::entity(cx).update(cx, |runtime, cx| runtime.remove_window(id, cx));
    });
    cx.set_global(ApplicationWindows {
        root,
        windows: Vec::new(),
        saved: RefCell::new(Vec::new()),
        ignored: RefCell::new(HashSet::new()),
        saved_bytes: RefCell::new(Vec::new()),
        restoring: Cell::new(false),
        closing: false,
        _closed: closed,
        #[cfg(test)]
        quit_requested: false,
    });
    cx.on_action(|action: &NewTerminal, cx| {
        if show_workspace(cx).is_ok() {
            with_workspace(cx, |app, window, cx| app.new_terminal(action, window, cx));
        }
    });
    cx.on_action(|action: &OpenFolder, cx| {
        if show_workspace(cx).is_ok() {
            with_workspace(cx, |app, window, cx| app.open_folder(action, window, cx));
        }
    });
    cx.on_action(|action: &Settings, cx| {
        if show_workspace(cx).is_ok() {
            with_workspace(cx, |app, window, cx| app.settings(action, window, cx));
        }
    });
    cx.on_action(|action: &CheckForUpdates, cx| {
        if show_workspace(cx).is_ok() {
            with_workspace(cx, |app, window, cx| {
                app.check_for_updates(action, window, cx)
            });
        }
    });
    cx.on_action(|action: &Quit, cx| {
        if !with_workspace(cx, |app, window, cx| app.quit(action, window, cx)) {
            quit_application(cx);
        }
    });
}

pub(crate) fn windows(cx: &App) -> Vec<WorkspaceWindow> {
    cx.global::<ApplicationWindows>().windows.clone()
}
fn selected(cx: &App) -> Option<WorkspaceWindow> {
    let windows = &cx.global::<ApplicationWindows>().windows;
    cx.active_window()
        .and_then(|active| windows.iter().find(|w| w.handle == active))
        .or_else(|| windows.last())
        .cloned()
}
fn with_workspace(
    cx: &mut App,
    action: impl FnOnce(&mut Vyber, &mut Window, &mut Context<Vyber>),
) -> bool {
    let Some(workspace) = selected(cx) else {
        return false;
    };
    let Some(content) = workspace.content.upgrade() else {
        return false;
    };
    workspace
        .handle
        .update(cx, |_, window, cx| {
            content.update(cx, |app, cx| action(app, window, cx))
        })
        .is_ok()
}

pub fn open_workspace(launch: WorkspaceLaunch, cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
    if !matches!(launch, WorkspaceLaunch::Restore) {
        return open_at(launch, None, None, cx);
    }
    #[cfg(not(test))]
    let restore_allowed = std::env::args_os().nth(1).is_none();
    #[cfg(test)]
    let restore_allowed = true;
    let updated = crate::update::take_relaunched();
    let saved = (restore_allowed
        && (updated.is_some() || crate::config::Config::load().restore_workspace))
        .then(|| std::fs::read(crate::workspace::data_dir().join("workspace.json")).ok())
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<SavedState>(&bytes).ok());
    let Some(mut saved) = saved else {
        return open_at(WorkspaceLaunch::Restore, None, None, cx);
    };
    let additional = std::mem::take(&mut saved.windows);
    cx.global::<ApplicationWindows>().restoring.set(true);
    let result = open_at(WorkspaceLaunch::Saved(saved), None, None, cx);
    if result.is_ok() {
        for state in additional {
            if let Err(error) = open_at(WorkspaceLaunch::Saved(state), None, None, cx) {
                log::warn!("Restore window: {error:#}");
            }
        }
    }
    cx.global::<ApplicationWindows>().restoring.set(false);
    write_saved(cx);
    if updated.is_some_and(|from| from != env!("CARGO_PKG_VERSION")) {
        with_workspace(cx, |app, _, cx| {
            app.updated_notice(cx);
        });
    }
    result
}

pub(crate) fn open_at(
    launch: WorkspaceLaunch,
    bounds: Option<Bounds<Pixels>>,
    display: Option<DisplayId>,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let root = cx.global::<ApplicationWindows>().root.clone();
    let saved_bounds = match &launch {
        WorkspaceLaunch::Saved(saved) => saved.bounds.as_ref().and_then(SavedBounds::bounds),
        _ => None,
    };
    let bounds = bounds
        .or(saved_bounds)
        .map(|bounds| visible_bounds(bounds, display, cx))
        .unwrap_or_else(|| Bounds::centered(None, size(px(1380.), px(850.)), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        window_min_size: Some(size(px(160.), px(200.))),
        app_id: Some("dev.vyber.terminal".into()),
        ..TitleBar::window_options()
    };
    let (handle, content) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Vyber");
        cx.new(|cx| Vyber::new(root, launch, window, cx))
    })?;
    cx.global_mut::<ApplicationWindows>()
        .windows
        .push(WorkspaceWindow {
            handle,
            content: content.downgrade(),
        });
    content.read(cx).persist(cx);
    cx.activate(true);
    Ok(handle)
}

fn visible_bounds(
    mut bounds: Bounds<Pixels>,
    display: Option<DisplayId>,
    cx: &App,
) -> Bounds<Pixels> {
    let displays = cx.displays();
    let target = display
        .and_then(|id| displays.iter().find(|d| d.id() == id))
        .or_else(|| {
            displays
                .iter()
                .find(|d| d.bounds().contains(&bounds.center()))
        })
        .or_else(|| displays.first());
    if let Some(target) = target {
        let area = target.visible_bounds();
        bounds.size.width = bounds.size.width.min(area.size.width);
        bounds.size.height = bounds.size.height.min(area.size.height);
        bounds.origin.x = bounds
            .origin
            .x
            .max(area.left())
            .min(area.right() - bounds.size.width);
        bounds.origin.y = bounds
            .origin
            .y
            .max(area.top())
            .min(area.bottom() - bounds.size.height);
    }
    bounds
}

pub(crate) fn save_window(id: WindowId, state: SavedState, cx: &App) {
    let app = cx.global::<ApplicationWindows>();
    if app.ignored.borrow().contains(&id)
        || !cx.windows().iter().any(|window| window.window_id() == id)
    {
        return;
    }
    let mut saved = app.saved.borrow_mut();
    if let Some((_, old)) = saved.iter_mut().find(|(other, _)| *other == id) {
        *old = state;
    } else {
        saved.push((id, state));
    }
    drop(saved);
    write_saved(cx);
}
pub(crate) fn begin_transfer(cx: &App) {
    cx.global::<ApplicationWindows>().restoring.set(true);
}
pub(crate) fn end_transfer(cx: &App) {
    cx.global::<ApplicationWindows>().restoring.set(false);
    write_saved(cx);
}
pub(crate) fn forget_window(id: WindowId, cx: &App) {
    let app = cx.global::<ApplicationWindows>();
    app.ignored.borrow_mut().insert(id);
    app.saved.borrow_mut().retain(|(other, _)| *other != id);
    write_saved(cx);
}
fn write_saved(cx: &App) {
    let app = cx.global::<ApplicationWindows>();
    if app.restoring.get() {
        return;
    }
    let mut states = app
        .saved
        .borrow()
        .iter()
        .map(|(_, state)| state.clone())
        .collect::<Vec<_>>();
    let mut state = if states.is_empty() {
        SavedState::default()
    } else {
        states.remove(0)
    };
    state.windows = states;
    let Ok(bytes) = serde_json::to_vec(&state) else {
        return;
    };
    if *app.saved_bytes.borrow() == bytes {
        return;
    }
    let directory = crate::workspace::data_dir();
    let result = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(&directory)?;
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(directory.join("workspace.json"))?;
        Ok(())
    })();
    match result {
        Ok(()) => *app.saved_bytes.borrow_mut() = bytes,
        Err(error) => log::warn!("Save workspace: {error:#}"),
    }
}

pub fn show_workspace(cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<ApplicationWindows>() {
        return Ok(());
    }
    if with_workspace(cx, |_, window, cx| {
        window.activate_window();
        cx.activate(true);
    }) {
        return Ok(());
    }
    open_workspace(WorkspaceLaunch::Empty, cx)?;
    Ok(())
}

pub(crate) fn is_closing(cx: &App) -> bool {
    cx.global::<ApplicationWindows>().closing
}
pub(crate) fn set_closing(value: bool, cx: &mut App) {
    cx.global_mut::<ApplicationWindows>().closing = value;
}
pub fn quit_application(cx: &mut App) {
    #[cfg(test)]
    {
        cx.global_mut::<ApplicationWindows>().quit_requested = true;
    }
    cx.quit();
}
#[cfg(test)]
#[allow(dead_code)]
pub fn workspace_content(cx: &App) -> Option<Entity<Vyber>> {
    selected(cx)?.content.upgrade()
}
#[cfg(test)]
#[allow(dead_code)]
pub fn quit_requested(cx: &App) -> bool {
    cx.global::<ApplicationWindows>().quit_requested
}
