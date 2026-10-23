//! Observe a small set of OSC messages and ConPTY's input mode in the
//! terminal byte stream. Shell prompt markers are followed by input mode resets.
use crate::terminal::Proxy;
use alacritty_terminal::{
    event::{Event, EventListener, OnResize, WindowSize},
    term::ClipboardType,
    tty::{self, ChildEvent, EventedPty, EventedReadWrite},
};
use polling::{PollMode, Poller};
use std::{
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Pops every Kitty keyboard mode. It follows each shell prompt, so a
/// program that exits or crashes without restoring the keyboard cannot leave
/// Escape, Ctrl+C and the other keys encoded for itself. Windows' console
/// drops those encodings for the programs that read console key events.
pub const KEYBOARD_RESET: &[u8] = b"\x1b[<65535u";

/// A local Vyber prompt also ends mouse/focus reporting left behind by a
/// crashed program or disconnected SSH session. Do not do this for arbitrary
/// OSC 7 messages: remote applications can report their cwd while still running.
pub const PROMPT_RESET: &[u8] = b"\x1b[<65535u\x1b[?9;1000;1002;1003;1004;1005;1006;1015;1016l";

/// The folder a shell prompt reports in the window title. PowerShell
/// alternates the two prefixes, because the console reports a title only
/// when it changes.
pub fn prompt_cwd(title: &str) -> Option<&str> {
    title
        .strip_prefix("__VYBER_CWD__")
        .or_else(|| title.strip_prefix("__VYBER_CWD2__"))
}

/// Whether an OSC message marks a shell prompt: Vyber's prompt title, or the
/// working directory a shell reports (zsh does so before each prompt).
fn marks_prompt(osc: &str) -> bool {
    osc.starts_with("7;")
        || osc
            .strip_prefix("0;")
            .or_else(|| osc.strip_prefix("2;"))
            .is_some_and(|title| prompt_cwd(title).is_some())
}

const OSC52_LIMIT: usize = 1 << 20;

fn vte_osc_ignored(b: u8) -> bool {
    matches!(b, 0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f)
}

fn decode_base64(input: &[u8], strict: bool) -> Option<Vec<u8>> {
    let mut vals = Vec::with_capacity(input.len());
    let mut pad = 0usize;
    for &b in input {
        if vte_osc_ignored(b) || (!strict && (b.is_ascii_whitespace() || b < 0x20)) {
            continue;
        }
        if b == b'=' {
            pad += 1;
            if pad > 2 {
                return None;
            }
            continue;
        }
        if pad > 0 {
            return None;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        vals.push(val);
    }
    if (strict || pad > 0) && !(vals.len() + pad).is_multiple_of(4) {
        return None;
    }
    if vals.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    let (chunks, rem) = vals.as_chunks::<4>();
    for c in chunks {
        out.push((c[0] << 2) | (c[1] >> 4));
        out.push((c[1] << 4) | (c[2] >> 2));
        out.push((c[2] << 6) | c[3]);
    }
    match rem.len() {
        2 => {
            if strict && (rem[1] & 0x0f) != 0 {
                return None;
            }
            out.push((rem[0] << 2) | (rem[1] >> 4));
        }
        3 => {
            if strict && (rem[2] & 0x03) != 0 {
                return None;
            }
            out.push((rem[0] << 2) | (rem[1] >> 4));
            out.push((rem[1] << 4) | (rem[2] >> 2));
        }
        _ => {}
    }
    Some(out)
}

/// Clipboard stores requested by an OSC 52 message that `alacritty_terminal`
/// does not emit on its own (extra targets in `pc`/`cp`, cut-buffer `0`,
/// whitespace-separated or unpadded base64).
fn osc52_stores(osc: &str) -> Vec<(ClipboardType, String)> {
    let Some(rest) = osc.strip_prefix("52;") else {
        return Vec::new();
    };
    let Some((targets, after)) = rest.split_once(';') else {
        return Vec::new();
    };
    let payload = after.split(';').next().unwrap_or(after);
    if payload.trim_ascii() == "?" {
        return Vec::new();
    }
    let Some(bytes) = decode_base64(payload.as_bytes(), false) else {
        return Vec::new();
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Vec::new();
    };
    if text.is_empty() {
        return Vec::new();
    }
    let clean_targets = || targets.bytes().filter(|&b| !vte_osc_ignored(b));
    let mut want_clipboard = clean_targets().next().is_none()
        || clean_targets().any(|b| matches!(b, b'c' | b'q' | b'0'..=b'7'));
    let mut want_selection = clean_targets().any(|b| matches!(b, b'p' | b's'));
    if decode_base64(payload.as_bytes(), true).is_some() {
        match clean_targets().next().unwrap_or(b'c') {
            b'c' => want_clipboard = false,
            b'p' | b's' => want_selection = false,
            _ => {}
        }
    }
    let mut stores = Vec::new();
    if want_clipboard {
        stores.push((ClipboardType::Clipboard, text.clone()));
    }
    if want_selection {
        stores.push((ClipboardType::Selection, text));
    }
    stores
}

#[derive(Default)]
pub struct OscParser {
    state: u8,
    bytes: Vec<u8>,
}
impl OscParser {
    #[cfg(test)]
    pub fn feed(&mut self, input: &[u8]) -> Vec<String> {
        self.feed_at(input)
            .into_iter()
            .map(|(osc, _)| osc)
            .collect()
    }
    /// Each complete message with the offset just past its terminator.
    pub fn feed_at(&mut self, input: &[u8]) -> Vec<(String, usize)> {
        let mut messages = vec![];
        for (i, &b) in input.iter().enumerate() {
            match self.state {
                0 => {
                    if b == 27 {
                        self.state = 1
                    }
                }
                1 => {
                    self.state = if b == b']' {
                        2
                    } else if b == 27 {
                        1
                    } else {
                        0
                    };
                    self.bytes.clear();
                }
                2 => {
                    let limit = if self.bytes.starts_with(b"52;") {
                        OSC52_LIMIT
                    } else {
                        8192
                    };
                    if b == 7 {
                        messages.push((String::from_utf8_lossy(&self.bytes).into(), i + 1));
                        self.state = 0;
                    } else if b == 27 {
                        self.state = 3;
                    } else if self.bytes.len() < limit {
                        self.bytes.push(b);
                    } else {
                        self.state = 0;
                        self.bytes.clear();
                    }
                }
                _ => {
                    if b == b'\\' {
                        messages.push((String::from_utf8_lossy(&self.bytes).into(), i + 1));
                        self.state = 0;
                    } else if b == 27 {
                        self.state = 3;
                    } else if b == b']' {
                        self.state = 2;
                        self.bytes.clear();
                    } else {
                        self.state = 0;
                    }
                }
            }
        }
        messages
    }
}

/// Follows one DEC private mode (`CSI ? Pm h` sets it, `CSI ? Pm l` resets it).
pub struct ModeWatch {
    mode: u32,
    state: u8,
    param: u32,
    matched: bool,
}
impl ModeWatch {
    pub fn new(mode: u32) -> Self {
        Self {
            mode,
            state: 0,
            param: 0,
            matched: false,
        }
    }
    /// The mode's last change in `input`, if any.
    pub fn feed(&mut self, input: &[u8]) -> Option<bool> {
        let mut change = None;
        for &b in input {
            self.state = match (self.state, b) {
                (_, 27) => 1,
                (1, b'[') => 2,
                (2, b'?') => {
                    self.param = 0;
                    self.matched = false;
                    3
                }
                (3, b'0'..=b'9') => {
                    self.param = (self.param * 10 + u32::from(b - b'0')).min(1 << 20);
                    3
                }
                (3, b';') => {
                    self.matched |= self.param == self.mode;
                    self.param = 0;
                    3
                }
                (3, b'h' | b'l') => {
                    if self.matched || self.param == self.mode {
                        change = Some(b == b'h');
                    }
                    0
                }
                _ => 0,
            };
        }
        change
    }
}

/// Inserts `extra` at `at` among the first `len` bytes of `buf`, returning
/// the new length, or `None` when it does not fit.
fn insert(buf: &mut [u8], len: usize, at: usize, extra: &[u8]) -> Option<usize> {
    let grown = len + extra.len();
    if grown > buf.len() {
        return None;
    }
    buf.copy_within(at..len, at + extra.len());
    buf[at..at + extra.len()].copy_from_slice(extra);
    Some(grown)
}
/// What Vyber reads from the terminal stream, kept apart from the pty so
/// it can be tested without one.
pub struct Observer {
    parser: OscParser,
    proxy: Proxy,
    /// ConPTY asks for win32-input-mode (private mode 9001) when it starts.
    win32_mode: ModeWatch,
    win32_input: Arc<AtomicBool>,
    /// An input mode reset that did not fit after the last read goes first in
    /// the next one.
    reset_pending: Option<&'static [u8]>,
}
impl Observer {
    pub fn new(proxy: Proxy, win32_input: Arc<AtomicBool>) -> Self {
        Self {
            parser: OscParser::default(),
            proxy,
            win32_mode: ModeWatch::new(9001),
            win32_input,
            reset_pending: None,
        }
    }
    /// Room to keep free at the start of a read buffer of `len` bytes.
    fn lead(&self, len: usize) -> usize {
        self.reset_pending
            .filter(|reset| len > reset.len())
            .map_or(0, |reset| reset.len())
    }
    /// Looks at `n` bytes read after `lead` free bytes and returns how many
    /// bytes of `bytes` the terminal should parse.
    fn observe(&mut self, bytes: &mut [u8], lead: usize, n: usize) -> usize {
        if lead > 0 {
            bytes[..lead].copy_from_slice(self.reset_pending.take().unwrap());
        }
        let data = lead..lead + n;
        if cfg!(windows)
            && let Some(on) = self.win32_mode.feed(&bytes[data.clone()])
        {
            self.win32_input.store(on, Ordering::Relaxed);
        }
        let mut prompt_end = None;
        for (osc, end) in self.parser.feed_at(&bytes[data.clone()]) {
            if marks_prompt(&osc) {
                let local_prompt = osc
                    .strip_prefix("0;")
                    .or_else(|| osc.strip_prefix("2;"))
                    .is_some_and(|title| prompt_cwd(title).is_some());
                let reset = if local_prompt {
                    PROMPT_RESET
                } else {
                    KEYBOARD_RESET
                };
                prompt_end = Some((lead + end, reset));
            }
            if let Some(path) = osc.strip_prefix("7;")
                && let Ok(url) = url::Url::parse(path)
                && let Ok(path) = url.to_file_path()
            {
                self.proxy
                    .send_event(Event::Title(format!("__VYBER_CWD__{}", path.display())));
            }
            let message = osc
                .strip_prefix("9;")
                .filter(|s| !s.starts_with("4;"))
                .or_else(|| osc.strip_prefix("777;notify;"));
            if let Some(message) = message {
                self.proxy.send_event(Event::Title(format!(
                    "__VYBER_NOTIFY__{}",
                    message.replace(';', " · ")
                )));
            }
            for (kind, text) in osc52_stores(&osc) {
                self.proxy.send_event(Event::ClipboardStore(kind, text));
            }
        }
        let len = data.end;
        match prompt_end {
            // Right after the prompt's message, in stream order, so a program
            // started at this prompt negotiates on a clean keyboard.
            Some((at, reset)) => insert(bytes, len, at, reset).unwrap_or_else(|| {
                self.reset_pending = Some(reset);
                len
            }),
            None => len,
        }
    }
}
pub struct ObservedPty {
    inner: tty::Pty,
    observer: Observer,
}
impl ObservedPty {
    pub fn new(inner: tty::Pty, observer: Observer) -> Self {
        Self { inner, observer }
    }
}
impl Read for ObservedPty {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let lead = self.observer.lead(bytes.len());
        let n = self.inner.reader().read(&mut bytes[lead..])?;
        if n == 0 {
            return Ok(0);
        }
        Ok(self.observer.observe(bytes, lead, n))
    }
}
impl EventedReadWrite for ObservedPty {
    type Reader = Self;
    type Writer = <tty::Pty as EventedReadWrite>::Writer;
    unsafe fn register(
        &mut self,
        p: &Arc<Poller>,
        e: polling::Event,
        m: PollMode,
    ) -> io::Result<()> {
        unsafe { self.inner.register(p, e, m) }
    }
    fn reregister(&mut self, p: &Arc<Poller>, e: polling::Event, m: PollMode) -> io::Result<()> {
        self.inner.reregister(p, e, m)
    }
    fn deregister(&mut self, p: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(p)
    }
    fn reader(&mut self) -> &mut Self {
        self
    }
    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}
impl EventedPty for ObservedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}
impl OnResize for ObservedPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.inner.on_resize(size);
    }
}
#[cfg(test)]
mod tests {
    use super::{KEYBOARD_RESET, ModeWatch, Observer, OscParser, PROMPT_RESET};
    use std::sync::{Arc, atomic::AtomicBool};
    #[test]
    fn split_osc_and_limits() {
        let mut p = OscParser::default();
        assert!(p.feed(b"text\x1b]777;notify;Vyber;").is_empty());
        assert_eq!(p.feed(b"Done\x1b\\"), vec!["777;notify;Vyber;Done"]);
        assert_eq!(p.feed(b"\x1b]9;hello\x07"), vec!["9;hello"]);
        p.feed(&[27, b']']);
        p.feed(&vec![b'a'; 9000]);
        assert!(p.bytes.len() <= 8192);
    }
    #[test]
    fn win32_input_mode_follows_conpty() {
        let mut watch = ModeWatch::new(9001);
        assert_eq!(watch.feed(b"\x1b[?1004h\x1b[?90"), None);
        assert_eq!(watch.feed(b"01h"), Some(true));
        assert_eq!(watch.feed(b"\x1b[?25;9001l"), Some(false));
        assert_eq!(watch.feed(b"\x1b[?19001h\x1b[9001h\x1b[?900h"), None);
        assert_eq!(watch.feed(b"\x1b[?9001;1004h"), Some(true));
    }
    fn observe(observer: &mut Observer, input: &[u8], room: usize) -> Vec<u8> {
        let mut buf = vec![0; input.len() + room];
        buf[..input.len()].copy_from_slice(input);
        let len = observer.observe(&mut buf, 0, input.len());
        buf.truncate(len);
        buf
    }
    #[test]
    fn keyboard_reset_follows_each_prompt_in_stream_order() {
        let (proxy, _events) = crate::terminal::Proxy::channel();
        let mut observer = Observer::new(proxy, Arc::new(AtomicBool::new(false)));
        let prompt = b"\x1b]0;__VYBER_CWD__C:/repo\x07";
        let out = observe(
            &mut observer,
            &[b"out".as_slice(), prompt, b"$ "].concat(),
            64,
        );
        assert_eq!(
            out,
            [b"out".as_slice(), prompt, PROMPT_RESET, b"$ "].concat()
        );
        // PowerShell's alternate title and zsh's OSC 7 mark prompts too.
        for (osc, reset) in [
            (
                b"\x1b]2;__VYBER_CWD2__C:\\repo\x1b\\".as_slice(),
                PROMPT_RESET,
            ),
            (b"\x1b]7;file://host/repo\x07".as_slice(), KEYBOARD_RESET),
        ] {
            assert_eq!(observe(&mut observer, osc, 64), [osc, reset].concat());
        }
        // A program's own title is not a prompt.
        let title = b"\x1b]0;codex\x07";
        assert_eq!(observe(&mut observer, title, 64), title);
        // A reset that does not fit goes first in the next read.
        assert_eq!(observe(&mut observer, prompt, 0), prompt);
        let mut next = vec![0; 64];
        let lead = observer.lead(next.len());
        assert_eq!(lead, PROMPT_RESET.len());
        next[lead..lead + 3].copy_from_slice(b"abc");
        let len = observer.observe(&mut next, lead, 3);
        assert_eq!(&next[..len], [PROMPT_RESET, b"abc"].concat());
        assert_eq!(observer.lead(64), 0);
    }

