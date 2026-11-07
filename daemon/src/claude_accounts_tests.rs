use super::*;

// Tests use file-backed credentials (see `read_credentials`) and temp folders only: they never
// touch the Keychain, the real HOME, `~/.warp-oss` or `~/.claude*`.

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("naru-claude-accounts-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    static BEFORE_STATE_WRITE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = Default::default();
}

pub(super) fn before_state_write() {
    BEFORE_STATE_WRITE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

pub(super) fn interrupt_if_requested(root: &Path, stage: &str) {
    if std::env::var_os("NARU_TEST_SWITCH_ROOT").as_deref() == Some(root.as_os_str())
        && std::env::var("NARU_TEST_SWITCH_STAGE").as_deref() == Ok(stage)
    {
        assert_eq!(
            unsafe { libc::kill(std::process::id() as libc::pid_t, libc::SIGKILL) },
            0
        );
        // The test runs on a worker; a normal exit can beat signal delivery on the main thread.
        loop {
            std::thread::park();
        }
    }
}

pub(super) fn interrupt_auth_if_requested(config: &Path, stage: &str) {
    if let Some(root) = std::env::var_os("NARU_TEST_SWITCH_ROOT") {
        let root = PathBuf::from(root);
        if config.starts_with(&root) {
            interrupt_if_requested(&root, stage);
        }
    }
}

/// An account folder with account.json and a signed-in config; no login vault yet.
fn make_account(root: &Path, label: &str) -> (Uuid, PathBuf) {
    let id = Uuid::new_v4();
    let config = root.join(id.to_string()).join("config");
    private_dir(&config).unwrap();
    replace_private(
        &root.join(id.to_string()).join("account.json"),
        &serde_json::to_vec(&AccountFile {
            id,
            label: label.into(),
            agent: "Claude".into(),
            shared_claude_history: false,
            authenticated: true,
        })
        .unwrap(),
    )
    .unwrap();
    (id, config)
}

fn sign_in(config: &Path, email: &str) {
    write_auth(
        config,
        &json!({"oauthAccount":{"emailAddress":email,"planDisplayName":"Max"}}),
        &json!({"claudeAiOauth":{"accessToken":email,"refreshToken":email,"expiresAt":100}}),
    )
    .unwrap();
}

fn token(config: &Path) -> Value {
    read_credentials(config).unwrap()["claudeAiOauth"]["accessToken"].clone()
}

#[test]
fn credential_write_rechecks_lock_before_replacing_settings() {
    let temp = Temp::new();
    let config = temp.path().to_owned();
    replace_private(&state_path(&config), br#"{"theme":"old"}"#).unwrap();
    let lock = lock_config(&config).unwrap();
    let lock_path = lock.path.clone();
    let state = state_path(&config);
    let updated_state = state.clone();
    BEFORE_STATE_WRITE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::remove_dir(&lock_path).unwrap();
            fs::create_dir(&lock_path).unwrap();
            fs::File::open(&lock_path)
                .unwrap()
                .set_modified(std::time::SystemTime::now() + Duration::from_secs(1))
                .unwrap();
            replace_private(&updated_state, br#"{"theme":"external-update"}"#).unwrap();
        }))
    });
    let result = write_auth_locked(&config, &json!({"theme":"stale"}), &json!({}), &mut [lock]);
    assert!(result.is_err());
    assert_eq!(read_object(&state).unwrap()["theme"], "external-update");
}

#[test]
fn lost_lock_prevents_any_credential_write_and_keeps_replacement_lock() {
    let config = Temp::new();
    write_credentials(
        config.path(),
        &json!({"claudeAiOauth":{"accessToken":"keep"}}),
    )
    .unwrap();
    let lock = lock_config(config.path()).unwrap();
    let lock_path = lock.path.clone();
    let previous_mtime = lock_mtime(&lock_path).unwrap();
    fs::remove_dir(&lock_path).unwrap();
    fs::create_dir(&lock_path).unwrap();
    fs::File::open(&lock_path)
        .unwrap()
        .set_modified(previous_mtime)
        .unwrap();
    assert!(
        write_auth_locked(
            config.path(),
            &json!({}),
            &json!({"claudeAiOauth":{"accessToken":"wrong"}}),
            &mut [lock]
        )
        .is_err()
    );
    assert_eq!(token(config.path()), "keep");
    assert!(lock_path.is_dir());
}

