use super::{Browser, BrowserState, Config, Kind, Message, PendingLoad, SavedDocument, View};
use crate::workspace;
use core::prelude::v1::test;
use gpui::{Focusable, TestAppContext, WindowHandle};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

struct Harness {
    window: WindowHandle<Browser>,
    root: PathBuf,
    _data: workspace::TestDataDir,
    _directory: tempfile::TempDir,
}

fn setup(cx: &mut TestAppContext) -> Harness {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let data = workspace::TestDataDir::set(directory.path().join("data"));
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Config::default());
        crate::theme::apply(cx);
    });
    let window = cx.add_window({
        let root = root.clone();
        move |window, cx| Browser::new(root, window, cx)
    });
    Harness {
        window,
        root,
        _data: data,
        _directory: directory,
    }
}

#[gpui_kit::test]
fn asynchronous_reads_preserve_latest_selection_and_each_source_location(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let first = harness.root.join("first.md");
    let second = harness.root.join("second.md");
    let third = harness.root.join("third.md");
    let text = b"# Heading\nsecond\nthird\n";
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.visible = true;
            for (generation, path, line) in [(1, &first, 3), (2, &second, 2)] {
                browser.pending_loads.insert(
                    path.clone(),
                    PendingLoad {
                        generation,
                        pinned: true,
                    },
                );
                browser.pending_links.insert(path.clone(), Some((line, 1)));
            }
            browser.intended_document = Some(second.clone());
            // Finish the newest request first, then the slower older read.
            for (generation, path) in [(2, &second), (1, &first)] {
                browser
                    .document_sender
                    .send(Message::Loaded(
                        generation,
                        path.clone(),
                        Ok(text.to_vec()),
                        text.len() as u64,
                    ))
                    .unwrap();
            }
            browser.poll(window, cx);
            assert_eq!(browser.active_document().unwrap().path, second);
            for (path, line) in [(&first, 2), (&second, 1)] {
                let doc = browser.docs.iter().find(|doc| &doc.path == path).unwrap();
                assert!(!doc.preview);
                assert_eq!(doc.editor.read(cx).cursor_position().line, line);
            }
            browser.pending_loads.insert(
                third.clone(),
                PendingLoad {
                    generation: 3,
                    pinned: true,
                },
            );
            browser.intended_document = Some(third.clone());
            let first_index = browser
                .docs
                .iter()
                .position(|doc| doc.path == first)
                .unwrap();
            browser.select_document(first_index, cx);
            browser
                .document_sender
                .send(Message::Loaded(
                    3,
                    third,
                    Ok(text.to_vec()),
                    text.len() as u64,
                ))
                .unwrap();
            browser.poll(window, cx);
            assert_eq!(browser.active_document().unwrap().path, first);
            assert!(browser.pending_links.is_empty());
            let stale_preview = harness.root.join("stale.txt");
            browser.pending_loads.insert(
                stale_preview.clone(),
                PendingLoad {
                    generation: 4,
                    pinned: false,
                },
            );
            browser
                .pending_links
                .insert(stale_preview.clone(), Some((7, 1)));
            browser
                .document_sender
                .send(Message::Loaded(4, stale_preview, Ok(b"stale".to_vec()), 5))
                .unwrap();
            browser.poll(window, cx);
            assert_eq!(browser.docs.len(), 3);
            assert!(browser.pending_links.is_empty());
        })
        .unwrap();
}

#[gpui_kit::test]
fn closed_document_and_closed_panel_ignore_late_reads(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let path = harness.root.join("closed.txt");
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.intended_document = Some(path.clone());
            browser.loaded(path.clone(), Ok(b"before".to_vec()), 6, true, window, cx);
            browser.pending_loads.insert(
                path.clone(),
                PendingLoad {
                    generation: 1,
                    pinned: true,
                },
            );
            browser.close_doc(0, cx);
            browser
                .document_sender
                .send(Message::Loaded(1, path.clone(), Ok(b"late".to_vec()), 4))
                .unwrap();
            browser.poll(window, cx);
            assert!(browser.docs.is_empty());
            browser.pending_loads.insert(
                path.clone(),
                PendingLoad {
                    generation: 2,
                    pinned: true,
                },
            );
            browser.close_panel(cx);
            browser
                .document_sender
                .send(Message::Loaded(2, path, Ok(b"late".to_vec()), 4))
                .unwrap();
            browser.poll(window, cx);
            assert!(browser.docs.is_empty() && !browser.visible);
        })
        .unwrap();
}

