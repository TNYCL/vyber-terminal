use super::{
    CloseTarget, NewTerminal, Quit, SavedSlot, SavedState, Vyber, WorkspaceLaunch, bind_keys,
};
use crate::{layout::Layout, workspace};
use crate::{lifecycle, processes::ProcessSnapshot};
use gpui::{AnyWindowHandle, App, Context, QuitMode, WeakEntity, Window};
use gpui::{TestAppContext, VisualTestContext};
use std::path::PathBuf;

struct Harness {
    window: AnyWindowHandle,
    view: WeakEntity<Vyber>,
    root: PathBuf,
    _data: workspace::TestDataDir,
    _directory: tempfile::TempDir,
}

fn setup(cx: &mut TestAppContext) -> Harness {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let data = workspace::TestDataDir::set(directory.path().join("data"));
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(crate::config::Config::load());
        crate::theme::apply(cx);
        cx.set_reduce_motion(true);
        cx.set_quit_mode(QuitMode::Explicit);
        bind_keys(cx);
        crate::terminal::bind_keys(cx);
        crate::browser::bind_keys(cx);
        lifecycle::init(root.clone(), cx);
        let window = lifecycle::open_workspace(WorkspaceLaunch::Empty, cx).unwrap();
        let view = lifecycle::workspace_content(cx).unwrap().downgrade();
        (window, view)
    });
    Harness {
        window,
        view,
        root,
        _data: data,
        _directory: directory,
    }
}

fn update<R>(
    h: &Harness,
    cx: &mut TestAppContext,
    callback: impl FnOnce(&mut Vyber, &mut Window, &mut Context<Vyber>) -> R,
) -> R {
    h.window
        .update(cx, |_, window, cx| {
            h.view
                .update(cx, |app, cx| callback(app, window, cx))
                .unwrap()
        })
        .unwrap()
}

fn read<R>(h: &Harness, cx: &TestAppContext, callback: impl FnOnce(&Vyber, &App) -> R) -> R {
    h.view.upgrade().unwrap().read_with(cx, callback)
}

fn add(h: &Harness, cx: &mut TestAppContext, split: bool) -> usize {
    update(h, cx, |app, window, cx| {
        app.add_terminal(h.root.clone(), split, false, window, cx);
        app.active
    })
}

fn agents(h: &Harness, cx: &mut TestAppContext, ids: &[usize]) {
    update(h, cx, |app, _, cx| {
        // Gerçek Codex başlatmadan onun süreç ağacını kapanma akışına veririz.
        let mut rows = String::new();
        for (id, slot) in &app.slots {
            let pid = slot.terminal.read(cx).process.pid.unwrap();
            rows.push_str(&format!("{pid} 1 S /bin/sh\n"));
            if ids.contains(id) {
                rows.push_str(&format!(
                    "{} {pid} S /usr/bin/codex\n",
                    u32::MAX - *id as u32
                ));
            }
        }
        app.process_snapshot = Some(ProcessSnapshot::parse(&rows).unwrap());
    });
}

fn ids(h: &Harness, cx: &TestAppContext) -> Vec<usize> {
    read(h, cx, |app, _| {
        let mut ids = app.slots.keys().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    })
}

#[gpui_kit::test]
fn cmd_w_on_empty_split_terminal_preserves_the_agent_sibling(cx: &mut TestAppContext) {
    let h = setup(cx);
    let left = add(&h, cx, false);
    let right = add(&h, cx, true);
    agents(&h, cx, &[left]);
    let left_pid = read(&h, cx, |app, cx| {
        app.slots[&left].terminal.read(cx).process.pid
    });
    update(&h, cx, |app, window, cx| app.focus_pane(right, window, cx));
    cx.simulate_keystrokes(h.window, "cmd-w");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(ids(&h, cx), vec![left]);
    assert_eq!(
        read(&h, cx, |app, _| app.tabs.clone()),
        vec![Layout::Leaf(left)]
    );
    assert_eq!(
        read(&h, cx, |app, cx| app.slots[&left]
            .terminal
            .read(cx)
            .process
            .pid),
        left_pid
    );
    assert!(!cx.read(lifecycle::quit_requested));
}