#[test]
fn held_settings_lock_is_refreshed_while_the_caller_is_blocked() {
    let config = Temp::new();
    let lock = lock_config(config.path()).unwrap();
    let initial_stamp = lock_mtime(&lock.path).unwrap();
    let deadline = std::time::Instant::now();
    // Backdating mtime on APFS also backdates birthtime, which intentionally loses identity.
    while lock_mtime(&lock.path).unwrap() <= initial_stamp {
        assert!(deadline.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(lock_identity(&lock.path).unwrap() == lock.identity);
    let path = lock.path.clone();
    drop(lock);
    assert!(!path.exists());
}

#[test]
fn default_claude_uses_the_legacy_state_file_and_profiles_are_scoped() {
    let home = Path::new("/home/someone");
    let default = home.join(".claude");
    assert!(is_default_claude_in(&default, Some(home), false));
    assert!(!is_default_claude_in(&default, Some(home), true));
    assert!(!is_default_claude_in(
        &home.join("other"),
        Some(home),
        false
    ));
    assert!(!is_default_claude_in(&default, None, false));
    assert_eq!(default.with_extension("json"), home.join(".claude.json"));
    let service = claude_keychain_service(Path::new("/some/config")).unwrap();
    assert!(service.starts_with("Claude Code-credentials-"));
    assert_eq!(service.len(), "Claude Code-credentials-".len() + 8);
}

#[test]
fn refreshed_tokens_require_the_same_account_and_organization() {
    let state = json!({"oauthAccount":{"emailAddress":"first@test","accountUuid":"first","organizationUuid":"org"}});
    let saved = json!({"claudeAiOauth":{"accessToken":"old","refreshToken":"old","expiresAt":100}});
    let mut refreshed =
        json!({"claudeAiOauth":{"accessToken":"new","refreshToken":"new","expiresAt":200}});
    assert!(matches_saved_login(
        &state,
        &refreshed,
        &state,
        &saved,
        "first@test"
    ));
    let mut other = state.clone();
    other["oauthAccount"]["accountUuid"] = json!("other");
    assert!(!matches_saved_login(
        &other,
        &refreshed,
        &state,
        &saved,
        "first@test"
    ));
    other = state.clone();
    other["oauthAccount"]["organizationUuid"] = json!("other-org");
    assert!(!matches_saved_login(
        &other,
        &refreshed,
        &state,
        &saved,
        "first@test"
    ));
    refreshed["claudeAiOauth"]["email"] = json!("other@test");
    assert!(!matches_saved_login(
        &state,
        &refreshed,
        &state,
        &saved,
        "first@test"
    ));
    refreshed["claudeAiOauth"]
        .as_object_mut()
        .unwrap()
        .remove("email");
    let no_identity = json!({"oauthAccount":{"emailAddress":"first@test"}});
    assert!(!matches_saved_login(
        &no_identity,
        &refreshed,
        &state,
        &saved,
        "first@test"
    ));
    assert!(matches_saved_login(
        &no_identity,
        &saved,
        &state,
        &saved,
        "first@test"
    ));
}

#[test]
fn live_switch_keeps_settings_history_and_account_logins_and_reads_back_refreshes() {
    let temp = Temp::new();
    let root = temp.path().join("accounts");
    let mut configs = Vec::new();
    let mut ids = Vec::new();
    for label in ["first@test", "second@test"] {
        let (id, config) = make_account(&root, label);
        replace_private(
            &state_path(&config),
            &serde_json::to_vec(&json!({"oauthAccount":{"emailAddress":label},"projects":{"/test":{"hasTrustDialogAccepted":true}}})).unwrap(),
        )
        .unwrap();
        replace_private(
            &config.join(".credentials.json"),
            &serde_json::to_vec(&json!({"claudeAiOauth":{"accessToken":label,"refreshToken":label,"expiresAt":100},"mcpOAuth":{"keep":"private"}})).unwrap(),
        )
        .unwrap();
        fs::write(config.join("history.jsonl"), "existing conversation").unwrap();
        ids.push(id);
        configs.push(config);
    }
    let (first, second) = (ids[0], ids[1]);
    let personal = temp.path().join("personal");
    fs::create_dir(&personal).unwrap();
    fs::write(
        state_path(&personal),
        br#"{"theme":"dark","projects":{"/original":{"hasTrustDialogAccepted":true}}}"#,
    )
    .unwrap();
    fs::write(
        personal.join("settings.json"),
        br#"{"statusLine":{"command":"my-status"}}"#,
    )
    .unwrap();
    fs::write(personal.join("history.jsonl"), "personal history").unwrap();
    let personal_settings = fs::read(personal.join("settings.json")).unwrap();
    configs.push(personal.clone());
    switch_account(&root, second, configs.clone(), personal.clone()).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("runtime-claude")).unwrap(),
        format!("{}\n", personal.display())
    );
    assert_eq!(
        fs::read(personal.join("settings.json")).unwrap(),
        personal_settings
    );
    assert_eq!(
        fs::read_to_string(personal.join("history.jsonl")).unwrap(),
        "personal history"
    );
    let personal_state = read_object(&state_path(&personal)).unwrap();
    assert_eq!(personal_state["theme"], "dark");
    assert_eq!(
        personal_state["projects"]["/original"]["hasTrustDialogAccepted"],
        true
    );
    assert_eq!(token(&personal), "second@test");
    assert_eq!(selected(&root), Some(second));
    assert_eq!(token(&configs[0]), "second@test");
    assert_eq!(
        read_credentials(&configs[0]).unwrap()["mcpOAuth"]["keep"],
        "private"
    );
    assert_eq!(
        fs::read_to_string(configs[0].join("history.jsonl")).unwrap(),
        "existing conversation"
    );
    assert_eq!(
        read_object(&state_path(&configs[0])).unwrap()["projects"]["/test"]["hasTrustDialogAccepted"],
        true
    );
    // The first switch kept the hand-made default login.
    assert!(state_path(&root.join("system-default-login")).is_file());
    let mut refreshed = read_credentials(&configs[0]).unwrap();
    refreshed["claudeAiOauth"]["accessToken"] = json!("refreshed-second");
    refreshed["claudeAiOauth"]["expiresAt"] = json!(200);
    write_credentials(&configs[0], &refreshed).unwrap();
    switch_account(&root, first, configs.clone(), personal.clone()).unwrap();
    assert_eq!(token(&configs[0]), "first@test");
    switch_account(&root, second, configs.clone(), personal.clone()).unwrap();
    assert_eq!(token(&configs[0]), "refreshed-second");
    fs::create_dir(state_path(&configs[0]).with_extension("json.lock")).unwrap();
    assert!(switch_account(&root, first, configs.clone(), personal).is_err());
    assert_eq!(selected(&root), Some(second));
    assert_eq!(token(&configs[0]), "refreshed-second");
}

