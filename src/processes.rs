use std::{collections::HashSet, path::Path};

#[derive(Clone)]
pub struct TerminalProcess {
    pub pid: Option<u32>,
    pub shell: String,
}

#[derive(Clone)]
struct Process {
    pid: u32,
    parent: u32,
    state: String,
    name: String,
}

#[derive(Clone)]
pub struct ProcessSnapshot {
    processes: Vec<Process>,
}

impl ProcessSnapshot {
    pub fn capture() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            // Komut argümanları hassas bilgi içerebilir; yalnızca işlem adlarını okuruz.
            let output = std::process::Command::new("/bin/ps")
                .args(["-axo", "pid=,ppid=,stat=,comm="])
                .output()?;
            if !output.status.success() {
                return Err(std::io::Error::other("Could not read terminal processes"));
            }
            Self::parse(&String::from_utf8_lossy(&output.stdout))
        }
        #[cfg(not(unix))]
        {
            Err(std::io::Error::other("Process inspection is unavailable"))
        }
    }

    pub(crate) fn parse(text: &str) -> std::io::Result<Self> {
        let mut processes = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let mut fields = line.split_whitespace();
            let pid = fields.next().and_then(|value| value.parse().ok());
            let parent = fields.next().and_then(|value| value.parse().ok());
            let state = fields.next();
            let name = fields.collect::<Vec<_>>().join(" ");
            let (Some(pid), Some(parent), Some(state)) = (pid, parent, state) else {
                return Err(std::io::Error::other("Invalid process information"));
            };
            if name.is_empty() {
                return Err(std::io::Error::other("Missing process name"));
            }
            processes.push(Process {
                pid,
                parent,
                state: state.to_owned(),
                name,
            });
        }
        if processes.is_empty() {
            return Err(std::io::Error::other("Empty process information"));
        }
        Ok(Self { processes })
    }

    pub fn running(&self, terminal: &TerminalProcess) -> Option<Vec<String>> {
        let pid = terminal.pid?;
        let Some(root) = self.processes.iter().find(|process| process.pid == pid) else {
            return Some(Vec::new());
        };
        let mut descendants = HashSet::from([pid]);
        loop {
            let before = descendants.len();
            for process in &self.processes {
                if descendants.contains(&process.parent) {
                    descendants.insert(process.pid);
                }
            }
            if before == descendants.len() {
                break;
            }
        }
        let mut names = self
            .processes
            .iter()
            .filter(|process| {
                process.pid != pid
                    && descendants.contains(&process.pid)
                    && !process.state.starts_with('Z')
            })
            .map(|process| process_name(&process.name).to_owned())
            .collect::<Vec<_>>();
        // Shell exec ile Codex'e dönüşmüşse alt işlem olmadan da onay gerekir.
        let shell = process_name(&terminal.shell);
        if !root.state.starts_with('Z') && (!is_shell(shell) || process_name(&root.name) != shell) {
            names.push(process_name(&root.name).to_owned());
        }
        names.sort();
        names.dedup();
        Some(names)
    }
}

fn process_name(name: &str) -> &str {
    Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(name)
        .trim_start_matches('-')
}

fn is_shell(name: &str) -> bool {
    matches!(
        name,
        "zsh" | "bash" | "sh" | "fish" | "dash" | "ksh" | "csh" | "tcsh" | "nu"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> TerminalProcess {
        TerminalProcess {
            pid: Some(10),
            shell: "/bin/zsh".into(),
        }
    }

    #[test]
    fn idle_shell_does_not_require_confirmation() {
        let snapshot = ProcessSnapshot::parse("10 1 S /bin/zsh\n20 1 S /usr/bin/codex\n").unwrap();
        assert_eq!(snapshot.running(&terminal()), Some(vec![]));
    }

    #[test]
    fn agent_descendants_background_and_stopped_jobs_require_confirmation() {
        let snapshot = ProcessSnapshot::parse(
            "14 13 S /usr/bin/codex\n13 10 S /usr/bin/node\n10 1 S /bin/zsh\n15 10 T /usr/bin/vim\n16 10 S /usr/bin/sleep\n17 10 Z dead\n20 1 S unrelated\n",
        ).unwrap();
        assert_eq!(
            snapshot.running(&terminal()),
            Some(vec![
                "codex".into(),
                "node".into(),
                "sleep".into(),
                "vim".into()
            ])
        );
    }

    #[test]
    fn exec_replacement_and_direct_agent_are_protected() {
        let snapshot = ProcessSnapshot::parse("10 1 S /opt/homebrew/bin/codex\n").unwrap();
        assert_eq!(snapshot.running(&terminal()), Some(vec!["codex".into()]));
        let direct = TerminalProcess {
            shell: "codex".into(),
            ..terminal()
        };
        assert_eq!(snapshot.running(&direct), Some(vec!["codex".into()]));
    }

    #[test]
    fn malformed_information_is_not_treated_as_idle() {
        assert!(ProcessSnapshot::parse("").is_err());
        assert!(ProcessSnapshot::parse("broken output").is_err());
        let snapshot = ProcessSnapshot::parse("10 1 S /bin/zsh\n").unwrap();
        assert!(
            snapshot
                .running(&TerminalProcess {
                    pid: None,
                    ..terminal()
                })
                .is_none()
        );
    }

    #[test]
    fn login_shell_and_names_with_spaces_are_parsed() {
        let snapshot = ProcessSnapshot::parse(
            "10 1 S -zsh\n11 10 S /Applications/My Tool.app/Contents/MacOS/tool\n",
        )
        .unwrap();
        assert_eq!(snapshot.running(&terminal()), Some(vec!["tool".into()]));
    }

    #[test]
    #[cfg(unix)]
    fn live_process_tree_detects_a_command_and_then_its_exit() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let mut child = Command::new("/bin/sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let terminal = TerminalProcess {
            pid: Some(child.id()),
            shell: "/bin/sh".into(),
        };
        assert_eq!(
            ProcessSnapshot::capture().unwrap().running(&terminal),
            Some(vec![])
        );
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"sleep 1\nexit\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_millis(800);
        let detected = loop {
            let names = ProcessSnapshot::capture()
                .unwrap()
                .running(&terminal)
                .unwrap();
            if names.iter().any(|name| name == "sleep") {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        child.wait().unwrap();
        assert!(
            detected,
            "The live shell's running command must be detected"
        );
        assert_eq!(
            ProcessSnapshot::capture().unwrap().running(&terminal),
            Some(vec![])
        );
    }
}
