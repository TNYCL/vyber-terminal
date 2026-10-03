//! Terminal link detection. Grid inspection is bounded and never accesses disk;
//! ambiguous wrapped paths are resolved on the background executor.
use alacritty_terminal::{
    event::EventListener,
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Term, cell::Flags},
};
use std::{ops::Range, path::PathBuf};

const MAX_ROWS: i32 = 8;
const MAX_CHARS: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileLink {
    pub path: PathBuf,
    /// One-based line and column; absent for a normal file preview.
    pub location: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    File(FileLink),
    Url(String),
}

impl Target {
    pub fn label(&self) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::File(file) => match file.location {
                Some((line, column)) => format!("{}:{line}:{column}", file.path.display()),
                None => file.path.display().to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: Target,
    pub cells: Vec<Point>,
    /// An OSC 8 destination or a quoted path beats whitespace fragments.
    explicit: bool,
}

impl Link {
    pub fn spans(&self) -> Vec<(Line, Range<usize>)> {
        let mut spans: Vec<(Line, Range<usize>)> = Vec::new();
        for point in &self.cells {
            if let Some((line, columns)) = spans.last_mut()
                && *line == point.line
                && columns.end == point.column.0
            {
                columns.end += 1;
            } else {
                spans.push((point.line, point.column.0..point.column.0 + 1));
            }
        }
        spans
    }
}

#[derive(Default)]
struct Text {
    value: String,
    cells: Vec<(Range<usize>, Point)>,
}

impl Text {
    fn row<T: EventListener>(&mut self, term: &Term<T>, line: Line, skip_indent: bool, trim: bool) {
        let grid = term.grid();
        let cols = term.columns();
        let start = if skip_indent {
            (0..cols)
                .find(|&col| !grid[Point::new(line, Column(col))].c.is_whitespace())
                .unwrap_or(cols)
        } else {
            0
        };
        let end = if trim {
            (start..cols)
                .rfind(|&col| !grid[Point::new(line, Column(col))].c.is_whitespace())
                .map_or(start, |col| col + 1)
        } else {
            cols
        };
        for col in start..end {
            let point = Point::new(line, Column(col));
            let cell = &grid[point];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let begin = self.value.len();
            self.value.push(if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            });
            if !cell.flags.contains(Flags::HIDDEN)
                && let Some(extra) = cell.zerowidth()
            {
                self.value.extend(extra);
            }
            let range = begin..self.value.len();
            self.cells.push((range.clone(), point));
            if cell.flags.contains(Flags::WIDE_CHAR) && col + 1 < cols {
                self.cells.push((range, Point::new(line, Column(col + 1))));
            }
            if self.cells.len() >= MAX_CHARS {
                break;
            }
        }
    }

    fn links(&self, point: Point, root: &std::path::Path) -> Vec<Link> {
        let mut ranges = Vec::new();
        // Quoted/enclosed paths may contain spaces. Keep their cell positions,
        // including wide characters, instead of equating bytes with columns.
        for (start, open) in self.value.char_indices() {
            let close = match open {
                '\'' | '"' | '`' => open,
                '(' => ')',
                '[' => ']',
                '<' => '>',
                _ => continue,
            };
            let begin = start + open.len_utf8();
            if let Some(end) = self.value[begin..].find(close) {
                ranges.push((begin..begin + end, matches!(open, '\'' | '"' | '`')));
            }
        }
        let mut start = None;
        for (index, c) in self.value.char_indices() {
            if c.is_whitespace() {
                if let Some(start) = start.take() {
                    ranges.push((start..index, false));
                }
            } else if start.is_none() {
                start = Some(index);
            }
        }
        if let Some(start) = start {
            ranges.push((start..self.value.len(), false));
        }
        let mut links = Vec::new();
        for (range, explicit) in ranges {
            let raw = &self.value[range.clone()];
            let cleaned = clean(raw);
            if cleaned.is_empty() {
                continue;
            }
            let begin = range.start + raw.len() - raw.trim_start_matches(opening).len();
            let end = begin + cleaned.len();
            let cells: Vec<_> = self
                .cells
                .iter()
                .filter(|(r, _)| r.start < end && r.end > begin)
                .map(|(_, point)| *point)
                .collect();
            if !cells.contains(&point) {
                continue;
            }
            if let Some(target) = parse(cleaned, root) {
                let link = Link {
                    target,
                    cells,
                    explicit,
                };
                if !links.contains(&link) {
                    links.push(link);
                }
            }
        }
        links
    }
}

