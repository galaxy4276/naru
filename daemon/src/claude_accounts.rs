//! Claude account switching, ported from the old Naru (`settings_view/cli_account_*.rs`).
//!
//! The on-disk layout under `$HOME/.warp-oss/cli-accounts` and the switch journal JSON are shared
//! with the old app, so both can read the same state. Switching rewrites the default Claude login
//! (Keychain `Claude Code-credentials` + `~/.claude.json` `oauthAccount`) under Claude's own lock,
//! with a journal so an interrupted switch is rolled back on the next start.

use std::collections::HashSet;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, MetadataExt as _, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

static SWITCH_LOCK: Mutex<()> = Mutex::new(());
const SWITCH_JOURNAL: &str = "claude-switch-journal.json";
const LOCK_STALE: Duration = Duration::from_secs(10);
const LOCK_REFRESH: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeAccount {
    pub id: Uuid,
    pub email: String,
    /// `oauthAccount.planDisplayName` from the saved login state.
    pub plan: Option<String>,
}

/// Same fields as the old app's `Account`; `agent` is a string so other providers' folders load.
#[derive(Serialize, Deserialize)]
struct AccountFile {
    id: Uuid,
    label: String,
    agent: String,
    #[serde(default)]
    shared_claude_history: bool,
    #[serde(default)]
    authenticated: bool,
}

pub fn root(home: &Path) -> PathBuf {
    home.join(".warp-oss").join("cli-accounts")
}

// ---------------------------------------------------------------- process identity

/// Pid plus start time, so a live owner can be told from a dead one whose pid was reused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ProcessIdentity {
    pid: u32,
    parent: u32,
    group: u32,
    started: u64,
    boot_id: Option<String>,
}

impl ProcessIdentity {
    fn is_current(&self) -> bool {
        Self::read(self.pid).as_ref() == Some(self)
    }

    #[cfg(target_os = "macos")]
    fn read(pid: u32) -> Option<Self> {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        // SAFETY: `info` is a writable buffer of exactly `size` bytes.
        let read = unsafe {
            libc::proc_pidinfo(
                pid.try_into().ok()?,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size.try_into().ok()?,
            )
        };
        if usize::try_from(read).ok()? != size {
            return None;
        }
        // SAFETY: proc_pidinfo filled the whole struct.
        let info = unsafe { info.assume_init() };
        if info.pbi_pid != pid || info.pbi_start_tvsec == 0 {
            return None;
        }
        Some(Self {
            pid,
            parent: info.pbi_ppid,
            group: info.pbi_pgid,
            started: info
                .pbi_start_tvsec
                .checked_mul(1_000_000)?
                .checked_add(info.pbi_start_tvusec)?,
            boot_id: None,
        })
    }

    // ponytail: the daemon only ships on macOS; add the /proc reader if that changes.
    #[cfg(not(target_os = "macos"))]
    fn read(_pid: u32) -> Option<Self> {
        None
    }
}

// ---------------------------------------------------------------- Keychain (`security` CLI)

