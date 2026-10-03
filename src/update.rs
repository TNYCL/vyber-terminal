//! Updates from GitHub releases.
//!
//! Packages built by CI look for a newer published release a little after
//! launch and every few hours, reading the release manifest the Release
//! workflow attaches. A newer version is downloaded, checked against the
//! manifest's size and SHA-256 and unpacked in the background; the title bar
//! then shows Update, which replaces this copy of Vyber and restarts it.
//! Drafts and prereleases are never offered. Local builds never update unless
//! `VYBER_UPDATE_URL` points them at a manifest, which is how this is tested.
use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, PoisonError, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const REPOSITORY: &str = "TNYCL/vyber-terminal";
const MANIFEST: &str =
    "https://github.com/TNYCL/vyber-terminal/releases/latest/download/release-manifest.json";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TARGET: &str = env!("VYBER_TARGET");
/// Set by CI for the packages it builds.
const OFFICIAL: bool = option_env!("VYBER_OFFICIAL_BUILD").is_some();
const FIRST_CHECK: Duration = Duration::from_secs(30);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);
const MAX_PACKAGE: u64 = 256 * 1024 * 1024;

/// A newer version, downloaded and unpacked, waiting to be installed.
#[derive(Clone, Debug, PartialEq)]
pub struct Ready {
    pub version: String,
    /// The release page, for when this copy cannot replace itself.
    pub page: String,
    /// The unpacked package: the folder holding `Vyber.exe`, `bin/vyber` or `Vyber.app`.
    package: PathBuf,
}

/// What a check the user asked for found, shown once.
pub struct Message {
    pub text: String,
    /// Good news clears itself; progress and failures stay until replaced
    /// or dismissed.
    pub brief: bool,
}

fn say(text: String, brief: bool) {
    shared().message = Some(Message { text, brief });
}

struct Shared {
    ready: Option<Ready>,
    message: Option<Message>,
}
static SHARED: Mutex<Shared> = Mutex::new(Shared {
    ready: None,
    message: None,
});
static REQUESTS: OnceLock<mpsc::Sender<()>> = OnceLock::new();
/// The version this process replaced, when it was started by an update.
static RELAUNCHED: Mutex<Option<String>> = Mutex::new(None);

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn ready() -> Option<Ready> {
    shared().ready.clone()
}

pub fn take_message() -> Option<Message> {
    shared().message.take()
}

/// The version an update replaced, once, for the first window after it.
pub fn take_relaunched() -> Option<String> {
    RELAUNCHED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

/// Where release manifests come from, or `None` for a build that doesn't update.
fn source() -> Option<String> {
    match std::env::var("VYBER_UPDATE_URL") {
        Ok(url) if !url.is_empty() => match test_source(&url) {
            Ok(()) => Some(url),
            Err(error) => {
                log::warn!("VYBER_UPDATE_URL: {error}");
                None
            }
        },
        _ => OFFICIAL.then(|| MANIFEST.to_string()),
    }
}

/// A test manifest comes over HTTPS or from this machine.
fn test_source(url: &str) -> Result<()> {
    let url = url::Url::parse(url)?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "only HTTPS or a local HTTP server"
    );
    Ok(())
}

/// Starts looking for updates in the background, in builds that update.
pub fn start() {
    let Some(url) = source() else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    if REQUESTS.set(sender).is_err() {
        return;
    }
    let first = if url == MANIFEST {
        FIRST_CHECK
    } else {
        Duration::from_secs(3)
    };
    let spawned = std::thread::Builder::new()
        .name("vyber-update".into())
        .spawn(move || run(url, first, receiver));
    if let Err(error) = spawned {
        log::warn!("Update checks: {error}");
    }
}

/// Looks for an update now, for the menu item.
pub fn check_now() -> Result<(), &'static str> {
    let sender = REQUESTS
        .get()
        .ok_or("This build of Vyber doesn't update itself.")?;
    sender
        .send(())
        .map_err(|_| "Update checks stopped. Restart Vyber to check again.")
}

fn automatic_checks() -> bool {
    crate::config::Config::read().is_none_or(|config| config.check_for_updates)
}