fn opening(c: char) -> bool {
    matches!(c, '\'' | '"' | '`' | '(' | '[' | '<')
}

fn clean(text: &str) -> &str {
    let text = text.trim_start_matches(opening);
    let mut end = text.len();
    while let Some(c) = text[..end].chars().next_back() {
        let trim = matches!(c, '\'' | '"' | '`' | ',' | ';' | '.' | '!' | '?' | '>')
            || (c == ')' && text[..end].matches(')').count() > text[..end].matches('(').count())
            || (c == ']' && text[..end].matches(']').count() > text[..end].matches('[').count());
        if !trim {
            break;
        }
        end -= c.len_utf8();
    }
    &text[..end]
}

fn hash_location(text: &str) -> Option<(usize, usize)> {
    let text = text.strip_prefix('L')?;
    let (line, column) = text.split_once('C').unwrap_or((text, "1"));
    Some((
        line.parse::<usize>().ok()?.max(1),
        column.parse::<usize>().ok()?.max(1),
    ))
}

pub fn parse(text: &str, root: &std::path::Path) -> Option<Target> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    if text.starts_with("http://") || text.starts_with("https://") {
        let url = url::Url::parse(text).ok()?;
        return url.host_str().map(|_| Target::Url(text.to_owned()));
    }
    if text.starts_with("file://") {
        let url = url::Url::parse(text).ok()?;
        let location = url.fragment().and_then(hash_location);
        return Some(Target::File(FileLink {
            path: url.to_file_path().ok()?,
            location,
        }));
    }
    let mut path = text;
    let mut location = None;
    if let Some((before, suffix)) = path.rsplit_once('#')
        && let Some(position) = hash_location(suffix)
    {
        path = before;
        location = Some(position);
    } else if let Some((before, suffix)) = path.rsplit_once(':')
        && let Ok(number) = suffix.parse::<usize>()
    {
        path = before;
        location = Some((number.max(1), 1));
        if let Some((before, suffix)) = path.rsplit_once(':')
            && let Ok(line) = suffix.parse::<usize>()
        {
            path = before;
            location = Some((line.max(1), number.max(1)));
        }
    }
    let file_name = path.rsplit(['/', '\\']).next()?;
    let named_file =
        file_name.rsplit_once('.').is_some_and(|(stem, extension)| {
            !stem.is_empty()
                && !extension.is_empty()
                && extension.len() <= 16
                && extension
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }) || (file_name.starts_with('.') && file_name.len() > 1 && !file_name.ends_with('.'))
            || matches!(
                file_name,
                "README" | "LICENSE" | "Makefile" | "Dockerfile" | "Gemfile" | "Justfile"
            );
    if !named_file && !path.contains('/') && !path.contains('\\') {
        return None;
    }
    if path.is_empty() || path.ends_with(['/', '\\']) || path.contains("://") {
        return None;
    }
    let path = if let Some(relative) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
    {
        dirs::home_dir()?.join(relative)
    } else if cfg!(windows)
        && path.starts_with('/')
        && path.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
        && path.as_bytes().get(2) == Some(&b'/')
    {
        PathBuf::from(format!("{}:{}", &path[1..2], &path[2..]))
    } else {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    Some(Target::File(FileLink { path, location }))
}

fn continuation<T: EventListener>(term: &Term<T>, line: Line) -> Option<bool> {
    if line >= term.bottommost_line() {
        return None;
    }
    let grid = term.grid();
    let cols = term.columns();
    if grid[Point::new(line, Column(cols - 1))]
        .flags
        .contains(Flags::WRAPLINE)
    {
        return Some(false);
    }
    // TUIs such as Codex wrap before writing rows, so there is no WRAPLINE.
    // Only join text at the right margin with a short, equally styled indent.
    let last = (0..cols).rfind(|&col| !grid[Point::new(line, Column(col))].c.is_whitespace())?;
    let first = (0..cols).find(|&col| {
        !grid[Point::new(Line(line.0 + 1), Column(col))]
            .c
            .is_whitespace()
    })?;
    let a = &grid[Point::new(line, Column(last))];
    let b = &grid[Point::new(Line(line.0 + 1), Column(first))];
    (last + 4 >= cols
        && first <= 8
        && a.fg == b.fg
        && a.bg == b.bg
        && a.flags.intersection(Flags::BOLD | Flags::ITALIC)
            == b.flags.intersection(Flags::BOLD | Flags::ITALIC)
        && !matches!(a.c, ')' | ']' | ',' | ';' | '.' | '!' | '?')
        && !matches!(b.c, '(' | '[' | '\'' | '"' | '`'))
    .then_some(true)
}

