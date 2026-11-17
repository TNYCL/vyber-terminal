use super::*;
use crate::lifecycle;
use core::prelude::v1::test;
use gpui_kit::test::TestWindowExt;

struct Harness {
    root: PathBuf,
    _data: crate::workspace::TestDataDir,
    _directory: tempfile::TempDir,
}
fn setup(cx: &mut TestAppContext) -> Harness {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let data = crate::workspace::TestDataDir::set(directory.path().join("data"));
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Config {
            toolbelt: false,
            git_autofetch: false,
            ..Config::default()
        });
        theme::apply(cx);
        cx.set_reduce_motion(true);
        cx.set_quit_mode(QuitMode::Explicit);
        bind_keys(cx);
        crate::terminal::bind_keys(cx);
        crate::browser::bind_keys(cx);
        lifecycle::init(root.clone(), cx);
    });
    Harness {
        root,
        _data: data,
        _directory: directory,
    }
}
fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Vyber>) {
    let handle = cx.update(|cx| lifecycle::open_workspace(WorkspaceLaunch::Empty, cx).unwrap());
    let app = cx.read(|cx| {
        lifecycle::windows(cx)
            .iter()
            .find(|w| w.handle == handle)
            .unwrap()
            .content
            .upgrade()
            .unwrap()
    });
    (handle, app)
}
fn change<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<Vyber>,
    f: impl FnOnce(&mut Vyber, &mut Window, &mut Context<Vyber>) -> R,
) -> R {
    handle
        .update(cx, |_, window, cx| {
            app.update(cx, |app, cx| f(app, window, cx))
        })
        .unwrap()
}
fn add(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<Vyber>,
    root: &std::path::Path,
    split: bool,
) -> usize {
    change(cx, handle, app, |app, window, cx| {
        app.add_terminal(root.to_owned(), split, false, window, cx);
        app.active
    })
}
fn drag(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<Vyber>,
    id: usize,
    group: bool,
) -> TabDrag {
    change(cx, handle, app, |app, _, cx| {
        app.drag_value(id, group, "test".into(), cx)
    })
}
fn saved() -> SavedState {
    serde_json::from_slice(
        &std::fs::read(crate::workspace::data_dir().join("workspace.json")).unwrap(),
    )
    .unwrap()
}

#[gpui_kit::test]
fn group_moves_between_windows_without_restarting_terminals_or_losing_layout(
    cx: &mut TestAppContext,
) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let first = add(cx, source, &app, &h.root, false);
    let second = add(cx, source, &app, &h.root, true);
    change(cx, source, &app, |app, _, _| {
        app.tab_state.rename(app.tab_state.groups[0].id, "Backend");
        app.tabs[0].resize_split(second, 0.7);
    });
    let (layout, terminals) = app.read_with(cx, |app, _| {
        (
            app.tabs[0].clone(),
            vec![
                app.slots[&first].terminal.clone(),
                app.slots[&second].terminal.clone(),
            ],
        )
    });
    let pids = terminals
        .iter()
        .map(|t| t.read_with(cx, |t, _| t.process.pid))
        .collect::<Vec<_>>();
    let (target, destination) = open(cx);
    let value = drag(cx, source, &app, first, true);
    change(cx, target, &destination, |app, window, cx| {
        app.drop_on_tab(&value, None, true, window, cx)
    });
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.slots.is_empty()));
    destination.read_with(cx, |app, cx| {
        assert_eq!(app.tabs, [layout]);
        assert_eq!(app.tab_state.groups[0].name.as_deref(), Some("Backend"));
        assert_eq!(app.active, second);
        for (index, id) in [first, second].into_iter().enumerate() {
            assert_eq!(app.slots[&id].terminal, terminals[index]);
            assert_eq!(app.slots[&id].terminal.read(cx).process.pid, pids[index]);
        }
    });
    assert_eq!(saved().windows.len(), 1);
    assert_eq!(saved().windows[0].slots.len(), 2);
    assert_eq!(add(cx, source, &app, &h.root, false), second + 1);
}

