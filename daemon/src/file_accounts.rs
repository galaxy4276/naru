//! Agents whose whole login is one file: Codex, Grok and agy.
//!
//! A saved account is a folder under `~/.naru/accounts/<agent>/<id>` holding a copy of that file
//! where the agent's own login command writes it when pointed at the folder. Switching puts the
//! copy in place, after writing the signed-in file back to its own account so tokens the CLI
//! refreshed meanwhile are not lost. A signed-in file that belongs to no saved account is never
//! overwritten.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail, ensure};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::suggestions::write_private;

/// Who a login file is signed in as, as far as the file says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Login {
    /// Tells accounts apart; `None` when the file does not say whose it is.
    pub key: Option<String>,
    pub email: Option<String>,
    pub plan: Option<String>,
    /// What else the file holds.
    pub detail: Option<String>,
}

pub struct FileAgent {
    /// The file the CLI reads.
    pub live: PathBuf,
    /// Where a saved account keeps its copy, inside its folder.
    pub saved: &'static str,
    read: fn(&Value) -> Option<Login>,
    /// The process name, to refuse a switch the CLI could undo by writing its old login back.
    process: &'static str,
    /// Names an account whose file does not say whose it is.
    pub fallback: &'static str,
}

#[derive(Serialize, Deserialize, Debug)]
struct Saved {
    id: String,
    /// The name given when the file names nobody, e.g. "프로필 2".
    #[serde(default)]
    label: Option<String>,
}

pub struct Account {
    pub id: String,
    pub dir: PathBuf,
    label: Option<String>,
    /// Read from the saved copy; `None` when it is missing or unreadable.
    pub login: Option<Login>,
}

impl Account {
    pub fn label(&self, fallback: &str) -> String {
        self.login
            .as_ref()
            .and_then(|login| login.email.clone())
            .or_else(|| self.label.clone())
            .unwrap_or_else(|| fallback.to_owned())
    }

    fn key(&self) -> Option<&str> {
        self.login.as_ref()?.key.as_deref()
    }
}

/// The signed-in file and who it belongs to.
pub struct Live {
    pub bytes: Vec<u8>,
    pub login: Option<Login>,
}

impl FileAgent {
    pub fn codex(home: &Path) -> Self {
        Self {
            // ponytail: ~/.codex only; an app launched from Finder sees no CODEX_HOME anyway.
            live: home.join(".codex/auth.json"),
            saved: "home/auth.json",
            read: codex_login,
            process: "codex",
            fallback: "Codex 계정",
        }
    }

    pub fn grok(home: &Path) -> Self {
        Self {
            live: home.join(".grok/auth.json"),
            saved: "home/auth.json",
            read: grok_login,
            process: "grok",
            fallback: "Grok 계정",
        }
    }

    pub fn agy(home: &Path) -> Self {
        Self {
            live: home.join(".gemini/antigravity-cli/antigravity-oauth-token"),
            saved: "home/.gemini/antigravity-cli/antigravity-oauth-token",
            read: agy_login,
            process: "agy",
            fallback: "Google 계정",
        }
    }

    pub fn read_live(&self) -> anyhow::Result<Option<Live>> {
        Ok(read_json_file(&self.live)?.map(|(bytes, value)| Live {
            login: (self.read)(&value),
            bytes,
        }))
    }

