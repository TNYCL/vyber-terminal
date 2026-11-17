//! Quit and update inspect all windows before terminating any session.
use super::*;

struct WindowPlan {
    handle: AnyWindowHandle,
    content: WeakEntity<Vyber>,
    plan: ClosePlan,
}

pub(super) fn request(target: CloseTarget, origin: WindowId, cx: &mut App) {
    if crate::lifecycle::is_closing(cx) {
        return;
    }
    let windows = crate::lifecycle::windows(cx);
    if windows.iter().any(|w| {
        w.content.upgrade().is_none_or(|app| {
            let app = app.read(cx);
            app.close_pending || app.updating
        })
    }) {
        return;
    }
    let mut plans = Vec::new();
    let mut terminals = Vec::new();
    #[cfg(test)]
    let mut snapshots = Vec::new();
    for window in windows {
        let Some(app) = window.content.upgrade() else {
            return;
        };
        let app = app.read(cx);
        let ids = target
            .ids(&app.tabs, app.slots.keys().copied().collect())
            .unwrap();
        let plan = ClosePlan {
            target: target.clone(),
            sessions: app.close_sessions(&ids, cx),
            ids,
        };
        for id in &plan.ids {
            let terminal = app.slots[id].terminal.read(cx);
            if !terminal.exited {
                terminals.push((
                    window.handle.window_id(),
                    format!(
                        "Terminal {} · {} · {}",
                        terminal.id + 1,
                        terminal.title,
                        crate::platform::folder_name(&terminal.root)
                    ),
                    terminal.process.clone(),
                ));
            }
        }
        #[cfg(test)]
        snapshots.push((window.handle.window_id(), app.process_snapshot.clone()));
        plans.push(WindowPlan {
            handle: window.handle,
            content: window.content,
            plan,
        });
    }
    crate::lifecycle::set_closing(true, cx);
    let inspection = cx.background_executor().spawn(async move {
        let snapshot = crate::processes::ProcessSnapshot::capture();
        terminals
            .into_iter()
            .filter_map(|(_window, label, process)| {
                #[cfg(test)]
                let snapshot = snapshots
                    .iter()
                    .find(|(window, _)| *window == _window)
                    .and_then(|(_, snapshot)| snapshot.as_ref())
                    .map(Ok)
                    .unwrap_or(snapshot.as_ref());
                #[cfg(not(test))]
                let snapshot = snapshot.as_ref();
                match snapshot
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
    cx.spawn(async move |cx| {
        let running = inspection.await;
        let is_valid = cx.update(|cx| valid(&plans, cx));
        if !is_valid { cx.update(|cx| cancel(origin, cx)); return; }
        let confirmed = if running.is_empty() { true } else {
            let answer = cx.update(|cx| {
                let window = plans.iter().find(|w| w.handle.window_id() == origin)?;
                window.handle.update(cx, |_, window, cx| {
                    let (message, button) = if matches!(target, CloseTarget::Update) {
                        ("Restart Vyber to update and stop running processes?", "Restart and update")
                    } else { ("Quit Vyber and stop running processes?", "Quit Vyber") };
                    window.prompt(PromptLevel::Warning, message,
                        Some(&format!("Running terminal sessions in all windows:\n\n{}\n\nClosing will interrupt these sessions.", running.join("\n"))),
                        &[PromptButton::cancel("Cancel"), PromptButton::new(button)], cx)
                }).ok()
            });
            match answer { Some(answer) => matches!(answer.await, Ok(1)), None => false }
        };
        cx.update(|cx| {
            if !confirmed || !valid(&plans, cx) { cancel(origin, cx); return; }
            for window in &plans {
                if let Some(app) = window.content.upgrade() { app.read(cx).persist(cx); }
            }
            if matches!(target, CloseTarget::Update) {
                if let Some(window) = plans.iter().find(|w| w.handle.window_id() == origin)
                    && let Some(app) = window.content.upgrade() {
                    app.update(cx, |app, cx| app.finish_update(cx));
                } else { cancel(origin, cx); }
            } else { crate::lifecycle::quit_application(cx); }
        });
    }).detach();
}

fn valid(plans: &[WindowPlan], cx: &App) -> bool {
    let windows = crate::lifecycle::windows(cx);
    windows.len() == plans.len()
        && plans.iter().all(|w| {
            windows.iter().any(|live| live.handle == w.handle)
                && w.content
                    .upgrade()
                    .is_some_and(|app| app.read(cx).plan_valid(&w.plan, cx))
        })
}
fn cancel(origin: WindowId, cx: &mut App) {
    crate::lifecycle::set_closing(false, cx);
    if let Some(window) = crate::lifecycle::windows(cx)
        .iter()
        .find(|w| w.handle.window_id() == origin)
        && let Some(app) = window.content.upgrade()
    {
        app.update(cx, |_, cx| cx.notify());
    }
}