#[gpui_kit::test]
fn outside_mouse_release_opens_a_real_workspace_window(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let id = add(cx, source, &app, &h.root, false);
    let terminal = app.read_with(cx, |app, _| app.slots[&id].terminal.clone());
    source
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            let bounds = app
                .read(cx)
                .drop_regions
                .borrow()
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Tab(_)).then_some(*bounds)
                })
                .unwrap();
            window.drag(bounds.center(), point(px(-260.), px(240.)), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let windows = cx.read(lifecycle::windows);
    assert_eq!(windows.len(), 2);
    let destination = windows
        .iter()
        .find(|w| w.handle != source)
        .unwrap()
        .content
        .upgrade()
        .unwrap();
    assert!(app.read_with(cx, |app, _| app.slots.is_empty()));
    assert_eq!(
        destination.read_with(cx, |app, _| app.slots[&id].terminal.clone()),
        terminal
    );
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn pane_transfer_preserves_its_source_group_and_joins_destination_split(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let first = add(cx, source, &app, &h.root, false);
    let second = add(cx, source, &app, &h.root, true);
    change(cx, source, &app, |app, _, _| {
        app.tab_state.rename(app.tab_state.groups[0].id, "Source")
    });
    let (target, destination) = open(cx);
    let third = add(cx, target, &destination, &h.root, false);
    let value = drag(cx, source, &app, first, false);
    change(cx, target, &destination, |app, window, cx| {
        app.receive_drop(
            &value,
            DropHint::Pane {
                id: third,
                edge: DropEdge::Right,
            },
            window,
            cx,
        );
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.tabs, [Layout::Leaf(second)]);
        assert_eq!(app.tab_state.groups[0].name.as_deref(), Some("Source"));
    });
    destination.read_with(cx, |app, _| {
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].leaves(), [third, first]);
        assert_eq!(app.active, first);
    });
}

