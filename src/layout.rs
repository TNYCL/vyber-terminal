use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Layout {
    Leaf(usize),
    Split {
        key: usize,
        vertical: bool,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}
impl Layout {
    pub fn leaves(&self) -> Vec<usize> {
        match self {
            Self::Leaf(id) => vec![*id],
            Self::Split { first, second, .. } => {
                let mut ids = first.leaves();
                ids.extend(second.leaves());
                ids
            }
        }
    }
    fn replace_leaves(&mut self, ids: &mut impl Iterator<Item = usize>) {
        match self {
            Self::Leaf(id) => *id = ids.next().expect("same number of terminal leaves"),
            Self::Split { first, second, .. } => {
                first.replace_leaves(ids);
                second.replace_leaves(ids);
            }
        }
    }
    fn max_key(&self) -> usize {
        match self {
            Self::Leaf(id) => *id,
            Self::Split {
                key, first, second, ..
            } => (*key).max(first.max_key()).max(second.max_key()),
        }
    }
    fn insert(&mut self, target: usize, id: usize, edge: DropEdge, key: usize) -> bool {
        self.insert_tree(target, &Self::Leaf(id), edge, key)
    }
    fn insert_tree(&mut self, target: usize, tree: &Self, edge: DropEdge, key: usize) -> bool {
        match self {
            Self::Leaf(current) if *current == target => {
                let (first, second) = if matches!(edge, DropEdge::Left | DropEdge::Top) {
                    (tree.clone(), Self::Leaf(target))
                } else {
                    (Self::Leaf(target), tree.clone())
                };
                *self = Self::Split {
                    key,
                    vertical: matches!(edge, DropEdge::Top | DropEdge::Bottom),
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Split { first, second, .. } => {
                first.insert_tree(target, tree, edge, key)
                    || second.insert_tree(target, tree, edge, key)
            }
            _ => false,
        }
    }
    pub fn split(&mut self, target: usize, new: usize, vertical: bool) -> bool {
        let key = self.max_key().max(new) + 1;
        self.insert(
            target,
            new,
            if vertical {
                DropEdge::Bottom
            } else {
                DropEdge::Right
            },
            key,
        )
    }
    pub fn first(&self) -> usize {
        match self {
            Self::Leaf(id) => *id,
            Self::Split { first, .. } => first.first(),
        }
    }
    pub fn contains(&self, id: usize) -> bool {
        match self {
            Self::Leaf(i) => *i == id,
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }
    pub fn remove(&self, target: usize) -> Option<Self> {
        match self {
            Self::Leaf(id) => {
                if *id == target {
                    None
                } else {
                    Some(self.clone())
                }
            }
            Self::Split {
                key,
                vertical,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(a), Some(b)) => Some(Self::Split {
                    key: *key,
                    vertical: *vertical,
                    first: Box::new(a),
                    second: Box::new(b),
                }),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropEdge {
    Left,
    Right,
    Top,
    Bottom,
}
impl DropEdge {
    pub fn at(x: f32, y: f32) -> Self {
        if y < x.min(1. - x) {
            Self::Top
        } else if 1. - y < x.min(1. - x) {
            Self::Bottom
        } else if x < 0.5 {
            Self::Left
        } else {
            Self::Right
        }
    }
}

/// Changes only the tree; terminal entities, PTYs and document state stay alive.
pub fn move_to_edge(tabs: &mut Vec<Layout>, id: usize, target: usize, edge: DropEdge) -> bool {
    if id == target
        || !tabs.iter().any(|t| t.contains(id))
        || !tabs.iter().any(|t| t.contains(target))
    {
        return false;
    }
    let key = tabs.iter().map(Layout::max_key).max().unwrap_or(0) + 1;
    let mut next: Vec<_> = tabs.iter().filter_map(|t| t.remove(id)).collect();
    if let Some(tree) = next.iter_mut().find(|t| t.contains(target)) {
        tree.insert(target, id, edge, key);
        *tabs = next;
        true
    } else {
        false
    }
}

pub fn reorder_pane(tabs: &mut Vec<Layout>, id: usize, target: usize, after: bool) -> bool {
    if id == target {
        return false;
    }
    if let Some(tree) = tabs
        .iter_mut()
        .find(|t| t.contains(id) && t.contains(target))
    {
        let mut ids = tree.leaves();
        let from = ids.iter().position(|i| *i == id).unwrap();
        ids.remove(from);
        let to = ids.iter().position(|i| *i == target).unwrap() + usize::from(after);
        ids.insert(to, id);
        tree.replace_leaves(&mut ids.into_iter());
        true
    } else {
        move_pane_to_tab(tabs, id, Some(target), after)
    }
}

pub fn reorder_group(
    tabs: &mut Vec<Layout>,
    anchor: usize,
    target: Option<usize>,
    after: bool,
) -> bool {
    let Some(from) = tabs.iter().position(|t| t.contains(anchor)) else {
        return false;
    };
    if let Some(target) = target
        && (tabs[from].contains(target) || !tabs.iter().any(|t| t.contains(target)))
    {
        return false;
    }
    let tree = tabs.remove(from);
    let to = target
        .map(|id| tabs.iter().position(|t| t.contains(id)).unwrap() + usize::from(after))
        .unwrap_or(tabs.len());
    tabs.insert(to, tree);
    true
}

pub fn move_pane_to_tab(
    tabs: &mut Vec<Layout>,
    id: usize,
    target: Option<usize>,
    after: bool,
) -> bool {
    if !tabs.iter().any(|t| t.contains(id))
        || target.is_some_and(|t| t == id || !tabs.iter().any(|tree| tree.contains(t)))
    {
        return false;
    }
    let mut next: Vec<_> = tabs.iter().filter_map(|t| t.remove(id)).collect();
    let to = target
        .map(|id| next.iter().position(|t| t.contains(id)).unwrap() + usize::from(after))
        .unwrap_or(next.len());
    next.insert(to, Layout::Leaf(id));
    *tabs = next;
    true
}

pub fn move_group_to_edge(
    tabs: &mut Vec<Layout>,
    anchor: usize,
    target: usize,
    edge: DropEdge,
) -> bool {
    let Some(from) = tabs.iter().position(|t| t.contains(anchor)) else {
        return false;
    };
    if tabs[from].contains(target) || !tabs.iter().any(|t| t.contains(target)) {
        return false;
    }
    let key = tabs.iter().map(Layout::max_key).max().unwrap_or(0) + 1;
    let mut next = tabs.clone();
    let source = next.remove(from);
    let destination = next.iter_mut().find(|t| t.contains(target)).unwrap();
    destination.insert_tree(target, &source, edge, key);
    *tabs = next;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn three() -> Vec<Layout> {
        let mut t = Layout::Leaf(1);
        t.split(1, 2, false);
        t.split(2, 3, true);
        vec![t]
    }
    #[test]
    fn reorder_preserves_terminals_and_nested_geometry() {
        let mut tabs = three();
        assert!(reorder_pane(&mut tabs, 1, 3, true));
        assert_eq!(tabs[0].leaves(), vec![2, 3, 1]);
        let json = serde_json::to_string(&tabs).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Layout>>(&json).unwrap(), tabs);
        assert_eq!(json.matches("Split").count(), 2);
    }
    #[test]
    fn moving_between_groups_collapses_empty_source_and_never_duplicates() {
        for edge in [
            DropEdge::Left,
            DropEdge::Right,
            DropEdge::Top,
            DropEdge::Bottom,
        ] {
            let mut tabs = three();
            tabs.push(Layout::Leaf(4));
            assert!(move_to_edge(&mut tabs, 4, 2, edge));
            assert_eq!(tabs.len(), 1);
            let mut ids = tabs[0].leaves();
            ids.sort();
            assert_eq!(ids, vec![1, 2, 3, 4]);
            assert!(move_to_edge(&mut tabs, 1, 3, edge));
            let mut ids = tabs[0].leaves();
            ids.sort();
            assert_eq!(ids, vec![1, 2, 3, 4]);
        }
    }
    #[test]
    fn invalid_drops_leave_layout_unchanged_and_groups_can_be_reordered() {
        let mut tabs = three();
        tabs.push(Layout::Leaf(4));
        let before = tabs.clone();
        assert!(!move_to_edge(&mut tabs, 1, 1, DropEdge::Right));
        assert!(!move_to_edge(&mut tabs, 99, 2, DropEdge::Left));
        assert_eq!(tabs, before);
        assert!(reorder_group(&mut tabs, 4, Some(1), false));
        assert_eq!(tabs[0], Layout::Leaf(4));
    }

    #[test]
    fn tab_insertions_match_the_visible_before_and_after_marker() {
        for (id, target, after, expected) in [
            (1, 3, false, vec![2, 1, 3]),
            (1, 3, true, vec![2, 3, 1]),
            (3, 1, false, vec![3, 1, 2]),
            (3, 1, true, vec![1, 3, 2]),
        ] {
            let mut panes = three();
            reorder_pane(&mut panes, id, target, after);
            assert_eq!(panes[0].leaves(), expected);
            let mut groups = vec![Layout::Leaf(1), Layout::Leaf(2), Layout::Leaf(3)];
            reorder_group(&mut groups, id, Some(target), after);
            assert_eq!(
                groups.iter().map(Layout::first).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn dragging_a_workspace_into_a_split_preserves_its_nested_layout() {
        for edge in [
            DropEdge::Left,
            DropEdge::Right,
            DropEdge::Top,
            DropEdge::Bottom,
        ] {
            let source = three().remove(0);
            let mut tabs = vec![source.clone(), Layout::Leaf(4)];
            assert!(move_group_to_edge(&mut tabs, 2, 4, edge));
            assert_eq!(tabs.len(), 1);
            let Layout::Split {
                first,
                second,
                vertical,
                ..
            } = &tabs[0]
            else {
                panic!("missing split")
            };
            assert_eq!(*vertical, matches!(edge, DropEdge::Top | DropEdge::Bottom));
            let moved = if matches!(edge, DropEdge::Left | DropEdge::Top) {
                first
            } else {
                second
            };
            assert_eq!(**moved, source);
            let before = tabs.clone();
            assert!(!move_group_to_edge(&mut tabs, 1, 3, edge));
            assert_eq!(tabs, before);
        }
    }

    #[test]
    fn pane_can_be_detached_to_a_new_tab_without_losing_other_terminals() {
        let mut tabs = three();
        tabs.push(Layout::Leaf(4));
        assert!(move_pane_to_tab(&mut tabs, 2, Some(4), false));
        assert_eq!(
            tabs.iter().map(Layout::leaves).collect::<Vec<_>>(),
            vec![vec![1, 3], vec![2], vec![4]]
        );
        assert!(move_pane_to_tab(&mut tabs, 3, None, true));
        assert_eq!(
            tabs.iter().map(Layout::leaves).collect::<Vec<_>>(),
            vec![vec![1], vec![2], vec![4], vec![3]]
        );
        let before = tabs.clone();
        assert!(!move_pane_to_tab(&mut tabs, 1, Some(99), false));
        assert_eq!(tabs, before);
    }
}
