use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn reads_the_claude_statusline_cache() {
    let seen = parse_claude_cache(
        r#"{"five_hour":{"utilization":29,"resets_at":"2026-11-06T10:00:00+00:00"},
            "seven_day":{"utilization":88.5,"resets_at":null},"cached_at":"2026-11-06T08:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(seen.windows.len(), 2);
    assert_eq!(
        (seen.windows[0].minutes, seen.windows[0].percent),
        (300, 29.0)
    );
    assert_eq!(seen.windows[0].resets_at, Some(1_793_959_200));
    assert_eq!(
        (seen.windows[1].percent, seen.windows[1].resets_at),
        (88.5, None)
    );
    assert_eq!(seen.at, 1_793_952_000);
    assert!(parse_claude_cache(r#"{"cached_at":"2026-11-06T08:00:00Z"}"#).is_none());
    assert!(parse_claude_cache(r#"{"five_hour":{"utilization":1}}"#).is_none());
}

#[test]
fn reads_the_newest_codex_rate_limits() {
    let line = |percent: f64, at: &str| {
        serde_json::json!({
            "timestamp": at,
            "payload": { "rate_limits": {
                "primary": { "used_percent": percent, "window_minutes": 300, "resets_at": 1_793_959_200 },
                "secondary": { "used_percent": 22.0, "window_minutes": 10080, "resets_at": 1_794_000_000 },
            }},
        })
        .to_string()
    };
    let tail = [
        line(10.0, "2026-11-06T07:00:00Z"),
        line(12.0, "2026-11-06T08:00:00Z"),
        r#"{"payload":{"type":"message"}}"#.into(),
    ]
    .join("\n");
    let seen = parse_codex_tail(&tail).unwrap();
    assert_eq!(seen.at, 1_793_952_000);
    assert_eq!(seen.windows[0].percent, 12.0);
    assert_eq!(seen.windows[1].minutes, 10080);
    assert!(parse_codex_tail(r#"{"payload":{"rate_limits":null}}"#).is_none());
}

#[test]
fn agy_usage_keeps_the_most_used_group_per_window() {
    let text = r#"{"status":"SUCCESS","command":{"data":{"groups":[
        {"buckets":[{"window":"weekly","remaining_fraction":0.9,"reset_time":"2026-11-10T00:00:00Z"},
                    {"window":"5h","remaining_fraction":1.0}]},
        {"buckets":[{"window":"weekly","remaining_fraction":0.25},{"window":"daily","remaining_fraction":0}]}
    ]}}}"#;
    let seen = parse_agy(text, 7).unwrap();
    assert_eq!(seen.at, 7);
    assert_eq!(seen.windows.len(), 2);
    assert_eq!(
        (seen.windows[0].minutes, seen.windows[0].percent),
        (300, 0.0)
    );
    assert_eq!(
        (seen.windows[1].percent, seen.windows[1].resets_at),
        (75.0, None)
    );
    assert!(parse_agy(r#"{"status":"ERROR"}"#, 0).is_err());
    assert!(
        parse_agy(
            r#"{"status":"SUCCESS","command":{"data":{"groups":[]}}}"#,
            0
        )
        .is_err()
    );
}

#[test]
fn reads_a_real_agy_usage_reply() {
    // agy 1.5.2 `-p /usage --output-format json`, descriptions emptied.
    let seen = parse_agy(include_str!("testdata/agy-usage.json"), 7).unwrap();
    let shown: Vec<_> = seen
        .windows
        .iter()
        .map(|w| (w.minutes, w.percent, w.resets_at))
        .collect();
    assert_eq!(
        shown,
        [
            (300, 2.6, rfc3339(&Value::from("2026-11-06T13:10:17Z"))),
            (10080, 0.7, rfc3339(&Value::from("2026-11-13T08:10:17Z"))),
        ]
    );
}

#[test]
fn usage_is_kept_only_when_newer() {
    let mut store = UsageStore::new();
    let seen = |at| Seen {
        windows: Vec::new(),
        at,
    };
    assert!(record(&mut store, AgentKind::Codex, "a", seen(5)));
    assert!(!record(&mut store, AgentKind::Codex, "a", seen(5)));
    assert!(!record(&mut store, AgentKind::Codex, "a", seen(4)));
    assert!(record(&mut store, AgentKind::Claude, "a", seen(4)));
    assert_eq!(store["codex:a"].at, 5);
}

#[test]
fn codex_switch_is_refused_only_when_its_config_names_another_store() {
    let dir = scratch("codex-config");
    let config = dir.join("config.toml");
    assert!(
        !codex_uses_keychain(&dir),
        "no config keeps the default file store"
    );
    for (text, keychain) in [
        ("model = \"x\"\n", false),
        ("cli_auth_credentials_store = \"file\" # keep\n", false),
        ("cli_auth_credentials_store = \"keyring\"\n", true),
        ("cli_auth_credentials_store = \"auto\"\n", true),
        (
            "[profile]\ncli_auth_credentials_store = \"keyring\"\n",
            false,
        ),
    ] {
        std::fs::write(&config, text).unwrap();
        assert_eq!(codex_uses_keychain(&dir), keychain, "{text}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn detail_skips_the_email_the_label_already_shows() {
    assert_eq!(
        detail([Some("a@x.com"), Some("max")], false).as_deref(),
        Some("max")
    );
    assert_eq!(
        detail([Some("a@x.com"), Some("max")], true).as_deref(),
        Some("a@x.com · max")
    );
    assert_eq!(detail([Some("a@x.com"), None], false), None);
}
