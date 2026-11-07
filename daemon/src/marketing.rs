//! 마케팅 모드: the organic-lab admin screen beside a claude session in organic-lab's folder.
//! Settings live in `marketing.json` next to the state file; the session id is chosen here and
//! saved before claude starts, so the session resumes after a restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

use crate::agent::{Agent, is_plain_word};
use crate::core::Daemon;
use crate::state::PaneId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub dir: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// Which pane runs the session and which runs the server, while Naru is up.
static PANES: Mutex<Option<HashMap<&'static str, PaneId>>> = Mutex::new(None);
/// Held through a whole open or start, so two quick presses cannot make two sessions or servers.
static BUSY: Mutex<()> = Mutex::new(());

pub fn settings_path(state_file: &Path) -> PathBuf {
    state_file.with_file_name("marketing.json")
}

/// Reads the settings, writing organic-lab's known place on first use. That place is where the
/// project and its server actually are on this machine (2026-11-07), not a guess.
pub fn load(path: &Path, home: &Path) -> anyhow::Result<Settings> {
    if !path.exists() {
        let first = Settings {
            dir: home
                .join("projects/organic-lab")
                .to_string_lossy()
                .into_owned(),
            url: "http://127.0.0.1:3900/app/".to_owned(),
            session: None,
        };
        validate(&first)?;
        eprintln!("marketing: writing first settings to {}", path.display());
        save(path, &first)?;
        return Ok(first);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{}을 읽지 못했어요", path.display()))?;
    let settings: Settings = serde_json::from_str(&text)
        .with_context(|| format!("{}의 형식이 틀렸어요", path.display()))?;
    validate(&settings)?;
    Ok(settings)
}

fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(settings)?;
    crate::suggestions::write_private(path, text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Only a server on this machine: the screen is shown inside Naru.
pub fn validate(settings: &Settings) -> anyhow::Result<()> {
    let local = ["http://127.0.0.1:", "http://localhost:"]
        .iter()
        .any(|prefix| settings.url.starts_with(prefix));
    ensure!(
        local,
        "마케팅 화면 주소는 이 컴퓨터(127.0.0.1)만 돼요: {}",
        settings.url
    );
    ensure!(
        Path::new(&settings.dir).is_dir(),
        "오가닉랩 폴더가 없어요: {}",
        settings.dir
    );
    if let Some(session) = &settings.session {
        ensure!(
            uuid::Uuid::parse_str(session).is_ok() && is_plain_word(session),
            "저장된 세션 id가 이상해요: {session}"
        );
    }
    Ok(())
}

fn server_up(url: &str) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1)))
        .build()
        .into();
    agent.get(url).call().is_ok()
}

pub struct Opened {
    pub url: String,
    pub up: bool,
    pub pane: PaneId,
}

impl Daemon {
    fn marketing_pane(&self, key: &'static str) -> Option<PaneId> {
        let pane = PANES
            .lock()
            .expect("marketing panes lock")
            .as_ref()?
            .get(key)
            .copied()?;
        self.core().live.contains_key(&pane).then_some(pane)
    }

    fn remember(&self, key: &'static str, pane: PaneId) {
        PANES
            .lock()
            .expect("marketing panes lock")
            .get_or_insert_default()
            .insert(key, pane);
    }

    /// Checks the server and starts or resumes the organic-lab session.
    pub fn marketing_open(self: &Arc<Self>) -> anyhow::Result<Opened> {
        let _busy = BUSY.lock().expect("marketing busy lock");
        let path = settings_path(&self.state_file);
        let mut settings = load(&path, &self.home)?;
        let up = server_up(&settings.url);
        if let Some(pane) = self.marketing_pane("session") {
            return Ok(Opened {
                url: settings.url,
                up,
                pane,
            });
        }
        let launcher = {
            let core = self.core();
            crate::conversations::usual_launcher(
                core.workspace
                    .panes
                    .values()
                    .filter_map(|record| record.session.as_ref()),
            )
        };
        let session = match settings.session.clone() {
            Some(session) => session,
            None => {
                let session = uuid::Uuid::new_v4().to_string();
                settings.session = Some(session.clone());
                save(&path, &settings)?;
                session
            }
        };
        // A session whose start never reached claude has no transcript; it starts again under the
        // same id.
        let resume =
            crate::transcript::locate(Agent::Claude, None, Some(&session), &self.home).is_some();
        let command = if resume {
            Agent::Claude
                .resume_command(launcher.as_deref(), &session)
                .with_context(|| format!("cannot resume session {session}"))?
        } else {
            format!(
                "{} --session-id {session}",
                launcher.as_deref().unwrap_or("claude")
            )
        };
        self.ensure_group(&settings.dir);
        let pane = self.background_tab(
            settings.dir.clone(),
            command,
            "마케팅 · 오가닉랩".to_owned(),
        )?;
        self.remember("session", pane);
        Ok(Opened {
            url: settings.url,
            up,
            pane,
        })
    }

    /// Runs organic-lab's server in a background tab. Only on the user's press.
    pub fn marketing_start_server(self: &Arc<Self>) -> anyhow::Result<()> {
        let _busy = BUSY.lock().expect("marketing busy lock");
        if self.marketing_pane("server").is_some() {
            return Ok(());
        }
        let settings = load(&settings_path(&self.state_file), &self.home)?;
        self.ensure_group(&settings.dir);
        let pane = self.background_tab(
            settings.dir,
            "node src/server.ts".to_owned(),
            "오가닉랩 서버".to_owned(),
        )?;
        self.remember("server", pane);
        Ok(())
    }

    pub fn marketing_server_up(&self) -> anyhow::Result<bool> {
        let settings = load(&settings_path(&self.state_file), &self.home)?;
        Ok(server_up(&settings.url))
    }

    fn ensure_group(&self, dir: &str) {
        let mut core = self.core();
        if core.workspace.group_for(dir).is_none() {
            core.workspace
                .add_group("오가닉랩".to_owned(), Some(dir.to_owned()));
            self.changed(&core);
        }
    }
}

#[cfg(test)]
#[path = "marketing_tests.rs"]
mod tests;
