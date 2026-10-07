//! What runs in each terminal, for the toolbelt: the processes under its
//! shell, which of them is Claude Code or Codex, and what that agent is
//! doing right now.
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
}
impl AgentKind {
    pub fn name(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
        }
    }
    /// The agent a process is, from its executable name and command line.
    /// Node runs both programs when they are installed with npm, and macOS
    /// names Claude Code's native build after its version.
    pub fn of(name: &str, command: &str) -> Option<Self> {
        let by_name = |name: &str| match program_stem(name).as_str() {
            "claude" => Some(AgentKind::Claude),
            "codex" => Some(AgentKind::Codex),
            _ => None,
        };
        by_name(name)
            .or_else(|| by_name(argv0(command)))
            .or_else(|| {
                let interpreter = program_stem(name);
                if !matches!(interpreter.as_str(), "node" | "bun") {
                    return None;
                }
                let command = command.to_lowercase().replace('\\', "/");
                if command.contains("@anthropic-ai/claude-code") {
                    Some(AgentKind::Claude)
                } else if command.contains("@openai/codex") {
                    Some(AgentKind::Codex)
                } else {
                    None
                }
            })
    }
}

/// A program's file name in lowercase, without its folder or `.exe`.
fn program_stem(path: &str) -> String {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .trim_start_matches('-')
        .to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

/// The program a command line starts, quoted or not.
fn argv0(command: &str) -> &str {
    let command = command.trim_start();
    if let Some(quoted) = command.strip_prefix('"') {
        quoted.split('"').next().unwrap_or("")
    } else {
        command.split_whitespace().next().unwrap_or("")
    }
}

/// What follows the program in a command line.
fn arguments(command: &str) -> &str {
    let command = command.trim_start();
    let first = argv0(command);
    command
        .strip_prefix('"')
        .and_then(|s| s.strip_prefix(first))
        .and_then(|s| s.strip_prefix('"'))
        .or_else(|| command.strip_prefix(first))
        .unwrap_or("")
        .trim()
}

/// A command line as the toolbelt shows it: the program by its short name,
/// then its arguments with the home folder written as `~`.
pub fn display_command(name: &str, command: &str, home: Option<&Path>) -> String {
    let command = command.trim();
    if command.is_empty() {
        return program_name(name);
    }
    let first = argv0(command);
    let mut rest = arguments(command).to_owned();
    if let Some(home) = home.map(|h| h.to_string_lossy().into_owned())
        && home.len() > 1
    {
        for form in [home.clone(), home.replace('\\', "/")] {
            rest = rest.replace(&form, "~");
        }
    }
    let program = program_name(first);
    let program = if program.is_empty() {
        program_name(name)
    } else {
        program
    };
    if rest.is_empty() {
        program
    } else {
        format!("{program} {rest}")
    }
}

/// A program's file name without its folder or `.exe`, case kept.
fn program_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.len().checked_sub(4) {
        Some(cut) if name[cut..].eq_ignore_ascii_case(".exe") => name[..cut].to_owned(),
        _ => name.to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
    /// The command line, for processes under a terminal's shell.
    pub command: String,
}

/// One row of a terminal's process tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub pid: u32,
    pub depth: usize,
    pub name: String,
    pub command: String,
    pub children: bool,
    pub agent: Option<AgentKind>,
}

/// What a Claude Code session says about itself in
/// `~/.claude/sessions/<pid>.json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeSession {
    pub session: String,
    /// The name the user gave the session; derived names are left out.
    pub name: Option<String>,
    /// `busy`, `idle` or `waiting`.
    pub status: String,
    /// What a waiting session waits for, such as `permission prompt`.
    pub waiting_for: Option<String>,
}

/// The processes and agent sessions at one moment.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    processes: HashMap<u32, Process>,
    children: HashMap<u32, Vec<u32>>,
    /// Claude Code sessions by process id.
    pub claude: HashMap<u32, ClaudeSession>,
}