    pub fn accounts(&self, root: &Path) -> Vec<Account> {
        let mut accounts: Vec<Account> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let dir = entry.path();
                let saved: Saved =
                    serde_json::from_slice(&std::fs::read(dir.join("account.json")).ok()?).ok()?;
                (saved.id == entry.file_name().to_string_lossy()).then(|| Account {
                    login: read_json_file(&dir.join(self.saved))
                        .ok()
                        .flatten()
                        .and_then(|(_, value)| (self.read)(&value)),
                    id: saved.id,
                    label: saved.label,
                    dir,
                })
            })
            .collect();
        accounts.sort_by_key(|account| account.label(self.fallback));
        accounts
    }

    /// The saved account the signed-in file belongs to. A file that says whose it is matches by
    /// that; one that does not, or matches several accounts, belongs to the account last
    /// switched to or saved from here.
    pub fn owner<'a>(
        &self,
        root: &Path,
        accounts: &'a [Account],
        live: Option<&Live>,
    ) -> Option<&'a Account> {
        let live = live?;
        let marked = std::fs::read_to_string(root.join("selected")).ok();
        let marked = |account: &&Account| marked.as_deref().map(str::trim) == Some(&account.id);
        match live.login.as_ref().and_then(|login| login.key.as_deref()) {
            Some(key) => {
                let same: Vec<&Account> = accounts
                    .iter()
                    .filter(|account| account.key() == Some(key))
                    .collect();
                match same.as_slice() {
                    [only] => Some(only),
                    many => many.iter().copied().find(marked),
                }
            }
            None => accounts.iter().find(marked),
        }
    }

    /// Saves the signed-in file as an account.
    pub fn save_current(&self, root: &Path) -> anyhow::Result<Account> {
        let live = self.read_live()?.context("로그인한 계정이 없어요")?;
        let accounts = self.accounts(root);
        if let Some(known) = self.owner(root, &accounts, Some(&live)) {
            bail!("이미 저장된 계정이에요: {}", known.label(self.fallback));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let dir = root.join(&id);
        write_private(&dir.join(self.saved), &live.bytes)?;
        self.write_saved(root, &id, &accounts, live.login.as_ref())?;
        write_private(&root.join("selected"), id.as_bytes())?;
        self.accounts(root)
            .into_iter()
            .find(|account| account.id == id)
            .context("saved account vanished")
    }

    /// Puts account `id`'s file in place of the signed-in one.
    pub fn switch(&self, root: &Path, id: &str) -> anyhow::Result<String> {
        let accounts = self.accounts(root);
        let target = accounts
            .iter()
            .find(|account| account.id == id)
            .context("저장된 계정이 없어요")?;
        let (target_bytes, _) = read_json_file(&target.dir.join(self.saved))?
            .context("저장된 로그인 파일이 없어요. 다시 로그인해 주세요")?;
        if let Some(live) = self.read_live()? {
            let owner = self
                .owner(root, &accounts, Some(&live))
                .context("지금 로그인이 저장된 계정이 아니에요. 먼저 계정으로 저장해 주세요")?;
            if owner.id == id {
                return Ok(target.label(self.fallback));
            }
            // Without a key saying whose the file is, a running CLI that writes its old login
            // back would make it look like the new account's, and the next switch would save it
            // there.
            let keyed = live.login.as_ref().is_some_and(|l| l.key.is_some());
            ensure!(
                keyed || !running(self.process),
                "실행 중인 {} 프로세스를 모두 닫은 뒤 바꿔 주세요. 열려 있으면 이전 로그인이 다시 저장될 수 있어요",
                self.process
            );
            write_private(&owner.dir.join(self.saved), &live.bytes)?;
        }
        write_private(&self.live, &target_bytes)?;
        write_private(&root.join("selected"), id.as_bytes())?;
        Ok(target.label(self.fallback))
    }

    /// Saves a login the agent wrote into account `id`'s folder; `None` while it is not done.
    pub fn finish_login(&self, root: &Path, id: &str) -> anyhow::Result<Option<String>> {
        let dir = root.join(id);
        let path = dir.join(self.saved);
        // A file still being written is not JSON yet.
        let Ok(Some((bytes, value))) = read_json_file(&path) else {
            return Ok(None);
        };
        // The CLI wrote it with its own mode; a saved login stays private like every other.
        write_private(&path, &bytes)?;
        let login = (self.read)(&value);
        let accounts = self.accounts(root);
        if let Some(key) = login.as_ref().and_then(|l| l.key.as_deref())
            && let Some(known) = accounts.iter().find(|a| a.id != id && a.key() == Some(key))
        {
            std::fs::remove_dir_all(&dir)?;
            bail!(
                "이미 저장된 계정이라 더하지 않았어요: {}",
                known.label(self.fallback)
            );
        }
        self.write_saved(root, id, &accounts, login.as_ref())?;
        Ok(self
            .accounts(root)
            .into_iter()
            .find(|account| account.id == id)
            .map(|account| account.label(self.fallback)))
    }

    fn write_saved(
        &self,
        root: &Path,
        id: &str,
        accounts: &[Account],
        login: Option<&Login>,
    ) -> anyhow::Result<()> {
        let label = login
            .and_then(|login| login.email.as_ref())
            .is_none()
            .then(|| format!("{} {}", self.fallback, accounts.len() + 1));
        let saved = Saved {
            id: id.to_owned(),
            label,
        };
        write_private(
            &root.join(id).join("account.json"),
            &serde_json::to_vec(&saved)?,
        )?;
        Ok(())
    }

    pub fn remove(&self, root: &Path, id: &str) -> anyhow::Result<()> {
        let accounts = self.accounts(root);
        let account = accounts
            .iter()
            .find(|account| account.id == id)
            .context("저장된 계정이 없어요")?;
        let live = self.read_live()?;
        ensure!(
            self.owner(root, &accounts, live.as_ref())
                .is_none_or(|owner| owner.id != id),
            "지금 쓰는 계정은 뺄 수 없어요. 다른 계정으로 바꾼 뒤 빼 주세요"
        );
        std::fs::remove_dir_all(&account.dir)?;
        let marker = root.join("selected");
        if std::fs::read_to_string(&marker).is_ok_and(|marked| marked.trim() == id) {
            std::fs::remove_file(marker)?;
        }
        Ok(())
    }
}

