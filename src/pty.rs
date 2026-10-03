//! Observe a small set of OSC messages and ConPTY's input mode in the
//! terminal byte stream. The stream changes in one way only: a keyboard mode
//! reset follows each shell prompt.
use crate::terminal::Proxy;
use alacritty_terminal::{
    event::{Event, EventListener, OnResize, WindowSize},
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
                    self.state = if b == b']' { 2 } else { 0 };
                    self.bytes.clear();
                }
                2 => {
                    if b == 7 {
                        messages.push((String::from_utf8_lossy(&self.bytes).into(), i + 1));
                        self.state = 0;
                    } else if b == 27 {
                        self.state = 3;
                    } else if self.bytes.len() < 8192 {
                        self.bytes.push(b);
                    } else {
                        self.state = 0;
                        self.bytes.clear();
                    }
                }
                _ => {
                    if b == b'\\' {
                        messages.push((String::from_utf8_lossy(&self.bytes).into(), i + 1));
                    }
                    self.state = 0;
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
    /// A keyboard reset that did not fit after the last read goes first in
    /// the next one.
    reset_pending: bool,
}
impl Observer {
    pub fn new(proxy: Proxy, win32_input: Arc<AtomicBool>) -> Self {
        Self {
            parser: OscParser::default(),
            proxy,
            win32_mode: ModeWatch::new(9001),
            win32_input,
            reset_pending: false,
        }
    }
    /// Room to keep free at the start of a read buffer of `len` bytes.
    fn lead(&self, len: usize) -> usize {
        if self.reset_pending && len > 2 * KEYBOARD_RESET.len() {
            KEYBOARD_RESET.len()
        } else {
            0
        }
    }
    /// Looks at `n` bytes read after `lead` free bytes and returns how many
    /// bytes of `bytes` the terminal should parse.
    fn observe(&mut self, bytes: &mut [u8], lead: usize, n: usize) -> usize {
        if lead > 0 {
            bytes[..lead].copy_from_slice(KEYBOARD_RESET);
            self.reset_pending = false;
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
                prompt_end = Some(lead + end);
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
        }
        let len = data.end;
        match prompt_end {
            // Right after the prompt's message, in stream order, so a program
            // started at this prompt negotiates on a clean keyboard.
            Some(at) => insert(bytes, len, at, KEYBOARD_RESET).unwrap_or_else(|| {
                self.reset_pending = true;
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
    use super::{KEYBOARD_RESET, ModeWatch, Observer, OscParser};
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
            [b"out".as_slice(), prompt, KEYBOARD_RESET, b"$ "].concat()
        );
        // PowerShell's alternate title and zsh's OSC 7 mark prompts too.
        for osc in [
            b"\x1b]2;__VYBER_CWD2__C:\\repo\x1b\\".as_slice(),
            b"\x1b]7;file://host/repo\x07",
        ] {
            assert_eq!(
                observe(&mut observer, osc, 64),
                [osc, KEYBOARD_RESET].concat()
            );
        }
        // A program's own title is not a prompt.
        let title = b"\x1b]0;codex\x07";
        assert_eq!(observe(&mut observer, title, 64), title);
        // A reset that does not fit goes first in the next read.
        assert_eq!(observe(&mut observer, prompt, 0), prompt);
        let mut next = vec![0; 64];
        let lead = observer.lead(next.len());
        assert_eq!(lead, KEYBOARD_RESET.len());
        next[lead..lead + 3].copy_from_slice(b"abc");
        let len = observer.observe(&mut next, lead, 3);
        assert_eq!(&next[..len], [KEYBOARD_RESET, b"abc"].concat());
        assert_eq!(observer.lead(64), 0);
    }
}
