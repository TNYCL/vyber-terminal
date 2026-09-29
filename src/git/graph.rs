//! Lane layout for the commit graph: which column each commit sits in and
//! which lines cross each row. Rows are drawn independently, so every line
//! leaving the bottom of one row enters the top of the next at the same lane.

/// Lane colors: HEAD's line first, then the palette VS Code and Cursor use
/// for the source control graph.
pub const COLORS: [u32; 6] = [0x59a4f9, 0xffb000, 0xdc267f, 0x40b0a6, 0xb66dff, 0x994f00];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeKind {
    /// A line passing from the top to the bottom of the row.
    Through,
    /// A line from the top of `from` into the commit.
    Into,
    /// A line from the commit to the bottom of `to`.
    OutOf,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    /// Index into [`COLORS`].
    pub color: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Lane of the commit.
    pub lane: usize,
    pub color: usize,
    /// Lanes the row spans, for its width.
    pub width: usize,
    pub edges: Vec<Edge>,
    pub merge: bool,
}

/// Lays out `commits` (hash and parents, in topological order, newest
/// first). The line starting at `head` gets the first color.
pub fn layout<'a>(
    commits: impl IntoIterator<Item = (&'a str, &'a [String])>,
    head: Option<&str>,
) -> Vec<Row> {
    let mut lanes: Vec<Option<(String, usize)>> = Vec::new();
    let mut next = 0;
    let mut color = |lanes: &[Option<(String, usize)>]| -> usize {
        // Skip colors of neighbouring lanes when possible.
        for _ in 0..COLORS.len() - 1 {
            next = next % (COLORS.len() - 1) + 1;
            if !lanes.iter().flatten().any(|(_, c)| *c == next) {
                return next;
            }
        }
        next
    };
    let free = |lanes: &mut Vec<Option<(String, usize)>>| -> usize {
        match lanes.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                lanes.push(None);
                lanes.len() - 1
            }
        }
    };
    let mut rows = Vec::new();
    for (hash, parents) in commits {
        let expected = lanes
            .iter()
            .position(|l| l.as_ref().is_some_and(|(h, _)| h == hash));
        let (lane, own) = match expected {
            Some(i) => (i, lanes[i].as_ref().map_or(0, |(_, c)| *c)),
            None => {
                let i = free(&mut lanes);
                let c = if Some(hash) == head { 0 } else { color(&lanes) };
                (i, c)
            }
        };
        let inputs = lanes.clone();
        let mut edges = Vec::new();
        for (j, input) in inputs.iter().enumerate() {
            match input {
                Some((h, c)) if h == hash => edges.push(Edge {
                    from: j,
                    to: lane,
                    kind: EdgeKind::Into,
                    color: *c,
                }),
                Some((_, c)) => edges.push(Edge {
                    from: j,
                    to: j,
                    kind: EdgeKind::Through,
                    color: *c,
                }),
                None => {}
            }
        }
        // Every line that was waiting for this commit ends here.
        for slot in lanes.iter_mut() {
            if slot.as_ref().is_some_and(|(h, _)| h == hash) {
                *slot = None;
            }
        }
        for (n, parent) in parents.iter().enumerate() {
            let waiting = lanes
                .iter()
                .position(|l| l.as_ref().is_some_and(|(h, _)| h == parent));
            let (to, c) = match waiting {
                // The first parent waits further right: pull that line over
                // into this commit's lane so history stays on the left.
                Some(k) if n == 0 && k > lane && lanes[lane].is_none() => {
                    lanes[lane] = lanes[k].take().map(|(h, _)| (h, own));
                    if let Some(edge) = edges
                        .iter_mut()
                        .find(|e| e.kind == EdgeKind::Through && e.from == k)
                    {
                        edge.to = lane;
                    }
                    (lane, own)
                }
                // The parent already has a line: join it.
                Some(k) => (k, if n == 0 { own } else { lanes[k].as_ref().map_or(own, |(_, c)| *c) }),
                None if n == 0 && lanes.get(lane).is_some_and(Option::is_none) => {
                    lanes[lane] = Some((parent.clone(), own));
                    (lane, own)
                }
                None => {
                    let k = free(&mut lanes);
                    let c = color(&lanes);
                    lanes[k] = Some((parent.clone(), c));
                    (k, c)
                }
            };
            edges.push(Edge {
                from: lane,
                to,
                kind: EdgeKind::OutOf,
                color: c,
            });
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        rows.push(Row {
            lane,
            color: own,
            width: inputs.len().max(lanes.len()).max(lane + 1),
            edges,
            merge: parents.len() > 1,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(commits: &[(&str, &[&str])], head: Option<&str>) -> Vec<Row> {
        let owned: Vec<(String, Vec<String>)> = commits
            .iter()
            .map(|(h, p)| (h.to_string(), p.iter().map(|s| s.to_string()).collect()))
            .collect();
        layout(owned.iter().map(|(h, p)| (h.as_str(), p.as_slice())), head)
    }

    /// Every line leaving a row's bottom must enter the next row's top.
    fn continuous(rows: &[Row]) {
        for pair in rows.windows(2) {
            let mut bottom: Vec<usize> = pair[0]
                .edges
                .iter()
                .filter(|e| e.kind != EdgeKind::Into)
                .map(|e| e.to)
                .collect();
            let mut top: Vec<usize> = pair[1]
                .edges
                .iter()
                .filter(|e| e.kind != EdgeKind::OutOf)
                .map(|e| e.from)
                .collect();
            bottom.sort();
            bottom.dedup();
            top.sort();
            top.dedup();
            assert_eq!(bottom, top, "{pair:?}");
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = run(&[("c", &["b"]), ("b", &["a"]), ("a", &[])], Some("c"));
        assert!(rows.iter().all(|r| r.lane == 0 && r.color == 0));
        assert_eq!(rows[2].edges.len(), 1);
        continuous(&rows);
    }

    #[test]
    fn merge_opens_and_closes_a_second_lane() {
        // m merges f into b; f and b both come from a.
        let rows = run(
            &[("m", &["b", "f"]), ("f", &["a"]), ("b", &["a"]), ("a", &[])],
            Some("m"),
        );
        assert!(rows[0].merge);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        // f's line joins a's lane instead of waiting in its own.
        assert_eq!(rows[3].lane, 0);
        assert_ne!(rows[1].color, rows[0].color);
        continuous(&rows);
    }

    #[test]
    fn two_branch_tips_and_a_gap_are_reused() {
        let rows = run(
            &[
                ("x", &["a"]),
                ("y", &["b"]),
                ("b", &["a"]),
                ("z", &["a"]),
                ("a", &[]),
            ],
            Some("y"),
        );
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[1].color, 0);
        continuous(&rows);
        assert!(rows.iter().all(|r| r.width <= 3));
    }
}