impl Scan {
    /// Lists the processes, reads the command lines of those under `shells`
    /// and the Claude Code sessions. This blocks; run it off the UI thread.
    pub fn capture(shells: &[u32]) -> Self {
        let mut scan = Self::from_rows(crate::platform::process_rows());
        let ids = shells
            .iter()
            .flat_map(|shell| scan.descendants(*shell))
            .collect::<Vec<_>>();
        for (pid, command) in crate::platform::command_lines(&ids) {
            if let Some(process) = scan.processes.get_mut(&pid) {
                process.command = command;
            }
        }
        scan.claude = claude_sessions();
        scan
    }

    pub fn from_rows(rows: Vec<(u32, u32, String)>) -> Self {
        let mut processes = HashMap::new();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (pid, parent, name) in rows {
            if parent != pid {
                children.entry(parent).or_default().push(pid);
            }
            processes.insert(
                pid,
                Process {
                    pid,
                    parent,
                    name,
                    command: String::new(),
                },
            );
        }
        for list in children.values_mut() {
            list.sort_unstable();
        }
        Self {
            processes,
            children,
            claude: HashMap::new(),
        }
    }

    /// `root` and everything under it, parents before their children.
    /// Windows reuses process ids, so a visited set keeps stale parents
    /// from forming a loop.
    fn descendants(&self, root: u32) -> Vec<u32> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![root];
        while let Some(pid) = stack.pop() {
            if !self.processes.contains_key(&pid) || !seen.insert(pid) {
                continue;
            }
            order.push(pid);
            if let Some(children) = self.children.get(&pid) {
                stack.extend(children.iter().rev());
            }
        }
        order
    }

    /// The process tree under `shell`, the shell first. Children of
    /// `collapsed` processes are left out.
    pub fn jobs(&self, shell: u32, collapsed: &HashSet<u32>) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![(shell, 0)];
        while let Some((pid, depth)) = stack.pop() {
            let pid = self.launched(pid);
            let Some(process) = self.processes.get(&pid) else {
                continue;
            };
            if !seen.insert(pid) || depth > 32 {
                continue;
            }
            let children = self
                .children
                .get(&pid)
                .map(|list| {
                    list.iter()
                        .filter(|child| self.processes.contains_key(child) && !seen.contains(child))
                        .copied()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            jobs.push(Job {
                pid,
                depth,
                name: process.name.clone(),
                command: process.command.clone(),
                children: !children.is_empty(),
                agent: AgentKind::of(&process.name, &process.command),
            });
            if !collapsed.contains(&pid) {
                stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
            }
        }
        jobs
    }

    /// The process that `pid` only launches: Git Bash's `bin\bash.exe`
    /// starts `usr\bin\bash.exe` with the same arguments and waits, so
    /// the tree shows the inner one alone. A fork runs the same file, so a
    /// shell's subshell stays in view.
    fn launched(&self, mut pid: u32) -> u32 {
        for _ in 0..8 {
            let (Some(process), Some([child])) = (
                self.processes.get(&pid),
                self.children.get(&pid).map(Vec::as_slice),
            ) else {
                break;
            };
            match self.processes.get(child) {
                Some(inner)
                    if program_stem(&inner.name) == program_stem(&process.name)
                        && argv0(&inner.command) != argv0(&process.command)
                        && arguments(&inner.command) == arguments(&process.command) =>
                {
                    pid = *child
                }
                _ => break,
            }
        }
        pid
    }

    /// The agent running in the terminal of `shell`: the one nearest the
    /// shell, which may be the shell itself when it was replaced by `exec`.
    pub fn agent(&self, shell: u32) -> Option<(AgentKind, u32)> {
        let mut level = vec![shell];
        let mut seen = HashSet::new();
        for _ in 0..32 {
            let mut found = level
                .iter()
                .filter_map(|pid| {
                    let process = self.processes.get(pid)?;
                    AgentKind::of(&process.name, &process.command).map(|kind| (kind, *pid))
                })
                .collect::<Vec<_>>();
            found.sort_by_key(|(_, pid)| *pid);
            if let Some(agent) = found.into_iter().next() {
                return Some(agent);
            }
            level = level
                .iter()
                .filter(|pid| seen.insert(**pid))
                .flat_map(|pid| self.children.get(pid).into_iter().flatten().copied())
                .collect();
            if level.is_empty() {
                break;
            }
        }
        None
    }
}