#[test]
fn broken_login_of_another_account_does_not_block_switching_or_get_overwritten() {
    let temp = Temp::new();
    let root = temp.path().join("accounts");
    let mut accounts = Vec::new();
    for (label, signed_in_as) in [("good@test", "good@test"), ("broken@test", "good@test")] {
        let (id, config) = make_account(&root, label);
        replace_private(
            &state_path(&config),
            &serde_json::to_vec(&json!({"oauthAccount":{"emailAddress":signed_in_as}})).unwrap(),
        )
        .unwrap();
        replace_private(&config.join(".credentials.json"), &serde_json::to_vec(&json!({"claudeAiOauth":{"accessToken":label,"refreshToken":label,"expiresAt":100}})).unwrap()).unwrap();
        accounts.push((id, config));
    }
    let personal = temp.path().join("personal");
    fs::create_dir(&personal).unwrap();
    let broken_before = fs::read(accounts[1].1.join(".credentials.json")).unwrap();

    switch(&root, accounts[0].0, &personal).unwrap();
    assert_eq!(selected(&root), Some(accounts[0].0));
    assert_eq!(token(&personal), "good@test");
    assert_eq!(
        fs::read(accounts[1].1.join(".credentials.json")).unwrap(),
        broken_before
    );
    assert!(switch(&root, accounts[1].0, &personal).is_err());
    assert_eq!(selected(&root), Some(accounts[0].0));
}

