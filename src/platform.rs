use std::path::{Path, PathBuf};

/// Prefer the installed Git for Windows shell; explicit config still takes precedence.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        git_bash_path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "powershell.exe".into())
    }
    #[cfg(not(windows))]
    {
        let candidates = if cfg!(target_os = "macos") {
            &["/bin/zsh", "/bin/bash", "/bin/sh"][..]
        } else {
            &["/bin/bash", "/usr/bin/bash", "/bin/sh"][..]
        };
        unix_shell(std::env::var("SHELL").ok().as_deref(), candidates)
    }
}

#[cfg(unix)]
fn unix_shell(shell: Option<&str>, candidates: &[&str]) -> String {
    use std::os::unix::fs::PermissionsExt;
    shell
        .into_iter()
        .chain(candidates.iter().copied())
        .find(|path| {
            std::fs::metadata(path)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .unwrap_or("/bin/sh")
        .to_owned()
}

#[cfg(windows)]
pub fn git_bash_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path) {
            if entry.join("git.exe").is_file() {
                roots.extend(entry.ancestors().take(4).map(std::path::Path::to_owned));
            }
        }
    }
    for name in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(path) = std::env::var_os(name) {
            roots.push(PathBuf::from(path).join("Git"));
        }
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(path).join("Programs/Git"));
    }
    roots.into_iter().find_map(|root| {
        let bash = root.join("bin/bash.exe");
        (root.join("cmd/git.exe").is_file() && bash.is_file()).then_some(bash)
    })
}

/// Dock launches do not inherit the user's login shell PATH.
#[cfg(target_os = "macos")]
pub fn login_environment() {
    use std::{
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(mut child) = std::process::Command::new(shell)
        .args(["-l", "-c", "printf '\\0%s\\0' \"$PATH\""])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = stdout.take(1024 * 1024).read_to_end(&mut bytes);
        bytes
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(4) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    if let Ok(bytes) = reader.join() {
        if let Some(path) = bytes
            .split(|b| *b == 0)
            .nth(1)
            .and_then(|b| std::str::from_utf8(b).ok())
        {
            if !path.is_empty() {
                // Called once before the application starts any threads.
                unsafe {
                    std::env::set_var("PATH", path);
                }
            }
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub fn login_environment() {}

/// Opens a file with the operating system's default application.
pub fn open_default(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = crate::workspace::command("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

/// Shows a file selected in File Explorer or Finder.
pub fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        crate::workspace::command("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        // XDG has no portable select-file operation; open its containing folder.
        open_default(if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        })
    }
}

/// Opens a file in VS Code through its `code` launcher.
pub fn open_in_code(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = {
        let mut command = crate::workspace::command("cmd");
        command.args(["/D", "/C", "code"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = std::process::Command::new("code");
    let status = command
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "VS Code launcher `code` was not found on PATH",
        ))
    }
}

pub fn startup_root(directory: Option<PathBuf>) -> PathBuf {
    directory
        .filter(|path| path.is_dir())
        .map(|path| path.canonicalize().unwrap_or(path))
        .or_else(|| dirs::home_dir().filter(|path| path.is_dir()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub fn folder_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            let name = path.display().to_string();
            if name.is_empty() {
                "Terminal".into()
            } else {
                name
            }
        })
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn missing_or_non_executable_shell_uses_a_working_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("not-a-shell");
        std::fs::write(&file, "").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(unix_shell(Some("/missing/shell"), &["/bin/sh"]), "/bin/sh");
        assert_eq!(unix_shell(file.to_str(), &["/bin/sh"]), "/bin/sh");
        assert_eq!(unix_shell(Some("/bin/sh"), &["/missing"]), "/bin/sh");
    }

    #[test]
    fn startup_uses_home_and_respects_an_explicit_folder() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            startup_root(Some(directory.path().to_owned())),
            directory.path().canonicalize().unwrap()
        );
        assert_eq!(
            startup_root(Some(PathBuf::from("."))),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
        if let Some(home) = dirs::home_dir().filter(|path| path.is_dir()) {
            assert_eq!(startup_root(None), home);
            assert_eq!(startup_root(Some(directory.path().join("missing"))), home);
        }
    }

    #[test]
    fn folder_names_include_unicode_and_a_root_fallback() {
        assert_eq!(folder_name(Path::new("/Users/tnycl")), "tnycl");
        assert_eq!(
            folder_name(Path::new("/Users/tnycl/Çalışma alanı")),
            "Çalışma alanı"
        );
        assert_eq!(folder_name(Path::new("")), "Terminal");
        #[cfg(unix)]
        assert_eq!(folder_name(Path::new("/")), "/");
        #[cfg(windows)]
        assert_eq!(folder_name(Path::new("C:\\")), "C:\\");
    }
}

/// The running processes, each with its parent and executable name, for
/// telling which terminal an agent runs in.
#[derive(Default)]
pub struct Processes(std::collections::HashMap<u32, (u32, String)>);

impl Processes {
    pub fn list() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::{
                Foundation::CloseHandle,
                System::Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                    TH32CS_SNAPPROCESS,
                },
            };
            let mut list = std::collections::HashMap::new();
            // Safety: the snapshot handle is closed below, and each entry
            // carries its size as Process32FirstW/NextW require.
            unsafe {
                let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                    return Self::default();
                };
                let mut entry = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };
                let mut next = Process32FirstW(snapshot, &mut entry);
                while next.is_ok() {
                    let name = &entry.szExeFile;
                    let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
                    list.insert(
                        entry.th32ProcessID,
                        (
                            entry.th32ParentProcessID,
                            String::from_utf16_lossy(&name[..len]),
                        ),
                    );
                    next = Process32NextW(snapshot, &mut entry);
                }
                let _ = CloseHandle(snapshot);
            }
            Self(list)
        }
        #[cfg(not(windows))]
        {
            let Ok(out) = std::process::Command::new("ps")
                .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "comm="])
                .output()
            else {
                return Self::default();
            };
            Self(
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter_map(|line| {
                        let mut fields = line.split_whitespace();
                        let id = fields.next()?.parse().ok()?;
                        let parent = fields.next()?.parse().ok()?;
                        let command = fields.collect::<Vec<_>>().join(" ");
                        let name = std::path::Path::new(&command).file_name()?;
                        Some((id, (parent, name.to_string_lossy().into_owned())))
                    })
                    .collect(),
            )
        }
    }

    /// Whether process `id` runs under `ancestor`, through its parents.
    pub fn descends(&self, mut id: u32, ancestor: u32) -> bool {
        for _ in 0..64 {
            if id == ancestor {
                return true;
            }
            match self.0.get(&id) {
                Some(&(parent, _)) if parent != id && parent != 0 => id = parent,
                _ => return false,
            }
        }
        false
    }

    /// Whether a program whose name starts with `name`, in lowercase, runs
    /// under `ancestor`.
    pub fn runs(&self, ancestor: u32, name: &str) -> bool {
        self.0.iter().any(|(id, (_, program))| {
            *id != ancestor
                && program.to_lowercase().starts_with(name)
                && self.descends(*id, ancestor)
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn processes_know_what_runs_under_them() {
        // Git waits for input here, so it is still running when listed.
        let mut child = crate::workspace::command("git")
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let processes = super::Processes::list();
        let me = std::process::id();
        assert!(processes.descends(child.id(), me));
        assert!(!processes.descends(me, child.id()));
        assert!(processes.runs(me, "git"));
        assert!(!processes.runs(child.id(), "git"));
        child.kill().unwrap();
        let _ = child.wait();
    }
}