/// The Claude Code sessions running on this computer, by process id.
fn claude_sessions() -> HashMap<u32, ClaudeSession> {
    let Some(dir) = dirs::home_dir().map(|home| home.join(".claude").join("sessions")) else {
        return HashMap::new();
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "json"))
        .filter_map(|entry| parse_claude_session(&std::fs::read(entry.path()).ok()?))
        .collect()
}

pub fn parse_claude_session(bytes: &[u8]) -> Option<(u32, ClaudeSession)> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let pid = u32::try_from(value["pid"].as_u64()?).ok()?;
    let named = matches!(value["nameSource"].as_str(), None | Some("user" | "peer"));
    Some((
        pid,
        ClaudeSession {
            session: value["sessionId"].as_str().unwrap_or("").to_owned(),
            name: value["name"]
                .as_str()
                .filter(|name| named && !name.trim().is_empty())
                .map(str::to_owned),
            status: value["status"].as_str().unwrap_or("").to_owned(),
            waiting_for: value["waitingFor"]
                .as_str()
                .filter(|w| !w.is_empty())
                .map(str::to_owned),
        },
    ))
}

/// The question Claude Code or Codex shows while it waits for approval, if
/// the bottom of the screen holds one with its choices.
pub fn approval_prompt(screen: &str) -> bool {
    const QUESTIONS: [&str; 6] = [
        "Do you want to proceed?",
        "Do you want to make this edit",
        "Do you want to create ",
        "Would you like to run the following command?",
        "Would you like to make the following edits?",
        "Would you like to grant",
    ];
    const CHOICES: [&str; 4] = ["1. Yes", "Yes, proceed", "Yes, allow", "Yes, and"];
    QUESTIONS.iter().any(|q| screen.contains(q)) && CHOICES.iter().any(|c| screen.contains(c))
}

/// What the logs and the screen say an agent is doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Working,
    /// Waiting for the user, for the reason given.
    Waiting(Reason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Claude Code says so in its session file.
    Claude,
    /// An approval question is on screen.
    Approval,
    /// The program rang the bell or sent a notification mid-turn.
    Attention,
}

/// Combines the signals about an agent. Claude Code's own status wins when
/// it writes one; otherwise a turn in the logs means it works.
pub fn activity(
    claude: Option<&ClaudeSession>,
    turn_active: bool,
    prompt_on_screen: bool,
    attention: bool,
) -> Activity {
    let status = claude.map_or("", |c| c.status.as_str());
    if status == "waiting" {
        return Activity::Waiting(Reason::Claude);
    }
    let working = match status {
        "busy" => true,
        "idle" => false,
        _ => turn_active,
    };
    if working && prompt_on_screen {
        Activity::Waiting(Reason::Approval)
    } else if working && attention {
        Activity::Waiting(Reason::Attention)
    } else if working {
        Activity::Working
    } else {
        Activity::Idle
    }
}

/// An agent's state as the toolbelt and tabs show it. The order ranks how
/// much a state asks for the user: a tab shows its neediest terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentState {
    Idle,
    /// It finished while the user looked elsewhere.
    Responded,
    Working,
    Waiting,
}

/// Follows one terminal's agent over time, to tell a reply the user has not
/// seen from one they have.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    pid: u32,
    last: Option<Activity>,
    worked: bool,
    since: Option<Instant>,
    finished: Option<Instant>,
    seen: Option<Instant>,
}

