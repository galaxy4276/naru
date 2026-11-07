use super::*;

fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-marketing-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn first_load_writes_organic_lab_place() {
    let home = temp();
    let path = home.join("marketing.json");
    std::fs::create_dir_all(home.join("projects/organic-lab")).unwrap();
    let settings = load(&path, &home).unwrap();
    assert_eq!(settings.url, "http://127.0.0.1:3900/app/");
    assert!(settings.dir.ends_with("projects/organic-lab"));
    assert_eq!(load(&path, &home).unwrap(), settings);
}

#[test]
fn refuses_remote_url_missing_dir_and_odd_session() {
    let dir = temp().to_string_lossy().into_owned();
    let ok = Settings {
        dir: dir.clone(),
        url: "http://127.0.0.1:3900/app/".into(),
        session: None,
    };
    assert!(validate(&ok).is_ok());
    let remote = Settings {
        url: "https://example.com/app/".into(),
        ..ok.clone()
    };
    assert!(
        validate(&remote)
            .unwrap_err()
            .to_string()
            .contains("127.0.0.1")
    );
    let missing = Settings {
        dir: format!("{dir}/nope"),
        ..ok.clone()
    };
    assert!(
        validate(&missing)
            .unwrap_err()
            .to_string()
            .contains("폴더가 없어요")
    );
    let odd = Settings {
        session: Some("x; rm -rf ~".into()),
        ..ok
    };
    assert!(validate(&odd).unwrap_err().to_string().contains("세션 id"));
}

#[test]
fn server_down_is_false() {
    assert!(!server_up("http://127.0.0.1:9/app/"));
}
