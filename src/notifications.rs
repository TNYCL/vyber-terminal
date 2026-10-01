use std::sync::mpsc;
pub struct Notifications {
    pub receiver: mpsc::Receiver<usize>,
    sender: mpsc::Sender<usize>,
    last: std::time::Instant,
}
impl Notifications {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            receiver,
            sender,
            last: std::time::Instant::now() - std::time::Duration::from_secs(10),
        }
    }
    pub fn show(&mut self, pane: usize, message: String) {
        if self.last.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        self.last = std::time::Instant::now();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            if let Err(e) = show(pane, &message, sender) {
                log::warn!("Notification: {e}");
            }
        });
    }
}
#[cfg(windows)]
fn show(pane: usize, message: &str, sender: mpsc::Sender<usize>) -> anyhow::Result<()> {
    use windows::{
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
        core::HSTRING,
    };
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        let _ = crate::workspace::command("reg.exe")
            .args([
                "add",
                r"HKCU\Software\Classes\AppUserModelId\dev.vyber.terminal",
                "/v",
                "DisplayName",
                "/t",
                "REG_SZ",
                "/d",
                "Vyber",
                "/f",
            ])
            .output();
    });
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(format!("<toast><visual><binding template='ToastGeneric'><text>Vyber · Terminal {}</text><text>{}</text></binding></visual></toast>",pane+1,escape(message))))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    toast.Activated(&TypedEventHandler::new(move |_, _| {
        let _ = sender.send(pane);
        Ok(())
    }))?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from("dev.vyber.terminal"))?;
    notifier.Show(&toast)?;
    // Keep the COM event handler alive while the toast is actionable.
    std::thread::sleep(std::time::Duration::from_secs(90));
    Ok(())
}
#[cfg(target_os = "macos")]
fn show(_: usize, message: &str, _: mpsc::Sender<usize>) -> anyhow::Result<()> {
    // The bundle must have notification permission on macOS.
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    crate::workspace::command("osascript")
        .args([
            "-e",
            &format!("display notification \"{escaped}\" with title \"Vyber\""),
        ])
        .spawn()?;
    Ok(())
}
#[cfg(target_os = "linux")]
fn show(_: usize, message: &str, _: mpsc::Sender<usize>) -> anyhow::Result<()> {
    notify_rust::Notification::new()
        .summary("Vyber")
        .body(message)
        .appname("Vyber")
        .icon("dev.vyber.terminal")
        .show()?;
    Ok(())
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn show(_: usize, _: &str, _: mpsc::Sender<usize>) -> anyhow::Result<()> {
    Ok(())
}
