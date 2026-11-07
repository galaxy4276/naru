use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-ui-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("ui.json")
}

#[test]
fn settings_are_saved_replaced_and_forgotten() {
    let file = scratch("roundtrip");
    assert!(read(&file).unwrap().is_empty());

    set(&file, "naru.rail.order", Some(r#"["2","1"]"#)).unwrap();
    set(&file, "naru.theme", Some("dark")).unwrap();
    set(&file, "naru.theme", Some("light")).unwrap();
    set(&file, "naru.rail.order", None).unwrap();

    let values = read(&file).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values["naru.theme"], "light");
}

#[test]
fn only_small_naru_settings_are_kept() {
    let file = scratch("limits");
    assert!(set(&file, "other.key", Some("x")).is_err());
    let huge = "x".repeat(MAX_VALUE + 1);
    let error = set(&file, "naru.rail.thumbs", Some(&huge)).unwrap_err();
    assert!(error.to_string().contains("naru.rail.thumbs"));
    assert!(read(&file).unwrap().is_empty());
}

#[test]
fn an_unreadable_file_is_not_overwritten() {
    let file = scratch("corrupt");
    std::fs::write(&file, b"not json").unwrap();
    assert!(set(&file, "naru.theme", Some("dark")).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"not json");
}
