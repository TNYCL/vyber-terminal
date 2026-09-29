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
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
    }
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
    #[cfg(not(windows))]
    let mut command = std::process::Command::new("open");
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
    #[cfg(not(windows))]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
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
        Err(std::io::Error::other("VS Code launcher `code` was not found on PATH"))
    }
}
