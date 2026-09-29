//! Passive readers: transcripts provide boundaries; snapshots provide file changes.
use crate::workspace::{self, Change, Checkpoint};
use notify::Watcher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime},
};

/// Every observed task carries this caveat; other warnings are worth showing.
pub const BASELINE_NOTE: &str = "Observed baseline; early writes and manual edits may be included.";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskReview {
    pub id: String,
    pub agent: String,
    pub label: String,
    pub root: PathBuf,
    /// Agent session the turn belongs to, so its terminal can be found again.
    #[serde(default)]
    pub session: String,
    /// The program that logged the session, such as `codex-tui`, `Codex
    /// Desktop` or Claude Code's `cli`, when the log names it.
    #[serde(default)]
    pub client: String,
    /// When Vyber saw the turn start, in Unix milliseconds.
    #[serde(default)]
    pub started: i64,
    pub before: Option<Checkpoint>,
    pub after: Option<Checkpoint>,
    pub changes: Vec<Change>,
    pub active: bool,
    pub warning: String,
}
impl TaskReview {
    /// Whether the agent may run in a terminal. Codex's desktop app and the
    /// editor extensions log their turns in the same place as the CLIs.
    pub fn from_terminal(&self) -> bool {
        let client = self.client.to_lowercase();
        client.is_empty() || ["tui", "cli", "exec"].iter().any(|k| client.contains(k))
    }
}
/// The process of a Claude Code session, which Claude Code names in a file
/// of its own for every session it runs.
pub fn claude_process(session: &str) -> Option<u32> {
    let dir = dirs::home_dir()?.join(".claude").join("sessions");
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let bytes = fs::read(entry.path()).ok()?;
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        if value["sessionId"].as_str()? != session {
            return None;
        }
        value["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
    })
}
#[derive(Clone, Debug, PartialEq)]
pub enum Boundary {
    Start {
        id: String,
        agent: String,
        session: String,
        client: String,
        root: PathBuf,
        label: String,
    },
    Label {
        id: String,
        label: String,
    },
    End {
        id: String,
        interrupted: bool,
    },
}
#[derive(Default)]
pub struct Parser {
    session: String,
    client: String,
    root: PathBuf,
    current: Option<String>,
    paginated: bool,
}
fn label(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}
fn text_content(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.into();
    }
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
impl Parser {
    pub fn feed(&mut self, v: &Value) -> Vec<Boundary> {
        let mut events = vec![];
        let kind = v["type"].as_str().unwrap_or("");
        if kind == "session_meta" {
            let p = &v["payload"];
            self.session = p["session_id"]
                .as_str()
                .or(p["id"].as_str())
                .unwrap_or("")
                .into();
            self.root = p["cwd"].as_str().unwrap_or("").into();
            self.client = p["originator"].as_str().unwrap_or("").into();
            self.paginated = p["history_mode"] == "paginated";
        }
        if v["isSidechain"] == true {
            return events;
        }
        if let Some(cwd) = v["cwd"].as_str() {
            self.root = cwd.into();
        }
        if let Some(id) = v["sessionId"].as_str() {
            self.session = id.into();
        }
        if let Some(client) = v["entrypoint"].as_str() {
            self.client = client.into();
        }
        if kind == "event_msg" {
            let p = &v["payload"];
            let typ = p["type"].as_str().unwrap_or("");
            let turn = p["turn_id"].as_str().unwrap_or("");
            if matches!(typ, "task_started" | "turn_started") && !turn.is_empty() {
                let id = format!("codex:{}:{turn}", self.session);
                if self.current.as_ref() != Some(&id) {
                    if let Some(old) = self.current.replace(id.clone()) {
                        events.push(Boundary::End {
                            id: old,
                            interrupted: true,
                        });
                    }
                    events.push(Boundary::Start {
                        id,
                        agent: "Codex".into(),
                        session: self.session.clone(),
                        client: self.client.clone(),
                        root: self.root.clone(),
                        label: String::new(),
                    });
                }
            }
            if matches!(typ, "task_complete" | "turn_complete" | "turn_aborted") {
                if let Some(id) = self.current.clone() {
                    if turn.is_empty() || id.ends_with(&format!(":{turn}")) {
                        self.current = None;
                        events.push(Boundary::End {
                            id,
                            interrupted: typ == "turn_aborted",
                        });
                    }
                }
            }
            if typ == "user_message" && !self.paginated {
                if let Some(id) = &self.current {
                    events.push(Boundary::Label {
                        id: id.clone(),
                        label: label(&text_content(&p["message"])),
                    });
                }
            }
            if typ == "item_completed" {
                let item = &p["item"];
                if matches!(item["type"].as_str(), Some("UserMessage" | "user_message")) {
                    if let Some(id) = &self.current {
                        let text = text_content(item.get("content").unwrap_or(&item["text"]));
                        events.push(Boundary::Label {
                            id: id.clone(),
                            label: label(&text),
                        });
                    }
                }
            }
        }
        if kind == "user" && v["isMeta"] != true {
            let content = &v["message"]["content"];
            let prompt = content.is_string()
                || content.as_array().is_some_and(|a| {
                    a.iter().any(|i| i["type"] == "text")
                        && !a.iter().any(|i| i["type"] == "tool_result")
                });
            if prompt {
                if let Some(prompt_id) = v["promptId"].as_str().or(v["uuid"].as_str()) {
                    let queued = v["isQueued"] == true || v["isQueuedMessage"] == true;
                    if queued && self.current.is_some() {
                        return events;
                    }
                    let id = format!("claude:{}:{prompt_id}", self.session);
                    if self.current.as_ref() != Some(&id) {
                        if let Some(old) = self.current.replace(id.clone()) {
                            events.push(Boundary::End {
                                id: old,
                                interrupted: true,
                            });
                        }
                        events.push(Boundary::Start {
                            id,
                            agent: "Claude".into(),
                            session: self.session.clone(),
                            client: self.client.clone(),
                            root: self.root.clone(),
                            label: label(&text_content(content)),
                        });
                    }
                }
            }
        }
        if kind == "system" && v["subtype"] == "turn_duration" {
            if let Some(id) = self.current.take() {
                events.push(Boundary::End {
                    id,
                    interrupted: false,
                });
            }
        }
        events
    }
}
struct Tail {
    offset: u64,
    partial: Vec<u8>,
    parser: Parser,
}
impl Tail {
    fn initial(path: &Path, existing: bool) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let mut parser = Parser::default();
        if existing {
            let mut bytes = vec![];
            file.by_ref().take(512 * 1024).read_to_end(&mut bytes).ok();
            for line in bytes.split(|b| *b == b'\n') {
                if let Ok(v) = serde_json::from_slice(line) {
                    parser.feed(&v);
                }
            }
            parser.current = None;
        }
        Some(Self {
            offset: if existing { len } else { 0 },
            partial: vec![],
            parser,
        })
    }
    fn read(&mut self, path: &Path) -> Vec<Boundary> {
        let Ok(mut file) = File::open(path) else {
            return vec![];
        };
        let Ok(meta) = file.metadata() else {
            return vec![];
        };
        if meta.len() < self.offset {
            self.offset = 0;
            self.partial.clear();
            self.parser = Parser::default();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return vec![];
        }
        let mut bytes = vec![];
        if file.take(8 * 1024 * 1024).read_to_end(&mut bytes).is_err() {
            return vec![];
        }
        self.offset += bytes.len() as u64;
        self.partial.extend(bytes);
        let Some(last) = self.partial.iter().rposition(|b| *b == b'\n') else {
            if self.partial.len() > 16 * 1024 * 1024 {
                self.partial.clear();
            }
            return vec![];
        };
        let pending = self.partial.split_off(last + 1);
        let complete = std::mem::replace(&mut self.partial, pending);
        complete
            .split(|b| *b == b'\n')
            .filter_map(|l| serde_json::from_slice(l).ok())
            .flat_map(|v| self.parser.feed(&v))
            .collect()
    }
}
pub fn matches_root(cwd: &Path, workspace: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = p
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_owned();
        if cfg!(windows) { s.to_lowercase() } else { s }
    }
    let cwd = norm(cwd);
    let root = norm(workspace);
    cwd == root || cwd.starts_with(&(root + "/"))
}
pub fn persist(review: &TaskReview) {
    let dir = workspace::data_dir().join("tasks");
    let _ = fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_vec_pretty(review) {
        let key = blake3::hash(review.id.as_bytes()).to_hex();
        let _ = fs::write(dir.join(format!("{key}.json")), json);
    }
}
pub fn history(root: &Path) -> Vec<TaskReview> {
    let mut reviews: Vec<_> = fs::read_dir(workspace::data_dir().join("tasks"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let bytes = fs::read(e.path()).ok()?;
            let task: TaskReview = serde_json::from_slice(&bytes).ok()?;
            if matches_root(&task.root, root) {
                Some(task)
            } else {
                None
            }
        })
        .collect();
    reviews.sort_by(|a, b| {
        a.before
            .as_ref()
            .map(|s| &s.id)
            .cmp(&b.before.as_ref().map(|s| &s.id))
    });
    let config = crate::config::Config::load();
    let cutoff =
        chrono::Utc::now().timestamp_millis() - (config.task_history_days as i64) * 86400000;
    reviews.retain(|r| {
        r.before
            .as_ref()
            .and_then(|s| s.id.split('-').next())
            .and_then(|s| s.parse::<i64>().ok())
            .is_some_and(|t| t >= cutoff)
    });
    if reviews.len() > config.task_history_limit {
        reviews.drain(..reviews.len() - config.task_history_limit);
    }
    reviews
}
fn capture_end(review: &mut TaskReview, interrupted: bool) {
    review.active = false;
    if interrupted {
        review
            .warning
            .push_str(" Interrupted; end boundary was observed late.");
    }
    match Checkpoint::capture(&review.root, "Task complete") {
        Ok(after) => {
            if let Some(before) = &review.before {
                match before.changes_to(&after) {
                    Ok(changes) => review.changes = changes,
                    Err(e) => review.warning = e.to_string(),
                }
            }
            review.after = Some(after);
        }
        Err(e) => review.warning = format!("End snapshot failed: {e}"),
    }
    persist(review);
}
pub struct Monitor {
    pub receiver: mpsc::Receiver<TaskReview>,
    stop: Arc<AtomicBool>,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Monitor {
    pub fn start(roots: Arc<std::sync::Mutex<Vec<PathBuf>>>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        std::thread::spawn(move || {
            let mut tails: HashMap<PathBuf, Tail> = HashMap::new();
            let mut reviews: HashMap<String, TaskReview> = HashMap::new();
            let mut first = true;
            let mut discovery = Instant::now() - Duration::from_secs(10);
            let mut refresh = Instant::now();
            let mut changed = HashSet::new();
            // Changed paths an active turn's latest snapshot has not read yet.
            let mut unread: HashMap<String, HashSet<PathBuf>> = HashMap::new();
            let (tx, rx) = mpsc::channel();
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let _ = tx.send(event);
                })
                .ok();
            let mut watched = HashSet::new();
            while !done.load(Ordering::Relaxed) {
                if discovery.elapsed() > Duration::from_secs(3) {
                    for root in roots.lock().unwrap().iter() {
                        if watched.insert(root.clone()) {
                            if let Some(w) = &mut watcher {
                                let _ = w.watch(root, notify::RecursiveMode::Recursive);
                            }
                        }
                    }
                    for path in recent_logs() {
                        if !tails.contains_key(&path) {
                            if let Some(tail) = Tail::initial(&path, first) {
                                tails.insert(path, tail);
                            }
                        }
                    }
                    first = false;
                    discovery = Instant::now();
                }
                for event in rx.try_iter().flatten() {
                    for path in event.paths {
                        if !path.components().any(|c| {
                            matches!(
                                c.as_os_str().to_str(),
                                Some(".git" | "target" | "node_modules")
                            )
                        }) {
                            changed.insert(path);
                        }
                    }
                }
                let events: Vec<_> = tails.iter_mut().flat_map(|(p, t)| t.read(p)).collect();
                for event in events {
                    match event {
                        Boundary::Start {
                            id,
                            agent,
                            session,
                            client,
                            root,
                            label,
                        } => {
                            if reviews.contains_key(&id) || root.as_os_str().is_empty() {
                                continue;
                            }
                            if !roots.lock().unwrap().iter().any(|r| matches_root(&root, r)) {
                                continue;
                            }
                            let mut overlaps = false;
                            // A turn in a folder of projects overlaps turns in each of them.
                            for old in reviews.values_mut().filter(|r| {
                                r.active
                                    && (matches_root(&r.root, &root)
                                        || matches_root(&root, &r.root))
                            }) {
                                overlaps = true;
                                if !old.warning.contains("Concurrent") {
                                    old.warning
                                        .push_str(" Concurrent task: changes may overlap.");
                                    let _ = sender.send(old.clone());
                                }
                            }
                            let before = Checkpoint::capture(&root, &format!("{agent} task start"));
                            let warning = match &before {
                                Ok(_) => format!(
                                    "{BASELINE_NOTE}{}",
                                    if overlaps {
                                        " Concurrent task: changes may overlap."
                                    } else {
                                        ""
                                    }
                                ),
                                Err(e) => format!("No baseline: {e}. Manual checkpoint mode."),
                            };
                            let label = if label.is_empty() {
                                format!("{agent} · {}", chrono::Local::now().format("%H:%M"))
                            } else {
                                format!("{agent} · {label}")
                            };
                            let review = TaskReview {
                                id: id.clone(),
                                agent,
                                label,
                                root,
                                session,
                                client,
                                started: chrono::Utc::now().timestamp_millis(),
                                before: before.ok(),
                                after: None,
                                changes: vec![],
                                active: true,
                                warning,
                            };
                            let _ = sender.send(review.clone());
                            reviews.insert(id, review);
                        }
                        Boundary::Label { id, label } => {
                            if !label.is_empty() {
                                if let Some(r) = reviews.get_mut(&id) {
                                    r.label = format!("{} · {label}", r.agent);
                                    let _ = sender.send(r.clone());
                                }
                            }
                        }
                        Boundary::End { id, interrupted } => {
                            unread.remove(&id);
                            if let Some(r) = reviews.get_mut(&id) {
                                capture_end(r, interrupted);
                                let _ = sender.send(r.clone());
                            }
                        }
                    }
                }
                if refresh.elapsed() > Duration::from_secs(2) && !changed.is_empty() {
                    for r in reviews.values_mut().filter(|r| r.active) {
                        let Some(before) = &r.before else {
                            continue;
                        };
                        let pending = unread.entry(r.id.clone()).or_default();
                        pending
                            .extend(changed.iter().filter(|p| matches_root(p, &r.root)).cloned());
                        if pending.is_empty() {
                            continue;
                        }
                        let latest = r.after.as_ref().unwrap_or(before);
                        let paths: Vec<_> = pending.iter().cloned().collect();
                        if let Ok(after) = latest.refresh("Live review", &paths) {
                            pending.clear();
                            if let Ok(changes) = before.changes_to(&after) {
                                r.changes = changes;
                            }
                            r.after = Some(after);
                            let _ = sender.send(r.clone());
                        }
                    }
                    changed.clear();
                    refresh = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        Self { receiver, stop }
    }
}
fn recent_logs() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let mut result = vec![];
    for base in [home.join(".codex/sessions"), home.join(".claude/projects")] {
        for entry in ignore::WalkBuilder::new(base)
            .hidden(false)
            .ignore(false)
            .git_ignore(false)
            .max_depth(Some(5))
            .build()
            .flatten()
        {
            if entry.path().extension().is_some_and(|e| e == "jsonl") {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if SystemTime::now()
                            .duration_since(modified)
                            .unwrap_or_default()
                            < Duration::from_secs(24 * 3600)
                        {
                            result.push((modified, entry.path().to_owned()));
                        }
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| b.0.cmp(&a.0));
    result.into_iter().take(64).map(|(_, p)| p).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn codex_legacy_and_aliases() {
        let mut p = Parser::default();
        p.feed(&json!({"type":"session_meta","payload":{"id":"s","cwd":"/repo"}}));
        assert!(matches!(
            &p.feed(&json!({"type":"event_msg","payload":{"type":"turn_started","turn_id":"t"}}))
                [0],
            Boundary::Start { .. }
        ));
        assert!(
            matches!(&p.feed(&json!({"type":"event_msg","payload":{"type":"user_message","message":"edit README"}}))[0],Boundary::Label{label,..}if label=="edit README")
        );
        assert!(matches!(
            &p.feed(&json!({"type":"event_msg","payload":{"type":"turn_complete","turn_id":"t"}}))
                [0],
            Boundary::End { .. }
        ));
    }
    #[test]
    fn claude_tool_results_and_sidechains() {
        let mut p = Parser::default();
        assert!(p.feed(&json!({"type":"user","uuid":"x","message":{"content":[{"type":"tool_result","content":"result"}]}})).is_empty());
        assert!(
            p.feed(
                &json!({"type":"user","uuid":"x","isSidechain":true,"message":{"content":"child"}})
            )
            .is_empty()
        );
        assert!(matches!(&p.feed(&json!({"type":"user","uuid":"u","sessionId":"s","cwd":"/r","message":{"content":"hello"}}))[0],Boundary::Start{..}));
        assert!(matches!(
            &p.feed(&json!({"type":"system","subtype":"turn_duration"}))[0],
            Boundary::End { .. }
        ));
    }
    #[test]
    fn interrupted_task_closes_before_new_task() {
        let mut p = Parser::default();
        p.feed(&json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"one"}}));
        let e =
            p.feed(&json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"two"}}));
        assert_eq!(e.len(), 2);
        assert!(matches!(
            e[0],
            Boundary::End {
                interrupted: true,
                ..
            }
        ));
    }
    #[test]
    fn partial_unicode_record_is_buffered() {
        use std::io::Write;
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut tail = Tail::initial(f.path(), false).unwrap();
        let bytes =
            "{\"type\":\"user\",\"uuid\":\"t\",\"message\":{\"content\":\"Türkçe\"}}\n".as_bytes();
        let cut = bytes.iter().position(|b| *b == 0xc3).unwrap() + 1;
        fs::write(f.path(), &bytes[..cut]).unwrap();
        assert!(tail.read(f.path()).is_empty());
        fs::OpenOptions::new()
            .append(true)
            .open(f.path())
            .unwrap()
            .write_all(&bytes[cut..])
            .unwrap();
        assert!(matches!(&tail.read(f.path())[0],Boundary::Start{label,..}if label=="Türkçe"));
    }
    #[test]
    fn unknown_schema_is_ignored() {
        assert!(
            Parser::default()
                .feed(&json!({"new_event":true}))
                .is_empty()
        );
    }
    #[test]
    fn turns_name_the_program_that_logged_them() {
        let client = |events: Vec<Boundary>| match &events[0] {
            Boundary::Start { client, .. } => client.clone(),
            other => panic!("{other:?}"),
        };
        let mut p = Parser::default();
        p.feed(&json!({"type":"session_meta","payload":{"id":"s","cwd":"/r","originator":"Codex Desktop"}}));
        let start =
            p.feed(&json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}}));
        assert_eq!(client(start), "Codex Desktop");
        let start = Parser::default().feed(&json!({"type":"user","uuid":"u","sessionId":"s","cwd":"/r","entrypoint":"cli","message":{"content":"hi"}}));
        assert_eq!(client(start), "cli");
        let review = |client: &str| TaskReview {
            id: "t".into(),
            agent: "Codex".into(),
            label: String::new(),
            root: PathBuf::new(),
            session: "s".into(),
            client: client.into(),
            started: 0,
            before: None,
            after: None,
            changes: vec![],
            active: true,
            warning: String::new(),
        };
        for client in ["codex-tui", "codex_cli_rs", "codex_exec", "cli", ""] {
            assert!(review(client).from_terminal(), "{client}");
        }
        for client in ["Codex Desktop", "codex_vscode", "claude-vscode"] {
            assert!(!review(client).from_terminal(), "{client}");
        }
    }
}
