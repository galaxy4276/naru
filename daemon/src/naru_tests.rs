use super::*;

#[test]
fn accepts_only_plain_nonces() {
    assert!(is_nonce("0123456789abcdef"));
    assert!(is_nonce("abc-1234"));
    assert!(!is_nonce("short"));
    assert!(!is_nonce("../../../etc/passwd"));
    assert!(!is_nonce(&"a".repeat(65)));
}

#[test]
fn reads_the_helpers_requests() {
    let tell: Request = serde_json::from_str(
        r#"{"token":"t","reply":"0123456789","op":"tell","to":"parent","kind":"block","text":"hi","now":false}"#,
    )
    .unwrap();
    assert_eq!(
        tell.op,
        Op::Tell {
            to: "parent".into(),
            kind: Kind::Block,
            text: "hi".into(),
            now: false
        }
    );
    let send: Request =
        serde_json::from_str(r#"{"token":"t","reply":"0123456789","op":"send","to":"a2"}"#)
            .unwrap();
    assert!(matches!(send.op, Op::Send { enter: false, .. }));
    let watch =
        r#"{"token":"t","reply":"0123456789","op":"tell","to":"a1","kind":"watch","text":"x"}"#;
    assert!(
        serde_json::from_str::<Request>(watch).is_err(),
        "only Naru writes watch letters"
    );
}

#[test]
fn shows_only_existing_absolute_files() {
    let dir = std::env::temp_dir().join(format!("naru-view-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.md");
    std::fs::write(&report, "# r\n\n## Findings\n\n### x\n").unwrap();
    let body = |path: &str, kind: &str| format!(r#"{{"path":"{path}"{kind}}}"#);
    let (path, proof) = parse_view(&body(report.to_str().unwrap(), r#","kind":"proof""#)).unwrap();
    assert!(proof && path.ends_with("report.md"));
    assert!(has_findings(&path));
    assert!(parse_view(&body("report.md", "")).is_none());
    assert!(parse_view(&body(dir.to_str().unwrap(), "")).is_none());
    assert!(parse_view(&body("/nonexistent/x.md", "")).is_none());
    std::fs::write(&report, "# r\nno findings heading\n").unwrap();
    assert!(!has_findings(&report));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn publishes_a_private_token() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("naru-token-{}", uuid::Uuid::new_v4()));
    let token = publish(&dir).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("token")).unwrap(), token);
    let mode = std::fs::metadata(dir.join("token"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(dir.join("replies").is_dir());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_a_socket_another_naru_serves() {
    let dir = std::env::temp_dir().join(format!(
        "naru-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(refuse_if_served(&dir).is_ok());
    let listener = UnixListener::bind(dir.join("naru.sock")).unwrap();
    assert!(refuse_if_served(&dir).is_err());
    drop(listener);
    assert!(
        refuse_if_served(&dir).is_ok(),
        "a socket file nobody answers is not a running Naru"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
