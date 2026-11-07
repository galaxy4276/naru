use super::*;
use serde_json::json;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn jwt(claims: Value) -> String {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("e30.{payload}.sig")
}

fn codex_auth(account: &str, email: &str, access: &str) -> String {
    let claims = json!({
        "email": email,
        "https://api.openai.com/auth": { "chatgpt_plan_type": "pro", "chatgpt_account_id": account },
    });
    json!({
        "auth_mode": "chatgpt",
        "tokens": { "id_token": jwt(claims), "access_token": access, "account_id": account },
    })
    .to_string()
}

fn agy_token(email: &str, token: &str) -> String {
    json!({ "auth_method": "oauth", "id_token_email": email, "token": token }).to_string()
}

fn write(path: &Path, text: &str) {
    write_private(path, text.as_bytes()).unwrap();
}

#[test]
fn logins_name_their_account() {
    let codex =
        codex_login(&serde_json::from_str(&codex_auth("acct-1", "a@x.com", "t")).unwrap()).unwrap();
    assert_eq!(codex.key.as_deref(), Some("acct-1|a@x.com"));
    assert_eq!(
        (codex.email.as_deref(), codex.plan.as_deref()),
        (Some("a@x.com"), Some("pro"))
    );
    assert!(codex_login(&json!({ "OPENAI_API_KEY": "sk" })).is_none());

    let grok = grok_login(&json!({ "https://auth.x.ai": {
        "key": jwt(json!({ "sub": "u-9", "email": "g@x.com" })) } }))
    .unwrap();
    assert_eq!(
        (grok.key.as_deref(), grok.email.as_deref()),
        (Some("u-9"), Some("g@x.com"))
    );
    assert!(grok_login(&json!({ "https://auth.x.ai": { "key": "opaque" } })).is_none());

    let agy = agy_login(&serde_json::from_str(&agy_token("Me@Gmail.com", "t")).unwrap()).unwrap();
    assert_eq!(agy.key.as_deref(), Some("me@gmail.com"));
}

#[test]
fn switch_swaps_logins_and_keeps_refreshed_tokens() {
    let home = scratch("codex-accounts");
    let codex = FileAgent::codex(&home);
    let root = home.join("accounts");

    write(&codex.live, &codex_auth("acct-a", "a@x.com", "a1"));
    let a = codex.save_current(&root).unwrap().id;
    assert!(
        codex.save_current(&root).is_err(),
        "the same login is saved once"
    );

    write(&codex.live, &codex_auth("acct-b", "b@x.com", "b1"));
    let b = codex.save_current(&root).unwrap().id;

    // Codex refreshed b's tokens while it was signed in; switching away keeps them.
    write(&codex.live, &codex_auth("acct-b", "b@x.com", "b2"));
    assert_eq!(codex.switch(&root, &a).unwrap(), "a@x.com");
    assert!(
        std::fs::read_to_string(&codex.live)
            .unwrap()
            .contains("\"a1\"")
    );
    let saved_b = root.join(&b).join(codex.saved);
    assert!(std::fs::read_to_string(saved_b).unwrap().contains("\"b2\""));
    assert_eq!(
        codex.switch(&root, &a).unwrap(),
        "a@x.com",
        "switching to the live one is a no-op"
    );

    // A login saved as no account is never overwritten.
    write(&codex.live, &codex_auth("acct-c", "c@x.com", "c1"));
    assert!(codex.switch(&root, &b).is_err());
    assert!(
        std::fs::read_to_string(&codex.live)
            .unwrap()
            .contains("\"c1\"")
    );

    assert!(
        codex.remove(&root, &a).is_ok(),
        "an account not signed in can go"
    );
    write(&codex.live, &codex_auth("acct-b", "b@x.com", "b3"));
    assert!(
        codex.remove(&root, &b).is_err(),
        "the signed-in account stays"
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_new_login_finishes_once_and_only_for_a_new_account() {
    let home = scratch("agy-accounts");
    let agy = FileAgent::agy(&home);
    let root = home.join("accounts");
    let first = "11111111-1111-4111-8111-111111111111";
    std::fs::create_dir_all(root.join(first)).unwrap();
    assert_eq!(agy.finish_login(&root, first).unwrap(), None);
    // agy writes the login with its own mode; the saved copy is made private.
    let path = root.join(first).join(agy.saved);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, agy_token("a@x.com", "t1")).unwrap();
    assert_eq!(
        agy.finish_login(&root, first).unwrap().as_deref(),
        Some("a@x.com")
    );
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(agy.accounts(&root).len(), 1);

    let again = "22222222-2222-4222-8222-222222222222";
    write(
        &root.join(again).join(agy.saved),
        &agy_token("A@x.com", "t2"),
    );
    assert!(
        agy.finish_login(&root, again).is_err(),
        "the same account is not added twice"
    );
    assert!(!root.join(again).exists());

    // Signed in as nobody saved: switching would lose that login, so it is refused.
    write(&agy.live, &agy_token("other@x.com", "t3"));
    assert!(agy.switch(&root, first).is_err());
    std::fs::remove_file(&agy.live).unwrap();
    assert_eq!(
        agy.switch(&root, first).unwrap(),
        "a@x.com",
        "no login yet: just sign in"
    );
    assert!(std::fs::read_to_string(&agy.live).unwrap().contains("t1"));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn reads_a_real_grok_login_entry() {
    // grok 1.0.x `auth.json`: one entry per issuer and team, with the fields of a real login.
    let auth = json!({ "https://auth.x.ai::00000000-0000-4000-8000-000000000000": {
        "auth_mode": "oidc", "coding_data_retention_opt_out": false, "create_time": "2026-08-12T00:00:00Z",
        "email": "g@x.com", "expires_at": "2026-08-19T00:00:00Z", "first_name": "G", "last_name": "X",
        "key": jwt(json!({ "sub": "sub-1", "principal_id": "p-1", "team_id": "t-1" })),
        "oidc_client_id": "c", "oidc_issuer": "https://auth.x.ai", "principal_id": "p-1",
        "principal_type": "user", "profile_image_asset_id": null, "refresh_token": "r",
        "team_id": "t-1", "user_id": "u-1",
    } });
    let login = grok_login(&auth).unwrap();
    assert_eq!(
        (login.key.as_deref(), login.email.as_deref()),
        (Some("u-1"), Some("g@x.com"))
    );
}
