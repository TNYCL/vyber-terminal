#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
mod app;
mod browser;
mod config;
mod icons;
mod layout;
mod notifications;
mod platform;
mod pty;
mod tasks;
mod terminal;
mod theme;
mod workspace;
use gpui::*;
use gpui_kit::component::TitleBar;
fn main() {
    platform::login_environment();
    let logs = workspace::data_dir();
    let _ = std::fs::create_dir_all(&logs);
    if let Ok(file) = std::fs::File::create(logs.join("vyber.log")) {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
            .target(env_logger::Target::Pipe(Box::new(file)))
            .init();
    } else {
        env_logger::init();
    }
    std::panic::set_hook(Box::new(|info| log::error!("{info}")));
    let root = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| {
            let cwd = std::env::current_dir().unwrap_or_default();
            if cwd.to_string_lossy().contains("WindowsApps") {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            } else {
                cwd
            }
        });
    gpui_kit::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(config::Config::load().reduced_motion);
            theme::apply(cx);
            app::bind_keys(cx);
            terminal::bind_keys(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1380.), px(850.)),
                    cx,
                ))),
                window_min_size: Some(size(px(900.), px(550.))),
                app_id: Some("dev.vyber.terminal".into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                window.set_window_title("Vyber");
                cx.new(|cx| app::Vyber::new(root, window, cx))
            })
            .expect("open Vyber");
            cx.activate(true);
        });
}