#[test]
fn corrupted_unselected_vault_does_not_block_or_get_overwritten() {
    for file in [".claude.json", ".credentials.json"] {
        let temp = Temp::new();
        let root = temp.path().join("accounts");
        let mut accounts = Vec::new();
        for label in ["good@test", "broken@test"] {
            let (id, config) = make_account(&root, label);
            sign_in(&config, label);
            let vault = root.join(id.to_string()).join("login");
            save_login(&config, &vault, label).unwrap();
            accounts.push((id, vault));
        }
        let corrupt = accounts[1].1.join(file);
        fs::write(&corrupt, "broken JSON").unwrap();
        let runtime = temp.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        switch(&root, accounts[0].0, &runtime).unwrap();
        assert_eq!(token(&runtime), "good@test");
        assert_eq!(fs::read_to_string(&corrupt).unwrap(), "broken JSON");
        assert!(switch(&root, accounts[1].0, &runtime).is_err());
        assert_eq!(selected(&root), Some(accounts[0].0));
        assert_eq!(token(&runtime), "good@test");
    }
}

#[test]
fn missing_or_mismatched_login_does_not_replace_running_authentication() {
    let temp = Temp::new();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(
        state_path(&config),
        br#"{"oauthAccount":{"emailAddress":"other@test"}}"#,
    )
    .unwrap();
    fs::write(
        config.join(".credentials.json"),
        br#"{"claudeAiOauth":{"accessToken":"keep"}}"#,
    )
    .unwrap();
    assert!(save_login(&config, &temp.path().join("login"), "expected@test").is_err());
    assert!(!temp.path().join("login").exists());
    assert_eq!(token(&config), "keep");
}

