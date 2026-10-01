//! Input detection only chooses a badge's position; it never decides whether
//! an active turn's badge exists. No mouse/hover state participates here.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BadgePosition {
    Cell { row: usize, right: usize },
    Corner,
}

#[derive(Default)]
pub(super) struct BadgeAnchorMemory {
    turn: Option<String>,
    last: Option<(usize, usize, usize, usize)>,
}

impl BadgeAnchorMemory {
    /// Retain a verified live-screen anchor through partial TUI redraws.
    /// Scrollback uses the corner instead of covering historical input, and
    /// resizing or changing turns invalidates the previous cell coordinates.
    pub fn select(
        &mut self,
        turn: &str,
        active: bool,
        detected: Option<(usize, usize)>,
        grid: (usize, usize),
        live: bool,
    ) -> Option<BadgePosition> {
        if self.turn.as_deref() != Some(turn) {
            self.turn = Some(turn.to_owned());
            self.last = None;
        }
        let (rows, columns) = grid;
        if self
            .last
            .is_some_and(|(row, right, old_rows, old_columns)| {
                (old_rows, old_columns) != grid || row >= rows || right >= columns
            })
        {
            self.last = None;
        }
        if live {
            if let Some((row, right)) =
                detected.filter(|(row, right)| *row < rows && *right < columns)
            {
                self.last = Some((row, right, rows, columns));
                return Some(BadgePosition::Cell { row, right });
            }
            if active && let Some((row, right, _, _)) = self.last {
                return Some(BadgePosition::Cell { row, right });
            }
        }
        active.then_some(BadgePosition::Corner)
    }
}

pub(super) fn badge_visible(active: bool, files: usize, dismissed: bool) -> bool {
    !dismissed && (active || files > 0)
}

/// Clamp to the current pixel bounds, including during a resize before the
/// PTY has reflowed. The caller also bounds the badge's own width and height.
pub(super) fn fit_badge(
    position: BadgePosition,
    viewport: (f32, f32),
    cell: (f32, f32),
    badge: (f32, f32),
    inset: f32,
) -> Option<(f32, f32)> {
    let (width, height) = viewport;
    if width <= 0. || height <= 0. {
        return None;
    }
    let (top, right) = match position {
        BadgePosition::Cell { row, right } => (
            inset + row as f32 * cell.1 + (cell.1 - badge.1) / 2.,
            width - (inset + (right + 1) as f32 * cell.0),
        ),
        BadgePosition::Corner => (inset, inset),
    };
    Some((
        top.clamp(0., (height - badge.1).max(0.)),
        right.clamp(0., (width - badge.0).max(0.)),
    ))
}

fn blank(c: char) -> bool {
    c == ' ' || c == '\0'
}

fn prompt_start(row: &[(char, bool)]) -> Option<usize> {
    let start = row
        .iter()
        .position(|(c, _)| !blank(*c) && !matches!(c, '│' | '┃'))?;
    (start <= 4
        && matches!(row[start].0, '❯' | '›' | '>')
        && row.get(start + 1).is_none_or(|(c, _)| blank(*c)))
    .then_some(start)
}

/// A border's own span matters, not its fraction of the entire terminal.
/// This also recognizes a small boxed composer in a wide terminal.
fn border_span(row: &[(char, bool)]) -> Option<(usize, usize)> {
    let start = row.iter().position(|(c, _)| !blank(*c))?;
    let end = row.iter().rposition(|(c, _)| !blank(*c))?;
    let horizontal = |c| matches!(c, '─' | '━' | '═' | '╌' | '┄');
    let border = |c| {
        horizontal(c)
            || matches!(
                c,
                '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴'
            )
    };
    (row[start..=end].iter().all(|(c, _)| border(*c))
        && row[start..=end]
            .iter()
            .filter(|(c, _)| horizontal(*c))
            .count()
            >= 8)
        .then_some((start, end))
}

