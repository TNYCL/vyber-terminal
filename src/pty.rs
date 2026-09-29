//! Observe a small set of OSC messages without changing the terminal byte stream.
use crate::terminal::Proxy;
use alacritty_terminal::{
    event::{Event, EventListener, OnResize, WindowSize},
    tty::{self, ChildEvent, EventedPty, EventedReadWrite},
};
use polling::{PollMode, Poller};
use std::{
    io::{self, Read},
    sync::Arc,
};
#[derive(Default)]
pub struct OscParser {
    state: u8,
    bytes: Vec<u8>,
}
impl OscParser {
    pub fn feed(&mut self, input: &[u8]) -> Vec<String> {
        let mut messages = vec![];
        for &b in input {
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
                        messages.push(String::from_utf8_lossy(&self.bytes).into());
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
                        messages.push(String::from_utf8_lossy(&self.bytes).into());
                    }
                    self.state = 0;
                }
            }
        }
        messages
    }
}
pub struct ObservedPty {
    inner: tty::Pty,
    parser: OscParser,
    proxy: Proxy,
}
impl ObservedPty {
    pub fn new(inner: tty::Pty, proxy: Proxy) -> Self {
        Self {
            inner,
            proxy,
            parser: OscParser::default(),
        }
    }
}
impl Read for ObservedPty {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.reader().read(bytes)?;
        for osc in self.parser.feed(&bytes[..n]) {
            if let Some(path) = osc.strip_prefix("7;") {
                if let Ok(url) = url::Url::parse(path) {
                    if let Ok(path) = url.to_file_path() {
                        self.proxy
                            .send_event(Event::Title(format!("__VYBER_CWD__{}", path.display())));
                    }
                }
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
        Ok(n)
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
    use super::OscParser;
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
}
