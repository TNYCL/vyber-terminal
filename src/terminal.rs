use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as TermPoint, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags},
    tty,
    vte::ansi::{Color, NamedColor},
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
    time::{Duration, Instant},
};

mod badge;
mod links;
pub use badge::badge_spot;
use badge::{BadgeAnchorMemory, badge_visible, fit_badge};
pub use links::FileLink;

#[derive(Clone)]
pub struct Proxy {
    dirty: Arc<AtomicBool>,
    sender: mpsc::Sender<Event>,
}

#[cfg(test)]
impl Proxy {
    pub fn channel() -> (Self, mpsc::Receiver<Event>) {
        let (sender, events) = mpsc::channel();
        let proxy = Self {
            dirty: Arc::new(AtomicBool::new(false)),
            sender,
        };
        (proxy, events)
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
/// Bytes of typed text kept to match an agent's prompt to its terminal.
const TYPED_LIMIT: usize = 4096;

/// Summary of an agent turn that ran in this terminal, drawn as a badge
/// above the agent's input box. Vyber only draws it: the program in the
/// terminal never sees it, and the terminal size does not change.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnBadge {
    pub id: String,
    pub files: usize,
    pub additions: usize,
    pub deletions: usize,
    pub active: bool,
    pub warning: Option<String>,
}

/// A cell to paint, copied from the terminal under its lock.
struct PaintCell {
    column: usize,
    row: usize,
    /// The character, unless the cell only has a background.
    c: Option<char>,
    extra: Option<Vec<char>>,
    fg: u32,
    bg: u32,
    bold: bool,
    italic: bool,
}

pub enum TerminalEvent {
    /// The turn badge was clicked.
    OpenTurn(String),
}
impl EventEmitter<TerminalEvent> for Terminal {}

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
    pub process: crate::processes::TerminalProcess,
    pub bell: bool,
    pub notification: Option<String>,
    pub open_path: Option<FileLink>,
    pub badge: Option<TurnBadge>,
    badge_dismissed: Option<String>,
    badge_anchor: BadgeAnchorMemory,
    /// Text typed since the last Enter, and the line and time of that Enter.
    typed: String,
    pub last_line: String,
    pub last_submit: Option<Instant>,
    /// Process id of the shell, which the programs started in it run under.
    pub shell: Option<u32>,
    font_family: String,
    pub focus: FocusHandle,
    pub term: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    events: mpsc::Receiver<Event>,
    dirty: Arc<AtomicBool>,
    /// Set while ConPTY takes keys as win32-input-mode events.
    win32_input: Arc<AtomicBool>,
    /// Cells copied for the next paint, kept to reuse the allocation.
    cells: Vec<PaintCell>,
    bounds: Bounds<Pixels>,
    cols: usize,
    rows: usize,
    cell_width: f32,
    cell_height: f32,
    font_size: f32,
    /// While set, the grid follows a size change at most this often; see
    /// [`Terminal::set_resize_interval`].
    resize_interval: Option<Duration>,
    resized_at: Instant,
    selecting: bool,
    pointer_inside: bool,
    link_candidates: Vec<links::Link>,
    hovered_link: Option<links::Link>,
    link_generation: u64,
    link_click_consumed: bool,
    link_notice: Option<(String, Instant)>,
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
    #[cfg(unix)]
    _shell_integration: Option<tempfile::TempDir>,
}
pub struct Backend {
    term: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    events: mpsc::Receiver<Event>,
    dirty: Arc<AtomicBool>,
    win32_input: Arc<AtomicBool>,
    program: String,
    shell: Option<u32>,
    process: crate::processes::TerminalProcess,
    #[cfg(unix)]
    shell_integration: Option<tempfile::TempDir>,
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
            kitty_keyboard: true,
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
        let is_bash = Path::new(&program)
            .file_stem()
            .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case("bash"));
        let is_windows_bash = cfg!(windows) && is_bash;
        let args = if program.to_lowercase().contains("powershell")
            || program.to_lowercase().contains("pwsh")
        {
            // The console reports only a changed title, so the prefix
            // alternates and every prompt reaches Vyber (see `pty::prompt_cwd`).
            vec!["-NoLogo".into(),"-NoExit".into(),"-Command".into(),"if (-not (Get-Module PSReadLine)) { Import-Module PSReadLine -ErrorAction SilentlyContinue }; $global:VyberOriginalPrompt = $function:prompt; function global:prompt { $global:VyberPromptMark = -not $global:VyberPromptMark; $Host.UI.RawUI.WindowTitle = $(if ($global:VyberPromptMark) { '__VYBER_CWD__' } else { '__VYBER_CWD2__' }) + (Get-Location).Path; & $global:VyberOriginalPrompt }".into()]
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
        } else if program == "/bin/sh" {
            // Başsız test kabuğu kullanıcı profillerini çalıştırmaz.
            vec![]
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
        #[cfg(unix)]
        if is_bash {
            let inherited = std::env::var("PROMPT_COMMAND").unwrap_or_default();
            env.insert(
                "PROMPT_COMMAND".into(),
                format!("{inherited}\n__vyber_status=$?; printf '\\033]0;__VYBER_CWD__%s\\007' \"$PWD\"; (exit \"$__vyber_status\")"),
            );
        }
        #[cfg(unix)]
        let shell_integration = if Path::new(&program)
            .file_stem()
            .is_some_and(|name| name == "zsh")
        {
            Some(crate::shell::zsh_integration(
                &mut env,
                std::env::var_os("ZDOTDIR"),
            )?)
        } else {
            None
        };
        // Windows passes "ignore Ctrl+C" from a process to its children. Vyber
        // inherits it when a parent that ignores Ctrl+C starts it, and then
        // Ctrl+C would not stop a command in PowerShell or cmd.
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(None, false);
        }
        #[allow(
            clippy::needless_update,
            reason = "Unix PTY options have additional fields."
        )]
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
        #[cfg(windows)]
        let shell = pty.child_watcher().pid().map(|pid| pid.get());
        #[cfg(not(windows))]
        let shell = Some(pty.child().id());
        let process = crate::processes::TerminalProcess {
            pid: shell,
            shell: program.clone(),
        };
        let win32_input = Arc::new(AtomicBool::new(false));
        let pty = crate::pty::ObservedPty::new(
            pty,
            crate::pty::Observer::new(proxy.clone(), win32_input.clone()),
        );
        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Backend {
            term,
            sender,
            events,
            dirty,
            win32_input,
            program,
            shell,
            process,
            #[cfg(unix)]
            shell_integration,
        })
    }
    pub fn new(id: usize, root: &Path, backend: Backend, cx: &mut Context<Self>) -> Self {
        let Backend {
            term,
            sender,
            events,
            dirty,
            win32_input,
            program,
            shell,
            process,
            #[cfg(unix)]
            shell_integration,
        } = backend;
        let config = cx.global::<crate::config::Config>();
        let (font_family, font_size) = (config.font_family.clone(), config.font_size);
        cx.spawn(async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
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
            process,
            bell: false,
            notification: None,
            open_path: None,
            badge: None,
            badge_dismissed: None,
            badge_anchor: BadgeAnchorMemory::default(),
            typed: String::new(),
            last_line: String::new(),
            last_submit: None,
            shell,
            font_family,
            focus: cx.focus_handle(),
            term,
            sender,
            events,
            dirty,
            win32_input,
            cells: Vec::new(),
            bounds: Bounds::default(),
            cols: 100,
            rows: 30,
            cell_width: 8.4,
            cell_height: 22.,
            font_size,
            resize_interval: None,
            resized_at: Instant::now(),
            selecting: false,
            pointer_inside: false,
            link_candidates: Vec::new(),
            hovered_link: None,
            link_generation: 0,
            link_click_consumed: false,
            link_notice: None,
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
            #[cfg(unix)]
            _shell_integration: shell_integration,
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
    fn links_at(&self, point: Point<Pixels>) -> Vec<links::Link> {
        let p = self.position(point);
        let term = self.term.lock();
        links::detect(&term, p, &self.root)
    }
    pub(crate) fn update_link_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let candidates = if self.pointer_inside
            && window.is_window_active()
            && (window.modifiers().control || window.modifiers().platform)
            && !self.selecting
            && !cx.has_active_drag()
            && self.bounds.contains(&window.mouse_position())
        {
            self.links_at(window.mouse_position())
        } else {
            Vec::new()
        };
        if candidates == self.link_candidates {
            return;
        }
        self.link_generation = self.link_generation.wrapping_add(1);
        let generation = self.link_generation;
        self.hovered_link = candidates.first().cloned();
        self.link_candidates = candidates.clone();
        cx.notify();
        if candidates.len() < 2 {
            return;
        }
        let task = cx
            .background_executor()
            .spawn(async move { links::resolve(&candidates) });
        cx.spawn(async move |entity, cx| {
            let resolved = task.await.ok().flatten();
            let _ = entity.update(cx, |this, cx| {
                if this.link_generation == generation && this.hovered_link != resolved {
                    this.hovered_link = resolved;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn link_at(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let candidates = self.links_at(point);
        if candidates.is_empty() {
            return false;
        }
        let task = cx
            .background_executor()
            .spawn(async move { links::resolve(&candidates) });
        cx.spawn(async move |entity, cx| {
            let resolved = task.await;
            let _ = entity.update(cx, |this, cx| {
                match resolved {
                    Ok(Some(link)) => match link.target {
                        links::Target::Url(url) => cx.open_url(&url),
                        links::Target::File(file) => this.open_path = Some(file),
                    },
                    Err(message) => this.link_notice = Some((message.into(), Instant::now())),
                    Ok(None) => {}
                }
                cx.notify();
            });
        })
        .detach();
        true
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.dirty.swap(false, Ordering::Relaxed);
        if self
            .link_notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(4))
        {
            self.link_notice = None;
            changed = true;
        }
        for event in self.events.try_iter().take(1024) {
            changed = true;
            match event {
                Event::Title(t) => {
                    if let Some(path) = crate::pty::prompt_cwd(&t) {
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
    /// Cell metrics and the PTY size follow on the next paint.
    pub fn set_font(&mut self, family: &str, size: f32, cx: &mut Context<Self>) {
        if self.font_family != family || (self.font_size - size).abs() > f32::EPSILON {
            self.font_family = family.to_owned();
            self.font_size = size;
            cx.notify();
        }
    }
    /// Holds the grid size while the layout around the terminal moves: the
    /// program then reflows at most once per `interval` (never for
    /// `Duration::MAX`) instead of on every frame of a slide or drag. `None`
    /// follows every change again, starting with the next paint.
    pub fn set_resize_interval(&mut self, interval: Option<Duration>, cx: &mut Context<Self>) {
        if self.resize_interval != interval {
            self.resize_interval = interval;
            cx.notify();
        }
    }
    pub fn send(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.sender.send(Msg::Input(Cow::Owned(bytes.into())));
    }
    pub fn set_badge(&mut self, badge: Option<TurnBadge>, cx: &mut Context<Self>) {
        if self.badge != badge {
            self.badge = badge;
            cx.notify();
        }
    }
    /// Types a review comment into the program's input without submitting it.
    /// With bracketed paste the comment ends on its own line; otherwise a
    /// newline would act as Enter, so a space separates comments instead.
    pub fn insert_comment(&mut self, text: &str) {
        let bracket = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let line = text.replace(['\r', '\n'], " ");
        self.paste(&if bracket {
            format!("{line}\n")
        } else {
            format!("{line} ")
        });
    }
    pub fn paste(&mut self, text: &str) {
        self.track_typing(text);
        let bracket = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.send(paste_bytes(text, bracket));
    }
    fn track_typing(&mut self, text: &str) {
        let part = typed_part(text, self.typed.len());
        self.typed.push_str(part);
    }
    /// Bytes for a key press in the program's current keyboard mode.
    fn key_input(
        &self,
        key: &str,
        text: Option<&str>,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Option<Vec<u8>> {
        let mode = *self.term.lock().mode();
        let win32 = self.win32_input.load(Ordering::Relaxed);
        encode_key(key, text, ctrl, alt, shift, mode, win32)
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
            if self
                .resize_interval
                .is_some_and(|interval| self.resized_at.elapsed() < interval)
            {
                return;
            }
            self.resized_at = Instant::now();
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
        if let Some(bytes) = self.key_input("tab", None, false, false, backwards) {
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
        // Ctrl+V pastes text, as in Windows Terminal. Without text on the
        // clipboard (an image, files) the program gets the key and reads the
        // clipboard itself, as Codex does for images.
        if cfg!(windows)
            && key.key == "v"
            && key.modifiers.control
            && !key.modifiers.shift
            && !key.modifiers.alt
            && let Some(text) = cx.read_from_clipboard().and_then(|i| i.text())
        {
            self.paste(&text);
            cx.stop_propagation();
            return;
        }
        if let Some(bytes) = self.key_input(
            &key.key,
            key.key_char.as_deref(),
            key.modifiers.control,
            key.modifiers.alt,
            key.modifiers.shift,
        ) {
            {
                let mut term = self.term.lock();
                term.scroll_display(Scroll::Bottom);
                term.selection = None;
            }
            let plain = !key.modifiers.shift && !key.modifiers.control && !key.modifiers.alt;
            match key.key.as_str() {
                "enter" if plain => {
                    self.last_line = std::mem::take(&mut self.typed);
                    self.last_submit = Some(Instant::now());
                }
                "backspace" => {
                    self.typed.pop();
                }
                "escape" => self.typed.clear(),
                "c" | "u" if key.modifiers.control => self.typed.clear(),
                _ => {}
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
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
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
        } else if cfg!(target_os = "macos") {
            vec!["Apple Symbols".into(), "Apple Color Emoji".into()]
        } else {
            vec!["DejaVu Sans".into(), "Noto Color Emoji".into()]
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
        let scale = window.scale_factor();
        // Copy the visible cells and let go of the terminal before shaping
        // text: the PTY thread waits for this lock to parse output, and keys
        // queue behind it meanwhile.
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();
        let (cursor, offset) = {
            let term = self.term.lock();
            let content = term.renderable_content();
            let offset = content.display_offset as i32;
            for cell in content.display_iter {
                let row = cell.point.line.0 + offset;
                if row < 0 || row >= self.rows as i32 {
                    continue;
                }
                let mut fg = foreground(cell.fg, cell.flags, content.colors);
                let mut bg = color(cell.bg, content.colors);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if content.selection.is_some_and(|s| s.contains(cell.point)) {
                    bg = 0x363145;
                }
                let text = cell.c != ' '
                    && !cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN);
                if bg == 0x000000 && !text {
                    continue;
                }
                cells.push(PaintCell {
                    column: cell.point.column.0,
                    row: row as usize,
                    c: text.then_some(cell.c),
                    extra: cell.zerowidth().map(<[char]>::to_vec),
                    fg,
                    bg,
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                });
            }
            let cursor = content.cursor.point;
            let row = cursor.line.0 + offset;
            (
                (row >= 0
                    && row < self.rows as i32
                    && content.mode.contains(TermMode::SHOW_CURSOR))
                .then_some((row, cursor.column.0)),
                offset,
            )
        };
        // Re-hit-test after output, scrollback or a resize, including when the
        // mouse is stationary. All filesystem resolution stays off this thread.
        self.update_link_hover(window, cx);
        for cell in &cells {
            let corner = |column: f32, row: f32| {
                point(
                    bounds.origin.x + px(TERMINAL_INSET + column * self.cell_width),
                    bounds.origin.y + px(TERMINAL_INSET + row * self.cell_height),
                )
            };
            let (column, line) = (cell.column as f32, cell.row as f32);
            let origin = corner(column, line);
            // Both corners come from cell numbers and snap to device pixels, so
            // neighbouring cells share their edges and fills leave no seams.
            let area = crate::glyphs::snap(
                Bounds::from_corners(origin, corner(column + 1., line + 1.)),
                scale,
            );
            if cell.bg != 0x000000 {
                window.paint_quad(fill(area, rgb(cell.bg)));
            }
            let Some(c) = cell.c else {
                continue;
            };
            if crate::glyphs::paint(c, area, rgb(cell.fg).into(), self.font_size, window) {
                continue;
            }
            let mut text = c.to_string();
            if let Some(extra) = &cell.extra {
                text.extend(extra);
            }
            let mut run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: rgb(cell.fg).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            if cell.bold {
                run.font.weight = FontWeight::BOLD;
            }
            if cell.italic {
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
        if let Some(link) = &self.hovered_link {
            for (line, columns) in link.spans() {
                let row = line.0 + offset;
                if row < 0 || row >= self.rows as i32 {
                    continue;
                }
                let fg = cells
                    .iter()
                    .find(|cell| cell.row == row as usize && cell.column == columns.start)
                    .map_or(0xb4a5ff, |cell| cell.fg);
                window.paint_quad(fill(
                    crate::glyphs::snap(
                        Bounds::new(
                            point(
                                bounds.origin.x
                                    + px(TERMINAL_INSET + columns.start as f32 * self.cell_width),
                                bounds.origin.y
                                    + px(TERMINAL_INSET + (row + 1) as f32 * self.cell_height - 2.),
                            ),
                            size(
                                px((columns.end - columns.start) as f32 * self.cell_width),
                                px(1.),
                            ),
                        ),
                        scale,
                    ),
                    rgb(fg),
                ));
            }
        }
        self.cells = cells;
        if let Some((row, column)) = cursor
            && self.focus.is_focused(window)
        {
            window.paint_quad(fill(
                Bounds::new(
                    point(
                        bounds.origin.x + px(TERMINAL_INSET + column as f32 * self.cell_width),
                        bounds.origin.y + px(TERMINAL_INSET + row as f32 * self.cell_height),
                    ),
                    size(px(2.), px(self.cell_height)),
                ),
                rgb(0xb4a5ff),
            ));
        }
    }
}
impl Terminal {
    /// An active turn stays visible even through TUI redraws, tall task lists
    /// and scrollback. Input detection controls placement, never visibility.
    fn badge_element(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::theme::{ADDED, DELETED, TEXT_2, WARNING, icon, ui};
        let badge = self.badge.clone()?;
        if !badge_visible(
            badge.active,
            badge.files,
            self.badge_dismissed.as_deref() == Some(badge.id.as_str()),
        ) {
            return None;
        }
        let label = format!(
            "{} {} changed",
            badge.files,
            if badge.files == 1 { "file" } else { "files" }
        );
        let counts = format!("+{} −{}", badge.additions, badge.deletions);
        let width = 6.8 * (label.len() + counts.len()) as f32 + 52.;
        let available = (f32::from(self.bounds.size.width) - 4.).max(0.);
        let show_counts = width <= available;
        let width = width.min(available);
        let height = self
            .cell_height
            .clamp(16., 24.)
            .min((f32::from(self.bounds.size.height) - 4.).max(0.));
        let need = (width / self.cell_width.max(1.)).ceil() as usize + 1;
        let (detected, grid, live) = {
            let term = self.term.lock();
            let lines = term.screen_lines().min(self.rows);
            let columns = term.columns().min(self.cols);
            let live = term.grid().display_offset() == 0;
            let screen = (0..if live { lines } else { 0 })
                .map(|row| {
                    let line = &term.grid()[Line(row as i32)];
                    (0..columns)
                        .map(|column| {
                            let cell = &line[Column(column)];
                            (cell.c, cell.bg == Color::Named(NamedColor::Background))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            (badge_spot(&screen, need), (lines, columns), live)
        };
        let position = self
            .badge_anchor
            .select(&badge.id, badge.active, detected, grid, live)?;
        let (top, right) = fit_badge(
            position,
            (
                f32::from(self.bounds.size.width),
                f32::from(self.bounds.size.height),
            ),
            (self.cell_width, self.cell_height),
            (width, height),
            TERMINAL_INSET,
        )?;
        let open = badge.id.clone();
        let dismiss = badge.id.clone();
        let status = match &badge.warning {
            Some(warning) => format!("Review this turn's changes · {warning}"),
            None if badge.active => "Review changes so far · the agent is still working".into(),
            None => "Review this turn's changes".into(),
        };
        let tooltip: SharedString = format!("{status}\n{label} {counts}").into();
        let group: SharedString = "turn-badge".into();
        Some(
            div()
                .id("turn-badge")
                .group(group.clone())
                .absolute()
                .occlude()
                .top(px(top))
                .right(px(right.max(2.)))
                .max_w(px(available))
                .overflow_hidden()
                .h(px(height))
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(9.))
                .pr(px(4.))
                .rounded_full()
                .bg(rgb(0x1b1b1b))
                .border_1()
                .border_color(rgb(0x303030))
                .text_size(px(11.5))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0x232323)).border_color(rgb(0x3a3a3a)))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                // Hovering Vyber's native widget is not a mouse movement in
                // the agent's TUI underneath it.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(TerminalEvent::OpenTurn(open.clone()));
                }))
                .when(badge.warning.is_some(), |s| {
                    s.child(div().size(px(6.)).rounded_full().bg(rgb(WARNING)))
                })
                .when(badge.active && badge.warning.is_none(), |s| {
                    s.child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(rgb(TEXT_2))
                            .with_animation(
                                "turn-badge-active",
                                Animation::new(Duration::from_millis(1400))
                                    .repeat()
                                    .with_easing(pulsating_between(0.25, 1.)),
                                |el, t| el.opacity(t),
                            ),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(0xc4c4c4))
                        .child(label),
                )
                .when(show_counts, |s| {
                    s.child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(ADDED))
                            .child(format!("+{}", badge.additions)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(DELETED))
                            .child(format!("−{}", badge.deletions)),
                    )
                })
                .child(
                    div()
                        .id("turn-badge-dismiss")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(15.))
                        .flex_shrink_0()
                        .rounded_full()
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .hover(|s| s.bg(rgb(0x363636)))
                        .child(icon(ui("x"), TEXT_2, 10.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.badge_dismissed = Some(dismiss.clone());
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}

impl Render for Terminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let badge = self.badge_element(cx);
        div()
            .id(("terminal", self.id))
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .cursor_text()
            .when(
                self.hovered_link.is_some()
                    && window.is_window_active()
                    && (window.modifiers().control || window.modifiers().platform),
                |el| el.cursor_pointer(),
            )
            .when_some(self.hovered_link.clone(), |el, link| {
                let label = link.target.label();
                el.tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                })
            })
            .hover_listener_mode(HoverListenerMode::InputModalityIndependent)
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                this.pointer_inside = *hovered;
                this.update_link_hover(window, cx);
            }))
            .on_modifiers_changed(cx.listener(|this, _: &ModifiersChangedEvent, window, cx| {
                this.update_link_hover(window, cx);
            }))
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
                        this.link_click_consumed = true;
                        this.selecting = false;
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
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                this.update_link_hover(window, cx);
                if this.link_click_consumed
                    || ((e.modifiers.control || e.modifiers.platform)
                        && this.hovered_link.is_some())
                {
                    cx.stop_propagation();
                    return;
                }
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
                    if std::mem::take(&mut this.link_click_consumed) {
                        cx.stop_propagation();
                        return;
                    }
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
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if std::mem::take(&mut this.link_click_consumed) {
                        cx.stop_propagation();
                    }
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
            .when_some(self.link_notice.clone(), |el, (message, _)| {
                el.child(
                    div()
                        .absolute()
                        .bottom_3()
                        .left_3()
                        .max_w(px(460.))
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .bg(rgb(0x24212c))
                        .text_size(px(12.))
                        .text_color(rgb(0xd6d6d6))
                        .child(message),
                )
            })
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
            // Keep the native overlay above the custom terminal canvas even
            // on partial hover/canvas repaints; app menus use priority 2.
            .children(badge.map(|badge| deferred(badge).with_priority(1)))
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
        self.track_typing(text);
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

/// Bytes for a key press. A program that negotiated Kitty's keyboard
/// protocol gets CSI u for the keys it covers. On Windows, once ConPTY has
/// asked for win32-input-mode (`win32`), every other key goes as a complete
/// win32 key event: after one such event ConPTY takes a raw ESC for the start
/// of a sequence and holds it until more input arrives, so a raw Escape would
/// wait for the next key and merge into it (ESC + x arrives as Alt+X, ESC + a
/// mouse report vanishes). Other terminals get the legacy sequences.
pub fn encode_key(
    key: &str,
    text: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    mode: TermMode,
    win32: bool,
) -> Option<Vec<u8>> {
    kitty_key(key, ctrl, alt, shift, mode).or_else(|| {
        if win32 {
            win32_key(key, text, ctrl, alt, shift)
        } else {
            legacy_key(key, text, ctrl, alt, shift, mode)
        }
    })
}

/// Bytes for a key press outside win32-input-mode.
#[cfg(test)]
pub fn key_bytes(
    key: &str,
    text: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    mode: TermMode,
) -> Option<Vec<u8>> {
    encode_key(key, text, ctrl, alt, shift, mode, false)
}

fn kitty_key(key: &str, ctrl: bool, alt: bool, shift: bool, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC)
        || (ctrl && alt)
    {
        return None;
    }
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let code = match key {
        "enter" => 13,
        "escape" => 27,
        "tab" => 9,
        "backspace" => 127,
        "space" => 32,
        _ => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            c as u32
        }
    };
    (modifier > 1 || key == "escape" || mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC))
        .then(|| format!("\x1b[{code};{modifier}u").into_bytes())
}

/// A key as win32-input-mode's `CSI Vk;Sc;Uc;Kd;Cs;Rc _`, pressed and then
/// released. ConPTY turns it into a console key event for programs that read
/// those (Codex, PowerShell) and into VT input for the others. It covers the
/// same keys as the legacy sequences; plain characters arrive as text.
pub fn win32_key(
    key: &str,
    text: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> Option<Vec<u8>> {
    const SHIFT: u32 = 0x10;
    const CTRL: u32 = 0x08;
    const ALT: u32 = 0x02;
    const ENHANCED: u32 = 0x100;
    // Virtual key, the character the key types, and whether it is one of the
    // keys Windows marks as enhanced (not on the numeric keypad).
    let (vk, ch, enhanced): (u16, u32, bool) = match key {
        "escape" => (0x1b, 27, false),
        // Ctrl+Enter types LF in Windows. Shift+Enter does too here: programs
        // that read VT input lose Shift on Enter, and LF still breaks the line.
        "enter" => (0x0d, if shift || ctrl { 10 } else { 13 }, false),
        "tab" => (0x09, 9, false),
        "backspace" => (0x08, if ctrl { 0x7f } else { 8 }, false),
        "up" => (0x26, 0, true),
        "down" => (0x28, 0, true),
        "left" => (0x25, 0, true),
        "right" => (0x27, 0, true),
        "home" => (0x24, 0, true),
        "end" => (0x23, 0, true),
        "insert" => (0x2d, 0, true),
        "delete" => (0x2e, 0, true),
        "pageup" => (0x21, 0, true),
        "pagedown" => (0x22, 0, true),
        "space" if ctrl || alt => (0x20, 32, false),
        _ => {
            if let Some(n) = key
                .strip_prefix('f')
                .and_then(|n| n.parse::<u16>().ok())
                .filter(|n| (1..=12).contains(n))
            {
                (0x6f + n, 0, false)
            } else {
                // A character is a key only with Ctrl or Alt; with both it is
                // AltGr typing, which arrives as text.
                if ctrl == alt {
                    return None;
                }
                let mut chars = key.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                let ch = if ctrl {
                    if !c.is_ascii() {
                        return None;
                    }
                    match c.to_ascii_uppercase() {
                        letter @ 'A'..='Z' => letter as u32 & 31,
                        '@' | '[' | '\\' | ']' | '^' | '_' => c as u32 & 31,
                        _ => 0,
                    }
                } else {
                    let mut typed = text?.chars();
                    let typed_char = typed.next()?;
                    if typed.next().is_some() || typed_char as u32 > 0xffff {
                        return None;
                    }
                    typed_char as u32
                };
                (virtual_key(c), ch, false)
            }
        }
    };
    let state = SHIFT * u32::from(shift)
        + CTRL * u32::from(ctrl)
        + ALT * u32::from(alt)
        + if enhanced { ENHANCED } else { 0 };
    let sc = scan_code(vk);
    Some(format!("\x1b[{vk};{sc};{ch};1;{state};1_\x1b[{vk};{sc};{ch};0;{state};1_").into_bytes())
}

/// The virtual key for a character key. GPUI names letter keys by their
/// virtual key, so letters map back directly.
fn virtual_key(c: char) -> u16 {
    match c.to_ascii_uppercase() {
        key @ ('A'..='Z' | '0'..='9') => key as u16,
        _ => {
            #[cfg(windows)]
            {
                use windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;
                let mut units = [0u16; 2];
                if c.encode_utf16(&mut units).len() == 1 {
                    let scan = unsafe { VkKeyScanW(units[0]) };
                    if scan != -1 {
                        return (scan as u16) & 0xff;
                    }
                }
            }
            0
        }
    }
}

fn scan_code(vk: u16) -> u16 {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW};
        unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) as u16 }
    }
    #[cfg(not(windows))]
    {
        let _ = vk;
        0
    }
}

fn legacy_key(
    key: &str,
    text: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let modified = modifier > 1;
    let value = match key {
        "enter" => {
            if shift {
                format!("\x1b[13;{modifier}u")
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
            if alt
                && !ctrl
                && let Some(text) = text
            {
                return Some(format!("\x1b{text}").into_bytes());
            }
            return None;
        }
    };
    Some(value.into_bytes())
}

/// The start of `text` that still fits in the typed text after `used`
/// bytes, so a large paste keeps at most 4 KB of it.
fn typed_part(text: &str, used: usize) -> &str {
    let mut end = TYPED_LIMIT.saturating_sub(used).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// What a paste writes: bracketed when the program asked for it, with ESC
/// removed so the text cannot end the paste early; otherwise line breaks go
/// as Enter.
fn paste_bytes(text: &str, bracket: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    let mut bytes = text.bytes().filter(|b| *b != 0x1b).peekable();
    if bracket {
        out.extend_from_slice(b"\x1b[200~");
        out.extend(bytes);
        out.extend_from_slice(b"\x1b[201~");
    } else {
        while let Some(b) = bytes.next() {
            match b {
                b'\r' => {
                    bytes.next_if_eq(&b'\n');
                    out.push(b'\r');
                }
                b'\n' => out.push(b'\r'),
                _ => out.push(b),
            }
        }
    }
    out
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
    fn badge_sits_on_blank_cells_above_claude_input() {
        let rule = "─".repeat(60);
        let rows = [
            "● Updated src/app.rs with 12 additions",
            "",
            "✻ Working… (esc to interrupt)",
            &rule,
            "❯ next prompt",
            &rule,
            "  ⏵⏵ accept edits on",
        ];
        assert_eq!(super::badge_spot(&screen(&rows, 60), 20), Some((2, 58)));
        // Text under the badge's cells pushes it up to the next blank row.
        let busy = format!("✻ Working… {}", "x".repeat(48));
        let rows = [rows[0], rows[1], &busy, &rule, rows[4], &rule, rows[6]];
        assert_eq!(super::badge_spot(&screen(&rows, 60), 20), Some((1, 58)));
    }

    #[test]
    fn badge_follows_codex_and_boxed_prompts() {
        let rows = [
            "• Working (5s • esc to interrupt)",
            "",
            "› Ask Codex",
            "",
            "  ⏎ send",
        ];
        assert_eq!(super::badge_spot(&screen(&rows, 50), 12), Some((1, 48)));
        let rows = [
            "",
            "╭──────────────────────────────╮",
            "│ > type here                  │",
            "╰──────────────────────────────╯",
        ];
        assert_eq!(super::badge_spot(&screen(&rows, 40), 12), Some((0, 31)));
        // The composer's shaded background is not blank space.
        let mut shaded = screen(&["", "", "› Ask Codex", "", "  ⏎ send"], 50);
        shaded[1].iter_mut().for_each(|cell| cell.1 = false);
        assert_eq!(super::badge_spot(&shaded, 12), Some((0, 48)));
    }

    #[test]
    fn no_badge_without_an_input_box() {
        let rows = [
            "user@host MINGW64 ~/repo",
            "$ cargo test",
            "   Compiling vyber",
        ];
        assert_eq!(super::badge_spot(&screen(&rows, 60), 20), None);
        assert_eq!(super::badge_spot(&screen(&["❯ x"], 60), 20), None);
        assert_eq!(super::badge_spot(&screen(&["", "❯ x"], 20), 20), None);
    }

    #[test]
    fn badge_survives_real_vt_hover_redraws_and_a_tall_task_list() {
        use super::badge::{BadgeAnchorMemory, BadgePosition};
        use super::{Column, Line, Proxy, Size};
        use alacritty_terminal::vte::ansi::{Color, NamedColor};
        use std::sync::{Arc, atomic::AtomicBool, mpsc};

        let (sender, _) = mpsc::channel();
        let proxy = Proxy {
            dirty: Arc::new(AtomicBool::new(false)),
            sender,
        };
        let mut term =
            alacritty_terminal::Term::new(Default::default(), &Size { cols: 80, rows: 48 }, proxy);
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut term, b"\x1b[?1003h\x1b[?1006h");
        let rule = "─".repeat(80);
        for (row, text) in [
            "✻ Working… (4h)",
            "",
            &rule,
            "❯ ",
            &rule,
            "auto mode on · 2 shells",
            "",
            "❯ ● main",
        ]
        .iter()
        .enumerate()
        {
            parser.advance(&mut term, format!("\x1b[{};1H{text}", row + 1).as_bytes());
        }
        for row in 9..=44 {
            parser.advance(
                &mut term,
                format!("\x1b[{row};1H○ fork Running task {row}").as_bytes(),
            );
        }
        let snapshot = |term: &alacritty_terminal::Term<Proxy>| {
            (0..48)
                .map(|row| {
                    (0..80)
                        .map(|column| {
                            let cell = &term.grid()[Line(row)][Column(column)];
                            (cell.c, cell.bg == Color::Named(NamedColor::Background))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let expected = Some((1, 78));
        let mut memory = BadgeAnchorMemory::default();
        let anchor = Some(BadgePosition::Cell { row: 1, right: 78 });
        assert_eq!(super::badge_spot(&snapshot(&term), 30), expected);
        for text in ["› ○ main", "❯ ● main", "○ main", "❯ ● main"] {
            parser.advance(
                &mut term,
                format!("\x1b[48;2;35;35;35m\x1b[8;1H\x1b[2K{text}\x1b[2;1H\x1b[2K\x1b[0m",)
                    .as_bytes(),
            );
            let spot = super::badge_spot(&snapshot(&term), 30);
            assert_eq!(spot, expected);
            assert_eq!(
                memory.select("long-turn", true, spot, (48, 80), true),
                anchor
            );
        }
        // A TUI can clear a frame and restore it in separate PTY writes.
        parser.advance(&mut term, b"\x1b[3;1H\x1b[2K");
        let spot = super::badge_spot(&snapshot(&term), 30);
        assert_eq!(spot, None);
        assert_eq!(
            memory.select("long-turn", true, spot, (48, 80), true),
            anchor
        );
        parser.advance(&mut term, format!("\x1b[3;1H{rule}").as_bytes());
        assert_eq!(super::badge_spot(&snapshot(&term), 30), expected);
        assert!(
            term.mode()
                .contains(TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE)
        );
    }

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
        let win32 = backend
            .win32_input
            .load(std::sync::atomic::Ordering::Relaxed);
        if initial.contains("Update available") && initial.contains("esc skip") {
            let mode = *backend.term.lock().mode();
            backend.sender.send(Msg::Input(
                super::encode_key("escape", None, false, false, false, mode, win32)
                    .unwrap()
                    .into(),
            ))?;
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
        // If a broken Shift+Enter submits, /status remains a local command
        // instead of starting an agent request.
        backend
            .sender
            .send(Msg::Input(b"/status".to_vec().into()))?;
        read_for(0.3);
        backend.sender.send(Msg::Input(
            super::encode_key("enter", None, false, false, true, mode, win32)
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
            .rfind(|(_, s)| s.contains("/status"))
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
            first.is_some() && second == first.map(|row| row + 1),
            "CLI did not show two adjacent input rows; may be at an onboarding/trust screen"
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
    /// A shell in a real ConPTY, driven the way Vyber drives it.
    #[cfg(windows)]
    struct Console(super::Backend);
    #[cfg(windows)]
    impl Console {
        fn start(id: usize, program: &str) -> anyhow::Result<Self> {
            let root = std::env::temp_dir();
            Ok(Self(super::Terminal::prepare(id, &root, Some(program))?))
        }
        fn pump(&self) {
            for event in self.0.events.try_iter() {
                if let Event::PtyWrite(text) = event {
                    self.send(text.as_bytes());
                }
            }
        }
        fn send(&self, bytes: &[u8]) {
            let _ = self.0.sender.send(Msg::Input(bytes.to_vec().into()));
        }
        fn key(
            &self,
            key: &str,
            text: Option<&str>,
            ctrl: bool,
            alt: bool,
            shift: bool,
        ) -> Vec<u8> {
            let mode = *self.0.term.lock().mode();
            let win32 = self
                .0
                .win32_input
                .load(std::sync::atomic::Ordering::Relaxed);
            super::encode_key(key, text, ctrl, alt, shift, mode, win32).unwrap()
        }
        /// Every line, scrollback included.
        fn text(&self) -> String {
            let term = self.0.term.lock();
            let grid = term.grid();
            let top = -(grid.history_size() as i32);
            (top..grid.screen_lines() as i32)
                .map(|line| {
                    (0..grid.columns())
                        .map(|column| grid[TermPoint::new(Line(line), Column(column))].c)
                        .collect::<String>()
                        .trim_end()
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        fn wait_for(&self, done: impl Fn(&str) -> bool, seconds: u64) -> (bool, String) {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            loop {
                self.pump();
                let text = self.text();
                if done(&text) {
                    return (true, text);
                }
                if Instant::now() > deadline {
                    return (false, text);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    #[cfg(windows)]
    impl Drop for Console {
        fn drop(&mut self) {
            let _ = self.0.sender.send(Msg::Shutdown);
        }
    }
    #[cfg(windows)]
    use super::{Column, Dimensions, Event, Line, Msg, TermPoint};
    #[cfg(windows)]
    use std::time::{Duration, Instant};

    /// Keys a console program reads with ReadConsoleInput, as Codex does,
    /// after the cases that used to lose Escape: a win32 key before it (Vyber's
    /// Shift+Enter), a mouse or focus report in the same write, a double press.
    #[test]
    #[cfg(windows)]
    fn real_conpty_console_programs_get_every_key() -> anyhow::Result<()> {
        let console = Console::start(9004, "powershell.exe")?;
        console.send(b"Write-Output ('VYBER_' + 'READY'); while ($true) { $k = [Console]::ReadKey($true); Write-Output ('K=' + $k.Key + '/' + $k.Modifiers + ';'); if ($k.Key -eq 'Q') { break } }\r");
        let (ready, text) = console.wait_for(|t| t.contains("VYBER_READY"), 20);
        assert!(ready, "PowerShell did not start: {text}");
        assert!(
            console
                .0
                .win32_input
                .load(std::sync::atomic::Ordering::Relaxed),
            "ConPTY did not ask for win32-input-mode"
        );
        let esc = console.key("escape", None, false, false, false);
        let mut glued = esc.clone();
        glued.extend_from_slice(b"\x1b[<35;10;5M\x1b[I");
        let mut double = esc.clone();
        double.extend_from_slice(&esc);
        let presses = [
            console.key("enter", None, false, false, true),
            esc.clone(),
            glued,
            double,
            console.key("x", Some("x"), false, true, false),
            console.key("enter", None, true, false, false),
            console.key("up", None, false, false, false),
            console.key("space", None, true, false, false),
            b"q".to_vec(),
        ];
        for press in presses {
            console.send(&press);
            std::thread::sleep(Duration::from_millis(60));
        }
        let keys = |text: &str| {
            text.lines()
                .filter_map(|line| line.trim().strip_prefix("K="))
                .filter_map(|rest| rest.split_once(';').map(|(key, _)| key.to_owned()))
                .collect::<Vec<_>>()
        };
        let expected = [
            "Enter/Shift",
            "Escape/0",
            "Escape/0",
            "Escape/0",
            "Escape/0",
            "X/Alt",
            "Enter/Control",
            "UpArrow/0",
            "Spacebar/Control",
            "Q/0",
        ];
        let (done, text) = console.wait_for(|t| keys(t).len() >= expected.len(), 10);
        assert!(done, "keys went missing: {:?}\n{text}", keys(&text));
        assert_eq!(keys(&text), expected);
        Ok(())
    }

    /// Ctrl+C still stops the running command in each shell Vyber starts.
    #[test]
    #[cfg(windows)]
    fn real_conpty_ctrl_c_interrupts_commands() -> anyhow::Result<()> {
        let bash = "C:/Program Files/Git/bin/bash.exe";
        let mut shells = vec![(
            "powershell.exe",
            "Start-Sleep -Seconds 20; Write-Output ('VYBER_' + 'SLEPT')\r",
            "Write-Output ('VYBER_' + 'AFTER')\r",
        )];
        if std::path::Path::new(bash).exists() {
            shells.push((
                bash,
                "sleep 20 && echo VYBER_$((1))SLEPT\r",
                "echo VYBER_$((0))AFTER\r",
            ));
        }
        for (id, (program, slow, after)) in shells.into_iter().enumerate() {
            let console = Console::start(9010 + id, program)?;
            let (ready, text) = console.wait_for(|t| t.contains('>') || t.contains('$'), 20);
            assert!(ready, "{program} did not start: {text}");
            std::thread::sleep(Duration::from_millis(500));
            console.send(slow.as_bytes());
            std::thread::sleep(Duration::from_millis(1500));
            console.send(&console.key("c", None, true, false, false));
            std::thread::sleep(Duration::from_millis(500));
            console.send(after.as_bytes());
            let (stopped, text) = console.wait_for(
                |t| t.contains("VYBER_0AFTER") || t.contains("VYBER_AFTER"),
                10,
            );
            assert!(
                stopped && !text.contains("VYBER_SLEPT") && !text.contains("VYBER_1SLEPT"),
                "{program}: Ctrl+C did not interrupt:\n{text}"
            );
        }
        Ok(())
    }

    /// Programs that read VT input (Node, so Claude Code) get the bytes
    /// ConPTY derives from win32 key events.
    #[test]
    #[ignore = "needs Node.js"]
    #[cfg(windows)]
    fn real_conpty_vt_programs_get_vt_keys() -> anyhow::Result<()> {
        let script = std::env::temp_dir().join("vyber-vt-keys.js");
        std::fs::write(
            &script,
            "process.stdin.setRawMode(true);process.stdin.resume();console.log('VYBER_READY');\
             process.stdin.on('data',d=>{process.stdout.write('K='+d.toString('hex')+';\\r\\n');if(d.includes(113))process.exit()});",
        )?;
        let console = Console::start(9020, "powershell.exe")?;
        console.send(format!("node '{}'\r", script.display()).as_bytes());
        let (ready, text) = console.wait_for(|t| t.contains("VYBER_READY"), 20);
        assert!(ready, "node did not start: {text}");
        // ConPTY gives these programs nothing for Ctrl+Space, as a win32
        // event or as the raw NUL Vyber sent before, so it is not listed.
        type VtKeyCase<'a> = (&'a str, Option<&'a str>, bool, bool, bool, &'a str);
        let cases: [VtKeyCase<'_>; 11] = [
            ("escape", None, false, false, false, "1b"),
            ("enter", None, false, false, true, "0a"),
            ("x", Some("x"), false, true, false, "1b78"),
            ("a", None, true, false, false, "01"),
            ("backspace", None, false, false, false, "7f"),
            ("backspace", None, true, false, false, "08"),
            ("tab", None, false, false, true, "1b5b5a"),
            ("up", None, false, false, false, "1b5b41"),
            ("left", None, true, false, false, "1b5b313b3544"),
            ("delete", None, false, false, false, "1b5b337e"),
            ("f5", None, false, false, false, "1b5b31357e"),
        ];
        for (key, text, ctrl, alt, shift, _) in cases {
            console.send(&console.key(key, text, ctrl, alt, shift));
            std::thread::sleep(Duration::from_millis(150));
        }
        console.send(b"q");
        let count = |t: &str| t.lines().filter(|l| l.trim().starts_with("K=")).count();
        let (done, text) = console.wait_for(|t| count(t) > cases.len(), 10);
        let got = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("K="))
            .filter_map(|rest| rest.split_once(';').map(|(key, _)| key.to_owned()))
            .collect::<Vec<_>>();
        let _ = std::fs::remove_file(&script);
        assert!(done, "keys went missing: {got:?}");
        for (i, case) in cases.iter().enumerate() {
            assert_eq!(
                got[i], case.5,
                "{} ctrl={} alt={} shift={}",
                case.0, case.2, case.3, case.4
            );
        }
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn real_unix_pty_roundtrip_and_shutdown() -> anyhow::Result<()> {
        use super::{Event, Msg, Terminal};
        let root = tempfile::tempdir()?;
        let backend = Terminal::prepare(9000, root.path(), Some("/bin/sh"))?;
        backend.sender.send(Msg::Input(
            b"printf 'VYBER_%s\\n' PROBE_OK; exit\r".to_vec().into(),
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
        assert!(exited, "Unix PTY child did not exit");
        assert!(
            output.contains("VYBER_PROBE_OK"),
            "Unix PTY output not parsed: {output}"
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
        // Wait for the line editor to consume the text before sending Tab.
        // A displayed prompt can precede PSReadLine's input readiness on CI.
        let (typed, typed_output) = read_until("vyber_completion_pro", 10);
        if !typed {
            let _ = backend.sender.send(Msg::Shutdown);
            anyhow::bail!("PowerShell did not echo completion input: {typed_output}");
        }
        let mode = *backend.term.lock().mode();
        backend.sender.send(Msg::Input(
            key_bytes("tab", None, false, false, false, mode)
                .unwrap()
                .into(),
        ))?;
        let (completed, completion_output) = read_until("vyber_completion_probe.txt", 10);
        backend.sender.send(Msg::Input(vec![13].into()))?;
        let (success, output) = read_until("VYBER_COMPLETION_SUCCESS", 5);
        let _ = backend.sender.send(Msg::Shutdown);
        assert!(
            completed,
            "Tab did not expand the filename: {completion_output}"
        );
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
        let expected_root = dir.path().canonicalize()?;
        let expected_child = dir.path().join("child").canonicalize()?;
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
        let mut read_until = |needle: &str,
                              expected_cwd: Option<&std::path::Path>,
                              want_exit: bool,
                              want_prompt: bool| {
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
                    && expected_cwd.is_none_or(|path| {
                        cwd.as_deref()
                            .and_then(|directory| directory.canonicalize().ok())
                            .as_deref()
                            == Some(path)
                    })
                    && (!want_exit || exited)
                    && (!want_prompt || output.trim_end().ends_with('$'))
                {
                    return (true, output);
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            (false, output)
        };
        let ready = read_until("$", Some(&expected_root), false, true);
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
        let completed = read_until("vyber_bash_probe.txt", None, false, false).0;
        backend.sender.send(Msg::Input(vec![13].into()))?;
        let success = read_until("VYBER_BASH_COMPLETE", Some(&expected_root), false, true).0;
        backend
            .sender
            .send(Msg::Input(b"cd child\r".to_vec().into()))?;
        let moved = read_until("", Some(&expected_child), false, true);
        backend.sender.send(Msg::Input(b"exit\r".to_vec().into()))?;
        let exited = read_until("", None, true, false).0;
        let _ = backend.sender.send(Msg::Shutdown);
        assert!(completed && success, "Git Bash Tab completion failed");
        assert!(
            moved.0,
            "Git Bash did not report the changed directory: cwd={cwd:?}, output={}",
            moved.1
        );
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
    fn keyboard_protocol_negotiates_and_restores_legacy_input() {
        use super::{Event, Proxy, Size};
        use std::sync::{Arc, atomic::AtomicBool, mpsc};

        let (tx, rx) = mpsc::channel();
        let proxy = Proxy {
            dirty: Arc::new(AtomicBool::new(false)),
            sender: tx,
        };
        let config = alacritty_terminal::term::Config {
            kitty_keyboard: true,
            ..Default::default()
        };
        let mut term = alacritty_terminal::Term::new(config, &Size { cols: 80, rows: 24 }, proxy);
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut term, b"\x1b[?u");
        assert!(
            rx.try_iter()
                .any(|event| matches!(event, Event::PtyWrite(text) if text == "\x1b[?0u"))
        );

        parser.advance(&mut term, b"\x1b[>1u");
        assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        assert_eq!(
            key_bytes("enter", None, false, false, true, *term.mode()),
            Some(b"\x1b[13;2u".to_vec())
        );
        assert_eq!(
            key_bytes("t", None, true, false, false, *term.mode()),
            Some(b"\x1b[116;5u".to_vec())
        );

        parser.advance(&mut term, b"\x1b[<u");
        assert!(!term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        assert_eq!(
            key_bytes("enter", None, false, false, false, *term.mode()),
            Some(vec![13])
        );
        assert_eq!(
            key_bytes("t", None, true, false, false, *term.mode()),
            Some(vec![20])
        );
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
            key_bytes(
                "enter",
                None,
                false,
                false,
                true,
                TermMode::DISAMBIGUATE_ESC_CODES
            ),
            Some(b"\x1b[13;2u".to_vec())
        );
        assert_eq!(
            key_bytes(
                "enter",
                None,
                true,
                false,
                true,
                TermMode::DISAMBIGUATE_ESC_CODES
            ),
            Some(b"\x1b[13;6u".to_vec())
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
    /// The fields of a win32-input-mode press and release.
    fn win32_fields(bytes: &[u8]) -> Vec<Vec<u32>> {
        let text = std::str::from_utf8(bytes).unwrap();
        text.split('\x1b')
            .filter(|s| !s.is_empty())
            .map(|s| {
                let body = s.strip_prefix('[').unwrap().strip_suffix('_').unwrap();
                body.split(';').map(|n| n.parse().unwrap()).collect()
            })
            .collect()
    }
    #[test]
    fn win32_keys_carry_virtual_key_character_and_modifiers() {
        use super::{scan_code, win32_key};
        // (key, text, ctrl, alt, shift) -> (virtual key, character, state)
        let cases = [
            (("escape", None, false, false, false), (27, 27, 0)),
            (("enter", None, false, false, false), (13, 13, 0)),
            (("enter", None, false, false, true), (13, 10, 0x10)),
            (("enter", None, true, false, false), (13, 10, 0x08)),
            (("backspace", None, true, false, false), (8, 0x7f, 0x08)),
            (("tab", None, false, false, true), (9, 9, 0x10)),
            (("up", None, false, false, false), (0x26, 0, 0x100)),
            (("delete", None, false, false, true), (0x2e, 0, 0x110)),
            (("f5", None, false, false, false), (0x74, 0, 0)),
            (("f12", None, true, false, false), (0x7b, 0, 0x08)),
            (("c", None, true, false, false), (0x43, 3, 0x08)),
            (
                ("x", Some("X"), false, true, true),
                (0x58, 'X' as u32, 0x12),
            ),
            (("space", Some(" "), false, true, false), (0x20, 32, 0x02)),
            (("space", None, true, false, false), (0x20, 32, 0x08)),
        ];
        for ((key, text, ctrl, alt, shift), (vk, ch, state)) in cases {
            let bytes = win32_key(key, text, ctrl, alt, shift).unwrap();
            let sc = u32::from(scan_code(vk as u16));
            assert_eq!(
                win32_fields(&bytes),
                vec![vec![vk, sc, ch, 1, state, 1], vec![vk, sc, ch, 0, state, 1]],
                "{key} ctrl={ctrl} alt={alt} shift={shift}"
            );
        }
        // Characters are text unless Ctrl or Alt alone is held.
        for (key, text, ctrl, alt) in [
            ("a", Some("a"), false, false),
            ("q", Some("@"), true, true),
            ("space", Some(" "), false, false),
            ("ş", Some("ş"), true, false),
            ("f13", None, false, false),
        ] {
            assert_eq!(win32_key(key, text, ctrl, alt, false), None, "{key}");
        }
    }
    #[test]
    fn keys_follow_kitty_then_win32_then_legacy() {
        use super::encode_key;
        let kitty = TermMode::DISAMBIGUATE_ESC_CODES;
        // A program that negotiated Kitty keeps CSI u, even under ConPTY.
        assert_eq!(
            encode_key("escape", None, false, false, false, kitty, true),
            Some(b"\x1b[27;1u".to_vec())
        );
        // Keys Kitty leaves alone go as win32 events, never a bare ESC.
        let up = encode_key("up", None, false, false, false, kitty, true).unwrap();
        assert_eq!(win32_fields(&up)[0][0], 0x26);
        let esc = encode_key("escape", None, true, true, false, kitty, true).unwrap();
        assert_eq!(win32_fields(&esc)[0][0], 27);
        // Without ConPTY's request the legacy sequences stay.
        assert_eq!(
            encode_key(
                "escape",
                None,
                false,
                false,
                false,
                TermMode::empty(),
                false
            ),
            Some(vec![27])
        );
        assert_eq!(
            encode_key("enter", None, false, false, true, TermMode::empty(), false),
            Some(b"\x1b[13;2u".to_vec())
        );
        // Plain characters are text in every mode.
        for win32 in [false, true] {
            assert_eq!(
                encode_key(
                    "a",
                    Some("a"),
                    false,
                    false,
                    false,
                    TermMode::empty(),
                    win32
                ),
                None
            );
        }
    }
    #[test]
    fn keyboard_reset_restores_legacy_keys() {
        use super::Size;
        let (proxy, _events) = super::Proxy::channel();
        let config = alacritty_terminal::term::Config {
            kitty_keyboard: true,
            ..Default::default()
        };
        let mut term = alacritty_terminal::Term::new(config, &Size { cols: 80, rows: 24 }, proxy);
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        // A program pushed modes twice and set one more, then crashed.
        parser.advance(&mut term, b"\x1b[>1u\x1b[>3u\x1b[=8;2u");
        assert!(term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
        parser.advance(&mut term, crate::pty::KEYBOARD_RESET);
        assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
        assert_eq!(
            key_bytes("c", None, true, false, false, *term.mode()),
            Some(vec![3])
        );
    }
    #[test]
    fn pastes_are_sanitized_without_extra_copies() {
        use super::paste_bytes;
        assert_eq!(
            paste_bytes("a\x1b[201~b\r\nc", true),
            b"\x1b[200~a[201~b\r\nc\x1b[201~".to_vec()
        );
        assert_eq!(
            paste_bytes("a\r\nb\nc\rd\x1b", false),
            b"a\rb\rc\rd".to_vec()
        );
        let big = "ğ".repeat(3_000_000);
        assert_eq!(paste_bytes(&big, true).len(), big.len() + 12);
    }
    #[test]
    fn typed_text_stays_within_its_limit() {
        use super::{TYPED_LIMIT, typed_part};
        let big = "ğ".repeat(10_000);
        let first = typed_part(&big, 0);
        assert!(first.len() <= TYPED_LIMIT && first.len() > TYPED_LIMIT - 2);
        assert_eq!(typed_part(&big, first.len()), "");
        assert_eq!(typed_part("abc", TYPED_LIMIT - 2), "ab");
        assert_eq!(typed_part("abc", TYPED_LIMIT + 10), "");
    }
}

#[cfg(test)]
mod link_interaction_tests {
    use super::{FileLink, Proxy, Size, Terminal};
    use alacritty_terminal::{sync::FairMutex, term::Term};
    use gpui::{Modifiers, MouseButton, TestAppContext, point, px};
    use std::sync::Arc;

    #[gpui_kit::test]
    fn ctrl_hover_without_mouse_motion_and_click_release_use_the_same_file(
        cx: &mut TestAppContext,
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("report.md");
        std::fs::write(&path, "# Report").unwrap();
        let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let mut backend = Terminal::prepare(9901, root.path(), Some(shell)).unwrap();
        // Keep the real input backend, but use a separate deterministic output
        // grid: a shell startup prompt must not overwrite the link fixture.
        let (proxy, _events) = Proxy::channel();
        let mut term = Term::new(
            Default::default(),
            &Size {
                cols: 100,
                rows: 30,
            },
            proxy,
        );
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut term, b"Rapor (report.md).");
        backend.term = Arc::new(FairMutex::new(term));
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(crate::config::Config::default());
            crate::theme::apply(cx);
        });
        let terminal_root = root.path().to_owned();
        let (view, cx) = cx.add_window_view(move |window, cx| {
            window.activate_window();
            let terminal = Terminal::new(9901, &terminal_root, backend, cx);
            window.focus(&terminal.focus, cx);
            terminal
        });
        let position = view.read_with(cx, |terminal, _| {
            terminal.bounds.origin
                + point(
                    px(super::TERMINAL_INSET + terminal.cell_width * 8.5),
                    px(super::TERMINAL_INSET + terminal.cell_height * 0.5),
                )
        });
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        cx.simulate_mouse_move(position, None, Modifiers::none());
        assert!(view.read_with(cx, |terminal, _| terminal.hovered_link.is_none()));
        cx.simulate_modifiers_change(ctrl);
        assert!(view.read_with(cx, |terminal, _| terminal.hovered_link.is_some()));
        assert_eq!(
            view.read_with(cx, |terminal, _| terminal
                .hovered_link
                .as_ref()
                .unwrap()
                .spans()
                .len()),
            1
        );
        cx.simulate_modifiers_change(Modifiers::none());
        assert!(view.read_with(cx, |terminal, _| terminal.hovered_link.is_none()));
        cx.simulate_modifiers_change(ctrl);
        cx.simulate_mouse_down(position, MouseButton::Left, ctrl);
        assert!(view.read_with(cx, |terminal, _| terminal.link_click_consumed));
        assert!(!view.read_with(cx, |terminal, _| terminal.selecting));
        cx.simulate_mouse_up(position, MouseButton::Left, ctrl);
        assert!(!view.read_with(cx, |terminal, _| terminal.link_click_consumed));
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |terminal, _| terminal.open_path.clone()),
            Some(FileLink {
                path,
                location: None
            })
        );
        cx.simulate_mouse_move(point(px(-10.), px(-10.)), None, ctrl);
        assert!(view.read_with(cx, |terminal, _| terminal.hovered_link.is_none()));
    }
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