#[cfg(target_os = "macos")]
// Tests keep credentials in files, so the Keychain calls are unused there.
#[cfg_attr(test, allow(dead_code))]
mod keychain {
    use std::io::Write as _;
    use std::process::{Command, Output, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use anyhow::{Context, Result, ensure};

    fn run(mut command: Command, input: Option<&str>) -> Result<Output> {
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let pid = child.id();
        if let Some(input) = input {
            child
                .stdin
                .take()
                .context("Missing Keychain command input")?
                .write_all(input.as_bytes())?;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        match receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(output) => Ok(output?),
            Err(_) => {
                // SAFETY: plain kill(2) on the child we just spawned.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
                anyhow::bail!("macOS Keychain access timed out")
            }
        }
    }

    pub(super) fn parse_read(output: Output) -> Result<Option<String>> {
        if output.status.code() == Some(44) {
            return Ok(None);
        }
        ensure!(
            output.status.success(),
            "macOS Keychain refused access to the Claude login"
        );
        let value = String::from_utf8(output.stdout)?;
        ensure!(
            !value.trim().is_empty(),
            "macOS Keychain returned an empty Claude login"
        );
        Ok(Some(value.trim().to_owned()))
    }

    pub(super) fn write_command(service: &str, user: &str, value: &str) -> Result<String> {
        let args = [
            "add-generic-password",
            "-U",
            "-s",
            service,
            "-a",
            user,
            "-w",
            value,
        ];
        ensure!(
            args.iter().all(|arg| !arg.contains(['\n', '\r', '\0'])),
            "Invalid Keychain command input"
        );
        // Keep credentials off process arguments (`ps`) and out of command logs: `security -i`
        // reads the command from stdin.
        let input = args
            .iter()
            .map(|arg| format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(" ")
            + "\n";
        // ponytail: security -i has a 4096-byte line limit; larger logins need a signed helper.
        ensure!(
            input.len() < 4096,
            "Claude credentials exceed the macOS Keychain input limit; the existing login was kept"
        );
        Ok(input)
    }

    fn user() -> String {
        std::env::var("USER")
            .ok()
            .filter(|user| {
                !user.is_empty()
                    && user
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            })
            .unwrap_or_else(|| "claude-code-user".into())
    }

    pub(super) fn read(service: &str) -> Result<Option<String>> {
        let mut command = Command::new("/usr/bin/security");
        command.args(["find-generic-password", "-s", service, "-a", &user(), "-w"]);
        parse_read(run(command, None)?)
    }

    pub(super) fn write(service: &str, value: &str) -> Result<()> {
        let input = write_command(service, &user(), value)?;
        let mut command = Command::new("/usr/bin/security");
        command.args(["-q", "-i"]);
        let output = run(command, Some(&input))?;
        ensure!(
            output.status.success() && output.stderr.is_empty(),
            "Cannot save the Claude login in macOS Keychain"
        );
        ensure!(
            read(service)?.as_deref() == Some(value),
            "macOS Keychain did not save the Claude login"
        );
        Ok(())
    }

    pub(super) fn remove(service: &str) -> Result<()> {
        let mut command = Command::new("/usr/bin/security");
        command.args(["delete-generic-password", "-s", service, "-a", &user()]);
        let output = run(command, None)?;
        ensure!(
            output.status.success() || output.status.code() == Some(44),
            "Cannot remove the Claude login from macOS Keychain"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn run_for_test(command: Command, input: Option<&str>) -> Result<Output> {
        run(command, input)
    }
}

// ---------------------------------------------------------------- small file helpers

fn read_object(path: &Path) -> Result<Value> {
    let value: Value = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("Cannot read Claude preferences: {}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.into()),
    };
    ensure!(value.is_object(), "Claude preferences must be an object");
    Ok(value)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Atomic 0600 write: temp file in the same folder, then rename.
fn replace_private(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent folder")?;
    let name = path
        .file_name()
        .context("Missing file name")?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn private_dir(path: &Path) -> Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    Ok(())
}

fn restore_pointer(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = bytes {
        replace_private(path, bytes)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

// ---------------------------------------------------------------- Claude config locations

fn is_default_claude_in(config: &Path, home: Option<&Path>, config_dir_env_set: bool) -> bool {
    !config_dir_env_set && home.is_some_and(|home| config == home.join(".claude"))
}

fn is_default_claude(config: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    is_default_claude_in(
        config,
        home.as_deref(),
        std::env::var_os("CLAUDE_CONFIG_DIR").is_some(),
    )
}

fn state_path(config: &Path) -> PathBuf {
    if config.join(".config.json").is_file() {
        config.join(".config.json")
    } else if is_default_claude(config) {
        config.with_extension("json")
    } else {
        config.join(".claude.json")
    }
}

/// Claude names its Keychain item after the config folder: the legacy name for `~/.claude`,
/// otherwise a suffix from the first 8 hex digits of the path's SHA-256.
fn claude_keychain_service(config: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let config = config.to_str().context("The account path is not UTF-8")?;
    let digest = Sha256::digest(config.as_bytes());
    let suffix: String = digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("Claude Code-credentials-{suffix}"))
}

#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))]
fn credential_service(config: &Path) -> Result<String> {
    if is_default_claude(config) {
        Ok("Claude Code-credentials".into())
    } else {
        claude_keychain_service(config)
    }
}

fn read_credentials(config: &Path) -> Result<Value> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        if let Some(value) = keychain::read(&credential_service(config)?)? {
            return Ok(serde_json::from_str(&value)?);
        }
    }
    read_object(&config.join(".credentials.json"))
}

fn write_credentials(config: &Path, credentials: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(credentials)?;
    #[cfg(all(target_os = "macos", not(test)))]
    {
        keychain::write(&credential_service(config)?, std::str::from_utf8(&bytes)?)?;
        // The CLI checks this mtime to invalidate its OAuth cache; tokens stay in Keychain.
        let file = config.join(".credentials.json");
        let existing = read_object(&file)?;
        replace_private(
            &file,
            if existing.get("claudeAiOauth").is_some() {
                &bytes
            } else {
                b"{}"
            },
        )
    }
    #[cfg(any(not(target_os = "macos"), test))]
    replace_private(&config.join(".credentials.json"), &bytes)
}

/// Removes the Keychain item Claude keeps for `config`. File-backed credentials need nothing.
fn remove_credential_item(config: &Path) -> Result<()> {
    #[cfg(all(target_os = "macos", not(test)))]
    keychain::remove(&credential_service(config)?)?;
    let _ = config;
    Ok(())
}

// ---------------------------------------------------------------- login validation

fn valid_login(state: &Value, credentials: &Value, label: &str) -> Result<()> {
    ensure!(
        state["oauthAccount"]["emailAddress"]
            .as_str()
            .is_some_and(|email| email.eq_ignore_ascii_case(label)),
        "The saved login does not match {label}. Sign in again; running sessions were kept."
    );
    ensure!(
        credentials["claudeAiOauth"]["accessToken"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "The saved login for {label} is missing. Sign in again; running sessions were kept."
    );
    ensure!(
        credentials["claudeAiOauth"]["refreshToken"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "The saved login for {label} can no longer refresh. Sign in again; running sessions were kept."
    );
    Ok(())
}

fn matches_saved_login(
    state: &Value,
    credentials: &Value,
    saved_state: &Value,
    saved_credentials: &Value,
    label: &str,
) -> bool {
    if valid_login(state, credentials, label).is_err() {
        return false;
    }
    let identity = &state["oauthAccount"];
    let saved_identity = &saved_state["oauthAccount"];
    let oauth = &credentials["claudeAiOauth"];
    for (key, aliases) in [
        ("accountUuid", ["accountUuid", "accountId"]),
        ("organizationUuid", ["organizationUuid", "organizationId"]),
        ("emailAddress", ["email", "email"]),
    ] {
        let current = identity[key].as_str();
        let saved = saved_identity[key].as_str();
        let token = aliases.iter().find_map(|key| oauth[key].as_str());
        if current
            .zip(saved)
            .is_some_and(|(a, b)| !a.eq_ignore_ascii_case(b))
            || token
                .zip(current.or(saved))
                .is_some_and(|(a, b)| !a.eq_ignore_ascii_case(b))
        {
            return false;
        }
    }
    oauth["refreshToken"] == saved_credentials["claudeAiOauth"]["refreshToken"]
        || identity["accountUuid"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && saved_identity["accountUuid"].as_str() == Some(id))
}

fn save_login(source: &Path, vault: &Path, label: &str) -> Result<()> {
    let state = read_object(&state_path(source))?;
    let credentials = read_credentials(source)?;
    valid_login(&state, &credentials, label)?;
    private_dir(vault)?;
    write_credentials(
        vault,
        &json!({"claudeAiOauth": credentials["claudeAiOauth"]}),
    )?;
    replace_private(
        &state_path(vault),
        &serde_json::to_vec(&json!({"oauthAccount": state["oauthAccount"]}))?,
    )
}

fn merge_claude_credentials(original: &str, fresh: &str) -> Result<String> {
    let mut original: Value = serde_json::from_str(original)?;
    let fresh: Value = serde_json::from_str(fresh)?;
    let login = fresh
        .get("claudeAiOauth")
        .filter(|value| value.is_object())
        .context(
            "Claude returned an unsupported credential format; the existing account was kept",
        )?;
    original
        .as_object_mut()
        .context("The existing Claude credential format is unsupported")?
        .insert("claudeAiOauth".into(), login.clone());
    Ok(serde_json::to_string(&original)?)
}

// ---------------------------------------------------------------- Claude's settings lock

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LockIdentity {
    device: u64,
    inode: u64,
    created: std::time::SystemTime,
}

fn lock_identity(path: &Path) -> std::io::Result<LockIdentity> {
    let metadata = path.metadata()?;
    Ok(LockIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        created: metadata.created()?,
    })
}

fn lock_mtime(path: &Path) -> Option<std::time::SystemTime> {
    path.metadata().and_then(|meta| meta.modified()).ok()
}

/// Claude CLI's `proper-lockfile` lock on `.claude.json`. A holder refreshes the folder's mtime,
/// and one untouched for `LOCK_STALE` is abandoned and may be taken over.
struct ConfigLock {
    path: PathBuf,
    identity: LockIdentity,
    file: fs::File,
    heartbeat: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ConfigLock {
    /// Refresh the held inode only; never stamp a replacement lock belonging to Claude.
    fn reclaim(&mut self) -> Result<()> {
        ensure!(
            lock_identity(&self.path).ok().as_ref() == Some(&self.identity),
            "Claude reclaimed its settings during the switch. Try switching again."
        );
        self.file.set_modified(std::time::SystemTime::now())?;
        ensure!(
            lock_identity(&self.path).ok().as_ref() == Some(&self.identity),
            "Claude reclaimed its settings during the switch. Try switching again."
        );
        Ok(())
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let (stopped, wake) = &*self.heartbeat;
        *stopped.lock().unwrap_or_else(|error| error.into_inner()) = true;
        wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if lock_identity(&self.path).ok().as_ref() == Some(&self.identity) {
            let _ = fs::remove_dir(&self.path);
        }
    }
}

fn lock_config(config: &Path) -> Result<ConfigLock> {
    lock_config_wait(config, 50)
}

fn lock_config_wait(config: &Path, attempts: u32) -> Result<ConfigLock> {
    let lock = state_path(config).with_extension("json.lock");
    for attempt in 0..attempts {
        match fs::create_dir(&lock) {
            Ok(()) => {
                let identity = lock_identity(&lock)?;
                let file = fs::File::open(&lock)?;
                {
                    let metadata = file.metadata()?;
                    ensure!(
                        metadata.ino() == identity.inode
                            && metadata.dev() == identity.device
                            && metadata.created()? == identity.created,
                        "Claude settings lock changed while opening it"
                    );
                }
                let heartbeat = Arc::new((Mutex::new(false), Condvar::new()));
                let worker_heartbeat = heartbeat.clone();
                let worker_path = lock.clone();
                let worker_identity = identity.clone();
                let worker_file = file.try_clone()?;
                let worker = std::thread::Builder::new()
                    .name("claude-settings-lock".into())
                    .spawn(move || {
                        let (stopped, wake) = &*worker_heartbeat;
                        let mut stopped = stopped.lock().unwrap_or_else(|error| error.into_inner());
                        loop {
                            let (next, _) = wake
                                .wait_timeout_while(stopped, LOCK_REFRESH, |stopped| !*stopped)
                                .unwrap_or_else(|error| error.into_inner());
                            stopped = next;
                            if *stopped
                                || lock_identity(&worker_path).ok().as_ref()
                                    != Some(&worker_identity)
                            {
                                break;
                            }
                            if worker_file
                                .set_modified(std::time::SystemTime::now())
                                .is_err()
                            {
                                break;
                            }
                        }
                    })?;
                return Ok(ConfigLock {
                    path: lock,
                    identity,
                    file,
                    heartbeat,
                    worker: Some(worker),
                });
            }
            // Left behind by a process that died while holding it, e.g. a crash mid-switch.
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && lock_mtime(&lock)
                        .and_then(|time| time.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE) =>
            {
                eprintln!("Removing stale Claude settings lock {}", lock.display());
                let _ = fs::remove_dir(&lock);
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists && attempt + 1 < attempts =>
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(error).context("Claude is saving its settings. Try switching again.");
            }
        }
    }
    unreachable!()
}

// ---------------------------------------------------------------- preferences sharing

fn inherit_preferences(current: &mut Value, source: &Value, previous: Option<&Value>) {
    if let (Some(current), Some(source)) = (current.as_object_mut(), source.as_object()) {
        if let Some(previous) = previous.and_then(Value::as_object) {
            for (key, old) in previous {
                if !source.contains_key(key) {
                    if current.get(key) == Some(old) {
                        current.remove(key);
                    } else if let Some(current) = current.get_mut(key) {
                        if old.is_object() {
                            inherit_preferences(current, &json!({}), Some(old));
                        } else if old.is_array() {
                            inherit_preferences(current, &json!([]), Some(old));
                        }
                    }
                }
            }
        }
        for (key, value) in source {
            if let Some(current) = current.get_mut(key) {
                inherit_preferences(current, value, previous.and_then(|old| old.get(key)));
            } else {
                current.insert(key.clone(), value.clone());
            }
        }
    } else if let (Some(current), Some(source)) = (current.as_array_mut(), source.as_array()) {
        let old = previous.and_then(Value::as_array);
        current
            .retain(|value| old.is_none_or(|old| !old.contains(value)) || source.contains(value));
        for value in source {
            if !current.contains(value) {
                current.push(value.clone());
            }
        }
    } else if previous == Some(&*current) {
        *current = source.clone();
    }
}

/// Copies the user's non-identity Claude preferences into a new account's config folder.
fn share_claude_preferences(config: &Path, personal: &Path) -> Result<()> {
    let ledger_path = config.join(".naru-personal-preferences.json");
    let mut ledger = read_object(&ledger_path)?;
    let before = ledger.clone();
    let mut settings = read_object(&personal.join("settings.json"))?;
    if let Some(object) = settings.as_object_mut() {
        for key in [
            "apiKeyHelper",
            "awsAuthRefresh",
            "awsCredentialExport",
            "forceLoginMethod",
            "forceLoginOrgUUID",
        ] {
            object.remove(key);
        }
    }
    if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
        for key in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "ANTHROPIC_PROFILE",
            "ANTHROPIC_FEDERATION_RULE_ID",
        ] {
            env.remove(key);
        }
    }
    let global = read_object(&state_path(personal))?;
    // Claude's global state also contains account identity; only share these preferences.
    let global = Value::Object(
        [
            "mcpServers",
            "theme",
            "hasCompletedClaudeInChromeOnboarding",
            "claudeInChromeDefaultEnabled",
        ]
        .into_iter()
        .filter_map(|key| global.get(key).map(|value| (key.to_owned(), value.clone())))
        .collect(),
    );
    for (name, source) in [
        ("settings.json", settings),
        (".claude.json", global),
        (
            "plugins/installed_plugins.json",
            read_object(&personal.join("plugins/installed_plugins.json"))?,
        ),
        (
            "plugins/known_marketplaces.json",
            read_object(&personal.join("plugins/known_marketplaces.json"))?,
        ),
    ] {
        let _lock = if name == ".claude.json" {
            Some(lock_config(config)?)
        } else {
            None
        };
        let path = if name == ".claude.json" {
            state_path(config)
        } else {
            config.join(name)
        };
        let mut current = read_object(&path)?;
        let original = current.clone();
        inherit_preferences(&mut current, &source, ledger.get(name));
        // A completed OAuth login must not re-enter the wizard and rebind this profile.
        if name == ".claude.json"
            && current["oauthAccount"]["emailAddress"]
                .as_str()
                .is_some_and(|email| !email.is_empty())
        {
            current["hasCompletedOnboarding"] = Value::Bool(true);
        }
        if current != original {
            fs::create_dir_all(path.parent().context("Missing preferences folder")?)?;
            replace_private(&path, &serde_json::to_vec_pretty(&current)?)?;
        }
        ledger[name] = source;
    }
    for name in [
        "skills",
        "commands",
        "agents",
        "hooks",
        "rules",
        "output-styles",
        "themes",
        "workflows",
        "plugins",
    ] {
        let source = personal.join(name);
        if !source.is_dir() {
            continue;
        }
        let target = config.join(name);
        fs::create_dir_all(&target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            // Synced content belongs to the signed-in account, not the local skill library.
            if name.to_string_lossy().starts_with('.') || name == "synced" || name == "data" {
                continue;
            }
            link_missing(&entry.path(), &target.join(name))?;
        }
    }
    for name in ["CLAUDE.md", "keybindings.json"] {
        let source = personal.join(name);
        if source.is_file() {
            link_missing(&source, &config.join(name))?;
        }
    }
    if ledger != before {
        replace_private(&ledger_path, &serde_json::to_vec_pretty(&ledger)?)?;
    }
    Ok(())
}

fn link_missing(source: &Path, target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::os::unix::fs::symlink(source, target)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

// ---------------------------------------------------------------- listing

pub fn list(root: &Path) -> Result<(Vec<ClaudeAccount>, Vec<String>)> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => return Err(error.into()),
    };
    let mut accounts = Vec::new();
    let mut errors = Vec::new();
    for entry in entries {
        let entry = entry?;
        // A folder without account.json is a sign-in still in progress.
        if !entry.file_type()?.is_dir() || !entry.path().join("account.json").exists() {
            continue;
        }
        let result = (|| -> Result<Option<ClaudeAccount>> {
            let account: AccountFile =
                serde_json::from_slice(&fs::read(entry.path().join("account.json"))?)?;
            ensure!(
                entry.file_name() == account.id.to_string().as_str(),
                "Account folder and ID do not match"
            );
            if account.agent != "Claude" {
                return Ok(None);
            }
            let plan = read_object(&state_path(&entry.path().join("login")))
                .ok()
                .and_then(|state| {
                    state["oauthAccount"]["planDisplayName"]
                        .as_str()
                        .map(str::to_owned)
                });
            Ok(Some(ClaudeAccount {
                id: account.id,
                email: account.label,
                plan,
            }))
        })();
        match result {
            Ok(Some(account)) => accounts.push(account),
            Ok(None) => {}
            Err(error) => {
                let message = format!(
                    "Unavailable account {}: {error:#}",
                    entry.file_name().to_string_lossy()
                );
                eprintln!("{message}");
                errors.push(message);
            }
        }
    }
    accounts.sort_by_key(|account| account.email.to_lowercase());
    Ok((accounts, errors))
}

pub fn selected(root: &Path) -> Option<Uuid> {
    Uuid::parse_str(
        fs::read_to_string(root.join("selected-claude"))
            .ok()?
            .trim(),
    )
    .ok()
}

/// `oauthAccount.emailAddress` of the live login, from the state file only.
pub fn current_email(personal: &Path) -> Option<String> {
    let state = read_object(&state_path(personal)).ok()?;
    state["oauthAccount"]["emailAddress"]
        .as_str()
        .filter(|email| !email.is_empty())
        .map(str::to_owned)
}

// ---------------------------------------------------------------- journal and recovery

#[derive(Serialize, Deserialize)]
struct SwitchJournal {
    owner: ProcessIdentity,
    id: Uuid,
    configs: Vec<(PathBuf, Value, usize)>,
    locks: Vec<(PathBuf, LockIdentity)>,
    runtime: Option<Vec<u8>>,
    selected: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct BackupOwner {
    owner: ProcessIdentity,
    locks: Vec<(PathBuf, LockIdentity)>,
}

fn recovery_vault(root: &Path, id: Uuid, index: usize) -> PathBuf {
    root.join("claude-switch-backups")
        .join(id.to_string())
        .join(index.to_string())
}

fn remove_recovery_backup(root: &Path, id: Uuid) -> Result<()> {
    let directory = root.join("claude-switch-backups").join(id.to_string());
    if !directory.exists() {
        return Ok(());
    }
    // Keep the owner marker until every scoped login and credential file has been removed.
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if entry.file_name() == "owner.json" {
            continue;
        }
        ensure!(
            entry.file_type()?.is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.parse::<usize>().is_ok()),
            "Unexpected file in Claude recovery backup; keeping it for inspection"
        );
        remove_credential_item(&entry.path())?;
        fs::remove_dir_all(entry.path())?;
    }
    sync_dir(&directory)?;
    restore_pointer(&directory.join("owner.json"), None)?;
    fs::remove_dir(&directory)?;
    sync_dir(directory.parent().context("Missing recovery folder")?)?;
    Ok(())
}

fn clean_orphaned_backups(root: &Path, active: Option<Uuid>) -> Result<()> {
    let directory = root.join("claude-switch-backups");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let current = ProcessIdentity::read(std::process::id());
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
        else {
            continue;
        };
        if Some(id) == active {
            continue;
        }
        // Older unmarked backups cannot safely be distinguished from another live switch.
        let owner = fs::read(entry.path().join("owner.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<BackupOwner>(&bytes).ok());
        if let Some(backup) = owner
            && (Some(&backup.owner) == current.as_ref() || !backup.owner.is_current())
        {
            for (path, identity) in &backup.locks {
                if lock_identity(path).ok().as_ref() == Some(identity) {
                    fs::remove_dir(path)?;
                }
            }
            if let Err(error) = remove_recovery_backup(root, id) {
                eprintln!("Could not remove orphaned Claude recovery login: {error:#}");
            }
        }
    }
    Ok(())
}

fn clear_switch_journal(root: &Path, journal: &SwitchJournal) -> Result<()> {
    restore_pointer(&root.join(SWITCH_JOURNAL), None)?;
    sync_dir(root)?;
    #[cfg(test)]
    tests::interrupt_if_requested(root, "committed");
    if let Err(error) = remove_recovery_backup(root, journal.id) {
        // The owner marker retains enough information to retry cleanup on the next recovery.
        eprintln!("Could not remove committed Claude recovery login: {error:#}");
    }
    Ok(())
}

fn rollback_switch(root: &Path, journal: &SwitchJournal, locks: &mut [ConfigLock]) -> Result<()> {
    for (config, state, index) in &journal.configs {
        let credentials = read_credentials(&recovery_vault(root, journal.id, *index))?;
        let mut current_state = read_object(&state_path(config))?;
        let mut current_credentials = read_credentials(config)?;
        // Roll back only fields changed by the switch, preserving later CLI settings and MCP refreshes.
        for (current, original, keys) in [
            (
                &mut current_state,
                state,
                &["oauthAccount", "hasCompletedOnboarding"][..],
            ),
            (
                &mut current_credentials,
                &credentials,
                &["claudeAiOauth"][..],
            ),
        ] {
            let object = current
                .as_object_mut()
                .context("Invalid Claude recovery state")?;
            for key in keys {
                if let Some(value) = original.get(*key) {
                    object.insert((*key).into(), value.clone());
                } else {
                    object.remove(*key);
                }
            }
        }
        for lock in &mut *locks {
            lock.reclaim()?;
        }
        write_auth_locked(config, &current_state, &current_credentials, locks)?;
    }
    restore_pointer(&root.join("runtime-claude"), journal.runtime.as_deref())?;
    restore_pointer(&root.join("selected-claude"), journal.selected.as_deref())?;
    sync_dir(root)?;
    clear_switch_journal(root, journal)
}

fn recover_switch_locked(root: &Path) -> Result<()> {
    let Some(bytes) = read_optional(&root.join(SWITCH_JOURNAL))? else {
        return clean_orphaned_backups(root, None);
    };
    let mut journal: SwitchJournal =
        serde_json::from_slice(&bytes).context("Invalid Claude switch recovery record")?;
    let current = ProcessIdentity::read(std::process::id());
    ensure!(
        current.as_ref() == Some(&journal.owner) || !journal.owner.is_current(),
        "Another process is changing Claude accounts"
    );
    clean_orphaned_backups(root, Some(journal.id))?;
    // Only reclaim the dead switch's own locks; a later CLI lock must remain untouched.
    for (path, identity) in &journal.locks {
        if lock_identity(path).ok().as_ref() == Some(identity) {
            fs::remove_dir(path)?;
        }
    }
    let mut locks = Vec::new();
    let mut paths = HashSet::new();
    for (config, _, _) in &journal.configs {
        if paths.insert(config.canonicalize()?) {
            locks.push(lock_config_wait(config, 120)?);
        }
    }
    journal.owner = current.context("Cannot identify the recovery process")?;
    journal.locks = locks
        .iter()
        .map(|lock| lock_identity(&lock.path).map(|identity| (lock.path.clone(), identity)))
        .collect::<std::io::Result<_>>()?;
    replace_private(&root.join(SWITCH_JOURNAL), &serde_json::to_vec(&journal)?)?;
    sync_dir(root)?;
    rollback_switch(root, &journal, &mut locks)
}

/// Finishes or rolls back an interrupted switch. Call at daemon start.
pub fn recover(root: &Path) -> Result<()> {
    let _switch = SWITCH_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Another account switch failed"))?;
    recover_switch_locked(root)
}

// ---------------------------------------------------------------- switching

fn write_auth_locked(
    config: &Path,
    state: &Value,
    credentials: &Value,
    locks: &mut [ConfigLock],
) -> Result<()> {
    for lock in &mut *locks {
        lock.reclaim()?;
    }
    write_credentials(config, credentials)?;
    #[cfg(test)]
    {
        tests::interrupt_auth_if_requested(config, "credentials");
        tests::before_state_write();
    }
    for lock in locks {
        lock.reclaim()?;
    }
    replace_private(&state_path(config), &serde_json::to_vec(state)?)?;
    #[cfg(test)]
    tests::interrupt_auth_if_requested(config, "state");
    Ok(())
}

#[cfg(test)]
fn write_auth(config: &Path, state: &Value, credentials: &Value) -> Result<()> {
    write_auth_locked(config, state, credentials, &mut [])
}

/// Makes `id`'s saved login the default Claude login in `personal`.
pub fn switch(root: &Path, id: Uuid, personal: &Path) -> Result<()> {
    switch_account(root, id, Vec::new(), personal.to_owned())
}

fn switch_account(
    root: &Path,
    id: Uuid,
    mut configs: Vec<PathBuf>,
    runtime: PathBuf,
) -> Result<()> {
    let _switch = SWITCH_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Another account switch failed"))?;
    recover_switch_locked(root)?;
    let accounts = list(root)?.0;
    let target = accounts
        .iter()
        .find(|account| account.id == id)
        .context("The Claude account is unavailable")?;
    ensure!(
        runtime.is_absolute() && runtime.is_dir(),
        "The Claude settings folder is unavailable"
    );
    configs.push(runtime.clone());
    let old_pointer = read_optional(&root.join("runtime-claude"))?;
    let old_selected = read_optional(&root.join("selected-claude"))?;
    // Keep saved logins separate before any runtime profile changes identity. Another account's
    // broken login must not block switching; it fails on its own when selected.
    for account in &accounts {
        let directory = root.join(account.id.to_string());
        let vault = directory.join("login");
        if !state_path(&vault).is_file()
            && let Err(error) = save_login(&directory.join("config"), &vault, &account.email)
        {
            if account.id == id {
                return Err(error);
            }
            eprintln!("Skipped saving Claude login {}: {error:#}", account.id);
        }
    }
    configs.sort();
    configs.dedup();
    // Symlink aliases share a state file but have distinct scoped Keychain identities.
    let mut locks = Vec::new();
    let mut locked_paths = HashSet::new();
    for config in &configs {
        if locked_paths.insert(config.canonicalize()?) {
            locks.push(lock_config(config)?);
        }
    }
    let originals: Vec<_> = configs
        .iter()
        .map(|config| Ok((read_object(&state_path(config))?, read_credentials(config)?)))
        .collect::<Result<_>>()?;
    for (config, (state, credentials)) in configs.iter().zip(&originals) {
        // The first switch keeps the hand-made login so it can be restored by hand later.
        if *config == runtime && !state_path(&root.join("system-default-login")).is_file() {
            let backup = root.join("system-default-login");
            private_dir(&backup)?;
            write_credentials(&backup, credentials)?;
            replace_private(&state_path(&backup), &serde_json::to_vec(state)?)?;
        }
    }
    for account in &accounts {
        let result = (|| -> Result<()> {
            let vault = root.join(account.id.to_string()).join("login");
            let saved_state = read_object(&state_path(&vault))?;
            let saved = read_credentials(&vault)?;
            // Preserve refreshes made by a live CLI, without attributing another account's tokens.
            if let Some((state, credentials)) = originals
                .iter()
                .filter(|(state, credentials)| {
                    matches_saved_login(state, credentials, &saved_state, &saved, &account.email)
                })
                .max_by_key(|(_, credentials)| {
                    credentials["claudeAiOauth"]["expiresAt"]
                        .as_i64()
                        .unwrap_or(0)
                })
                && credentials["claudeAiOauth"]["expiresAt"]
                    .as_i64()
                    .unwrap_or(0)
                    >= saved["claudeAiOauth"]["expiresAt"].as_i64().unwrap_or(0)
            {
                write_credentials(
                    &vault,
                    &json!({"claudeAiOauth": credentials["claudeAiOauth"]}),
                )?;
                replace_private(
                    &state_path(&vault),
                    &serde_json::to_vec(&json!({"oauthAccount": state["oauthAccount"]}))?,
                )?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if account.id == id {
                return Err(error);
            }
            eprintln!("Skipped refreshing Claude login {}: {error:#}", account.id);
        }
    }
    let vault = root.join(id.to_string()).join("login");
    let state = read_object(&state_path(&vault))?;
    let credentials = read_credentials(&vault)?;
    valid_login(&state, &credentials, &target.email)?;
    for lock in &mut locks {
        lock.reclaim()?;
    }
    let journal_id = Uuid::new_v4();
    let owner = ProcessIdentity::read(std::process::id())
        .context("Cannot identify the account-switch process")?;
    let backup_directory = root
        .join("claude-switch-backups")
        .join(journal_id.to_string());
    private_dir(&backup_directory)?;
    let backup_owner = BackupOwner {
        owner: owner.clone(),
        locks: locks
            .iter()
            .map(|lock| (lock.path.clone(), lock.identity.clone()))
            .collect(),
    };
    replace_private(
        &backup_directory.join("owner.json"),
        &serde_json::to_vec(&backup_owner)?,
    )?;
    sync_dir(&backup_directory)?;
    sync_dir(
        backup_directory
            .parent()
            .context("Missing recovery folder")?,
    )?;
    sync_dir(root)?;
    for (index, (_, credentials)) in originals.iter().enumerate() {
        let vault = recovery_vault(root, journal_id, index);
        private_dir(&vault)?;
        sync_dir(&backup_directory)?;
        write_credentials(&vault, credentials)?;
        #[cfg(test)]
        tests::interrupt_if_requested(root, "backup");
    }
    for lock in &mut locks {
        lock.reclaim()?;
    }
    let journal = SwitchJournal {
        owner,
        id: journal_id,
        configs: configs
            .iter()
            .zip(&originals)
            .enumerate()
            .map(|(index, (path, (state, _)))| (path.clone(), state.clone(), index))
            .collect(),
        locks: locks
            .iter()
            .map(|lock| lock_identity(&lock.path).map(|identity| (lock.path.clone(), identity)))
            .collect::<std::io::Result<_>>()?,
        runtime: old_pointer,
        selected: old_selected,
    };
    replace_private(&root.join(SWITCH_JOURNAL), &serde_json::to_vec(&journal)?)?;
    sync_dir(root)?;
    #[cfg(test)]
    tests::interrupt_if_requested(root, "journal");
    let result = (|| {
        for (config, (original, existing)) in configs.iter().zip(&originals) {
            for lock in &mut locks {
                lock.reclaim()?;
            }
            let mut next = original.clone();
            next["oauthAccount"] = state["oauthAccount"].clone();
            next["hasCompletedOnboarding"] = Value::Bool(true);
            let merged = merge_claude_credentials(
                &serde_json::to_string(existing)?,
                &serde_json::to_string(&credentials)?,
            )?;
            write_auth_locked(config, &next, &serde_json::from_str(&merged)?, &mut locks)?;
        }
        replace_private(
            &root.join("runtime-claude"),
            format!(
                "{}\n",
                runtime.to_str().context("The runtime path is not UTF-8")?
            )
            .as_bytes(),
        )?;
        #[cfg(test)]
        tests::interrupt_if_requested(root, "runtime");
        replace_private(&root.join("selected-claude"), id.to_string().as_bytes())?;
        #[cfg(test)]
        tests::interrupt_if_requested(root, "selected");
        sync_dir(root)?;
        clear_switch_journal(root, &journal)
    })();
    if let Err(error) = result {
        rollback_switch(root, &journal, &mut locks)
            .context("Account switch and rollback failed; restart to retry recovery")?;
        return Err(error);
    }
    Ok(())
}

// ---------------------------------------------------------------- account management

fn account_email(value: &str) -> Result<String> {
    let email = value.trim();
    ensure!(
        email.contains('@') && email.len() <= 320 && !email.chars().any(char::is_control),
        "The provider returned an invalid account identity"
    );
    Ok(email.to_owned())
}

fn ensure_new_email(root: &Path, email: &str) -> Result<()> {
    ensure!(
        !list(root)?
            .0
            .iter()
            .any(|account| account.email.eq_ignore_ascii_case(email)),
        "{email} is already saved as an account"
    );
    Ok(())
}

fn write_account(root: &Path, id: Uuid, email: &str) -> Result<ClaudeAccount> {
    let file = AccountFile {
        id,
        label: email.to_owned(),
        agent: "Claude".into(),
        shared_claude_history: false,
        authenticated: true,
    };
    replace_private(
        &root.join(id.to_string()).join("account.json"),
        &serde_json::to_vec(&file)?,
    )?;
    let plan = read_object(&state_path(&root.join(id.to_string()).join("login")))
        .ok()
        .and_then(|state| {
            state["oauthAccount"]["planDisplayName"]
                .as_str()
                .map(str::to_owned)
        });
    Ok(ClaudeAccount {
        id,
        email: email.to_owned(),
        plan,
    })
}

/// Deletes an account folder and its scoped Keychain items. Refuses the selected account.
pub fn remove(root: &Path, id: Uuid) -> Result<()> {
    let _switch = SWITCH_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Another account switch failed"))?;
    recover_switch_locked(root)?;
    ensure!(
        selected(root) != Some(id),
        "This account is in use. Switch to another account before removing it."
    );
    let directory = root.join(id.to_string());
    ensure!(
        directory.join("account.json").is_file(),
        "The Claude account is unavailable"
    );
    // Keychain first: a failure here leaves the account intact.
    remove_credential_item(&directory.join("config"))?;
    remove_credential_item(&directory.join("login"))?;
    fs::remove_dir_all(&directory)?;
    Ok(())
}

/// Creates the folder for a new sign-in and returns its id and `CLAUDE_CONFIG_DIR`.
pub fn begin_login(root: &Path, personal: &Path) -> Result<(Uuid, PathBuf)> {
    ensure!(
        root.is_absolute(),
        "The accounts folder must be an absolute path"
    );
    let id = Uuid::new_v4();
    let directory = root.join(id.to_string());
    let config = directory.join("config");
    private_dir(&config)?;
    if let Err(error) = share_claude_preferences(&config, personal) {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }
    Ok((id, config))
}

/// The shell line that signs in to a new account without touching the default login.
pub fn login_command(config: &Path) -> Result<String> {
    let config = config.to_str().context("The account path is not UTF-8")?;
    Ok(format!(
        "env -u ANTHROPIC_API_KEY -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_BASE_URL \
         -u CLAUDE_CODE_OAUTH_TOKEN -u CLAUDE_CODE_USE_BEDROCK -u CLAUDE_CODE_USE_VERTEX \
         -u CLAUDE_CODE_USE_FOUNDRY -u ANTHROPIC_PROFILE -u ANTHROPIC_FEDERATION_RULE_ID \
         CLAUDE_CONFIG_DIR={} claude auth login --claudeai",
        shell_words::quote(config)
    ))
}

/// `Ok(None)` while the sign-in has no email and tokens yet. Otherwise saves the account.
pub fn finish_login(root: &Path, id: Uuid) -> Result<Option<ClaudeAccount>> {
    let directory = root.join(id.to_string());
    let config = directory.join("config");
    ensure!(config.is_dir(), "No sign-in is in progress for {id}");
    ensure!(
        !directory.join("account.json").exists(),
        "This sign-in was already saved as an account"
    );
    let state = read_object(&state_path(&config))?;
    let Some(email) = state["oauthAccount"]["emailAddress"]
        .as_str()
        .filter(|email| !email.is_empty())
    else {
        return Ok(None);
    };
    let credentials = read_credentials(&config)?;
    let has = |key: &str| {
        credentials["claudeAiOauth"][key]
            .as_str()
            .is_some_and(|token| !token.is_empty())
    };
    if !has("accessToken") || !has("refreshToken") {
        return Ok(None);
    }
    let email = account_email(email)?;
    if let Err(error) = ensure_new_email(root, &email) {
        let _ = remove_credential_item(&config);
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }
    save_login(&config, &directory.join("login"), &email)?;
    write_account(root, id, &email).map(Some)
}

/// Saves the live login in `personal` as a new account without changing it.
pub fn save_current(root: &Path, personal: &Path) -> Result<ClaudeAccount> {
    let _switch = SWITCH_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Another account switch failed"))?;
    recover_switch_locked(root)?;
    let state = read_object(&state_path(personal))?;
    let credentials = read_credentials(personal)?;
    let email = account_email(
        state["oauthAccount"]["emailAddress"]
            .as_str()
            .context("The current Claude login has no email. Sign in to Claude first.")?,
    )?;
    valid_login(&state, &credentials, &email)?;
    ensure_new_email(root, &email)?;
    let id = Uuid::new_v4();
    let directory = root.join(id.to_string());
    let result = (|| {
        private_dir(&directory.join("config"))?;
        save_login(personal, &directory.join("login"), &email)?;
        write_account(root, id, &email)
    })();
    if result.is_err() {
        let _ = remove_credential_item(&directory.join("login"));
        let _ = fs::remove_dir_all(&directory);
    }
    result
}

#[cfg(test)]
#[path = "claude_accounts_tests.rs"]
mod tests;