#[gpui_kit::test]
fn root_change_clears_search_and_keeps_document_completions(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let other = harness.root.join("other");
    std::fs::create_dir(&other).unwrap();
    let path = harness.root.join("pending.txt");
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.visible = true;
            browser
                .filter
                .update(cx, |filter, cx| filter.set_value("old", window, cx));
            browser.search_query = "old".into();
            browser.search_results.push((path.clone(), 1, "old".into()));
            browser.pending_loads.insert(
                path.clone(),
                PendingLoad {
                    generation: 1,
                    pinned: true,
                },
            );
            browser.intended_document = Some(path.clone());
            browser.pending_links.insert(path.clone(), Some((1, 3)));
            let document_sender = browser.document_sender.clone();
            browser.change_root(other.clone(), window, cx);
            assert!(browser.filter_query(cx).is_empty());
            assert!(browser.search_query.is_empty() && browser.search_results.is_empty());
            assert!(browser.name_results.is_empty() && !browser.name_loading);
            document_sender
                .send(Message::Loaded(1, path.clone(), Ok(b"opened".to_vec()), 6))
                .unwrap();
            browser.poll(window, cx);
            assert_eq!(browser.root, other);
            assert_eq!(browser.active_document().unwrap().path, path);
            assert_eq!(
                browser
                    .active_document()
                    .unwrap()
                    .editor
                    .read(cx)
                    .cursor_position(),
                gpui_kit::component::input::Position::new(0, 2)
            );
            assert!(browser.pending_links.is_empty());
        })
        .unwrap();
}

#[gpui_kit::test]
fn opening_outside_the_selected_tree_root_restarts_search_and_rejects_old_results(
    cx: &mut TestAppContext,
) {
    let harness = setup(cx);
    std::fs::create_dir(harness.root.join("scope")).unwrap();
    let old_path = harness.root.join("scope/old-needle.txt");
    let selected = harness.root.join("new-needle.txt");
    std::fs::write(&selected, "needle").unwrap();
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser
                .filter
                .update(cx, |filter, cx| filter.set_value("needle", window, cx));
            browser.tree_root = Some("scope".into());
            browser.name_query = "needle".into();
            browser.name_generation = 10;
            browser.name_loading = true;
            browser.name_results = vec![workspace::FileEntry {
                path: old_path.clone(),
                relative: "scope/old-needle.txt".into(),
                ..Default::default()
            }];
            browser.search_query = "needle".into();
            browser.search_generation = 20;
            browser.search_results = vec![(old_path.clone(), 1, "needle".into())];
            let old_name_cancel = browser.name_cancel.clone();
            let old_search_cancel = browser.search_cancel.clone();
            browser.open(selected, true, cx);
            assert!(browser.tree_root.is_none());
            assert_eq!(browser.filter_query(cx), "needle");
            assert!(old_name_cancel.load(std::sync::atomic::Ordering::Relaxed));
            assert!(browser.name_generation > 10);
            assert!(old_search_cancel.load(std::sync::atomic::Ordering::Relaxed));
            assert!(browser.search_generation > 20);
            browser.name_search_loaded(
                10,
                "needle".into(),
                workspace::NameSearchResult {
                    entries: vec![workspace::FileEntry {
                        path: old_path.clone(),
                        relative: "scope/old-needle.txt".into(),
                        ..Default::default()
                    }],
                    errors: vec!["obsolete scoped result".into()],
                    truncated: false,
                },
                window,
                cx,
            );
            assert!(
                browser.name_results.is_empty()
                    && browser.name_errors.is_empty()
                    && browser.name_loading
            );
            browser
                .sender
                .send(Message::Search(
                    20,
                    "needle".into(),
                    Ok(vec![(old_path, 1, "needle".into())]),
                ))
                .unwrap();
            browser.poll(window, cx);
            assert!(browser.search_query.is_empty() && browser.search_results.is_empty());
        })
        .unwrap();
}