fn run(url: String, first: Duration, requests: mpsc::Receiver<()>) {
    let directory = updates_dir();
    cleanup(&directory);
    let agent = agent();
    let mut wait = first;
    loop {
        let manual = match requests.recv_timeout(wait) {
            Ok(()) => true,
            Err(mpsc::RecvTimeoutError::Timeout) => false,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        // Extra clicks while a check runs are answered by that check.
        while requests.try_recv().is_ok() {}
        if !manual && !automatic_checks() {
            wait = INTERVAL;
            continue;
        }
        let result = check(&agent, &url, &directory, manual);
        wait = if result.is_ok() { INTERVAL } else { RETRY };
        match result {
            Ok(Some(ready)) => {
                if manual {
                    say(
                        format!(
                            "Vyber {} is ready. Click Update to restart into it.",
                            ready.version
                        ),
                        true,
                    );
                }
                shared().ready = Some(ready);
            }
            Ok(None) => {
                if manual {
                    say(format!("Vyber {VERSION} is up to date."), true);
                }
                shared().ready = None;
            }
            Err(error) => {
                log::warn!("Update check: {error:#}");
                if manual {
                    say(format!("Couldn't check for updates: {error:#}"), false);
                }
            }
        }
    }
}

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .user_agent(format!("Vyber/{VERSION} ({TARGET})"))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
        .build()
        .new_agent()
}

#[derive(Debug, Deserialize)]
struct Manifest {
    repository: String,
    tag: String,
    packages: Vec<Package>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Package {
    file: String,
    target: String,
    version: String,
    sha256: String,
    bytes: u64,
}

/// The latest release's package for this build, downloaded and unpacked, or
/// `None` when this build is the latest.
fn check(agent: &ureq::Agent, url: &str, directory: &Path, manual: bool) -> Result<Option<Ready>> {
    let text = agent
        .get(url)
        .call()
        .context("the release manifest could not be fetched")?
        .into_body()
        .into_with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .context("the release manifest could not be read")?;
    let manifest: Manifest =
        serde_json::from_str(&text).context("the release manifest could not be read")?;
    let Some(package) = newer(&manifest, VERSION, TARGET)? else {
        return Ok(None);
    };
    let folder = directory.join(&package.version);
    let ready = Ready {
        version: package.version.clone(),
        page: format!(
            "https://github.com/{REPOSITORY}/releases/tag/{}",
            manifest.tag
        ),
        package: folder.join("package"),
    };
    let marker = folder.join("ready");
    if fs::read_to_string(&marker).is_ok_and(|sha| sha == package.sha256)
        && unpacked(&ready.package).is_some()
    {
        return Ok(Some(ready));
    }
    if manual {
        say(format!("Downloading Vyber {}…", package.version), false);
    }
    fs::create_dir_all(&folder)?;
    let archive = folder.join(&package.file);
    let address = package_url(url, &manifest.tag, &package.file)?;
    download(agent, &address, &package, &archive)?;
    unpack(&archive, &ready.package)?;
    fs::write(&marker, &package.sha256)?;
    let _ = fs::remove_file(&archive);
    // An older download that was never installed is no longer needed.
    for entry in fs::read_dir(directory)?.flatten() {
        if entry.path() != folder && entry.path().is_dir() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(Some(ready))
}

/// The package to install from `manifest`, if it holds a newer version.
fn newer(manifest: &Manifest, current: &str, target: &str) -> Result<Option<Package>> {
    ensure!(
        manifest.repository == REPOSITORY,
        "the manifest is for {}",
        manifest.repository
    );
    let tag = manifest
        .tag
        .strip_prefix('v')
        .with_context(|| format!("unexpected release tag {}", manifest.tag))?;
    let latest = Version::parse(tag).with_context(|| format!("unexpected release tag {tag}"))?;
    if latest <= Version::parse(current)? {
        return Ok(None);
    }
    let package = manifest
        .packages
        .iter()
        .find(|p| p.target == target)
        .with_context(|| format!("Vyber {latest} has no package for {target}"))?;
    ensure!(
        package.version == tag,
        "the {target} package is version {}",
        package.version
    );
    let extension = if target.contains("windows") {
        ".zip"
    } else if target.contains("apple") {
        ".dmg"
    } else {
        ".tar.gz"
    };
    ensure!(
        package.file.starts_with("vyber-")
            && package.file.ends_with(extension)
            && package
                .file
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')),
        "unexpected package name {}",
        package.file
    );
    ensure!(
        package.sha256.len() == 64 && package.sha256.chars().all(|c| c.is_ascii_hexdigit()),
        "the manifest has no SHA-256 for {}",
        package.file
    );
    ensure!(
        package.bytes > 0 && package.bytes <= MAX_PACKAGE,
        "unexpected package size {}",
        package.bytes
    );
    Ok(Some(Package {
        sha256: package.sha256.to_ascii_lowercase(),
        ..package.clone()
    }))
}

/// Packages of the published release are fetched by tag, so a release
/// published meanwhile can't mix with this one; a test manifest's packages sit
/// beside it.
fn package_url(manifest_url: &str, tag: &str, file: &str) -> Result<String> {
    if manifest_url == MANIFEST {
        Ok(format!(
            "https://github.com/{REPOSITORY}/releases/download/{tag}/{file}"
        ))
    } else {
        Ok(url::Url::parse(manifest_url)?.join(file)?.to_string())
    }
}

fn download(agent: &ureq::Agent, url: &str, package: &Package, path: &Path) -> Result<()> {
    if path.is_file() && sha256(path).is_ok_and(|sha| sha == package.sha256) {
        return Ok(());
    }
    let partial = path.with_file_name(format!("{}.part", package.file));
    let result = (|| {
        let mut reader = agent
            .get(url)
            .call()
            .with_context(|| format!("{} could not be downloaded", package.file))?
            .into_body()
            .into_with_config()
            .limit(package.bytes + 1)
            .reader();
        let mut file = fs::File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 64 * 1024];
        let mut total = 0;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            total += read as u64;
            ensure!(
                total <= package.bytes,
                "{} is larger than its manifest says",
                package.file
            );
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])?;
        }
        file.sync_all()?;
        ensure!(total == package.bytes, "{} was cut short", package.file);
        ensure!(
            hex(&hasher.finalize()) == package.sha256,
            "{} doesn't match its SHA-256",
            package.file
        );
        Ok(())
    })();
    match result {
        Ok(()) => Ok(fs::rename(&partial, path)?),
        Err(error) => {
            let _ = fs::remove_file(&partial);
            Err(error)
        }
    }
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn updates_dir() -> PathBuf {
    crate::workspace::data_dir().join("updates")
}