/// Find a real composer, not the lowest `❯` in a task/menu list. Search the
/// entire visible screen so tall task footers cannot push the input out of
/// an arbitrary last-16-rows window. Framed inputs take priority over the
/// unboxed Codex composer; both the upper and lower frame must agree.
pub fn badge_spot(screen: &[Vec<(char, bool)>], need: usize) -> Option<(usize, usize)> {
    let height = screen.len();
    let width = screen.first()?.len();
    if height < 2 || width < need + 4 || screen.iter().any(|row| row.len() != width) {
        return None;
    }
    let borders: Vec<_> = screen.iter().map(|row| border_span(row)).collect();
    let framed = (1..height).rev().find_map(|prompt| {
        let start = prompt_start(&screen[prompt])?;
        let (left, right) = borders[prompt - 1]?;
        if start < left || start > right {
            return None;
        }
        let bottom = (prompt + 1..height).find_map(|row| borders[row])?;
        (bottom == (left, right)).then_some((prompt - 1, right.min(width - 2), true))
    });
    let (top, right, framed) = framed.or_else(|| {
        (1..height).rev().find_map(|prompt| {
            let row = &screen[prompt];
            let start = prompt_start(row)?;
            if row[start].0 != '›' || !screen[prompt - 1].iter().all(|(c, _)| blank(*c)) {
                return None;
            }
            // Codex's unboxed composer is followed by its keyboard hint.
            // A task selector (e.g. `› ● main`) is not a composer.
            let text: String = row[start + 1..].iter().map(|(c, _)| c).collect();
            if text.trim_start().starts_with(['●', '○', '◉', '◌', '⏺']) {
                return None;
            }
            let footer = screen[prompt + 1..].iter().take(4).any(|row| {
                let text: String = row.iter().map(|(c, _)| c).collect();
                text.contains('⏎')
                    || text.contains("? for shortcuts")
                    || text.contains("context left")
            });
            footer.then_some((prompt, width - 2, false))
        })
    })?;
    if right + 1 < need || top == 0 {
        return None;
    }
    let left = right + 1 - need;
    let free = |row: usize| {
        screen[row][left..=right].iter().all(|(c, plain)| {
            // Outside a verified frame, hover backgrounds on otherwise
            // empty cells must not move the badge. The unboxed composer
            // still avoids its own shaded padding.
            blank(*c) && (framed || *plain)
        })
    };
    (top.saturating_sub(6)..top)
        .rev()
        .find(|&row| free(row))
        .or(Some(top - 1))
        .map(|row| (row, right))
}

#[cfg(test)]
mod tests {
    use super::{BadgeAnchorMemory, BadgePosition, badge_spot, badge_visible, fit_badge};

    fn screen(rows: &[&str], width: usize) -> Vec<Vec<(char, bool)>> {
        rows.iter()
            .map(|row| {
                let mut cells: Vec<_> = row.chars().map(|c| (c, true)).collect();
                cells.resize(width, (' ', true));
                cells
            })
            .collect()
    }

    #[test]
    fn task_selectors_never_override_the_real_input() {
        let rule = "─".repeat(80);
        let rows = [
            "✻ Befuddling… (3h 25m)",
            "",
            &rule,
            "❯ ",
            &rule,
            "auto mode on · 2 shells",
            "",
            "❯ ● main",
            "○ fork Running clippy",
            "○ fork Reading tests",
        ];
        let mut cells = screen(&rows, 80);
        assert_eq!(badge_spot(&cells, 30), Some((1, 78)));
        // Hover can change the selection arrow and background independently.
        cells[7] = screen(&["› ● main"], 80).remove(0);
        cells[1].iter_mut().for_each(|cell| cell.1 = false);
        assert_eq!(badge_spot(&cells, 30), Some((1, 78)));
    }

    #[test]
    fn input_remains_detectable_above_a_long_task_footer() {
        let rule = "─".repeat(80);
        let mut rows = vec!["", &rule, "❯ ", &rule];
        rows.extend(std::iter::repeat_n("○ fork Working…", 40));
        assert_eq!(badge_spot(&screen(&rows, 80), 30), Some((0, 78)));
    }

    #[test]
    fn frameless_task_and_menu_selectors_are_not_inputs() {
        for selector in [
            "❯ ● main",
            "❯ fork Reading tests",
            "> Run tests",
            "› ● main",
        ] {
            assert_eq!(
                badge_spot(&screen(&["", selector, "", "? for shortcuts"], 80), 20),
                None
            );
        }
        // A bare arrow with no frame or composer hint is not enough proof.
        assert_eq!(badge_spot(&screen(&["", "› some item"], 80), 20), None);
    }

    #[test]
    fn framed_input_can_contain_the_same_text_as_a_task_selector() {
        let rule = "─".repeat(80);
        assert_eq!(
            badge_spot(&screen(&["", &rule, "❯ ● main", &rule], 80), 20),
            Some((0, 78))
        );
    }