impl Tracker {
    /// Folds in what the agent `pid` does now. `looking` says the user has
    /// its terminal in front of them.
    pub fn update(
        &mut self,
        pid: u32,
        activity: Activity,
        looking: bool,
        now: Instant,
    ) -> AgentState {
        if self.pid != pid {
            *self = Self {
                pid,
                ..Self::default()
            };
        }
        if self.last != Some(activity) {
            self.last = Some(activity);
            self.since = Some(now);
        }
        match activity {
            Activity::Working | Activity::Waiting(_) => self.worked = true,
            Activity::Idle => {
                if std::mem::take(&mut self.worked) {
                    self.finished = Some(now);
                }
            }
        }
        if looking {
            self.seen = Some(now);
        }
        match activity {
            Activity::Working => AgentState::Working,
            Activity::Waiting(_) => AgentState::Waiting,
            Activity::Idle
                if self
                    .finished
                    .is_some_and(|f| self.seen.is_none_or(|seen| seen < f)) =>
            {
                AgentState::Responded
            }
            Activity::Idle => AgentState::Idle,
        }
    }

    /// When the agent started doing what it does now.
    pub fn since(&self) -> Option<Instant> {
        self.since
    }

    /// When the agent last finished working.
    pub fn finished(&self) -> Option<Instant> {
        self.finished
    }
}