    #[test]
    fn local_prompt_clears_mouse_and_focus_modes_after_disconnect() {
        use alacritty_terminal::{
            grid::Dimensions,
            term::{Config, Term, TermMode},
            vte::ansi,
        };
        struct Size;
        impl Dimensions for Size {
            fn total_lines(&self) -> usize {
                10
            }
            fn screen_lines(&self) -> usize {
                10
            }
            fn columns(&self) -> usize {
                80
            }
        }
        let (proxy, _events) = crate::terminal::Proxy::channel();
        let mut term = Term::new(Config::default(), &Size, proxy.clone());
        let mut parser: ansi::Processor = ansi::Processor::new();
        let mut observer = Observer::new(proxy, Arc::new(AtomicBool::new(false)));
        parser.advance(&mut term, b"\x1b[?1003;1004;1006h");
        assert!(
            term.mode()
                .contains(TermMode::MOUSE_MOTION | TermMode::FOCUS_IN_OUT)
        );
        // A remote cwd update must not stop a running application's mouse input.
        parser.advance(
            &mut term,
            &observe(&mut observer, b"\x1b]7;file://remote/home/user\x07", 64),
        );
        assert!(
            term.mode()
                .contains(TermMode::MOUSE_MOTION | TermMode::FOCUS_IN_OUT)
        );
        let reset = observe(&mut observer, b"\x1b]0;__VYBER_CWD__C:/repo\x07", 64);
        parser.advance(&mut term, &reset);
        assert!(
            !term
                .mode()
                .intersects(TermMode::MOUSE_MODE | TermMode::FOCUS_IN_OUT | TermMode::SGR_MOUSE)
        );
        // A later program can negotiate mouse input normally again.
        parser.advance(&mut term, b"\x1b[?1003;1004;1006h");
        assert!(
            term.mode()
                .contains(TermMode::MOUSE_MOTION | TermMode::FOCUS_IN_OUT | TermMode::SGR_MOUSE)
        );
    }