#[gpui_kit::test]
fn stale_transfer_and_escape_leave_source_untouched(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let first = add(cx, source, &app, &h.root, false);
    let stale = drag(cx, source, &app, first, true);
    let second = add(cx, source, &app, &h.root, true);
    let (target, destination) = open(cx);
    change(cx, target, &destination, |app, window, cx| {
        app.drop_on_tab(&stale, None, true, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.slots.len()), 2);
    assert!(destination.read_with(cx, |app, _| app.slots.is_empty()));
    source
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            let from = app
                .read(cx)
                .drop_regions
                .borrow()
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Tab(_)).then_some(bounds.center())
                })
                .unwrap();
            window.dispatch_event(
                MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseMoveEvent {
                    position: from + point(px(20.), px(30.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.press("escape", cx);
            window.dispatch_event(
                MouseUpEvent {
                    position: point(px(-200.), px(-200.)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(lifecycle::windows).len(), 2);
    assert_eq!(
        app.read_with(cx, |app, _| app.tabs[0].leaves()),
        [first, second]
    );
}

#[gpui_kit::test]
fn all_windows_restore_and_closed_windows_are_not_resurrected(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (first, app) = open(cx);
    let one = add(cx, first, &app, &h.root, false);
    let (second, other) = open(cx);
    let two = add(cx, second, &other, &h.root, false);
    let state = saved();
    assert_eq!(state.slots[0].id, one);
    assert_eq!(state.windows[0].slots[0].id, two);
    // Simulate process teardown without the user's explicit Close Window action.
    first
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    second
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    drop(app);
    drop(other);
    cx.run_until_parked();
    cx.update(|cx| {
        lifecycle::init(h.root.clone(), cx);
        lifecycle::open_workspace(WorkspaceLaunch::Restore, cx).unwrap();
    });
    cx.run_until_parked();
    let windows = cx.read(lifecycle::windows);
    assert_eq!(windows.len(), 2);
    assert_eq!(
        windows
            .iter()
            .map(|w| w
                .content
                .upgrade()
                .unwrap()
                .read_with(cx, |a, _| a.slots.len()))
            .sum::<usize>(),
        2
    );
    let closed = windows[1].content.upgrade().unwrap();
    change(cx, windows[1].handle, &closed, |app, window, cx| {
        for slot in app.slots.values() {
            slot.terminal.update(cx, |t, _| t.exited = true);
        }
        app.request_close(CloseTarget::Window, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.read(lifecycle::windows).len(), 1);
    assert!(saved().windows.is_empty());
    assert_eq!(saved().slots.len(), 1);
}

#[gpui_kit::test]
fn quit_checks_jobs_in_another_window_and_cancel_keeps_both_alive(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (first, app) = open(cx);
    let (second, other) = open(cx);
    let id = add(cx, second, &other, &h.root, false);
    change(cx, second, &other, |app, _, cx| {
        let pid = app.slots[&id].terminal.read(cx).process.pid.unwrap();
        app.process_snapshot = Some(
            crate::processes::ProcessSnapshot::parse(&format!(
                "{pid} 1 S /bin/sh\n{} {pid} S /usr/bin/codex\n",
                u32::MAX
            ))
            .unwrap(),
        );
    });
    change(cx, first, &app, |app, window, cx| {
        app.quit(&Quit, window, cx)
    });
    cx.run_until_parked();
    assert!(cx.pending_prompt().unwrap().0.contains("Quit Vyber"));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.read(lifecycle::quit_requested));
    assert!(!cx.read(lifecycle::is_closing));
    assert_eq!(cx.read(lifecycle::windows).len(), 2);
    assert_eq!(other.read_with(cx, |app, _| app.slots.len()), 1);
}

#[gpui_kit::test]
fn edited_document_keeps_text_selection_and_undo_after_source_window_closes(
    cx: &mut TestAppContext,
) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let id = add(cx, source, &app, &h.root, false);
    let browser = app.read_with(cx, |app, _| app.slots[&id].browser.clone());
    change(cx, source, &app, |_, window, cx| {
        browser.update(cx, |b, cx| {
            b.test_document(h.root.join("draft.txt"), "original\n", window, cx)
        });
    });
    change(cx, source, &app, |_, window, cx| {
        browser.update(cx, |b, cx| b.test_edit_document("changed\n", window, cx));
    });
    cx.run_until_parked();
    assert!(browser.read_with(cx, |b, _| b.has_dirty()));
    let (target, destination) = open(cx);
    let value = drag(cx, source, &app, id, true);
    change(cx, target, &destination, |app, window, cx| {
        app.drop_on_tab(&value, None, true, window, cx)
    });
    cx.run_until_parked();
    source
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    drop(app);
    cx.run_until_parked();
    assert_eq!(
        destination.read_with(cx, |app, _| app.slots[&id].browser.clone()),
        browser
    );
    assert_eq!(
        browser.read_with(cx, |b, cx| b.test_document_selection(cx)),
        1..4
    );
    change(cx, target, &destination, |_, window, cx| {
        browser.update(cx, |b, cx| b.test_focus_document(window, cx));
    });
    target
        .update(cx, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let undo = if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    };
    cx.simulate_keystrokes(target, undo);
    cx.run_until_parked();
    assert_eq!(
        browser.read_with(cx, |b, cx| b.test_document_value(cx)),
        "original\n"
    );
    let redo = if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    };
    cx.simulate_keystrokes(target, redo);
    cx.run_until_parked();
    assert_eq!(
        browser.read_with(cx, |b, cx| b.test_document_value(cx)),
        "changed\n"
    );
    assert!(browser.read_with(cx, |b, _| b.has_dirty()));
    let state = serde_json::to_value(saved()).unwrap();
    assert_eq!(
        state["windows"][0]["slots"][0]["browser"]["docs"][0]["draft"],
        "changed\n"
    );
}

#[gpui_kit::test]
fn captured_release_over_another_window_moves_into_its_layout(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let first = add(cx, source, &app, &h.root, false);
    let target = cx.update(|cx| {
        lifecycle::open_at(
            WorkspaceLaunch::Empty,
            Some(Bounds::new(
                point(px(2400.), px(100.)),
                size(px(700.), px(500.)),
            )),
            None,
            cx,
        )
        .unwrap()
    });
    let destination = cx.read(|cx| {
        lifecycle::windows(cx)
            .iter()
            .find(|w| w.handle == target)
            .unwrap()
            .content
            .upgrade()
            .unwrap()
    });
    let second = add(cx, target, &destination, &h.root, false);
    let target_point = target
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            let local = destination
                .read(cx)
                .drop_regions
                .borrow()
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Pane(id) if *id == second)
                        .then(|| point(bounds.right() - px(10.), bounds.center().y))
                })
                .unwrap();
            window.bounds().origin + local
        })
        .unwrap();
    source
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            let from = app
                .read(cx)
                .drop_regions
                .borrow()
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Tab(_)).then_some(bounds.center())
                })
                .unwrap();
            window.drag(from, target_point - window.bounds().origin, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(lifecycle::windows).len(), 2);
    assert!(app.read_with(cx, |app, _| app.slots.is_empty()));
    assert_eq!(
        destination.read_with(cx, |app, _| app.tabs[0].leaves()),
        [second, first]
    );
}