#[test]
fn abandoned_settings_lock_is_taken_over_and_a_lost_lock_stops_the_switch() {
    let config = Temp::new();
    let lock_path = state_path(config.path()).with_extension("json.lock");
    fs::create_dir(&lock_path).unwrap();
    fs::File::open(&lock_path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
        .unwrap();
    let mut lock = lock_config(config.path()).unwrap();
    lock.reclaim().unwrap();

    // Claude treats a held lock as stale and recreates it as its own.
    fs::remove_dir(&lock_path).unwrap();
    fs::create_dir(&lock_path).unwrap();
    fs::File::open(&lock_path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert!(lock.reclaim().is_err());
    drop(lock);
    assert!(lock_path.is_dir(), "Claude's own lock must survive");
}

#[test]
fn interrupted_switch_child() {
    let Some(root) = std::env::var_os("NARU_TEST_SWITCH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let id = Uuid::parse_str(&std::env::var("NARU_TEST_SWITCH_TARGET").unwrap()).unwrap();
    let configs = list(&root)
        .unwrap()
        .0
        .iter()
        .map(|account| root.join(account.id.to_string()).join("config"))
        .collect();
    switch_account(&root, id, configs, root.join("runtime")).unwrap();
    panic!("The requested crash checkpoint was not reached");
}

#[test]
fn interrupted_recovery_child() {
    let Some(root) = std::env::var_os("NARU_TEST_SWITCH_ROOT") else {
        return;
    };
    recover(&PathBuf::from(root)).unwrap();
    panic!("The recovery crash checkpoint was not reached");
}

#[test]
fn killed_switch_recovers_each_partial_write_without_losing_history() {
    use std::os::unix::process::ExitStatusExt;
    for stage in [
        "backup",
        "journal",
        "credentials",
        "state",
        "runtime",
        "selected",
        "committed",
    ] {
        let temp = Temp::new();
        let root = temp.path().join("accounts");
        let mut accounts = Vec::new();
        for label in ["first@test", "second@test"] {
            let (id, config) = make_account(&root, label);
            write_auth(&config, &json!({"oauthAccount":{"emailAddress":label},"theme":"dark"}),
                &json!({"claudeAiOauth":{"accessToken":label,"refreshToken":label},"mcpOAuth":{"keep":"private"}})).unwrap();
            fs::write(config.join("history.jsonl"), "saved conversation\n").unwrap();
            accounts.push((id, config));
        }
        let runtime = root.join("runtime");
        fs::create_dir(&runtime).unwrap();
        write_auth(
            &runtime,
            &json!({"oauthAccount":{"emailAddress":"first@test"},"theme":"dark"}),
            &json!({"claudeAiOauth":{"accessToken":"first@test","refreshToken":"first@test"}}),
        )
        .unwrap();
        replace_private(
            &root.join("runtime-claude"),
            runtime.to_str().unwrap().as_bytes(),
        )
        .unwrap();
        replace_private(
            &root.join("selected-claude"),
            accounts[0].0.to_string().as_bytes(),
        )
        .unwrap();
        let originals: Vec<_> = accounts
            .iter()
            .map(|(_, path)| {
                (
                    path.clone(),
                    read_object(&state_path(path)).unwrap(),
                    read_credentials(path).unwrap(),
                )
            })
            .collect();
        let mut runtime_state = read_object(&state_path(&runtime)).unwrap();
        let mut runtime_login = read_credentials(&runtime).unwrap();
        let test = format!(
            "{}::interrupted_switch_child",
            module_path!().split_once("::").unwrap().1
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture"])
            .env("NARU_TEST_SWITCH_ROOT", &root)
            .env("NARU_TEST_SWITCH_STAGE", stage)
            .env("NARU_TEST_SWITCH_TARGET", accounts[1].0.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.signal(),
            Some(libc::SIGKILL),
            "Crash stage {stage} was not reached: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            root.join(SWITCH_JOURNAL).is_file(),
            !matches!(stage, "backup" | "committed")
        );
        if stage == "committed" {
            recover(&root).unwrap();
            assert_eq!(selected(&root), Some(accounts[1].0));
            assert_eq!(token(&runtime), "second@test");
            assert_eq!(
                fs::read_dir(root.join("claude-switch-backups"))
                    .unwrap()
                    .count(),
                0
            );
            for (_, config) in &accounts {
                assert_eq!(
                    fs::read_to_string(config.join("history.jsonl")).unwrap(),
                    "saved conversation\n"
                );
            }
            continue;
        }
        if stage == "credentials" {
            let test = format!(
                "{}::interrupted_recovery_child",
                module_path!().split_once("::").unwrap().1
            );
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test, "--nocapture"])
                .env("NARU_TEST_SWITCH_ROOT", &root)
                .env("NARU_TEST_SWITCH_STAGE", "credentials")
                .output()
                .unwrap();
            assert_eq!(output.status.signal(), Some(libc::SIGKILL));
            let journal: SwitchJournal =
                serde_json::from_slice(&fs::read(root.join(SWITCH_JOURNAL)).unwrap()).unwrap();
            assert!(!journal.owner.is_current());
            assert!(
                journal
                    .locks
                    .iter()
                    .all(|(path, identity)| lock_identity(path).ok().as_ref() == Some(identity))
            );
        }
        if stage != "backup" {
            let journal_bytes = fs::read_to_string(root.join(SWITCH_JOURNAL)).unwrap();
            assert!(
                !journal_bytes.contains("accessToken"),
                "Recovery journal must not contain tokens"
            );
        }
        let mut updated = read_object(&state_path(&runtime)).unwrap();
        updated["theme"] = json!("external-update");
        replace_private(
            &state_path(&runtime),
            &serde_json::to_vec(&updated).unwrap(),
        )
        .unwrap();
        runtime_state["theme"] = json!("external-update");
        let mut updated_login = read_credentials(&runtime).unwrap();
        updated_login["mcpOAuth"] = json!({"external":"updated"});
        write_credentials(&runtime, &updated_login).unwrap();
        runtime_login["mcpOAuth"] = json!({"external":"updated"});
        recover(&root).unwrap();
        assert_eq!(
            fs::read_dir(root.join("claude-switch-backups"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(selected(&root), Some(accounts[0].0));
        assert!(!root.join(SWITCH_JOURNAL).exists());
        assert_eq!(read_object(&state_path(&runtime)).unwrap(), runtime_state);
        assert_eq!(read_credentials(&runtime).unwrap(), runtime_login);
        for (path, state, login) in &originals {
            assert_eq!(read_object(&state_path(path)).unwrap(), *state);
            assert_eq!(read_credentials(path).unwrap(), *login);
            assert_eq!(
                fs::read_to_string(path.join("history.jsonl")).unwrap(),
                "saved conversation\n"
            );
        }
        switch_account(
            &root,
            accounts[1].0,
            originals.iter().map(|(path, _, _)| path.clone()).collect(),
            runtime,
        )
        .unwrap();
        assert_eq!(selected(&root), Some(accounts[1].0));
    }
}

#[test]
fn orphan_cleanup_keeps_a_live_owner_and_an_active_journal_backup() {
    let temp = Temp::new();
    let root = temp.path();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let live_owner = ProcessIdentity::read(child.id()).unwrap();
    let current_owner = ProcessIdentity::read(std::process::id()).unwrap();
    let live = Uuid::new_v4();
    let active = Uuid::new_v4();
    let orphan = Uuid::new_v4();
    for (id, owner) in [
        (live, &live_owner),
        (active, &current_owner),
        (orphan, &current_owner),
    ] {
        let vault = recovery_vault(root, id, 0);
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join(".credentials.json"), "private").unwrap();
        replace_private(
            &vault.parent().unwrap().join("owner.json"),
            &serde_json::to_vec(&BackupOwner {
                owner: owner.clone(),
                locks: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
    }
    clean_orphaned_backups(root, Some(active)).unwrap();
    assert!(recovery_vault(root, live, 0).exists());
    assert!(recovery_vault(root, active, 0).exists());
    assert!(!recovery_vault(root, orphan, 0).exists());
    child.kill().unwrap();
    child.wait().unwrap();
    clean_orphaned_backups(root, Some(active)).unwrap();
    assert!(!recovery_vault(root, live, 0).exists());
    assert!(recovery_vault(root, active, 0).exists());
}

#[test]
fn journal_and_account_json_keep_the_old_apps_shapes() {
    let owner = ProcessIdentity::read(std::process::id()).unwrap();
    let value = serde_json::to_value(&owner).unwrap();
    for key in ["pid", "parent", "group", "started", "boot_id"] {
        assert!(value.get(key).is_some(), "missing {key}");
    }
    let journal = serde_json::to_value(SwitchJournal {
        owner,
        id: Uuid::new_v4(),
        configs: Vec::new(),
        locks: Vec::new(),
        runtime: None,
        selected: None,
    })
    .unwrap();
    for key in ["owner", "id", "configs", "locks", "runtime", "selected"] {
        assert!(journal.get(key).is_some(), "missing {key}");
    }
    let temp = Temp::new();
    let (id, _) = make_account(temp.path(), "a@test");
    let file: Value = serde_json::from_slice(
        &fs::read(temp.path().join(id.to_string()).join("account.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(file["agent"], "Claude");
    assert_eq!(file["label"], "a@test");
    assert_eq!(file["authenticated"], true);
    assert_eq!(file["shared_claude_history"], false);
}

#[test]
fn list_reports_claude_accounts_with_plan_and_skips_pending_and_other_providers() {
    let temp = Temp::new();
    let root = temp.path();
    let (zed, zed_config) = make_account(root, "Zed@test");
    let (alpha, alpha_config) = make_account(root, "alpha@test");
    sign_in(&zed_config, "Zed@test");
    save_login(
        &zed_config,
        &root.join(zed.to_string()).join("login"),
        "Zed@test",
    )
    .unwrap();
    // A sign-in in progress has no account.json and is not an account.
    fs::create_dir_all(root.join(Uuid::new_v4().to_string()).join("config")).unwrap();
    // Another provider's folder is ignored; a mismatched folder is reported.
    let other = Uuid::new_v4();
    fs::create_dir_all(root.join(other.to_string())).unwrap();
    fs::write(
        root.join(other.to_string()).join("account.json"),
        serde_json::to_vec(&json!({"id":other,"label":"c@test","agent":"Codex"})).unwrap(),
    )
    .unwrap();
    let bad = Uuid::new_v4();
    fs::create_dir_all(root.join(bad.to_string())).unwrap();
    fs::write(
        root.join(bad.to_string()).join("account.json"),
        serde_json::to_vec(&json!({"id":Uuid::new_v4(),"label":"x","agent":"Claude"})).unwrap(),
    )
    .unwrap();
    let _ = alpha_config;
    let (accounts, errors) = list(root).unwrap();
    assert_eq!(
        accounts,
        vec![
            ClaudeAccount {
                id: alpha,
                email: "alpha@test".into(),
                plan: None
            },
            ClaudeAccount {
                id: zed,
                email: "Zed@test".into(),
                plan: Some("Max".into())
            },
        ]
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(list(&root.join("missing")).unwrap().0.len(), 0);
}

#[test]
fn current_email_reads_only_the_state_file() {
    let temp = Temp::new();
    assert_eq!(current_email(temp.path()), None);
    fs::write(
        state_path(temp.path()),
        br#"{"oauthAccount":{"emailAddress":"me@test"}}"#,
    )
    .unwrap();
    assert_eq!(current_email(temp.path()).as_deref(), Some("me@test"));
    fs::write(state_path(temp.path()), "not json").unwrap();
    assert_eq!(current_email(temp.path()), None);
}

#[test]
fn login_command_quotes_the_config_path() {
    let command = login_command(Path::new("/tmp/it's a $(path)/config")).unwrap();
    let words = shell_words::split(&command).unwrap();
    assert_eq!(words[0], "env");
    assert!(words.contains(&"CLAUDE_CONFIG_DIR=/tmp/it's a $(path)/config".to_owned()));
    assert_eq!(
        &words[words.len() - 4..],
        ["claude", "auth", "login", "--claudeai"]
    );
}

#[test]
fn sign_in_flow_adds_an_account_and_rejects_a_duplicate_email() {
    let temp = Temp::new();
    let root = temp.path().join("accounts");
    let personal = temp.path().join("personal");
    fs::create_dir_all(&personal).unwrap();
    fs::write(personal.join("settings.json"), br#"{"statusLine":"x"}"#).unwrap();

    let (id, config) = begin_login(&root, &personal).unwrap();
    assert!(config.ends_with("config"));
    assert!(
        fs::read_to_string(config.join("settings.json"))
            .unwrap()
            .contains("statusLine")
    );
    // Not signed in yet.
    assert_eq!(finish_login(&root, id).unwrap(), None);
    // Email in state but no tokens yet.
    let mut state = read_object(&state_path(&config)).unwrap();
    state["oauthAccount"] = json!({"emailAddress":"new@test","planDisplayName":"Pro"});
    replace_private(&state_path(&config), &serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(finish_login(&root, id).unwrap(), None);
    replace_private(
        &config.join(".credentials.json"),
        &serde_json::to_vec(&json!({"claudeAiOauth":{"accessToken":"a","refreshToken":"r"}}))
            .unwrap(),
    )
    .unwrap();
    let account = finish_login(&root, id).unwrap().unwrap();
    assert_eq!(account.email, "new@test");
    assert_eq!(account.plan.as_deref(), Some("Pro"));
    assert_eq!(list(&root).unwrap().0, vec![account]);
    assert!(finish_login(&root, id).is_err());

    // A second sign-in as the same email is refused and cleaned up.
    let (again, config) = begin_login(&root, &personal).unwrap();
    let mut state = read_object(&state_path(&config)).unwrap();
    state["oauthAccount"] = json!({"emailAddress":"NEW@test"});
    replace_private(&state_path(&config), &serde_json::to_vec(&state).unwrap()).unwrap();
    replace_private(
        &config.join(".credentials.json"),
        &serde_json::to_vec(&json!({"claudeAiOauth":{"accessToken":"a","refreshToken":"r"}}))
            .unwrap(),
    )
    .unwrap();
    let error = finish_login(&root, again).unwrap_err().to_string();
    assert!(error.contains("already saved"), "{error}");
    assert!(!root.join(again.to_string()).exists());
    assert_eq!(list(&root).unwrap().0.len(), 1);
}

#[test]
fn remove_refuses_the_selected_account_and_deletes_the_rest() {
    let temp = Temp::new();
    let root = temp.path().join("accounts");
    let personal = temp.path().join("personal");
    fs::create_dir_all(&personal).unwrap();
    let mut ids = Vec::new();
    for label in ["a@test", "b@test"] {
        let (id, config) = make_account(&root, label);
        sign_in(&config, label);
        ids.push(id);
    }
    switch(&root, ids[0], &personal).unwrap();
    assert!(remove(&root, ids[0]).is_err());
    assert!(root.join(ids[0].to_string()).exists());
    remove(&root, ids[1]).unwrap();
    assert!(!root.join(ids[1].to_string()).exists());
    assert_eq!(list(&root).unwrap().0.len(), 1);
    assert!(remove(&root, ids[1]).is_err());
}

#[test]
fn save_current_stores_the_live_login_without_touching_it() {
    let temp = Temp::new();
    let root = temp.path().join("accounts");
    let personal = temp.path().join("personal");
    fs::create_dir_all(&personal).unwrap();
    // No login yet.
    assert!(save_current(&root, &personal).is_err());
    assert!(list(&root).unwrap().0.is_empty());
    sign_in(&personal, "hand@test");
    let state_before = fs::read(state_path(&personal)).unwrap();
    let credentials_before = fs::read(personal.join(".credentials.json")).unwrap();
    let account = save_current(&root, &personal).unwrap();
    assert_eq!(account.email, "hand@test");
    assert_eq!(account.plan.as_deref(), Some("Max"));
    assert_eq!(fs::read(state_path(&personal)).unwrap(), state_before);
    assert_eq!(
        fs::read(personal.join(".credentials.json")).unwrap(),
        credentials_before
    );
    // The vault is what switch() needs.
    let vault = root.join(account.id.to_string()).join("login");
    assert_eq!(token(&vault), "hand@test");
    assert!(
        save_current(&root, &personal)
            .unwrap_err()
            .to_string()
            .contains("already saved")
    );
    assert_eq!(list(&root).unwrap().0.len(), 1);

    // Incomplete live login (email but no refresh token) is refused.
    sign_in(&personal, "other@test");
    replace_private(
        &personal.join(".credentials.json"),
        &serde_json::to_vec(&json!({"claudeAiOauth":{"accessToken":"a"}})).unwrap(),
    )
    .unwrap();
    assert!(save_current(&root, &personal).is_err());
    assert_eq!(list(&root).unwrap().0.len(), 1);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);

    // A saved account can be switched to afterwards.
    let (other, config) = make_account(&root, "other@test");
    sign_in(&config, "other@test");
    switch(&root, other, &personal).unwrap();
    switch(&root, account.id, &personal).unwrap();
    assert_eq!(token(&personal), "hand@test");
}

#[cfg(target_os = "macos")]
mod keychain_helpers {
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::{Command, Output};

    use super::super::keychain::{parse_read, run_for_test, write_command};

    #[test]
    fn only_a_missing_keychain_item_allows_a_fallback() {
        let output = |code, value: &str| Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: value.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(parse_read(output(44, "")).unwrap(), None);
        assert!(parse_read(output(36, "")).is_err());
        assert!(parse_read(output(0, "")).is_err());
        assert_eq!(
            parse_read(output(0, "credentials\n")).unwrap().as_deref(),
            Some("credentials")
        );
    }

    #[test]
    fn credentials_are_quoted_for_stdin_without_command_injection() {
        let value = r#"{"token":"single' double\" $(no-command)"}"#;
        let input = write_command("service", "user", value).unwrap();
        assert_eq!(shell_words::split(&input).unwrap().last().unwrap(), value);
        assert!(write_command("service\nquit", "user", "value").is_err());
        assert!(write_command("service", "user", &"x".repeat(4096)).is_err());
        let output = run_for_test(Command::new("/bin/cat"), Some(&input)).unwrap();
        assert_eq!(output.stdout, input.as_bytes());
    }

    #[test]
    fn stalled_keychain_commands_are_killed_after_three_seconds() {
        let start = std::time::Instant::now();
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        assert!(run_for_test(command, None).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(6));
    }
}
