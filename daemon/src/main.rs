//! naru-d: owns Naru's panes and state; screens attach over a local WebSocket.

mod accounts;
mod agent;
mod ai;
mod background;
mod brain;
mod claude_accounts;
mod code_map;
mod conversations;
mod core;
mod cost;
mod devtools;
mod file_accounts;
mod global_instruction;
mod grid;
mod import;
mod learn;
mod learn_map;
mod letters;
mod library;
mod marketing;
mod memory;
mod meta_ads;
mod naru;
mod nodes;
mod osc;
mod pane;
mod paths;
mod plans;
mod record;
mod session;
mod state;
mod suggestions;
mod team;
mod titles;
mod transcript;
mod ui;
mod ws;

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use axum::Router;
use axum::routing::get;
use tokio::signal::unix::{SignalKind, signal};

use crate::core::Daemon;
use crate::state::Workspace;

const ZSH_FILES: [(&str, &str); 5] = [
    (".zshenv", include_str!("../../shell/zsh/.zshenv")),
    (".zprofile", include_str!("../../shell/zsh/.zprofile")),
    (".zshrc", include_str!("../../shell/zsh/.zshrc")),
    (".zlogin", include_str!("../../shell/zsh/.zlogin")),
    ("naru.zsh", include_str!("../../shell/zsh/naru.zsh")),
];

/// Owner-only file, replaced whole.
fn write_private(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(contents)?;
    Ok(())
}

/// Holds the control socket for as long as the daemon runs; a second daemon finds it answering.
fn claim_single_instance(state: &Path) -> anyhow::Result<()> {
    let control = state.join("daemon.sock");
    if UnixStream::connect(&control).is_ok() {
        bail!("naru-d is already running ({})", control.display());
    }
    let _ = fs::remove_file(&control);
    let listener = UnixListener::bind(&control)?;
    std::thread::spawn(move || listener.incoming().for_each(drop));
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let state = home.join("Library/Application Support/Naru");
    fs::create_dir_all(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    claim_single_instance(&state)?;
    let naru_dir = home.join(".naru");
    naru::refuse_if_served(&naru_dir)?;

    let zsh_dir = state.join("shell/zsh");
    fs::create_dir_all(&zsh_dir)?;
    for (name, contents) in ZSH_FILES {
        fs::write(zsh_dir.join(name), contents)?;
    }

    let token: String = rand::random::<[u8; 32]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    write_private(&state.join("ui-token"), token.as_bytes())?;

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let info = serde_json::json!({ "port": port, "pid": std::process::id() });
    write_private(&state.join("daemon.json"), info.to_string().as_bytes())?;
    eprintln!(
        "naru-d listening on 127.0.0.1:{port}, state in {}",
        state.display()
    );

    let state_file = state.join("state.json");
    if !state_file.exists() {
        // First run: bring over what the Warp fork had open. Its database is only read.
        let db = import::warp_db(&home);
        if db.exists() {
            match import::import(&db, &naru_dir) {
                Ok(imported) => {
                    eprintln!(
                        "imported {} tabs and {} panes from {}",
                        imported.tabs.len(),
                        imported.panes.len(),
                        db.display()
                    );
                    imported.save(&state_file)?;
                }
                Err(error) => eprintln!("not importing {}: {error:#}", db.display()),
            }
        }
    }
    // A Claude account switch cut off by a crash is rolled back before anything reads the login.
    if let Err(error) = claude_accounts::recover(&claude_accounts::root(&home)) {
        eprintln!("claude accounts: cannot recover an interrupted switch: {error:#}");
    }
    let workspace = Workspace::load(&state_file)?;
    let naru_token = naru::publish(&naru_dir)?;
    naru::serve_socket(&naru_dir, naru_token.clone())?;
    let daemon = Daemon::new(token, home, state_file, zsh_dir, workspace, naru_token);
    daemon.restore();
    library::queue::start(&daemon);
    {
        // The first ⌘K search would otherwise wait for every transcript to be read.
        let daemon = daemon.clone();
        tokio::task::spawn_blocking(move || {
            daemon
                .conversations
                .refresh_if_stale(&daemon.claude_projects)
        });
    }
    {
        let daemon = daemon.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                daemon.tick();
            }
        });
    }
    {
        let daemon = daemon.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                daemon.ai_tick();
            }
        });
    }
    {
        let daemon = daemon.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));
                daemon.title_pass();
            }
        });
    }
    // Shells end when the daemon does; that must not read as the user closing every pane.
    for kind in [
        SignalKind::terminate(),
        SignalKind::interrupt(),
        SignalKind::hangup(),
    ] {
        let mut stream = signal(kind)?;
        let daemon = daemon.clone();
        tokio::spawn(async move {
            stream.recv().await;
            daemon.terminate();
            std::process::exit(0);
        });
    }
    let app = Router::new()
        .route("/ws", get(ws::handler))
        .route(
            "/library/{id}/pdf",
            get(ws::book_pdf).layer(ws::book_cors()),
        )
        .with_state(daemon);
    axum::serve(listener, app).await?;
    Ok(())
}
