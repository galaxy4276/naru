//! Naru's window: finds naru-d (starting it when none answers) and shows the screen attached to it.

use std::fs;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

fn state_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Naru"))
}

/// The daemon outlives this window, so it is started in its own process group and never waited on.
fn start_daemon(state: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(state)?;
    let daemon = std::env::current_exe()?.with_file_name("naru-d");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("daemon.log"))?;
    // ponytail: a daemon that exits while this window runs stays a zombie until the window quits.
    Command::new(&daemon)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn()
        .with_context(|| format!("cannot start {}", daemon.display()))?;
    Ok(())
}

fn daemon_url() -> anyhow::Result<String> {
    let state = state_dir()?;
    if UnixStream::connect(state.join("daemon.sock")).is_err() {
        // A daemon that is not answering left these behind; the new one writes them again.
        let _ = fs::remove_file(state.join("daemon.json"));
        start_daemon(&state)?;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // The daemon writes its token before daemon.json, so a daemon.json means both are current.
        if let (Ok(info), Ok(token)) = (
            fs::read_to_string(state.join("daemon.json")),
            fs::read_to_string(state.join("ui-token")),
        ) {
            let info: serde_json::Value = serde_json::from_str(&info)?;
            let port = info["port"].as_u64().context("daemon.json has no port")?;
            return Ok(format!("ws://127.0.0.1:{port}/ws?token={}", token.trim()));
        }
        if Instant::now() > deadline {
            let log = state.join("daemon.log");
            let last = fs::read_to_string(&log).unwrap_or_default();
            let last = last
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            bail!("naru-d did not start: {last} (see {})", log.display());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The standard menus without Tauri's default Close Window: ⌘W, ⌘T and ⌘D belong to panes, and
/// the screen handles them.
fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let separator = || PredefinedMenuItem::separator(app);
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "Naru",
                true,
                &[
                    &PredefinedMenuItem::about(app, None, None)?,
                    &separator()?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::show_all(app, None)?,
                    &separator()?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &separator()?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                    &PredefinedMenuItem::fullscreen(app, None)?,
                ],
            )?,
        ],
    )
}

fn main() {
    tauri::Builder::default()
        .menu(menu)
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let script = match daemon_url() {
                Ok(url) => format!("window.__NARU_WS__ = {};", serde_json::to_string(&url)?),
                Err(error) => {
                    let message = serde_json::to_string(&format!("{error:#}"))?;
                    format!("window.__NARU_ERROR__ = {message};")
                }
            };
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Naru")
                .inner_size(1400.0, 900.0)
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .initialization_script(&script)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Naru failed to start");
}
