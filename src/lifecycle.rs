use crate::app::{
    CheckForUpdates, NewTerminal, OpenFolder, Quit, Settings, Vyber, WorkspaceLaunch,
};
use gpui::*;
use gpui_kit::component::TitleBar;
use std::path::PathBuf;

#[derive(Clone)]
struct WorkspaceWindow {
    handle: AnyWindowHandle,
    content: WeakEntity<Vyber>,
}

struct ApplicationWindows {
    root: PathBuf,
    workspace: Option<WorkspaceWindow>,
    _closed: Subscription,
    #[cfg(test)]
    quit_requested: bool,
}
impl Global for ApplicationWindows {}

pub fn init(root: PathBuf, cx: &mut App) {
    let closed = cx.on_window_closed(|cx, id| {
        let state = cx.global_mut::<ApplicationWindows>();
        if state
            .workspace
            .as_ref()
            .is_some_and(|window| window.handle.window_id() == id)
        {
            state.workspace = None;
        }
    });
    cx.set_global(ApplicationWindows {
        root,
        workspace: None,
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
            // Penceresiz macOS uygulaması da menüden normal çıkabilmelidir.
            quit_application(cx);
        }
    });
}

fn with_workspace(
    cx: &mut App,
    action: impl FnOnce(&mut Vyber, &mut Window, &mut Context<Vyber>),
) -> bool {
    let workspace = cx.global::<ApplicationWindows>().workspace.clone();
    if let Some(workspace) = workspace
        && let Some(content) = workspace.content.upgrade()
        && workspace
            .handle
            .update(cx, |_, window, cx| {
                content.update(cx, |app, cx| action(app, window, cx))
            })
            .is_ok()
    {
        true
    } else {
        cx.global_mut::<ApplicationWindows>().workspace = None;
        false
    }
}

pub fn open_workspace(launch: WorkspaceLaunch, cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
    let root = cx.global::<ApplicationWindows>().root.clone();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1380.), px(850.)),
            cx,
        ))),
        window_min_size: Some(size(px(900.), px(550.))),
        app_id: Some("dev.vyber.terminal".into()),
        ..TitleBar::window_options()
    };
    let (handle, content) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Vyber");
        cx.new(|cx| Vyber::new(root, launch, window, cx))
    })?;
    cx.global_mut::<ApplicationWindows>().workspace = Some(WorkspaceWindow {
        handle,
        content: content.downgrade(),
    });
    cx.activate(true);
    Ok(handle)
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
    // Kapatılmış terminaller yeniden açılmaz; yeni pencere boş başlayabilir.
    open_workspace(WorkspaceLaunch::Empty, cx)?;
    Ok(())
}

pub fn quit_application(cx: &mut App) {
    #[cfg(test)]
    if cx.has_global::<ApplicationWindows>() {
        cx.global_mut::<ApplicationWindows>().quit_requested = true;
    }
    cx.quit();
}

#[cfg(test)]
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "Used by macOS interaction tests.")
)]
pub fn workspace_content(cx: &App) -> Option<Entity<Vyber>> {
    cx.global::<ApplicationWindows>()
        .workspace
        .as_ref()?
        .content
        .upgrade()
}

#[cfg(test)]
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "Used by macOS interaction tests.")
)]
pub fn quit_requested(cx: &App) -> bool {
    cx.global::<ApplicationWindows>().quit_requested
}