    #[test]
    fn osc52_copies_across_direct_tmux_passthrough_wrapped_and_combined_targets() {
        use alacritty_terminal::{
            event::Event,
            grid::Dimensions,
            term::{ClipboardType, Config, Term},
            vte::ansi,
        };
        struct Size;
        impl Dimensions for Size {
            fn total_lines(&self) -> usize {
                10
            }
            fn screen_lines(&self) -> usize {
                10
            }
            fn columns(&self) -> usize {
                80
            }
        }
        let (proxy, events) = crate::terminal::Proxy::channel();
        let mut term = Term::new(Config::default(), &Size, proxy.clone());
        let mut parser: ansi::Processor = ansi::Processor::new();
        let mut observer = Observer::new(proxy, Arc::new(AtomicBool::new(false)));
        let mut run = |chunks: &[&[u8]]| -> Vec<(ClipboardType, String)> {
            for chunk in chunks {
                let bytes = observe(&mut observer, chunk, 64);
                parser.advance(&mut term, &bytes);
            }
            events
                .try_iter()
                .filter_map(|event| match event {
                    Event::ClipboardStore(kind, text) => Some((kind, text)),
                    _ => None,
                })
                .collect()
        };

        // Standard direct OSC 52 (BEL, ST, empty selection target) emits once.
        assert_eq!(
            run(&[b"\x1b]52;c;SGVsbG8=\x07"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        assert_eq!(
            run(&[b"\x1b]52;;SGVsbG8=\x1b\\"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        // Combined `pc` target updates both selection and system clipboard.
        let mut pc = run(&[b"\x1b]52;pc;SGVsbG8=\x07"]);
        pc.sort_by_key(|(kind, _)| matches!(kind, ClipboardType::Clipboard));
        assert_eq!(
            pc,
            vec![
                (ClipboardType::Selection, "Hello".into()),
                (ClipboardType::Clipboard, "Hello".into()),
            ]
        );
        // Cut-buffer `0`, line-wrapped base64, space-separated and unpadded base64.
        assert_eq!(
            run(&[b"\x1b]52;0;SGVsbG8=\x07"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        assert_eq!(
            run(&[b"\x1b]52;c;SGVs\r\nbG8=\x07"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        assert_eq!(
            run(&[b"\x1b]52;c;SGVs bG8\x07"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        // tmux DCS passthrough (both BEL and doubled-ESC ST, split across reads).
        assert_eq!(
            run(&[b"\x1bPtmux;\x1b", b"\x1b]52;c;SGVs", b"bG8=\x07\x1b\\"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        assert_eq!(
            run(&[b"\x1bPtmux;\x1b\x1b]52;c;SGVsbG8=\x1b", b"\x1b\\\x1b\\"]),
            vec![(ClipboardType::Clipboard, "Hello".into())]
        );
        // Clipboard read queries (`?`) are never answered or stored.
        assert!(run(&[b"\x1b]52;c;?\x07"]).is_empty());
        assert!(events.try_iter().next().is_none());
    }
}