#[gpui_kit::test]
fn new_terminal_actions_use_the_focused_window(cx: &mut TestAppContext) {
    let _h = setup(cx);
    let (first, app) = open(cx);
    let (_second, other) = open(cx);
    first
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.update(|cx| cx.dispatch_action(&NewTerminal));
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.slots.len()), 1);
    assert!(other.read_with(cx, |app, _| app.slots.is_empty()));
}

#[gpui_kit::test]
fn unchanged_in_window_drag_still_merges_groups(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (window, app) = open(cx);
    let first = add(cx, window, &app, &h.root, false);
    let second = add(cx, window, &app, &h.root, false);
    window
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            let regions = app.read(cx).drop_regions.borrow().clone();
            let from = regions
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Tab(id) if *id == first)
                        .then_some(bounds.center())
                })
                .unwrap();
            let to = regions
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Pane(id) if *id == second)
                        .then(|| point(bounds.right() - px(10.), bounds.center().y))
                })
                .unwrap();
            window.drag(from, to, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(lifecycle::windows).len(), 1);
    assert_eq!(
        app.read_with(cx, |app, _| app.tabs[0].leaves()),
        [second, first]
    );
}

#[gpui_kit::test]
fn immediate_release_after_drag_start_does_not_need_an_extra_move_event(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (source, app) = open(cx);
    let id = add(cx, source, &app, &h.root, false);
    source
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            let from = app
                .read(cx)
                .drop_regions
                .borrow()
                .iter()
                .find_map(|(bounds, region)| {
                    matches!(region, transfer::DropRegion::Tab(_)).then_some(bounds.center())
                })
                .unwrap();
            let to = point(px(-230.), px(230.));
            window.dispatch_event(
                MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseMoveEvent {
                    position: to,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    position: to,
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(lifecycle::windows).len(), 2);
    assert!(app.read_with(cx, |app, _| app.slots.is_empty()));
    let other = cx.read(|cx| lifecycle::windows(cx)[1].content.upgrade().unwrap());
    assert!(other.read_with(cx, |app, _| app.slots.contains_key(&id)));
}

#[gpui_kit::test]
fn last_window_close_saves_empty_workspace_and_quit_saves_all_drafts(cx: &mut TestAppContext) {
    let h = setup(cx);
    let (handle, app) = open(cx);
    let id = add(cx, handle, &app, &h.root, false);
    change(cx, handle, &app, |app, window, cx| {
        app.slots[&id].terminal.update(cx, |t, _| t.exited = true);
        app.request_close(CloseTarget::Window, window, cx);
    });
    cx.run_until_parked();
    assert!(saved().slots.is_empty());
    assert!(saved().windows.is_empty());
    let (one, first) = open(cx);
    let (two, second) = open(cx);
    for (handle, app) in [(one, &first), (two, &second)] {
        let id = add(cx, handle, app, &h.root, false);
        change(cx, handle, app, |app, window, cx| {
            app.slots[&id].browser.update(cx, |b, cx| {
                b.test_document(h.root.join(format!("draft-{id}.txt")), "before", window, cx);
                b.test_edit_document("unsaved", window, cx);
            });
            app.process_snapshot =
                Some(crate::processes::ProcessSnapshot::parse("1 0 S init\n").unwrap());
        });
    }
    change(cx, one, &first, |app, window, cx| {
        app.quit(&Quit, window, cx)
    });
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(cx.read(lifecycle::quit_requested));
    let state = serde_json::to_value(saved()).unwrap();
    assert_eq!(state["slots"][0]["browser"]["docs"][0]["draft"], "unsaved");
    assert_eq!(
        state["windows"][0]["slots"][0]["browser"]["docs"][0]["draft"],
        "unsaved"
    );
}