#[gpui_kit::test]
fn agent_confirmation_can_cancel_and_remains_bound_when_focus_changes(cx: &mut TestAppContext) {
    let h = setup(cx);
    let left = add(&h, cx, false);
    let right = add(&h, cx, true);
    agents(&h, cx, &[left]);
    update(&h, cx, |app, window, cx| app.focus_pane(left, window, cx));
    cx.simulate_keystrokes(h.window, "cmd-w");
    cx.run_until_parked();
    assert!(cx.pending_prompt().unwrap().0.contains("this terminal"));
    assert!(cx.pending_prompt().unwrap().1.contains("codex"));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx), vec![left, right]);
    assert!(!read(&h, cx, |app, _| app.close_pending));
    cx.simulate_keystrokes(h.window, "cmd-w");
    cx.run_until_parked();
    update(&h, cx, |app, window, cx| app.focus_pane(right, window, cx));
    cx.simulate_prompt_answer("Close terminal");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx), vec![right]);
    assert_eq!(read(&h, cx, |app, _| app.active), right);
}

#[gpui_kit::test]
fn explicit_group_close_and_last_terminal_leave_a_usable_empty_window(cx: &mut TestAppContext) {
    let h = setup(cx);
    let left = add(&h, cx, false);
    let right = add(&h, cx, true);
    let other = add(&h, cx, false);
    agents(&h, cx, &[left, right]);
    update(&h, cx, |app, window, cx| app.close_tab(0, window, cx));
    cx.run_until_parked();
    let prompt = cx.pending_prompt().unwrap();
    assert!(prompt.0.contains("this group"));
    assert_eq!(prompt.1.matches("codex").count(), 2);
    cx.simulate_prompt_answer("Close group");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx), vec![other]);
    assert_eq!(read(&h, cx, |app, _| app.active), other);
    cx.simulate_keystrokes(h.window, "cmd-w");
    cx.run_until_parked();
    assert!(ids(&h, cx).is_empty());
    assert_eq!(cx.read(|cx| cx.windows().len()), 1);
    assert!(!cx.read(lifecycle::quit_requested));
    let saved: SavedState = serde_json::from_slice(
        &std::fs::read(workspace::data_dir().join("workspace.json")).unwrap(),
    )
    .unwrap();
    assert!(saved.slots.is_empty() && saved.tabs.is_empty());
    cx.simulate_keystrokes(h.window, "cmd-w ctrl-tab ctrl-shift-tab cmd-]");
    cx.simulate_keystrokes(h.window, "cmd-t");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx).len(), 1);
    assert!(!ids(&h, cx).contains(&other));
}

#[gpui_kit::test]
fn native_window_close_can_cancel_and_reopen_without_resurrecting_terminals(
    cx: &mut TestAppContext,
) {
    let h = setup(cx);
    let id = add(&h, cx, false);
    agents(&h, cx, &[id]);
    let mut visual = VisualTestContext::from_window(h.window, cx);
    assert!(!visual.simulate_close());
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx), vec![id]);
    assert!(!visual.simulate_close());
    cx.run_until_parked();
    cx.simulate_prompt_answer("Close window");
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.windows().is_empty()));
    assert!(h.view.upgrade().is_none());
    assert!(!cx.read(lifecycle::quit_requested));
    cx.update(|cx| lifecycle::show_workspace(cx).unwrap());
    assert_eq!(cx.read(|cx| cx.windows().len()), 1);
    let reopened = cx.read(lifecycle::workspace_content).unwrap();
    assert!(reopened.read_with(cx, |app, _| app.slots.is_empty()));
    cx.update(|cx| lifecycle::show_workspace(cx).unwrap());
    assert_eq!(cx.read(|cx| cx.windows().len()), 1);
}

