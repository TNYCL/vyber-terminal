use crate::layout::Layout;

#[derive(Clone, Debug)]
pub enum CloseTarget {
    Pane(usize),
    Tab(Vec<usize>),
    Window,
    Quit,
}

impl CloseTarget {
    pub fn tab_index(&self, tabs: &[Layout]) -> Option<usize> {
        let Self::Tab(ids) = self else { return None };
        tabs.iter().position(|layout| {
            let mut leaves = layout.leaves();
            leaves.sort_unstable();
            leaves == *ids
        })
    }

    pub fn ids(&self, tabs: &[Layout], mut live: Vec<usize>) -> Option<Vec<usize>> {
        live.sort_unstable();
        match self {
            Self::Pane(id) => live.contains(id).then(|| vec![*id]),
            Self::Tab(ids) => self
                .tab_index(tabs)
                .filter(|_| ids.iter().all(|id| live.contains(id)))
                .map(|_| ids.clone()),
            Self::Window | Self::Quit => Some(live),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClosePlan {
    pub target: CloseTarget,
    pub ids: Vec<usize>,
    pub sessions: Vec<(usize, Option<u32>)>,
}

impl ClosePlan {
    pub fn valid(
        &self,
        tabs: &[Layout],
        live: Vec<usize>,
        sessions: &[(usize, Option<u32>)],
    ) -> bool {
        self.target.ids(tabs, live).as_ref() == Some(&self.ids) && self.sessions == sessions
    }
}

pub fn remove_panes(tabs: &mut Vec<Layout>, ids: &[usize]) {
    for id in ids {
        *tabs = tabs
            .iter()
            .filter_map(|layout| layout.remove(*id))
            .collect();
    }
}

pub fn selection(
    tabs: &[Layout],
    preferred_pane: usize,
    preferred_tab: usize,
) -> Option<(usize, usize)> {
    if let Some(index) = tabs
        .iter()
        .position(|layout| layout.contains(preferred_pane))
    {
        return Some((index, preferred_pane));
    }
    let index = preferred_tab.min(tabs.len().checked_sub(1)?);
    Some((index, tabs[index].first()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split() -> Layout {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, false);
        layout
    }

    #[test]
    fn closing_one_split_terminal_preserves_its_sibling_and_other_tabs() {
        let mut tabs = vec![split(), Layout::Leaf(3)];
        remove_panes(&mut tabs, &[2]);
        assert_eq!(tabs, vec![Layout::Leaf(1), Layout::Leaf(3)]);
        assert_eq!(selection(&tabs, 2, 0), Some((0, 1)));
        assert_eq!(selection(&tabs, 3, 0), Some((1, 3)));
    }

    #[test]
    fn last_terminal_has_no_selection_and_a_new_terminal_restores_it() {
        let mut tabs = vec![Layout::Leaf(1)];
        remove_panes(&mut tabs, &[1]);
        assert_eq!(selection(&tabs, 1, 0), None);
        tabs.push(Layout::Leaf(2));
        assert_eq!(selection(&tabs, 1, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn confirmation_stays_with_the_original_group_after_reordering() {
        let target = CloseTarget::Tab(vec![1, 2]);
        assert_eq!(target.tab_index(&[Layout::Leaf(3), split()]), Some(1));
        let plan = ClosePlan {
            target,
            ids: vec![1, 2],
            sessions: vec![(1, Some(10)), (2, Some(20))],
        };
        assert!(plan.valid(&[Layout::Leaf(3), split()], vec![3, 2, 1], &plan.sessions));
    }

    #[test]
    fn approval_does_not_cover_an_expanded_group_or_replaced_session() {
        let plan = ClosePlan {
            target: CloseTarget::Tab(vec![1]),
            ids: vec![1],
            sessions: vec![(1, Some(10))],
        };
        assert!(!plan.valid(&[split()], vec![1, 2], &plan.sessions));
        assert!(!plan.valid(&[Layout::Leaf(1)], vec![1], &[(1, Some(99))]));
        assert!(!plan.valid(&[], vec![], &[]));
    }

    #[test]
    fn quit_approval_covers_all_sessions_and_rejects_added_terminals() {
        let plan = ClosePlan {
            target: CloseTarget::Quit,
            ids: vec![1, 2],
            sessions: vec![(1, Some(10)), (2, Some(20))],
        };
        assert!(plan.valid(&[split()], vec![2, 1], &plan.sessions));
        assert!(!plan.valid(&[split(), Layout::Leaf(3)], vec![1, 2, 3], &plan.sessions));
        assert_eq!(
            CloseTarget::Pane(2).ids(&[split()], vec![1, 2]),
            Some(vec![2])
        );
    }
}