fn running(process: &str) -> bool {
    Command::new("/usr/bin/pgrep")
        .args(["-x", process])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn read_json_file(path: &Path) -> anyhow::Result<Option<(Vec<u8>, Value)>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let value = serde_json::from_slice(&bytes)
                .with_context(|| format!("{} is not JSON", path.display()))?;
            Ok(Some((bytes, value)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn jwt_claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|payload| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload.trim_end_matches('='))
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Codex's `auth.json`: the ChatGPT account and user from its ID token.
pub fn codex_login(auth: &Value) -> Option<Login> {
    let tokens = &auth["tokens"];
    let claims = tokens["id_token"]
        .as_str()
        .map(jwt_claims)
        .unwrap_or_default();
    let openai = &claims["https://api.openai.com/auth"];
    let account = text(&tokens["account_id"]).or_else(|| text(&openai["chatgpt_account_id"]));
    let email = text(&claims["email"]);
    if account.is_none() && email.is_none() {
        return None;
    }
    Some(Login {
        // A ChatGPT team shares one account id among its members.
        key: Some(format!(
            "{}|{}",
            account.unwrap_or_default(),
            email.clone().unwrap_or_default()
        )),
        email,
        plan: text(&openai["chatgpt_plan_type"]),
        detail: None,
    })
}

/// Grok's `auth.json`: one entry per sign-in issuer, each holding a session token.
pub fn grok_login(auth: &Value) -> Option<Login> {
    auth.as_object()?.values().find_map(|entry| {
        let claims = entry["key"].as_str().map(jwt_claims).unwrap_or_default();
        let email = text(&entry["email"]).or_else(|| text(&claims["email"]));
        let user = text(&entry["user_id"])
            .or_else(|| text(&claims["user_id"]))
            .or_else(|| text(&claims["sub"]));
        (user.is_some() || email.is_some()).then(|| Login {
            key: user.or_else(|| email.clone()),
            email,
            plan: None,
            detail: None,
        })
    })
}

/// agy's `antigravity-oauth-token`, which names the Google account it signed in.
pub fn agy_login(token: &Value) -> Option<Login> {
    let email = text(&token["id_token_email"])?;
    Some(Login {
        key: Some(email.to_lowercase()),
        email: Some(email),
        plan: None,
        detail: None,
    })
}

#[cfg(test)]
#[path = "file_accounts_tests.rs"]
mod tests;
