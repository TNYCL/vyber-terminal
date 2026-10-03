#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
mod app;
mod browser;
mod changeset;
mod cli;
mod closing;
mod config;
mod git;
mod glyphs;
mod icons;
mod layout;
mod lifecycle;
mod notifications;
mod panel;
mod platform;
mod processes;
mod project;
mod project_dialog;
mod pty;
#[cfg(unix)]
mod shell;
mod split;
mod tab_state;
mod tasks;
mod terminal;
mod theme;
mod update;
mod workspace;
use gpui::*;
fn main() {
    let directory = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Command::Version) => {
            println!("vyber {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Ok(cli::Command::Help) => {
            println!(
                "Vyber {}\n\nUsage: vyber [DIRECTORY]\n       vyber --version\n       vyber --help\n\nUse -- before a directory whose name starts with a dash.",
                env!("CARGO_PKG_VERSION")
            );
            return;
        }
        Ok(cli::Command::Launch(directory)) => directory,
        Err(error) => {
            eprintln!("vyber: {error}\nTry 'vyber --help'.");
            std::process::exit(2);
        }
    };
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
    update::startup();
    let root = platform::startup_root(directory);
    let application = gpui_kit::application()
        .with_assets(icons::Assets)
        .with_quit_mode(if cfg!(target_os = "macos") {
            QuitMode::Explicit
        } else {
            QuitMode::LastWindowClosed
        });
    application.on_reopen(|cx| {
        if let Err(error) = lifecycle::show_workspace(cx) {
            log::error!("Reopen workspace: {error}");
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        let config = config::Config::load();
        cx.set_reduce_motion(config.reduced_motion);
        cx.set_global(config);
        theme::apply(cx);
        app::bind_keys(cx);
        terminal::bind_keys(cx);
        browser::bind_keys(cx);
        lifecycle::init(root, cx);
        lifecycle::open_workspace(app::WorkspaceLaunch::Restore, cx).expect("open Vyber");
        cx.activate(true);
        update::start();
    });
}