#[gpui_kit::test]
fn repeated_saves_keep_bom_crlf_and_finish_after_root_change(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let path = harness.root.join("saved.txt");
    let original = b"\xef\xbb\xbfline\r\n";
    std::fs::write(&path, original).unwrap();
    let other = harness.root.join("other");
    std::fs::create_dir(&other).unwrap();
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.visible = true;
            browser.intended_document = Some(path.clone());
            browser.loaded(
                path.clone(),
                Ok(original.to_vec()),
                original.len() as u64,
                true,
                window,
                cx,
            );
            browser.docs[0].editor.update(cx, |editor, cx| {
                editor.set_value("discarded draft", window, cx)
            });
            browser.docs[0].dirty = true;
            browser.docs[0].conflict = true;
            browser.reload_from_disk(window, cx);
            assert!(!browser.docs[0].dirty && !browser.docs[0].conflict);
            assert_eq!(browser.docs[0].baseline, original);
            assert_eq!(
                browser.docs[0].text,
                browser.docs[0].editor.read(cx).value()
            );
            for (iteration, text) in ["first\nsecond\n", "changed\nagain\n"]
                .into_iter()
                .enumerate()
            {
                browser.docs[0]
                    .editor
                    .update(cx, |editor, cx| editor.set_value(text, window, cx));
                browser.docs[0].dirty = true;
                browser.save(cx);
                if iteration == 0 {
                    browser.change_root(other.clone(), window, cx);
                }
                let deadline = Instant::now() + Duration::from_secs(3);
                while browser.docs[0].saving && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                    browser.poll(window, cx);
                }
                let expected = [
                    b"\xef\xbb\xbf".as_slice(),
                    text.replace('\n', "\r\n").as_bytes(),
                ]
                .concat();
                assert_eq!(std::fs::read(&path).unwrap(), expected);
                assert!(
                    !browser.docs[0].dirty && !browser.docs[0].saving && !browser.docs[0].conflict,
                    "{}",
                    browser.notice
                );
                assert_eq!(browser.docs[0].size, expected.len() as u64);
                assert_eq!(browser.docs[0].baseline, expected);
            }
        })
        .unwrap();
}

#[gpui_kit::test]
fn outside_root_documents_reload_clean_buffers_and_preserve_dirty_edits(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let path = harness.root.join("observed.txt");
    std::fs::write(&path, "initial").unwrap();
    let other = harness.root.join("other");
    std::fs::create_dir(&other).unwrap();
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.intended_document = Some(path.clone());
            browser.loaded(path.clone(), Ok(b"initial".to_vec()), 7, true, window, cx);
            browser.change_root(other, window, cx);
            std::fs::write(&path, "external replacement").unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while browser.docs[0].editor.read(cx).value().as_ref() != "external replacement"
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
                browser.poll(window, cx);
            }
            assert_eq!(
                browser.docs[0].editor.read(cx).value().as_ref(),
                "external replacement"
            );
            browser.docs[0]
                .editor
                .update(cx, |editor, cx| editor.set_value("my draft", window, cx));
            browser.docs[0].dirty = true;
            browser
                .document_sender
                .send(Message::DocumentChanged(
                    path,
                    Some(b"later disk edit".to_vec()),
                ))
                .unwrap();
            browser.poll(window, cx);
            assert_eq!(browser.docs[0].editor.read(cx).value().as_ref(), "my draft");
            assert!(browser.docs[0].dirty && browser.docs[0].conflict);
        })
        .unwrap();
}

