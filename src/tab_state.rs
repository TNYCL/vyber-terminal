//! Names and focus memory belong to groups, not to their current position or
//! first terminal. Layout edits never recreate a terminal session.
use crate::layout::Layout;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: usize,
    pub name: Option<String>,
    pub active: usize,
    members: Vec<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TabState {
    pub groups: Vec<Group>,
    next: usize,
}

impl TabState {
    /// Match surviving groups by membership, largest overlap first. When two
    /// groups are joined, the drop destination keeps its name and identity.
    pub fn sync(&mut self, layouts: &[Layout], destination: Option<usize>) {
        let members: Vec<_> = layouts.iter().map(Layout::leaves).collect();
        let mut previous: Vec<_> = std::mem::take(&mut self.groups)
            .into_iter()
            .map(Some)
            .collect();
        self.next = self.next.max(
            previous
                .iter()
                .flatten()
                .map(|g| g.id + 1)
                .max()
                .unwrap_or(0),
        );
        let mut candidates = Vec::new();
        for (new, ids) in members.iter().enumerate() {
            for (old, group) in previous.iter().enumerate() {
                let group = group.as_ref().unwrap();
                let overlap = ids.iter().filter(|id| group.members.contains(id)).count();
                if overlap > 0 {
                    let preferred = destination
                        .is_some_and(|id| ids.contains(&id) && group.members.contains(&id));
                    let exact = overlap == ids.len() && overlap == group.members.len();
                    candidates.push((preferred, exact, overlap, new, old));
                }
            }
        }
        candidates.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(b.2.cmp(&a.2))
                .then(a.3.cmp(&b.3))
                .then(a.4.cmp(&b.4))
        });
        let mut matched = vec![None; layouts.len()];
        for (_, _, _, new, old) in candidates {
            if matched[new].is_none() && previous[old].is_some() {
                matched[new] = previous[old].take();
            }
        }
        self.groups = members
            .into_iter()
            .zip(matched)
            .map(|(members, group)| {
                let mut group = group.unwrap_or_else(|| {
                    let id = self.next;
                    self.next += 1;
                    Group {
                        id,
                        name: None,
                        active: members[0],
                        members: vec![],
                    }
                });
                if !members.contains(&group.active) {
                    group.active = members[0];
                }
                group.members = members;
                group
            })
            .collect();
    }

    pub fn focus(&mut self, terminal: usize) {
        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|g| g.members.contains(&terminal))
        {
            group.active = terminal;
        }
    }

    pub fn index(&self, id: usize) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    pub fn rename(&mut self, id: usize, name: &str) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            let name = name.trim();
            group.name = (!name.is_empty()).then(|| name.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;

    fn split() -> Layout {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, false);
        layout.split(2, 3, true);
        layout
    }

    #[test]
    fn name_and_focus_survive_reordering_and_anchor_removal() {
        let mut layouts = vec![split(), Layout::Leaf(4)];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        let id = state.groups[0].id;
        state.rename(id, " Backend ");
        state.focus(3);
        assert!(layout::reorder_group(&mut layouts, 1, Some(4), true));
        state.sync(&layouts, None);
        assert_eq!(state.index(id), Some(1));
        assert_eq!(state.groups[1].active, 3);
        layouts[1] = layouts[1].remove(1).unwrap();
        state.sync(&layouts, None);
        assert_eq!(state.groups[1].id, id);
        assert_eq!(state.groups[1].name.as_deref(), Some("Backend"));
        assert_eq!(state.groups[1].active, 3);
    }

    #[test]
    fn detached_terminal_does_not_steal_the_group_name() {
        let mut layouts = vec![split()];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        let id = state.groups[0].id;
        state.rename(id, "Agent");
        state.focus(1);
        assert!(layout::move_pane_to_tab(&mut layouts, 1, None, true));
        state.sync(&layouts, None);
        assert_eq!(state.groups[0].id, id);
        assert_eq!(state.groups[0].name.as_deref(), Some("Agent"));
        assert_eq!(state.groups[0].active, 2);
        assert_ne!(state.groups[1].id, id);
        assert!(state.groups[1].name.is_none());
    }

    #[test]
    fn detaching_half_before_the_source_keeps_its_identity() {
        let mut group = Layout::Leaf(1);
        group.split(1, 2, false);
        let mut layouts = vec![group, Layout::Leaf(3)];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        let id = state.groups[0].id;
        state.rename(id, "Backend");
        assert!(layout::move_pane_to_tab(&mut layouts, 1, Some(2), false));
        state.sync(&layouts, Some(2));
        assert_eq!(state.groups[1].id, id);
        assert_eq!(state.groups[1].name.as_deref(), Some("Backend"));
        assert!(state.groups[0].name.is_none());
    }

    #[test]
    fn merging_keeps_the_destination_even_when_it_is_smaller() {
        for edge in [layout::DropEdge::Left, layout::DropEdge::Right] {
            let mut layouts = vec![split(), Layout::Leaf(4)];
            let mut state = TabState::default();
            state.sync(&layouts, None);
            let destination = state.groups[1].id;
            state.rename(destination, "Tests");
            assert!(layout::move_group_to_edge(&mut layouts, 1, 4, edge));
            state.sync(&layouts, Some(4));
            assert_eq!(state.groups[0].id, destination);
            assert_eq!(state.groups[0].name.as_deref(), Some("Tests"));
            assert_eq!(state.groups[0].active, 4);
        }
    }

    #[test]
    fn closed_focus_falls_back_and_removed_groups_do_not_reuse_ids() {
        let mut layouts = vec![split(), Layout::Leaf(4)];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        state.focus(3);
        let removed = state.groups[1].id;
        layouts[0] = layouts[0].remove(3).unwrap();
        layouts.pop();
        state.sync(&layouts, None);
        assert_eq!(state.groups[0].active, 1);
        layouts.push(Layout::Leaf(5));
        state.sync(&layouts, None);
        assert!(state.groups[1].id > removed);
    }

    #[test]
    fn saved_metadata_round_trips_and_legacy_layouts_get_defaults() {
        let layouts = vec![split(), Layout::Leaf(4)];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        state.rename(state.groups[0].id, "Build");
        state.focus(2);
        let mut restored: TabState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        restored.sync(&layouts, None);
        assert_eq!(restored.groups[0].name.as_deref(), Some("Build"));
        assert_eq!(restored.groups[0].active, 2);
        restored.rename(restored.groups[0].id, "  ");
        assert!(restored.groups[0].name.is_none());
        let mut legacy = TabState::default();
        legacy.sync(&layouts, None);
        assert_eq!(legacy.groups[0].active, 1);
        assert!(legacy.groups.iter().all(|g| g.name.is_none()));
    }

    #[test]
    fn each_group_remembers_its_own_focus_after_a_split_is_added() {
        let mut layouts = vec![split(), Layout::Leaf(4)];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        state.focus(2);
        state.focus(4);
        assert_eq!(state.groups[0].active, 2);
        assert_eq!(state.groups[1].active, 4);
        layouts[0].split(2, 5, false);
        state.sync(&layouts, None);
        assert_eq!(state.groups[0].active, 2);
        state.focus(5);
        assert_eq!(state.groups[0].active, 5);
        assert_eq!(state.groups[1].active, 4);
    }

    #[test]
    fn moving_one_pane_keeps_both_group_names() {
        let mut layouts = vec![split(), Layout::Leaf(4)];
        let mut state = TabState::default();
        state.sync(&layouts, None);
        let source = state.groups[0].id;
        let destination = state.groups[1].id;
        state.rename(source, "Agent");
        state.rename(destination, "Tests");
        assert!(layout::move_to_edge(
            &mut layouts,
            1,
            4,
            layout::DropEdge::Left
        ));
        state.sync(&layouts, Some(4));
        assert_eq!(state.groups[0].id, source);
        assert_eq!(state.groups[0].name.as_deref(), Some("Agent"));
        assert_eq!(state.groups[1].id, destination);
        assert_eq!(state.groups[1].name.as_deref(), Some("Tests"));
    }
}
