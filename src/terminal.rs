use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as TermPoint, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags},
    tty,
    vte::ansi::Color,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Sizable,
    input::{Input, InputEvent, InputState},
};
use std::{
    borrow::Cow,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Clone)]
pub struct Proxy {
    dirty: Arc<AtomicBool>,
    sender: mpsc::Sender<Event>,
}
impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        if matches!(event, Event::Wakeup | Event::MouseCursorDirty) {
            self.dirty.store(true, Ordering::Relaxed);
        } else {
            let _ = self.sender.send(event);
        }
    }
}
#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

const TERMINAL_INSET: f32 = 3.;

actions!(vyber_terminal, [TerminalTab, TerminalBackTab]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", TerminalTab, Some("Terminal && !Input")),
        KeyBinding::new("shift-tab", TerminalBackTab, Some("Terminal && !Input")),
    ]);
}

pub struct Terminal {
    pub id: usize,
    pub root: PathBuf,
    pub title: String,
    pub exited: bool,
    pub bell: bool,
    pub notification: Option<String>,
    pub open_path: Option<(PathBuf, usize)>,
    font_family: String,
    pub focus: FocusHandle,
    pub term: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    events: mpsc::Receiver<Event>,
    dirty: Arc<AtomicBool>,
    bounds: Bounds<Pixels>,
    cols: usize,
    rows: usize,
    cell_width: f32,
    cell_height: f32,
    font_size: f32,
    selecting: bool,
    last_mouse: Option<(usize, usize)>,
    last_focus: bool,
    selection_start: Option<TermPoint>,
    selection_dragged: bool,
    selection_clicks: usize,
    marked: String,
    search_input: Option<Entity<InputState>>,
    search_visible: bool,
    search_origin: Option<TermPoint>,
    search_subscription: Option<Subscription>,
}
pub struct Backend {
    term: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    events: mpsc::Receiver<Event>,
    dirty: Arc<AtomicBool>,
    program: String,
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}
impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Terminal {
    pub fn prepare(id: usize, root: &Path, shell: Option<&str>) -> anyhow::Result<Backend> {
        let (sender, events) = mpsc::channel();
        let dirty = Arc::new(AtomicBool::new(true));
        let proxy = Proxy {
            dirty: dirty.clone(),
            sender,
        };
        let dimensions = Size {
            cols: 100,
            rows: 30,
        };
        let config = Config {
            scrolling_history: crate::config::Config::load().scrollback,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &dimensions,
            proxy.clone(),
        )));
        let program = shell
            .map(str::to_owned)
            .unwrap_or_else(crate::platform::default_shell);
        let is_windows_bash = cfg!(windows)
            && Path::new(&program)
                .file_stem()
                .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case("bash"));
        let args = if program.to_lowercase().contains("powershell")
            || program.to_lowercase().contains("pwsh")
        {
            vec!["-NoLogo".into(),"-NoExit".into(),"-Command".into(),"if (-not (Get-Module PSReadLine)) { Import-Module PSReadLine -ErrorAction SilentlyContinue }; $global:VyberOriginalPrompt = $function:prompt; function global:prompt { $Host.UI.RawUI.WindowTitle = '__VYBER_CWD__' + (Get-Location).Path; & $global:VyberOriginalPrompt }".into()]
        } else if is_windows_bash {
            vec!["--login".into(), "-i".into()]
        } else if ["zsh", "bash", "fish", "sh"].iter().any(|name| {
            Path::new(&program)
                .file_stem()
                .is_some_and(|stem| stem == *name)
        }) {
            vec!["-l".into()]
        } else {
            vec![]
        };
        #[cfg(test)]
        let args = if program.to_lowercase().contains("codex") {
            vec!["--no-daemon".into()]
        } else {
            args
        };
        let mut env = std::collections::HashMap::new();
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        env.insert("TERM_PROGRAM".into(), "Vyber".into());
        env.insert("VYBER_PANE_ID".into(), id.to_string());
        if is_windows_bash {
            // Preserve the requested directory and report cd changes without modifying profiles.
            env.insert("CHERE_INVOKING".into(), "1".into());
            let inherited = std::env::var("PROMPT_COMMAND").unwrap_or_default();
            env.insert(
                "PROMPT_COMMAND".into(),
                format!(
                    "{inherited}\n__vyber_status=$?; printf '\\033]0;__VYBER_CWD__%s\\007' \"$(pwd -W)\"; (exit \"$__vyber_status\")"
                ),
            );
        }
        let options = tty::Options {
            shell: Some(tty::Shell::new(program.clone(), args)),
            working_directory: Some(root.to_owned()),
            env,
            drain_on_exit: true,
            #[cfg(windows)]
            escape_args: true,
            ..Default::default()
        };
        let pty = tty::new(
            &options,
            WindowSize {
                num_lines: 30,
                num_cols: 100,
                cell_width: 8,
                cell_height: 20,
            },
            0,
        )?;
        let pty = crate::pty::ObservedPty::new(pty, proxy.clone());
        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Backend {
            term,
            sender,
            events,
            dirty,
            program,
        })
    }
    pub fn new(id: usize, root: &Path, backend: Backend, cx: &mut Context<Self>) -> Self {
        let Backend {
            term,
            sender,
            events,
            dirty,
            program,
        } = backend;
        cx.spawn(async move |entity, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(16)).await;
                if entity.update(cx, |view, cx| view.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            id,
            root: root.to_owned(),
            title: Path::new(&program)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            exited: false,
            bell: false,
            notification: None,
            open_path: None,
            font_family: crate::config::Config::load().font_family,
            focus: cx.focus_handle(),
            term,
            sender,
            events,
            dirty,
            bounds: Bounds::default(),
            cols: 100,
            rows: 30,
            cell_width: 8.4,
            cell_height: 22.,
            font_size: crate::config::Config::load().font_size,
            selecting: false,
            last_mouse: None,
            last_focus: false,
            selection_start: None,
            selection_dragged: false,
            selection_clicks: 0,
            marked: String::new(),
            search_input: None,
            search_visible: false,
            search_origin: None,
            search_subscription: None,
        }
    }
    pub fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_input.is_none() {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder("Search scrollback (regex)"));
            self.search_subscription =
                Some(cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.find(false, cx);
                    }
                }));
            self.search_input = Some(input);
        }
        self.search_visible = !self.search_visible;
        if self.search_visible {
            if let Some(input) = &self.search_input {
                input.update(cx, |i, cx| i.focus(window, cx));
            }
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }
    fn find(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(input) = &self.search_input else {
            return;
        };
        let query = input.read(cx).value();
        if query.is_empty() {
            return;
        }
        let Ok(mut regex) = alacritty_terminal::term::search::RegexSearch::new(&query) else {
            return;
        };
        let mut term = self.term.lock();
        let origin = if next {
            self.search_origin
                .map(|p| TermPoint::new(p.line, Column(p.column.0.saturating_sub(1))))
                .unwrap_or(term.grid().cursor.point)
        } else {
            term.grid().cursor.point
        };
        if let Some(found) = term.search_next(
            &mut regex,
            origin,
            alacritty_terminal::index::Direction::Left,
            Side::Left,
            None,
        ) {
            let start = *found.start();
            let end = *found.end();
            let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
            selection.update(end, Side::Right);
            term.selection = Some(selection);
            term.scroll_to_point(start);
            self.search_origin = Some(start);
        }
        cx.notify();
    }
    fn link_at(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let p = self.position(point);
        let term = self.term.lock();
        if let Some(link) = term.grid()[p].hyperlink() {
            let uri = link.uri();
            if uri.starts_with("https://") || uri.starts_with("http://") {
                cx.open_url(uri);
                return true;
            }
            if let Ok(url) = url::Url::parse(uri) {
                if let Ok(path) = url.to_file_path() {
                    drop(term);
                    self.open_path = Some((path, 1));
                    return true;
                }
            }
        }
        let mut start = p.column.0;
        let mut end = start;
        while start > 0
            && !term.grid()[TermPoint::new(p.line, Column(start - 1))]
                .c
                .is_whitespace()
        {
            start -= 1;
        }
        while end < self.cols
            && !term.grid()[TermPoint::new(p.line, Column(end))]
                .c
                .is_whitespace()
        {
            end += 1;
        }
        let text = (start..end)
            .map(|c| term.grid()[TermPoint::new(p.line, Column(c))].c)
            .collect::<String>();
        drop(term);
        let text = text
            .trim_matches(|c: char| matches!(c, '\'' | '"' | '(' | ')' | '[' | ']' | ',' | ';'));
        if text.starts_with("https://") || text.starts_with("http://") {
            cx.open_url(text);
            return true;
        }
        if let Some((path, line)) = local_link(text, &self.root) {
            self.open_path = Some((path, line));
            cx.notify();
            return true;
        }
        false
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.dirty.swap(false, Ordering::Relaxed);
        for event in self.events.try_iter().take(1024) {
            changed = true;
            match event {
                Event::Title(t) => {
                    if let Some(path) = t.strip_prefix("__VYBER_CWD__") {
                        self.root = PathBuf::from(path);
                    } else if let Some(message) = t.strip_prefix("__VYBER_NOTIFY__") {
                        self.notification = Some(message.into());
                    } else {
                        self.title = if t.ends_with(".exe") {
                            Path::new(&t)
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into()
                        } else {
                            t
                        };
                    }
                }
                Event::PtyWrite(text) => {
                    let _ = self.sender.send(Msg::Input(text.into_bytes().into()));
                }
                Event::ColorRequest(i, format) => {
                    let c = self.term.lock().colors()[i]
                        .map(|c| ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32)
                        .unwrap_or_else(|| palette(i));
                    let _ = self.sender.send(Msg::Input(
                        format(alacritty_terminal::vte::ansi::Rgb {
                            r: (c >> 16) as u8,
                            g: (c >> 8) as u8,
                            b: c as u8,
                        })
                        .into_bytes()
                        .into(),
                    ));
                }
                Event::TextAreaSizeRequest(format) => {
                    let _ = self
                        .sender
                        .send(Msg::Input(format(self.window_size()).into_bytes().into()));
                }
                Event::ChildExit(_) | Event::Exit => self.exited = true,
                Event::Bell => self.bell = true,
                _ => {}
            }
        }
        if changed {
            cx.notify();
        }
    }
    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_cols: self.cols as u16,
            num_lines: self.rows as u16,
            cell_width: self.cell_width as u16,
            cell_height: self.cell_height as u16,
        }
    }
    pub fn font_size(&self) -> f32 {
        self.font_size
    }
    /// Cell metrics and the PTY size follow on the next paint.
    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        if (self.font_size - size).abs() > f32::EPSILON {
            self.font_size = size;
            cx.notify();
        }
    }
    pub fn send(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.sender.send(Msg::Input(Cow::Owned(bytes.into())));
    }
    pub fn paste(&mut self, text: &str) {
        let bracket = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace('\x1b', "");
        self.send(
            if bracket {
                format!("\x1b[200~{text}\x1b[201~")
            } else {
                text.replace("\r\n", "\r").replace('\n', "\r")
            }
            .into_bytes(),
        );
    }
    fn resize(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
        let cols = ((f32::from(bounds.size.width) - 2. * TERMINAL_INSET) / self.cell_width)
            .floor()
            .max(2.) as usize;
        let rows = ((f32::from(bounds.size.height) - 2. * TERMINAL_INSET) / self.cell_height)
            .floor()
            .max(1.) as usize;
        if (cols, rows) != (self.cols, self.rows) {
            self.cols = cols;
            self.rows = rows;
            self.term.lock().resize(Size { cols, rows });
            let _ = self.sender.send(Msg::Resize(self.window_size()));
        }
    }
    fn position(&self, p: Point<Pixels>) -> TermPoint {
        let offset = self.term.lock().grid().display_offset();
        TermPoint::new(
            Line(
                (((f32::from(p.y - self.bounds.origin.y) - TERMINAL_INSET) / self.cell_height)
                    .floor() as i32)
                    .clamp(0, self.rows as i32 - 1)
                    - offset as i32,
            ),
            Column(
                (((f32::from(p.x - self.bounds.origin.x) - TERMINAL_INSET) / self.cell_width)
                    .floor()
                    .max(0.) as usize)
                    .min(self.cols - 1),
            ),
        )
    }
    fn tab(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            cx.propagate();
            return;
        }
        let mode = *self.term.lock().mode();
        if let Some(bytes) = key_bytes("tab", None, false, false, backwards, mode) {
            self.term.lock().scroll_display(Scroll::Bottom);
            self.send(bytes);
            cx.notify();
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.key == "escape" && cx.stop_active_drag(window) {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let application = key.modifiers.platform || (key.modifiers.control && key.modifiers.shift);
        if application && key.key == "c" {
            if let Some(text) = self.term.lock().selection_to_string() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            cx.stop_propagation();
            return;
        }
        if application && key.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                self.paste(&text);
            }
            cx.stop_propagation();
            return;
        }
        let mode = *self.term.lock().mode();
        if let Some(bytes) = key_bytes(
            &key.key,
            key.key_char.as_deref(),
            key.modifiers.control,
            key.modifiers.alt,
            key.modifiers.shift,
            mode,
        ) {
            {
                let mut term = self.term.lock();
                term.scroll_display(Scroll::Bottom);
                term.selection = None;
            }
            self.send(bytes);
            cx.stop_propagation();
            cx.notify();
        }
        let _ = window;
    }
    fn mouse(&mut self, point: Point<Pixels>, button: u8, release: bool) -> bool {
        let term = self.term.lock();
        let mode = *term.mode();
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return false;
        }
        let p = self.position_without_lock(point);
        let x = p.0 + 1;
        let y = p.1 + 1;
        if mode.contains(TermMode::SGR_MOUSE) {
            self.send(
                format!("\x1b[<{button};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes(),
            );
        } else if x < 224 && y < 224 {
            self.send(vec![
                27,
                b'[',
                b'M',
                32 + if release { 3 } else { button },
                32 + x as u8,
                32 + y as u8,
            ]);
        }
        true
    }
    fn position_without_lock(&self, p: Point<Pixels>) -> (usize, usize) {
        (
            ((f32::from(p.x - self.bounds.origin.x) - TERMINAL_INSET) / self.cell_width).max(0.)
                as usize,
            ((f32::from(p.y - self.bounds.origin.y) - TERMINAL_INSET) / self.cell_height).max(0.)
                as usize,
        )
    }
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let focused = self.focus.is_focused(window) && window.is_window_active();
        if focused != self.last_focus {
            self.last_focus = focused;
            if self.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                self.send(if focused {
                    b"\x1b[I".to_vec()
                } else {
                    b"\x1b[O".to_vec()
                });
            }
        }
        let mut font = font(self.font_family.clone());
        font.fallbacks = Some(FontFallbacks::from_fonts(if cfg!(windows) {
            vec!["Segoe UI Symbol".into(), "Segoe UI Emoji".into()]
        } else {
            vec!["Apple Symbols".into(), "Apple Color Emoji".into()]
        }));
        let font_id = window.text_system().resolve_font(&font);
        if let Ok(advance) = window
            .text_system()
            .advance(font_id, px(self.font_size), 'M')
        {
            self.cell_width = f32::from(advance.width);
        }
        self.cell_height = (self.font_size * 1.25).ceil();
        self.resize(bounds);
        let term = self.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        for cell in content.display_iter {
            let row = cell.point.line.0 + offset;
            if row < 0 || row >= self.rows as i32 {
                continue;
            }
            let origin = point(
                bounds.origin.x + px(TERMINAL_INSET + cell.point.column.0 as f32 * self.cell_width),
                bounds.origin.y + px(TERMINAL_INSET + row as f32 * self.cell_height),
            );
            let mut fg = foreground(cell.fg, cell.flags, content.colors);
            let mut bg = color(cell.bg, content.colors);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let selected = content.selection.is_some_and(|s| s.contains(cell.point));
            if selected {
                bg = 0x363145;
            }
            if bg != 0x000000 {
                window.paint_quad(fill(
                    Bounds::new(origin, size(px(self.cell_width), px(self.cell_height))),
                    rgb(bg),
                ));
            }
            if cell.c == ' '
                || cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
            {
                continue;
            }
            let mut text = cell.c.to_string();
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra);
            }
            let mut run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: rgb(fg).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            if cell.flags.contains(Flags::BOLD) {
                run.font.weight = FontWeight::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                run.font.style = FontStyle::Italic;
            }
            let shaped =
                window
                    .text_system()
                    .shape_line(text.into(), px(self.font_size), &[run], None);
            let _ = shaped.paint(
                origin,
                px(self.cell_height),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        let cursor = content.cursor.point;
        let row = cursor.line.0 + offset;
        if row >= 0
            && row < self.rows as i32
            && content.mode.contains(TermMode::SHOW_CURSOR)
            && self.focus.is_focused(window)
        {
            window.paint_quad(fill(
                Bounds::new(
                    point(
                        bounds.origin.x
                            + px(TERMINAL_INSET + cursor.column.0 as f32 * self.cell_width),
                        bounds.origin.y + px(TERMINAL_INSET + row as f32 * self.cell_height),
                    ),
                    size(px(2.), px(self.cell_height)),
                ),
                rgb(0xb4a5ff),
            ));
        }
    }
}
impl Render for Terminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .id(("terminal", self.id))
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .cursor_text()
            .on_action(cx.listener(|this, _: &TerminalTab, window, cx| this.tab(false, window, cx)))
            .on_action(
                cx.listener(|this, _: &TerminalBackTab, window, cx| this.tab(true, window, cx)),
            )
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    window.focus(&this.focus, cx);
                    if (e.modifiers.control || e.modifiers.platform) && this.link_at(e.position, cx)
                    {
                        cx.stop_propagation();
                        return;
                    }
                    if !e.modifiers.shift && this.mouse(e.position, 0, false) {
                        return;
                    }
                    let p = this.position(e.position);
                    this.term.lock().selection = Some(Selection::new(
                        if e.click_count == 2 {
                            SelectionType::Semantic
                        } else if e.click_count >= 3 {
                            SelectionType::Lines
                        } else {
                            SelectionType::Simple
                        },
                        p,
                        Side::Left,
                    ));
                    this.selecting = true;
                    this.selection_start = Some(p);
                    this.selection_dragged = false;
                    this.selection_clicks = e.click_count;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if cx.has_active_drag() {
                    return;
                }
                let cell = this.position_without_lock(e.position);
                let mode = *this.term.lock().mode();
                if !e.modifiers.shift
                    && this.last_mouse != Some(cell)
                    && (mode.contains(TermMode::MOUSE_MOTION)
                        || (mode.contains(TermMode::MOUSE_DRAG) && e.pressed_button.is_some()))
                {
                    let button = match e.pressed_button {
                        Some(MouseButton::Left) => 0,
                        Some(MouseButton::Middle) => 1,
                        Some(MouseButton::Right) => 2,
                        _ => 3,
                    };
                    this.mouse(e.position, button + 32, false);
                }
                this.last_mouse = Some(cell);
                if this.selecting {
                    let p = this.position(e.position);
                    this.selection_dragged |= this.selection_start != Some(p);
                    if let Some(s) = this.term.lock().selection.as_mut() {
                        s.update(p, Side::Right);
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, _, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    this.selecting = false;
                    if !this.selection_dragged && this.selection_clicks == 1 {
                        this.term.lock().selection = None;
                    }
                    if !e.modifiers.shift {
                        this.mouse(e.position, 0, true);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if !e.modifiers.shift && this.mouse(e.position, 2, false) {
                        return;
                    }
                    if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                        this.paste(&text);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, e: &MouseUpEvent, _, _| {
                    if !e.modifiers.shift {
                        this.mouse(e.position, 2, true);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseDownEvent, _, _| {
                    if !e.modifiers.shift {
                        this.mouse(e.position, 1, false);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseUpEvent, _, _| {
                    if !e.modifiers.shift {
                        this.mouse(e.position, 1, true);
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                let y = f32::from(e.delta.pixel_delta(px(this.cell_height)).y);
                let lines = (y / this.cell_height).round() as i32;
                let mode = *this.term.lock().mode();
                if !e.modifiers.shift && mode.intersects(TermMode::MOUSE_MODE) {
                    for _ in 0..lines.abs().clamp(1, 10) {
                        this.mouse(e.position, if y > 0. { 64 } else { 65 }, false);
                    }
                } else if !e.modifiers.shift
                    && mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
                {
                    for _ in 0..lines.abs().clamp(1, 10) {
                        this.send(if y > 0. {
                            b"\x1bOA".to_vec()
                        } else {
                            b"\x1bOB".to_vec()
                        });
                    }
                } else {
                    this.term.lock().scroll_display(Scroll::Delta(lines));
                }
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let handle = entity.read(cx).focus.clone();
                        window.handle_input(
                            &handle,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        entity.update(cx, |this, cx| this.paint(bounds, window, cx));
                    },
                )
                .size_full(),
            )
            .when(self.search_visible, |el| {
                el.child(
                    div()
                        .absolute()
                        .occlude()
                        .top_2()
                        .right_2()
                        .w(px(350.))
                        .p_2()
                        .rounded_lg()
                        .bg(rgb(0x24212c))
                        .flex()
                        .gap_2()
                        .when_some(self.search_input.clone(), |el, input| {
                            el.child(Input::new(&input).small())
                        })
                        .child(
                            crate::theme::chip("find-next", "↓")
                                .on_click(cx.listener(|this, _, _, cx| this.find(true, cx))),
                        )
                        .child(
                            crate::theme::chip("close-search", "×")
                                .on_click(cx.listener(|this, _, w, cx| this.toggle_search(w, cx))),
                        ),
                )
            })
    }
}

impl EntityInputHandler for Terminal {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.marked.clone())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        if self.marked.is_empty() {
            None
        } else {
            Some(0..self.marked.encode_utf16().count())
        }
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked.clear();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        {
            let mut term = self.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.send(text.as_bytes().to_vec());
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.into();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let p = self.term.lock().grid().cursor.point;
        Some(Bounds::new(
            point(
                self.bounds.origin.x + px(TERMINAL_INSET + p.column.0 as f32 * self.cell_width),
                self.bounds.origin.y + px(TERMINAL_INSET + p.line.0 as f32 * self.cell_height),
            ),
            size(px(self.cell_width), px(self.cell_height)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

pub fn key_bytes(
    key: &str,
    text: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let modified = modifier > 1;
    if mode.intersects(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC)
        && !(ctrl && alt)
    {
        let code = match key {
            "enter" => Some(13),
            "escape" => Some(27),
            "tab" => Some(9),
            "backspace" => Some(127),
            "space" => Some(32),
            _ => {
                if key.chars().count() == 1 {
                    key.chars().next().map(|c| c as u32)
                } else {
                    None
                }
            }
        };
        if let Some(code) = code {
            if modified || key == "escape" || mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
                return Some(format!("\x1b[{code};{modifier}u").into_bytes());
            }
        }
    }

    let value = match key {
        "enter" => {
            if shift {
                "\x1b[13;2u".into()
            } else {
                "\r".into()
            }
        }
        "escape" => "\x1b".into(),
        "backspace" => {
            if ctrl {
                "\x08".into()
            } else if alt {
                "\x1b\x7f".into()
            } else {
                "\x7f".into()
            }
        }
        "tab" => {
            if shift {
                "\x1b[Z".into()
            } else {
                "\t".into()
            }
        }
        "up" | "down" | "right" | "left" | "home" | "end" => {
            let code = match key {
                "up" => 'A',
                "down" => 'B',
                "right" => 'C',
                "left" => 'D',
                "home" => 'H',
                _ => 'F',
            };
            if modified {
                format!("\x1b[1;{modifier}{code}")
            } else if mode.contains(TermMode::APP_CURSOR) {
                format!("\x1bO{code}")
            } else {
                format!("\x1b[{code}")
            }
        }
        "insert" | "delete" | "pageup" | "pagedown" => {
            let code = match key {
                "insert" => 2,
                "delete" => 3,
                "pageup" => 5,
                _ => 6,
            };
            if modified {
                format!("\x1b[{code};{modifier}~")
            } else {
                format!("\x1b[{code}~")
            }
        }
        "f1" => "\x1bOP".into(),
        "f2" => "\x1bOQ".into(),
        "f3" => "\x1bOR".into(),
        "f4" => "\x1bOS".into(),
        "f5" => "\x1b[15~".into(),
        "f6" => "\x1b[17~".into(),
        "f7" => "\x1b[18~".into(),
        "f8" => "\x1b[19~".into(),
        "f9" => "\x1b[20~".into(),
        "f10" => "\x1b[21~".into(),
        "f11" => "\x1b[23~".into(),
        "f12" => "\x1b[24~".into(),
        _ => {
            if ctrl && !alt {
                let c = key.chars().next()?;
                if key.chars().count() == 1 && c.is_ascii() {
                    return Some(vec![(c.to_ascii_uppercase() as u8) & 31]);
                }
                if key == "space" {
                    return Some(vec![0]);
                }
            }
            if alt && !ctrl {
                if let Some(text) = text {
                    return Some(format!("\x1b{text}").into_bytes());
                }
            }
            return None;
        }
    };
    Some(value.into_bytes())
}
pub fn local_link(text: &str, root: &Path) -> Option<(PathBuf, usize)> {
    let mut path = text;
    let mut line = 1;
    if let Some((p, n)) = path.rsplit_once(':') {
        if let Ok(value) = n.parse::<usize>() {
            path = p;
            line = value;
            if let Some((p, n)) = path.rsplit_once(':') {
                if let Ok(value) = n.parse::<usize>() {
                    path = p;
                    line = value;
                }
            }
        }
    }
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if path.is_file() {
        Some((path, line))
    } else {
        None
    }
}

fn color(c: Color, overrides: &alacritty_terminal::term::color::Colors) -> u32 {
    let index = match c {
        Color::Spec(c) => return ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32,
        Color::Indexed(i) => i as usize,
        Color::Named(n) => n as usize,
    };
    overrides[index]
        .map(|c| ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32)
        .unwrap_or_else(|| palette(index))
}
fn dim_color(c: u32) -> u32 {
    let scale = |v: u32| v * 3 / 4;
    (scale((c >> 16) & 255) << 16) | (scale((c >> 8) & 255) << 8) | scale(c & 255)
}
fn foreground(c: Color, flags: Flags, overrides: &alacritty_terminal::term::color::Colors) -> u32 {
    let c = if flags.contains(Flags::BOLD) && !flags.contains(Flags::DIM) {
        match c {
            Color::Named(n) => Color::Named(n.to_bright()),
            Color::Indexed(i @ 0..=7) => Color::Indexed(i + 8),
            _ => c,
        }
    } else {
        c
    };
    let resolved = color(c, overrides);
    if flags.contains(Flags::DIM) {
        dim_color(resolved)
    } else {
        resolved
    }
}
fn palette(index: usize) -> u32 {
    const COLORS: [u32; 16] = [
        0x000000, 0xff4d5a, 0x23e18e, 0xffd866, 0x3b8eea, 0xd670d6, 0x29d9e3, 0xf2f2f2, 0x7b8190,
        0xff6b76, 0x5aff9a, 0xfff176, 0x67a9ff, 0xff8cff, 0x65ffff, 0xffffff,
    ];
    match index {
        256 => return 0xf2f2f2,
        257 => return 0x000000,
        258 => return 0xb77bff,
        259..=266 => return dim_color(COLORS[index - 259]),
        267 => return 0xffffff,
        268 => return dim_color(0xf2f2f2),
        _ => {}
    }
    if index < 16 {
        COLORS[index]
    } else if index < 232 {
        let n = index - 16;
        let c = |v| if v == 0 { 0 } else { 55 + 40 * v };
        ((c(n / 36) as u32) << 16) | ((c(n / 6 % 6) as u32) << 8) | c(n % 6) as u32
    } else if index < 256 {
        let v = (8 + 10 * (index - 232)) as u32;
        (v << 16) | (v << 8) | v
    } else {
        0xf2f2f2
    }
}

#[cfg(test)]
mod tests {
    use super::key_bytes;
    use alacritty_terminal::term::TermMode;
    #[test]
    fn terminal_colors_preserve_rgb_cube_and_application_overrides() {
        use super::{Color, Flags, color, foreground, palette};
        use alacritty_terminal::{
            term::color::Colors,
            vte::ansi::{NamedColor, Rgb},
        };
        let mut colors = Colors::default();
        let explicit = Color::Spec(Rgb {
            r: 12,
            g: 34,
            b: 56,
        });
        assert_eq!(color(explicit, &colors), 0x0c2238);
        assert_eq!(foreground(explicit, Flags::BOLD, &colors), 0x0c2238);
        assert_eq!(palette(16), 0x000000);
        assert_eq!(palette(196), 0xff0000);
        assert_eq!(palette(231), 0xffffff);
        assert_eq!(palette(232), 0x080808);
        assert_eq!(palette(255), 0xeeeeee);
        colors[NamedColor::Green] = Some(Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(color(Color::Named(NamedColor::Green), &colors), 0x010203);
        assert_eq!(
            foreground(Color::Named(NamedColor::Red), Flags::BOLD, &colors),
            palette(9)
        );
        assert_ne!(
            palette(NamedColor::DimGreen as usize),
            palette(NamedColor::Foreground as usize)
        );
    }
    #[test]
    #[ignore = "starts installed CLIs; no request is submitted"]
    #[cfg(windows)]
    fn installed_cli_input_smoke() -> anyhow::Result<()> {
        use super::{Event, Msg, Terminal};
        let program = std::env::var("VYBER_CLI_SMOKE")?;
        let backend = Terminal::prepare(9001, &std::env::current_dir()?, Some(&program))?;
        let read_for = |seconds: f32| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f32(seconds);
            while std::time::Instant::now() < deadline {
                for event in backend.events.try_iter() {
                    match event {
                        Event::PtyWrite(text) => {
                            let _ = backend.sender.send(Msg::Input(text.into_bytes().into()));
                        }
                        Event::ColorRequest(i, f) => {
                            let c = super::palette(i);
                            let text = f(alacritty_terminal::vte::ansi::Rgb {
                                r: (c >> 16) as u8,
                                g: (c >> 8) as u8,
                                b: c as u8,
                            });
                            let _ = backend.sender.send(Msg::Input(text.into_bytes().into()));
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        read_for(8.);
        let initial = backend
            .term
            .lock()
            .renderable_content()
            .display_iter
            .map(|c| c.c)
            .collect::<String>();
        if initial.contains("Update available") && initial.contains("esc skip") {
            backend.sender.send(Msg::Input(vec![27].into()))?;
            read_for(3.);
        }
        let screen = backend
            .term
            .lock()
            .renderable_content()
            .display_iter
            .map(|c| c.c)
            .collect::<String>();
        if screen.contains("trust this folder") || screen.contains("Agent command center") {
            let _ = backend.sender.send(Msg::Shutdown);
            anyhow::bail!(
                "CLI is at an onboarding/session-management screen; input probe was not sent"
            );
        }
        let mode = *backend.term.lock().mode();
        backend
            .sender
            .send(Msg::Input(b"vyber_input_first".to_vec().into()))?;
        read_for(0.3);
        backend.sender.send(Msg::Input(
            key_bytes("enter", None, false, false, true, mode)
                .unwrap()
                .into(),
        ))?;
        backend
            .sender
            .send(Msg::Input(b"vyber_input_second".to_vec().into()))?;
        read_for(0.7);
        let mut rows = std::collections::BTreeMap::<i32, String>::new();
        for cell in backend.term.lock().renderable_content().display_iter {
            rows.entry(cell.point.line.0).or_default().push(cell.c);
        }
        let first = rows
            .iter()
            .find(|(_, s)| s.contains("vyber_input_first"))
            .map(|(r, _)| *r);
        let second = rows
            .iter()
            .find(|(_, s)| s.contains("vyber_input_second"))
            .map(|(r, _)| *r);
        let _ = backend.sender.send(Msg::Input(vec![3, 3].into()));
        read_for(0.2);
        let _ = backend.sender.send(Msg::Shutdown);
        println!(
            "{program}: keyboard flags={mode:?}, first line={first:?}, second line={second:?}; no Enter submission sent"
        );
        assert!(
            first.is_some() && second.is_some() && first != second,
            "CLI did not show two separate input rows; may be at an onboarding/trust screen"
        );
        Ok(())
    }
    #[test]
    #[cfg(windows)]
    fn real_conpty_roundtrip_and_shutdown() -> anyhow::Result<()> {
        use super::{Event, Msg, Terminal};
        let root = tempfile::tempdir()?;
        let backend = Terminal::prepare(9000, root.path(), Some("powershell.exe"))?;
        backend.sender.send(Msg::Input(
            b"Write-Output ('VYBER_' + 'PROBE_OK'); exit\r"
                .to_vec()
                .into(),
        ))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut exited = false;
        let mut output = String::new();
        while std::time::Instant::now() < deadline {
            for event in backend.events.try_iter() {
                match event {
                    Event::ChildExit(_) | Event::Exit => exited = true,
                    Event::PtyWrite(text) => {
                        let _ = backend.sender.send(Msg::Input(text.into_bytes().into()));
                    }
                    _ => {}
                }
            }
            output = backend
                .term
                .lock()
                .renderable_content()
                .display_iter
                .map(|c| c.c)
                .collect();
            if exited && output.contains("VYBER_PROBE_OK") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = backend.sender.send(Msg::Shutdown);
        assert!(exited, "ConPTY child did not exit");
        assert!(
            output.contains("VYBER_PROBE_OK"),
            "ConPTY output not parsed: {output}"
        );
        Ok(())
    }
    #[test]
    #[cfg(windows)]
    fn real_powershell_tab_completes_a_filename() -> anyhow::Result<()> {
        use super::{Event, Msg, Terminal};
        let dir = tempfile::tempdir()?;
        std::fs::write(
            dir.path().join("vyber_completion_probe.txt"),
            "VYBER_COMPLETION_SUCCESS",
        )?;
        let backend = Terminal::prepare(9002, dir.path(), Some("powershell.exe"))?;
        let read_until = |needle: &str, timeout: u64| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            let mut output = String::new();
            while std::time::Instant::now() < deadline {
                for event in backend.events.try_iter() {
                    if let Event::PtyWrite(text) = event {
                        let _ = backend.sender.send(Msg::Input(text.into_bytes().into()));
                    }
                }
                output = backend
                    .term
                    .lock()
                    .renderable_content()
                    .display_iter
                    .map(|c| c.c)
                    .collect();
                if output.contains(needle) {
                    return (true, output);
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            (false, output)
        };
        let ready = read_until(">", 10).0;
        if !ready {
            let _ = backend.sender.send(Msg::Shutdown);
            anyhow::bail!("PowerShell prompt not ready");
        }
        backend.sender.send(Msg::Input(
            b"Get-Content .\\vyber_completion_pro".to_vec().into(),
        ))?;
        let mode = *backend.term.lock().mode();
        backend.sender.send(Msg::Input(
            key_bytes("tab", None, false, false, false, mode)
                .unwrap()
                .into(),
        ))?;
        let (completed, _) = read_until("vyber_completion_probe.txt", 5);
        backend.sender.send(Msg::Input(vec![13].into()))?;
        let (success, output) = read_until("VYBER_COMPLETION_SUCCESS", 5);
        let _ = backend.sender.send(Msg::Shutdown);
        assert!(completed, "Tab did not expand the filename");
        assert!(
            success,
            "Completed filename did not execute correctly: {output}"
        );
        Ok(())
    }
    #[test]
    #[cfg(windows)]
    fn real_git_bash_default_cwd_tab_and_exit() -> anyhow::Result<()> {
        use super::{Event, Msg, Terminal};
        let dir = tempfile::Builder::new()
            .prefix("vyber bash Türkçe ")
            .tempdir()?;
        std::fs::create_dir(dir.path().join("child"))?;
        std::fs::write(
            dir.path().join("vyber_bash_probe.txt"),
            "VYBER_BASH_COMPLETE",
        )?;
        let backend = Terminal::prepare(9003, dir.path(), None)?;
        assert!(
            backend.program.ends_with("bash.exe"),
            "Git Bash was not selected"
        );
        let mut cwd = None;
        let mut exited = false;
        let mut read_until =
            |needle: &str, expected_cwd: Option<&std::path::Path>, want_exit: bool| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let mut output = String::new();
                while std::time::Instant::now() < deadline {
                    for event in backend.events.try_iter() {
                        match event {
                            Event::Title(title) => {
                                if let Some(path) = title.strip_prefix("__VYBER_CWD__") {
                                    cwd = Some(std::path::PathBuf::from(path));
                                }
                            }
                            Event::ChildExit(_) | Event::Exit => exited = true,
                            Event::PtyWrite(text) => {
                                let _ = backend.sender.send(Msg::Input(text.into_bytes().into()));
                            }
                            _ => {}
                        }
                    }
                    output = backend
                        .term
                        .lock()
                        .renderable_content()
                        .display_iter
                        .map(|c| c.c)
                        .collect();
                    if output.contains(needle)
                        && expected_cwd.is_none_or(|path| cwd.as_deref() == Some(path))
                        && (!want_exit || exited)
                    {
                        return (true, output);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                (false, output)
            };
        let ready = read_until("$", Some(dir.path()), false);
        if !ready.0 {
            let _ = backend.sender.send(Msg::Shutdown);
            anyhow::bail!(
                "Git Bash did not preserve/report the initial directory: {}",
                ready.1
            );
        }
        backend
            .sender
            .send(Msg::Input(b"cat vyber_bash_pro".to_vec().into()))?;
        let mode = *backend.term.lock().mode();
        backend.sender.send(Msg::Input(
            key_bytes("tab", None, false, false, false, mode)
                .unwrap()
                .into(),
        ))?;
        let completed = read_until("vyber_bash_probe.txt", None, false).0;
        backend.sender.send(Msg::Input(vec![13].into()))?;
        let success = read_until("VYBER_BASH_COMPLETE", None, false).0;
        backend
            .sender
            .send(Msg::Input(b"cd child\r".to_vec().into()))?;
        let moved = read_until("", Some(&dir.path().join("child")), false).0;
        backend.sender.send(Msg::Input(b"exit\r".to_vec().into()))?;
        let exited = read_until("", None, true).0;
        let _ = backend.sender.send(Msg::Shutdown);
        assert!(completed && success, "Git Bash Tab completion failed");
        assert!(moved, "Git Bash did not report the changed directory");
        assert!(exited, "Git Bash did not emit a child-exit event");
        Ok(())
    }
    #[test]
    fn parser_supports_tui_modes_colors_and_hyperlinks() {
        use super::{Proxy, Size};
        use std::sync::{Arc, atomic::AtomicBool, mpsc};
        let (tx, _) = mpsc::channel();
        let proxy = Proxy {
            dirty: Arc::new(AtomicBool::new(false)),
            sender: tx,
        };
        let mut term =
            alacritty_terminal::Term::new(Default::default(), &Size { cols: 80, rows: 24 }, proxy);
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut term,b"\x1b[?1049h\x1b[?2004h\x1b[?1006h\x1b[38;2;1;2;3mA\x1b]8;;https://example.com\x07B\x1b]8;;\x07");
        assert!(
            term.mode()
                .contains(TermMode::ALT_SCREEN | TermMode::BRACKETED_PASTE | TermMode::SGR_MOUSE)
        );
        let content = term.renderable_content();
        let cells = content.display_iter.take(2).collect::<Vec<_>>();
        assert_eq!(cells[0].c, 'A');
        assert_eq!(cells[1].hyperlink().unwrap().uri(), "https://example.com");
        parser.advance(&mut term, b"\x1b[?1049l");
        assert!(!term.mode().contains(TermMode::ALT_SCREEN));
    }
    #[test]
    fn key_sequences() {
        assert_eq!(
            key_bytes("tab", None, false, false, false, TermMode::empty()),
            Some(vec![9])
        );
        assert_eq!(
            key_bytes("tab", None, false, false, true, TermMode::empty()),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            key_bytes(
                "tab",
                None,
                false,
                false,
                false,
                TermMode::REPORT_ALL_KEYS_AS_ESC
            ),
            Some(b"\x1b[9;1u".to_vec())
        );
        assert_eq!(
            key_bytes("c", None, true, false, false, TermMode::empty()),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes("enter", None, false, false, true, TermMode::empty()),
            Some(b"\x1b[13;2u".to_vec())
        );
        assert_eq!(
            key_bytes("up", None, false, false, false, TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes("q", Some("@"), true, true, false, TermMode::empty()),
            None
        );
    }
}