#[gpui_kit::test]
fn save_completion_replays_external_changes_received_while_saving(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let path = harness.root.join("save-race.txt");
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.intended_document = Some(path.clone());
            browser.loaded(path.clone(), Ok(b"original".to_vec()), 8, true, window, cx);
            browser.docs[0]
                .editor
                .update(cx, |editor, cx| editor.set_value("saved", window, cx));
            browser.docs[0].dirty = true;
            browser.docs[0].saving = true;
            browser
                .document_sender
                .send(Message::DocumentChanged(
                    path.clone(),
                    Some(b"external after save".to_vec()),
                ))
                .unwrap();
            browser
                .document_sender
                .send(Message::Saved(
                    path,
                    b"saved".to_vec(),
                    "saved".into(),
                    Ok(()),
                ))
                .unwrap();
            browser.poll(window, cx);
            assert_eq!(
                browser.docs[0].editor.read(cx).value().as_ref(),
                "external after save"
            );
            assert_eq!(browser.docs[0].baseline, b"external after save");
            assert!(!browser.docs[0].saving && !browser.docs[0].dirty);
        })
        .unwrap();
}

#[gpui_kit::test]
fn save_failure_keeps_the_draft_and_external_file(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let path = harness.root.join("conflict.txt");
    std::fs::write(&path, "original").unwrap();
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.intended_document = Some(path.clone());
            browser.loaded(path.clone(), Ok(b"original".to_vec()), 8, true, window, cx);
            browser.docs[0]
                .editor
                .update(cx, |editor, cx| editor.set_value("my draft", window, cx));
            browser.docs[0].dirty = true;
            std::fs::write(&path, "external edit").unwrap();
            browser.save(cx);
            let deadline = Instant::now() + Duration::from_secs(3);
            while browser.docs[0].saving && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
                browser.poll(window, cx);
            }
            assert!(browser.docs[0].dirty && !browser.docs[0].saving);
            assert_eq!(browser.docs[0].editor.read(cx).value().as_ref(), "my draft");
            assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
            assert!(
                browser.notice.contains("changed on disk"),
                "{}",
                browser.notice
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn restoration_preserves_git_view_and_user_selection(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let first = harness.root.join("first.txt");
    let second = harness.root.join("second.txt");
    let third = harness.root.join("third.txt");
    let saved = |path: PathBuf| SavedDocument {
        path,
        preview: false,
        pinned: true,
        scroll: (0., 0.),
        draft: None,
        baseline: None,
    };
    harness
        .window
        .update(cx, |browser, window, cx| {
            let mut first_saved = saved(first.clone());
            first_saved.draft = Some("restored draft".into());
            first_saved.baseline = Some(b"first".to_vec());
            browser.restore_state(
                BrowserState {
                    visible: true,
                    git: true,
                    active: Some(second.clone()),
                    docs: vec![first_saved, saved(second.clone()), saved(third.clone())],
                    ..Default::default()
                },
                cx,
            );
            let pending_state = browser.state(cx);
            assert_eq!(pending_state.docs.len(), 3);
            assert_eq!(pending_state.active, Some(second.clone()));
            assert_eq!(
                pending_state
                    .docs
                    .iter()
                    .find(|doc| doc.path == first)
                    .unwrap()
                    .draft
                    .as_deref(),
                Some("restored draft")
            );
            browser.loaded(second.clone(), Ok(b"second".to_vec()), 6, true, window, cx);
            browser.loaded(first.clone(), Ok(b"first".to_vec()), 5, true, window, cx);
            let partial_state = browser.state(cx);
            assert_eq!(partial_state.docs.len(), 3);
            assert_eq!(
                partial_state
                    .docs
                    .iter()
                    .filter(|doc| doc.path == first)
                    .count(),
                1
            );
            assert_eq!(partial_state.active, Some(second.clone()));
            assert!(browser.view == View::Git);
            assert_eq!(browser.active_document().unwrap().path, second);
            let first_index = browser
                .docs
                .iter()
                .position(|doc| doc.path == first)
                .unwrap();
            browser.select_document(first_index, cx);
            browser.loaded(third, Ok(b"third".to_vec()), 5, true, window, cx);
            assert!(browser.view == View::Files);
            assert_eq!(browser.active_document().unwrap().path, first);
        })
        .unwrap();
}

#[gpui_kit::test]
fn unavailable_restored_file_keeps_its_draft_until_a_successful_retry(cx: &mut TestAppContext) {
    let harness = setup(cx);
    let path = harness.root.join("repaired-later.txt");
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.restore_state(
                BrowserState {
                    visible: true,
                    active: Some(path.clone()),
                    docs: vec![SavedDocument {
                        path: path.clone(),
                        preview: false,
                        pinned: true,
                        scroll: (0., 0.),
                        draft: Some("protected draft\n".into()),
                        baseline: Some(b"original\n".to_vec()),
                    }],
                    ..Default::default()
                },
                cx,
            );
            let generation = browser.pending_loads[&path].generation;
            browser
                .document_sender
                .send(Message::Loaded(
                    generation,
                    path.clone(),
                    Err("File not found".into()),
                    0,
                ))
                .unwrap();
            browser.poll(window, cx);
            assert!(matches!(browser.docs[0].kind, Kind::Unsupported(_)));
            assert!(browser.has_dirty());
            let failed_state = browser.state(cx);
            assert_eq!(failed_state.docs.len(), 1);
            assert_eq!(
                failed_state.docs[0].draft.as_deref(),
                Some("protected draft\n")
            );
            assert_eq!(
                failed_state.docs[0].baseline.as_deref(),
                Some(b"original\n".as_slice())
            );
            browser.close_doc(0, cx);
            assert_eq!(browser.docs.len(), 1);
            std::fs::write(&path, "repaired content\n").unwrap();
            browser.open(path.clone(), true, cx);
            let generation = browser.pending_loads[&path].generation;
            browser
                .document_sender
                .send(Message::Loaded(
                    generation,
                    path.clone(),
                    Ok(b"repaired content\n".to_vec()),
                    17,
                ))
                .unwrap();
            browser.poll(window, cx);
            assert!(browser.restore_docs.is_empty());
            assert!(browser.docs[0].kind == Kind::Text);
            assert_eq!(
                browser.docs[0].editor.read(cx).value().as_ref(),
                "protected draft\n"
            );
            assert!(browser.docs[0].dirty && browser.docs[0].conflict);
            assert_eq!(browser.docs[0].baseline, b"original\n");
            let recovered_state = browser.state(cx);
            assert_eq!(recovered_state.docs.len(), 1);
            assert_eq!(
                recovered_state.docs[0].draft.as_deref(),
                Some("protected draft\n")
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn quick_open_reveals_the_tree_and_leaves_quick_look(cx: &mut TestAppContext) {
    let harness = setup(cx);
    harness
        .window
        .update(cx, |browser, window, cx| {
            browser.view = View::Git;
            browser.sidebar_hidden[0] = true;
            browser.quick_look = true;
            browser.focus_filter(window, cx);
            assert!(
                browser.visible
                    && browser.view == View::Files
                    && browser.sidebar_open()
                    && !browser.quick_look
            );
            assert!(browser.filter.read(cx).focus_handle(cx).is_focused(window));
        })
        .unwrap();
}

#[test]
fn content_search_includes_ignored_and_hidden_paths_and_reports_failures() {
    if workspace::command("rg").arg("--version").output().is_err() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(".gitignore"),
        "target/\nnode_modules/\n.hidden\n",
    )
    .unwrap();
    for folder in ["target", "node_modules"] {
        std::fs::create_dir(directory.path().join(folder)).unwrap();
        std::fs::write(directory.path().join(folder).join("match.txt"), "needle").unwrap();
    }
    std::fs::write(directory.path().join(".hidden"), "needle").unwrap();
    let cancelled = AtomicBool::new(false);
    let results = super::search_contents_in(directory.path(), "needle", &cancelled).unwrap();
    assert_eq!(results.len(), 3);
    assert!(
        super::search_contents_in(&directory.path().join("missing"), "needle", &cancelled).is_err()
    );
}