pub fn detect<T: EventListener>(term: &Term<T>, point: Point, root: &std::path::Path) -> Vec<Link> {
    if point.line < term.topmost_line()
        || point.line > term.bottommost_line()
        || point.column.0 >= term.columns()
    {
        return Vec::new();
    }
    if let Some(hyperlink) = term.grid()[point].hyperlink()
        && let Some(target) = parse(hyperlink.uri(), root)
    {
        let mut cells = Vec::new();
        for line in (point.line.0 - MAX_ROWS).max(term.topmost_line().0)
            ..=(point.line.0 + MAX_ROWS).min(term.bottommost_line().0)
        {
            for col in 0..term.columns() {
                let p = Point::new(Line(line), Column(col));
                if term.grid()[p].hyperlink().as_ref() == Some(&hyperlink) {
                    cells.push(p);
                }
            }
        }
        return vec![Link {
            target,
            cells,
            explicit: true,
        }];
    }
    let mut first = point.line;
    while first > term.topmost_line() && point.line.0 - first.0 < MAX_ROWS {
        let previous = Line(first.0 - 1);
        if continuation(term, previous).is_none() {
            break;
        }
        first = previous;
    }
    let mut text = Text::default();
    let mut line = first;
    let mut skip_indent = false;
    loop {
        let next = continuation(term, line);
        text.row(term, line, skip_indent, next == Some(true));
        if next.is_none() || line.0 - first.0 >= MAX_ROWS || text.cells.len() >= MAX_CHARS {
            break;
        }
        skip_indent = next == Some(true);
        line = Line(line.0 + 1);
    }
    let mut links = text.links(point, root);
    // Keep the unjoined row as an alternative. Disk validation in resolve()
    // prevents a plausible neighbouring line from replacing a real file.
    if first != point.line || line != point.line {
        let mut row = Text::default();
        row.row(term, point.line, false, false);
        for link in row.links(point, root) {
            if !links.contains(&link) {
                links.push(link);
            }
        }
    }
    links
}

pub fn resolve(links: &[Link]) -> Result<Option<Link>, &'static str> {
    let explicit = links.iter().any(|link| link.explicit);
    let choices = || links.iter().filter(|link| !explicit || link.explicit);
    let mut candidates = choices();
    let first = candidates.next();
    if candidates.next().is_none() {
        return Ok(first.cloned());
    }
    let mut found: Option<&Link> = None;
    for link in choices() {
        if matches!(&link.target, Target::Url(_)) {
            return Ok(Some(link.clone()));
        }
        let Target::File(file) = &link.target else {
            continue;
        };
        if file.path.is_file() {
            if let Some(previous) = found
                && previous.target != link.target
            {
                return Err("Multiple files match this link. Use the full file path.");
            }
            found = found.or(Some(link));
        }
    }
    // An explicit missing path is still a link: opening it gives an error,
    // rather than silently turning Ctrl+click into a text selection.
    Ok(found.or_else(|| choices().next()).cloned())
}

#[cfg(test)]
mod tests {
    use super::super::{Proxy, Size};
    use super::*;
    use alacritty_terminal::grid::Scroll;
    use std::path::Path;