    #[test]
    fn small_boxed_and_multiline_composers_are_recognized() {
        let rows = [
            "",
            "╭──────────────────────────────╮",
            "│ > type here                  │",
            "│ more text                    │",
            "╰──────────────────────────────╯",
            "❯ ● main",
        ];
        assert_eq!(badge_spot(&screen(&rows, 100), 12), Some((0, 31)));
    }

    #[test]
    fn partial_frames_and_ragged_screens_are_rejected() {
        let rule = "─".repeat(80);
        assert_eq!(badge_spot(&screen(&["", &rule, "❯ "], 80), 20), None);
        let mut cells = screen(&["", &rule, "❯ ", &rule], 80);
        cells[1].pop();
        assert_eq!(badge_spot(&cells, 20), None);
    }

    #[test]
    fn latest_real_composer_wins_over_old_transcript_input() {
        let rule = "─".repeat(80);
        let rows = [
            "",
            &rule,
            "❯ old prompt",
            &rule,
            "response",
            "",
            &rule,
            "❯ new prompt",
            &rule,
            "",
            "❯ ● main",
        ];
        assert_eq!(badge_spot(&screen(&rows, 80), 20), Some((5, 78)));
    }

    #[test]
    fn active_badge_keeps_its_anchor_through_partial_redraws() {
        let mut memory = BadgeAnchorMemory::default();
        let anchor = BadgePosition::Cell { row: 10, right: 78 };
        assert_eq!(
            memory.select("turn-1", true, Some((10, 78)), (30, 80), true),
            Some(anchor)
        );
        for _ in 0..200 {
            assert_eq!(
                memory.select("turn-1", true, None, (30, 80), true),
                Some(anchor)
            );
        }
        assert_eq!(memory.select("turn-1", false, None, (30, 80), true), None);
    }

    #[test]
    fn active_badge_without_an_input_has_a_visible_fallback() {
        let mut memory = BadgeAnchorMemory::default();
        assert_eq!(
            memory.select("turn-1", true, None, (30, 80), true),
            Some(BadgePosition::Corner)
        );
        assert_eq!(memory.select("turn-1", false, None, (30, 80), true), None);
        assert!(badge_visible(true, 0, false));
        assert!(badge_visible(true, 506, false));
        assert!(badge_visible(false, 506, false));
        assert!(!badge_visible(false, 0, false));
        assert!(!badge_visible(true, 506, true));
    }

    #[test]
    fn scrollback_keeps_active_badge_visible_without_using_historical_input() {
        let mut memory = BadgeAnchorMemory::default();
        let anchor = BadgePosition::Cell { row: 10, right: 78 };
        memory.select("turn-1", true, Some((10, 78)), (30, 80), true);
        assert_eq!(
            memory.select("turn-1", true, Some((1, 20)), (30, 80), false),
            Some(BadgePosition::Corner)
        );
        assert_eq!(
            memory.select("turn-1", true, None, (30, 80), true),
            Some(anchor)
        );
        assert_eq!(memory.select("turn-1", false, None, (30, 80), false), None);
    }

    #[test]
    fn resizing_and_new_turns_never_reuse_stale_cell_coordinates() {
        let mut memory = BadgeAnchorMemory::default();
        memory.select("turn-1", true, Some((25, 78)), (30, 80), true);
        assert_eq!(
            memory.select("turn-1", true, None, (10, 40), true),
            Some(BadgePosition::Corner)
        );
        assert_eq!(
            memory.select("turn-1", true, None, (30, 80), true),
            Some(BadgePosition::Corner)
        );
        memory.select("turn-1", true, Some((10, 78)), (30, 80), true);
        assert_eq!(
            memory.select("turn-2", true, None, (30, 80), true),
            Some(BadgePosition::Corner)
        );
        assert_eq!(
            memory.select("turn-2", true, Some((100, 100)), (30, 80), true),
            Some(BadgePosition::Corner)
        );
    }

    #[test]
    fn placement_is_clamped_during_pixel_resize_and_in_narrow_panes() {
        let position = BadgePosition::Cell { row: 29, right: 78 };
        let (top, right) = fit_badge(position, (100., 50.), (8., 20.), (96., 20.), 3.).unwrap();
        assert_eq!((top, right), (30., 0.));
        assert!(top + 20. <= 50. && right + 96. <= 100.);
        assert_eq!(
            fit_badge(
                BadgePosition::Corner,
                (100., 50.),
                (8., 20.),
                (96., 20.),
                3.
            ),
            Some((3., 3.))
        );
        assert_eq!(
            fit_badge(position, (0., 0.), (8., 20.), (96., 20.), 3.),
            None
        );
    }
}