/// Removes downloads that aren't newer than this version, left by an update
/// that has been installed or a release that was replaced.
fn cleanup(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let current = Version::parse(VERSION).ok();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let newer = Version::parse(&name)
            .ok()
            .zip(current.as_ref())
            .is_some_and(|(version, current)| version > *current);
        if entry.path().is_dir() && !newer {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The executable inside an unpacked package.
fn unpacked(package: &Path) -> Option<PathBuf> {
    let executable = if cfg!(windows) {
        package.join("Vyber.exe")
    } else if cfg!(target_os = "macos") {
        package.join("Vyber.app/Contents/MacOS/vyber")
    } else {
        package.join("bin/vyber")
    };
    executable.is_file().then_some(executable)
}

#[cfg(windows)]
fn unpack(archive: &Path, package: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(package);
    zip::ZipArchive::new(fs::File::open(archive)?)?.extract(package)?;
    unpacked(package).context("the package holds no Vyber.exe")?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn unpack(archive: &Path, package: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let staging = package.with_file_name("unpacked");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?)).unpack(&staging)?;
    // The archive holds one folder named after the package.
    let mut folders = fs::read_dir(&staging)?.flatten().map(|e| e.path());
    let root = folders.next().context("the package is empty")?;
    ensure!(
        folders.next().is_none(),
        "the package holds more than one folder"
    );
    fs::rename(&root, package)?;
    let _ = fs::remove_dir_all(&staging);
    let executable = unpacked(package).context("the package holds no bin/vyber")?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn unpack(archive: &Path, package: &Path) -> Result<()> {
    use std::process::Command;
    let _ = fs::remove_dir_all(package);
    fs::create_dir_all(package)?;
    let mount = tempfile::Builder::new().prefix("vyber-update-").tempdir()?;
    tool(
        Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(mount.path())
            .arg(archive),
    )?;
    let copied = tool(
        Command::new("ditto")
            .arg(mount.path().join("Vyber.app"))
            .arg(package.join("Vyber.app")),
    );
    let _ = tool(
        Command::new("hdiutil")
            .arg("detach")
            .arg(mount.path())
            .arg("-force"),
    );
    copied?;
    tool(
        Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(package.join("Vyber.app")),
    )?;
    unpacked(package).context("the package holds no Vyber.app")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn tool(command: &mut std::process::Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("{program} could not be started"))?;
    ensure!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// What an update replaces: the executable, or on macOS the app bundle.
fn installed() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    if cfg!(target_os = "macos") {
        let bundle = executable
            .ancestors()
            .nth(3)
            .filter(|b| b.extension().is_some_and(|e| e == "app"))
            .context("Vyber isn't running from an app bundle")?;
        ensure!(
            !bundle.to_string_lossy().contains("/AppTranslocation/"),
            "macOS runs this copy of Vyber from a temporary location; move it to Applications"
        );
        Ok(bundle.to_path_buf())
    } else {
        Ok(executable)
    }
}

/// Whether this copy of Vyber can replace itself, before asking to restart.
pub fn installable() -> Result<()> {
    let current = installed()?;
    let folder = current.parent().context("Vyber has no folder")?;
    tempfile::Builder::new()
        .prefix(".vyber-write-check")
        .tempfile_in(folder)
        .with_context(|| format!("Vyber can't write to {}", folder.display()))?;
    Ok(())
}

/// Replaces this copy of Vyber with `ready`'s. The running process keeps
/// working from the old files until it quits.
pub fn install(ready: &Ready) -> Result<()> {
    let current = installed()?;
    let folder = current.parent().context("Vyber has no folder")?;
    let package = &ready.package;
    #[cfg(windows)]
    {
        replace_running(&package.join("Vyber.exe"), &current)?;
        refresh_package_files(package, folder, Path::new("Vyber.exe"));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let next = beside(&current, "new");
        fs::copy(package.join("bin/vyber"), &next)?;
        fs::set_permissions(&next, fs::Permissions::from_mode(0o755))?;
        if let Err(error) = fs::rename(&next, &current) {
            let _ = fs::remove_file(&next);
            return Err(error).context("the new vyber could not be put in place");
        }
        // An extracted package keeps its licenses and desktop file beside bin/.
        if folder.file_name().is_some_and(|f| f == "bin")
            && let Some(root) = folder.parent()
        {
            refresh_package_files(package, root, Path::new("bin/vyber"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = folder;
        let next = beside(&current, "new");
        let old = beside(&current, "old");
        let _ = fs::remove_dir_all(&next);
        tool(
            std::process::Command::new("ditto")
                .arg(package.join("Vyber.app"))
                .arg(&next),
        )?;
        let _ = fs::remove_dir_all(&old);
        if let Err(error) = fs::rename(&current, &old) {
            let _ = fs::remove_dir_all(&next);
            return Err(error).context("the running Vyber.app could not be moved aside");
        }
        if let Err(error) = fs::rename(&next, &current) {
            let _ = fs::rename(&old, &current);
            return Err(error).context("the new Vyber.app could not be put in place");
        }
    }
    Ok(())
}

/// Where an update copies the new version (`new`) and keeps the one it
/// replaced (`old`) beside `current`; hidden outside Windows.
fn beside(current: &Path, suffix: &str) -> PathBuf {
    let name = current
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if cfg!(windows) {
        current.with_file_name(format!("{name}.{suffix}"))
    } else {
        current.with_file_name(format!(".{name}.{suffix}"))
    }
}

/// Puts `new` in place of the executable at `current`, keeping the old one
/// beside it: Windows lets a running executable be renamed but not replaced.
/// The old file is removed when the new version starts.
#[cfg(windows)]
fn replace_running(new: &Path, current: &Path) -> Result<()> {
    let next = beside(current, "new");
    let old = beside(current, "old");
    if let Err(error) = fs::copy(new, &next) {
        let _ = fs::remove_file(&next);
        return Err(error).context("the new Vyber.exe could not be copied");
    }
    let _ = fs::remove_file(&old);
    if let Err(error) = fs::rename(current, &old) {
        let _ = fs::remove_file(&next);
        return Err(error).context("the running Vyber.exe could not be moved aside");
    }
    if let Err(error) = fs::rename(&next, current) {
        let _ = fs::rename(&old, current);
        return Err(error).context("the new Vyber.exe could not be put in place");
    }
    Ok(())
}

/// Copies the licenses and notices of a package next to an executable that
/// was installed from one, so they describe the new build.
#[cfg(not(target_os = "macos"))]
fn refresh_package_files(package: &Path, root: &Path, executable: &Path) {
    if !root.join("LICENSE-MIT").is_file() {
        return;
    }
    let mut pending = vec![package.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(package) else {
                continue;
            };
            if path.is_dir() {
                pending.push(path);
            } else if relative != executable {
                let target = root.join(relative);
                let copied = target
                    .parent()
                    .map_or(Ok(()), fs::create_dir_all)
                    .and_then(|()| fs::copy(&path, &target));
                if let Err(error) = copied {
                    log::warn!("Update {}: {error}", target.display());
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Relaunch {
    from: String,
    pid: u32,
    at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn relaunch_path() -> PathBuf {
    updates_dir().join("relaunch.json")
}

/// Starts the installed version once this process has quit.
pub fn relaunch() -> Result<()> {
    let marker = Relaunch {
        from: VERSION.into(),
        pid: std::process::id(),
        at: now(),
    };
    fs::create_dir_all(updates_dir())?;
    fs::write(relaunch_path(), serde_json::to_vec(&marker)?)?;
    let current = installed()?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        // The new process waits for this one to quit before it starts.
        std::process::Command::new(&current)
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("the new Vyber could not be started")?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new("/bin/sh");
        command.args([
            "-c",
            r#"while kill -0 "$1" 2>/dev/null; do sleep 0.1; done; shift; exec "$@""#,
            "vyber-relaunch",
            &marker.pid.to_string(),
        ]);
        if cfg!(target_os = "macos") {
            command.arg("/usr/bin/open").arg(&current);
        } else {
            command.arg(&current);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .context("the new Vyber could not be started")?;
    }
    Ok(())
}

/// Finishes an update at startup: waits for the version it replaced to quit
/// and removes what that version left behind.
pub fn startup() {
    if let Ok(text) = fs::read(relaunch_path()) {
        let _ = fs::remove_file(relaunch_path());
        if let Ok(marker) = serde_json::from_slice::<Relaunch>(&text)
            && now().saturating_sub(marker.at) < 120
            && marker.pid != std::process::id()
        {
            #[cfg(windows)]
            wait_for_exit(marker.pid);
            *RELAUNCHED.lock().unwrap_or_else(PoisonError::into_inner) = Some(marker.from);
        }
    }
    if let Ok(current) = installed() {
        for leftover in [beside(&current, "old"), beside(&current, "new")] {
            if leftover.is_dir() {
                let _ = fs::remove_dir_all(&leftover);
            } else if leftover.exists() {
                let _ = fs::remove_file(&leftover);
            }
        }
    }
}

#[cfg(windows)]
fn wait_for_exit(pid: u32) {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: the handle is only waited on and closed here.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(handle, 15_000);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(tag: &str, packages: Vec<Package>) -> Manifest {
        Manifest {
            repository: REPOSITORY.into(),
            tag: tag.into(),
            packages,
        }
    }

    fn package(version: &str, target: &str, file: &str) -> Package {
        Package {
            file: file.into(),
            target: target.into(),
            version: version.into(),
            sha256: "AB".repeat(32),
            bytes: 14_096_600,
        }
    }

    fn windows(version: &str) -> Package {
        package(
            version,
            "x86_64-pc-windows-msvc",
            &format!("vyber-{version}-windows-x86_64.zip"),
        )
    }

    #[test]
    fn only_a_newer_release_for_this_target_is_offered() {
        let target = "x86_64-pc-windows-msvc";
        let latest = manifest(
            "v0.2.0",
            vec![
                package(
                    "0.2.0",
                    "aarch64-apple-darwin",
                    "vyber-0.2.0-macos-aarch64.dmg",
                ),
                windows("0.2.0"),
            ],
        );
        let offered = newer(&latest, "0.1.0", target).unwrap().unwrap();
        assert_eq!(offered.file, "vyber-0.2.0-windows-x86_64.zip");
        assert_eq!(offered.sha256, "ab".repeat(32));
        assert_eq!(newer(&latest, "0.2.0", target).unwrap(), None);
        assert_eq!(newer(&latest, "0.3.0-alpha.1", target).unwrap(), None);
        // A prerelease build moves on to its release.
        assert!(newer(&latest, "0.2.0-alpha.1", target).unwrap().is_some());
        assert!(newer(&latest, "0.1.0", "riscv64gc-unknown-linux-gnu").is_err());
    }

    #[test]
    fn a_manifest_that_does_not_add_up_is_refused() {
        let target = "x86_64-pc-windows-msvc";
        let mut other = manifest("v0.2.0", vec![windows("0.2.0")]);
        other.repository = "someone/else".into();
        assert!(newer(&other, "0.1.0", target).is_err());
        assert!(newer(&manifest("0.2.0", vec![windows("0.2.0")]), "0.1.0", target).is_err());
        assert!(newer(&manifest("v0.2.0", vec![windows("0.1.9")]), "0.1.0", target).is_err());
        let mut escaping = windows("0.2.0");
        escaping.file = "vyber-0.2.0/../../Vyber.exe.zip".into();
        assert!(newer(&manifest("v0.2.0", vec![escaping]), "0.1.0", target).is_err());
        let mut wrong_kind = windows("0.2.0");
        wrong_kind.file = "vyber-0.2.0-windows-x86_64.exe".into();
        assert!(newer(&manifest("v0.2.0", vec![wrong_kind]), "0.1.0", target).is_err());
        let mut unhashed = windows("0.2.0");
        unhashed.sha256 = "not a hash".into();
        assert!(newer(&manifest("v0.2.0", vec![unhashed]), "0.1.0", target).is_err());
        let mut huge = windows("0.2.0");
        huge.bytes = MAX_PACKAGE + 1;
        assert!(newer(&manifest("v0.2.0", vec![huge]), "0.1.0", target).is_err());
    }

    #[test]
    fn the_published_manifest_parses() {
        let text = r#"{
          "repository": "TNYCL/vyber-terminal",
          "tag": "v0.1.0",
          "commit": "cad30e235b7d5747eda49bd45e49998945c1293b",
          "packages": [{
            "file": "vyber-0.1.0-windows-x86_64.zip",
            "target": "x86_64-pc-windows-msvc",
            "version": "0.1.0",
            "commit": "cad30e235b7d5747eda49bd45e49998945c1293b",
            "sha256": "374bbd7c999b24381c8f4414374f80bcc1b2812b4423c7c7a0242e399bf5460a",
            "bytes": 14096600,
            "rust": "rustc 1.98.1 (48a229cea 2026-09-01)",
            "signing": "unsigned"
          }]
        }"#;
        let manifest: Manifest = serde_json::from_str(text).unwrap();
        assert_eq!(
            newer(&manifest, "0.0.9", "x86_64-pc-windows-msvc")
                .unwrap()
                .unwrap()
                .bytes,
            14_096_600
        );
    }

    #[test]
    fn packages_come_from_the_tag_or_beside_a_test_manifest() {
        assert_eq!(
            package_url(MANIFEST, "v0.2.0", "vyber-0.2.0-linux-x86_64.tar.gz").unwrap(),
            "https://github.com/TNYCL/vyber-terminal/releases/download/v0.2.0/vyber-0.2.0-linux-x86_64.tar.gz"
        );
        assert_eq!(
            package_url(
                "http://127.0.0.1:8765/test/release-manifest.json",
                "v0.2.0",
                "vyber-0.2.0-windows-x86_64.zip"
            )
            .unwrap(),
            "http://127.0.0.1:8765/test/vyber-0.2.0-windows-x86_64.zip"
        );
        assert!(test_source("http://127.0.0.1:8765/release-manifest.json").is_ok());
        assert!(test_source("https://example.com/release-manifest.json").is_ok());
        assert!(test_source("http://example.com/release-manifest.json").is_err());
        assert!(test_source("file:///tmp/release-manifest.json").is_err());
    }

    #[test]
    fn cleanup_keeps_only_newer_downloads() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["0.0.1", VERSION, "999.0.0", "junk"] {
            fs::create_dir_all(directory.path().join(name)).unwrap();
        }
        fs::write(directory.path().join("relaunch.json"), "{}").unwrap();
        cleanup(directory.path());
        let mut left = fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        left.sort();
        assert_eq!(left, ["999.0.0", "relaunch.json"]);
    }

    #[test]
    fn file_digests_match_the_manifest_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        fs::write(&path, "abc").unwrap();
        assert_eq!(
            sha256(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_swap_puts_the_old_executable_back() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("Vyber.exe");
        let new = directory.path().join("staged.exe");
        fs::write(&current, "old").unwrap();
        fs::write(&new, "new").unwrap();
        replace_running(&new, &current).unwrap();
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(directory.path().join("Vyber.exe.old")).unwrap(),
            "old"
        );
        // A missing download leaves the installed file alone.
        replace_running(&directory.path().join("missing.exe"), &current).unwrap_err();
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
        assert!(!directory.path().join("Vyber.exe.new").exists());
    }
}