    fn screen(text: &str, cols: usize, rows: usize) -> Term<Proxy> {
        let (proxy, _events) = Proxy::channel();
        let mut term = Term::new(Default::default(), &Size { cols, rows }, proxy);
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut term, text.as_bytes());
        term
    }
    fn file(link: &Link) -> &FileLink {
        let Target::File(file) = &link.target else {
            panic!("expected a file link")
        };
        file
    }
    fn hit(term: &Term<Proxy>, row: i32, col: usize, root: &Path) -> Link {
        resolve(&detect(term, Point::new(Line(row), Column(col)), root))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn screenshot_report_punctuation_opens_the_file_without_a_line_target() {
        let root = tempfile::tempdir().unwrap();
        let name = "DOTLOOM-DEVELOPMENT-REPORT-2026-10-02.md";
        let path = root.path().join(name);
        std::fs::write(&path, "# Report").unwrap();
        let term = screen(&format!("Rapor ({name})."), 120, 4);
        let link = hit(&term, 0, 12, root.path());
        assert_eq!(
            file(&link),
            &FileLink {
                path,
                location: None
            }
        );
        assert_eq!(link.spans(), vec![(Line(0), 7..7 + name.len())]);
    }

    #[test]
    fn quotes_backticks_and_balanced_parentheses_preserve_the_filename() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("a(test).md");
        std::fs::write(&path, "report").unwrap();
        for text in ["'a(test).md';", "`a(test).md`", "(a(test).md)."] {
            let term = screen(text, 80, 4);
            assert_eq!(file(&hit(&term, 0, 4, root.path())).path, path, "{text}");
        }
    }

    #[test]
    fn quoted_absolute_paths_include_spaces_unicode_and_wide_cells() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("设计项目");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("rapor Türkçe.md");
        std::fs::write(&path, "report").unwrap();
        // A file matching only the last whitespace fragment must not steal
        // clicks from an explicit quoted path.
        std::fs::write(root.path().join("Türkçe.md"), "other file").unwrap();
        let term = screen(
            &format!("Test-Path -LiteralPath '{}'", path.display()),
            240,
            4,
        );
        let wide = (0..term.columns())
            .find(|&col| {
                term.grid()[Point::new(Line(0), Column(col))]
                    .flags
                    .contains(Flags::WIDE_CHAR_SPACER)
            })
            .unwrap();
        let link = hit(&term, 0, wide, root.path());
        assert_eq!(file(&link).path, path);
        assert!(link.cells.contains(&Point::new(Line(0), Column(wide - 1))));
        assert!(link.cells.contains(&Point::new(Line(0), Column(wide))));
        // Clicking the space inside a quoted path opens that same full path.
        let space = link
            .cells
            .iter()
            .copied()
            .find(|&p| term.grid()[p].c == ' ')
            .unwrap();
        assert_eq!(
            file(&hit(&term, space.line.0, space.column.0, root.path())).path,
            path
        );
    }

    #[test]
    fn missing_command_path_is_a_link_and_does_not_create_a_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("DOTLOOM-GOAL-BRIEF.md");
        let term = screen(
            &format!("Test-Path -LiteralPath '{}'", path.display()),
            240,
            4,
        );
        let link = hit(&term, 0, 35, root.path());
        assert_eq!(file(&link).path, path);
        assert_eq!(file(&link).location, None);
        assert!(!path.exists());
    }

    #[test]
    fn explicit_locations_keep_line_column_and_preview_links_have_none() {
        let root = Path::new("workspace");
        for (suffix, location) in [
            ("", None),
            (":42", Some((42, 1))),
            (":42:7", Some((42, 7))),
            ("#L42", Some((42, 1))),
            ("#L42C7", Some((42, 7))),
        ] {
            let Target::File(file) = parse(&format!("src/main.rs{suffix}"), root).unwrap() else {
                unreachable!()
            };
            assert_eq!(file.path, root.join("src/main.rs"));
            assert_eq!(file.location, location);
        }
    }

    #[test]
    fn terminal_soft_wrap_is_clickable_from_each_row() {
        let root = tempfile::tempdir().unwrap();
        let name = "DOTLOOM-DEVELOPMENT-REPORT-2026-10-02.md";
        let path = root.path().join(name);
        std::fs::write(&path, "report").unwrap();
        let term = screen(&format!("Rapor ({name})."), 24, 5);
        for (row, col) in [(0, 12), (1, 5)] {
            let link = hit(&term, row, col, root.path());
            assert_eq!(file(&link).path, path);
            assert_eq!(link.spans().len(), 2);
        }
    }

    #[test]
    fn codex_hard_wrap_and_indent_are_clickable_from_both_halves() {
        let root = tempfile::tempdir().unwrap();
        let name = "DOTLOOM-DEVELOPMENT-REPORT-2026-10-02.md";
        let path = root.path().join(name);
        std::fs::write(&path, "report").unwrap();
        let split = name.find("10-02").unwrap();
        let prefix = " ".repeat(50 - split - 1);
        let term = screen(
            &format!("{prefix}({}\r\n  {}).", &name[..split], &name[split..]),
            52,
            4,
        );
        assert!(
            !term.grid()[Point::new(Line(0), Column(51))]
                .flags
                .contains(Flags::WRAPLINE)
        );
        for (row, col) in [(0, prefix.len() + 5), (1, 4)] {
            let link = hit(&term, row, col, root.path());
            assert_eq!(file(&link).path, path);
            assert_eq!(link.spans().len(), 2);
        }
    }

    #[test]
    fn unrelated_rows_do_not_replace_an_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.rs");
        std::fs::write(&path, "source").unwrap();
        let term = screen("             source.rs\r\n  README.md", 24, 4);
        assert_eq!(file(&hit(&term, 0, 16, root.path())).path, path);
        // If both interpretations exist, never silently open the wrong file.
        std::fs::write(root.path().join("source.rsREADME.md"), "unrelated").unwrap();
        assert!(resolve(&detect(&term, Point::new(Line(0), Column(16)), root.path())).is_err());
    }

    #[test]
    fn differing_styles_are_not_treated_as_a_tui_path_continuation() {
        let root = tempfile::tempdir().unwrap();
        let term = screen(
            "\x1b[32m             source.rs\r\n\x1b[0m  README.md",
            24,
            4,
        );
        let links = detect(&term, Point::new(Line(0), Column(16)), root.path());
        assert!(
            links
                .iter()
                .all(|link| file(link).path == root.path().join("source.rs"))
        );
    }

    #[test]
    fn osc8_preserves_hidden_file_destinations_across_labels_and_wraps() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("report with space.md");
        let mut uri = url::Url::from_file_path(&path).unwrap();
        uri.set_fragment(Some("L8C3"));
        let term = screen(
            &format!("\x1b]8;;{uri}\x07open this report\x1b]8;;\x07"),
            10,
            4,
        );
        let link = hit(&term, 1, 3, root.path());
        assert_eq!(
            file(&link),
            &FileLink {
                path,
                location: Some((8, 3))
            }
        );
        assert_eq!(link.spans(), vec![(Line(0), 0..10), (Line(1), 0..6)]);
    }

    #[test]
    #[cfg(windows)]
    fn windows_drive_colons_and_raw_osc8_paths_are_supported() {
        let root = Path::new(r"C:\workspace");
        let path = PathBuf::from(r"C:\Users\tunay\Documents\report.md");
        let Target::File(target) = parse(r"C:\Users\tunay\Documents\report.md:42:7", root).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(target.path, path);
        assert_eq!(target.location, Some((42, 7)));
        let term = screen(
            &format!("\x1b]8;;{}\x07report\x1b]8;;\x07", path.display()),
            80,
            4,
        );
        assert_eq!(file(&hit(&term, 0, 3, root)).path, path);
    }

    #[test]
    fn scrollback_links_keep_their_grid_positions() {
        let root = tempfile::tempdir().unwrap();
        let mut term = screen("report.md\r\nsecond\r\nthird\r\nfourth\r\nfifth", 40, 3);
        term.scroll_display(Scroll::Top);
        let p = term
            .renderable_content()
            .display_iter
            .find(|cell| cell.c == 'r')
            .unwrap()
            .point;
        assert!(p.line.0 < 0);
        let link = hit(&term, p.line.0, p.column.0, root.path());
        assert_eq!(file(&link).path, root.path().join("report.md"));
        assert_eq!(link.spans(), vec![(p.line, 0..9)]);
    }

    #[test]
    fn urls_trim_prose_punctuation_and_keep_balanced_parentheses() {
        let root = Path::new("workspace");
        for (text, expected) in [
            ("(https://example.com/doc).", "https://example.com/doc"),
            (
                "https://example.com/doc_(part).",
                "https://example.com/doc_(part)",
            ),
        ] {
            let term = screen(text, 100, 4);
            assert_eq!(hit(&term, 0, 10, root).target, Target::Url(expected.into()));
        }
    }

    #[test]
    fn ordinary_words_and_hidden_text_do_not_become_links() {
        let root = Path::new("workspace");
        for text in ["ordinary words", "\x1b[8msecret.md\x1b[0m"] {
            let term = screen(text, 80, 4);
            assert!(detect(&term, Point::new(Line(0), Column(3)), root).is_empty());
        }
    }
}