#[gpui_kit::test]
fn cmd_q_checks_all_terminals_and_cancel_preserves_the_workspace(cx: &mut TestAppContext) {
    let h = setup(cx);
    let left = add(&h, cx, false);
    let right = add(&h, cx, true);
    agents(&h, cx, &[left]);
    cx.simulate_keystrokes(h.window, "cmd-q");
    cx.run_until_parked();
    assert!(cx.pending_prompt().unwrap().0.contains("Quit Vyber"));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx), vec![left, right]);
    assert!(!cx.read(lifecycle::quit_requested));
    cx.simulate_keystrokes(h.window, "cmd-q");
    cx.run_until_parked();
    cx.simulate_prompt_answer("Quit Vyber");
    cx.run_until_parked();
    assert!(cx.read(lifecycle::quit_requested));
    let saved: SavedState = serde_json::from_slice(
        &std::fs::read(workspace::data_dir().join("workspace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.slots.len(), 2);
}

#[gpui_kit::test]
fn shell_exit_leaves_an_empty_workspace_and_windowless_menu_actions_work(cx: &mut TestAppContext) {
    let h = setup(cx);
    let id = add(&h, cx, false);
    update(&h, cx, |app, window, cx| {
        app.slots[&id]
            .terminal
            .update(cx, |terminal, _| terminal.exited = true);
        app.poll(window, cx);
    });
    assert!(ids(&h, cx).is_empty());
    assert!(!cx.read(lifecycle::quit_requested));
    update(&h, cx, |app, window, cx| {
        app.request_close(CloseTarget::Window, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.windows().is_empty()));
    cx.update(|cx| cx.dispatch_action(&NewTerminal));
    cx.run_until_parked();
    let app = cx.read(lifecycle::workspace_content).unwrap();
    assert_eq!(app.read_with(cx, |app, _| app.slots.len()), 1);
    let window = cx.read(|cx| cx.windows()[0]);
    window
        .update(cx, |_, window, cx| {
            app.update(cx, |app, cx| {
                let ids = app.slots.keys().copied().collect::<Vec<_>>();
                for id in &ids {
                    app.slots[id]
                        .terminal
                        .update(cx, |terminal, _| terminal.exited = true);
                }
                app.request_close(CloseTarget::Window, window, cx);
            })
        })
        .unwrap();
    drop(app);
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.windows().is_empty()));
    cx.update(|cx| cx.dispatch_action(&Quit));
    assert!(cx.read(lifecycle::quit_requested));
}

#[gpui_kit::test]
fn cmd_w_closes_the_file_then_panel_before_asking_to_close_the_agent(cx: &mut TestAppContext) {
    let h = setup(cx);
    let id = add(&h, cx, false);
    agents(&h, cx, &[id]);
    update(&h, cx, |app, window, cx| {
        app.slots[&id].browser.update(cx, |browser, cx| {
            browser.test_document(h.root.join("file.txt"), "original\n", window, cx);
        });
    });
    cx.simulate_keystrokes(h.window, "cmd-w");
    assert!(!cx.has_pending_prompt());
    assert!(read(&h, cx, |app, cx| {
        let browser = app.slots[&id].browser.read(cx);
        browser.visible && browser.state(cx).docs.is_empty()
    }));
    cx.simulate_keystrokes(h.window, "cmd-w");
    assert!(!read(&h, cx, |app, cx| app.slots[&id]
        .browser
        .read(cx)
        .visible));
    assert_eq!(ids(&h, cx), vec![id]);
    cx.simulate_keystrokes(h.window, "cmd-w");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn unsaved_file_blocks_file_and_terminal_closure_and_keeps_its_draft(cx: &mut TestAppContext) {
    let h = setup(cx);
    let id = add(&h, cx, false);
    agents(&h, cx, &[id]);
    update(&h, cx, |app, window, cx| {
        app.slots[&id].browser.update(cx, |browser, cx| {
            browser.test_document(h.root.join("file.txt"), "original\n", window, cx);
        });
    });
    cx.simulate_keystrokes(h.window, "cmd-a");
    let mut visual = VisualTestContext::from_window(h.window, cx);
    visual.simulate_input("changed\n");
    cx.run_until_parked();
    assert!(read(&h, cx, |app, cx| app.slots[&id]
        .browser
        .read(cx)
        .has_dirty()));
    cx.simulate_keystrokes(h.window, "cmd-w");
    assert!(!cx.has_pending_prompt());
    assert_eq!(
        read(&h, cx, |app, cx| app.slots[&id]
            .browser
            .read(cx)
            .state(cx)
            .docs
            .len()),
        1
    );
    update(&h, cx, |app, window, cx| app.close_pane(id, window, cx));
    assert!(!cx.has_pending_prompt());
    assert_eq!(ids(&h, cx), vec![id]);
    let bytes = std::fs::read(workspace::data_dir().join("workspace.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        saved["slots"][0]["browser"]["docs"][0]["draft"],
        "changed\n"
    );
    cx.simulate_keystrokes(h.window, "cmd-q");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Quit Vyber");
    cx.run_until_parked();
    assert!(cx.read(lifecycle::quit_requested));
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace::data_dir().join("workspace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        saved["slots"][0]["browser"]["docs"][0]["draft"],
        "changed\n"
    );
}

#[gpui_kit::test]
fn saved_empty_workspace_stays_empty_on_a_cold_restore(cx: &mut TestAppContext) {
    let h = setup(cx);
    update(&h, cx, |app, window, cx| {
        app.request_close(CloseTarget::Window, window, cx);
    });
    cx.run_until_parked();
    cx.update(|cx| lifecycle::open_workspace(WorkspaceLaunch::Restore, cx).unwrap());
    let view = cx.read(lifecycle::workspace_content).unwrap();
    assert!(view.read_with(cx, |app, _| app.slots.is_empty() && app.tabs.is_empty()));
    assert!(!cx.read(lifecycle::quit_requested));
}

#[gpui_kit::test]
fn cold_restore_keeps_remaining_groups_and_excludes_the_closed_split_terminal(
    cx: &mut TestAppContext,
) {
    let h = setup(cx);
    let left = add(&h, cx, false);
    let right = add(&h, cx, true);
    let other = add(&h, cx, false);
    agents(&h, cx, &[]);
    update(&h, cx, |app, window, cx| app.focus_pane(right, window, cx));
    cx.simulate_keystrokes(h.window, "cmd-w");
    cx.run_until_parked();
    assert_eq!(ids(&h, cx), vec![left, other]);
    let expected = vec![Layout::Leaf(left), Layout::Leaf(other)];
    assert_eq!(read(&h, cx, |app, _| app.tabs.clone()), expected);
    // Süreç çıkışındaki pencere temizliğini simüle eder; bu bir kapatma isteği değildir.
    h.window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(h.view.upgrade().is_none());
    cx.update(|cx| lifecycle::open_workspace(WorkspaceLaunch::Restore, cx).unwrap());
    let restored = cx.read(lifecycle::workspace_content).unwrap();
    restored.read_with(cx, |app, _| {
        assert_eq!(app.tabs, expected);
        assert_eq!(app.slots.len(), 2);
        assert!(app.slots.contains_key(&left) && app.slots.contains_key(&other));
        assert!(!app.slots.contains_key(&right));
        assert_eq!(app.active, left);
        assert_eq!(app.next, other + 1);
    });
    let saved: SavedState = serde_json::from_slice(
        &std::fs::read(workspace::data_dir().join("workspace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.tabs, expected);
    assert_eq!(saved.slots.len(), 2);
}

#[gpui_kit::test]
fn restore_prunes_high_numbered_terminals_whose_folders_no_longer_exist(cx: &mut TestAppContext) {
    let h = setup(cx);
    update(&h, cx, |app, window, cx| {
        app.request_close(CloseTarget::Window, window, cx)
    });
    cx.run_until_parked();
    let mut group = Layout::Leaf(0);
    group.split(0, 99, false);
    let saved = SavedState {
        slots: vec![
            SavedSlot {
                id: 0,
                root: h.root.clone(),
                browser: Default::default(),
            },
            SavedSlot {
                id: 99,
                root: h.root.join("deleted-folder"),
                browser: Default::default(),
            },
        ],
        tabs: vec![group],
        active: 99,
        ..Default::default()
    };
    std::fs::write(
        workspace::data_dir().join("workspace.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let window = cx.update(|cx| lifecycle::open_workspace(WorkspaceLaunch::Restore, cx).unwrap());
    let restored = cx.read(lifecycle::workspace_content).unwrap();
    restored.read_with(cx, |app, _| {
        assert_eq!(app.slots.len(), 1);
        assert_eq!(app.tabs, vec![Layout::Leaf(0)]);
        assert_eq!(app.active, 0);
    });
    cx.simulate_keystrokes(window, "cmd-] ctrl-tab");
    assert!(!cx.has_pending_prompt());
}
