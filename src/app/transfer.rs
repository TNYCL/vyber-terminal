//! Transfers keep the terminal, editor, and PTY entities alive throughout.
use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Copy)]
pub(super) enum DropRegion {
    Pane(usize),
    Tab(usize),
    End,
}
pub(super) type DropRegions = Rc<RefCell<Vec<(Bounds<Pixels>, DropRegion)>>>;

struct Transfer {
    layout: Layout,
    slots: Vec<(usize, Slot)>,
    group: Option<crate::tab_state::Group>,
    active: usize,
    panel: Option<PanelMotion>,
}

impl Vyber {
    pub(super) fn mark_drop_region(
        &self,
        element: Stateful<Div>,
        region: DropRegion,
    ) -> Stateful<Div> {
        let regions = self.drop_regions.clone();
        element.relative().child(
            canvas(
                move |bounds, window, _| {
                    regions
                        .borrow_mut()
                        .push((bounds.intersect(&window.content_mask().bounds), region))
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
    }

    pub(super) fn drag_value(
        &self,
        id: usize,
        group: bool,
        label: String,
        cx: &Context<Self>,
    ) -> TabDrag {
        let members = if group {
            self.tabs
                .iter()
                .find(|t| t.contains(id))
                .map(Layout::leaves)
                .unwrap_or_default()
        } else {
            vec![id]
        };
        TabDrag {
            source: cx.entity().downgrade(),
            source_window: self.window_id,
            members,
            id,
            group,
            label,
        }
    }

    fn drag_valid(&self, drag: &TabDrag) -> bool {
        if self.close_pending || self.updating || drag.source_window != self.window_id {
            return false;
        }
        if !drag.members.iter().all(|id| self.slots.contains_key(id)) {
            return false;
        }
        if drag.group {
            self.tabs.iter().any(|t| t.leaves() == drag.members)
        } else {
            drag.members == [drag.id]
        }
    }

    pub(super) fn finish_drag_outside(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left
            || Bounds::new(Point::default(), window.viewport_size()).contains(&event.position)
        {
            return;
        }
        let app = cx.entity();
        let Some(drag) = self.tab_drag.clone() else {
            return;
        };
        if drag.source_window != window.window_handle().window_id() || !cx.has_active_drag() {
            return;
        }
        let position = crate::platform::drag_screen_position(window, event.position);
        let source_size = window.viewport_size();
        cx.stop_active_drag(window);
        self.tab_drag = None;
        self.drop_hint = None;
        cx.notify();
        cx.defer(move |cx| {
            if crate::lifecycle::is_closing(cx) || !app.read(cx).drag_valid(&drag) {
                return;
            }
            // A captured Windows drag keeps delivering events to its source.
            // Hit-test the other application windows explicitly on release.
            for target in crate::lifecycle::windows(cx).into_iter().rev() {
                if target.handle.window_id() == drag.source_window {
                    continue;
                }
                let Some(content) = target.content.upgrade() else {
                    continue;
                };
                let hit = target
                    .handle
                    .update(cx, |_, window, cx| {
                        let local = crate::platform::drag_local_position(window, position);
                        if !crate::platform::drag_hits_window(window, position) {
                            return None;
                        }
                        content.read(cx).drop_target(local)
                    })
                    .ok()
                    .flatten();
                if let Some(hint) = hit {
                    Self::transfer_to(drag, target.handle, hint, cx);
                    return;
                }
                // Inside another Vyber window, but outside its drop targets.
                let inside = target
                    .handle
                    .update(cx, |_, window, _| {
                        crate::platform::drag_hits_window(window, position)
                    })
                    .unwrap_or(false);
                if inside {
                    return;
                }
            }
            let (position, display) = crate::platform::drag_window_position(position, cx);
            let bounds = Bounds::new(position - point(px(96.), px(18.)), source_size);
            crate::lifecycle::begin_transfer(cx);
            match crate::lifecycle::open_at(WorkspaceLaunch::Empty, Some(bounds), display, cx) {
                Ok(target) => {
                    Self::transfer_to(drag, target, DropHint::End, cx);
                }
                Err(error) => {
                    app.update(cx, |app, cx| {
                        app.notice = format!("Couldn't open window: {error}");
                        cx.notify();
                    });
                }
            }
            crate::lifecycle::end_transfer(cx);
        });
    }

    fn drop_target(&self, position: Point<Pixels>) -> Option<DropHint> {
        if self.close_pending || self.updating {
            return None;
        }
        for (bounds, region) in self.drop_regions.borrow().iter().rev() {
            if !bounds.contains(&position) {
                continue;
            }
            return Some(match region {
                DropRegion::End => DropHint::End,
                DropRegion::Tab(id) => DropHint::Tab {
                    id: *id,
                    after: position.x > bounds.center().x,
                },
                DropRegion::Pane(id) => DropHint::Pane {
                    id: *id,
                    edge: DropEdge::at(
                        f32::from(position.x - bounds.left())
                            / f32::from(bounds.size.width).max(1.),
                        f32::from(position.y - bounds.top())
                            / f32::from(bounds.size.height).max(1.),
                    ),
                },
            });
        }
        self.tabs.is_empty().then_some(DropHint::End)
    }

    pub(super) fn receive_drop(
        &mut self,
        drag: &TabDrag,
        hint: DropHint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if drag.source_window == self.window_id {
            return false;
        }
        let target = window.window_handle();
        let drag = drag.clone();
        cx.defer(move |cx| {
            Self::transfer_to(drag, target, hint, cx);
        });
        true
    }

    fn extract_transfer(
        &mut self,
        drag: &TabDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Transfer> {
        if !self.drag_valid(drag) {
            return None;
        }
        let index = self.tabs.iter().position(|t| t.contains(drag.id))?;
        let layout = if drag.group {
            self.tabs.remove(index)
        } else {
            let layout = Layout::Leaf(drag.id);
            self.tabs = self.tabs.iter().filter_map(|t| t.remove(drag.id)).collect();
            layout
        };
        let group = if drag.group
            || !self.tabs.iter().any(|t| t.contains(drag.id))
                && drag.members.len() == 1
                && self.tab_state.groups[index].members() == drag.members
        {
            Some(self.tab_state.groups.remove(index))
        } else {
            None
        };
        let active = group.as_ref().map_or(drag.id, |g| g.active);
        let panel = self.panel.filter(|p| drag.members.contains(&p.slot));
        if panel.is_some() {
            self.panel = None;
        }
        let slots = drag
            .members
            .iter()
            .map(|id| (*id, self.slots.remove(id).expect("validated transfer")))
            .collect();
        self.tab_state.sync(&self.tabs, None);
        self.normalize_selection();
        if let Some(group) = self.tab_state.groups.get(self.tab) {
            self.active = group.active;
        }
        self.tab_drag = None;
        self.drop_hint = None;
        self.tab_menu = None;
        self.rename_tab = None;
        self.zoomed = false;
        let focus = self
            .slots
            .get(&self.active)
            .map(|s| s.terminal.read(cx).focus.clone())
            .unwrap_or_else(|| self.focus.clone());
        window.focus(&focus, cx);
        cx.notify();
        Some(Transfer {
            layout,
            slots,
            group,
            active,
            panel,
        })
    }

    fn transfer_to(drag: TabDrag, target: AnyWindowHandle, hint: DropHint, cx: &mut App) -> bool {
        if crate::lifecycle::is_closing(cx) || target.window_id() == drag.source_window {
            return false;
        }
        let Some(source) = drag.source.upgrade() else {
            return false;
        };
        let Some(destination) = crate::lifecycle::windows(cx)
            .iter()
            .find(|w| w.handle == target)
            .and_then(|w| w.content.upgrade())
        else {
            return false;
        };
        let ready = destination.read(cx);
        if ready.close_pending
            || ready.updating
            || !match hint {
                DropHint::End => true,
                DropHint::Tab { id, .. } | DropHint::Pane { id, .. } => {
                    ready.slots.contains_key(&id)
                }
            }
        {
            return false;
        }
        if !source.read(cx).drag_valid(&drag) {
            return false;
        }
        let source_handle = crate::lifecycle::windows(cx)
            .iter()
            .find(|w| w.handle.window_id() == drag.source_window)
            .map(|w| w.handle);
        let Some(source_handle) = source_handle else {
            return false;
        };
        crate::lifecycle::begin_transfer(cx);
        // Nest the destination update first: a missing window cannot remove the source.
        let result = target
            .update(cx, |_, window, cx| {
                let transfer = source_handle
                    .update(cx, |_, source_window, cx| {
                        source.update(cx, |source, cx| {
                            source.extract_transfer(&drag, source_window, cx)
                        })
                    })
                    .ok()
                    .flatten();
                let Some(transfer) = transfer else {
                    return false;
                };
                destination.update(cx, |app, cx| {
                    app.accept_transfer(transfer, hint, window, cx)
                });
                true
            })
            .unwrap_or(false);
        source.update(cx, |source, cx| {
            source.update_roots(cx);
            source.persist(cx);
        });
        crate::lifecycle::end_transfer(cx);
        result
    }

    fn accept_transfer(
        &mut self,
        mut transfer: Transfer,
        hint: DropHint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let members = transfer.layout.leaves();
        for (id, mut slot) in transfer.slots.drain(..) {
            slot._subscriptions =
                self.slot_subscriptions(id, &slot.terminal, &slot.browser, window, cx);
            slot.browser.update(cx, |b, cx| b.bind_window(window, cx));
            slot.terminal.update(cx, |t, cx| t.bind_window(window, cx));
            self.next = self.next.max(id + 1);
            self.slots.insert(id, slot);
        }
        match hint {
            DropHint::Pane { id, edge } => {
                self.tabs.push(transfer.layout);
                layout::move_group_to_edge(&mut self.tabs, members[0], id, edge);
                self.tab_state.sync(&self.tabs, Some(id));
            }
            DropHint::Tab { id, after } => {
                let index = self
                    .tabs
                    .iter()
                    .position(|t| t.contains(id))
                    .unwrap_or(self.tabs.len());
                self.tabs.insert(
                    (index + usize::from(after)).min(self.tabs.len()),
                    transfer.layout,
                );
                if let Some(group) = transfer.group {
                    self.tab_state.import(group);
                }
                self.tab_state.sync(&self.tabs, None);
            }
            DropHint::End => {
                self.tabs.push(transfer.layout);
                if let Some(group) = transfer.group {
                    self.tab_state.import(group);
                }
                self.tab_state.sync(&self.tabs, None);
            }
        }
        self.panel = transfer.panel;
        self.scan = crate::runtime::entity(cx).read(cx).scan.clone();
        self.zoomed = false;
        self.focus_pane(transfer.active, window, cx);
        self.update_roots(cx);
        self.persist(cx);
        window.activate_window();
    }
}