/// A short "how long ago": `now`, `12s`, `4m`, `3h`, `2d`.
pub fn elapsed(seconds: u64) -> String {
    match seconds {
        0..=4 => "now".into(),
        5..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scan() -> Scan {
        let mut scan = Scan::from_rows(vec![
            (1, 0, "launchd".into()),
            (10, 1, "zsh".into()),
            (11, 10, "2.1.292".into()),
            (12, 11, "node".into()),
            (13, 12, "node".into()),
            (14, 11, "bash".into()),
            (20, 1, "zsh".into()),
            (21, 20, "node".into()),
            (22, 21, "codex".into()),
            (30, 1, "pwsh.exe".into()),
            (31, 30, "claude.exe".into()),
        ]);
        let commands = [
            (
                11,
                "/Users/me/.local/bin/claude --allow-dangerously-skip-permissions",
            ),
            (12, "node /Users/me/.claude/plugins/server.js"),
            (21, "node /usr/lib/node_modules/@openai/codex/bin/codex.js"),
        ];
        for (pid, command) in commands {
            scan.processes.get_mut(&pid).unwrap().command = command.into();
        }
        scan
    }

    #[test]
    fn agents_are_found_by_name_command_or_npm_package() {
        let scan = scan();
        assert_eq!(scan.agent(10), Some((AgentKind::Claude, 11)));
        assert_eq!(scan.agent(20), Some((AgentKind::Codex, 21)));
        assert_eq!(scan.agent(30), Some((AgentKind::Claude, 31)));
        assert_eq!(scan.agent(14), None);
        assert_eq!(scan.agent(99), None);
        // Claude Code's own MCP servers run from ~/.claude but are not Claude.
        assert_eq!(
            AgentKind::of("node", "node /Users/me/.claude/plugins/server.js"),
            None
        );
        assert_eq!(
            AgentKind::of(
                "node.exe",
                r"C:\Program Files\nodejs\node.exe C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js"
            ),
            Some(AgentKind::Claude)
        );
    }

    #[test]
    fn jobs_list_the_tree_in_order_and_skip_collapsed_children() {
        let scan = scan();
        let rows = |collapsed: &[u32]| {
            scan.jobs(10, &collapsed.iter().copied().collect())
                .into_iter()
                .map(|job| (job.pid, job.depth, job.children))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(&[]),
            vec![
                (10, 0, true),
                (11, 1, true),
                (12, 2, true),
                (13, 3, false),
                (14, 2, false)
            ]
        );
        assert_eq!(
            rows(&[12]),
            vec![(10, 0, true), (11, 1, true), (12, 2, true), (14, 2, false)]
        );
        assert_eq!(rows(&[10]), vec![(10, 0, true)]);
        assert!(scan.jobs(99, &HashSet::new()).is_empty());
    }

    #[test]
    fn launchers_that_start_the_same_program_are_left_out() {
        let mut scan = Scan::from_rows(vec![
            (1, 0, "bash.exe".into()),
            (2, 1, "bash.exe".into()),
            (3, 2, "claude.exe".into()),
            (4, 3, "bash.exe".into()),
            (5, 3, "node.exe".into()),
            (6, 4, "bash.exe".into()),
        ]);
        let commands = [
            (1, r#""C:\Git\bin\bash.exe" --login -i"#),
            (2, r#""C:\Git\usr\bin\bash.exe" --login -i"#),
            (4, r"C:\Git\usr\bin\bash.exe -c ls"),
            (6, r"C:\Git\usr\bin\bash.exe -c ls"),
        ];
        for (pid, command) in commands {
            scan.processes.get_mut(&pid).unwrap().command = command.into();
        }
        let rows = scan
            .jobs(1, &HashSet::new())
            .into_iter()
            .map(|job| (job.pid, job.depth))
            .collect::<Vec<_>>();
        // A subshell forked by bash runs the same file and stays.
        assert_eq!(rows, vec![(2, 0), (3, 1), (4, 2), (6, 3), (5, 2)]);
    }

    #[test]
    fn reused_process_ids_never_loop() {
        let scan = Scan::from_rows(vec![(5, 6, "a".into()), (6, 5, "b".into())]);
        assert_eq!(scan.jobs(5, &HashSet::new()).len(), 2);
        assert_eq!(scan.agent(5), None);
        assert_eq!(scan.descendants(6), vec![6, 5]);
    }

    #[test]
    fn commands_show_short_programs_and_home_as_tilde() {
        let home = Path::new("/Users/me");
        assert_eq!(
            display_command(
                "node",
                "/usr/bin/node /Users/me/.claude/x.js --port 1",
                Some(home)
            ),
            "node ~/.claude/x.js --port 1"
        );
        assert_eq!(
            display_command(
                "bash.exe",
                r#""C:\Program Files\Git\usr\bin\bash.exe" --login -i"#,
                None
            ),
            "bash --login -i"
        );
        assert_eq!(display_command("Claude.EXE", "", None), "Claude");
        assert_eq!(display_command("zsh", "-zsh", Some(home)), "-zsh");
        let windows_home = Path::new(r"C:\Users\me");
        assert_eq!(
            display_command(
                "node.exe",
                r"node C:\Users\me\x.js C:/Users/me/y.js",
                Some(windows_home)
            ),
            r"node ~\x.js ~/y.js"
        );
    }

    #[test]
    fn claude_session_files_give_status_and_user_names() {
        let (pid, session) = parse_claude_session(
            br#"{"pid":10556,"sessionId":"s","name":"vyber-terminal-a4","nameSource":"derived","status":"busy"}"#,
        )
        .unwrap();
        assert_eq!(pid, 10556);
        assert_eq!(session.status, "busy");
        assert_eq!(session.name, None);
        let (_, session) = parse_claude_session(
            br#"{"pid":1,"sessionId":"s","name":"veti","nameSource":"user","status":"waiting","waitingFor":"permission prompt"}"#,
        )
        .unwrap();
        assert_eq!(session.name.as_deref(), Some("veti"));
        assert_eq!(session.waiting_for.as_deref(), Some("permission prompt"));
        assert!(parse_claude_session(b"{}").is_none());
        assert!(parse_claude_session(b"not json").is_none());
    }

    #[test]
    fn approval_questions_need_their_choices() {
        assert!(approval_prompt(
            " Bash command\n   cargo test\n Do you want to proceed?\n ❯ 1. Yes\n   2. No"
        ));
        assert!(approval_prompt(
            "Would you like to run the following command?\n  $ rm -rf target\n› 1. Yes, proceed (y)"
        ));
        assert!(!approval_prompt(
            "Do you want to proceed? I can push it next."
        ));
        assert!(!approval_prompt("1. Yes, this works"));
    }

    #[test]
    fn claude_status_wins_over_turns_and_screens() {
        let claude = |status: &str| ClaudeSession {
            status: status.into(),
            ..ClaudeSession::default()
        };
        assert_eq!(
            activity(Some(&claude("waiting")), false, false, false),
            Activity::Waiting(Reason::Claude)
        );
        assert_eq!(
            activity(Some(&claude("idle")), true, true, true),
            Activity::Idle
        );
        assert_eq!(
            activity(Some(&claude("busy")), false, false, false),
            Activity::Working
        );
        assert_eq!(activity(None, true, false, false), Activity::Working);
        assert_eq!(
            activity(None, true, true, false),
            Activity::Waiting(Reason::Approval)
        );
        assert_eq!(
            activity(None, true, false, true),
            Activity::Waiting(Reason::Attention)
        );
        // A bell after the turn ended is not a question.
        assert_eq!(activity(None, false, true, true), Activity::Idle);
    }

    #[test]
    fn a_reply_stays_green_until_its_terminal_is_seen() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut tracker = Tracker::default();
        // An agent found idle has not answered anything yet.
        assert_eq!(
            tracker.update(7, Activity::Idle, false, at(0)),
            AgentState::Idle
        );
        assert_eq!(
            tracker.update(7, Activity::Working, false, at(1)),
            AgentState::Working
        );
        assert_eq!(tracker.since(), Some(at(1)));
        assert_eq!(
            tracker.update(7, Activity::Waiting(Reason::Claude), false, at(2)),
            AgentState::Waiting
        );
        assert_eq!(
            tracker.update(7, Activity::Idle, false, at(3)),
            AgentState::Responded
        );
        assert_eq!(tracker.finished(), Some(at(3)));
        assert_eq!(
            tracker.update(7, Activity::Idle, false, at(4)),
            AgentState::Responded
        );
        assert_eq!(
            tracker.update(7, Activity::Idle, true, at(5)),
            AgentState::Idle
        );
        assert_eq!(
            tracker.update(7, Activity::Idle, false, at(6)),
            AgentState::Idle
        );
        // Finishing in front of the user is seen at once.
        tracker.update(7, Activity::Working, true, at(7));
        assert_eq!(
            tracker.update(7, Activity::Idle, true, at(8)),
            AgentState::Idle
        );
        // Another agent in the terminal starts afresh.
        tracker.update(7, Activity::Working, false, at(9));
        assert_eq!(
            tracker.update(8, Activity::Idle, false, at(10)),
            AgentState::Idle
        );
    }

    #[test]
    fn a_live_scan_reads_command_lines_under_a_shell() {
        // Git waits for input here, so it is still running when listed.
        let mut child = crate::workspace::command("git")
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let me = std::process::id();
        let scan = Scan::capture(&[me]);
        let jobs = scan.jobs(me, &HashSet::new());
        assert_eq!(jobs[0].pid, me);
        let job = jobs.iter().find(|j| j.pid == child.id()).unwrap();
        assert!(job.command.contains("cat-file --batch"), "{job:?}");
        assert_eq!(scan.agent(child.id()), None);
        child.kill().unwrap();
        let _ = child.wait();
    }

    #[test]
    fn elapsed_times_are_short() {
        assert_eq!(elapsed(0), "now");
        assert_eq!(elapsed(12), "12s");
        assert_eq!(elapsed(125), "2m");
        assert_eq!(elapsed(7200), "2h");
        assert_eq!(elapsed(200_000), "2d");
    }
}
